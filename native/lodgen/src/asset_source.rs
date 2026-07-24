use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum AssetLocation {
    Loose(PathBuf),
    #[cfg(feature = "archive-assets")]
    Archive {
        archive_path: PathBuf,
        member_path: String,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResolvedAsset {
    logical_path: String,
    location: AssetLocation,
}

impl ResolvedAsset {
    pub fn loose(logical_path: impl Into<String>, path: PathBuf) -> Self {
        Self {
            logical_path: normalize_relative(&logical_path.into()),
            location: AssetLocation::Loose(path),
        }
    }

    pub fn logical_path(&self) -> &str {
        &self.logical_path
    }

    pub fn loose_path(&self) -> Option<&Path> {
        match &self.location {
            AssetLocation::Loose(path) => Some(path),
            #[cfg(feature = "archive-assets")]
            AssetLocation::Archive { .. } => None,
        }
    }

    pub fn label(&self) -> String {
        match &self.location {
            AssetLocation::Loose(path) => path.display().to_string(),
            #[cfg(feature = "archive-assets")]
            AssetLocation::Archive {
                archive_path,
                member_path,
                ..
            } => format!("{}!{member_path}", archive_path.display()),
        }
    }
}

struct AssetResolver {
    loose_dirs: Vec<PathBuf>,
    #[cfg(feature = "archive-assets")]
    archive_paths: Vec<PathBuf>,
    #[cfg(feature = "archive-assets")]
    archive_members: HashMap<String, usize>,
}

impl AssetResolver {
    fn open(sources: &[PathBuf]) -> anyhow::Result<Self> {
        let loose_dirs = sources
            .iter()
            .filter(|path| path.is_dir())
            .cloned()
            .collect();

        #[cfg(feature = "archive-assets")]
        {
            let mut archive_paths = Vec::new();
            let mut archive_members = HashMap::new();
            for archive_path in sources.iter().filter(|path| is_ba2(path)) {
                let members =
                    bsarchive_native::list_archive_files(archive_path).map_err(|error| {
                        anyhow::anyhow!(
                            "index LOD asset archive {}: {error}",
                            archive_path.display()
                        )
                    })?;
                let archive_index = archive_paths.len();
                for member_path in members {
                    let key = normalize_relative(&member_path);
                    if is_lod_asset_member(&key) {
                        archive_members.insert(key, archive_index);
                    }
                }
                archive_paths.push(archive_path.clone());
            }
            return Ok(Self {
                loose_dirs,
                archive_paths,
                archive_members,
            });
        }

        #[cfg(not(feature = "archive-assets"))]
        {
            if let Some(path) = sources.iter().find(|path| is_ba2(path)) {
                anyhow::bail!(
                    "LOD BA2 asset source requires the archive-assets feature: {}",
                    path.display()
                );
            }
            Ok(Self { loose_dirs })
        }
    }

    fn resolve(&self, relative: &str) -> Option<ResolvedAsset> {
        let logical_path = normalize_relative(relative);
        if logical_path.is_empty() {
            return None;
        }
        for directory in &self.loose_dirs {
            let candidate = directory.join(logical_path.replace('/', "\\"));
            if candidate.is_file() {
                return Some(ResolvedAsset::loose(logical_path, candidate));
            }
            if let Some(path) = resolve_case_insensitive(directory, &logical_path) {
                return Some(ResolvedAsset::loose(logical_path, path));
            }
        }

        #[cfg(feature = "archive-assets")]
        if let Some(archive_index) = self.archive_members.get(&logical_path) {
            let archive_path = self.archive_paths.get(*archive_index)?;
            return Some(ResolvedAsset {
                logical_path: logical_path.clone(),
                location: AssetLocation::Archive {
                    archive_path: archive_path.clone(),
                    member_path: logical_path,
                },
            });
        }
        None
    }

    fn read(&self, asset: &ResolvedAsset) -> anyhow::Result<Vec<u8>> {
        match &asset.location {
            AssetLocation::Loose(path) => std::fs::read(path)
                .map_err(|error| anyhow::anyhow!("read {}: {error}", path.display())),
            #[cfg(feature = "archive-assets")]
            AssetLocation::Archive {
                archive_path,
                member_path,
            } => bsarchive_native::python::extract_one_impl(archive_path, member_path).map_err(
                |error| anyhow::anyhow!("read {}!{member_path}: {error}", archive_path.display()),
            ),
        }
    }
}

struct ResolverEntry {
    resolver: Arc<AssetResolver>,
    active_runs: usize,
}

static RESOLVERS: LazyLock<Mutex<HashMap<Vec<PathBuf>, ResolverEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub struct AssetRunGuard {
    key: Vec<PathBuf>,
}

impl Drop for AssetRunGuard {
    fn drop(&mut self) {
        let mut resolvers = RESOLVERS.lock().unwrap_or_else(|error| error.into_inner());
        let remove = if let Some(entry) = resolvers.get_mut(&self.key) {
            entry.active_runs = entry.active_runs.saturating_sub(1);
            entry.active_runs == 0
        } else {
            false
        };
        if remove {
            resolvers.remove(&self.key);
        }
    }
}

pub fn prepare(sources: &[PathBuf]) -> anyhow::Result<AssetRunGuard> {
    let key = sources.to_vec();
    let mut resolvers = RESOLVERS.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(entry) = resolvers.get_mut(&key) {
        entry.active_runs += 1;
        return Ok(AssetRunGuard { key });
    }
    let resolver = Arc::new(AssetResolver::open(sources)?);
    resolvers.insert(
        key.clone(),
        ResolverEntry {
            resolver,
            active_runs: 1,
        },
    );
    Ok(AssetRunGuard { key })
}

fn resolver(sources: &[PathBuf]) -> anyhow::Result<Arc<AssetResolver>> {
    let key = sources.to_vec();
    {
        let resolvers = RESOLVERS.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(entry) = resolvers.get(&key) {
            return Ok(Arc::clone(&entry.resolver));
        }
    }
    let opened = Arc::new(AssetResolver::open(sources)?);
    let mut resolvers = RESOLVERS.lock().unwrap_or_else(|error| error.into_inner());
    let entry = resolvers.entry(key).or_insert_with(|| ResolverEntry {
        resolver: Arc::clone(&opened),
        active_runs: 0,
    });
    Ok(Arc::clone(&entry.resolver))
}

pub fn resolve(sources: &[PathBuf], relative: &str) -> Option<ResolvedAsset> {
    resolver(sources).ok()?.resolve(relative)
}

pub fn read(sources: &[PathBuf], asset: &ResolvedAsset) -> anyhow::Result<Vec<u8>> {
    resolver(sources)?.read(asset)
}

fn normalize_relative(path: &str) -> String {
    let mut normalized = path
        .trim_end_matches('\0')
        .trim()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase();
    if let Some(rest) = normalized.strip_prefix("data/") {
        normalized = rest.to_string();
    }
    normalized
}

fn is_ba2(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ba2"))
}

fn is_lod_asset_member(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "nif" | "bgsm" | "bgem" | "dds" | "txt"
            )
        })
}

fn resolve_case_insensitive(base: &Path, relative: &str) -> Option<PathBuf> {
    let mut current = base.to_path_buf();
    for part in relative.split('/').filter(|part| !part.is_empty()) {
        current = std::fs::read_dir(&current)
            .ok()?
            .flatten()
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(part)
            })?
            .path();
    }
    current.is_file().then_some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loose_files_win_and_resolve_case_insensitively() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("Meshes").join("LOD").join("Tree.nif");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"nif").unwrap();
        let sources = vec![temp.path().to_path_buf()];
        let _guard = prepare(&sources).unwrap();

        let asset = resolve(&sources, r"meshes\lod\tree.nif").unwrap();

        assert!(asset.loose_path().is_some_and(Path::is_file));
        assert_eq!(read(&sources, &asset).unwrap(), b"nif");
    }

    #[test]
    fn archive_index_keeps_only_lod_asset_types() {
        assert!(is_lod_asset_member("textures/landscape/base.dds"));
        assert!(is_lod_asset_member("meshes/lod/tree.nif"));
        assert!(!is_lod_asset_member("sound/fx/ambient.xwm"));
        assert!(!is_lod_asset_member("scripts/source/example.psc"));
    }

    #[cfg(feature = "archive-assets")]
    #[test]
    fn reads_one_member_directly_from_ba2() {
        use bsarchive_native::fo4::{
            Archive, ArchiveKey, ArchiveOptions, File, FileReadOptions, Format,
        };
        use bsarchive_native::prelude::*;
        use bsarchive_native::{Borrowed, CompressionResult};

        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("Fallout4 - Test.ba2");
        let read_options = FileReadOptions::builder()
            .format(Format::GNRL)
            .compression_result(CompressionResult::Decompressed)
            .build();
        let file = File::read(Borrowed(b"archive nif"), &read_options).unwrap();
        let mut archive = Archive::new();
        archive.insert(ArchiveKey::from(r"meshes\lod\tree.nif".as_bytes()), file);
        let options = ArchiveOptions::builder()
            .format(Format::GNRL)
            .strings(true)
            .build();
        let mut output = std::fs::File::create(&archive_path).unwrap();
        archive.write(&mut output, &options).unwrap();
        drop(output);

        let sources = vec![archive_path];
        let _guard = prepare(&sources).unwrap();
        let asset = resolve(&sources, r"Meshes\LOD\Tree.nif").unwrap();

        assert!(asset.loose_path().is_none());
        assert_eq!(read(&sources, &asset).unwrap(), b"archive nif");
    }
}
