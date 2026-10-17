//! Data-relative asset lookup over loose folders and BA2 archives.
//!
//! Sources are applied in order; a later source overrides an earlier one. Inside
//! one folder, its `*.ba2` archives load first (sorted by name) and loose files
//! override them, matching the game's own loose-over-archive rule.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(feature = "game-data")]
use bsarchive_native::python::ArchiveReader;

/// Stand-in so unit tests can link without the second DirectXTex FFI copy `bsarchive_native` brings.
#[cfg(not(feature = "game-data"))]
struct ArchiveReader;

#[cfg(not(feature = "game-data"))]
impl ArchiveReader {
    fn open(_path: &Path) -> Result<Self, String> {
        Err("built without BA2 support (feature game-data)".into())
    }
    fn list_files(&self) -> Vec<String> {
        Vec::new()
    }
    fn read_file(&self, _member: &str) -> Result<Vec<u8>, String> {
        Err("built without BA2 support (feature game-data)".into())
    }
}

const ASSET_ROOTS: [&str; 3] = ["materials", "textures", "meshes"];

pub fn normalize(path: &str) -> String {
    let mut p = path.trim_end_matches('\0').trim().replace('\\', "/").to_ascii_lowercase();
    while p.starts_with('/') {
        p.remove(0);
    }
    if let Some(rest) = p.strip_prefix("data/") {
        p = rest.to_string();
    }
    p
}

/// `Foo/Bar_d.dds` (as stored in a BGSM / texture set) -> `textures/foo/bar_d.dds`.
pub fn texture_key(path: &str) -> String {
    let p = normalize(path);
    if p.is_empty() || p.starts_with("textures/") {
        p
    } else {
        format!("textures/{p}")
    }
}

/// `textures/foo/bar_d.dds` -> `Foo/Bar_d.dds` style path stored in materials.
pub fn material_texture_ref(key: &str) -> String {
    key.strip_prefix("textures/").unwrap_or(key).to_string()
}

enum Location {
    Loose(PathBuf),
    Archive(usize, String),
}

pub struct AssetSources {
    archives: Vec<(PathBuf, Mutex<ArchiveReader>)>,
    index: HashMap<String, Location>,
    /// Position in `open`'s list of the source whose copy of each key wins.
    winning_source: HashMap<String, usize>,
}

fn is_ba2(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("ba2"))
}

impl AssetSources {
    pub fn open(sources: &[PathBuf]) -> Result<Self, String> {
        let mut this = Self { archives: Vec::new(), index: HashMap::new(), winning_source: HashMap::new() };
        for (position, source) in sources.iter().enumerate() {
            if source.is_file() && is_ba2(source) {
                this.add_archive(source, position)?;
            } else if source.is_dir() {
                let mut archives: Vec<PathBuf> = std::fs::read_dir(source)
                    .map_err(|err| format!("read {}: {err}", source.display()))?
                    .filter_map(|entry| entry.ok().map(|e| e.path()))
                    .filter(|path| path.is_file() && is_ba2(path))
                    .collect();
                archives.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
                for archive in archives {
                    this.add_archive(&archive, position)?;
                }
                this.add_loose(source, position);
            } else {
                return Err(format!("source not found: {}", source.display()));
            }
        }
        Ok(this)
    }

    fn add_archive(&mut self, path: &Path, position: usize) -> Result<(), String> {
        let reader = ArchiveReader::open(path).map_err(|err| format!("open {}: {err}", path.display()))?;
        let id = self.archives.len();
        for member in reader.list_files() {
            let key = normalize(&member);
            if ASSET_ROOTS.iter().any(|root| key.starts_with(root)) {
                self.winning_source.insert(key.clone(), position);
                self.index.insert(key, Location::Archive(id, member.clone()));
            }
        }
        self.archives.push((path.to_path_buf(), Mutex::new(reader)));
        Ok(())
    }

    fn add_loose(&mut self, root: &Path, position: usize) {
        for sub in ASSET_ROOTS {
            let dir = root.join(sub);
            if !dir.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(&dir).into_iter().filter_map(Result::ok) {
                if !entry.file_type().is_file() {
                    continue;
                }
                if let Ok(rel) = entry.path().strip_prefix(root) {
                    let key = normalize(&rel.to_string_lossy());
                    self.winning_source.insert(key.clone(), position);
                    self.index.insert(key, Location::Loose(entry.path().to_path_buf()));
                }
            }
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.index.contains_key(&normalize(key))
    }

    pub fn keys_with(&self, prefix: &str, extension: &str) -> Vec<String> {
        let mut keys: Vec<String> = self
            .index
            .keys()
            .filter(|k| k.starts_with(prefix) && k.ends_with(extension))
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    /// Position in `open`'s list of the source that supplies `key`.
    pub fn winning_source(&self, key: &str) -> Option<usize> {
        self.winning_source.get(&normalize(key)).copied()
    }

    pub fn origin(&self, key: &str) -> Option<String> {
        match self.index.get(&normalize(key))? {
            Location::Loose(path) => Some(path.display().to_string()),
            Location::Archive(id, member) => Some(format!("{}!{member}", self.archives[*id].0.display())),
        }
    }

    pub fn read(&self, key: &str) -> Result<Vec<u8>, String> {
        let key = normalize(key);
        match self.index.get(&key) {
            Some(Location::Loose(path)) => std::fs::read(path).map_err(|err| format!("read {}: {err}", path.display())),
            Some(Location::Archive(id, member)) => {
                let (path, reader) = &self.archives[*id];
                let reader = reader.lock().map_err(|_| "archive lock poisoned".to_string())?;
                reader.read_file(member).map_err(|err| format!("{}!{key}: {err}", path.display()))
            }
            None => Err(format!("asset not found: {key}")),
        }
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_paths_and_texture_keys() {
        assert_eq!(normalize("Data\\Textures\\Foo\\Bar_d.DDS\0"), "textures/foo/bar_d.dds");
        assert_eq!(texture_key("Foo\\Bar_d.dds"), "textures/foo/bar_d.dds");
        assert_eq!(texture_key("textures/foo/bar_d.dds"), "textures/foo/bar_d.dds");
        assert_eq!(texture_key(""), "");
        assert_eq!(material_texture_ref("textures/foo/bar_l.dds"), "foo/bar_l.dds");
    }

    #[test]
    fn later_sources_and_loose_files_win() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        for (root, body) in [(&a, b"first".as_slice()), (&b, b"second".as_slice())] {
            std::fs::create_dir_all(root.join("Materials/Foo")).unwrap();
            std::fs::write(root.join("Materials/Foo/Bar.BGSM"), body).unwrap();
        }
        std::fs::write(a.join("Materials/Foo/OnlyA.bgsm"), b"a").unwrap();
        let sources = AssetSources::open(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(sources.read("materials/foo/bar.bgsm").unwrap(), b"second");
        assert_eq!(sources.read("Materials\\Foo\\OnlyA.bgsm").unwrap(), b"a");
        assert_eq!(sources.keys_with("materials/", ".bgsm").len(), 2);
        assert!(sources.read("materials/missing.bgsm").is_err());
        assert!(AssetSources::open(&[tmp.path().join("nope")]).is_err());
    }
}
