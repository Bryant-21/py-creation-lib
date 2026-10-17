//! Finds CELLs whose precombines or previs a load order has broken, outdated
//! or disabled, and says why, so a patch can rebuild exactly those.
//!
//! The rules follow how the runtime treats previsbines: overrides with the
//! partial flag are ignored; a record whose version-control date is 0 or newer
//! than PCMB makes the game drop the CELL's precalculated data when it loads
//! from the same file as the CELL (see `metadata::date_version_control`);
//! precombined meshes and tomes belong to whichever plugin's archive provides
//! them; previs covers 3x3 exterior clusters.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::asset_index::{AssetIndex, DataSource};
use crate::error::Result;
use crate::load_order::{
    CellPlacement, Entry, GlobalId, LoadOrderIndex, defining_plugin, global_id, is_partial, object_id, version_date,
};
use crate::metadata::Xcri;
use crate::plugin::{Plugin, RecordRef, Subrecords, read_i32, read_u32, zstring};
use crate::previs::decode_xcri;
use crate::records::{BASE_FLAG_IS_MARKER, RECORD_FLAG_DELETED};

const CELL_FLAG_INTERIOR: u16 = 0x0001;
const CELL_FLAG_NO_PREVIS: u16 = 0x0080;
const REFERENCE_FLAG_INITIALLY_DISABLED: u32 = 0x0000_0800;
/// Fallout4.esm's `001C26C2`: CK's placeholder for an otherwise empty XPRI.
pub(crate) const DUMMY_XPRI_OBJECT: u32 = 0x001C_26C2;
/// Fallout4.esm IDs at or below this are engine markers, never precombined.
const LAST_ENGINE_MARKER: u32 = 0xC4;
/// Fallout4.esm `MultirefLOD`: the one linked-reference keyword precombines keep.
const MULTIREF_LOD_KEYWORD: u32 = 0x0019_5411;
const STAT_BLOCKING_FIELDS: [&[u8; 4]; 4] = [b"XESP", b"XATR", b"XEMI", b"XLRT"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Raw,
    /// A single FormID, compared by load-order identity.
    Form,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Any,
    Interior,
    Exterior,
}

/// CELL configuration that mods edit and overrides commonly revert
/// (location, music, acoustics, image space, water, lighting, ...).
const CELL_FIELDS: [(&[u8; 4], FieldKind, Scope); 15] = [
    (b"EDID", FieldKind::Raw, Scope::Any),
    (b"XLCN", FieldKind::Form, Scope::Any),
    (b"XCMO", FieldKind::Form, Scope::Any),
    (b"XCAS", FieldKind::Form, Scope::Any),
    (b"XCIM", FieldKind::Form, Scope::Any),
    (b"XCWT", FieldKind::Form, Scope::Any),
    (b"XWCU", FieldKind::Raw, Scope::Any),
    (b"XCLW", FieldKind::Raw, Scope::Any),
    (b"XEZN", FieldKind::Form, Scope::Any),
    (b"XCLL", FieldKind::Raw, Scope::Interior),
    (b"LTMP", FieldKind::Form, Scope::Interior),
    (b"XGDR", FieldKind::Form, Scope::Interior),
    (b"XCCM", FieldKind::Form, Scope::Interior),
    (b"XWEM", FieldKind::Raw, Scope::Interior),
    (b"MHDT", FieldKind::Raw, Scope::Exterior),
];

/// A CELL field the winning copy reverts; `best` indexes the copy whose
/// value should win.
pub(crate) struct FieldRegression {
    pub signature: [u8; 4],
    pub best: usize,
    /// The value is one FormID, not raw bytes.
    pub form: bool,
}

/// Regressed fields across `copies` (a CELL's non-partial copies in load
/// order, the winner last).
pub(crate) fn field_regressions(index: &LoadOrderIndex, base: &[bool], fixer: &[bool], copies: &[Entry], interior: bool) -> Vec<FieldRegression> {
    let mut regressions = Vec::new();
    if copies.len() < 2 {
        return regressions;
    }
    let raw: Vec<Option<Subrecords>> = copies.iter().map(|e| index.plugins[e.plugin].subrecords_at(&e.record).ok()).collect();
    let winner = copies.len() - 1;
    for &(signature, kind, scope) in &CELL_FIELDS {
        if (scope == Scope::Interior && !interior) || (scope == Scope::Exterior && interior) {
            continue;
        }
        let values: Vec<Option<Vec<u8>>> = copies
            .iter()
            .zip(&raw)
            .map(|(entry, subrecords)| {
                let data = subrecords.as_ref()?.first(signature)?;
                Some(match kind {
                    FieldKind::Raw => data.to_vec(),
                    FieldKind::Form => index.resolve(entry.plugin, read_u32(data, 0)?).to_le_bytes().to_vec(),
                })
            })
            .collect();
        let Some(best) = best_override(base, fixer, copies, &values) else { continue };
        if best != winner && values[best] != values[winner] {
            regressions.push(FieldRegression { signature: *signature, best, form: kind == FieldKind::Form });
        }
    }
    regressions
}

/// PJM's "last override unlike base": mods beat bug-fix plugins, which
/// beat Bethesda's; a mod copy equal to the master or to the last
/// Bethesda value (or missing the field) changes nothing.
fn best_override(base: &[bool], fixer: &[bool], copies: &[Entry], values: &[Option<Vec<u8>>]) -> Option<usize> {
    let master = values.first()?;
    let mut last_base = master;
    let mut best: Option<(usize, bool)> = None;
    for (index, (entry, value)) in copies.iter().zip(values).enumerate().skip(1) {
        if base[entry.plugin] {
            last_base = value;
        } else if fixer[entry.plugin] {
            if value != master && !best.is_some_and(|(_, by_mod)| by_mod) {
                best = Some((index, false));
            }
        } else if value.is_some() && value != master && value != last_base {
            best = Some((index, true));
        }
    }
    best.map(|(index, _)| index)
}

/// Regions (XCLR) a mod copy added that the winner drops, per copy.
pub(crate) fn lost_regions(index: &LoadOrderIndex, base: &[bool], copies: &[Entry]) -> Vec<(usize, Vec<GlobalId>)> {
    let regions = |copy: usize| -> BTreeSet<GlobalId> {
        let entry = &copies[copy];
        index.plugins[entry.plugin]
            .subrecords_at(&entry.record)
            .ok()
            .and_then(|s| s.first(b"XCLR").map(|data| data.chunks_exact(4).map(|c| index.resolve(entry.plugin, u32::from_le_bytes(c.try_into().unwrap()))).collect()))
            .unwrap_or_default()
    };
    let mut lost_by_copy = Vec::new();
    let Some(winner) = copies.len().checked_sub(1) else { return lost_by_copy };
    let kept = regions(winner);
    for copy in 1..winner {
        if base[copies[copy].plugin] {
            continue;
        }
        let lost: Vec<GlobalId> = regions(copy).difference(&kept).copied().collect();
        if lost.is_empty() || lost.iter().all(|r| regions(0).contains(r)) {
            continue;
        }
        lost_by_copy.push((copy, lost));
    }
    lost_by_copy
}

#[derive(Clone, Debug, Deserialize)]
pub struct CheckRequest {
    /// Plugin paths in load order.
    pub plugins: Vec<PathBuf>,
    /// Data sources, lowest priority first.
    #[serde(default)]
    pub data_sources: Vec<DataSource>,
    /// Bethesda plugins (game, DLC, Creation Club), by file name.
    #[serde(default)]
    pub base_plugins: Vec<String>,
    /// Bug-fix plugins whose CELL edits beat Bethesda's but not other mods'.
    #[serde(default = "default_fixer_plugins")]
    pub fixer_plugins: Vec<String>,
    #[serde(default)]
    pub options: CheckOptions,
}

pub(crate) fn default_fixer_plugins() -> Vec<String> {
    vec!["Unofficial Fallout 4 Patch.esp".to_string()]
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct CheckOptions {
    /// Drop findings where both the CELL's winner and the blamed plugin are
    /// Bethesda plugins (vanilla's own inconsistencies).
    pub ignore_base_only: bool,
    /// Treat disabled previs/precombines as broken instead of intentional.
    pub restore_disabled: bool,
    /// Rebuild precombined CELLs holding at least this many precombinable
    /// references that mods added or changed outside the precombines.
    pub unprecombined_threshold: Option<usize>,
    /// Rebuild every CELL this plugin (by file name) defines, overrides or
    /// places references in, broken or not.
    pub rebuild_plugin: Option<String>,
    /// Rebuild every CELL of the load order, broken or not (a full base such
    /// as a previs framework ESM built over the game, its DLC and a fix patch).
    pub rebuild_all: bool,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self { ignore_base_only: true, restore_disabled: false, unprecombined_threshold: None, rebuild_plugin: None, rebuild_all: false }
    }
}

/// What a patch must do; ordered by severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Info,
    Warn,
    /// Forward the CELL (or an empty XCRI / placeholder XPRI) without rebuilding.
    Fix,
    /// Regenerate the cluster's previs only.
    RebuildPrevis,
    /// Regenerate the CELL's precombines (and so its cluster's previs).
    RebuildPrecombines,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub action: Action,
    /// Stable machine-readable rule name.
    pub code: &'static str,
    pub message: String,
    /// Plugin whose record caused it.
    pub blamed: Option<String>,
    /// FormKey (`ObjectID:Plugin`) of the record involved.
    pub record: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CellReport {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub world: Option<String>,
    pub grid: Option<(i32, i32)>,
    pub interior: bool,
    pub winning_plugin: String,
    /// A non-Bethesda plugin overrides the CELL or changes its children.
    pub overridden_by_mod: bool,
    /// FormKey of the previs cluster root (the CELL itself for interiors).
    pub cluster: Option<String>,
    pub action: Option<Action>,
    pub findings: Vec<Finding>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClusterReport {
    pub form_key: String,
    pub world: Option<String>,
    pub grid: Option<(i32, i32)>,
    pub members: Vec<String>,
    pub previs_owner: Option<String>,
    pub action: Option<Action>,
    pub findings: Vec<Finding>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CheckSummary {
    pub plugins: usize,
    pub cells: usize,
    pub skipped_esl_origin: usize,
    pub overridden_by_mod: usize,
    pub actions: Vec<(Action, usize)>,
    pub clusters_to_rebuild: usize,
    /// Loose previsbine files hide which plugin owns them.
    pub loose_previsbines: bool,
    pub seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckReport {
    pub summary: CheckSummary,
    /// CELLs with findings or mod overrides.
    pub cells: Vec<CellReport>,
    pub clusters: Vec<ClusterReport>,
}

/// The previsbine-relevant state of one copy of a CELL.
#[derive(Clone, Debug, Default)]
struct Stamps {
    flags: u16,
    editor_id: Option<String>,
    grid: Option<(i32, i32)>,
    pcmb: Option<u16>,
    visi: Option<u16>,
    xcri: Option<Xcri>,
    xcri_malformed: bool,
    xpri: Option<Vec<u32>>,
    rvis: Option<u32>,
}

impl Stamps {
    fn previs_enabled(&self) -> bool {
        self.visi.is_some_and(|d| d != 0) && self.flags & CELL_FLAG_NO_PREVIS == 0
    }

    fn precombined(&self) -> bool {
        self.pcmb.is_some_and(|d| d != 0) && self.xcri.as_ref().is_some_and(|x| !x.mesh_ids.is_empty())
    }
}

fn read_stamps(plugin: &Plugin, record: &RecordRef) -> Stamps {
    let Ok(subrecords) = plugin.subrecords_at(record) else { return Stamps::default() };
    let mut stamps = Stamps::default();
    for (signature, data) in subrecords.iter() {
        match &signature {
            b"EDID" => stamps.editor_id = Some(zstring(data)),
            b"DATA" => stamps.flags = match data.len() {
                0 => 0,
                1 => data[0] as u16,
                _ => u16::from_le_bytes([data[0], data[1]]),
            },
            b"XCLC" if data.len() >= 8 => stamps.grid = Some((read_i32(data, 0).unwrap(), read_i32(data, 4).unwrap())),
            b"PCMB" => stamps.pcmb = data.get(..2).map(|d| u16::from_le_bytes([d[0], d[1]])),
            b"VISI" => stamps.visi = data.get(..2).map(|d| u16::from_le_bytes([d[0], d[1]])),
            b"RVIS" => stamps.rvis = read_u32(data, 0),
            b"XPRI" => stamps.xpri = Some(data.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()),
            b"XCRI" => match decode_xcri(data) {
                Some(xcri) => stamps.xcri = Some(xcri),
                None => stamps.xcri_malformed = true,
            },
            _ => {}
        }
    }
    stamps
}

struct Checker<'a> {
    index: &'a LoadOrderIndex,
    assets: &'a AssetIndex,
    options: &'a CheckOptions,
    base: Vec<bool>,
    fixer: Vec<bool>,
    /// `(world, x, y)` to exterior CELL.
    grid: FxHashMap<(GlobalId, i32, i32), GlobalId>,
}

/// One CELL's evaluation, before cluster checks.
struct CellState {
    id: GlobalId,
    placement: CellPlacement,
    winner: Entry,
    stamps: Stamps,
    /// Non-partial copies in load order with their stamps.
    copies: Vec<(Entry, Stamps)>,
    precombine_owner: Option<usize>,
    findings: Vec<Finding>,
    overridden_by_mod: bool,
}

pub fn check(request: &CheckRequest, progress: &(dyn Fn(&str, usize, usize) + Sync)) -> Result<CheckReport> {
    let started = Instant::now();
    progress("loading plugins", 0, 3);
    let index = LoadOrderIndex::open(&request.plugins)?;
    progress("indexing previsbine assets", 1, 3);
    let names: Vec<String> = index.plugins.iter().map(|p| p.name.clone()).collect();
    let assets = AssetIndex::build(request.data_sources.clone(), &names)?;
    progress("checking cells", 2, 3);
    let mut report = check_index(&index, &assets, &request.base_plugins, &request.fixer_plugins, &request.options, progress);
    report.summary.seconds = started.elapsed().as_secs_f64();
    Ok(report)
}

pub fn check_index(
    index: &LoadOrderIndex,
    assets: &AssetIndex,
    base_plugins: &[String],
    fixer_plugins: &[String],
    options: &CheckOptions,
    progress: &(dyn Fn(&str, usize, usize) + Sync),
) -> CheckReport {
    let named = |names: &[String]| -> Vec<bool> {
        index.plugins.iter().map(|p| names.iter().any(|b| b.eq_ignore_ascii_case(&p.name))).collect()
    };
    let mut checker = Checker { index, assets, options, base: named(base_plugins), fixer: named(fixer_plugins), grid: FxHashMap::default() };
    let cells: Vec<(GlobalId, CellPlacement)> = index.cell_ids().collect();
    let total = cells.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let states: Vec<Option<CellState>> = cells
        .par_iter()
        .map(|&(id, placement)| {
            let state = checker.check_cell(id, placement);
            let count = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if count % 2048 == 0 || count == total {
                progress("checking cells", count, total);
            }
            state
        })
        .collect();
    let skipped_esl_origin = cells
        .iter()
        .filter(|(id, p)| index.is_esl(defining_plugin(*id)) && (p.world.is_none() || p.exterior))
        .count();
    let mut states: Vec<CellState> = states.into_iter().flatten().collect();
    for state in &states {
        if let (Some(world), Some((x, y)), true) = (state.placement.world, state.stamps.grid, state.placement.exterior) {
            checker.grid.insert((world, x, y), state.id);
        }
    }
    let by_id: FxHashMap<GlobalId, usize> = states.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
    let roots: BTreeSet<(GlobalId, GlobalId)> = states.iter().filter_map(|s| checker.cluster_root(s).map(|root| (root, s.id))).collect();
    let mut members: FxHashMap<GlobalId, Vec<usize>> = FxHashMap::default();
    for (root, cell) in roots {
        members.entry(root).or_default().push(by_id[&cell]);
    }
    let mut clusters: Vec<(GlobalId, ClusterReport)> = members
        .par_iter()
        .map(|(&root, member_indices)| (root, checker.check_cluster(root, member_indices, &states)))
        .collect();
    // A member whose previs must go means the whole 3x3 is solved again.
    for (root, cluster) in clusters.iter_mut().filter(|(_, c)| c.grid.is_some()) {
        let needing: Vec<String> = members[root]
            .iter()
            .filter(|&&m| states[m].findings.iter().any(|f| f.action >= Action::RebuildPrevis))
            .map(|&m| checker.form_key(states[m].id))
            .collect();
        if needing.is_empty() {
            continue;
        }
        cluster.findings.push(checker.finding(
            Action::RebuildPrevis,
            "member_needs_previs",
            format!("{} member CELL(s) need new previs: {}", needing.len(), needing.join(", ")),
            None,
            None,
        ));
        cluster.action = cluster.action.max(Some(Action::RebuildPrevis));
    }
    let mut cluster_of: FxHashMap<GlobalId, String> = FxHashMap::default();
    for (root, cluster) in &clusters {
        for &member in &members[root] {
            cluster_of.insert(states[member].id, cluster.form_key.clone());
        }
    }

    let mut summary = CheckSummary { plugins: index.plugins.len(), cells: total, skipped_esl_origin, ..Default::default() };
    summary.loose_previsbines = states.iter().any(|s| s.findings.iter().any(|f| f.code == "loose_precombined_mesh"))
        || clusters.iter().any(|(_, c)| c.findings.iter().any(|f| f.code == "loose_tome"));
    let mut counts: FxHashMap<Action, usize> = FxHashMap::default();
    let mut cells_out = Vec::new();
    for state in states.iter_mut() {
        let findings = std::mem::take(&mut state.findings);
        let action = findings.iter().map(|f| f.action).max();
        if let Some(action) = action {
            *counts.entry(action).or_default() += 1;
        }
        summary.overridden_by_mod += state.overridden_by_mod as usize;
        if findings.is_empty() && !state.overridden_by_mod {
            continue;
        }
        cells_out.push(CellReport {
            form_key: checker.form_key(state.id),
            editor_id: state.stamps.editor_id.clone(),
            world: state.placement.world.and_then(|w| checker.editor_id_of(w)),
            grid: state.stamps.grid.filter(|_| state.placement.exterior),
            interior: state.stamps.flags & CELL_FLAG_INTERIOR != 0,
            winning_plugin: index.plugins[state.winner.plugin].name.clone(),
            overridden_by_mod: state.overridden_by_mod,
            cluster: cluster_of.get(&state.id).cloned(),
            action,
            findings,
        });
    }
    let mut clusters_out: Vec<ClusterReport> = clusters.into_iter().map(|(_, c)| c).filter(|c| !c.findings.is_empty()).collect();
    summary.clusters_to_rebuild = clusters_out.iter().filter(|c| c.action >= Some(Action::RebuildPrevis)).count();
    let mut actions: Vec<(Action, usize)> = counts.into_iter().collect();
    actions.sort();
    summary.actions = actions;
    cells_out.sort_by(|a, b| b.action.cmp(&a.action).then_with(|| a.form_key.cmp(&b.form_key)));
    clusters_out.sort_by(|a, b| b.action.cmp(&a.action).then_with(|| a.form_key.cmp(&b.form_key)));
    CheckReport { summary, cells: cells_out, clusters: clusters_out }
}

/// Exterior cluster root coordinate: CK groups cells `-1..=1`, `2..=4`, `-4..=-2`, ...
pub fn cluster_center(value: i32) -> i32 {
    ((value + value.signum()) / 3) * 3
}

impl<'a> Checker<'a> {
    fn name(&self, plugin: usize) -> String {
        self.index.plugins[plugin].name.clone()
    }

    fn form_key(&self, id: GlobalId) -> String {
        format!("{:06X}:{}", object_id(id), self.index.plugins[defining_plugin(id)].name)
    }

    fn editor_id_of(&self, id: GlobalId) -> Option<String> {
        let (p, record) = self.index.record(id)?;
        let subrecords = self.index.plugins[p].subrecords_at(&record).ok()?;
        subrecords.first(b"EDID").map(zstring)
    }

    fn finding(&self, action: Action, code: &'static str, message: String, blamed: Option<usize>, record: Option<GlobalId>) -> Finding {
        Finding { action, code, message, blamed: blamed.map(|p| self.name(p)), record: record.map(|id| self.form_key(id)) }
    }

    fn push(&self, findings: &mut Vec<Finding>, cell_winner: usize, finding: Finding, blamed: Option<usize>) {
        if self.options.ignore_base_only && self.base[cell_winner] && blamed.is_none_or(|p| self.base[p]) {
            return;
        }
        if findings.iter().any(|f| f.code == finding.code && f.record == finding.record) {
            return;
        }
        findings.push(finding);
    }

    fn check_cell(&self, id: GlobalId, placement: CellPlacement) -> Option<CellState> {
        let index = self.index;
        let winner = *index.winner(id)?;
        if winner.record.flags & RECORD_FLAG_DELETED != 0 || (placement.world.is_some() && !placement.exterior) {
            return None;
        }
        let copies: Vec<(Entry, Stamps)> = index
            .chain(id)
            .iter()
            .filter(|e| !is_partial(e))
            .map(|e| (*e, read_stamps(&index.plugins[e.plugin], &e.record)))
            .collect();
        let stamps = copies.last()?.1.clone();
        let mut state = CellState {
            id,
            placement,
            winner,
            stamps,
            copies,
            precombine_owner: None,
            findings: Vec::new(),
            overridden_by_mod: false,
        };
        state.overridden_by_mod = state.copies.iter().any(|(e, _)| !self.base[e.plugin])
            || index.children(id).iter().any(|&child| index.winner(child).is_some_and(|e| !self.base[e.plugin]));
        if index.is_esl(defining_plugin(id)) {
            if state.stamps.precombined() {
                let finding = self.finding(
                    Action::Warn,
                    "esl_origin_precombined",
                    "CELL is defined in an ESL-flagged plugin; the game ignores its precombines".into(),
                    Some(defining_plugin(id)),
                    None,
                );
                state.findings.push(finding);
            }
            return Some(state);
        }
        self.check_disabled(&mut state);
        if state.stamps.precombined() {
            self.check_precombined_references(&mut state);
            self.check_ownership(&mut state);
        }
        self.check_land(&mut state);
        self.check_unprecombined(&mut state);
        self.check_field_regressions(&mut state);
        self.check_requested(&mut state);
        Some(state)
    }

    fn check_requested(&self, state: &mut CellState) {
        if self.options.rebuild_all {
            let finding = self.finding(Action::RebuildPrecombines, "rebuild_requested", "rebuilding every CELL of the load order".into(), None, None);
            state.findings.push(finding);
            return;
        }
        let Some(wanted) = &self.options.rebuild_plugin else { return };
        let Some(plugin) = self.index.plugins.iter().position(|p| p.name.eq_ignore_ascii_case(wanted)) else { return };
        let touches = |id: GlobalId| self.index.chain(id).iter().any(|e| e.plugin == plugin);
        if touches(state.id) || self.index.children(state.id).iter().any(|&child| touches(child)) {
            let finding = self.finding(
                Action::RebuildPrecombines,
                "rebuild_requested",
                format!("rebuilding every CELL {} touches", self.name(plugin)),
                Some(plugin),
                None,
            );
            state.findings.push(finding);
        }
    }

    /// A field a mod (or a bug-fix plugin) changed that a later override of
    /// the CELL silently reverts. Not a previsbine break, but a patch that
    /// overrides the CELL must forward these values instead of the winner's.
    fn check_field_regressions(&self, state: &mut CellState) {
        let copies: Vec<Entry> = state.copies.iter().map(|(e, _)| *e).collect();
        let interior = state.stamps.flags & CELL_FLAG_INTERIOR != 0;
        let culprit = state.winner.plugin;
        for regression in field_regressions(self.index, &self.base, &self.fixer, &copies, interior) {
            let source = copies[regression.best].plugin;
            let finding = self.finding(
                Action::Info,
                "cell_field_regressed",
                format!("{} from {} is reverted by {}", String::from_utf8_lossy(&regression.signature), self.name(source), self.name(culprit)),
                Some(culprit),
                None,
            );
            self.push(&mut state.findings, culprit, finding, Some(culprit));
        }
        if !interior {
            for (copy, lost) in lost_regions(self.index, &self.base, &copies) {
                let finding = self.finding(
                    Action::Info,
                    "cell_regions_regressed",
                    format!("{} region(s) {} added are dropped by {}", lost.len(), self.name(copies[copy].plugin), self.name(culprit)),
                    Some(culprit),
                    None,
                );
                self.push(&mut state.findings, culprit, finding, Some(culprit));
            }
        }
    }

    /// Previs or precombines an earlier copy had and the winner dropped.
    fn check_disabled(&self, state: &mut CellState) {
        let winner = state.winner.plugin;
        let earlier = &state.copies[..state.copies.len() - 1];
        let stamps = &state.stamps;
        if stamps.xcri_malformed {
            let finding = self.finding(Action::RebuildPrecombines, "xcri_malformed", "the winning CELL's XCRI cannot be decoded".into(), Some(winner), None);
            self.push(&mut state.findings, winner, finding, Some(winner));
        }
        if stamps.flags & CELL_FLAG_NO_PREVIS != 0 {
            if let Some(position) = earlier.iter().rposition(|(_, s)| s.previs_enabled()) {
                let culprit = state.copies[position + 1].0.plugin;
                let action = if self.options.restore_disabled { Action::RebuildPrevis } else { Action::Warn };
                let finding = self.finding(action, "previs_disabled", "the No Previs flag disables this CELL's previs".into(), Some(culprit), None);
                self.push(&mut state.findings, winner, finding, Some(culprit));
            }
        }
        let had_precombines = earlier.iter().rposition(|(_, s)| s.precombined());
        match (&stamps.xcri, stamps.pcmb.filter(|&d| d != 0), had_precombines) {
            (None, Some(_), Some(_)) => {
                let finding = self.finding(
                    Action::Fix,
                    "xcri_bleed_through",
                    "PCMB without XCRI: an earlier override's combined references apply; needs an empty XCRI".into(),
                    Some(winner),
                    None,
                );
                self.push(&mut state.findings, winner, finding, Some(winner));
            }
            (None, None, Some(position)) => {
                let culprit = state.copies[position + 1].0.plugin;
                let action = if self.options.restore_disabled { Action::RebuildPrecombines } else { Action::Warn };
                let finding = self.finding(action, "precombines_removed", "an override removed this CELL's precombines (XCRI)".into(), Some(culprit), None);
                self.push(&mut state.findings, winner, finding, Some(culprit));
            }
            (Some(xcri), None, Some(position)) if !xcri.mesh_ids.is_empty() => {
                let culprit = state.copies[position + 1].0.plugin;
                let action = if self.options.restore_disabled { Action::RebuildPrecombines } else { Action::Warn };
                let finding = self.finding(action, "precombines_disabled", "an override removed PCMB, disabling the precombines".into(), Some(culprit), None);
                self.push(&mut state.findings, winner, finding, Some(culprit));
            }
            _ => {}
        }
    }

    /// The combined references against their winning copies.
    fn check_precombined_references(&self, state: &mut CellState) {
        let index = self.index;
        let cell_plugin = state.winner.plugin;
        let pcmb = state.stamps.pcmb.unwrap_or(0);
        let interior = state.stamps.flags & CELL_FLAG_INTERIOR != 0;
        let Some(xcri) = state.stamps.xcri.clone() else { return };
        let references: FxHashSet<u32> = xcri.reference_mesh_pairs.iter().map(|&(r, _)| r).collect();
        for raw in references {
            let reference = index.resolve(cell_plugin, raw);
            let chain = index.chain(reference);
            let Some(&winner) = chain.last() else {
                let finding = self.finding(Action::Warn, "precombined_reference_missing", "a combined reference exists in no plugin".into(), None, Some(reference));
                self.push(&mut state.findings, cell_plugin, finding, None);
                continue;
            };
            let plugin = &index.plugins[winner.plugin];
            let date = version_date(plugin, &winner.record);
            let overridden = chain.len() > 1;
            let blamed = Some(winner.plugin);
            let mut report = |action: Action, code: &'static str, message: String| {
                let finding = self.finding(action, code, message, blamed, Some(reference));
                self.push(&mut state.findings, cell_plugin, finding, blamed);
            };
            if overridden
                && (date == 0 || date > pcmb)
                && defining_plugin(state.id) == defining_plugin(reference)
                && (winner.plugin == cell_plugin || (interior && index.is_esm(winner.plugin)))
            {
                report(
                    Action::RebuildPrecombines,
                    "game_disables_precombines",
                    format!("a combined reference override dated {} after PCMB {} disables the CELL's precombines in game", crate::load_order::date_label(date), crate::load_order::date_label(pcmb)),
                );
                continue;
            }
            if winner.record.flags & RECORD_FLAG_DELETED != 0 {
                report(Action::RebuildPrecombines, "precombined_reference_deleted", "a deleted combined reference is still rendered by the precombine".into());
                continue;
            }
            if winner.record.flags & REFERENCE_FLAG_INITIALLY_DISABLED != 0 {
                report(Action::RebuildPrecombines, "precombined_reference_disabled", "a disabled combined reference is still rendered by the precombine".into());
                continue;
            }
            if winner.home != Some(state.id) && !(winner.persistent && !interior) {
                report(Action::RebuildPrecombines, "precombined_reference_moved", "a combined reference moved to another CELL and is rendered twice".into());
                continue;
            }
            if !self.precombinable(&winner) {
                report(Action::RebuildPrecombines, "precombined_reference_unprecombinable", "a combined reference can no longer be precombined (base, links or flags changed)".into());
                continue;
            }
            if self.base_mesh_changed(&winner, pcmb) {
                report(Action::RebuildPrecombines, "precombined_mesh_changed", "the combined reference's base model was replaced after the precombine was built".into());
                continue;
            }
            if overridden && date > pcmb && !self.same_as_previous(chain) {
                report(
                    Action::RebuildPrecombines,
                    "precombined_reference_changed",
                    format!("a combined reference was changed ({}) after PCMB {}", crate::load_order::date_label(date), crate::load_order::date_label(pcmb)),
                );
            }
        }
    }

    /// The winning copy of `chain` equals the copy before it (an ITM override).
    fn same_as_previous(&self, chain: &[Entry]) -> bool {
        let [.., previous, last] = chain else { return true };
        let bytes = |e: &Entry| self.index.plugins[e.plugin].subrecords_at(&e.record).map(|s| s.raw().to_vec()).ok();
        let flags = |e: &Entry| e.record.flags & (RECORD_FLAG_DELETED | REFERENCE_FLAG_INITIALLY_DISABLED);
        // Raw FormIDs inside differ between plugins with different master lists.
        let previous_owner = &self.index.plugins[previous.plugin];
        let last_owner = &self.index.plugins[last.plugin];
        previous_owner.masters == last_owner.masters[..last_owner.masters.len().min(previous_owner.masters.len())]
            && flags(previous) == flags(last)
            && bytes(previous).is_some_and(|b| Some(b) == bytes(last))
    }

    fn base_of(&self, reference: &Entry) -> Option<(GlobalId, usize, RecordRef)> {
        let plugin = &self.index.plugins[reference.plugin];
        let subrecords = plugin.subrecords_at(&reference.record).ok()?;
        let raw = read_u32(subrecords.first(b"NAME")?, 0)?;
        let id = self.index.resolve(reference.plugin, raw);
        let (p, record) = self.index.record(id)?;
        Some((id, p, record))
    }

    /// CK's eligibility for a placed reference's winning copy.
    fn precombinable(&self, reference: &Entry) -> bool {
        if &reference.record.signature != b"REFR" || reference.record.flags & (RECORD_FLAG_DELETED | REFERENCE_FLAG_INITIALLY_DISABLED) != 0 {
            return false;
        }
        let Some((base_id, base_plugin, base)) = self.base_of(reference) else { return false };
        if base.flags & BASE_FLAG_IS_MARKER != 0 || (defining_plugin(base_id) == 0 && object_id(base_id) <= LAST_ENGINE_MARKER) {
            return false;
        }
        match &base.signature {
            b"SCOL" => true,
            b"STAT" => {
                let plugin = &self.index.plugins[reference.plugin];
                let Ok(subrecords) = plugin.subrecords_at(&reference.record) else { return false };
                let mut links = 0;
                for (signature, data) in subrecords.iter() {
                    if STAT_BLOCKING_FIELDS.contains(&&signature) {
                        return false;
                    }
                    if &signature == b"XLKR" {
                        links += 1;
                        let keyword = read_u32(data, 0).map(|raw| self.index.resolve(reference.plugin, raw));
                        if keyword != Some(global_id(0, MULTIREF_LOD_KEYWORD)) {
                            return false;
                        }
                    }
                }
                links <= 1
                    && self.index.plugins[base_plugin]
                        .subrecords_at(&base)
                        .is_ok_and(|s| s.first(b"FTYP").is_none())
            }
            _ => false,
        }
    }

    fn base_mesh_changed(&self, reference: &Entry, pcmb: u16) -> bool {
        let Some((base_id, base_plugin, base)) = self.base_of(reference) else { return false };
        if base_plugin == defining_plugin(base_id) {
            return false;
        }
        let date = version_date(&self.index.plugins[base_plugin], &base);
        if date != 0 && date <= pcmb {
            return false;
        }
        let model = |p: usize, r: &RecordRef| {
            self.index.plugins[p].subrecords_at(r).ok().and_then(|s| s.first(b"MODL").map(zstring)).map(|m| m.to_ascii_lowercase())
        };
        let original = self.index.original(base_id).and_then(|r| model(defining_plugin(base_id), &r));
        model(base_plugin, &base) != original
    }

    /// Which plugin's archive provides the precombined meshes, and whether
    /// the winner still matches what that plugin built.
    fn check_ownership(&self, state: &mut CellState) {
        let cell_plugin = state.winner.plugin;
        let Some(xcri) = state.stamps.xcri.clone() else { return };
        let mut owners: BTreeSet<usize> = BTreeSet::new();
        let cell_owner = &self.index.plugins[defining_plugin(state.id)].name;
        for &mesh in &xcri.mesh_ids {
            let (path, source) = self.assets.precombined_mesh(cell_owner, object_id(state.id), mesh);
            let Some(source) = source else {
                let introducer = state
                    .copies
                    .iter()
                    .find(|(_, s)| s.xcri.as_ref().is_some_and(|x| x.mesh_ids.contains(&mesh)))
                    .map(|(e, _)| e.plugin);
                let finding = self.finding(
                    Action::RebuildPrecombines,
                    "precombined_mesh_missing",
                    format!("precombined mesh {:08X}_{mesh:08X}_OC.nif is in no data source", object_id(state.id)),
                    introducer,
                    None,
                );
                self.push(&mut state.findings, cell_plugin, finding, introducer);
                continue;
            };
            match self.assets.source_owner[source] {
                Some(owner) => {
                    owners.insert(owner);
                }
                None if self.assets.sources[source].is_loose() => {
                    let finding = self.finding(
                        Action::Warn,
                        "loose_precombined_mesh",
                        format!("{path} is loose ({}); its owner is unknown", self.assets.sources[source].label()),
                        None,
                        None,
                    );
                    self.push(&mut state.findings, cell_plugin, finding, None);
                }
                None => {
                    let finding = self.finding(
                        Action::Warn,
                        "precombined_mesh_unowned_archive",
                        format!("{path} comes from {}, which no loaded plugin owns", self.assets.sources[source].label()),
                        None,
                        None,
                    );
                    self.push(&mut state.findings, cell_plugin, finding, None);
                }
            }
        }
        let modded: Vec<usize> = owners.iter().copied().filter(|&p| !self.base[p]).collect();
        if owners.len() > 1 {
            let action = if modded.len() <= 1 { Action::Warn } else { Action::RebuildPrecombines };
            let names: Vec<String> = owners.iter().map(|&p| self.name(p)).collect();
            let blamed = owners.iter().next_back().copied();
            let finding = self.finding(action, "mixed_precombine_owners", format!("precombined meshes come from several plugins: {}", names.join(", ")), blamed, None);
            self.push(&mut state.findings, cell_plugin, finding, blamed);
        }
        let Some(&owner) = owners.iter().next_back() else { return };
        state.precombine_owner = Some(owner);
        let Some(position) = state.copies.iter().rposition(|(e, _)| e.plugin == owner) else {
            let finding = self.finding(
                Action::Warn,
                "precombine_owner_has_no_cell",
                "the plugin whose archive provides the meshes does not override this CELL".into(),
                Some(owner),
                None,
            );
            self.push(&mut state.findings, cell_plugin, finding, Some(owner));
            return;
        };
        let owner_stamps = state.copies[position].1.clone();
        let owner_view = |s: &Stamps| s.xcri.clone().map(|x| self.xcri_ids(owner, &x));
        let winner_view = state.stamps.xcri.clone().map(|x| self.xcri_ids(cell_plugin, &x));
        if owner_view(&owner_stamps) != winner_view {
            let culprit = state.copies[position + 1..].iter().find(|(e, s)| s.xcri.clone().map(|x| self.xcri_ids(e.plugin, &x)) != owner_view(&owner_stamps));
            let blamed = culprit.map(|(e, _)| e.plugin).or(Some(cell_plugin));
            let finding = self.finding(
                Action::RebuildPrecombines,
                "xcri_differs_from_owner",
                format!("the winning XCRI is not the one {} built its meshes for", self.name(owner)),
                blamed,
                None,
            );
            self.push(&mut state.findings, cell_plugin, finding, blamed);
        }
        let owner_pcmb = owner_stamps.pcmb.unwrap_or(0);
        let Some(owner_xcri) = owner_stamps.xcri else { return };
        let references: FxHashSet<u32> = owner_xcri.reference_mesh_pairs.iter().map(|&(r, _)| r).collect();
        for raw in references {
            let reference = self.index.resolve(owner, raw);
            let chain = self.index.chain(reference);
            let Some(&winner) = chain.last() else { continue };
            if winner.plugin == owner || chain.len() < 2 || winner.record.flags & RECORD_FLAG_DELETED != 0 || winner.home != Some(state.id) {
                continue;
            }
            let date = version_date(&self.index.plugins[winner.plugin], &winner.record);
            let newer = date == 0 || date > owner_pcmb;
            if (newer || !self.index.is_master_of(winner.plugin, owner)) && !self.same_as_previous(chain) {
                let finding = self.finding(
                    Action::RebuildPrecombines,
                    "reference_overridden_after_precombine",
                    format!("{} changed a combined reference that {}'s precombines do not include", self.name(winner.plugin), self.name(owner)),
                    Some(winner.plugin),
                    Some(reference),
                );
                self.push(&mut state.findings, cell_plugin, finding, Some(winner.plugin));
            }
        }
    }

    /// XCRI with its FormIDs made load-order independent, for comparing copies.
    fn xcri_ids(&self, plugin: usize, xcri: &Xcri) -> (Vec<u32>, Vec<(GlobalId, u32)>) {
        let mut pairs: Vec<(GlobalId, u32)> = xcri.reference_mesh_pairs.iter().map(|&(r, m)| (self.index.resolve(plugin, r), m)).collect();
        pairs.sort_unstable();
        let mut meshes = xcri.mesh_ids.clone();
        meshes.sort_unstable();
        (meshes, pairs)
    }

    /// A LAND override saved with the winning CELL after PCMB drops the precombines.
    fn check_land(&self, state: &mut CellState) {
        let Some(pcmb) = state.stamps.pcmb.filter(|_| state.stamps.precombined()) else { return };
        let cell_plugin = state.winner.plugin;
        for &child in self.index.children(state.id) {
            let Some(land) = self.index.winner(child) else { continue };
            if &land.record.signature != b"LAND" || land.plugin != cell_plugin || land.plugin == defining_plugin(child) {
                continue;
            }
            let date = version_date(&self.index.plugins[land.plugin], &land.record);
            if date == 0 || date > pcmb {
                let finding = self.finding(
                    Action::Fix,
                    "land_disables_precombines",
                    "a LAND override saved with the winning CELL after PCMB disables its precombines; forwarding the CELL fixes it".into(),
                    Some(land.plugin),
                    Some(child),
                );
                self.push(&mut state.findings, cell_plugin, finding, Some(land.plugin));
            }
        }
    }

    /// Mod-added or mod-changed precombinable references left out of precombines.
    fn check_unprecombined(&self, state: &mut CellState) {
        let Some(threshold) = self.options.unprecombined_threshold else { return };
        if !state.stamps.precombined() {
            return;
        }
        let combined: FxHashSet<GlobalId> = state
            .stamps
            .xcri
            .iter()
            .flat_map(|x| x.reference_mesh_pairs.iter().map(|&(r, _)| self.index.resolve(state.winner.plugin, r)))
            .collect();
        let count = self
            .index
            .children(state.id)
            .iter()
            .filter(|&&child| !combined.contains(&child))
            .filter_map(|&child| self.index.winner(child))
            .filter(|e| !self.base[e.plugin] && e.home == Some(state.id) && self.precombinable(e))
            .count();
        if count >= threshold {
            let finding = self.finding(
                Action::RebuildPrecombines,
                "unprecombined_references",
                format!("{count} precombinable references from mods are rendered outside the precombines"),
                None,
                None,
            );
            self.push(&mut state.findings, state.winner.plugin, finding, None);
        }
    }

    fn cluster_root(&self, state: &CellState) -> Option<GlobalId> {
        if state.stamps.flags & CELL_FLAG_INTERIOR != 0 || state.placement.world.is_none() {
            return Some(state.id);
        }
        let (x, y) = state.stamps.grid?;
        let computed = self.grid.get(&(state.placement.world?, cluster_center(x), cluster_center(y))).copied();
        let rvis = state.stamps.rvis.map(|raw| self.index.resolve(state.winner.plugin, raw));
        rvis.filter(|r| self.index.winner(*r).is_some()).or(computed)
    }

    fn check_cluster(&self, root: GlobalId, member_indices: &[usize], states: &[CellState]) -> ClusterReport {
        let index = self.index;
        let members: Vec<&CellState> = member_indices.iter().map(|&i| &states[i]).collect();
        let root_state = members.iter().find(|s| s.id == root).copied();
        let root_winner = root_state.map(|s| s.winner.plugin).or_else(|| index.winner(root).map(|e| e.plugin)).unwrap_or(0);
        let mut report = ClusterReport {
            form_key: self.form_key(root),
            world: members.first().and_then(|s| s.placement.world).and_then(|w| self.editor_id_of(w)),
            grid: root_state.and_then(|s| s.stamps.grid).filter(|_| root_state.is_some_and(|s| s.placement.exterior)),
            members: members.iter().map(|s| self.form_key(s.id)).collect(),
            previs_owner: None,
            action: None,
            findings: Vec::new(),
        };
        if index.is_esl(defining_plugin(root)) {
            return report;
        }
        let needs_tome = members.iter().any(|s| {
            s.stamps.previs_enabled()
                && (s.stamps.precombined() || s.stamps.xpri.as_ref().is_some_and(|x| x.iter().any(|&r| !self.is_dummy_xpri(s.winner.plugin, r))))
        });
        for state in &members {
            self.check_xpri_bleed(state, &mut report.findings);
        }
        let master_name = &index.plugins[defining_plugin(root)].name;
        let path = AssetIndex::uvd_path(master_name, object_id(root));
        let providers = self.assets.providers(&path);
        let Some(&source) = providers.last() else {
            if needs_tome {
                let finding = self.finding(Action::RebuildPrevis, "tome_missing", format!("{path} is in no data source"), None, None);
                self.push(&mut report.findings, root_winner, finding, None);
            }
            report.action = report.findings.iter().map(|f| f.action).max();
            return report;
        };
        let Some(owner) = self.assets.source_owner[source] else {
            let code = if self.assets.sources[source].is_loose() { "loose_tome" } else { "tome_unowned_archive" };
            let finding = self.finding(Action::Warn, code, format!("{path} comes from {}; its owner is unknown", self.assets.sources[source].label()), None, None);
            self.push(&mut report.findings, root_winner, finding, None);
            report.action = report.findings.iter().map(|f| f.action).max();
            return report;
        };
        report.previs_owner = Some(self.name(owner));
        let visi = members
            .iter()
            .flat_map(|s| s.copies.iter().filter(|(e, _)| e.plugin == owner).filter_map(|(_, st)| st.visi))
            .max()
            .unwrap_or(0);
        for state in &members {
            let cell_plugin = state.winner.plugin;
            if let Some(pcmb) = state.stamps.pcmb.filter(|&d| d > visi && state.stamps.precombined()) {
                let finding = self.finding(
                    Action::RebuildPrevis,
                    "precombines_newer_than_previs",
                    format!("{} PCMB {} is newer than the cluster's VISI {}", self.form_key(state.id), crate::load_order::date_label(pcmb), crate::load_order::date_label(visi)),
                    Some(cell_plugin),
                    Some(state.id),
                );
                self.push(&mut report.findings, cell_plugin, finding, Some(cell_plugin));
            }
            if let Some(precombines) = state.precombine_owner.filter(|&p| p != owner && !index.is_master_of(p, owner)) {
                let finding = self.finding(
                    Action::RebuildPrevis,
                    "previs_predates_precombines",
                    format!("previs from {} was not built against {}'s precombines", self.name(owner), self.name(precombines)),
                    Some(precombines),
                    Some(state.id),
                );
                self.push(&mut report.findings, cell_plugin, finding, Some(precombines));
            }
            for &child in index.children(state.id) {
                let Some(land) = index.winner(child) else { continue };
                if &land.record.signature != b"LAND" || land.plugin == owner || index.is_master_of(land.plugin, owner) {
                    continue;
                }
                let date = version_date(&index.plugins[land.plugin], &land.record);
                if date == 0 || date > visi {
                    let finding = self.finding(
                        Action::RebuildPrevis,
                        "land_changed_after_previs",
                        format!("{} changed terrain after the cluster's previs was built", self.name(land.plugin)),
                        Some(land.plugin),
                        Some(child),
                    );
                    self.push(&mut report.findings, cell_plugin, finding, Some(land.plugin));
                }
            }
        }
        report.action = report.findings.iter().map(|f| f.action).max();
        report
    }

    fn is_dummy_xpri(&self, plugin: usize, raw: u32) -> bool {
        self.index.resolve(plugin, raw) == global_id(0, DUMMY_XPRI_OBJECT)
    }

    /// The game skips an empty XPRI and uses an earlier override's list.
    fn check_xpri_bleed(&self, state: &CellState, findings: &mut Vec<Finding>) {
        let blank = state.stamps.xpri.as_ref().is_some_and(Vec::is_empty);
        if !blank || !state.stamps.previs_enabled() {
            return;
        }
        let earlier = &state.copies[..state.copies.len() - 1];
        if earlier.iter().any(|(_, s)| s.xpri.as_ref().is_some_and(|x| !x.is_empty())) {
            let finding = self.finding(
                Action::Fix,
                "xpri_bleed_through",
                format!("{} has an empty XPRI, so an earlier override's list applies; needs the placeholder XPRI", self.form_key(state.id)),
                Some(state.winner.plugin),
                Some(state.id),
            );
            self.push(findings, state.winner.plugin, finding, Some(state.winner.plugin));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_order::tests::{PluginBuilder, interior_cell, reference, subrecord};
    use crate::load_order::{PLUGIN_FLAG_ESM, PLUGIN_FLAG_ESL};

    const OLD: u16 = (20 << 9) | (1 << 5) | 1;
    const NEW: u16 = (26 << 9) | (10 << 5) | 11;

    fn xcri(meshes: &[u32], pairs: &[(u32, u32)]) -> Vec<u8> {
        let xcri = Xcri { mesh_ids: meshes.to_vec(), reference_mesh_pairs: pairs.to_vec() };
        subrecord(b"XCRI", &xcri.encode())
    }

    fn stat(form_id: u32) -> Vec<u8> {
        let stat = crate::load_order::tests::record(b"STAT", form_id, 0, OLD, &[subrecord(b"MODL", b"Test\\Box.nif\0")]);
        crate::load_order::tests::group(u32::from_le_bytes(*b"STAT"), 0, &[stat])
    }

    /// Fallout4.esm-like master: one precombined interior with two references,
    /// its mesh and tome shipped in `Master - Main.ba2`-like loose folder.
    struct Fixture {
        dir: tempfile::TempDir,
        master: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let stamps = [
            subrecord(b"VISI", &OLD.to_le_bytes()),
            subrecord(b"PCMB", &OLD.to_le_bytes()),
            xcri(&[0xAAAA], &[(0x0000_0801, 0xAAAA), (0x0000_0802, 0xAAAA)]),
        ];
        let master = PluginBuilder::new(PLUGIN_FLAG_ESM, &[])
            .push(stat(0x0000_0900))
            .push(interior_cell(0x0000_0800, 0, OLD, &stamps, &[reference(0x0000_0801, 0, OLD, 0x0000_0900), reference(0x0000_0802, 0, OLD, 0x0000_0900)]))
            .write(dir.path(), "Master.esm");
        Fixture { dir, master }
    }

    fn data_with_assets(dir: &std::path::Path) -> Vec<DataSource> {
        let data = dir.join("Data");
        let meshes = data.join("Meshes").join("PreCombined").join("Master.esm");
        std::fs::create_dir_all(&meshes).unwrap();
        std::fs::write(meshes.join("00000800_0000AAAA_OC.NIF"), []).unwrap();
        let vis = data.join("Vis").join("Master.esm");
        std::fs::create_dir_all(&vis).unwrap();
        std::fs::write(vis.join("00000800.uvd"), []).unwrap();
        vec![DataSource::Loose { path: data }]
    }

    fn run(plugins: Vec<PathBuf>, sources: Vec<DataSource>, options: CheckOptions) -> CheckReport {
        let request = CheckRequest { plugins, data_sources: sources, base_plugins: vec!["Master.esm".into()], fixer_plugins: vec!["Fixer.esp".into()], options };
        check(&request, &|_, _, _| {}).unwrap()
    }

    fn codes(report: &CheckReport) -> Vec<&'static str> {
        report.cells.iter().flat_map(|c| c.findings.iter().map(|f| f.code)).chain(report.clusters.iter().flat_map(|c| c.findings.iter().map(|f| f.code))).collect()
    }

    #[test]
    fn vanilla_alone_reports_nothing() {
        let f = fixture();
        let report = run(vec![f.master.clone()], data_with_assets(f.dir.path()), CheckOptions::default());
        assert!(codes(&report).is_empty(), "{:?}", codes(&report));
        assert_eq!(report.summary.cells, 1);
    }

    #[test]
    fn a_newer_reference_override_saved_with_the_cell_makes_the_game_drop_precombines() {
        let f = fixture();
        // An ESM override of the interior CELL and one combined reference, dated after PCMB.
        let modded = PluginBuilder::new(PLUGIN_FLAG_ESM, &["Master.esm"])
            .push(interior_cell(0x0000_0800, 0, NEW, &[subrecord(b"VISI", &OLD.to_le_bytes()), subrecord(b"PCMB", &OLD.to_le_bytes()), xcri(&[0xAAAA], &[(0x0000_0801, 0xAAAA), (0x0000_0802, 0xAAAA)])], &[reference(0x0000_0801, 0, NEW, 0x0000_0900)]))
            .write(f.dir.path(), "Mod.esm");
        let report = run(vec![f.master.clone(), modded], data_with_assets(f.dir.path()), CheckOptions::default());
        let cell = &report.cells[0];
        assert_eq!(cell.action, Some(Action::RebuildPrecombines));
        assert!(cell.findings.iter().any(|x| x.code == "game_disables_precombines" && x.blamed.as_deref() == Some("Mod.esm")), "{:?}", cell.findings);
        assert!(cell.overridden_by_mod);
    }

    #[test]
    fn a_deleted_combined_reference_needs_a_rebuild_and_the_tome_is_stale() {
        let f = fixture();
        let deleted = crate::load_order::tests::record(b"REFR", 0x0000_0802, RECORD_FLAG_DELETED, OLD, &[]);
        let modded = PluginBuilder::new(0, &["Master.esm"])
            .push(interior_cell(0x0000_0800, crate::load_order::RECORD_FLAG_PARTIAL, OLD, &[], &[deleted]))
            .write(f.dir.path(), "Mod.esp");
        let report = run(vec![f.master.clone(), modded], data_with_assets(f.dir.path()), CheckOptions::default());
        assert!(codes(&report).contains(&"precombined_reference_deleted"), "{:?}", codes(&report));
    }

    #[test]
    fn missing_meshes_and_tomes_are_reported_for_mod_cells() {
        let f = fixture();
        let modded = PluginBuilder::new(0, &["Master.esm"])
            .push(interior_cell(0x0000_0800, 0, OLD, &[subrecord(b"VISI", &OLD.to_le_bytes()), subrecord(b"PCMB", &OLD.to_le_bytes()), xcri(&[0xBBBB], &[(0x0000_0801, 0xBBBB)])], &[]))
            .write(f.dir.path(), "Mod.esp");
        let report = run(vec![f.master.clone(), modded], Vec::new(), CheckOptions::default());
        let found = codes(&report);
        assert!(found.contains(&"precombined_mesh_missing") && found.contains(&"tome_missing"), "{found:?}");
    }

    #[test]
    fn no_previs_flag_is_a_warning_unless_restoring() {
        let f = fixture();
        let flagged = |flags: u16| {
            let mut fields = vec![subrecord(b"VISI", &OLD.to_le_bytes()), subrecord(b"PCMB", &OLD.to_le_bytes()), xcri(&[0xAAAA], &[(0x0000_0801, 0xAAAA), (0x0000_0802, 0xAAAA)])];
            fields.insert(0, subrecord(b"DATA", &flags.to_le_bytes()));
            fields
        };
        // interior_cell already writes DATA=interior; a second DATA wins when read last.
        let modded = PluginBuilder::new(0, &["Master.esm"])
            .push(interior_cell(0x0000_0800, 0, OLD, &flagged(CELL_FLAG_INTERIOR | CELL_FLAG_NO_PREVIS), &[]))
            .write(f.dir.path(), "Mod.esp");
        let sources = data_with_assets(f.dir.path());
        let report = run(vec![f.master.clone(), modded.clone()], sources.clone(), CheckOptions::default());
        let finding = report.cells[0].findings.iter().find(|x| x.code == "previs_disabled").unwrap();
        assert_eq!(finding.action, Action::Warn);
        let restoring = CheckOptions { restore_disabled: true, ..CheckOptions::default() };
        let report = run(vec![f.master.clone(), modded], sources, restoring);
        assert_eq!(report.cells[0].action, Some(Action::RebuildPrevis));
    }

    #[test]
    fn esl_origin_cells_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let stamps = [subrecord(b"PCMB", &OLD.to_le_bytes()), xcri(&[1], &[(0x0000_0801, 1)])];
        let light = PluginBuilder::new(PLUGIN_FLAG_ESL, &[])
            .push(interior_cell(0x0000_0800, 0, OLD, &stamps, &[reference(0x0000_0801, 0, OLD, 0x0000_0900)]))
            .write(dir.path(), "Light.esp");
        let report = run(vec![light], Vec::new(), CheckOptions::default());
        assert_eq!(report.summary.skipped_esl_origin, 1);
        assert_eq!(codes(&report), vec!["esl_origin_precombined"]);
    }

    #[test]
    fn a_later_override_reverting_a_mods_lighting_is_reported() {
        let f = fixture();
        let lighting = |value: u8| subrecord(b"XCLL", &[value; 4]);
        let override_with = |name: &str, value: u8| {
            PluginBuilder::new(0, &["Master.esm"])
                .push(interior_cell(0x0000_0800, 0, OLD, &[lighting(value)], &[]))
                .write(f.dir.path(), name)
        };
        // Master has no XCLL; Lighting.esp sets it, Later.esp's copy predates it.
        let lit = override_with("Lighting.esp", 7);
        let later = PluginBuilder::new(0, &["Master.esm"])
            .push(interior_cell(0x0000_0800, 0, OLD, &[lighting(7)], &[]))
            .write(f.dir.path(), "Same.esp");
        let report = run(vec![f.master.clone(), lit.clone(), later], data_with_assets(f.dir.path()), CheckOptions::default());
        assert!(!codes(&report).contains(&"cell_field_regressed"), "matching values are not a regression");

        let own_value = override_with("Other.esp", 3);
        let report = run(vec![f.master.clone(), lit.clone(), own_value], data_with_assets(f.dir.path()), CheckOptions::default());
        assert!(!codes(&report).contains(&"cell_field_regressed"), "a later mod's own value wins deliberately");

        // A copy saved without the mod's lighting (the master's state) reverts it.
        let reverting = PluginBuilder::new(0, &["Master.esm"])
            .push(interior_cell(0x0000_0800, 0, OLD, &[], &[]))
            .write(f.dir.path(), "Revert.esp");
        let report = run(vec![f.master.clone(), lit, reverting], data_with_assets(f.dir.path()), CheckOptions::default());
        let finding = report.cells[0].findings.iter().find(|x| x.code == "cell_field_regressed").expect("regression");
        assert_eq!(finding.blamed.as_deref(), Some("Revert.esp"));
        assert_eq!(finding.message, "XCLL from Lighting.esp is reverted by Revert.esp");
    }

    #[test]
    fn every_cell_a_chosen_plugin_touches_is_rebuilt_on_request() {
        let f = fixture();
        let modded = PluginBuilder::new(0, &["Master.esm"])
            .push(interior_cell(0x0100_0A00, 0, OLD, &[], &[reference(0x0100_0A01, 0, OLD, 0x0000_0900)]))
            .write(f.dir.path(), "Mod.esp");
        let plugins = vec![f.master.clone(), modded];
        let report = run(plugins.clone(), data_with_assets(f.dir.path()), CheckOptions::default());
        assert!(!codes(&report).contains(&"rebuild_requested"));

        let all_cells = CheckOptions { rebuild_plugin: Some("mod.esp".into()), ..CheckOptions::default() };
        let report = run(plugins, data_with_assets(f.dir.path()), all_cells);
        let requested: Vec<&str> = report
            .cells
            .iter()
            .filter(|c| c.findings.iter().any(|x| x.code == "rebuild_requested"))
            .map(|c| c.form_key.as_str())
            .collect();
        assert_eq!(requested, vec!["000A00:Mod.esp"], "the master's untouched CELL stays out");
        assert_eq!(report.cells.iter().find(|c| c.form_key == "000A00:Mod.esp").unwrap().action, Some(Action::RebuildPrecombines));
    }

    #[test]
    fn every_cell_of_the_load_order_is_rebuilt_on_request() {
        let f = fixture();
        let modded = PluginBuilder::new(0, &["Master.esm"])
            .push(interior_cell(0x0100_0A00, 0, OLD, &[], &[]))
            .write(f.dir.path(), "Mod.esp");
        let all = CheckOptions { rebuild_all: true, ..CheckOptions::default() };
        let report = run(vec![f.master.clone(), modded], data_with_assets(f.dir.path()), all);
        let requested: Vec<&str> = report
            .cells
            .iter()
            .filter(|c| c.findings.iter().any(|x| x.code == "rebuild_requested"))
            .map(|c| c.form_key.as_str())
            .collect();
        assert_eq!(requested, vec!["000800:Master.esm", "000A00:Mod.esp"], "vanilla CELLs too, with nothing broken");
    }

    #[test]
    fn cluster_centers_group_cells_in_threes_around_zero() {
        let centers: Vec<i32> = (-5..=5).map(cluster_center).collect();
        assert_eq!(centers, vec![-6, -3, -3, -3, 0, 0, 0, 3, 3, 3, 6]);
    }
}
