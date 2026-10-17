//! Replays interior planning with a chosen cache budget; compares scene hashes across budgets.
use std::path::PathBuf;
use std::time::Instant;

use previs_native::assets::AssetResolver;
use previs_native::plugin::LoadOrder;
use previs_native::previs::{Context, ModelCache, plan_interior_cell};
use previs_native::scene::SceneBuilder;

#[path = "../src/memory.rs"]
mod memory;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(args.len(), 6, "<plugin> <master-dir> <loose-root> <cache-MiB> <cell-count> <report.json>");
    let mut memory = memory::MemoryMonitor::start();
    let plugin = PathBuf::from(&args[0]);
    let load_order = LoadOrder::open(&plugin, &[plugin.parent().unwrap().to_owned(), PathBuf::from(&args[1])]).unwrap();
    let assets = AssetResolver::new(vec![plugin.parent().unwrap().join("data"), PathBuf::from(&args[2])], &[]).unwrap();
    let models = ModelCache::with_budget(args[3].parse::<usize>().unwrap() << 20);
    let context = Context { load_order: &load_order, assets: &assets, models: &models, policy: Default::default(), small_model_cull: true };
    let cells: Vec<_> = load_order.target().records_of(b"CELL").filter(|cell| {
        load_order.target().subrecords_at(cell).unwrap().first(b"DATA").is_some_and(|data| data[0] & 1 != 0)
    }).take(args[4].parse().unwrap()).map(|cell| cell.form_id).collect();
    let started = Instant::now();
    let mut outputs = Vec::new();
    for cell in cells {
        let mut scene = SceneBuilder::default();
        let output = match plan_interior_cell(&context, cell, &mut scene) {
            Ok(plan) => {
                let scene = scene.finish(plan.volume.unwrap()).encode();
                serde_json::json!({"cell": cell, "sha256": previs_native::sha256_hex(&scene), "bytes": scene.len(), "non_parity": plan.non_parity})
            }
            Err(error) => serde_json::json!({"cell": cell, "error": error.to_string()}),
        };
        outputs.push(output);
    }
    let report = serde_json::json!({"seconds": started.elapsed().as_secs_f64(), "cache": models.stats(), "memory": memory.finish(), "outputs": outputs});
    std::fs::write(&args[5], serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("{:.3}s, {} cells", report["seconds"].as_f64().unwrap(), outputs.len());
}
