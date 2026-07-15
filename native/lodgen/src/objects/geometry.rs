/// In-memory LOD mesh container and dedup/optimize operations.
///
/// Port: Geometry.cs (LODGenerator/Geometry.cs).
use crate::descriptors::BBox;

/// In-memory LOD geometry — port: Geometry.cs.
#[derive(Clone)]
pub struct LodGeometry {
    pub vertices: Vec<[f32; 3]>,
    pub uvcoords: Vec<[f32; 2]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
    pub bitangents: Vec<[f32; 3]>,
    pub vertex_colors: Vec<[f32; 4]>,
    pub triangles: Vec<[u32; 3]>,
    pub bbox: BBox,
}

impl LodGeometry {
    pub fn new() -> Self {
        LodGeometry {
            vertices: Vec::new(),
            uvcoords: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            bitangents: Vec::new(),
            vertex_colors: Vec::new(),
            triangles: Vec::new(),
            bbox: BBox::empty(),
        }
    }

    pub fn num_vertices(&self) -> usize {
        self.vertices.len()
    }

    pub fn num_triangles(&self) -> usize {
        self.triangles.len()
    }

    pub fn has_vertex_colors(&self) -> bool {
        !self.vertex_colors.is_empty()
    }

    pub fn has_normals(&self) -> bool {
        !self.normals.is_empty()
    }

    pub fn has_tangents(&self) -> bool {
        !self.tangents.is_empty()
    }

    pub fn update_bbox(&mut self) {
        self.bbox = BBox::empty();
        for &v in &self.vertices {
            self.bbox.grow_vertex(v);
        }
    }

    pub fn set_uvcoords(&mut self, uvs: Vec<[f32; 2]>) {
        // port: Geometry.cs:1072
        self.uvcoords = uvs;
    }

    /// Remove duplicate vertices within threshold.
    /// port: Geometry.cs:705-818 RemoveDuplicate
    ///
    /// Thresholds: pos=0.5, normal=0.1, color=0.15, uv=0.005 (or 0.001 if high).
    /// For each vertex i, scans j<i; first match maps i→j. Remaps triangle indices
    /// then calls remove_unused to compact geometry.
    pub fn remove_duplicate(&mut self, high: bool) {
        let pos_t: f32 = 0.5;
        let norm_t: f32 = 0.1;
        let color_t: f32 = 0.15;
        let uv_t: f32 = if high { 0.001 } else { 0.005 };

        let has_tris = !self.triangles.is_empty();
        let has_normals = !self.normals.is_empty();
        let has_tangents = !self.tangents.is_empty();
        let has_uv = !self.uvcoords.is_empty();
        let has_colors = !self.vertex_colors.is_empty();

        // dup[i] = j means vertex i should be remapped to j (j < i)
        let mut dup: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();

        for i in 0..self.vertices.len() {
            let vi = self.vertices[i];
            for j in 0..i {
                // position check
                if (vi[0] - self.vertices[j][0]).abs() > pos_t
                    || (vi[1] - self.vertices[j][1]).abs() > pos_t
                    || (vi[2] - self.vertices[j][2]).abs() > pos_t
                {
                    continue;
                }
                // normal check
                if has_normals {
                    let ni = self.normals[i];
                    let nj = self.normals[j];
                    if (ni[0] - nj[0]).abs() > norm_t
                        || (ni[1] - nj[1]).abs() > norm_t
                        || (ni[2] - nj[2]).abs() > norm_t
                    {
                        continue;
                    }
                }
                // uv check
                if has_uv {
                    let ui = self.uvcoords[i];
                    let uj = self.uvcoords[j];
                    if (ui[0] - uj[0]).abs() > uv_t || (ui[1] - uj[1]).abs() > uv_t {
                        continue;
                    }
                }
                // color check
                if has_colors {
                    let ci = self.vertex_colors[i];
                    let cj = self.vertex_colors[j];
                    if (ci[0] - cj[0]).abs() > color_t
                        || (ci[1] - cj[1]).abs() > color_t
                        || (ci[2] - cj[2]).abs() > color_t
                    {
                        continue;
                    }
                }
                dup.insert(i, j);
                break;
            }
        }

        if dup.is_empty() {
            return;
        }

        if has_tris {
            // Remap triangle indices then compact via remove_unused
            for tri in &mut self.triangles {
                for idx in tri.iter_mut() {
                    if let Some(&canonical) = dup.get(&(*idx as usize)) {
                        *idx = canonical as u32;
                    }
                }
            }
            self.remove_unused();
        } else {
            // No triangles: compact the vertex arrays directly (port: Geometry.cs:779-816)
            let n = self.vertices.len();
            let mut new_vertices = Vec::with_capacity(n);
            let mut new_normals = Vec::with_capacity(if has_normals { n } else { 0 });
            let mut new_tangents = Vec::with_capacity(if has_tangents { n } else { 0 });
            let mut new_bitangents = Vec::with_capacity(if has_tangents { n } else { 0 });
            let mut new_uv = Vec::with_capacity(if has_uv { n } else { 0 });
            let mut new_colors = Vec::with_capacity(if has_colors { n } else { 0 });
            for m in 0..n {
                if dup.contains_key(&m) {
                    continue;
                }
                new_vertices.push(self.vertices[m]);
                if has_normals {
                    new_normals.push(self.normals[m]);
                    if has_tangents {
                        new_tangents.push(self.tangents[m]);
                        new_bitangents.push(self.bitangents[m]);
                    }
                }
                if has_uv {
                    new_uv.push(self.uvcoords[m]);
                }
                if has_colors {
                    new_colors.push(self.vertex_colors[m]);
                }
            }
            self.vertices = new_vertices;
            self.normals = new_normals;
            self.tangents = new_tangents;
            self.bitangents = new_bitangents;
            self.uvcoords = new_uv;
            self.vertex_colors = new_colors;
        }
    }

    /// Drop vertices not referenced by any triangle, compacting all arrays.
    /// port: Geometry.cs:892-955 RemoveUnused
    pub fn remove_unused(&mut self) {
        if self.triangles.is_empty() {
            return;
        }
        let has_uv = !self.uvcoords.is_empty();
        let has_colors = !self.vertex_colors.is_empty();
        let has_normals = !self.normals.is_empty();
        let has_tangents = !self.tangents.is_empty();

        // old_index → new_index mapping; build in triangle traversal order
        let mut old_to_new: std::collections::HashMap<usize, usize> =
            std::collections::HashMap::new();
        let mut counter = 0usize;

        let mut new_vertices: Vec<[f32; 3]> = Vec::new();
        let mut new_uv: Vec<[f32; 2]> = Vec::new();
        let mut new_colors: Vec<[f32; 4]> = Vec::new();
        let mut new_normals: Vec<[f32; 3]> = Vec::new();
        let mut new_tangents: Vec<[f32; 3]> = Vec::new();
        let mut new_bitangents: Vec<[f32; 3]> = Vec::new();
        let mut new_bbox = BBox::empty();

        for tri in &self.triangles {
            for &raw_idx in tri {
                let old = raw_idx as usize;
                if !old_to_new.contains_key(&old) {
                    old_to_new.insert(old, counter);
                    counter += 1;
                    if !self.vertices.is_empty() {
                        let v = self.vertices[old];
                        new_vertices.push(v);
                        new_bbox.grow_vertex(v);
                    }
                    if has_uv {
                        new_uv.push(self.uvcoords[old]);
                    }
                    if has_colors {
                        new_colors.push(self.vertex_colors[old]);
                    }
                    if has_normals {
                        new_normals.push(self.normals[old]);
                    }
                    if has_tangents {
                        new_tangents.push(self.tangents[old]);
                        new_bitangents.push(self.bitangents[old]);
                    }
                }
            }
        }

        // Remap triangles
        for tri in &mut self.triangles {
            for idx in tri.iter_mut() {
                *idx = old_to_new[&(*idx as usize)] as u32;
            }
        }

        self.vertices = new_vertices;
        self.uvcoords = new_uv;
        self.vertex_colors = new_colors;
        self.normals = new_normals;
        self.tangents = new_tangents;
        self.bitangents = new_bitangents;
        self.bbox = new_bbox;
    }

    /// Drop unused vertices without triangle data (no-op if triangles present — use remove_unused).
    /// port: Geometry.Optimize — drops verts unreferenced by any triangle.
    pub fn optimize(&mut self) {
        self.remove_unused();
    }

    pub fn qem_decimate(
        &mut self,
        target_triangles: usize,
        options: crate::objects::qem::QemOptions,
    ) -> bool {
        crate::objects::qem::decimate(self, target_triangles, options)
    }

    // -----------------------------------------------------------------------
    // ReUV / loose re-weld / Simplify — the atlassed-object decimation path.
    //
    // Port: Geometry.ReUV (Geometry.cs:1122) common (non-UV-tile-split) branch
    // + Geometry.Simplify (Geometry.cs:1999) + Geometry.CreateTriangles
    // (Geometry.cs:1526). Drives the heavy-tail collapse for atlassed object LOD.
    //
    // DEFERRED (noted): the UV-tile-boundary split sub-loop (Geometry.cs:1224-1402
    // — SplitUV / PointInTriangle / CreateDuplicate / the `flag` recompute of
    // FaceNormals+SmoothNormals+UpdateTangents) is NOT ported. It only fires when
    // a triangle's UVs span more than one [0,1] tile (Ceiling-Floor > 1), which
    // does not happen for atlassed LOD shapes whose UVs are remapped into a single
    // atlas sub-rect inside [0,1]. Porting it would not change the tri-ratio
    // convergence the heavy tail needs.
    // -----------------------------------------------------------------------

    /// Per-triangle UV break + loose re-weld + Simplify, applied in place.
    ///
    /// Port: the common path of `Geometry.ReUV` (Geometry.cs:1122):
    ///   1. QUVx-quantize UVs + NaN/Inf fixup (Geometry.cs:1153-1167).
    ///   2. Break every triangle into 3 unique vertices (REUVFace1, :1172-1201).
    ///   3. Optionally merge vertex colors (:1202-1209) — gated by `merge_vertex_colors`.
    ///   4. Loose re-weld via RemoveDuplicate (:1429).
    ///   5. Drop duplicate triangles (:1430-1445).
    ///   6. Simplify loop, up to 10 iterations (:1451-1458).
    ///
    /// `generate_tangents` mirrors `geometry.HasTangents()` captured at :1141; it is
    /// only consumed by the deferred `flag` recompute, so it is accepted for
    /// signature parity but unused on the common path.
    ///
    /// `cross_seam_weld` enables a BEYOND-SOURCE pass (NOT in Geometry.cs) that
    /// welds coincident-position / different-UV vertices BEFORE the Simplify loop so
    /// flat fans can collapse across former UV seams — see `cross_seam_weld_vertices`.
    pub fn reuv_break_reweld_simplify(
        &mut self,
        merge_vertex_colors: bool,
        _generate_tangents: bool,
        cross_seam_weld: bool,
    ) {
        if self.triangles.is_empty() {
            return;
        }

        // 1. QUVx-quantize + NaN/Inf fixup — Geometry.cs:1153-1167.
        for uv in &mut self.uvcoords {
            uv[0] = quvx(uv[0]);
            uv[1] = quvx(uv[1]);
            for c in uv.iter_mut() {
                if c.is_nan() || *c == f32::NEG_INFINITY {
                    *c = 0.0;
                }
                if *c == f32::INFINITY {
                    *c = 1.0;
                }
            }
        }

        // 2. Per-triangle break (REUVFace1) — Geometry.cs:1172-1201.
        let has_normals = !self.normals.is_empty();
        let has_tangents = !self.tangents.is_empty();
        let has_bitangents = !self.bitangents.is_empty();
        let has_colors = !self.vertex_colors.is_empty();
        let has_uv = !self.uvcoords.is_empty();

        let mut nv: Vec<[f32; 3]> = Vec::with_capacity(self.triangles.len() * 3);
        let mut nuv: Vec<[f32; 2]> =
            Vec::with_capacity(if has_uv { self.triangles.len() * 3 } else { 0 });
        let mut nnorm: Vec<[f32; 3]> = Vec::new();
        let mut ntan: Vec<[f32; 3]> = Vec::new();
        let mut nbit: Vec<[f32; 3]> = Vec::new();
        let mut ncol: Vec<[f32; 4]> = Vec::new();
        let mut ntris: Vec<[u32; 3]> = Vec::with_capacity(self.triangles.len());

        for tri in &self.triangles {
            for &l in tri {
                let l = l as usize;
                if has_uv {
                    nuv.push(self.uvcoords[l]);
                }
                nv.push(self.vertices[l]);
                if has_colors {
                    ncol.push(self.vertex_colors[l]);
                }
                if has_normals {
                    nnorm.push(self.normals[l]);
                }
                if has_tangents {
                    ntan.push(self.tangents[l]);
                }
                if has_bitangents {
                    nbit.push(self.bitangents[l]);
                }
            }
            let n = nv.len() as u32;
            ntris.push([n - 3, n - 2, n - 1]);
        }

        self.vertices = nv;
        self.uvcoords = nuv;
        self.normals = nnorm;
        self.tangents = ntan;
        self.bitangents = nbit;
        self.vertex_colors = ncol;
        self.triangles = ntris;

        // 3. Merge vertex colors — Geometry.cs:1202-1209.
        if merge_vertex_colors {
            self.merge_vertex_colors(false);
        }

        // 4. Loose re-weld — Geometry.cs:1429.
        self.remove_duplicate(false);

        // 5. Drop duplicate triangles — Geometry.cs:1430-1445.
        self.dedup_triangles();

        // 5b. BEYOND-SOURCE cross-UV-seam weld (not in Geometry.cs). Merges verts
        // that share a position across UV seams so the Simplify fans below can span
        // the former seam. Gated; safety-bounded (see cross_seam_weld_vertices).
        if cross_seam_weld {
            self.cross_seam_weld_vertices();
        }

        // 6. Simplify loop, up to 10 iterations — Geometry.cs:1451-1458.
        let mut iter = 0;
        while self.simplify() && iter < 10 {
            iter += 1;
        }

        // 6b. A successful cross-seam weld opens NEW flat fans that the first
        // Simplify pass could not see (a former seam vertex only becomes interior
        // once both sides are welded). Re-run break+reweld+Simplify ONCE more with
        // the weld so the freshly-merged regions get a second collapse pass. Bounded
        // to a single recursion (no `cross_seam_weld` arg) to stay deterministic and
        // terminating; the inner Simplify loop is itself fixed-point.
        if cross_seam_weld {
            let before = self.triangles.len();
            self.cross_seam_weld_vertices();
            if self.triangles.len() != before {
                let mut iter = 0;
                while self.simplify() && iter < 10 {
                    iter += 1;
                }
            }
        }
    }

    /// BEYOND-SOURCE cross-UV-seam weld (NOT a port — absent from Geometry.cs).
    ///
    /// xLODGen's faithful `RemoveDuplicate` welds vertices only when BOTH position
    /// (0.5) AND UV (0.005) coincide, so it preserves every UV seam: two verts at
    /// the same position but on opposite sides of a UV seam stay split, and
    /// `Simplify`'s coplanar-fan collapse cannot cross that seam. On UV-seamed
    /// atlassed shapes (boats, etc.) this leaves the tri count well above golden.
    ///
    /// This pass welds vertices that share a POSITION (within `POS_EPS`, the same
    /// 0.5 the faithful weld uses) even when their source UVs differ, so the
    /// Simplify fans below can span the former seam.
    ///
    /// TEXTURING SAFETY — the binding constraint:
    /// * Each atlassed shape maps to exactly ONE atlas rect (transform_shape looks
    ///   up a single `rect` for the shape's `textures_key` and remaps ALL of this
    ///   shape's UVs through it). So welding two verts of THIS shape can never pull
    ///   texture across an atlas-TILE boundary — the whole shape lives in one tile.
    /// * The only intra-shape risk is smearing the SOURCE texture by welding verts
    ///   whose UVs sit in far-apart regions of the source image. We forbid that:
    ///   two verts are welded only if their UVs lie within the band returned by
    ///   `cross_seam_uv_band()` (default 0.10 in each axis ≈ 10% of the
    ///   source/atlas-rect extent) of the chosen representative. A genuine far-apart
    ///   seam (e.g. u≈0.0 vs u≈1.0) FAILS the band and is left split — exactly the
    ///   texturing-safe choice. The representative KEEPS its own UV (we pick, never
    ///   average across distant regions), bounding any single weld's UV displacement
    ///   to ≤ the band. MEASURED: widening the band does not improve convergence (see
    ///   `cross_seam_uv_band`), so the safest band is also the best — no tradeoff.
    ///
    /// Determinism: positions are bucketed into a sorted `BTreeMap` keyed by
    /// quantized integer coordinates; within a bucket the lowest vertex index is the
    /// representative and members are visited in ascending index order. No HashMap
    /// iteration-order or RNG/clock dependence.
    fn cross_seam_weld_vertices(&mut self) {
        if self.triangles.is_empty() || self.vertices.is_empty() {
            return;
        }
        const POS_EPS: f32 = 0.5; // matches RemoveDuplicate's position threshold.
        let uv_band = cross_seam_uv_band();
        let has_uv = !self.uvcoords.is_empty();

        // Bucket vertices by quantized position. Quantize to a 0.5 grid then also
        // probe the 3x3x3 neighbor cells so two verts within POS_EPS but straddling
        // a cell boundary still land in the same group. Deterministic via BTreeMap.
        let q = |v: f32| (v / POS_EPS).round() as i64;
        let mut buckets: std::collections::BTreeMap<(i64, i64, i64), Vec<u32>> =
            std::collections::BTreeMap::new();
        for (i, v) in self.vertices.iter().enumerate() {
            buckets
                .entry((q(v[0]), q(v[1]), q(v[2])))
                .or_default()
                .push(i as u32);
        }

        // remap[i] = representative vertex index for i (defaults to self).
        let mut remap: Vec<u32> = (0..self.vertices.len() as u32).collect();

        // Visit candidate vertices in ascending index order (deterministic).
        for i in 0..self.vertices.len() as u32 {
            // Skip if i was already merged into an earlier representative.
            if remap[i as usize] != i {
                continue;
            }
            let vi = self.vertices[i as usize];
            let ui = if has_uv {
                self.uvcoords[i as usize]
            } else {
                [0.0, 0.0]
            };
            // Search this cell and its neighbors for higher-index, unmerged,
            // coincident-position verts within the UV band.
            let (cx, cy, cz) = (q(vi[0]), q(vi[1]), q(vi[2]));
            let mut candidates: Vec<u32> = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(list) = buckets.get(&(cx + dx, cy + dy, cz + dz)) {
                            candidates.extend(list.iter().copied());
                        }
                    }
                }
            }
            candidates.sort_unstable();
            candidates.dedup();
            for j in candidates {
                if j <= i || remap[j as usize] != j {
                    continue;
                }
                let vj = self.vertices[j as usize];
                if (vi[0] - vj[0]).abs() > POS_EPS
                    || (vi[1] - vj[1]).abs() > POS_EPS
                    || (vi[2] - vj[2]).abs() > POS_EPS
                {
                    continue; // not coincident position.
                }
                if has_uv {
                    let uj = self.uvcoords[j as usize];
                    if (ui[0] - uj[0]).abs() > uv_band || (ui[1] - uj[1]).abs() > uv_band {
                        continue; // far-apart UV region — preserve the seam (SAFETY).
                    }
                }
                remap[j as usize] = i; // merge j into representative i (keeps i's UV).
            }
        }

        // Remap triangle indices, drop degenerate tris, dedup, then compact.
        let mut changed = false;
        for tri in &mut self.triangles {
            for idx in tri.iter_mut() {
                let r = remap[*idx as usize];
                if r != *idx {
                    *idx = r;
                    changed = true;
                }
            }
        }
        if !changed {
            return;
        }
        self.triangles
            .retain(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
        self.dedup_triangles();
        self.remove_unused();
    }

    /// Merge near-coincident vertex colors (in place).
    /// Port: Geometry.MergeVertexColors(high=false) (Geometry.cs:820), the only
    /// caller in ReUV. With `high=false` no remap/RemoveUnused happens — colors are
    /// only averaged between matched pairs (dictionary stays empty).
    fn merge_vertex_colors(&mut self, high: bool) {
        if self.vertex_colors.is_empty() || self.uvcoords.is_empty() {
            return;
        }
        let pos_t = 0.5f32;
        let uv_t = if high { 0.001f32 } else { 0.005f32 };
        for i in 0..self.vertices.len() {
            let vi = self.vertices[i];
            for j in 0..i {
                let vj = self.vertices[j];
                if (vi[0] - vj[0]).abs() > pos_t
                    || (vi[1] - vj[1]).abs() > pos_t
                    || (vi[2] - vj[2]).abs() > pos_t
                {
                    continue;
                }
                let ui = self.uvcoords[i];
                let uj = self.uvcoords[j];
                if (ui[0] - uj[0]).abs() > uv_t || (ui[1] - uj[1]).abs() > uv_t {
                    continue;
                }
                let avg = [
                    (self.vertex_colors[i][0] + self.vertex_colors[j][0]) / 2.0,
                    (self.vertex_colors[i][1] + self.vertex_colors[j][1]) / 2.0,
                    (self.vertex_colors[i][2] + self.vertex_colors[j][2]) / 2.0,
                    (self.vertex_colors[i][3] + self.vertex_colors[j][3]) / 2.0,
                ];
                self.vertex_colors[i] = avg;
                self.vertex_colors[j] = avg;
            }
        }
    }

    /// Remove duplicate triangles by (v1,v2,v3) index key, keeping first occurrence.
    /// Port: Geometry.cs:1430-1445.
    fn dedup_triangles(&mut self) {
        let mut seen: std::collections::HashSet<[u32; 3]> = std::collections::HashSet::new();
        let mut kept: Vec<[u32; 3]> = Vec::with_capacity(self.triangles.len());
        for &t in &self.triangles {
            if seen.insert(t) {
                kept.push(t);
            }
        }
        self.triangles = kept;
    }

    /// Coplanar-fan collapse decimation (one pass). Returns `true` if the mesh changed.
    ///
    /// Faithful port of `Geometry.Simplify()` (Geometry.cs:1999-2163). For every
    /// vertex, gather the triangles using it. If the triangles sharing that vertex
    /// all have the SAME face normal AND their angles at the vertex sum to ~360°
    /// (interior) or ~180° (edge), the vertex is interior to a flat region: remove
    /// it and re-triangulate the surrounding boundary polygon (the fan) without it.
    ///
    /// Determinism: the C# version iterates a ConcurrentDictionary (nondeterministic);
    /// we iterate vertices in stable ascending index order via a BTreeMap so the
    /// collapse order — and therefore the output — is reproducible.
    pub fn simplify(&mut self) -> bool {
        if self.triangles.is_empty() {
            return false;
        }

        // tris: triangle index -> [v0,v1,v2]; verts: vertex index -> triangle indices.
        // BTreeMap for deterministic iteration order (C# uses ConcurrentDictionary).
        let mut tris: std::collections::BTreeMap<usize, [u32; 3]> =
            std::collections::BTreeMap::new();
        let mut norms: std::collections::BTreeMap<usize, String> =
            std::collections::BTreeMap::new();
        let mut verts: std::collections::BTreeMap<u32, Vec<usize>> =
            std::collections::BTreeMap::new();
        for (ti, t) in self.triangles.iter().enumerate() {
            tris.insert(ti, *t);
            for &v in t {
                verts.entry(v).or_default().push(ti);
            }
        }
        let mut next_tri = self.triangles.len();

        let mut changed_outer = false;
        let mut flag = true;
        while flag {
            flag = false;
            // Snapshot keys for deterministic, mutation-safe iteration.
            let vkeys: Vec<u32> = verts.keys().copied().collect();
            for vkey in vkeys {
                let tlist = match verts.get_mut(&vkey) {
                    Some(l) => {
                        l.retain(|ti| tris.contains_key(ti));
                        if l.len() < 2 {
                            continue;
                        }
                        l.clone()
                    }
                    _ => continue,
                };

                // Reference winding & normal of the first triangle — Geometry.cs:2031-2036.
                let Some(&t0) = tris.get(&tlist[0]) else {
                    continue;
                };
                let flag2 = uv_clockwise(
                    self.uvcoords[t0[0] as usize],
                    self.uvcoords[t0[1] as usize],
                    self.uvcoords[t0[2] as usize],
                );
                let ref_norm = match norms.get(&tlist[0]) {
                    Some(n) => n.clone(),
                    None => {
                        let n = face_normal_key(&self.vertices, &t0);
                        norms.insert(tlist[0], n.clone());
                        n
                    }
                };

                // Sum the angles at vkey over coplanar same-winding triangles — :2037-2070.
                let mut angle_sum = 0.0f32;
                for &nt in &tlist {
                    let Some(&tri) = tris.get(&nt) else {
                        continue;
                    };
                    let nkey = match norms.get(&nt) {
                        Some(n) => n.clone(),
                        None => {
                            let n = face_normal_key(&self.vertices, &tri);
                            norms.insert(nt, n.clone());
                            n
                        }
                    };
                    if nkey != ref_norm {
                        continue;
                    }
                    // Opposite-winding neighbor → not a clean flat fan (bail) — :2047-2051.
                    if flag2
                        != uv_clockwise(
                            self.uvcoords[tri[0] as usize],
                            self.uvcoords[tri[1] as usize],
                            self.uvcoords[tri[2] as usize],
                        )
                    {
                        angle_sum -= 360.0;
                        break;
                    }
                    // Rotate so vkey is the apex (index slot), then add its angle — :2052-2067.
                    let (mut a, mut b, mut c) = (tri[0], tri[1], tri[2]);
                    if vkey == b {
                        a = tri[1];
                        b = tri[2];
                        c = tri[0];
                    } else if vkey == c {
                        a = tri[2];
                        b = tri[0];
                        c = tri[1];
                    }
                    angle_sum += vec3_corner_angle_deg(
                        self.vertices[a as usize],
                        self.vertices[b as usize],
                        self.vertices[c as usize],
                    );
                }
                let angle_sum = angle_sum.round();
                // Interior (~360) or on-edge (~180) flat fan — :2071.
                if !(angle_sum >= 355.0) && !((175.0..=185.0).contains(&angle_sum)) {
                    continue;
                }

                // Collect the boundary loop (verts != vkey) and the fan triangles — :2075-2093.
                let mut boundary: Vec<u32> = Vec::new();
                let mut fan_tris: Vec<usize> = Vec::new();
                for &nt in &tlist {
                    if norms.get(&nt).map(|s| *s != ref_norm).unwrap_or(true) {
                        continue;
                    }
                    let Some(tri) = tris.get(&nt) else {
                        continue;
                    };
                    fan_tris.push(nt);
                    for &vi in tri {
                        if vi != vkey && !boundary.contains(&vi) {
                            boundary.push(vi);
                        }
                    }
                }

                // Average vkey's vertex color into the boundary verts — :2094-2111.
                let center_color = if !self.vertex_colors.is_empty() {
                    self.vertex_colors[vkey as usize]
                } else {
                    [0.0; 4]
                };
                let mut fan_pts: Vec<crate::terrain::delaunay::Vertex> =
                    Vec::with_capacity(boundary.len());
                for &bi in &boundary {
                    let uv = self.uvcoords[bi as usize];
                    fan_pts.push(crate::terrain::delaunay::Vertex { x: uv[0], y: uv[1] });
                    if !self.vertex_colors.is_empty() {
                        let c = &mut self.vertex_colors[bi as usize];
                        c[0] = (c[0] + center_color[0]) / 2.0;
                        c[1] = (c[1] + center_color[1]) / 2.0;
                        c[2] = (c[2] + center_color[2]) / 2.0;
                        c[3] = (c[3] + center_color[3]) / 2.0;
                    }
                }

                let center_uv = self.uvcoords[vkey as usize];
                let center = crate::terrain::delaunay::Vertex {
                    x: center_uv[0],
                    y: center_uv[1],
                };

                // Re-triangulate the fan WITHOUT vkey — :2112-2116.
                let prev_count = self.triangles.len();
                if !self
                    .create_triangles(&fan_pts, &boundary, center, false, flag2, 1.0, 1.0, false)
                {
                    continue;
                }

                // Register the newly appended triangles — :2117-2135.
                for ti in prev_count..self.triangles.len() {
                    let t = self.triangles[ti];
                    tris.insert(ti, t);
                    norms.insert(ti, face_normal_key(&self.vertices, &t));
                    for &v in &t {
                        verts.entry(v).or_default().push(ti);
                    }
                }
                next_tri = self.triangles.len();

                // Remove the old fan triangles from the working maps — :2136-2145.
                for &old in fan_tris.iter().rev() {
                    tris.remove(&old);
                    norms.remove(&old);
                    for &bi in &boundary {
                        if let Some(l) = verts.get_mut(&bi) {
                            l.retain(|&x| x != old);
                        }
                    }
                }
                verts.remove(&vkey);
                flag = true;
                changed_outer = true;
                break;
            }
        }

        // Rebuild the triangle list from the surviving working set — :2150-2155.
        let tri_count_before = self.triangles.len();
        self.triangles = tris.values().copied().collect();
        let vert_count_before = self.vertices.len();

        // Final loose re-weld — :2157.
        self.remove_duplicate(false);

        let _ = next_tri;
        let _ = changed_outer;
        // Changed iff vertex or triangle count moved — :2158-2161.
        self.vertices.len() != vert_count_before || self.triangles.len() != tri_count_before
    }

    /// Re-triangulate a fan of boundary points around `center` in UV space.
    ///
    /// Faithful port of `Geometry.CreateTriangles` (Geometry.cs:1526). Sorts the
    /// boundary points counter-clockwise around `center`; rejects (returns false) on
    /// a duplicate UV key, fewer than 3 points, or — when `!hull` — a non-convex fan
    /// (some point interior to the hull). Otherwise Delaunay-triangulates and appends
    /// triangles with winding consistent with `front`.
    ///
    /// `create_new` (always false from Simplify) would duplicate vertices and rescale
    /// their UVs; only the false branch is needed here.
    #[allow(clippy::too_many_arguments)]
    fn create_triangles(
        &mut self,
        pts: &[crate::terrain::delaunay::Vertex],
        idx: &[u32],
        center: crate::terrain::delaunay::Vertex,
        create_new: bool,
        front: bool,
        _scale_u: f32,
        _scale_v: f32,
        hull: bool,
    ) -> bool {
        debug_assert!(!create_new, "create_new=true path unused by Simplify");
        use crate::terrain::delaunay::{
            Vertex as DVertex, convex_hull_len, triangulate_reject_dups,
        };

        // Counter-clockwise sort around center; reject duplicate UV keys — :1528-1545.
        // SortedDictionary keyed by SortCounterClockwise; ContainsKey check rejects dups.
        let mut entries: Vec<(DVertex, u32)> = Vec::with_capacity(pts.len());
        for (p, &i) in pts.iter().zip(idx.iter()) {
            // Duplicate detection uses Vertex.Equals (exact x==x && y==y) — :1533.
            if entries.iter().any(|(q, _)| q.x == p.x && q.y == p.y) {
                return false;
            }
            entries.push((*p, i));
        }
        entries.sort_by(|a, b| sort_counter_clockwise(&a.0, &b.0, &center));
        let sorted_pts: Vec<DVertex> = entries.iter().map(|(p, _)| *p).collect();
        let sorted_idx: Vec<u32> = entries.iter().map(|(_, i)| *i).collect();

        if sorted_pts.len() < 3 {
            return false;
        }

        // Degeneracy guard: a collinear (zero-area) fan or one with fewer than 3
        // unique points has no valid triangulation. The S-hull routines report this
        // as a value (`None` / hull len 0), but this cheap pre-check rejects the
        // common flat-edge case early. xLODGen tolerates degeneracy via ReUV's outer
        // try/catch (the collapse is simply skipped); we reject cleanly instead.
        if collinear(&sorted_pts) {
            return false;
        }

        // Convexity guard — :1549. Reject if some point is interior to the hull.
        // `convex_hull_len` returns 0 on degenerate input it can't seed, which can
        // never equal a real point count, so the check fails cleanly there too.
        if !hull && convex_hull_len(&sorted_pts, true) != sorted_pts.len() {
            return false;
        }

        // Delaunay-triangulate the fan. `None` means a degenerate fan the S-hull
        // algorithm cannot seed — treat it as a rejected collapse (same observable
        // behavior as xLODGen's outer try/catch).
        let triads = match triangulate_reject_dups(&sorted_pts) {
            Some(t) => t,
            None => return false,
        };
        for tri in &triads {
            // createnew == false branch — :1564-1568. C# emits (c, b, a).
            let i0 = sorted_idx[tri.c];
            let i1 = sorted_idx[tri.b];
            let i2 = sorted_idx[tri.a];
            // Winding fix to match `front` — :1569-1576.
            if front
                == uv_clockwise(
                    self.uvcoords[i0 as usize],
                    self.uvcoords[i1 as usize],
                    self.uvcoords[i2 as usize],
                )
            {
                self.triangles.push([i0, i1, i2]);
            } else {
                self.triangles.push([i0, i2, i1]);
            }
        }
        true
    }
}

/// Same-region UV bound for the cross-UV-seam weld (see `cross_seam_weld_vertices`
/// SAFETY). Default 0.10.
///
/// MEASURED (FarHarbor boat LOD corpus, golden_object_weld_e2e band sweep): widening
/// the band does NOT improve convergence — boat-quad tris at band 0.10/0.20/0.35/0.50
/// were 404/411/421/422 (golden 178). Welding distant-UV verts feeds `Simplify` fans
/// that are non-convex in UV, which `create_triangles` then REJECTS (so they don't
/// collapse) — and the extra welded vertices re-triangulate into slightly MORE tris.
/// So the smallest texturing-safe band is also the best-converging one; there is no
/// convergence-vs-texturing tradeoff to take. 0.10 ≈ 10% of the source/atlas-rect
/// extent: welds only near-coincident-UV seams, preserves all far-apart shell seams.
/// Overridable via `LODGEN_WELD_UV_BAND` for the band-sweep diagnostic (test
/// instrumentation only; this is the native crate, not the env-restricted Python
/// service layer).
fn cross_seam_uv_band() -> f32 {
    const DEFAULT: f32 = 0.10;
    match std::env::var("LODGEN_WELD_UV_BAND") {
        Ok(s) => s
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(DEFAULT),
        Err(_) => DEFAULT,
    }
}

/// True if all 2D points are (numerically) collinear — no triangle has area.
/// Used to reject fans the S-hull triangulator cannot seed (collinear → no
/// circumcircle). Mirrors the practical effect of xLODGen's try/catch around ReUV.
fn collinear(pts: &[crate::terrain::delaunay::Vertex]) -> bool {
    if pts.len() < 3 {
        return true;
    }
    let p0 = pts[0];
    // Find the first point distinct from p0 to form a reference direction.
    let mut dir: Option<(f32, f32)> = None;
    for p in &pts[1..] {
        let d = (p.x - p0.x, p.y - p0.y);
        if d.0 != 0.0 || d.1 != 0.0 {
            dir = Some(d);
            break;
        }
    }
    let (dx, dy) = match dir {
        Some(d) => d,
        None => return true, // all coincident
    };
    // Any point off the p0+dir line (nonzero cross) → not collinear.
    for p in &pts[1..] {
        let cross = dx * (p.y - p0.y) - dy * (p.x - p0.x);
        if cross.abs() > 1e-12 {
            return false;
        }
    }
    true
}

/// QUVx: signed-truncate a UV to 3 decimal places — Utils.cs:325.
fn quvx(value: f32) -> f32 {
    if value < 0.0 {
        (value * 1000.0).ceil() / 1000.0
    } else {
        (value * 1000.0).floor() / 1000.0
    }
}

/// UV-space clockwise test — UVCoord.Clockwise (UVCoord.cs:58): cross((B-A),(C-B)) > 0.
fn uv_clockwise(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let bc = [c[0] - b[0], c[1] - b[1]];
    ab[0] * bc[1] - ab[1] * bc[0] > 0.0
}

/// Face-normal string key: normalized cross product, rounded to 1 decimal (ToString2).
/// Port: CreateNormal (Geometry.cs:1941) + Vector3.ToString2 (Vector3.cs:139).
/// Used as the coplanarity test in Simplify — two triangles are "coplanar" iff
/// their rounded normal strings are equal.
fn face_normal_key(vertices: &[[f32; 3]], tri: &[u32; 3]) -> String {
    let a = vertices[tri[0] as usize];
    let b = vertices[tri[1] as usize];
    let c = vertices[tri[2] as usize];
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let mut n = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 0.0 {
        n[0] /= len;
        n[1] /= len;
        n[2] /= len;
    }
    format!("{},{},{}", round1(n[0]), round1(n[1]), round1(n[2]))
}

/// C# Math.Round(x, 1) — banker's rounding to 1 decimal place. Matches ToString2.
fn round1(x: f32) -> f32 {
    let scaled = (x as f64) * 10.0;
    let r = scaled.round_ties_even() / 10.0;
    // Normalize -0.0 to 0.0 so "+0" and "-0" normals share a key (C# prints "0").
    let r = if r == 0.0 { 0.0 } else { r };
    r as f32
}

/// Corner angle (degrees) at `a` in triangle (a,b,c) — Vector3.Angle(v1,v2,v3)
/// (Vector3.cs:89): acos(dot(normalize(b-a), normalize(c-a))) * 180/pi.
fn vec3_corner_angle_deg(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let mut u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let mut v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let nu = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
    let nv = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if nu > 0.0 {
        u = [u[0] / nu, u[1] / nu, u[2] / nu];
    }
    if nv > 0.0 {
        v = [v[0] / nv, v[1] / nv, v[2] / nv];
    }
    let dot = (u[0] * v[0] + u[1] * v[1] + u[2] * v[2]).clamp(-1.0, 1.0) as f64;
    (dot.acos() * 180.0 / std::f64::consts::PI) as f32
}

/// SortCounterClockwise comparator around `center` — Vertex.cs:9-71. Ports the
/// exact branch order so the fan-point ordering matches xLODGen's SortedDictionary.
fn sort_counter_clockwise(
    a: &crate::terrain::delaunay::Vertex,
    b: &crate::terrain::delaunay::Vertex,
    center: &crate::terrain::delaunay::Vertex,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let cx = center.x;
    let cy = center.y;
    let adx = a.x - cx;
    let bdx = b.x - cx;
    if adx >= 0.0 && bdx < 0.0 {
        return Ordering::Less;
    }
    if adx < 0.0 && bdx >= 0.0 {
        return Ordering::Greater;
    }
    if adx == 0.0 && bdx == 0.0 {
        if a.y - cy >= 0.0 || b.y - cy >= 0.0 {
            if a.y > b.y {
                return Ordering::Less;
            }
            if a.y < b.y {
                return Ordering::Greater;
            }
        } else {
            if b.y > a.y {
                return Ordering::Less;
            }
            if b.y < a.y {
                return Ordering::Greater;
            }
        }
    }
    let det = (a.x - cx) * (b.y - cy) - (b.x - cx) * (a.y - cy);
    if det < 0.0 {
        return Ordering::Less;
    }
    if det > 0.0 {
        return Ordering::Greater;
    }
    let da = (a.x - cx) * (a.x - cx) + (a.y - cy) * (a.y - cy);
    let db = (b.x - cx) * (b.x - cx) + (b.y - cy) * (b.y - cy);
    if da > db {
        return Ordering::Less;
    }
    if da < db {
        return Ordering::Greater;
    }
    Ordering::Equal
}

impl Default for LodGeometry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod simplify_tests {
    use super::*;

    /// A flat unit square fanned from a center vertex into 4 coplanar triangles.
    /// XY positions == UV coords so winding/sort/normal tests are self-consistent.
    /// Vertices: 0=(0,0) 1=(1,0) 2=(1,1) 3=(0,1) corners; 4=(0.5,0.5) center.
    fn flat_fan_square() -> LodGeometry {
        let mut g = LodGeometry::new();
        g.vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.5, 0.5, 0.0],
        ];
        g.uvcoords = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.5, 0.5]];
        g.normals = vec![[0.0, 0.0, 1.0]; 5];
        // 4 triangles around center 4, consistent CCW winding.
        g.triangles = vec![[0, 1, 4], [1, 2, 4], [2, 3, 4], [3, 0, 4]];
        g
    }

    #[test]
    fn simplify_collapses_flat_fan_center() {
        let mut g = flat_fan_square();
        assert_eq!(g.num_triangles(), 4);
        let changed = g.simplify();
        assert!(changed, "flat fan must simplify");
        // 4 boundary corners → 2 triangles. (Faithful to C#: Simplify ends with
        // RemoveDuplicate(false), which does NOT compact orphaned verts unless they
        // are duplicates — the orphan center vertex is dropped by a later optimize().)
        assert_eq!(g.num_triangles(), 2, "square fan collapses to 2 tris");
        // No surviving triangle references the (now-orphaned) center vertex 4.
        assert!(
            g.triangles.iter().all(|t| !t.contains(&4)),
            "center vertex no longer referenced"
        );
        // optimize() compacts the orphan → 4 verts.
        g.optimize();
        assert_eq!(g.num_vertices(), 4, "center vertex removed after optimize");
        for t in &g.triangles {
            for &i in t {
                assert!((i as usize) < g.num_vertices());
            }
        }
    }

    #[test]
    fn simplify_is_deterministic() {
        let mut a = flat_fan_square();
        let mut b = flat_fan_square();
        a.simplify();
        b.simplify();
        assert_eq!(a.triangles, b.triangles, "same input → same triangles");
        assert_eq!(a.vertices, b.vertices, "same input → same vertices");
    }

    #[test]
    fn simplify_preserves_corner_uvs_and_normals() {
        let mut g = flat_fan_square();
        g.simplify();
        g.optimize(); // drop the orphaned center vertex (see note above)
        // The 4 surviving corner verts keep their UVs and the +Z normal.
        for uv in &g.uvcoords {
            assert!(uv[0] == 0.0 || uv[0] == 1.0, "corner u preserved: {uv:?}");
            assert!(uv[1] == 0.0 || uv[1] == 1.0, "corner v preserved: {uv:?}");
        }
        for n in &g.normals {
            assert!((n[2] - 1.0).abs() < 1e-4, "normal stays +Z: {n:?}");
        }
        assert_eq!(g.uvcoords.len(), g.vertices.len());
        assert_eq!(g.normals.len(), g.vertices.len());
    }

    #[test]
    fn simplify_noop_on_already_minimal_quad() {
        // Two triangles forming a flat quad — no interior vertex to remove.
        let mut g = LodGeometry::new();
        g.vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        g.uvcoords = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        g.normals = vec![[0.0, 0.0, 1.0]; 4];
        g.triangles = vec![[0, 1, 2], [0, 2, 3]];
        let changed = g.simplify();
        assert!(!changed, "minimal quad cannot simplify further");
        assert_eq!(g.num_triangles(), 2);
    }

    #[test]
    fn simplify_tolerates_malformed_duplicate_vertex_triangles() {
        let mut g = flat_fan_square();
        g.triangles.push([4, 0, 0]);
        g.triangles.push([1, 1, 4]);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            g.simplify();
        }));
        assert!(
            result.is_ok(),
            "malformed source triangles must not poison simplify's working maps"
        );
        for t in &g.triangles {
            for &i in t {
                assert!((i as usize) < g.num_vertices());
            }
        }
    }

    #[test]
    fn simplify_keeps_non_coplanar_pyramid() {
        // A pyramid apex (center vertex lifted in Z) — fan is NOT coplanar, so the
        // apex must be kept (angle/normal test fails).
        let mut g = flat_fan_square();
        g.vertices[4] = [0.5, 0.5, 0.5]; // lift the apex out of plane
        // Recompute apex normal isn't needed; Simplify uses face normals.
        let before_tris = g.num_triangles();
        let before_verts = g.num_vertices();
        let changed = g.simplify();
        assert!(!changed, "non-coplanar pyramid apex must not collapse");
        assert_eq!(g.num_triangles(), before_tris);
        assert_eq!(g.num_vertices(), before_verts);
    }

    #[test]
    fn reuv_break_reweld_simplify_collapses_subdivided_grid() {
        // A flat 2x2 quad grid (3x3 vertices) → 8 triangles, all coplanar. After
        // per-triangle break + reweld + Simplify, the interior + edge-midpoint verts
        // collapse; tri count must drop well below 8 (ideally toward 2).
        let mut g = LodGeometry::new();
        let n = 3; // 3x3 grid
        for j in 0..n {
            for i in 0..n {
                let x = i as f32;
                let y = j as f32;
                g.vertices.push([x, y, 0.0]);
                g.uvcoords.push([x / 2.0, y / 2.0]);
                g.normals.push([0.0, 0.0, 1.0]);
            }
        }
        let vid = |i: usize, j: usize| (j * n + i) as u32;
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                g.triangles
                    .push([vid(i, j), vid(i + 1, j), vid(i + 1, j + 1)]);
                g.triangles
                    .push([vid(i, j), vid(i + 1, j + 1), vid(i, j + 1)]);
            }
        }
        assert_eq!(g.num_triangles(), 8);
        g.reuv_break_reweld_simplify(false, false, false);
        assert!(
            g.num_triangles() < 8,
            "flat grid must decimate (got {} tris)",
            g.num_triangles()
        );
        assert!(g.num_triangles() >= 2, "must keep a valid surface");
        // UV/normal arrays stay consistent with vertex count.
        assert_eq!(g.uvcoords.len(), g.vertices.len());
        assert_eq!(g.normals.len(), g.vertices.len());
        // No NaN UVs introduced.
        for uv in &g.uvcoords {
            assert!(uv[0].is_finite() && uv[1].is_finite());
        }
    }

    #[test]
    fn reuv_break_reweld_simplify_is_deterministic() {
        let build = || {
            let mut g = LodGeometry::new();
            let n = 3;
            for j in 0..n {
                for i in 0..n {
                    g.vertices.push([i as f32, j as f32, 0.0]);
                    g.uvcoords.push([i as f32 / 2.0, j as f32 / 2.0]);
                    g.normals.push([0.0, 0.0, 1.0]);
                }
            }
            let vid = |i: usize, j: usize| (j * n + i) as u32;
            for j in 0..n - 1 {
                for i in 0..n - 1 {
                    g.triangles
                        .push([vid(i, j), vid(i + 1, j), vid(i + 1, j + 1)]);
                    g.triangles
                        .push([vid(i, j), vid(i + 1, j + 1), vid(i, j + 1)]);
                }
            }
            g
        };
        let mut a = build();
        let mut b = build();
        a.reuv_break_reweld_simplify(false, false, true);
        b.reuv_break_reweld_simplify(false, false, true);
        assert_eq!(a.triangles, b.triangles);
        assert_eq!(a.vertices, b.vertices);
    }

    #[test]
    fn quvx_truncates_signed() {
        assert_eq!(super::quvx(0.123456), 0.123);
        assert_eq!(super::quvx(-0.123456), -0.123);
        assert_eq!(super::quvx(1.0), 1.0);
    }

    #[test]
    fn face_normal_key_matches_for_coplanar() {
        let verts = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        let k1 = super::face_normal_key(&verts, &[0, 1, 2]);
        let k2 = super::face_normal_key(&verts, &[1, 3, 2]);
        assert_eq!(k1, k2, "two coplanar +Z tris share a normal key");
    }

    // -----------------------------------------------------------------------
    // cross_seam_weld_vertices — BEYOND-SOURCE weld (gated by cross_seam_weld).
    // -----------------------------------------------------------------------

    /// Two flat triangles meeting along a shared POSITION edge, but the edge verts
    /// are DUPLICATED with near-equal UVs (a UV seam within the weld band). The
    /// faithful reweld would keep them split; the cross-seam weld merges them.
    ///
    /// Layout (XY plane, +Z normal):
    ///   left tri  : p0(0,0) p1(1,0) p2(0,1)
    ///   right tri : p3(1,0)' p4(1,1) p5(0,1)'   where p3≈p1 pos, p5≈p2 pos
    /// p3 and p5 carry slightly different (in-band) UVs from p1/p2.
    fn seam_split_quad(uv_delta: f32) -> LodGeometry {
        let mut g = LodGeometry::new();
        g.vertices = vec![
            [0.0, 0.0, 0.0], // 0
            [1.0, 0.0, 0.0], // 1
            [0.0, 1.0, 0.0], // 2
            [1.0, 0.0, 0.0], // 3 == pos of 1
            [1.0, 1.0, 0.0], // 4
            [0.0, 1.0, 0.0], // 5 == pos of 2
        ];
        // UVs: seam verts (3,5) differ from their position-twins (1,2) by uv_delta.
        g.uvcoords = vec![
            [0.0, 0.0],            // 0
            [1.0, 0.0],            // 1
            [0.0, 1.0],            // 2
            [1.0 + uv_delta, 0.0], // 3 (twin of 1)
            [1.0 + uv_delta, 1.0], // 4
            [0.0 + uv_delta, 1.0], // 5 (twin of 2)
        ];
        g.normals = vec![[0.0, 0.0, 1.0]; 6];
        g.triangles = vec![[0, 1, 2], [3, 4, 5]];
        g
    }

    #[test]
    fn cross_seam_weld_merges_coincident_position_different_uv() {
        // In-band UV delta (0.02 < UV_BAND 0.10): the seam twins must merge.
        let mut g = seam_split_quad(0.02);
        assert_eq!(g.num_vertices(), 6);
        g.cross_seam_weld_vertices();
        // Verts 3,5 fold into 1,2 → 4 unique verts after compaction.
        assert_eq!(g.num_vertices(), 4, "seam twins welded → 4 verts");
        // Triangles still reference valid, in-range indices.
        for t in &g.triangles {
            for &i in t {
                assert!((i as usize) < g.num_vertices());
            }
        }
        // No degenerate triangles survive.
        for t in &g.triangles {
            assert!(
                t[0] != t[1] && t[1] != t[2] && t[0] != t[2],
                "no degenerate tri"
            );
        }
    }

    #[test]
    fn cross_seam_weld_respects_uv_band_far_apart_not_welded() {
        // Out-of-band UV delta (0.5 > UV_BAND 0.10): a far-apart seam (would smear
        // the source texture). Must NOT weld — the seam is preserved.
        let mut g = seam_split_quad(0.5);
        assert_eq!(g.num_vertices(), 6);
        g.cross_seam_weld_vertices();
        assert_eq!(
            g.num_vertices(),
            6,
            "far-apart-UV seam preserved (texturing safety)"
        );
        assert_eq!(g.num_triangles(), 2, "both triangles kept");
    }

    #[test]
    fn cross_seam_weld_representative_keeps_own_uv() {
        // The representative (lower index) keeps its OWN UV — never averaged across
        // the (in-band) twin's region. Bounds each weld's UV displacement.
        let mut g = seam_split_quad(0.02);
        // Capture vert 1's and vert 2's UVs (the representatives).
        let u1 = g.uvcoords[1];
        let u2 = g.uvcoords[2];
        g.cross_seam_weld_vertices();
        // After compaction the surviving verts must include exactly the rep UVs for
        // the welded positions (1.0,0.0) and (0.0,1.0) — NOT the twins' shifted UVs
        // and NOT an average.
        let has = |target: [f32; 2]| {
            g.uvcoords
                .iter()
                .any(|uv| (uv[0] - target[0]).abs() < 1e-6 && (uv[1] - target[1]).abs() < 1e-6)
        };
        assert!(has(u1), "representative vert 1 UV preserved: {u1:?}");
        assert!(has(u2), "representative vert 2 UV preserved: {u2:?}");
        // The twins' shifted UVs (u=1.02, 0.02) must be gone.
        assert!(!has([1.02, 0.0]), "twin UV (1.02,0.0) removed");
        assert!(!has([0.02, 1.0]), "twin UV (0.02,1.0) removed");
    }

    #[test]
    fn cross_seam_weld_drops_degenerate_triangles() {
        // A triangle whose two verts are position-coincident seam twins collapses to
        // a degenerate (two equal indices) after the weld and must be dropped.
        let mut g = LodGeometry::new();
        g.vertices = vec![
            [0.0, 0.0, 0.0], // 0
            [1.0, 0.0, 0.0], // 1
            [1.0, 0.0, 0.0], // 2 == pos of 1 (seam twin)
            [0.0, 1.0, 0.0], // 3
        ];
        g.uvcoords = vec![
            [0.0, 0.0],  // 0
            [1.0, 0.0],  // 1
            [1.0, 0.01], // 2 in-band twin of 1
            [0.0, 1.0],  // 3
        ];
        g.normals = vec![[0.0, 0.0, 1.0]; 4];
        // Tri (0,1,3) is real; tri (1,2,3) becomes degenerate once 2→1.
        g.triangles = vec![[0, 1, 3], [1, 2, 3]];
        g.cross_seam_weld_vertices();
        assert_eq!(g.num_triangles(), 1, "degenerate tri dropped after weld");
        for t in &g.triangles {
            assert!(t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
        }
    }

    #[test]
    fn cross_seam_weld_is_deterministic() {
        let mut a = seam_split_quad(0.02);
        let mut b = seam_split_quad(0.02);
        a.cross_seam_weld_vertices();
        b.cross_seam_weld_vertices();
        assert_eq!(a.triangles, b.triangles, "same input → same triangles");
        assert_eq!(a.vertices, b.vertices, "same input → same vertices");
        assert_eq!(a.uvcoords, b.uvcoords, "same input → same UVs");
    }

    #[test]
    fn cross_seam_weld_noop_when_no_seams() {
        // A clean quad with no coincident-position twins must be untouched.
        let mut g = LodGeometry::new();
        g.vertices = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        g.uvcoords = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        g.normals = vec![[0.0, 0.0, 1.0]; 4];
        g.triangles = vec![[0, 1, 2], [0, 2, 3]];
        let before_v = g.num_vertices();
        let before_t = g.num_triangles();
        g.cross_seam_weld_vertices();
        assert_eq!(g.num_vertices(), before_v, "no twins → verts unchanged");
        assert_eq!(g.num_triangles(), before_t, "no twins → tris unchanged");
    }

    /// END-TO-END (synthetic): a flat surface split down the middle by a UV seam,
    /// each half subdivided into a fan. With cross_seam_weld OFF the faithful
    /// pipeline cannot collapse across the seam; with it ON the whole surface
    /// decimates further — the mechanism the boat meshes need, in miniature.
    fn seamed_subdivided_surface() -> LodGeometry {
        // 3x3 vertex grid over a 2x2 unit square, but the middle column (i==1) is
        // DUPLICATED so the left and right halves have independent UVs (a seam down
        // x=1). Positions of the two middle columns coincide.
        let mut g = LodGeometry::new();
        // Left half cols i=0,1 ; right half cols i=1(dup),2. We'll store columns as
        // separate vertices to create the seam.
        // Vertex layout: for each row j in 0..3, push left(0), leftmid(1),
        // rightmid(1 dup), right(2).
        for j in 0..3 {
            let y = j as f32;
            g.vertices.push([0.0, y, 0.0]); // left
            g.vertices.push([1.0, y, 0.0]); // left-mid
            g.vertices.push([1.0, y, 0.0]); // right-mid (dup pos, seam)
            g.vertices.push([2.0, y, 0.0]); // right
            // UVs: left half u in [0,0.50], right half u in [0.52,1.0]. The seam
            // twins sit at u=0.50 (left-mid) vs u=0.52 (right-mid): a 0.02 UV jump
            // — ABOVE the faithful reweld threshold (0.005) so the faithful path
            // keeps them split, but WITHIN the cross-seam UV_BAND (0.10) so the weld
            // merges them. This is the genuine UV seam the boat meshes exhibit.
            g.uvcoords.push([0.0, y / 2.0]);
            g.uvcoords.push([0.50, y / 2.0]);
            g.uvcoords.push([0.52, y / 2.0]);
            g.uvcoords.push([1.0, y / 2.0]);
            for _ in 0..4 {
                g.normals.push([0.0, 0.0, 1.0]);
            }
        }
        let idx = |i: usize, j: usize| (j * 4 + i) as u32; // i: 0=L,1=Lmid,2=Rmid,3=R
        // Left-half tris (cols 0..1), right-half tris (cols 2..3).
        for j in 0..2 {
            // left cell
            g.triangles.push([idx(0, j), idx(1, j), idx(1, j + 1)]);
            g.triangles.push([idx(0, j), idx(1, j + 1), idx(0, j + 1)]);
            // right cell
            g.triangles.push([idx(2, j), idx(3, j), idx(3, j + 1)]);
            g.triangles.push([idx(2, j), idx(3, j + 1), idx(2, j + 1)]);
        }
        g
    }

    #[test]
    fn cross_seam_weld_enables_more_decimation_than_faithful() {
        let mut faithful = seamed_subdivided_surface();
        let mut welded = seamed_subdivided_surface();
        let t0 = faithful.num_triangles();
        faithful.reuv_break_reweld_simplify(false, false, false);
        welded.reuv_break_reweld_simplify(false, false, true);
        eprintln!(
            "seam surface tris: start={t0} faithful={} welded={}",
            faithful.num_triangles(),
            welded.num_triangles()
        );
        // The weld must not INCREASE triangles vs faithful, and on this seamed
        // surface it strictly decimates further (crosses the x=1 seam).
        assert!(
            welded.num_triangles() <= faithful.num_triangles(),
            "weld never worse than faithful ({} > {})",
            welded.num_triangles(),
            faithful.num_triangles()
        );
        assert!(
            welded.num_triangles() < faithful.num_triangles(),
            "weld crosses the seam and decimates further (welded={} faithful={})",
            welded.num_triangles(),
            faithful.num_triangles()
        );
        // Sanity: arrays stay consistent, UVs finite.
        assert_eq!(welded.uvcoords.len(), welded.vertices.len());
        for uv in &welded.uvcoords {
            assert!(uv[0].is_finite() && uv[1].is_finite());
        }
    }
}
