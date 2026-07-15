// Half-float quantization matching Utils.FloatToShort (Utils.cs:69-96).
// Used by the structural-equality test to decode a golden .btr's vertex stream.
// The actual mesh write delegates to nif_core (which packs fp16 via its own write_hfloat_*).

/// Encode an f32 to fp16 bits using the `half` crate (IEEE round-to-nearest-even).
pub(crate) fn f32_to_half_bits(v: f32) -> u16 {
    half::f16::from_f32(v).to_bits()
}

/// Decode fp16 bits to f32 using the `half` crate.
pub(crate) fn half_bits_to_f32(b: u16) -> f32 {
    half::f16::from_bits(b).to_f32()
}

// ---------------------------------------------------------------------------
// TerrainMesh — output of the block assembly + Terra decimation pipeline.
// Produced by `build_terrain_mesh`; written to disk by `output::btr::write_btr`.
// Positions are in 0..4096 local-block space (un-scaled); the NIF node gets
// `scale = lodLevel` at write time (TerrainLOD.cs:1433).
// ---------------------------------------------------------------------------

/// Assembled terrain mesh in local block space (0..4096 on each axis).
///
/// UV = (post/(32*level), 1 - post/(32*level)) per CreateGeometry (TerrainLOD.cs:284,293).
/// Triangles are u16 indices (FO4 BSTriShape is always u16; max 65535 verts enforced by Terra).
#[derive(Debug, Clone)]
pub struct TerrainMesh {
    pub verts: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub tris: Vec<[u16; 3]>,
    pub bbox: crate::descriptors::BBox,
}

// ---------------------------------------------------------------------------
// assemble_block — TerrainLOD.GenerateQuad block assembly (TerrainLOD.cs:305-455)
//
// Produces the (size, heights) flat heightmap consumed by build_terrain_mesh.
// Heights are divided by lodLevel so the block lives in a 0..4096 local space;
// the NIF node has scale = lodLevel to restore world scale at render time.
// ---------------------------------------------------------------------------

/// Assemble a `level × level` cell block into a `(32*level+1)²` height grid.
///
/// Returns `(size, heights)` where `size = 32*level + 1` (capped at 1025) and
/// `heights` is a row-major `Vec<f32>` of length `size * size`.
///
/// Port of `TerrainLOD.GenerateQuad` block-fill section (`TerrainLOD.cs:305-455`).
pub fn assemble_block(
    world: &crate::input::WorldspaceInput,
    quad: &crate::descriptors::QuadDesc,
    _settings: &crate::settings::LodSettings,
) -> (usize, Vec<f32>) {
    let (size, heights, _) = assemble_block_with_water(world, quad, _settings);
    (size, heights)
}

fn assemble_block_with_water(
    world: &crate::input::WorldspaceInput,
    quad: &crate::descriptors::QuadDesc,
    _settings: &crate::settings::LodSettings,
) -> (usize, Vec<f32>, Vec<f32>) {
    let level = quad.quad_level as usize;
    let size = (32 * level + 1).min(1025);
    let mut heights = vec![0.0f32; size * size];
    let mut water_heights = vec![world.water_height / level as f32; size * size];

    // Default fill heights (missing cells) from worldspace water_height.
    // TerrainLOD.cs:374-383: missing -> landHeight/lodLevel.
    // We use 0.0/lodLevel = 0.0 as landHeight default (matches TerrainLOD default).
    let fallback = 0.0f32;

    // Walk row i (cell_row) and col j (cell_col) of the quad's level×level cell grid.
    // TerrainLOD.cs:321-388: rows i = quad.y .. quad.y+lodLevel, cols j = quad.x .. +lodLevel.
    for row in 0..level {
        for col in 0..level {
            let cell_x = quad.x + col as i32;
            let cell_y = quad.y + row as i32;

            // Find the cell in world.cells.
            let cell = world.cells.iter().find(|c| c.x == cell_x && c.y == cell_y);

            match cell {
                Some(c) => {
                    let cell_water = c.water_height / level as f32;
                    // Copy 33×33 posts into block at offset (col*32, row*32).
                    // TerrainLOD.cs:337-346: offset = (num6*32+l, num5*32+k),
                    // divide each height by lodLevel.
                    let div = level as f32;
                    for k in 0..33usize {
                        for l in 0..33usize {
                            let block_col = col * 32 + l;
                            let block_row = row * 32 + k;
                            // Clamp to avoid overrun at boundaries (size may be capped).
                            if block_col < size && block_row < size {
                                // c.heights is row-major [col + row*33].
                                let h = c.heights[l + k * 33];
                                heights[block_col + block_row * size] = h / div;
                                water_heights[block_col + block_row * size] = cell_water;
                            }
                        }
                    }
                }
                None => {
                    // Missing cell: fill with fallback/lodLevel.
                    let fill = fallback / level as f32;
                    let col_start = col * 32;
                    let row_start = row * 32;
                    for k in 0..33usize {
                        for l in 0..33usize {
                            let block_col = col_start + l;
                            let block_row = row_start + k;
                            if block_col < size && block_row < size {
                                heights[block_col + block_row * size] = fill;
                                water_heights[block_col + block_row * size] =
                                    world.water_height / level as f32;
                            }
                        }
                    }
                }
            }
        }
    }

    (size, heights, water_heights)
}

// ---------------------------------------------------------------------------
// build_terrain_mesh — Terra decimate + CreateGeometry (TerrainLOD.cs:266-303)
//
// 1. Calls assemble_block to get (size, heights).
// 2. Constructs a Terra engine with the level's quality/max_vertices settings.
// 3. Calls terra.triangulate().
// 4. Calls terra.generate_output() to get raw (verts, tris).
// 5. Port of CreateGeometry (TerrainLOD.cs:266-303):
//    - Sorts verts by (y*4096 + x) key (SortedDictionary ordering).
//    - Scales x/y by num3 = 128/level.
//    - UV = (x/(32*level), 1 - y/(32*level)).
//    - Computes BBox over scaled verts.
//    - Remaps triangle indices to sorted-vertex order.
// 6. Returns TerrainMesh with u16 triangle indices.
// ---------------------------------------------------------------------------

/// Decimate a block into a `TerrainMesh`.
///
/// Positions are in local block space (0..4096 x/y); z is the raw height divided by lodLevel.
/// The caller (generate_quad / write_btr) applies `scale = lodLevel` and `z_translation`.
///
/// Port of `TerrainLOD.CreateGeometry` + outer block-assembly call
/// (`TerrainLOD.cs:266-303`, `TerrainLOD.cs:389-395`).
pub fn build_terrain_mesh(
    world: &crate::input::WorldspaceInput,
    quad: &crate::descriptors::QuadDesc,
    settings: &crate::settings::LodSettings,
) -> anyhow::Result<TerrainMesh> {
    use crate::descriptors::BBox;

    let level = quad.quad_level as usize;

    // LOD index: lodLevel = 4<<lodIndex -> lodIndex = log2(lodLevel/4).
    let lod_index = match level {
        4 => 0,
        8 => 1,
        16 => 2,
        32 => 3,
        _ => anyhow::bail!("unsupported lodLevel {level}"),
    };

    let (size, heights, water_heights) = assemble_block_with_water(world, quad, settings);

    let lvl = &settings.terrain.levels[lod_index];
    let error_threshold = lvl.quality;
    // Cap vertex budget: min(max_vertices, 65535) (R1 §4, Game.cs:48).
    // Subtract skirt reservation if skirts enabled (R1 §4): skirts != 0 -> reserve 2000.
    let mut max_verts = lvl.max_vertices.min(65535) as i64;
    if settings.terrain.skirts != 0 {
        max_verts = (max_verts - 2000).max(512);
    }

    // DEFERRED (perf-only, low fidelity impact): optimize_unseen
    // (ScriptedPreInsertion of below-water posts, TerrainLOD.cs:392-455) and
    // hide_quads (RemoveUnseenTerrain, TerrainLOD.cs:526-551) are not ported.
    // They remove faces; their absence only over-keeps geometry, never breaks it.
    // The driver emits a one-time stats.warnings entry while they are unported.

    let mut terra = super::terra::Terra::new(error_threshold, max_verts, size, size, &heights);

    // Forced cell-border + grid skeleton (protect_cell_borders, gap 2/3).
    // Port of GenerateQuad's coarse pre-insertion (TerrainLOD.cs:497-512):
    //   for y in (0..size step level*2): for x in (0..size step level*2):
    //     if x % (size-1) == 0 || y % (size-1) == 0 -> ScriptedPreInsertion(state=1)
    // Force-inserting the border posts preserves cell-border vertices through
    // decimation (no cracks between adjacent quads) AND guarantees a tessellated
    // skeleton so flat / landless blocks don't collapse to 2 triangles.
    // (The optimize-unseen `heightValues < waterheight` term of list4 is deferred
    // with the rest of optimize_unseen.)
    if settings.terrain.protect_cell_borders && size > 1 {
        let stride = (level * 2).max(1);
        let edge = size - 1;
        let mut forced: Vec<(usize, usize)> = Vec::new();
        let mut yy = 0usize;
        while yy < size {
            let mut xx = 0usize;
            while xx < size {
                let idx = xx + yy * size;
                let under_water = settings.terrain.emit_water
                    && level != 4
                    && water_heights[idx] < 16_777_216.0
                    && heights[idx] < water_heights[idx];
                if xx % edge == 0 || yy % edge == 0 || under_water {
                    forced.push((xx, yy));
                }
                xx += stride;
            }
            yy += stride;
        }
        terra.scripted_pre_insertion(&forced, 1);
    }

    terra.triangulate();

    let (raw_verts, raw_tris) = terra.generate_output();

    // CreateGeometry (TerrainLOD.cs:266-303):
    // Sort vertices by key = (y as i32) * 4096 + (x as i32) to match C# SortedDictionary.
    let num3 = 128.0f32 / level as f32; // position scale
    let num4 = (32 * level) as f32; // UV denominator

    // Build a sorted list of (key, original_index) pairs.
    let mut sort_pairs: Vec<(i32, usize)> = raw_verts
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = v[0] as i32;
            let y = v[1] as i32;
            (y * 4096 + x, i)
        })
        .collect();
    sort_pairs.sort_by_key(|p| p.0);

    // old_to_new[old_idx] = new sorted position.
    let mut old_to_new = vec![0usize; raw_verts.len()];
    for (new_idx, &(_, old_idx)) in sort_pairs.iter().enumerate() {
        old_to_new[old_idx] = new_idx;
    }

    let mut bbox = BBox::empty();
    let mut verts: Vec<[f32; 3]> = Vec::with_capacity(sort_pairs.len());
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(sort_pairs.len());

    for &(_, old_idx) in &sort_pairs {
        let v = raw_verts[old_idx];
        let x = v[0];
        let y = v[1];
        let z = v[2];
        let scaled = [x * num3, y * num3, z];
        bbox.grow_vertex(scaled);
        verts.push(scaled);
        uvs.push([x / num4, 1.0 - y / num4]);
    }

    // Remap triangle indices through old_to_new and cast to u16.
    let mut tris: Vec<[u16; 3]> = Vec::with_capacity(raw_tris.len());
    for tri in &raw_tris {
        let a = old_to_new[tri[0]] as u16;
        let b = old_to_new[tri[1]] as u16;
        let c = old_to_new[tri[2]] as u16;
        tris.push([a, b, c]);
    }

    // Skirt ring (gap 1). port: TerrainLOD.AddSkirts (TerrainLOD.cs:569-624).
    // Adds a downward edge-skirt around the quad border to hide LOD seams.
    if settings.terrain.skirts != 0 {
        add_skirts(
            &mut verts,
            &mut uvs,
            &mut tris,
            &mut bbox,
            settings.terrain.skirts as f32,
            lod_index,
            level,
        );
    }

    Ok(TerrainMesh {
        verts,
        uvs,
        tris,
        bbox,
    })
}

/// Add a border skirt ring to a terrain mesh (gap 1).
///
/// Faithful port of `TerrainLOD.AddSkirts` (TerrainLOD.cs:569-624). For every
/// triangle EDGE that lies on a quad boundary (x==0, x==4096, y==0 or y==4096 on
/// both endpoints), two skirt vertices are appended (the edge endpoints dropped
/// in Z by `depth`) and two skirt triangles are added, wound to face outward.
/// This hides the seam between adjacent LOD quads.
///
/// Skirt depth (TerrainLOD.cs:571-579): `skirts < 0 → skirts/level`; otherwise
/// `skirts - (lodIndex-1)*63` (a positive drop applied relative to the edge z).
/// The positive-depth branch drops each skirt vertex to `z - depth`; the
/// negative branch sets an absolute floor z. Verts/UVs/bbox match the C# in
/// lockstep so the seam geometry is identical.
fn add_skirts(
    verts: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    tris: &mut Vec<[u16; 3]>,
    bbox: &mut crate::descriptors::BBox,
    skirts: f32,
    lod_index: usize,
    level: usize,
) {
    // depth (TerrainLOD.cs:571-579).
    let depth = if skirts < 0.0 {
        skirts / level as f32
    } else {
        skirts - (lod_index as f32 - 1.0) * 63.0
    };
    let positive = depth > 0.0;

    // Iterate the ORIGINAL triangle set only (C# walks tris.Count-1..0 and appends
    // beyond the original count; snapshot the original length so appended skirt
    // tris are not themselves skirted).
    let original_tris = tris.len();
    for ti in (0..original_tris).rev() {
        let tri = tris[ti];
        for i in 0..3usize {
            let a = tri[i] as usize;
            let b = if i >= 2 {
                tri[0] as usize
            } else {
                tri[i + 1] as usize
            };
            let va = verts[a];
            let vb = verts[b];
            let on_border = (va[0] == 0.0 && vb[0] == 0.0)
                || (va[0] == 4096.0 && vb[0] == 4096.0)
                || (va[1] == 0.0 && vb[1] == 0.0)
                || (va[1] == 4096.0 && vb[1] == 4096.0);
            if !on_border {
                continue;
            }
            let count = verts.len();
            if positive {
                verts.push([va[0], va[1], va[2] - depth]);
                verts.push([vb[0], vb[1], vb[2] - depth]);
            } else {
                verts.push([va[0], va[1], depth]);
                verts.push([vb[0], vb[1], depth]);
            }
            bbox.grow_vertex(verts[count]);
            bbox.grow_vertex(verts[count + 1]);
            // UVs follow the source endpoints (TerrainLOD.cs:608-609).
            uvs.push(uvs[a]);
            uvs.push(uvs[b]);

            // Winding (TerrainLOD.cs:611-620): orient the skirt quad outward based
            // on the edge direction along the border.
            let outward = (va[0] < vb[0] && va[1] == 0.0 && vb[1] == 0.0)
                || (va[0] > vb[0] && va[1] == 4096.0 && vb[1] == 4096.0)
                || (va[1] > vb[1] && va[0] == 0.0 && vb[0] == 0.0)
                || (va[1] < vb[1] && va[0] == 4096.0 && vb[0] == 4096.0);
            let (c0, c1) = (count as u16, (count + 1) as u16);
            let (na, nb) = (a as u16, b as u16);
            if outward {
                tris.push([nb, na, c0]);
                tris.push([nb, c0, c1]);
            } else {
                tris.push([nb, c0, na]);
                tris.push([nb, c1, c0]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::quads_for;
    use crate::settings::LodSettings;

    fn flat_world(level_cells: i32) -> crate::input::WorldspaceInput {
        let cells = (0..level_cells)
            .flat_map(|y| (0..level_cells).map(move |x| (x, y)))
            .map(|(x, y)| crate::input::CellInput {
                x,
                y,
                heights: vec![10.0; 33 * 33],
                vertex_colors: Vec::new(),
                layers: Vec::new(),
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            })
            .collect();
        crate::input::WorldspaceInput::from_cells("W", cells)
    }

    fn flat_underwater_world(level_cells: i32) -> crate::input::WorldspaceInput {
        let cells = (0..level_cells)
            .flat_map(|y| (0..level_cells).map(move |x| (x, y)))
            .map(|(x, y)| crate::input::CellInput {
                x,
                y,
                heights: vec![-10.0; 33 * 33],
                vertex_colors: Vec::new(),
                layers: Vec::new(),
                hidden_quadrants: [false; 4],
                water_height: 100.0,
            })
            .collect();
        let mut world = crate::input::WorldspaceInput::from_cells("W", cells);
        world.water_height = 100.0;
        world
    }

    #[test]
    fn half_roundtrip_exact_for_small_ints() {
        for v in [0.0f32, 1.0, -1.0, 256.0, 1024.0, 2048.0] {
            let b = f32_to_half_bits(v);
            assert_eq!(half_bits_to_f32(b), v, "value {v}");
        }
    }

    #[test]
    fn half_quantizes_within_tolerance() {
        // 4096 is exactly representable in fp16; 4097 rounds to 4096
        assert_eq!(half_bits_to_f32(f32_to_half_bits(4096.0)), 4096.0);
        let q = half_bits_to_f32(f32_to_half_bits(1234.5));
        assert!((q - 1234.5).abs() < 1.0, "got {q}");
    }

    #[test]
    fn assemble_block_size_lod4() {
        // LOD4 block = 4 cells/side => 32*4+1 = 129 posts/side
        let w = flat_world(4);
        let s = LodSettings::fo4_default();
        let quad = quads_for(&w, 4, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let (size, heights) = assemble_block(&w, &quad, &s);
        assert_eq!(size, 129);
        assert_eq!(heights.len(), 129 * 129);
        // block heights are divided by lodLevel (TerrainLOD.cs:345-346): 10/4 = 2.5
        assert!((heights[0] - 2.5).abs() < 1e-3);
    }

    /// Gap 2/3: at a COARSE level (8/16/32) a flat (or landless) block must NOT
    /// collapse to 2 triangles. xLODGen force-inserts the cell-border + grid
    /// skeleton via ScriptedPreInsertion(list4, 1) (TerrainLOD.cs:497-512), so the
    /// block keeps a full tessellated grid even when perfectly flat. Without the
    /// fix, the flat plane decimates to a single quad (2 tris).
    #[test]
    fn coarse_flat_block_keeps_border_skeleton() {
        let w = flat_world(16);
        let s = LodSettings::fo4_default();
        let quad = quads_for(&w, 16, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let mesh = build_terrain_mesh(&w, &quad, &s).unwrap();
        // A 16×16 flat block with border-protect on must have far more than the
        // 4-corner / 2-tri degenerate result. The forced grid stride is level*2=32
        // border posts on each edge of a size=513 grid.
        assert!(
            mesh.tris.len() > 50,
            "coarse flat block should keep a tessellated border skeleton, got {} tris",
            mesh.tris.len()
        );
        // All four edge-midpoint border vertices must be present (post 256 of 512
        // on each edge → scaled by 128/16 = 8 → 2048 local units).
        let mid = 256.0f32 * (128.0 / 16.0); // = 2048
        let has = |x: f32, y: f32| {
            mesh.verts
                .iter()
                .any(|v| (v[0] - x).abs() < 1.0 && (v[1] - y).abs() < 1.0)
        };
        assert!(has(0.0, mid), "left-edge midpoint border vertex missing");
        assert!(
            has(4096.0, mid),
            "right-edge midpoint border vertex missing"
        );
        assert!(has(mid, 0.0), "bottom-edge midpoint border vertex missing");
        assert!(has(mid, 4096.0), "top-edge midpoint border vertex missing");
    }

    #[test]
    fn coarse_underwater_block_keeps_stride_grid() {
        let w = flat_underwater_world(16);
        let s = LodSettings::fo4_default();
        let quad = quads_for(&w, 16, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let mesh = build_terrain_mesh(&w, &quad, &s).unwrap();
        assert_eq!(
            mesh.tris.len(),
            640,
            "underwater coarse terrain should keep the full stride grid plus skirts"
        );
    }

    /// Gap 3: a quad whose cells have NO LAND record (landless) must still
    /// synthesize a tessellated terrain plane (filled at landHeight), not collapse
    /// to ~2 flat tris. This is the `terrains` cache-miss fill branch of
    /// GenerateQuad (TerrainLOD.cs:372-384) combined with the forced grid.
    #[test]
    fn landless_quad_synthesizes_tessellated_terrain() {
        // World has land only in a faraway island; the quad at (0,0) covers cells
        // with no LAND record.
        let land: Vec<_> = (40..44)
            .flat_map(|y| (40..44).map(move |x| (x, y)))
            .map(|(x, y)| crate::input::CellInput {
                x,
                y,
                heights: vec![10.0; 33 * 33],
                vertex_colors: Vec::new(),
                layers: Vec::new(),
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            })
            .collect();
        let mut w = crate::input::WorldspaceInput::from_cells("W", land);
        w.sw_cell = (0, 0);
        w.ne_cell = (43, 43);

        let s = LodSettings::fo4_default();
        // L16 quad at (0,0): all 16×16 cells are landless.
        let quad = crate::descriptors::QuadDesc {
            z_order: 0,
            x: 0,
            y: 0,
            quad_level: 16,
            quad_index: 0,
            quad_offset: 16384.0,
            static_indices: Vec::new(),
            statics: Vec::new(),
            out_values: crate::descriptors::OutDesc::default(),
        };
        let mesh = build_terrain_mesh(&w, &quad, &s).unwrap();
        assert!(
            mesh.tris.len() > 50,
            "landless quad must synthesize tessellated terrain (got {} tris)",
            mesh.tris.len()
        );
        // X/Y bounds must span the full quad (non-degenerate).
        let max_x = mesh.verts.iter().map(|v| v[0]).fold(f32::MIN, f32::max);
        let max_y = mesh.verts.iter().map(|v| v[1]).fold(f32::MIN, f32::max);
        assert!((max_x - 4096.0).abs() < 1.0, "max_x {max_x}");
        assert!((max_y - 4096.0).abs() < 1.0, "max_y {max_y}");
    }

    /// With protect_cell_borders OFF, a coarse flat block decimates freely (no
    /// forced skeleton) — confirms the flag actually gates the behavior.
    #[test]
    fn coarse_flat_block_no_border_protect_decimates() {
        let w = flat_world(16);
        let mut s = LodSettings::fo4_default();
        s.terrain.protect_cell_borders = false;
        s.terrain.skirts = 0; // isolate the border-protect effect from skirts
        let quad = quads_for(&w, 16, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let mesh = build_terrain_mesh(&w, &quad, &s).unwrap();
        // Flat plane with no forced grid → 2 triangles.
        assert_eq!(mesh.tris.len(), 2, "got {} tris", mesh.tris.len());
    }

    /// Gap 1: skirts add a downward edge ring around the quad border.
    /// A unit square (one quad, 2 tris) has 4 border edges → 4*2 = 8 skirt tris and
    /// 8 skirt verts appended. Skirt verts sit below the source edge by `depth`.
    #[test]
    fn add_skirts_rings_a_unit_quad() {
        use crate::descriptors::BBox;
        // 0..4096 unit quad, two triangles (CCW), flat at z=0.
        let mut verts = vec![
            [0.0f32, 0.0, 0.0],
            [4096.0, 0.0, 0.0],
            [4096.0, 4096.0, 0.0],
            [0.0, 4096.0, 0.0],
        ];
        let mut uvs = vec![[0.0f32, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
        let mut tris = vec![[0u16, 1, 2], [0, 2, 3]];
        let mut bbox = BBox::empty();
        for v in &verts {
            bbox.grow_vertex(*v);
        }
        let before = tris.len();
        // L8 (lod_index 1): depth = 256 - 0 = 256.
        add_skirts(&mut verts, &mut uvs, &mut tris, &mut bbox, 256.0, 1, 8);
        // Each of the 4 border edges of this quad produces 2 skirt tris.
        assert_eq!(tris.len() - before, 8, "expected 8 skirt tris");
        assert_eq!(verts.len(), 4 + 8, "expected 8 skirt verts");
        assert_eq!(uvs.len(), verts.len());
        // Skirt verts are dropped to z = 0 - 256 = -256; bbox min.z follows.
        assert!(
            (bbox.min[2] - (-256.0)).abs() < 1e-3,
            "bbox min z {}",
            bbox.min[2]
        );
        assert!(verts.iter().skip(4).all(|v| (v[2] - (-256.0)).abs() < 1e-3));
    }

    /// Skirt depth follows the per-level formula skirts-(lodIndex-1)*63
    /// (TerrainLOD.cs:578): L4→319, L16→193, L32→130.
    #[test]
    fn skirt_depth_per_level() {
        use crate::descriptors::BBox;
        let make = || {
            (
                vec![[0.0f32, 0.0, 0.0], [0.0, 4096.0, 0.0], [0.0, 2048.0, 0.0]],
                vec![[0.0f32, 0.0], [0.0, 0.0], [0.0, 0.0]],
                vec![[0u16, 1, 2]],
                BBox::empty(),
            )
        };
        for (lod_index, level, expect_depth) in
            [(0usize, 4usize, 319.0f32), (2, 16, 193.0), (3, 32, 130.0)]
        {
            let (mut v, mut u, mut t, mut b) = make();
            for vv in &v {
                b.grow_vertex(*vv);
            }
            add_skirts(&mut v, &mut u, &mut t, &mut b, 256.0, lod_index, level);
            // The left-edge (x==0) skirt verts drop to -expect_depth.
            assert!(
                (b.min[2] - (-expect_depth)).abs() < 1e-3,
                "L(idx {lod_index}): depth {} != {expect_depth}",
                -b.min[2]
            );
        }
    }

    /// The skirts flag gates the ring: skirts==0 leaves the mesh untouched.
    #[test]
    fn skirts_zero_no_ring() {
        let w = flat_world(16);
        let mut s = LodSettings::fo4_default();
        s.terrain.skirts = 0;
        let quad = quads_for(&w, 16, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let mesh = build_terrain_mesh(&w, &quad, &s).unwrap();
        // No skirt verts below z=0 (flat world is at z=0).
        assert!(
            mesh.verts.iter().all(|v| v[2] >= 0.0),
            "skirts==0 must not add below-plane verts"
        );
    }

    /// With skirts on (default), a coarse block has a skirt ring → some verts sit
    /// below the terrain plane.
    #[test]
    fn skirts_on_adds_below_plane_ring() {
        let w = flat_world(16);
        let s = LodSettings::fo4_default(); // skirts=256
        let quad = quads_for(&w, 16, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let mesh = build_terrain_mesh(&w, &quad, &s).unwrap();
        assert!(
            mesh.verts.iter().any(|v| v[2] < 0.0),
            "skirts on must add a below-plane ring"
        );
    }

    #[test]
    fn flat_block_uv_and_scale() {
        let w = flat_world(4);
        // protect_cell_borders forces a grid skeleton (gap 2/3) and skirts add a
        // border ring (gap 1); disable both so the flat block decimates to the
        // 4-corner case this UV/scale test asserts.
        let mut s = LodSettings::fo4_default();
        s.terrain.protect_cell_borders = false;
        s.terrain.skirts = 0;
        let quad = quads_for(&w, 4, &s)
            .into_iter()
            .find(|q| q.x == 0 && q.y == 0)
            .unwrap();
        let mesh = build_terrain_mesh(&w, &quad, &s).unwrap();
        // flat => 4 corners, 2 tris
        assert_eq!(mesh.verts.len(), 4);
        assert_eq!(mesh.tris.len(), 2);
        // CreateGeometry: x scaled by 128/level = 32 ; far corner post index 128 -> 128*32 = 4096
        let max_x = mesh.verts.iter().map(|v| v[0]).fold(f32::MIN, f32::max);
        assert!((max_x - 4096.0).abs() < 1e-2, "max_x {max_x}");
        // UV = post/(32*level)=post/128, V flipped: corner (0,0) -> uv (0,1)
        let corner = mesh
            .verts
            .iter()
            .position(|v| v[0] == 0.0 && v[1] == 0.0)
            .unwrap();
        assert!((mesh.uvs[corner][1] - 1.0).abs() < 1e-3);
    }
}
