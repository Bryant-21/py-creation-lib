use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use crate::error::{HavokError, HavokResult};

pub const HAVOK_VERSION: &str = "hk_2014.1.0-r1";

// Complete enumeration of every concrete `hcl*` / `hk*` class observed in
// FO4 BSClothExtraData blobs and declared in the Havok 2018.1.0 SDK
// (`refs/hk2018_1_0_r1/Source/Cloth/Cloth/`). Sorted alphabetically.
//
const KNOWN_CLASS_LIST: &[&str] = &[
    // ---- HKX metadata / root ----
    "hkClass",
    "hkClassEnum",
    "hkClassEnumItem",
    "hkClassMember",
    "hkRootLevelContainer",
    "hkaSkeleton",
    // ---- Action ----
    "hclAction",
    "hclSimpleWindAction",
    // ---- Buffer / TransformSet ----
    "hclBufferDefinition",
    "hclSceneDataBuffer",
    "hclScratchBuffer",
    "hclScratchBufferDefinition",
    "hclShadowBuffer",
    "hclShadowBufferDefinition",
    "hclStaticShadowBuffer",
    "hclStaticShadowBufferDefinition",
    "hclTransformSet",
    "hclTransformSetDefinition",
    // ---- Cloth data / Container ----
    "hclClothContainer",
    "hclClothData",
    "hclClothInstance",
    "hclSimClothData",
    "hclSimClothInstance",
    "hclSimClothPose",
    "hclVirtualCollisionPointsData",
    // ---- Collide ----
    "hclCapsuleShape",
    "hclCollidable",
    "hclCollisionConvexes",
    "hclCollisionTriangles",
    "hclConvexGeometryShape",
    "hclConvexHeightFieldShape",
    "hclConvexPlanesShape",
    "hclPlaneShape",
    "hclPointContactPlanesShape",
    "hclShape",
    "hclSphereShape",
    "hclTaperedCapsuleShape",
    // ---- Constraint sets (concrete + _Mx SIMD batched variants) ----
    "hclAntiPinchConstraintSet",
    "hclBendLinkConstraintSet",
    "hclBendLinkConstraintSetMx",
    "hclBendStiffnessConstraintSet",
    "hclBendStiffnessConstraintSetMx",
    "hclBonePlanesConstraintSet",
    "hclCompressibleLinkConstraintSet",
    "hclCompressibleLinkConstraintSetMx",
    "hclConstraintSet",
    "hclContactPointSet",
    "hclLocalRangeConstraintSet",
    "hclStandardLinkConstraintSet",
    "hclStandardLinkConstraintSetMx",
    "hclStretchLinkConstraintSet",
    "hclStretchLinkConstraintSetMx",
    "hclTransitionConstraintSet",
    "hclVolumeConstraint",
    "hclVolumeConstraintMx",
    // ---- Operators (full SDK list — 25 concrete) ----
    "hclBlendSomeVerticesOperator",
    "hclBoneSpaceMeshMeshDeformPNOperator",
    "hclBoneSpaceMeshMeshDeformPNTBOperator",
    "hclBoneSpaceMeshMeshDeformPNTOperator",
    "hclBoneSpaceMeshMeshDeformPOperator",
    "hclBoneSpaceSkinPNOperator",
    "hclBoneSpaceSkinPNTBOperator",
    "hclBoneSpaceSkinPNTOperator",
    "hclBoneSpaceSkinPOperator",
    "hclBoneSpaceTransferSimulationOperator",
    "hclCopyVerticesOperator",
    "hclGatherAllVerticesOperator",
    "hclGatherSomeVerticesOperator",
    "hclInputConvertOperator",
    "hclMeshBoneDeformOperator",
    "hclMeshMeshDeformOperator",
    "hclMoveParticlesOperator",
    "hclObjectSpaceMeshMeshDeformPNOperator",
    "hclObjectSpaceMeshMeshDeformPNTBOperator",
    "hclObjectSpaceMeshMeshDeformPNTOperator",
    "hclObjectSpaceMeshMeshDeformPOperator",
    "hclObjectSpaceSkinPNOperator",
    "hclObjectSpaceSkinPNTBOperator",
    "hclObjectSpaceSkinPNTOperator",
    "hclObjectSpaceSkinPOperator",
    "hclObjectSpaceTransferSimulationOperator",
    "hclOperator",
    "hclOutputConvertOperator",
    "hclSimpleMeshBoneDeformOperator",
    "hclSimulateOperator",
    "hclSkinOperator",
    "hclUpdateAllVertexFramesOperator",
    "hclUpdateSomeVertexFramesOperator",
    // ---- State ----
    "hclClothState",
    "hclStateDependencyGraph",
    "hclStateOperatorMask",
    "hclStateTransition",
];

pub static KNOWN_CLASSES: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| KNOWN_CLASS_LIST.iter().copied().collect());

pub fn is_known(class_name: &str) -> bool {
    KNOWN_CLASSES.contains(class_name)
}

#[derive(serde::Deserialize)]
struct ClassnameEntry {
    name: String,
}

pub fn expand_from_fixture(path: &Path) -> HavokResult<HashSet<String>> {
    let bytes = std::fs::read(path).map_err(|source| HavokError::Io {
        path: path.display().to_string(),
        operation: "read",
        source,
    })?;
    let entries: Vec<ClassnameEntry> = serde_json::from_slice(&bytes).map_err(|error| {
        HavokError::InvalidInput(format!(
            "failed to parse classnames JSON at {}: {error}",
            path.display()
        ))
    })?;
    Ok(entries.into_iter().map(|entry| entry.name).collect())
}
