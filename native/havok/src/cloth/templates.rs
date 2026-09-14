// Cloth template registry: vanilla-derived particle geometry and constraint
// configuration per template. `template_blob` bakes one into an HCL blob.
use std::f32::consts::PI;

use crate::cloth::units::GRAVITY_Z;
use crate::error::{HavokError, HavokResult};

/// Capsule collidable definition embedded in a template.
pub struct CapsuleDef {
    pub name: &'static str,
    pub driving_bone: &'static str,
    pub start: [f32; 4],
    pub end: [f32; 4],
    pub radius: f32,
}

/// A built-in cloth template.
pub struct ClothTemplate {
    pub name: &'static str,
    pub description: &'static str,
    pub material_preset: &'static str,
    pub parent_bone: &'static str,

    // Particle geometry (positions are computed lazily — see build_* fns)
    pub rows: usize,
    pub cols: usize,
    pub bone_prefix: &'static str,

    // Constraint params
    pub standard_link_stiffness: f32,
    pub stretch_link_stiffness: f32,
    pub bend_stiffness: f32,
    pub use_stretch_links: bool,
    pub use_bend_stiffness: bool,

    // Simulation params
    pub gravity: [f32; 4],
    pub global_damping: f32,
    pub collision_tolerance: f32,
    pub num_substeps: usize,
    pub num_solve_iterations: usize,

    pub capsules: &'static [CapsuleDef],

    /// Generate particle positions. Stored as a function pointer so each
    /// template can define its own geometry without heap allocations at
    /// compile time.
    pub build_positions_fn: fn() -> Vec<[f32; 4]>,

    /// Generate triangles.
    pub build_triangles_fn: fn(rows: usize, cols: usize, wrap: bool) -> Vec<[u32; 3]>,

    /// Whether the cylinder wraps (bathrobe, long_dress) vs. open (cape, duster, robe_split).
    pub wrap_cylinder: bool,

    /// Fixed particle count (top row = first `fixed_count` particles).
    pub fixed_count: usize,
}

impl ClothTemplate {
    pub fn num_particles(&self) -> usize {
        self.rows * self.cols
    }
}

// ---------------------------------------------------------------------------
// Shared triangle builders
// ---------------------------------------------------------------------------

/// Build triangles for a cylindrical wrap-around grid (rows x cols).
/// When `wrap` is true, columns wrap at the end (closed cylinder).
/// When `wrap` is false, no wrap-around (open panel).
pub fn build_cylinder_triangles(rows: usize, cols: usize, wrap: bool) -> Vec<[u32; 3]> {
    let mut tris = Vec::new();
    let col_limit = if wrap { cols } else { cols.saturating_sub(1) };
    for ri in 0..(rows.saturating_sub(1)) {
        for ci in 0..col_limit {
            let nc = if wrap { (ci + 1) % cols } else { ci + 1 };
            let i0 = (ri * cols + ci) as u32;
            let i1 = (ri * cols + nc) as u32;
            let i2 = ((ri + 1) * cols + ci) as u32;
            let i3 = ((ri + 1) * cols + nc) as u32;
            tris.push([i0, i1, i2]);
            tris.push([i1, i3, i2]);
        }
    }
    tris
}

// ---------------------------------------------------------------------------
// Bathrobe — 11x6 cylindrical wrap-around robe
// ---------------------------------------------------------------------------

fn bathrobe_positions() -> Vec<[f32; 4]> {
    let rows = 11;
    let cols = 6;
    let radius = 15.0f32;
    let z_top = -30.0f32;
    let z_bottom = -90.0f32;
    let mut positions = Vec::with_capacity(rows * cols);
    for ri in 0..rows {
        let z = z_top + (z_bottom - z_top) * ri as f32 / (rows - 1) as f32;
        for ci in 0..cols {
            let angle = 2.0 * PI * ci as f32 / cols as f32;
            let x = radius * angle.cos();
            let y = radius * angle.sin();
            positions.push([x, y, z, 0.0]);
        }
    }
    positions
}

static BATHROBE_CAPSULES: &[CapsuleDef] = &[
    CapsuleDef {
        name: "UpperLegL",
        driving_bone: "LLeg_Thigh",
        start: [-5.0, 0.0, -35.0, 0.0],
        end: [-5.0, 0.0, -55.0, 0.0],
        radius: 5.0,
    },
    CapsuleDef {
        name: "UpperLegR",
        driving_bone: "RLeg_Thigh",
        start: [5.0, 0.0, -35.0, 0.0],
        end: [5.0, 0.0, -55.0, 0.0],
        radius: 5.0,
    },
    CapsuleDef {
        name: "LowerLegL",
        driving_bone: "LLeg_Calf",
        start: [-5.0, 0.0, -55.0, 0.0],
        end: [-5.0, 0.0, -75.0, 0.0],
        radius: 4.0,
    },
    CapsuleDef {
        name: "LowerLegR",
        driving_bone: "RLeg_Calf",
        start: [5.0, 0.0, -55.0, 0.0],
        end: [5.0, 0.0, -75.0, 0.0],
        radius: 4.0,
    },
    CapsuleDef {
        name: "Pelvis",
        driving_bone: "Pelvis",
        start: [-8.0, 0.0, -30.0, 0.0],
        end: [8.0, 0.0, -30.0, 0.0],
        radius: 8.0,
    },
];

static BATHROBE: ClothTemplate = ClothTemplate {
    name: "Bathrobe",
    description: "Full wrap-around robe, waist to ankles. 11x6 particle grid.",
    material_preset: "Cotton",
    parent_bone: "COM",
    rows: 11,
    cols: 6,
    bone_prefix: "Cloth_BN_Robes",
    standard_link_stiffness: 0.6,
    stretch_link_stiffness: 0.7,
    bend_stiffness: 0.3,
    use_stretch_links: true,
    use_bend_stiffness: true,
    gravity: [0.0, 0.0, GRAVITY_Z, 0.0],
    global_damping: 0.1,
    collision_tolerance: 0.5,
    num_substeps: 1,
    num_solve_iterations: 3,
    capsules: BATHROBE_CAPSULES,
    build_positions_fn: bathrobe_positions,
    build_triangles_fn: build_cylinder_triangles,
    wrap_cylinder: true,
    fixed_count: 6, // top row = cols
};

// ---------------------------------------------------------------------------
// LongDress — 8x8 cylindrical dress with slight flare
// ---------------------------------------------------------------------------

fn long_dress_positions() -> Vec<[f32; 4]> {
    let rows = 8;
    let cols = 8;
    let radius = 14.0f32;
    let z_top = -32.0f32;
    let z_bottom = -88.0f32;
    let mut positions = Vec::with_capacity(rows * cols);
    for ri in 0..rows {
        let z = z_top + (z_bottom - z_top) * ri as f32 / (rows - 1) as f32;
        let r = radius + 3.0 * (ri as f32 / (rows - 1) as f32);
        for ci in 0..cols {
            let angle = 2.0 * PI * ci as f32 / cols as f32;
            let x = r * angle.cos();
            let y = r * angle.sin();
            positions.push([x, y, z, 0.0]);
        }
    }
    positions
}

static LONG_DRESS_CAPSULES: &[CapsuleDef] = &[
    CapsuleDef {
        name: "UpperLegL",
        driving_bone: "LLeg_Thigh",
        start: [-5.0, 0.0, -35.0, 0.0],
        end: [-5.0, 0.0, -55.0, 0.0],
        radius: 5.0,
    },
    CapsuleDef {
        name: "UpperLegR",
        driving_bone: "RLeg_Thigh",
        start: [5.0, 0.0, -35.0, 0.0],
        end: [5.0, 0.0, -55.0, 0.0],
        radius: 5.0,
    },
    CapsuleDef {
        name: "LowerLegL",
        driving_bone: "LLeg_Calf",
        start: [-5.0, 0.0, -55.0, 0.0],
        end: [-5.0, 0.0, -75.0, 0.0],
        radius: 4.0,
    },
    CapsuleDef {
        name: "LowerLegR",
        driving_bone: "RLeg_Calf",
        start: [5.0, 0.0, -55.0, 0.0],
        end: [5.0, 0.0, -75.0, 0.0],
        radius: 4.0,
    },
    CapsuleDef {
        name: "Pelvis",
        driving_bone: "Pelvis",
        start: [-8.0, 0.0, -30.0, 0.0],
        end: [8.0, 0.0, -30.0, 0.0],
        radius: 8.0,
    },
];

static LONG_DRESS: ClothTemplate = ClothTemplate {
    name: "LongDress",
    description: "Floor-length dress/skirt. 8x8 particle grid with slight flare.",
    material_preset: "Silk",
    parent_bone: "COM",
    rows: 8,
    cols: 8,
    bone_prefix: "Cloth_BN_Dress",
    standard_link_stiffness: 0.7,
    stretch_link_stiffness: 0.8,
    bend_stiffness: 0.15,
    use_stretch_links: true,
    use_bend_stiffness: true,
    gravity: [0.0, 0.0, GRAVITY_Z, 0.0],
    global_damping: 0.08,
    collision_tolerance: 0.4,
    num_substeps: 1,
    num_solve_iterations: 3,
    capsules: LONG_DRESS_CAPSULES,
    build_positions_fn: long_dress_positions,
    build_triangles_fn: build_cylinder_triangles,
    wrap_cylinder: true,
    fixed_count: 8, // top row = cols
};

// ---------------------------------------------------------------------------
// Duster — 10x4 open-front coat
// ---------------------------------------------------------------------------

fn duster_positions() -> Vec<[f32; 4]> {
    let rows = 10;
    let cols = 4;
    let radius = 14.0f32;
    let z_top = -28.0f32;
    let z_bottom = -85.0f32;
    let arc_start = PI * 0.25f32;
    let arc_end = PI * 1.75f32;
    let mut positions = Vec::with_capacity(rows * cols);
    for ri in 0..rows {
        let z = z_top + (z_bottom - z_top) * ri as f32 / (rows - 1) as f32;
        for ci in 0..cols {
            let t = if cols > 1 {
                ci as f32 / (cols - 1) as f32
            } else {
                0.0
            };
            let angle = arc_start + (arc_end - arc_start) * t;
            let x = radius * angle.cos();
            let y = radius * angle.sin();
            positions.push([x, y, z, 0.0]);
        }
    }
    positions
}

static DUSTER_CAPSULES: &[CapsuleDef] = &[
    CapsuleDef {
        name: "UpperLegL",
        driving_bone: "LLeg_Thigh",
        start: [-6.0, 0.0, -32.0, 0.0],
        end: [-6.0, 0.0, -52.0, 0.0],
        radius: 5.5,
    },
    CapsuleDef {
        name: "UpperLegR",
        driving_bone: "RLeg_Thigh",
        start: [6.0, 0.0, -32.0, 0.0],
        end: [6.0, 0.0, -52.0, 0.0],
        radius: 5.5,
    },
    CapsuleDef {
        name: "LowerLegL",
        driving_bone: "LLeg_Calf",
        start: [-5.0, 0.0, -52.0, 0.0],
        end: [-5.0, 0.0, -72.0, 0.0],
        radius: 4.5,
    },
    CapsuleDef {
        name: "LowerLegR",
        driving_bone: "RLeg_Calf",
        start: [5.0, 0.0, -52.0, 0.0],
        end: [5.0, 0.0, -72.0, 0.0],
        radius: 4.5,
    },
];

static DUSTER: ClothTemplate = ClothTemplate {
    name: "Duster",
    description: "Long open-front coat. 10x4 grid, open at front. Leather/canvas feel.",
    material_preset: "Leather",
    parent_bone: "COM",
    rows: 10,
    cols: 4,
    bone_prefix: "Cloth_BN_Duster",
    standard_link_stiffness: 0.85,
    stretch_link_stiffness: 0.9,
    bend_stiffness: 0.6,
    use_stretch_links: true,
    use_bend_stiffness: true,
    gravity: [0.0, 0.0, GRAVITY_Z, 0.0],
    global_damping: 0.15,
    collision_tolerance: 0.6,
    num_substeps: 1,
    num_solve_iterations: 3,
    capsules: DUSTER_CAPSULES,
    build_positions_fn: duster_positions,
    build_triangles_fn: build_cylinder_triangles,
    wrap_cylinder: false,
    fixed_count: 4,
};

// ---------------------------------------------------------------------------
// RobeSplit — 8x6 front-split robe
// ---------------------------------------------------------------------------

fn robe_split_positions() -> Vec<[f32; 4]> {
    let rows = 8;
    let cols = 6;
    let radius = 14.0f32;
    let z_top = -30.0f32;
    let z_bottom = -82.0f32;
    let gap_angle = PI * 0.3f32;
    let mut positions = Vec::with_capacity(rows * cols);
    for ri in 0..rows {
        let z = z_top + (z_bottom - z_top) * ri as f32 / (rows - 1) as f32;
        for ci in 0..cols {
            let t = ci as f32 / cols as f32;
            let angle = gap_angle / 2.0 + (2.0 * PI - gap_angle) * t;
            let x = radius * angle.cos();
            let y = radius * angle.sin();
            positions.push([x, y, z, 0.0]);
        }
    }
    positions
}

static ROBE_SPLIT_CAPSULES: &[CapsuleDef] = &[
    CapsuleDef {
        name: "UpperLegL",
        driving_bone: "LLeg_Thigh",
        start: [-5.0, 0.0, -34.0, 0.0],
        end: [-5.0, 0.0, -54.0, 0.0],
        radius: 5.0,
    },
    CapsuleDef {
        name: "UpperLegR",
        driving_bone: "RLeg_Thigh",
        start: [5.0, 0.0, -34.0, 0.0],
        end: [5.0, 0.0, -54.0, 0.0],
        radius: 5.0,
    },
    CapsuleDef {
        name: "LowerLegL",
        driving_bone: "LLeg_Calf",
        start: [-5.0, 0.0, -54.0, 0.0],
        end: [-5.0, 0.0, -74.0, 0.0],
        radius: 4.0,
    },
    CapsuleDef {
        name: "LowerLegR",
        driving_bone: "RLeg_Calf",
        start: [5.0, 0.0, -54.0, 0.0],
        end: [5.0, 0.0, -74.0, 0.0],
        radius: 4.0,
    },
];

static ROBE_SPLIT: ClothTemplate = ClothTemplate {
    name: "RobeSplit",
    description: "Front-split robe with two independent panels. 8x6 grid.",
    material_preset: "Linen",
    parent_bone: "COM",
    rows: 8,
    cols: 6,
    bone_prefix: "Cloth_BN_SplitRobe",
    standard_link_stiffness: 0.55,
    stretch_link_stiffness: 0.65,
    bend_stiffness: 0.25,
    use_stretch_links: true,
    use_bend_stiffness: true,
    gravity: [0.0, 0.0, GRAVITY_Z, 0.0],
    global_damping: 0.1,
    collision_tolerance: 0.5,
    num_substeps: 1,
    num_solve_iterations: 3,
    capsules: ROBE_SPLIT_CAPSULES,
    build_positions_fn: robe_split_positions,
    build_triangles_fn: build_cylinder_triangles,
    wrap_cylinder: false,
    fixed_count: 6,
};

// ---------------------------------------------------------------------------
// Cape — 6x5 flat panel back cape
// ---------------------------------------------------------------------------

fn cape_positions() -> Vec<[f32; 4]> {
    let rows = 6;
    let cols = 5;
    let width = 28.0f32;
    let y_offset = -14.0f32;
    let z_top = -20.0f32;
    let z_bottom = -75.0f32;
    let mut positions = Vec::with_capacity(rows * cols);
    for ri in 0..rows {
        let z = z_top + (z_bottom - z_top) * ri as f32 / (rows - 1) as f32;
        for ci in 0..cols {
            let x = -width / 2.0 + width * ci as f32 / (cols - 1) as f32;
            let y = y_offset;
            positions.push([x, y, z, 0.0]);
        }
    }
    positions
}

static CAPE_CAPSULES: &[CapsuleDef] = &[
    CapsuleDef {
        name: "Torso",
        driving_bone: "Spine2",
        start: [-10.0, -8.0, -20.0, 0.0],
        end: [10.0, -8.0, -20.0, 0.0],
        radius: 10.0,
    },
    CapsuleDef {
        name: "Pelvis",
        driving_bone: "Pelvis",
        start: [-8.0, -6.0, -35.0, 0.0],
        end: [8.0, -6.0, -35.0, 0.0],
        radius: 8.0,
    },
];

static CAPE: ClothTemplate = ClothTemplate {
    name: "Cape",
    description: "Back-mounted cape/cloak. 6x5 flat panel from shoulders.",
    material_preset: "Cotton",
    parent_bone: "Spine2",
    rows: 6,
    cols: 5,
    bone_prefix: "Cloth_BN_Cape",
    standard_link_stiffness: 0.5,
    stretch_link_stiffness: 0.6,
    bend_stiffness: 0.1,
    use_stretch_links: true,
    use_bend_stiffness: true,
    gravity: [0.0, 0.0, GRAVITY_Z, 0.0],
    global_damping: 0.06,
    collision_tolerance: 0.4,
    num_substeps: 1,
    num_solve_iterations: 3,
    capsules: CAPE_CAPSULES,
    build_positions_fn: cape_positions,
    build_triangles_fn: build_cylinder_triangles,
    wrap_cylinder: false,
    fixed_count: 5,
};

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

pub static TEMPLATES: &[&ClothTemplate] = &[&BATHROBE, &LONG_DRESS, &DUSTER, &ROBE_SPLIT, &CAPE];

pub fn get_template(name: &str) -> HavokResult<&'static ClothTemplate> {
    if let Some(t) = TEMPLATES.iter().find(|t| t.name == name) {
        return Ok(t);
    }
    let lower = name.to_lowercase();
    if let Some(t) = TEMPLATES.iter().find(|t| t.name.to_lowercase() == lower) {
        return Ok(t);
    }
    Err(HavokError::InvalidInput(format!(
        "Unknown cloth template: {:?}. Available: {:?}",
        name,
        TEMPLATES.iter().map(|t| t.name).collect::<Vec<_>>()
    )))
}

// ---------------------------------------------------------------------------
// JSON helpers for pyfunctions
// ---------------------------------------------------------------------------

pub fn template_list_json() -> HavokResult<String> {
    let arr: Vec<serde_json::Value> = TEMPLATES
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "material_preset": t.material_preset,
                "parent_bone": t.parent_bone,
                "bone_grid": format!("{}x{}", t.rows, t.cols),
                "num_particles": t.num_particles(),
                "num_fixed": t.fixed_count,
                "num_capsules": t.capsules.len(),
            })
        })
        .collect();
    serde_json::to_string(&arr).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

pub fn template_get_json(name: &str) -> HavokResult<String> {
    let t = get_template(name)?;
    let capsules: Vec<serde_json::Value> = t
        .capsules
        .iter()
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "driving_bone": c.driving_bone,
                "start": c.start,
                "end": c.end,
                "radius": c.radius,
            })
        })
        .collect();
    let v = serde_json::json!({
        "name": t.name,
        "description": t.description,
        "material_preset": t.material_preset,
        "parent_bone": t.parent_bone,
        "rows": t.rows,
        "cols": t.cols,
        "bone_prefix": t.bone_prefix,
        "num_particles": t.num_particles(),
        "num_fixed": t.fixed_count,
        "bone_grid": format!("{}x{}", t.rows, t.cols),
        "standard_link_stiffness": t.standard_link_stiffness,
        "stretch_link_stiffness": t.stretch_link_stiffness,
        "bend_stiffness": t.bend_stiffness,
        "use_stretch_links": t.use_stretch_links,
        "use_bend_stiffness": t.use_bend_stiffness,
        "gravity": t.gravity,
        "global_damping": t.global_damping,
        "collision_tolerance": t.collision_tolerance,
        "num_substeps": t.num_substeps,
        "num_solve_iterations": t.num_solve_iterations,
        "capsules": capsules,
    });
    serde_json::to_string(&v).map_err(|e| HavokError::InvalidInput(e.to_string()))
}

/// Build a `ClothSetupObject` for the named template with optional overrides.
///
/// Exposed for testing and programmatic use; does not require a source NIF.
pub fn build_cloth_setup(
    name: &str,
    args_json: &str,
) -> HavokResult<crate::cloth::setup::cloth_setup::ClothSetupObject> {
    use crate::cloth::setup::buffer_setup::{
        BufferSetupObject, BufferType, TransformSetSetupObject,
    };
    use crate::cloth::setup::cloth_setup::ClothSetupObject;
    use crate::cloth::setup::collidable_setup::{CapsuleShapeSetup, CollidableSetup};
    use crate::cloth::setup::constraint_setup::{
        BendStiffnessSetup, ConstraintSetupObject, StandardLinkSetup, StretchLinkSetup,
    };
    use crate::cloth::setup::mesh::{SetupMesh, SimulationSetupMesh};
    use crate::cloth::setup::operator_setup::{
        CopyVerticesSetup, MoveParticlesSetup, OperatorSetupObject, SimulateSetup,
        SimulateSetupConfig, SkinSetup,
    };
    use crate::cloth::setup::sim_cloth_setup::SimClothSetupObject;
    use crate::cloth::setup::types::{VertexFloatInput, VertexSelectionInput};
    use crate::cloth::skeleton;
    use crate::cloth::skinning;

    #[derive(serde::Deserialize, Default)]
    struct Args {
        material: Option<String>,
        parent_bone: Option<String>,
    }
    let args: Args = if args_json.trim() == "{}" || args_json.trim().is_empty() {
        Args::default()
    } else {
        serde_json::from_str(args_json)
            .map_err(|e| HavokError::InvalidInput(format!("args JSON: {e}")))?
    };

    let tmpl = get_template(name)?;
    let parent_bone = args.parent_bone.as_deref().unwrap_or(tmpl.parent_bone);

    let positions = (tmpl.build_positions_fn)();
    let triangles = (tmpl.build_triangles_fn)(tmpl.rows, tmpl.cols, tmpl.wrap_cylinder);
    let n_particles = positions.len();

    let bones = skeleton::generate_bones_from_particles(
        &positions,
        tmpl.bone_prefix,
        tmpl.rows,
        tmpl.cols,
        parent_bone,
    );
    let (bone_names, bone_positions) = skeleton::bones_to_transform_set(&bones);
    let skin_weights = skinning::auto_skin_to_cloth_bones(&positions, &bone_positions, 4, 2.0);

    let bone_weights_mesh: Vec<Vec<[f32; 2]>> = skin_weights
        .iter()
        .map(|vw| vw.iter().map(|&(bi, w)| [bi as f32, w]).collect())
        .collect();

    let setup_mesh = SetupMesh {
        name: tmpl.name.to_string(),
        positions: positions.clone(),
        triangles: triangles.clone(),
        bone_names: bone_names.clone(),
        bone_weights: bone_weights_mesh,
        ..Default::default()
    };

    let n = n_particles as u32;
    let sim_mesh = SimulationSetupMesh {
        positions: positions.clone(),
        triangles: triangles.clone(),
        sim_to_render_map: (0..n).map(|i| vec![i]).collect(),
        render_to_sim_map: (0..n).collect(),
        source_mesh: Some(Box::new(setup_mesh.clone())),
        ..Default::default()
    };

    let fixed_sel = if tmpl.fixed_count > 0 {
        VertexSelectionInput::all()
    } else {
        VertexSelectionInput::none()
    };

    let vfi = |v: f32| VertexFloatInput::constant(v);
    let mut constraint_setups: Vec<ConstraintSetupObject> =
        vec![ConstraintSetupObject::StandardLink(StandardLinkSetup {
            name: format!("{}_StandardLinks", tmpl.name),
            stiffness: vfi(tmpl.standard_link_stiffness),
            ..Default::default()
        })];
    if tmpl.use_stretch_links {
        constraint_setups.push(ConstraintSetupObject::StretchLink(StretchLinkSetup {
            name: format!("{}_StretchLinks", tmpl.name),
            stiffness: vfi(tmpl.stretch_link_stiffness),
            ..Default::default()
        }));
    }
    if tmpl.use_bend_stiffness {
        constraint_setups.push(ConstraintSetupObject::BendStiffness(BendStiffnessSetup {
            name: format!("{}_BendStiffness", tmpl.name),
            bend_stiffness: vfi(tmpl.bend_stiffness),
            ..Default::default()
        }));
    }

    let collidable_setups: Vec<CollidableSetup> = tmpl
        .capsules
        .iter()
        .map(|c| CollidableSetup {
            name: c.name.to_string(),
            shape: Some(CapsuleShapeSetup {
                start: c.start,
                end: c.end,
                big_radius: c.radius,
                small_radius: c.radius,
            }),
            driving_bone_name: c.driving_bone.to_string(),
            ..Default::default()
        })
        .collect();

    let mat_name = args.material.as_deref().unwrap_or(tmpl.material_preset);
    let mat = crate::cloth::materials::get_preset(mat_name)?;

    let sim_cloth = SimClothSetupObject {
        name: tmpl.name.to_string(),
        simulation_mesh: Some(sim_mesh),
        gravity: tmpl.gravity,
        global_damping_per_second: tmpl.global_damping,
        collision_tolerance: tmpl.collision_tolerance,
        particle_mass: vfi(mat.particle_mass),
        particle_radius: vfi(mat.particle_radius),
        particle_friction: vfi(mat.particle_friction),
        fixed_particles: fixed_sel,
        constraint_setups,
        collidable_setups,
        ..Default::default()
    };

    let ts_name = "skeleton".to_string();
    Ok(ClothSetupObject {
        name: tmpl.name.to_string(),
        buffer_setups: vec![
            BufferSetupObject {
                name: "display".to_string(),
                buffer_type: BufferType::Display as u8,
                setup_mesh: Some(setup_mesh),
                has_normals: true,
                ..Default::default()
            },
            BufferSetupObject {
                name: "static_display".to_string(),
                buffer_type: BufferType::StaticDisplay as u8,
                ..Default::default()
            },
        ],
        transform_set_setups: vec![TransformSetSetupObject {
            name: ts_name.clone(),
            bone_names: bone_names.clone(),
            skeleton_name: String::new(),
        }],
        sim_cloth_setups: vec![sim_cloth],
        operator_setups: vec![
            OperatorSetupObject::Simulate(SimulateSetup {
                name: "simulate".to_string(),
                sim_cloth_setup_name: tmpl.name.to_string(),
                configs: vec![SimulateSetupConfig {
                    name: "default".to_string(),
                    num_substeps: tmpl.num_substeps,
                    num_solve_iterations: tmpl.num_solve_iterations,
                    ..Default::default()
                }],
            }),
            OperatorSetupObject::Skin(SkinSetup {
                name: "skin".to_string(),
                transform_set_name: ts_name.clone(),
                output_buffer_name: "display".to_string(),
                skin_normals: true,
                ..Default::default()
            }),
            OperatorSetupObject::CopyVertices(CopyVerticesSetup {
                name: "copy".to_string(),
                input_buffer_name: "display".to_string(),
                output_buffer_name: "static_display".to_string(),
                copy_normals: true,
            }),
            OperatorSetupObject::MoveParticles(MoveParticlesSetup {
                name: "move_fixed".to_string(),
                sim_cloth_setup_name: tmpl.name.to_string(),
                display_buffer_name: "display".to_string(),
            }),
        ],
        state_setups: vec![serde_json::json!({
            "name": format!("{}State", tmpl.name),
            "operator_indices": [0, 1, 2, 3],
        })],
    })
}

/// Build a template cloth setup and return the serialized HCL packfile blob.
///
/// Pipeline:
///   1. Build particle positions from template geometry
///   2. Generate cloth bones (rows x cols grid)
///   3. Auto-skin particles to bones
///   4. Build ClothSetupObject (includes a default ClothState)
///   5. Bake to HKX
pub fn template_blob(name: &str, args_json: &str) -> HavokResult<Vec<u8>> {
    use crate::cloth::bake::bake_cloth_setup;
    use crate::hkx;

    // 4. Build ClothSetupObject (including default state)
    let setup = build_cloth_setup(name, args_json)?;

    let hkx_file = bake_cloth_setup(&setup)?;

    let mut reg = hkx::descriptors::DescriptorRegistry::new();
    Ok(hkx::write_hkx(&hkx_file, &mut reg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bathrobe_has_66_particles() {
        assert_eq!(BATHROBE.num_particles(), 66);
    }

    #[test]
    fn all_templates_have_nonzero_particles() {
        for t in TEMPLATES {
            let positions = (t.build_positions_fn)();
            assert!(
                !positions.is_empty(),
                "template '{}' has no positions",
                t.name
            );
            assert_eq!(positions.len(), t.num_particles());
        }
    }

    #[test]
    fn get_template_case_insensitive() {
        assert!(get_template("bathrobe").is_ok());
        assert!(get_template("CAPE").is_ok());
        assert!(get_template("NotATemplate").is_err());
    }

    #[test]
    fn template_list_json_has_five_entries() {
        let json = template_list_json().unwrap();
        let arr: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(arr.len(), 5);
    }

    #[test]
    fn all_templates_build_cloth_setup_with_state() {
        use crate::cloth::bake::bake_cloth_setup;
        use crate::cloth::runtime::ClothData;
        use crate::cloth::validate::validate_cloth_data;

        for t in TEMPLATES {
            let setup = build_cloth_setup(t.name, "{}")
                .unwrap_or_else(|e| panic!("build_cloth_setup('{}') failed: {e}", t.name));
            assert!(
                !setup.state_setups.is_empty(),
                "template '{}' has no state_setups",
                t.name
            );
            let hkx = bake_cloth_setup(&setup)
                .unwrap_or_else(|e| panic!("bake_cloth_setup('{}') failed: {e}", t.name));
            let cloth_data = ClothData::from_hkx_file(&hkx);
            let result = validate_cloth_data(cloth_data.as_ref());
            let no_state_errors: Vec<_> = result
                .errors()
                .into_iter()
                .filter(|i| i.code == "NO_CLOTH_STATES")
                .collect();
            assert!(
                no_state_errors.is_empty(),
                "template '{}' baked cloth fails NO_CLOTH_STATES: {:?}",
                t.name,
                no_state_errors,
            );
        }
    }
}
