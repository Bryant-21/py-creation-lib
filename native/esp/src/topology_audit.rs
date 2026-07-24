use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use bytes::Bytes;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::plugin_runtime::{
    ParsedRecord, decode_cp1252, detect_header_size, parse_record, read_plugin_source_bytes,
};

const WORLD_CHILD_GROUP: i32 = 1;
const EXTERIOR_CELL_BLOCK: i32 = 4;
const EXTERIOR_CELL_SUBBLOCK: i32 = 5;
const CELL_CHILD_GROUP: i32 = 6;
const PERSISTENT_GROUP: i32 = 8;
const TEMPORARY_GROUP: i32 = 9;
const VISIBLE_DISTANT_GROUP: i32 = 10;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PluginTopologyReport {
    pub plugin: String,
    pub path: String,
    pub game: Option<String>,
    pub header_size: usize,
    pub worldspaces: Vec<WorldspaceTopologyReport>,
    pub summary: PluginTopologySummary,
    pub anomalies: PluginTopologyAnomalies,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct PluginTopologySummary {
    pub worldspaces: usize,
    pub exterior_cells: usize,
    pub unique_cell_coordinates: usize,
    pub persistent_groups: usize,
    pub temporary_groups: usize,
    pub land_records: usize,
    pub navm_records: usize,
    pub valid_land_records: usize,
    pub valid_navm_records: usize,
    pub missing_land_cells: usize,
    pub duplicate_cell_coordinates: usize,
    pub duplicate_land_cells: usize,
    pub flat_exterior_cells: usize,
    pub flat_land_records: usize,
    pub flat_navm_records: usize,
    pub orphan_land_records: usize,
    pub orphan_navm_records: usize,
    pub misplaced_land_records: usize,
    pub misplaced_navm_records: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct PluginTopologyAnomalies {
    pub flat_exterior_cells: Vec<CoordinateRecord>,
    pub flat_land_records: Vec<FlatRecord>,
    pub flat_navm_records: Vec<FlatRecord>,
    pub duplicate_record_form_ids: Vec<DuplicateRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WorldspaceTopologyReport {
    pub editor_id: Option<String>,
    pub form_id: String,
    pub cells: Vec<CellTopologyReport>,
    pub summary: WorldspaceTopologySummary,
    pub anomalies: WorldspaceTopologyAnomalies,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct WorldspaceTopologySummary {
    pub exterior_cells: usize,
    pub unique_cell_coordinates: usize,
    pub non_exterior_cells: usize,
    pub cell_child_groups: usize,
    pub persistent_groups: usize,
    pub temporary_groups: usize,
    pub visible_distant_groups: usize,
    pub persistent_records: usize,
    pub temporary_records: usize,
    pub visible_distant_records: usize,
    pub land_records: usize,
    pub navm_records: usize,
    pub valid_land_records: usize,
    pub valid_navm_records: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct WorldspaceTopologyAnomalies {
    pub duplicate_cell_coordinates: Vec<CoordinateCount>,
    pub cells_missing_coordinates: Vec<String>,
    pub cells_missing_child_groups: Vec<CoordinateRecord>,
    pub cells_missing_land: Vec<CoordinateRecord>,
    pub cells_with_duplicate_land: Vec<CoordinateCount>,
    pub orphan_cell_groups: Vec<String>,
    pub orphan_land_records: Vec<PlacedRecord>,
    pub orphan_navm_records: Vec<PlacedRecord>,
    pub misplaced_land_records: Vec<PlacedRecord>,
    pub misplaced_navm_records: Vec<PlacedRecord>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct CellTopologyReport {
    pub form_id: String,
    pub x: i32,
    pub y: i32,
    pub child_groups: usize,
    pub persistent_groups: usize,
    pub temporary_groups: usize,
    pub visible_distant_groups: usize,
    pub persistent_records: usize,
    pub temporary_records: usize,
    pub visible_distant_records: usize,
    pub land_records: usize,
    pub land_in_temporary_group: usize,
    pub misplaced_land_records: usize,
    pub navm_records: usize,
    pub navm_in_temporary_group: usize,
    pub misplaced_navm_records: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CoordinateRecord {
    pub form_id: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CoordinateCount {
    pub x: i32,
    pub y: i32,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct FlatRecord {
    pub signature: String,
    pub form_id: String,
    pub top_group: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PlacedRecord {
    pub signature: String,
    pub form_id: String,
    pub cell_form_id: String,
    pub group_type: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DuplicateRecord {
    pub signature: String,
    pub form_id: String,
    pub count: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct WalkContext {
    top_group: Option<[u8; 4]>,
    world_form_id: Option<u32>,
    in_exterior_block: bool,
    in_exterior_subblock: bool,
    in_cell_child_group: bool,
    cell_form_id: Option<u32>,
    section_group_type: Option<i32>,
}

#[derive(Debug, Clone)]
struct ExteriorCell {
    form_id: u32,
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Copy)]
struct RecordPlacement {
    form_id: u32,
    group_type: Option<i32>,
}

#[derive(Debug, Clone, Default)]
struct CellGroupStats {
    child_groups: usize,
    persistent_groups: usize,
    temporary_groups: usize,
    visible_distant_groups: usize,
    persistent_records: usize,
    temporary_records: usize,
    visible_distant_records: usize,
    land: Vec<RecordPlacement>,
    navm: Vec<RecordPlacement>,
}

#[derive(Debug, Clone, Default)]
struct WorldspaceBuilder {
    editor_id: Option<String>,
    cells: Vec<ExteriorCell>,
    cells_missing_coordinates: Vec<u32>,
    non_exterior_cells: Vec<u32>,
    cell_groups: FxHashMap<u32, CellGroupStats>,
}

struct TopologyScanner<'a> {
    data: &'a Bytes,
    header_size: usize,
    worlds: FxHashMap<u32, WorldspaceBuilder>,
    flat_exterior_cells: Vec<ExteriorCell>,
    flat_exterior_cell_ids: BTreeSet<u32>,
    flat_land_records: Vec<FlatRecord>,
    flat_navm_records: Vec<FlatRecord>,
    record_counts: FxHashMap<([u8; 4], u32), usize>,
}

impl<'a> TopologyScanner<'a> {
    fn new(data: &'a Bytes, header_size: usize) -> Self {
        Self {
            data,
            header_size,
            worlds: FxHashMap::default(),
            flat_exterior_cells: Vec::new(),
            flat_exterior_cell_ids: BTreeSet::new(),
            flat_land_records: Vec::new(),
            flat_navm_records: Vec::new(),
            record_counts: FxHashMap::default(),
        }
    }

    fn scan_range(
        &mut self,
        mut cursor: usize,
        end: usize,
        context: WalkContext,
    ) -> Result<(), String> {
        if end > self.data.len() {
            return Err(format!("topology span ends past file at 0x{end:X}"));
        }
        while cursor < end {
            if cursor + self.header_size > end || cursor + 16 > self.data.len() {
                return Err(format!("truncated item header at 0x{cursor:X}"));
            }
            let signature = read_signature(self.data, cursor)?;
            let size = read_u32_at(self.data, cursor + 4)? as usize;
            if &signature == b"GRUP" {
                let group_end = cursor
                    .checked_add(size)
                    .ok_or_else(|| format!("GRUP size overflow at 0x{cursor:X}"))?;
                if size < self.header_size || group_end > end {
                    return Err(format!(
                        "invalid GRUP size {size} at 0x{cursor:X} (span ends 0x{end:X})"
                    ));
                }
                let label = read_signature(self.data, cursor + 8)?;
                let group_type = read_u32_at(self.data, cursor + 12)? as i32;
                let child_context = self.enter_group(context, label, group_type);
                self.scan_range(cursor + self.header_size, group_end, child_context)?;
                cursor = group_end;
                continue;
            }

            let next = cursor
                .checked_add(self.header_size)
                .and_then(|value| value.checked_add(size))
                .ok_or_else(|| format!("record size overflow at 0x{cursor:X}"))?;
            if next > end {
                return Err(format!(
                    "record {} at 0x{cursor:X} extends past span 0x{end:X}",
                    signature_text(signature)
                ));
            }
            let form_id = read_u32_at(self.data, cursor + 12)?;
            self.process_record(signature, form_id, cursor, context)?;
            cursor = next;
        }
        Ok(())
    }

    fn enter_group(
        &mut self,
        context: WalkContext,
        label: [u8; 4],
        group_type: i32,
    ) -> WalkContext {
        let mut child = context;
        if group_type == 0 {
            child = WalkContext {
                top_group: Some(label),
                ..WalkContext::default()
            };
            return child;
        }
        if group_type == WORLD_CHILD_GROUP && context.top_group == Some(*b"WRLD") {
            let world_form_id = u32::from_le_bytes(label);
            self.worlds.entry(world_form_id).or_default();
            child.world_form_id = Some(world_form_id);
            child.in_exterior_block = false;
            child.in_exterior_subblock = false;
            child.in_cell_child_group = false;
            child.cell_form_id = None;
            child.section_group_type = None;
            return child;
        }
        if group_type == EXTERIOR_CELL_BLOCK && context.world_form_id.is_some() {
            child.in_exterior_block = true;
            child.in_exterior_subblock = false;
            child.in_cell_child_group = false;
            child.cell_form_id = None;
            child.section_group_type = None;
            return child;
        }
        if group_type == EXTERIOR_CELL_SUBBLOCK
            && context.world_form_id.is_some()
            && context.in_exterior_block
        {
            child.in_exterior_subblock = true;
            child.in_cell_child_group = false;
            child.cell_form_id = None;
            child.section_group_type = None;
            return child;
        }
        if group_type == CELL_CHILD_GROUP {
            let cell_form_id = u32::from_le_bytes(label);
            child.in_cell_child_group = true;
            child.cell_form_id = Some(cell_form_id);
            if let Some(world_form_id) = context.world_form_id {
                let stats = self
                    .worlds
                    .entry(world_form_id)
                    .or_default()
                    .cell_groups
                    .entry(cell_form_id)
                    .or_default();
                stats.child_groups += 1;
                child.section_group_type = None;
            }
            return child;
        }
        if matches!(
            group_type,
            PERSISTENT_GROUP | TEMPORARY_GROUP | VISIBLE_DISTANT_GROUP
        ) {
            if let (Some(world_form_id), Some(cell_form_id)) =
                (context.world_form_id, context.cell_form_id)
            {
                let stats = self
                    .worlds
                    .entry(world_form_id)
                    .or_default()
                    .cell_groups
                    .entry(cell_form_id)
                    .or_default();
                match group_type {
                    PERSISTENT_GROUP => stats.persistent_groups += 1,
                    TEMPORARY_GROUP => stats.temporary_groups += 1,
                    VISIBLE_DISTANT_GROUP => stats.visible_distant_groups += 1,
                    _ => {}
                }
                child.section_group_type = Some(group_type);
            }
        }
        child
    }

    fn process_record(
        &mut self,
        signature: [u8; 4],
        form_id: u32,
        offset: usize,
        context: WalkContext,
    ) -> Result<(), String> {
        if matches!(&signature, b"CELL" | b"LAND" | b"NAVM") {
            *self.record_counts.entry((signature, form_id)).or_default() += 1;
        }

        if let (Some(world_form_id), Some(cell_form_id), Some(section)) = (
            context.world_form_id,
            context.cell_form_id,
            context.section_group_type,
        ) {
            let stats = self
                .worlds
                .entry(world_form_id)
                .or_default()
                .cell_groups
                .entry(cell_form_id)
                .or_default();
            match section {
                PERSISTENT_GROUP => stats.persistent_records += 1,
                TEMPORARY_GROUP => stats.temporary_records += 1,
                VISIBLE_DISTANT_GROUP => stats.visible_distant_records += 1,
                _ => {}
            }
        }

        match &signature {
            b"WRLD" => {
                let record = self.parse_metadata_record(offset)?;
                let editor_id = record
                    .subrecords
                    .iter()
                    .find(|subrecord| subrecord.signature == "EDID")
                    .map(|subrecord| decode_cp1252(&subrecord.data))
                    .filter(|value| !value.is_empty());
                self.worlds.entry(form_id).or_default().editor_id = editor_id;
            }
            b"CELL" => {
                let coordinates = self.cell_coordinates(offset)?;
                if let Some(world_form_id) = context.world_form_id {
                    let world = self.worlds.entry(world_form_id).or_default();
                    if context.in_exterior_subblock {
                        if let Some((x, y)) = coordinates {
                            world.cells.push(ExteriorCell { form_id, x, y });
                        } else {
                            world.cells_missing_coordinates.push(form_id);
                        }
                    } else {
                        world.non_exterior_cells.push(form_id);
                    }
                } else if context.top_group == Some(*b"CELL") {
                    if let Some((x, y)) = coordinates {
                        self.flat_exterior_cell_ids.insert(form_id);
                        self.flat_exterior_cells
                            .push(ExteriorCell { form_id, x, y });
                    }
                }
            }
            b"LAND" | b"NAVM" => {
                if signature == *b"NAVM"
                    && context.top_group == Some(*b"CELL")
                    && context.in_cell_child_group
                    && !context.cell_form_id.is_some_and(|cell_form_id| {
                        self.flat_exterior_cell_ids.contains(&cell_form_id)
                    })
                {
                    return Ok(());
                }
                let placement = RecordPlacement {
                    form_id,
                    group_type: context.section_group_type,
                };
                if let (Some(world_form_id), Some(cell_form_id)) =
                    (context.world_form_id, context.cell_form_id)
                {
                    let stats = self
                        .worlds
                        .entry(world_form_id)
                        .or_default()
                        .cell_groups
                        .entry(cell_form_id)
                        .or_default();
                    if signature == *b"LAND" {
                        stats.land.push(placement);
                    } else {
                        stats.navm.push(placement);
                    }
                } else {
                    let flat = FlatRecord {
                        signature: signature_text(signature),
                        form_id: form_id_text(form_id),
                        top_group: context.top_group.map(signature_text),
                    };
                    if signature == *b"LAND" {
                        self.flat_land_records.push(flat);
                    } else {
                        self.flat_navm_records.push(flat);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn parse_metadata_record(&self, offset: usize) -> Result<ParsedRecord, String> {
        parse_record(self.data, offset, self.header_size, true)
            .map(|(record, _)| record)
            .map_err(|error| format!("failed to parse record at 0x{offset:X}: {error}"))
    }

    fn cell_coordinates(&self, offset: usize) -> Result<Option<(i32, i32)>, String> {
        let record = self.parse_metadata_record(offset)?;
        let Some(xclc) = record
            .subrecords
            .iter()
            .find(|subrecord| subrecord.signature == "XCLC")
        else {
            return Ok(None);
        };
        if xclc.data.len() < 8 {
            return Ok(None);
        }
        let x = i32::from_le_bytes(xclc.data[0..4].try_into().unwrap());
        let y = i32::from_le_bytes(xclc.data[4..8].try_into().unwrap());
        Ok(Some((x, y)))
    }

    fn finish(self, plugin: String, path: String, game: Option<String>) -> PluginTopologyReport {
        let mut worldspaces = self
            .worlds
            .into_iter()
            .map(|(form_id, builder)| finish_worldspace(form_id, builder))
            .collect::<Vec<_>>();
        worldspaces.sort_by(|left, right| {
            left.editor_id
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase()
                .cmp(
                    &right
                        .editor_id
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase(),
                )
                .then_with(|| left.form_id.cmp(&right.form_id))
        });

        let mut duplicate_record_form_ids = self
            .record_counts
            .into_iter()
            .filter_map(|((signature, form_id), count)| {
                (count > 1).then(|| DuplicateRecord {
                    signature: signature_text(signature),
                    form_id: form_id_text(form_id),
                    count,
                })
            })
            .collect::<Vec<_>>();
        duplicate_record_form_ids.sort_by(|left, right| {
            left.signature
                .cmp(&right.signature)
                .then_with(|| left.form_id.cmp(&right.form_id))
        });

        let mut flat_exterior_cells = self
            .flat_exterior_cells
            .into_iter()
            .map(coordinate_record)
            .collect::<Vec<_>>();
        flat_exterior_cells.sort_by_key(|cell| (cell.x, cell.y, cell.form_id.clone()));
        let mut flat_land_records = self.flat_land_records;
        let mut flat_navm_records = self.flat_navm_records;
        flat_land_records.sort_by(|left, right| left.form_id.cmp(&right.form_id));
        flat_navm_records.sort_by(|left, right| left.form_id.cmp(&right.form_id));

        let anomalies = PluginTopologyAnomalies {
            flat_exterior_cells,
            flat_land_records,
            flat_navm_records,
            duplicate_record_form_ids,
        };
        let summary = summarize_plugin(&worldspaces, &anomalies);
        PluginTopologyReport {
            plugin,
            path,
            game,
            header_size: self.header_size,
            worldspaces,
            summary,
            anomalies,
        }
    }
}

fn finish_worldspace(form_id: u32, builder: WorldspaceBuilder) -> WorldspaceTopologyReport {
    let known_cell_ids = builder
        .cells
        .iter()
        .map(|cell| cell.form_id)
        .chain(builder.cells_missing_coordinates.iter().copied())
        .chain(builder.non_exterior_cells.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut anomalies = WorldspaceTopologyAnomalies {
        cells_missing_coordinates: builder
            .cells_missing_coordinates
            .iter()
            .copied()
            .map(form_id_text)
            .collect(),
        ..WorldspaceTopologyAnomalies::default()
    };

    for (cell_form_id, stats) in &builder.cell_groups {
        if known_cell_ids.contains(cell_form_id) {
            continue;
        }
        anomalies
            .orphan_cell_groups
            .push(form_id_text(*cell_form_id));
        anomalies.orphan_land_records.extend(
            stats
                .land
                .iter()
                .map(|placement| placed_record("LAND", *cell_form_id, *placement)),
        );
        anomalies.orphan_navm_records.extend(
            stats
                .navm
                .iter()
                .map(|placement| placed_record("NAVM", *cell_form_id, *placement)),
        );
    }

    let mut coordinate_counts = BTreeMap::<(i32, i32), usize>::new();
    for cell in &builder.cells {
        *coordinate_counts.entry((cell.x, cell.y)).or_default() += 1;
    }
    anomalies.duplicate_cell_coordinates = coordinate_counts
        .iter()
        .filter_map(|(&(x, y), &count)| (count > 1).then_some(CoordinateCount { x, y, count }))
        .collect();

    let mut cells = Vec::with_capacity(builder.cells.len());
    for cell in &builder.cells {
        let stats = builder.cell_groups.get(&cell.form_id);
        if stats.is_none() {
            anomalies
                .cells_missing_child_groups
                .push(coordinate_record(cell.clone()));
        }
        let stats = stats.cloned().unwrap_or_default();
        let valid_land = stats
            .land
            .iter()
            .filter(|placement| placement.group_type == Some(TEMPORARY_GROUP))
            .count();
        let valid_navm = stats
            .navm
            .iter()
            .filter(|placement| placement.group_type == Some(TEMPORARY_GROUP))
            .count();
        let misplaced_land = stats.land.len() - valid_land;
        let misplaced_navm = stats.navm.len() - valid_navm;

        if valid_land == 0 {
            anomalies
                .cells_missing_land
                .push(coordinate_record(cell.clone()));
        } else if valid_land > 1 {
            anomalies.cells_with_duplicate_land.push(CoordinateCount {
                x: cell.x,
                y: cell.y,
                count: valid_land,
            });
        }
        anomalies.misplaced_land_records.extend(
            stats
                .land
                .iter()
                .filter(|placement| placement.group_type != Some(TEMPORARY_GROUP))
                .map(|placement| placed_record("LAND", cell.form_id, *placement)),
        );
        anomalies.misplaced_navm_records.extend(
            stats
                .navm
                .iter()
                .filter(|placement| placement.group_type != Some(TEMPORARY_GROUP))
                .map(|placement| placed_record("NAVM", cell.form_id, *placement)),
        );
        cells.push(CellTopologyReport {
            form_id: form_id_text(cell.form_id),
            x: cell.x,
            y: cell.y,
            child_groups: stats.child_groups,
            persistent_groups: stats.persistent_groups,
            temporary_groups: stats.temporary_groups,
            visible_distant_groups: stats.visible_distant_groups,
            persistent_records: stats.persistent_records,
            temporary_records: stats.temporary_records,
            visible_distant_records: stats.visible_distant_records,
            land_records: stats.land.len(),
            land_in_temporary_group: valid_land,
            misplaced_land_records: misplaced_land,
            navm_records: stats.navm.len(),
            navm_in_temporary_group: valid_navm,
            misplaced_navm_records: misplaced_navm,
        });
    }
    cells.sort_by_key(|cell| (cell.x, cell.y, cell.form_id.clone()));

    anomalies.orphan_cell_groups.sort();
    anomalies.cells_missing_coordinates.sort();
    sort_placed_records(&mut anomalies.orphan_land_records);
    sort_placed_records(&mut anomalies.orphan_navm_records);
    sort_placed_records(&mut anomalies.misplaced_land_records);
    sort_placed_records(&mut anomalies.misplaced_navm_records);

    let summary = WorldspaceTopologySummary {
        exterior_cells: cells.len(),
        unique_cell_coordinates: coordinate_counts.len(),
        non_exterior_cells: builder.non_exterior_cells.len(),
        cell_child_groups: cells.iter().map(|cell| cell.child_groups).sum(),
        persistent_groups: cells.iter().map(|cell| cell.persistent_groups).sum(),
        temporary_groups: cells.iter().map(|cell| cell.temporary_groups).sum(),
        visible_distant_groups: cells.iter().map(|cell| cell.visible_distant_groups).sum(),
        persistent_records: cells.iter().map(|cell| cell.persistent_records).sum(),
        temporary_records: cells.iter().map(|cell| cell.temporary_records).sum(),
        visible_distant_records: cells.iter().map(|cell| cell.visible_distant_records).sum(),
        land_records: cells.iter().map(|cell| cell.land_records).sum(),
        navm_records: cells.iter().map(|cell| cell.navm_records).sum(),
        valid_land_records: cells.iter().map(|cell| cell.land_in_temporary_group).sum(),
        valid_navm_records: cells.iter().map(|cell| cell.navm_in_temporary_group).sum(),
    };

    WorldspaceTopologyReport {
        editor_id: builder.editor_id,
        form_id: form_id_text(form_id),
        cells,
        summary,
        anomalies,
    }
}

fn summarize_plugin(
    worldspaces: &[WorldspaceTopologyReport],
    anomalies: &PluginTopologyAnomalies,
) -> PluginTopologySummary {
    PluginTopologySummary {
        worldspaces: worldspaces.len(),
        exterior_cells: worldspaces
            .iter()
            .map(|world| world.summary.exterior_cells)
            .sum(),
        unique_cell_coordinates: worldspaces
            .iter()
            .map(|world| world.summary.unique_cell_coordinates)
            .sum(),
        persistent_groups: worldspaces
            .iter()
            .map(|world| world.summary.persistent_groups)
            .sum(),
        temporary_groups: worldspaces
            .iter()
            .map(|world| world.summary.temporary_groups)
            .sum(),
        land_records: worldspaces
            .iter()
            .map(|world| world.summary.land_records)
            .sum(),
        navm_records: worldspaces
            .iter()
            .map(|world| world.summary.navm_records)
            .sum(),
        valid_land_records: worldspaces
            .iter()
            .map(|world| world.summary.valid_land_records)
            .sum(),
        valid_navm_records: worldspaces
            .iter()
            .map(|world| world.summary.valid_navm_records)
            .sum(),
        missing_land_cells: worldspaces
            .iter()
            .map(|world| world.anomalies.cells_missing_land.len())
            .sum(),
        duplicate_cell_coordinates: worldspaces
            .iter()
            .map(|world| world.anomalies.duplicate_cell_coordinates.len())
            .sum(),
        duplicate_land_cells: worldspaces
            .iter()
            .map(|world| world.anomalies.cells_with_duplicate_land.len())
            .sum(),
        flat_exterior_cells: anomalies.flat_exterior_cells.len(),
        flat_land_records: anomalies.flat_land_records.len(),
        flat_navm_records: anomalies.flat_navm_records.len(),
        orphan_land_records: worldspaces
            .iter()
            .map(|world| world.anomalies.orphan_land_records.len())
            .sum(),
        orphan_navm_records: worldspaces
            .iter()
            .map(|world| world.anomalies.orphan_navm_records.len())
            .sum(),
        misplaced_land_records: worldspaces
            .iter()
            .map(|world| world.anomalies.misplaced_land_records.len())
            .sum(),
        misplaced_navm_records: worldspaces
            .iter()
            .map(|world| world.anomalies.misplaced_navm_records.len())
            .sum(),
    }
}

fn coordinate_record(cell: ExteriorCell) -> CoordinateRecord {
    CoordinateRecord {
        form_id: form_id_text(cell.form_id),
        x: cell.x,
        y: cell.y,
    }
}

fn placed_record(signature: &str, cell_form_id: u32, placement: RecordPlacement) -> PlacedRecord {
    PlacedRecord {
        signature: signature.to_string(),
        form_id: form_id_text(placement.form_id),
        cell_form_id: form_id_text(cell_form_id),
        group_type: placement.group_type,
    }
}

fn sort_placed_records(records: &mut [PlacedRecord]) {
    records.sort_by(|left, right| {
        left.cell_form_id
            .cmp(&right.cell_form_id)
            .then_with(|| left.form_id.cmp(&right.form_id))
    });
}

fn form_id_text(form_id: u32) -> String {
    format!("{form_id:08X}")
}

fn signature_text(signature: [u8; 4]) -> String {
    String::from_utf8_lossy(&signature).into_owned()
}

fn read_signature(data: &[u8], offset: usize) -> Result<[u8; 4], String> {
    data.get(offset..offset + 4)
        .ok_or_else(|| format!("signature read past end at 0x{offset:X}"))?
        .try_into()
        .map_err(|_| format!("invalid signature at 0x{offset:X}"))
}

fn read_u32_at(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .ok_or_else(|| format!("u32 read past end at 0x{offset:X}"))?
        .try_into()
        .map_err(|_| format!("invalid u32 at 0x{offset:X}"))?;
    Ok(u32::from_le_bytes(bytes))
}

pub(crate) fn audit_plugin_topology(
    path: &Path,
    game: Option<&str>,
) -> Result<PluginTopologyReport, String> {
    let data = read_plugin_source_bytes(path).map_err(|error| error.to_string())?;
    let plugin = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Plugin.esp")
        .to_string();
    audit_plugin_topology_bytes(
        &data,
        plugin,
        path.to_string_lossy().into_owned(),
        game.map(str::to_string),
    )
}

fn audit_plugin_topology_bytes(
    data: &Bytes,
    plugin: String,
    path: String,
    game: Option<String>,
) -> Result<PluginTopologyReport, String> {
    let header_size = detect_header_size(data);
    let (header, root_start) = parse_record(data, 0, header_size, true)
        .map_err(|error| format!("failed to parse TES4 header: {error}"))?;
    if header.signature != "TES4" {
        return Err(format!("expected TES4 header, found {}", header.signature));
    }
    let mut scanner = TopologyScanner::new(data, header_size);
    scanner.scan_range(root_start, data.len(), WalkContext::default())?;
    Ok(scanner.finish(plugin, path, game))
}

#[pyfunction(name = "audit_plugin_topology_native", signature = (plugin_path, game=None))]
pub(crate) fn audit_plugin_topology_native(
    py: Python<'_>,
    plugin_path: &str,
    game: Option<&str>,
) -> PyResult<String> {
    let path = plugin_path.to_string();
    let game = game.map(str::to_string);
    py.detach(move || {
        let report = audit_plugin_topology(Path::new(&path), game.as_deref())
            .map_err(PyRuntimeError::new_err)?;
        serde_json::to_string(&report).map_err(|error| {
            PyRuntimeError::new_err(format!("topology report encoding failed: {error}"))
        })
    })
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use smol_str::SmolStr;

    use super::*;
    use crate::plugin_runtime::{
        ParsedGroup, ParsedItem, ParsedPlugin, ParsedPluginHeader, ParsedRecord, ParsedSubrecord,
        build_plugin_bytes,
    };

    const HEADER_SIZE: usize = 24;

    fn subrecord(signature: &str, data: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: SmolStr::new(signature),
            data: Bytes::from(data),
            semantic_type: None,
        }
    }

    fn record(signature: &str, form_id: u32, subrecords: Vec<ParsedSubrecord>) -> ParsedItem {
        ParsedItem::Record(ParsedRecord {
            signature: SmolStr::new(signature),
            form_id,
            flags: 0,
            version_control: 0,
            form_version: Some(0),
            version2: Some(0),
            subrecords,
            raw_payload: None,
            parse_error: None,
        })
    }

    fn group(label: [u8; 4], group_type: i32, children: Vec<ParsedItem>) -> ParsedItem {
        ParsedItem::Group(ParsedGroup {
            label,
            group_type,
            tail: Bytes::from(vec![0; HEADER_SIZE - 16]),
            children,
        })
    }

    fn form_label(form_id: u32) -> [u8; 4] {
        form_id.to_le_bytes()
    }

    fn xclc(x: i32, y: i32) -> ParsedSubrecord {
        let mut data = Vec::new();
        data.extend_from_slice(&x.to_le_bytes());
        data.extend_from_slice(&y.to_le_bytes());
        subrecord("XCLC", data)
    }

    fn plugin_bytes(root_items: Vec<ParsedItem>) -> Bytes {
        let mut plugin = ParsedPlugin {
            plugin_name: "Topology.esp".to_string(),
            file_path: "Topology.esp".to_string(),
            header_size: HEADER_SIZE,
            header: ParsedPluginHeader::default_for_test(),
            root_items,
            game: Some("fo4".to_string()),
        };
        Bytes::from(build_plugin_bytes(&mut plugin).expect("plugin bytes"))
    }

    fn audit(root_items: Vec<ParsedItem>) -> PluginTopologyReport {
        audit_plugin_topology_bytes(
            &plugin_bytes(root_items),
            "Topology.esp".to_string(),
            "Topology.esp".to_string(),
            Some("fo4".to_string()),
        )
        .expect("topology audit")
    }

    #[test]
    fn scans_nested_world_cell_sections_without_materializing_the_plugin_tree() {
        let world_id = 0x800;
        let cell_id = 0x801;
        let root = vec![group(
            *b"WRLD",
            0,
            vec![
                record(
                    "WRLD",
                    world_id,
                    vec![subrecord("EDID", b"WastelandNV\0".to_vec())],
                ),
                group(
                    form_label(world_id),
                    WORLD_CHILD_GROUP,
                    vec![
                        record("CELL", 0x805, vec![]),
                        group(
                            form_label(0x805),
                            CELL_CHILD_GROUP,
                            vec![group(
                                form_label(0x805),
                                PERSISTENT_GROUP,
                                vec![record("REFR", 0x806, vec![])],
                            )],
                        ),
                        group(
                            [0, 0, 0, 0],
                            EXTERIOR_CELL_BLOCK,
                            vec![group(
                                [0, 0, 0, 0],
                                EXTERIOR_CELL_SUBBLOCK,
                                vec![
                                    record("CELL", cell_id, vec![xclc(-1, 2)]),
                                    group(
                                        form_label(cell_id),
                                        CELL_CHILD_GROUP,
                                        vec![
                                            group(
                                                form_label(cell_id),
                                                PERSISTENT_GROUP,
                                                vec![record("REFR", 0x810, vec![])],
                                            ),
                                            group(
                                                form_label(cell_id),
                                                TEMPORARY_GROUP,
                                                vec![
                                                    record("LAND", 0x811, vec![]),
                                                    record("NAVM", 0x812, vec![]),
                                                ],
                                            ),
                                        ],
                                    ),
                                ],
                            )],
                        ),
                    ],
                ),
            ],
        )];

        let report = audit(root);
        assert_eq!(report.worldspaces.len(), 1);
        let world = &report.worldspaces[0];
        assert_eq!(world.editor_id.as_deref(), Some("WastelandNV"));
        assert_eq!(world.cells.len(), 1);
        assert_eq!((world.cells[0].x, world.cells[0].y), (-1, 2));
        assert_eq!(world.cells[0].persistent_groups, 1);
        assert_eq!(world.cells[0].temporary_groups, 1);
        assert_eq!(world.cells[0].land_in_temporary_group, 1);
        assert_eq!(world.cells[0].navm_in_temporary_group, 1);
        assert_eq!(world.summary.non_exterior_cells, 1);
        assert!(world.anomalies.orphan_cell_groups.is_empty());
        assert!(world.anomalies.cells_missing_land.is_empty());
        assert_eq!(report.summary.exterior_cells, 1);
    }

    #[test]
    fn reports_flat_orphan_duplicate_missing_and_misplaced_topology() {
        let world_id = 0x900;
        let cell_a = 0x901;
        let cell_b = 0x902;
        let orphan_cell = 0x9FF;
        let root = vec![
            group(
                *b"WRLD",
                0,
                vec![
                    record(
                        "WRLD",
                        world_id,
                        vec![subrecord("EDID", b"AuditWorld\0".to_vec())],
                    ),
                    group(
                        form_label(world_id),
                        WORLD_CHILD_GROUP,
                        vec![group(
                            [0, 0, 0, 0],
                            EXTERIOR_CELL_BLOCK,
                            vec![group(
                                [0, 0, 0, 0],
                                EXTERIOR_CELL_SUBBLOCK,
                                vec![
                                    record("CELL", cell_a, vec![xclc(3, 4)]),
                                    group(
                                        form_label(cell_a),
                                        CELL_CHILD_GROUP,
                                        vec![
                                            group(
                                                form_label(cell_a),
                                                TEMPORARY_GROUP,
                                                vec![
                                                    record("LAND", 0x910, vec![]),
                                                    record("LAND", 0x910, vec![]),
                                                ],
                                            ),
                                            group(
                                                form_label(cell_a),
                                                PERSISTENT_GROUP,
                                                vec![record("NAVM", 0x912, vec![])],
                                            ),
                                        ],
                                    ),
                                    record("CELL", cell_b, vec![xclc(3, 4)]),
                                    group(
                                        form_label(orphan_cell),
                                        CELL_CHILD_GROUP,
                                        vec![group(
                                            form_label(orphan_cell),
                                            TEMPORARY_GROUP,
                                            vec![
                                                record("LAND", 0x920, vec![]),
                                                record("NAVM", 0x921, vec![]),
                                            ],
                                        )],
                                    ),
                                ],
                            )],
                        )],
                    ),
                ],
            ),
            group(
                *b"CELL",
                0,
                vec![
                    record("CELL", 0xA00, vec![xclc(8, 9)]),
                    group(
                        form_label(0xA00),
                        CELL_CHILD_GROUP,
                        vec![group(
                            form_label(0xA00),
                            TEMPORARY_GROUP,
                            vec![record("NAVM", 0xA03, vec![])],
                        )],
                    ),
                    record("CELL", 0xA10, vec![]),
                    group(
                        form_label(0xA10),
                        CELL_CHILD_GROUP,
                        vec![group(
                            form_label(0xA10),
                            TEMPORARY_GROUP,
                            vec![record("NAVM", 0xA11, vec![])],
                        )],
                    ),
                ],
            ),
            group(*b"LAND", 0, vec![record("LAND", 0xA01, vec![])]),
            group(*b"NAVM", 0, vec![record("NAVM", 0xA02, vec![])]),
        ];

        let report = audit(root);
        let world = &report.worldspaces[0];
        assert_eq!(world.anomalies.duplicate_cell_coordinates.len(), 1);
        assert_eq!(world.anomalies.cells_with_duplicate_land.len(), 1);
        assert_eq!(world.anomalies.cells_missing_child_groups.len(), 1);
        assert_eq!(world.anomalies.cells_missing_land.len(), 1);
        assert_eq!(world.anomalies.orphan_land_records.len(), 1);
        assert_eq!(world.anomalies.orphan_navm_records.len(), 1);
        assert_eq!(world.anomalies.misplaced_navm_records.len(), 1);
        assert_eq!(report.anomalies.flat_exterior_cells.len(), 1);
        assert_eq!(report.anomalies.flat_land_records.len(), 1);
        assert_eq!(report.anomalies.flat_navm_records.len(), 2);
        assert!(
            report
                .anomalies
                .flat_navm_records
                .iter()
                .any(|record| record.form_id == "00000A03")
        );
        assert!(
            report
                .anomalies
                .flat_navm_records
                .iter()
                .all(|record| record.form_id != "00000A11")
        );
        assert_eq!(report.anomalies.duplicate_record_form_ids.len(), 1);
        assert_eq!(report.summary.duplicate_cell_coordinates, 1);
        assert_eq!(report.summary.orphan_land_records, 1);
        assert_eq!(report.summary.misplaced_navm_records, 1);
    }
}
