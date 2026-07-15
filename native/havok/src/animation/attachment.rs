/// Bone-attachment math primitive.
///
/// Provides `BoneAttachment` (the binding from a skeleton bone to an attached
/// object) and `compose_world_transform`, which computes the world-space
/// transform of the attached object given the skeleton's current model-space
/// bone transform.
///
/// Mirrors `hkaBoneAttachment` semantics: an attachment records the
/// bone-relative offset in `bone_from_attachment` (the transform that maps
/// from the attachment's local frame into the bone's local frame, i.e.
/// T_bone_from_attachment = inv(T_world_from_bone) * T_world_from_attachment).
///
/// To recover the attachment's world transform:
///   T_world_from_attachment = T_world_from_bone * T_bone_from_attachment
use crate::animation::pose::{
    QsTransform, quat_mul, quat_normalize, quat_rotate, vec3_add, vec3_scale,
};

/// Binding from a skeleton bone to an attached object node.
#[derive(Debug, Clone)]
pub struct BoneAttachment {
    /// Name of the bone on the skeleton that this attachment drives.
    pub bone_name: String,
    /// Name of the attached object (e.g. a NIF node or particle emitter).
    pub attached_object_name: String,
    /// Optional human-readable label.
    pub name: String,
    /// Offset from the bone's local frame to the attachment's local frame.
    /// Stored as (translation, rotation xyzw, scale).
    pub bone_from_attachment: QsTransform,
}

/// Compute the world-space transform of an attached object.
///
/// `bone_world` — model-space transform of the driving bone (from `Pose::model_at`).
/// `attachment` — the attachment descriptor.
///
/// Returns the world-space `QsTransform` of the attached object.
pub fn compose_world_transform(
    bone_world: &QsTransform,
    attachment: &BoneAttachment,
) -> QsTransform {
    QsTransform::compose(bone_world, &attachment.bone_from_attachment)
}

/// Convert a world-space attachment transform to bone-relative given the bone's
/// world transform. Useful when authoring: "I want the attachment at this
/// world position — what bone_from_attachment should I store?"
pub fn world_to_bone_relative(
    bone_world: &QsTransform,
    attachment_world: &QsTransform,
) -> QsTransform {
    // bone_from_attachment = inv(bone_world) * attachment_world
    // Inverse of a QsTransform (assuming uniform scale = 1 per component):
    //   inv_t = -q_conj(rot) * t / scale
    //   inv_r = q_conj(rot)
    //   inv_s = 1/scale
    let inv_rot = [
        -bone_world.rotation[0],
        -bone_world.rotation[1],
        -bone_world.rotation[2],
        bone_world.rotation[3],
    ];
    let neg_t = [
        -bone_world.translation[0],
        -bone_world.translation[1],
        -bone_world.translation[2],
    ];
    let inv_scale = [
        if bone_world.scale[0].abs() > 1e-10 {
            1.0 / bone_world.scale[0]
        } else {
            1.0
        },
        if bone_world.scale[1].abs() > 1e-10 {
            1.0 / bone_world.scale[1]
        } else {
            1.0
        },
        if bone_world.scale[2].abs() > 1e-10 {
            1.0 / bone_world.scale[2]
        } else {
            1.0
        },
    ];
    let neg_t_rot = quat_rotate(&inv_rot, &neg_t);
    let inv_t = [
        neg_t_rot[0] * inv_scale[0],
        neg_t_rot[1] * inv_scale[1],
        neg_t_rot[2] * inv_scale[2],
    ];
    let inv_bone = QsTransform {
        translation: inv_t,
        rotation: inv_rot,
        scale: inv_scale,
    };
    QsTransform::compose(&inv_bone, attachment_world)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::pose::QsTransform;

    fn approx_eq(a: &[f32; 3], b: &[f32; 3], eps: f32) -> bool {
        (a[0] - b[0]).abs() < eps && (a[1] - b[1]).abs() < eps && (a[2] - b[2]).abs() < eps
    }

    #[test]
    fn compose_identity_attachment() {
        let bone_world = QsTransform {
            translation: [1.0, 2.0, 3.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        };
        let attachment = BoneAttachment {
            bone_name: "Hand".into(),
            attached_object_name: "Weapon".into(),
            name: "grip".into(),
            bone_from_attachment: QsTransform::IDENTITY,
        };
        let result = compose_world_transform(&bone_world, &attachment);
        assert!(approx_eq(&result.translation, &[1.0, 2.0, 3.0], 1e-5));
    }

    #[test]
    fn compose_offset_attachment() {
        let bone_world = QsTransform::IDENTITY;
        let attachment = BoneAttachment {
            bone_name: "Root".into(),
            attached_object_name: "Marker".into(),
            name: "".into(),
            bone_from_attachment: QsTransform {
                translation: [0.0, 5.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
        };
        let result = compose_world_transform(&bone_world, &attachment);
        assert!(approx_eq(&result.translation, &[0.0, 5.0, 0.0], 1e-5));
    }

    #[test]
    fn round_trip_world_to_bone_relative() {
        let bone_world = QsTransform {
            translation: [2.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        };
        let attachment_world = QsTransform {
            translation: [2.0, 1.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        };
        let bone_from_att = world_to_bone_relative(&bone_world, &attachment_world);
        // Should give translation ~[0, 1, 0] in bone space.
        assert!(approx_eq(
            &bone_from_att.translation,
            &[0.0, 1.0, 0.0],
            1e-4
        ));
        // Round-trip: compose gives back attachment_world.
        let att = BoneAttachment {
            bone_name: "".into(),
            attached_object_name: "".into(),
            name: "".into(),
            bone_from_attachment: bone_from_att,
        };
        let back = compose_world_transform(&bone_world, &att);
        assert!(approx_eq(
            &back.translation,
            &attachment_world.translation,
            1e-4
        ));
    }
}
