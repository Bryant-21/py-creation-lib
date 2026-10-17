//! Phase 2 instancing prototype (plan Task 11): from one real L4 tile, write
//! 1. `<tile>.bto`: the tile plus three instanced (shape, data) stream-root pairs over its first cell,
//! 2. `<tile>.children9000.bto`: 9,000 one-triangle root children (root-child limit),
//! 3. `<tile>.empty_instances.bto`: the tile plus one pair whose data block has no instances.
//! Usage: proto_instanced_tile <source L4 .bto> <out dir> <mesh 1> <mesh 2> <mesh 3>
use std::path::{Path, PathBuf};

use anyhow::Context;
use lodgen_native::audit::{CELL, parse_tile_name, terrain_height_at};
use lodgen_native::descriptors::QuadDesc;
use lodgen_native::objects::fo76_bto::extract_inline_geometry;
use lodgen_native::objects::geometry::LodGeometry;
use lodgen_native::objects::object_lod::{BtoShader, BtoShape, MultiBoundAabb, generate_segments};
use lodgen_native::output::bto::{add_instance_trishape, build_bto_nif};
use lodgen_native::output::instance_block::{Instance, add_to_nif};
use nif_core_native::model::NifFile;

const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
// 90° about z, x,y,z,w (LOD_RUNTIME.md Q8).
const YAW_90: [f32; 4] = [0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2];

fn main() -> anyhow::Result<()> {
    let usage = "usage: proto_instanced_tile <source L4 .bto> <out dir> <mesh 1> <mesh 2> <mesh 3>";
    let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    anyhow::ensure!(args.len() == 5, usage);
    let (source, out, meshes) = (&args[0], &args[1], &args[2..5]);
    let name = source.file_name().and_then(|n| n.to_str()).context("source name")?.to_string();
    let (world, level, tile_x, tile_y) = parse_tile_name(&name).context("source is not a tile name")?;
    anyhow::ensure!(level == 4, "source must be an L4 tile");
    let origin = [tile_x as f32 * CELL, tile_y as f32 * CELL];
    let terrain = source
        .parent()
        .and_then(Path::parent)
        .map(|dir| dir.join(format!("{world}.{level}.{tile_x}.{tile_y}.btr")))
        .context("terrain dir")?;
    let terrain = std::fs::read(&terrain).with_context(|| format!("read {}", terrain.display()))?;
    let ground = |x: f32, y: f32| -> anyhow::Result<f32> {
        terrain_height_at(&terrain, origin, x, y)?.with_context(|| format!("no terrain at {x},{y}"))
    };
    let atlas = |suffix: &str| format!(r"Textures\Terrain\{world}\Objects\{world}.Objects{suffix}.dds");
    let mut textures: [String; 10] = Default::default();
    textures[0] = atlas("");
    textures[1] = atlas("_n");
    textures[7] = atlas("_s");
    std::fs::create_dir_all(out)?;
    let source_bytes = std::fs::read(source)?;

    let mut tile = NifFile::from_bytes(&source_bytes, None).map_err(|e| anyhow::anyhow!("read source: {e:?}"))?;
    anyhow::ensure!(tile.header.footer_roots.len() == 1, "source tile already has stream-root pairs");
    for (i, mesh) in meshes.iter().enumerate() {
        let shape = instance_shape(mesh_geometry(mesh)?, &textures);
        let x = origin[0] + 1024.0 * (i as f32 + 1.0);
        let spots = [(x, origin[1] + 1024.0), (x, origin[1] + 2560.0)];
        let rotations = [IDENTITY, if i == 0 { YAW_90 } else { IDENTITY }];
        let instances = spots
            .iter()
            .zip(rotations)
            .map(|(&(x, y), rot)| Ok(Instance { pos: [x, y, ground(x, y)?], scale: 1.0, rot }))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let shape_id = add_instance_trishape(&mut tile, &shape);
        let data_id = add_to_nif(&mut tile, i as u32 + 1, &instances, &[0, 0]);
        tile.header.footer_roots.extend([shape_id as i32, data_id as i32]);
    }
    let instanced = out.join(&name);
    std::fs::write(&instanced, tile.to_bytes().map_err(|e| anyhow::anyhow!("{e:?}"))?)?;

    let quad = QuadDesc {
        z_order: 0,
        x: tile_x,
        y: tile_y,
        quad_level: 4,
        quad_index: 0,
        quad_offset: 4.0 * CELL,
        static_indices: Vec::new(),
        statics: Vec::new(),
        out_values: Default::default(),
    };
    let mut crowd = Vec::with_capacity(9000);
    for n in 0..9000 {
        let local = [(n % 90) as f32 * 180.0 + 64.0, (n / 90) as f32 * 160.0 + 64.0];
        let z = ground(origin[0] + local[0], origin[1] + local[1])?;
        crowd.push(triangle_shape(&quad, origin, local, z));
    }
    let children = out.join(name.replace(".bto", ".children9000.bto"));
    std::fs::write(&children, build_bto_nif(&crowd)?.to_bytes().map_err(|e| anyhow::anyhow!("{e:?}"))?)?;

    let mut empty = NifFile::from_bytes(&source_bytes, None).map_err(|e| anyhow::anyhow!("read source: {e:?}"))?;
    let shape = instance_shape(mesh_geometry(&meshes[0])?, &textures);
    let shape_id = add_instance_trishape(&mut empty, &shape);
    let data_id = add_to_nif(&mut empty, 4, &[], &[]);
    empty.header.footer_roots.extend([shape_id as i32, data_id as i32]);
    let empty_path = out.join(name.replace(".bto", ".empty_instances.bto"));
    std::fs::write(&empty_path, empty.to_bytes().map_err(|e| anyhow::anyhow!("{e:?}"))?)?;

    for path in [&instanced, &children, &empty_path] {
        println!("{}", path.display());
    }
    Ok(())
}

/// The first geometry block of an FO4 LOD mesh, in model space.
fn mesh_geometry(path: &Path) -> anyhow::Result<LodGeometry> {
    let nif = NifFile::from_bytes(&std::fs::read(path)?, Some(path.to_path_buf()))
        .map_err(|e| anyhow::anyhow!("read {}: {e:?}", path.display()))?;
    let mut geometry = nif
        .blocks
        .iter()
        .filter(|b| b.type_name == "BSTriShape" || b.type_name == "BSSubIndexTriShape")
        .find_map(extract_inline_geometry)
        .with_context(|| format!("no geometry in {}", path.display()))?;
    geometry.update_bbox();
    Ok(geometry)
}

fn instance_shape(geometry: LodGeometry, textures: &[String; 10]) -> BtoShape {
    let center = geometry.bbox.center(false);
    let radius = geometry.bbox.radius();
    BtoShape {
        geometry,
        segments: Vec::new(),
        segment_count: 0,
        translation: [0.0; 3],
        scale: 1.0,
        center,
        radius,
        multibound: MultiBoundAabb { position: center, extent: [radius; 3] },
        name_index_at: false,
        shader: lod_shader(textures.clone()),
        alpha: None,
        enable_parent: 0,
    }
}

/// One 64-unit triangle at tile-local `local`, stored the way lodgen stores L4 shapes: vertices relative to
/// the tile origin divided by the level, so the half-float positions keep their precision.
fn triangle_shape(quad: &QuadDesc, origin: [f32; 2], local: [f32; 2], z: f32) -> BtoShape {
    let scale = 4.0;
    let v = |dx: f32, dy: f32| [(local[0] + dx) / scale, (local[1] + dy) / scale, z / scale];
    let mut geometry = LodGeometry::new();
    geometry.vertices = vec![v(0.0, 0.0), v(64.0, 0.0), v(0.0, 64.0)];
    geometry.uvcoords = vec![[0.0, 0.0]; 3];
    geometry.normals = vec![[0.0, 0.0, 1.0]; 3];
    geometry.tangents = vec![[1.0, 0.0, 0.0]; 3];
    geometry.bitangents = vec![[0.0, 1.0, 0.0]; 3];
    geometry.triangles = vec![[0, 1, 2]];
    geometry.update_bbox();
    let center = geometry.bbox.center(false);
    let radius = geometry.bbox.radius();
    BtoShape {
        segments: generate_segments(quad, local[0], local[1], 1),
        segment_count: 4,
        translation: [origin[0], origin[1], 0.0],
        scale,
        center,
        radius,
        multibound: MultiBoundAabb {
            position: [origin[0] + local[0] + 32.0, origin[1] + local[1] + 32.0, z],
            extent: [32.0, 32.0, 1.0],
        },
        name_index_at: false,
        shader: lod_shader(Default::default()),
        alpha: None,
        enable_parent: 0,
        geometry,
    }
}

/// Vanilla object-LOD flags: Specular | ZBuffer_Test (+ engine bits), ZBuffer_Write | LOD_Objects.
fn lod_shader(texture_set: [String; 10]) -> BtoShader {
    BtoShader::Lighting { texture_set, flags1: 0x8040_0001, flags2: 5, clamp: 0, backlight: None, grayscale_scale: None }
}
