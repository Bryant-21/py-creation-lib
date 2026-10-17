//! Which data source provides each precombined mesh and previs tome, in
//! load-order priority, for deciding who owns a CELL's previsbines.

use std::path::{Path, PathBuf};

use bsarchive_native::python::ArchiveReader;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde::Deserialize;

use crate::assets::normalize;
use crate::error::{PrevisError, Result};

const PRECOMBINED_DIR: &str = "meshes\\precombined\\";
const VIS_DIR: &str = "vis\\";

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataSource {
    /// A loose `Data`-like folder (game Data, an MO2 mod folder, overwrite).
    Loose { path: PathBuf },
    Archive { path: PathBuf },
}

impl DataSource {
    pub fn path(&self) -> &Path {
        match self {
            Self::Loose { path } | Self::Archive { path } => path,
        }
    }

    pub fn is_loose(&self) -> bool {
        matches!(self, Self::Loose { .. })
    }

    pub fn label(&self) -> String {
        let name = self.path().file_name().map_or_else(|| self.path().display().to_string(), |n| n.to_string_lossy().into_owned());
        if self.is_loose() { format!("{name} (loose)") } else { name }
    }
}

pub struct AssetIndex {
    pub sources: Vec<DataSource>,
    /// Plugin (load-order index) whose `<stem> - *.ba2` name an archive carries.
    pub source_owner: Vec<Option<usize>>,
    /// Normalized path to providing sources, lowest priority first.
    paths: FxHashMap<String, Vec<usize>>,
}

impl AssetIndex {
    /// `sources` are lowest priority first; `plugin_names` in load order.
    pub fn build(sources: Vec<DataSource>, plugin_names: &[String]) -> Result<Self> {
        let listed: Vec<Vec<String>> = sources.par_iter().map(previsbine_files).collect::<Result<_>>()?;
        let stems: FxHashMap<String, usize> = plugin_names
            .iter()
            .enumerate()
            .map(|(i, name)| (plugin_stem(name).to_ascii_lowercase(), i))
            .collect();
        let source_owner = sources.iter().map(|source| archive_owner(source, &stems)).collect();
        let mut paths: FxHashMap<String, Vec<usize>> = FxHashMap::default();
        for (source, files) in listed.into_iter().enumerate() {
            for path in files {
                paths.entry(path).or_default().push(source);
            }
        }
        Ok(Self { sources, source_owner, paths })
    }

    /// Sources providing `relative`, lowest priority first.
    pub fn providers(&self, relative: &str) -> &[usize] {
        self.paths.get(&normalize(relative)).map_or(&[], Vec::as_slice)
    }

    /// The path the game loads a precombined mesh from and its winning
    /// source: flat under `meshes\precombined\` for a CELL Fallout4.esm
    /// defines, else in the folder named after the defining file (CK 1.11.240
    /// `DoRequestHandles`; retail DLC archives match). Copies elsewhere are
    /// never loaded for this CELL.
    pub fn precombined_mesh(&self, cell_owner: &str, cell_object_id: u32, mesh_id: u32) -> (String, Option<usize>) {
        let folder = if cell_owner.eq_ignore_ascii_case("Fallout4.esm") { String::new() } else { format!("{}\\", cell_owner.to_ascii_lowercase()) };
        let path = format!("{PRECOMBINED_DIR}{folder}{:08x}_{mesh_id:08x}_oc.nif", cell_object_id & 0x00FF_FFFF);
        let source = self.paths.get(&path).and_then(|p| p.last()).copied();
        (path, source)
    }

    pub fn uvd_path(master_name: &str, cluster_object_id: u32) -> String {
        format!("{VIS_DIR}{}\\{:08x}.uvd", master_name.to_ascii_lowercase(), cluster_object_id & 0x00FF_FFFF)
    }
}

pub fn plugin_stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

fn archive_owner(source: &DataSource, stems: &FxHashMap<String, usize>) -> Option<usize> {
    let DataSource::Archive { path } = source else { return None };
    let file = path.file_stem()?.to_string_lossy().to_ascii_lowercase();
    let (stem, _) = file.rsplit_once(" - ")?;
    stems.get(stem).copied()
}

fn previsbine_files(source: &DataSource) -> Result<Vec<String>> {
    match source {
        DataSource::Archive { path } => {
            let reader = ArchiveReader::open(path).map_err(|e| PrevisError::invalid(format!("{}: {e}", path.display())))?;
            Ok(reader
                .list_files()
                .into_iter()
                .map(|name| normalize(&name))
                .filter(|name| name.starts_with(PRECOMBINED_DIR) || name.starts_with(VIS_DIR))
                .collect())
        }
        DataSource::Loose { path } => {
            let mut out = Vec::new();
            for (dir, prefix) in [(path.join("meshes").join("precombined"), PRECOMBINED_DIR), (path.join("vis"), VIS_DIR)] {
                collect_loose(&dir, prefix, &mut out)?;
            }
            Ok(out)
        }
    }
}

fn collect_loose(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(PrevisError::io(dir.display().to_string(), error)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| PrevisError::io(dir.display().to_string(), e))?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let relative = format!("{prefix}{name}");
        if entry.file_type().map_err(|e| PrevisError::io(dir.display().to_string(), e))?.is_dir() {
            collect_loose(&entry.path(), &format!("{relative}\\"), out)?;
        } else {
            out.push(relative);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meshes_are_found_where_the_game_loads_them_and_later_sources_win() {
        let dir = tempfile::tempdir().unwrap();
        let low = dir.path().join("low");
        let high = dir.path().join("high");
        for root in [&low, &high] {
            let meshes = root.join("Meshes").join("PreCombined");
            std::fs::create_dir_all(meshes.join("DLCCoast.esm")).unwrap();
            std::fs::write(meshes.join("0001F2A3_0BADF00D_OC.NIF"), []).unwrap();
            std::fs::write(meshes.join("DLCCoast.esm").join("0001F2A3_0000BEEF_OC.NIF"), []).unwrap();
        }
        let flat = high.join("Meshes").join("PreCombined");
        std::fs::create_dir_all(flat.join("Mod.esp")).unwrap();
        std::fs::write(flat.join("Mod.esp").join("0001F2A3_0000CAFE_OC.NIF"), []).unwrap();
        std::fs::create_dir_all(high.join("Vis").join("Fallout4.esm")).unwrap();
        std::fs::write(high.join("Vis").join("Fallout4.esm").join("0001F2A3.uvd"), []).unwrap();

        let sources = vec![DataSource::Loose { path: low }, DataSource::Loose { path: high }];
        let index = AssetIndex::build(sources, &["Fallout4.esm".to_string()]).unwrap();
        assert_eq!(index.precombined_mesh("Fallout4.esm", 0x0001_F2A3, 0x0BAD_F00D), ("meshes\\precombined\\0001f2a3_0badf00d_oc.nif".to_string(), Some(1)));
        assert_eq!(index.precombined_mesh("DLCCoast.esm", 0x0001_F2A3, 0xBEEF).1, Some(1));
        assert_eq!(index.precombined_mesh("Fallout4.esm", 0x0001_F2A3, 0xCAFE).1, None, "another plugin's folder is not loaded for this CELL");
        assert_eq!(index.providers(&AssetIndex::uvd_path("Fallout4.esm", 0x0001_F2A3)), &[1]);
        assert_eq!(index.source_owner, vec![None, None]);
    }

    #[test]
    fn archives_are_owned_by_the_plugin_named_before_the_suffix() {
        let stems: FxHashMap<String, usize> = [("dlccoast".to_string(), 3), ("ppf".to_string(), 7)].into_iter().collect();
        let archive = |name: &str| DataSource::Archive { path: PathBuf::from(format!("C:\\Data\\{name}")) };
        assert_eq!(archive_owner(&archive("DLCCoast - Main.ba2"), &stems), Some(3));
        assert_eq!(archive_owner(&archive("PPF - Main.ba2"), &stems), Some(7));
        assert_eq!(archive_owner(&archive("Unrelated - Main.ba2"), &stems), None);
    }
}
