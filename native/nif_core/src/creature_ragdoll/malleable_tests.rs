use super::*;
use havok_native::convert::creature_ragdoll::{
    lower_primitive_shapes_to_fo4_convex_hulls, reconstruct_fo4_creature_ragdoll_packfile,
};
use havok_native::hkx::{HkxFile, types::HkxValue};

fn vector(x: f64, y: f64, z: f64) -> NifValue {
    NifValue::Struct(IndexMap::from([
        ("x".to_string(), NifValue::Float(x)),
        ("y".to_string(), NifValue::Float(y)),
        ("z".to_string(), NifValue::Float(z)),
        ("w".to_string(), NifValue::Float(0.0)),
    ]))
}

fn malleable_fixture() -> (NifFile, usize) {
    let mut nif = NifFile::new("fnv");
    let constraint = nif.add_block("bhkMalleableConstraint", None);
    let mut ragdoll = IndexMap::new();
    for side in ["A", "B"] {
        ragdoll.insert(format!("Twist {side}"), vector(1.0, 0.0, 0.0));
        ragdoll.insert(format!("Plane {side}"), vector(0.0, 1.0, 0.0));
        ragdoll.insert(format!("Motor {side}"), vector(0.0, 0.0, 1.0));
        ragdoll.insert(format!("Pivot {side}"), vector(0.25, 0.5, 0.75));
    }
    for (name, value) in [
        ("Cone Max Angle", 0.78),
        ("Plane Min Angle", -0.87),
        ("Plane Max Angle", 0.52),
        ("Twist Min Angle", -0.17),
        ("Twist Max Angle", 0.17),
        ("Max Friction", 100.0),
    ] {
        ragdoll.insert(name.to_string(), NifValue::Float(value));
    }
    nif.blocks[constraint].set_field(
        "Constraint Info",
        NifValue::Struct(IndexMap::from([
            ("Entity A".to_string(), NifValue::Ref(10)),
            ("Entity B".to_string(), NifValue::Ref(20)),
        ])),
    );
    nif.blocks[constraint].set_field(
        "Constraint",
        NifValue::Struct(IndexMap::from([
            ("Type".to_string(), NifValue::UInt(7)),
            ("Ragdoll".to_string(), NifValue::Struct(ragdoll)),
            ("Strength".to_string(), NifValue::Float(0.9)),
        ])),
    );
    (nif, constraint)
}

#[test]
fn malleable_source_constraint_keeps_strength_limits_and_scaled_pivots() {
    let (nif, _) = malleable_fixture();
    let constraints = source_constraints(&nif, &HashMap::from([(10, 1), (20, 2)])).unwrap();
    assert_eq!(constraints.len(), 1);
    assert_eq!((constraints[0].body_a, constraints[0].body_b), (10, 20));
    let ConstraintKindIr::Malleable { inner, strength } = &constraints[0].kind else {
        panic!("wrapper lost")
    };
    assert_eq!(*strength, 0.9);
    let ConstraintKindIr::Ragdoll(limits) = inner.as_ref() else {
        panic!("ragdoll lost")
    };
    assert_eq!(limits.max_friction_torque, 100.0);
    assert_eq!(limits.twist_min, -0.17);
    assert_eq!(limits.twist_max, 0.17);
    assert_eq!(
        limits.frame_a.translation,
        [0.25, 0.5, 0.75].map(|v| v * LEGACY_BHK_TO_FO4_CHARACTER_SCALE)
    );
}

#[test]
fn malleable_source_constraint_rejects_unknown_inner_type() {
    let (mut nif, constraint) = malleable_fixture();
    let Some(NifValue::Struct(fields)) = nif.blocks[constraint].get_field_mut("Constraint") else {
        unreachable!()
    };
    fields.insert("Type".to_string(), NifValue::UInt(99));
    assert!(matches!(
        parse_malleable(&nif.blocks[constraint]),
        Err(LegacyCreatureRagdollError::UnsupportedConstraint { .. })
    ));
}

#[test]
#[ignore = "requires MODKIT_FNV_CREATURE_MESH_ROOT source game assets"]
fn source_fnv_malleable_ragdoll_corpus_packs_strength_without_donor_constraints() {
    let root = std::path::PathBuf::from(
        std::env::var_os("MODKIT_FNV_CREATURE_MESH_ROOT").expect("source mesh root"),
    );
    for relative in [
        "characters/_male/skeleton.nif",
        "nvdlc01/creatures/ghosts/skeleton.nif",
        "nvdlc01/creatures/hologram/skeleton.nif",
        "nvdlc03/creatures/hologram/skeleton.nif",
    ] {
        let nif = NifFile::load(root.join(relative)).unwrap();
        let expected = nif
            .find_blocks("bhkMalleableConstraint")
            .into_iter()
            .map(|id| {
                let fields = required_struct(&nif.blocks[id], "Constraint").unwrap();
                required_float(fields, "Strength", id).unwrap()
            })
            .collect::<Vec<_>>();
        assert!(!expected.is_empty(), "{relative}");
        let ir = extract_fnv_fo3_creature_ragdoll(&nif)
            .unwrap_or_else(|error| panic!("{relative}: {error}"));
        let ir = lower_primitive_shapes_to_fo4_convex_hulls(&ir, 12).unwrap();
        let packed = reconstruct_fo4_creature_ragdoll_packfile(&ir).unwrap();
        let embedded = reconstruct_fo4_embedded_creature_ragdoll(&ir).unwrap();
        for bytes in [packed, embedded.binary_data] {
            let reread = HkxFile::read(&bytes).unwrap();
            let mut strengths = reread
                .objects()
                .iter()
                .filter(|object| object.class_name == "hknpMalleableConstraintData")
                .map(|object| {
                    let strength = &object
                        .members
                        .iter()
                        .find(|member| member.name == "strength")
                        .unwrap()
                        .value;
                    let HkxValue::F32(value) = strength else {
                        panic!("invalid strength")
                    };
                    *value
                })
                .collect::<Vec<_>>();
            let mut expected = expected.clone();
            strengths.sort_by(f32::total_cmp);
            expected.sort_by(f32::total_cmp);
            assert_eq!(strengths, expected, "{relative}");
        }
    }
}
