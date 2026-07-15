use crate::cloth::setup::constraint_setup::{
    BendStiffnessSetup, BonePlanesSetup, ConstraintSetupObject, LocalRangeSetup, StandardLinkSetup,
    StretchLinkSetup, VolumeSetup,
};
use crate::cloth::setup::types::VertexFloatInput;
use crate::error::{HavokError, HavokResult};

/// Named constraint recipe for a cloth region.
pub struct TopologyPreset {
    pub name: &'static str,
    pub description: &'static str,

    pub use_standard_links: bool,
    pub use_stretch_links: bool,
    pub use_bend_stiffness: bool,
    pub use_local_range: bool,
    pub use_bone_planes: bool,
    pub use_volume: bool,

    pub standard_link_stiffness: f32,
    pub stretch_link_stiffness: f32,
    pub bend_stiffness: f32,
    pub local_range_max_distance: f32,
    pub local_range_stiffness: f32,
    pub volume_stiffness: f32,

    pub default_material: &'static str,
    pub auto_capsule_radius: f32,
    pub num_substeps: u32,
    pub num_solve_iterations: u32,
}

impl TopologyPreset {
    /// Generate constraint setup objects for this topology.
    pub fn build_constraints(
        &self,
        region_name: &str,
        _num_particles: usize,
        _fixed_indices: &[usize],
    ) -> Vec<ConstraintSetupObject> {
        let mut constraints = Vec::new();

        if self.use_standard_links {
            let mut s = StandardLinkSetup::default();
            s.name = format!("{region_name}_StandardLinks");
            s.stiffness = VertexFloatInput::constant(self.standard_link_stiffness);
            constraints.push(ConstraintSetupObject::StandardLink(s));
        }

        if self.use_stretch_links {
            let mut s = StretchLinkSetup::default();
            s.name = format!("{region_name}_StretchLinks");
            s.stiffness = VertexFloatInput::constant(self.stretch_link_stiffness);
            constraints.push(ConstraintSetupObject::StretchLink(s));
        }

        if self.use_bend_stiffness {
            let mut s = BendStiffnessSetup::default();
            s.name = format!("{region_name}_BendStiffness");
            s.bend_stiffness = VertexFloatInput::constant(self.bend_stiffness);
            constraints.push(ConstraintSetupObject::BendStiffness(s));
        }

        if self.use_local_range {
            let mut s = LocalRangeSetup::default();
            s.name = format!("{region_name}_LocalRange");
            s.maximum_distance = VertexFloatInput::constant(self.local_range_max_distance);
            s.stiffness = self.local_range_stiffness;
            constraints.push(ConstraintSetupObject::LocalRange(s));
        }

        if self.use_bone_planes {
            let mut s = BonePlanesSetup::default();
            s.name = format!("{region_name}_BonePlanes");
            constraints.push(ConstraintSetupObject::BonePlanes(s));
        }

        if self.use_volume {
            let mut s = VolumeSetup::default();
            s.name = format!("{region_name}_Volume");
            s.stiffness = VertexFloatInput::constant(self.volume_stiffness);
            constraints.push(ConstraintSetupObject::Volume(s));
        }

        constraints
    }
}

// ---------------------------------------------------------------------------
// Built-in topology presets — mirrors Python PRESETS
// ---------------------------------------------------------------------------

pub const THIN_CLOTH: TopologyPreset = TopologyPreset {
    name: "thin_cloth",
    description: "Light, flowing fabric — low stiffness, minimal bend resistance.",
    use_standard_links: true,
    use_stretch_links: true,
    use_bend_stiffness: true,
    use_local_range: false,
    use_bone_planes: false,
    use_volume: false,
    standard_link_stiffness: 0.5,
    stretch_link_stiffness: 0.6,
    bend_stiffness: 0.05,
    local_range_max_distance: 5.0,
    local_range_stiffness: 0.8,
    volume_stiffness: 0.5,
    default_material: "Silk",
    auto_capsule_radius: 4.0,
    num_substeps: 1,
    num_solve_iterations: 3,
};

pub const THICK_CLOTH: TopologyPreset = TopologyPreset {
    name: "thick_cloth",
    description: "Heavy, structured fabric — high stiffness, strong bend resistance.",
    use_standard_links: true,
    use_stretch_links: true,
    use_bend_stiffness: true,
    use_local_range: true,
    use_bone_planes: false,
    use_volume: false,
    standard_link_stiffness: 0.85,
    stretch_link_stiffness: 0.9,
    bend_stiffness: 0.6,
    local_range_max_distance: 8.0,
    local_range_stiffness: 0.7,
    volume_stiffness: 0.5,
    default_material: "Leather",
    auto_capsule_radius: 6.0,
    num_substeps: 1,
    num_solve_iterations: 4,
};

pub const CHAIN: TopologyPreset = TopologyPreset {
    name: "chain",
    description: "Linked chain segments — very high link stiffness, no bend resistance.",
    use_standard_links: true,
    use_stretch_links: true,
    use_bend_stiffness: false,
    use_local_range: true,
    use_bone_planes: false,
    use_volume: false,
    standard_link_stiffness: 0.98,
    stretch_link_stiffness: 0.99,
    bend_stiffness: 0.0,
    local_range_max_distance: 3.0,
    local_range_stiffness: 0.95,
    volume_stiffness: 0.5,
    default_material: "Chain Mail",
    auto_capsule_radius: 3.0,
    num_substeps: 2,
    num_solve_iterations: 6,
};

pub const SKIRT_FLAPS: TopologyPreset = TopologyPreset {
    name: "skirt_flaps",
    description: "Split panels that flap independently — moderate stiffness, bone-plane containment.",
    use_standard_links: true,
    use_stretch_links: true,
    use_bend_stiffness: true,
    use_local_range: false,
    use_bone_planes: true,
    use_volume: false,
    standard_link_stiffness: 0.6,
    stretch_link_stiffness: 0.7,
    bend_stiffness: 0.2,
    local_range_max_distance: 5.0,
    local_range_stiffness: 0.8,
    volume_stiffness: 0.5,
    default_material: "Cotton",
    auto_capsule_radius: 5.0,
    num_substeps: 1,
    num_solve_iterations: 3,
};

pub const SOFT_BODY: TopologyPreset = TopologyPreset {
    name: "soft_body",
    description: "Volume-preserving soft body — high stiffness with volume constraint.",
    use_standard_links: true,
    use_stretch_links: true,
    use_bend_stiffness: true,
    use_local_range: true,
    use_bone_planes: false,
    use_volume: true,
    standard_link_stiffness: 0.7,
    stretch_link_stiffness: 0.8,
    bend_stiffness: 0.4,
    local_range_max_distance: 4.0,
    local_range_stiffness: 0.6,
    volume_stiffness: 0.8,
    default_material: "Squishy",
    auto_capsule_radius: 4.0,
    num_substeps: 1,
    num_solve_iterations: 4,
};

/// All topology presets.
pub const TOPOLOGY_PRESETS: &[&TopologyPreset] =
    &[&THIN_CLOTH, &THICK_CLOTH, &CHAIN, &SKIRT_FLAPS, &SOFT_BODY];

/// Look up a topology preset by name (case-insensitive).
pub fn get_preset(name: &str) -> HavokResult<&'static TopologyPreset> {
    if let Some(p) = TOPOLOGY_PRESETS.iter().find(|p| p.name == name) {
        return Ok(p);
    }
    let lower = name.to_lowercase();
    if let Some(p) = TOPOLOGY_PRESETS
        .iter()
        .find(|p| p.name.to_lowercase() == lower)
    {
        return Ok(p);
    }
    Err(HavokError::InvalidInput(format!(
        "Unknown topology preset: {:?}. Available: {:?}",
        name,
        TOPOLOGY_PRESETS.iter().map(|p| p.name).collect::<Vec<_>>()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_cloth_has_standard_stretch_and_bend() {
        let c = THIN_CLOTH.build_constraints("Test", 50, &[]);
        assert_eq!(c.len(), 3);
        assert!(c.iter().any(|x| x.setup_type() == "StandardLink"));
        assert!(c.iter().any(|x| x.setup_type() == "BendStiffness"));
    }

    #[test]
    fn chain_has_no_bend() {
        let c = CHAIN.build_constraints("Ch", 50, &[]);
        assert!(!c.iter().any(|x| x.setup_type() == "BendStiffness"));
    }
}
