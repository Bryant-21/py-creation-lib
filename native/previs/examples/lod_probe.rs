//! Scratch (not committed): leaf vs non-leaf tile summary of a tome.
//! Args: <file.uvd>...
use previs_native::umbra_tome::parse;

fn main() {
    for path in std::env::args().skip(1) {
        let tome = parse(&std::fs::read(&path).unwrap()).unwrap();
        let (mut leaf, mut inner, mut leaf_cells, mut inner_cells, mut empty) = (0, 0, 0, 0, 0);
        for t in &tome.tiles {
            match t {
                None => empty += 1,
                Some(t) if t.is_leaf() => { leaf += 1; leaf_cells += t.num_cells(); }
                Some(t) => { inner += 1; inner_cells += t.num_cells(); }
            }
        }
        let lods: Vec<f32> = tome.tile_lod_levels.iter().flatten().map(|&b| f32::from_bits(b)).filter(|v| *v >= 0.0).collect();
        let (lmin, lmax) = lods.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        println!("{path}: lod_base {} flags {:#x} objects {} tiles leaf {leaf} ({leaf_cells} cells) inner {inner} ({inner_cells} cells) empty {empty} lod {lmin:.2}..{lmax:.2}", tome.lod_base_distance, tome.flags, tome.num_objects);
    }
}
