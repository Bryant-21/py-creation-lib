//! Whole-plugin precombine generation: plans every interior and exterior
//! CELL, writes the group NIFs and the CELL's merged `_Physics.NIF`
//! collision, the plugin `.csg` shared geometry and its `.cdx` index, and
//! reports the CELL metadata to stamp.
//!
//! The CSG and CDX are plugin sidecars: retail keeps them loose beside the
//! plugin, so they are written next to it rather than into the archived
//! `Data` tree.
//!
//! CELLs are processed in bounded batches, in plugin order: plan in parallel,
//! place new geometry in the CSG stream (sequentially, so offsets follow
//! first appearance as in CK's PSG), then build and write in parallel. Only
//! the report and the loaded source models outlive a batch.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use nif_core_native::model::NifFile;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::assets::AssetResolver;
use crate::cdx::{self, CellLocation};
use crate::combined_physics::{
    PhysicsInstance, build_combined_physics, physics_file_name, physics_nif, physics_root_name, serialize_packfile,
};
use crate::csg::GeometryWriter;
use crate::error::{PrevisError, Result};
use crate::group::{GroupRequest, GroupSource, SourceModel, build_group_inspected};
use crate::plugin::{LoadOrder, Plugin, RecordRef, read_u32};
use crate::precombine::{
    CellPrecombine, Context, ExcludedReference, SourceCache, exterior_persistent_references, linked_references,
    plan_cell,
};
use crate::previs::cell_xcri;
use crate::records;
use crate::sha256_hex;

/// CELLs planned, placed and built together; bounds what one batch holds.
const BATCH_CELLS: usize = 1024;

#[derive(Debug, Deserialize)]
pub struct PrecombineRequest {
    pub plugin: PathBuf,
    pub master_dirs: Vec<PathBuf>,
    pub loose_roots: Vec<PathBuf>,
    pub archives: Vec<PathBuf>,
    /// Mod `Data` root receiving `Meshes\PreCombined\`.
    pub output_dir: PathBuf,
    /// Leave exterior CELLs out of the index (see `write_index`).
    #[serde(default)]
    pub interior_cdx_only: bool,
    /// Keep exterior rows in `<stem>.cdx` (see `ExteriorRows::Inline`).
    #[serde(default)]
    pub single_cdx: bool,
    /// Exactly these CELLs (raw FormIDs, overrides allowed) instead of every
    /// CELL the plugin defines. Stamps on the plugin's other CELLs are kept:
    /// a repair patch carries complete copies of override CELLs, but only
    /// the listed ones are rebuilt.
    #[serde(default)]
    pub cells: Option<Vec<u32>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct XcriEntry {
    pub mesh_ids: Vec<u32>,
    pub reference_mesh_pairs: Vec<(u32, u32)>,
}

#[derive(Debug, Serialize)]
pub struct CombinedCell {
    pub cell_form_id: u32,
    pub xcri: XcriEntry,
    pub groups: Vec<GroupOutput>,
    /// The merged collision, absent when every combined collision is cloned.
    pub physics: Option<PhysicsOutput>,
    pub excluded_references: Vec<ExcludedReference>,
    #[serde(skip)]
    pub resource_rows: Result<Vec<cdx::Row>>,
}

#[derive(Debug, Serialize)]
pub struct GroupOutput {
    pub mesh_id: u32,
    pub relative_path: String,
    pub sha256: String,
}

#[derive(Debug, Serialize)]
pub struct PhysicsOutput {
    pub relative_path: String,
    pub sha256: String,
}

/// A CELL's report entries with the bytes to write for each.
struct CellOutputs {
    groups: Vec<(GroupOutput, Vec<u8>, Result<Vec<cdx::Row>>)>,
    physics: Option<(PhysicsOutput, Vec<u8>)>,
}

#[derive(Debug, Serialize)]
pub struct CellSkip {
    pub cell_form_id: u32,
    pub unsupported: bool,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct PrecombineReport {
    pub plugin_name: String,
    pub csg_relative_path: Option<String>,
    pub csg_sha256: Option<String>,
    /// Hash of the equivalent uncompressed CK `.psg`, for oracle comparison.
    pub psg_sha256: Option<String>,
    pub cells: Vec<CombinedCell>,
    pub skipped: Vec<CellSkip>,
    #[serde(skip)]
    pub timing: GenerationTiming,
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct GenerationTiming {
    pub setup_seconds: f64,
    pub plan_seconds: f64,
    pub place_geometry_seconds: f64,
    pub build_write_seconds: f64,
    pub finish_geometry_seconds: f64,
}

#[derive(Debug, Serialize)]
/// Written after the CELLs are stamped (the index reads their XCRI).
pub struct IndexReport {
    pub cdx_relative_path: String,
    pub cdx_sha256: String,
    /// Every indexed CELL, interior and exterior, in either file.
    pub cells: usize,
    pub cells_with_visibility: usize,
    /// The exterior rows kept out of `cdx_relative_path` (see `write_index`).
    pub exterior_cdx_relative_path: Option<String>,
}

pub struct IndexResources {
    pub plugin: PathBuf,
    pub data_dir: PathBuf,
    pub cells: FxHashMap<u32, CellResources>,
}

pub struct CellResources {
    pub mesh_ids: Vec<u32>,
    pub rows: Vec<cdx::Row>,
}

/// Where `write_index` puts exterior CELL rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExteriorRows {
    /// `<stem> - Exterior.cdx`, read by Tales only.
    #[default]
    Separate,
    Omit,
    /// `<stem>.cdx` with the interiors: a repair patch's few exteriors fit
    /// the game's index.
    Inline,
}

impl ExteriorRows {
    pub fn from_flags(interior_only: bool, single_cdx: bool) -> Self {
        match (interior_only, single_cdx) {
            (true, _) => Self::Omit,
            (false, true) => Self::Inline,
            (false, false) => Self::Separate,
        }
    }
}

fn plugin_stem(plugin_name: &str) -> &str {
    Path::new(plugin_name).file_stem().and_then(|s| s.to_str()).unwrap_or(plugin_name)
}

pub fn sidecar_dir(plugin_path: &Path) -> &Path {
    plugin_path.parent().unwrap_or(Path::new("."))
}

/// Retail ships `<stem> - Geometry.csg`; group NIFs hash the same stem.
pub fn geometry_file_name(plugin_name: &str) -> String {
    format!("{} - Geometry.csg", plugin_stem(plugin_name))
}

pub fn index_file_name(plugin_name: &str) -> String {
    format!("{}.cdx", plugin_stem(plugin_name))
}

/// The engine only opens `<stem>.cdx`; Tales loads this one into a tree of
/// its own (docs/re/CDX_SHADOW_INDEX.md in the Tales mod).
pub fn exterior_index_file_name(plugin_name: &str) -> String {
    format!("{} - Exterior.cdx", plugin_stem(plugin_name))
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| PrevisError::io(parent.display().to_string(), e))?;
    }
    std::fs::write(path, bytes).map_err(|e| PrevisError::io(path.display().to_string(), e))
}

/// Own CELLs to plan, in plugin order: interiors and exterior grid CELLs (a
/// world's persistent CELL has no grid position; its REFRs are planned with
/// the grid CELL each stands in).
fn target_cells(plugin: &Plugin) -> Vec<u32> {
    let self_index = plugin.self_index();
    let grid_cells: FxHashSet<u32> = plugin.world_cells.values().flatten().copied().collect();
    plugin
        .records_of(b"CELL")
        // Only CELLs this plugin defines: an override sees just its own children.
        .filter(|cell| cell.form_id >> 24 == self_index)
        .filter(|cell| {
            grid_cells.contains(&cell.form_id)
                || (plugin.cells.get(&cell.form_id).is_none_or(|t| t.world.is_none())
                    && records::read_cell(plugin, cell).is_ok_and(|c| c.interior))
        })
        .map(|cell| cell.form_id)
        .collect()
}

fn listed_cells(plugin: &Plugin, cells: &[u32]) -> Result<Vec<u32>> {
    for &cell in cells {
        if plugin.record(cell).is_none_or(|r| &r.signature != b"CELL") {
            return Err(PrevisError::invalid(format!("CELL {cell:08X} is not in {}", plugin.name)));
        }
    }
    Ok(cells.to_vec())
}

fn skip(cell_form_id: u32, error: &PrevisError) -> CellSkip {
    CellSkip {
        cell_form_id,
        unsupported: error.is_unsupported(),
        reason: error.to_string(),
    }
}

pub fn generate(request: &PrecombineRequest) -> Result<PrecombineReport> {
    generate_with_progress(request, &|_, _| {})
}

/// `progress(done, total)` counts target CELLs after each batch is built.
pub fn generate_with_progress(request: &PrecombineRequest, progress: &(dyn Fn(usize, usize) + Sync)) -> Result<PrecombineReport> {
    let started = Instant::now();
    let load_order = LoadOrder::open(&request.plugin, &request.master_dirs)?;
    let assets = AssetResolver::new(request.loose_roots.clone(), &request.archives)?;
    let sources = SourceCache::default();
    let plugin = load_order.target();
    let linked_from = linked_references(plugin)?;
    let exterior_persistent = exterior_persistent_references(plugin)?;
    let context = Context {
        load_order: &load_order,
        assets: &assets,
        sources: &sources,
        linked_from: &linked_from,
        exterior_persistent: &exterior_persistent,
    };
    let geometry_name = geometry_file_name(&plugin.name);
    let mut report = PrecombineReport {
        plugin_name: plugin.name.clone(),
        csg_relative_path: None,
        csg_sha256: None,
        psg_sha256: None,
        cells: Vec::new(),
        skipped: Vec::new(),
        timing: GenerationTiming::default(),
    };
    let mut placer = GeometryPlacer::new(GeometryWriter::create(&sidecar_dir(&request.plugin).join(&geometry_name))?);
    let targets = match &request.cells {
        Some(cells) => listed_cells(plugin, cells)?,
        None => target_cells(plugin),
    };
    report.timing.setup_seconds = started.elapsed().as_secs_f64();
    let mut done = 0;
    for batch in targets.chunks(BATCH_CELLS) {
        let started = Instant::now();
        let outcomes: Vec<(u32, Result<Option<CellPrecombine>>)> =
            batch.par_iter().map(|&cell| (cell, plan_cell(&context, cell))).collect();
        report.timing.plan_seconds += started.elapsed().as_secs_f64();
        let started = Instant::now();
        let mut placed = Vec::with_capacity(outcomes.len());
        for (cell, outcome) in outcomes {
            match outcome {
                Ok(Some(plan)) => match placer.place(&plan)? {
                    Some(offsets) => placed.push((plan, offsets)),
                    None => report.skipped.push(skip(
                        cell,
                        &PrevisError::unsupported("its shared geometry would pass the CSG's 4 GiB offset limit"),
                    )),
                },
                Ok(None) => {}
                Err(error) => report.skipped.push(skip(cell, &error)),
            }
        }
        report.timing.place_geometry_seconds += started.elapsed().as_secs_f64();
        let started = Instant::now();
        // A CELL's NIFs are written as it is built. Planning already excluded
        // every reference whose shapes a group rejects, so a CELL failing here
        // leaves only its (unreferenced) geometry rows in the CSG.
        let built: Vec<Result<(Vec<GroupOutput>, Option<PhysicsOutput>, Result<Vec<cdx::Row>>)>> = placed
            .par_iter()
            .map(|(plan, offsets)| -> Result<Result<(Vec<GroupOutput>, Option<PhysicsOutput>, Result<Vec<cdx::Row>>)>> {
                let mesh_dir = plugin.precombined_dir(plan.cell_form_id);
                let cell = match build_cell(plan, offsets, &geometry_name, &mesh_dir) {
                    Ok(cell) => cell,
                    Err(error) => return Ok(Err(error)),
                };
                let target = |relative_path: &str| request.output_dir.join(relative_path.replace('\\', "/"));
                if !cell.groups.is_empty() || cell.physics.is_some() {
                    let directory = request.output_dir.join(mesh_dir.replace('\\', "/"));
                    std::fs::create_dir_all(&directory).map_err(|e| PrevisError::io(directory.display().to_string(), e))?;
                }
                let write_output = |relative_path: &str, bytes: &[u8]| {
                    let path = target(relative_path);
                    std::fs::write(&path, bytes).map_err(|e| PrevisError::io(path.display().to_string(), e))
                };
                let mut groups = Vec::with_capacity(cell.groups.len());
                let mut resource_rows = Ok(Vec::new());
                for (output, bytes, rows) in cell.groups {
                    write_output(&output.relative_path, &bytes)?;
                    groups.push(output);
                    if let Ok(collected) = &mut resource_rows {
                        match rows {
                            Ok(rows) => collected.extend(rows),
                            Err(error) => resource_rows = Err(error),
                        }
                    }
                }
                if let Ok(rows) = &mut resource_rows {
                    rows.sort_unstable();
                    rows.dedup();
                    rows.shrink_to_fit();
                }
                let physics = match cell.physics {
                    Some((output, bytes)) => {
                        write_output(&output.relative_path, &bytes)?;
                        Some(output)
                    }
                    None => None,
                };
                Ok(Ok((groups, physics, resource_rows)))
            })
            .collect::<Result<_>>()?;
        for ((plan, _), outcome) in placed.into_iter().zip(built) {
            match outcome {
                Ok((groups, physics, resource_rows)) => report.cells.push(CombinedCell {
                    cell_form_id: plan.cell_form_id,
                    xcri: XcriEntry {
                        mesh_ids: plan.xcri.mesh_ids,
                        reference_mesh_pairs: plan.xcri.reference_mesh_pairs,
                    },
                    groups,
                    physics,
                    excluded_references: plan.excluded,
                    resource_rows,
                }),
                Err(error) => report.skipped.push(skip(plan.cell_form_id, &error)),
            }
        }
        done += batch.len();
        report.timing.build_write_seconds += started.elapsed().as_secs_f64();
        progress(done, targets.len());
    }
    if report.cells.is_empty() {
        placer.writer.abandon();
        return Ok(report);
    }
    let started = Instant::now();
    let written = placer.writer.finish()?;
    report.timing.finish_geometry_seconds = started.elapsed().as_secs_f64();
    report.csg_relative_path = Some(geometry_name);
    report.csg_sha256 = Some(written.csg_sha256);
    report.psg_sha256 = Some(written.psg_sha256);
    Ok(report)
}

/// Rebuilds the `<stem>.cdx` sidecar from the plugin's XCRI-stamped CELLs
/// whose group NIFs are under `data_dir`. Previs reruns it so CELLs index
/// their UVDs.
/// A CELL with none of its groups present is not ours and is left out.
///
/// Exterior rows go to `<stem> - Exterior.cdx`, which Tales loads into a tree
/// of its own; `interior_only` leaves them out. FO4 loads every plugin's CDX
/// rows into one B-tree whose insert keeps a three-level path (about 7.4M
/// rows); retail fills 2.3M and Appalachia's exteriors need 4.7M more, which
/// crashes the game at startup. A CELL without rows still loads its
/// `_OC.NIF`s and their CSG geometry.
pub fn write_index(plugin_path: &Path, data_dir: &Path, exteriors: ExteriorRows) -> Result<Option<IndexReport>> {
    write_index_with_resources(plugin_path, data_dir, exteriors, None)
}

pub fn write_index_with_resources(plugin_path: &Path, data_dir: &Path, exteriors: ExteriorRows, resources: Option<&IndexResources>) -> Result<Option<IndexReport>> {
    let resources = resources.filter(|r| r.plugin == plugin_path && r.data_dir == data_dir);
    let plugin = Plugin::open(plugin_path)?;
    // Override CELLs too: a repair patch indexes the CELLs it rebuilt for
    // their defining files. A CELL whose groups are not under `data_dir` is
    // someone else's and stays out.
    let cells: Vec<_> = plugin.records_of(b"CELL").collect();
    let indexed: Vec<Option<IndexedCell>> = cells
        .par_iter()
        .map(|cell| index_cell(&plugin, cell, data_dir, resources.and_then(|r| r.cells.get(&cell.form_id))))
        .collect::<Result<_>>()?;
    let mut rows = Vec::new();
    let mut exterior_rows = Vec::new();
    let mut cells = 0;
    let mut cells_with_visibility = 0;
    for cell in indexed.into_iter().flatten() {
        match (cell.interior, exteriors) {
            (true, _) | (false, ExteriorRows::Inline) => rows.extend(cell.rows),
            (false, ExteriorRows::Omit) => continue,
            (false, ExteriorRows::Separate) => exterior_rows.extend(cell.rows),
        }
        cells += 1;
        cells_with_visibility += cell.has_visibility as usize;
    }
    let relative = index_file_name(&plugin.name);
    let path = sidecar_dir(plugin_path).join(&relative);
    let exterior_relative = exterior_index_file_name(&plugin.name);
    let exterior_path = sidecar_dir(plugin_path).join(&exterior_relative);
    if cells == 0 || exterior_rows.is_empty() {
        remove_if_present(&exterior_path)?;
    }
    if cells == 0 {
        remove_if_present(&path)?;
        return Ok(None);
    }
    let exterior_cdx_relative_path = if exterior_rows.is_empty() {
        None
    } else {
        write(&exterior_path, &cdx::encode(exterior_rows))?;
        Some(exterior_relative)
    };
    let bytes = cdx::encode(rows);
    write(&path, &bytes)?;
    Ok(Some(IndexReport {
        cdx_relative_path: relative,
        cdx_sha256: sha256_hex(&bytes),
        cells,
        cells_with_visibility,
        exterior_cdx_relative_path,
    }))
}

fn remove_if_present(path: &Path) -> Result<()> {
    if path.is_file() {
        std::fs::remove_file(path).map_err(|e| PrevisError::io(path.display().to_string(), e))?;
    }
    Ok(())
}

struct IndexedCell {
    rows: Vec<cdx::Row>,
    has_visibility: bool,
    interior: bool,
}

/// One stamped CELL's CDX rows and whether a UVD covers it; `None` when it
/// has no XCRI or none of its groups are under `data_dir`.
fn index_cell(plugin: &Plugin, cell: &RecordRef, data_dir: &Path, resources: Option<&CellResources>) -> Result<Option<IndexedCell>> {
    let Some(xcri) = cell_xcri(plugin, cell.form_id)? else {
        return Ok(None);
    };
    let details = records::read_cell(plugin, cell)?;
    let mesh_dir = data_dir.join(plugin.precombined_dir(cell.form_id).replace('\\', "/"));
    let local = cell.form_id & 0x00FF_FFFF;
    let paths: Vec<PathBuf> = xcri.mesh_ids.iter().map(|id| mesh_dir.join(format!("{local:08X}_{id:08X}_OC.NIF"))).collect();
    let present = paths.iter().filter(|path| path.is_file()).count();
    if present == 0 {
        return Ok(None);
    }
    if present != paths.len() {
        return Err(PrevisError::invalid(format!(
            "CELL {:08X} has {present} of {} precombined groups",
            cell.form_id,
            paths.len()
        )));
    }
    // One group NIF at a time: a CELL can have hundreds.
    let groups = xcri.mesh_ids.iter().zip(&paths).map(|(&mesh_id, path)| {
        let bytes = std::fs::read(path).map_err(|e| PrevisError::io(path.display().to_string(), e))?;
        let nif = NifFile::from_bytes_raw_arrays(&bytes, None)
            .map_err(|e| PrevisError::invalid(format!("{}: NIF parse failed: {e}", path.display())))?;
        Ok((mesh_id, nif))
    });
    // An exterior CELL shares its cluster root's UVD, which its RVIS names.
    let world = plugin.cells.get(&cell.form_id).and_then(|t| t.world);
    let (location, visibility_cell) = match (details.interior, details.grid, world) {
        (false, Some(grid), Some(world)) => (
            CellLocation::Exterior { world, grid },
            plugin.subrecords_at(cell)?.first(b"RVIS").and_then(|d| read_u32(d, 0)),
        ),
        _ => (CellLocation::Interior, Some(cell.form_id)),
    };
    // The UVD sits in the folder of the file defining the root, which for an
    // exterior can differ from the CELL's own.
    let visibility = visibility_cell
        .filter(|root| data_dir.join("Vis").join(plugin.cell_owner(*root)).join(format!("{:08X}.uvd", root & 0x00FF_FFFF)).is_file());
    let vis_owner = plugin.cell_owner(visibility.unwrap_or(cell.form_id));
    let rows = match resources.filter(|r| r.mesh_ids == xcri.mesh_ids) {
        Some(resources) => {
            let mut rows = resources.rows.clone();
            cdx::locate_rows(&mut rows, cell.form_id, location);
            rows.push(cdx::visibility_row(vis_owner, cell.form_id, location, visibility));
            rows
        }
        None => cdx::cell_rows(vis_owner, cell.form_id, location, groups, visibility)?,
    };
    Ok(Some(IndexedCell { rows, has_visibility: visibility.is_some(), interior: details.interior }))
}

/// Assigns each planned shape its CSG data offset, appending buffers first
/// seen to the stream. Identical buffers share one offset, as in CK's PSG.
struct GeometryPlacer {
    writer: GeometryWriter,
    /// Each model's shape offsets by model address, so a model is hashed
    /// once. The map holds the model: a freed one's address can name another
    /// model later (concurrent cache misses load duplicates that die with
    /// their batch).
    by_model: FxHashMap<usize, (Arc<SourceModel>, Vec<u32>)>,
    by_content: FxHashMap<[u8; 32], u32>,
}

impl GeometryPlacer {
    fn new(writer: GeometryWriter) -> Self {
        Self {
            writer,
            by_model: FxHashMap::default(),
            by_content: FxHashMap::default(),
        }
    }

    /// Offsets per source per retained shape, or `None` (placing nothing)
    /// when the CELL's new geometry would pass the u32 offsets.
    fn place(&mut self, plan: &CellPrecombine) -> Result<Option<Vec<Vec<u32>>>> {
        use sha2::{Digest, Sha256};
        let address = |model: &Arc<SourceModel>| Arc::as_ptr(model) as usize;
        let mut next = self.writer.data_bytes();
        let mut new_models: FxHashMap<usize, (Arc<SourceModel>, Vec<u64>)> = FxHashMap::default();
        let mut new_content: FxHashMap<[u8; 32], u64> = FxHashMap::default();
        let mut appended = Vec::new();
        for source in &plan.sources {
            let key = address(&source.model);
            if self.by_model.contains_key(&key) || new_models.contains_key(&key) {
                continue;
            }
            let mut offsets = Vec::with_capacity(source.model.shapes.len());
            for shape in &source.model.shapes {
                let digest: [u8; 32] = Sha256::digest(&shape.geometry).into();
                offsets.push(match self.by_content.get(&digest) {
                    Some(&offset) => offset as u64,
                    None => *new_content.entry(digest).or_insert_with(|| {
                        appended.push((shape, digest));
                        next += shape.geometry.len() as u64;
                        next - shape.geometry.len() as u64
                    }),
                });
            }
            new_models.insert(key, (source.model.clone(), offsets));
        }
        if next > u32::MAX as u64 {
            return Ok(None);
        }
        for (shape, digest) in appended {
            let offset = self.writer.push(shape.vertex_desc, shape.vertex_count, shape.triangle_count, &shape.geometry)?;
            if new_content.get(&digest) != Some(&(offset as u64)) {
                return Err(PrevisError::invalid("shared geometry offsets diverged from their placement"));
            }
            self.by_content.insert(digest, offset);
        }
        for (key, (model, offsets)) in new_models {
            self.by_model.insert(key, (model, offsets.into_iter().map(|o| o as u32).collect()));
        }
        Ok(Some(plan.sources.iter().map(|source| self.by_model[&address(&source.model)].1.clone()).collect()))
    }
}

fn build_cell(plan: &CellPrecombine, offsets: &[Vec<u32>], geometry_name: &str, mesh_dir: &str) -> Result<CellOutputs> {
    let context = format!("CELL {:08X}", plan.cell_form_id);
    Ok(CellOutputs {
        groups: build_cell_groups(plan, offsets, geometry_name, mesh_dir)?,
        physics: build_cell_physics(plan, mesh_dir).map_err(|e| e.with_context(&context))?,
    })
}

/// The CELL's `_Physics.NIF`: every merged collision placed by each
/// reference, in CK's body order: group by group as the group NIFs are
/// written (CELL 1294F4's material table follows only this order).
pub fn build_cell_physics(plan: &CellPrecombine, mesh_dir: &str) -> Result<Option<(PhysicsOutput, Vec<u8>)>> {
    let mut instances = Vec::new();
    for member in plan.groups.iter().flat_map(|g| &g.members) {
        for source in &plan.sources[plan.bases[member.base].sources.clone()] {
            let Some(collision) = &source.model.collision else { continue };
            let Some(merged) = collision.merged.as_ref().filter(|_| source.merges_collision) else { continue };
            for placement in &member.instances {
                let placed = match &source.component {
                    Some(component) => placement.compose(component),
                    None => *placement,
                };
                for (node, bodies) in merged {
                    instances.push(PhysicsInstance {
                        source: bodies.clone(),
                        transform: placed.compose(&collision.nodes[*node].local),
                        reference_scale: placement.scale,
                        component_scale: source.component.map_or(1.0, |c| c.scale),
                    });
                }
            }
        }
    }
    if instances.is_empty() {
        return Ok(None);
    }
    let physics = build_combined_physics(&instances)?;
    // CK 1.11.240 and retail write pointer-8 packfiles.
    let packfile = serialize_packfile(&physics.mesh, 8)?;
    let bytes = physics_nif(&packfile, &physics_root_name(plan.cell_form_id), physics.root_translation);
    let output = PhysicsOutput {
        relative_path: format!("{mesh_dir}\\{}", physics_file_name(plan.cell_form_id)),
        sha256: sha256_hex(&bytes),
    };
    Ok(Some((output, bytes)))
}

fn build_cell_groups(
    plan: &CellPrecombine,
    offsets: &[Vec<u32>],
    geometry_name: &str,
    mesh_dir: &str,
) -> Result<Vec<(GroupOutput, Vec<u8>, Result<Vec<cdx::Row>>)>> {
    let mut groups = Vec::with_capacity(plan.groups.len());
    for group in &plan.groups {
        let mut sources = Vec::new();
        for member in &group.members {
            for index in plan.bases[member.base].sources.clone() {
                let source = &plan.sources[index];
                sources.push(GroupSource {
                    model: &source.model,
                    instances: match &source.component {
                        Some(component) => member.instances.iter().map(|i| i.compose(component)).collect(),
                        None => member.instances.clone(),
                    },
                    base_record_flags: source.base_record_flags,
                    leaf: source.leaf,
                    editor_id: &source.editor_id,
                    data_offsets: offsets[index].clone(),
                    merges_collision: source.merges_collision,
                });
            }
        }
        // The export bytes are CK's uninitialised pointer residue; the first
        // member's base type decides which of the two observed values to use.
        let export_info = plan.bases[group.members[0].base].export_info;
        let (bytes, resource_rows) = build_group_inspected(&GroupRequest {
            root_name: group.root_name.clone(),
            sources,
            psg_file_name: geometry_name.to_string(),
            export_info,
        }, |nif| cdx::group_rows(plan.cell_form_id, group.mesh_id, nif))
        .map_err(|e| e.with_context(&format!("CELL {:08X}", plan.cell_form_id)))?;
        groups.push((
            GroupOutput {
                mesh_id: group.mesh_id,
                relative_path: format!("{mesh_dir}\\{}.NIF", group.root_name),
                sha256: sha256_hex(&bytes),
            },
            bytes,
            resource_rows,
        ));
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interiors_and_exterior_grid_cells_are_planned_but_not_persistent_cells() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = crate::precombine::tests::world_fixture(dir.path());
        assert_eq!(target_cells(&plugin), [0x800, 0x910, 0x911]);
    }

    #[test]
    fn retained_resource_rows_match_reparsed_indexes_and_missing_file_errors() {
        use crate::metadata::{CellMetadata, Xcri, write_patched_plugin};
        use nif_core_native::model::NifValue;

        let dir = tempfile::tempdir().unwrap();
        let source = crate::precombine::tests::world_fixture(dir.path());
        let plugin = dir.path().join("Indexed.esp");
        let data = dir.path().join("data");
        let metadata = [0x800, 0x910, 0x911].into_iter().map(|cell| (cell, CellMetadata {
            precombine: true, previs: cell != 0x911, root_visibility_cell: (cell != 0x911).then_some(if cell == 0x910 { 0x911 } else { cell }),
            xcri: Some(Xcri { mesh_ids: vec![7, 9], reference_mesh_pairs: Vec::new() }), ..Default::default()
        })).collect();
        write_patched_plugin(&source, &plugin, &metadata, 1).unwrap();
        let meshes = data.join("Meshes/PreCombined/Indexed.esp");
        let vis = data.join("Vis/Indexed.esp");
        std::fs::create_dir_all(&meshes).unwrap();
        std::fs::create_dir_all(&vis).unwrap();
        std::fs::write(vis.join("00000800.uvd"), []).unwrap();
        std::fs::write(vis.join("00000911.uvd"), []).unwrap();
        let mut resources = IndexResources { plugin: plugin.clone(), data_dir: data.clone(), cells: FxHashMap::default() };
        for cell in [0x800, 0x910, 0x911] {
            let mut rows = Vec::new();
            for mesh in [7, 9] {
                let mut nif = NifFile::new("fo4");
                let shader = nif.add_block("BSEffectShaderProperty", None);
                for (field, value) in [("Name", "Materials/Effects/Glow.BGEM"), ("Source Texture", "Textures/Glow_d.dds")] {
                    crate::shader::set_bare(&mut nif.blocks[shader].fields, field, NifValue::String(value.into()));
                }
                let lighting = nif.add_block("BSLightingShaderProperty", None);
                let textures = nif.add_block("BSShaderTextureSet", None);
                nif.blocks[lighting].fields.insert("Name".into(), NifValue::String("Materials/Wall.BGSM".into()));
                nif.blocks[lighting].fields.insert("Texture Set".into(), NifValue::Ref(textures as i32));
                nif.blocks[textures].fields.insert("Num Textures".into(), NifValue::UInt(8));
                nif.blocks[textures].fields.insert("Textures".into(), NifValue::Array((0..8).map(|slot| NifValue::String(format!("Textures/Wall_{slot}.dds"))).collect()));
                rows.extend(cdx::group_rows(cell, mesh, &nif).unwrap());
                std::fs::write(meshes.join(format!("{cell:08X}_{mesh:08X}_OC.NIF")), nif.to_bytes().unwrap()).unwrap();
            }
            resources.cells.insert(cell, CellResources { mesh_ids: vec![7, 9], rows });
        }
        for exteriors in [ExteriorRows::Separate, ExteriorRows::Omit] {
            let expected = write_index(&plugin, &data, exteriors).unwrap();
            let interior = std::fs::read(dir.path().join("Indexed.cdx")).unwrap();
            let exterior = std::fs::read(dir.path().join("Indexed - Exterior.cdx")).ok();
            let actual = write_index_with_resources(&plugin, &data, exteriors, Some(&resources)).unwrap();
            assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(expected).unwrap());
            assert_eq!(std::fs::read(dir.path().join("Indexed.cdx")).unwrap(), interior);
            assert_eq!(std::fs::read(dir.path().join("Indexed - Exterior.cdx")).ok(), exterior);
        }
        std::fs::remove_file(vis.join("00000800.uvd")).unwrap();
        let expected = write_index(&plugin, &data, ExteriorRows::Separate).unwrap().unwrap();
        let actual = write_index_with_resources(&plugin, &data, ExteriorRows::Separate, Some(&resources)).unwrap().unwrap();
        assert_eq!(actual.cdx_sha256, expected.cdx_sha256);
        assert_eq!(actual.cells_with_visibility, 1);
        std::fs::write(vis.join("00000800.uvd"), []).unwrap();
        let missing = meshes.join("00000910_00000007_OC.NIF");
        std::fs::remove_file(missing).unwrap();
        let expected = write_index(&plugin, &data, ExteriorRows::Separate).unwrap_err().to_string();
        assert_eq!(write_index_with_resources(&plugin, &data, ExteriorRows::Separate, Some(&resources)).unwrap_err().to_string(), expected);
        std::fs::remove_file(meshes.join("00000910_00000009_OC.NIF")).unwrap();
        let expected = write_index(&plugin, &data, ExteriorRows::Separate).unwrap().unwrap();
        let actual = write_index_with_resources(&plugin, &data, ExteriorRows::Separate, Some(&resources)).unwrap().unwrap();
        assert_eq!(actual.cdx_sha256, expected.cdx_sha256);
        assert_eq!(actual.cells, expected.cells);

        resources.cells.get_mut(&0x800).unwrap().mesh_ids = vec![42];
        std::fs::write(meshes.join("00000800_00000007_OC.NIF"), NifFile::new("fo4").to_bytes().unwrap()).unwrap();
        std::fs::write(meshes.join("00000800_00000009_OC.NIF"), NifFile::new("fo4").to_bytes().unwrap()).unwrap();
        let changed = write_index(&plugin, &data, ExteriorRows::Separate).unwrap().unwrap().cdx_sha256;
        assert_ne!(changed, expected.cdx_sha256);
        assert_eq!(write_index_with_resources(&plugin, &data, ExteriorRows::Separate, Some(&resources)).unwrap().unwrap().cdx_sha256, changed);
        resources.cells.get_mut(&0x800).unwrap().mesh_ids = vec![7, 9];
        resources.plugin = dir.path().join("Other.esp");
        assert_eq!(write_index_with_resources(&plugin, &data, ExteriorRows::Separate, Some(&resources)).unwrap().unwrap().cdx_sha256, changed);
        resources.plugin = plugin.clone();
        resources.data_dir = dir.path().join("other-data");
        assert_eq!(write_index_with_resources(&plugin, &data, ExteriorRows::Separate, Some(&resources)).unwrap().unwrap().cdx_sha256, changed);
    }
}
