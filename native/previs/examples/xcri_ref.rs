//! Scratch (not committed): the CELL whose XCRI combines a reference, the
//! mesh it lands in and that mesh's place in the XCRI. Args: <plugin> <ref hex>
use std::path::PathBuf;

use previs_native::plugin::{Plugin, read_u32};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let plugin = Plugin::open(&PathBuf::from(&args[0])).unwrap();
    let wanted = u32::from_str_radix(&args[1], 16).unwrap() & 0xFF_FFFF;
    for cell in plugin.records_of(b"CELL") {
        let subs = plugin.subrecords_at(&cell).unwrap();
        if let Some(x) = subs.first(b"XPRI")
            && (0..x.len() / 4).any(|i| read_u32(x, 4 * i).unwrap() & 0xFF_FFFF == wanted)
        {
            println!("CELL {:08X}: in XPRI ({} physics refs)", cell.form_id, x.len() / 4);
        }
        let Some(x) = subs.first(b"XCRI") else { continue };
        let meshes = read_u32(x, 0).unwrap() as usize;
        let words = read_u32(x, 4).unwrap() as usize;
        let ids: Vec<u32> = (0..meshes).map(|i| read_u32(x, 8 + 4 * i).unwrap()).collect();
        for i in 0..words / 2 {
            let r = read_u32(x, 8 + 4 * meshes + 8 * i).unwrap();
            if r & 0xFF_FFFF != wanted {
                continue;
            }
            let m = read_u32(x, 12 + 4 * meshes + 8 * i).unwrap();
            let refs_in_mesh = (0..words / 2).filter(|&j| read_u32(x, 12 + 4 * meshes + 8 * j).unwrap() == m).count();
            println!(
                "CELL {:08X}: ref {r:08X} in mesh {m:08X} (#{} of {meshes}, {refs_in_mesh} refs); meshes {}",
                cell.form_id,
                ids.iter().position(|&id| id == m).map_or(-1, |p| p as i64),
                ids.iter().map(|id| format!("{id:08X}")).collect::<Vec<_>>().join(" ")
            );
        }
    }
}
