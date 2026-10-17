//! Repair patch plugins for a whole load order.
//!
//! The seed is a full ESM named like the final patch. It holds complete
//! copies of the CELLs to rebuild: the winning CELL with regressed fields
//! forwarded, every winning child placed in it, its world, and the world's
//! persistent references standing in it, all renumbered into the patch's
//! master list. The generator rebuilds and stamps the seed like any plugin;
//! the patch is then cut from it: the CELLs whose stamps changed, the WRLDs
//! above them, and the children whose version-control date the seed had to
//! set (an undated or later-dated record makes the game drop the CELL's
//! precalculated data, see `metadata::date_version_control`).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use esp_authoring_core::plugin_runtime::{record_form_ids, remap_record_bytes};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::detect::{DUMMY_XPRI_OBJECT, cluster_center, default_fixer_plugins, field_regressions, lost_regions};
use crate::error::{PrevisError, Result};
use crate::load_order::{Entry, GlobalId, LoadOrderIndex, defining_plugin, global_id, is_partial, object_id};
use crate::metadata::{REPLACED, encode_subrecord};
use crate::plugin::{COMPRESSED_FLAG, HEADER_SIZE, Plugin, RecordRef, SubrecordIter, read_f32, read_i32, read_u32};
use crate::records::RECORD_FLAG_DELETED;

const GAME: &str = "fo4";
/// The plugin's own records take index `masters.len()`; 0xFF is the runtime's.
const MAX_MASTERS: usize = 0xFE;
const FORM_VERSION: u16 = 131;
const PLUGIN_FLAG_ESM: u32 = 0x1;
const CELL_FLAG_INTERIOR: u16 = 0x1;
const EXTERIOR_CELL_SIZE: f32 = 4096.0;
/// Records whose own references the generator follows (models, statics
/// collections, swaps): the files overriding those must be masters too, or
/// the generator would read an older copy.
const FOLLOWED: [&[u8; 4]; 20] = [
    b"STAT", b"SCOL", b"MSTT", b"FURN", b"ACTI", b"TACT", b"DOOR", b"CONT", b"LIGH", b"TREE", b"FLOR", b"MISC",
    b"MSWP", b"TXST", b"PKIN", b"MOVS", b"BNDS", b"TERM", b"IDLM", b"WRLD",
];

#[derive(Debug, Deserialize)]
pub struct SeedRequest {
    /// Plugin paths in load order, without any earlier copy of the patch.
    pub plugins: Vec<PathBuf>,
    #[serde(default)]
    pub base_plugins: Vec<String>,
    #[serde(default = "default_fixer_plugins")]
    pub fixer_plugins: Vec<String>,
    /// CELL form keys (`XXXXXX:Plugin.esm`) to copy.
    pub cells: Vec<String>,
    /// The copied CELLs whose precombines are rebuilt. Only their references
    /// are restamped: the game drops a CELL's precombines when a combined
    /// reference's override is dated 0 or after its PCMB, and only these get
    /// a new PCMB.
    #[serde(default)]
    pub precombine_cells: Vec<String>,
    /// `<work dir>\<Patch>.esm`: the seed carries the final patch's name.
    pub output: PathBuf,
    /// The packed date the generator will stamp with.
    pub date: u16,
    /// The seed stands in for the load-order plugin it is named after, to be
    /// written back into a copy of it: that plugin's records become the
    /// seed's own instead of a master's, so every file the generator names
    /// after the plugin (CSG, CDX, its PreCombined and Vis folders) is right.
    #[serde(default)]
    pub stand_in: bool,
    /// Load-order plugins the patch masters even when it copies nothing from
    /// them, so it always loads after them (a patch for a mod against PPF.esm
    /// masters both).
    #[serde(default)]
    pub required_masters: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SeedCell {
    pub form_key: String,
    /// Raw FormID in the seed (and the patch).
    pub form_id: u32,
    /// Digest of the stamp subrecords as copied, to tell rebuilt CELLs apart.
    pub stamps: String,
    /// An earlier copy has XCRI / XPRI, which would show through a patch
    /// CELL without one.
    pub earlier_xcri: bool,
    pub earlier_xpri: bool,
    /// CELL fields taken from an earlier override the winner reverts.
    pub forwarded_fields: Vec<String>,
    /// Copied only so previs sees the targets around a rebuilt cluster (the
    /// ring two CELLs out from its root); never planned or patched.
    #[serde(default)]
    pub context: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SkippedCell {
    pub form_key: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SeedReport {
    pub plugin: PathBuf,
    pub masters: Vec<String>,
    pub cells: Vec<SeedCell>,
    /// Children whose version-control date the seed set to the stamp date.
    pub redated: Vec<u32>,
    pub skipped: Vec<SkippedCell>,
    pub records: usize,
    pub seconds: f64,
}

#[derive(Debug, Deserialize)]
pub struct PatchRequest {
    pub seed: SeedReport,
    /// The patch to write; its file stem must match the seed's (the CSG and
    /// CDX carry it), its extension follows `esm`.
    pub output: PathBuf,
    /// An ESM-flagged patch; else a plain ESP (the default), which any patch
    /// mastering a plain ESP must be: an ESM always loads before every ESP.
    #[serde(default)]
    pub esm: bool,
}

#[derive(Debug, Serialize)]
pub struct PatchReport {
    pub plugin: PathBuf,
    pub masters: usize,
    /// CELLs the generator restamped, now in the patch.
    pub cells: Vec<String>,
    /// CELLs the generator left as they were; not in the patch.
    pub unchanged: Vec<String>,
    pub forwarded_children: usize,
}

/// A copy of one record from the load order, not yet renumbered.
#[derive(Clone, Copy)]
struct Source {
    plugin: usize,
    record: RecordRef,
}

enum Forwarded {
    Raw([u8; 4], Vec<u8>),
    Form([u8; 4], GlobalId),
    Regions(Vec<GlobalId>),
}

struct SeedCellPlan {
    form_key: String,
    source: Source,
    forwarded: Vec<Forwarded>,
    temporary: Vec<Source>,
    persistent: Vec<Source>,
    /// Copied for the camera only: never restamped, so never in the patch.
    navmeshes: Vec<Source>,
    earlier_xcri: bool,
    earlier_xpri: bool,
    context: bool,
    rebuild_precombines: bool,
}

#[derive(Default)]
struct WorldPlan {
    cells: Vec<usize>,
    grids: FxHashSet<(i32, i32)>,
    rebuilt_grids: FxHashSet<(i32, i32)>,
    /// The world's persistent CELL and the persistent references standing
    /// in a seeded grid CELL, each with whether that CELL is rebuilt.
    persistent: Option<(GlobalId, Source, Vec<(Source, bool)>)>,
}

pub fn write_seed(request: &SeedRequest) -> Result<SeedReport> {
    let index = LoadOrderIndex::open(&request.plugins)?;
    write_seed_from(&index, request)
}

pub fn write_seed_from(index: &LoadOrderIndex, request: &SeedRequest) -> Result<SeedReport> {
    let started = Instant::now();
    let seed_name = file_name(&request.output)?;
    let named_like_seed = index.plugins.iter().position(|p| p.name.eq_ignore_ascii_case(&seed_name));
    let stand_in = match (named_like_seed, request.stand_in) {
        (Some(_), false) => {
            return Err(PrevisError::invalid(format!("{seed_name} is in the load order; leave the old patch out when building a new one")));
        }
        (None, true) => return Err(PrevisError::invalid(format!("{seed_name} is not in the load order, so the seed can't stand in for it"))),
        (stand_in, _) => stand_in,
    };
    let named = |names: &[String]| -> Vec<bool> {
        index.plugins.iter().map(|p| names.iter().any(|n| n.eq_ignore_ascii_case(&p.name))).collect()
    };
    let (base, fixer) = (named(&request.base_plugins), named(&request.fixer_plugins));
    let by_name: FxHashMap<String, usize> = index.plugins.iter().enumerate().map(|(i, p)| (p.name.to_ascii_lowercase(), i)).collect();

    let mut skipped = Vec::new();
    let mut cells: Vec<SeedCellPlan> = Vec::new();
    let mut interiors = Vec::new();
    let mut worlds: BTreeMap<GlobalId, WorldPlan> = BTreeMap::new();
    let mut seen = FxHashSet::default();
    fn add(
        plan: SeedCellPlan,
        grid: Option<(GlobalId, (i32, i32))>,
        cells: &mut Vec<SeedCellPlan>,
        worlds: &mut BTreeMap<GlobalId, WorldPlan>,
        interiors: &mut Vec<usize>,
    ) {
        match grid {
            Some((world, grid)) => {
                let world_plan = worlds.entry(world).or_default();
                world_plan.cells.push(cells.len());
                world_plan.grids.insert(grid);
                if plan.rebuild_precombines {
                    world_plan.rebuilt_grids.insert(grid);
                }
            }
            None => interiors.push(cells.len()),
        }
        cells.push(plan);
    }
    let rebuilt: FxHashSet<GlobalId> = request.precombine_cells.iter().filter_map(|key| parse_form_key(key, &by_name)).collect();
    for form_key in &request.cells {
        let Some(id) = parse_form_key(form_key, &by_name) else {
            skipped.push(SkippedCell { form_key: form_key.clone(), reason: "not a CELL form key of this load order".into() });
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        match plan_seed_cell(index, &base, &fixer, id, form_key, false)? {
            Ok((mut plan, grid)) => {
                plan.rebuild_precombines = rebuilt.contains(&id);
                add(plan, grid, &mut cells, &mut worlds, &mut interiors)
            }
            Err(reason) => skipped.push(SkippedCell { form_key: form_key.clone(), reason: reason.into() }),
        }
    }

    // Previs lists the targets of the 16 CELLs around each rebuilt cluster.
    let mut ring = Vec::new();
    for (&world, world_plan) in &worlds {
        for &(x, y) in &world_plan.grids {
            let (root_x, root_y) = (cluster_center(x), cluster_center(y));
            for dx in -2..=2i32 {
                for dy in -2..=2i32 {
                    if dx.abs() == 2 || dy.abs() == 2 {
                        ring.push((world, root_x + dx, root_y + dy));
                    }
                }
            }
        }
    }
    if !ring.is_empty() {
        let mut grid_cells: FxHashMap<(GlobalId, i32, i32), GlobalId> = FxHashMap::default();
        for (cell, placement) in index.cell_ids() {
            let Some(world) = placement.world.filter(|w| placement.exterior && worlds.contains_key(w)) else { continue };
            let Some(winner) = index.winner(cell) else { continue };
            let subrecords = index.plugins[winner.plugin].subrecords_at(&winner.record)?;
            if let Some(xclc) = subrecords.first(b"XCLC").filter(|d| d.len() >= 8) {
                grid_cells.insert((world, read_i32(xclc, 0).unwrap(), read_i32(xclc, 4).unwrap()), cell);
            }
        }
        ring.sort_unstable();
        ring.dedup();
        for key in ring {
            let Some(&id) = grid_cells.get(&key) else { continue };
            if !seen.insert(id) {
                continue;
            }
            if let Ok((plan, grid)) = plan_seed_cell(index, &base, &fixer, id, &form_key_of(index, id), true)? {
                add(plan, grid, &mut cells, &mut worlds, &mut interiors);
            }
        }
    }

    if !worlds.is_empty() {
        for (cell, placement) in index.cell_ids() {
            let Some(world) = placement.world.filter(|_| !placement.exterior) else { continue };
            let Some(world_plan) = worlds.get_mut(&world) else { continue };
            let Some(winner) = index.winner(cell) else { continue };
            let mut standing = Vec::new();
            for &child in index.children(cell) {
                let Some(entry) = index.winner(child).filter(|e| e.home == Some(cell) && &e.record.signature == b"REFR") else { continue };
                let plugin = &index.plugins[entry.plugin];
                let data = plugin.subrecords_at(&entry.record)?;
                let Some((x, y)) = data.first(b"DATA").and_then(|d| Some((read_f32(d, 0)?, read_f32(d, 4)?))) else { continue };
                let grid = ((x / EXTERIOR_CELL_SIZE).floor() as i32, (y / EXTERIOR_CELL_SIZE).floor() as i32);
                if world_plan.grids.contains(&grid) {
                    standing.push((Source { plugin: entry.plugin, record: entry.record }, world_plan.rebuilt_grids.contains(&grid)));
                }
            }
            if !standing.is_empty() {
                world_plan.persistent = Some((cell, Source { plugin: winner.plugin, record: winner.record }, standing));
            }
        }
    }
    let world_sources: BTreeMap<GlobalId, Source> = worlds
        .keys()
        .map(|&world| {
            index
                .record(world)
                .map(|(plugin, record)| (world, Source { plugin, record }))
                .ok_or_else(|| PrevisError::invalid(format!("WRLD {} is missing", form_key_of(index, world))))
        })
        .collect::<Result<_>>()?;

    // Every file a copied record, a forwarded value, or a followed base needs.
    let mut sources: Vec<Source> = world_sources.values().copied().collect();
    for cell in &cells {
        sources.push(cell.source);
        sources.extend(cell.temporary.iter().chain(&cell.persistent).chain(&cell.navmeshes).copied());
    }
    for world_plan in worlds.values() {
        if let Some((_, cell, standing)) = &world_plan.persistent {
            sources.push(*cell);
            sources.extend(standing.iter().map(|&(source, _)| source));
        }
    }
    let mut needed = FxHashSet::default();
    let mut queue: Vec<GlobalId> = Vec::new();
    for source in &sources {
        needed.insert(source.plugin);
        queue.extend(references(index, *source)?);
    }
    for forwarded in cells.iter().flat_map(|c| &c.forwarded) {
        match forwarded {
            Forwarded::Form(_, id) => queue.push(*id),
            Forwarded::Regions(regions) => queue.extend(regions.iter().copied()),
            Forwarded::Raw(..) => {}
        }
    }
    let mut visited = FxHashSet::default();
    // The seed replaces a plugin it stands in for, so it carries the bases that plugin wins.
    let mut stand_in_bases = Vec::new();
    while let Some(id) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        needed.insert(defining_plugin(id));
        if let Some((plugin, record)) = index.record(id) {
            needed.insert(plugin);
            if FOLLOWED.contains(&&record.signature) {
                if Some(plugin) == stand_in && &record.signature != b"WRLD" {
                    stand_in_bases.push(Source { plugin, record });
                }
                queue.extend(references(index, Source { plugin, record })?);
            }
        }
    }
    for name in &request.required_masters {
        let plugin = *by_name
            .get(&name.to_ascii_lowercase())
            .ok_or_else(|| PrevisError::invalid(format!("required master {name} is not in the load order")))?;
        needed.insert(plugin);
    }
    let mut master_indices: Vec<usize> = needed.into_iter().filter(|&p| p < index.plugins.len() && Some(p) != stand_in).collect();
    master_indices.sort_unstable();
    if master_indices.len() > MAX_MASTERS {
        return Err(PrevisError::unsupported(format!(
            "the patch would need {} masters; a plugin can have {MAX_MASTERS}. Rebuild fewer CELLs at a time",
            master_indices.len()
        )));
    }
    let masters: Vec<String> = master_indices.iter().map(|&p| index.plugins[p].name.clone()).collect();
    let seed_raw = |id: GlobalId| -> Result<u32> {
        if Some(defining_plugin(id)) == stand_in {
            return Ok(((masters.len() as u32) << 24) | object_id(id));
        }
        master_indices
            .binary_search(&defining_plugin(id))
            .map(|position| ((position as u32) << 24) | object_id(id))
            .map_err(|_| PrevisError::invalid(format!("{} is not among the patch's masters", form_key_of(index, id))))
    };
    let copy = |source: Source| -> Result<Vec<u8>> {
        let plugin = &index.plugins[source.plugin];
        remap_record_bytes(plugin.raw_record(&source.record), GAME, &plugin.masters, &plugin.name, &masters, &seed_name)
            .map_err(|e| PrevisError::invalid(format!("{} {:08X} in {}: {e}", String::from_utf8_lossy(&source.record.signature), source.record.form_id, plugin.name)))
    };
    let mut redated = Vec::new();
    let mut copy_child = |source: Source, restamp: bool| -> Result<Vec<u8>> {
        let mut bytes = copy(source)?;
        let current = u16::from_le_bytes([bytes[16], bytes[17]]);
        if restamp && (current == 0 || current > request.date) {
            bytes[16..18].copy_from_slice(&request.date.to_le_bytes());
            redated.push(u32::from_le_bytes(bytes[12..16].try_into().unwrap()));
        }
        Ok(bytes)
    };

    let mut layout = Layout::default();
    for source in stand_in_bases {
        layout.records.entry(source.record.signature).or_default().push(copy(source)?);
    }
    let mut seed_cells = Vec::with_capacity(cells.len());
    let mut copied = Vec::with_capacity(cells.len());
    for cell in &cells {
        let mut fields: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        for forwarded in &cell.forwarded {
            fields.push(match forwarded {
                Forwarded::Raw(signature, data) => (*signature, data.clone()),
                Forwarded::Form(signature, id) => (*signature, seed_raw(*id)?.to_le_bytes().to_vec()),
                Forwarded::Regions(regions) => (*b"XCLR", regions.iter().map(|&r| seed_raw(r)).collect::<Result<Vec<u32>>>()?.iter().flat_map(|r| r.to_le_bytes()).collect()),
            });
        }
        let record = set_fields(&copy(cell.source)?, &fields)?;
        let form_id = u32::from_le_bytes(record[12..16].try_into().unwrap());
        seed_cells.push(SeedCell {
            form_key: cell.form_key.clone(),
            form_id,
            stamps: stamp_digest(&record)?,
            earlier_xcri: cell.earlier_xcri,
            earlier_xpri: cell.earlier_xpri,
            forwarded_fields: fields.iter().map(|(s, _)| String::from_utf8_lossy(s).into_owned()).collect(),
            context: cell.context,
        });
        let persistent = cell.persistent.iter().map(|&s| copy_child(s, cell.rebuild_precombines)).collect::<Result<_>>()?;
        let temporary = cell
            .temporary
            .iter()
            .map(|&s| copy_child(s, cell.rebuild_precombines))
            .chain(cell.navmeshes.iter().map(|&s| copy(s)))
            .collect::<Result<_>>()?;
        copied.push(CellLayout { record, persistent, temporary });
    }
    let mut copied: Vec<Option<CellLayout>> = copied.into_iter().map(Some).collect();
    for &position in &interiors {
        let cell = copied[position].take().unwrap();
        layout.interiors.insert(seed_cells[position].form_id, cell);
    }
    for (world, world_plan) in &worlds {
        let mut world_layout = WorldLayout { record: copy(world_sources[world])?, ..WorldLayout::default() };
        if let Some((_, cell, standing)) = &world_plan.persistent {
            world_layout.persistent = Some(CellLayout {
                record: copy(*cell)?,
                persistent: standing.iter().map(|&(s, rebuilt)| copy_child(s, rebuilt)).collect::<Result<_>>()?,
                temporary: Vec::new(),
            });
        }
        for &position in &world_plan.cells {
            let cell = copied[position].take().unwrap();
            let grid = cell_grid(&cell.record)?;
            world_layout.cells.insert((grid, seed_cells[position].form_id), cell);
        }
        layout.worlds.insert(seed_raw(*world)?, world_layout);
    }
    let records = write_plugin(&request.output, &masters, &layout, true)?;
    Ok(SeedReport {
        plugin: request.output.clone(),
        masters,
        cells: seed_cells,
        redated,
        skipped,
        records,
        seconds: started.elapsed().as_secs_f64(),
    })
}

/// Cuts the patch from the stamped seed.
pub fn write_patch(request: &PatchRequest) -> Result<PatchReport> {
    let seed = Plugin::open(&request.seed.plugin)?;
    let stem = |path: &Path| path.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase());
    if stem(&request.output) != stem(Path::new(&seed.name)) {
        return Err(PrevisError::invalid(format!("the patch must keep the seed's name, {}", seed.name)));
    }
    let (layout, report) = cut_patch(&seed, &request.seed, &request.output)?;
    write_plugin(&request.output, &seed.masters, &layout, request.esm)?;
    Ok(report)
}

#[derive(Debug, Deserialize)]
pub struct IntoPluginRequest {
    /// A seed written with `stand_in` for `target`, then stamped.
    pub seed: SeedReport,
    /// The plugin to stamp; it is read, never written.
    pub target: PathBuf,
    /// Where the stamped copy of `target` goes; it keeps the target's name.
    pub output: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct IntoPluginReport {
    pub plugin: PathBuf,
    /// CELLs the generator restamped, now stamped in the copy.
    pub cells: Vec<String>,
    /// CELLs the generator left as they were.
    pub unchanged: Vec<String>,
    /// CELLs the plugin did not override until now (a previs cluster's
    /// other members, mostly).
    pub added_cells: Vec<String>,
    /// References whose version-control date had to move to the stamp date.
    pub redated_children: usize,
    /// References of a master the plugin now overrides only to redate them.
    pub added_children: usize,
}

/// Writes the stamps of a seed that stood in for `target` back into a copy of
/// it. The plugin keeps its own CELL fields: only the precombine and previs
/// stamps change. References get the seed's dates, and the records the
/// plugin lacked (cluster CELLs, references to redate) join as overrides.
pub fn write_into_plugin(request: &IntoPluginRequest) -> Result<IntoPluginReport> {
    let seed = Plugin::open(&request.seed.plugin)?;
    let name = file_name(&request.target)?;
    if !name.eq_ignore_ascii_case(&seed.name) || !file_name(&request.output)?.eq_ignore_ascii_case(&name) {
        return Err(PrevisError::invalid(format!("the seed and the copy must both be named {name}")));
    }
    let (layout, cut) = cut_patch(&seed, &request.seed, &request.output)?;
    let target = std::fs::read(&request.target).map_err(|e| PrevisError::io(request.target.display().to_string(), e))?;
    let merged = crate::embed::merge(&target, &seed.masters, &seed.name, &layout)?;
    if let Some(parent) = request.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| PrevisError::io(parent.display().to_string(), e))?;
    }
    std::fs::write(&request.output, &merged.bytes).map_err(|e| PrevisError::io(request.output.display().to_string(), e))?;
    Ok(IntoPluginReport {
        plugin: request.output.clone(),
        cells: cut.cells,
        unchanged: cut.unchanged,
        added_cells: merged.added_cells.iter().map(|&id| seed_form_key(&seed, &request.seed, id)).collect(),
        redated_children: merged.redated_children,
        added_children: merged.added_children,
    })
}

fn seed_form_key(seed: &Plugin, report: &SeedReport, form_id: u32) -> String {
    report.cells.iter().find(|c| c.form_id == form_id).map_or_else(
        || {
            let (owner, object) = seed.owner_of(form_id);
            format!("{object:06X}:{owner}")
        },
        |c| c.form_key.clone(),
    )
}

fn cut_patch(seed: &Plugin, seed_report: &SeedReport, output: &Path) -> Result<(Layout, PatchReport)> {
    let redated: FxHashSet<u32> = seed_report.redated.iter().copied().collect();
    let placeholder_xpri = seed
        .raw_for("Fallout4.esm", DUMMY_XPRI_OBJECT)
        .ok_or_else(|| PrevisError::invalid("the seed does not master Fallout4.esm"))?;
    let mut report = PatchReport { plugin: output.to_path_buf(), masters: seed.masters.len(), cells: Vec::new(), unchanged: Vec::new(), forwarded_children: 0 };
    let mut layout = Layout::default();
    let children = |records: &[RecordRef]| -> Vec<Vec<u8>> {
        records.iter().filter(|r| redated.contains(&r.form_id)).map(|r| seed.raw_record(r).to_vec()).collect()
    };
    for cell in seed_report.cells.iter().filter(|c| !c.context) {
        let record = seed.record(cell.form_id).ok_or_else(|| PrevisError::invalid(format!("CELL {:08X} is not in the seed", cell.form_id)))?;
        let mut bytes = seed.raw_record(&record).to_vec();
        if stamp_digest(&bytes)? == cell.stamps {
            report.unchanged.push(cell.form_key.clone());
            continue;
        }
        let present: Vec<[u8; 4]> = SubrecordIter::new(&payload(&bytes)?).map(|(s, _)| s).collect();
        let mut fields = Vec::new();
        if cell.earlier_xcri && !present.contains(b"XCRI") {
            fields.push((*b"XCRI", vec![0; 8]));
        }
        if cell.earlier_xpri && !present.contains(b"XPRI") {
            fields.push((*b"XPRI", placeholder_xpri.to_le_bytes().to_vec()));
        }
        if !fields.is_empty() {
            bytes = set_fields(&bytes, &fields)?;
        }
        let topology = seed.cells.get(&cell.form_id).cloned().unwrap_or_default();
        let layout_cell = CellLayout { record: bytes, persistent: children(&topology.persistent), temporary: children(&topology.temporary) };
        report.forwarded_children += layout_cell.persistent.len() + layout_cell.temporary.len();
        report.cells.push(cell.form_key.clone());
        match topology.world {
            Some(world) => {
                let world_layout = layout.worlds.entry(world).or_default();
                let grid = cell_grid(&layout_cell.record)?;
                world_layout.cells.insert((grid, cell.form_id), layout_cell);
            }
            None => {
                layout.interiors.insert(cell.form_id, layout_cell);
            }
        }
    }
    for (&world, world_layout) in &mut layout.worlds {
        let record = seed.record(world).ok_or_else(|| PrevisError::invalid(format!("WRLD {world:08X} is not in the seed")))?;
        world_layout.record = seed.raw_record(&record).to_vec();
        let persistent_cell = seed.cells.iter().find(|(id, t)| t.world == Some(world) && !seed.world_cells.get(&world).is_some_and(|c| c.contains(id)));
        if let Some((&id, topology)) = persistent_cell {
            let standing = children(&topology.persistent);
            if !standing.is_empty() {
                report.forwarded_children += standing.len();
                let record = seed.record(id).unwrap();
                world_layout.persistent = Some(CellLayout { record: seed.raw_record(&record).to_vec(), persistent: standing, temporary: Vec::new() });
            }
        }
    }
    Ok((layout, report))
}

/// One CELL's copy plan and, for an exterior, its world and grid; `Err` says
/// why the CELL cannot be seeded.
fn plan_seed_cell(
    index: &LoadOrderIndex,
    base: &[bool],
    fixer: &[bool],
    id: GlobalId,
    form_key: &str,
    context: bool,
) -> Result<std::result::Result<(SeedCellPlan, Option<(GlobalId, (i32, i32))>), &'static str>> {
    let (Some(placement), Some(winner)) = (index.placement(id), index.winner(id).copied()) else {
        return Ok(Err("no CELL with this id"));
    };
    if index.is_esl(defining_plugin(id)) {
        return Ok(Err("defined in an ESL-flagged plugin, which can't have previsbines"));
    }
    if winner.record.flags & RECORD_FLAG_DELETED != 0 {
        return Ok(Err("the winning CELL is deleted"));
    }
    if placement.world.is_some() && !placement.exterior {
        return Ok(Err("a world's persistent CELL has no previsbines"));
    }
    let copies: Vec<Entry> = index.chain(id).iter().filter(|e| !is_partial(e)).copied().collect();
    let subrecords = index.plugins[winner.plugin].subrecords_at(&winner.record)?;
    let grid = match (placement.world, subrecords.first(b"XCLC").filter(|d| d.len() >= 8)) {
        (Some(world), Some(xclc)) => Some((world, (read_i32(xclc, 0).unwrap(), read_i32(xclc, 4).unwrap()))),
        (Some(_), None) => return Ok(Err("exterior CELL without a grid position")),
        (None, _) => None,
    };
    let interior = subrecords.first(b"DATA").and_then(|d| d.first()).is_some_and(|flags| *flags as u16 & CELL_FLAG_INTERIOR != 0);
    let has = |signature: &[u8; 4]| {
        copies.iter().any(|e| index.plugins[e.plugin].subrecords_at(&e.record).is_ok_and(|s| s.first(signature).is_some()))
    };
    let mut forwarded = Vec::new();
    for regression in field_regressions(index, base, fixer, &copies, interior) {
        let copy = &copies[regression.best];
        let data = index.plugins[copy.plugin].subrecords_at(&copy.record)?.first(&regression.signature).map(<[u8]>::to_vec).unwrap_or_default();
        forwarded.push(match (regression.form, read_u32(&data, 0)) {
            (true, Some(raw)) => Forwarded::Form(regression.signature, index.resolve(copy.plugin, raw)),
            _ => Forwarded::Raw(regression.signature, data),
        });
    }
    if !interior {
        let lost: Vec<GlobalId> = lost_regions(index, base, &copies).into_iter().flat_map(|(_, lost)| lost).collect();
        if !lost.is_empty() {
            let mut regions: Vec<GlobalId> = subrecords
                .first(b"XCLR")
                .map(|d| d.chunks_exact(4).map(|c| index.resolve(winner.plugin, u32::from_le_bytes(c.try_into().unwrap()))).collect())
                .unwrap_or_default();
            for region in lost {
                if !regions.contains(&region) {
                    regions.push(region);
                }
            }
            forwarded.push(Forwarded::Regions(regions));
        }
    }
    let (mut temporary, mut persistent) = (Vec::new(), Vec::new());
    for &child in index.children(id) {
        let Some(entry) = index.winner(child).filter(|e| e.home == Some(id)) else { continue };
        let source = Source { plugin: entry.plugin, record: entry.record };
        if entry.persistent { persistent.push(source) } else { temporary.push(source) }
    }
    // Interior previs seeds its camera from the navmesh, which the index skips:
    // the last copy of each wins.
    let mut navmeshes: Vec<(GlobalId, Source)> = Vec::new();
    for entry in copies.iter().filter(|_| interior) {
        let Some(topology) = index.plugins[entry.plugin].cells.get(&entry.record.form_id) else { continue };
        for record in topology.temporary.iter().chain(&topology.persistent).filter(|r| &r.signature == b"NAVM") {
            let (key, source) = (index.resolve(entry.plugin, record.form_id), Source { plugin: entry.plugin, record: *record });
            match navmeshes.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = source,
                None => navmeshes.push((key, source)),
            }
        }
    }
    let plan = SeedCellPlan {
        form_key: form_key.to_string(),
        source: Source { plugin: winner.plugin, record: winner.record },
        forwarded,
        temporary,
        persistent,
        navmeshes: navmeshes.into_iter().map(|(_, source)| source).collect(),
        earlier_xcri: has(b"XCRI"),
        earlier_xpri: has(b"XPRI"),
        context,
        rebuild_precombines: false,
    };
    Ok(Ok((plan, grid)))
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| PrevisError::invalid(format!("{} has no file name", path.display())))
}

fn parse_form_key(form_key: &str, by_name: &FxHashMap<String, usize>) -> Option<GlobalId> {
    let (object, plugin) = form_key.split_once(':')?;
    let object = u32::from_str_radix(object.trim(), 16).ok()?;
    let plugin = *by_name.get(&plugin.trim().to_ascii_lowercase())?;
    Some(global_id(plugin, object))
}

fn form_key_of(index: &LoadOrderIndex, id: GlobalId) -> String {
    let plugin = index.plugins.get(defining_plugin(id)).map_or("?", |p| p.name.as_str());
    format!("{:06X}:{plugin}", object_id(id))
}

fn references(index: &LoadOrderIndex, source: Source) -> Result<Vec<GlobalId>> {
    let plugin = &index.plugins[source.plugin];
    let raws = record_form_ids(plugin.raw_record(&source.record), GAME)
        .map_err(|e| PrevisError::invalid(format!("{:08X} in {}: {e}", source.record.form_id, plugin.name)))?;
    Ok(raws.into_iter().map(|raw| index.resolve(source.plugin, raw)).collect())
}

pub(crate) fn payload(record: &[u8]) -> Result<Vec<u8>> {
    let body = &record[HEADER_SIZE..];
    let flags = u32::from_le_bytes(record[8..12].try_into().unwrap());
    if flags & COMPRESSED_FLAG == 0 {
        return Ok(body.to_vec());
    }
    let expected = read_u32(body, 0).ok_or_else(|| PrevisError::invalid("compressed record is truncated"))? as usize;
    let mut decoded = Vec::with_capacity(expected);
    flate2::read::ZlibDecoder::new(&body[4..]).read_to_end(&mut decoded).map_err(|e| PrevisError::io("decompress record", e))?;
    Ok(decoded)
}

pub(crate) fn with_payload(record: &[u8], decoded: Vec<u8>) -> Result<Vec<u8>> {
    let flags = u32::from_le_bytes(record[8..12].try_into().unwrap());
    let body = if flags & COMPRESSED_FLAG == 0 {
        decoded
    } else {
        let mut compressed = (decoded.len() as u32).to_le_bytes().to_vec();
        let mut encoder = flate2::write::ZlibEncoder::new(&mut compressed, flate2::Compression::default());
        encoder.write_all(&decoded).and_then(|_| encoder.finish().map(|_| ())).map_err(|e| PrevisError::io("compress record", e))?;
        compressed
    };
    let mut out = record[..HEADER_SIZE].to_vec();
    out[4..8].copy_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Replaces each field's first occurrence, or appends it. The runtime reads
/// CELL fields by signature, so an appended one loads like any other.
fn set_fields(record: &[u8], fields: &[([u8; 4], Vec<u8>)]) -> Result<Vec<u8>> {
    if fields.is_empty() {
        return Ok(record.to_vec());
    }
    let decoded = payload(record)?;
    let mut subrecords: Vec<([u8; 4], Vec<u8>)> = SubrecordIter::new(&decoded).map(|(s, d)| (s, d.to_vec())).collect();
    for (signature, data) in fields {
        match subrecords.iter_mut().find(|(s, _)| s == signature) {
            Some(existing) => existing.1 = data.clone(),
            None => subrecords.push((*signature, data.clone())),
        }
    }
    let mut encoded = Vec::with_capacity(decoded.len() + 64);
    for (signature, data) in &subrecords {
        encode_subrecord(&mut encoded, signature, data);
    }
    with_payload(record, encoded)
}

fn stamp_digest(record: &[u8]) -> Result<String> {
    let decoded = payload(record)?;
    let mut stamps = Vec::new();
    for (signature, data) in SubrecordIter::new(&decoded).filter(|(s, _)| REPLACED.contains(&s)) {
        encode_subrecord(&mut stamps, &signature, data);
    }
    Ok(crate::sha256_hex(&stamps))
}

pub(crate) fn cell_grid(record: &[u8]) -> Result<(i32, i32)> {
    let decoded = payload(record)?;
    SubrecordIter::new(&decoded)
        .find(|(s, _)| s == b"XCLC")
        .and_then(|(_, d)| Some((read_i32(d, 0)?, read_i32(d, 4)?)))
        .ok_or_else(|| PrevisError::invalid("exterior CELL without XCLC"))
}

#[derive(Default)]
pub(crate) struct CellLayout {
    pub(crate) record: Vec<u8>,
    pub(crate) persistent: Vec<Vec<u8>>,
    pub(crate) temporary: Vec<Vec<u8>>,
}

#[derive(Default)]
pub(crate) struct WorldLayout {
    pub(crate) record: Vec<u8>,
    pub(crate) persistent: Option<CellLayout>,
    /// Keyed by grid then FormID, which also orders the block groups.
    pub(crate) cells: BTreeMap<((i32, i32), u32), CellLayout>,
}

#[derive(Default)]
pub(crate) struct Layout {
    /// Top-level records outside CELL and WRLD, by signature.
    pub(crate) records: BTreeMap<[u8; 4], Vec<Vec<u8>>>,
    pub(crate) interiors: BTreeMap<u32, CellLayout>,
    pub(crate) worlds: BTreeMap<u32, WorldLayout>,
}

pub(crate) const GROUP_TOP: i32 = 0;
pub(crate) const GROUP_WORLD_CHILDREN: i32 = 1;
pub(crate) const GROUP_INTERIOR_BLOCK: i32 = 2;
pub(crate) const GROUP_INTERIOR_SUB_BLOCK: i32 = 3;
pub(crate) const GROUP_EXTERIOR_BLOCK: i32 = 4;
pub(crate) const GROUP_EXTERIOR_SUB_BLOCK: i32 = 5;
pub(crate) const GROUP_CELL_CHILDREN: i32 = 6;
pub(crate) const GROUP_CELL_PERSISTENT: i32 = 8;
pub(crate) const GROUP_CELL_TEMPORARY: i32 = 9;

struct Writer {
    out: Vec<u8>,
    open: Vec<usize>,
    records: usize,
}

impl Writer {
    fn begin(&mut self, label: [u8; 4], group_type: i32) {
        self.open.push(self.out.len());
        self.out.extend_from_slice(b"GRUP");
        self.out.extend_from_slice(&[0; 4]);
        self.out.extend_from_slice(&label);
        self.out.extend_from_slice(&group_type.to_le_bytes());
        self.out.extend_from_slice(&[0; 8]);
        self.records += 1;
    }

    fn end(&mut self) {
        let start = self.open.pop().unwrap();
        let size = (self.out.len() - start) as u32;
        self.out[start + 4..start + 8].copy_from_slice(&size.to_le_bytes());
    }

    fn record(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
        self.records += 1;
    }

    fn cell(&mut self, form_id: u32, cell: &CellLayout) {
        self.record(&cell.record);
        if cell.persistent.is_empty() && cell.temporary.is_empty() {
            return;
        }
        self.begin(form_id.to_le_bytes(), GROUP_CELL_CHILDREN);
        for (records, group_type) in [(&cell.persistent, GROUP_CELL_PERSISTENT), (&cell.temporary, GROUP_CELL_TEMPORARY)] {
            if records.is_empty() {
                continue;
            }
            self.begin(form_id.to_le_bytes(), group_type);
            for record in records {
                self.record(record);
            }
            self.end();
        }
        self.end();
    }
}

pub(crate) fn grid_label(x: i32, y: i32) -> [u8; 4] {
    let mut label = [0; 4];
    label[..2].copy_from_slice(&(y as i16).to_le_bytes());
    label[2..].copy_from_slice(&(x as i16).to_le_bytes());
    label
}

/// Writes a full plugin: TES4, then the CELL and WRLD top groups in the
/// interior block / exterior block layout the game and CK expect. An ESM
/// lists the masters' temporary references it overrides in ONAM, without
/// which the game skips those overrides.
fn write_plugin(path: &Path, masters: &[String], layout: &Layout, esm: bool) -> Result<usize> {
    let mut overridden: Vec<u32> = Vec::new();
    if esm {
        let cells = layout.interiors.values().chain(layout.worlds.values().flat_map(|w| w.cells.values()));
        for record in cells.flat_map(|c| &c.temporary) {
            let form_id = u32::from_le_bytes(record[12..16].try_into().unwrap());
            if ((form_id >> 24) as usize) < masters.len() {
                overridden.push(form_id);
            }
        }
    }
    let mut writer = Writer { out: Vec::new(), open: Vec::new(), records: 0 };
    for (signature, records) in &layout.records {
        writer.begin(*signature, GROUP_TOP);
        for record in records {
            writer.record(record);
        }
        writer.end();
    }
    if !layout.interiors.is_empty() {
        writer.begin(*b"CELL", GROUP_TOP);
        let mut blocks: BTreeMap<(u32, u32), Vec<(u32, &CellLayout)>> = BTreeMap::new();
        for (&form_id, cell) in &layout.interiors {
            let object = form_id & 0x00FF_FFFF;
            blocks.entry((object % 10, (object / 10) % 10)).or_default().push((form_id, cell));
        }
        let mut current_block = None;
        for ((block, sub_block), cells) in &blocks {
            if current_block != Some(*block) {
                if current_block.is_some() {
                    writer.end();
                }
                writer.begin((*block as i32).to_le_bytes(), GROUP_INTERIOR_BLOCK);
                current_block = Some(*block);
            }
            writer.begin((*sub_block as i32).to_le_bytes(), GROUP_INTERIOR_SUB_BLOCK);
            for (form_id, cell) in cells {
                writer.cell(*form_id, cell);
            }
            writer.end();
        }
        writer.end();
        writer.end();
    }
    if !layout.worlds.is_empty() {
        writer.begin(*b"WRLD", GROUP_TOP);
        for (&world, world_layout) in &layout.worlds {
            writer.record(&world_layout.record);
            writer.begin(world.to_le_bytes(), GROUP_WORLD_CHILDREN);
            if let Some(persistent) = &world_layout.persistent {
                let form_id = u32::from_le_bytes(persistent.record[12..16].try_into().unwrap());
                writer.cell(form_id, persistent);
            }
            let mut blocks: BTreeMap<(i32, i32), BTreeMap<(i32, i32), Vec<(u32, &CellLayout)>>> = BTreeMap::new();
            for (&((x, y), form_id), cell) in &world_layout.cells {
                blocks
                    .entry((x.div_euclid(32), y.div_euclid(32)))
                    .or_default()
                    .entry((x.div_euclid(8), y.div_euclid(8)))
                    .or_default()
                    .push((form_id, cell));
            }
            for ((bx, by), sub_blocks) in &blocks {
                writer.begin(grid_label(*bx, *by), GROUP_EXTERIOR_BLOCK);
                for ((sx, sy), cells) in sub_blocks {
                    writer.begin(grid_label(*sx, *sy), GROUP_EXTERIOR_SUB_BLOCK);
                    for (form_id, cell) in cells {
                        writer.cell(*form_id, cell);
                    }
                    writer.end();
                }
                writer.end();
            }
            writer.end();
        }
        writer.end();
    }

    let mut header = Vec::new();
    let mut hedr = 1.0f32.to_le_bytes().to_vec();
    hedr.extend_from_slice(&(writer.records as u32).to_le_bytes());
    hedr.extend_from_slice(&0x800u32.to_le_bytes());
    encode_subrecord(&mut header, b"HEDR", &hedr);
    encode_subrecord(&mut header, b"CNAM", b"ModBox21\0");
    for master in masters {
        let mut name = master.as_bytes().to_vec();
        name.push(0);
        encode_subrecord(&mut header, b"MAST", &name);
        encode_subrecord(&mut header, b"DATA", &[0; 8]);
    }
    if !overridden.is_empty() {
        encode_subrecord(&mut header, b"ONAM", &overridden.iter().flat_map(|id| id.to_le_bytes()).collect::<Vec<u8>>());
    }
    let mut file = Vec::with_capacity(HEADER_SIZE + header.len() + writer.out.len());
    file.extend_from_slice(b"TES4");
    file.extend_from_slice(&(header.len() as u32).to_le_bytes());
    file.extend_from_slice(&(if esm { PLUGIN_FLAG_ESM } else { 0 }).to_le_bytes());
    file.extend_from_slice(&[0; 8]);
    file.extend_from_slice(&FORM_VERSION.to_le_bytes());
    file.extend_from_slice(&[0; 2]);
    file.extend_from_slice(&header);
    file.extend_from_slice(&writer.out);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| PrevisError::io(parent.display().to_string(), e))?;
    }
    std::fs::write(path, file).map_err(|e| PrevisError::io(path.display().to_string(), e))?;
    Ok(writer.records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_order::PLUGIN_FLAG_ESM as ESM;
    use crate::load_order::tests::{PluginBuilder, group, interior_cell, record, reference, subrecord};
    use crate::metadata::{CellMetadata, Xcri, write_patched_plugin};

    const DATE: u16 = (26 << 9) | (10 << 5) | 11;

    fn stat(form_id: u32, model: &str) -> Vec<u8> {
        let mut path = model.as_bytes().to_vec();
        path.push(0);
        group(u32::from_le_bytes(*b"STAT"), 0, &[record(b"STAT", form_id, 0, 10, &[subrecord(b"EDID", b"Thing\0"), subrecord(b"MODL", &path)])])
    }

    fn xcri(reference: u32, mesh: u32) -> Vec<u8> {
        [1u32, 2, mesh, reference, mesh].iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn field(plugin: &Plugin, form_id: u32, signature: &[u8; 4]) -> Option<Vec<u8>> {
        let record = plugin.record(form_id).unwrap();
        plugin.subrecords_at(&record).unwrap().first(signature).map(<[u8]>::to_vec)
    }

    #[test]
    fn seed_copies_merged_cell_and_patch_keeps_only_restamped_cells() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        let game = PluginBuilder::new(ESM, &[])
            .push(stat(0x900, "Old.nif"))
            .push(interior_cell(0x800, 0, 10, &[subrecord(b"XCLL", b"AAAA"), subrecord(b"XCRI", &xcri(0x801, 5))], &[reference(0x801, 0, 10, 0x900)]))
            .push(interior_cell(0x810, 0, 10, &[], &[]))
            .write(dir.path(), "Fallout4.esm");
        let unrelated = PluginBuilder::new(0, &["Fallout4.esm"]).write(dir.path(), "Unrelated.esp");
        let library = PluginBuilder::new(0, &["Fallout4.esm"]).push(stat(0x900, "New.nif")).write(dir.path(), "Lib.esp");
        let modded = PluginBuilder::new(0, &["Fallout4.esm", "Unrelated.esp", "Lib.esp"])
            .push(interior_cell(0x800, 0, 20, &[subrecord(b"XCLL", b"BBBB")], &[reference(0x801, 0, 0, 0x900), reference(0x0300_0802, 0, 0, 0x900)]))
            .write(dir.path(), "Mod.esp");
        let revert = PluginBuilder::new(0, &["Fallout4.esm"])
            .push(interior_cell(0x800, 0, 30, &[subrecord(b"XCLL", b"AAAA"), subrecord(b"XPRI", &0x801u32.to_le_bytes())], &[]))
            .push(interior_cell(0x810, 0, 30, &[], &[]))
            .write(dir.path(), "Revert.esp");
        let request = SeedRequest {
            plugins: vec![game, unrelated, library, modded, revert],
            base_plugins: vec!["Fallout4.esm".into()],
            fixer_plugins: vec![],
            cells: vec!["000800:Fallout4.esm".into(), "000810:Fallout4.esm".into(), "123456:Nowhere.esp".into()],
            precombine_cells: vec!["000800:Fallout4.esm".into(), "000810:Fallout4.esm".into()],
            output: work.join("Repair.esm"),
            date: DATE,
            stand_in: false,
            required_masters: vec![],
        };
        let seed = write_seed(&request).unwrap();
        assert_eq!(seed.masters, ["Fallout4.esm", "Lib.esp", "Mod.esp", "Revert.esp"], "the STAT override's file is a master; unused masters are not");
        assert_eq!(seed.skipped.len(), 1);
        assert_eq!(seed.cells[0].forwarded_fields, ["XCLL"]);
        assert!(seed.cells[0].earlier_xcri && seed.cells[0].earlier_xpri);
        let plugin = Plugin::open(&seed.plugin).unwrap();
        assert_eq!(field(&plugin, 0x800, b"XCLL").unwrap(), b"BBBB", "the reverted XCLL is forwarded");
        let children = &plugin.cells[&0x800].temporary;
        assert_eq!(children.iter().map(|r| r.form_id).collect::<Vec<_>>(), [0x801, 0x0200_0802], "Mod.esp's own reference is renumbered");
        let mut redated = seed.redated.clone();
        redated.sort();
        assert_eq!(redated, [0x801, 0x0200_0802]);
        assert!(children.iter().all(|r| crate::load_order::version_date(&plugin, r) == DATE));

        // The generator restamps 800 (precombines only, so XPRI goes) and leaves 810.
        let metadata = CellMetadata { precombine: true, xcri: Some(Xcri { mesh_ids: vec![7], reference_mesh_pairs: vec![(0x801, 7)] }), ..CellMetadata::default() };
        let stamped = work.join("stamped.esm");
        write_patched_plugin(&plugin, &stamped, &FxHashMap::from_iter([(0x800, metadata)]), DATE).unwrap();
        drop(plugin);
        std::fs::rename(&stamped, &seed.plugin).unwrap();

        let output = dir.path().join("out").join("Repair.esm");
        let report = write_patch(&PatchRequest { seed, output: output.clone(), esm: true }).unwrap();
        assert_eq!(report.cells, ["000800:Fallout4.esm"]);
        assert_eq!(report.unchanged, ["000810:Fallout4.esm"]);
        assert_eq!(report.forwarded_children, 2);
        let patch = Plugin::open(&output).unwrap();
        assert_eq!(patch.masters, ["Fallout4.esm", "Lib.esp", "Mod.esp", "Revert.esp"]);
        assert!(patch.record(0x810).is_none());
        assert_eq!(field(&patch, 0x800, b"XPRI").unwrap(), 0x001C_26C2u32.to_le_bytes(), "an earlier XPRI must not show through");
        assert_eq!(field(&patch, 0x800, b"PCMB").unwrap(), DATE.to_le_bytes());
        let order = ["Fallout4.esm", "Unrelated.esp", "Lib.esp", "Mod.esp", "Revert.esp"].map(|name| dir.path().join(name));
        let mut paths = order.to_vec();
        paths.push(output);
        let index = LoadOrderIndex::open(&paths).unwrap();
        assert_eq!(index.winner(global_id(0, 0x800)).unwrap().plugin, 5);
        assert_eq!(index.winner(global_id(3, 0x802)).unwrap().plugin, 5);
    }

    /// A previs-only CELL keeps its PCMB, so restamping its undated reference
    /// would make the game drop its precombines.
    #[test]
    fn only_references_of_rebuilt_precombines_are_restamped() {
        let dir = tempfile::tempdir().unwrap();
        let game = PluginBuilder::new(ESM, &[])
            .push(stat(0x900, "A.nif"))
            .push(interior_cell(0x800, 0, 10, &[], &[reference(0x801, 0, 0, 0x900)]))
            .write(dir.path(), "Fallout4.esm");
        let seed = |precombine_cells: Vec<String>, name: &str| {
            write_seed(&SeedRequest {
                plugins: vec![game.clone()],
                base_plugins: vec![],
                fixer_plugins: vec![],
                cells: vec!["000800:Fallout4.esm".into()],
                precombine_cells,
                output: dir.path().join(name).join("Repair.esm"),
                date: DATE,
                stand_in: false,
                required_masters: vec![],
            })
            .unwrap()
        };
        assert!(seed(vec![], "previs").redated.is_empty());
        assert_eq!(seed(vec!["000800:Fallout4.esm".into()], "precombines").redated, [0x801]);
    }

    #[test]
    fn stamp_references_are_renumbered_into_the_seed() {
        let dir = tempfile::tempdir().unwrap();
        let game = PluginBuilder::new(ESM, &[]).push(stat(0x900, "A.nif")).push(interior_cell(0x800, 0, 10, &[], &[])).write(dir.path(), "Fallout4.esm");
        let unrelated = PluginBuilder::new(0, &["Fallout4.esm"]).write(dir.path(), "Unrelated.esp");
        let own = 0x0200_0802u32;
        let modded = PluginBuilder::new(0, &["Fallout4.esm", "Unrelated.esp"])
            .push(interior_cell(0x800, 0, 20, &[subrecord(b"XPRI", &own.to_le_bytes()), subrecord(b"XCRI", &xcri(own, 5))], &[reference(own, 0, 20, 0x900)]))
            .write(dir.path(), "Mod.esp");
        let seed = write_seed(&SeedRequest {
            plugins: vec![game, unrelated, modded],
            base_plugins: vec![],
            fixer_plugins: vec![],
            cells: vec!["000800:Fallout4.esm".into()],
            precombine_cells: vec!["000800:Fallout4.esm".into()],
            output: dir.path().join("work").join("Repair.esm"),
            date: DATE,
            stand_in: false,
            required_masters: vec![],
        })
        .unwrap();
        assert_eq!(seed.masters, ["Fallout4.esm", "Mod.esp"]);
        let plugin = Plugin::open(&seed.plugin).unwrap();
        assert_eq!(field(&plugin, 0x800, b"XPRI").unwrap(), 0x0100_0802u32.to_le_bytes());
        assert_eq!(field(&plugin, 0x800, b"XCRI").unwrap(), xcri(0x0100_0802, 5));
    }

    fn header_of(path: &Path) -> (u32, Vec<u32>) {
        let bytes = std::fs::read(path).unwrap();
        let flags = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let onam = SubrecordIter::new(&bytes[HEADER_SIZE..HEADER_SIZE + size])
            .find(|(s, _)| s == b"ONAM")
            .map(|(_, d)| d.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
            .unwrap_or_default();
        (flags, onam)
    }

    #[test]
    fn an_esm_patch_lists_the_temporary_references_it_overrides_and_an_esp_needs_none() {
        let dir = tempfile::tempdir().unwrap();
        let game = PluginBuilder::new(ESM, &[])
            .push(stat(0x900, "A.nif"))
            .push(interior_cell(0x800, 0, 10, &[], &[reference(0x801, 0, 0, 0x900)]))
            .write(dir.path(), "Fallout4.esm");
        let seed = write_seed(&SeedRequest {
            plugins: vec![game],
            base_plugins: vec![],
            fixer_plugins: vec![],
            cells: vec!["000800:Fallout4.esm".into()],
            precombine_cells: vec!["000800:Fallout4.esm".into()],
            output: dir.path().join("work").join("Repair.esm"),
            date: DATE,
            stand_in: false,
            required_masters: vec![],
        })
        .unwrap();
        stamp(&seed, &[(0x800, 0x801)]);
        let report = serde_json::to_string(&seed).unwrap();
        let esm = dir.path().join("esm").join("Repair.esm");
        write_patch(&PatchRequest { seed: serde_json::from_str(&report).unwrap(), output: esm.clone(), esm: true }).unwrap();
        assert_eq!(header_of(&esm), (ESM, vec![0x801]), "without ONAM the game skips an ESM's temporary reference overrides");
        let esp = dir.path().join("esp").join("Repair.esp");
        write_patch(&PatchRequest { seed, output: esp.clone(), esm: false }).unwrap();
        assert_eq!(header_of(&esp), (0, vec![]));
    }

    #[test]
    fn required_masters_are_kept_even_when_nothing_is_copied_from_them() {
        let dir = tempfile::tempdir().unwrap();
        let game = PluginBuilder::new(ESM, &[]).push(stat(0x900, "A.nif")).push(interior_cell(0x800, 0, 10, &[], &[])).write(dir.path(), "Fallout4.esm");
        let framework = PluginBuilder::new(ESM, &["Fallout4.esm"]).write(dir.path(), "PPF.esm");
        let modded = PluginBuilder::new(0, &["Fallout4.esm"]).push(interior_cell(0x800, 0, 20, &[], &[])).write(dir.path(), "Mod.esp");
        let request = |required: Vec<String>| SeedRequest {
            plugins: vec![game.clone(), framework.clone(), modded.clone()],
            base_plugins: vec!["Fallout4.esm".into(), "PPF.esm".into()],
            fixer_plugins: vec![],
            cells: vec!["000800:Fallout4.esm".into()],
            precombine_cells: vec!["000800:Fallout4.esm".into()],
            output: dir.path().join("work").join("Mod - PPF Patch.esm"),
            date: DATE,
            stand_in: false,
            required_masters: required,
        };
        assert_eq!(write_seed(&request(vec![])).unwrap().masters, ["Fallout4.esm", "Mod.esp"]);
        let seed = write_seed(&request(vec!["ppf.esm".into(), "Mod.esp".into()])).unwrap();
        assert_eq!(seed.masters, ["Fallout4.esm", "PPF.esm", "Mod.esp"], "in load order");
        assert!(write_seed(&request(vec!["Elsewhere.esm".into()])).is_err());
    }

    /// Stamps a seed's CELLs with one precombine group of `reference`.
    fn stamp(seed: &SeedReport, cells: &[(u32, u32)]) {
        let plugin = Plugin::open(&seed.plugin).unwrap();
        let metadata = cells.iter().map(|&(cell, reference)| {
            (cell, CellMetadata { precombine: true, xcri: Some(Xcri { mesh_ids: vec![7], reference_mesh_pairs: vec![(reference, 7)] }), ..CellMetadata::default() })
        });
        let stamped = seed.plugin.with_extension("stamped");
        write_patched_plugin(&plugin, &stamped, &FxHashMap::from_iter(metadata), DATE).unwrap();
        drop(plugin);
        std::fs::rename(&stamped, &seed.plugin).unwrap();
    }

    /// Interior previs seeds its camera from the navmesh, which load-order
    /// detection never indexes.
    #[test]
    fn interior_navmeshes_ride_in_the_seed_but_never_in_the_patch() {
        let dir = tempfile::tempdir().unwrap();
        let navmesh = |date: u16, z: f32| {
            let mut nvnm: Vec<u8> = [15u32, 0, 0, 0x800, 3].iter().flat_map(|v| v.to_le_bytes()).collect();
            nvnm.extend([0.0f32, 0.0, z, 30.0, 0.0, z, 0.0, 30.0, z].iter().flat_map(|v| v.to_le_bytes()));
            nvnm.extend(1u32.to_le_bytes());
            nvnm.extend([0u16, 1, 2].iter().flat_map(|v| v.to_le_bytes()));
            nvnm.extend([0u8; 15]);
            record(b"NAVM", 0x802, 0, date, &[subrecord(b"NVNM", &nvnm)])
        };
        let game = PluginBuilder::new(ESM, &[])
            .push(stat(0x900, "A.nif"))
            .push(interior_cell(0x800, 0, 10, &[], &[reference(0x801, 0, 0, 0x900), navmesh(10, 0.0)]))
            .write(dir.path(), "Fallout4.esm");
        let modded = PluginBuilder::new(0, &["Fallout4.esm"]).push(interior_cell(0x800, 0, 20, &[], &[navmesh(20, 50.0)])).write(dir.path(), "Mod.esp");
        let seed = write_seed(&SeedRequest {
            plugins: vec![game, modded],
            base_plugins: vec![],
            fixer_plugins: vec![],
            cells: vec!["000800:Fallout4.esm".into()],
            precombine_cells: vec!["000800:Fallout4.esm".into()],
            output: dir.path().join("work").join("Repair.esm"),
            date: DATE,
            stand_in: false,
            required_masters: vec![],
        })
        .unwrap();
        let plugin = Plugin::open(&seed.plugin).unwrap();
        let camera = crate::previs::interior_camera(&plugin, 0x800).unwrap();
        assert_eq!(camera.seeds, vec![[10.0, 10.0, 50.0 + 118.0]], "the winning navmesh seeds the camera");
        assert!(!seed.redated.contains(&0x802), "a navmesh is never restamped");
        drop(plugin);
        stamp(&seed, &[(0x800, 0x801)]);
        let output = dir.path().join("out").join("Repair.esm");
        write_patch(&PatchRequest { seed, output: output.clone(), esm: true }).unwrap();
        let patch = Plugin::open(&output).unwrap();
        assert!(patch.cells[&0x800].temporary.iter().all(|r| &r.signature != b"NAVM"), "the patch never overrides a navmesh");
    }

    #[test]
    fn a_seed_standing_in_for_a_plugin_is_written_back_into_a_copy_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let game = PluginBuilder::new(ESM, &[])
            .push(stat(0x900, "A.nif"))
            .push(interior_cell(0x800, 0, 10, &[], &[reference(0x801, 0, 0, 0x900)]))
            .write(dir.path(), "Fallout4.esm");
        let target = PluginBuilder::new(0, &["Fallout4.esm"])
            .push(stat(0x0100_0950, "Own.nif"))
            .push(interior_cell(0x800, 0, 20, &[subrecord(b"XCLL", b"MINE")], &[reference(0x0100_0802, 0, 0, 0x900)]))
            .push(interior_cell(0x0100_0A00, 0, 20, &[subrecord(b"EDID", b"MyRoom\0")], &[reference(0x0100_0A01, 0, 0, 0x0100_0950)]))
            .write(dir.path(), "Mod.esp");
        let before = std::fs::read(&target).unwrap();
        let seed = write_seed(&SeedRequest {
            plugins: vec![game.clone(), target.clone()],
            base_plugins: vec!["Fallout4.esm".into()],
            fixer_plugins: vec![],
            cells: vec!["000800:Fallout4.esm".into(), "000A00:Mod.esp".into()],
            precombine_cells: vec!["000800:Fallout4.esm".into(), "000A00:Mod.esp".into()],
            output: dir.path().join("work").join("Mod.esp"),
            date: DATE,
            stand_in: true,
            required_masters: vec![],
        })
        .unwrap();
        assert_eq!(seed.masters, ["Fallout4.esm"], "the plugin a seed stands in for is never its own master");
        let standing_in = Plugin::open(&seed.plugin).unwrap();
        assert_eq!(field(&standing_in, 0x0100_0950, b"MODL").unwrap(), b"Own.nif\0", "the seed replaces Mod.esp, so it carries the bases Mod.esp places");
        assert!(standing_in.record(0x900).is_none(), "a master's base resolves through the master");
        drop(standing_in);
        stamp(&seed, &[(0x800, 0x0100_0802), (0x0100_0A00, 0x0100_0A01)]);

        let output = dir.path().join("out").join("Mod.esp");
        let report = write_into_plugin(&IntoPluginRequest { seed, target: target.clone(), output: output.clone() }).unwrap();
        assert_eq!(report.cells, ["000800:Fallout4.esm", "000A00:Mod.esp"]);
        assert_eq!(report.added_cells, Vec::<String>::new());
        assert_eq!(std::fs::read(&target).unwrap(), before, "the original plugin is never written");

        let written = Plugin::open(&output).unwrap();
        assert_eq!(written.masters, ["Fallout4.esm"]);
        assert!(written.record(0x0100_0950).is_some(), "records outside the CELLs are kept");
        for cell in [0x800, 0x0100_0A00] {
            assert_eq!(field(&written, cell, b"PCMB").unwrap(), DATE.to_le_bytes());
        }
        assert_eq!(field(&written, 0x800, b"XCLL").unwrap(), b"MINE", "the plugin's own CELL fields stay");
        assert_eq!(field(&written, 0x0100_0A00, b"XCRI").unwrap(), xcri(0x0100_0A01, 7));
        let dates = |cell: u32| -> Vec<(u32, u16)> {
            written.cells[&cell].temporary.iter().map(|r| (r.form_id, crate::load_order::version_date(&written, r))).collect()
        };
        assert_eq!(dates(0x800), [(0x0100_0802, DATE), (0x801, DATE)], "the master's undated reference joins as a dated override");
        assert_eq!(dates(0x0100_0A00), [(0x0100_0A01, DATE)]);
        let index = LoadOrderIndex::open(&[game, output]).unwrap();
        assert_eq!(index.winner(global_id(0, 0x800)).unwrap().plugin, 1);
    }

    #[test]
    fn cluster_cells_a_plugin_lacks_join_its_world_in_their_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let grid_cell = |form_id: u32, x: i32, date: u16| {
            let mut xclc = x.to_le_bytes().to_vec();
            xclc.extend_from_slice(&0i32.to_le_bytes());
            xclc.push(0);
            record(b"CELL", form_id, 0, date, &[subrecord(b"DATA", &0u16.to_le_bytes()), subrecord(b"XCLC", &xclc)])
        };
        let mut placed = 50.0f32.to_le_bytes().to_vec();
        placed.extend_from_slice(&[0; 20]);
        let standing = record(b"REFR", 0x710, 0x400, 0, &[subrecord(b"NAME", &0x900u32.to_le_bytes()), subrecord(b"DATA", &placed)]);
        let world = group(u32::from_le_bytes(*b"WRLD"), 0, &[
            record(b"WRLD", 0x700, 0, 10, &[subrecord(b"EDID", b"World\0")]),
            group(0x700, 1, &[
                record(b"CELL", 0x701, 0x400, 10, &[subrecord(b"DATA", &2u16.to_le_bytes())]),
                group(0x701, 6, &[group(0x701, 8, &[standing])]),
                group(0, 4, &[group(0, 5, &[grid_cell(0x720, 0, 10), grid_cell(0x730, 1, 10)])]),
                group(grid_label(1, 0).map(|b| b as u32).iter().rev().fold(0, |a, &b| (a << 8) | b), 4, &[group(u32::from_le_bytes(grid_label(5, 0)), 5, &[grid_cell(0x740, 40, 10)])]),
            ]),
        ]);
        let game = PluginBuilder::new(ESM, &[]).push(stat(0x900, "A.nif")).push(world).write(dir.path(), "Fallout4.esm");
        let target = PluginBuilder::new(0, &["Fallout4.esm"])
            .push(group(u32::from_le_bytes(*b"WRLD"), 0, &[
                record(b"WRLD", 0x700, 0, 20, &[subrecord(b"EDID", b"World\0")]),
                group(0x700, 1, &[group(0, 4, &[group(0, 5, &[grid_cell(0x720, 0, 20)])])]),
            ]))
            .write(dir.path(), "Mod.esp");
        let cells = vec!["000720:Fallout4.esm".to_string(), "000730:Fallout4.esm".into(), "000740:Fallout4.esm".into()];
        let seed = write_seed(&SeedRequest {
            plugins: vec![game.clone(), target.clone()],
            base_plugins: vec!["Fallout4.esm".into()],
            fixer_plugins: vec![],
            cells: cells.clone(),
            precombine_cells: cells,
            output: dir.path().join("work").join("Mod.esp"),
            date: DATE,
            stand_in: true,
            required_masters: vec![],
        })
        .unwrap();
        stamp(&seed, &[(0x720, 0x710), (0x730, 0x710), (0x740, 0x710)]);
        let output = dir.path().join("out").join("Mod.esp");
        let report = write_into_plugin(&IntoPluginRequest { seed, target, output: output.clone() }).unwrap();
        assert_eq!(report.added_cells, ["000701:Fallout4.esm", "000730:Fallout4.esm", "000740:Fallout4.esm"], "the persistent CELL holds the redated reference");
        assert_eq!(report.added_children, 1, "the standing persistent reference is redated as an override");

        let written = Plugin::open(&output).unwrap();
        assert_eq!(written.cells_at(0x700, 0, 0), [0x720]);
        assert_eq!(written.cells_at(0x700, 1, 0), [0x730]);
        assert_eq!(written.cells_at(0x700, 40, 0), [0x740]);
        for cell in [0x720, 0x730, 0x740] {
            assert_eq!(field(&written, cell, b"PCMB").unwrap(), DATE.to_le_bytes());
        }
        let persistent = &written.cells[&0x701].persistent;
        assert_eq!(persistent.iter().map(|r| (r.form_id, crate::load_order::version_date(&written, r))).collect::<Vec<_>>(), [(0x710, DATE)]);
        let index = LoadOrderIndex::open(&[game, output]).unwrap();
        assert_eq!(index.winner(global_id(0, 0x740)).unwrap().plugin, 1);
    }

    #[test]
    fn exterior_seed_carries_world_and_persistent_references_in_its_grid() {
        let dir = tempfile::tempdir().unwrap();
        let place = |form_id: u32, persistent: bool, x: f32| {
            let mut data = x.to_le_bytes().to_vec();
            data.extend_from_slice(&[0; 20]);
            record(b"REFR", form_id, if persistent { 0x400 } else { 0 }, 0, &[subrecord(b"NAME", &0x900u32.to_le_bytes()), subrecord(b"DATA", &data)])
        };
        let grid_cell = |form_id: u32, x: i32| {
            let mut xclc = x.to_le_bytes().to_vec();
            xclc.extend_from_slice(&0i32.to_le_bytes());
            xclc.push(0);
            record(b"CELL", form_id, 0, 10, &[subrecord(b"DATA", &0u16.to_le_bytes()), subrecord(b"XCLC", &xclc)])
        };
        let world = group(u32::from_le_bytes(*b"WRLD"), 0, &[
            record(b"WRLD", 0x700, 0, 10, &[subrecord(b"EDID", b"World\0")]),
            group(0x700, 1, &[
                record(b"CELL", 0x701, 0x400, 10, &[subrecord(b"DATA", &2u16.to_le_bytes())]),
                group(0x701, 6, &[group(0x701, 8, &[place(0x710, true, 100.0), place(0x711, true, 9000.0)])]),
                group(0, 4, &[group(0, 5, &[
                    grid_cell(0x720, 0),
                    group(0x720, 6, &[group(0x720, 9, &[place(0x721, false, 50.0)])]),
                    grid_cell(0x730, 2),
                ])]),
            ]),
        ]);
        let game = PluginBuilder::new(ESM, &[]).push(stat(0x900, "A.nif")).push(world).write(dir.path(), "Fallout4.esm");
        let seed = write_seed(&SeedRequest {
            plugins: vec![game],
            base_plugins: vec![],
            fixer_plugins: vec![],
            cells: vec!["000720:Fallout4.esm".into()],
            precombine_cells: vec!["000720:Fallout4.esm".into()],
            output: dir.path().join("work").join("Repair.esm"),
            date: DATE,
            stand_in: false,
            required_masters: vec![],
        })
        .unwrap();
        let context: Vec<(u32, bool)> = seed.cells.iter().map(|c| (c.form_id, c.context)).collect();
        assert_eq!(context, [(0x720, false), (0x730, true)], "(2,0) rings cluster (0,0), so previs sees its targets");
        let plugin = Plugin::open(&seed.plugin).unwrap();
        assert!(plugin.record(0x700).is_some());
        assert_eq!(plugin.world_cells[&0x700], [0x720, 0x730]);
        assert_eq!(plugin.cells[&0x701].persistent.iter().map(|r| r.form_id).collect::<Vec<_>>(), [0x710, 0x711], "persistent references standing in seeded CELLs");
        assert_eq!(plugin.cells[&0x720].temporary.len(), 1);
        assert_eq!(plugin.cells_at(0x700, 0, 0), [0x720]);
        drop(plugin);
        let report = write_patch(&PatchRequest { seed, output: dir.path().join("out").join("Repair.esm"), esm: true }).unwrap();
        assert_eq!(report.unchanged, ["000720:Fallout4.esm"], "context CELLs are never patched or reported");
    }
}
