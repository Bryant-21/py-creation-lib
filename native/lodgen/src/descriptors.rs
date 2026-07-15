/// Axis-aligned bounding box (ported from LODGenerator/BBox.cs).
#[derive(Clone, Debug)]
pub struct BBox {
    /// Minimum corner (px1, py1, pz1 in BBox.cs)
    pub min: [f32; 3],
    /// Maximum corner (px2, py2, pz2 in BBox.cs)
    pub max: [f32; 3],
}

impl BBox {
    /// Empty bbox: min = +inf, max = -inf (BBox.cs:20-28)
    pub fn empty() -> Self {
        BBox {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        }
    }

    /// Expand to include vertex `v` (BBox.cs:30-40)
    pub fn grow_vertex(&mut self, v: [f32; 3]) {
        for i in 0..3 {
            if v[i] < self.min[i] {
                self.min[i] = v[i];
            }
            if v[i] > self.max[i] {
                self.max[i] = v[i];
            }
        }
    }

    /// Expand to include another BBox (BBox.cs:42-52)
    pub fn grow_box(&mut self, o: &BBox) {
        self.grow_vertex(o.min);
        self.grow_vertex(o.max);
    }

    /// Center of the bbox (BBox.cs:55-63).
    /// If `zero_clamp` is true and max.z < 0, the top is clamped to 0
    /// before averaging (used for water-submerged terrain blocks).
    pub fn center(&self, zero_clamp: bool) -> [f32; 3] {
        let pz2 = if zero_clamp && self.max[2] < 0.0 {
            0.0
        } else {
            self.max[2]
        };
        [
            (self.min[0] + self.max[0]) / 2.0,
            (self.min[1] + self.max[1]) / 2.0,
            (self.min[2] + pz2) / 2.0,
        ]
    }

    /// Extent (half-size) of the bbox (BBox.cs:65-73).
    /// Applies the same zero_clamp rule on the Z axis.
    pub fn extent(&self, zero_clamp: bool) -> [f32; 3] {
        let pz2 = if zero_clamp && self.max[2] < 0.0 {
            0.0
        } else {
            self.max[2]
        };
        [
            (self.max[0] - self.min[0]) / 2.0,
            (self.max[1] - self.min[1]) / 2.0,
            (pz2 - self.min[2]) / 2.0,
        ]
    }

    /// Distance from center to max corner (BBox.cs:75-78)
    pub fn radius(&self) -> f32 {
        let c = self.center(false);
        let dx = self.max[0] - c[0];
        let dy = self.max[1] - c[1];
        let dz = self.max[2] - c[2];
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

/// Per-quad triangle reduction counters (OutDesc.cs)
#[derive(Clone, Debug, Default)]
pub struct OutDesc {
    pub total_tri_count: u32,
    pub reduced_tri_count: u32,
}

/// Per-quad LOD block descriptor (ported from LODGenerator/QuadDesc.cs).
/// One QuadDesc per (level, x, y) block.
#[derive(Clone, Debug)]
pub struct QuadDesc {
    pub z_order: u32,
    pub x: i32,
    pub y: i32,
    pub quad_level: i32,
    pub quad_index: i32,
    /// Default 16384.0 (QuadDesc.cs:38)
    pub quad_offset: f32,
    pub static_indices: Vec<usize>,
    pub statics: Vec<crate::input::StaticDesc>,
    pub out_values: OutDesc,
}

impl QuadDesc {
    pub fn static_refs<'a>(
        &'a self,
        world: &'a crate::input::WorldspaceInput,
    ) -> impl Iterator<Item = &'a crate::input::StaticDesc> + 'a {
        self.static_indices
            .iter()
            .filter_map(move |&idx| world.refs.get(idx))
            .chain(self.statics.iter())
    }
}

/// Build the list of LOD blocks for a given level and worldspace.
///
/// Ported from `TerrainLOD.GenerateLOD` block-origin loop (R1 §2/§10,
/// `TerrainLOD.cs:1756-1801`). Steps `level` across the worldspace bbox.
/// If `settings.global.chunk` is Some and its `level` matches, keeps only
/// blocks within `[w..=e] x [s..=n]`.
pub fn quads_for(
    world: &crate::input::WorldspaceInput,
    level: i32,
    settings: &crate::settings::LodSettings,
) -> Vec<QuadDesc> {
    let sw = world.sw_cell;
    let ne = world.ne_cell;

    // chunk filter (applied only when chunk.level matches)
    let chunk_filter = settings.global.chunk.as_ref().filter(|c| c.level == level);

    let mut quads = Vec::new();
    let mut quad_index: i32 = 0;

    let mut y = sw.1;
    while y <= ne.1 {
        let mut x = sw.0;
        while x <= ne.0 {
            // Apply chunk filter if active
            let include = chunk_filter
                .map(|c| x >= c.w && x <= c.e && y >= c.s && y <= c.n)
                .unwrap_or(true);

            if include {
                quads.push(QuadDesc {
                    z_order: 0,
                    x,
                    y,
                    quad_level: level,
                    quad_index,
                    quad_offset: 16384.0,
                    static_indices: Vec::new(),
                    statics: Vec::new(),
                    out_values: OutDesc::default(),
                });
                quad_index += 1;
            }
            x += level;
        }
        y += level;
    }

    quads
}

pub(crate) fn aligned_sw_cell(sw: (i32, i32), align: i32) -> (i32, i32) {
    if align <= 0 {
        return sw;
    }
    let snap = |v: i32| v.div_euclid(align) * align;
    (snap(sw.0), snap(sw.1))
}

/// Build the list of TERRAIN LOD quads for a given level.
///
/// Unlike `quads_for` (which steps the full declared SW..NE box and is used by the
/// object/tree paths), terrain emission is bounded by the land-cell extent
/// (`bbWorld`) and grid-anchored to the SW corner — a faithful port of the
/// `GenerateLOD` terrain loop (`TerrainLOD.cs:1756-1758`):
///
/// ```text
/// for j = southWestY + (bbWorld.py1 - southWestY)/level*level; j <= bbWorld.py2; j += level
///   for k = southWestX + (bbWorld.px1 - southWestX)/level*level; k <= bbWorld.px2; k += level
/// ```
///
/// Quads entirely outside `bbWorld` are never emitted (gap 4: stops the
/// ~1432-extra-btr over-production). In-bounds quads whose cells lack a LAND
/// record are still emitted — `build_terrain_mesh` synthesizes their terrain
/// (gap 3). The `chunk` filter is honored identically to `quads_for`.
pub fn terrain_quads_for(
    world: &crate::input::WorldspaceInput,
    level: i32,
    settings: &crate::settings::LodSettings,
) -> Vec<QuadDesc> {
    // No land → no terrain quads (xLODGen writes nothing when bbWorld is empty).
    let Some((mut land_min_x, mut land_min_y, mut land_max_x, mut land_max_y)) =
        world.land_cell_bounds()
    else {
        return Vec::new();
    };

    if let Some(bounds) = settings.global.bounds {
        land_min_x = land_min_x.max(bounds.w);
        land_min_y = land_min_y.max(bounds.s);
        land_max_x = land_max_x.min(bounds.e);
        land_max_y = land_max_y.min(bounds.n);
        if land_min_x > land_max_x || land_min_y > land_max_y {
            return Vec::new();
        }
    }

    let configured_sw = settings
        .global
        .southwest_cell
        .map(|[x, y]| (x, y))
        .or_else(|| settings.global.bounds.map(|bounds| (bounds.w, bounds.s)))
        .unwrap_or(world.sw_cell);
    let (sw_x, sw_y) = aligned_sw_cell(configured_sw, settings.global.align);

    // Grid anchor: southWest + (bbWorld_min - southWest)/level*level. C# integer
    // division truncates toward zero; replicate with i32 division (not floor) so
    // the anchor matches xLODGen exactly even for negative offsets.
    let anchor = |sw: i32, land_min: i32| sw + (land_min - sw) / level * level;
    let start_x = anchor(sw_x, land_min_x);
    let start_y = anchor(sw_y, land_min_y);

    let chunk_filter = settings.global.chunk.as_ref().filter(|c| c.level == level);

    let mut quads = Vec::new();
    let mut quad_index: i32 = 0;

    let mut y = start_y;
    while y <= land_max_y {
        let mut x = start_x;
        while x <= land_max_x {
            let include = chunk_filter
                .map(|c| x >= c.w && x <= c.e && y >= c.s && y <= c.n)
                .unwrap_or(true);
            if include {
                quads.push(QuadDesc {
                    z_order: 0,
                    x,
                    y,
                    quad_level: level,
                    quad_index,
                    quad_offset: 16384.0,
                    static_indices: Vec::new(),
                    statics: Vec::new(),
                    out_values: OutDesc::default(),
                });
                quad_index += 1;
            }
            x += level;
        }
        y += level;
    }

    quads
}

/// Per-terrain-block descriptor (TerrainDesc.cs + TerrainData.GenerateTerrainDesc).
#[derive(Clone, Debug)]
pub struct TerrainDesc {
    pub index: i32,
    pub x: i32,
    pub y: i32,
    pub quad_level: i32,
    pub water_height: f32,
    pub land_flags: i32,
    pub bbox: BBox,
    /// 33*33 height posts, row-major `[col + row*33]`
    pub height_values: Vec<f32>,
}

impl TerrainDesc {
    /// True if the surface point `z` is beneath the water level (TerrainDesc.cs:60-68).
    /// Returns false for NaN or sentinel water (>= 16_777_216).
    pub fn is_under_water(&self, z: f32) -> bool {
        !self.water_height.is_nan() && self.water_height < 16_777_216.0 && self.water_height > z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_grow_and_center() {
        let mut b = BBox::empty();
        b.grow_vertex([0.0, 0.0, 2.0]);
        b.grow_vertex([4.0, 6.0, -2.0]);
        assert_eq!(b.min, [0.0, 0.0, -2.0]);
        assert_eq!(b.max, [4.0, 6.0, 2.0]);
        // center, no zero clamp
        assert_eq!(b.center(false), [2.0, 3.0, 0.0]);
    }

    #[test]
    fn bbox_center_zero_clamp() {
        // BBox.cs:55-63: if zero && pz2 < 0 => use 0 for top
        let mut b = BBox::empty();
        b.grow_vertex([0.0, 0.0, -10.0]);
        b.grow_vertex([2.0, 2.0, -4.0]);
        // pz2 = -4 < 0 => top clamped to 0; center.z = (pz1 + 0)/2 = -5
        assert_eq!(b.center(true)[2], -5.0);
    }

    #[test]
    fn terrain_desc_is_under_water() {
        let td = TerrainDesc {
            index: -1,
            x: 0,
            y: 0,
            quad_level: 4,
            water_height: 100.0,
            land_flags: 0,
            bbox: BBox::empty(),
            height_values: vec![0.0; 33 * 33],
        };
        assert!(td.is_under_water(50.0)); // 100 > 50
        assert!(!td.is_under_water(150.0)); // 100 < 150
        // NaN / sentinel water => never underwater (TerrainDesc.cs:62-63)
        let td2 = TerrainDesc {
            water_height: f32::NAN,
            ..td
        };
        assert!(!td2.is_under_water(50.0));
    }

    fn world_8x8() -> crate::input::WorldspaceInput {
        let cells = (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .map(|(x, y)| crate::input::CellInput {
                x,
                y,
                heights: vec![0.0; 33 * 33],
                vertex_colors: Vec::new(),
                layers: Vec::new(),
                hidden_quadrants: [false; 4],
                water_height: 0.0,
            })
            .collect();
        crate::input::WorldspaceInput::from_cells("W", cells)
    }

    #[test]
    fn quads_for_level4_8x8() {
        let s = crate::settings::LodSettings::fo4_default();
        let quads = quads_for(&world_8x8(), 4, &s);
        let mut origins: Vec<(i32, i32)> = quads.iter().map(|q| (q.x, q.y)).collect();
        origins.sort();
        assert_eq!(origins, vec![(0, 0), (0, 4), (4, 0), (4, 4)]);
        assert!(quads.iter().all(|q| q.quad_level == 4));
        assert_eq!(quads[0].quad_offset, 16384.0);
    }

    #[test]
    fn quads_for_chunk_filter() {
        let mut s = crate::settings::LodSettings::fo4_default();
        s.global.chunk = Some(crate::settings::ChunkBounds {
            level: 4,
            w: 0,
            s: 0,
            e: 3,
            n: 3,
        });
        let quads = quads_for(&world_8x8(), 4, &s);
        // chunk bounds restrict to the single (0,0) block (covers cells 0..3)
        assert_eq!(quads.len(), 1);
        assert_eq!((quads[0].x, quads[0].y), (0, 0));
    }

    /// A worldspace whose DECLARED bounds extend far past the cells that carry
    /// LAND. `terrain_quads_for` must enumerate only quads that overlap the
    /// land-cell extent (xLODGen's `bbWorld` loop, TerrainLOD.cs:1756-1758),
    /// NOT the full declared SW..NE box. This is the over-production fix (gap 4).
    fn world_land_island() -> crate::input::WorldspaceInput {
        // LAND only in cells x∈[2,5], y∈[2,5]; declared bounds widened to [-4,11].
        let cells: Vec<_> = (2..6)
            .flat_map(|y| (2..6).map(move |x| (x, y)))
            .map(|(x, y)| crate::input::CellInput {
                x,
                y,
                heights: vec![0.0; 33 * 33],
                vertex_colors: Vec::new(),
                layers: Vec::new(),
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            })
            .collect();
        let mut w = crate::input::WorldspaceInput::from_cells("W", cells);
        w.sw_cell = (-4, -4);
        w.ne_cell = (11, 11);
        w
    }

    #[test]
    fn terrain_quads_for_skips_out_of_bounds_landless() {
        let w = world_land_island();
        let s = crate::settings::LodSettings::fo4_default();

        // Naive declared-bounds enumeration would emit far more quads.
        let declared = quads_for(&w, 4, &s);

        // L4: land cells x∈[2,5], y∈[2,5]. Grid anchored to sw=-4:
        // start = -4 + floor((2 - -4)/4)*4 = -4 + 1*4 = 0; quads at x∈{0,4}, y∈{0,4}
        // (4 overlaps cells 4..7 which includes 4,5 land). End at bbWorld_max=5.
        let terr = terrain_quads_for(&w, 4, &s);
        let mut origins: Vec<(i32, i32)> = terr.iter().map(|q| (q.x, q.y)).collect();
        origins.sort();
        assert_eq!(
            origins,
            vec![(0, 0), (0, 4), (4, 0), (4, 4)],
            "terrain emission must cover only land-extent quads"
        );
        assert!(
            terr.len() < declared.len(),
            "terrain emission ({}) must be fewer than declared-bounds ({})",
            terr.len(),
            declared.len()
        );
    }

    /// The grid anchor (start of the loop) is snapped to the SW corner, matching
    /// `southWest + (bbWorld_min - southWest) / level * level` (TerrainLOD.cs:1756).
    #[test]
    fn terrain_quads_for_anchors_grid_to_sw() {
        let w = world_land_island(); // land x∈[2,5], sw=-4
        let s = crate::settings::LodSettings::fo4_default();
        // L8: start = -4 + floor((2 - -4)/8)*8 = -4 + 0 = -4; end at 5 → only x=-4 quad
        // (covers cells -4..3 which includes land 2,3; next quad x=4 covers 4..11
        // which includes land 4,5). So x∈{-4,4}, y∈{-4,4}.
        let terr = terrain_quads_for(&w, 8, &s);
        let mut origins: Vec<(i32, i32)> = terr.iter().map(|q| (q.x, q.y)).collect();
        origins.sort();
        assert_eq!(origins, vec![(-4, -4), (-4, 4), (4, -4), (4, 4)]);
    }

    #[test]
    fn terrain_quads_for_respects_global_align() {
        let mut w = world_8x8();
        w.sw_cell = (2, 3);
        w.ne_cell = (9, 10);
        for cell in &mut w.cells {
            cell.x += 2;
            cell.y += 3;
        }

        let mut s = crate::settings::LodSettings::fo4_default();
        s.global.align = 4;
        let aligned = terrain_quads_for(&w, 4, &s);
        assert_eq!((aligned[0].x, aligned[0].y), (0, 0));

        s.global.align = 0;
        let unaligned = terrain_quads_for(&w, 4, &s);
        assert_eq!((unaligned[0].x, unaligned[0].y), (2, 3));
    }

    #[test]
    fn terrain_quads_for_respects_explicit_southwest_cell_and_bounds() {
        let w = world_8x8();
        let mut s = crate::settings::LodSettings::fo4_default();
        s.global.southwest_cell = Some([2, 3]);
        s.global.bounds = Some(crate::settings::LodBounds {
            w: 2,
            s: 3,
            e: 5,
            n: 6,
        });

        let terr = terrain_quads_for(&w, 4, &s);
        let origins: Vec<(i32, i32)> = terr.iter().map(|q| (q.x, q.y)).collect();
        assert_eq!(origins, vec![(2, 3)]);
    }

    /// A fully-populated world (land == declared bounds) enumerates identically
    /// under both functions — the new rule is a strict superset guard, not a change
    /// for the common "every cell has LAND" case.
    #[test]
    fn terrain_quads_for_full_world_matches_quads_for() {
        let w = world_8x8(); // land fills 0..7, declared = land
        let s = crate::settings::LodSettings::fo4_default();
        for level in [4, 8, 16, 32] {
            let mut a: Vec<(i32, i32)> = quads_for(&w, level, &s)
                .iter()
                .map(|q| (q.x, q.y))
                .collect();
            let mut b: Vec<(i32, i32)> = terrain_quads_for(&w, level, &s)
                .iter()
                .map(|q| (q.x, q.y))
                .collect();
            a.sort();
            b.sort();
            assert_eq!(a, b, "level {level}: full world must match");
        }
    }
}
