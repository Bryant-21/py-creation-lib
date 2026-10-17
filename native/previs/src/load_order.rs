//! Whole-load-order record index: override chains, winning records and CELL
//! children merged across every plugin, keyed by load-order-independent ids.

use std::path::PathBuf;

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::error::{PrevisError, Result};
use crate::plugin::{Plugin, RecordRef};

/// `(load-order index of the defining plugin) << 24 | object id`.
pub type GlobalId = u64;

pub const PLUGIN_FLAG_ESM: u32 = 0x0000_0001;
pub const PLUGIN_FLAG_ESL: u32 = 0x0000_0200;
/// CELL "partial form": the game ignores the override, LAND under it included.
pub const RECORD_FLAG_PARTIAL: u32 = 0x0000_4000;

/// CELL child signatures detection reads; navmeshes and actors are skipped.
const INDEXED_CHILDREN: [&[u8; 4]; 5] = [b"REFR", b"PGRE", b"PMIS", b"PHZD", b"LAND"];

pub fn global_id(plugin: usize, object_id: u32) -> GlobalId {
    ((plugin as u64) << 24) | (object_id & 0x00FF_FFFF) as u64
}

pub fn defining_plugin(id: GlobalId) -> usize {
    (id >> 24) as usize
}

pub fn object_id(id: GlobalId) -> u32 {
    (id & 0x00FF_FFFF) as u32
}

/// One plugin's copy of a record.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub plugin: usize,
    pub record: RecordRef,
    /// The CELL this copy is placed in (children only).
    pub home: Option<GlobalId>,
    pub persistent: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CellPlacement {
    pub world: Option<GlobalId>,
    /// In an exterior block (a grid CELL), as opposed to an interior or the
    /// world's persistent CELL.
    pub exterior: bool,
}

pub struct LoadOrderIndex {
    pub plugins: Vec<Plugin>,
    header_flags: Vec<u32>,
    /// Per plugin: local master byte to load-order index.
    master_maps: Vec<Vec<usize>>,
    /// Per plugin: load-order index to local master byte.
    local_maps: Vec<FxHashMap<usize, u32>>,
    /// Per plugin: every plugin it masters, recursively.
    master_closure: Vec<FxHashSet<usize>>,
    chains: FxHashMap<GlobalId, Vec<Entry>>,
    cell_children: FxHashMap<GlobalId, Vec<GlobalId>>,
    cells: FxHashMap<GlobalId, CellPlacement>,
}

impl LoadOrderIndex {
    /// Opens `paths` in load order. Every master must load earlier.
    pub fn open(paths: &[PathBuf]) -> Result<Self> {
        let plugins: Vec<Plugin> = paths.par_iter().map(|path| Plugin::open(path)).collect::<Result<_>>()?;
        Self::from_plugins(plugins)
    }

    pub fn from_plugins(plugins: Vec<Plugin>) -> Result<Self> {
        let mut by_name: FxHashMap<String, usize> = FxHashMap::default();
        let mut master_maps = Vec::with_capacity(plugins.len());
        let mut missing = Vec::new();
        for (index, plugin) in plugins.iter().enumerate() {
            let mut map = Vec::with_capacity(plugin.masters.len());
            for master in &plugin.masters {
                match by_name.get(&master.to_ascii_lowercase()) {
                    Some(&position) => map.push(position),
                    None => {
                        missing.push(format!("{} requires {master}", plugin.name));
                        map.push(usize::MAX);
                    }
                }
            }
            master_maps.push(map);
            if by_name.insert(plugin.name.to_ascii_lowercase(), index).is_some() {
                return Err(PrevisError::invalid(format!("{} is listed twice", plugin.name)));
            }
        }
        if !missing.is_empty() {
            return Err(PrevisError::invalid(format!("missing or late masters: {}", missing.join("; "))));
        }
        let local_maps = master_maps
            .iter()
            .enumerate()
            .map(|(index, map)| {
                let mut local: FxHashMap<usize, u32> = map.iter().enumerate().map(|(byte, &p)| (p, byte as u32)).collect();
                local.insert(index, map.len() as u32);
                local
            })
            .collect();
        let mut master_closure: Vec<FxHashSet<usize>> = Vec::with_capacity(plugins.len());
        for map in &master_maps {
            let mut closure = FxHashSet::default();
            for &master in map {
                closure.insert(master);
                closure.extend(master_closure[master].iter().copied());
            }
            master_closure.push(closure);
        }
        let header_flags = plugins
            .iter()
            .map(|p| u32::from_le_bytes(p.bytes()[8..12].try_into().unwrap()))
            .collect();
        let mut index = Self {
            plugins,
            header_flags,
            master_maps,
            local_maps,
            master_closure,
            chains: FxHashMap::default(),
            cell_children: FxHashMap::default(),
            cells: FxHashMap::default(),
        };
        index.build_chains();
        Ok(index)
    }

    fn build_chains(&mut self) {
        let per_plugin: Vec<Vec<(GlobalId, Entry)>> = (0..self.plugins.len())
            .into_par_iter()
            .map(|p| self.plugin_entries(p))
            .collect();
        let mut children_seen: FxHashSet<(GlobalId, GlobalId)> = FxHashSet::default();
        for entries in per_plugin {
            for (id, entry) in entries {
                if let Some(home) = entry.home {
                    if children_seen.insert((home, id)) {
                        self.cell_children.entry(home).or_default().push(id);
                    }
                }
                self.chains.entry(id).or_default().push(entry);
            }
        }
        for p in 0..self.plugins.len() {
            let plugin = &self.plugins[p];
            for (&cell, topology) in &plugin.cells {
                if plugin.record(cell).is_none_or(|r| &r.signature != b"CELL") {
                    continue;
                }
                let id = self.resolve(p, cell);
                let world = topology.world.map(|w| self.resolve(p, w));
                let exterior = topology.world.is_some_and(|w| plugin.world_cells.get(&w).is_some_and(|c| c.contains(&cell)));
                self.cells.entry(id).or_insert(CellPlacement { world, exterior });
            }
        }
    }

    fn plugin_entries(&self, p: usize) -> Vec<(GlobalId, Entry)> {
        let plugin = &self.plugins[p];
        let mut out = Vec::new();
        for record in plugin.records_of(b"CELL") {
            out.push((self.resolve(p, record.form_id), Entry { plugin: p, record, home: None, persistent: false }));
        }
        for (&cell, topology) in &plugin.cells {
            let home = self.resolve(p, cell);
            for (records, persistent) in [(&topology.temporary, false), (&topology.persistent, true)] {
                for record in records.iter().filter(|r| INDEXED_CHILDREN.contains(&&r.signature)) {
                    let entry = Entry { plugin: p, record: *record, home: Some(home), persistent };
                    out.push((self.resolve(p, record.form_id), entry));
                }
            }
        }
        out
    }

    /// The load-order identity of a raw FormID read from plugin `p`.
    pub fn resolve(&self, p: usize, raw: u32) -> GlobalId {
        let byte = (raw >> 24) as usize;
        let owner = self.master_maps[p].get(byte).copied().unwrap_or(p);
        global_id(owner, raw)
    }

    /// The raw FormID plugin `p` uses for `id`, if `p` can see it.
    pub fn raw_in(&self, p: usize, id: GlobalId) -> Option<u32> {
        let byte = self.local_maps[p].get(&defining_plugin(id))?;
        Some((byte << 24) | object_id(id))
    }

    pub fn is_esm(&self, p: usize) -> bool {
        self.header_flags[p] & PLUGIN_FLAG_ESM != 0
    }

    pub fn is_esl(&self, p: usize) -> bool {
        self.header_flags[p] & PLUGIN_FLAG_ESL != 0
    }

    /// Whether `master` is a direct or indirect master of `plugin`.
    pub fn is_master_of(&self, master: usize, plugin: usize) -> bool {
        self.master_closure[plugin].contains(&master)
    }

    /// Every copy of a CELL or indexed child, in load order.
    pub fn chain(&self, id: GlobalId) -> &[Entry] {
        self.chains.get(&id).map_or(&[], Vec::as_slice)
    }

    /// The copy the game uses: the last one, skipping partial CELL overrides.
    pub fn winner(&self, id: GlobalId) -> Option<&Entry> {
        self.chain(id).iter().rev().find(|e| !is_partial(e))
    }

    pub fn cell_ids(&self) -> impl Iterator<Item = (GlobalId, CellPlacement)> + '_ {
        self.cells.iter().map(|(&id, &placement)| (id, placement))
    }

    pub fn placement(&self, cell: GlobalId) -> Option<CellPlacement> {
        self.cells.get(&cell).copied()
    }

    /// Every indexed child any plugin places in `cell`.
    pub fn children(&self, cell: GlobalId) -> &[GlobalId] {
        self.cell_children.get(&cell).map_or(&[], Vec::as_slice)
    }

    /// The winning copy of any record (bases, swaps, worlds): last plugin that has it.
    pub fn record(&self, id: GlobalId) -> Option<(usize, RecordRef)> {
        (defining_plugin(id)..self.plugins.len()).rev().find_map(|p| {
            let raw = self.raw_in(p, id)?;
            self.plugins[p].record(raw).map(|record| (p, record))
        })
    }

    /// The defining copy of any record.
    pub fn original(&self, id: GlobalId) -> Option<RecordRef> {
        let p = defining_plugin(id);
        self.plugins.get(p)?.record(self.raw_in(p, id)?)
    }
}

pub fn is_partial(entry: &Entry) -> bool {
    &entry.record.signature == b"CELL" && entry.record.flags & RECORD_FLAG_PARTIAL != 0
}

/// The record header's version-control date (low half of VCI1); 0 is none.
pub fn version_date(plugin: &Plugin, record: &RecordRef) -> u16 {
    let bytes = plugin.bytes();
    u16::from_le_bytes([bytes[record.offset + 16], bytes[record.offset + 17]])
}

/// Packed dates compare as `(year, month, day)`: the bit layout already orders them.
pub fn date_label(date: u16) -> String {
    if date == 0 {
        return "none".to_string();
    }
    format!("{:04}-{:02}-{:02}", 2000 + (date >> 9), (date >> 5) & 0x0F, date & 0x1F)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a minimal plugin file: header with masters, then top-level records/groups.
    pub(crate) struct PluginBuilder {
        flags: u32,
        masters: Vec<String>,
        body: Vec<u8>,
    }

    pub(crate) fn subrecord(signature: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = signature.to_vec();
        out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    pub(crate) fn record(signature: &[u8; 4], form_id: u32, flags: u32, date: u16, fields: &[Vec<u8>]) -> Vec<u8> {
        let payload: Vec<u8> = fields.concat();
        let mut out = signature.to_vec();
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&form_id.to_le_bytes());
        out.extend_from_slice(&(date as u32).to_le_bytes());
        out.extend_from_slice(&[131, 0, 0, 0]);
        out.extend_from_slice(&payload);
        out
    }

    pub(crate) fn group(label: u32, group_type: u32, children: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = children.concat();
        let mut out = b"GRUP".to_vec();
        out.extend_from_slice(&((24 + body.len()) as u32).to_le_bytes());
        out.extend_from_slice(&label.to_le_bytes());
        out.extend_from_slice(&group_type.to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&body);
        out
    }

    impl PluginBuilder {
        pub(crate) fn new(flags: u32, masters: &[&str]) -> Self {
            Self { flags, masters: masters.iter().map(|m| m.to_string()).collect(), body: Vec::new() }
        }

        pub(crate) fn push(mut self, bytes: Vec<u8>) -> Self {
            self.body.extend(bytes);
            self
        }

        pub(crate) fn write(&self, dir: &std::path::Path, name: &str) -> PathBuf {
            let mut fields = vec![subrecord(b"HEDR", &[0; 12])];
            for master in &self.masters {
                let mut zstr = master.as_bytes().to_vec();
                zstr.push(0);
                fields.push(subrecord(b"MAST", &zstr));
                fields.push(subrecord(b"DATA", &[0; 8]));
            }
            let mut bytes = record(b"TES4", 0, self.flags, 0, &fields);
            bytes.extend_from_slice(&self.body);
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }

    /// An interior CELL with temporary children, as a top-level CELL group.
    pub(crate) fn interior_cell(form_id: u32, flags: u32, date: u16, fields: &[Vec<u8>], temporary: &[Vec<u8>]) -> Vec<u8> {
        let mut cell_fields = vec![subrecord(b"DATA", &1u16.to_le_bytes())];
        cell_fields.extend_from_slice(fields);
        let mut children = vec![record(b"CELL", form_id, flags, date, &cell_fields)];
        if !temporary.is_empty() {
            children.push(group(form_id, 6, &[group(form_id, 9, temporary)]));
        }
        group(u32::from_le_bytes(*b"CELL"), 0, &[group(0, 2, &[group(0, 3, &children)])])
    }

    pub(crate) fn reference(form_id: u32, flags: u32, date: u16, base: u32) -> Vec<u8> {
        record(b"REFR", form_id, flags, date, &[subrecord(b"NAME", &base.to_le_bytes()), subrecord(b"DATA", &[0; 24])])
    }

    #[test]
    fn chains_resolve_overrides_across_master_numbering() {
        let dir = tempfile::tempdir().unwrap();
        let master = PluginBuilder::new(PLUGIN_FLAG_ESM, &[])
            .push(interior_cell(0x0000_0800, 0, 10, &[], &[reference(0x0000_0801, 0, 10, 0x0000_0900)]))
            .write(dir.path(), "Master.esm");
        let other = PluginBuilder::new(PLUGIN_FLAG_ESL, &[]).write(dir.path(), "Other.esl");
        // The mod lists an unrelated master first, so Master.esm is local byte 1.
        let modded = PluginBuilder::new(0, &["Other.esl", "Master.esm"])
            .push(interior_cell(0x0100_0800, RECORD_FLAG_PARTIAL, 20, &[], &[reference(0x0100_0801, 0, 20, 0x0100_0900), reference(0x0200_0802, 0, 20, 0x0100_0900)]))
            .write(dir.path(), "Mod.esp");
        let index = LoadOrderIndex::open(&[master, other, modded]).unwrap();
        let cell = global_id(0, 0x800);
        assert_eq!(index.chain(cell).len(), 2);
        assert_eq!(index.winner(cell).unwrap().plugin, 0, "partial overrides never win");
        let reference_id = global_id(0, 0x801);
        assert_eq!(index.winner(reference_id).unwrap().plugin, 2);
        assert_eq!(index.children(cell), &[reference_id, global_id(2, 0x802)]);
        assert_eq!(index.raw_in(2, cell), Some(0x0100_0800));
        assert!(index.is_master_of(0, 2) && !index.is_master_of(2, 0));
        assert!(index.is_esl(1) && index.is_esm(0));
        assert_eq!(version_date(&index.plugins[2], &index.winner(reference_id).unwrap().record), 20);
    }

    #[test]
    fn missing_masters_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let modded = PluginBuilder::new(0, &["Absent.esm"]).write(dir.path(), "Mod.esp");
        let error = LoadOrderIndex::open(&[modded]).err().unwrap().to_string();
        assert!(error.contains("Mod.esp requires Absent.esm"), "{error}");
    }

    #[test]
    fn dates_label_as_calendar_days() {
        assert_eq!(date_label(crate::metadata::pack_generation_date(2026, 10, 11).unwrap()), "2026-10-11");
        assert_eq!(date_label(0), "none");
    }
}
