//! Dev harness: compares two tomes' object tables by user ID (0xFD shapes
//! per CELL, other IDs as references) and reports IDs whose bounds differ
//! or that only one tome lists. Args: <a.uvd> <b.uvd>
//! `SHOW=n` prints up to n example IDs per row.
use std::collections::{BTreeMap, HashMap};

use previs_native::umbra_tome::parse;

fn table(path: &str) -> HashMap<u32, [f32; 6]> {
    let tome = parse(&std::fs::read(path).unwrap()).unwrap();
    tome.user_ids.unwrap_or_default().into_iter().zip(tome.object_bounds.unwrap_or_default()).collect()
}

fn kind(id: u32) -> String {
    if id >> 24 == 0xFD { format!("FD ({},{})", (id >> 19) & 0x1F, (id >> 14) & 0x1F) } else { "ref".into() }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (a, b) = (table(&args[0]), table(&args[1]));
    let show: usize = std::env::var("SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut rows: BTreeMap<(String, &str), Vec<String>> = BTreeMap::new();
    for (&id, ba) in &a {
        let verdict = match b.get(&id) {
            None => "only in a",
            Some(bb) if (0..6).all(|k| (ba[k] - bb[k]).abs() <= 1.0) => "same box",
            Some(bb) => {
                rows.entry((kind(id), "different box")).or_default().push(format!(
                    "{id:08X} a[{:.0} {:.0} {:.0}] b[{:.0} {:.0} {:.0}]",
                    ba[0], ba[1], ba[2], bb[0], bb[1], bb[2]
                ));
                continue;
            }
        };
        rows.entry((kind(id), verdict)).or_default().push(format!("{id:08X}"));
    }
    for &id in b.keys().filter(|id| !a.contains_key(id)) {
        rows.entry((kind(id), "only in b")).or_default().push(format!("{id:08X}"));
    }
    for ((kind, verdict), ids) in rows {
        let examples = ids.iter().take(show).cloned().collect::<Vec<_>>().join(", ");
        println!("{kind:12} {verdict:14} {:5} {examples}", ids.len());
    }
}
