use nif_core_native::sf_mesh::read_sf_mesh;
use std::path::PathBuf;

fn starfield_geometries_dir() -> PathBuf {
    let root = std::env::var_os("STARFIELD_EXTRACTED_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/starfield")
        });
    root.join("geometries")
}

#[test]
fn parses_real_starfield_mesh_matching_python_renderer() {
    let path = starfield_geometries_dir().join("0024965a94937a847041/f66e90898c3334f02c7d.mesh");
    if !path.exists() {
        eprintln!("skip: starfield extracted data not present");
        return;
    }
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/sf_mesh_expected.json")).unwrap();
    let mesh = read_sf_mesh(&path).expect("mesh parses");

    assert_eq!(
        mesh.positions.len() as u64,
        expected["vertex_count"].as_u64().unwrap()
    );
    assert_eq!(
        mesh.triangles.len() as u64,
        expected["triangle_count"].as_u64().unwrap()
    );
    for tri in &mesh.triangles {
        assert!(tri.iter().all(|&i| (i as usize) < mesh.positions.len()));
    }

    let first_vertex = mesh.positions[0];
    let expected_first = expected["first_vertex"].as_array().unwrap();
    for i in 0..3 {
        let e = expected_first[i].as_f64().unwrap() as f32;
        assert!(
            (first_vertex[i] - e).abs() < 1e-3,
            "first_vertex[{i}]: got {} want {e}",
            first_vertex[i]
        );
    }

    let bbox_min = [
        mesh.positions.iter().map(|p| p[0]).fold(f32::MAX, f32::min),
        mesh.positions.iter().map(|p| p[1]).fold(f32::MAX, f32::min),
        mesh.positions.iter().map(|p| p[2]).fold(f32::MAX, f32::min),
    ];
    let bbox_max = [
        mesh.positions.iter().map(|p| p[0]).fold(f32::MIN, f32::max),
        mesh.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max),
        mesh.positions.iter().map(|p| p[2]).fold(f32::MIN, f32::max),
    ];
    let expected_min = expected["bbox_min"].as_array().unwrap();
    let expected_max = expected["bbox_max"].as_array().unwrap();
    for i in 0..3 {
        assert!((bbox_min[i] - expected_min[i].as_f64().unwrap() as f32).abs() < 1e-3);
        assert!((bbox_max[i] - expected_max[i].as_f64().unwrap() as f32).abs() < 1e-3);
    }

    assert_eq!(mesh.normals.len(), mesh.positions.len());
    assert_eq!(mesh.uvs.len(), mesh.positions.len());
    // Ground truth for this fixture: the file carries no per-vertex color block.
    assert!(mesh.vertex_colors.is_none());
}

#[test]
fn resolves_real_starfield_geometry_path() {
    let geometries_root = starfield_geometries_dir();
    if !geometries_root.exists() {
        eprintln!("skip: starfield extracted data not present");
        return;
    }
    let resolved = nif_core_native::sf_mesh::resolve_geometry_path(
        &geometries_root,
        r"0024965a94937a847041\f66e90898c3334f02c7d",
    );
    assert!(
        resolved.exists(),
        "resolved path should exist: {}",
        resolved.display()
    );
}
