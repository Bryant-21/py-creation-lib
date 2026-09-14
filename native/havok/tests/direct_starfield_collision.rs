use std::path::{Path, PathBuf};

use havok_native::collision::direct_starfield::decode_starfield_collision;
use havok_native::collision::{
    BuildOptions, CompoundChildKind, MultiBodyShape, build_fo4_multi_body_collision,
};

const SAMPLES: &str = include_str!("fixtures/starfield_tag0_samples.txt");

struct Sample {
    label: String,
    payload: Vec<u8>,
}

fn starfield_extracted_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("STARFIELD_EXTRACTED_DIR") {
        return Some(PathBuf::from(path));
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/starfield");
    path.exists().then_some(path)
}

fn load_sample_payloads() -> Vec<Sample> {
    let Some(root) = starfield_extracted_dir() else {
        return Vec::new();
    };
    let mut samples = Vec::new();
    for line in SAMPLES.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split('|');
        let relative_path = parts.next().expect("sample path");
        let path = root.join(relative_path);
        let offset: usize = parts
            .next()
            .expect("sample offset")
            .parse()
            .expect("offset");
        let length: usize = parts
            .next()
            .expect("sample length")
            .parse()
            .expect("length");
        let label = parts.next().unwrap_or("");
        if !path.exists() {
            continue;
        }
        let bytes = std::fs::read(&path).expect("sample nif reads");
        assert!(
            offset + length <= bytes.len(),
            "{}: sample range ends at {} but the file is {} bytes",
            path.display(),
            offset + length,
            bytes.len()
        );
        assert_eq!(
            &bytes[offset + 4..offset + 8],
            b"TAG0",
            "{}: sample offset does not point at a TAG0 section header",
            path.display()
        );
        samples.push(Sample {
            label: format!("{label} [{}]", path.display()),
            payload: bytes[offset..offset + length].to_vec(),
        });
    }
    samples
}

fn assert_finite_vertices(vertices: &[[f32; 3]], context: &str) {
    assert!(!vertices.is_empty(), "{context}: no vertices");
    for vertex in vertices {
        assert!(
            vertex.iter().all(|value| value.is_finite()),
            "{context}: non-finite vertex {vertex:?}"
        );
    }
    let spans_volume = (0..3).any(|axis| {
        let min = vertices.iter().map(|v| v[axis]).fold(f32::MAX, f32::min);
        let max = vertices.iter().map(|v| v[axis]).fold(f32::MIN, f32::max);
        max - min > 1e-5
    });
    assert!(spans_volume, "{context}: degenerate (zero-extent) geometry");
}

fn assert_mesh_is_sane(vertices: &[[f32; 3]], triangles: &[[u32; 3]], context: &str) {
    assert_finite_vertices(vertices, context);
    assert!(!triangles.is_empty(), "{context}: mesh has no triangles");
    for triangle in triangles {
        assert!(
            triangle
                .iter()
                .all(|index| (*index as usize) < vertices.len()),
            "{context}: triangle {triangle:?} indexes past {} vertices",
            vertices.len()
        );
        assert!(
            triangle[0] != triangle[1] && triangle[1] != triangle[2] && triangle[0] != triangle[2],
            "{context}: degenerate triangle {triangle:?}"
        );
    }
}

fn assert_shape_is_sane(shape: &MultiBodyShape, context: &str) {
    match shape {
        MultiBodyShape::SourcePolytope { shape } => {
            assert_finite_vertices(&shape.vertices, context);
            assert!(
                !shape.planes.is_empty(),
                "{context}: polytope has no planes"
            );
            assert!(!shape.faces.is_empty(), "{context}: polytope has no faces");
            assert!(
                shape.convex_radius.is_finite() && shape.convex_radius >= 0.0,
                "{context}: bad convex radius {}",
                shape.convex_radius
            );
        }
        MultiBodyShape::CompressedMesh {
            vertices,
            triangles,
        } => assert_mesh_is_sane(vertices, triangles, context),
        MultiBodyShape::Compound { children } => {
            assert!(!children.is_empty(), "{context}: compound has no children");
            for (index, child) in children.iter().enumerate() {
                assert!(
                    child
                        .transform
                        .iter()
                        .flatten()
                        .all(|value| value.is_finite()),
                    "{context}: child {index} has a non-finite transform"
                );
                let child_context = format!("{context} child {index}");
                match &child.kind {
                    CompoundChildKind::SourcePolytope { shape } => {
                        assert_finite_vertices(&shape.vertices, &child_context);
                    }
                    CompoundChildKind::Polytope { vertices } => {
                        assert_finite_vertices(vertices, &child_context);
                    }
                    CompoundChildKind::CompressedMesh {
                        vertices,
                        triangles,
                    } => assert_mesh_is_sane(vertices, triangles, &child_context),
                }
            }
        }
        MultiBodyShape::Sphere { radius, position } => {
            assert!(radius.is_finite() && *radius > 0.0, "{context}: bad radius");
            assert!(
                position.iter().all(|value| value.is_finite()),
                "{context}: non-finite sphere centre"
            );
        }
        other => panic!("{context}: unexpected decoded shape variant {other:?}"),
    }
}

#[test]
fn decodes_real_starfield_collision_shapes() {
    let samples = load_sample_payloads();
    if samples.is_empty() {
        eprintln!("skip: starfield extracted data not present");
        return;
    }
    for sample in &samples {
        let shapes = decode_starfield_collision(&sample.payload)
            .unwrap_or_else(|error| panic!("{}: {error}", sample.label));
        assert!(!shapes.is_empty(), "{}: decoded no shapes", sample.label);
        for (index, shape) in shapes.iter().enumerate() {
            assert_shape_is_sane(shape, &format!("{} body {index}", sample.label));
        }
    }
}

#[test]
fn decodes_each_sampled_shape_family() {
    let samples = load_sample_payloads();
    if samples.is_empty() {
        eprintln!("skip: starfield extracted data not present");
        return;
    }
    let mut saw_polytope = false;
    let mut saw_compressed_mesh = false;
    let mut saw_compound = false;
    let mut saw_sphere = false;
    for sample in &samples {
        for shape in decode_starfield_collision(&sample.payload)
            .unwrap_or_else(|error| panic!("{}: {error}", sample.label))
        {
            match shape {
                MultiBodyShape::SourcePolytope { .. } => saw_polytope = true,
                MultiBodyShape::CompressedMesh { .. } => saw_compressed_mesh = true,
                MultiBodyShape::Compound { .. } => saw_compound = true,
                MultiBodyShape::Sphere { .. } => saw_sphere = true,
                _ => {}
            }
        }
    }
    assert!(saw_polytope, "no convex-vertices shape decoded");
    assert!(saw_compressed_mesh, "no compressed mesh shape decoded");
    assert!(saw_compound, "no static compound decoded");
    assert!(saw_sphere, "no sphere primitive decoded");
}

fn static_build_options() -> BuildOptions {
    BuildOptions {
        layer: 1,
        mass: 0.0,
        ..BuildOptions::default()
    }
}

/// Two structural limitations of the shared FO4 writer that decoded Starfield
/// graphs can legitimately hit. Both are properties of the FO4 target format,
/// not decode failures, so the decoder reports the source faithfully and the
/// caller treats the re-encode error as its fallback trigger.
const KNOWN_FO4_WRITER_LIMITS: &[&str] = &[
    "Mixed-kind compound shapes not supported",
    "body order violation",
];

/// The decoded model is only useful if the shared FO4 writer accepts it
/// unchanged — that is the whole point of returning `MultiBodyShape`.
#[test]
fn decoded_shapes_re_encode_through_the_shared_fo4_writer() {
    let samples = load_sample_payloads();
    if samples.is_empty() {
        eprintln!("skip: starfield extracted data not present");
        return;
    }
    let mut re_encoded = 0usize;
    for sample in &samples {
        let shapes = decode_starfield_collision(&sample.payload)
            .unwrap_or_else(|error| panic!("{}: {error}", sample.label));
        match build_fo4_multi_body_collision(&shapes, &static_build_options(), None, None) {
            Ok(blob) => {
                assert!(
                    blob.len() > 128,
                    "{}: re-encoded collision is implausibly small ({} bytes)",
                    sample.label,
                    blob.len()
                );
                re_encoded += 1;
            }
            Err(error) => {
                let message = error.to_string();
                assert!(
                    KNOWN_FO4_WRITER_LIMITS
                        .iter()
                        .any(|limit| message.contains(limit)),
                    "{}: unexpected re-encode failure: {message}",
                    sample.label
                );
            }
        }
    }
    assert!(
        re_encoded * 10 >= samples.len() * 8,
        "only {re_encoded} of {} samples re-encoded",
        samples.len()
    );
}

fn tag0_payloads(bytes: &[u8]) -> Vec<&[u8]> {
    let mut payloads = Vec::new();
    let mut cursor = 0;
    while let Some(magic) = bytes[cursor..]
        .windows(4)
        .position(|window| window == b"TAG0")
        .map(|offset| cursor + offset)
    {
        cursor = magic + 1;
        if magic < 4 {
            continue;
        }
        let start = magic - 4;
        let header = u32::from_be_bytes(bytes[start..start + 4].try_into().unwrap());
        if header >> 30 != 0 {
            continue;
        }
        let size = (header & 0x3FFF_FFFF) as usize;
        if size >= 32 && start + size <= bytes.len() {
            payloads.push(&bytes[start..start + size]);
        }
    }
    payloads
}

fn nifs_under(root: &Path, groups: &[&str], per_group_limit: usize) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for group in groups {
        let mut stack = vec![root.join(group)];
        let before = files.len();
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|extension| extension == "nif") {
                    files.push(path);
                }
            }
            if files.len() - before >= per_group_limit {
                break;
            }
        }
    }
    files
}

/// Sweeps real world content so a layout regression surfaces as a coverage drop
/// rather than as silently-missing collision. Cloth payloads (`hclClothData`
/// roots, no `hknpPhysicsSystemData`) are counted apart: they are a different
/// subsystem this decoder is not meant to handle.
#[test]
fn decodes_the_bulk_of_real_world_collision() {
    let Some(root) = starfield_extracted_dir().map(|path| path.join("meshes")) else {
        eprintln!("skip: starfield extracted data not present");
        return;
    };
    let files = nifs_under(
        &root,
        &[
            "architecture",
            "setdressing",
            "furniture",
            "items",
            "ships",
            "landscape",
            "outpost",
        ],
        120,
    );
    let mut decoded = 0usize;
    let mut re_encoded = 0usize;
    let mut failed = 0usize;
    let mut cloth = 0usize;
    let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        for payload in tag0_payloads(&bytes) {
            match decode_starfield_collision(payload) {
                Ok(shapes) => {
                    assert!(!shapes.is_empty(), "{}: decoded no shapes", path.display());
                    decoded += 1;
                    match build_fo4_multi_body_collision(
                        &shapes,
                        &static_build_options(),
                        None,
                        None,
                    ) {
                        Ok(_) => re_encoded += 1,
                        Err(error) => {
                            let message = error.to_string();
                            assert!(
                                KNOWN_FO4_WRITER_LIMITS
                                    .iter()
                                    .any(|limit| message.contains(limit)),
                                "{}: unexpected re-encode failure: {message}",
                                path.display()
                            );
                        }
                    }
                }
                Err(error) if error.contains("no hknpPhysicsSystemData") => cloth += 1,
                Err(error) => {
                    failed += 1;
                    *reasons.entry(error).or_default() += 1;
                }
            }
        }
    }
    let collision = decoded + failed;
    for (reason, count) in &reasons {
        eprintln!("  {count:5}  {reason}");
    }
    eprintln!(
        "collision={collision} decoded={decoded} re-encoded={re_encoded} cloth-skipped={cloth}"
    );
    assert!(
        collision > 400,
        "sweep only found {collision} collision payloads; the corpus moved"
    );
    assert!(
        decoded * 100 >= collision * 95,
        "decoded only {decoded} of {collision} collision payloads"
    );
    assert!(
        re_encoded * 100 >= collision * 90,
        "only {re_encoded} of {collision} payloads survived the FO4 re-encode"
    );
}

#[test]
fn rejects_fo76_era_payloads() {
    let fo76 = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../extracted/fo76/meshes/setdressing/tireswing/tireswing01.nif"
    ));
    if !fo76.exists() {
        eprintln!("skip: extracted fo76 data not present");
        return;
    }
    let bytes = std::fs::read(fo76).expect("fo76 nif reads");
    let Some(payload) = tag0_payloads(&bytes).into_iter().next() else {
        eprintln!("skip: fo76 fixture carries no TAG0 payload");
        return;
    };
    let error = decode_starfield_collision(payload)
        .expect_err("fo76 payloads must not decode as starfield");
    assert!(error.contains("SDK version"), "{error}");
}
