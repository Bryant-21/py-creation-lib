//! FO4 weapon physics-system prototypes used by conversion transforms.
//!
//! The FO76 -> FO4 migration sometimes receives loose weapon collision shapes
//! without a `hknpPhysicsSystemData` root. Build that root as normal model data
//! here instead of embedding a repo-level binary template.

use crate::hkx::types::HkxValue;
use crate::hkx::{HkxMember, HkxObject};

fn member(name: &str, value: HkxValue) -> HkxMember {
    HkxMember {
        name: name.to_string(),
        value,
    }
}

fn null_string() -> HkxValue {
    HkxValue::String {
        value: String::new(),
        is_null: true,
    }
}

fn inline_object(members: Vec<HkxMember>) -> HkxValue {
    HkxValue::Object(members)
}

fn material_prototype() -> HkxValue {
    inline_object(vec![
        member("name", null_string()),
        member("isExclusive", HkxValue::Bool(false)),
        member("flags", HkxValue::U32(0)),
        member("triggerType", HkxValue::U8(0)),
        member(
            "triggerManifoldTolerance",
            inline_object(vec![member("value", HkxValue::U8(255))]),
        ),
        member("dynamicFriction", HkxValue::Half(1.75)),
        member("staticFriction", HkxValue::Half(1.75)),
        member("restitution", HkxValue::Half(1.700_195)),
        member("frictionCombinePolicy", HkxValue::U8(1)),
        member("restitutionCombinePolicy", HkxValue::U8(2)),
        member("weldingTolerance", HkxValue::Half(1.324_219)),
        member("maxContactImpulse", HkxValue::F32(f32::MAX)),
        member("fractionOfClippedImpulseToApply", HkxValue::Half(1.0)),
        member("massChangerCategory", HkxValue::U8(0)),
        member("massChangerHeavyObjectFactor", HkxValue::Half(1.875)),
        member("softContactForceFactor", HkxValue::Half(0.0)),
        member("softContactDampFactor", HkxValue::Half(0.0)),
        member(
            "softContactSeperationVelocity",
            inline_object(vec![member("value", HkxValue::U8(0))]),
        ),
        member("surfaceVelocity", HkxValue::Pointer(None)),
        member(
            "disablingCollisionsBetweenCvxCvxDynamicObjectsDistance",
            HkxValue::Half(2.3125),
        ),
        member("userData", HkxValue::U64(0)),
        member("isShared", HkxValue::Bool(false)),
    ])
}

pub(crate) fn motion_properties_prototype() -> HkxValue {
    inline_object(vec![
        member("isExclusive", HkxValue::Bool(false)),
        member("flags", HkxValue::U32(0)),
        member("gravityFactor", HkxValue::F32(1.0)),
        member("timeFactor", HkxValue::F32(1.0)),
        member("maxLinearSpeed", HkxValue::F32(104.375)),
        member("maxAngularSpeed", HkxValue::F32(31.570_312)),
        member("linearDamping", HkxValue::F32(0.100_098)),
        member("angularDamping", HkxValue::F32(0.050_049)),
        member("solverStabilizationSpeedThreshold", HkxValue::F32(0.17)),
        member("solverStabilizationSpeedReduction", HkxValue::F32(0.4905)),
        member("maxDistSqrd", HkxValue::F32(0.0025)),
        member("maxRotSqrd", HkxValue::F32(0.0025)),
        member("invBlockSize", HkxValue::F32(1.0)),
        member("pathingUpperThreshold", HkxValue::I16(26623)),
        member("pathingLowerThreshold", HkxValue::I16(-26213)),
        member("numDeactivationFrequencyPasses", HkxValue::U8(4)),
        member("deactivationVelocityScaleSquare", HkxValue::U8(115)),
        member("minimumPathingVelocityScaleSquare", HkxValue::U8(117)),
        member("spikingVelocityScaleThresholdSquared", HkxValue::U8(6)),
        member("minimumSpikingVelocityScaleSquared", HkxValue::U8(115)),
    ])
}

fn motion_cinfo_prototype() -> HkxValue {
    inline_object(vec![
        member("motionPropertiesId", HkxValue::U16(0)),
        member("enableDeactivation", HkxValue::Bool(true)),
        member("inverseMass", HkxValue::F32(2.0)),
        member("massFactor", HkxValue::F32(1069.528_6)),
        member(
            "maxLinearAccelerationDistancePerStep",
            HkxValue::F32(1.844_672_6e19),
        ),
        member(
            "maxRotationToPreventTunneling",
            HkxValue::F32(1.844_672_6e19),
        ),
        member(
            "inverseInertiaLocal",
            HkxValue::F32List(vec![287.250_2, 306.347_6, 2965.847_2, 1.0]),
        ),
        member(
            "centerOfMassWorld",
            HkxValue::F32List(vec![-0.006_23, 0.012_487, 0.013_444, -0.008_465]),
        ),
        member(
            "orientation",
            HkxValue::F32List(vec![0.221_445, -0.499_984, -0.500_015, 0.671_537]),
        ),
        member(
            "linearVelocity",
            HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
        ),
        member(
            "angularVelocity",
            HkxValue::F32List(vec![0.0, 0.0, 0.0, 0.0]),
        ),
    ])
}

fn body_cinfo_prototype() -> HkxValue {
    inline_object(vec![
        member("shape", HkxValue::Pointer(None)),
        member("reservedBodyId", HkxValue::I32(i32::MAX)),
        member("motionId", HkxValue::U32(0)),
        member("qualityId", HkxValue::U8(255)),
        member("materialId", HkxValue::U32(0)),
        member("collisionFilterInfo", HkxValue::U32(5)),
        member("flags", HkxValue::U32(128)),
        member("collisionLookAheadDistance", HkxValue::Half(0.0)),
        member("name", null_string()),
        member("userData", HkxValue::U64(0)),
        member(
            "position",
            HkxValue::F32List(vec![-0.000_238, 0.008_465, 0.005_992, 0.0]),
        ),
        member(
            "orientation",
            HkxValue::F32List(vec![0.370_788, -0.602_093, -0.370_788, 0.602_093]),
        ),
        member("spuFlags", HkxValue::U8(0)),
        member("localFrame", HkxValue::Pointer(None)),
    ])
}

/// Return a vanilla-shaped FO4 weapon `hknpPhysicsSystemData` object.
///
/// Callers resize the array members and wire `bodyCinfos[*].shape` plus
/// `referencedObjects[*]` to the source collision shapes being migrated.
pub fn fo4_weapon_psd_object_template() -> HkxObject {
    HkxObject {
        name: Some("#0001".to_string()),
        offset: 0,
        signature: 0xb857_718b,
        class_name: "hknpPhysicsSystemData".to_string(),
        members: vec![
            member("materials", HkxValue::Array(vec![material_prototype()])),
            member(
                "motionProperties",
                HkxValue::Array(vec![motion_properties_prototype()]),
            ),
            member(
                "motionCinfos",
                HkxValue::Array(vec![motion_cinfo_prototype()]),
            ),
            member("bodyCinfos", HkxValue::Array(vec![body_cinfo_prototype()])),
            member("constraintCinfos", HkxValue::Array(Vec::new())),
            member(
                "referencedObjects",
                HkxValue::Array(vec![HkxValue::Pointer(None)]),
            ),
            member("name", null_string()),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn array_len(object: &HkxObject, name: &str) -> usize {
        object
            .members
            .iter()
            .find(|member| member.name == name)
            .and_then(|member| match &member.value {
                HkxValue::Array(values) => Some(values.len()),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn weapon_psd_object_template_has_required_arrays() {
        let object = fo4_weapon_psd_object_template();
        assert_eq!(object.class_name, "hknpPhysicsSystemData");
        assert_eq!(array_len(&object, "materials"), 1);
        assert_eq!(array_len(&object, "motionProperties"), 1);
        assert_eq!(array_len(&object, "motionCinfos"), 1);
        assert_eq!(array_len(&object, "bodyCinfos"), 1);
        assert_eq!(array_len(&object, "constraintCinfos"), 0);
        assert_eq!(array_len(&object, "referencedObjects"), 1);
    }
}
