//! Scratch (not committed): objects listed by leaf tiles inside each inner
//! tile's box but by none of the inner tile's own cells. Args: <file.uvd> [watch id hex]
use std::collections::HashSet;

use previs_native::umbra_query::cell_objects;
use previs_native::umbra_tome::parse;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let tome = parse(&std::fs::read(&a[0]).unwrap()).unwrap();
    let watch = a.get(1).map(|h| u32::from_str_radix(h, 16).unwrap());
    let ids = tome.user_ids.clone().unwrap_or_default();
    let lists = |t: &previs_native::umbra_tome::Tile| -> HashSet<u32> {
        let mut out = Vec::new();
        for c in 0..t.num_cells() as u32 {
            cell_objects(&tome, t, c, &mut out);
        }
        out.into_iter().collect()
    };
    let inside = |lo: [f32; 3], hi: [f32; 3], t: &previs_native::umbra_tome::Tile| (0..3).all(|k| t.tree_min[k] >= lo[k] - 0.5 && t.tree_max[k] <= hi[k] + 0.5);
    let (mut inner, mut short, mut missing_total) = (0, 0, 0);
    for t in tome.tiles.iter().flatten().filter(|t| !t.is_leaf()) {
        inner += 1;
        let own = lists(t);
        let mut below = HashSet::new();
        for l in tome.tiles.iter().flatten().filter(|l| l.is_leaf() && inside(t.tree_min, t.tree_max, l)) {
            below.extend(lists(l));
        }
        let missing: Vec<u32> = below.difference(&own).copied().collect();
        if !missing.is_empty() {
            short += 1;
            missing_total += missing.len();
        }
        if let Some(w) = watch {
            if below.iter().any(|&o| ids.get(o as usize) == Some(&w)) {
                println!("inner {:?}..{:?} lists {:08X}: {}", t.tree_min, t.tree_max, w, own.iter().any(|&o| ids.get(o as usize) == Some(&w)));
            }
        }
    }
    println!("inner tiles {inner}, missing some leaf-listed objects: {short}, total missing {missing_total}");
}
