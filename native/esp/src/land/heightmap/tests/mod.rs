// Tests for esp_authoring_core::land::heightmap
use super::*;

#[test]
fn vhgt_roundtrip_synthetic() {
    let mut bytes = Vec::with_capacity(1096);
    bytes.extend_from_slice(&100.0f32.to_le_bytes());
    for r in 0..33u8 {
        for c in 0..33u8 {
            bytes.push(((r as i8) ^ (c as i8)) as u8);
        }
    }
    bytes.extend_from_slice(&[0u8; 3]);
    assert_eq!(bytes.len(), 1096);
    let parsed = parse_heightmap(&bytes).expect("parse");
    assert_eq!(parsed.base, 100.0);
    assert_eq!(parsed.deltas[5][7], 5i8 ^ 7i8);
    let written = write_heightmap(&parsed);
    assert_eq!(written, bytes);
}

#[test]
fn vhgt_rejects_short_payload() {
    let bytes = vec![0u8; 1095];
    assert!(matches!(
        parse_heightmap(&bytes),
        Err(LandError::VhgtTruncated(1095))
    ));
}

#[test]
fn vhgt_rejects_long_payload() {
    let bytes = vec![0u8; 1097];
    assert!(matches!(
        parse_heightmap(&bytes),
        Err(LandError::VhgtTrailing(1))
    ));
}

#[test]
fn vnml_roundtrip_synthetic() {
    let mut bytes = Vec::with_capacity(3267);
    for r in 0..33u8 {
        for c in 0..33u8 {
            bytes.push(((r as i8).wrapping_add(c as i8)) as u8);
            bytes.push(((r as i8).wrapping_sub(c as i8)) as u8);
            bytes.push(((r as i8) ^ (c as i8)) as u8);
        }
    }
    assert_eq!(bytes.len(), 3267);
    let parsed = parse_vertex_normals(&bytes).expect("parse");
    let (nx, ny, nz) = parsed.normals[5][7];
    assert_eq!(nx, 5i8.wrapping_add(7));
    assert_eq!(ny, 5i8.wrapping_sub(7));
    assert_eq!(nz, 5i8 ^ 7i8);
    let written = write_vertex_normals(&parsed);
    assert_eq!(written, bytes);
}

#[test]
fn vnml_rejects_short_payload() {
    let bytes = vec![0u8; 3266];
    assert!(matches!(
        parse_vertex_normals(&bytes),
        Err(LandError::VnmlTruncated(3266))
    ));
}

#[test]
fn vnml_rejects_long_payload() {
    let bytes = vec![0u8; 3268];
    assert!(matches!(
        parse_vertex_normals(&bytes),
        Err(LandError::VnmlTrailing(1))
    ));
}

macro_rules! vhgt_corpus_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            let bytes = include_bytes!($path);
            let parsed = parse_heightmap(bytes).expect("parse");
            let written = write_heightmap(&parsed);
            assert_eq!(&written[..], &bytes[..], "VHGT byte-roundtrip mismatch");
        }
    };
}

macro_rules! vnml_corpus_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            let bytes = include_bytes!($path);
            let parsed = parse_vertex_normals(bytes).expect("parse");
            let written = write_vertex_normals(&parsed);
            assert_eq!(&written[..], &bytes[..], "VNML byte-roundtrip mismatch");
        }
    };
}

vhgt_corpus_test!(vhgt_corpus_roundtrip_00f53c, "fixtures/fo4_00f53c.vhgt.bin");
vhgt_corpus_test!(vhgt_corpus_roundtrip_00f53d, "fixtures/fo4_00f53d.vhgt.bin");
vhgt_corpus_test!(vhgt_corpus_roundtrip_00f53e, "fixtures/fo4_00f53e.vhgt.bin");
vhgt_corpus_test!(vhgt_corpus_roundtrip_00f53f, "fixtures/fo4_00f53f.vhgt.bin");
vhgt_corpus_test!(vhgt_corpus_roundtrip_00f540, "fixtures/fo4_00f540.vhgt.bin");

vnml_corpus_test!(vnml_corpus_roundtrip_00f53c, "fixtures/fo4_00f53c.vnml.bin");
vnml_corpus_test!(vnml_corpus_roundtrip_00f53d, "fixtures/fo4_00f53d.vnml.bin");
vnml_corpus_test!(vnml_corpus_roundtrip_00f53e, "fixtures/fo4_00f53e.vnml.bin");
vnml_corpus_test!(vnml_corpus_roundtrip_00f53f, "fixtures/fo4_00f53f.vnml.bin");
vnml_corpus_test!(vnml_corpus_roundtrip_00f540, "fixtures/fo4_00f540.vnml.bin");

// ---------------------------------------------------------------------------
// YAML codec roundtrip
// ---------------------------------------------------------------------------

#[test]
fn vhgt_yaml_roundtrip_synthetic() {
    let mut deltas = [[0i8; 33]; 33];
    for r in 0..33 {
        for c in 0..33 {
            deltas[r][c] = (r as i8) ^ (c as i8);
        }
    }
    let map = LandHeightMap {
        base: 100.0,
        deltas,
    };
    let yaml = heightmap_to_yaml(&map);
    let parsed_back = heightmap_from_yaml(&yaml).expect("from_yaml");
    assert_eq!(parsed_back, map);
}

#[test]
fn vnml_yaml_roundtrip_synthetic() {
    let mut normals = [[(0i8, 0i8, 0i8); 33]; 33];
    for r in 0..33 {
        for c in 0..33 {
            normals[r][c] = (
                (r as i8).wrapping_add(c as i8),
                (r as i8).wrapping_sub(c as i8),
                (r as i8) ^ (c as i8),
            );
        }
    }
    let n = LandVertexNormals { normals };
    let yaml = vertex_normals_to_yaml(&n);
    let parsed_back = vertex_normals_from_yaml(&yaml).expect("from_yaml");
    assert_eq!(parsed_back, n);
}

macro_rules! vhgt_yaml_corpus_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            let bytes = include_bytes!($path);
            let parsed = parse_heightmap(bytes).expect("parse");
            let yaml = heightmap_to_yaml(&parsed);
            let from_yaml = heightmap_from_yaml(&yaml).expect("from_yaml");
            assert_eq!(from_yaml, parsed, "yaml roundtrip mismatch");
            let rewritten = write_heightmap(&from_yaml);
            assert_eq!(
                &rewritten[..],
                &bytes[..],
                "VHGT byte-roundtrip mismatch after yaml roundtrip"
            );
        }
    };
}

macro_rules! vnml_yaml_corpus_test {
    ($name:ident, $path:literal) => {
        #[test]
        fn $name() {
            let bytes = include_bytes!($path);
            let parsed = parse_vertex_normals(bytes).expect("parse");
            let yaml = vertex_normals_to_yaml(&parsed);
            let from_yaml = vertex_normals_from_yaml(&yaml).expect("from_yaml");
            assert_eq!(from_yaml, parsed, "yaml roundtrip mismatch");
            let rewritten = write_vertex_normals(&from_yaml);
            assert_eq!(
                &rewritten[..],
                &bytes[..],
                "VNML byte-roundtrip mismatch after yaml roundtrip"
            );
        }
    };
}

vhgt_yaml_corpus_test!(vhgt_yaml_roundtrip_00f53c, "fixtures/fo4_00f53c.vhgt.bin");
vhgt_yaml_corpus_test!(vhgt_yaml_roundtrip_00f53d, "fixtures/fo4_00f53d.vhgt.bin");
vhgt_yaml_corpus_test!(vhgt_yaml_roundtrip_00f53e, "fixtures/fo4_00f53e.vhgt.bin");
vhgt_yaml_corpus_test!(vhgt_yaml_roundtrip_00f53f, "fixtures/fo4_00f53f.vhgt.bin");
vhgt_yaml_corpus_test!(vhgt_yaml_roundtrip_00f540, "fixtures/fo4_00f540.vhgt.bin");

vnml_yaml_corpus_test!(vnml_yaml_roundtrip_00f53c, "fixtures/fo4_00f53c.vnml.bin");
vnml_yaml_corpus_test!(vnml_yaml_roundtrip_00f53d, "fixtures/fo4_00f53d.vnml.bin");
vnml_yaml_corpus_test!(vnml_yaml_roundtrip_00f53e, "fixtures/fo4_00f53e.vnml.bin");
vnml_yaml_corpus_test!(vnml_yaml_roundtrip_00f53f, "fixtures/fo4_00f53f.vnml.bin");
vnml_yaml_corpus_test!(vnml_yaml_roundtrip_00f540, "fixtures/fo4_00f540.vnml.bin");
