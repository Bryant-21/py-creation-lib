//! Landscape layers: LTEX -> TXST (TX00 diffuse, TX01 normal, TX07 smooth spec).
//!
//! FO4CS consumes per-layer PBR terrain only through the raw BTD4 material
//! bank, whose per-material files are `<stem>_source_d.dds` (BC3 sRGB, height in
//! A), `<stem>_source_n.dds` (BC5 SNORM), `<stem>_source_l.dds` (BC3: R
//! smoothness, G AO, A emission mask), all one size with full mips, and
//! `<stem>_source_r.dds` (grey F0, BC4, 256x256 for every material). This module
//! stages those files for FO4's own landscape textures and reads each worldspace's
//! LAND layers; `terrain_btd4` writes the BTD4 sidecar that delivers them.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::classify::MaterialClass;
use crate::plan::{Unit, UnitKind};
use crate::sources::AssetSources;

#[derive(Debug, Clone, PartialEq)]
pub struct TerrainLayerSource {
    pub diffuse: String,
    pub normal: String,
    pub smoothspec: Option<String>,
}

pub type GlobalId = (String, u32);

#[derive(Debug, Clone, Default)]
pub struct TerrainRecords {
    /// LTEX -> TXST (global ids), later plugins override.
    pub ltex: BTreeMap<GlobalId, GlobalId>,
    /// TXST -> (diffuse, normal, smooth spec) texture keys.
    pub txst: HashMap<GlobalId, (String, String, String)>,
    /// WRLD -> its exterior LAND by cell grid; later plugins override per cell.
    pub worldspaces: BTreeMap<GlobalId, crate::terrain_btd4::WorldLand>,
}

impl TerrainRecords {
    /// Diffuse texture key an LTEX resolves to, if its texture set is known.
    pub fn ltex_diffuse(&self, ltex: &GlobalId) -> Option<&str> {
        self.ltex.get(ltex).and_then(|txst| self.txst.get(txst)).map(|t| t.0.as_str()).filter(|d| !d.is_empty())
    }
}

#[cfg(feature = "game-data")]
mod reader {
    use super::*;
    use crate::sources::texture_key;
    use esp_authoring_core::plugin_runtime::{ParsedItem, ParsedPlugin, ParsedRecord, parse_plugin_file};

    fn visit<'a>(items: &'a [ParsedItem], out: &mut Vec<&'a ParsedRecord>) {
        for item in items {
            match item {
                ParsedItem::Record(record) if matches!(record.signature.as_str(), "LTEX" | "TXST") => out.push(record),
                ParsedItem::Group(group) => visit(&group.children, out),
                _ => {}
            }
        }
    }

    fn subrecord<'a>(record: &'a ParsedRecord, sig: &str) -> Option<&'a [u8]> {
        record.subrecords.iter().find(|s| s.signature.as_str() == sig).map(|s| s.data.as_ref())
    }

    fn zstring(data: &[u8]) -> String {
        let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        String::from_utf8_lossy(&data[..end]).into_owned()
    }

    fn u32_at(data: &[u8]) -> Option<u32> {
        data.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Plugin-local form id -> (owning file, object id) through the master list.
    fn global_id(plugin: &ParsedPlugin, form_id: u32) -> (String, u32) {
        let index = (form_id >> 24) as usize;
        let owner = plugin
            .header
            .masters
            .get(index)
            .cloned()
            .unwrap_or_else(|| plugin.plugin_name.clone());
        (owner.to_ascii_lowercase(), form_id & 0x00FF_FFFF)
    }

    fn label_u32(label: &[u8; 4]) -> u32 {
        u32::from_le_bytes(*label)
    }

    struct WorldWalk<'a> {
        plugin: &'a ParsedPlugin,
        world: Option<GlobalId>,
        cell: Option<(i32, i32)>,
        cell_grid: HashMap<u32, (i32, i32)>,
    }

    impl WorldWalk<'_> {
        fn items(&mut self, items: &[ParsedItem], out: &mut TerrainRecords) {
            for item in items {
                match item {
                    ParsedItem::Record(record) => self.record(record, out),
                    ParsedItem::Group(group) => {
                        let (world, cell) = (self.world.clone(), self.cell);
                        match group.group_type {
                            // World children: the label is the WRLD form id.
                            1 => self.world = Some(global_id(self.plugin, label_u32(&group.label))),
                            // Cell children: the label is the CELL form id.
                            6 => self.cell = self.cell_grid.get(&label_u32(&group.label)).copied(),
                            _ => {}
                        }
                        if group.group_type == 0 && &group.label != b"WRLD" {
                            continue;
                        }
                        self.items(&group.children, out);
                        (self.world, self.cell) = (world, cell);
                    }
                }
            }
        }

        fn record(&mut self, record: &ParsedRecord, out: &mut TerrainRecords) {
            match record.signature.as_str() {
                "WRLD" => {
                    let editor_id = subrecord(record, "EDID").map(zstring).unwrap_or_default();
                    out.worldspaces.entry(global_id(self.plugin, record.form_id)).or_default().editor_id = editor_id;
                }
                "CELL" if self.world.is_some() => {
                    if let Some(xclc) = subrecord(record, "XCLC").filter(|d| d.len() >= 8) {
                        let x = i32::from_le_bytes(xclc[0..4].try_into().unwrap());
                        let y = i32::from_le_bytes(xclc[4..8].try_into().unwrap());
                        self.cell_grid.insert(record.form_id, (x, y));
                    }
                }
                "LAND" => {
                    let (Some(world), Some(cell)) = (self.world.clone(), self.cell) else { return };
                    let land = crate::terrain_btd4::land_cell(
                        record.subrecords.iter().map(|s| (s.signature.as_str(), s.data.as_ref())),
                        |form_id| global_id(self.plugin, form_id),
                    );
                    out.worldspaces.entry(world).or_default().cells.insert(cell, land);
                }
                _ => {}
            }
        }
    }

    pub fn read_records(plugins: &[PathBuf]) -> Result<TerrainRecords, String> {
        let mut out = TerrainRecords::default();
        for path in plugins {
            let plugin = parse_plugin_file(&path.to_string_lossy(), Some("fo4".into()), true)
                .map_err(|err| format!("parse {}: {err}", path.display()))?;
            WorldWalk { plugin: &plugin, world: None, cell: None, cell_grid: HashMap::new() }.items(&plugin.root_items, &mut out);
            let mut records = Vec::new();
            visit(&plugin.root_items, &mut records);
            for record in records {
                let id = global_id(&plugin, record.form_id);
                match record.signature.as_str() {
                    "LTEX" => {
                        if let Some(txst) = subrecord(record, "TNAM").and_then(u32_at) {
                            out.ltex.insert(id, global_id(&plugin, txst));
                        }
                    }
                    _ => {
                        let slot = |sig| subrecord(record, sig).map(zstring).map(|s| texture_key(&s)).unwrap_or_default();
                        out.txst.insert(id, (slot("TX00"), slot("TX01"), slot("TX07")));
                    }
                }
            }
        }
        Ok(out)
    }
}

#[cfg(feature = "game-data")]
pub use reader::read_records;

#[cfg(not(feature = "game-data"))]
pub fn read_records(_plugins: &[PathBuf]) -> Result<TerrainRecords, String> {
    Err("material_upgrade_native was built without plugin support (feature game-data)".into())
}

pub fn units_from_records(records: &TerrainRecords, sources: &AssetSources) -> Vec<Unit> {
    let mut by_diffuse: BTreeMap<String, Unit> = BTreeMap::new();
    for ((owner, object), txst_id) in &records.ltex {
        let key = format!("ltex:{owner}:{object:06X}");
        let mut u = Unit::for_terrain(&key);
        match records.txst.get(txst_id) {
            Some((diffuse, normal, spec)) if !diffuse.is_empty() => {
                if by_diffuse.contains_key(diffuse) {
                    continue; // one bank entry per texture stem
                }
                u.diffuse = diffuse.clone();
                u.normal = normal.clone();
                u.smoothspec = spec.clone();
                if !sources.contains(diffuse) {
                    u.skip(format!("diffuse missing: {diffuse}"));
                } else if normal.is_empty() || !sources.contains(normal) {
                    u.skip("normal missing");
                }
                by_diffuse.insert(diffuse.clone(), u);
            }
            _ => {
                u.skip("texture set unresolved");
                by_diffuse.insert(key, u);
            }
        }
    }
    by_diffuse.into_values().collect()
}

pub fn terrain_units(plugins: &[PathBuf], sources: &AssetSources) -> Result<(Vec<Unit>, TerrainRecords), String> {
    let records = read_records(plugins)?;
    Ok((units_from_records(&records, sources), records))
}

impl Unit {
    pub(crate) fn for_terrain(key: &str) -> Self {
        let mut u = crate::plan::Unit::for_test_or_terrain(UnitKind::TerrainLayer, key, MaterialClass::Stone);
        u.warnings.push(
            "terrain layer: FO4CS reads it through terrain/fo4cs/<worldspace>.btd4 with [Terrain.Layers] bNativeWorldspaces=1".into(),
        );
        u
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ltex_units_dedupe_by_diffuse_and_report_missing_textures() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir_all(data.join("Textures/Landscape")).unwrap();
        for name in ["Dirt01_d.dds", "Dirt01_n.dds"] {
            std::fs::write(data.join("Textures/Landscape").join(name), b"x").unwrap();
        }
        let sources = AssetSources::open(&[data]).unwrap();
        let mut records = TerrainRecords::default();
        let dirt = ("textures/landscape/dirt01_d.dds".to_string(), "textures/landscape/dirt01_n.dds".to_string(), String::new());
        records.txst.insert(("fallout4.esm".into(), 1), dirt);
        records.txst.insert(("fallout4.esm".into(), 2), ("textures/landscape/gone_d.dds".into(), String::new(), String::new()));
        records.ltex.insert(("fallout4.esm".into(), 10), ("fallout4.esm".into(), 1));
        records.ltex.insert(("fallout4.esm".into(), 11), ("fallout4.esm".into(), 1));
        records.ltex.insert(("fallout4.esm".into(), 12), ("fallout4.esm".into(), 2));
        records.ltex.insert(("fallout4.esm".into(), 13), ("fallout4.esm".into(), 99));
        let units = units_from_records(&records, &sources);
        assert_eq!(units.len(), 3);
        let ok: Vec<_> = units.iter().filter(|u| u.is_upgrade()).collect();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].key, "ltex:fallout4.esm:00000A");
        assert!(units.iter().any(|u| u.reason.as_deref() == Some("texture set unresolved")));
        assert!(units.iter().any(|u| u.reason.as_deref().is_some_and(|r| r.starts_with("diffuse missing"))));
    }
}
