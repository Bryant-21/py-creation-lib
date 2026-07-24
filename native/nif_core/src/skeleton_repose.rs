//! Repose a creature `skeleton.nif` into the FO4 inverse-bind rest pose.
//!
//! For an undeformed skin, every skinned bone must satisfy
//! `bone_world @ bone_bind == IDENTITY`, where `bone_bind` is the bind
//! (skin→bone) matrix stored in the body mesh's `BSSkin::BoneData` and
//! `bone_world` is the bone's accumulated world transform in the skeleton.
//! A converted FO76 skeleton keeps the raw FO76 bone locals, so this
//! invariant is broken and the rest pose renders as a deformed blob.
//!
//! This module reads the bind matrices from a body mesh (matched by bone
//! name), then walks the skeleton parent→child and sets each *skinned*
//! bone's `world = inverse(bind)`, deriving `local = inverse(parent_world)
//! @ world`. The repose is GATED per-bone: bones already satisfying the
//! invariant (and all non-skinned helper nodes) are left untouched, so an
//! already-correct skeleton round-trips byte-identically (no write).
//!
//! Matrix convention is verified against a known-good fan skeleton: the NIF
//! rotation struct `m11..m33` maps to the math matrix `M[r][c]` as
//! `M[0][0]=m11, M[0][1]=m21, M[0][2]=m31, M[1][0]=m12, ...` (the same
//! mapping the working skinned renderer uses), and world transforms compose
//! as `world = parent_world @ local` with column vectors.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::model::{NifBlock, NifFile, NifValue};

/// A 4×4 affine transform, row-major (`m[row][col]`), bottom row `[0,0,0,1]`.
pub type BindMatrix = [[f64; 4]; 4];

#[derive(Debug, Clone, Copy, Default)]
pub struct ReposeReport {
    /// Skeleton bones whose name matched a body-mesh bind (repose candidates).
    pub skinned_bones: usize,
    /// Bones whose local was rewritten to satisfy `world @ bind == I`.
    pub reposed: usize,
    /// Bones already satisfying the invariant (left untouched by the gate).
    pub already_consistent: usize,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract per-bone bind matrices from a skinned body mesh's `BSSkin` chain,
/// keyed by bone name. Each value is the bind (skin→bone) matrix from
/// `BSSkin::BoneData.Bone List[i]` for the bone referenced by
/// `BSSkin::Instance.Bones[i]`. Scale is ignored (rotation + translation
/// only), matching the renderer's bind-pose convention.
pub fn collect_bind_matrices_by_name(nif: &NifFile) -> HashMap<String, BindMatrix> {
    let mut out: HashMap<String, BindMatrix> = HashMap::new();
    for block in &nif.blocks {
        if block.type_name != "BSSkin::Instance" {
            continue;
        }
        let Some(NifValue::Array(bone_refs)) = block.get_field("Bones") else {
            continue;
        };
        let Some(data_id) = ref_value(block.get_field("Data")) else {
            continue;
        };
        let Some(data) = nif.get_block(data_id) else {
            continue;
        };
        let Some(NifValue::Array(bone_list)) = data.get_field("Bone List") else {
            continue;
        };
        for (i, bone_ref) in bone_refs.iter().enumerate() {
            let Some(bone_id) = ref_value(Some(bone_ref)) else {
                continue;
            };
            let Some(name) = nif.get_block(bone_id).and_then(node_name) else {
                continue;
            };
            let Some(NifValue::Struct(entry)) = bone_list.get(i) else {
                continue;
            };
            let rot = sget(entry, "Rotation")
                .map(read_rotation)
                .unwrap_or_else(identity3);
            let trans = sget(entry, "Translation")
                .map(read_translation)
                .unwrap_or([0.0; 3]);
            out.entry(name)
                .or_insert_with(|| rot_trans_to_mat4(rot, trans));
        }
    }
    out
}

/// Count `(consistent, skinned_total)` skinned bones — those whose current
/// accumulated `world @ bind` is within `tol` of the identity. Read-only;
/// shared by the gate and by tests/diagnostics.
pub fn skeleton_bind_consistency(
    skel: &NifFile,
    bind_by_name: &HashMap<String, BindMatrix>,
    tol: f64,
) -> (usize, usize) {
    let (order, parents) = node_order_and_parents(skel);
    let mut world: HashMap<usize, BindMatrix> = HashMap::new();
    let mut consistent = 0usize;
    let mut total = 0usize;
    for &bid in &order {
        let block = &skel.blocks[bid];
        let local = local_of(block);
        let parent_world = parents
            .get(&bid)
            .and_then(|p| world.get(p))
            .copied()
            .unwrap_or_else(identity4);
        let current_world = mat4_mul(&parent_world, &local);
        world.insert(bid, current_world);
        if let Some(bind) = node_name(block).and_then(|n| bind_by_name.get(&n)) {
            total += 1;
            if max_identity_residual(&mat4_mul(&current_world, bind)) <= tol {
                consistent += 1;
            }
        }
    }
    (consistent, total)
}

/// Max per-component spread between bone residuals for the field to count as
/// one common rigid offset (scorched source spreads ~2.9; genuinely broken
/// skeletons spread 100–240).
const UNIFORM_RESIDUAL_DEV_TOL: f64 = 5.0;
/// Minimum magnitude of that common offset before it is treated as a binding
/// convention rather than float noise worth reposing away.
const CONVENTION_MIN_OFFSET: f64 = 10.0;

/// Detect a binding-convention offset: every skinned bone's `world @ bind`
/// equals the same large rigid offset. FO4's own humanoid assets have this
/// (vanilla human skeleton vs malebody/femalebody: uniform ~120.84 world
/// offset on every bone) and the engine resolves it at skinning time, so it
/// is not a broken rest pose. Reposing such a skeleton shifts every bone the
/// engine drives through the humanoid rig by that offset and smears the
/// actor (FO76 Scorched regression). A uniform offset never deforms a skin;
/// only per-bone disagreement does.
fn uniform_convention_offset(residuals: &[BindMatrix]) -> bool {
    let Some(first) = residuals.first() else {
        return false;
    };
    let mut median = *first;
    for i in 0..3 {
        for j in 0..4 {
            let mut vals: Vec<f64> = residuals.iter().map(|r| r[i][j]).collect();
            vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            median[i][j] = vals[vals.len() / 2];
        }
    }
    let spread = residuals
        .iter()
        .map(|r| {
            let mut d = 0.0_f64;
            for i in 0..3 {
                for j in 0..4 {
                    d = d.max((r[i][j] - median[i][j]).abs());
                }
            }
            d
        })
        .fold(0.0_f64, f64::max);
    spread <= UNIFORM_RESIDUAL_DEV_TOL && max_identity_residual(&median) >= CONVENTION_MIN_OFFSET
}

fn collect_residuals(
    skel: &NifFile,
    bind_by_name: &HashMap<String, BindMatrix>,
) -> Vec<BindMatrix> {
    let (order, parents) = node_order_and_parents(skel);
    let mut world: HashMap<usize, BindMatrix> = HashMap::new();
    let mut residuals = Vec::new();
    for &bid in &order {
        let block = &skel.blocks[bid];
        let parent_world = parents
            .get(&bid)
            .and_then(|p| world.get(p))
            .copied()
            .unwrap_or_else(identity4);
        let current_world = mat4_mul(&parent_world, &local_of(block));
        world.insert(bid, current_world);
        if let Some(bind) = node_name(block).and_then(|n| bind_by_name.get(&n)) {
            residuals.push(mat4_mul(&current_world, bind));
        }
    }
    residuals
}

/// Repose `skel` in place so every skinned bone satisfies
/// `world @ bind == I`. Walks parent→child; for each bone whose name matches
/// a body-mesh bind, sets `world = inverse(bind)` and rewrites its local
/// (`inverse(parent_world) @ world`). Bones already within `tol` of the
/// invariant, and all non-skinned nodes, are left untouched. Returns counts.
///
/// Skeletons whose residual field is one common large rigid offset (the FO4
/// humanoid binding convention — see [`uniform_convention_offset`]) are left
/// untouched entirely.
pub fn repose_skeleton_to_inverse_bind(
    skel: &mut NifFile,
    bind_by_name: &HashMap<String, BindMatrix>,
    tol: f64,
) -> ReposeReport {
    let residuals = collect_residuals(skel, bind_by_name);
    if uniform_convention_offset(&residuals) {
        return ReposeReport {
            skinned_bones: residuals.len(),
            reposed: 0,
            already_consistent: residuals.len(),
        };
    }

    let (order, parents) = node_order_and_parents(skel);
    let mut world: HashMap<usize, BindMatrix> = HashMap::new();
    let mut report = ReposeReport::default();

    for &bid in &order {
        let (name, local) = {
            let block = &skel.blocks[bid];
            (node_name(block), local_of(block))
        };
        let parent_world = parents
            .get(&bid)
            .and_then(|p| world.get(p))
            .copied()
            .unwrap_or_else(identity4);
        let current_world = mat4_mul(&parent_world, &local);

        let bind = name.as_ref().and_then(|n| bind_by_name.get(n));
        let Some(bind) = bind else {
            // Non-skinned helper node (twist/collision/COM/bumper/...): keep its
            // local untouched, just propagate its world to children.
            world.insert(bid, current_world);
            continue;
        };

        report.skinned_bones += 1;
        if max_identity_residual(&mat4_mul(&current_world, bind)) <= tol {
            report.already_consistent += 1;
            world.insert(bid, current_world);
            continue;
        }

        // Repose: world := inverse(bind); local := inverse(parent_world) @ world.
        match (
            mat4_affine_inverse(bind),
            mat4_affine_inverse(&parent_world),
        ) {
            (Some(desired_world), Some(inv_parent)) => {
                let new_local = mat4_mul(&inv_parent, &desired_world);
                set_local(&mut skel.blocks[bid], &new_local);
                world.insert(bid, desired_world);
                report.reposed += 1;
            }
            _ => {
                // Degenerate (non-invertible) — leave as-is to avoid corruption.
                world.insert(bid, current_world);
            }
        }
    }

    report
}

// ---------------------------------------------------------------------------
// Skeleton traversal
// ---------------------------------------------------------------------------

/// Return node block ids in parent-before-child order, plus a `child → parent`
/// map. Nodes are every `NiNode` (and subtype) block.
fn node_order_and_parents(skel: &NifFile) -> (Vec<usize>, HashMap<usize, usize>) {
    let nodes = skel.find_blocks("NiNode");
    let node_set: HashSet<usize> = nodes.iter().copied().collect();

    let mut parents: HashMap<usize, usize> = HashMap::new();
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    for &bid in &nodes {
        if let Some(NifValue::Array(kids)) = skel.blocks[bid].get_field("Children") {
            for kid in kids {
                let Some(cid) = ref_value(Some(kid)) else {
                    continue;
                };
                if node_set.contains(&cid) {
                    parents.entry(cid).or_insert(bid);
                    children.entry(bid).or_default().push(cid);
                }
            }
        }
    }

    let mut order: Vec<usize> = Vec::with_capacity(nodes.len());
    let mut visited: HashSet<usize> = HashSet::new();
    // Roots first (stable input order), then any stragglers / cycle members.
    let mut stack: Vec<usize> = nodes
        .iter()
        .copied()
        .filter(|n| !parents.contains_key(n))
        .rev()
        .collect();
    let mut all_iter = nodes.iter().copied();
    loop {
        while let Some(node) = stack.pop() {
            if !visited.insert(node) {
                continue;
            }
            order.push(node);
            if let Some(kids) = children.get(&node) {
                for &kid in kids.iter().rev() {
                    if !visited.contains(&kid) {
                        stack.push(kid);
                    }
                }
            }
        }
        // Seed the next unvisited node (handles forests / parent-cycles).
        match all_iter.find(|n| !visited.contains(n)) {
            Some(n) => stack.push(n),
            None => break,
        }
    }

    (order, parents)
}

fn node_name(block: &NifBlock) -> Option<String> {
    match block.get_field("Name") {
        Some(NifValue::String(s)) => Some(s.trim_end_matches('\0').to_string()),
        _ => None,
    }
}

fn local_of(block: &NifBlock) -> BindMatrix {
    let rot = block
        .get_field("Rotation")
        .map(read_rotation)
        .unwrap_or_else(identity3);
    let trans = block
        .get_field("Translation")
        .map(read_translation)
        .unwrap_or([0.0; 3]);
    rot_trans_to_mat4(rot, trans)
}

/// Write a local 4×4 affine back onto a node, preserving the on-disk
/// representation (Struct `m11..m33` / `x,y,z`, or the rare `Matrix33`/`Vec3`
/// variant). Scale is left untouched.
fn set_local(block: &mut NifBlock, m: &BindMatrix) {
    let rot = [
        [m[0][0], m[0][1], m[0][2]],
        [m[1][0], m[1][1], m[1][2]],
        [m[2][0], m[2][1], m[2][2]],
    ];
    let trans = [m[0][3], m[1][3], m[2][3]];
    let rot_val = write_rotation_value(block.get_field("Rotation"), rot);
    block.set_field("Rotation", rot_val);
    let trans_val = write_translation_value(block.get_field("Translation"), trans);
    block.set_field("Translation", trans_val);
}

// ---------------------------------------------------------------------------
// NifValue <-> matrix conversion
// ---------------------------------------------------------------------------

fn read_rotation(value: &NifValue) -> [[f64; 3]; 3] {
    match value {
        // In-memory Matrix33 variant: A[i][j] already equals the math M[i][j].
        NifValue::Matrix33(a) => [
            [a[0][0] as f64, a[0][1] as f64, a[0][2] as f64],
            [a[1][0] as f64, a[1][1] as f64, a[1][2] as f64],
            [a[2][0] as f64, a[2][1] as f64, a[2][2] as f64],
        ],
        // On-disk struct form. Mapping matches the working skinned renderer:
        // M[0]=[m11,m21,m31], M[1]=[m12,m22,m32], M[2]=[m13,m23,m33].
        NifValue::Struct(fields) => {
            let g = |key: &str, default: f64| sget(fields, key).and_then(num).unwrap_or(default);
            [
                [g("m11", 1.0), g("m21", 0.0), g("m31", 0.0)],
                [g("m12", 0.0), g("m22", 1.0), g("m32", 0.0)],
                [g("m13", 0.0), g("m23", 0.0), g("m33", 1.0)],
            ]
        }
        _ => identity3(),
    }
}

fn read_translation(value: &NifValue) -> [f64; 3] {
    match value {
        NifValue::Vec3(v) => [v[0] as f64, v[1] as f64, v[2] as f64],
        NifValue::Struct(fields) => [
            sget(fields, "x").and_then(num).unwrap_or(0.0),
            sget(fields, "y").and_then(num).unwrap_or(0.0),
            sget(fields, "z").and_then(num).unwrap_or(0.0),
        ],
        _ => [0.0; 3],
    }
}

fn write_rotation_value(existing: Option<&NifValue>, rot: [[f64; 3]; 3]) -> NifValue {
    match existing {
        Some(NifValue::Matrix33(_)) => NifValue::Matrix33([
            [rot[0][0] as f32, rot[0][1] as f32, rot[0][2] as f32],
            [rot[1][0] as f32, rot[1][1] as f32, rot[1][2] as f32],
            [rot[2][0] as f32, rot[2][1] as f32, rot[2][2] as f32],
        ]),
        Some(NifValue::Struct(existing)) => {
            let mut fields = existing.clone();
            let pairs = [
                ("m11", rot[0][0]),
                ("m21", rot[0][1]),
                ("m31", rot[0][2]),
                ("m12", rot[1][0]),
                ("m22", rot[1][1]),
                ("m32", rot[1][2]),
                ("m13", rot[2][0]),
                ("m23", rot[2][1]),
                ("m33", rot[2][2]),
            ];
            for (key, val) in pairs {
                set_struct_num(&mut fields, key, val);
            }
            NifValue::Struct(fields)
        }
        _ => {
            let mut fields = IndexMap::new();
            let pairs = [
                ("m11", rot[0][0]),
                ("m21", rot[0][1]),
                ("m31", rot[0][2]),
                ("m12", rot[1][0]),
                ("m22", rot[1][1]),
                ("m32", rot[1][2]),
                ("m13", rot[2][0]),
                ("m23", rot[2][1]),
                ("m33", rot[2][2]),
            ];
            for (key, val) in pairs {
                fields.insert(key.to_string(), NifValue::Float(val));
            }
            NifValue::Struct(fields)
        }
    }
}

fn write_translation_value(existing: Option<&NifValue>, trans: [f64; 3]) -> NifValue {
    match existing {
        Some(NifValue::Vec3(_)) => {
            NifValue::Vec3([trans[0] as f32, trans[1] as f32, trans[2] as f32])
        }
        Some(NifValue::Struct(existing)) => {
            let mut fields = existing.clone();
            for (key, val) in [("x", trans[0]), ("y", trans[1]), ("z", trans[2])] {
                set_struct_num(&mut fields, key, val);
            }
            NifValue::Struct(fields)
        }
        _ => {
            let mut fields = IndexMap::new();
            for (key, val) in [("x", trans[0]), ("y", trans[1]), ("z", trans[2])] {
                fields.insert(key.to_string(), NifValue::Float(val));
            }
            NifValue::Struct(fields)
        }
    }
}

// ---------------------------------------------------------------------------
// Small struct/value helpers
// ---------------------------------------------------------------------------

fn bare_name(key: &str) -> &str {
    key.split(':').next().unwrap_or(key)
}

fn sget<'a>(fields: &'a IndexMap<String, NifValue>, key: &str) -> Option<&'a NifValue> {
    if let Some(v) = fields.get(key) {
        return Some(v);
    }
    fields
        .iter()
        .find(|(k, _)| bare_name(k) == key)
        .map(|(_, v)| v)
}

fn set_struct_num(fields: &mut IndexMap<String, NifValue>, key: &str, val: f64) {
    if fields.contains_key(key) {
        fields.insert(key.to_string(), NifValue::Float(val));
        return;
    }
    if let Some(existing) = fields.keys().find(|k| bare_name(k) == key).cloned() {
        fields.insert(existing, NifValue::Float(val));
        return;
    }
    fields.insert(key.to_string(), NifValue::Float(val));
}

fn num(value: &NifValue) -> Option<f64> {
    match value {
        NifValue::Float(f) => Some(*f),
        NifValue::Int(i) => Some(*i as f64),
        NifValue::UInt(u) => Some(*u as f64),
        _ => None,
    }
}

fn ref_value(value: Option<&NifValue>) -> Option<usize> {
    match value? {
        NifValue::Ref(r) if *r >= 0 => Some(*r as usize),
        NifValue::Int(i) if *i >= 0 => Some(*i as usize),
        NifValue::UInt(u) => Some(*u as usize),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 4×4 affine matrix math (row-major, bottom row [0,0,0,1])
// ---------------------------------------------------------------------------

fn identity3() -> [[f64; 3]; 3] {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}

fn identity4() -> BindMatrix {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn rot_trans_to_mat4(rot: [[f64; 3]; 3], trans: [f64; 3]) -> BindMatrix {
    [
        [rot[0][0], rot[0][1], rot[0][2], trans[0]],
        [rot[1][0], rot[1][1], rot[1][2], trans[1]],
        [rot[2][0], rot[2][1], rot[2][2], trans[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn mat4_mul(a: &BindMatrix, b: &BindMatrix) -> BindMatrix {
    let mut out = [[0.0_f64; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j] + a[i][3] * b[3][j];
        }
    }
    out
}

/// Inverse of an affine 4×4 (bottom row `[0,0,0,1]`) via 3×3 adjugate.
/// Returns `None` when the linear part is singular. Handles baked scale
/// (non-orthonormal linear part), so it is general for bind matrices.
fn mat4_affine_inverse(m: &BindMatrix) -> Option<BindMatrix> {
    let a = [
        [m[0][0], m[0][1], m[0][2]],
        [m[1][0], m[1][1], m[1][2]],
        [m[2][0], m[2][1], m[2][2]],
    ];
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
        - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv_det = 1.0 / det;
    let inv = [
        [
            (a[1][1] * a[2][2] - a[1][2] * a[2][1]) * inv_det,
            (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * inv_det,
            (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * inv_det,
        ],
        [
            (a[1][2] * a[2][0] - a[1][0] * a[2][2]) * inv_det,
            (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * inv_det,
            (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * inv_det,
        ],
        [
            (a[1][0] * a[2][1] - a[1][1] * a[2][0]) * inv_det,
            (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * inv_det,
            (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * inv_det,
        ],
    ];
    let t = [m[0][3], m[1][3], m[2][3]];
    let nt = [
        -(inv[0][0] * t[0] + inv[0][1] * t[1] + inv[0][2] * t[2]),
        -(inv[1][0] * t[0] + inv[1][1] * t[1] + inv[1][2] * t[2]),
        -(inv[2][0] * t[0] + inv[2][1] * t[1] + inv[2][2] * t[2]),
    ];
    Some([
        [inv[0][0], inv[0][1], inv[0][2], nt[0]],
        [inv[1][0], inv[1][1], inv[1][2], nt[1]],
        [inv[2][0], inv[2][1], inv[2][2], nt[2]],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

/// Max absolute deviation of the upper 3×4 (rotation + translation) of `m`
/// from the identity. Bottom row is invariant for affine products.
fn max_identity_residual(m: &BindMatrix) -> f64 {
    let mut max = 0.0_f64;
    for i in 0..3 {
        for j in 0..4 {
            let id = if i == j { 1.0 } else { 0.0 };
            max = max.max((m[i][j] - id).abs());
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_identity(m: &BindMatrix, tol: f64) -> bool {
        max_identity_residual(m) <= tol
    }

    fn rz(deg: f64) -> [[f64; 3]; 3] {
        let r = deg.to_radians();
        let (s, c) = r.sin_cos();
        [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
    }

    /// NifValue rotation struct (`m11..m33`) encoding math matrix `M[r][c]`.
    fn rot_struct(m: [[f64; 3]; 3]) -> NifValue {
        let mut f = IndexMap::new();
        for (key, val) in [
            ("m11", m[0][0]),
            ("m21", m[0][1]),
            ("m31", m[0][2]),
            ("m12", m[1][0]),
            ("m22", m[1][1]),
            ("m32", m[1][2]),
            ("m13", m[2][0]),
            ("m23", m[2][1]),
            ("m33", m[2][2]),
        ] {
            f.insert(key.to_string(), NifValue::Float(val));
        }
        NifValue::Struct(f)
    }

    fn vec_struct(t: [f64; 3]) -> NifValue {
        let mut f = IndexMap::new();
        f.insert("x".to_string(), NifValue::Float(t[0]));
        f.insert("y".to_string(), NifValue::Float(t[1]));
        f.insert("z".to_string(), NifValue::Float(t[2]));
        NifValue::Struct(f)
    }

    /// Add a NiNode bone with the given local rotation/translation + children.
    fn add_bone(
        nif: &mut NifFile,
        name: &str,
        rot: [[f64; 3]; 3],
        trans: [f64; 3],
        children: &[usize],
    ) -> usize {
        let mut fields = IndexMap::new();
        fields.insert("Name".to_string(), NifValue::String(name.to_string()));
        fields.insert("Translation".to_string(), vec_struct(trans));
        fields.insert("Rotation".to_string(), rot_struct(rot));
        fields.insert(
            "Num Children".to_string(),
            NifValue::UInt(children.len() as u64),
        );
        fields.insert(
            "Children".to_string(),
            NifValue::Array(children.iter().map(|c| NifValue::Ref(*c as i32)).collect()),
        );
        nif.add_block("NiNode", Some(fields))
    }

    #[test]
    fn affine_inverse_roundtrips() {
        let m = rot_trans_to_mat4(rz(90.0), [10.0, -5.0, 3.0]);
        let inv = mat4_affine_inverse(&m).unwrap();
        assert!(approx_identity(&mat4_mul(&m, &inv), 1e-9));
        assert!(approx_identity(&mat4_mul(&inv, &m), 1e-9));
    }

    #[test]
    fn matmul_identity_is_neutral() {
        let m = rot_trans_to_mat4(rz(30.0), [1.0, 2.0, 3.0]);
        assert_eq!(mat4_mul(&identity4(), &m), m);
        assert_eq!(mat4_mul(&m, &identity4()), m);
    }

    /// The target rest pose: bone A world = Rz(30°)+t, bone B world = Rz(75°)+t.
    fn target_worlds() -> (BindMatrix, BindMatrix) {
        let w_a = rot_trans_to_mat4(rz(30.0), [10.0, 0.0, 0.0]);
        let w_b = rot_trans_to_mat4(rz(75.0), [10.0, 20.0, 0.0]);
        (w_a, w_b)
    }

    fn bind_map() -> HashMap<String, BindMatrix> {
        let (w_a, w_b) = target_worlds();
        let mut map = HashMap::new();
        map.insert("BoneA".to_string(), mat4_affine_inverse(&w_a).unwrap());
        map.insert("BoneB".to_string(), mat4_affine_inverse(&w_b).unwrap());
        map
    }

    #[test]
    fn repose_fixes_broken_skeleton() {
        // Broken skeleton: both bones at identity local (raw, wrong pose).
        let mut skel = NifFile::new("fo4");
        let b = add_bone(&mut skel, "BoneB", identity3(), [0.0; 3], &[]);
        let _a = add_bone(&mut skel, "BoneA", identity3(), [0.0; 3], &[b]);
        let bind = bind_map();

        let (before, total) = skeleton_bind_consistency(&skel, &bind, REPOSE_TOL);
        assert_eq!(total, 2, "two skinned bones");
        assert_eq!(before, 0, "raw locals are inconsistent");

        let report = repose_skeleton_to_inverse_bind(&mut skel, &bind, REPOSE_TOL);
        assert_eq!(report.skinned_bones, 2);
        assert_eq!(report.reposed, 2);
        assert_eq!(report.already_consistent, 0);

        let (after, total) = skeleton_bind_consistency(&skel, &bind, REPOSE_TOL);
        assert_eq!(after, total, "all skinned bones consistent after repose");
    }

    const REPOSE_TOL: f64 = 1e-3;

    #[test]
    fn repose_skips_uniform_convention_offset() {
        // Humanoid convention: every bone's world @ bind is the SAME large
        // offset (vanilla FO4 human assets show a uniform ~120.84). Must not
        // repose even though no bone satisfies world @ bind == I.
        let (w_a, w_b) = target_worlds();
        let offset = rot_trans_to_mat4(identity3(), [0.0, 0.0, 120.844]);
        // bind := inv(world) @ offset  =>  world @ bind == offset for both.
        let mut map = HashMap::new();
        map.insert(
            "BoneA".to_string(),
            mat4_mul(&mat4_affine_inverse(&w_a).unwrap(), &offset),
        );
        map.insert(
            "BoneB".to_string(),
            mat4_mul(&mat4_affine_inverse(&w_b).unwrap(), &offset),
        );

        // Skeleton posed at the target worlds (locals consistent with them).
        let local_b = mat4_mul(&mat4_affine_inverse(&w_a).unwrap(), &w_b);
        let lb_rot = [
            [local_b[0][0], local_b[0][1], local_b[0][2]],
            [local_b[1][0], local_b[1][1], local_b[1][2]],
            [local_b[2][0], local_b[2][1], local_b[2][2]],
        ];
        let lb_trans = [local_b[0][3], local_b[1][3], local_b[2][3]];
        let mut skel = NifFile::new("fo4");
        let b = add_bone(&mut skel, "BoneB", lb_rot, lb_trans, &[]);
        let _a = add_bone(&mut skel, "BoneA", rz(30.0), [10.0, 0.0, 0.0], &[b]);

        let before_hashes: Vec<u64> = skel.blocks.iter().map(|b| b.content_hash()).collect();
        let report = repose_skeleton_to_inverse_bind(&mut skel, &map, REPOSE_TOL);
        assert_eq!(report.reposed, 0, "convention offset must not be reposed");
        assert_eq!(report.already_consistent, 2);
        let after_hashes: Vec<u64> = skel.blocks.iter().map(|b| b.content_hash()).collect();
        assert_eq!(before_hashes, after_hashes, "blocks must be untouched");
    }

    #[test]
    fn repose_is_noop_on_consistent_skeleton() {
        // Already-correct skeleton: localA = W_A, localB = inv(W_A) @ W_B.
        let (w_a, w_b) = target_worlds();
        let local_b = mat4_mul(&mat4_affine_inverse(&w_a).unwrap(), &w_b);
        let lb_rot = [
            [local_b[0][0], local_b[0][1], local_b[0][2]],
            [local_b[1][0], local_b[1][1], local_b[1][2]],
            [local_b[2][0], local_b[2][1], local_b[2][2]],
        ];
        let lb_trans = [local_b[0][3], local_b[1][3], local_b[2][3]];

        let mut skel = NifFile::new("fo4");
        let b = add_bone(&mut skel, "BoneB", lb_rot, lb_trans, &[]);
        let _a = add_bone(&mut skel, "BoneA", rz(30.0), [10.0, 0.0, 0.0], &[b]);
        let bind = bind_map();

        let (before, total) = skeleton_bind_consistency(&skel, &bind, REPOSE_TOL);
        assert_eq!((before, total), (2, 2), "already consistent");

        // Snapshot per-block content hashes to prove byte-stability.
        let before_hashes: Vec<u64> = skel.blocks.iter().map(|b| b.content_hash()).collect();
        let report = repose_skeleton_to_inverse_bind(&mut skel, &bind, REPOSE_TOL);
        assert_eq!(report.reposed, 0, "no-op gate must not rewrite any bone");
        assert_eq!(report.already_consistent, 2);
        let after_hashes: Vec<u64> = skel.blocks.iter().map(|b| b.content_hash()).collect();
        assert_eq!(before_hashes, after_hashes, "blocks must be untouched");
    }

    #[test]
    fn collect_bind_matrices_reads_bsskin_chain() {
        let (w_a, w_b) = target_worlds();
        let bind_a = mat4_affine_inverse(&w_a).unwrap();
        let bind_b = mat4_affine_inverse(&w_b).unwrap();

        let mut nif = NifFile::new("fo4");
        let a = add_bone(&mut nif, "BoneA", identity3(), [0.0; 3], &[]);
        let b = add_bone(&mut nif, "BoneB", identity3(), [0.0; 3], &[]);

        let bone_entry = |m: &BindMatrix| {
            let mut e = IndexMap::new();
            e.insert(
                "Rotation".to_string(),
                rot_struct([
                    [m[0][0], m[0][1], m[0][2]],
                    [m[1][0], m[1][1], m[1][2]],
                    [m[2][0], m[2][1], m[2][2]],
                ]),
            );
            e.insert(
                "Translation".to_string(),
                vec_struct([m[0][3], m[1][3], m[2][3]]),
            );
            NifValue::Struct(e)
        };

        let mut data_fields = IndexMap::new();
        data_fields.insert("Num Bones".to_string(), NifValue::UInt(2));
        data_fields.insert(
            "Bone List".to_string(),
            NifValue::Array(vec![bone_entry(&bind_a), bone_entry(&bind_b)]),
        );
        let data = nif.add_block("BSSkin::BoneData", Some(data_fields));

        let mut inst_fields = IndexMap::new();
        inst_fields.insert("Data".to_string(), NifValue::Ref(data as i32));
        inst_fields.insert(
            "Bones".to_string(),
            NifValue::Array(vec![NifValue::Ref(a as i32), NifValue::Ref(b as i32)]),
        );
        nif.add_block("BSSkin::Instance", Some(inst_fields));

        let binds = collect_bind_matrices_by_name(&nif);
        assert_eq!(binds.len(), 2);
        assert!(approx_identity(
            &mat4_mul(&w_a, binds.get("BoneA").unwrap()),
            1e-6
        ));
        assert!(approx_identity(
            &mat4_mul(&w_b, binds.get("BoneB").unwrap()),
            1e-6
        ));
    }
}
