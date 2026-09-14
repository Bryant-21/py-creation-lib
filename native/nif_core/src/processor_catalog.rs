#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NifProcessorDescriptor {
    pub id: &'static str,
    pub class_name: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub games: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub parity: &'static str,
}

const TES3_TO_FO4: &[&str] = &[
    "morrowind",
    "oblivion",
    "fo3",
    "fnv",
    "skyrim",
    "skyrimse",
    "fo4",
];
const TES4_TO_FO4: &[&str] = &["oblivion", "fo3", "fnv", "skyrim", "skyrimse", "fo4"];
const TES4_TO_FNV: &[&str] = &["oblivion", "fo3", "fnv"];
const FO3_TO_FO4: &[&str] = &["fo3", "fnv", "skyrim", "skyrimse", "fo4"];
const TES4_TO_SSE: &[&str] = &["oblivion", "fo3", "fnv", "skyrim", "skyrimse"];
const FO3_FNV: &[&str] = &["fo3", "fnv"];

macro_rules! processor {
    ($id:literal, $class:literal, $title:literal, $category:literal, $games:expr, [$($extension:literal),+], $parity:literal) => {
        NifProcessorDescriptor {
            id: $id,
            class_name: $class,
            title: $title,
            category: $category,
            games: $games,
            extensions: &[$($extension),+],
            parity: $parity,
        }
    };
}

pub const NIF_PROCESSORS: &[NifProcessorDescriptor] = &[
    processor!(
        "ps4-converter",
        "TProcPS4Converter",
        "Convert Fallout 4 mesh to PS4",
        "NIF",
        &["fo4"],
        ["nif", "bto"],
        "native"
    ),
    processor!(
        "update-tangents",
        "TProcTangents",
        "Update tangents and binormals",
        "NIF",
        TES4_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "update-bounds",
        "TProcUpdateBounds",
        "Update bounds",
        "NIF",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "optimize-mesh",
        "TProcOptimize",
        "Optimize mesh",
        "NIF",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "replace-assets",
        "TProcReplaceAssets",
        "Search and replace assets",
        "NIF",
        TES3_TO_FO4,
        ["nif", "bgsm", "bgem"],
        "partial"
    ),
    processor!(
        "json-converter",
        "TProcJsonConverter",
        "Convert to and from JSON",
        "NIF",
        TES3_TO_FO4,
        ["nif", "kf", "json"],
        "partial"
    ),
    processor!(
        "universal-tweaker",
        "TProcUniversalTweaker",
        "Universal tweaker",
        "NIF",
        TES3_TO_FO4,
        ["nif", "kf", "bgsm", "bgem"],
        "partial"
    ),
    processor!(
        "universal-fixer",
        "TProcUniversalFixer",
        "Universal fixer",
        "NIF",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "apply-transform",
        "TProcApplyTransform",
        "Apply transformation",
        "NIF",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "adjust-transform",
        "TProcAdjustTransform",
        "Adjust transformation",
        "NIF",
        TES3_TO_FO4,
        ["nif", "kf"],
        "partial"
    ),
    processor!(
        "attach-parent",
        "TProcAttachParent",
        "Attach parent NiNode",
        "NIF",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "copy-geometry-blocks",
        "TProcCopyGeometryBlocks",
        "Copy geometry blocks",
        "NIF",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "vertex-paint",
        "TProcVertexPaint",
        "Vertex color painting",
        "NIF",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "group-shapes",
        "TProcGroupShapes",
        "Group shapes",
        "NIF",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "merge-shapes",
        "TProcMergeShapes",
        "Merge shapes",
        "NIF",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "merge-properties",
        "TProcMergeProperties",
        "Merge properties",
        "NIF",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "remove-nodes",
        "TProcRemoveNodes",
        "Remove nodes",
        "NIF",
        TES3_TO_FO4,
        ["nif", "kf"],
        "partial"
    ),
    processor!(
        "remove-unused-nodes",
        "TProcRemoveUnusedNodes",
        "Remove unused nodes",
        "NIF",
        TES3_TO_FO4,
        ["nif", "kf", "kfm"],
        "partial"
    ),
    processor!(
        "convert-block-type",
        "TProcConvertRootNode",
        "Convert block type",
        "NIF",
        TES4_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "unskin-mesh",
        "TProcUnskinMesh",
        "Unskin mesh",
        "NIF",
        &["oblivion", "fo3", "fnv", "skyrim"],
        ["nif"],
        "partial"
    ),
    processor!(
        "add-lod-node",
        "TProcAddLODNode",
        "Add NiLODNode",
        "NIF",
        TES4_TO_FNV,
        ["nif"],
        "partial"
    ),
    processor!(
        "add-root-collision-node",
        "TProcAddRootCollisionNode",
        "Add RootCollisionNode",
        "NIF",
        &["morrowind"],
        ["nif"],
        "partial"
    ),
    processor!(
        "add-bounding-box",
        "TProcAddBoundingBox",
        "Add bounding box",
        "NIF",
        &["morrowind"],
        ["nif"],
        "partial"
    ),
    processor!(
        "set-missing-names",
        "TProcSetMissingNames",
        "Set missing names",
        "NIF",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "check-for-errors",
        "TProcCheckForErrors",
        "Check for errors",
        "Report",
        TES4_TO_FO4,
        ["nif", "kf", "dds"],
        "partial"
    ),
    processor!(
        "analyze-mesh",
        "TProcAnalyzeMesh",
        "Analyze mesh",
        "Report",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "transform-information",
        "TProcTransformInfo",
        "Transform information",
        "Report",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "havok-information",
        "TProcHavokInfo",
        "Havok information",
        "Report",
        TES4_TO_SSE,
        ["nif"],
        "partial"
    ),
    processor!(
        "find-unwelded-vertices",
        "TProcUnweldedVertices",
        "Find unwelded vertices",
        "Report",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "find-excessive-draw-calls",
        "TProcFindDrawCalls",
        "Find excessive draw calls",
        "Report",
        FO3_FNV,
        ["nif"],
        "partial"
    ),
    processor!(
        "find-uvs",
        "TProcFindUVs",
        "Find UVs",
        "Report",
        TES3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "find-textures",
        "TProcFindTextures",
        "Find textures",
        "Report",
        TES3_TO_FO4,
        ["dds"],
        "partial"
    ),
    processor!(
        "copy-controlled-blocks",
        "TProcCopyControlledBlocks",
        "Copy anim controlled blocks",
        "Animation",
        TES4_TO_FNV,
        ["kf"],
        "partial"
    ),
    processor!(
        "copy-priorities",
        "TProcCopyPriorities",
        "Copy anim priorities",
        "Animation",
        TES4_TO_FNV,
        ["kf"],
        "partial"
    ),
    processor!(
        "remove-controlled-blocks",
        "TProcRemoveControlledBlocks",
        "Remove controlled blocks",
        "Animation",
        TES4_TO_FNV,
        ["nif", "kf"],
        "partial"
    ),
    processor!(
        "quadratic-to-linear",
        "TProcAnimQuadraticToLinear",
        "Quadratic to linear anim",
        "Animation",
        TES4_TO_FNV,
        ["kf"],
        "partial"
    ),
    processor!(
        "fix-exported-kf",
        "TProcFixExportedKFAnim",
        "Fix 3DS exported KF",
        "Animation",
        TES4_TO_FNV,
        ["kf"],
        "partial"
    ),
    processor!(
        "optimize-animations",
        "TProcOptimizeKF",
        "Optimize Animations",
        "Animation",
        TES4_TO_FO4,
        ["kf", "nif"],
        "partial"
    ),
    processor!(
        "add-headtracking-anim",
        "TProcAddHeadtrackingAnim",
        "Add headtracking anim",
        "Animation",
        FO3_FNV,
        ["kf"],
        "partial"
    ),
    processor!(
        "add-facial-anim",
        "TProcAddFacialAnim",
        "Add facial anim",
        "Animation",
        FO3_FNV,
        ["kf"],
        "partial"
    ),
    processor!(
        "add-transform-data",
        "TProcJamAnim",
        "Add NiTransformData",
        "Animation",
        TES4_TO_FNV,
        ["kf"],
        "partial"
    ),
    processor!(
        "weijiesen-blow-up",
        "TProcWeiExplosion",
        "Weijiesen's blow up thing",
        "Animation",
        FO3_FNV,
        ["nif"],
        "partial"
    ),
    processor!(
        "add-skeleton-blocks",
        "TProcAnimSkeletonDeath",
        "Add blocks from skeleton",
        "Animation",
        TES4_TO_FNV,
        ["kf"],
        "partial"
    ),
    processor!(
        "update-mopp-code",
        "TProcMoppUpdate",
        "Update MOPP code",
        "Collision",
        TES4_TO_FNV,
        ["nif"],
        "partial"
    ),
    processor!(
        "update-havok-settings",
        "TProcHavokSettingsUpdate",
        "Update Havok settings",
        "Collision",
        TES4_TO_SSE,
        ["nif"],
        "partial"
    ),
    processor!(
        "update-havok-inertia",
        "TProcInertiaUpdate",
        "Update Havok inertia",
        "Collision",
        TES4_TO_SSE,
        ["nif"],
        "partial"
    ),
    processor!(
        "update-ragdoll-constraint",
        "TProcRagdollConstraintUpdate",
        "Update ragdoll constraint",
        "Collision",
        &["fo3", "fnv", "skyrim", "skyrimse"],
        ["nif"],
        "partial"
    ),
    processor!(
        "search-havok-material",
        "TProcHavokSearchMaterial",
        "Search for Havok material",
        "Collision",
        TES4_TO_SSE,
        ["nif"],
        "partial"
    ),
    processor!(
        "update-shader-flags",
        "TProcShaderFlagsUpdate",
        "Update shader flags",
        "Shader",
        FO3_TO_FO4,
        ["nif"],
        "partial"
    ),
    processor!(
        "walls-reflection-flag",
        "TProcWallsReflectionFlag",
        "Real Time Reflections - NVSE",
        "Shader",
        FO3_FNV,
        ["nif"],
        "partial"
    ),
    processor!(
        "soft-particles",
        "TProcSoftParticles",
        "Vanilla Plus Particles - NVSE",
        "Shader",
        FO3_FNV,
        ["nif"],
        "partial"
    ),
];

pub fn processor_command(id: &str) -> &'static str {
    match id {
        "check-for-errors" => "validate",
        "analyze-mesh"
        | "transform-information"
        | "havok-information"
        | "find-unwelded-vertices"
        | "find-excessive-draw-calls"
        | "find-uvs"
        | "find-textures" => "report",
        "update-tangents"
        | "optimize-mesh"
        | "update-bounds"
        | "replace-assets"
        | "json-converter"
        | "universal-tweaker"
        | "universal-fixer"
        | "remove-unused-nodes"
        | "remove-nodes"
        | "attach-parent"
        | "adjust-transform"
        | "copy-geometry-blocks"
        | "merge-properties"
        | "group-shapes"
        | "vertex-paint"
        | "merge-shapes"
        | "apply-transform"
        | "add-root-collision-node"
        | "add-lod-node"
        | "add-bounding-box"
        | "convert-block-type"
        | "set-missing-names"
        | "unskin-mesh"
        | "copy-controlled-blocks"
        | "copy-priorities"
        | "remove-controlled-blocks"
        | "quadratic-to-linear"
        | "fix-exported-kf"
        | "optimize-animations"
        | "add-transform-data"
        | "add-headtracking-anim"
        | "add-facial-anim"
        | "weijiesen-blow-up"
        | "add-skeleton-blocks"
        | "update-mopp-code"
        | "update-havok-settings"
        | "update-havok-inertia"
        | "update-ragdoll-constraint"
        | "search-havok-material"
        | "update-shader-flags"
        | "walls-reflection-flag"
        | "soft-particles" => "process",
        _ => "",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NifValidationCheckDescriptor {
    pub id: &'static str,
    pub title: &'static str,
    pub group: &'static str,
    pub extensions: &'static [&'static str],
    pub optional: bool,
}

macro_rules! check {
    ($id:literal, $title:literal, $group:literal, [$($extension:literal),+], $optional:literal) => {
        NifValidationCheckDescriptor {
            id: $id,
            title: $title,
            group: $group,
            extensions: &[$($extension),+],
            optional: $optional,
        }
    };
}

pub const NIF_VALIDATION_CHECKS: &[NifValidationCheckDescriptor] = &[
    check!(
        "invalid-string-index",
        "Invalid string index",
        "Meshes",
        ["nif", "kf"],
        false
    ),
    check!(
        "invalid-block-order",
        "Invalid blocks order",
        "Meshes",
        ["nif", "kf"],
        false
    ),
    check!("unused-blocks", "Unused blocks", "Meshes", ["nif"], false),
    check!(
        "repeated-node-children-names",
        "Repeated NiNode childen names",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "wrong-link-types",
        "Wrong link types",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "invalid-array-links",
        "Invalid array links",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "invalid-geometry",
        "Invalid geometry",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "hardcoded-block-names",
        "Hardcoded block names",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "collision-havok",
        "Collision Havok issues",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "collision-mopp",
        "Collision MOPP issues",
        "Meshes",
        ["nif"],
        false
    ),
    check!("bsx-flags", "Check BSXFlags", "Meshes", ["nif"], false),
    check!(
        "consistency-flags",
        "Check consistency flags",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "texture-set-slots",
        "Check texture set slots",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "alpha-property",
        "Check NiAlphaProperty",
        "Meshes",
        ["nif"],
        true
    ),
    check!(
        "shader-types-flags",
        "Invalid shader types and flags",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "particle-systems",
        "Particle system checks",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "invalid-target",
        "Invalid Target field",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "animation-stop-time",
        "Animation stop time",
        "Meshes",
        ["nif", "kf"],
        false
    ),
    check!("skinning", "Skinning issues", "Meshes", ["nif"], false),
    check!(
        "miscellaneous",
        "Miscellaneous checks",
        "Meshes",
        ["nif", "kf"],
        false
    ),
    check!(
        "vertex-colors",
        "Check vertex colors",
        "Meshes",
        ["nif"],
        false
    ),
    check!(
        "clamped-tiling-uvs",
        "Clamped tiling UVs",
        "Meshes",
        ["nif"],
        true
    ),
    check!(
        "optional-shader-checks",
        "Optional checks",
        "Meshes",
        ["nif"],
        true
    ),
    check!(
        "repeated-degenerate-strips",
        "Repeated denegerate tris in strips",
        "Meshes",
        ["nif"],
        true
    ),
    check!(
        "invalid-texture-size-format",
        "Invalid texture size or format",
        "Textures",
        ["dds"],
        false
    ),
    check!(
        "sse-unsupported-nif",
        "Unsupported mesh formats",
        "Skyrim SE",
        ["nif"],
        true
    ),
    check!(
        "sse-unsupported-dds",
        "Unsupported texture formats",
        "Skyrim SE",
        ["dds"],
        true
    ),
];

pub fn check_id_for_finding_rule(rule: &str) -> &'static str {
    match rule {
        "invalid-string-index" => "invalid-string-index",
        "block-order" => "invalid-block-order",
        "missing-root" | "multiple-roots" | "unused-block" => "unused-blocks",
        "duplicate-name" => "repeated-node-children-names",
        "wrong-link-type" | "broken-link" => "wrong-link-types",
        "invalid-array-link" | "repeated-array-link" => "invalid-array-links",
        "duplicate-vertices"
        | "triangle-index"
        | "strip-index"
        | "unused-vertices"
        | "multiple-triangle-strips" => "invalid-geometry",
        "hardcoded-name"
        | "hardcoded-weapon-name"
        | "addon-node-name"
        | "oblivion-material-name" => "hardcoded-block-names",
        rule if rule.starts_with("collision-")
            || matches!(
                rule,
                "dynamic-mopp" | "static-body-flags" | "constraint-cone-angle" | "collision-body"
            ) =>
        {
            if rule == "collision-mopp-complexity" {
                "collision-mopp"
            } else {
                "collision-havok"
            }
        }
        "bsx-flags" => "bsx-flags",
        "consistency-flags" => "consistency-flags",
        "shader-texture-slot" | "asset-path" => "texture-set-slots",
        "alpha-property-single-pass" => "alpha-property",
        "shader-type-flags"
        | "shader-controller-type"
        | "skinned-shader-flag"
        | "missing-shader-property"
        | "missing-tangent-space"
        | "sse-facegen-shape" => "shader-types-flags",
        rule if rule.starts_with("particle-") || rule == "orphan-particle-emitter" => {
            "particle-systems"
        }
        "controller-target"
        | "controller-type"
        | "animation-accum-root"
        | "animation-controlled-block-order"
        | "animation-object-palette"
        | "animation-extra-targets"
        | "animation-target"
        | "hidden-animation-target"
        | "animation-property"
        | "collision-target"
        | "collision-target-name" => "invalid-target",
        "animation-stop-time" => "animation-stop-time",
        rule if rule.starts_with("skin-")
            || matches!(rule, "missing-skin-instance" | "morph-model-parity") =>
        {
            "skinning"
        }
        "all-white-vertex-colors"
        | "hdr-vertex-colors"
        | "vertex-alpha-flag"
        | "vertex-alpha-shader-flag"
        | "vertex-color-shader-flag" => "vertex-colors",
        "clamped-tiling-uvs" => "clamped-tiling-uvs",
        "optional-shader-flags" => "optional-shader-checks",
        "repeated-degenerate-strips" => "repeated-degenerate-strips",
        "sse-unsupported-block" => "sse-unsupported-nif",
        _ => "miscellaneous",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_matches_registered_nif_surface() {
        assert_eq!(NIF_PROCESSORS.len(), 50);
        assert_eq!(
            NIF_PROCESSORS
                .iter()
                .filter(|processor| processor.category == "NIF")
                .count(),
            23
        );
        assert_eq!(NIF_VALIDATION_CHECKS.len(), 27);
        assert_eq!(
            NIF_PROCESSORS
                .iter()
                .filter(|processor| !processor_command(processor.id).is_empty())
                .count(),
            50
        );
    }
}
