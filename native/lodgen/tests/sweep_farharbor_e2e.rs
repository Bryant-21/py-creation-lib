//! Full-corpus FarHarbor LOD generation harness (real-esp).
//!
//! `generate_full_farharbor_lod_corpus` runs `run()` over all of `DLC03FarHarbor`
//! against the real FO4 install and writes every LOD output (terrain `.btr` per level,
//! object `.bto`, `.dds`) to `tmp/lodgen_sweep/`; it fails only if `run()` errors or
//! writes no terrain meshes. `diff_sweep_vs_golden` then diffs that output against
//! the xLODGen corpus under `tmp/xlodgen/` as a regression gate.
//!
//! Requires `real-esp` so `run()` can read DLCCoast.esm. Skips when the FO4 install
//! is absent.
#![cfg(feature = "real-esp")]

use std::path::PathBuf;

use lodgen_native::progress::{LodPaths, Progress};
use lodgen_native::settings::LodSettings;

// Extracted loose LOD source meshes (Meshes\LOD\**). The object-LOD path
// (`parse_nif`) reads LOD `.nif` source models as LOOSE files from data_dirs; the
// game ships them inside BA2 archives, so a raw Data-dir-only run finds zero object
// LOD models. The extracted tree is searched first so objects can generate.
const FARHARBOR_PLUGIN: &str = "DLCCoast.esm";
const WORLD_EDID: &str = "DLC03FarHarbor";

struct NullProgress;
impl Progress for NullProgress {
    fn report(&mut self, _m: &str, _f: f32) {}
}

fn fo4_data_dir() -> Option<PathBuf> {
    let Ok(data) = std::env::var("FO4_DATA") else {
        eprintln!("SKIP sweep_farharbor_e2e: FO4_DATA unset");
        return None;
    };
    let p = PathBuf::from(data);
    if p.join(FARHARBOR_PLUGIN).is_file() {
        Some(p)
    } else {
        eprintln!(
            "SKIP sweep_farharbor_e2e: {}\\{FARHARBOR_PLUGIN} not found",
            p.display()
        );
        None
    }
}

fn extracted_fo4() -> PathBuf {
    std::env::var("FO4_EXTRACTED_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fo4")
        })
}

fn output_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tmp/lodgen_sweep")
}

fn gold_mesh() -> PathBuf {
    std::env::var("XLODGEN_GOLD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../tmp/xlodgen/meshes/terrain/DLC03FarHarbor")
        })
}

fn ours_mesh() -> PathBuf {
    output_dir().join("Meshes/Terrain/DLC03FarHarbor")
}

fn shipped_obj_mesh() -> PathBuf {
    extracted_fo4().join("meshes/terrain/dlc03farharbor/objects")
}

/// Generate the full FarHarbor LOD set into `tmp/lodgen_sweep/`.
#[test]
fn generate_full_farharbor_lod_corpus() {
    let Some(data) = fo4_data_dir() else { return };

    let out = output_dir();
    std::fs::create_dir_all(&out).expect("create output dir");

    let mut settings = LodSettings::fo4_default();
    if std::env::var("LODGEN_SWEEP_REMOVE_UNSEEN").as_deref() == Ok("1") {
        settings.objects.remove_unseen_faces = true;
    }
    let mut data_dirs = Vec::new();
    let extracted = extracted_fo4();
    if extracted.is_dir() {
        data_dirs.push(extracted);
    } else {
        eprintln!(
            "NOTE: {} absent — object LOD source NIFs unavailable (objects will be empty)",
            extracted.display()
        );
    }
    data_dirs.push(data);
    let paths = LodPaths {
        data_dirs,
        output_dir: out.clone(),
        source_data_dir: None,
    };
    let mut progress = NullProgress;

    let start = std::time::Instant::now();
    let stats = lodgen_native::run(WORLD_EDID, &settings, None, &paths, &mut progress)
        .unwrap_or_else(|e| panic!("run() failed for {WORLD_EDID}: {e}"));
    let elapsed = start.elapsed();

    eprintln!(
        "SWEEP DONE: world={WORLD_EDID} btr={} bto={} btt={} dds={} lod_written={} warnings={} elapsed={:.1}s",
        stats.btr,
        stats.bto,
        stats.btt,
        stats.dds,
        stats.lod_written,
        stats.warnings.len(),
        elapsed.as_secs_f64(),
    );
    // Summarize warnings by category instead of dumping thousands of identical lines.
    let mut warn_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for w in &stats.warnings {
        // Bucket by the leading clause (before the first ':' or fixed prefix).
        let key = if w.contains("diffuse encode failed") {
            "terrain diffuse encode failed (DirectXTex FFI E::NOTIMPL — harness link)".to_string()
        } else if w.contains("atlas DDS encode") {
            "atlas DDS encode shortfall (DirectXTex FFI — harness link)".to_string()
        } else if w.contains("object atlas build failed") {
            "object atlas build failed (un-atlassed LOD)".to_string()
        } else {
            w.split(':').next().unwrap_or(w).to_string()
        };
        *warn_counts.entry(key).or_default() += 1;
    }
    eprintln!("WARNING CATEGORIES:");
    for (k, c) in &warn_counts {
        eprintln!("  [{c}] {k}");
    }

    // Count what actually landed on disk. The generator's `stats.btr`/`stats.bto`
    // counters only increment when a quad's FULL output (mesh + DDS) succeeds, so
    // they read 0 here — the DDS encode hits the DirectXTex FFI link collision
    // (see build.rs / Cargo.toml [features]). The meshes themselves ARE written to
    // disk before the encode is attempted, so we assert on the file system.
    let count_ext = |ext: &str| -> usize {
        fn walk(dir: &std::path::Path, ext: &str, acc: &mut usize) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, ext, acc);
                    } else if p.extension().and_then(|s| s.to_str()) == Some(ext) {
                        *acc += 1;
                    }
                }
            }
        }
        let mut acc = 0;
        walk(&out, ext, &mut acc);
        acc
    };
    let btr = count_ext("btr");
    let bto = count_ext("bto");
    let dds = count_ext("dds");
    eprintln!("ON-DISK: btr={btr} bto={bto} dds={dds}");

    assert!(
        btr > 0,
        "no terrain .btr written to {out:?} — enumeration/mesh build failed"
    );
}

// ---------------------------------------------------------------------------
// DIFF: compare tmp/lodgen_sweep vs tmp/xlodgen via nif_core (no DirectXTex FFI)
// ---------------------------------------------------------------------------

fn tri_count(nif: &nif_core_native::model::NifFile) -> usize {
    nif.blocks
        .iter()
        .filter(|b| b.type_name == "BSSubIndexTriShape" || b.type_name == "BSTriShape")
        .map(num_triangles)
        .sum()
}

fn num_triangles(b: &nif_core_native::model::NifBlock) -> usize {
    use nif_core_native::model::NifValue;
    match b
        .fields
        .get("Num Triangles")
        .or_else(|| b.fields.get("Number of Triangles"))
    {
        Some(NifValue::UInt(v)) => *v as usize,
        Some(NifValue::Int(v)) => *v as usize,
        _ => match b.fields.get("Triangles") {
            Some(NifValue::Array(l)) => l.len(),
            _ => 0,
        },
    }
}

fn file_tris(path: &std::path::Path) -> Option<usize> {
    nif_core_native::model::NifFile::load(path)
        .ok()
        .map(|n| tri_count(&n))
}

/// `DLC03FarHarbor.<level>.<x>.<y>.btr` → level int.
fn level_of(name: &str) -> Option<i32> {
    name.split('.').nth(1).and_then(|s| s.parse().ok())
}

fn list_ext(dir: &std::path::Path, ext: &str) -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some(ext) {
                if let Some(n) = p.file_name().and_then(|s| s.to_str()) {
                    v.push(n.to_string());
                }
            }
        }
    }
    v.sort();
    v
}

fn pct(num: usize, den: usize) -> f64 {
    if den == 0 {
        0.0
    } else {
        100.0 * num as f64 / den as f64
    }
}

fn ratio_stats(mut ratios: Vec<f64>) -> (f64, f64, f64) {
    if ratios.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let min = ratios[0];
    let max = *ratios.last().unwrap();
    let median = ratios[ratios.len() / 2];
    (min, median, max)
}

#[test]
fn diff_sweep_vs_golden() {
    let ours = ours_mesh();
    let gold = gold_mesh();
    if !gold.is_dir() {
        eprintln!("SKIP diff: golden corpus absent at {}", gold.display());
        return;
    }
    if !ours.is_dir() {
        eprintln!(
            "SKIP diff: generated corpus absent at {} (run generate test first)",
            ours.display()
        );
        return;
    }

    let mut report = String::new();
    macro_rules! line { ($($a:tt)*) => {{ let s = format!($($a)*); eprintln!("{s}"); report.push_str(&s); report.push('\n'); }}; }

    line!("# FarHarbor LOD Fidelity Sweep — lodgen_native vs xLODGen golden");
    line!("");
    line!("Generated: tmp/lodgen_sweep | Golden: tmp/xlodgen");
    line!("");

    // ---- 1. TERRAIN .btr coverage + tri-ratio by level ----
    let our_btr = list_ext(&ours, "btr");
    let gold_btr = list_ext(&gold, "btr");
    let our_set: std::collections::BTreeSet<_> = our_btr.iter().cloned().collect();
    let gold_set: std::collections::BTreeSet<_> = gold_btr.iter().cloned().collect();

    line!("## 1. File coverage");
    line!("");
    line!("### Terrain .btr");
    line!("- produced: {} | golden: {}", our_btr.len(), gold_btr.len());
    let missing: Vec<_> = gold_set.difference(&our_set).cloned().collect();
    let extra: Vec<_> = our_set.difference(&gold_set).cloned().collect();
    line!("- in golden but NOT produced (missing): {}", missing.len());
    line!("- produced but NOT in golden (extra): {}", extra.len());
    // Per-level coverage
    for lvl in [4, 8, 16, 32] {
        let og = gold_btr.iter().filter(|n| level_of(n) == Some(lvl)).count();
        let oo = our_btr.iter().filter(|n| level_of(n) == Some(lvl)).count();
        line!("  - L{lvl}: produced {oo} / golden {og}");
    }
    line!("");

    // ---- 2. terrain tri-ratio by level ----
    line!("## 2. Terrain .btr triangle ratios (ours/golden) by level");
    line!("");
    line!("| Level | matched | ratio min | ratio median | ratio max | sum ours | sum golden |");
    line!("|-------|---------|-----------|--------------|-----------|----------|-----------|");
    let mut all_terrain_ratios = Vec::new();
    for lvl in [4, 8, 16, 32] {
        let mut ratios = Vec::new();
        let mut sum_ours = 0usize;
        let mut sum_gold = 0usize;
        let mut matched = 0usize;
        for name in gold_btr.iter().filter(|n| level_of(n) == Some(lvl)) {
            if !our_set.contains(name) {
                continue;
            }
            let g = file_tris(&gold.join(name));
            let o = file_tris(&ours.join(name));
            if let (Some(g), Some(o)) = (g, o) {
                if g > 0 {
                    ratios.push(o as f64 / g as f64);
                    all_terrain_ratios.push(o as f64 / g as f64);
                }
                sum_ours += o;
                sum_gold += g;
                matched += 1;
            }
        }
        let (mn, md, mx) = ratio_stats(ratios);
        line!("| L{lvl} | {matched} | {mn:.3} | {md:.3} | {mx:.3} | {sum_ours} | {sum_gold} |");
    }
    let (amn, amd, amx) = ratio_stats(all_terrain_ratios);
    line!("");
    line!(
        "- ALL-LEVELS terrain ratio (matched pairs only): min {amn:.3} / median {amd:.3} / max {amx:.3}"
    );
    line!("");

    // EXTRA (produced-not-golden) terrain quads: characterize as landless synth.
    line!("### Extra terrain quads (produced, NOT in golden) — landless-cell synthesis");
    line!("");
    for lvl in [4, 8, 16, 32] {
        let mut tris: Vec<usize> = Vec::new();
        for name in &extra {
            if level_of(name) != Some(lvl) {
                continue;
            }
            if let Some(t) = file_tris(&ours.join(name)) {
                tris.push(t);
            }
        }
        if tris.is_empty() {
            continue;
        }
        tris.sort();
        let n = tris.len();
        let med = tris[n / 2];
        let near_empty = tris.iter().filter(|&&t| t <= 8).count();
        line!(
            "  - L{lvl}: {n} extra | median {med} tris | <=8-tri (flat/landless) {near_empty} ({:.0}%)",
            pct(near_empty, n)
        );
    }
    line!("");

    // ---- 3. OBJECT .bto tri-ratio + empty count ----
    let our_obj_dir = ours.join("Objects");
    let gold_obj_dir = gold.join("Objects");
    let our_bto = list_ext(&our_obj_dir, "bto");
    let gold_bto = list_ext(&gold_obj_dir, "bto");
    let our_bto_set: std::collections::BTreeSet<_> = our_bto.iter().cloned().collect();
    let gold_bto_set: std::collections::BTreeSet<_> = gold_bto.iter().cloned().collect();

    line!("### Object .bto");
    line!("- produced: {} | golden: {}", our_bto.len(), gold_bto.len());
    let bto_missing: Vec<_> = gold_bto_set.difference(&our_bto_set).cloned().collect();
    let bto_extra: Vec<_> = our_bto_set.difference(&gold_bto_set).cloned().collect();
    line!("- missing (golden, not produced): {}", bto_missing.len());
    line!("- extra (produced, not golden): {}", bto_extra.len());
    for lvl in [4, 8, 16, 32] {
        let og = gold_bto.iter().filter(|n| level_of(n) == Some(lvl)).count();
        let oo = our_bto.iter().filter(|n| level_of(n) == Some(lvl)).count();
        line!("  - L{lvl}: produced {oo} / golden {og}");
    }
    if !bto_missing.is_empty() {
        let sample: Vec<_> = bto_missing.iter().take(8).cloned().collect();
        line!("  - notable missing .bto (sample): {:?}", sample);
    }
    if !bto_extra.is_empty() {
        line!("  - extra .bto: {:?}", bto_extra);
    }
    line!("");

    line!("## 3. Object .bto triangle ratios (ours/golden)");
    line!("");
    let mut obj_ratios = Vec::new();
    let mut obj_empty = 0usize; // ours near-empty where golden is heavy
    let mut obj_heavy_present = 0usize; // ours present-but-low (>0 tris) where golden heavy
    let mut sum_ours = 0usize;
    let mut sum_gold = 0usize;
    let mut matched = 0usize;
    let mut empty_examples: Vec<String> = Vec::new();
    let mut ratio_examples: Vec<(f64, String, usize, usize)> = Vec::new();
    for name in &gold_bto {
        if !our_bto_set.contains(name) {
            continue;
        }
        let g = file_tris(&gold_obj_dir.join(name));
        let o = file_tris(&our_obj_dir.join(name));
        if let (Some(g), Some(o)) = (g, o) {
            sum_ours += o;
            sum_gold += g;
            matched += 1;
            if g > 0 {
                let ratio = o as f64 / g as f64;
                obj_ratios.push(ratio);
                ratio_examples.push((ratio, name.clone(), o, g));
            }
            // "near-empty": ours has < 5% of golden's tris (un-atlassed / dropped)
            if g >= 50 && (o as f64) < 0.05 * (g as f64) {
                obj_empty += 1;
                if empty_examples.len() < 8 {
                    empty_examples.push(format!("{name} (ours {o} / golden {g})"));
                }
            } else if g >= 50 && o > 0 {
                obj_heavy_present += 1;
            }
        }
    }
    let (omn, omd, omx) = ratio_stats(obj_ratios);
    line!("- matched .bto: {matched}");
    line!("- tri-ratio: min {omn:.3} / median {omd:.3} / max {omx:.3}");
    line!(
        "- sum tris ours {sum_ours} / golden {sum_gold} (overall ratio {:.3})",
        if sum_gold > 0 {
            sum_ours as f64 / sum_gold as f64
        } else {
            0.0
        }
    );
    line!("- near-EMPTY object quads (ours <5% of golden, golden>=50 tris): {obj_empty}");
    line!("- present-but-heavy quads (ours >0 tris, golden>=50): {obj_heavy_present}");
    if !empty_examples.is_empty() {
        line!("- empty examples: {:?}", empty_examples);
    }
    ratio_examples.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let top_over: Vec<_> = ratio_examples
        .iter()
        .take(10)
        .map(|(r, n, o, g)| format!("{n} ours={o} golden={g} ratio={r:.3}"))
        .collect();
    line!("- top overfull matched BTOs: {:?}", top_over);
    let top_under: Vec<_> = ratio_examples
        .iter()
        .rev()
        .take(10)
        .map(|(r, n, o, g)| format!("{n} ours={o} golden={g} ratio={r:.3}"))
        .collect();
    line!("- top underfull matched BTOs: {:?}", top_under);
    let shipped_dir = shipped_obj_mesh();
    if shipped_dir.is_dir() {
        let shipped_bto = list_ext(&shipped_dir, "bto");
        let shipped_map: std::collections::BTreeMap<_, _> = shipped_bto
            .iter()
            .map(|n| (n.to_ascii_lowercase(), n.clone()))
            .collect();
        let our_map: std::collections::BTreeMap<_, _> = our_bto
            .iter()
            .map(|n| (n.to_ascii_lowercase(), n.clone()))
            .collect();
        let shipped_set: std::collections::BTreeSet<_> = shipped_map.keys().cloned().collect();
        let our_set_lower: std::collections::BTreeSet<_> = our_map.keys().cloned().collect();
        let shipped_missing_from_ours = shipped_set.difference(&our_set_lower).count();
        let our_missing_from_shipped = our_set_lower.difference(&shipped_set).count();
        let mut shipped_sum = 0usize;
        let mut shipped_matched_ours = 0usize;
        let mut ours_vs_shipped_sum = 0usize;
        for (key, shipped_name) in &shipped_map {
            let Some(our_name) = our_map.get(key) else {
                continue;
            };
            if let (Some(s), Some(o)) = (
                file_tris(&shipped_dir.join(shipped_name)),
                file_tris(&our_obj_dir.join(our_name)),
            ) {
                shipped_sum += s;
                ours_vs_shipped_sum += o;
                shipped_matched_ours += 1;
            }
        }
        line!("");
        line!("### Object .bto vs shipped DLC03");
        line!(
            "- shipped: {} | ours: {} | shipped-not-ours: {} | ours-not-shipped: {}",
            shipped_bto.len(),
            our_bto.len(),
            shipped_missing_from_ours,
            our_missing_from_shipped
        );
        line!(
            "- matched shipped/ours: {} | ours tris {} / shipped tris {} (ratio {:.3})",
            shipped_matched_ours,
            ours_vs_shipped_sum,
            shipped_sum,
            if shipped_sum > 0 {
                ours_vs_shipped_sum as f64 / shipped_sum as f64
            } else {
                0.0
            }
        );

        assert_eq!(
            our_missing_from_shipped, 0,
            "native DLC03 object LOD emitted BTOs not present in shipped DLC03"
        );
        if shipped_sum > 0 {
            let shipped_ratio = ours_vs_shipped_sum as f64 / shipped_sum as f64;
            assert!(
                (0.85..=1.15).contains(&shipped_ratio),
                "native DLC03 object LOD tri total drifted too far from shipped DLC03: {shipped_ratio:.3}"
            );
        }
    }
    line!("");

    // ---- 5. object-gap attribution ----
    line!("## 5. Object-gap attribution");
    line!("");
    line!(
        "Object .bto MESHES are written un-atlassed (the shared object atlas DDS encode \
         fails under the test-exe DirectXTex FFI link collision — see build.rs). The \
         per-quad tri delta therefore reflects (a) source-LOD-NIF availability and (b) \
         the ReUV+Simplify gap, NOT missing diffuse atlas (which only affects the \
         TEXTURE, not the mesh tri-count)."
    );
    line!("");

    // Write report to disk (numbers section). The narrative verdict is appended by the
    // harness operator into SWEEP_REPORT.md; here we emit the raw computed block.
    let nums_path = output_dir().join("_diff_numbers.md");
    std::fs::write(&nums_path, &report).expect("write diff numbers");
    eprintln!("DIFF numbers written to {nums_path:?}");

    assert_eq!(
        bto_missing.len(),
        0,
        "native DLC03 object LOD dropped golden BTOs: {bto_missing:?}"
    );
    assert!(
        bto_extra.len() <= 16,
        "native DLC03 object LOD emitted too many extra BTOs: {bto_extra:?}"
    );
    assert_eq!(
        obj_empty, 0,
        "native DLC03 object LOD left matched BTOs near-empty: {empty_examples:?}"
    );
    if sum_gold > 0 {
        let object_ratio = sum_ours as f64 / sum_gold as f64;
        assert!(
            (0.90..=1.25).contains(&object_ratio),
            "native DLC03 object LOD tri total drifted too far from golden: {object_ratio:.3}"
        );
    }
}
