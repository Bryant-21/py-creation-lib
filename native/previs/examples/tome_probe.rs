//! Samples a tome on a grid: which points land in a view cell, whether that
//! cell's box contains them (the runtime starts its portal walk there), and
//! how many objects each cell reaches with gates open.
//!
//! `cargo run --release --example tome_probe -- <file.uvd> [step] [x y z]...`
//! Explicit points print their own hit and reachable-object count.
//! `<file.uvd> cell <tile> <cell>` prints one cell's box and portals;
//! `<file.uvd> tiles` tallies leaf-tile sizes; `<file.uvd> objects` lists objects
//! no cell the sky reaches lists. `NO_REACH=1` skips reachability.

use previs_native::umbra_query::{CellHit, KdView, cell_bounds, cell_objects, find_cell, portal_quad, read_bits, visible_objects};
use previs_native::umbra_tome::{Tome, parse};

/// Samples every outside portal of every tome-boundary face on a `step` grid
/// and tallies what the face's matching tree names there: the portal's own
/// cell, another cell, or no cell (a hole the runtime stitch cannot cross).
fn border_coverage(tome: &Tome, step: f32) {
    let (min, max) = (tome.tree_min, tome.tree_max);
    let leaves = tome.tiles.iter().enumerate().filter_map(|(i, t)| t.as_ref().filter(|t| t.is_leaf()).map(|t| (i, t)));
    let (mut own, mut other, mut hole, mut samples) = (0usize, 0usize, 0usize, 0usize);
    let mut shown = 0;
    for ((t, tile), m) in leaves.zip(&tome.leaf_matches) {
        let faces: Vec<u32> =
            (0..6u32).filter(|&f| if f & 1 == 0 { tile.tree_min[(f >> 1) as usize] == min[(f >> 1) as usize] } else { tile.tree_max[(f >> 1) as usize] == max[(f >> 1) as usize] }).collect();
        for (k, &face) in faces.iter().enumerate() {
            let tree = &tome.matching_trees[m.first_tree() as usize + k];
            let mut named = Vec::new();
            if tree.data.is_some() {
                let view = KdView::new(tree).unwrap();
                let empty = (1u64 << tree.map_width) as u32 - 1;
                view.walk(tile.tree_min, tile.tree_max, |node, lo, hi, _| {
                    if view.is_leaf(node) {
                        let v = read_bits(tree.map.as_deref().unwrap_or(&[]), (view.leaf_index(node) * tree.map_width) as usize, tree.map_width);
                        named.push(((v != empty).then_some(v), *lo, *hi));
                    }
                })
                .unwrap();
            }
            let axis = (face >> 1) as usize;
            let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
            for (c, node) in tile.cell_nodes.iter().flatten().enumerate() {
                let portals = &tile.portals.as_deref().unwrap_or(&[])[node.portal_index as usize..(node.portal_index + node.portal_count) as usize];
                for p in portals.iter().filter(|p| p.is_outside() && p.face() == face) {
                    let (_, lo, hi) = portal_quad(&tile.tree_min, &tile.tree_max, p);
                    let mut u = lo[0] + step * 0.5;
                    while u < hi[0] {
                        let mut v = lo[1] + step * 0.5;
                        while v < hi[1] {
                            samples += 1;
                            let leaf = named.iter().find(|(_, l, h)| l[a] <= u && u < h[a] && l[b] <= v && v < h[b]);
                            match leaf.and_then(|l| l.0) {
                                Some(n) if n == c as u32 => own += 1,
                                Some(_) => other += 1,
                                None => {
                                    hole += 1;
                                    if shown < 5 {
                                        shown += 1;
                                        println!("  hole: tile {t} face {face} cell {c} at ({u}, {v}) in quad {lo:?}..{hi:?}");
                                    }
                                }
                            }
                            v += step;
                        }
                        u += step;
                    }
                }
            }
        }
    }
    println!("outside-portal samples {samples}: tree names own cell {own}, another cell {other}, no cell {hole}");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = std::fs::read(&args[0]).unwrap();
    let tome = parse(&data).unwrap();
    if args.get(1).map(String::as_str) == Some("tiles") {
        let mut sizes = std::collections::BTreeMap::new();
        for tile in tome.tiles.iter().flatten().filter(|t| t.is_leaf() && t.num_cells() > 1) {
            let size: Vec<i32> = (0..3).map(|k| (tile.tree_max[k] - tile.tree_min[k]).round() as i32).collect();
            let has_tree = tile.tree.data.is_some();
            *sizes.entry((size, has_tree)).or_insert(0usize) += 1;
        }
        for ((size, has_tree), count) in sizes {
            let dyadic = size.iter().all(|s| (*s as u32).is_power_of_two());
            println!("{size:?} tree={has_tree} power-of-two={dyadic}: {count}");
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("validate") {
        println!("{:?}", previs_native::umbra_validate::validate(&tome));
        return;
    }
    if args.get(1).map(String::as_str) == Some("border") {
        border_coverage(&tome, args.get(2).and_then(|s| s.parse().ok()).unwrap_or(16.0));
        return;
    }
    if args.get(1).map(String::as_str) == Some("near") {
        let at: Vec<f32> = args[2..5].iter().map(|s| s.parse().unwrap()).collect();
        let ids = tome.user_ids.as_deref().unwrap_or(&[]);
        let bounds = tome.object_bounds.as_deref().unwrap_or(&[]);
        let distances = tome.object_distances.as_deref().unwrap_or(&[]);
        let mut listed = vec![0usize; ids.len()];
        let mut objects = Vec::new();
        for tile in tome.tiles.iter().flatten().filter(|t| t.is_leaf()) {
            for cell in 0..tile.num_cells() as u32 {
                objects.clear();
                cell_objects(&tome, tile, cell, &mut objects);
                objects.iter().for_each(|&o| listed[o as usize] += 1);
            }
        }
        for (o, b) in bounds.iter().enumerate().filter(|(_, b)| (0..3).all(|k| b[k] - 1.0 <= at[k] && at[k] <= b[k + 3] + 1.0)) {
            println!("object {o} id {:08X} bounds {b:?} distance {:?} in {} leaf cells", ids.get(o).copied().unwrap_or(0), distances.get(o), listed[o]);
        }
        return;
    }
    if args.get(1).map(String::as_str) == Some("cell") {
        let (t, c): (usize, u32) = (args[2].parse().unwrap(), args[3].parse().unwrap());
        let tile = tome.tiles[t].as_ref().unwrap();
        println!("tile {t}: {:?} .. {:?} leaf={} cells={}", tile.tree_min, tile.tree_max, tile.is_leaf(), tile.num_cells());
        let node = &tile.cell_nodes.as_ref().unwrap()[c as usize];
        let mut objects = Vec::new();
        cell_objects(&tome, tile, c, &mut objects);
        println!("cell {c}: bounds {:?} objects {} portals {}", cell_bounds(tile, c), objects.len(), node.portal_count);
        let portals = tile.portals.as_deref().unwrap_or(&[]);
        for p in &portals[node.portal_index as usize..(node.portal_index + node.portal_count) as usize] {
            let kind = match (p.is_outside(), p.is_user(), p.is_hierarchy()) {
                (true, _, _) => "outside",
                (_, true, _) => "user",
                (_, _, true) => "hierarchy",
                _ => "regular",
            };
            let quad = portal_quad(&tile.tree_min, &tile.tree_max, p);
            let target = tome.tiles.get(p.target() as usize).and_then(Option::as_ref);
            let target_bounds = target.and_then(|tt| cell_bounds(tt, p.target_index as u32));
            println!(
                "  {kind:9} face {} -> tile {} cell {} quad plane={:.1} {:?}..{:?} target cell {:?}",
                p.face(),
                p.target(),
                p.target_index,
                quad.0,
                quad.1,
                quad.2,
                target_bounds
            );
        }
        return;
    }
    let step: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(128.0);
    let points: Vec<[f32; 3]> = args[2.min(args.len())..]
        .chunks(3)
        .filter(|c| c.len() == 3)
        .map(|c| [c[0].parse().unwrap(), c[1].parse().unwrap(), c[2].parse().unwrap()])
        .collect();
    if !points.is_empty() {
        for p in points {
            let hit = find_cell(&tome, p).unwrap();
            let visible = visible_objects(&tome, p).unwrap().map(|v| v.len());
            println!("{p:?}: {hit:?} reachable={visible:?}");
        }
        return;
    }

    let (min, max) = (tome.tree_min, tome.tree_max);
    if args.get(1).map(String::as_str) != Some("objects") {
        println!("bounds {min:?} .. {max:?}, {} objects, step {step}", tome.num_objects);
    }
    // Leaf cells a walk over regular portals reaches from those open to the sky.
    let mut sky = std::collections::HashSet::new();
    let mut stack: Vec<(usize, u32)> = Vec::new();
    for (t, tile) in tome.tiles.iter().enumerate() {
        let Some(tile) = tile.as_ref().filter(|t| t.is_leaf()) else { continue };
        for (c, node) in tile.cell_nodes.iter().flatten().enumerate() {
            let portals = &tile.portals.as_deref().unwrap_or(&[])[node.portal_index as usize..(node.portal_index + node.portal_count) as usize];
            if portals.iter().any(|p| p.is_outside() && p.face() == 5) {
                stack.push((t, c as u32));
            }
        }
    }
    while let Some((t, c)) = stack.pop() {
        if !sky.insert((t, c)) {
            continue;
        }
        let tile = tome.tiles[t].as_ref().unwrap();
        let node = &tile.cell_nodes.as_ref().unwrap()[c as usize];
        for p in &tile.portals.as_deref().unwrap_or(&[])[node.portal_index as usize..(node.portal_index + node.portal_count) as usize] {
            if !p.is_outside() && !p.is_hierarchy() && !p.is_user() {
                stack.push((p.target() as usize, p.target_index as u32));
            }
        }
    }
    if args.get(1).map(String::as_str) == Some("objects") {
        let mut listed = vec![false; tome.num_objects as usize];
        let mut objects = Vec::new();
        for &(t, c) in &sky {
            objects.clear();
            cell_objects(&tome, tome.tiles[t].as_ref().unwrap(), c, &mut objects);
            for &o in &objects {
                listed[o as usize] = true;
            }
        }
        let ids = tome.user_ids.as_deref().unwrap_or(&[]);
        let bounds = tome.object_bounds.as_deref().unwrap_or(&[]);
        for o in (0..listed.len()).filter(|&o| !listed[o]) {
            println!("object {o} id {:08X} bounds {:?} in no cell the sky reaches", ids.get(o).copied().unwrap_or(0), bounds.get(o));
        }
        return;
    }
    let mut unreached = 0usize;
    let (mut outside, mut empty, mut cells) = (0usize, 0usize, 0usize);
    let mut outside_own_cell = 0usize;
    let mut empty_near_open = 0usize;
    // Cells without a single portal: a camera starting there sees nothing.
    let mut dead = 0usize;
    let skip_reach = std::env::var_os("NO_REACH").is_some();
    let mut miss_by_distance = [0usize; 5];
    let mut counts = Vec::new();
    let mut cache = std::collections::HashMap::new();
    let mut z = min[2] + step / 2.0;
    while z < max[2] {
        let mut y = min[1] + step / 2.0;
        while y < max[1] {
            let mut x = min[0] + step / 2.0;
            while x < max[0] {
                match find_cell(&tome, [x, y, z]).unwrap() {
                    CellHit::Outside => outside += 1,
                    CellHit::Empty { .. } => {
                        empty += 1;
                        // A camera can stand there if open space is within 24 units.
                        let near = [[24.0, 0.0, 0.0], [-24.0, 0.0, 0.0], [0.0, 24.0, 0.0], [0.0, -24.0, 0.0], [0.0, 0.0, 24.0], [0.0, 0.0, -24.0]]
                            .iter()
                            .any(|d| matches!(find_cell(&tome, [x + d[0], y + d[1], z + d[2]]), Ok(CellHit::Cell { .. })));
                        if near {
                            empty_near_open += 1;
                            if empty_near_open <= 5 {
                                println!("  empty beside open space at {:?}", [x, y, z]);
                            }
                        }
                    }
                    CellHit::Cell { tile, cell } => {
                        cells += 1;
                        let t = tome.tiles[tile as usize].as_ref().unwrap();
                        if t.cell_nodes.as_ref().is_some_and(|nodes| nodes[cell as usize].portal_count == 0) {
                            dead += 1;
                        }
                        if !sky.contains(&(tile as usize, cell)) {
                            unreached += 1;
                        }
                        if let Some((lo, hi)) = cell_bounds(t, cell)
                            && (0..3).any(|k| [x, y, z][k] < lo[k] - 1.0 || [x, y, z][k] > hi[k] + 1.0)
                        {
                            outside_own_cell += 1;
                            let p = [x, y, z];
                            let off = (0..3).map(|k| (lo[k] - p[k]).max(p[k] - hi[k]).max(0.0)).fold(0.0f32, f32::max);
                            let bucket = [16.0, 32.0, 64.0, 128.0].iter().position(|&b| off <= b).unwrap_or(4);
                            miss_by_distance[bucket] += 1;
                            if outside_own_cell <= 3 {
                                println!("  point {:?} -> tile {tile} cell {cell} whose box is {lo:?}..{hi:?}", [x, y, z]);
                            }
                        }
                        let n = *cache
                            .entry((tile, cell))
                            .or_insert_with(|| {
                                if skip_reach {
                                    return 0;
                                }
                                visible_objects(&tome, [x, y, z]).unwrap().map_or(0, |v| v.len())
                            });
                        counts.push((n, [x, y, z]));
                    }
                }
                x += step;
            }
            y += step;
        }
        z += step;
    }
    println!(
        "samples: {cells} in a cell ({dead} in one without portals, {unreached} the sky does not reach), {empty} empty/solid ({empty_near_open} beside open space), {outside} outside the tree"
    );
    println!("distinct cells sampled: {}", cache.len());
    println!("samples outside their own cell's box: {outside_own_cell} of {cells}");
    println!("  by distance outside it (<=16, <=32, <=64, <=128, more): {miss_by_distance:?}");
    counts.sort_by_key(|c| c.0);
    let pct = |p: f64| counts.get(((counts.len() as f64 - 1.0) * p) as usize).map_or(0, |c| c.0);
    println!(
        "reachable objects per sample: min {} p10 {} p50 {} p90 {} max {}",
        pct(0.0),
        pct(0.1),
        pct(0.5),
        pct(0.9),
        pct(1.0)
    );
    let mut sizes: Vec<usize> = cache.values().copied().collect();
    sizes.sort_unstable();
    sizes.dedup();
    println!("distinct reachable-set sizes (smallest 20): {:?}", &sizes[..sizes.len().min(20)]);
    let full = counts.last().map_or(0, |c| c.0);
    let isolated: Vec<&(usize, [f32; 3])> = counts.iter().filter(|c| c.0 * 10 < full * 9).collect();
    println!("samples reaching <90% of the max: {} of {}", isolated.len(), counts.len());
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for (_, p) in &isolated {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    println!("isolated samples span {lo:?} .. {hi:?}");
    for (n, p) in isolated.iter().step_by((isolated.len() / 12).max(1)) {
        println!("  isolated: {n} at {p:?}");
    }
}
