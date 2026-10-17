//! FO4 worldspace LAND -> raw BTD4 sidecars for the staged terrain layer bank.
//!
//! One sidecar per worldspace, `Terrain/FO4CS/<WorldEditorID>.btd4` in the output
//! mod. FO4CS attaches its patches to the engine's own LAND quadrants (no dense
//! geometry), so the BTD4 HGTS channel only satisfies the file contract. Bank rows
//! sample at the engine's landscape UV (scale 1, no rotation); each layer's F0 is
//! its staged `_source_r.dds`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde::Serialize;
use terrain_native::btd4::{Btd4Header, Btd4Writer, CellChannels};
use terrain_native::fo4_raw::{self, Fo4Quadrant, QuadrantSelectors, QUAD_VERTEX_COUNT};
use terrain_native::raw_terrain::{self, MaterialSource};

use crate::classify::{ClassHints, MaterialClass, classify_material};
use crate::terrain::GlobalId;

pub const SIDECAR_DIR: &str = "terrain/fo4cs";
/// Large softness makes coverage follow the selector alone (FO4 has no height blend).
pub const FO4_BLEND_SOFTNESS: f32 = 256.0;
const GRID: usize = 33;
const HGTS_SIDE: usize = 129;

#[derive(Debug, Clone, PartialEq)]
pub struct LandLayer {
    pub ltex: GlobalId,
    pub layer: i16,
    pub opacity: Vec<f32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LandQuadrant {
    pub base: Option<GlobalId>,
    pub overlays: Vec<LandLayer>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LandCell {
    /// 33x33 world-unit heights, row 0 = south.
    pub heights: Option<Vec<f32>>,
    pub quadrants: [LandQuadrant; 4],
}

#[derive(Debug, Clone, Default)]
pub struct WorldLand {
    pub editor_id: String,
    pub cells: BTreeMap<(i32, i32), LandCell>,
}

/// One bank material: the staged `<prefix>_source_{d,n,l,r}.dds`.
#[derive(Debug, Clone, PartialEq)]
pub struct BankLayer {
    pub prefix: String,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct SidecarReport {
    pub worldspace: String,
    pub path: String,
    pub cells: usize,
    pub materials: usize,
    pub raw_quadrants: usize,
    /// Quadrants left to native shading: no base texture or a layer outside the bank.
    pub native_quadrants: usize,
    /// Distinct layers beyond the six a quadrant palette holds, folded into the others.
    pub dropped_layers: usize,
    pub unbanked_ltex: Vec<String>,
    /// Bank texture memory at the staged layer size, all mips (BC3 + BC5 + BC3).
    pub estimated_vram_mib: f32,
}

pub fn sidecar_path(output: &Path, editor_id: &str) -> PathBuf {
    output.join(SIDECAR_DIR).join(format!("{editor_id}.btd4"))
}

/// VHGT: f32 offset, 33x33 i8 deltas (column 0 accumulates down rows, the rest along
/// the row), all scaled by 8. Row 0 = south.
pub fn decode_vhgt(data: &[u8]) -> Option<Vec<f32>> {
    if data.len() < 4 + GRID * GRID {
        return None;
    }
    let offset = f32::from_le_bytes(data[..4].try_into().ok()?) * 8.0;
    let mut heights = vec![0.0f32; GRID * GRID];
    let mut row_start = offset;
    for row in 0..GRID {
        row_start += (data[4 + row * GRID] as i8) as f32 * 8.0;
        let mut height = row_start;
        heights[row * GRID] = height;
        for column in 1..GRID {
            height += (data[4 + row * GRID + column] as i8) as f32 * 8.0;
            heights[row * GRID + column] = height;
        }
    }
    heights.iter().all(|h| h.is_finite()).then_some(heights)
}

/// VTXT: (position u16 in the 17x17 quadrant, 2 bytes, opacity f32) entries.
pub fn decode_vtxt(data: &[u8]) -> Vec<f32> {
    let mut opacity = vec![0.0f32; QUAD_VERTEX_COUNT];
    for entry in data.chunks_exact(8) {
        let position = u16::from_le_bytes([entry[0], entry[1]]) as usize;
        let value = f32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
        if position < opacity.len() && value.is_finite() {
            opacity[position] = value.clamp(0.0, 1.0);
        }
    }
    opacity
}

/// BTXT/ATXT: LTEX form id, quadrant, unused byte, layer.
pub fn decode_layer_header(data: &[u8]) -> Option<(u32, u8, i16)> {
    (data.len() >= 8).then(|| {
        (u32::from_le_bytes([data[0], data[1], data[2], data[3]]), data[4], i16::from_le_bytes([data[6], data[7]]))
    })
}

/// A LAND record from its subrecords in file order; `resolve` maps a plugin-local
/// LTEX form id to its global id. Overlays are ordered by their ATXT layer.
pub fn land_cell<'a>(subrecords: impl IntoIterator<Item = (&'a str, &'a [u8])>, resolve: impl Fn(u32) -> GlobalId) -> LandCell {
    let mut cell = LandCell::default();
    let mut pending: Option<(usize, LandLayer)> = None;
    let flush = |cell: &mut LandCell, pending: &mut Option<(usize, LandLayer)>| {
        if let Some((quadrant, layer)) = pending.take() {
            cell.quadrants[quadrant].overlays.push(layer);
        }
    };
    for (signature, data) in subrecords {
        match signature {
            "VHGT" => cell.heights = decode_vhgt(data),
            "BTXT" => {
                flush(&mut cell, &mut pending);
                if let Some((form_id, quadrant, _)) = decode_layer_header(data).filter(|h| h.1 < 4 && h.0 != 0) {
                    cell.quadrants[quadrant as usize].base = Some(resolve(form_id));
                }
            }
            "ATXT" => {
                flush(&mut cell, &mut pending);
                if let Some((form_id, quadrant, layer)) = decode_layer_header(data).filter(|h| h.1 < 4 && h.0 != 0) {
                    pending = Some((quadrant as usize, LandLayer { ltex: resolve(form_id), layer, opacity: vec![0.0; QUAD_VERTEX_COUNT] }));
                }
            }
            "VTXT" => {
                if let Some((_, layer)) = pending.as_mut() {
                    layer.opacity = decode_vtxt(data);
                }
            }
            _ => {}
        }
    }
    flush(&mut cell, &mut pending);
    for quadrant in &mut cell.quadrants {
        quadrant.overlays.sort_by_key(|layer| layer.layer);
    }
    cell
}

/// Per-layer F0 for the staged `_r`: layers whose file name reads as metal get the
/// object upgrade's metal F0, everything else is a dielectric. Folders
/// (`landscape/ground/`) are ignored.
pub fn terrain_reflectance(diffuse: &str) -> f32 {
    let name = diffuse.rsplit(['/', '\\']).next().unwrap_or(diffuse);
    match classify_material(&ClassHints { diffuse: name, ..Default::default() }) {
        MaterialClass::Metal => crate::kernels::METAL_F0,
        _ => crate::kernels::DIELECTRIC_F0,
    }
}

fn hgts(heights: &[f32], height_min: f32, height_scale: f32) -> Vec<u16> {
    let mut out = vec![0u16; HGTS_SIDE * HGTS_SIDE];
    for y in 0..HGTS_SIDE {
        for x in 0..HGTS_SIDE {
            let (fx, fy) = (x as f32 / 4.0, y as f32 / 4.0);
            let (x0, y0) = ((fx as usize).min(GRID - 1), (fy as usize).min(GRID - 1));
            let (x1, y1) = ((x0 + 1).min(GRID - 1), (y0 + 1).min(GRID - 1));
            let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
            let at = |cx: usize, cy: usize| heights[cy * GRID + cx];
            let h = (at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty;
            out[y * HGTS_SIDE + x] = ((h - height_min) / height_scale).round().clamp(0.0, u16::MAX as f32) as u16;
        }
    }
    out
}

fn ltex_name(id: &GlobalId) -> String {
    format!("{}:{:06X}", id.0, id.1)
}

/// Writes the worldspace sidecar. `layer` resolves an LTEX to its staged bank layer;
/// `layer_size` is the staged texture edge, used for the memory estimate only.
pub fn write_sidecar(
    world: &WorldLand,
    plugin_names: &[String],
    layer: &dyn Fn(&GlobalId) -> Option<BankLayer>,
    layer_size: u32,
    path: &Path,
) -> Result<SidecarReport, String> {
    let cells: Vec<(&(i32, i32), &LandCell, &Vec<f32>)> =
        world.cells.iter().filter_map(|(key, cell)| cell.heights.as_ref().map(|h| (key, cell, h))).collect();
    if cells.is_empty() {
        return Err(format!("worldspace {} has no LAND heights", world.editor_id));
    }
    // Bank: one material per staged prefix, in prefix order.
    let mut prefixes: BTreeSet<String> = BTreeSet::new();
    let mut ltex_prefix: HashMap<&GlobalId, Option<String>> = HashMap::new();
    let mut unbanked = Vec::new();
    for (_, cell, _) in &cells {
        for quadrant in &cell.quadrants {
            for id in quadrant.base.iter().chain(quadrant.overlays.iter().map(|l| &l.ltex)) {
                ltex_prefix.entry(id).or_insert_with(|| match layer(id) {
                    Some(bank) => {
                        prefixes.insert(bank.prefix.clone());
                        Some(bank.prefix)
                    }
                    None => {
                        unbanked.push(ltex_name(id));
                        None
                    }
                });
            }
        }
    }
    if prefixes.is_empty() {
        return Err(format!("worldspace {}: none of its landscape layers are staged", world.editor_id));
    }
    if prefixes.len() > 255 {
        return Err(format!("worldspace {}: {} layers exceed the 255-material bank", world.editor_id, prefixes.len()));
    }
    let bank_index: HashMap<&str, u8> = prefixes.iter().enumerate().map(|(i, p)| (p.as_str(), (i + 1) as u8)).collect();
    let materials: Vec<MaterialSource> = prefixes
        .iter()
        .enumerate()
        .map(|(i, prefix)| MaterialSource {
            source_ltex_form_id: (i + 1) as u32,
            rows: [[FO4_BLEND_SOFTNESS, 0.0, 1.0, 1.0], [0.0; 4], [0.0; 4]],
            textures: ["d", "n", "l", "r"].map(|m| format!("{prefix}_{m}.dds")),
        })
        .collect();
    let bank = raw_terrain::encode_bank(&materials)?;

    let mut report = SidecarReport {
        worldspace: world.editor_id.clone(),
        path: path.display().to_string(),
        cells: cells.len(),
        materials: materials.len(),
        unbanked_ltex: unbanked,
        estimated_vram_mib: materials.len() as f32 * (layer_size as f32).powi(2) * 3.0 * 4.0 / 3.0 / (1024.0 * 1024.0),
        ..Default::default()
    };
    let mut selectors: HashMap<(i32, i32), QuadrantSelectors> = HashMap::new();
    for ((x, y), cell, _) in &cells {
        for (q, quadrant) in cell.quadrants.iter().enumerate() {
            let id = |ltex: &GlobalId| ltex_prefix.get(ltex).cloned().flatten().map(|p| bank_index[p.as_str()]);
            let base = quadrant.base.as_ref().map(id);
            let overlays: Option<Vec<(u8, Vec<f32>)>> =
                quadrant.overlays.iter().map(|l| id(&l.ltex).map(|i| (i, l.opacity.clone()))).collect();
            let encoded = match (base, overlays) {
                (Some(Some(base)), Some(overlays)) => fo4_raw::encode_quadrant(&Fo4Quadrant { base: Some(base), overlays }),
                _ => None,
            };
            match encoded {
                Some((encoded, stats)) => {
                    report.raw_quadrants += 1;
                    report.dropped_layers += stats.dropped_materials;
                    selectors.insert((2 * x + (q as i32 & 1), 2 * y + (q as i32 >> 1)), encoded);
                }
                None => report.native_quadrants += 1,
            }
        }
    }

    let (mut height_min, mut height_max) = (f32::MAX, f32::MIN);
    for (_, _, heights) in &cells {
        for h in heights.iter() {
            height_min = height_min.min(*h);
            height_max = height_max.max(*h);
        }
    }
    let height_scale = if height_max > height_min { (height_max - height_min) / u16::MAX as f32 } else { 1.0 };
    let header = Btd4Header {
        density: 128,
        height_min,
        height_scale,
        worldspace_editor_id: world.editor_id.clone(),
        plugin_names: plugin_names.to_vec(),
        cell_min_x: cells.iter().map(|c| c.0.0).min().unwrap(),
        cell_min_y: cells.iter().map(|c| c.0.1).min().unwrap(),
        cell_max_x: cells.iter().map(|c| c.0.0).max().unwrap(),
        cell_max_y: cells.iter().map(|c| c.0.1).max().unwrap(),
    };
    let mut writer = Btd4Writer::with_spill_file(header, bank, &terrain_native::btd4::spill_path_for(path))?;
    for ((x, y), _, heights) in &cells {
        let mut raw = Vec::with_capacity(raw_terrain::CELL_BYTES);
        for q in 0..4 {
            let (gx, gy) = (2 * x + (q & 1), 2 * y + (q >> 1));
            let patch = if selectors.contains_key(&(gx, gy)) {
                fo4_raw::assemble_patch(gx, gy, |qx, qy| selectors.get(&(qx, qy)))
            } else {
                fo4_raw::assemble_patch(gx, gy, |_, _| None)
            };
            raw.extend(raw_terrain::encode_patch(&patch)?);
        }
        writer.add_cell(*x, *y, CellChannels {
            raw_terrain: Some(raw),
            heights: Some(hgts(heights, height_min, height_scale)),
            alphas: None,
            layers: None,
            gcvr: None,
            colors: None,
            grass_points: None,
        })?;
    }
    writer.finish(path)?;
    report.unbanked_ltex.sort();
    Ok(report)
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SidecarRun {
    pub written: Vec<SidecarReport>,
    pub errors: Vec<(String, String)>,
}

/// Every requested worldspace whose layers were staged by this plan. Bank textures
/// must exist in the output mod (written by this run or an earlier one).
pub fn write_sidecars(plan: &crate::plan::Plan, opts: &crate::options::UpgradeOptions) -> SidecarRun {
    use crate::plan::UnitKind;
    let mut run = SidecarRun::default();
    let Some(records) = plan.terrain.as_deref() else { return run };
    let staged: HashMap<&str, &str> = plan
        .units
        .iter()
        .filter(|u| u.is_upgrade() && u.kind == UnitKind::TerrainLayer)
        .map(|u| (u.diffuse.as_str(), u.output.as_str()))
        .collect();
    let on_disk = |prefix: &str| ["d", "n", "l", "r"].iter().all(|m| opts.output.join(format!("{prefix}_{m}.dds")).is_file());
    let layer = |ltex: &GlobalId| {
        let diffuse = records.ltex_diffuse(ltex)?;
        let prefix = *staged.get(diffuse)?;
        on_disk(prefix).then(|| BankLayer { prefix: prefix.to_string() })
    };
    let plugin_names: Vec<String> = opts
        .plugins
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    let wanted = |editor_id: &str| {
        opts.terrain_worldspaces.is_empty() || opts.terrain_worldspaces.iter().any(|w| w.eq_ignore_ascii_case(editor_id))
    };
    for world in records.worldspaces.values() {
        if world.editor_id.is_empty() || world.cells.is_empty() || !wanted(&world.editor_id) {
            continue;
        }
        let path = sidecar_path(&opts.output, &world.editor_id);
        match write_sidecar(world, &plugin_names, &layer, opts.terrain_layer_size, &path) {
            Ok(report) => run.written.push(report),
            Err(err) => run.errors.push((world.editor_id.clone(), err)),
        }
    }
    for requested in &opts.terrain_worldspaces {
        if !records.worldspaces.values().any(|w| w.editor_id.eq_ignore_ascii_case(requested)) {
            run.errors.push((requested.clone(), "no worldspace with this editor id has LAND in --plugin".into()));
        }
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrain_native::btd4::Btd4Reader;

    fn id(object: u32) -> GlobalId {
        ("fallout4.esm".into(), object)
    }

    fn vhgt(offset: f32, delta: i8) -> Vec<u8> {
        let mut data = offset.to_le_bytes().to_vec();
        data.extend(std::iter::repeat_n(delta as u8, GRID * GRID));
        data.extend([0, 0, 0]);
        data
    }

    fn header(form: u32, quadrant: u8, layer: i16) -> Vec<u8> {
        let mut data = form.to_le_bytes().to_vec();
        data.extend([quadrant, 0]);
        data.extend(layer.to_le_bytes());
        data
    }

    #[test]
    fn vhgt_accumulates_column_zero_down_rows_then_along_each_row() {
        let heights = decode_vhgt(&vhgt(2.0, 1)).unwrap();
        assert_eq!(heights[0], 16.0 + 8.0);
        assert_eq!(heights[1], 16.0 + 16.0);
        assert_eq!(heights[GRID], 16.0 + 16.0, "row 1 column 0 adds its own column-0 delta");
        assert_eq!(heights[GRID * GRID - 1], 16.0 + 33.0 * 8.0 + 32.0 * 8.0);
        assert!(decode_vhgt(&[0; 10]).is_none());
    }

    #[test]
    fn land_layers_pair_atxt_with_the_following_vtxt_and_sort_by_layer() {
        let mut vtxt = Vec::new();
        vtxt.extend(5u16.to_le_bytes());
        vtxt.extend([0, 0]);
        vtxt.extend(0.75f32.to_le_bytes());
        let subs: Vec<(&str, Vec<u8>)> = vec![
            ("VHGT", vhgt(0.0, 0)),
            ("BTXT", header(0x100, 2, 0)),
            ("ATXT", header(0x200, 2, 1)),
            ("VTXT", vtxt),
            ("ATXT", header(0x300, 2, 0)),
        ];
        let cell = land_cell(subs.iter().map(|(s, d)| (*s, d.as_slice())), id);
        let q = &cell.quadrants[2];
        assert_eq!(q.base, Some(id(0x100)));
        assert_eq!(q.overlays.iter().map(|l| l.ltex.1).collect::<Vec<_>>(), [0x300, 0x200]);
        assert_eq!(q.overlays[1].opacity[5], 0.75);
        assert!(q.overlays[0].opacity.iter().all(|o| *o == 0.0), "an ATXT without VTXT is transparent");
        assert!(cell.quadrants[0].base.is_none());
    }

    #[test]
    fn metal_layers_get_a_metallic_surface_row() {
        assert_eq!(terrain_reflectance("textures/landscape/ground/metalgrate01_d.dds"), crate::kernels::METAL_F0);
        assert_eq!(terrain_reflectance("textures/landscape/dirt01_d.dds"), crate::kernels::DIELECTRIC_F0);
    }

    fn world() -> WorldLand {
        let mut cell = LandCell { heights: Some(vec![100.0; GRID * GRID]), ..Default::default() };
        for q in 0..4 {
            cell.quadrants[q].base = Some(id(0x10));
        }
        cell.quadrants[1].overlays.push(LandLayer { ltex: id(0x20), layer: 0, opacity: vec![0.5; QUAD_VERTEX_COUNT] });
        cell.quadrants[3].overlays.push(LandLayer { ltex: id(0x99), layer: 0, opacity: vec![0.5; QUAD_VERTEX_COUNT] });
        let mut cells = BTreeMap::new();
        cells.insert((-2, 7), cell.clone());
        cells.insert((-1, 7), LandCell { heights: Some(vec![300.0; GRID * GRID]), ..cell });
        cells.insert((0, 0), LandCell::default());
        WorldLand { editor_id: "Commonwealth".into(), cells }
    }

    fn bank_layer(ltex: &GlobalId) -> Option<BankLayer> {
        match ltex.1 {
            0x10 => Some(BankLayer { prefix: "textures/landscape/dirt01_source".into() }),
            0x20 => Some(BankLayer { prefix: "textures/landscape/metalplate01_source".into() }),
            _ => None,
        }
    }

    #[test]
    fn sidecar_round_trips_through_the_btd4_reader() {
        let tmp = tempfile::tempdir().unwrap();
        let path = sidecar_path(tmp.path(), "Commonwealth");
        let report = write_sidecar(&world(), &["Fallout4.esm".into()], &bank_layer, 1024, &path).unwrap();
        assert_eq!(report.cells, 2, "cells without heights are skipped");
        assert_eq!(report.materials, 2);
        assert_eq!(report.raw_quadrants, 6);
        assert_eq!(report.native_quadrants, 2, "quadrant 3 references an unstaged LTEX");
        assert_eq!(report.unbanked_ltex, ["fallout4.esm:000099"]);
        assert!(path.ends_with("terrain/fo4cs/Commonwealth.btd4"));

        let reader = Btd4Reader::open(&path).unwrap();
        assert_eq!(reader.header().worldspace_editor_id, "Commonwealth");
        assert_eq!(raw_terrain::validate_bank(reader.raw_material_bank()).unwrap(), 2);
        let cell = reader.cell(-2, 7).unwrap();
        let heights = cell.heights.unwrap();
        assert_eq!(heights[0], 0, "the lower cell sits at height_min");
        let top = reader.cell(-1, 7).unwrap().heights.unwrap();
        assert_eq!(top[0], u16::MAX, "the higher cell sits at height_max");
        assert!((reader.header().height_scale - 200.0 / u16::MAX as f32).abs() < 1e-6);
        let raw = cell.raw_terrain.unwrap();
        let palette_at = |quadrant: usize| {
            let start = quadrant * raw_terrain::PATCH_BYTES + 16 + 67 * 67 * 2 + 4 * 8;
            raw[start..start + 8].to_vec()
        };
        // Bank ids follow prefix order: dirt01 = 1, metalplate01 = 2.
        assert_eq!(palette_at(0), [1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(palette_at(1), [2, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(palette_at(3), [0; 8], "native quadrant has an empty queue");
    }

    #[test]
    fn sidecars_use_staged_units_whose_bank_textures_exist() {
        use crate::plan::{Plan, Unit, UnitKind};
        use crate::terrain::TerrainRecords;
        let tmp = tempfile::tempdir().unwrap();
        let mut records = TerrainRecords::default();
        records.ltex.insert(id(0x10), id(0x1010));
        records.ltex.insert(id(0x20), id(0x1020));
        records.txst.insert(id(0x1010), ("textures/landscape/dirt01_d.dds".into(), String::new(), String::new()));
        records.txst.insert(id(0x1020), ("textures/landscape/metalplate01_d.dds".into(), String::new(), String::new()));
        records.worldspaces.insert(id(0x3C), world());
        let mut units = Vec::new();
        for (key, diffuse) in [("ltex:a", "textures/landscape/dirt01_d.dds"), ("ltex:b", "textures/landscape/metalplate01_d.dds")] {
            let mut unit = Unit::for_terrain(key);
            assert_eq!(unit.kind, UnitKind::TerrainLayer);
            unit.diffuse = diffuse.into();
            unit.output = diffuse.replace("_d.dds", "_source");
            units.push(unit);
        }
        // Only dirt01's bank files exist; metalplate01 failed to write.
        for m in ["d", "n", "l", "r"] {
            let path = tmp.path().join(format!("textures/landscape/dirt01_source_{m}.dds"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"dds").unwrap();
        }
        let plan = Plan { units, textures: Vec::new(), terrain: Some(std::sync::Arc::new(records)) };
        let opts = crate::options::UpgradeOptions {
            output: tmp.path().into(),
            plugins: vec!["C:/Games/Fallout 4/Data/Fallout4.esm".into()],
            terrain_worldspaces: vec!["commonwealth".into(), "Nowhere".into()],
            ..Default::default()
        };
        let run = write_sidecars(&plan, &opts);
        assert_eq!(run.written.len(), 1);
        assert_eq!(run.written[0].materials, 1);
        assert_eq!(run.written[0].unbanked_ltex.len(), 2, "metal plate (no files) and 0x99 (no unit)");
        assert_eq!(run.errors, [("Nowhere".to_string(), "no worldspace with this editor id has LAND in --plugin".to_string())]);
        let reader = Btd4Reader::open(&sidecar_path(tmp.path(), "Commonwealth")).unwrap();
        assert_eq!(reader.header().plugin_names, ["Fallout4.esm"]);
    }

    #[test]
    fn a_worldspace_without_staged_layers_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let err = write_sidecar(&world(), &["Fallout4.esm".into()], &|_| None, 1024, &tmp.path().join("x.btd4")).unwrap_err();
        assert!(err.contains("none of its landscape layers"));
    }
}
