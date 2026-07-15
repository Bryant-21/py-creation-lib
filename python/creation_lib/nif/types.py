"""Basic mathematical types for NIF fields, with JSON serialization."""
from __future__ import annotations
from dataclasses import dataclass, field
from typing import Any


@dataclass
class Vector3:
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0


@dataclass
class Vector4:
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0
    w: float = 0.0


@dataclass
class Matrix33:
    rows: list[list[float]] = field(default_factory=lambda: [[0.0]*3 for _ in range(3)])

    @classmethod
    def identity(cls) -> Matrix33:
        return cls(rows=[[1, 0, 0], [0, 1, 0], [0, 0, 1]])


@dataclass
class Matrix44:
    rows: list[list[float]] = field(default_factory=lambda: [[0.0]*4 for _ in range(4)])

    @classmethod
    def identity(cls) -> Matrix44:
        return cls(rows=[[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]])


@dataclass
class Color3:
    r: float = 0.0
    g: float = 0.0
    b: float = 0.0


@dataclass
class Color4:
    r: float = 0.0
    g: float = 0.0
    b: float = 0.0
    a: float = 1.0


@dataclass
class Quaternion:
    w: float = 1.0
    x: float = 0.0
    y: float = 0.0
    z: float = 0.0


# --- JSON serialization ---

_TYPE_MAP = {
    "Vector3": (Vector3, ["x", "y", "z"]),
    "Vector4": (Vector4, ["x", "y", "z", "w"]),
    "Color3": (Color3, ["r", "g", "b"]),
    "Color4": (Color4, ["r", "g", "b", "a"]),
    "Quaternion": (Quaternion, ["w", "x", "y", "z"]),
}


def to_json(obj: Any) -> Any:
    """Convert a type instance to a JSON-safe value."""
    if isinstance(obj, (Vector3, Vector4, Color3, Color4, Quaternion)):
        cls_name = type(obj).__name__
        _, attrs = _TYPE_MAP[cls_name]
        return [getattr(obj, a) for a in attrs]
    if isinstance(obj, (Matrix33, Matrix44)):
        return [list(row) for row in obj.rows]
    return obj


def from_json(type_name: str, val: Any) -> Any:
    """Convert a JSON value back to a type instance."""
    if type_name in _TYPE_MAP:
        cls, attrs = _TYPE_MAP[type_name]
        if isinstance(val, (list, tuple)) and len(val) == len(attrs):
            return cls(**{a: val[i] for i, a in enumerate(attrs)})
    if type_name == "Matrix33" and isinstance(val, list):
        return Matrix33(rows=[list(row) for row in val])
    if type_name == "Matrix44" and isinstance(val, list):
        return Matrix44(rows=[list(row) for row in val])
    return val


# --- FO4 Block Type Categories (for UI: Insert Block, scene tree colors) ---

BLOCK_CATEGORIES = {
    "Scene Nodes": {
        "rule": "inherits NiNode",
        "types": [
            "NiNode", "BSFadeNode", "BSMultiBoundNode", "BSLeafAnimNode",
            "BSOrderedNode", "BSRangeNode", "BSValueNode", "BSBlastNode",
            "BSDebrisNode", "BSDamageStage", "BSMultiBound",
        ],
    },
    "Geometry": {
        "rule": "inherits BSTriShape or NiTriShape",
        "types": [
            "BSTriShape", "BSSubIndexTriShape", "BSMeshLODTriShape",
            "BSDynamicTriShape", "NiTriShape", "NiTriStrips",
            "BSSegmentedTriShape",
        ],
    },
    "Shader Properties": {
        "rule": "inherits BSShaderProperty or is shader-related NiProperty",
        "types": [
            "BSLightingShaderProperty", "BSEffectShaderProperty",
            "BSWaterShaderProperty", "BSSkyShaderProperty",
            "BSShaderPPLightingProperty", "BSShaderNoLightingProperty",
        ],
    },
    "Material Data": {
        "rule": "BSShaderTextureSet and related",
        "types": ["BSShaderTextureSet"],
    },
    "Alpha/Blending": {
        "rule": "alpha and stencil properties",
        "types": ["NiAlphaProperty", "NiStencilProperty"],
    },
    "Extra Data": {
        "rule": "inherits NiExtraData or BSConnectPoint",
        "types": [
            "NiStringExtraData", "NiIntegerExtraData", "NiFloatExtraData",
            "NiBooleanExtraData", "NiBinaryExtraData", "NiIntegersExtraData",
            "NiFloatsExtraData", "NiStringsExtraData",
            "BSXFlags", "BSBehaviorGraphExtraData",
            "BSDecalPlacementVectorExtraData", "BSDistantObjectExtraData",
            "BSInvMarker", "BSBoneLODExtraData", "BSClothExtraData",
            "BSConnectPoint::Parents", "BSConnectPoint::Children",
            "BSPositionData", "BSFurnitureMarkerNode",
            "BSWArray", "NiExtraData",
        ],
    },
    "Animation Controllers": {
        "rule": "inherits NiTimeController or NiInterpolator or NiSequence",
        "types": [
            "NiControllerManager", "NiControllerSequence",
            "NiMultiTargetTransformController", "NiTransformController",
            "NiTransformInterpolator", "NiTransformData",
            "NiFloatInterpolator", "NiFloatData",
            "NiBlendFloatInterpolator", "NiBlendTransformInterpolator",
            "NiPoint3Interpolator", "NiPoint3Data",
            "NiBoolInterpolator", "NiBoolData",
            "NiPSysUpdateCtlr", "NiPSysEmitterCtlr",
            "NiDefaultAVObjectPalette", "NiTextKeyExtraData",
            "BSLagBoneController", "BSProceduralLightningController",
        ],
    },
    "Skinning": {
        "rule": "skin-related blocks",
        "types": [
            "NiSkinInstance", "NiSkinData", "NiSkinPartition",
            "BSSkin::Instance", "BSSkin::BoneData",
            "BSDismemberSkinInstance",
        ],
    },
    "Collision": {
        "rule": "type name starts with bhk (excluding constraints)",
        "types": [
            "bhkCollisionObject", "bhkRigidBody", "bhkRigidBodyT",
            "bhkBoxShape", "bhkSphereShape", "bhkCapsuleShape",
            "bhkConvexVerticesShape", "bhkMoppBvTreeShape",
            "bhkCompressedMeshShape", "bhkCompressedMeshShapeData",
            "bhkListShape", "bhkConvexTransformShape",
            "bhkNiTriStripsShape", "bhkSimpleShapePhantom",
            "bhkBlendCollisionObject", "bhkSPCollisionObject",
            "bhkPhysicsSystem", "bhkRagdollSystem",
        ],
    },
    "Constraints": {
        "rule": "inherits bhkConstraint",
        "types": [
            "bhkLimitedHingeConstraint", "bhkRagdollConstraint",
            "bhkHingeConstraint", "bhkBallAndSocketConstraint",
            "bhkStiffSpringConstraint", "bhkBreakableConstraint",
            "bhkMalleableConstraint",
        ],
    },
    "Particles": {
        "rule": "inherits NiParticleSystem or NiPSys*",
        "types": [
            "NiParticleSystem", "NiPSysData", "NiPSysEmitter",
            "NiPSysMeshEmitter", "NiPSysBoxEmitter", "NiPSysSphereEmitter",
            "NiPSysCylinderEmitter", "NiPSysGravityModifier",
            "NiPSysRotationModifier", "NiPSysSpawnModifier",
            "NiPSysPositionModifier", "NiPSysBoundUpdateModifier",
            "NiPSysDragModifier", "NiPSysColorModifier",
            "NiPSysGrowFadeModifier", "BSStripParticleSystem",
        ],
    },
}


def categorize_block_type(type_name: str, schema=None) -> str:
    """Return the category name for a block type.
    Falls back to schema inheritance check, then 'Other'."""
    # Direct lookup in hardcoded types
    for cat_name, cat_info in BLOCK_CATEGORIES.items():
        if type_name in cat_info["types"]:
            return cat_name

    # Schema-based fallback
    if schema:
        hierarchy = schema.get_type_hierarchy(type_name)
        for ancestor in hierarchy:
            for cat_name, cat_info in BLOCK_CATEGORIES.items():
                if ancestor in cat_info["types"]:
                    return cat_name
        # Prefix-based fallback for collision
        if type_name.startswith("bhk"):
            return "Constraints" if "Constraint" in type_name else "Collision"
        if type_name.startswith("NiPSys") or type_name.startswith("BSPSys"):
            return "Particles"

    return "Other"
