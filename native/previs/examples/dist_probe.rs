//! Scratch (not committed): histogram of tome object cull distances.
//! Args: <file.uvd>... [ENV ID=hex prints one object]
use std::collections::BTreeMap;

use previs_native::umbra_tome::parse;

fn main() {
    let id = std::env::var("ID").ok().and_then(|h| u32::from_str_radix(&h, 16).ok());
    for path in std::env::args().skip(1) {
        let tome = parse(&std::fs::read(&path).unwrap()).unwrap();
        let ids = tome.user_ids.clone().unwrap_or_default();
        let mut hist: BTreeMap<String, usize> = BTreeMap::new();
        for (i, r) in tome.object_distances.iter().flatten().enumerate() {
            let f = r.map(f32::from_bits);
            let d = f[7].sqrt();
            let key = if d > 1e9 { "inf".into() } else { format!("{d:.0}") };
            *hist.entry(format!("{} {}", if ids.get(i).is_some_and(|&u| u >= 0xFD00_0000) { "combined" } else { "ref" }, key)).or_default() += 1;
            if id.is_some_and(|w| ids.get(i) == Some(&w)) {
                println!("  {:08X} record {f:?}", ids[i]);
            }
        }
        println!("{path}: {hist:?}");
    }
}
