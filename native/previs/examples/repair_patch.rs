//! Dev harness: the repair-patch pipeline of `creation_lib.previs.build`
//! without Python (seed, precombines, previs, patch cut), for real-data runs
//! while the extension cannot be rebuilt.
//!
//! Args: <spec.json> <work dir> <output dir> [workers]
//! The spec holds `plugins`, `data_sources` (lowest priority first),
//! `base_plugins`, `precombine_cells` and `seed_cells` (form keys).
use std::path::PathBuf;

use serde::Deserialize;

use previs_native::asset_index::DataSource;
use previs_native::metadata::pack_generation_date;
use previs_native::patch::{PatchRequest, SeedRequest, write_patch, write_seed};
use previs_native::precombine_stage::PrecombineRequest;
use previs_native::stage::PlanRequest;

#[derive(Deserialize)]
struct Spec {
    plugins: Vec<PathBuf>,
    data_sources: Vec<DataSource>,
    base_plugins: Vec<String>,
    precombine_cells: Vec<String>,
    seed_cells: Vec<String>,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let spec: Spec = serde_json::from_slice(&std::fs::read(args.next().unwrap()).unwrap()).unwrap();
    let work = PathBuf::from(args.next().unwrap());
    let output = PathBuf::from(args.next().unwrap());
    let workers = args.next().map(|w| w.parse().unwrap());
    let pool = previs_native::worker_pool(workers);
    let name = "ModBox21 Previs Repair.esm";
    let _ = std::fs::remove_dir_all(&work);
    let data = work.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let date = pack_generation_date(2026, 10, 11).unwrap();
    let started = std::time::Instant::now();

    let seed = write_seed(&SeedRequest {
        plugins: spec.plugins.clone(),
        base_plugins: spec.base_plugins.clone(),
        fixer_plugins: vec!["Unofficial Fallout 4 Patch.esp".into()],
        cells: spec.seed_cells.clone(),
        precombine_cells: spec.precombine_cells.clone(),
        output: work.join(name),
        date,
        stand_in: false,
        required_masters: vec![],
    })
    .unwrap();
    println!("seed: {} cells, {} records, {} masters, {} skipped in {:.1}s", seed.cells.len(), seed.records, seed.masters.len(), seed.skipped.len(), seed.seconds);
    for skip in &seed.skipped {
        println!("  skipped {}: {}", skip.form_key, skip.reason);
    }

    let mut loose = Vec::new();
    let mut archives = Vec::new();
    for source in &spec.data_sources {
        match source {
            DataSource::Loose { path } => loose.push(path.clone()),
            DataSource::Archive { path } => archives.push(path.clone()),
        }
    }
    let mut loose_roots = vec![data.clone()];
    loose_roots.extend(loose.into_iter().rev());
    archives.reverse();
    let mut master_dirs: Vec<PathBuf> = Vec::new();
    for plugin in &spec.plugins {
        let parent = plugin.parent().unwrap().to_path_buf();
        if !master_dirs.contains(&parent) {
            master_dirs.push(parent);
        }
    }
    let planned: Vec<u32> = seed.cells.iter().filter(|c| !c.context).map(|c| c.form_id).collect();
    let rebuild: Vec<u32> = seed.cells.iter().filter(|c| spec.precombine_cells.contains(&c.form_key)).map(|c| c.form_id).collect();

    let resources = pool.install(|| {
        let request = PrecombineRequest {
            plugin: seed.plugin.clone(),
            master_dirs: master_dirs.clone(),
            loose_roots: loose_roots.clone(),
            archives: archives.clone(),
            output_dir: data.clone(),
            interior_cdx_only: false,
            single_cdx: true,
            cells: Some(rebuild),
        };
        let prepared = previs_native::precombine_generation::prepare(&request, &work.join("precombine_report.json"), &work.join("precombine_timing.json"), &|_, _| {}).unwrap();
        let (summary, resources) = prepared.finalize_with_resources(date).unwrap();
        println!(
            "precombines: {} cells, {} groups, {} unsupported, {} invalid, index {:?}",
            summary.cells, summary.groups, summary.skipped_unsupported, summary.skipped_invalid.len(), summary.index.as_ref().map(|i| (&i.cdx_relative_path, i.cells))
        );
        for reason in &summary.skipped_invalid {
            println!("  invalid: {reason}");
        }
        resources
    });

    pool.install(|| {
        let request = PlanRequest {
            plugin: seed.plugin.clone(),
            master_dirs: master_dirs.clone(),
            loose_roots: loose_roots.clone(),
            archives: archives.clone(),
            scene_dir: work.join("scenes"),
            worlds: Vec::new(),
            interiors: true,
            accept_reference_layer: false,
            small_model_cull: false,
            solve_into: Some(data.clone()),
            keep_scenes: false,
            interior_cdx_only: false,
            single_cdx: true,
            cells: Some(planned.clone()),
        };
        let prepared = previs_native::generation::prepare_with_resources(&request, &work.join("previs_plan.json"), &work.join("previs_timing.json"), &|_, _| {}, Some(resources)).unwrap();
        let summary = prepared.finalize(date).unwrap();
        println!(
            "previs: {} solved, {} stamp-only, {} unsupported, {} invalid, {} failed, index {:?}",
            summary.generated, summary.stamped, summary.skipped_unsupported, summary.skipped_invalid, summary.failed.len(),
            summary.index.as_ref().map(|i| (&i.cdx_relative_path, i.cells, i.cells_with_visibility))
        );
        for warning in &summary.warnings {
            println!("  {warning}");
        }
    });

    let esm = name.to_ascii_lowercase().ends_with(".esm");
    let report = write_patch(&PatchRequest { seed, output: output.join(name), esm }).unwrap();
    println!("patch: {} cells, {} unchanged, {} forwarded children, {} masters", report.cells.len(), report.unchanged.len(), report.forwarded_children, report.masters);
    for cell in &report.unchanged {
        println!("  unchanged {cell}");
    }
    println!("total {:.1}s", started.elapsed().as_secs_f64());
}
