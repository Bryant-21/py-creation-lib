use std::collections::BTreeMap;
use std::path::PathBuf;

use rayon::prelude::*;

const WORKERS: usize = 8;

fn main() -> anyhow::Result<()> {
    let usage = "usage: audit_terrain_contact <objects dir> <terrain dir> <out.json>";
    let mut args = std::env::args().skip(1);
    let objects = PathBuf::from(args.next().expect(usage));
    let terrain = PathBuf::from(args.next().expect(usage));
    let out = PathBuf::from(args.next().expect(usage));
    let mut pairs = Vec::new();
    for entry in std::fs::read_dir(&objects)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let Some((world, level, x, y)) = lodgen_native::audit::parse_tile_name(name) else { continue };
        let btr = terrain.join(format!("{world}.{level}.{x}.{y}.btr"));
        if btr.exists() {
            let cell = lodgen_native::audit::CELL;
            pairs.push((path, btr, level, [x as f32 * cell, y as f32 * cell]));
        }
    }
    let pool = rayon::ThreadPoolBuilder::new().num_threads(WORKERS).build()?;
    let rows: Vec<_> = pool.install(|| {
        pairs
            .par_iter()
            .filter_map(|(bto, btr, level, origin)| {
                let counts = std::fs::read(bto)
                    .and_then(|o| std::fs::read(btr).map(|t| (o, t)))
                    .map_err(anyhow::Error::from)
                    .and_then(|(o, t)| lodgen_native::audit::terrain_contact(&o, &t, *origin));
                match counts {
                    Ok(c) => Some((bto.display().to_string(), *level, c)),
                    Err(e) => {
                        eprintln!("skip {}: {e}", bto.display());
                        None
                    }
                }
            })
            .collect()
    });
    let mut per_level: BTreeMap<u32, [u64; 4]> = BTreeMap::new();
    for (_, level, c) in &rows {
        let sum = per_level.entry(*level).or_default();
        for i in 0..4 {
            sum[i] += c[i];
        }
    }
    let tiles: Vec<_> = rows.iter().map(|(p, l, c)| serde_json::json!({ "path": p, "level": l, "contact": c })).collect();
    std::fs::write(&out, serde_json::to_vec_pretty(&serde_json::json!({ "per_level": per_level, "tiles": tiles }))?)?;
    println!("pairs {} per_level {}", rows.len(), serde_json::to_string(&per_level)?);
    Ok(())
}
