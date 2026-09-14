//! Time a full AnimTextData generation against a real converted mod, without going
//! through the Python extension (so it never contends for the `.pyd`).
//!
//! ```text
//! cargo run -p ck_native --release --example anim_text_data_timing -- \
//!     <plugin.esm> <src_meshes_root> <out_meshes_root> [base_meshes_root] [base_plugin.esm]
//! ```
//! `out_meshes_root` should be a scratch dir — the run writes real bucket files there.

use std::path::PathBuf;
use std::time::Instant;

use ck_native::anim_text_data::emit::{AnimTextDataInputs, generate_anim_text_data_with_progress};
use ck_native::anim_text_data::race_decode::subgraph_inputs_from_plugin;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!(
            "usage: anim_text_data_timing <plugin> <src_meshes> <out_meshes> \
             [base_meshes] [base_plugin]"
        );
        std::process::exit(2);
    }
    let plugin = PathBuf::from(&args[0]);
    let src = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let base_meshes = args.get(3).map(PathBuf::from);
    let base_plugins: Vec<PathBuf> = args.get(4).map(PathBuf::from).into_iter().collect();

    // The plugin decode reaches code that constructs pyo3 objects.
    pyo3::Python::initialize();

    let started = Instant::now();
    let decoded = subgraph_inputs_from_plugin(&plugin, "fo4", &base_plugins)
        .unwrap_or_else(|error| panic!("decode {}: {error}", plugin.display()));
    let inputs = AnimTextDataInputs::from(decoded);
    println!(
        "[{:.1}s] decoded plugin inputs",
        started.elapsed().as_secs_f64()
    );

    let report = generate_anim_text_data_with_progress(
        &inputs,
        &src,
        &out,
        base_meshes.as_deref(),
        Some("B21"),
        &mut |message| println!("[{:.1}s] {message}", started.elapsed().as_secs_f64()),
    )
    .unwrap_or_else(|error| panic!("generation failed: {error}"));

    println!(
        "[{:.1}s] TOTAL — wrote {} file(s)",
        started.elapsed().as_secs_f64(),
        report.written
    );
}
