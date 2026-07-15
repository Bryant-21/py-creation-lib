#![allow(dead_code)]
//! Shared test fixture builders for lodgen integration / golden tests.

/// Build a flat worldspace with all-zero heights covering a `cells_per_side × cells_per_side`
/// grid starting at cell (0, 0). Used by driver, golden, and generate_quad tests.
pub fn flat_world(editor_id: &str, cells_per_side: i32) -> lodgen_native::input::WorldspaceInput {
    let cells = (0..cells_per_side)
        .flat_map(|y| (0..cells_per_side).map(move |x| (x, y)))
        .map(|(x, y)| lodgen_native::input::CellInput {
            x,
            y,
            heights: vec![0.0f32; 33 * 33],
            vertex_colors: vec![[255u8, 255, 255]; 33 * 33],
            layers: Vec::new(),
            hidden_quadrants: [false; 4],
            water_height: f32::MIN,
        })
        .collect();
    lodgen_native::input::WorldspaceInput::from_cells(editor_id, cells)
}

/// Build a NON-FLAT worldspace: per-post height ramps from 0 at the SW corner
/// up to `z_max` at the NE corner across the whole `cells_per_side × cells_per_side`
/// grid. Exercises the FO4 ShiftZ / center.z-zeroing path that flat fixtures
/// (z range == 0) silently mask.
pub fn ramp_world(
    editor_id: &str,
    cells_per_side: i32,
    z_max: f32,
) -> lodgen_native::input::WorldspaceInput {
    let posts_per_side = (cells_per_side * 32) as f32; // 32 posts per cell + shared edges
    let cells = (0..cells_per_side)
        .flat_map(|y| (0..cells_per_side).map(move |x| (x, y)))
        .map(|(x, y)| {
            let mut heights = vec![0.0f32; 33 * 33];
            for k in 0..33usize {
                for l in 0..33usize {
                    // Global post index across the world (cells share edge posts).
                    let gx = (x * 32) as f32 + l as f32;
                    let gy = (y * 32) as f32 + k as f32;
                    let t = (gx + gy) / (2.0 * posts_per_side);
                    heights[l + k * 33] = t * z_max;
                }
            }
            lodgen_native::input::CellInput {
                x,
                y,
                heights,
                vertex_colors: vec![[255u8, 255, 255]; 33 * 33],
                layers: Vec::new(),
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            }
        })
        .collect();
    lodgen_native::input::WorldspaceInput::from_cells(editor_id, cells)
}
