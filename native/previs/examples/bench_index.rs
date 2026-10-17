//! Compare two CDX rebuilds with one resource collection and a retained-row rebuild.
//! The plugin must be a scratch copy: its adjacent CDXs are overwritten.
//! Args: <scratch plugin> <data dir> <report.json> <reparse|retained> <workers>
use std::path::PathBuf;
use std::time::Instant;

use nif_core_native::model::NifFile;
use previs_native::cdx::{self, CellLocation};
use previs_native::plugin::{Plugin, read_u32};
use previs_native::precombine_stage::{CellResources, IndexResources, write_index, write_index_with_resources};
use rayon::prelude::*;

#[path = "../src/memory.rs"]
mod memory;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let plugin_path = PathBuf::from(&args[0]);
    let data = PathBuf::from(&args[1]);
    let mut memory = memory::MemoryMonitor::start();
    let pool = previs_native::worker_pool(Some(args[4].parse().unwrap()));
    let (index, retained_bytes, prepare_seconds, index_seconds) = pool.install(|| {
        let started = Instant::now();
        let resources = if args[3] == "retained" {
            let plugin = Plugin::open(&plugin_path).unwrap();
            let mesh_dir = data.join("Meshes/PreCombined").join(&plugin.name);
            let cells: Vec<_> = plugin.records_of(b"CELL").filter(|r| r.form_id >> 24 == plugin.self_index()).collect();
            let resources = cells.par_iter().filter_map(|cell| {
                let fields = plugin.subrecords_at(cell).unwrap();
                let xcri = fields.first(b"XCRI")?;
                let mesh_ids: Vec<_> = (0..read_u32(xcri, 0).unwrap() as usize).map(|i| read_u32(xcri, 8 + i * 4).unwrap()).collect();
                let local = cell.form_id & 0xFFFFFF;
                let groups: Vec<_> = mesh_ids.iter().map(|id| mesh_dir.join(format!("{local:08X}_{id:08X}_OC.NIF"))).collect();
                if groups.is_empty() || !groups.iter().all(|p| p.is_file()) {
                    return None;
                }
                let groups = mesh_ids.iter().zip(groups).map(|(&id, path)| {
                    Ok((id, NifFile::from_bytes_raw_arrays(&std::fs::read(path).unwrap(), None).unwrap()))
                });
                let mut rows = cdx::cell_rows(&plugin.name, cell.form_id, CellLocation::Interior, groups, None).unwrap();
                rows.pop();
                rows.sort_unstable();
                rows.dedup();
                rows.shrink_to_fit();
                Some((cell.form_id, CellResources { mesh_ids, rows }))
            }).collect();
            Some(IndexResources { plugin: plugin_path.clone(), data_dir: data.clone(), cells: resources })
        } else {
            assert_eq!(args[3], "reparse");
            write_index(&plugin_path, &data, previs_native::precombine_stage::ExteriorRows::Separate).unwrap();
            None
        };
        let prepare_seconds = started.elapsed().as_secs_f64();
        let retained_bytes: usize = resources.as_ref().map_or(0, |r| r.cells.values().map(|c| c.rows.capacity() * 16 + c.mesh_ids.capacity() * 4).sum());
        let started = Instant::now();
        let index = write_index_with_resources(&plugin_path, &data, previs_native::precombine_stage::ExteriorRows::Separate, resources.as_ref()).unwrap();
        drop(resources);
        (index, retained_bytes, prepare_seconds, started.elapsed().as_secs_f64())
    });
    let exterior = plugin_path.with_file_name(format!("{} - Exterior.cdx", plugin_path.file_stem().unwrap().to_string_lossy()));
    let exterior_sha256 = std::fs::read(exterior).ok().map(|bytes| previs_native::sha256_hex(&bytes));
    std::fs::write(&args[2], serde_json::to_vec_pretty(&serde_json::json!({
        "index": index, "exterior_sha256": exterior_sha256, "retained_bytes": retained_bytes,
        "prepare_seconds": prepare_seconds, "index_seconds": index_seconds, "memory": memory.finish(),
    })).unwrap()).unwrap();
}
