use nif_core_native::fo4_block_types::*;

#[test]
fn known_fo4_block_types_are_present() {
    for name in [
        "NiNode",
        "BSTriShape",
        "BSSubIndexTriShape",
        "BSSkin::Instance",
        "BSSkin::BoneData",
        "BSLightingShaderProperty",
        "BSEffectShaderProperty",
        "NiAlphaProperty",
        "bhkNPCollisionObject",
        "bhkPhysicsSystem",
        "BSXFlags",
        "NiControllerManager",
        "BSLightingShaderPropertyFloatController",
    ] {
        assert!(
            is_fo4_block_type(name),
            "{name} should be an FO4 block type"
        );
    }
}

#[test]
fn legacy_gamebryo_block_types_are_absent() {
    for name in [
        "BSDismemberSkinInstance",
        "BSRefractionFirePeriodController",
        "BSRefractionStrengthController",
        "NiGeomMorpherController",
        "NiTexturingProperty",
        "NiSourceTexture",
        "bhkRigidBody",
        "bhkBoxShape",
        "bhkSimpleShapePhantom",
        "BSDecalPlacementVectorExtraData",
    ] {
        assert!(
            !is_fo4_block_type(name),
            "{name} should not be an FO4 block type"
        );
    }
}

#[test]
fn table_is_sorted_for_binary_search() {
    assert!(FO4_BLOCK_TYPES.windows(2).all(|pair| pair[0] < pair[1]));
}
