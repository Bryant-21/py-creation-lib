//! Writes a cut patch layout into the bytes of the plugin a seed stood in
//! for. Only the CELL and WRLD trees change: a CELL the plugin has keeps its
//! own fields and takes the new stamps, a reference it has takes the new
//! version-control date, and what it lacks joins in the block layout the
//! game expects. Every other record is copied byte for byte.

use esp_authoring_core::plugin_runtime::remap_record_bytes;

use crate::error::{PrevisError, Result};
use crate::metadata::{REPLACED, encode_subrecord};
use crate::patch::{
    CellLayout, GROUP_CELL_CHILDREN, GROUP_CELL_PERSISTENT, GROUP_CELL_TEMPORARY, GROUP_EXTERIOR_BLOCK, GROUP_EXTERIOR_SUB_BLOCK,
    GROUP_INTERIOR_BLOCK, GROUP_INTERIOR_SUB_BLOCK, GROUP_TOP, GROUP_WORLD_CHILDREN, Layout, cell_grid, grid_label, payload,
    with_payload,
};
use crate::plugin::{HEADER_SIZE, SubrecordIter, read_u32};

const GAME: &str = "fo4";
const PLUGIN_FLAG_ESM: u32 = 0x1;
const PLUGIN_FLAG_ESL: u32 = 0x200;
/// Fallout 4's top-group order; a new CELL or WRLD group goes before the
/// first existing group that follows it here.
const TOP_GROUP_ORDER: [&[u8; 4]; 26] = [
    b"LVLI", b"WTHR", b"CLMT", b"SPGD", b"RFCT", b"REGN", b"NAVI", b"CELL", b"WRLD", b"QUST", b"IDLE", b"PACK", b"CSTY",
    b"LSCR", b"LVSP", b"ANIO", b"WATR", b"EFSH", b"EXPL", b"DEBR", b"IMGS", b"IMAD", b"FLST", b"PERK", b"BPTD", b"ADDN",
];

enum Item {
    Record(Vec<u8>),
    Group { label: [u8; 4], kind: i32, stamp: [u8; 8], children: Vec<Item> },
}

impl Item {
    fn group(label: [u8; 4], kind: i32) -> Self {
        Item::Group { label, kind, stamp: [0; 8], children: Vec::new() }
    }

    fn form_id(&self) -> Option<u32> {
        match self {
            Item::Record(bytes) => read_u32(bytes, 12),
            Item::Group { .. } => None,
        }
    }

    fn is_group(&self, label: [u8; 4], kind: i32) -> bool {
        matches!(self, Item::Group { label: l, kind: k, .. } if *l == label && *k == kind)
    }

    fn children(&mut self) -> &mut Vec<Item> {
        match self {
            Item::Group { children, .. } => children,
            Item::Record(_) => unreachable!("only groups have children"),
        }
    }
}

pub(crate) struct Merged {
    pub(crate) bytes: Vec<u8>,
    /// Seed FormIDs of the CELLs the plugin did not have.
    pub(crate) added_cells: Vec<u32>,
    pub(crate) redated_children: usize,
    pub(crate) added_children: usize,
}

fn parse(bytes: &[u8]) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let header = bytes.get(offset..offset + HEADER_SIZE).ok_or_else(|| PrevisError::invalid("truncated record header"))?;
        let size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        if &header[..4] == b"GRUP" {
            let body = bytes.get(offset + HEADER_SIZE..offset + size).ok_or_else(|| PrevisError::invalid("truncated group"))?;
            items.push(Item::Group {
                label: header[8..12].try_into().unwrap(),
                kind: i32::from_le_bytes(header[12..16].try_into().unwrap()),
                stamp: header[16..24].try_into().unwrap(),
                children: parse(body)?,
            });
            offset += size;
        } else {
            let end = offset + HEADER_SIZE + size;
            items.push(Item::Record(bytes.get(offset..end).ok_or_else(|| PrevisError::invalid("truncated record"))?.to_vec()));
            offset = end;
        }
    }
    Ok(items)
}

fn write(items: &[Item], out: &mut Vec<u8>) -> usize {
    let mut count = 0;
    for item in items {
        count += 1;
        match item {
            Item::Record(bytes) => out.extend_from_slice(bytes),
            Item::Group { label, kind, stamp, children } => {
                let start = out.len();
                out.extend_from_slice(b"GRUP");
                out.extend_from_slice(&[0; 4]);
                out.extend_from_slice(label);
                out.extend_from_slice(&kind.to_le_bytes());
                out.extend_from_slice(stamp);
                count += write(children, out);
                let size = (out.len() - start) as u32;
                out[start + 4..start + 8].copy_from_slice(&size.to_le_bytes());
            }
        }
    }
    count
}

/// The plugin's stamps replaced by `stamped`'s: present ones set, missing ones removed.
fn restamp(existing: &[u8], stamped: &[u8]) -> Result<Vec<u8>> {
    let new_stamps: Vec<([u8; 4], Vec<u8>)> =
        SubrecordIter::new(&payload(stamped)?).filter(|(s, _)| REPLACED.contains(&s)).map(|(s, d)| (s, d.to_vec())).collect();
    let decoded = payload(existing)?;
    let mut encoded = Vec::with_capacity(decoded.len() + 64);
    let mut written = Vec::new();
    for (signature, data) in SubrecordIter::new(&decoded) {
        if !REPLACED.contains(&&signature) {
            encode_subrecord(&mut encoded, &signature, data);
        } else if let Some((_, data)) = new_stamps.iter().find(|(s, _)| *s == signature).filter(|_| !written.contains(&signature)) {
            encode_subrecord(&mut encoded, &signature, data);
            written.push(signature);
        }
    }
    for (signature, data) in new_stamps.iter().filter(|(s, _)| !written.contains(s)) {
        encode_subrecord(&mut encoded, signature, data);
    }
    with_payload(existing, encoded)
}

struct Merger {
    masters: Vec<String>,
    esm: bool,
    added_cells: Vec<u32>,
    redated_children: usize,
    added_children: usize,
    overridden: Vec<u32>,
}

impl Merger {
    fn own(&self) -> u32 {
        self.masters.len() as u32
    }

    /// A CELL with its children into `siblings`: restamped where the plugin
    /// has it, else added to the end of the list `place` picks (or its
    /// front, for a world's persistent CELL, which precedes the blocks).
    fn cell(
        &mut self,
        siblings: &mut Vec<Item>,
        seed_id: u32,
        cell: &RemappedCell,
        front: bool,
        place: impl FnOnce(&mut Vec<Item>) -> &mut Vec<Item>,
    ) -> Result<()> {
        let form_id = read_u32(&cell.record, 12).unwrap();
        match locate(siblings, form_id) {
            Some(path) => {
                let parent = parent_mut(siblings, &path[..path.len() - 1]);
                let position = path[path.len() - 1];
                let Item::Record(existing) = &mut parent[position] else { unreachable!() };
                *existing = restamp(existing, &cell.record)?;
                self.children(parent, position, form_id, cell);
            }
            None => {
                self.added_cells.push(seed_id);
                let list = place(siblings);
                let position = if front { 0 } else { list.len() };
                list.insert(position, Item::Record(cell.record.clone()));
                self.children(list, position, form_id, cell);
            }
        }
        Ok(())
    }

    fn children(&mut self, siblings: &mut Vec<Item>, cell_position: usize, form_id: u32, cell: &RemappedCell) {
        let label = form_id.to_le_bytes();
        if cell.persistent.is_empty() && cell.temporary.is_empty() {
            return;
        }
        let group = match siblings.iter().position(|i| i.is_group(label, GROUP_CELL_CHILDREN)) {
            Some(position) => position,
            None => {
                siblings.insert(cell_position + 1, Item::group(label, GROUP_CELL_CHILDREN));
                cell_position + 1
            }
        };
        let groups = siblings[group].children();
        for (records, kind) in [(&cell.persistent, GROUP_CELL_PERSISTENT), (&cell.temporary, GROUP_CELL_TEMPORARY)] {
            for record in records {
                let child = read_u32(record, 12).unwrap();
                let existing = groups.iter_mut().flat_map(|g| match g {
                    Item::Group { children, .. } => children.iter_mut().collect::<Vec<_>>(),
                    Item::Record(_) => Vec::new(),
                });
                if let Some(Item::Record(bytes)) = existing.into_iter().find(|i| i.form_id() == Some(child)) {
                    bytes[16..18].copy_from_slice(&record[16..18]);
                    self.redated_children += 1;
                    continue;
                }
                let sub = match groups.iter().position(|g| g.is_group(label, kind)) {
                    Some(position) => position,
                    None => {
                        let at = if kind == GROUP_CELL_PERSISTENT { 0 } else { groups.len() };
                        groups.insert(at, Item::group(label, kind));
                        at
                    }
                };
                groups[sub].children().push(Item::Record(record.clone()));
                self.added_children += 1;
                if self.esm && kind == GROUP_CELL_TEMPORARY && child >> 24 < self.own() {
                    self.overridden.push(child);
                }
            }
        }
    }
}

struct RemappedCell {
    record: Vec<u8>,
    persistent: Vec<Vec<u8>>,
    temporary: Vec<Vec<u8>>,
}

/// Path of the record with `form_id` under `items`, searching groups too.
fn locate(items: &[Item], form_id: u32) -> Option<Vec<usize>> {
    for (index, item) in items.iter().enumerate() {
        match item {
            Item::Record(_) if item.form_id() == Some(form_id) => return Some(vec![index]),
            Item::Group { kind, children, .. } if *kind != GROUP_CELL_CHILDREN => {
                if let Some(mut path) = locate(children, form_id) {
                    path.insert(0, index);
                    return Some(path);
                }
            }
            _ => {}
        }
    }
    None
}

fn parent_mut<'a>(items: &'a mut Vec<Item>, path: &[usize]) -> &'a mut Vec<Item> {
    path.iter().fold(items, |list, &index| list[index].children())
}

/// The group `(label, kind)` in `items`, added at the end when missing.
fn group_mut(items: &mut Vec<Item>, label: [u8; 4], kind: i32) -> &mut Vec<Item> {
    let position = items.iter().position(|i| i.is_group(label, kind)).unwrap_or_else(|| {
        items.push(Item::group(label, kind));
        items.len() - 1
    });
    items[position].children()
}

fn top_group(items: &mut Vec<Item>, label: &[u8; 4]) -> usize {
    if let Some(position) = items.iter().position(|i| i.is_group(*label, GROUP_TOP)) {
        return position;
    }
    let later = &TOP_GROUP_ORDER[TOP_GROUP_ORDER.iter().position(|l| l == &label).unwrap() + 1..];
    let at = items
        .iter()
        .position(|i| matches!(i, Item::Group { label: l, kind: GROUP_TOP, .. } if later.contains(&l)))
        .unwrap_or(items.len());
    items.insert(at, Item::group(*label, GROUP_TOP));
    at
}

pub(crate) fn merge(target: &[u8], seed_masters: &[String], name: &str, layout: &Layout) -> Result<Merged> {
    let header_size = HEADER_SIZE + u32::from_le_bytes(target[4..8].try_into().unwrap()) as usize;
    if &target[..4] != b"TES4" || target.len() < header_size {
        return Err(PrevisError::invalid(format!("{name} is not a plugin")));
    }
    let mut header = target[..header_size].to_vec();
    let flags = u32::from_le_bytes(header[8..12].try_into().unwrap());
    if flags & PLUGIN_FLAG_ESL != 0 {
        return Err(PrevisError::unsupported(format!("{name} is ESL-flagged; the game ignores precombines and previs in light plugins")));
    }
    let masters: Vec<String> = SubrecordIter::new(&payload(&header)?)
        .filter(|(s, _)| s == b"MAST")
        .map(|(_, d)| String::from_utf8_lossy(d.split(|&b| b == 0).next().unwrap_or_default()).into_owned())
        .collect();
    let remap = |bytes: &[u8]| -> Result<Vec<u8>> {
        remap_record_bytes(bytes, GAME, seed_masters, name, &masters, name)
            .map_err(|e| PrevisError::unsupported(format!("{name} needs another master to hold its new previsbines: {e}")))
    };
    let remap_cell = |cell: &CellLayout| -> Result<RemappedCell> {
        Ok(RemappedCell {
            record: remap(&cell.record)?,
            persistent: cell.persistent.iter().map(|r| remap(r)).collect::<Result<_>>()?,
            temporary: cell.temporary.iter().map(|r| remap(r)).collect::<Result<_>>()?,
        })
    };
    let mut items = parse(&target[header_size..])?;
    let mut merger = Merger { masters: masters.clone(), esm: flags & PLUGIN_FLAG_ESM != 0, added_cells: Vec::new(), redated_children: 0, added_children: 0, overridden: Vec::new() };

    for (&seed_id, cell) in &layout.interiors {
        let cell = remap_cell(cell)?;
        let object = read_u32(&cell.record, 12).unwrap() & 0x00FF_FFFF;
        let (block, sub_block) = ((object % 10) as i32, ((object / 10) % 10) as i32);
        merger.cell(&mut items, seed_id, &cell, false, |items| {
            let top = top_group(items, b"CELL");
            let blocks = group_mut(items[top].children(), block.to_le_bytes(), GROUP_INTERIOR_BLOCK);
            group_mut(blocks, sub_block.to_le_bytes(), GROUP_INTERIOR_SUB_BLOCK)
        })?;
    }
    if !layout.worlds.is_empty() {
        let top = top_group(&mut items, b"WRLD");
        for world in layout.worlds.values() {
            let record = remap(&world.record)?;
            let world_id = read_u32(&record, 12).unwrap();
            let worlds = items[top].children();
            let position = match worlds.iter().position(|i| i.form_id() == Some(world_id)) {
                Some(position) => position,
                None => {
                    worlds.push(Item::Record(record));
                    worlds.len() - 1
                }
            };
            let label = world_id.to_le_bytes();
            let children = match worlds.iter().position(|i| i.is_group(label, GROUP_WORLD_CHILDREN)) {
                Some(children) => children,
                None => {
                    worlds.insert(position + 1, Item::group(label, GROUP_WORLD_CHILDREN));
                    position + 1
                }
            };
            let children = worlds[children].children();
            if let Some(persistent) = &world.persistent {
                let cell = remap_cell(persistent)?;
                let seed_id = read_u32(&persistent.record, 12).unwrap();
                merger.cell(children, seed_id, &cell, true, |cells| cells)?;
            }
            for (&(_, seed_id), cell) in &world.cells {
                let cell = remap_cell(cell)?;
                let (x, y) = cell_grid(&cell.record)?;
                merger.cell(children, seed_id, &cell, false, |cells| {
                    let blocks = group_mut(cells, grid_label(x.div_euclid(32), y.div_euclid(32)), GROUP_EXTERIOR_BLOCK);
                    group_mut(blocks, grid_label(x.div_euclid(8), y.div_euclid(8)), GROUP_EXTERIOR_SUB_BLOCK)
                })?;
            }
        }
    }

    let mut body = Vec::with_capacity(target.len() + 4096);
    let count = write(&items, &mut body) as u32;
    let mut fields: Vec<([u8; 4], Vec<u8>)> = SubrecordIter::new(&payload(&header)?).map(|(s, d)| (s, d.to_vec())).collect();
    if let Some((_, hedr)) = fields.iter_mut().find(|(s, _)| s == b"HEDR").filter(|(_, d)| d.len() >= 8) {
        hedr[4..8].copy_from_slice(&count.to_le_bytes());
    }
    if !merger.overridden.is_empty() {
        let onam = match fields.iter().position(|(s, _)| s == b"ONAM") {
            Some(position) => position,
            None => {
                let after = fields.iter().rposition(|(s, _)| s == b"MAST" || s == b"DATA").map_or(fields.len(), |p| p + 1);
                fields.insert(after, (*b"ONAM", Vec::new()));
                after
            }
        };
        for id in &merger.overridden {
            fields[onam].1.extend_from_slice(&id.to_le_bytes());
        }
    }
    let mut encoded = Vec::new();
    for (signature, data) in &fields {
        encode_subrecord(&mut encoded, signature, data);
    }
    header = with_payload(&header, encoded)?;
    let mut bytes = header;
    bytes.extend_from_slice(&body);
    Ok(Merged { bytes, added_cells: merger.added_cells, redated_children: merger.redated_children, added_children: merger.added_children })
}
