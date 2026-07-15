use serde::{Deserialize, Serialize};

use super::types::VertexSelectionInput;

pub type Vec4 = [f32; 4];

/// Capsule collision shape in bone-local space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapsuleShapeSetup {
    pub start: Vec4,
    pub end: Vec4,
    pub big_radius: f32,
    pub small_radius: f32,
}

impl Default for CapsuleShapeSetup {
    fn default() -> Self {
        Self {
            start: [0.0; 4],
            end: [0.0; 4],
            big_radius: 0.0,
            small_radius: 0.0,
        }
    }
}

/// Tapered capsule collision shape (FO4 NPC limb cloth).
///
/// Authored as the convex hull of two spheres with different radii. The
/// runtime stores additional precomputed geometry fields (cone apex/axis,
/// theta angles, etc.) which `bake.rs` derives from these four authored
/// inputs at emit time.
///
/// Mirrors `hclTaperedCapsuleShape` (refs/hk2018_1_0_r1/Source/Cloth/Cloth/
/// Collide/Shape/TaperedCapsule/hclTaperedCapsuleShape.h).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaperedCapsuleShapeSetup {
    /// Center of the small sphere in bone-local space.
    pub small: Vec4,
    /// Center of the big sphere in bone-local space.
    pub big: Vec4,
    pub small_radius: f32,
    pub big_radius: f32,
}

impl Default for TaperedCapsuleShapeSetup {
    fn default() -> Self {
        Self {
            small: [0.0; 4],
            big: [0.0; 4],
            small_radius: 0.0,
            big_radius: 0.0,
        }
    }
}

/// Discriminator for the shape variant attached to a `CollidableSetup`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShapeSetup {
    Capsule(CapsuleShapeSetup),
    TaperedCapsule(TaperedCapsuleShapeSetup),
}

/// Per-instance collidable associated with a sim cloth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollidableSetup {
    #[serde(default)]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<CapsuleShapeSetup>,
    /// Tapered capsule shape. When present, takes precedence over `shape`
    /// (a CollidableSetup carries one shape variant; the two fields are
    /// mutually exclusive in practice).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tapered_shape: Option<TaperedCapsuleShapeSetup>,
    #[serde(default)]
    pub colliding_particles: VertexSelectionInput,
    #[serde(default)]
    pub driving_bone_name: String,
    #[serde(default)]
    pub pinch_detection_enabled: bool,
    #[serde(default)]
    pub pinch_detection_priority: i32,
    #[serde(default)]
    pub pinch_detection_radius: f32,
}

impl Default for CollidableSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            shape: None,
            tapered_shape: None,
            colliding_particles: VertexSelectionInput::default(),
            driving_bone_name: String::new(),
            pinch_detection_enabled: false,
            pinch_detection_priority: 0,
            pinch_detection_radius: 0.0,
        }
    }
}
