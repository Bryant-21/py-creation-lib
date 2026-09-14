mod output_writer;

use super::*;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

fn run(root: &Path, inputs: &[PathBuf], writers: Option<usize>) -> (f64, profiling::Timings) {
    let directories = Arc::new(output_directories::OutputDirectories::default());
    let writer =
        writers.map(|count| output_writer::OutputWriter::new(count, 32 * 1024 * 1024).unwrap());
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(20)
        .build()
        .unwrap();
    let started = Instant::now();
    let mut total = profiling::Timings::default();
    for _ in 0..4 {
        let reports = pool.install(|| {
            (0..512)
                .into_par_iter()
                .map(|index| {
                    directories.scope(|| {
                        profiling::capture(|| {
                            let decoded =
                                read_dds_rgba_image(&inputs[index % inputs.len()]).unwrap();
                            let mut rgba = decoded.rgba;
                            for pixel in rgba.chunks_exact_mut(4) {
                                pixel[2] = 0;
                            }
                            let chain =
                                rgba8_box_mip_chain(decoded.width, decoded.height, &rgba).unwrap();
                            let format = if index % 4 == 0 {
                                "BC5_UNORM"
                            } else {
                                "R8G8B8A8_UNORM"
                            };
                            let bytes =
                                encode_dds_from_rgba8_chain(&chain, format, false, None).unwrap();
                            let path = root.join(format!("dir_{}/{}.dds", index % 16, index));
                            profiling::create_dir_all(path.parent().unwrap()).unwrap();
                            match &writer {
                                Some(writer) => writer.write(&path, bytes).unwrap(),
                                None => profiling::write(path, bytes).unwrap(),
                            }
                        })
                        .1
                    })
                })
                .collect::<Vec<_>>()
        });
        for report in reports {
            total.add(report);
        }
    }
    if let Some(writer) = &writer {
        total.add(writer.timings());
        assert!(writer.peak_queued_bytes() <= 32 * 1024 * 1024);
        assert!(writer.peak_writers() <= writers.unwrap());
    }
    (started.elapsed().as_secs_f64(), total)
}

#[test]
#[ignore = "Controlled 20-converter writer concurrency benchmark; writes only under repo tmp"]
fn compare_writer_concurrency() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap()
        .join("tmp/texture_optimization_20260909/write_benchmark");
    std::fs::create_dir_all(root.join("inputs")).unwrap();
    let inputs: Vec<_> = [128, 256, 512, 1024]
        .into_iter()
        .enumerate()
        .map(|(i, side)| {
            let path = root.join(format!("inputs/{i}.dds"));
            let rgba = (0..side * side * 4)
                .map(|x| ((x * 17 + x / 37) % 256) as u8)
                .collect::<Vec<_>>();
            write_dds_rgba_image(&path, side, side, &rgba, "BC5_UNORM", true).unwrap();
            path
        })
        .collect();
    let mut rows = Vec::new();
    for (round, order) in [
        [None, Some(2), Some(4), Some(8), Some(20)],
        [Some(20), Some(8), Some(4), Some(2), None],
    ]
    .into_iter()
    .enumerate()
    {
        for writers in order {
            let label = writers.map(|n| n.to_string()).unwrap_or("direct".into());
            let output = root.join(format!("round_{round}/{label}"));
            let (seconds, timings) = run(&output, &inputs, writers);
            let reference = root.join("round_0/direct");
            if output != reference {
                for index in 0..512 {
                    let relative = format!("dir_{}/{}.dds", index % 16, index);
                    assert_eq!(
                        std::fs::read(output.join(&relative)).unwrap(),
                        std::fs::read(reference.join(relative)).unwrap()
                    );
                }
            }
            let row = format!(
                "{{\"round\":{round},\"writers\":\"{label}\",\"seconds\":{seconds},\"bytes\":{},\"directory_ms\":{},\"open_ms\":{},\"write_ms\":{},\"close_ms\":{}}}",
                timings.write_bytes,
                timings.directory_ns as f64 / 1e6,
                timings.open_ns as f64 / 1e6,
                timings.file_write_ns as f64 / 1e6,
                timings.close_ns as f64 / 1e6
            );
            eprintln!("{row}");
            rows.push(row);
            std::fs::write(
                root.join("results.json"),
                format!("[{}]\n", rows.join(",\n")),
            )
            .unwrap();
        }
    }
}
