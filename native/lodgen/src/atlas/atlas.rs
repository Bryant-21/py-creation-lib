/// Object LOD atlas — atlas-map .txt writer/reader, AtlasRect UV derivation, AtlasList.
///
/// Ports:
/// - `write_atlas_map`          → wbLOD.pas:1557-1566
/// - `parse_atlas_map`          → wbLOD.pas:1607-1655
/// - `AtlasRect::from_map_row`  → Program.cs:1350-1359
/// - `AtlasRect::uv_atlas`      → AtlasDesc.cs:85-92
/// - `AtlasList` (keyed map, case-insensitive) → AtlasList.cs
/// - `build_object_atlas`       → wbBuildAtlasFromTexturesList (wbLOD.pas:1402)
///                                + TexturesList gather (LODApp.cs:3507)
///                                + atlas-map→AtlasList load (Program.cs:1290)
use indexmap::IndexMap;
use rayon::prelude::*;
use std::path::Path;

// ---------------------------------------------------------------------------
// AtlasMapRow — one row in the 8-column TSV
// ---------------------------------------------------------------------------

/// One row in the atlas-map text file (wbLOD.pas:1557-1566, cols 0-7).
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasMapRow {
    /// Source tile texture path (Data-relative, backslash-separated).
    pub source: String,
    /// Tile width in pixels (after any resize).
    pub tile_w: u32,
    /// Tile height in pixels.
    pub tile_h: u32,
    /// X position of the tile in the atlas image.
    pub x: u32,
    /// Y position of the tile in the atlas image.
    pub y: u32,
    /// Atlas texture path (Data-relative, backslash-separated).
    pub atlas: String,
    /// Atlas image width.
    pub atlas_w: u32,
    /// Atlas image height.
    pub atlas_h: u32,
}

// ---------------------------------------------------------------------------
// write_atlas_map / parse_atlas_map
// ---------------------------------------------------------------------------

/// Write the 8-column TAB-separated atlas-map `.txt` file.
/// Port: wbLOD.pas:1557-1566 — `Name #9 W #9 H #9 X #9 Y #9 AtlasName #9 W #9 H`,
/// one line per row, LF line endings, no BOM.
pub fn write_atlas_map(path: &Path, rows: &[AtlasMapRow]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut buf = Vec::with_capacity(rows.len() * 128);
    for r in rows {
        write!(
            buf,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            r.source, r.tile_w, r.tile_h, r.x, r.y, r.atlas, r.atlas_w, r.atlas_h
        )?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &buf)?;
    Ok(())
}

/// Parse an atlas-map `.txt` file.  Lines with column count != 8 are silently
/// skipped (port: wbLOD.pas:1607-1655 — `sl.Count` must = 8).
pub fn parse_atlas_map(text: &str) -> Vec<AtlasMapRow> {
    text.lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() != 8 {
                return None;
            }
            Some(AtlasMapRow {
                source: cols[0].to_string(),
                tile_w: cols[1].parse().ok()?,
                tile_h: cols[2].parse().ok()?,
                x: cols[3].parse().ok()?,
                y: cols[4].parse().ok()?,
                atlas: cols[5].to_string(),
                atlas_w: cols[6].parse().ok()?,
                atlas_h: cols[7].parse().ok()?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// AtlasRect — per-tile UV bake parameters
// ---------------------------------------------------------------------------

/// Per-tile UV bake parameters derived from an atlas-map row.
/// Port: Program.cs:1350-1359 (from_map_row) + AtlasDesc.cs:85-92 (uv_atlas).
#[derive(Clone, Debug)]
pub struct AtlasRect {
    /// Tile X origin in atlas (UV space: pos_u = x / atlas_w).
    pub pos_u: f32,
    /// Tile Y origin in atlas (UV space: pos_v = y / atlas_h).
    pub pos_v: f32,
    /// UV scale (default 1.0).
    pub scale_u: f32,
    /// UV scale (default 1.0).
    pub scale_v: f32,
    /// Inner clamp min U (= 1/(atlas_w*2)).
    pub min_u: f32,
    /// Inner clamp max U (= texture_scale_u - min_u).
    pub max_u: f32,
    /// Inner clamp min V.
    pub min_v: f32,
    /// Inner clamp max V.
    pub max_v: f32,
    /// tile_w / atlas_w (the fraction of the atlas this tile occupies horizontally).
    pub texture_scale_u: f32,
    /// tile_h / atlas_h.
    pub texture_scale_v: f32,
    /// True if this tile is an HD (non-atlasable) texture (AtlasDesc.HDTexture).
    pub hd_texture: bool,
    /// Atlas diffuse path (the atlas .dds, not the source tile).
    pub atlas_diffuse: String,
    /// Atlas normal path (_n.dds).
    pub atlas_normal: String,
    /// Atlas specular path (_s.dds).
    pub atlas_specular: String,
}

impl AtlasRect {
    /// Derive an `AtlasRect` from an atlas-map row.
    /// Port: Program.cs:1350-1359.
    pub fn from_map_row(
        tile_w: u32,
        tile_h: u32,
        pos_x: u32,
        pos_y: u32,
        atlas_w: u32,
        atlas_h: u32,
        atlas_diffuse: &str,
        hd: bool,
    ) -> Self {
        let aw = atlas_w as f32;
        let ah = atlas_h as f32;
        let texture_scale_u = tile_w as f32 / aw;
        let texture_scale_v = tile_h as f32 / ah;
        let pos_u = pos_x as f32 / aw;
        let pos_v = pos_y as f32 / ah;
        let min_u = 1.0_f32 / (aw * 2.0);
        let min_v = 1.0_f32 / (ah * 2.0);
        let max_u = texture_scale_u - min_u;
        let max_v = texture_scale_v - min_v;

        // Derive _n and _s atlas paths from the diffuse path.
        let atlas_normal = derive_normal_atlas(atlas_diffuse);
        let atlas_specular = derive_specular_atlas(atlas_diffuse);

        AtlasRect {
            pos_u,
            pos_v,
            scale_u: 1.0,
            scale_v: 1.0,
            min_u,
            max_u,
            min_v,
            max_v,
            texture_scale_u,
            texture_scale_v,
            hd_texture: hd,
            atlas_diffuse: atlas_diffuse.to_string(),
            atlas_normal,
            atlas_specular,
        }
    }

    /// Remap a source (u,v) into atlas UV space.
    /// Port: AtlasDesc.cs:85-92.
    ///
    /// ```text
    /// u *= TextureScaleU; v *= TextureScaleV;
    /// u = clamp(u, MinU, MaxU); v = clamp(v, MinV, MaxV);
    /// u += PosU; v += PosV;
    /// ```
    pub fn uv_atlas(&self, u: f32, v: f32) -> (f32, f32) {
        let u = (u * self.texture_scale_u).clamp(self.min_u, self.max_u) + self.pos_u;
        let v = (v * self.texture_scale_v).clamp(self.min_v, self.max_v) + self.pos_v;
        (u, v)
    }
}

/// Derive the normal-atlas path from the diffuse atlas path.
/// Replaces the final `.dds` suffix with `_n.dds`.
fn derive_normal_atlas(diffuse: &str) -> String {
    let lower = diffuse.to_lowercase();
    if let Some(pos) = lower.rfind(".dds") {
        format!("{}_n.dds", &diffuse[..pos])
    } else {
        format!("{}_n.dds", diffuse)
    }
}

/// Derive the specular-atlas path from the diffuse atlas path.
fn derive_specular_atlas(diffuse: &str) -> String {
    let lower = diffuse.to_lowercase();
    if let Some(pos) = lower.rfind(".dds") {
        format!("{}_s.dds", &diffuse[..pos])
    } else {
        format!("{}_s.dds", diffuse)
    }
}

// ---------------------------------------------------------------------------
// AtlasList — case-insensitive, insertion-ordered keyed map
// ---------------------------------------------------------------------------

/// Case-insensitive, insertion-ordered map from texture key → `AtlasRect`.
/// Port: AtlasList.cs — `Dictionary<string,AtlasDesc>(StringComparer.OrdinalIgnoreCase)`.
/// We store all keys lowercased so lookups are O(1) case-insensitive.
pub struct AtlasList {
    /// Keys stored lowercased; insertion order preserved by IndexMap.
    map: IndexMap<String, AtlasRect>,
}

impl AtlasList {
    pub fn new() -> Self {
        AtlasList {
            map: IndexMap::new(),
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.map.contains_key(&key.to_lowercase())
    }

    pub fn get(&self, key: &str) -> Option<&AtlasRect> {
        self.map.get(&key.to_lowercase())
    }

    /// Insert with the given key (stored lowercased).
    pub fn insert(&mut self, key: String, rect: AtlasRect) {
        self.map.insert(key.to_lowercase(), rect);
    }

    /// Test-friendly alias: insert with the given key (stored lowercased).
    pub fn insert_key(&mut self, key: String, rect: AtlasRect) {
        self.insert(key, rect);
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterate over (key, rect) pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &AtlasRect)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl Default for AtlasList {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// AtlasResult — handoff between build_object_atlas and objects::generate_quad
// ---------------------------------------------------------------------------

/// Result of the once-per-worldspace atlas build pass.
/// Contract fields: `map_path`, `diffuse`, `uv`.
/// Additive fields: `normal`, `specular`, `atlas_size`, `list`.
pub struct AtlasResult {
    /// Path to the written atlas-map `.txt` file.
    pub map_path: std::path::PathBuf,
    /// Path to the diffuse atlas DDS.
    pub diffuse: std::path::PathBuf,
    /// Path to the normal atlas DDS.
    pub normal: std::path::PathBuf,
    /// Path to the specular atlas DDS.
    pub specular: std::path::PathBuf,
    /// Final atlas dimensions in pixels (width, height).
    pub atlas_size: (u32, u32),
    /// Per-tile UV rects keyed by texture key (same key as `AtlasList`).
    pub uv: std::collections::HashMap<String, AtlasRect>,
    /// Loaded `AtlasList` (insertion-ordered, case-insensitive).
    pub list: AtlasList,
    /// Number of DDS files actually written to disk (0–3; < 3 means partial encode failure).
    pub dds_written: u32,
}

// ---------------------------------------------------------------------------
// build_object_atlas
// ---------------------------------------------------------------------------

/// Gather object-LOD diffuse tiles from `refs`, pack them into one or more
/// atlas DDS images, write the atlas-map `.txt`, and load it back into an
/// `AtlasList`.
///
/// Port: `wbBuildAtlasFromTexturesList` (wbLOD.pas:1402-1581) +
///       `LODApp.TexturesList` gather (LODApp.cs:3507) +
///       atlas-map→AtlasList load (Program.cs:1290-1382).
pub fn build_object_atlas(
    refs: &[crate::input::StaticDesc],
    ctx: &crate::progress::QuadCtx<'_>,
) -> anyhow::Result<AtlasResult> {
    build_object_atlas_with_progress(refs, ctx, None)
}

fn report_progress(
    progress: &mut Option<&mut dyn crate::progress::Progress>,
    msg: &str,
    frac: f32,
) {
    if let Some(p) = progress.as_deref_mut() {
        p.report(msg, frac);
    }
}

pub fn build_object_atlas_with_progress(
    refs: &[crate::input::StaticDesc],
    ctx: &crate::progress::QuadCtx<'_>,
    mut progress: Option<&mut dyn crate::progress::Progress>,
) -> anyhow::Result<AtlasResult> {
    let settings = &ctx.settings.objects;
    let world = ctx.world.editor_id.as_str();

    let uv_range = settings.uv_range;
    let tol_min = 0.0_f32 - (uv_range - 1.0);
    let tol_max = 1.0_f32 + (uv_range - 1.0);

    let mut seen_diffuse: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut diffuse_paths: Vec<std::path::PathBuf> = Vec::new();

    let total_refs = refs.len().max(1);
    let report_every = (total_refs / 20).max(1);
    report_progress(
        &mut progress,
        &format!("object atlas: scanning {} refs", refs.len()),
        0.0,
    );

    let (ref_keys, scan_jobs) = build_atlas_scan_jobs(refs);
    let worker_count = atlas_worker_count(ctx.settings.global.workers);
    if !refs.is_empty() {
        report_progress(
            &mut progress,
            &format!(
                "object atlas: scan jobs unique={} workers={}",
                scan_jobs.len(),
                worker_count
            ),
            0.0,
        );
    }
    let scan_cache = collect_atlas_scan_cache(&scan_jobs, ctx, tol_min, tol_max, worker_count);
    let cache_hits = refs.len().saturating_sub(scan_jobs.len());

    for (idx, key) in ref_keys.iter().enumerate() {
        if let Some(tiles) = scan_cache.get(key) {
            for tile in tiles {
                if seen_diffuse.insert(tile.key.clone()) {
                    if let Some(path) = &tile.path {
                        diffuse_paths.push(path.clone());
                    }
                }
            }
        }
        let done = idx + 1;
        if done == refs.len() || done % report_every == 0 {
            report_progress(
                &mut progress,
                &format!(
                    "object atlas: scanned {done}/{} refs, {} texture tiles, cache={} hits={}",
                    refs.len(),
                    diffuse_paths.len(),
                    scan_cache.len(),
                    cache_hits
                ),
                (done as f32 / total_refs as f32) * 0.45,
            );
        }
    }

    if diffuse_paths.is_empty() {
        // No valid tiles — return an empty AtlasResult without error.
        report_progress(&mut progress, "object atlas: no valid texture tiles", 1.0);
        return Ok(AtlasResult {
            map_path: std::path::PathBuf::new(),
            diffuse: std::path::PathBuf::new(),
            normal: std::path::PathBuf::new(),
            specular: std::path::PathBuf::new(),
            atlas_size: (0, 0),
            uv: std::collections::HashMap::new(),
            list: AtlasList::new(),
            dds_written: 0,
        });
    }

    // --- Step 2: resolve output paths ---
    // port: naming::object_atlas
    let atlas_rel = crate::naming::object_atlas(world);
    let atlas_path = ctx.paths.output_dir.join(atlas_rel.replace('\\', "/"));
    let map_path = atlas_path.with_extension("txt");

    // --- Step 3: build the atlas DDS + map + AtlasList ---
    let format_diffuse = format_to_str(&settings.diffuse_format);
    let format_normal = format_to_str(&settings.normal_format);
    let format_specular = format_to_str(&settings.specular_format);

    build_atlas_from_tiles_with_progress(
        &diffuse_paths,
        &atlas_path,
        &map_path,
        settings.atlas_size,
        settings.max_tile_size,
        format_diffuse,
        format_normal,
        format_specular,
        settings.atlas_mip_flooding,
        progress,
    )
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct AtlasScanKey {
    lod_models: [Option<String>; 4],
    material_name: String,
    base_flags: u32,
    is_billboard: bool,
    is_grass: bool,
    material_swap: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
struct AtlasTileCandidate {
    key: String,
    path: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug)]
struct AtlasScanJob {
    key: AtlasScanKey,
    stat: crate::input::StaticDesc,
}

impl AtlasScanKey {
    fn from_static(stat: &crate::input::StaticDesc) -> Self {
        Self {
            lod_models: stat.lod_models.clone(),
            material_name: stat.material_name.clone(),
            base_flags: stat.base_flags,
            is_billboard: stat.is_billboard,
            is_grass: stat.is_grass,
            material_swap: stat
                .material_swap
                .iter()
                .map(|(from, to)| (from.clone(), to.clone()))
                .collect(),
        }
    }
}

fn atlas_worker_count(configured_workers: usize) -> usize {
    if configured_workers > 0 {
        return configured_workers;
    }
    std::thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(1)
        .max(1)
}

fn build_atlas_scan_jobs(
    refs: &[crate::input::StaticDesc],
) -> (Vec<AtlasScanKey>, Vec<AtlasScanJob>) {
    let mut seen: std::collections::HashSet<AtlasScanKey> = std::collections::HashSet::new();
    let mut ref_keys: Vec<AtlasScanKey> = Vec::with_capacity(refs.len());
    let mut jobs: Vec<AtlasScanJob> = Vec::new();

    for stat in refs {
        let key = AtlasScanKey::from_static(stat);
        ref_keys.push(key.clone());
        if seen.insert(key.clone()) {
            jobs.push(AtlasScanJob {
                key,
                stat: stat.clone(),
            });
        }
    }

    (ref_keys, jobs)
}

fn collect_atlas_scan_cache(
    jobs: &[AtlasScanJob],
    ctx: &crate::progress::QuadCtx<'_>,
    tol_min: f32,
    tol_max: f32,
    workers: usize,
) -> std::collections::HashMap<AtlasScanKey, Vec<AtlasTileCandidate>> {
    let scan = |job: &AtlasScanJob| {
        (
            job.key.clone(),
            collect_object_atlas_tiles_for_stat(&job.stat, ctx, tol_min, tol_max),
        )
    };

    let results: Vec<(AtlasScanKey, Vec<AtlasTileCandidate>)> = if workers > 1 && jobs.len() > 1 {
        match rayon::ThreadPoolBuilder::new().num_threads(workers).build() {
            Ok(pool) => pool.install(|| jobs.par_iter().map(scan).collect()),
            Err(_) => jobs.iter().map(scan).collect(),
        }
    } else {
        jobs.iter().map(scan).collect()
    };

    results.into_iter().collect()
}

fn collect_object_atlas_tiles_for_stat(
    stat: &crate::input::StaticDesc,
    ctx: &crate::progress::QuadCtx<'_>,
    tol_min: f32,
    tol_max: f32,
) -> Vec<AtlasTileCandidate> {
    let mut seen_diffuse: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_lod_materials: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut tiles: Vec<AtlasTileCandidate> = Vec::new();

    // --- Step 1: gather unique diffuse paths across all refs (levels 0-2) ---
    // port: LODApp.TexturesList (LODApp.cs:3507)
    for level in 0..3usize {
        let shapes = match crate::objects::parse_nif::parse_nif(stat, level, ctx) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for shape in &shapes {
            let in_tol = shape.geometry.uvcoords.iter().all(|uv| {
                uv[0] >= tol_min && uv[0] <= tol_max && uv[1] >= tol_min && uv[1] <= tol_max
            });
            if in_tol {
                for material in &shape.source_materials {
                    push_lod_material_diffuse(
                        ctx,
                        material,
                        &mut seen_lod_materials,
                        &mut seen_diffuse,
                        &mut tiles,
                    );
                }
            }
            let diffuse = &shape.textures[0];
            if diffuse.is_empty() || !diffuse_lower_contains(diffuse, ".dds") {
                continue;
            }
            // Normalise to the canonical `Textures\...` form for dedup and
            // resolution. This means `Data\LOD\foo.dds` and `lod\foo.dds`
            // both map to the same `textures\lod\foo.dds` key, preventing
            // duplicate tile loads and ensuring the resolved abs path agrees
            // with the atlas key that `data_relative_path` will produce.
            let norm = strip_normalize_texture_path(diffuse);
            let key = norm.to_lowercase();
            if !seen_diffuse.insert(key.clone()) {
                continue;
            }
            if !in_tol {
                tiles.push(AtlasTileCandidate { key, path: None });
                continue;
            }
            // Resolve the normalised path in the data dirs. `resolve_data_path_ci`
            // already applies `strip_normalize_texture_path` internally, but
            // passing the pre-normalised form is cheaper and avoids double work.
            let rel = norm.replace('/', "\\");
            let path = resolve_data_path_ci(ctx, &rel);
            tiles.push(AtlasTileCandidate { key, path });
        }
    }
    for material in stat.material_swap.values() {
        push_lod_material_diffuse(
            ctx,
            material,
            &mut seen_lod_materials,
            &mut seen_diffuse,
            &mut tiles,
        );
    }

    tiles
}

/// Normalise an object-texture path to the canonical Data-relative form used for
/// atlas key building and filesystem resolution.
///
/// Strips a leading `Data\` / `Data/` prefix (case-insensitive) then, if the
/// remaining path does not already start with `Textures\` / `textures/` (or any
/// other known sub-dir such as `Materials\`, `Meshes\`), prepends `Textures\`.
/// Slashes are normalised to backslash in the returned value.
///
/// This function is the single normalization point shared by the atlas tile
/// resolver (`resolve_data_path_ci`) and the atlas-key lookup (`atlas_get_key`)
/// so both sides agree on the canonical key form and a path stored as
/// `Data\LOD\foo_d.dds` in a NIF resolves the same as `Textures\LOD\foo_d.dds`.
pub fn strip_normalize_texture_path(s: &str) -> String {
    // Normalise slashes to backslash first so all subsequent tests use one form.
    let mut p = s.replace('/', "\\");

    // Strip leading `Data\` prefix (case-insensitive).
    let lower = p.to_lowercase();
    if let Some(rest) = lower.strip_prefix("data\\") {
        // Keep the original-case suffix after `data\`.
        let stripped_start = p.len() - rest.len();
        p = p[stripped_start..].to_string();
    }

    // If, after stripping, the path does not start with a known sub-directory
    // that the game would resolve from the `Data\` root, prepend `Textures\`.
    // Known roots that already include the folder prefix:
    //   textures\ materials\ meshes\ sounds\ interface\ strings\ shadersfx\ lodsettings\ vis\
    let lower2 = p.to_lowercase();
    let has_known_prefix = lower2.starts_with("textures\\")
        || lower2.starts_with("materials\\")
        || lower2.starts_with("meshes\\")
        || lower2.starts_with("sounds\\")
        || lower2.starts_with("interface\\")
        || lower2.starts_with("strings\\")
        || lower2.starts_with("shadersfx\\")
        || lower2.starts_with("lodsettings\\")
        || lower2.starts_with("vis\\");

    // Only prepend `Textures\` when the path has a sub-directory component
    // (contains a `\` separator), meaning it's a game-relative path missing its
    // root prefix — not a bare filename like `a_d.dds` used in synthetic tests.
    if !has_known_prefix && p.contains('\\') {
        p = format!("Textures\\{p}");
    }

    p
}

/// Resolve a Data-relative path (backslash-separated) against the QuadCtx data_dirs.
///
/// Applies `strip_normalize_texture_path` before the lookup so that paths stored
/// in LOD NIFs as `Data\Textures\LOD\...` or `Data\LOD\...` both resolve against
/// the extracted corpus in the same way as a clean `Textures\LOD\...` path.
fn resolve_data_path_ci(
    ctx: &crate::progress::QuadCtx<'_>,
    rel: &str,
) -> Option<std::path::PathBuf> {
    let normalized = strip_normalize_texture_path(rel).replace('\\', "/");
    for dir in &ctx.paths.data_dirs {
        let candidate = dir.join(&normalized);
        if candidate.is_file() {
            return Some(candidate);
        }
        // Case-insensitive walk.
        if let Some(found) = ci_resolve(dir, &normalized) {
            return Some(found);
        }
    }
    None
}

fn normalize_lod_material_path(s: &str) -> Option<String> {
    let mut p = s.trim_end_matches('\0').trim().replace('/', "\\");
    let lower = p.to_lowercase();
    if let Some(pos) = lower.rfind("data\\") {
        p = p[pos + 5..].to_string();
    }
    let p = p.to_lowercase();
    let lod = p.strip_prefix("materials\\").unwrap_or(&p);
    (lod.starts_with("lod\\") && lod.ends_with(".bgsm")).then(|| lod.to_string())
}

fn resolve_material_path_ci(
    ctx: &crate::progress::QuadCtx<'_>,
    rel: &str,
) -> Option<std::path::PathBuf> {
    let normalized = if rel.to_lowercase().starts_with("materials\\") {
        rel.replace('\\', "/")
    } else {
        format!("Materials\\{rel}").replace('\\', "/")
    };
    for dir in &ctx.paths.data_dirs {
        let candidate = dir.join(&normalized);
        if candidate.is_file() {
            return Some(candidate);
        }
        if let Some(found) = ci_resolve(dir, &normalized) {
            return Some(found);
        }
    }
    None
}

fn push_lod_material_diffuse(
    ctx: &crate::progress::QuadCtx<'_>,
    material: &str,
    seen_lod_materials: &mut std::collections::HashSet<String>,
    seen_diffuse: &mut std::collections::HashSet<String>,
    diffuse_paths: &mut Vec<AtlasTileCandidate>,
) {
    let Some(lod_material) = normalize_lod_material_path(material) else {
        return;
    };
    if !seen_lod_materials.insert(lod_material.clone()) {
        return;
    }
    let Some(material_path) = resolve_material_path_ci(ctx, &lod_material) else {
        return;
    };
    let Ok(bytes) = std::fs::read(material_path) else {
        return;
    };
    let Ok(bgsm) = materials_native::bgsm::parse(&bytes) else {
        return;
    };
    let diffuse = bgsm.DiffuseTexture.trim_end_matches('\0').trim();
    if diffuse.is_empty() {
        return;
    }
    let norm = strip_normalize_texture_path(diffuse);
    let key = norm.to_lowercase();
    if !seen_diffuse.insert(key.clone()) {
        return;
    }
    let rel = norm.replace('/', "\\");
    let path = resolve_data_path_ci(ctx, &rel);
    diffuse_paths.push(AtlasTileCandidate { key, path });
}

fn ci_resolve(base: &std::path::Path, rel: &str) -> Option<std::path::PathBuf> {
    let mut cur = base.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        let entries = std::fs::read_dir(&cur).ok()?;
        let mut matched: Option<std::path::PathBuf> = None;
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(part)
            {
                matched = Some(entry.path());
                break;
            }
        }
        cur = matched?;
    }
    if cur.is_file() { Some(cur) } else { None }
}

fn diffuse_lower_contains(s: &str, needle: &str) -> bool {
    s.to_lowercase().contains(needle)
}

fn format_to_str(f: &crate::settings::Format) -> &'static str {
    use crate::settings::Format;
    match f {
        Format::Bc1 => "BC1_UNORM",
        Format::Bc2 => "BC2_UNORM",
        Format::Bc3 => "BC3_UNORM",
        Format::Bc5 => "BC5_UNORM",
        Format::Bc7 => "BC7_UNORM",
        Format::Rgba8 => "R8G8B8A8_UNORM",
        Format::Bgr565 => "BC1_UNORM",
    }
}

/// Load diffuse tiles from disk, pack via BinPacker, blit an RGBA atlas,
/// write BCn DDS + atlas-map .txt, derive _n/_s siblings, and load back into
/// AtlasList.
///
/// Exposed as `pub` for unit testing without requiring live refs or a QuadCtx.
///
/// Port: wbBuildAtlasFromTexturesList (wbLOD.pas:1402-1581).
pub fn build_atlas_from_tiles(
    diffuse_tile_paths: &[std::path::PathBuf],
    atlas_diffuse_path: &std::path::Path,
    atlas_map_path: &std::path::Path,
    atlas_size: u32,
    max_tile_size: u32,
    format_diffuse: &str,
    format_normal: &str,
    format_specular: &str,
) -> anyhow::Result<AtlasResult> {
    build_atlas_from_tiles_with_progress(
        diffuse_tile_paths,
        atlas_diffuse_path,
        atlas_map_path,
        atlas_size,
        max_tile_size,
        format_diffuse,
        format_normal,
        format_specular,
        false,
        None,
    )
}

fn build_atlas_from_tiles_with_progress(
    diffuse_tile_paths: &[std::path::PathBuf],
    atlas_diffuse_path: &std::path::Path,
    atlas_map_path: &std::path::Path,
    atlas_size: u32,
    max_tile_size: u32,
    format_diffuse: &str,
    format_normal: &str,
    format_specular: &str,
    mip_flooding: bool,
    mut progress: Option<&mut dyn crate::progress::Progress>,
) -> anyhow::Result<AtlasResult> {
    use super::binpacker::{BinBlock, BinPacker};
    use directxtex_native::read_dds_rgba_image;

    // Load + optionally resize each diffuse tile.
    // port: wbLOD.pas:1440-1480
    struct Tile {
        /// Data-relative diffuse path (backslash), stored in the atlas-map.
        diffuse_rel: String,
        /// Data-relative normal path (backslash) — empty if no `_n` sibling on disk.
        /// Used to build the `diffuse,normal` AtlasList key (port: AtlasList.GetKey).
        normal_rel: String,
        w: u32,
        h: u32,
        rgba_d: Vec<u8>,
        rgba_n: Vec<u8>,
        rgba_s: Vec<u8>,
    }

    let mut tiles: Vec<Tile> = Vec::new();

    let total_tiles = diffuse_tile_paths.len().max(1);
    let report_every = (total_tiles / 20).max(1);
    report_progress(
        &mut progress,
        &format!(
            "object atlas: loading {} texture tiles",
            diffuse_tile_paths.len()
        ),
        0.45,
    );

    for (idx, abs_path) in diffuse_tile_paths.iter().enumerate() {
        // Skip tiles too large for the atlas (port: wbLOD.pas:1453)
        let img = match read_dds_rgba_image(abs_path) {
            Ok(i) => i,
            Err(_) => continue,
        };
        if img.width > atlas_size || img.height > atlas_size {
            continue;
        }

        // Resize if > max_tile_size (port: wbLOD.pas:1456-1469)
        let (w, h, rgba_d) = if img.width > max_tile_size || img.height > max_tile_size {
            let new_w = img.width.min(max_tile_size);
            let new_h = img.height.min(max_tile_size);
            let resized = resize_rgba(&img.rgba, img.width, img.height, new_w, new_h);
            (new_w, new_h, resized)
        } else {
            (img.width, img.height, img.rgba)
        };

        // Derive _n and _s sibling paths.
        // port: wbLOD.pas:1423-1435
        let n_path = derive_sibling_path(abs_path, "_n.dds");
        let s_path = derive_sibling_path(abs_path, "_s.dds");

        let n_on_disk = n_path.as_ref().filter(|p| p.is_file());
        let rgba_n = if let Some(p) = n_on_disk {
            read_dds_rgba_image(p)
                .map(|i| maybe_resize_rgba(i, w, h))
                .unwrap_or_else(|_| flat_normal_rgba(w, h))
        } else {
            flat_normal_rgba(w, h)
        };
        let rgba_s = if let Some(p) = s_path.as_ref().filter(|p| p.is_file()) {
            read_dds_rgba_image(p)
                .map(|i| maybe_resize_rgba(i, w, h))
                .unwrap_or_else(|_| white_rgba(w, h))
        } else {
            white_rgba(w, h)
        };

        // Data-relative path for the atlas-map: take the part after "textures\" (case-insensitive).
        let rel = data_relative_path(abs_path);
        // Data-relative normal path (only when an `_n` sibling exists on disk), used
        // for the `diffuse,normal` AtlasList key. port: AtlasList.GetKey / atlas-map
        // source column = `diffuse,normal` (Program.cs:1306-1311).
        let normal_rel = n_on_disk.map(|p| data_relative_path(p)).unwrap_or_default();

        tiles.push(Tile {
            diffuse_rel: rel,
            normal_rel,
            w,
            h,
            rgba_d,
            rgba_n,
            rgba_s,
        });

        let done = idx + 1;
        if done == diffuse_tile_paths.len() || done % report_every == 0 {
            report_progress(
                &mut progress,
                &format!(
                    "object atlas: loaded {done}/{} texture tiles, {} usable",
                    diffuse_tile_paths.len(),
                    tiles.len()
                ),
                0.45 + (done as f32 / total_tiles as f32) * 0.35,
            );
        }
    }

    if tiles.is_empty() {
        report_progress(
            &mut progress,
            "object atlas: no loadable texture tiles",
            1.0,
        );
        return Ok(AtlasResult {
            map_path: atlas_map_path.to_path_buf(),
            diffuse: atlas_diffuse_path.to_path_buf(),
            normal: atlas_diffuse_path
                .with_extension("")
                .with_file_name(format!(
                    "{}_n.dds",
                    atlas_diffuse_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("atlas")
                )),
            specular: atlas_diffuse_path
                .with_extension("")
                .with_file_name(format!(
                    "{}_s.dds",
                    atlas_diffuse_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("atlas")
                )),
            atlas_size: (0, 0),
            uv: std::collections::HashMap::new(),
            list: AtlasList::new(),
            dds_written: 0,
        });
    }

    // --- Pack tiles via BinPacker ---
    // port: wbLOD.pas:1482-1521
    report_progress(
        &mut progress,
        &format!("object atlas: packing {} usable texture tiles", tiles.len()),
        0.82,
    );
    let mut blocks: Vec<BinBlock> = tiles
        .iter()
        .enumerate()
        .map(|(i, t)| BinBlock {
            index: i,
            w: t.w,
            h: t.h,
            x: 0,
            y: 0,
            fit: false,
        })
        .collect();

    // Reduce tiles until everything fits (drop smallest from the end after sort).
    // port: wbLOD.pas:1505-1521 — at least 2 tiles remain.
    let packer = BinPacker::new(atlas_size, atlas_size);
    while blocks.len() >= 2 && !packer.fit(&mut blocks) {
        // Drop the last (smallest after sort)
        let dropped_idx = blocks.pop().unwrap().index;
        tiles.remove(dropped_idx);
        // Rebuild blocks to match updated tiles vec
        blocks = tiles
            .iter()
            .enumerate()
            .map(|(i, t)| BinBlock {
                index: i,
                w: t.w,
                h: t.h,
                x: 0,
                y: 0,
                fit: false,
            })
            .collect();
    }

    if !packer.fit(&mut blocks) {
        // Even with 1 tile it didn't fit — use the one tile anyway at (0,0)
        for b in &mut blocks {
            b.x = 0;
            b.y = 0;
            b.fit = true;
        }
    }

    // Compute atlas dimensions: next power of 2 >= (max_x+w, max_y+h).
    // port: wbLOD.pas:1524-1536
    let (max_w, max_h) = blocks.iter().fold((0u32, 0u32), |(mx, my), b| {
        if b.fit {
            (mx.max(b.x + b.w), my.max(b.y + b.h))
        } else {
            (mx, my)
        }
    });
    let atlas_w = next_pow2(max_w.max(1));
    let atlas_h = next_pow2(max_h.max(1));

    // --- Blit tiles into RGBA buffers ---
    // port: wbLOD.pas:1538-1556
    let buf_size = (atlas_w * atlas_h * 4) as usize;
    let mut buf_d = vec![0u8; buf_size];
    let mut buf_n = vec![128u8; buf_size]; // flat normal default
    let mut buf_s = vec![255u8; buf_size]; // white specular default

    let mut map_rows: Vec<AtlasMapRow> = Vec::new();
    let mut uv_map: std::collections::HashMap<String, AtlasRect> = std::collections::HashMap::new();
    let mut atlas_list = AtlasList::new();

    // Derive atlas DDS data-relative path for the map rows.
    let atlas_rel = data_relative_path(atlas_diffuse_path);
    report_progress(
        &mut progress,
        &format!("object atlas: composing {atlas_w}x{atlas_h} atlas"),
        0.9,
    );

    for b in &blocks {
        if !b.fit {
            continue;
        }
        let tile = &tiles[b.index];
        blit_rgba(&tile.rgba_d, tile.w, tile.h, &mut buf_d, atlas_w, b.x, b.y);
        blit_rgba(&tile.rgba_n, tile.w, tile.h, &mut buf_n, atlas_w, b.x, b.y);
        blit_rgba(&tile.rgba_s, tile.w, tile.h, &mut buf_s, atlas_w, b.x, b.y);

        // Atlas key + atlas-map source column = "diffuse,normal" (lowercased) when
        // a normal sibling exists, else bare "diffuse" — port: AtlasList.GetKey /
        // the atlas-map loader's column-0 split (Program.cs:1295-1322). Keying by
        // "diffuse,normal" lets transform_shape's atlas_build_key -> atlas_get_key
        // resolve via the diffuse,normal branch instead of the bare-diffuse fallback.
        // (Glow is omitted: no glow atlas is built.)
        let source = if tile.normal_rel.is_empty() {
            tile.diffuse_rel.clone()
        } else {
            format!("{},{}", tile.diffuse_rel, tile.normal_rel)
        };
        let row = AtlasMapRow {
            source: source.clone(),
            tile_w: tile.w,
            tile_h: tile.h,
            x: b.x,
            y: b.y,
            atlas: atlas_rel.clone(),
            atlas_w,
            atlas_h,
        };
        let rect = AtlasRect::from_map_row(
            tile.w, tile.h, b.x, b.y, atlas_w, atlas_h, &atlas_rel, false,
        );
        // key = source column lowercased (matches AtlasList's case-insensitive keys).
        let key = source.to_lowercase();
        uv_map.insert(key.clone(), rect.clone());
        atlas_list.insert(key, rect);
        map_rows.push(row);
    }

    // --- Write DDS files ---
    // port: wbLOD.pas:1557-1566
    //
    // DDS encoding is best-effort: the atlas UV `list`/`uv`/`atlas_size` (which is
    // what `transform_shape` consumes to remap object-LOD UVs) is already fully
    // computed above. If the DDS encoder is unavailable (e.g. the real-esp test
    // link resolves the external DirectXTex FFI copy, whose BC `Compress` returns
    // E_NOTIMPL — the documented `/FORCE:MULTIPLE` collision), we log and keep the
    // valid AtlasResult rather than failing the whole object-LOD pass. The umbrella
    // `_native.pyd` links our directxtex_native and writes the atlas correctly.
    if let Some(parent) = atlas_diffuse_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let atlas_normal_path = sibling_dds(atlas_diffuse_path, "_n");
    let atlas_specular_path = sibling_dds(atlas_diffuse_path, "_s");
    let mut dds_written: u32 = 0;
    for (idx, (path, buf, fmt)) in [
        (atlas_diffuse_path, &buf_d, format_diffuse),
        (atlas_normal_path.as_path(), &buf_n, format_normal),
        (atlas_specular_path.as_path(), &buf_s, format_specular),
    ]
    .into_iter()
    .enumerate()
    {
        report_progress(
            &mut progress,
            &format!("object atlas: writing {}", path.display()),
            0.92 + (idx as f32 * 0.02),
        );
        if let Err(e) = write_atlas_dds(path, atlas_w, atlas_h, buf, fmt, idx == 0 && mip_flooding)
        {
            eprintln!("[lodgen atlas] DDS write skipped ({}): {e}", path.display());
        } else {
            dds_written += 1;
        }
    }

    // --- Write atlas-map .txt ---
    write_atlas_map(atlas_map_path, &map_rows)?;
    report_progress(
        &mut progress,
        &format!(
            "object atlas: done {} tiles {atlas_w}x{atlas_h} dds={dds_written}/3",
            atlas_list.len()
        ),
        1.0,
    );

    Ok(AtlasResult {
        map_path: atlas_map_path.to_path_buf(),
        diffuse: atlas_diffuse_path.to_path_buf(),
        normal: atlas_normal_path,
        specular: atlas_specular_path,
        atlas_size: (atlas_w, atlas_h),
        uv: uv_map,
        list: atlas_list,
        dds_written,
    })
}

// ---------------------------------------------------------------------------
// pack_and_compose_atlas — test-friendly lower-level helper
// ---------------------------------------------------------------------------

/// Pack pre-loaded RGBA tiles into a minimal RGBA atlas buffer.
/// Returns `(atlas_w, atlas_h, rgba_buffer)`.
///
/// Exposed as `pub` for unit testing.
pub fn pack_and_compose_atlas(
    tiles: &[(u32, u32, Vec<u8>)],
    atlas_size: u32,
) -> (u32, u32, Vec<u8>) {
    use super::binpacker::{BinBlock, BinPacker};

    if tiles.is_empty() {
        return (1, 1, vec![0u8; 4]);
    }

    let mut blocks: Vec<BinBlock> = tiles
        .iter()
        .enumerate()
        .map(|(i, (w, h, _))| BinBlock {
            index: i,
            w: *w,
            h: *h,
            x: 0,
            y: 0,
            fit: false,
        })
        .collect();

    let packer = BinPacker::new(atlas_size, atlas_size);
    let _ = packer.fit(&mut blocks);

    let (max_w, max_h) = blocks.iter().fold((0u32, 0u32), |(mx, my), b| {
        if b.fit {
            (mx.max(b.x + b.w), my.max(b.y + b.h))
        } else {
            (mx, my)
        }
    });
    let atlas_w = next_pow2(max_w.max(1));
    let atlas_h = next_pow2(max_h.max(1));

    let buf_size = (atlas_w * atlas_h * 4) as usize;
    let mut buf = vec![0u8; buf_size];

    for b in &blocks {
        if !b.fit {
            continue;
        }
        let (tw, th, ref rgba) = tiles[b.index];
        blit_rgba(rgba, tw, th, &mut buf, atlas_w, b.x, b.y);
    }

    (atlas_w, atlas_h, buf)
}

// ---------------------------------------------------------------------------
// Local helpers
// ---------------------------------------------------------------------------

/// Resize an RGBA8 tile to (dw, dh) using directxtex's high-quality resampler.
///
/// Port intent (#8): route tile resize through directxtex instead of a hand-rolled
/// nearest-neighbor box filter. DEVIATION: the contract names "Lanczos", but
/// DirectXTex exposes no LANCZOS filter — its highest-quality kernel is CUBIC
/// (TEX_FILTER_CUBIC), which we use here. Falls back to nearest-neighbor only if
/// directxtex fails (e.g. zero dimensions), so callers always get a buffer.
pub(crate) fn resize_rgba(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    match directxtex_resize_rgba(src, sw, sh, dw, dh) {
        Some(out) => out,
        None => nearest_resize_rgba(src, sw, sh, dw, dh),
    }
}

/// directxtex-backed RGBA8 resize (TEX_FILTER_CUBIC). Returns None on any failure.
/// Uses the supported ScratchImage init + resize path (the same pattern as
/// directxtex's own write_dds_bytes), not a hand-built Image pointer.
fn directxtex_resize_rgba(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Option<Vec<u8>> {
    use directxtex_native::{CP_FLAGS, DXGI_FORMAT, ScratchImage, TEX_FILTER_FLAGS};
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return None;
    }
    let expected = (sw as usize) * (sh as usize) * 4;
    if src.len() < expected {
        return None;
    }
    let mut scratch = ScratchImage::default();
    if let Err(e) = scratch.initialize_2d(
        DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
        sw as usize,
        sh as usize,
        1,
        1,
        CP_FLAGS::CP_FLAGS_NONE,
    ) {
        eprintln!("[lodgen resize] initialize_2d failed: {e:?}");
        return None;
    }
    // Copy the source RGBA into the scratch image (handle row_pitch >= sw*4).
    let src_pitch = (sw as usize) * 4;
    let img_pitch = scratch.image(0, 0, 0)?.row_pitch;
    {
        let dst = scratch.pixels_mut();
        for row in 0..(sh as usize) {
            let s = row * src_pitch;
            let d = row * img_pitch;
            if d + src_pitch <= dst.len() && s + src_pitch <= src.len() {
                dst[d..d + src_pitch].copy_from_slice(&src[s..s + src_pitch]);
            }
        }
    }

    // CUBIC via DirectXTex's CUSTOM (non-WIC) resampler. FORCE_NON_WIC avoids the
    // WIC/COM path (which returns E_NOINTERFACE without CoInitialize and is
    // unavailable off the main STA thread) — the custom path supports point/linear/
    // cubic/triangle and is deterministic.
    let filter =
        TEX_FILTER_FLAGS::TEX_FILTER_CUBIC.union(TEX_FILTER_FLAGS::TEX_FILTER_FORCE_NON_WIC);
    let resized = match scratch.resize(dw as usize, dh as usize, filter) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[lodgen resize] scratch.resize failed: {e:?}");
            return None;
        }
    };
    let out_img = resized.image(0, 0, 0)?;
    let mut out = vec![0u8; (dw as usize) * (dh as usize) * 4];
    let dst_pitch = (dw as usize) * 4;
    let pixels = resized.pixels();
    for row in 0..(dh as usize) {
        let s = row * out_img.row_pitch;
        let d = row * dst_pitch;
        if s + dst_pitch <= pixels.len() {
            out[d..d + dst_pitch].copy_from_slice(&pixels[s..s + dst_pitch]);
        }
    }
    Some(out)
}

/// Nearest-neighbor RGBA resize — fallback only (directxtex failure path).
fn nearest_resize_rgba(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dw * dh * 4) as usize];
    let scale_x = sw as f32 / dw as f32;
    let scale_y = sh as f32 / dh as f32;
    for dy in 0..dh {
        for dx in 0..dw {
            let sx = (dx as f32 * scale_x) as u32;
            let sy = (dy as f32 * scale_y) as u32;
            let si = ((sy * sw + sx) * 4) as usize;
            let di = ((dy * dw + dx) * 4) as usize;
            if si + 3 < src.len() {
                dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
    dst
}

/// Resize a DdsRgbaImage to (dw, dh) if it doesn't already match.
pub(crate) fn maybe_resize_rgba(img: directxtex_native::DdsRgbaImage, dw: u32, dh: u32) -> Vec<u8> {
    if img.width == dw && img.height == dh {
        img.rgba
    } else {
        resize_rgba(&img.rgba, img.width, img.height, dw, dh)
    }
}

/// Flat normal map (128, 128, 255, 255) for missing _n siblings.
pub(crate) fn flat_normal_rgba(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        v.extend_from_slice(&[128, 128, 255, 255]);
    }
    v
}

/// White specular (255, 255, 255, 255) for missing _s siblings.
pub(crate) fn white_rgba(w: u32, h: u32) -> Vec<u8> {
    vec![255u8; (w * h * 4) as usize]
}

/// Blit src (w×h RGBA) into dst at (ox, oy) in an atlas of width `atlas_w`.
pub(crate) fn blit_rgba(
    src: &[u8],
    sw: u32,
    sh: u32,
    dst: &mut [u8],
    atlas_w: u32,
    ox: u32,
    oy: u32,
) {
    for row in 0..sh {
        let src_off = (row * sw * 4) as usize;
        let dst_off = ((oy + row) * atlas_w * 4 + ox * 4) as usize;
        let len = (sw * 4) as usize;
        if src_off + len <= src.len() && dst_off + len <= dst.len() {
            dst[dst_off..dst_off + len].copy_from_slice(&src[src_off..src_off + len]);
        }
    }
}

pub(crate) fn infinite_dilate_transparent_rgb(rgba: &mut [u8], width: u32, height: u32) -> bool {
    let pixel_count = (width as usize).saturating_mul(height as usize);
    if width == 0
        || height == 0
        || pixel_count > u32::MAX as usize
        || rgba.len() != pixel_count.saturating_mul(4)
    {
        return false;
    }

    let has_transparent = (0..pixel_count).any(|i| rgba[i * 4 + 3] == 0);
    let has_visible = (0..pixel_count).any(|i| rgba[i * 4 + 3] != 0);
    if !has_transparent || !has_visible {
        return false;
    }

    let mut queued_seed = vec![false; pixel_count];
    let mut queue = Vec::<u32>::new();
    for index in 0..pixel_count {
        if rgba[index * 4 + 3] != 0 {
            continue;
        }
        let x = index as u32 % width;
        let y = index as u32 / width;
        for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
            for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                let neighbor = (ny * width + nx) as usize;
                if rgba[neighbor * 4 + 3] != 0 && !queued_seed[neighbor] {
                    queued_seed[neighbor] = true;
                    queue.push(neighbor as u32);
                }
            }
        }
    }
    drop(queued_seed);

    let seed_count = queue.len();
    let mut head = 0;
    while head < queue.len() {
        let index = queue[head] as usize;
        head += 1;
        let source = [rgba[index * 4], rgba[index * 4 + 1], rgba[index * 4 + 2]];
        let x = index as u32 % width;
        let y = index as u32 / width;
        for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
            for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                let neighbor = (ny * width + nx) as usize;
                let offset = neighbor * 4;
                if rgba[offset + 3] == 0 {
                    rgba[offset..offset + 3].copy_from_slice(&source);
                    rgba[offset + 3] = 1;
                    queue.push(neighbor as u32);
                }
            }
        }
    }

    for &index in &queue[seed_count..] {
        rgba[index as usize * 4 + 3] = 0;
    }
    queue.len() > seed_count
}

pub(crate) fn write_atlas_dds(
    path: &std::path::Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    format: &str,
    mip_flooding: bool,
) -> Result<(), String> {
    if !mip_flooding {
        return directxtex_native::write_dds_rgba_image(path, width, height, rgba, format, true);
    }

    match directxtex_native::write_dds_rgba_image_mip_flooded(path, width, height, rgba, format) {
        Ok(()) => Ok(()),
        Err(mip_flood_error) => {
            let mut fallback = rgba.to_vec();
            infinite_dilate_transparent_rgb(&mut fallback, width, height);
            directxtex_native::write_dds_rgba_image(
                path, width, height, &fallback, format, true,
            )
            .map_err(|fallback_error| {
                format!(
                    "native mip flooding failed: {mip_flood_error}; infinite dilation fallback failed: {fallback_error}"
                )
            })
        }
    }
}

/// Next power of 2 >= n.
pub(crate) fn next_pow2(n: u32) -> u32 {
    if n == 0 {
        return 1;
    }
    let mut p = 1u32;
    while p < n {
        p <<= 1;
    }
    p
}

/// Derive a sibling DDS path by replacing the final `_d.dds` or `.dds` with `suffix`.
/// For _n: `foo_d.dds` → `foo_n.dds`; for _s: `foo_d.dds` → `foo_s.dds`.
fn derive_sibling_path(diffuse: &std::path::Path, suffix: &str) -> Option<std::path::PathBuf> {
    let name = diffuse.file_name()?.to_string_lossy().to_lowercase();
    let new_name = if name.ends_with("_d.dds") {
        format!("{}{}", &name[..name.len() - 6], suffix)
    } else if name.ends_with(".dds") {
        format!("{}{}", &name[..name.len() - 4], suffix)
    } else {
        return None;
    };
    Some(diffuse.parent()?.join(new_name))
}

/// Build a sibling atlas path by inserting a suffix before `.dds`.
pub(crate) fn sibling_dds(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("atlas");
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    parent.join(format!("{stem}{suffix}.dds"))
}

/// Extract the Data-relative path: the portion starting at (and including) `textures\`
/// in the absolute path, or the filename if `textures\` is not found.
/// Stores with backslashes for compatibility with the atlas-map format.
pub(crate) fn data_relative_path(abs: &std::path::Path) -> String {
    let s = abs.to_string_lossy().replace('/', "\\");
    let lower = s.to_lowercase();
    if let Some(pos) = lower.find("textures\\") {
        s[pos..].to_string()
    } else {
        abs.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.dds")
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_rgba_uses_directxtex() {
        // Fix #8: tile resize must route through directxtex, not the hand-rolled
        // nearest-neighbor. The directxtex path must SUCCEED (return Some) — that
        // is the load-bearing assertion of "route through directxtex".
        // Build a 4x4 horizontal black->white gradient and downscale to 2x2.
        let mut src = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let i = (y * 4 + x) * 4;
                let v = (x * 85) as u8; // 0,85,170,255 across width
                src[i] = v;
                src[i + 1] = v;
                src[i + 2] = v;
                src[i + 3] = 255;
            }
        }
        // The directxtex-backed path must succeed (not fall back).
        let dx = directxtex_resize_rgba(&src, 4, 4, 2, 2)
            .expect("directxtex_resize_rgba must succeed — #8 routes resize through directxtex");
        assert_eq!(dx.len(), 2 * 2 * 4, "2x2 RGBA output");
        // And the public wrapper returns the directxtex result.
        let out = resize_rgba(&src, 4, 4, 2, 2);
        assert_eq!(out, dx, "resize_rgba must return the directxtex result");
    }

    #[test]
    fn resize_rgba_identity_when_same_size() {
        let src = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let out = resize_rgba(&src, 2, 1, 2, 1);
        assert_eq!(out, src, "same-size resize is a no-op copy");
    }

    #[test]
    fn infinite_dilation_fills_rgb_and_preserves_alpha() {
        let mut rgba = vec![0u8; 5 * 5 * 4];
        let center = (2 * 5 + 2) * 4;
        rgba[center..center + 4].copy_from_slice(&[40, 80, 120, 128]);

        assert!(infinite_dilate_transparent_rgb(&mut rgba, 5, 5));
        assert!(
            rgba.chunks_exact(4)
                .all(|pixel| pixel[..3] == [40, 80, 120])
        );
        assert_eq!(rgba[center + 3], 128);
        assert_eq!(rgba[3], 0);
        assert_eq!(rgba[(5 * 5 - 1) * 4 + 3], 0);
    }

    #[test]
    fn infinite_dilation_skips_fully_opaque_and_fully_transparent_images() {
        let mut opaque = vec![255u8; 2 * 2 * 4];
        let opaque_before = opaque.clone();
        assert!(!infinite_dilate_transparent_rgb(&mut opaque, 2, 2));
        assert_eq!(opaque, opaque_before);

        let mut transparent = vec![0u8; 2 * 2 * 4];
        assert!(!infinite_dilate_transparent_rgb(&mut transparent, 2, 2));
        assert_eq!(transparent, vec![0u8; 2 * 2 * 4]);
    }

    #[test]
    fn atlas_dds_writer_uses_native_mip_flooding_for_diffuse() {
        let path = std::env::temp_dir().join(format!(
            "modbox21_lodgen_mip_flood_{}.dds",
            std::process::id()
        ));
        let mut rgba = vec![0u8; 4 * 4 * 4];
        rgba[..4].copy_from_slice(&[180, 30, 90, 255]);

        write_atlas_dds(&path, 4, 4, &rgba, "R8G8B8A8_UNORM", true).unwrap();
        let decoded = directxtex_native::read_dds_mips_rgba8(&path).unwrap();

        assert_eq!(decoded.mips.len(), 3);
        assert_eq!(&decoded.mips[0].2[4..7], &[180, 30, 90]);
        assert_eq!(decoded.mips[0].2[7], 0);
        std::fs::remove_file(path).ok();
    }
}
