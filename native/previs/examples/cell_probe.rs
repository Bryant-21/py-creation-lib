//! Scratch (not committed): one CELL's previs stamps, group-key classes and
//! room markers. Args: <plugin> <object id hex>
use std::collections::BTreeMap;
use std::path::PathBuf;

use previs_native::plugin::{Plugin, read_u32};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let plugin = Plugin::open(&PathBuf::from(&args[0])).unwrap();
    let object = u32::from_str_radix(&args[1], 16).unwrap();
    let cell = plugin.records_of(b"CELL").find(|c| c.form_id & 0xFF_FFFF == object).unwrap();
    let subs = plugin.subrecords_at(&cell).unwrap();
    println!("CELL {:08X}", cell.form_id);
    for sig in [b"DATA", b"PCMB", b"VISI", b"RVIS"] {
        println!("  {} {:?}", String::from_utf8_lossy(sig), subs.first(sig).map(|d| d.iter().map(|b| format!("{b:02X}")).collect::<String>()));
    }
    let xpri = subs.first(b"XPRI").map_or(0, |d| d.len() / 4);
    println!("  XPRI {xpri}");
    let topology = plugin.cells.get(&cell.form_id).unwrap();
    let mut markers: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for child in topology.persistent.iter().chain(&topology.temporary) {
        let s = plugin.subrecords_at(child).unwrap();
        let Some(name) = s.first(b"NAME").and_then(|d| read_u32(d, 0)) else { continue };
        let (owner, id) = plugin.owner_of(name);
        if owner.eq_ignore_ascii_case("Fallout4.esm") && [0x15, 0x1F, 0x20].contains(&id) {
            markers.entry(id).or_default().push(child.form_id);
        }
    }
    println!("  children {} markers {:?}", topology.persistent.len() + topology.temporary.len(), markers.iter().map(|(k, v)| (format!("{k:X}"), v.len())).collect::<Vec<_>>());
    let all: Vec<u32> = markers.values().flatten().copied().collect();
    if let Some(x) = subs.first(b"XCRI") {
        let meshes = read_u32(x, 0).unwrap() as usize;
        let words = read_u32(x, 4).unwrap() as usize;
        let mut classes: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        let ids: Vec<u32> = (0..meshes).map(|i| read_u32(x, 8 + 4 * i).unwrap()).collect();
        for &m in &ids {
            let class = if all.contains(&(m & 0x7FFF_FFFF)) { "marker" } else if m & 0x4000_0000 != 0 { "always" } else { "spatial" };
            classes.entry(class).or_default().0 += 1;
        }
        for i in 0..words / 2 {
            let m = read_u32(x, 12 + 4 * meshes + 8 * i).unwrap();
            let class = if all.contains(&(m & 0x7FFF_FFFF)) { "marker" } else if m & 0x4000_0000 != 0 { "always" } else { "spatial" };
            classes.entry(class).or_default().1 += 1;
            if args.get(2).is_some_and(|h| u32::from_str_radix(h, 16).unwrap() == m) {
                let r = read_u32(x, 8 + 4 * meshes + 8 * i).unwrap();
                let full = plugin.records_of(b"REFR").find(|c| c.form_id & 0xFF_FFFF == r & 0xFF_FFFF);
                let (name, pos) = full.map_or((0, vec![]), |c| {
                    let s = plugin.subrecords_at(&c).unwrap();
                    let name = s.first(b"NAME").and_then(|d| read_u32(d, 0)).unwrap_or(0);
                    let pos = s.first(b"DATA").map(|d| (0..6).map(|k| f32::from_le_bytes(d[4 * k..4 * k + 4].try_into().unwrap())).collect()).unwrap_or_default();
                    (name, pos)
                });
                println!("    ref {r:08X} base {name:08X} data {pos:?}");
            }
        }
        println!("  XCRI meshes {meshes} refs {} by class (groups, refs) {classes:?}", words / 2);
    } else {
        println!("  no XCRI");
    }
}
