//! Dev harness: does havok_native's typed packfile model round-trip FO4
//! collision packfiles byte-for-byte?
//!
//! Args: <dir of .nif files> [limit]
//! `DUMP=<dir>` writes the first differing packfile of each shape kind as
//! `<kind>.orig.hkx` and `<kind>.out.hkx`.
use std::collections::BTreeMap;
use std::path::PathBuf;

use havok_native::hkx::descriptors::DescriptorRegistry;
use havok_native::hkx::{read_packfile, write_hkx};
use nif_core_native::model::{NifFile, NifValue};

fn blobs(nif: &NifFile) -> Vec<Vec<u8>> {
    nif.blocks
        .iter()
        .filter(|b| b.type_name == "bhkPhysicsSystem")
        .filter_map(|b| match b.get_field("Binary Data") {
            Some(NifValue::Struct(f)) => match f.get("Data") {
                Some(NifValue::Array(items)) => Some(items.iter().map(|v| v.as_i64() as u8).collect()),
                Some(NifValue::Bytes(bytes)) => Some(bytes.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let limit: usize = args.get(1).map_or(200, |s| s.parse().unwrap());
    let mut stats: BTreeMap<String, u32> = BTreeMap::new();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut stack = vec![PathBuf::from(&args[0])];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("nif")) {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut seen = 0;
    for path in files {
        if seen >= limit {
            break;
        }
        let Ok(nif) = NifFile::from_bytes(&std::fs::read(&path).unwrap(), None) else { continue };
        for blob in blobs(&nif) {
            seen += 1;
            let key = match read_packfile(&blob) {
                Err(e) => format!("read error: {}", e.to_string().chars().take(60).collect::<String>()),
                Ok(hkx) => {
                    let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
                    let out = write_hkx(&hkx, &mut registry);
                    let classes: Vec<&str> = hkx.objects().iter().skip(1).map(|o| o.class_name.as_str()).collect();
                    let kind = classes.first().copied().unwrap_or("-").to_string();
                    if out == blob {
                        format!("exact {kind}")
                    } else {
                        let first = out.iter().zip(&blob).position(|(a, b)| a != b).unwrap_or(out.len().min(blob.len()));
                        let (key, tag) = if out.len() != blob.len() {
                            (format!("differs {kind}: length"), "length".to_string())
                        } else {
                            let words = out.chunks(4).zip(blob.chunks(4)).filter(|(a, b)| a != b).count();
                            let bucket = [1, 4, 16, 64].into_iter().find(|&b| words <= b).map_or("more".into(), |b| format!("le{b}"));
                            (format!("differs {kind}: {bucket} words"), bucket)
                        };
                        if !stats.contains_key(&key) {
                            eprintln!("{} {key}: len {} vs {}, first diff at {first:#x}", path.display(), out.len(), blob.len());
                            if let Some(dir) = std::env::var_os("DUMP") {
                                let dir = PathBuf::from(dir);
                                std::fs::write(dir.join(format!("{kind}.{tag}.orig.hkx")), &blob).unwrap();
                                std::fs::write(dir.join(format!("{kind}.{tag}.out.hkx")), &out).unwrap();
                            }
                        }
                        key
                    }
                }
            };
            *stats.entry(key).or_default() += 1;
            let scale = |b: &[u8]| previs_native::collision_scale::scale_packfile(b, 2.0);
            let normalize = |b: &[u8]| {
                let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
                write_hkx(&read_packfile(b).unwrap(), &mut registry)
            };
            // Scaling the source must agree with scaling its writer-normalized form, up to writer noise.
            let scaled = match scale(&blob) {
                Ok(out) if scale(&normalize(&blob)).is_ok_and(|n| n == normalize(&out)) => "scale ok".to_string(),
                Ok(_) => "scale MISMATCH".to_string(),
                Err(e) => format!("scale refused: {e}"),
            };
            *stats.entry(scaled).or_default() += 1;
        }
    }
    for (k, v) in stats {
        println!("{v:6} {k}");
    }
}
