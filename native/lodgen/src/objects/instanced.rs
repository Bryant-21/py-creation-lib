//! FO76 `BSDistantObjectInstancedNode` groups -> FO4 instanced object LOD (plan Task 15).
//! The engine keys instanced geometry by `uiObject` world-wide (LOD_RUNTIME.md Q9), draws every group with one
//! material bound to the world's unnumbered `Objects` atlas, and hides an instance whose id is a loaded ref's
//! FormID (Q3). Whatever can't meet those rules stays merged geometry.
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use nif_core_native::model::NifFile;

use crate::descriptors::QuadDesc;
use crate::input::RefInput;
use crate::objects::fo76_bto::ResourceId;
use crate::objects::geometry::LodGeometry;
use crate::objects::instance_layers::{AtlasTile, InstanceLayers};
use crate::objects::instance_transform::{InstanceTransform, decompose};
use crate::objects::object_lod::{BtoAlpha, BtoShader, BtoShape, MultiBoundAabb};
use crate::output::instance_block::Instance;
use crate::progress::QuadCtx;

/// One resource of an instance node, with each transform's index in the node's `Transforms` array.
pub struct ResourceInstances {
    pub resource: usize,
    pub rid: ResourceId,
    pub model: String,
    pub transforms: Vec<(usize, [[f32; 4]; 4])>,
}

pub struct InstanceGroup {
    pub object: u32,
    pub model: String,
    pub shape: BtoShape,
    /// Texture-array layer (`instance_layers`), or `NO_LAYER`.
    pub layer: u32,
    pub instances: Vec<Instance>,
    pub ids: Vec<u32>,
}

/// A transform that stays merged geometry: `(resource, transform)` indices into the node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MergedFallback {
    pub resource: usize,
    pub transform: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InstancedStats {
    pub degenerate: u64,
    pub unmatched_ids: u64,
}

/// One shape of a resource's LOD model after the atlas remap, in model space.
pub struct InstancePart {
    pub geometry: LodGeometry,
    pub diffuse: String,
    pub alpha: bool,
    /// The atlas tile its uvs were remapped into.
    pub tile: Option<AtlasTile>,
}

fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    for byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// FNV-1a 32 over the world's EditorID (case-folded) and the resource id's dir, file and ext.
pub fn object_id(world: &str, rid: &ResourceId) -> u32 {
    let rid_bytes = [rid.dir, rid.file, rid.ext].into_iter().flat_map(u32::to_le_bytes);
    fnv1a(world.to_lowercase().into_bytes().into_iter().chain([0]).chain(rid_bytes))
}

/// Owner of each object id. The engine keeps the first mesh it sees for an id for the whole session, across
/// worldspaces (LOD_RUNTIME Q9, Q16), so ids hash the world in; a second mesh hashing to a taken id stays merged.
/// Keyed by FO76 resource id, or by model and texture swap on the records path.
pub struct ObjectIds<K = ResourceId> {
    world: String,
    owners: Mutex<HashMap<u32, K>>,
}

impl<K: Clone + PartialEq> ObjectIds<K> {
    pub fn new(world: &str) -> Self {
        Self { world: world.to_lowercase(), owners: Mutex::new(HashMap::new()) }
    }

    pub fn claim_as(&self, id: u32, key: &K) -> Option<u32> {
        let mut owners = self.owners.lock().unwrap_or_else(|e| e.into_inner());
        (*owners.entry(id).or_insert_with(|| key.clone()) == *key).then_some(id)
    }
}

impl ObjectIds {
    pub fn claim(&self, rid: &ResourceId) -> Option<u32> {
        self.claim_as(object_id(&self.world, rid), rid)
    }
}

impl ObjectIds<String> {
    pub fn claim_record(&self, model: &str, material_swap: &BTreeMap<String, String>) -> Option<u32> {
        let mut key = model.replace('/', "\\").to_lowercase();
        for (from, to) in material_swap {
            key.push_str(&format!("|{from}={to}").to_lowercase());
        }
        let id = fnv1a(self.world.bytes().chain([0]).chain(key.bytes()));
        self.claim_as(id, &key)
    }
}

/// Splits a node's transforms into instance groups and merged fallbacks. `shape_for` builds a resource's object id,
/// model-space shape and layer (None keeps it merged); `id_for` finds the placed ref an instance stands for. With
/// `ids_required` (the segmented L4 level, whose loaded cells hide instances by id) an unmatched instance stays
/// merged, since id 0 never hides; elsewhere it keeps id 0.
pub fn group_tile_instances(
    resources: &[ResourceInstances],
    node_translation: [f32; 3],
    min_count: usize,
    ids_required: bool,
    mut shape_for: impl FnMut(&ResourceInstances) -> Option<(u32, BtoShape, u32)>,
    mut id_for: impl FnMut(&str, [f32; 3]) -> Option<u32>,
) -> (Vec<InstanceGroup>, Vec<MergedFallback>, InstancedStats) {
    let mut groups = Vec::new();
    let mut fallback = Vec::new();
    let mut stats = InstancedStats::default();
    for resource in resources {
        let mut candidates = Vec::new();
        for &(transform, matrix) in &resource.transforms {
            let merged = MergedFallback { resource: resource.resource, transform };
            match decompose(&matrix, node_translation) {
                InstanceTransform::Uniform { pos, scale, rot } => {
                    let id = id_for(&resource.model, pos);
                    if id.is_none() {
                        stats.unmatched_ids += 1;
                        if ids_required {
                            fallback.push(merged);
                            continue;
                        }
                    }
                    candidates.push((merged, Instance { pos, scale, rot }, id.unwrap_or(0)));
                }
                InstanceTransform::NonUniform { .. } => fallback.push(merged),
                InstanceTransform::Degenerate => stats.degenerate += 1,
            }
        }
        let shape = if candidates.len() >= min_count.max(1) { shape_for(resource) } else { None };
        let Some((object, shape, layer)) = shape else {
            fallback.extend(candidates.iter().map(|(merged, ..)| *merged));
            continue;
        };
        groups.push(InstanceGroup {
            object,
            model: resource.model.clone(),
            shape,
            layer,
            instances: candidates.iter().map(|(_, instance, _)| *instance).collect(),
            ids: candidates.iter().map(|(.., id)| *id).collect(),
        });
    }
    (groups, fallback, stats)
}

/// Placed refs sharing a LOD model and texture swap; `refs` index the slice given to `group_record_instances`.
pub struct RecordGroup {
    pub model: String,
    pub refs: Vec<usize>,
    pub instances: Vec<Instance>,
    pub ids: Vec<u32>,
}

/// Records-path grouping by `(LOD model, material swap)`. Returns the groups, the refs that stay merged (ascending)
/// and the counts. Trees, non-uniform transforms and SCOL parts with their own scale stay merged: the merged path
/// scales a part's vertices but not its node translation, which no instance transform reproduces.
pub fn group_record_instances(
    refs: &[&RefInput],
    level_index: usize,
    min_count: usize,
) -> (Vec<RecordGroup>, Vec<usize>, InstancedStats) {
    let ids_required = level_index == 0;
    let mut keys: HashMap<(String, &BTreeMap<String, String>), usize> = HashMap::new();
    let mut candidates: Vec<(String, Vec<(usize, Instance, u32)>)> = Vec::new();
    let mut merged = Vec::new();
    let mut stats = InstancedStats::default();
    for (i, r) in refs.iter().enumerate() {
        let Some(model) = r.lod_models.get(level_index).and_then(Option::as_ref) else {
            merged.push(i);
            continue;
        };
        if crate::trees::is_tree(r) || model.to_lowercase().ends_with(".dds") || (r.part_scale - 1.0).abs() > 1e-4 {
            merged.push(i);
            continue;
        }
        let instance = match decompose(&record_matrix(r), [0.0; 3]) {
            InstanceTransform::Uniform { pos, scale, rot } => Instance { pos, scale, rot },
            InstanceTransform::NonUniform { .. } => {
                merged.push(i);
                continue;
            }
            InstanceTransform::Degenerate => {
                stats.degenerate += 1;
                continue;
            }
        };
        let id = r
            .ref_id
            .get(..8)
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .filter(|id| *id != 0 && r.hideable_by_id);
        if id.is_none() {
            stats.unmatched_ids += 1;
            if ids_required {
                merged.push(i);
                continue;
            }
        }
        let key = (model.replace('/', "\\").to_lowercase(), &r.material_swap);
        let slot = *keys.entry(key).or_insert_with(|| {
            candidates.push((model.clone(), Vec::new()));
            candidates.len() - 1
        });
        candidates[slot].1.push((i, instance, id.unwrap_or(0)));
    }
    let mut groups = Vec::new();
    for (model, members) in candidates {
        if members.len() < min_count.max(1) {
            merged.extend(members.iter().map(|(i, ..)| *i));
            continue;
        }
        groups.push(RecordGroup {
            model,
            refs: members.iter().map(|(i, ..)| *i).collect(),
            instances: members.iter().map(|(_, instance, _)| *instance).collect(),
            ids: members.iter().map(|(.., id)| *id).collect(),
        });
    }
    merged.sort_unstable();
    (groups, merged, stats)
}

/// The ref's model-to-world transform as `transform_shape_with_world` applies it: position, `rotation_from_ref`,
/// uniform scale, then the SCOL part transform.
fn record_matrix(r: &RefInput) -> [[f32; 4]; 4] {
    let rot = crate::objects::parse_nif::rotation_from_ref(r.rot);
    let part = &r.part_transform;
    let mut m = [[0.0f32; 4]; 4];
    for i in 0..3 {
        for j in 0..4 {
            m[i][j] = (0..3).map(|k| rot[i][k] * part[k][j]).sum::<f32>() * r.scale;
        }
        m[i][3] += r.pos[i];
    }
    m[3][3] = 1.0;
    m
}

/// One model-space `BSTriShape` for an instance group, or None when any part is alpha-tested or samples anything
/// but the world atlas the instance renderer binds (`atlas_diffuse`). Vertex colours are dropped: the shared
/// material has none.
pub fn instance_shape(parts: Vec<InstancePart>, atlas_diffuse: &str, texture_set: [String; 10]) -> Option<BtoShape> {
    if parts.is_empty() || parts.iter().any(|p| p.alpha || !p.diffuse.eq_ignore_ascii_case(atlas_diffuse)) {
        return None;
    }
    let mut geometry = LodGeometry::new();
    for part in parts {
        append_geometry(&mut geometry, part.geometry);
    }
    if geometry.triangles.is_empty() {
        return None;
    }
    geometry.update_bbox();
    let center = geometry.bbox.center(false);
    let radius = geometry.bbox.radius();
    Some(BtoShape {
        geometry,
        segments: Vec::new(),
        segment_count: 0,
        translation: [0.0; 3],
        scale: 1.0,
        center,
        radius,
        multibound: MultiBoundAabb { position: center, extent: [radius; 3] },
        name_index_at: false,
        // Vanilla object-LOD flags: Specular | ZBuffer_Test (+ engine bits), ZBuffer_Write | LOD_Objects.
        shader: BtoShader::Lighting { texture_set, flags1: 0x8040_0001, flags2: 5, clamp: 0, backlight: None, grayscale_scale: None },
        // The instance prepass technique always alpha-tests and reads this property's threshold with no null check
        // (LOD_RUNTIME Q17). Threshold 0 keeps opaque atlas texels whatever their alpha.
        alpha: Some(BtoAlpha { flags: 4844, threshold: 0 }),
        enable_parent: 0,
    })
}

/// Appends the tile's instance pairs to the BTO the merged path wrote, or to a new root-only BTO when every object
/// was instanced. Returns the path, its size and whether the file is new. Scales carry layers once the world's
/// `layers` has one; a tile written before that is noted for `InstanceLayers::mark_unencoded_tiles`.
pub(crate) fn append_instance_groups(
    ctx: &QuadCtx<'_>,
    quad: &QuadDesc,
    written: Option<PathBuf>,
    groups: &[InstanceGroup],
    layers: Option<&InstanceLayers>,
) -> anyhow::Result<(PathBuf, u64, bool)> {
    let created = written.is_none();
    let (path, mut nif) = match written {
        Some(path) => {
            let bytes = std::fs::read(&path)?;
            let nif = NifFile::from_bytes(&bytes, Some(path.clone()))
                .map_err(|e| anyhow::anyhow!("re-read {}: {e:?}", path.display()))?;
            (path, nif)
        }
        None => {
            let season = ctx.settings.global.season.as_deref().unwrap_or("");
            let rel = crate::naming::bto(&ctx.world.editor_id, ctx.level, quad.x, quad.y, season);
            (ctx.paths.output_dir.join(rel.replace('\\', "/")), crate::output::bto::new_bto_nif_with_root().0)
        }
    };
    let encode = layers.is_some_and(|layers| !layers.is_empty());
    crate::output::bto::write_bto_with_instances(&mut nif, groups, encode);
    let bytes = nif.to_bytes().map_err(|e| anyhow::anyhow!("nif save failed: {e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &bytes)?;
    if let Some(layers) = layers.filter(|_| !encode) {
        layers.note_unencoded(path.clone());
    }
    Ok((path, bytes.len() as u64, created))
}

fn append_geometry(dst: &mut LodGeometry, src: LodGeometry) {
    let base = dst.vertices.len() as u32;
    let count = src.vertices.len();
    let fill = |values: Vec<[f32; 3]>, default: [f32; 3]| {
        if values.len() == count { values } else { vec![default; count] }
    };
    dst.uvcoords.extend(if src.uvcoords.len() == count { src.uvcoords } else { vec![[0.0, 0.0]; count] });
    dst.normals.extend(fill(src.normals, [0.0, 0.0, 1.0]));
    dst.tangents.extend(fill(src.tangents, [1.0, 0.0, 0.0]));
    dst.bitangents.extend(fill(src.bitangents, [0.0, 1.0, 0.0]));
    dst.vertices.extend(src.vertices);
    dst.triangles.extend(src.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
}

/// Placed refs by 64-unit bucket, for matching instances to the converted ESM's refs (Q3 ids are ref FormIDs).
pub struct RefIndex {
    buckets: HashMap<(i32, i32), Vec<RefPoint>>,
}

struct RefPoint {
    pos: [f32; 3],
    form_id: u32,
    models: Vec<String>,
    hideable: bool,
}

const BUCKET: f32 = 64.0;
const MATCH_DISTANCE: f32 = 1.0;

impl RefIndex {
    /// `refs`: (ref id as lodgen writes it, `{form_id:08X}` plus an optional suffix; position; LOD models; whether
    /// the engine can hide it by id).
    pub fn new<'a>(refs: impl IntoIterator<Item = (&'a str, [f32; 3], Vec<String>, bool)>) -> Self {
        let mut buckets: HashMap<(i32, i32), Vec<RefPoint>> = HashMap::new();
        for (ref_id, pos, models, hideable) in refs {
            let Some(form_id) = ref_id.get(..8).and_then(|hex| u32::from_str_radix(hex, 16).ok()) else {
                continue;
            };
            let models = models.into_iter().map(|m| m.to_ascii_lowercase()).collect();
            buckets.entry(bucket(pos)).or_default().push(RefPoint { pos, form_id, models, hideable });
        }
        Self { buckets }
    }

    /// The ref within 1 unit of `pos`: the one using `model` as a LOD model, else the only one there. None when that
    /// ref can't be hidden by id, since its instance would stay drawn over the loaded object.
    pub fn find(&self, model: &str, pos: [f32; 3]) -> Option<u32> {
        let (bx, by) = bucket(pos);
        let near: Vec<&RefPoint> = (bx - 1..=bx + 1)
            .flat_map(|x| (by - 1..=by + 1).map(move |y| (x, y)))
            .filter_map(|key| self.buckets.get(&key))
            .flatten()
            .filter(|p| p.pos.iter().zip(pos).map(|(a, b)| (a - b).powi(2)).sum::<f32>() <= MATCH_DISTANCE.powi(2))
            .collect();
        let model = model.to_ascii_lowercase();
        near.iter()
            .find(|p| p.models.iter().any(|m| *m == model))
            .or_else(|| (near.len() == 1).then(|| &near[0]))
            .filter(|p| p.hideable)
            .map(|p| p.form_id)
    }
}

fn bucket(pos: [f32; 3]) -> (i32, i32) {
    ((pos[0] / BUCKET).floor() as i32, (pos[1] / BUCKET).floor() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::instance_layers::{NO_LAYER, encode_scale, scale_layer};
    use nif_core_native::model::NifFile;

    fn uniform(x: f32, scale: f32) -> [[f32; 4]; 4] {
        [[scale, 0.0, 0.0, x], [0.0, scale, 0.0, 0.0], [0.0, 0.0, scale, 0.0], [0.0, 0.0, 0.0, 1.0]]
    }

    fn stretched() -> [[f32; 4]; 4] {
        [[2.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]
    }

    fn degenerate() -> [[f32; 4]; 4] {
        [[0.0; 4], [0.0; 4], [0.0; 4], [0.0, 0.0, 0.0, 1.0]]
    }

    fn triangle() -> LodGeometry {
        let mut g = LodGeometry::new();
        g.vertices = vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];
        g.uvcoords = vec![[0.0, 0.0], [0.5, 0.0], [0.0, 0.5]];
        g.normals = vec![[0.0, 0.0, 1.0]; 3];
        g.tangents = vec![[1.0, 0.0, 0.0]; 3];
        g.bitangents = vec![[0.0, 1.0, 0.0]; 3];
        g.triangles = vec![[0, 1, 2]];
        g
    }

    fn shape() -> BtoShape {
        let part = InstancePart { geometry: triangle(), diffuse: "w.dds".into(), alpha: false, tile: None };
        instance_shape(vec![part], "w.dds", Default::default()).unwrap()
    }

    fn tile() -> Vec<ResourceInstances> {
        vec![
            ResourceInstances {
                resource: 0,
                rid: ResourceId { dir: 1, file: 2, ext: 3 },
                model: "a.nif".into(),
                transforms: vec![(0, uniform(0.0, 1.0)), (1, uniform(100.0, 2.0)), (2, degenerate()), (3, uniform(200.0, 1.0))],
            },
            ResourceInstances {
                resource: 1,
                rid: ResourceId { dir: 1, file: 3, ext: 3 },
                model: "b.nif".into(),
                transforms: vec![(0, uniform(300.0, 1.0)), (1, stretched())],
            },
        ]
    }

    #[test]
    fn groups_uniform_instances_and_falls_back_for_the_rest() {
        let (groups, fallback, stats) =
            group_tile_instances(&tile(), [0.0; 3], 2, true, |r| Some((object_id("w", &r.rid), shape(), 3)), |_, _| Some(7));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].model, "a.nif");
        assert_eq!(groups[0].layer, 3);
        assert_eq!(groups[0].instances.len(), 3);
        assert_eq!(groups[0].ids, vec![7, 7, 7]);
        assert_eq!(groups[0].instances[1].scale, 2.0);
        let fallback: std::collections::HashSet<_> = fallback.into_iter().collect();
        let expected = [MergedFallback { resource: 1, transform: 0 }, MergedFallback { resource: 1, transform: 1 }];
        assert_eq!(fallback, expected.into_iter().collect());
        assert_eq!(stats.degenerate, 1);
    }

    #[test]
    fn node_translation_moves_every_instance() {
        let (groups, ..) =
            group_tile_instances(&tile(), [5.0, 6.0, 7.0], 2, false, |r| Some((object_id("w", &r.rid), shape(), NO_LAYER)), |_, _| Some(1));
        assert_eq!(groups[0].instances[0].pos, [5.0, 6.0, 7.0]);
        assert_eq!(groups[0].instances[2].pos, [205.0, 6.0, 7.0]);
    }

    #[test]
    fn unmatched_instances_stay_merged_at_l4_and_keep_id_zero_elsewhere() {
        let no_ref = |_: &str, pos: [f32; 3]| (pos[0] != 100.0).then_some(9);
        let (groups, fallback, stats) =
            group_tile_instances(&tile(), [0.0; 3], 2, true, |r| Some((object_id("w", &r.rid), shape(), NO_LAYER)), no_ref);
        assert_eq!(groups[0].instances.len(), 2);
        assert!(fallback.contains(&MergedFallback { resource: 0, transform: 1 }));
        assert_eq!(stats.unmatched_ids, 1);

        let (groups, _, stats) =
            group_tile_instances(&tile(), [0.0; 3], 2, false, |r| Some((object_id("w", &r.rid), shape(), NO_LAYER)), no_ref);
        assert_eq!(groups[0].ids, vec![9, 0, 9]);
        assert_eq!(stats.unmatched_ids, 1);
    }

    #[test]
    fn a_resource_without_a_shape_stays_merged() {
        let (groups, fallback, _) = group_tile_instances(&tile(), [0.0; 3], 2, true, |_| None, |_, _| Some(1));
        assert!(groups.is_empty());
        assert_eq!(fallback.len(), 5);
    }

    #[test]
    fn object_ids_are_stable_and_distinct() {
        let a = ResourceId { dir: 1, file: 2, ext: 3 };
        let b = ResourceId { dir: 1, file: 3, ext: 3 };
        assert_eq!(object_id("w", &a), object_id("w", &a));
        assert_ne!(object_id("w", &a), object_id("w", &b));
    }

    #[test]
    fn the_same_mesh_gets_a_different_id_in_another_world() {
        // The engine keeps an object id's first mesh for the whole session, across worldspaces.
        let rid = ResourceId { dir: 1, file: 2, ext: 3 };
        assert_ne!(ObjectIds::new("Appalachia").claim(&rid), ObjectIds::new("ThePitt").claim(&rid));
        let plain = BTreeMap::new();
        assert_ne!(
            ObjectIds::<String>::new("Appalachia").claim_record("a.nif", &plain),
            ObjectIds::<String>::new("Commonwealth").claim_record("a.nif", &plain)
        );
        assert_eq!(ObjectIds::new("Appalachia").claim(&rid), ObjectIds::new("APPALACHIA").claim(&rid));
    }

    #[test]
    fn an_object_id_belongs_to_the_first_resource_that_claims_it() {
        let ids = ObjectIds::new("w");
        let a = ResourceId { dir: 1, file: 2, ext: 3 };
        assert_eq!(ids.claim(&a), Some(object_id("w", &a)));
        assert_eq!(ids.claim(&a), Some(object_id("w", &a)));
        ids.owners.lock().unwrap().insert(object_id("w", &ResourceId { dir: 9, file: 9, ext: 9 }), a);
        assert_eq!(ids.claim(&ResourceId { dir: 9, file: 9, ext: 9 }), None);
    }

    #[test]
    fn instance_shapes_need_opaque_parts_on_the_world_atlas() {
        let part =
            |diffuse: &str, alpha: bool| InstancePart { geometry: triangle(), diffuse: diffuse.into(), alpha, tile: None };
        let both = instance_shape(vec![part("W.dds", false), part("w.dds", false)], "w.dds", Default::default()).unwrap();
        assert_eq!(both.geometry.vertices.len(), 6);
        assert_eq!(both.geometry.triangles[1], [3, 4, 5]);
        assert_eq!(both.translation, [0.0; 3]);
        assert!(instance_shape(vec![part("w.dds", true)], "w.dds", Default::default()).is_none());
        assert!(instance_shape(vec![part("w.001.dds", false)], "w.dds", Default::default()).is_none());
        assert!(instance_shape(Vec::new(), "w.dds", Default::default()).is_none());
    }

    #[test]
    fn instance_shapes_carry_an_alpha_property_that_never_discards() {
        let part = InstancePart { geometry: triangle(), diffuse: "w.dds".into(), alpha: false, tile: None };
        let shape = instance_shape(vec![part], "w.dds", Default::default()).unwrap();
        let alpha = shape.alpha.expect("the instance prepass reads the alpha threshold without a null check");
        assert_eq!((alpha.flags, alpha.threshold), (4844, 0));
    }

    #[test]
    fn refs_match_by_position_and_lod_model() {
        let index = RefIndex::new([
            ("0012ABCD", [100.0, 100.0, 0.0], vec!["Meshes\\A.nif".to_string()], true),
            ("0012ABCE", [100.5, 100.0, 0.0], vec!["b.nif".to_string()], true),
            ("0012ABCF", [500.0, 500.0, 0.0], vec![], true),
            ("bad", [0.0, 0.0, 0.0], vec![], true),
            ("0012ABD0", [900.0, 900.0, 0.0], vec!["d.nif".to_string()], false),
        ]);
        assert_eq!(index.find("meshes\\a.nif", [100.2, 100.0, 0.0]), Some(0x0012_ABCD));
        assert_eq!(index.find("b.nif", [100.2, 100.0, 0.0]), Some(0x0012_ABCE));
        assert_eq!(index.find("c.nif", [100.2, 100.0, 0.0]), None);
        assert_eq!(index.find("c.nif", [500.0, 499.5, 0.0]), Some(0x0012_ABCF));
        assert_eq!(index.find("c.nif", [503.0, 500.0, 0.0]), None);
        assert_eq!(index.find("c.nif", [63.9, 64.0, 0.0]), None);
        // A ref the engine can't resolve by id never hides its instance, so it counts as unmatched.
        assert_eq!(index.find("d.nif", [900.0, 900.0, 0.0]), None);
    }

    #[test]
    fn written_tile_has_scene_root_then_pairs() {
        let group = InstanceGroup {
            object: 7,
            model: "a.nif".into(),
            shape: shape(),
            layer: NO_LAYER,
            instances: vec![Instance { pos: [1.0, 2.0, 3.0], scale: 1.0, rot: [0.0, 0.0, 0.0, 1.0] }],
            ids: vec![0x0012_ABCD],
        };
        let mut nif = crate::output::bto::new_bto_nif_with_root().0;
        crate::output::bto::write_bto_with_instances(&mut nif, &[group], false);
        let bytes = nif.to_bytes().unwrap();
        let nif = NifFile::from_bytes(&bytes, None).unwrap();
        let roots = &nif.header.footer_roots;
        assert_eq!(roots.len(), 3);
        assert_eq!(nif.blocks[roots[0] as usize].type_name, "NiNode");
        assert_eq!(nif.blocks[roots[1] as usize].type_name, "BSTriShape");
        assert_eq!(nif.blocks[roots[2] as usize].type_name, "BSDistantObjectInstanceDataBlock");
        let (object, instances, ids) =
            crate::output::instance_block::decode(&nif.blocks[roots[2] as usize].remainder).unwrap();
        assert_eq!((object, instances.len(), ids), (7, 1, vec![0x0012_ABCD]));
    }

    fn layered_group(layer: u32) -> InstanceGroup {
        let at = |x: f32, scale: f32| Instance { pos: [x, 0.0, 0.0], scale, rot: [0.0, 0.0, 0.0, 1.0] };
        InstanceGroup {
            object: 9,
            model: "a.nif".into(),
            shape: shape(),
            layer,
            instances: vec![at(0.0, 1.3), at(10.0, 0.7), at(20.0, 2.1)],
            ids: vec![1, 2, 3],
        }
    }

    fn written_instances(nif: &NifFile) -> Vec<Vec<Instance>> {
        nif.blocks
            .iter()
            .filter(|b| b.type_name == crate::output::instance_block::BLOCK_TYPE)
            .map(|b| crate::output::instance_block::decode(&b.remainder).unwrap().1)
            .collect()
    }

    #[test]
    fn written_scales_carry_the_group_layer_only_when_encoding() {
        let group = layered_group(5);
        let mut plain = crate::output::bto::new_bto_nif_with_root().0;
        crate::output::bto::write_bto_with_instances(&mut plain, std::slice::from_ref(&group), false);
        let block = plain.blocks.iter().find(|b| b.type_name == crate::output::instance_block::BLOCK_TYPE).unwrap();
        assert_eq!(block.remainder, crate::output::instance_block::encode(9, &group.instances, &group.ids));

        let mut encoded = crate::output::bto::new_bto_nif_with_root().0;
        crate::output::bto::write_bto_with_instances(&mut encoded, &[layered_group(5), layered_group(NO_LAYER)], true);
        let written = written_instances(&encoded);
        assert_eq!(written.len(), 2);
        for (instances, layer) in written.iter().zip([5, NO_LAYER]) {
            for (instance, original) in instances.iter().zip(&group.instances) {
                assert_eq!(scale_layer(instance.scale), layer);
                assert_eq!(instance.scale, encode_scale(original.scale, layer));
                assert_eq!((instance.pos, instance.rot), (original.pos, original.rot));
            }
        }
    }

    #[test]
    fn tiles_written_before_the_first_layer_get_the_atlas_sentinel_once_one_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let world = crate::input::WorldspaceInput::from_cells("W", Vec::new());
        let settings = crate::settings::LodSettings::fo4_default();
        let game = crate::game::Game::fo4();
        let paths = crate::progress::LodPaths {
            data_dirs: Vec::new(),
            output_dir: tmp.path().to_path_buf(),
            source_data_dir: None,
        };
        let ctx = QuadCtx { world: &world, settings: &settings, game: &game, paths: &paths, level: 4 };
        let quad = |x: i32| QuadDesc {
            z_order: 0,
            x,
            y: 0,
            quad_level: 4,
            quad_index: 0,
            quad_offset: 16384.0,
            static_indices: Vec::new(),
            statics: Vec::new(),
            out_values: Default::default(),
        };
        let read = |path: &std::path::Path| {
            written_instances(&NifFile::from_bytes(&std::fs::read(path).unwrap(), None).unwrap())
        };
        let layers = crate::objects::instance_layers::InstanceLayers::default();
        let early = layered_group(NO_LAYER);
        let (early_path, ..) =
            append_instance_groups(&ctx, &quad(0), None, std::slice::from_ref(&early), Some(&layers)).unwrap();
        assert_eq!(read(&early_path), vec![early.instances.clone()], "no layer yet: today's scales");
        layers.mark_unencoded_tiles().unwrap();
        assert_eq!(read(&early_path), vec![early.instances.clone()], "a world without layers keeps today's scales");

        let rect = crate::atlas::AtlasRect::from_map_row(256, 256, 0, 0, 4096, 4096, "w.dds", false);
        let tile = crate::objects::instance_layers::AtlasTile {
            key: r"textures\a_d.dds".into(),
            rect: crate::objects::instance_layers::UvRect::of(&rect),
        };
        let late = layered_group(layers.assign(Some(&tile)));
        let (late_path, ..) =
            append_instance_groups(&ctx, &quad(1), None, std::slice::from_ref(&late), Some(&layers)).unwrap();
        let late_written = read(&late_path);
        assert!(late_written[0].iter().all(|i| scale_layer(i.scale) == 0));

        layers.mark_unencoded_tiles().unwrap();
        let fixed = read(&early_path);
        assert!(fixed[0].iter().zip(&early.instances).all(|(i, o)| i.scale == encode_scale(o.scale, NO_LAYER)));
        assert_eq!(read(&late_path), late_written, "only the early tile is rewritten");

        let (untracked, ..) = append_instance_groups(&ctx, &quad(2), None, std::slice::from_ref(&early), None).unwrap();
        assert_eq!(read(&untracked), vec![early.instances.clone()], "without a layer registry scales stay plain");
    }

    fn record(id: &str, model: &str, pos: [f32; 3], rot_z: f32, scale: f32) -> crate::input::RefInput {
        crate::input::RefInput {
            ref_id: id.to_string(),
            ref_flags: 0,
            enable_parent: 0,
            cell: (0, 0),
            pos,
            rot: [0.0, 0.0, rot_z],
            scale,
            color: 1.0,
            alpha_threshold: 128,
            is_billboard: false,
            is_grass: false,
            base_name: "Rock".to_string(),
            base_flags: 0,
            material_name: String::new(),
            full_model: String::new(),
            lod_models: [Some(model.to_string()), Some(model.to_string()), None, None],
            part_transform: crate::input::identity_part_transform(),
            part_scale: 1.0,
            material_swap: Default::default(),
            hideable_by_id: true,
        }
    }

    #[test]
    fn records_sharing_a_lod_model_and_texture_swap_become_one_group() {
        let rock = "meshes/lod/rock01_lod.nif";
        let mut swapped = record("00000104", rock, [0.0; 3], 0.0, 1.0);
        swapped.material_swap.insert("rock01.bgsm".into(), "rock01_moss.bgsm".into());
        let refs = [
            record("00000101", rock, [100.0, 0.0, 5.0], 0.0, 1.0),
            record("00000102", "Meshes\\LOD\\Rock01_LOD.nif", [200.0, 0.0, 5.0], std::f32::consts::FRAC_PI_2, 2.0),
            record("00000103", rock, [300.0, 0.0, 5.0], 0.0, 0.5),
            swapped,
            record("00000105", "meshes/lod/rock02_lod.nif", [0.0; 3], 0.0, 1.0),
        ];
        let refs: Vec<_> = refs.iter().collect();
        let (groups, merged, stats) = group_record_instances(&refs, 0, 2);
        assert_eq!(groups.len(), 1);
        let group = &groups[0];
        assert_eq!(group.refs, vec![0, 1, 2]);
        assert_eq!(group.ids, vec![0x101, 0x102, 0x103]);
        assert_eq!(group.instances[1].pos, [200.0, 0.0, 5.0]);
        assert!((group.instances[1].scale - 2.0).abs() < 1e-5);
        // rot z = 90 degrees through rotation_from_ref's Rz(-z): a -90 degree turn about z.
        let half = std::f32::consts::FRAC_1_SQRT_2;
        let rot = group.instances[1].rot;
        assert!((rot[2] + half).abs() < 1e-5 && (rot[3] - half).abs() < 1e-5, "{rot:?}");
        assert_eq!(merged, vec![3, 4]);
        assert_eq!(stats, InstancedStats::default());
    }

    #[test]
    fn refs_the_engine_cannot_hide_by_id_stay_merged_at_l4() {
        let rock = "meshes/lod/rock01_lod.nif";
        let mut hidden = record("00000301", rock, [10.0; 3], 0.0, 1.0);
        hidden.hideable_by_id = false;
        let refs = [hidden, record("00000302", rock, [20.0; 3], 0.0, 1.0), record("00000303", rock, [30.0; 3], 0.0, 1.0)];
        let refs: Vec<_> = refs.iter().collect();
        let (groups, merged, stats) = group_record_instances(&refs, 0, 2);
        assert_eq!(groups[0].ids, vec![0x302, 0x303]);
        assert_eq!(merged, vec![0]);
        assert_eq!(stats.unmatched_ids, 1);
        let (groups, merged, _) = group_record_instances(&refs, 1, 2);
        assert_eq!(groups[0].ids, vec![0, 0x302, 0x303]);
        assert!(merged.is_empty());
    }

    #[test]
    fn record_object_ids_follow_the_model_and_its_texture_swap() {
        let plain = BTreeMap::new();
        let moss = BTreeMap::from([("rock01.bgsm".to_string(), "rock01_moss.bgsm".to_string())]);
        let ids = ObjectIds::<String>::new("w");
        let rock = ids.claim_record("Meshes/LOD/Rock01_LOD.nif", &plain).unwrap();
        assert_eq!(ids.claim_record("meshes\\lod\\rock01_lod.nif", &plain), Some(rock));
        let mossy = ids.claim_record("meshes\\lod\\rock01_lod.nif", &moss).unwrap();
        assert_ne!(mossy, rock);
        assert_eq!(ids.claim_as(rock, &"another model".to_string()), None);
    }

    #[test]
    fn scaled_parts_and_trees_stay_merged_and_scol_parts_keep_their_ref_id() {
        let rock = "meshes/lod/rock01_lod.nif";
        let mut scaled_part = record("00000201:p0", rock, [0.0; 3], 0.0, 1.0);
        scaled_part.part_scale = 1.5;
        let mut tree = record("00000202", rock, [0.0; 3], 0.0, 1.0);
        tree.base_flags = 0x1000;
        let refs = [
            scaled_part,
            tree,
            record("00000203:p1", rock, [10.0; 3], 0.0, 1.0),
            record("00000204", rock, [20.0; 3], 0.0, 1.0),
            record("bad", rock, [30.0; 3], 0.0, 1.0),
        ];
        let refs: Vec<_> = refs.iter().collect();
        let (groups, merged, stats) = group_record_instances(&refs, 0, 2);
        assert_eq!(groups[0].ids, vec![0x203, 0x204]);
        assert_eq!(merged, vec![0, 1, 4]);
        assert_eq!(stats.unmatched_ids, 1);
        // Away from L4 an id-less instance stays instanced with id 0.
        let (groups, merged, _) = group_record_instances(&refs, 1, 2);
        assert_eq!(groups[0].ids, vec![0x203, 0x204, 0]);
        assert_eq!(merged, vec![0, 1]);
    }
}
