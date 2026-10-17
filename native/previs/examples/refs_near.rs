//! Scratch (not committed): references within `r` of a point, with whether a
//! CELL's XCRI (precombined render) or XPRI (precombined physics) claims them.
//! Args: <plugin> <x> <y> <z> <r>
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use previs_native::plugin::{Plugin, read_f32, read_u32};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let plugin = Plugin::open(&PathBuf::from(&args[0])).unwrap();
    let at: Vec<f32> = args[1..4].iter().map(|s| s.parse().unwrap()).collect();
    let r: f32 = args[4].parse().unwrap();
    let (mut xcri, mut xpri) = (HashMap::new(), HashSet::new());
    for cell in plugin.records_of(b"CELL") {
        let subs = plugin.subrecords_at(&cell).unwrap();
        if let Some(x) = subs.first(b"XPRI") {
            xpri.extend((0..x.len() / 4).map(|i| read_u32(x, 4 * i).unwrap() & 0xFF_FFFF));
        }
        if let Some(x) = subs.first(b"XCRI") {
            let meshes = read_u32(x, 0).unwrap() as usize;
            let words = read_u32(x, 4).unwrap() as usize;
            for i in 0..words / 2 {
                xcri.insert(read_u32(x, 8 + 4 * meshes + 8 * i).unwrap() & 0xFF_FFFF, read_u32(x, 12 + 4 * meshes + 8 * i).unwrap());
            }
        }
    }
    for record in plugin.records_of(b"REFR") {
        let subs = plugin.subrecords_at(&record).unwrap();
        let Some(data) = subs.first(b"DATA") else { continue };
        let p: Vec<f32> = (0..3).map(|k| read_f32(data, 4 * k).unwrap()).collect();
        if (0..3).map(|k| (p[k] - at[k]).powi(2)).sum::<f32>().sqrt() > r {
            continue;
        }
        let base = subs.first(b"NAME").and_then(|d| read_u32(d, 0)).unwrap_or(0);
        let id = record.form_id & 0xFF_FFFF;
        println!(
            "{:08X} base {base:08X} at [{:.0}, {:.0}, {:.0}] xcri {} xpri {}",
            record.form_id,
            p[0],
            p[1],
            p[2],
            xcri.get(&id).map_or("-".to_string(), |m| format!("{m:08X}")),
            xpri.contains(&id)
        );
    }
}
