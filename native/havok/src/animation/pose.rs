/// Pose container — local-space / model-space cache with lazy propagation.
///
/// Mirrors `hkaPose` semantics: bones are stored in local space; model-space
/// transforms are computed lazily on first access and invalidated when any
/// ancestor local transform changes.
///
/// `QsTransform` is (translation: [f32;3], rotation: [f32;4] quaternion xyzw, scale: [f32;3]).

/// A bone transform in either local or model space.
#[derive(Debug, Clone, PartialEq)]
pub struct QsTransform {
    pub translation: [f32; 3],
    /// Quaternion as (x, y, z, w).
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl QsTransform {
    pub const IDENTITY: QsTransform = QsTransform {
        translation: [0.0, 0.0, 0.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    };

    /// Compose parent * child in model space.
    pub fn compose(parent: &QsTransform, child: &QsTransform) -> QsTransform {
        let pt = parent.translation;
        let pr = parent.rotation;
        let ps = parent.scale;
        let ct = child.translation;
        let cr = child.rotation;
        let cs = child.scale;

        // Rotate + scale child translation by parent.
        let ct_scaled = [ct[0] * ps[0], ct[1] * ps[1], ct[2] * ps[2]];
        let ct_rot = quat_rotate(&pr, &ct_scaled);

        QsTransform {
            translation: [pt[0] + ct_rot[0], pt[1] + ct_rot[1], pt[2] + ct_rot[2]],
            rotation: quat_mul(&pr, &cr),
            scale: [ps[0] * cs[0], ps[1] * cs[1], ps[2] * cs[2]],
        }
    }
}

/// Rotate vector v by unit quaternion q = (x,y,z,w).
pub fn quat_rotate(q: &[f32; 4], v: &[f32; 3]) -> [f32; 3] {
    let [qx, qy, qz, qw] = *q;
    let [vx, vy, vz] = *v;
    // t = 2 * cross(q.xyz, v)
    let tx = 2.0 * (qy * vz - qz * vy);
    let ty = 2.0 * (qz * vx - qx * vz);
    let tz = 2.0 * (qx * vy - qy * vx);
    [
        vx + qw * tx + (qy * tz - qz * ty),
        vy + qw * ty + (qz * tx - qx * tz),
        vz + qw * tz + (qx * ty - qy * tx),
    ]
}

/// Hamilton product of two quaternions (x,y,z,w).
pub fn quat_mul(a: &[f32; 4], b: &[f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = *a;
    let [bx, by, bz, bw] = *b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// Normalize a quaternion. Returns identity if near-zero.
pub fn quat_normalize(q: &[f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = *q;
    let len = (x * x + y * y + z * z + w * w).sqrt();
    if len < 1e-10 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [x / len, y / len, z / len, w / len]
}

/// Inverse (conjugate for unit quaternion).
pub fn quat_conjugate(q: &[f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// Axis-angle to quaternion.
pub fn quat_from_axis_angle(axis: &[f32; 3], angle_rad: f32) -> [f32; 4] {
    let half = angle_rad * 0.5;
    let s = half.sin();
    let [ax, ay, az] = *axis;
    let len = (ax * ax + ay * ay + az * az).sqrt();
    if len < 1e-10 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [ax / len * s, ay / len * s, az / len * s, half.cos()]
}

/// Cross product.
pub fn vec3_cross(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Dot product.
pub fn vec3_dot(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Length.
pub fn vec3_len(v: &[f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Normalize; returns zero vector if degenerate.
pub fn vec3_normalize(v: &[f32; 3]) -> [f32; 3] {
    let len = vec3_len(v);
    if len < 1e-10 {
        return [0.0, 0.0, 0.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

pub fn vec3_sub(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn vec3_add(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn vec3_scale(v: &[f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

// ---------------------------------------------------------------------------
// Skeleton descriptor (minimal — mirrors hkaSkeleton)
// ---------------------------------------------------------------------------

/// Minimal skeleton descriptor used by the pose container.
#[derive(Debug, Clone)]
pub struct PoseSkeleton {
    pub bone_names: Vec<String>,
    /// Parent index per bone; root bones have index -1.
    pub parent_indices: Vec<i32>,
    /// Reference pose in local space.
    pub reference_local: Vec<QsTransform>,
}

impl PoseSkeleton {
    pub fn bone_count(&self) -> usize {
        self.bone_names.len()
    }
}

// ---------------------------------------------------------------------------
// Pose container
// ---------------------------------------------------------------------------

/// Local-space / model-space pose container with lazy model-space computation.
///
/// Model-space transforms are invalidated when any local transform is updated
/// and recomputed lazily on first access.
#[derive(Debug, Clone)]
pub struct Pose {
    skeleton: PoseSkeleton,
    local: Vec<QsTransform>,
    model_cache: Vec<Option<QsTransform>>,
}

impl Pose {
    /// Create a pose initialized to the skeleton's reference local transforms.
    pub fn from_reference(skeleton: PoseSkeleton) -> Self {
        let n = skeleton.bone_count();
        let local = skeleton.reference_local.clone();
        Pose {
            skeleton,
            local,
            model_cache: vec![None; n],
        }
    }

    /// Create a pose from explicit local transforms.
    pub fn from_local(skeleton: PoseSkeleton, local: Vec<QsTransform>) -> Self {
        let n = skeleton.bone_count();
        assert_eq!(local.len(), n, "local transforms must match bone count");
        Pose {
            skeleton,
            local,
            model_cache: vec![None; n],
        }
    }

    pub fn bone_count(&self) -> usize {
        self.skeleton.bone_count()
    }

    pub fn skeleton(&self) -> &PoseSkeleton {
        &self.skeleton
    }

    /// Read local-space transform for bone `idx`.
    pub fn local_at(&self, idx: usize) -> &QsTransform {
        &self.local[idx]
    }

    /// Write local-space transform for bone `idx`. Invalidates model cache for
    /// this bone and all its descendants.
    pub fn set_local(&mut self, idx: usize, t: QsTransform) {
        self.local[idx] = t;
        self.invalidate_subtree(idx);
    }

    /// Return model-space transform for bone `idx`, computing lazily if needed.
    pub fn model_at(&mut self, idx: usize) -> QsTransform {
        if self.model_cache[idx].is_some() {
            return self.model_cache[idx].clone().unwrap();
        }
        let parent_idx = self.skeleton.parent_indices[idx];
        let model = if parent_idx < 0 {
            self.local[idx].clone()
        } else {
            let parent = self.model_at(parent_idx as usize);
            QsTransform::compose(&parent, &self.local[idx])
        };
        self.model_cache[idx] = Some(model.clone());
        model
    }

    /// Return all model-space transforms (computes any uncached entries).
    pub fn all_model(&mut self) -> Vec<QsTransform> {
        (0..self.bone_count()).map(|i| self.model_at(i)).collect()
    }

    fn invalidate_subtree(&mut self, root: usize) {
        self.model_cache[root] = None;
        let n = self.skeleton.bone_count();
        for i in (root + 1)..n {
            if self.skeleton.parent_indices[i] >= 0 && self.model_cache[i].is_none() {
                // Already invalidated
                continue;
            }
            if self.is_descendant_of(i, root) {
                self.model_cache[i] = None;
            }
        }
    }

    fn is_descendant_of(&self, bone: usize, ancestor: usize) -> bool {
        let mut cur = bone as i32;
        loop {
            cur = self.skeleton.parent_indices[cur as usize];
            if cur < 0 {
                return false;
            }
            if cur as usize == ancestor {
                return true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_bone_skeleton() -> PoseSkeleton {
        PoseSkeleton {
            bone_names: vec!["Root".into(), "Child".into()],
            parent_indices: vec![-1, 0],
            reference_local: vec![
                QsTransform::IDENTITY,
                QsTransform {
                    translation: [1.0, 0.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
            ],
        }
    }

    #[test]
    fn pose_from_reference_model_space() {
        let skel = two_bone_skeleton();
        let mut pose = Pose::from_reference(skel);
        let root_model = pose.model_at(0);
        assert_eq!(root_model.translation, [0.0, 0.0, 0.0]);
        let child_model = pose.model_at(1);
        assert!((child_model.translation[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn set_local_invalidates_descendant_cache() {
        let skel = two_bone_skeleton();
        let mut pose = Pose::from_reference(skel);
        // Prime cache
        let _ = pose.model_at(1);
        assert!(pose.model_cache[1].is_some());
        // Mutate root
        pose.set_local(
            0,
            QsTransform {
                translation: [5.0, 0.0, 0.0],
                ..QsTransform::IDENTITY
            },
        );
        assert!(pose.model_cache[0].is_none());
        assert!(pose.model_cache[1].is_none());
        // Recompute
        let child = pose.model_at(1);
        assert!((child.translation[0] - 6.0).abs() < 1e-5);
    }
}
