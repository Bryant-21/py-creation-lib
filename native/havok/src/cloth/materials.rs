use crate::error::{HavokError, HavokResult};

/// Named set of cloth physics parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialPreset {
    pub name: &'static str,
    pub particle_mass: f32,
    pub particle_radius: f32,
    pub particle_friction: f32,
    pub standard_link_stiffness: f32,
    pub stretch_link_stiffness: f32,
    pub bend_stiffness: f32,
    pub global_damping_per_second: f32,
    pub gravity_factor: f32,
    pub collision_tolerance: f32,
    pub num_substeps: u32,
    pub num_solve_iterations: u32,
}

// ---------------------------------------------------------------------------
// Preset definitions — mirrors Python PRESETS exactly
// ---------------------------------------------------------------------------

const SILK: MaterialPreset = MaterialPreset {
    name: "Silk",
    particle_mass: 0.01,
    particle_radius: 0.3,
    particle_friction: 0.15,
    standard_link_stiffness: 0.8,
    stretch_link_stiffness: 0.9,
    bend_stiffness: 0.05,
    global_damping_per_second: 0.05,
    gravity_factor: 1.0,
    collision_tolerance: 0.3,
    num_substeps: 1,
    num_solve_iterations: 3,
};

const COTTON: MaterialPreset = MaterialPreset {
    name: "Cotton",
    particle_mass: 0.02,
    particle_radius: 0.5,
    particle_friction: 0.35,
    standard_link_stiffness: 0.6,
    stretch_link_stiffness: 0.7,
    bend_stiffness: 0.3,
    global_damping_per_second: 0.1,
    gravity_factor: 1.0,
    collision_tolerance: 0.5,
    num_substeps: 1,
    num_solve_iterations: 3,
};

const LINEN: MaterialPreset = MaterialPreset {
    name: "Linen",
    particle_mass: 0.025,
    particle_radius: 0.5,
    particle_friction: 0.3,
    standard_link_stiffness: 0.65,
    stretch_link_stiffness: 0.75,
    bend_stiffness: 0.35,
    global_damping_per_second: 0.12,
    gravity_factor: 1.0,
    collision_tolerance: 0.5,
    num_substeps: 1,
    num_solve_iterations: 3,
};

const DENIM: MaterialPreset = MaterialPreset {
    name: "Denim",
    particle_mass: 0.04,
    particle_radius: 0.6,
    particle_friction: 0.5,
    standard_link_stiffness: 0.8,
    stretch_link_stiffness: 0.85,
    bend_stiffness: 0.6,
    global_damping_per_second: 0.15,
    gravity_factor: 1.0,
    collision_tolerance: 0.6,
    num_substeps: 1,
    num_solve_iterations: 3,
};

const LEATHER: MaterialPreset = MaterialPreset {
    name: "Leather",
    particle_mass: 0.06,
    particle_radius: 0.7,
    particle_friction: 0.6,
    standard_link_stiffness: 0.9,
    stretch_link_stiffness: 0.95,
    bend_stiffness: 0.7,
    global_damping_per_second: 0.2,
    gravity_factor: 1.0,
    collision_tolerance: 0.7,
    num_substeps: 1,
    num_solve_iterations: 3,
};

const HEAVY_WOOL: MaterialPreset = MaterialPreset {
    name: "Heavy Wool",
    particle_mass: 0.05,
    particle_radius: 0.6,
    particle_friction: 0.55,
    standard_link_stiffness: 0.7,
    stretch_link_stiffness: 0.8,
    bend_stiffness: 0.5,
    global_damping_per_second: 0.18,
    gravity_factor: 1.0,
    collision_tolerance: 0.6,
    num_substeps: 1,
    num_solve_iterations: 3,
};

// CHAIN topology preset pairs with CHAIN_MAIL material; CHAIN topology has
// bend_stiffness=0.0, so this material must match to avoid inconsistent sim.
const CHAIN_MAIL: MaterialPreset = MaterialPreset {
    name: "Chain Mail",
    particle_mass: 0.12,
    particle_radius: 0.4,
    particle_friction: 0.2,
    standard_link_stiffness: 0.95,
    stretch_link_stiffness: 0.98,
    bend_stiffness: 0.0,
    global_damping_per_second: 0.08,
    gravity_factor: 1.0,
    collision_tolerance: 0.4,
    num_substeps: 1,
    num_solve_iterations: 3,
};

const ROPE: MaterialPreset = MaterialPreset {
    name: "Rope",
    particle_mass: 0.03,
    particle_radius: 0.4,
    particle_friction: 0.45,
    standard_link_stiffness: 0.95,
    stretch_link_stiffness: 0.98,
    bend_stiffness: 0.15,
    global_damping_per_second: 0.1,
    gravity_factor: 1.0,
    collision_tolerance: 0.4,
    num_substeps: 1,
    num_solve_iterations: 3,
};

const SQUISHY: MaterialPreset = MaterialPreset {
    name: "Squishy",
    particle_mass: 0.08,
    particle_radius: 0.8,
    particle_friction: 0.7,
    standard_link_stiffness: 0.3,
    stretch_link_stiffness: 0.4,
    bend_stiffness: 0.1,
    global_damping_per_second: 0.25,
    gravity_factor: 1.0,
    collision_tolerance: 0.8,
    num_substeps: 1,
    num_solve_iterations: 3,
};

/// All 9 material presets in registry order.
pub const PRESETS: &[&MaterialPreset] = &[
    &SILK,
    &COTTON,
    &LINEN,
    &DENIM,
    &LEATHER,
    &HEAVY_WOOL,
    &CHAIN_MAIL,
    &ROPE,
    &SQUISHY,
];

/// Look up a material preset by name (case-insensitive, trims whitespace).
pub fn get_preset(name: &str) -> HavokResult<&'static MaterialPreset> {
    let key = name.trim();
    // Exact match first
    if let Some(p) = PRESETS.iter().find(|p| p.name == key) {
        return Ok(p);
    }
    // Case-insensitive fallback
    let lower = key.to_lowercase();
    if let Some(p) = PRESETS.iter().find(|p| p.name.to_lowercase() == lower) {
        return Ok(p);
    }
    Err(HavokError::InvalidInput(format!(
        "Unknown material preset: {:?}. Available: {:?}",
        name,
        PRESETS.iter().map(|p| p.name).collect::<Vec<_>>()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_9_presets_present() {
        assert_eq!(PRESETS.len(), 9);
    }

    #[test]
    fn cotton_values_match_python() {
        let p = get_preset("Cotton").unwrap();
        assert_eq!(p.particle_mass, 0.02);
        assert_eq!(p.bend_stiffness, 0.3);
    }

    #[test]
    fn chain_mail_bend_stiffness_is_zero() {
        let p = get_preset("Chain Mail").unwrap();
        assert_eq!(p.bend_stiffness, 0.0);
    }
}
