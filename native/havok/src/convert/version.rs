use crate::error::{HavokError, HavokResult};

/// All 62 Havok version strings in linear order; the index is the numeric version id.
///
/// Patch corpus coverage (see `corpus.rs`):
/// - ids 46/48/50/52/53/55/56 have populated patch tables (hk_2012.2 through hk_2015.1)
/// - ids 57–61 (hk_2016.1 through hk_2018.1) have **no patches registered** — they exist
///   only so `get_version` and `get_version_chain` return valid names.
///   Routes crossing these packages must fail loudly until those patch packages are ported.
const VERSION_NAMES: [&str; 62] = [
    "hk_3.0.0",
    "hk_3.1.0",
    "hk_3.2.0",
    "hk_3.3.0-a2",
    "hk_3.3.0-b1",
    "hk_3.3.0-b2",
    "hk_3.3.0-r1",
    "hk_4.0.0-b1",
    "hk_4.0.0-b2",
    "hk_4.0.0-r1",
    "hk_4.0.2-r1",
    "hk_4.0.3-r1",
    "hk_4.1.0-b1",
    "hk_4.1.0-r1",
    "hk_4.5.0-b1",
    "hk_4.5.0-r1",
    "hk_4.5.1-r1",
    "hk_4.5.2-r1",
    "hk_4.6.0-b1",
    "hk_4.6.0-b2",
    "hk_4.6.0-r1",
    "hk_4.6.1-r1",
    "hk_5.0.0-b1",
    "hk_5.0.0-r1",
    "hk_5.1.0-r1",
    "hk_5.5.0-b1",
    "hk_5.5.0-b2",
    "hk_5.5.0-r1",
    "hk_6.0.0-b1",
    "hk_6.0.0-b2",
    "hk_6.0.0-r1",
    "hk_6.1.0-r1",
    "hk_6.5.0-b1",
    "hk_6.5.0-r1",
    "hk_6.6.0-b1",
    "hk_6.6.0-r1",
    "hk_7.0.0-b1",
    "hk_7.0.0-r1",
    "hk_7.1.0-r1",
    "hk_2010.1.0-r1",
    "hk_2010.2.0-r1",
    "hk_2011.1.0-r1",
    "hk_2011.2.0-r1",
    "hk_2011.3.0-r1",
    "hk_2011.3.1-r1",
    "hk_2012.1.0-r1",
    "hk_2012.2.0-r1",
    "hk_2012.2.1-r1",
    "hk_2013.1.0-r1",
    "hk_2013.1.1-r1",
    "hk_2013.2.0-r1",
    "hk_2013.2.5-r1",
    "hk_2013.3.0-r1",
    "hk_2014.1.0-r1",
    "hk_2014.1.0-r2",
    "hk_2014.2.0-r1",
    "hk_2015.1.0-r1",
    "hk_2016.1.0-r1",
    "hk_2016.2.0-r1",
    "hk_2017.1.0-r1",
    "hk_2017.2.0-r1",
    "hk_2018.1.0-r1",
];

pub const UNIMPLEMENTED_PATCH_PACKAGE_IDS: &[u8] = &[57, 58, 59, 60, 61];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HavokVersion {
    pub id: u8,
    pub name: &'static str,
}

impl HavokVersion {
    pub const fn new(id: u8, name: &'static str) -> Self {
        Self { id, name }
    }
}

pub fn all_versions() -> impl Iterator<Item = HavokVersion> {
    VERSION_NAMES
        .iter()
        .enumerate()
        .map(|(id, name)| HavokVersion::new(id as u8, name))
}

pub fn get_version(version_id: u8) -> HavokResult<HavokVersion> {
    VERSION_NAMES
        .get(version_id as usize)
        .map(|name| HavokVersion::new(version_id, name))
        .ok_or_else(|| HavokError::UnknownVersion(version_id.to_string()))
}

pub fn get_version_by_name(name: &str) -> HavokResult<HavokVersion> {
    all_versions()
        .find(|version| version.name == name)
        .ok_or_else(|| HavokError::UnknownVersion(name.to_string()))
}

pub fn parse_target_version(target: &str) -> HavokResult<HavokVersion> {
    let trimmed = target.trim();
    if let Ok(id) = trimmed.parse::<u8>() {
        return get_version(id);
    }

    match trimmed.to_ascii_lowercase().as_str() {
        "skyrim" | "skyrimse" | "skyrim_se" => get_version(40),
        "fo4" | "fallout4" | "fallout_4" => get_version(53),
        "fo76" | "fallout76" | "fallout_76" => get_version(56),
        _ => get_version_by_name(trimmed),
    }
}

pub fn get_version_chain(source: u8, target: u8) -> HavokResult<Vec<HavokVersion>> {
    get_version(source)?;
    get_version(target)?;
    let ids: Box<dyn Iterator<Item = u8>> = if source <= target {
        Box::new(source..=target)
    } else {
        Box::new((target..=source).rev())
    };
    ids.map(get_version).collect()
}

pub fn detect_version_id(version_name: &str) -> HavokResult<u8> {
    Ok(get_version_by_name(version_name)?.id)
}
