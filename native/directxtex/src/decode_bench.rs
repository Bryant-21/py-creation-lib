use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "directxtex_decode_{label}_{}_{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ))
}

fn patterned_rgba(width: u32, height: u32) -> Vec<u8> {
    (0..width as usize * height as usize)
        .flat_map(|i| {
            let x = (i % width as usize) as u8;
            let y = (i / width as usize) as u8;
            [
                x.wrapping_mul(17).wrapping_add(y.wrapping_mul(3)),
                x.wrapping_mul(5).wrapping_add(y.wrapping_mul(19)),
                x.wrapping_mul(11).wrapping_add(y.wrapping_mul(7)),
                if i % 5 == 0 { 0 } else { 255 },
            ]
        })
        .collect()
}

fn baseline_full_image_decode(
    bytes: &[u8],
) -> core::result::Result<(u32, u32, Vec<u8>, u32), String> {
    if let Some(decoded) = try_load_legacy_rgba8_dds(bytes)? {
        return Ok(decoded);
    }
    let scratch = ScratchImage::load_dds(bytes, DDS_FLAGS::DDS_FLAGS_NONE, None, None)
        .map_err(|err| err.to_string())?;
    let metadata = *scratch.metadata();
    let width = metadata
        .width
        .try_into()
        .map_err(|_| "dds width exceeds u32".to_string())?;
    let height = metadata
        .height
        .try_into()
        .map_err(|_| "dds height exceeds u32".to_string())?;
    let dxgi_format = metadata.format.bits();

    if metadata.is_cubemap()
        && matches!(
            metadata.format,
            DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM
                | DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
                | DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM
                | DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM_SRGB
        )
    {
        let image = scratch
            .image(0, 0, 0)
            .ok_or_else(|| "dds cubemap does not contain a readable base face".to_string())?;
        let rgba = packed_pixels_from_image(
            image,
            true,
            matches!(
                metadata.format,
                DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM
                    | DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM_SRGB
            ),
        )?;
        return Ok((width, height, rgba, dxgi_format));
    }

    if metadata.format == DXGI_FORMAT::DXGI_FORMAT_BC5_UNORM {
        let image = scratch
            .image(0, 0, 0)
            .ok_or_else(|| "dds BC5 image does not contain a readable base mip".to_string())?;
        let rgba = decode_bc5_unorm_image(image)?;
        return Ok((width, height, rgba, dxgi_format));
    }

    let rgba_scratch = if metadata.format == DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM {
        None
    } else if metadata.format.is_compressed() {
        Some(
            scratch
                .decompress(DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM)
                .map_err(|err| err.to_string())?,
        )
    } else {
        Some(
            scratch
                .convert(
                    DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
                    TEX_FILTER_FLAGS::TEX_FILTER_DEFAULT,
                    TEX_THRESHOLD_DEFAULT,
                )
                .map_err(|err| err.to_string())?,
        )
    };
    let image = rgba_scratch
        .as_ref()
        .unwrap_or(&scratch)
        .image(0, 0, 0)
        .ok_or_else(|| "dds conversion did not produce an RGBA image".to_string())?;
    let rgba = packed_pixels_from_image(image, false, false)?;
    Ok((width, height, rgba, dxgi_format))
}

fn assert_decode_and_reencode_parity(bytes: &[u8], label: &str) -> bool {
    let baseline = baseline_full_image_decode(bytes);
    let candidate = dds_base_rgba_bytes(bytes);
    let (baseline, candidate) = match (baseline, candidate) {
        (Ok(baseline), Ok(candidate)) => (baseline, candidate),
        (Err(baseline), Err(candidate)) => {
            assert_eq!(candidate, baseline, "decode error mismatch for {label}");
            eprintln!("matching unsupported decode for {label}: {baseline}");
            return false;
        }
        (baseline, candidate) => panic!(
            "decode result mismatch for {label}: baseline={baseline:?} candidate={candidate:?}"
        ),
    };
    assert_eq!(candidate, baseline, "decoded base mismatch for {label}");

    let baseline_path = temp_path(&format!("{label}_baseline.dds"));
    let candidate_path = temp_path(&format!("{label}_candidate.dds"));
    for (path, decoded) in [(&baseline_path, &baseline), (&candidate_path, &candidate)] {
        write_dds_rgba_image(path, decoded.0, decoded.1, &decoded.2, "BC7_UNORM", true).unwrap();
    }
    assert_eq!(
        std::fs::read(&candidate_path).unwrap(),
        std::fs::read(&baseline_path).unwrap(),
        "re-encoded DDS mismatch for {label}"
    );
    let _ = std::fs::remove_file(baseline_path);
    let _ = std::fs::remove_file(candidate_path);
    true
}

fn write_plain_fixture(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    format: &str,
    generate_mips: bool,
) -> core::result::Result<(), String> {
    if legacy_dxt_target(format).is_some() {
        return write_dds_rgba_image(path, width, height, rgba, format, generate_mips);
    }
    let target =
        parse_dxgi_format(format).ok_or_else(|| format!("unknown fixture format {format}"))?;
    if !matches!(
        target,
        DXGI_FORMAT::DXGI_FORMAT_R8_UNORM
            | DXGI_FORMAT::DXGI_FORMAT_R8G8_UNORM
            | DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM
            | DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM
            | DXGI_FORMAT::DXGI_FORMAT_R16G16B16A16_FLOAT
    ) {
        return write_dds_rgba_image(path, width, height, rgba, format, generate_mips);
    }

    let mut scratch = ScratchImage::default();
    scratch
        .initialize_2d(
            target,
            width as usize,
            height as usize,
            1,
            if generate_mips {
                full_mip_count(width, height) as usize
            } else {
                1
            },
            CP_FLAGS::CP_FLAGS_NONE,
        )
        .map_err(|error| error.to_string())?;
    if target == DXGI_FORMAT::DXGI_FORMAT_R16G16B16A16_FLOAT {
        let finite_half_values = [0x0000u16, 0x3400, 0x3800, 0x3c00];
        for (index, value) in scratch.pixels_mut().chunks_exact_mut(2).enumerate() {
            value.copy_from_slice(
                &finite_half_values[index % finite_half_values.len()].to_le_bytes(),
            );
        }
    } else {
        for (index, value) in scratch.pixels_mut().iter_mut().enumerate() {
            *value = index.wrapping_mul(31).wrapping_add(13) as u8;
        }
    }
    let encoded = scratch
        .save_dds(DDS_FLAGS::DDS_FLAGS_NONE)
        .map_err(|error| error.to_string())?;
    std::fs::write(path, encoded.buffer()).map_err(|error| error.to_string())
}

#[test]
fn base_only_decode_matches_full_image_decode_for_supported_formats() {
    let rgba = patterned_rgba(32, 16);
    for format in [
        "DXT1",
        "DXT5",
        "R8_UNORM",
        "R8G8_UNORM",
        "B8G8R8A8_UNORM",
        "B8G8R8X8_UNORM",
        "R16G16B16A16_FLOAT",
        "BC1_UNORM",
        "BC1_UNORM_SRGB",
        "BC2_UNORM",
        "BC2_UNORM_SRGB",
        "BC3_UNORM",
        "BC3_UNORM_SRGB",
        "BC4_UNORM",
        "BC4_SNORM",
        "BC5_UNORM",
        "BC5_SNORM",
        "BC6H_UF16",
        "BC6H_SF16",
        "BC7_UNORM",
        "BC7_UNORM_SRGB",
        "R8G8B8A8_UNORM",
        "R8G8B8A8_UNORM_SRGB",
    ] {
        for generate_mips in [false, true] {
            let path = temp_path(&format!("source_{format}_{generate_mips}.dds"));
            write_plain_fixture(&path, 32, 16, &rgba, format, generate_mips).unwrap_or_else(
                |error| panic!("fixture writer rejected {format} mips={generate_mips}: {error}"),
            );
            let bytes = std::fs::read(&path).unwrap();
            let decoded =
                assert_decode_and_reencode_parity(&bytes, &format!("{format}_{generate_mips}"));
            assert!(
                decoded || format == "R8_UNORM",
                "plain fixture must decode: {format} mips={generate_mips}"
            );
            let _ = std::fs::remove_file(path);
        }
    }
}

fn complex_dds(kind: &str) -> Vec<u8> {
    let mut source = ScratchImage::default();
    match kind {
        "array" => source
            .initialize_2d(
                DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM,
                16,
                8,
                2,
                3,
                CP_FLAGS::CP_FLAGS_NONE,
            )
            .unwrap(),
        "cubemap" => source
            .initialize_cube(
                DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
                16,
                16,
                1,
                3,
                CP_FLAGS::CP_FLAGS_NONE,
            )
            .unwrap(),
        "volume" | "volume_depth1" => source
            .initialize_3d(
                DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM,
                16,
                8,
                if kind == "volume" { 4 } else { 1 },
                3,
                CP_FLAGS::CP_FLAGS_NONE,
            )
            .unwrap(),
        _ => unreachable!(),
    }
    for (i, byte) in source.pixels_mut().iter_mut().enumerate() {
        *byte = i.wrapping_mul(29).wrapping_add(7) as u8;
    }
    let encoded_source = if kind == "cubemap" {
        source
            .compress(
                DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM,
                TEX_COMPRESS_FLAGS::TEX_COMPRESS_DEFAULT,
                TEX_THRESHOLD_DEFAULT,
            )
            .unwrap_or_else(|error| panic!("complex fixture conversion rejected {kind}: {error}"))
    } else {
        source
    };
    encoded_source
        .save_dds(DDS_FLAGS::DDS_FLAGS_NONE)
        .unwrap()
        .buffer()
        .to_vec()
}

#[test]
fn complex_resources_keep_full_image_fallback() {
    for kind in ["array", "cubemap", "volume", "volume_depth1"] {
        let bytes = complex_dds(kind);
        let metadata = ScratchImage::load_dds(&bytes, DDS_FLAGS::DDS_FLAGS_NONE, None, None)
            .unwrap()
            .metadata()
            .to_owned();
        assert!(
            metadata.dimension != TEX_DIMENSION::TEX_DIMENSION_TEXTURE2D
                || metadata.is_cubemap()
                || metadata.array_size > 1
                || metadata.depth > 1,
            "fixture is not complex: {kind}"
        );
        let _ = assert_decode_and_reencode_parity(&bytes, kind);
    }
}

#[test]
fn malformed_complete_load_errors_before_base_decode() {
    let path = temp_path("truncated_source.dds");
    let rgba = patterned_rgba(32, 32);
    write_dds_rgba_image(&path, 32, 32, &rgba, "BC7_UNORM", true).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len() - 1);
    assert!(baseline_full_image_decode(&bytes).is_err());
    assert!(dds_base_rgba_bytes(&bytes).is_err());
    let _ = std::fs::remove_file(path);
}

#[derive(Clone)]
struct CorpusEntry {
    path: std::path::PathBuf,
    relative: String,
    probe: DdsProbe,
    bytes: u64,
    legacy: bool,
}

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("crate lives at repo/py_creation_lib/native/directxtex")
        .to_path_buf()
}

fn fo76_textures_dir() -> Option<std::path::PathBuf> {
    let extracted = std::env::var("MODBOX_FO76_EXTRACTED")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| repo_root().join("extracted").join("fo76"));
    let textures = extracted.join("Textures");
    textures.is_dir().then_some(textures)
}

fn walk_dds_sorted(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    entries.sort();
    for path in &entries {
        if path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dds"))
        {
            files.push(path.clone());
        }
    }
    for path in entries {
        if path.is_dir() {
            walk_dds_sorted(&path, files);
        }
    }
}

fn stable_path_hash(path: &str) -> u64 {
    path.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn is_supported_corpus_format(format: u32) -> bool {
    matches!(format, 28 | 29 | 71 | 72 | 77 | 78 | 83 | 98 | 99)
}

fn format_family(format: u32) -> u32 {
    match format {
        71 | 72 => 71,
        77 | 78 => 77,
        98 | 99 => 98,
        other => other,
    }
}

fn size_band(probe: DdsProbe) -> usize {
    match probe.width.max(probe.height) {
        0..=256 => 0,
        257..=512 => 1,
        513..=2048 => 2,
        _ => 3,
    }
}

fn total_mip_pixels(probe: DdsProbe) -> u64 {
    let (mut width, mut height, mut depth) = (
        u64::from(probe.width),
        u64::from(probe.height),
        u64::from(probe.depth),
    );
    let mut total = 0;
    for _ in 0..probe.mip_levels {
        total += width * height * depth * u64::from(probe.array_size);
        width = (width / 2).max(1);
        height = (height / 2).max(1);
        depth = (depth / 2).max(1);
    }
    total
}

fn corpus_entries(root: &Path) -> Vec<CorpusEntry> {
    let mut files = Vec::new();
    walk_dds_sorted(root, &mut files);
    let mut entries = Vec::new();
    for path in files {
        let Ok(probe) = read_dds_probe(&path) else {
            continue;
        };
        if !is_supported_corpus_format(probe.dxgi_format) {
            continue;
        }
        let Ok(bytes) = std::fs::metadata(&path).map(|metadata| metadata.len()) else {
            continue;
        };
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let mut header = [0u8; 88];
        let legacy = std::fs::File::open(&path)
            .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut header))
            .is_ok()
            && &header[84..88] != b"DX10";
        entries.push(CorpusEntry {
            path,
            relative,
            probe,
            bytes,
            legacy,
        });
    }
    entries.sort_by_key(|entry| {
        (
            stable_path_hash(&entry.relative.to_ascii_lowercase()),
            entry.relative.clone(),
        )
    });
    entries
}

fn select_corpus(entries: &[CorpusEntry]) -> Vec<CorpusEntry> {
    let mut selected = Vec::<usize>::new();
    let mut seen = std::collections::HashSet::<usize>::new();
    let mut add_first = |predicate: &dyn Fn(&CorpusEntry) -> bool| {
        if let Some((index, _)) = entries
            .iter()
            .enumerate()
            .find(|(index, entry)| !seen.contains(index) && predicate(entry))
        {
            seen.insert(index);
            selected.push(index);
        }
    };

    for family in [71, 77, 83, 98, 28] {
        for band in 0..4 {
            add_first(&|entry| {
                format_family(entry.probe.dxgi_format) == family
                    && size_band(entry.probe) == band
                    && !entry.probe.is_cubemap
                    && entry.probe.array_size == 1
                    && entry.probe.depth <= 1
            });
        }
    }
    for format in [28, 29, 71, 72, 77, 78, 83, 98, 99] {
        add_first(&|entry| entry.probe.dxgi_format == format);
    }
    add_first(&|entry| entry.probe.mip_levels == 1);
    add_first(&|entry| {
        entry.probe.mip_levels > 1
            && entry.probe.mip_levels == full_mip_count(entry.probe.width, entry.probe.height)
    });
    add_first(&|entry| {
        entry.probe.mip_levels > 1
            && entry.probe.mip_levels < full_mip_count(entry.probe.width, entry.probe.height)
    });
    add_first(&|entry| entry.legacy && entry.probe.dxgi_format != 28);
    add_first(&|entry| entry.legacy && entry.probe.dxgi_format == 28);
    add_first(&|entry| entry.probe.is_cubemap);
    add_first(&|entry| entry.probe.array_size > 1 && !entry.probe.is_cubemap);
    add_first(&|entry| entry.probe.depth > 1);
    drop(add_first);

    let by_relative: std::collections::HashMap<_, _> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.relative.to_ascii_lowercase(), index))
        .collect();
    let mut triples = 0;
    for (index, diffuse) in entries.iter().enumerate() {
        if triples == 3 {
            break;
        }
        let lower = diffuse.relative.to_ascii_lowercase();
        let Some(stem) = lower.strip_suffix("_d.dds") else {
            continue;
        };
        let Some(&reflectivity) = by_relative.get(&format!("{stem}_r.dds")) else {
            continue;
        };
        let Some(&lighting) = by_relative.get(&format!("{stem}_l.dds")) else {
            continue;
        };
        for member in [index, reflectivity, lighting] {
            if seen.insert(member) {
                selected.push(member);
            }
        }
        triples += 1;
    }
    for index in 0..entries.len() {
        if selected.len() >= 40 {
            break;
        }
        if seen.insert(index) {
            selected.push(index);
        }
    }
    selected.truncate(48);
    selected
        .into_iter()
        .map(|index| entries[index].clone())
        .collect()
}

fn write_manifest(entries: &[CorpusEntry]) -> std::path::PathBuf {
    let path = repo_root()
        .join("tmp")
        .join("conversion_targets_20260912")
        .join("textures")
        .join("manifest.tsv");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut body = String::from(
        "path\tbytes\twidth\theight\tdepth\tmips\tarray\tdxgi\tcubemap\tlegacy\tbase_pixels\ttotal_mip_pixels\n",
    );
    for entry in entries {
        body.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            entry.relative,
            entry.bytes,
            entry.probe.width,
            entry.probe.height,
            entry.probe.depth,
            entry.probe.mip_levels,
            entry.probe.array_size,
            entry.probe.dxgi_format,
            entry.probe.is_cubemap,
            entry.legacy,
            u64::from(entry.probe.width)
                * u64::from(entry.probe.height)
                * u64::from(entry.probe.depth)
                * u64::from(entry.probe.array_size),
            total_mip_pixels(entry.probe),
        ));
    }
    std::fs::write(&path, body).unwrap();
    path
}

fn timed_baseline(
    path: &Path,
) -> (
    core::result::Result<(u32, u32, Vec<u8>, u32), String>,
    Duration,
) {
    let started = Instant::now();
    let result = std::fs::read(path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| baseline_full_image_decode(&bytes));
    (result, started.elapsed())
}

fn timed_candidate(
    path: &Path,
) -> (
    core::result::Result<(u32, u32, Vec<u8>, u32), String>,
    Duration,
) {
    let started = Instant::now();
    let result = dds_base_rgba(path);
    (result, started.elapsed())
}

#[test]
#[ignore]
fn bench_real_corpus_base_only_decode() {
    let Some(root) = fo76_textures_dir() else {
        eprintln!("skip: extracted FO76 textures are absent");
        return;
    };
    let entries = select_corpus(&corpus_entries(&root));
    assert!(
        (32..=48).contains(&entries.len()),
        "representative corpus has {} files",
        entries.len()
    );
    let manifest = write_manifest(&entries);
    let total_bytes: u64 = entries.iter().map(|entry| entry.bytes).sum();
    let base_pixels: u64 = entries
        .iter()
        .map(|entry| {
            u64::from(entry.probe.width)
                * u64::from(entry.probe.height)
                * u64::from(entry.probe.depth)
                * u64::from(entry.probe.array_size)
        })
        .sum();
    let total_mip_pixels: u64 = entries
        .iter()
        .map(|entry| total_mip_pixels(entry.probe))
        .sum();
    eprintln!(
        "manifest={} files={} compressed_bytes={} base_pixels={} total_mip_pixels={}",
        manifest.display(),
        entries.len(),
        total_bytes,
        base_pixels,
        total_mip_pixels
    );

    for entry in &entries {
        let baseline = timed_baseline(&entry.path).0.unwrap();
        let candidate = timed_candidate(&entry.path).0.unwrap();
        assert_eq!(candidate, baseline, "warmup mismatch: {}", entry.relative);
    }

    for pair in 0..3 {
        let mut baseline_elapsed = Duration::ZERO;
        let mut candidate_elapsed = Duration::ZERO;
        for entry in &entries {
            let (baseline, candidate) = if pair % 2 == 0 {
                let (baseline, elapsed) = timed_baseline(&entry.path);
                baseline_elapsed += elapsed;
                let (candidate, elapsed) = timed_candidate(&entry.path);
                candidate_elapsed += elapsed;
                (baseline, candidate)
            } else {
                let (candidate, elapsed) = timed_candidate(&entry.path);
                candidate_elapsed += elapsed;
                let (baseline, elapsed) = timed_baseline(&entry.path);
                baseline_elapsed += elapsed;
                (baseline, candidate)
            };
            assert_eq!(
                candidate.unwrap(),
                baseline.unwrap(),
                "pair {pair} mismatch: {}",
                entry.relative
            );
        }
        eprintln!(
            "pair={} order={} baseline_s={:.6} candidate_s={:.6} speedup={:.3}x",
            pair + 1,
            if pair % 2 == 0 { "B/C" } else { "C/B" },
            baseline_elapsed.as_secs_f64(),
            candidate_elapsed.as_secs_f64(),
            baseline_elapsed.as_secs_f64() / candidate_elapsed.as_secs_f64(),
        );
    }
}
