//! Plugin-driven previs planning.
//!
//! Inputs inside the CK-verified record subset produce CK byte-identical
//! scenes. Everything else that can still be modelled correctly is planned
//! too, and each such deviation is recorded in [`NonParity`]: the scene is
//! then conservative-but-correct geometry for the clean-room solver (occluders
//! only where they are certainly static and opaque, targets only where culling
//! them is safe), not CK's bytes. Only inputs that would give wrong geometry
//! are refused.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

use crate::assets::{AssetResolver, mesh_path};
use crate::error::{PrevisError, Result};
use crate::f32ops::{Transform, Vec3};
use nif_core_native::model::NifFile;

use crate::combined::{CombinedRow, PsgEntry, PsgSource, group_rows, no_combined_geometry};
use crate::csg::SharedGeometry;
use crate::group::{animates, moved_by_physics};
use crate::metadata::{CellMetadata, Xcri};
use crate::nif::direct_shapes;
use crate::plugin::{LoadOrder, Plugin, RecordRef, read_u32};
use crate::precombine::{REFERENCE_FLAG_INITIALLY_DISABLED, linked_references};
use crate::precombine_stage::{geometry_file_name, sidecar_dir};
use crate::records::{self, LandHeights, Reference, ReferencePolicy};
use crate::scene::{
    self, DirectFormFlags, DirectShape, Geometry, INSTANCE_GATE, INSTANCE_OCCLUDER, INSTANCE_TARGET, LandSurface, MODEL_ID_BASE, SMALL_MODEL_DISTANCE,
    Scene, SceneSink, exterior_volume, flat_heights, snapped_volume, vhgt_heights, vhgt_minimum_height,
};
use crate::umbra_solve::Aabb;

const FLAT_LAND_HEIGHT: f32 = -2048.0;
const NO_LANDSCAPE_MINIMUM_Z: f32 = -128.0;
const DIRECT_BASE_SIGNATURES: [&[u8; 4]; 2] = [b"ACTI", b"STAT"];
/// Further bases whose references retail CK tomes list as previs targets
/// (census of the 307 Fallout4.esm interiors with shipped tomes). Everything
/// else (doors, lights, loose items, ...) can open, move or be picked up and
/// is left out of visibility: never an occluder, never culled. Interior doors
/// CK makes gates of are gates instead (see [`gate_candidate`]).
const RELAXED_BASE_SIGNATURES: [&[u8; 4]; 7] = [b"MSTT", b"CONT", b"FURN", b"SCOL", b"TERM", b"TACT", b"FLOR"];
/// Exterior group-shape object IDs keep 14 bits for the shape index.
const EXTERIOR_COMBINED_INDEX_LIMIT: usize = 1 << 14;
/// CK's exterior tomes also list the targets of the CELLs around the cluster
/// whose boxes come within this of it. A target listed only in its own
/// cluster's tome is culled whenever the camera sees just its part across the
/// border (the Vault 76 walkway of CELL (-26, 22) seen from tome 00263E19).
const NEIGHBOUR_REACH: f32 = 32.0;

/// Parsed direct shapes per model path, shared across references.
#[derive(Default)]
pub struct ModelCache {
    /// Failures are cached too (as `(unsupported, message)`): SeventySix places
    /// some unusable models thousands of times.
    models: Mutex<DecodedModels>,
    shared_geometry: Mutex<FxHashMap<String, Arc<SharedGeometry>>>,
    linked: Mutex<Option<Arc<FxHashSet<u32>>>>,
    motions: Mutex<FxHashMap<String, Motion>>,
}

type CachedShapes = std::result::Result<Arc<Vec<DirectShape>>, (bool, String)>;

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn shape() -> CachedShapes {
        Ok(Arc::new(vec![DirectShape { positions: vec![[1.0, 2.0, 3.0]; 32], indices: vec![0, 1, 2], transforms: vec![], bound_center: [0.0; 3], bound_radius: 1.0, occludes: true }]))
    }

    #[test]
    fn bounded_cache_evicts_without_changing_in_flight_geometry() {
        let mut cache = DecodedModels::new(1200);
        let original = shape();
        let retained = original.as_ref().unwrap().clone();
        cache.insert("a".into(), original);
        cache.insert("b".into(), shape());
        assert!(cache.get("b").is_some());
        cache.insert("c".into(), shape());
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_some());
        assert_eq!(retained[0].positions, vec![[1.0, 2.0, 3.0]; 32]);
        for i in 0..100 { cache.insert(i.to_string(), shape()); }
        assert!(cache.stats.peak_retained_bytes <= cache.stats.limit_bytes);
        assert!(cache.stats.evictions > 0);
        let bytes = cache.stats.retained_bytes;
        cache.insert("99".into(), shape());
        assert_eq!(cache.stats.retained_bytes, bytes);
    }

    #[test]
    fn oversized_models_are_not_retained_and_failures_still_cache() {
        let mut cache = DecodedModels::new(256);
        cache.insert("large".into(), shape());
        assert!(cache.get("large").is_none());
        assert_eq!(cache.stats.oversized, 1);
        cache.insert("missing".into(), Err((true, "not found".into())));
        assert!(matches!(cache.get("missing"), Some(Err((true, _)))));
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ModelCacheStats {
    pub limit_bytes: usize,
    pub retained_bytes: usize,
    pub peak_retained_bytes: usize,
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub oversized: usize,
}

struct CachedModel {
    shapes: CachedShapes,
    bytes: usize,
    referenced: bool,
}

struct DecodedModels {
    entries: FxHashMap<String, CachedModel>,
    clock: VecDeque<String>,
    stats: ModelCacheStats,
}

impl Default for DecodedModels {
    fn default() -> Self { Self::new(512 << 20) }
}

impl DecodedModels {
    fn new(limit_bytes: usize) -> Self {
        Self { entries: FxHashMap::default(), clock: VecDeque::new(), stats: ModelCacheStats { limit_bytes, ..ModelCacheStats::default() } }
    }

    fn get(&mut self, key: &str) -> Option<CachedShapes> {
        match self.entries.get_mut(key) {
            Some(entry) => {
                entry.referenced = true;
                self.stats.hits += 1;
                Some(entry.shapes.clone())
            }
            None => { self.stats.misses += 1; None }
        }
    }

    fn insert(&mut self, key: String, shapes: CachedShapes) {
        if self.entries.contains_key(&key) { return; }
        let bytes = 2 * key.capacity() + std::mem::size_of::<CachedModel>() + match &shapes {
            Ok(shapes) => shapes.capacity() * std::mem::size_of::<DirectShape>() + shapes.iter().map(|shape| {
                shape.positions.capacity() * std::mem::size_of::<Vec3>()
                    + shape.indices.capacity() * std::mem::size_of::<u16>()
                    + shape.transforms.capacity() * std::mem::size_of::<Transform>()
            }).sum::<usize>(),
            Err((_, message)) => message.capacity(),
        };
        if bytes > self.stats.limit_bytes { self.stats.oversized += 1; return; }
        while self.stats.retained_bytes + bytes > self.stats.limit_bytes {
            let oldest = self.clock.pop_front().unwrap();
            let entry = self.entries.get_mut(&oldest).unwrap();
            if entry.referenced {
                entry.referenced = false;
                self.clock.push_back(oldest);
            } else {
                self.stats.retained_bytes -= entry.bytes;
                self.entries.remove(&oldest);
                self.stats.evictions += 1;
            }
        }
        self.stats.retained_bytes += bytes;
        self.stats.peak_retained_bytes = self.stats.peak_retained_bytes.max(self.stats.retained_bytes);
        self.clock.push_back(key.clone());
        self.entries.insert(key, CachedModel { shapes, bytes, referenced: false });
    }
}

/// How a placed model moves on its own: see [`moved_by_physics`] and [`animates`].
#[derive(Clone, Copy, Default)]
struct Motion {
    physics: bool,
    animation: bool,
}

impl ModelCache {
    pub fn with_budget(bytes: usize) -> Self {
        Self { models: Mutex::new(DecodedModels::new(bytes)), ..Self::default() }
    }

    pub fn stats(&self) -> ModelCacheStats { self.models.lock().unwrap().stats.clone() }

    /// The plugin's sidecar CSG, opened once for every combined CELL to read
    /// its rows from.
    fn shared_geometry(&self, plugin: &Plugin) -> Result<Arc<SharedGeometry>> {
        let mut opened = self.shared_geometry.lock().unwrap();
        if let Some(geometry) = opened.get(&plugin.name) {
            return Ok(geometry.clone());
        }
        let path = sidecar_dir(&plugin.path).join(geometry_file_name(&plugin.name));
        let geometry = match SharedGeometry::open(&path) {
            Ok(geometry) => Arc::new(geometry),
            Err(PrevisError::Io { .. }) => {
                return Err(PrevisError::unsupported(format!("precombined geometry not found: {}", path.display())));
            }
            Err(error) => return Err(error),
        };
        opened.insert(plugin.name.clone(), geometry.clone());
        Ok(geometry)
    }

    /// References some `XLKR` in the plugin points at; a script can reach and
    /// disable or move them through the link.
    fn linked(&self, plugin: &Plugin) -> Result<Arc<FxHashSet<u32>>> {
        let mut slot = self.linked.lock().unwrap();
        if let Some(linked) = slot.as_ref() {
            return Ok(linked.clone());
        }
        let linked = Arc::new(linked_references(plugin)?);
        *slot = Some(linked.clone());
        Ok(linked)
    }

    fn shapes(&self, model: &str, assets: &AssetResolver) -> Result<Arc<Vec<DirectShape>>> {
        let relative = mesh_path(model)?;
        let key = relative.to_ascii_lowercase();
        let rebuild = |cached: &std::result::Result<Arc<Vec<DirectShape>>, (bool, String)>| match cached {
            Ok(shapes) => Ok(shapes.clone()),
            Err((true, message)) => Err(PrevisError::Unsupported(message.clone())),
            Err((false, message)) => Err(PrevisError::Invalid(message.clone())),
        };
        if let Some(cached) = self.models.lock().unwrap().get(&key) {
            return rebuild(&cached);
        }
        let loaded = match assets.read(&relative) {
            Ok(Some(bytes)) => direct_shapes(&bytes, assets).map(Arc::new).map_err(|e| e.with_context(&relative)),
            Ok(None) => Err(PrevisError::unsupported(format!("model not found: {relative}"))),
            Err(error) => Err(error),
        };
        let cached = loaded.map_err(|error| match error {
            PrevisError::Unsupported(message) => (true, message),
            PrevisError::Invalid(message) => (false, message),
            other => (false, other.to_string()),
        });
        let result = rebuild(&cached);
        self.models.lock().unwrap().insert(key, cached);
        result
    }

    /// A model that cannot be read counts as still (planning it fails and
    /// notes why anyway).
    fn motion(&self, model: &str, assets: &AssetResolver) -> Motion {
        let Ok(relative) = mesh_path(model) else { return Motion::default() };
        let key = relative.to_ascii_lowercase();
        if let Some(&motion) = self.motions.lock().unwrap().get(&key) {
            return motion;
        }
        let motion = match assets.read(&relative) {
            Ok(Some(bytes)) => NifFile::from_bytes_raw_arrays(&bytes, None)
                .map_or(Motion::default(), |nif| Motion { physics: moved_by_physics(&nif), animation: animates(&nif) }),
            _ => Motion::default(),
        };
        self.motions.lock().unwrap().insert(key, motion);
        motion
    }
}

pub struct Context<'a> {
    pub load_order: &'a LoadOrder,
    pub assets: &'a AssetResolver,
    pub models: &'a ModelCache,
    pub policy: ReferencePolicy,
    /// Whether plans may keep CK's small-model cull distance at all.
    pub small_model_cull: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlannedReference {
    pub form_id: u32,
    pub base_form_key: String,
    pub model: String,
}

/// Deviations from the CK-verified subset, by kind. A plan with any entry is
/// `non_parity`: correct visibility geometry, but not CK byte-identical.
#[derive(Default)]
pub struct NonParity(BTreeMap<String, (usize, String)>);

impl NonParity {
    pub fn note(&mut self, kind: impl Into<String>, example: impl Into<String>) {
        let entry = self.0.entry(kind.into()).or_insert_with(|| (0, example.into()));
        entry.0 += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_notes(self) -> Vec<String> {
        self.0
            .into_iter()
            .map(|(kind, (count, example))| {
                if example.is_empty() {
                    format!("{kind} (x{count})")
                } else {
                    format!("{kind} (x{count}, e.g. {example})")
                }
            })
            .collect()
    }
}

/// A planned cluster. Its scene went to the planner's [`SceneSink`].
pub struct ClusterPlan {
    pub root_cell: u32,
    pub root_grid: Option<(i32, i32)>,
    pub scene_name: String,
    /// The scene's view volume; `None` when CK emits no visibility files for
    /// the cluster.
    pub volume: Option<[f32; 12]>,
    /// Explicit task bounds for exteriors; interiors pass none.
    pub task_bounds: Option<[f32; 6]>,
    /// Whether small models keep CK's 4096-unit cull distance. Only when the
    /// request asks for it; then interiors whose view volume reaches farther
    /// still drop it (FO76's Whitespring Mall otherwise hides its signs from
    /// across the atrium) and exterior clusters keep it, as CK does.
    pub small_model_cull: bool,
    /// CELL metadata to stamp; empty when CK leaves the cluster untouched.
    pub cells: Vec<(u32, CellMetadata)>,
    pub references: Vec<PlannedReference>,
    /// Empty for CK byte-identical scenes; see [`NonParity`].
    pub non_parity: Vec<String>,
}

fn clean_editor_id(value: &str) -> String {
    value.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

fn base_form_key(plugin: &Plugin, raw: u32) -> String {
    let (owner, object_id) = plugin.owner_of(raw);
    format!("{owner}:{object_id:06X}")
}

fn signature_text(signature: &[u8; 4]) -> String {
    String::from_utf8_lossy(signature).into_owned()
}

struct DirectCandidate {
    form_id: u32,
    base_signature: [u8; 4],
    model_key: String,
    model: String,
    transform: Transform,
    non_occluder: bool,
    is_water: bool,
}

/// Resolves one REFR; `None` when it contributes no visibility geometry.
///
/// Parity path: ACTI/STAT bases with verified fields. The rest is `non_parity`
/// and noted: unverified-but-harmless fields are ignored, runtime-dynamic
/// references and non-static bases are left out, and references whose base
/// cannot be read are dropped (a dropped reference is simply never culled).
fn direct_candidate(
    context: &Context,
    plugin: &Plugin,
    form_id: u32,
    notes: &mut NonParity,
) -> Result<Option<DirectCandidate>> {
    let record = plugin
        .record(form_id)
        .ok_or_else(|| PrevisError::invalid(format!("REFR {form_id:08X} is not indexed")))?;
    let reference = match records::read_reference(plugin, &record, context.policy) {
        Ok(reference) => reference,
        Err(error) => {
            notes.note("unreadable reference dropped", error.to_string());
            return Ok(None);
        }
    };
    let (base, transform, non_occluder) = match reference {
        Reference::Primitive => return Ok(None),
        // CK's precombine predicate and every retail tome leave disabled refs out.
        Reference::Excluded("initially disabled") => return Ok(None),
        Reference::Excluded(reason) => {
            notes.note(format!("{reason} reference left out of visibility"), format!("{form_id:08X}"));
            return Ok(None);
        }
        Reference::Placed { base, transform, unverified_field, non_occluder } => {
            if let Some(field) = unverified_field {
                notes.note(format!("REFR field {field} ignored"), format!("{form_id:08X}"));
            }
            if let Some(reason) = non_occluder {
                notes.note(format!("{reason} reference never occludes"), format!("{form_id:08X}"));
            }
            (base, transform, non_occluder.is_some())
        }
    };
    if context.models.linked(plugin)?.contains(&form_id) {
        notes.note("linked-to reference left out of visibility", format!("{form_id:08X}"));
        return Ok(None);
    }
    let Some((base_plugin, base_record)) = context.load_order.resolve(plugin, base) else {
        notes.note("reference with unresolved base dropped", format!("{form_id:08X} -> {base:08X}"));
        return Ok(None);
    };
    let base_info = match records::read_base(base_plugin, &base_record) {
        Ok(info) => info,
        Err(error) => {
            notes.note("reference with unreadable base dropped", format!("{form_id:08X}: {error}"));
            return Ok(None);
        }
    };
    if base_info.is_marker {
        return Ok(None);
    }
    let signature = base_info.signature;
    if !DIRECT_BASE_SIGNATURES.contains(&&signature) {
        if !RELAXED_BASE_SIGNATURES.contains(&&signature) {
            notes.note(format!("{} base left out of visibility", signature_text(&signature)), format!("{form_id:08X}"));
            return Ok(None);
        }
        notes.note(format!("{} base included", signature_text(&signature)), format!("{form_id:08X}"));
    }
    let Some(model) = base_info.model else {
        notes.note("base without a model path ignored", base_form_key(plugin, base));
        return Ok(None);
    };
    let motion = context.models.motion(&model, context.assets);
    if !matches!(&signature, b"STAT" | b"SCOL") && motion.physics {
        notes.note(format!("physics-driven {} reference left out of visibility", signature_text(&signature)), format!("{form_id:08X}"));
        return Ok(None);
    }
    if motion.animation && !non_occluder {
        notes.note("animated reference never occludes", format!("{form_id:08X}"));
    }
    Ok(Some(DirectCandidate {
        form_id,
        base_signature: signature,
        model_key: base_form_key(plugin, base),
        model,
        transform,
        non_occluder: non_occluder || motion.animation,
        is_water: base_info.is_water,
    }))
}

/// DOOR record flag that keeps CK from making the door a gate (every
/// non-gate temporary door of the Whitespring bunker has it, no gate does).
const DOOR_FLAG_NOT_A_GATE: u32 = 0x10;

/// A temporary REFR that CK plans as a gate: a placed DOOR whose base lacks
/// [`DOOR_FLAG_NOT_A_GATE`] (in the Whitespring bunker, exactly CK's 32).
/// Anything else, including a scripted door, stays out of visibility: an
/// open doorway never culls.
fn gate_candidate(context: &Context, plugin: &Plugin, form_id: u32) -> Option<DirectCandidate> {
    let record = plugin.record(form_id)?;
    let Ok(Reference::Placed { base, transform, .. }) = records::read_reference(plugin, &record, context.policy) else {
        return None;
    };
    let (base_plugin, base_record) = context.load_order.resolve(plugin, base)?;
    if &base_record.signature != b"DOOR" || base_record.flags & DOOR_FLAG_NOT_A_GATE != 0 {
        return None;
    }
    let base_info = records::read_base(base_plugin, &base_record).ok().filter(|b| !b.is_marker)?;
    Some(DirectCandidate {
        form_id,
        base_signature: base_info.signature,
        model_key: base_form_key(plugin, base),
        model: base_info.model?,
        transform,
        non_occluder: true,
        is_water: false,
    })
}

/// Pools each gate's whole model as one gate instance, `0xFE` above the
/// door's object index as CK numbers them. Doors whose model cannot be read
/// are dropped and noted: without the gate the doorway is simply open.
fn stream_gates(context: &Context, gates: Vec<DirectCandidate>, sink: &mut dyn SceneSink, notes: &mut NonParity) {
    for gate in gates {
        match direct_scene_for(context, &gate) {
            // A non-occluder's scene is one instance of its whole model.
            Ok((mut direct, _)) => {
                let instance = &mut direct.instances[0];
                instance.flags = INSTANCE_GATE;
                instance.object_id = 0xFE00_0000 | (gate.form_id & 0x00FF_FFFF);
                sink.pooled(&gate.model_key, direct);
            }
            Err(error) => notes.note("gate with unusable model dropped", format!("{:08X}: {error}", gate.form_id)),
        }
    }
}

/// The reference's scene and world-space vertices.
fn direct_scene_for(context: &Context, candidate: &DirectCandidate) -> Result<(Scene, Vec<Vec3>)> {
    let shapes = context.models.shapes(&candidate.model, context.assets)?;
    let reference = (!candidate.transform.is_identity()).then_some(&candidate.transform);
    let form = DirectFormFlags {
        non_occluder: candidate.non_occluder,
        is_water: candidate.is_water,
    };
    scene::direct_scene(&shapes, candidate.form_id, reference, form)
}

/// Running world-space bounds of planned geometry.
struct Bounds {
    min: Vec3,
    max: Vec3,
}

impl Default for Bounds {
    fn default() -> Self {
        Bounds {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        }
    }
}

impl Bounds {
    fn extend(&mut self, points: &[Vec3]) {
        for point in points {
            for axis in 0..3 {
                self.min[axis] = self.min[axis].min(point[axis]);
                self.max[axis] = self.max[axis].max(point[axis]);
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.min[0] > self.max[0]
    }

    /// `snapped_volume` of these two corners equals that of every point seen.
    fn corners(&self) -> [Vec3; 2] {
        [self.min, self.max]
    }

    fn include(&mut self, other: &Bounds) {
        if !other.is_empty() {
            self.extend(&other.corners());
        }
    }
}

/// Builds each candidate's scene in order and streams it into `sink` at once.
/// Candidates whose model cannot be read are dropped and noted: an absent
/// reference is never culled, so that is safe. Returns the planned ones.
fn stream_directs(
    context: &Context,
    candidates: Vec<DirectCandidate>,
    occluders_only: bool,
    sink: &mut dyn SceneSink,
    bounds: &mut Bounds,
    notes: &mut NonParity,
) -> Vec<DirectCandidate> {
    let mut planned = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        match direct_scene_for(context, &candidate) {
            Ok((direct, world)) => {
                bounds.extend(&world);
                sink.pooled(&candidate.model_key, if occluders_only { occluder_only(direct) } else { direct });
                planned.push(candidate);
            }
            Err(error) => {
                notes.note("reference with unusable model dropped", format!("{:08X}: {error}", candidate.form_id));
            }
        }
    }
    planned
}

/// Keeps only the occluding part of a scene. Used for precombined source
/// references whose group meshes are unavailable: the group mesh is what the
/// engine draws, so the source reference can occlude but must not be a target.
fn occluder_only(mut scene: Scene) -> Scene {
    scene.instances.retain_mut(|instance| {
        instance.flags &= INSTANCE_OCCLUDER;
        instance.flags != 0
    });
    scene
}

fn planned(candidate: &DirectCandidate) -> PlannedReference {
    PlannedReference {
        form_id: candidate.form_id,
        base_form_key: candidate.model_key.clone(),
        model: candidate.model.clone(),
    }
}

fn floor_to_128(value: f64) -> f32 {
    ((value / 128.0).floor() * 128.0) as f32
}

/// CK's object ID for group shape `index` of an exterior CELL. Retail tomes
/// pack the CELL grid, modulo 32, above a 14-bit shape index; interiors use
/// grid (0, 0), i.e. `MODEL_ID_BASE + index`.
fn exterior_combined_object_id(x: i32, y: i32, index: usize) -> u32 {
    MODEL_ID_BASE | (((x & 0x1F) as u32) << 19) | (((y & 0x1F) as u32) << 14) | index as u32
}

/// One plugin's CSG rows as one CELL's groups reach them, each inflated once.
struct PsgRows {
    geometry: Arc<SharedGeometry>,
    rows: FxHashMap<u32, PsgEntry>,
}

impl PsgSource for PsgRows {
    fn entry(&mut self, offset: u32) -> Result<Option<&PsgEntry>> {
        let Some(row) = self.geometry.row(offset) else {
            return Ok(None);
        };
        if !self.rows.contains_key(&offset) {
            let entry = PsgEntry {
                vertex_desc: row.vertex_desc,
                vertex_count: row.vertex_count as usize,
                index_count: row.index_count as usize,
                data: self.geometry.read(row)?,
            };
            self.rows.insert(offset, entry);
        }
        Ok(self.rows.get(&offset))
    }
}

/// How reading a CELL's groups from one directory against one plugin's CSG went.
enum GroupRead {
    /// A group NIF is absent from the directory.
    Missing,
    /// A group NIF cannot be read or parsed, whichever CSG it indexes.
    Unreadable(PrevisError),
    /// The groups do not index this CSG (or it cannot be opened).
    Mismatch(PrevisError),
    Streamed { rows: usize, placed: Bounds },
}

/// Streams the group shapes of a CELL from `dir`, one group NIF at a time,
/// into `sink` (pending), with the rows of `owner`'s CSG.
fn stream_groups(
    context: &Context,
    dir: &str,
    cell_form_id: u32,
    xcri: &Xcri,
    owner: &Plugin,
    sink: &mut dyn SceneSink,
) -> GroupRead {
    let mut psg: Option<PsgRows> = None;
    let (mut rows, mut placed) = (0, Bounds::default());
    for mesh_id in &xcri.mesh_ids {
        let relative = format!("{dir}{:08X}_{mesh_id:08X}_OC.NIF", cell_form_id & 0x00FF_FFFF);
        let nif = match context.assets.read(&relative) {
            Ok(Some(bytes)) => match NifFile::from_bytes_raw_arrays(&bytes, None) {
                Ok(nif) => nif,
                Err(e) => return GroupRead::Unreadable(PrevisError::invalid(format!("{relative}: NIF parse failed: {e}"))),
            },
            Ok(None) => return GroupRead::Missing,
            Err(error) => return GroupRead::Unreadable(error),
        };
        let psg = match &mut psg {
            Some(psg) => psg,
            None => match context.models.shared_geometry(owner) {
                Ok(geometry) => psg.insert(PsgRows { geometry, rows: FxHashMap::default() }),
                Err(error) => return GroupRead::Mismatch(error),
            },
        };
        let streamed = group_rows(&nif, psg, context.assets, &mut |row, world| {
            placed.extend(world);
            sink.combined(row);
            rows += 1;
        });
        if let Err(error) = streamed {
            return GroupRead::Mismatch(error);
        }
    }
    if rows == 0 || placed.is_empty() {
        return GroupRead::Mismatch(no_combined_geometry());
    }
    GroupRead::Streamed { rows, placed }
}

/// Streams a CELL's group shapes into `sink`, read back from group NIFs and
/// the CSG they index, and returns how many there are. The shapes stay
/// pending in the sink for the caller to commit; on failure none are left.
///
/// Group NIFs are read only where the game loads them (`precombined_dir`):
/// file names carry the CELL's object ID alone, which other plugins' CELLs
/// reuse (SeventySix.esm has its own 1601D7 beside Fallout4.esm's). A CELL
/// override may keep a master's XCRI, so which plugin's CSG the groups index
/// is found by trying every plugin, the target first, and validating
/// (`group_rows` checks every PSG offset, descriptor and count).
fn stream_combined(
    context: &Context,
    plugin: &Plugin,
    cell_form_id: u32,
    xcri: &Xcri,
    sink: &mut dyn SceneSink,
    bounds: &mut Bounds,
) -> Result<usize> {
    let owners = std::iter::once(plugin).chain(context.load_order.plugins.iter().rev().filter(|p| !std::ptr::eq(*p, plugin)));
    let dir = format!("{}\\", plugin.precombined_dir(cell_form_id));
    let mut first_error = None;
    for owner in owners {
        match stream_groups(context, &dir, cell_form_id, xcri, owner, sink) {
            GroupRead::Streamed { rows, placed } => {
                bounds.include(&placed);
                return Ok(rows);
            }
            GroupRead::Mismatch(error) => {
                sink.discard_combined();
                first_error.get_or_insert(error);
            }
            GroupRead::Unreadable(error) => {
                sink.discard_combined();
                first_error.get_or_insert(error);
                break;
            }
            GroupRead::Missing => {
                sink.discard_combined();
                break;
            }
        }
    }
    Err(first_error.unwrap_or_else(|| {
        PrevisError::unsupported(format!(
            "precombined asset not found: meshes/precombined/{}/{:08X}_*_OC.NIF",
            plugin.name,
            cell_form_id & 0x00FF_FFFF
        ))
    }))
}

/// Streams the group shapes of one precombined CELL (pending in `sink`) and
/// returns their count, or `None` after noting why they are unavailable (the
/// caller then falls back to the source references).
fn combined_or_note(
    context: &Context,
    plugin: &Plugin,
    cell_form_id: u32,
    xcri: &Xcri,
    sink: &mut dyn SceneSink,
    bounds: &mut Bounds,
    notes: &mut NonParity,
) -> Option<usize> {
    match stream_combined(context, plugin, cell_form_id, xcri, sink, bounds) {
        Ok(rows) => Some(rows),
        Err(error) => {
            notes.note(
                "precombined geometry unavailable; source references occlude only",
                format!("{cell_form_id:08X}: {error}"),
            );
            None
        }
    }
}

/// Source references of a precombined CELL, to be planned as occluders only
/// (see [`occluder_only`]).
fn source_candidates(
    context: &Context,
    plugin: &Plugin,
    sources: &[u32],
    notes: &mut NonParity,
) -> Result<Vec<DirectCandidate>> {
    let mut candidates = Vec::new();
    for &form_id in sources {
        if let Some(candidate) = direct_candidate(context, plugin, form_id, notes)? {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

struct ShellCell {
    form_id: u32,
    editor_id: Option<String>,
    x: i32,
    y: i32,
    cell: records::Cell,
}

/// The CELLs of the 3x3 shell around a root that exist, in plugin order.
fn cluster_shell(plugin: &Plugin, world_form_id: u32, root_x: i32, root_y: i32) -> Result<Vec<ShellCell>> {
    let mut records_in_shell = Vec::with_capacity(9);
    for x in root_x - 1..=root_x + 1 {
        for y in root_y - 1..=root_y + 1 {
            match plugin.cells_at(world_form_id, x, y) {
                [] => {}
                [cell] => records_in_shell.push(plugin.record(*cell).unwrap()),
                _ => return Err(PrevisError::invalid(format!("duplicate exterior CELL coordinates ({x}, {y})"))),
            }
        }
    }
    records_in_shell.sort_by_key(|r| r.offset);
    let mut shell: Vec<ShellCell> = Vec::with_capacity(9);
    for record in records_in_shell {
        let form_id = record.form_id;
        let cell = records::read_cell(plugin, &record)?;
        let (x, y) = cell.grid.unwrap();
        shell.push(ShellCell {
            form_id,
            editor_id: cell.editor_id.clone(),
            x,
            y,
            cell,
        });
    }
    Ok(shell)
}

/// Terrain of one exterior CELL, or `None` (noted) when it has none usable.
/// A missing surface only removes occlusion, so it is always safe.
fn cell_land(
    land_plugin: &Plugin,
    land_rows: &[RecordRef],
    land_cell: &ShellCell,
    no_landscape: bool,
    notes: &mut NonParity,
) -> Result<Option<(LandHeights, u8, u32)>> {
    let land = match land_rows {
        [] if no_landscape => return Ok(None),
        [] => {
            notes.note("CELL without LAND has no terrain surface", format!("{:08X}", land_cell.form_id));
            return Ok(None);
        }
        [land] => land,
        _ => {
            return Err(PrevisError::unsupported(format!(
                "CELL {:08X} has {} LAND records",
                land_cell.form_id,
                land_rows.len()
            )));
        }
    };
    let heights = match records::read_land(land_plugin, land) {
        Ok(heights) => heights,
        Err(PrevisError::Unsupported(reason)) => {
            notes.note("unverified LAND has no terrain surface", reason);
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let hide_flags = match land_cell.cell.verified_land_hide_flags() {
        Ok(flags) => flags,
        Err(error) => {
            notes.note("unverified XCLC land flags ignored beyond the quadrant bits", error.to_string());
            land_cell.cell.land_flags & 0x0F
        }
    };
    Ok(Some((heights, hide_flags, land.form_id)))
}

/// Passes on the targets of a neighbouring CELL that reach the cluster
/// footprint (`[x0, y0, x1, y1]`), as targets only; everything else loses its
/// role. Group-shape rows keep their index, so a listed shape keeps the object
/// ID its own cluster's tome gives it.
struct NeighbourTargets<'a> {
    sink: &'a mut dyn SceneSink,
    footprint: [f32; 4],
}

impl NeighbourTargets<'_> {
    fn role(&self, flags: u32, geometry: &Geometry, transform: &[f32; 12]) -> u32 {
        let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for v in &geometry.vertices {
            for a in 0..2 {
                let p = transform[a * 4] * v[0] + transform[a * 4 + 1] * v[1] + transform[a * 4 + 2] * v[2] + transform[a * 4 + 3];
                lo[a] = lo[a].min(p);
                hi[a] = hi[a].max(p);
            }
        }
        let reaches = (0..2).all(|a| lo[a] <= self.footprint[a + 2] && self.footprint[a] <= hi[a]);
        if reaches { flags & INSTANCE_TARGET } else { 0 }
    }
}

impl SceneSink for NeighbourTargets<'_> {
    fn lands(&mut self, _: &[LandSurface]) {}

    fn pooled(&mut self, model_key: &str, mut direct: Scene) {
        for instance in &mut direct.instances {
            instance.flags = match direct.geometries.get(instance.model_index as usize) {
                Some(geometry) => self.role(instance.flags, geometry, &instance.transform),
                None => 0,
            };
        }
        direct.instances.retain(|instance| instance.flags != 0);
        if !direct.instances.is_empty() {
            self.sink.pooled(model_key, direct);
        }
    }

    fn combined(&mut self, (geometry, transform, flags): CombinedRow) {
        let flags = self.role(flags, &geometry, &transform);
        self.sink.combined((geometry, transform, flags));
    }

    fn commit_combined(&mut self, occluders_only: bool, object_id: &dyn Fn(usize) -> u32) {
        self.sink.commit_combined(occluders_only, object_id);
    }

    fn discard_combined(&mut self) {
        self.sink.discard_combined();
    }

    fn has_target(&self) -> bool {
        self.sink.has_target()
    }

    fn has_geometry(&self) -> bool {
        self.sink.has_geometry()
    }
}

/// Streams the targets of the 16 CELLs around cluster `(root_x, root_y)` that
/// reach it (see [`NEIGHBOUR_REACH`]). Their own clusters note how planning
/// them deviates from CK.
fn stream_neighbour_targets(
    context: &Context,
    plugin: &Plugin,
    world_form_id: u32,
    root_x: i32,
    root_y: i32,
    sink: &mut dyn SceneSink,
) -> Result<()> {
    let edge = |root: i32, offset: i32| ((root + offset) * 4096) as f32;
    let footprint = [
        edge(root_x, -1) - NEIGHBOUR_REACH,
        edge(root_y, -1) - NEIGHBOUR_REACH,
        edge(root_x, 2) + NEIGHBOUR_REACH,
        edge(root_y, 2) + NEIGHBOUR_REACH,
    ];
    let mut sink = NeighbourTargets { sink, footprint };
    let mut notes = NonParity::default();
    for x in root_x - 2..=root_x + 2 {
        for y in root_y - 2..=root_y + 2 {
            if (x - root_x).abs() <= 1 && (y - root_y).abs() <= 1 {
                continue;
            }
            // A duplicated grid fails its own cluster's plan.
            let &[cell] = plugin.cells_at(world_form_id, x, y) else { continue };
            let xcri = cell_xcri(plugin, cell)?;
            let combined: FxHashSet<u32> = xcri
                .iter()
                .flat_map(|xcri| xcri.reference_mesh_pairs.iter().map(|&(reference, _)| reference))
                .collect();
            let mut candidates = Vec::new();
            let temporary = plugin.cells.get(&cell).map(|t| t.temporary.as_slice()).unwrap_or(&[]);
            for child in temporary.iter().chain(plugin.persistent_standing_in(cell)?) {
                if &child.signature == b"REFR"
                    && !combined.contains(&child.form_id)
                    && let Some(candidate) = direct_candidate(context, plugin, child.form_id, &mut notes)?
                {
                    candidates.push(candidate);
                }
            }
            if let Some(xcri) = &xcri
                && let Some(rows) = combined_or_note(context, plugin, cell, xcri, &mut sink, &mut Bounds::default(), &mut notes)
            {
                sink.commit_combined(rows > EXTERIOR_COMBINED_INDEX_LIMIT, &|index| exterior_combined_object_id(x, y, index));
            }
            stream_directs(context, candidates, false, &mut sink, &mut Bounds::default(), &mut notes);
        }
    }
    Ok(())
}

/// A precombined exterior CELL inside the cluster.
struct CombinedCell {
    x: i32,
    y: i32,
    form_id: u32,
    sources: Vec<u32>,
    xcri: Xcri,
}

/// Previs for one 3x3 exterior cluster centred on `(root_x, root_y)`.
pub fn plan_exterior_cluster(
    context: &Context,
    world_form_id: u32,
    root_x: i32,
    root_y: i32,
    sink: &mut dyn SceneSink,
) -> Result<ClusterPlan> {
    if root_x.rem_euclid(3) != 0 || root_y.rem_euclid(3) != 0 {
        return Err(PrevisError::invalid("exterior visibility root coordinates must be divisible by three"));
    }
    let plugin = context.load_order.target();
    let world_record = plugin
        .record(world_form_id)
        .filter(|r| &r.signature == b"WRLD")
        .ok_or_else(|| PrevisError::invalid(format!("WRLD {world_form_id:08X} is not in {}", plugin.name)))?;
    let world = records::read_world(plugin, &world_record)?;
    let mut notes = NonParity::default();
    if world.unverified_flags != 0 {
        notes.note("unverified WRLD DATA flags ignored", format!("{} 0x{:02X}", world.editor_id, world.unverified_flags));
    }
    let shell = cluster_shell(plugin, world_form_id, root_x, root_y)?;
    if !shell.iter().any(|c| (c.x, c.y) == (root_x, root_y)) {
        // The UVD and every member's RVIS name the root CELL; with none there
        // is nothing verified to name them after.
        return Err(PrevisError::unsupported(format!(
            "cluster ({root_x}, {root_y}) of {} has no root CELL ({} of 9 CELLs)",
            plugin.name,
            shell.len()
        )));
    }
    if shell.len() != 9 {
        // Retail CK leaves partial clusters without previs; the missing CELLs
        // hold no geometry, so the scene is still correct.
        notes.note("partial cluster planned", format!("{} of 9 CELLs", shell.len()));
    }

    let (land_plugin, land_shell) = match world.land_parent {
        None => (plugin, None),
        Some(parent) => {
            let (parent_plugin, parent_record) = context
                .load_order
                .resolve(plugin, parent)
                .filter(|(_, r)| &r.signature == b"WRLD")
                .ok_or_else(|| PrevisError::invalid(format!("land parent WRLD {parent:08X} is unresolved")))?;
            let parent_shell = cluster_shell(parent_plugin, parent_record.form_id, root_x, root_y)?;
            (parent_plugin, Some(parent_shell))
        }
    };
    let inherits_land = land_shell.is_some();

    let mut lands = Vec::new();
    let mut missing_land = false;
    let mut minimum_height = f64::INFINITY;
    let mut cells = Vec::with_capacity(9);
    let mut reference_ids_by_cell: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut combined_cells: Vec<CombinedCell> = Vec::new();
    let mut root = None;
    let mut sorted: Vec<&ShellCell> = shell.iter().collect();
    sorted.sort_by_key(|c| (c.x, c.y));
    for cell in sorted {
        let xcri = cell_xcri(plugin, cell.form_id)?;
        let combined: FxHashSet<u32> = xcri
            .iter()
            .flat_map(|x| x.reference_mesh_pairs.iter().map(|&(reference, _)| reference))
            .collect();
        let mut references = Vec::new();
        let mut sources = Vec::new();
        for child in plugin.cells.get(&cell.form_id).map(|t| t.temporary.as_slice()).unwrap_or(&[]) {
            match &child.signature {
                b"REFR" if combined.contains(&child.form_id) => sources.push(child.form_id),
                b"REFR" => references.push(child.form_id),
                b"LAND" => {}
                other => notes.note(format!("temporary {} child ignored", signature_text(other)), format!("{:08X}", child.form_id)),
            }
        }
        // Where CK's reference list puts them among the temporary ones is unknown.
        let standing = plugin.persistent_standing_in(cell.form_id)?;
        if !standing.is_empty() {
            notes.note("persistent REFRs planned in the CELL they stand in", format!("{:08X}: {}", cell.form_id, standing.len()));
        }
        for child in standing {
            if combined.contains(&child.form_id) {
                sources.push(child.form_id);
            } else {
                references.push(child.form_id);
            }
        }
        if let Some(xcri) = xcri {
            notes.note("precombined CELL planned", format!("{:08X}", cell.form_id));
            combined_cells.push(CombinedCell {
                x: cell.x,
                y: cell.y,
                form_id: cell.form_id,
                sources,
                xcri,
            });
        }
        let land_cell = match &land_shell {
            Some(parent) => parent.iter().find(|c| (c.x, c.y) == (cell.x, cell.y)),
            None => Some(cell),
        };
        let land = match land_cell {
            Some(land_cell) => {
                let land_rows: Vec<_> = land_plugin
                    .cells
                    .get(&land_cell.form_id)
                    .map(|t| t.temporary.iter().filter(|r| &r.signature == b"LAND").copied().collect())
                    .unwrap_or_default();
                cell_land(land_plugin, &land_rows, land_cell, world.no_landscape, &mut notes)?
            }
            None => {
                notes.note("CELL without a land-parent CELL has no terrain surface", format!("{:08X}", cell.form_id));
                None
            }
        };
        match land {
            Some((heights, hide_flags, land_form_id)) => {
                let heights = match heights {
                    LandHeights::Flat => {
                        minimum_height = minimum_height.min(FLAT_LAND_HEIGHT as f64);
                        flat_heights(FLAT_LAND_HEIGHT)
                    }
                    LandHeights::Vhgt { base, deltas } => {
                        minimum_height = minimum_height.min(vhgt_minimum_height(base, &deltas));
                        vhgt_heights(base, &deltas)
                    }
                };
                lands.push(LandSurface {
                    x: cell.x,
                    y: cell.y,
                    // Inherited LAND keeps the parent geometry but targets the child CELL.
                    target_id: if inherits_land { cell.form_id } else { land_form_id },
                    heights,
                    hide_flags,
                });
            }
            None => missing_land |= !world.no_landscape,
        }
        if (cell.x, cell.y) == (root_x, root_y) {
            root = Some((cell.form_id, cell.editor_id.clone()));
        }
        reference_ids_by_cell.insert(cell.form_id, references);
        cells.push(cell.form_id);
    }
    let (root_cell, root_editor_id) = root.ok_or_else(|| PrevisError::invalid("exterior slice has no root CELL"))?;

    // CK walks CELLs in plugin order with each CELL's references in order;
    // the scene later consumes this list reversed.
    let candidate_ids: Vec<u32> = shell
        .iter()
        .flat_map(|c| reference_ids_by_cell[&c.form_id].iter().copied())
        .collect();
    let mut candidates = Vec::new();
    for &form_id in &candidate_ids {
        if let Some(candidate) = direct_candidate(context, plugin, form_id, &mut notes)? {
            candidates.push(candidate);
        }
    }

    let mut bounds = Bounds::default();
    let mut combined_loaded = false;
    let mut fallback_candidates = Vec::new();
    for cell in &combined_cells {
        match combined_or_note(context, plugin, cell.form_id, &cell.xcri, sink, &mut bounds, &mut notes) {
            Some(rows) => {
                // No object ID exists for shapes beyond the range; none may be a target.
                let beyond = rows > EXTERIOR_COMBINED_INDEX_LIMIT;
                if beyond {
                    notes.note("precombined CELL beyond the object ID range occludes only", format!("{:08X}", cell.form_id));
                }
                let (x, y) = (cell.x, cell.y);
                sink.commit_combined(beyond, &|index| exterior_combined_object_id(x, y, index));
                combined_loaded = true;
            }
            None => fallback_candidates.extend(source_candidates(context, plugin, &cell.sources, &mut notes)?),
        }
    }

    let has_land = !lands.is_empty();
    let land_surfaces = if world.no_landscape { Vec::new() } else { lands };
    sink.lands(&land_surfaces);
    candidates.reverse();
    let mut directs = stream_directs(context, candidates, false, sink, &mut bounds, &mut notes);
    directs.reverse();
    let fallback = stream_directs(context, fallback_candidates, true, sink, &mut bounds, &mut notes);

    let eligible: FxHashSet<u32> = directs.iter().map(|d| d.form_id).collect();
    let xcri_by_cell: FxHashMap<u32, &Xcri> = combined_cells.iter().map(|c| (c.form_id, &c.xcri)).collect();
    let metadata_for = |form_id: u32| CellMetadata {
        previs: true,
        root_visibility_cell: Some(root_cell),
        previs_reference_ids: reference_ids_by_cell[&form_id]
            .iter()
            .copied()
            .filter(|id| eligible.contains(id))
            .collect(),
        // Previs rewrites PCMB/XCRI, so precombined CELLs must restate theirs.
        precombine: xcri_by_cell.contains_key(&form_id),
        xcri: xcri_by_cell.get(&form_id).map(|x| (*x).clone()),
    };
    let scene_name = format!(
        "{}_{}_{:+}_{:+}",
        clean_editor_id(&world.editor_id),
        root_editor_id
            .as_deref()
            .map(clean_editor_id)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Cell{:06X}", root_cell & 0x00FF_FFFF)),
        root_x,
        root_y
    );
    let references = directs.iter().map(planned).collect();

    let no_extra_geometry = !combined_loaded && fallback.is_empty();
    if directs.is_empty() && !has_land && no_extra_geometry && !world.no_landscape {
        return Err(PrevisError::unsupported("cluster has neither LAND nor an eligible direct REFR"));
    }
    if directs.is_empty() && no_extra_geometry && world.no_landscape {
        // No geometry: CK writes no files, but still stamps the cluster when
        // any candidate REFR (for example a marker) was visited.
        let stamped = if candidate_ids.is_empty() {
            Vec::new()
        } else {
            cells.iter().map(|&c| (c, metadata_for(c))).collect()
        };
        return Ok(ClusterPlan {
            root_cell,
            root_grid: Some((root_x, root_y)),
            scene_name,
            volume: None,
            task_bounds: None,
            small_model_cull: context.small_model_cull,
            cells: stamped,
            references,
            non_parity: notes.into_notes(),
        });
    }
    if !sink.has_geometry() {
        return Err(PrevisError::unsupported("exterior scene has no LAND or direct geometry"));
    }
    stream_neighbour_targets(context, plugin, world_form_id, root_x, root_y, sink)?;

    let mut minimum_z = if world.no_landscape {
        NO_LANDSCAPE_MINIMUM_Z
    } else if minimum_height.is_finite() {
        floor_to_128(minimum_height)
    } else {
        NO_LANDSCAPE_MINIMUM_Z
    };
    if missing_land && !bounds.is_empty() {
        // Without every CELL's terrain the floor must still enclose the geometry.
        minimum_z = minimum_z.min(floor_to_128(bounds.min[2] as f64));
    }
    let volume = exterior_volume(root_x, root_y, minimum_z);
    let mut task_bounds = [0.0f32; 6];
    task_bounds.copy_from_slice(&volume[..6]);
    Ok(ClusterPlan {
        root_cell,
        root_grid: Some((root_x, root_y)),
        scene_name,
        volume: Some(volume),
        task_bounds: Some(task_bounds),
        small_model_cull: context.small_model_cull,
        cells: cells.iter().map(|&c| (c, metadata_for(c))).collect(),
        references,
        non_parity: notes.into_notes(),
    })
}

/// How far the camera gets from an interior's navmesh. FO76 shelters ring a
/// navmesh about 10,000 units across with scenery reaching 300,000 units;
/// voxelizing all of it takes tens of GB for a tome the camera never enters
/// the far parts of. The Whitespring bunker's content all lies within 650
/// units of its navmesh, so its occupancy (and CK's tome) is unchanged.
const CAMERA_REACH: f32 = 2048.0;

/// CK seeds the camera at each navmesh triangle's centroid raised by this
/// (all 5,540 seed points of the Whitespring bunker's `input.scene`).
const CAMERA_EYE_HEIGHT: f32 = 118.0;

/// Where the camera can be in an interior, from its navmesh.
#[derive(Debug, Default)]
pub struct InteriorCamera {
    /// CK's seed points: each navmesh triangle's centroid at eye height.
    pub seeds: Vec<Vec3>,
    /// The box occluders are voxelized in: the navmesh vertices' bounds grown
    /// by [`CAMERA_REACH`], or `None` (everything) without a navmesh.
    /// Geometry outside stays a target, so it is never wrongly culled; it
    /// just does not occlude.
    pub region: Option<Aabb>,
}

pub fn interior_camera(plugin: &Plugin, cell_form_id: u32) -> Result<InteriorCamera> {
    let mut camera = InteriorCamera::default();
    let Some(topology) = plugin.cells.get(&cell_form_id) else {
        return Ok(camera);
    };
    for navmesh in topology.temporary.iter().chain(&topology.persistent).filter(|r| &r.signature == b"NAVM") {
        let subrecords = plugin.subrecords_at(navmesh)?;
        let Some(nvnm) = subrecords.first(b"NVNM") else { continue };
        let invalid = || PrevisError::invalid(format!("NAVM {:08X} NVNM is truncated", navmesh.form_id));
        // Version, flags, parent world, parent cell, the vertex count and XYZ
        // floats, then the triangle count and 21-byte rows led by three u16
        // vertex indices.
        let count = read_u32(nvnm, 16).unwrap_or(0) as usize;
        let vertices: Vec<Vec3> = nvnm
            .get(20..20 + 12 * count)
            .ok_or_else(invalid)?
            .chunks_exact(12)
            .map(|v| std::array::from_fn(|k| f32::from_le_bytes(v[4 * k..4 * k + 4].try_into().unwrap())))
            .collect();
        let triangles_at = 24 + 12 * count;
        let triangles = read_u32(nvnm, triangles_at - 4).ok_or_else(invalid)? as usize;
        for row in nvnm.get(triangles_at..triangles_at + 21 * triangles).ok_or_else(invalid)?.chunks_exact(21) {
            let corner = |k: usize| vertices.get(u16::from_le_bytes([row[2 * k], row[2 * k + 1]]) as usize).ok_or_else(invalid);
            let (a, b, c) = (corner(0)?, corner(1)?, corner(2)?);
            let mut seed: Vec3 = std::array::from_fn(|k| (a[k] + b[k] + c[k]) / 3.0);
            seed[2] += CAMERA_EYE_HEIGHT;
            camera.seeds.push(seed);
        }
        for point in &vertices {
            let bounds = camera.region.get_or_insert(Aabb { min: *point, max: *point });
            for k in 0..3 {
                bounds.min[k] = bounds.min[k].min(point[k]);
                bounds.max[k] = bounds.max[k].max(point[k]);
            }
        }
    }
    camera.region = camera.region.map(|r| Aabb { min: r.min.map(|v| v - CAMERA_REACH), max: r.max.map(|v| v + CAMERA_REACH) });
    Ok(camera)
}

/// Previs for one interior CELL: its group shapes (if precombined) and its
/// direct references.
///
/// Parity path: exactly one temporary ACTI REFR, or a precombined CELL whose
/// temporary children are all combined or disabled and that has no persistent
/// children. Both build the same scene as before (CK byte-identical); any
/// other content is planned `non_parity`.
pub fn plan_interior_cell(context: &Context, cell_form_id: u32, sink: &mut dyn SceneSink) -> Result<ClusterPlan> {
    let plugin = context.load_order.target();
    let record = plugin
        .record(cell_form_id)
        .filter(|r| &r.signature == b"CELL")
        .ok_or_else(|| PrevisError::invalid(format!("CELL {cell_form_id:08X} is not in {}", plugin.name)))?;
    let cell = records::read_cell(plugin, &record)?;
    if !cell.interior {
        return Err(PrevisError::invalid("interior planning requires an interior CELL"));
    }
    // The scene name is only a label; it never reaches the scene bytes.
    let editor_id = cell
        .editor_id
        .as_deref()
        .map(clean_editor_id)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Cell{:06X}", cell_form_id & 0x00FF_FFFF));
    let mut notes = NonParity::default();
    let xcri = cell_xcri(plugin, cell_form_id)?;
    let topology = plugin.cells.get(&cell_form_id).cloned().unwrap_or_default();
    let combined: FxHashSet<u32> = xcri
        .iter()
        .flat_map(|x| x.reference_mesh_pairs.iter().map(|&(reference, _)| reference))
        .collect();

    let mut sources = Vec::new();
    let mut candidate_ids = Vec::new();
    let mut gates = Vec::new();
    for child in &topology.temporary {
        match &child.signature {
            b"REFR" if combined.contains(&child.form_id) => sources.push(child.form_id),
            b"REFR" => match gate_candidate(context, plugin, child.form_id) {
                Some(gate) => gates.push(gate),
                None => candidate_ids.push(child.form_id),
            },
            other => notes.note(format!("temporary {} child ignored", signature_text(other)), format!("{:08X}", child.form_id)),
        }
    }
    if !topology.persistent.is_empty() {
        // Retail tomes list persistent references too (for example 312
        // persistent ACTIs); the same static-and-not-dynamic rules apply.
        notes.note("persistent children planned", format!("{}", topology.persistent.len()));
        candidate_ids.extend(topology.persistent.iter().filter(|c| &c.signature == b"REFR").map(|c| c.form_id));
    }
    let mut candidates = Vec::new();
    for &form_id in &candidate_ids {
        if let Some(candidate) = direct_candidate(context, plugin, form_id, &mut notes)? {
            candidates.push(candidate);
        }
    }
    let direct_count = candidates.len();
    let single_acti = direct_count == 1 && &candidates[0].base_signature == b"ACTI";
    let mut bounds = Bounds::default();
    let directs = stream_directs(context, candidates, false, sink, &mut bounds, &mut notes);
    stream_gates(context, gates, sink, &mut notes);

    if let Some(xcri) = &xcri {
        if !candidate_ids.iter().all(|id| {
            let flags = plugin.record(*id).map_or(0, |r| r.flags);
            flags & REFERENCE_FLAG_INITIALLY_DISABLED != 0
        }) {
            notes.note("precombined CELL with direct references", "");
        }
        match combined_or_note(context, plugin, cell_form_id, xcri, sink, &mut bounds, &mut notes) {
            Some(_) => sink.commit_combined(false, &|index| MODEL_ID_BASE + index as u32),
            None => {
                let sources = source_candidates(context, plugin, &sources, &mut notes)?;
                stream_directs(context, sources, true, sink, &mut bounds, &mut notes);
            }
        }
    } else if !(single_acti && directs.len() == 1 && topology.temporary.len() == 1) {
        notes.note("interior beyond the verified single-ACTI subset", format!("{direct_count} direct references"));
    }

    if bounds.is_empty() {
        return Err(PrevisError::unsupported("interior has no visibility geometry"));
    }
    let volume = snapped_volume(&bounds.corners());
    if (0..3).any(|axis| volume[axis] >= volume[axis + 3]) {
        // Flat geometry on a 128 grid line (a lone rug or mattress): the
        // solver has no view volume to partition, so there is nothing to plan.
        return Err(PrevisError::unsupported("interior geometry spans no view volume"));
    }
    if !sink.has_target() {
        return Err(PrevisError::unsupported("interior has no visibility target"));
    }
    Ok(ClusterPlan {
        root_cell: cell_form_id,
        root_grid: None,
        scene_name: format!("Interior_{editor_id}"),
        volume: Some(volume),
        task_bounds: None,
        small_model_cull: context.small_model_cull && !reaches_past_small_model_cull(&volume),
        cells: vec![(
            cell_form_id,
            CellMetadata {
                previs: true,
                precombine: xcri.is_some(),
                previs_reference_ids: directs.iter().map(|d| d.form_id).collect(),
                xcri,
                ..CellMetadata::default()
            },
        )],
        references: directs.iter().map(planned).collect(),
        non_parity: notes.into_notes(),
    })
}

/// Whether a camera in `volume` can see past [`SMALL_MODEL_DISTANCE`]: its
/// diagonal is longer than that.
fn reaches_past_small_model_cull(volume: &[f32; 12]) -> bool {
    let diagonal = (0..3).map(|axis| (volume[axis + 3] - volume[axis]).powi(2)).sum::<f32>().sqrt();
    diagonal > SMALL_MODEL_DISTANCE
}

/// The CELL's `XCRI` combined-reference table, if the plugin stamped one.
pub(crate) fn cell_xcri(plugin: &Plugin, cell_form_id: u32) -> Result<Option<Xcri>> {
    let Some(record) = plugin.record(cell_form_id) else {
        return Ok(None);
    };
    let subrecords = plugin.subrecords_at(&record)?;
    let Some(data) = subrecords.first(b"XCRI") else {
        return Ok(None);
    };
    decode_xcri(data)
        .map(Some)
        .ok_or_else(|| PrevisError::invalid(format!("CELL {cell_form_id:08X} XCRI is malformed")))
}

pub(crate) fn decode_xcri(data: &[u8]) -> Option<Xcri> {
    let meshes = read_u32(data, 0)? as usize;
    let words = read_u32(data, 4)? as usize;
    if words % 2 != 0 || data.len() != 8 + 4 * (meshes + words) {
        return None;
    }
    let word = |i: usize| read_u32(data, 8 + 4 * i).unwrap();
    Some(Xcri {
        mesh_ids: (0..meshes).map(word).collect(),
        reference_mesh_pairs: (0..words / 2).map(|i| (word(meshes + 2 * i), word(meshes + 2 * i + 1))).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{INSTANCE_TARGET, INSTANCE_TARGET_OCCLUDER, Instance, SceneBuilder};
    use std::path::PathBuf;

    const BOX_MODEL: &str = "Test\\Box.nif";

    fn subrecords(fields: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (signature, data) in fields {
            out.extend_from_slice(*signature);
            out.extend_from_slice(&(data.len() as u16).to_le_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    fn record(signature: &[u8; 4], form_id: u32, flags: u32, fields: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let payload = subrecords(fields);
        let mut out = signature.to_vec();
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&form_id.to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&payload);
        out
    }

    fn group(label: u32, kind: i32, children: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = children.concat();
        let mut out = b"GRUP".to_vec();
        out.extend_from_slice(&((24 + body.len()) as u32).to_le_bytes());
        out.extend_from_slice(&label.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&body);
        out
    }

    fn u32s(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn zstr(value: &str) -> Vec<u8> {
        let mut out = value.as_bytes().to_vec();
        out.push(0);
        out
    }

    fn base(signature: &[u8; 4], form_id: u32) -> Vec<u8> {
        group(u32::from_le_bytes(*signature), 0, &[record(signature, form_id, 0, &[(b"MODL", zstr(BOX_MODEL))])])
    }

    fn reference(form_id: u32, base: u32, flags: u32, extra: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let placement: Vec<u8> = [0.0f32; 6].iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut fields = vec![(b"NAME", base.to_le_bytes().to_vec()), (b"DATA", placement)];
        fields.extend(extra.iter().map(|(s, d)| (*s, d.clone())));
        record(b"REFR", form_id, flags, &fields)
    }

    fn cell_children(cell: u32, persistent: Vec<Vec<u8>>, temporary: Vec<Vec<u8>>) -> Vec<u8> {
        let mut groups = Vec::new();
        if !persistent.is_empty() {
            groups.push(group(cell, 8, &persistent));
        }
        if !temporary.is_empty() {
            groups.push(group(cell, 9, &temporary));
        }
        group(cell, 6, &groups)
    }

    struct Fixture {
        dir: PathBuf,
        load_order: LoadOrder,
        assets: AssetResolver,
        models: ModelCache,
    }

    impl Fixture {
        fn new(name: &str, top_groups: Vec<Vec<u8>>) -> Self {
            let dir = std::env::temp_dir().join(format!("previs_test_{name}_{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let mut bytes = record(b"TES4", 0, 0, &[(b"HEDR", vec![0; 12])]);
            for top in top_groups {
                bytes.extend(top);
            }
            let path = dir.join("Test.esp");
            std::fs::write(&path, bytes).unwrap();
            let load_order = LoadOrder::open(&path, &[]).unwrap();
            let assets = AssetResolver::new(vec![dir.clone()], &[]).unwrap();
            let models = ModelCache::default();
            // A 512-unit opaque box: large enough to occlude.
            let corners: Vec<Vec3> = (0..8)
                .map(|i| [(i & 1) as f32 * 512.0, (i >> 1 & 1) as f32 * 512.0, (i >> 2 & 1) as f32 * 512.0])
                .collect();
            let shape = DirectShape {
                positions: corners,
                indices: vec![0, 1, 3, 0, 3, 2, 4, 5, 7, 4, 7, 6, 0, 1, 5, 0, 5, 4],
                transforms: Vec::new(),
                bound_center: [256.0; 3],
                bound_radius: 444.0,
                occludes: true,
            };
            let key = mesh_path(BOX_MODEL).unwrap().to_ascii_lowercase();
            models.models.lock().unwrap().insert(key, Ok(Arc::new(vec![shape])));
            Fixture { dir, load_order, assets, models }
        }

        fn context(&self) -> Context<'_> {
            Context {
                load_order: &self.load_order,
                assets: &self.assets,
                models: &self.models,
                policy: ReferencePolicy::default(),
                small_model_cull: true,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn has_note(plan: &ClusterPlan, kind: &str) -> bool {
        plan.non_parity.iter().any(|n| n.starts_with(kind))
    }

    /// The plan and the scene it builds.
    fn planned_scene(planner: impl FnOnce(&mut dyn SceneSink) -> Result<ClusterPlan>) -> Result<(ClusterPlan, Scene)> {
        let mut builder = SceneBuilder::default();
        let plan = planner(&mut builder)?;
        let scene = builder.finish(plan.volume.unwrap());
        Ok((plan, scene))
    }

    fn instance_of(scene: &Scene, object_id: u32) -> Option<&Instance> {
        scene.instances.iter().find(|i| i.object_id == object_id)
    }

    const STAT: u32 = 0x0000_0010;
    const ACTI: u32 = 0x0000_0011;
    const CONT: u32 = 0x0000_0012;
    const DOOR: u32 = 0x0000_0013;
    const NON_GATE_DOOR: u32 = 0x0000_0014;
    const CELL: u32 = 0x0000_0100;

    fn interior(name: &str, editor_id: &str, persistent: Vec<Vec<u8>>, temporary: Vec<Vec<u8>>) -> Fixture {
        let cell = record(b"CELL", CELL, 0, &[(b"EDID", zstr(editor_id)), (b"DATA", vec![1, 0])]);
        let block = group(0, 3, &[cell, cell_children(CELL, persistent, temporary)]);
        let cells = group(u32::from_le_bytes(*b"CELL"), 0, &[group(0, 2, &[block])]);
        let doors = group(
            u32::from_le_bytes(*b"DOOR"),
            0,
            &[
                record(b"DOOR", DOOR, 0, &[(b"MODL", zstr(BOX_MODEL))]),
                record(b"DOOR", NON_GATE_DOOR, DOOR_FLAG_NOT_A_GATE, &[(b"MODL", zstr(BOX_MODEL))]),
            ],
        );
        Fixture::new(name, vec![base(b"STAT", STAT), base(b"ACTI", ACTI), base(b"CONT", CONT), doors, cells])
    }

    #[test]
    fn single_acti_interior_stays_on_the_ck_parity_path() {
        let fixture = interior("acti", "ParityCell", Vec::new(), vec![reference(0x0000_0200, ACTI, 0, &[])]);
        let (plan, scene) = planned_scene(|sink| plan_interior_cell(&fixture.context(), CELL, sink)).unwrap();
        assert!(plan.non_parity.is_empty(), "{:?}", plan.non_parity);
        let shapes = fixture.models.shapes(BOX_MODEL, &fixture.assets).unwrap();
        let (expected, _) = scene::direct_scene(&shapes, 0x0000_0200, None, DirectFormFlags::default()).unwrap();
        assert_eq!(scene, expected);
        assert_eq!(plan.cells[0].1.previs_reference_ids, vec![0x0000_0200]);
    }

    #[test]
    fn small_model_cull_is_kept_only_when_requested() {
        let fixture = interior("smallcull", "SmallCull", Vec::new(), vec![reference(0x0000_0200, ACTI, 0, &[])]);
        let plan_with = |small_model_cull| {
            let context = Context { small_model_cull, ..fixture.context() };
            planned_scene(|sink| plan_interior_cell(&context, CELL, sink)).unwrap().0.small_model_cull
        };
        assert!(plan_with(true));
        assert!(!plan_with(false));
    }

    #[test]
    fn mixed_interior_plans_non_parity_with_conservative_references() {
        let linked_target = 0x0000_0206;
        let temporary = vec![
            reference(0x0000_0201, STAT, 0, &[]),
            reference(0x0000_0202, STAT, 0, &[(b"XMSP", u32s(&[0x0000_0999]))]),
            reference(0x0000_0203, STAT, 0, &[(b"XLIB", u32s(&[0]))]),
            reference(0x0000_0204, NON_GATE_DOOR, 0, &[]),
            reference(0x0000_0205, STAT, 0, &[(b"VMAD", vec![0; 6])]),
            reference(linked_target, STAT, 0, &[]),
            reference(0x0000_0207, STAT, 0, &[(b"XLKR", u32s(&[0, linked_target]))]),
            reference(0x0000_0208, STAT, REFERENCE_FLAG_INITIALLY_DISABLED, &[]),
            reference(0x0000_0209, STAT, records::RECORD_FLAG_DELETED, &[]),
            reference(0x0000_020A, STAT, 0, &[(b"XBSD", vec![0; 4])]),
            record(b"REFR", 0x0000_020B, 0, &[(b"DATA", vec![0; 24])]),
        ];
        let persistent = vec![reference(0x0000_0300, CONT, 0, &[])];
        let fixture = interior("mixed", "--", persistent, temporary);
        let (plan, scene) = planned_scene(|sink| plan_interior_cell(&fixture.context(), CELL, sink)).unwrap();
        assert_eq!(plan.scene_name, "Interior_Cell000100");
        let scene = &scene;

        assert_eq!(instance_of(scene, 0x0000_0201).unwrap().flags, INSTANCE_TARGET_OCCLUDER);
        assert_eq!(instance_of(scene, 0x0000_0202).unwrap().flags, INSTANCE_TARGET, "a material swap never occludes");
        assert_eq!(instance_of(scene, 0x0000_0203).unwrap().flags, INSTANCE_TARGET_OCCLUDER);
        assert_eq!(instance_of(scene, 0x0000_0300).unwrap().flags, INSTANCE_TARGET_OCCLUDER);
        for excluded in [0x0000_0204, 0x0000_0205, linked_target, 0x0000_0207, 0x0000_0208, 0x0000_0209, 0x0000_020A] {
            assert!(instance_of(scene, excluded).is_none(), "{excluded:08X} must be left out");
        }
        let mut ids = plan.cells[0].1.previs_reference_ids.clone();
        ids.sort();
        assert_eq!(ids, vec![0x0000_0201, 0x0000_0202, 0x0000_0203, 0x0000_0300]);
        assert_eq!(scene.volumes, vec![snapped_volume(&[[0.0; 3], [512.0; 3]])]);

        for kind in [
            "persistent children planned",
            "interior beyond the verified single-ACTI subset",
            "material-swapped reference never occludes",
            "REFR field XLIB ignored",
            "DOOR base left out of visibility",
            "CONT base included",
            "scripted reference left out of visibility",
            "linked reference left out of visibility",
            "linked-to reference left out of visibility",
            "deleted reference left out of visibility",
            "spline-shaped reference left out of visibility",
            "unreadable reference dropped",
        ] {
            assert!(has_note(&plan, kind), "missing note {kind}: {:?}", plan.non_parity);
        }
        assert!(!plan.non_parity.iter().any(|n| n.contains("disabled")), "disabled refs are CK behaviour");
    }

    #[test]
    fn temporary_doors_become_gates_unless_their_base_opts_out() {
        let temporary = vec![
            reference(0x0000_0201, STAT, 0, &[]),
            reference(0x0000_0202, DOOR, 0, &[]),
            reference(0x0000_0203, NON_GATE_DOOR, 0, &[]),
            reference(0x0000_0204, DOOR, 0, &[(b"VMAD", vec![0; 6])]),
        ];
        let persistent = vec![reference(0x0000_0300, DOOR, 0, &[])];
        let fixture = interior("gates", "Gates", persistent, temporary);
        let (plan, scene) = planned_scene(|sink| plan_interior_cell(&fixture.context(), CELL, sink)).unwrap();
        let gates: Vec<u32> = scene.instances.iter().filter(|i| i.flags == INSTANCE_GATE).map(|i| i.object_id).collect();
        assert_eq!(gates, vec![0xFE00_0202]);
        assert_eq!(scene.instances.len(), 2, "the STAT and the gate");
        assert_eq!(plan.cells[0].1.previs_reference_ids, vec![0x0000_0201]);
    }

    #[test]
    fn small_model_cull_is_dropped_only_where_the_view_reaches_past_it() {
        let volume = |x: f32, y: f32| [0.0, 0.0, 0.0, x, y, 512.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert!(!reaches_past_small_model_cull(&volume(2048.0, 2048.0)));
        assert!(reaches_past_small_model_cull(&volume(3072.0, 3072.0)));
        assert!(reaches_past_small_model_cull(&volume(10752.0, 4608.0)));
    }

    #[test]
    fn interior_without_geometry_is_skipped() {
        let fixture = interior("empty", "Empty", Vec::new(), vec![reference(0x0000_0201, DOOR, 0, &[])]);
        assert!(planned_scene(|sink| plan_interior_cell(&fixture.context(), CELL, sink)).is_err());
    }

    #[test]
    fn interior_camera_seeds_triangle_centroids_and_grows_the_navmesh_box() {
        let mut nvnm = u32s(&[15, 0, 0, CELL, 3]);
        nvnm.extend([0.0f32, 10.0, -6.0, 90.0, 20.0, 48.0, 30.0, 0.0, 0.0].iter().flat_map(|v| v.to_le_bytes()));
        nvnm.extend(1u32.to_le_bytes());
        nvnm.extend([0u16, 1, 2].iter().flat_map(|v| v.to_le_bytes()));
        nvnm.extend([0u8; 15]);
        let navmesh = record(b"NAVM", 0x0000_0900, 0, &[(b"NVNM", nvnm)]);
        let fixture = interior("region", "Region", Vec::new(), vec![reference(0x0000_0201, STAT, 0, &[]), navmesh]);
        let camera = interior_camera(fixture.context().load_order.target(), CELL).unwrap();
        assert_eq!(camera.seeds, vec![[40.0, 10.0, 14.0 + CAMERA_EYE_HEIGHT]]);
        let (region, reach) = (camera.region.unwrap(), CAMERA_REACH);
        assert_eq!(region.min, [-reach, -reach, -6.0 - reach]);
        assert_eq!(region.max, [90.0 + reach, 20.0 + reach, 48.0 + reach]);

        let bare = interior("bare", "Bare", Vec::new(), vec![reference(0x0000_0201, STAT, 0, &[])]);
        let camera = interior_camera(bare.context().load_order.target(), CELL).unwrap();
        assert!(camera.seeds.is_empty() && camera.region.is_none());
    }

    const WORLD: u32 = 0x0000_0500;

    fn exterior_cell(form_id: u32, x: i32, y: i32, extra: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut xclc = x.to_le_bytes().to_vec();
        xclc.extend_from_slice(&y.to_le_bytes());
        xclc.extend_from_slice(&[0; 4]);
        let mut fields = vec![(b"DATA", vec![0, 0]), (b"XCLC", xclc)];
        fields.extend(extra.iter().map(|(s, d)| (*s, d.clone())));
        record(b"CELL", form_id, 0, &fields)
    }

    fn land(form_id: u32) -> Vec<u8> {
        record(b"LAND", form_id, 0, &[])
    }

    fn world(name: &str, flags: u8, cells: Vec<Vec<u8>>) -> Fixture {
        world_with(name, flags, cells, Vec::new())
    }

    /// A world after the STAT base and `bases` (further top groups).
    fn world_with(name: &str, flags: u8, cells: Vec<Vec<u8>>, bases: Vec<Vec<u8>>) -> Fixture {
        let wrld = record(
            b"WRLD",
            WORLD,
            0,
            &[(b"EDID", zstr("TestWorld")), (b"DATA", vec![flags]), (b"NAM0", vec![0; 8]), (b"NAM9", vec![0; 8])],
        );
        let children = group(WORLD, 1, &[group(0, 4, &[group(0, 5, &cells)])]);
        let worlds = group(u32::from_le_bytes(*b"WRLD"), 0, &[wrld, children]);
        let mut groups = vec![base(b"STAT", STAT)];
        groups.extend(bases);
        groups.push(worlds);
        Fixture::new(name, groups)
    }

    #[test]
    fn partial_precombined_exterior_plans_with_conservative_fallbacks() {
        let root = 0x0000_0600;
        let source = 0x0000_0700;
        let direct = 0x0000_0701;
        let xcri = u32s(&[1, 2, 0x1234_5678, source, 0x1234_5678]);
        let root_children = vec![
            land(0x0000_0800),
            reference(source, STAT, 0, &[]),
            reference(direct, STAT, 0, &[]),
            record(b"NAVM", 0x0000_0900, 0, &[]),
        ];
        let cells = vec![
            exterior_cell(root, 0, 0, &[(b"XCRI", xcri)]),
            cell_children(root, Vec::new(), root_children),
            exterior_cell(0x0000_0601, 1, 0, &[]),
            cell_children(0x0000_0601, Vec::new(), vec![land(0x0000_0801)]),
            exterior_cell(0x0000_0602, 0, 1, &[]),
        ];
        let fixture = world("exterior", 0x88, cells);
        let (plan, scene) = planned_scene(|sink| plan_exterior_cluster(&fixture.context(), WORLD, 0, 0, sink)).unwrap();
        for kind in [
            "partial cluster planned",
            "CELL without LAND has no terrain surface",
            "temporary NAVM child ignored",
            "precombined CELL planned",
            "precombined geometry unavailable; source references occlude only",
            "unverified WRLD DATA flags ignored",
        ] {
            assert!(has_note(&plan, kind), "missing note {kind}: {:?}", plan.non_parity);
        }
        let scene = &scene;
        let land_targets: Vec<u32> = scene.instances.iter().take(2).map(|i| i.object_id).collect();
        assert_eq!(land_targets, vec![0x0000_0800, 0x0000_0801]);
        assert_eq!(instance_of(scene, direct).unwrap().flags, INSTANCE_TARGET_OCCLUDER);
        assert_eq!(
            instance_of(scene, source).unwrap().flags,
            INSTANCE_OCCLUDER,
            "a precombined source never becomes a target"
        );

        let (_, root_metadata) = plan.cells.iter().find(|(c, _)| *c == root).unwrap();
        assert_eq!(root_metadata.previs_reference_ids, vec![direct]);
        assert!(root_metadata.precombine, "previs must restate PCMB for a precombined CELL");
        assert_eq!(root_metadata.xcri.as_ref().unwrap().reference_mesh_pairs, vec![(source, 0x1234_5678)]);
        assert_eq!(root_metadata.root_visibility_cell, Some(root));
    }

    #[test]
    fn cluster_without_a_root_cell_is_refused() {
        let cells = vec![exterior_cell(0x0000_0601, 1, 0, &[]), cell_children(0x0000_0601, Vec::new(), vec![land(0x0000_0801)])];
        let fixture = world("rootless", 0, cells);
        let error = planned_scene(|sink| plan_exterior_cluster(&fixture.context(), WORLD, 0, 0, sink)).err().unwrap();
        assert!(error.to_string().contains("no root CELL"), "{error}");
    }

    fn placed(form_id: u32, base: u32, position: Vec3) -> Vec<u8> {
        let placement: Vec<u8> = position.iter().chain(&[0.0; 3]).flat_map(|v| v.to_le_bytes()).collect();
        record(b"REFR", form_id, 0, &[(b"NAME", base.to_le_bytes().to_vec()), (b"DATA", placement)])
    }

    #[test]
    fn neighbouring_targets_that_reach_the_cluster_are_listed_but_never_occlude() {
        let (root, neighbour) = (0x0000_0600, 0x0000_0610);
        let (reaching, distant) = (0x0000_0710, 0x0000_0711);
        // Cluster (0, 0) spans x -4096..8192; the 512-unit boxes start at x.
        let cells = vec![
            exterior_cell(root, 0, 0, &[]),
            cell_children(root, Vec::new(), vec![land(0x0000_0800)]),
            exterior_cell(neighbour, 2, 0, &[]),
            cell_children(neighbour, Vec::new(), vec![placed(reaching, STAT, [8000.0, 0.0, 0.0]), placed(distant, STAT, [8300.0, 0.0, 0.0])]),
        ];
        let fixture = world("neighbours", 0, cells);
        let (plan, scene) = planned_scene(|sink| plan_exterior_cluster(&fixture.context(), WORLD, 0, 0, sink)).unwrap();
        assert_eq!(instance_of(&scene, reaching).unwrap().flags, INSTANCE_TARGET);
        assert!(instance_of(&scene, distant).is_none());
        assert!(plan.references.iter().all(|r| r.form_id != reaching), "a neighbour's reference belongs to its own cluster");
        assert!(plan.cells.iter().all(|(cell, _)| *cell != neighbour));
    }

    #[test]
    fn persistent_references_are_planned_in_the_cell_they_stand_in() {
        let (root, holder) = (0x0000_0600, 0x0000_06FF);
        let (standing, full_lod) = (0x0000_0720, 0x0000_0721);
        let placed_with = |form_id: u32, flags: u32, x: f32| {
            let placement: Vec<u8> = [x, 100.0, 0.0, 0.0, 0.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
            record(b"REFR", form_id, flags, &[(b"NAME", STAT.to_le_bytes().to_vec()), (b"DATA", placement)])
        };
        let wrld = record(b"WRLD", WORLD, 0, &[(b"EDID", zstr("TestWorld")), (b"DATA", vec![0]), (b"NAM0", vec![0; 8]), (b"NAM9", vec![0; 8])]);
        let persistent_cell = record(b"CELL", holder, 0x400, &[(b"DATA", vec![2, 0])]);
        let children = group(WORLD, 1, &[
            persistent_cell,
            cell_children(holder, vec![placed_with(standing, 0x400, 100.0), placed_with(full_lod, 0x400 | records::REFERENCE_FLAG_FULL_LOD, 1000.0)], Vec::new()),
            group(0, 4, &[group(0, 5, &[exterior_cell(root, 0, 0, &[]), cell_children(root, Vec::new(), vec![land(0x0000_0800)])])]),
        ]);
        let fixture = Fixture::new("persistent", vec![base(b"STAT", STAT), group(u32::from_le_bytes(*b"WRLD"), 0, &[wrld, children])]);
        let (plan, scene) = planned_scene(|sink| plan_exterior_cluster(&fixture.context(), WORLD, 0, 0, sink)).unwrap();
        assert!(has_note(&plan, "persistent REFRs planned in the CELL they stand in"), "{:?}", plan.non_parity);
        assert_eq!(instance_of(&scene, standing).unwrap().flags, INSTANCE_TARGET_OCCLUDER);
        assert!(instance_of(&scene, full_lod).is_none(), "Full-LOD references stay with the world");
        let (_, metadata) = plan.cells.iter().find(|(c, _)| *c == root).unwrap();
        assert_eq!(metadata.previs_reference_ids, vec![standing]);
        assert!(plan.cells.iter().all(|(cell, _)| *cell != holder), "the world's persistent CELL is never stamped");
    }

    #[test]
    fn references_physics_moves_are_left_out_of_visibility() {
        const MOVING_MODEL: &str = "Test\\Moving.nif";
        let (still_base, moving_base) = (0x0000_0020, 0x0000_0021);
        let (root, still, moving) = (0x0000_0600, 0x0000_0720, 0x0000_0721);
        let movables = group(
            u32::from_le_bytes(*b"MSTT"),
            0,
            &[
                record(b"MSTT", still_base, 0, &[(b"MODL", zstr(BOX_MODEL))]),
                record(b"MSTT", moving_base, 0, &[(b"MODL", zstr(MOVING_MODEL))]),
            ],
        );
        let cells = vec![
            exterior_cell(root, 0, 0, &[]),
            cell_children(root, Vec::new(), vec![land(0x0000_0800), placed(still, still_base, [0.0; 3]), placed(moving, moving_base, [600.0, 0.0, 0.0])]),
        ];
        let fixture = world_with("moving", 0, cells, vec![movables]);
        let mut nif = NifFile::new("fo4");
        let flags = nif.add_block("BSXFlags", None);
        nif.blocks[flags].set_field("Integer Data", nif_core_native::model::NifValue::UInt(0x2D3));
        let path = fixture.dir.join(mesh_path(MOVING_MODEL).unwrap().replace('\\', "/"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, nif.to_bytes().unwrap()).unwrap();
        let mut models = fixture.models.models.lock().unwrap();
        let shapes = models.get(&mesh_path(BOX_MODEL).unwrap().to_ascii_lowercase()).unwrap();
        models.insert(mesh_path(MOVING_MODEL).unwrap().to_ascii_lowercase(), shapes);
        drop(models);

        let (plan, scene) = planned_scene(|sink| plan_exterior_cluster(&fixture.context(), WORLD, 0, 0, sink)).unwrap();
        assert!(instance_of(&scene, still).is_some());
        assert!(instance_of(&scene, moving).is_none(), "a reference physics moves is never culled or occluding");
        assert!(has_note(&plan, "physics-driven MSTT reference left out of visibility"), "{:?}", plan.non_parity);
        let (_, metadata) = plan.cells.iter().find(|(c, _)| *c == root).unwrap();
        assert_eq!(metadata.previs_reference_ids, vec![still]);
    }

    #[test]
    fn water_activators_are_targets_that_never_occlude() {
        let (plain_base, water_base) = (0x0000_0020, 0x0000_0021);
        let (root, plain, water) = (0x0000_0600, 0x0000_0720, 0x0000_0721);
        let activators = group(
            u32::from_le_bytes(*b"ACTI"),
            0,
            &[
                record(b"ACTI", plain_base, 0, &[(b"MODL", zstr(BOX_MODEL))]),
                record(b"ACTI", water_base, 0, &[(b"MODL", zstr(BOX_MODEL)), (b"WNAM", 0x0000_0030u32.to_le_bytes().to_vec())]),
            ],
        );
        let cells = vec![
            exterior_cell(root, 0, 0, &[]),
            cell_children(root, Vec::new(), vec![land(0x0000_0800), placed(plain, plain_base, [0.0; 3]), placed(water, water_base, [600.0, 0.0, 0.0])]),
        ];
        let fixture = world_with("water", 0, cells, vec![activators]);
        let (_, scene) = planned_scene(|sink| plan_exterior_cluster(&fixture.context(), WORLD, 0, 0, sink)).unwrap();
        assert_eq!(instance_of(&scene, plain).unwrap().flags, INSTANCE_TARGET_OCCLUDER);
        assert_eq!(instance_of(&scene, water).unwrap().flags, INSTANCE_TARGET, "a water plane never occludes");
    }

    #[test]
    fn animated_references_are_targets_that_never_occlude() {
        const ANIMATED_MODEL: &str = "Test\\Animated.nif";
        let (still_base, animated_base) = (0x0000_0020, 0x0000_0021);
        let (root, still, animated) = (0x0000_0600, 0x0000_0720, 0x0000_0721);
        let statics = group(
            u32::from_le_bytes(*b"STAT"),
            0,
            &[
                record(b"STAT", still_base, 0, &[(b"MODL", zstr(BOX_MODEL))]),
                record(b"STAT", animated_base, 0, &[(b"MODL", zstr(ANIMATED_MODEL))]),
            ],
        );
        let cells = vec![
            exterior_cell(root, 0, 0, &[]),
            cell_children(root, Vec::new(), vec![land(0x0000_0800), placed(still, still_base, [0.0; 3]), placed(animated, animated_base, [600.0, 0.0, 0.0])]),
        ];
        let fixture = world_with("animated", 0, cells, vec![statics]);
        let mut nif = NifFile::new("fo4");
        nif.add_block("NiNode", None);
        nif.add_block("NiControllerManager", None);
        let path = fixture.dir.join(mesh_path(ANIMATED_MODEL).unwrap().replace('\\', "/"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, nif.to_bytes().unwrap()).unwrap();
        let mut models = fixture.models.models.lock().unwrap();
        let shapes = models.get(&mesh_path(BOX_MODEL).unwrap().to_ascii_lowercase()).unwrap();
        models.insert(mesh_path(ANIMATED_MODEL).unwrap().to_ascii_lowercase(), shapes);
        drop(models);

        let (plan, scene) = planned_scene(|sink| plan_exterior_cluster(&fixture.context(), WORLD, 0, 0, sink)).unwrap();
        assert_eq!(instance_of(&scene, still).unwrap().flags, INSTANCE_TARGET_OCCLUDER);
        assert_eq!(instance_of(&scene, animated).unwrap().flags, INSTANCE_TARGET, "an animated model never occludes");
        assert!(has_note(&plan, "animated reference never occludes"), "{:?}", plan.non_parity);
    }

    #[test]
    fn neighbouring_group_shapes_keep_their_row_index() {
        let mut builder = SceneBuilder::default();
        let mut sink = NeighbourTargets { sink: &mut builder, footprint: [0.0, 0.0, 100.0, 100.0] };
        let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        for x in [50.0, 500.0, 95.0] {
            let geometry = Geometry { vertices: vec![[x, 0.0, 0.0], [x + 10.0, 10.0, 10.0]], triangles: Vec::new() };
            sink.combined((geometry, identity, INSTANCE_TARGET_OCCLUDER));
        }
        sink.commit_combined(false, &|index| MODEL_ID_BASE + index as u32);
        let scene = builder.finish([0.0; 12]);
        let kept: Vec<(u32, u32)> = scene.instances.iter().map(|i| (i.object_id, i.flags)).collect();
        assert_eq!(kept, vec![(MODEL_ID_BASE, INSTANCE_TARGET), (MODEL_ID_BASE + 2, INSTANCE_TARGET)]);
    }

    #[test]
    fn exterior_combined_object_ids_match_retail_tomes() {
        // Observed in the object lists of Fallout4.esm 00000FE5.uvd and 00000FEE.uvd.
        assert_eq!(exterior_combined_object_id(1, 0, 2), 0xFD08_0002);
        assert_eq!(exterior_combined_object_id(3, -1, 3), 0xFD1F_C003);
        assert_eq!(exterior_combined_object_id(-1, -1, 0x128), 0xFDFF_C128);
        assert_eq!(exterior_combined_object_id(0, 0, 7), MODEL_ID_BASE + 7);
    }

    #[test]
    fn occluder_only_drops_targets_and_target_flags() {
        let make = |object_id, flags| Instance {
            object_id,
            model_index: 0,
            transform: [0.0; 12],
            flags,
            vector: [0.0; 3],
            bounds: [0.0; 6],
        };
        let scene = Scene {
            instances: vec![make(1, INSTANCE_TARGET), make(2, INSTANCE_TARGET_OCCLUDER), make(3, INSTANCE_OCCLUDER)],
            ..Scene::default()
        };
        let kept: Vec<(u32, u32)> = occluder_only(scene).instances.iter().map(|i| (i.object_id, i.flags)).collect();
        assert_eq!(kept, vec![(2, INSTANCE_OCCLUDER), (3, INSTANCE_OCCLUDER)]);
    }
}
