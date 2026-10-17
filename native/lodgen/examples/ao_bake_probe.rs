//! Prototype: bake UV-space ambient occlusion from an FO4 NIF and compare it with FO76's `_l.G`.
//!
//! ao_bake_probe index <fo4 extracted root> <out.json>
//! ao_bake_probe bake <fo4 extracted root> <index.json> <texture stem> <fo76 _l.dds> <out dir> [rays] [fractions...]
//!
//! `bake` bakes every FO4 mesh whose shapes use `<stem>_d.dds` into one map (texels averaged). Within a
//! mesh every triangle occludes, front faces only; rays reach `fraction` x that mesh's diagonal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lodgen_native::objects::fo76_bto::extract_inline_geometry;
use nif_core_native::model::{NifBlock, NifFile, NifValue};
use rayon::prelude::*;

type V3 = [f32; 3];

static BOTH_FACES: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 0.0 { mul(a, 1.0 / l) } else { [0.0, 0.0, 1.0] }
}

// ------------------------------------------------------------------ NIF

fn val_array(value: Option<&NifValue>) -> &[NifValue] {
    match value {
        Some(NifValue::Array(items)) => items,
        _ => &[],
    }
}

fn val_string(value: Option<&NifValue>) -> Option<String> {
    match value {
        Some(NifValue::String(s)) | Some(NifValue::Char(s)) => Some(s.trim_end_matches('\0').to_string()),
        _ => None,
    }
}

fn val_f32(value: Option<&NifValue>) -> Option<f32> {
    match value {
        Some(NifValue::Float(v)) => Some(*v as f32),
        Some(NifValue::Int(v)) => Some(*v as f32),
        Some(NifValue::UInt(v)) => Some(*v as f32),
        _ => None,
    }
}

fn vec3(value: Option<&NifValue>) -> Option<V3> {
    match value {
        Some(NifValue::Vec3(v)) => Some(*v),
        Some(NifValue::Struct(f)) => Some([val_f32(f.get("x"))?, val_f32(f.get("y"))?, val_f32(f.get("z"))?]),
        _ => None,
    }
}

struct Xf {
    t: V3,
    r: [[f32; 3]; 3],
    s: f32,
    transposed: bool,
}

impl Xf {
    fn of(block: &NifBlock, transposed: bool) -> Self {
        let r = match block.get_field("Rotation") {
            Some(NifValue::Matrix33(m)) => *m,
            _ => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        Xf { t: vec3(block.get_field("Translation")).unwrap_or([0.0; 3]), r, s: val_f32(block.get_field("Scale")).unwrap_or(1.0), transposed }
    }
    fn rotate(&self, p: V3) -> V3 {
        let r = &self.r;
        if self.transposed {
            [0, 1, 2].map(|i| r[0][i] * p[0] + r[1][i] * p[1] + r[2][i] * p[2])
        } else {
            [0, 1, 2].map(|i| r[i][0] * p[0] + r[i][1] * p[1] + r[i][2] * p[2])
        }
    }
    fn point(&self, p: V3) -> V3 {
        add(self.t, mul(self.rotate(p), self.s))
    }
}

fn block_ref(nif: &NifFile, block: &NifBlock, field: &str) -> Option<usize> {
    block.get_field(field).map(NifValue::as_i64).and_then(|v| usize::try_from(v).ok()).filter(|&i| i < nif.blocks.len())
}

fn parents(nif: &NifFile) -> HashMap<usize, usize> {
    let mut out = HashMap::new();
    for (id, block) in nif.blocks.iter().enumerate() {
        for child in val_array(block.get_field("Children")) {
            if let Ok(c) = usize::try_from(child.as_i64()) {
                out.entry(c).or_insert(id);
            }
        }
    }
    out
}

fn clean_path(s: &str) -> String {
    s.trim_end_matches('\0').replace('\\', "/").to_ascii_lowercase()
}

/// Diffuse texture of a shape, from its BGSM (FO4) or its inline texture set.
fn shape_diffuse(nif: &NifFile, shape: &NifBlock, root: &Path) -> Option<String> {
    let shader = &nif.blocks[block_ref(nif, shape, "Shader Property")?];
    if let Some(name) = val_string(shader.get_field("Name")).map(|n: String| clean_path(&n)).filter(|n| n.ends_with(".bgsm")) {
        let rel = if name.starts_with("materials/") { name } else { format!("materials/{name}") };
        let data = materials_native::bgsm::parse(&std::fs::read(root.join(&rel)).ok()?).ok()?;
        return Some(clean_path(&data.DiffuseTexture));
    }
    let set = &nif.blocks[block_ref(nif, shader, "Texture Set")?];
    match val_array(set.get_field("Textures")).first() {
        Some(NifValue::String(p)) => Some(clean_path(p)),
        _ => None,
    }
}

struct Tri {
    p: [V3; 3],
    n: [V3; 3],
    uv: [[f32; 2]; 3],
    bake: bool,
}

fn load(root: &Path, mesh: &Path, stem: &str, transposed: bool) -> Vec<Tri> {
    let nif = NifFile::from_bytes(&std::fs::read(mesh).expect("mesh"), None).expect("nif");
    let parents = parents(&nif);
    let want = format!("{stem}_d.dds");
    let mut tris = Vec::new();
    for shape in nif.blocks.iter().filter(|b| b.type_name == "BSTriShape" || b.type_name == "BSSubIndexTriShape") {
        let Some(geo) = extract_inline_geometry(shape) else { continue };
        let mut chain = vec![Xf::of(shape, transposed)];
        let mut id = shape.block_id;
        while let Some(&p) = parents.get(&id) {
            chain.push(Xf::of(&nif.blocks[p], transposed));
            id = p;
        }
        let diffuse = shape_diffuse(&nif, shape, root).unwrap_or_default();
        let bake = diffuse.trim_start_matches("textures/") == want;
        let world = |p: V3| chain.iter().fold(p, |acc, x| x.point(acc));
        let world_n = |n: V3| norm(chain.iter().fold(n, |acc, x| x.rotate(acc)));
        for t in &geo.triangles {
            let idx = t.map(|i| i as usize);
            if idx.iter().any(|&i| i >= geo.vertices.len()) {
                continue;
            }
            let p = idx.map(|i| world(geo.vertices[i]));
            let face = norm(cross(sub(p[1], p[0]), sub(p[2], p[0])));
            let n = if std::env::var_os("AO_FACE_NORMALS").is_some() {
                [face; 3]
            } else {
                idx.map(|i| geo.normals.get(i).map(|&n| world_n(n)).unwrap_or(face))
            };
            if std::env::var_os("AO_DEBUG").is_some() && tris.len() < 3 {
                eprintln!("face {face:?} vertex normals {n:?} raw {:?} uv {:?}", idx.map(|i| geo.normals.get(i).copied()), idx.map(|i| geo.uvcoords.get(i).copied()));
            }
            let uv = idx.map(|i| geo.uvcoords.get(i).copied().unwrap_or([0.0, 0.0]));
            tris.push(Tri { p, n, uv, bake });
        }
    }
    tris
}

// ------------------------------------------------------------------ BVH

struct Node {
    lo: V3,
    hi: V3,
    start: usize,
    count: usize, // 0 = interior; children at self+1 and `start`
}

struct Bvh {
    nodes: Vec<Node>,
    tris: Vec<[V3; 3]>,
}

impl Bvh {
    fn build(tris: Vec<[V3; 3]>) -> Self {
        let mut order: Vec<usize> = (0..tris.len()).collect();
        let mut nodes = Vec::new();
        Self::split(&tris, &mut order, 0, tris.len(), &mut nodes);
        let tris = order.iter().map(|&i| tris[i]).collect();
        Bvh { nodes, tris }
    }

    fn split(tris: &[[V3; 3]], order: &mut [usize], lo_i: usize, hi_i: usize, nodes: &mut Vec<Node>) -> usize {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for &t in &order[lo_i..hi_i] {
            for p in tris[t] {
                for a in 0..3 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
        }
        let me = nodes.len();
        nodes.push(Node { lo, hi, start: lo_i, count: hi_i - lo_i });
        if hi_i - lo_i <= 4 {
            return me;
        }
        let ext = sub(hi, lo);
        let axis = if ext[0] >= ext[1] && ext[0] >= ext[2] { 0 } else if ext[1] >= ext[2] { 1 } else { 2 };
        let c = |t: usize| tris[t][0][axis] + tris[t][1][axis] + tris[t][2][axis];
        order[lo_i..hi_i].sort_by(|&a, &b| c(a).total_cmp(&c(b)));
        let mid = (lo_i + hi_i) / 2;
        Self::split(tris, order, lo_i, mid, nodes);
        let right = Self::split(tris, order, mid, hi_i, nodes);
        nodes[me].count = 0;
        nodes[me].start = right;
        me
    }

    fn hits_box(n: &Node, o: V3, inv: V3, tmax: f32) -> bool {
        let (mut t0, mut t1) = (0.0f32, tmax);
        for a in 0..3 {
            let (mut ta, mut tb) = ((n.lo[a] - o[a]) * inv[a], (n.hi[a] - o[a]) * inv[a]);
            if ta > tb {
                std::mem::swap(&mut ta, &mut tb);
            }
            t0 = t0.max(ta);
            t1 = t1.min(tb);
            if t0 > t1 {
                return false;
            }
        }
        true
    }

    /// Möller–Trumbore, both faces, any hit in (eps, tmax).
    fn hits_tri(t: &[V3; 3], o: V3, d: V3, tmax: f32) -> bool {
        let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
        let p = cross(d, e2);
        let det = dot(e1, p);
        // Back faces don't occlude: a texel inside or under another part still bakes open, as FO76's do.
        if det < 1e-12 && (!*BOTH_FACES.get_or_init(|| std::env::var_os("AO_BOTH_FACES").is_some()) || det.abs() < 1e-12) {
            return false;
        }
        let inv = 1.0 / det;
        let s = sub(o, t[0]);
        let u = dot(s, p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return false;
        }
        let q = cross(s, e1);
        let v = dot(d, q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return false;
        }
        let t = dot(e2, q) * inv;
        t > 1e-4 && t < tmax
    }

    fn occluded(&self, o: V3, d: V3, tmax: f32) -> bool {
        let inv = d.map(|x| if x.abs() > 1e-12 { 1.0 / x } else { 1e12 });
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i];
            if !Self::hits_box(n, o, inv, tmax) {
                continue;
            }
            if n.count > 0 {
                if self.tris[n.start..n.start + n.count].iter().any(|t| Self::hits_tri(t, o, d, tmax)) {
                    return true;
                }
            } else {
                stack.push(i + 1);
                stack.push(n.start);
            }
        }
        false
    }
}

// ------------------------------------------------------------------ bake

fn hemisphere(rays: usize) -> Vec<V3> {
    // Cosine-weighted, stratified (Hammersley); +Z up.
    (0..rays)
        .map(|i| {
            let u1 = (i as f32 + 0.5) / rays as f32;
            let mut bits = i as u32;
            bits = bits.rotate_right(16);
            bits = ((bits & 0x5555_5555) << 1) | ((bits & 0xAAAA_AAAA) >> 1);
            bits = ((bits & 0x3333_3333) << 2) | ((bits & 0xCCCC_CCCC) >> 2);
            bits = ((bits & 0x0F0F_0F0F) << 4) | ((bits & 0xF0F0_F0F0) >> 4);
            bits = ((bits & 0x00FF_00FF) << 8) | ((bits & 0xFF00_FF00) >> 8);
            let u2 = bits as f32 / 4_294_967_296.0;
            let r = u1.sqrt();
            let phi = std::f32::consts::TAU * u2;
            [r * phi.cos(), r * phi.sin(), (1.0 - u1).sqrt()]
        })
        .collect()
}

fn basis(n: V3) -> (V3, V3) {
    let a = if n[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let t = norm(cross(a, n));
    (t, cross(n, t))
}

/// Returns per-texel AO (NaN where no baked triangle covers the texel) and the UV wrap count.
/// Adds this mesh's per-texel openness into `sum`/`cnt`; returns how many baked triangles sat outside the 0..1 tile.
fn bake(tris: &[Tri], bvh: &Bvh, w: u32, h: u32, rays: usize, max_dist: f32, sum: &mut [f32], cnt: &mut [u32]) -> usize {
    let dirs = hemisphere(rays);
    let eps: f32 = std::env::var("AO_EPS").ok().and_then(|v| v.parse().ok()).unwrap_or(0.01);
    let mut samples: Vec<(usize, V3, V3)> = Vec::new();
    let mut wrapped = 0usize;
    for t in tris.iter().filter(|t| t.bake) {
        // Tiling UVs map back into the 0..1 tile they start in.
        let off = [t.uv[0][0].floor(), t.uv[0][1].floor()];
        if off != [0.0, 0.0] {
            wrapped += 1;
        }
        let flip_v = std::env::var_os("AO_FLIP_V").is_some();
        let px: [[f32; 2]; 3] = t.uv.map(|uv| {
            let v = uv[1] - off[1];
            [(uv[0] - off[0]) * w as f32, if flip_v { 1.0 - v } else { v } * h as f32]
        });
        let (x0, x1) = (px.iter().map(|p| p[0]).fold(f32::MAX, f32::min), px.iter().map(|p| p[0]).fold(f32::MIN, f32::max));
        let (y0, y1) = (px.iter().map(|p| p[1]).fold(f32::MAX, f32::min), px.iter().map(|p| p[1]).fold(f32::MIN, f32::max));
        let area = (px[1][0] - px[0][0]) * (px[2][1] - px[0][1]) - (px[2][0] - px[0][0]) * (px[1][1] - px[0][1]);
        if area.abs() < 1e-9 {
            continue;
        }
        for y in (y0.floor().max(0.0) as i64)..=(y1.ceil() as i64).min(h as i64 - 1) {
            for x in (x0.floor().max(0.0) as i64)..=(x1.ceil() as i64).min(w as i64 - 1) {
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                let b1 = ((cx - px[0][0]) * (px[2][1] - px[0][1]) - (px[2][0] - px[0][0]) * (cy - px[0][1])) / area;
                let b2 = ((px[1][0] - px[0][0]) * (cy - px[0][1]) - (cx - px[0][0]) * (px[1][1] - px[0][1])) / area;
                let b0 = 1.0 - b1 - b2;
                if b0 < -1e-4 || b1 < -1e-4 || b2 < -1e-4 {
                    continue;
                }
                let pos = add(add(mul(t.p[0], b0), mul(t.p[1], b1)), mul(t.p[2], b2));
                let n = norm(add(add(mul(t.n[0], b0), mul(t.n[1], b1)), mul(t.n[2], b2)));
                samples.push(((y as usize) * w as usize + x as usize, pos, n));
            }
        }
    }
    let ao: Vec<(usize, f32)> = samples
        .par_iter()
        .map(|&(i, pos, n)| {
            let (t, b) = basis(n);
            let origin = add(pos, mul(n, eps));
            let open = dirs
                .iter()
                .filter(|d| !bvh.occluded(origin, add(add(mul(t, d[0]), mul(b, d[1])), mul(n, d[2])), max_dist))
                .count();
            (i, open as f32 / dirs.len() as f32)
        })
        .collect();
    for (i, v) in ao {
        sum[i] += v;
        cnt[i] += 1;
    }
    wrapped
}

fn dilate(img: &mut [f32], w: usize, h: usize, passes: usize) {
    for _ in 0..passes {
        let src = img.to_vec();
        for y in 0..h {
            for x in 0..w {
                if !src[y * w + x].is_nan() {
                    continue;
                }
                let (mut s, mut c) = (0.0, 0);
                for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        let v = src[ny as usize * w + nx as usize];
                        if !v.is_nan() {
                            s += v;
                            c += 1;
                        }
                    }
                }
                if c > 0 {
                    img[y * w + x] = s / c as f32;
                }
            }
        }
    }
}

fn stats(a: &[f32], b: &[f32]) -> (f32, f32, f32, f32, f32, usize) {
    let pairs: Vec<(f32, f32)> = a.iter().zip(b).filter(|(x, _)| !x.is_nan()).map(|(&x, &y)| (x, y)).collect();
    let n = pairs.len() as f32;
    let (ma, mb) = (pairs.iter().map(|p| p.0).sum::<f32>() / n, pairs.iter().map(|p| p.1).sum::<f32>() / n);
    let (mut sab, mut saa, mut sbb, mut mae) = (0.0, 0.0, 0.0, 0.0);
    for &(x, y) in &pairs {
        sab += (x - ma) * (y - mb);
        saa += (x - ma) * (x - ma);
        sbb += (y - mb) * (y - mb);
        mae += (x - y).abs();
    }
    let mut sa: Vec<f32> = pairs.iter().map(|p| p.0).collect();
    sa.sort_by(f32::total_cmp);
    (sab / (saa * sbb).sqrt().max(1e-12), ma, mb, mae / n, sa[sa.len() / 20], pairs.len())
}

const SKIP_MESH_DIRS: [&str; 4] = ["precombined/", "lod/", "terrain/", "interface/"];

fn nif_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            nif_files(&path, out);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("nif")) {
            out.push(path);
        }
    }
}

/// `<diffuse rel to textures/>` -> every FO4 mesh with a shape using it.
fn index(root: &Path, out: &Path) {
    let meshes = root.join("meshes");
    let mut files = Vec::new();
    nif_files(&meshes, &mut files);
    let files: Vec<String> = files
        .iter()
        .map(|p| p.strip_prefix(&meshes).unwrap().to_string_lossy().replace('\\', "/").to_ascii_lowercase())
        .filter(|rel| !SKIP_MESH_DIRS.iter().any(|d| rel.starts_with(d)))
        .collect();
    let pairs: Vec<(String, String)> = files
        .par_iter()
        .flat_map_iter(|rel| {
            let Some(nif) = std::fs::read(meshes.join(rel)).ok().and_then(|b| NifFile::from_bytes(&b, None).ok()) else {
                return Vec::new();
            };
            let mut found: Vec<(String, String)> = nif
                .blocks
                .iter()
                .filter(|b| b.type_name == "BSTriShape" || b.type_name == "BSSubIndexTriShape")
                .filter_map(|shape| shape_diffuse(&nif, shape, root))
                .map(|d| (d.trim_start_matches("textures/").to_string(), rel.clone()))
                .collect();
            found.sort();
            found.dedup();
            found
        })
        .collect();
    let mut map: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (texture, mesh) in pairs {
        map.entry(texture).or_default().push(mesh);
    }
    for meshes in map.values_mut() {
        meshes.sort();
    }
    println!("{} meshes scanned, {} textures indexed", files.len(), map.len());
    std::fs::write(out, serde_json::to_string(&map).unwrap()).unwrap();
}

fn bake_texture(args: &[String]) {
    let (root, index_path, stem, fo76_l, out) =
        (PathBuf::from(&args[0]), PathBuf::from(&args[1]), args[2].to_ascii_lowercase(), PathBuf::from(&args[3]), PathBuf::from(&args[4]));
    let rays: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(64);
    let fractions: Vec<f32> = args.iter().skip(6).filter_map(|s| s.parse().ok()).collect();
    let fractions = if fractions.is_empty() { vec![0.05, 0.1] } else { fractions };
    let transposed = std::env::var_os("AO_ASIS").is_none();
    std::fs::create_dir_all(&out).unwrap();
    let index: std::collections::BTreeMap<String, Vec<String>> =
        serde_json::from_str(&std::fs::read_to_string(&index_path).unwrap()).unwrap();
    let Some(meshes) = index.get(&format!("{stem}_d.dds")) else {
        println!("{stem}: no FO4 mesh uses it");
        return;
    };

    let reference = directxtex_native::read_dds_rgba_image(&fo76_l).expect("fo76 _l");
    let (w, h) = (reference.width, reference.height);
    let fo76_ao: Vec<f32> = reference.rgba.chunks_exact(4).map(|p| p[1] as f32 / 255.0).collect();

    // Kit textures can have hundreds of meshes; a capped sample still covers most of the sheet.
    let cap: usize = std::env::var("AO_MAX_MESHES").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let loaded: Vec<(Vec<Tri>, Bvh, f32)> = meshes
        .iter()
        .take(cap)
        .map(|rel| {
            let tris = load(&root, &root.join("meshes").join(rel), stem.as_str(), transposed);
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for t in &tris {
                for p in t.p {
                    for a in 0..3 {
                        lo[a] = lo[a].min(p[a]);
                        hi[a] = hi[a].max(p[a]);
                    }
                }
            }
            let diag = dot(sub(hi, lo), sub(hi, lo)).sqrt();
            let bvh = Bvh::build(tris.iter().map(|t| t.p).collect());
            (tris, bvh, diag)
        })
        .collect();
    let flat = stem.replace('/', "__");
    for &f in &fractions {
        let started = std::time::Instant::now();
        let mut sum = vec![0f32; (w * h) as usize];
        let mut cnt = vec![0u32; (w * h) as usize];
        let mut wrapped = 0;
        for (tris, bvh, diag) in &loaded {
            wrapped += bake(tris, bvh, w, h, rays, diag * f, &mut sum, &mut cnt);
        }
        let ao: Vec<f32> = sum.iter().zip(&cnt).map(|(s, &c)| if c > 0 { s / c as f32 } else { f32::NAN }).collect();
        let (r, ma, mb, mae, p5, covered) = stats(&ao, &fo76_ao);
        println!(
            "{stem} meshes {} f {f:.2}: covered {:.1}% corr {r:.3} baked mean {ma:.3} p5 {p5:.3} fo76 mean {mb:.3} MAE {mae:.3} wrapped {wrapped} {:.1}s",
            loaded.len(),
            100.0 * covered as f32 / (w * h) as f32,
            started.elapsed().as_secs_f32()
        );
        // RGB = AO dilated across seams; A = 255 where a triangle covered the texel.
        let mut img = ao.clone();
        dilate(&mut img, w as usize, h as usize, 8);
        let bytes: Vec<u8> = img
            .iter()
            .zip(&ao)
            .flat_map(|(&v, &raw)| {
                let g = if v.is_nan() { 255 } else { (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 };
                [g, g, g, if raw.is_nan() { 0 } else { 255 }]
            })
            .collect();
        let name = format!("{flat}_ao_{:03}.dds", (f * 100.0).round() as u32);
        directxtex_native::write_dds_rgba_image(&out.join(name), w, h, &bytes, "R8G8B8A8_UNORM", false).unwrap();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("index") => index(Path::new(&args[2]), Path::new(&args[3])),
        Some("bake") => bake_texture(&args[2..]),
        _ => eprintln!(
            "usage: ao_bake_probe index <fo4 root> <out.json> | bake <fo4 root> <index.json> <stem> <fo76 _l.dds> <out dir> [rays] [fractions...]"
        ),
    }
}
