// Landless/ocean coarse water-sheet generation.
//
// Port of xLODGen `TerrainLOD.GenerateWater` — the FO4 coarse
// (lodLevel != 4) path (`TerrainLOD.cs:980-1066`) plus the water shape
// transform from `Geometry.ToBSTriShape` / `Geometry.ShiftZ`
// (`Geometry.cs:315-352`, `Geometry.cs:2165-2175`). The writer emits this mesh
// as a segmented `BSSubIndexTriShape` for L4 water and as a plain `BSTriShape`
// for coarser levels, matching shipped FO4 and xLODGen BTR water.
//
// Per-quad rule (TerrainLOD.cs:980-995): for every cell `(j,k)` covered by the
// quad, the water height `num12` defaults to the worldspace `waterHeight` and the
// terrain floor `pz2` defaults to `landHeight` (0); if the cell carries a LAND
// record (`terrains.ContainsKey`) those become the cell's own `waterHeight` and
// `bbox.pz1` (terrain min-z). A water quad is emitted iff
//   `!NaN(num12) && num12 > pz2 && num12 < 16_777_216`
// i.e. water sits above the cell's terrain floor and isn't the "no water"
// sentinel. Each emitted cell adds a 4-vertex / 2-triangle plane at the cell's
// water height; `RemoveDuplicate` then welds coincident border verts (the golden
// `32.-41.-27` water block has 1974 tris over 987 cells but only 1070 verts).
//
use crate::descriptors::{BBox, QuadDesc};
use crate::input::WorldspaceInput;

/// The "no water" / sentinel ceiling (TerrainData.cs:63 / TerrainLOD.cs:863,992).
/// A water height at/above this is treated as absent.
const WATER_SENTINEL: f32 = 16_777_216.0;

/// A tessellated water sheet in local block space (0..4096 x/y), z = water/level.
/// Positions only — the water shape carries NO UVs (xLODGen clears the water
/// geom's uvcoords; the golden vertex desc is VERTEX-only, no UV bit).
#[derive(Debug, Clone)]
pub struct WaterMesh {
    pub verts: Vec<[f32; 3]>,
    pub tris: Vec<[u16; 3]>,
    pub segments: Vec<WaterSegment>,
    pub bbox: BBox,
}

#[derive(Debug, Clone)]
pub struct WaterSegment {
    pub id: i32,
    pub start_triangle: u32,
    pub num_triangles: u16,
}

/// Per-cell terrain floor (min height over the 33×33 posts). Port of
/// `TerrainDesc.bbox.pz1` for the cell's LAND record. World units (NOT divided by
/// lodLevel — the water-emit test compares raw world-unit heights).
fn cell_terrain_floor(heights: &[f32]) -> f32 {
    heights.iter().copied().fold(f32::INFINITY, f32::min)
}

/// True iff a water quad must be emitted for water height `num12` over terrain
/// floor `pz2` (TerrainLOD.cs:992): not NaN, above the floor, below the sentinel.
fn emit_water(num12: f32, pz2: f32) -> bool {
    !num12.is_nan() && num12 > pz2 && num12 < WATER_SENTINEL
}

/// Build the coarse water sheet for `quad`, or `None` if no cell passes the
/// water-emit rule (the common dry/landlocked case — no water block written).
///
/// Port of `GenerateWater`'s FO4 coarse `else` branch
/// (`TerrainLOD.cs:980-1066`):
///   - `num = 4096 / lodLevel` is the per-cell stride in local block space,
///   - each emitted cell adds its 4 corner verts at `z = water / lodLevel` and
///     two triangles `(0,1,2)`/`(1,3,2)`,
///   - `bbWater` accumulates the full sheet bbox (local units),
///   - `RemoveDuplicate` welds coincident verts after the per-cell fill.
pub fn build_water_mesh(world: &WorldspaceInput, quad: &QuadDesc) -> Option<WaterMesh> {
    let level = quad.quad_level;
    if level <= 0 {
        return None;
    }
    let num = 4096.0f32 / level as f32;

    // Worldspace defaults (TerrainLOD.cs:984-985): num12 = worldspace water,
    // pz2 = landHeight (0). For FarHarbor the per-cell water equals the
    // worldspace DNAM value; landless cells fall back to the worldspace water.
    let world_water = world.water_height;
    let land_height = 0.0f32;

    let mut verts: Vec<[f32; 3]> = Vec::new();
    let mut tris: Vec<[u16; 3]> = Vec::new();
    let mut segments: Vec<WaterSegment> = Vec::new();
    let mut bbox = BBox::empty();

    // Walk the quad's level×level cell grid (TerrainLOD.cs:980-982).
    for j in quad.x..quad.x + level {
        for k in quad.y..quad.y + level {
            let mut num12 = world_water;
            let mut pz2 = land_height;
            // terrains.ContainsKey(key) — does this cell carry a LAND record?
            if let Some(cell) = world.cells.iter().find(|c| c.x == j && c.y == k) {
                num12 = cell.water_height;
                pz2 = cell_terrain_floor(&cell.heights);
            }
            if !emit_water(num12, pz2) {
                continue;
            }

            // Cell origin in local block space (TerrainLOD.cs:1018-1019).
            let num14 = (j - quad.x) as f32 * num;
            let num15 = (k - quad.y) as f32 * num;
            // Water z in local space (TerrainLOD.cs:1020-1024): num16 = num12/level.
            let num16 = num12 / level as f32;

            let base = verts.len() as u16;
            let corners = [
                [num14, num15, num16],
                [num14 + num, num15, num16],
                [num14, num15 + num, num16],
                [num14 + num, num15 + num, num16],
            ];
            for c in corners {
                verts.push(c);
            }
            // bbWater grows by the SW and NE corners (TerrainLOD.cs:1031-1032).
            bbox.grow_vertex(corners[0]);
            bbox.grow_vertex(corners[3]);

            let start_triangle = tris.len() as u32;
            segments.push(WaterSegment {
                id: level * (j - quad.x) + (k - quad.y),
                start_triangle,
                num_triangles: 2,
            });
            // Two triangles (TerrainLOD.cs:1036-1037).
            tris.push([base, base + 1, base + 2]);
            tris.push([base + 1, base + 3, base + 2]);
        }
    }

    if tris.is_empty() {
        return None;
    }

    // RemoveDuplicate(high:false) — weld coincident verts, remap triangles
    // (Geometry.cs:705). The golden block has 1974 tris but only 1070 verts.
    let (verts, tris) = remove_duplicate(&verts, &tris);

    Some(WaterMesh {
        verts,
        tris,
        segments,
        bbox,
    })
}

/// Weld vertices that share an exact position; remap triangle indices to the
/// deduplicated set. Port of `Geometry.RemoveDuplicate(high:false)`
/// (`Geometry.cs:705`) for the position-only (no-UV) water geometry: it keys on
/// the f32 position bits so identical corner verts collapse to one.
fn remove_duplicate(verts: &[[f32; 3]], tris: &[[u16; 3]]) -> (Vec<[f32; 3]>, Vec<[u16; 3]>) {
    use std::collections::HashMap;
    let mut map: HashMap<[u32; 3], u16> = HashMap::with_capacity(verts.len());
    let mut out_verts: Vec<[f32; 3]> = Vec::with_capacity(verts.len());
    let mut old_to_new: Vec<u16> = Vec::with_capacity(verts.len());

    for v in verts {
        let key = [v[0].to_bits(), v[1].to_bits(), v[2].to_bits()];
        let new_idx = *map.entry(key).or_insert_with(|| {
            let idx = out_verts.len() as u16;
            out_verts.push(*v);
            idx
        });
        old_to_new.push(new_idx);
    }

    let out_tris: Vec<[u16; 3]> = tris
        .iter()
        .map(|t| {
            [
                old_to_new[t[0] as usize],
                old_to_new[t[1] as usize],
                old_to_new[t[2] as usize],
            ]
        })
        .collect();

    (out_verts, out_tris)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::OutDesc;
    use crate::input::CellInput;

    fn cell(x: i32, y: i32, flat_height: f32, water: f32) -> CellInput {
        CellInput {
            x,
            y,
            heights: vec![flat_height; 33 * 33],
            vertex_colors: Vec::new(),
            layers: Vec::new(),
            hidden_quadrants: [false; 4],
            water_height: water,
        }
    }

    fn quad(level: i32, x: i32, y: i32) -> QuadDesc {
        QuadDesc {
            z_order: 0,
            x,
            y,
            quad_level: level,
            quad_index: 0,
            quad_offset: 16384.0,
            static_indices: Vec::new(),
            statics: Vec::new(),
            out_values: OutDesc::default(),
        }
    }

    /// A cell whose water height sits above its terrain floor yields a water plane.
    #[test]
    fn below_water_cell_yields_water() {
        // terrain at z=-100, water at z=0 → water above floor → emit.
        let cells = (0..16)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .map(|(x, y)| cell(x, y, -100.0, 0.0))
            .collect();
        let mut w = WorldspaceInput::from_cells("W", cells);
        w.water_height = 0.0;
        let mesh = build_water_mesh(&w, &quad(16, 0, 0)).expect("water mesh expected");
        // 16x16 = 256 cells all under water → 256*2 = 512 tris.
        assert_eq!(mesh.tris.len(), 512, "got {} tris", mesh.tris.len());
        // The full sheet spans the quad (0..4096) in local space.
        assert!((mesh.bbox.min[0] - 0.0).abs() < 1e-3);
        assert!((mesh.bbox.max[0] - 4096.0).abs() < 1e-3);
        assert!((mesh.bbox.max[1] - 4096.0).abs() < 1e-3);
    }

    /// A land cell that rises above its water height yields NO water plane.
    #[test]
    fn land_above_water_yields_none() {
        // terrain at z=+500, water at z=0 → floor above water → no emit.
        let cells = (0..16)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .map(|(x, y)| cell(x, y, 500.0, 0.0))
            .collect();
        let mut w = WorldspaceInput::from_cells("W", cells);
        w.water_height = 0.0;
        assert!(
            build_water_mesh(&w, &quad(16, 0, 0)).is_none(),
            "land above water must not emit a water block"
        );
    }

    /// The sentinel "no water" worldspace (f32::MIN) over landless cells emits
    /// nothing (MIN > pz2 is false).
    #[test]
    fn sentinel_no_water_landless_yields_none() {
        // No cells at all → all landless; worldspace water = sentinel f32::MIN.
        let mut w = WorldspaceInput::from_cells("W", vec![cell(40, 40, 0.0, f32::MIN)]);
        w.sw_cell = (0, 0);
        w.ne_cell = (43, 43);
        w.water_height = f32::MIN;
        assert!(
            build_water_mesh(&w, &quad(16, 0, 0)).is_none(),
            "sentinel water must not emit"
        );
    }

    /// Landless cells (no LAND record) use the worldspace water over landHeight=0;
    /// a positive worldspace water height floods them.
    #[test]
    fn landless_cells_flood_at_worldspace_water() {
        // Land only on a faraway island; the quad at (0,0) is entirely landless.
        let island = (40..44)
            .flat_map(|y| (40..44).map(move |x| (x, y)))
            .map(|(x, y)| cell(x, y, 100.0, 50.0))
            .collect();
        let mut w = WorldspaceInput::from_cells("W", island);
        w.sw_cell = (0, 0);
        w.ne_cell = (43, 43);
        w.water_height = 50.0; // > landHeight(0) → landless cells flood.
        let mesh = build_water_mesh(&w, &quad(16, 0, 0)).expect("landless flood");
        assert_eq!(mesh.tris.len(), 16 * 16 * 2);
    }

    /// RemoveDuplicate welds shared corner verts: a 16×16 grid of touching cell
    /// quads has far fewer verts than 4 per cell (1024) — a 17×17 post lattice.
    #[test]
    fn remove_duplicate_welds_shared_corners() {
        let cells = (0..16)
            .flat_map(|y| (0..16).map(move |x| (x, y)))
            .map(|(x, y)| cell(x, y, -100.0, 0.0))
            .collect();
        let mut w = WorldspaceInput::from_cells("W", cells);
        w.water_height = 0.0;
        let mesh = build_water_mesh(&w, &quad(16, 0, 0)).unwrap();
        // A fully-flooded 16×16 quad welds to a 17×17 = 289 vertex lattice.
        assert_eq!(mesh.verts.len(), 17 * 17, "got {} verts", mesh.verts.len());
        // Every triangle index is in range.
        assert!(
            mesh.tris
                .iter()
                .flatten()
                .all(|&i| (i as usize) < mesh.verts.len())
        );
    }

    /// Water z is the water height divided by lodLevel (local block space).
    #[test]
    fn water_z_is_height_over_level() {
        let cells = vec![cell(0, 0, -100.0, 320.0)];
        let mut w = WorldspaceInput::from_cells("W", cells);
        w.sw_cell = (0, 0);
        w.ne_cell = (15, 15);
        w.water_height = 320.0;
        let mesh = build_water_mesh(&w, &quad(16, 0, 0)).unwrap();
        // z = 320 / 16 = 20.0 in local space.
        assert!(mesh.verts.iter().all(|v| (v[2] - 20.0).abs() < 1e-3));
    }
}
