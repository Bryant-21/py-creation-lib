//! Dev harness: summarizes a tome's object table (user-ID kinds, distance
//! records) so two builds can be compared. Args: <a.uvd>...
//! `IDS=1` also prints every non-0xFD user ID.
use std::collections::BTreeMap;

use previs_native::umbra_tome::parse;

fn main() {
    for path in std::env::args().skip(1) {
        let tome = parse(&std::fs::read(&path).unwrap()).unwrap();
        let ids = tome.user_ids.clone().unwrap_or_default();
        let distances = tome.object_distances.clone().unwrap_or_default();
        let bounds = tome.object_bounds.clone().unwrap_or_default();
        let mut kinds: BTreeMap<String, u32> = BTreeMap::new();
        for (k, id) in ids.iter().enumerate() {
            let kind = if id >> 24 == 0xFD { format!("FD cell ({},{})", ((id >> 19) & 0x1F) as i32, ((id >> 14) & 0x1F) as i32) } else { "ref".into() };
            let d = distances.get(k).map(|d| d.map(f32::from_bits));
            let far = match d {
                Some(d) if d[7] == f32::MAX => "max=MAX".to_string(),
                Some(d) => format!("max={} min={}", d[7], d[3]),
                None => "no distance".into(),
            };
            let matches = d.zip(bounds.get(k)).is_some_and(|(d, b)| [d[0], d[1], d[2], d[4], d[5], d[6]] == *b);
            *kinds.entry(format!("{kind} {far} bounds_match={matches}")).or_default() += 1;
        }
        println!("{path}: {} objects, {} distances, {} bounds", ids.len(), distances.len(), bounds.len());
        for (k, v) in kinds {
            println!("  {v:6} {k}");
        }
        if std::env::var_os("STRUCT").is_some() {
            let tiles: Vec<_> = tome.tiles.iter().flatten().collect();
            let leaves = tiles.iter().filter(|t| t.is_leaf()).count();
            let cells: usize = tiles.iter().map(|t| t.num_cells()).sum();
            let portals: Vec<_> = tiles.iter().flat_map(|t| t.portals.iter().flatten()).collect();
            let hierarchy = portals.iter().filter(|p| p.is_hierarchy()).count();
            let lods = tome.tile_lod_levels.as_deref().unwrap_or(&[]).iter().filter(|&&b| f32::from_bits(b) >= 0.0).count();
            println!(
                "  bounds {:?}..{:?} tree nodes {} tiles {} (leaf {leaves}) cells {cells} portals {} (hierarchy {hierarchy}) lod levels {lods} list width {} object list words {} cluster nodes {}",
                tome.tree_min,
                tome.tree_max,
                tome.tree.node_count,
                tiles.len(),
                portals.len(),
                tome.list_widths,
                tome.object_lists.as_ref().map_or(0, |l| l.words.len()),
                tome.cluster_nodes.as_ref().map_or(0, Vec::len),
            );
            let mut sizes: BTreeMap<String, u32> = BTreeMap::new();
            let (mut leaf_cells, mut leaf_portals, mut max_cells) = (0usize, 0usize, 0usize);
            for t in tiles.iter().filter(|t| t.is_leaf()) {
                let e: Vec<String> = (0..3).map(|a| format!("{}", (t.tree_max[a] - t.tree_min[a]).round())).collect();
                *sizes.entry(e.join("x")).or_default() += 1;
                leaf_cells += t.num_cells();
                leaf_portals += t.portals.as_ref().map_or(0, Vec::len);
                max_cells = max_cells.max(t.num_cells());
            }
            let over = |n: usize| tiles.iter().filter(|t| t.num_cells() > n).count();
            println!("  tiles over 95 cells {} / 127 {} / 255 {}", over(95), over(127), over(255));
            let inner: Vec<_> = tiles.iter().filter(|t| !t.is_leaf()).collect();
            let inner_cells: usize = inner.iter().map(|t| t.num_cells()).sum();
            println!(
                "  inner tiles {} cells {inner_cells} (max {}) portals {}",
                inner.len(),
                inner.iter().map(|t| t.num_cells()).max().unwrap_or(0),
                inner.iter().map(|t| t.portals.as_ref().map_or(0, Vec::len)).sum::<usize>()
            );
            // Connected components of the leaf cell graph (regular portals only).
            let mut first = vec![0usize; tome.tiles.len() + 1];
            for (i, t) in tome.tiles.iter().enumerate() {
                first[i + 1] = first[i] + t.as_ref().filter(|t| t.is_leaf()).map_or(0, |t| t.num_cells());
            }
            let mut set: Vec<usize> = (0..first[tome.tiles.len()]).collect();
            fn root(s: &mut [usize], mut i: usize) -> usize {
                while s[i] != i {
                    s[i] = s[s[i]];
                    i = s[i];
                }
                i
            }
            let (mut isolated, mut outside_cells) = (0usize, vec![false; set.len()]);
            for (i, t) in tome.tiles.iter().enumerate() {
                let Some(t) = t.as_ref().filter(|t| t.is_leaf()) else { continue };
                for (c, cell) in t.cell_nodes.iter().flatten().enumerate() {
                    let ps = &t.portals.as_deref().unwrap_or(&[])[cell.portal_index as usize..(cell.portal_index + cell.portal_count) as usize];
                    if ps.is_empty() {
                        isolated += 1;
                    }
                    for p in ps {
                        if p.is_outside() {
                            outside_cells[first[i] + c] = true;
                            continue;
                        }
                        if p.is_hierarchy() || first[p.target() as usize + 1] == first[p.target() as usize] {
                            continue;
                        }
                        let (a, b) = (root(&mut set, first[i] + c), root(&mut set, first[p.target() as usize] + p.target_index as usize));
                        set[a.max(b)] = a.min(b);
                    }
                }
            }
            let mut groups: BTreeMap<usize, usize> = BTreeMap::new();
            for g in 0..set.len() {
                *groups.entry(root(&mut set, g)).or_default() += 1;
            }
            let mut comp: Vec<usize> = groups.values().copied().collect();
            comp.sort_unstable_by(|a, b| b.cmp(a));
            println!(
                "  leaf cells {} isolated (no portals) {isolated} touching outside {} components {} largest {:?} singletons {}",
                set.len(),
                outside_cells.iter().filter(|&&o| o).count(),
                comp.len(),
                &comp[..comp.len().min(5)],
                comp.iter().filter(|&&s| s == 1).count()
            );
            let (mut olo, mut ohi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for b in &bounds {
                for a in 0..3 {
                    olo[a] = olo[a].min(b[a]);
                    ohi[a] = ohi[a].max(b[a + 3]);
                }
            }
            println!("  object bounds union {olo:?}..{ohi:?}");
            let leaves = leaves.max(1);
            println!(
                "  per leaf tile: cells {:.1} (max {max_cells}) portals {:.1}; per cell portals {:.1}",
                leaf_cells as f64 / leaves as f64,
                leaf_portals as f64 / leaves as f64,
                leaf_portals as f64 / leaf_cells.max(1) as f64
            );
            let mut sizes: Vec<_> = sizes.into_iter().collect();
            sizes.sort_by(|a, b| b.1.cmp(&a.1));
            println!("  leaf tile sizes: {}", sizes.iter().take(8).map(|(s, n)| format!("{s}:{n}")).collect::<Vec<_>>().join(" "));
        }
        if let Ok(at) = std::env::var("AT") {
            let p: Vec<f32> = at.split(',').map(|v| v.parse().unwrap()).collect();
            for (k, b) in bounds.iter().enumerate() {
                if (0..3).all(|a| b[a] <= p[a] && p[a] <= b[a + 3]) {
                    println!("  at {p:?}: {:08X} [{:.0} {:.0} {:.0}]..[{:.0} {:.0} {:.0}]", ids.get(k).copied().unwrap_or(0), b[0], b[1], b[2], b[3], b[4], b[5]);
                }
            }
        }
        if std::env::var_os("IDS").is_some() {
            let refs: Vec<String> = ids.iter().filter(|id| *id >> 24 != 0xFD).map(|id| format!("{id:08X}")).collect();
            println!("  ids: {}", refs.join(" "));
        }
    }
}
