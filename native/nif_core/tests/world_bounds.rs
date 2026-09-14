use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use nif_core_native::model::{NifFile, NifValue};
use nif_core_native::world_bounds::{
    WorldBounds, WorldBoundsError, aggregate_named_node_world_bounds, aggregate_render_world_bounds,
};

fn fields<const N: usize>(values: [(&str, NifValue); N]) -> IndexMap<String, NifValue> {
    values
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect()
}

fn identity() -> NifValue {
    NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]])
}

fn vertex(position: [f32; 3]) -> NifValue {
    NifValue::Struct(fields([
        ("Vertex", NifValue::Vec3(position)),
        ("Bitangent X", NifValue::Float(0.0)),
        (
            "UV",
            NifValue::Struct(fields([
                ("u", NifValue::Float(0.0)),
                ("v", NifValue::Float(0.0)),
            ])),
        ),
        ("Normal", NifValue::Vec3([0.0, 0.0, 1.0])),
        ("Bitangent Y", NifValue::Float(1.0)),
        ("Tangent", NifValue::Vec3([1.0, 0.0, 0.0])),
        ("Bitangent Z", NifValue::Float(0.0)),
    ]))
}

fn set_transform(
    nif: &mut NifFile,
    block_id: usize,
    translation: [f32; 3],
    rotation: [[f32; 3]; 3],
    scale: f32,
) {
    let block = nif.blocks.get_mut(block_id).expect("block");
    block.set_field("Translation", NifValue::Vec3(translation));
    block.set_field("Rotation", NifValue::Matrix33(rotation));
    block.set_field("Scale", NifValue::Float(scale as f64));
}

fn set_children(nif: &mut NifFile, block_id: usize, children: &[usize]) {
    let block = nif.blocks.get_mut(block_id).expect("node");
    block.set_field("Num Children", NifValue::UInt(children.len() as u64));
    block.set_field(
        "Children",
        NifValue::Array(
            children
                .iter()
                .map(|child| NifValue::Ref(*child as i32))
                .collect(),
        ),
    );
}

fn save_temp(mut nif: NifFile) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("bounds.nif");
    nif.save(Some(path.clone())).expect("save NIF");
    (temp, path)
}

fn assert_bounds(actual: WorldBounds, min: [f32; 3], max: [f32; 3]) {
    for axis in 0..3 {
        assert!((actual.min[axis] - min[axis]).abs() < 1e-5, "{actual:?}");
        assert!((actual.max[axis] - max[axis]).abs() < 1e-5, "{actual:?}");
    }
}

#[test]
fn render_bounds_compose_reachable_node_and_geometry_transforms() {
    let mut nif = NifFile::new("fo4");
    nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
    let node = nif.add_block(
        "NiNode",
        Some(fields([("Name", NifValue::String("Bone".to_string()))])),
    );
    let shape = nif.add_block(
        "BSTriShape",
        Some(fields([
            ("Name", NifValue::String("Body".to_string())),
            ("Vertex Desc", NifValue::Int(193_514_046_685_700)),
            ("Num Vertices", NifValue::UInt(2)),
            ("Num Triangles", NifValue::UInt(0)),
            ("Data Size", NifValue::UInt(0)),
            (
                "Vertex Data",
                NifValue::Array(vec![vertex([0.0, 0.0, 0.0]), vertex([1.0, 2.0, 0.0])]),
            ),
        ])),
    );
    set_children(&mut nif, 0, &[node]);
    set_children(&mut nif, node, &[shape]);
    set_transform(
        &mut nif,
        node,
        [10.0, 0.0, 0.0],
        [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        2.0,
    );
    set_transform(
        &mut nif,
        shape,
        [0.0, 1.0, 0.0],
        match identity() {
            NifValue::Matrix33(value) => value,
            _ => unreachable!(),
        },
        1.0,
    );
    nif.header.footer_roots = vec![0];

    let (_temp, path) = save_temp(nif);
    assert_bounds(
        aggregate_render_world_bounds(path).expect("render bounds"),
        [4.0, 0.0, 0.0],
        [8.0, 2.0, 0.0],
    );
}

#[test]
fn named_node_bounds_use_only_reachable_node_origins() {
    let mut nif = NifFile::new("fo4");
    nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
    let child = nif.add_block(
        "NiNode",
        Some(fields([("Name", NifValue::String("Child".to_string()))])),
    );
    let geometry = nif.add_block(
        "BSTriShape",
        Some(fields([
            ("Name", NifValue::String("IgnoredGeometry".to_string())),
            ("Vertex Desc", NifValue::Int(193_514_046_685_700)),
            ("Num Vertices", NifValue::UInt(1)),
            ("Num Triangles", NifValue::UInt(0)),
            ("Data Size", NifValue::UInt(0)),
            ("Vertex Data", NifValue::Array(vec![vertex([9999.0; 3])])),
        ])),
    );
    set_children(&mut nif, 0, &[child, geometry]);
    set_transform(
        &mut nif,
        0,
        [1.0, 2.0, 3.0],
        match identity() {
            NifValue::Matrix33(value) => value,
            _ => unreachable!(),
        },
        2.0,
    );
    set_transform(
        &mut nif,
        child,
        [2.0, 0.0, 0.0],
        match identity() {
            NifValue::Matrix33(value) => value,
            _ => unreachable!(),
        },
        1.0,
    );
    nif.header.footer_roots = vec![0];

    let (_temp, path) = save_temp(nif);
    assert_bounds(
        aggregate_named_node_world_bounds(path).expect("node bounds"),
        [1.0, 2.0, 3.0],
        [5.0, 2.0, 3.0],
    );
}

#[test]
fn empty_render_tree_is_typed() {
    let mut nif = NifFile::new("fo4");
    nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
    nif.header.footer_roots = vec![0];
    let (_temp, path) = save_temp(nif);

    assert_eq!(
        aggregate_render_world_bounds(path),
        Err(WorldBoundsError::EmptyGeometry)
    );
}

#[test]
fn named_node_tree_rejects_empty_names() {
    let mut nif = NifFile::new("fo4");
    nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
    let child = nif.add_block("NiNode", None);
    set_children(&mut nif, 0, &[child]);
    nif.header.footer_roots = vec![0];
    let (_temp, path) = save_temp(nif);

    assert_eq!(
        aggregate_named_node_world_bounds(path),
        Err(WorldBoundsError::EmptyNodeName { block_id: child })
    );
}

#[test]
fn named_node_tree_rejects_multiple_roots() {
    let mut nif = NifFile::new("fo4");
    nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
    let second = nif.add_block(
        "NiNode",
        Some(fields([("Name", NifValue::String("Second".to_string()))])),
    );
    nif.header.footer_roots = vec![0, second as i32];
    let (_temp, path) = save_temp(nif);

    assert_eq!(
        aggregate_named_node_world_bounds(path),
        Err(WorldBoundsError::RootCount { count: 2 })
    );
}

#[test]
fn named_node_tree_rejects_nonfinite_transforms() {
    let mut nif = NifFile::new("fo4");
    nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
    nif.blocks[0].set_field("Scale", NifValue::Float(f64::INFINITY));
    nif.header.footer_roots = vec![0];
    let (_temp, path) = save_temp(nif);

    assert_eq!(
        aggregate_named_node_world_bounds(path),
        Err(WorldBoundsError::NonFiniteTransform {
            block_id: 0,
            field: "Scale"
        })
    );
}

#[test]
fn reachable_cycle_is_typed() {
    let mut nif = NifFile::new("fo4");
    nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
    let child = nif.add_block(
        "NiNode",
        Some(fields([("Name", NifValue::String("Child".to_string()))])),
    );
    set_children(&mut nif, 0, &[child]);
    set_children(&mut nif, child, &[0]);
    nif.header.footer_roots = vec![0];
    let (_temp, path) = save_temp(nif);

    assert_eq!(
        aggregate_named_node_world_bounds(path),
        Err(WorldBoundsError::Cycle { block_id: 0 })
    );
}

#[test]
fn supported_geometry_without_vertices_is_typed() {
    let mut nif = NifFile::new("fo4");
    nif.blocks[0].set_field("Name", NifValue::String("Root".to_string()));
    let shape = nif.add_block(
        "BSTriShape",
        Some(fields([(
            "Name",
            NifValue::String("EmptyShape".to_string()),
        )])),
    );
    set_children(&mut nif, 0, &[shape]);
    nif.header.footer_roots = vec![0];
    let (_temp, path) = save_temp(nif);

    assert!(matches!(
        aggregate_render_world_bounds(path),
        Err(WorldBoundsError::UnsupportedGeometry { block_id, .. }) if block_id == shape
    ));
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn assert_real_pair(body: &Path, skeleton: &Path) {
    if !body.exists() || !skeleton.exists() {
        eprintln!(
            "skipping optional real pair: body={} skeleton={}",
            body.display(),
            skeleton.display()
        );
        return;
    }

    let body_bounds = aggregate_render_world_bounds(body).expect("body render bounds");
    let skeleton_bounds =
        aggregate_named_node_world_bounds(skeleton).expect("skeleton node bounds");
    for bounds in [body_bounds, skeleton_bounds] {
        assert!(bounds.min.into_iter().chain(bounds.max).all(f32::is_finite));
        assert!(
            bounds
                .min
                .iter()
                .zip(bounds.max)
                .any(|(min, max)| max > *min),
            "nonzero extent: {bounds:?}"
        );
    }
    assert_eq!(
        aggregate_render_world_bounds(skeleton),
        Err(WorldBoundsError::EmptyGeometry)
    );
}

#[test]
fn optional_real_creature_body_and_skeleton_pairs_have_distinct_bounds_sources() {
    let root = repo_root();
    for (body, skeleton) in [
        (
            "extracted/skyrimse/meshes/actors/canine/character assets wolf/wolf.nif",
            "extracted/skyrimse/meshes/actors/canine/character assets wolf/skeleton.nif",
        ),
        (
            "extracted/fnv/meshes/creatures/nvgecko/nvgecko.nif",
            "extracted/fnv/meshes/creatures/nvgecko/skeleton.nif",
        ),
        (
            "extracted/fo3/meshes/creatures/yaoguai/yaoguai.nif",
            "extracted/fo3/meshes/creatures/yaoguai/skeleton.nif",
        ),
    ] {
        assert_real_pair(&root.join(body), &root.join(skeleton));
    }
}
