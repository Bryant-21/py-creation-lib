use std::path::PathBuf;
use std::time::Instant;

use previs_native::metadata::{CellMetadata, Xcri, write_patched_plugin};
use previs_native::plugin::Plugin;
use rustc_hash::FxHashMap;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let source = Plugin::open(&PathBuf::from(&args[0])).unwrap();
    let output = PathBuf::from(&args[1]);
    let mut cells = FxHashMap::default();
    for cell in source.records_of(b"CELL") {
        let subrecords = source.subrecords_at(&cell).unwrap();
        let mut metadata = CellMetadata {
            previs: true,
            root_visibility_cell: subrecords.first(b"RVIS").map(|b| u32::from_le_bytes(b[..4].try_into().unwrap())),
            previs_reference_ids: subrecords.first(b"XPRI").unwrap_or_default().chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect(),
            ..CellMetadata::default()
        };
        if let Some(bytes) = subrecords.first(b"XCRI") {
            let words: Vec<_> = bytes.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
            let split = 2 + words[0] as usize;
            metadata.precombine = true;
            metadata.xcri = Some(Xcri {
                mesh_ids: words[2..split].to_vec(),
                reference_mesh_pairs: words[split..].chunks_exact(2).map(|p| (p[0], p[1])).collect(),
            });
        }
        cells.insert(cell.form_id, metadata);
    }
    let started = Instant::now();
    let patched = write_patched_plugin(&source, &output, &cells, 0x3548).unwrap();
    println!("{}", serde_json::json!({"cells": patched, "seconds": started.elapsed().as_secs_f64()}));
}
