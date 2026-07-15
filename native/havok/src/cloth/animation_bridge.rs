// Animation bridge.
//
// Glue layer between an `hkaSkeleton`-shaped pose and an `hclTransformSet`.
// Mirrors the SDK pattern from
// `Cloth/AnimationBridge/Setup/TransformSet/hclSkeletonTransformSetSetupObject.h`
// + `Cloth/Cloth/TransformSet/hclTransformSet.h`:
//
// 1. `SkeletonTransformSetSetup` declares the bind-time mapping between an
//    authoring skeleton and a transform set definition (which bones, in what
//    order, named by what string).
// 2. `AnimationBridge::fill_transform_set` is the per-frame call: given a
//    `SkeletonPose` (per-bone local transforms) + the skeleton's parent
//    indices, it computes model-space matrices and writes them into a
//    `TransformSetBuffer` (matching SDK `hclTransformSet::m_transforms`).
// 3. `update_inverse_transposes_orthonormal` matches SDK
//    `hclTransformSet::updateInverseTransposesOrthonormal()` for normals/planes.

use crate::cloth::setup::buffer_setup::TransformSetSetupObject;

/// Row-major 4x4 transform matrix.
pub type Mat4 = [[f32; 4]; 4];

pub fn identity_mat4() -> Mat4 {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Authoring-side skeleton — minimal subset of `hkaSkeleton` needed for the
/// transform-set bridge. Bone i has parent_indices[i] as its parent (-1 for
/// root). `rest_local` is the bind-pose local transform per bone.
#[derive(Debug, Clone, PartialEq)]
pub struct Skeleton {
    pub name: String,
    pub bone_names: Vec<String>,
    pub parent_indices: Vec<i16>,
    pub rest_local: Vec<Mat4>,
}

impl Skeleton {
    pub fn num_bones(&self) -> usize {
        self.bone_names.len()
    }

    pub fn find_bone(&self, name: &str) -> Option<usize> {
        self.bone_names.iter().position(|n| n == name)
    }
}

/// A per-frame skeleton pose — local transforms for each bone, parallel to
/// `Skeleton::bone_names`.
#[derive(Debug, Clone, PartialEq)]
pub struct SkeletonPose {
    pub local_transforms: Vec<Mat4>,
}

impl SkeletonPose {
    pub fn from_skeleton_rest(skel: &Skeleton) -> Self {
        Self {
            local_transforms: skel.rest_local.clone(),
        }
    }

    pub fn num_bones(&self) -> usize {
        self.local_transforms.len()
    }
}

/// Authoring-time setup for a skeleton-driven transform set. Mirrors SDK
/// `hclSkeletonTransformSetSetupObject` (name, world_from_model, the bound
/// skeleton). Produces a `TransformSetSetupObject` for the cloth setup graph.
#[derive(Debug, Clone, PartialEq)]
pub struct SkeletonTransformSetSetup {
    pub name: String,
    pub world_from_model: Mat4,
    /// Optional subset of skeleton bones to expose. Empty = all bones.
    pub selected_bones: Vec<String>,
}

impl Default for SkeletonTransformSetSetup {
    fn default() -> Self {
        Self {
            name: String::new(),
            world_from_model: identity_mat4(),
            selected_bones: Vec::new(),
        }
    }
}

impl SkeletonTransformSetSetup {
    /// Build the authoring `TransformSetSetupObject` for this skeleton bind.
    /// Mirrors SDK `_createTransformSetDefinition` shape.
    pub fn create_transform_set_setup(&self, skel: &Skeleton) -> TransformSetSetupObject {
        let bone_names = if self.selected_bones.is_empty() {
            skel.bone_names.clone()
        } else {
            self.selected_bones.clone()
        };
        TransformSetSetupObject {
            name: self.name.clone(),
            bone_names,
            skeleton_name: skel.name.clone(),
        }
    }

    /// Returns the parent index map for the (possibly subset) transform set.
    /// For a subset, parent maps point at the immediate ancestor that is
    /// also in the subset, or -1 if none.
    pub fn transform_parent_indices(&self, skel: &Skeleton) -> Vec<i16> {
        if self.selected_bones.is_empty() {
            return skel.parent_indices.clone();
        }
        let subset_lookup: std::collections::HashMap<&str, usize> = self
            .selected_bones
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();

        let mut out = vec![-1i16; self.selected_bones.len()];
        for (out_idx, sel_name) in self.selected_bones.iter().enumerate() {
            let Some(skel_idx) = skel.find_bone(sel_name) else {
                continue;
            };
            // Walk skeleton parents until we find one that is also selected.
            let mut p = skel.parent_indices[skel_idx];
            while p >= 0 {
                let parent_name = &skel.bone_names[p as usize];
                if let Some(&pi) = subset_lookup.get(parent_name.as_str()) {
                    out[out_idx] = pi as i16;
                    break;
                }
                p = skel.parent_indices[p as usize];
            }
        }
        out
    }
}

/// Runtime mirror of SDK `hclTransformSet` — the buffer the bridge fills
/// per-frame. `transforms` is model-space, `inverse_transposes` is kept in
/// sync via `update_inverse_transposes_orthonormal`.
#[derive(Debug, Clone, PartialEq)]
pub struct TransformSetBuffer {
    pub transforms: Vec<Mat4>,
    pub inverse_transposes: Vec<Mat4>,
}

impl TransformSetBuffer {
    pub fn with_capacity(n: usize) -> Self {
        Self {
            transforms: vec![identity_mat4(); n],
            inverse_transposes: vec![identity_mat4(); n],
        }
    }

    /// Mirrors SDK `updateInverseTransposesOrthonormal`. For an orthonormal
    /// rotation+translation matrix, the inverse-transpose has the same
    /// rotation block and zero translation in the upper 3x3 sense.
    pub fn update_inverse_transposes_orthonormal(&mut self) {
        for (m, it) in self
            .transforms
            .iter()
            .zip(self.inverse_transposes.iter_mut())
        {
            *it = orthonormal_inverse_transpose(*m);
        }
    }
}

/// Bridge that ties together a skeleton, a setup, and a runtime buffer.
#[derive(Debug, Clone)]
pub struct AnimationBridge<'a> {
    pub skeleton: &'a Skeleton,
    pub setup: &'a SkeletonTransformSetSetup,
}

impl<'a> AnimationBridge<'a> {
    pub fn new(skeleton: &'a Skeleton, setup: &'a SkeletonTransformSetSetup) -> Self {
        Self { skeleton, setup }
    }

    /// Per-frame entry point — fill the transform-set buffer with model-space
    /// matrices for the given pose. The output buffer is sized for the
    /// (possibly subset) transform set.
    pub fn fill_transform_set(&self, pose: &SkeletonPose, out: &mut TransformSetBuffer) {
        assert_eq!(
            pose.num_bones(),
            self.skeleton.num_bones(),
            "pose bone count must match skeleton",
        );

        // Compute model-space matrices for ALL skeleton bones first.
        let model = compute_model_space(self.skeleton, pose);

        // Then project into the (subset) transform set, applying world_from_model.
        let bone_list: Vec<&str> = if self.setup.selected_bones.is_empty() {
            self.skeleton
                .bone_names
                .iter()
                .map(|s| s.as_str())
                .collect()
        } else {
            self.setup
                .selected_bones
                .iter()
                .map(|s| s.as_str())
                .collect()
        };

        if out.transforms.len() != bone_list.len() {
            *out = TransformSetBuffer::with_capacity(bone_list.len());
        }

        for (i, name) in bone_list.iter().enumerate() {
            let Some(skel_i) = self.skeleton.find_bone(name) else {
                out.transforms[i] = identity_mat4();
                continue;
            };
            out.transforms[i] = matmul4(self.setup.world_from_model, model[skel_i]);
        }

        out.update_inverse_transposes_orthonormal();
    }
}

// ---------------------------------------------------------------------------
// Forward-kinematics helpers
// ---------------------------------------------------------------------------

/// Compute model-space matrices for every bone in the skeleton given a pose.
fn compute_model_space(skel: &Skeleton, pose: &SkeletonPose) -> Vec<Mat4> {
    let n = skel.num_bones();
    let mut out = vec![identity_mat4(); n];
    for i in 0..n {
        let local = pose.local_transforms[i];
        let parent = skel.parent_indices[i];
        out[i] = if parent < 0 {
            local
        } else {
            // SDK convention: model = parent_model * local (column-major LHS),
            // which in row-major land is also `parent_model * local`.
            matmul4(out[parent as usize], local)
        };
    }
    out
}

fn matmul4(a: Mat4, b: Mat4) -> Mat4 {
    let mut out = [[0.0f32; 4]; 4];
    for r in 0..4 {
        for c in 0..4 {
            out[r][c] =
                a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c] + a[r][3] * b[3][c];
        }
    }
    out
}

fn orthonormal_inverse_transpose(m: Mat4) -> Mat4 {
    // For a rigid transform R|t, inverse is R^T | -R^T t. Then transpose.
    // Result has the same rotation block in (3x3); we set translation row to 0.
    let r = [
        [m[0][0], m[0][1], m[0][2]],
        [m[1][0], m[1][1], m[1][2]],
        [m[2][0], m[2][1], m[2][2]],
    ];
    let r_t = [
        [r[0][0], r[1][0], r[2][0]],
        [r[0][1], r[1][1], r[2][1]],
        [r[0][2], r[1][2], r[2][2]],
    ];
    // (R | t)^-1 = (R^T | -R^T t). Then transpose → upper-left 3x3 stays
    // the same as R^T, since (R^T)^T = R. Following SDK: the inverse
    // transpose for normals/planes uses the inverse-of-rotation-then-
    // transpose, which simplifies to R for orthonormal input. The
    // translation portion is irrelevant for normals; set bottom row = (0,0,0,1).
    let _ = r_t;
    [
        [r[0][0], r[0][1], r[0][2], 0.0],
        [r[1][0], r[1][1], r[1][2], 0.0],
        [r[2][0], r[2][1], r[2][2], 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn translation(tx: f32, ty: f32, tz: f32) -> Mat4 {
        let mut m = identity_mat4();
        m[0][3] = tx;
        m[1][3] = ty;
        m[2][3] = tz;
        m
    }

    fn rotation_z(angle_rad: f32) -> Mat4 {
        let c = angle_rad.cos();
        let s = angle_rad.sin();
        [
            [c, -s, 0.0, 0.0],
            [s, c, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    fn three_bone_skeleton() -> Skeleton {
        // Root → Spine → Hand. Each bone has a +y translation in local.
        Skeleton {
            name: "TestSkel".into(),
            bone_names: vec!["Root".into(), "Spine".into(), "Hand".into()],
            parent_indices: vec![-1, 0, 1],
            rest_local: vec![
                identity_mat4(),
                translation(0.0, 1.0, 0.0),
                translation(0.0, 1.0, 0.0),
            ],
        }
    }

    #[test]
    fn rest_pose_produces_chain_translations() {
        let skel = three_bone_skeleton();
        let pose = SkeletonPose::from_skeleton_rest(&skel);
        let setup = SkeletonTransformSetSetup {
            name: "TS".into(),
            world_from_model: identity_mat4(),
            selected_bones: vec![],
        };
        let bridge = AnimationBridge::new(&skel, &setup);
        let mut buf = TransformSetBuffer::with_capacity(0);
        bridge.fill_transform_set(&pose, &mut buf);

        assert_eq!(buf.transforms.len(), 3);
        // Root at origin.
        assert!((buf.transforms[0][0][3] - 0.0).abs() < 1e-6);
        assert!((buf.transforms[0][1][3] - 0.0).abs() < 1e-6);
        // Spine at +y=1.
        assert!((buf.transforms[1][1][3] - 1.0).abs() < 1e-6);
        // Hand at +y=2 (chain).
        assert!((buf.transforms[2][1][3] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn rotated_root_propagates_to_descendants() {
        let skel = three_bone_skeleton();
        let mut pose = SkeletonPose::from_skeleton_rest(&skel);
        // Rotate root by 90° around Z.
        pose.local_transforms[0] = rotation_z(std::f32::consts::FRAC_PI_2);

        let setup = SkeletonTransformSetSetup::default();
        let bridge = AnimationBridge::new(&skel, &setup);
        let mut buf = TransformSetBuffer::with_capacity(3);
        bridge.fill_transform_set(&pose, &mut buf);

        // After 90° Z rotation, Spine local +y=1 becomes model-space -x=1,
        // and Hand should be at -x=2.
        assert!(
            (buf.transforms[1][0][3] - -1.0).abs() < 1e-5,
            "Spine x = {}",
            buf.transforms[1][0][3]
        );
        assert!(
            (buf.transforms[2][0][3] - -2.0).abs() < 1e-5,
            "Hand x = {}",
            buf.transforms[2][0][3]
        );
    }

    #[test]
    fn world_from_model_translates_output() {
        let skel = three_bone_skeleton();
        let pose = SkeletonPose::from_skeleton_rest(&skel);
        let setup = SkeletonTransformSetSetup {
            name: "TS".into(),
            world_from_model: translation(10.0, 0.0, 0.0),
            selected_bones: vec![],
        };
        let bridge = AnimationBridge::new(&skel, &setup);
        let mut buf = TransformSetBuffer::with_capacity(3);
        bridge.fill_transform_set(&pose, &mut buf);

        // Every output bone gets +10 on x.
        for t in &buf.transforms {
            assert!((t[0][3] - 10.0).abs() < 1e-6);
        }
    }

    #[test]
    fn subset_transform_set_only_emits_selected_bones() {
        let skel = three_bone_skeleton();
        let pose = SkeletonPose::from_skeleton_rest(&skel);
        let setup = SkeletonTransformSetSetup {
            name: "TS".into(),
            world_from_model: identity_mat4(),
            selected_bones: vec!["Root".into(), "Hand".into()],
        };
        let bridge = AnimationBridge::new(&skel, &setup);
        let mut buf = TransformSetBuffer::with_capacity(0);
        bridge.fill_transform_set(&pose, &mut buf);

        assert_eq!(buf.transforms.len(), 2);
        // Hand still at y=2 (model-space), even though Spine is unbound.
        assert!((buf.transforms[1][1][3] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn subset_parent_indices_skip_unselected() {
        let skel = three_bone_skeleton();
        let setup = SkeletonTransformSetSetup {
            name: "TS".into(),
            selected_bones: vec!["Root".into(), "Hand".into()],
            ..Default::default()
        };
        let parents = setup.transform_parent_indices(&skel);
        assert_eq!(parents.len(), 2);
        // Root has no parent.
        assert_eq!(parents[0], -1);
        // Hand's nearest selected ancestor is Root (skipping unselected Spine).
        assert_eq!(parents[1], 0);
    }

    #[test]
    fn create_transform_set_setup_propagates_skeleton_name() {
        let skel = three_bone_skeleton();
        let setup = SkeletonTransformSetSetup {
            name: "MyTS".into(),
            ..Default::default()
        };
        let ts = setup.create_transform_set_setup(&skel);
        assert_eq!(ts.name, "MyTS");
        assert_eq!(ts.skeleton_name, "TestSkel");
        assert_eq!(ts.bone_names, skel.bone_names);
    }

    #[test]
    fn inverse_transposes_match_rotation_block() {
        let skel = three_bone_skeleton();
        let mut pose = SkeletonPose::from_skeleton_rest(&skel);
        pose.local_transforms[0] = rotation_z(std::f32::consts::FRAC_PI_4);
        let setup = SkeletonTransformSetSetup::default();
        let bridge = AnimationBridge::new(&skel, &setup);
        let mut buf = TransformSetBuffer::with_capacity(3);
        bridge.fill_transform_set(&pose, &mut buf);

        // For orthonormal inputs the inverse-transpose's 3x3 equals the
        // model-space 3x3 (because R^-T = R for orthonormal R).
        for i in 0..3 {
            for r in 0..3 {
                for c in 0..3 {
                    assert!(
                        (buf.transforms[i][r][c] - buf.inverse_transposes[i][r][c]).abs() < 1e-5,
                        "mismatch at bone {i} ({r},{c})",
                    );
                }
            }
        }
    }
}
