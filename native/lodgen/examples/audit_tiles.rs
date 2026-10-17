use std::path::PathBuf;

use rayon::prelude::*;

// A parsed tile holds every vertex as a NifValue (~300 MB for the largest 13 MB tile), so cap the workers.
const WORKERS: usize = 8;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: audit_tiles <objects dir> <out.json>"));
    let out = PathBuf::from(args.next().expect("usage: audit_tiles <objects dir> <out.json>"));
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().map(|e| e.eq_ignore_ascii_case("bto")).unwrap_or(false) {
            paths.push(path);
        }
    }
    paths.sort();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(WORKERS).build()?;
    let audits: Vec<_> = pool.install(|| {
        paths
            .par_iter()
            .filter_map(|path| {
                let audit = std::fs::read(path)
                    .map_err(anyhow::Error::from)
                    .and_then(|bytes| lodgen_native::audit::audit_tile(path, &bytes));
                match audit {
                    Ok(a) => Some(a),
                    Err(e) => {
                        eprintln!("skip {}: {e}", path.display());
                        None
                    }
                }
            })
            .collect()
    });
    let summary = lodgen_native::audit::summarize(&audits);
    std::fs::write(&out, serde_json::to_vec_pretty(&serde_json::json!({ "summary": summary, "tiles": audits }))?)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
