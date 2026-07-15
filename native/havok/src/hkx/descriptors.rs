use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::hkx::types::HkxType;

/// Capability tag distinguishing authoring-side from runtime classes.
///
/// `Setup` classes are used during the cloth/animation authoring pipeline and
/// produce `Runtime` output; they are not present at game runtime. `Internal`
/// covers helper structs (math, utility) that appear only as inline members.
/// `Runtime` is the default for all classes not otherwise categorized.
///
/// The heuristic: classes whose names contain "Setup" are tagged `Setup`;
/// all others default to `Runtime`. Override via `DescriptorRegistry::insert`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClassKind {
    #[default]
    Runtime,
    Setup,
    Internal,
}

/// First-class enum/flags descriptor: ordered named values with i64 storage
/// so bitmask flags (> 2^31) don't truncate.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct EnumDef {
    pub values: Vec<(String, i64)>,
}

impl EnumDef {
    pub fn value_to_name(&self, int_value: i64) -> Option<&str> {
        self.values
            .iter()
            .find_map(|(name, v)| (*v == int_value).then(|| name.as_str()))
    }

    pub fn name_to_value(&self, name: &str) -> Option<i64> {
        self.values
            .iter()
            .find_map(|(n, v)| (n == name).then_some(*v))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DescriptorError {
    #[error("classxml directory does not exist: {0}")]
    MissingClassxmlDir(PathBuf),

    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("invalid _index.json in {path}: {source}")]
    InvalidIndex {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("unsafe classxml index path for {class_name}: {filename}")]
    UnsafeIndexPath {
        class_name: String,
        filename: String,
    },

    #[error("invalid classxml XML in {path}: {source}")]
    Xml {
        path: PathBuf,
        source: roxmltree::Error,
    },

    #[error("malformed classxml in {path}: {message}")]
    Malformed { path: PathBuf, message: String },

    #[error("inheritance cycle while resolving {class_name}")]
    InheritanceCycle { class_name: String },

    #[error("unknown contents version prefix — no classxml mapping for {0}")]
    UnknownVersion(String),
}

pub type DescriptorResult<T> = Result<T, DescriptorError>;

#[derive(Debug, Clone, PartialEq)]
pub struct MemberTemplate {
    pub name: String,
    pub offset: usize,
    pub vtype: HkxType,
    pub vsubtype: HkxType,
    pub ctype: String,
    pub arrsize: usize,
    pub flags: String,
    pub etype: String,
    /// Optional default value for this member. When set, the tagxml writer
    /// skips this member if its value equals the default; the tagxml reader
    /// fills in the default when the member is absent in the source XML.
    /// Not parsed from classxml (no SDK equivalent in the XML format) — set
    /// programmatically by code that knows class-specific defaults.
    pub default: Option<crate::hkx::types::HkxValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDescriptor {
    pub name: String,
    pub version: u32,
    pub signature: String,
    pub parent: Option<String>,
    pub is_struct: bool,
    pub members: Vec<MemberTemplate>,
    pub enums: HashMap<String, EnumDef>,
    /// Capability tag: authoring-side Setup vs runtime vs internal.
    /// Not derived from classxml; inferred from class name heuristic or
    /// set explicitly via `DescriptorRegistry::insert`.
    pub kind: ClassKind,
}

#[derive(Debug, Clone)]
pub struct DescriptorRegistry {
    dir: PathBuf,
    index: HashMap<String, String>,
    cache: HashMap<String, Option<ClassDescriptor>>,
}

impl Default for DescriptorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DescriptorRegistry {
    pub fn new() -> Self {
        Self::from_dir(default_classxml_dir()).expect("bundled classxml registry is valid")
    }

    pub fn for_version(version: &str) -> DescriptorResult<Self> {
        let versioned = resource_root().join(format!("classxml_{version}"));
        if versioned.exists() {
            Self::from_dir(versioned)
        } else {
            Err(DescriptorError::MissingClassxmlDir(versioned))
        }
    }

    /// Map a contents version string (e.g. `hk_2014.1.0-r1`) to the
    /// appropriate classxml variant and load it.
    ///
    /// Falls back to the default FO4 classxml for any recognised `hk_20xx`
    /// prefix when the versioned classxml directory does not exist, but logs
    /// a warning for completely unknown prefixes (e.g. `hk_2009`, `hk_2019+`)
    /// so callers are not silently using wrong member offsets.
    pub fn for_contents_version(contents_version: &str) -> Self {
        let suffix = if contents_version.starts_with("hk_2010")
            || contents_version.starts_with("hk_2012")
            || contents_version.starts_with("hk_2013")
        {
            Some("2012")
        } else if contents_version.starts_with("hk_2015")
            || contents_version.starts_with("hk_2016")
            || contents_version.starts_with("hk_2017")
            || contents_version.starts_with("hk_2018")
        {
            Some("2015")
        } else if contents_version.starts_with("hk_2014") || contents_version.is_empty() {
            // Default: FO4 (hk_2014) uses Self::new() directly.
            None
        } else {
            // Completely unknown version — warn so the caller knows member
            // offsets may be wrong. DescriptorError::UnknownVersion is the
            // typed form for callers that can propagate it.
            eprintln!(
                "havok: for_contents_version: no classxml mapping for {:?}; \
                 falling back to FO4 descriptors — member offsets may be incorrect",
                contents_version
            );
            None
        };
        suffix
            .and_then(|version| Self::for_version(version).ok())
            .unwrap_or_else(Self::new)
    }

    pub fn from_dir(path: impl AsRef<Path>) -> DescriptorResult<Self> {
        let dir = path.as_ref().to_path_buf();
        if !dir.exists() {
            return Err(DescriptorError::MissingClassxmlDir(dir));
        }
        let index = load_index(&dir)?;
        Ok(Self {
            dir,
            index,
            cache: HashMap::new(),
        })
    }

    pub fn get(&mut self, class_name: &str) -> DescriptorResult<Option<&ClassDescriptor>> {
        if !self.cache.contains_key(class_name) {
            let desc = self.load_descriptor(class_name)?;
            self.cache.insert(class_name.to_string(), desc);
        }
        Ok(self.cache.get(class_name).and_then(Option::as_ref))
    }

    /// Insert or replace a descriptor in the registry cache. Used by tests
    /// and code generators that need to set member defaults programmatically.
    pub fn insert(&mut self, descriptor: ClassDescriptor) {
        self.cache.insert(descriptor.name.clone(), Some(descriptor));
    }

    /// Return the `ClassKind` for a class. Falls back to the name heuristic
    /// when the class is not in the registry (e.g. no classxml file exists).
    pub fn class_kind(&mut self, class_name: &str) -> ClassKind {
        if let Ok(Some(desc)) = self.get(class_name) {
            return desc.kind;
        }
        // Name-based fallback: used for classes without classxml (e.g. cloth Setup classes).
        if class_name.contains("Setup") {
            ClassKind::Setup
        } else {
            ClassKind::Runtime
        }
    }

    /// Convenience: returns true when the class is a build-time Setup class.
    pub fn is_setup(&mut self, class_name: &str) -> bool {
        self.class_kind(class_name) == ClassKind::Setup
    }

    pub fn get_all_members(&mut self, class_name: &str) -> DescriptorResult<Vec<MemberTemplate>> {
        let mut chain = Vec::new();
        let mut current = Some(class_name.to_string());
        let mut seen_classes = HashSet::new();

        while let Some(name) = current {
            if !seen_classes.insert(name.clone()) {
                return Err(DescriptorError::InheritanceCycle {
                    class_name: class_name.to_string(),
                });
            }
            let Some(desc) = self.get(&name)? else {
                break;
            };
            chain.push(desc.clone());
            current = desc.parent.clone().filter(|parent| !parent.is_empty());
        }

        let mut members = Vec::new();
        let mut seen_names = Vec::<String>::new();
        for desc in chain.iter().rev() {
            for member in &desc.members {
                if !seen_names.iter().any(|name| name == &member.name) {
                    seen_names.push(member.name.clone());
                    members.push(member.clone());
                }
            }
        }
        members.sort_by_key(|member| member.offset);
        Ok(members)
    }

    pub fn get_enum_value(&mut self, class_name: &str, enum_name: &str, int_value: i32) -> String {
        let mut current = Some(class_name.to_string());
        let mut seen_classes = HashSet::new();
        while let Some(name) = current {
            if !seen_classes.insert(name.clone()) {
                break;
            }
            let Ok(Some(desc)) = self.get(&name) else {
                break;
            };
            if let Some(enum_def) = desc.enums.get(enum_name) {
                return enum_def
                    .value_to_name(int_value as i64)
                    .map(str::to_string)
                    .unwrap_or_else(|| int_value.to_string());
            }
            current = desc.parent.clone().filter(|parent| !parent.is_empty());
        }
        int_value.to_string()
    }

    pub fn get_enum_int(&mut self, class_name: &str, enum_name: &str, str_value: &str) -> i32 {
        if let Ok(value) = str_value.parse::<i32>() {
            return value;
        }

        let mut current = Some(class_name.to_string());
        let mut seen_classes = HashSet::new();
        while let Some(name) = current {
            if !seen_classes.insert(name.clone()) {
                break;
            }
            let Ok(Some(desc)) = self.get(&name) else {
                break;
            };
            if let Some(enum_def) = desc.enums.get(enum_name) {
                return enum_def.name_to_value(str_value).unwrap_or(0) as i32;
            }
            current = desc.parent.clone().filter(|parent| !parent.is_empty());
        }
        0
    }

    fn load_descriptor(&self, class_name: &str) -> DescriptorResult<Option<ClassDescriptor>> {
        let Some(filename) = self.index.get(class_name) else {
            return Ok(None);
        };
        let path = self.dir.join(filename);
        let xml = fs::read_to_string(&path).map_err(|source| DescriptorError::Read {
            path: path.clone(),
            source,
        })?;
        Ok(Some(parse_classxml(&path, &xml)?))
    }
}

fn parse_classxml(path: &Path, xml: &str) -> DescriptorResult<ClassDescriptor> {
    let doc = roxmltree::Document::parse(xml).map_err(|source| DescriptorError::Xml {
        path: path.to_path_buf(),
        source,
    })?;
    let root = doc.root_element();
    if !root.has_tag_name("class") && !root.has_tag_name("struct") {
        return Err(malformed(path, "root must be class or struct"));
    }
    let name = required_attr(path, root, "name")?;

    let class_kind = if name.contains("Setup") {
        ClassKind::Setup
    } else {
        ClassKind::Runtime
    };
    let mut desc = ClassDescriptor {
        name: name.to_string(),
        version: parse_optional_u32(path, root, "version")?.unwrap_or(0),
        signature: root.attribute("signature").unwrap_or_default().to_string(),
        parent: root.attribute("parent").map(str::to_string),
        is_struct: root.tag_name().name() == "struct",
        members: Vec::new(),
        enums: HashMap::new(),
        kind: class_kind,
    };

    for enum_el in root.descendants().filter(|node| node.has_tag_name("enum")) {
        let enum_name = enum_el.attribute("name").unwrap_or_default().to_string();
        let mut enum_def = EnumDef::default();
        for item in enum_el
            .children()
            .filter(|node| node.has_tag_name("enumitem"))
        {
            let item_name = item.attribute("name").unwrap_or_default().to_string();
            let item_value = parse_required_i64(path, item, "value")?;
            enum_def.values.push((item_name, item_value));
        }
        desc.enums.insert(enum_name, enum_def);
    }

    for member_el in root
        .descendants()
        .filter(|node| node.has_tag_name("member"))
    {
        let vtype_name = member_el.attribute("vtype").unwrap_or("TYPE_VOID");
        let vsubtype_name = member_el.attribute("vsubtype").unwrap_or("TYPE_VOID");
        desc.members.push(MemberTemplate {
            name: member_el.attribute("name").unwrap_or_default().to_string(),
            offset: parse_optional_usize(path, member_el, "offset")?.unwrap_or(0),
            vtype: HkxType::from_classxml_name(vtype_name)
                .ok_or_else(|| malformed(path, format!("unknown vtype {vtype_name}")))?,
            vsubtype: HkxType::from_classxml_name(vsubtype_name)
                .ok_or_else(|| malformed(path, format!("unknown vsubtype {vsubtype_name}")))?,
            ctype: member_el.attribute("ctype").unwrap_or_default().to_string(),
            arrsize: parse_optional_usize(path, member_el, "arrsize")?.unwrap_or(0),
            flags: member_el
                .attribute("flags")
                .unwrap_or("FLAGS_NONE")
                .to_string(),
            etype: member_el.attribute("etype").unwrap_or_default().to_string(),
            default: None,
        });
    }

    Ok(desc)
}

fn load_index(dir: &Path) -> DescriptorResult<HashMap<String, String>> {
    let index_path = dir.join("_index.json");
    if let Ok(text) = fs::read_to_string(index_path) {
        let index = serde_json::from_str::<HashMap<String, String>>(&text).map_err(|source| {
            DescriptorError::InvalidIndex {
                path: dir.join("_index.json"),
                source,
            }
        })?;
        for (class_name, filename) in &index {
            validate_index_filename(class_name, filename)?;
        }
        return Ok(index);
    }

    let mut index = HashMap::new();
    let entries = fs::read_dir(dir).map_err(|source| DescriptorError::Read {
        path: dir.to_path_buf(),
        source,
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("xml") {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let class_name = stem.rsplit_once('_').map_or(stem, |(name, _)| name);
        index.insert(class_name.to_string(), filename.to_string());
    }
    Ok(index)
}

fn validate_index_filename(class_name: &str, filename: &str) -> DescriptorResult<()> {
    let path = Path::new(filename);
    if path.is_absolute()
        || path.components().count() != 1
        || filename.contains('/')
        || filename.contains('\\')
        || path.extension().and_then(|ext| ext.to_str()) != Some("xml")
    {
        return Err(DescriptorError::UnsafeIndexPath {
            class_name: class_name.to_string(),
            filename: filename.to_string(),
        });
    }
    Ok(())
}

fn required_attr<'a>(
    path: &Path,
    node: roxmltree::Node<'a, 'a>,
    attr: &str,
) -> DescriptorResult<&'a str> {
    node.attribute(attr)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| malformed(path, format!("missing {attr}")))
}

fn parse_required_i64(
    path: &Path,
    node: roxmltree::Node<'_, '_>,
    attr: &str,
) -> DescriptorResult<i64> {
    let raw = required_attr(path, node, attr)?;
    // Accept decimal, or 0x-prefixed hex (used in some classxml flag values)
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).map_err(|_| malformed(path, format!("invalid {attr}")))
    } else {
        raw.parse::<i64>()
            .map_err(|_| malformed(path, format!("invalid {attr}")))
    }
}

fn parse_optional_u32(
    path: &Path,
    node: roxmltree::Node<'_, '_>,
    attr: &str,
) -> DescriptorResult<Option<u32>> {
    node.attribute(attr)
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_| malformed(path, format!("invalid {attr}")))
        })
        .transpose()
}

fn parse_optional_usize(
    path: &Path,
    node: roxmltree::Node<'_, '_>,
    attr: &str,
) -> DescriptorResult<Option<usize>> {
    node.attribute(attr)
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| malformed(path, format!("invalid {attr}")))
        })
        .transpose()
}

fn malformed(path: &Path, message: impl Into<String>) -> DescriptorError {
    DescriptorError::Malformed {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

fn default_classxml_dir() -> PathBuf {
    resource_root().join("classxml")
}

fn resource_root() -> PathBuf {
    if let Ok(value) = std::env::var("CREATION_LIB_RESOURCE_DIR") {
        return PathBuf::from(value);
    }
    lib_root()
        .join("python")
        .join("creation_lib")
        .join("resources")
}

/// Root of the `py_creation_lib` crate tree: two levels up from this crate's
/// `CARGO_MANIFEST_DIR` (`py_creation_lib/native/havok` -> `py_creation_lib`).
/// The classxml resources live at `python/creation_lib/resources` under this
/// root both in the private monorepo (where `py_creation_lib` is a
/// subdirectory) and in the standalone public repo (where `py_creation_lib`
/// itself is the repo root), so resolving from here works in both layouts.
fn lib_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("py_creation_lib/native/havok has a lib root two levels up")
        .to_path_buf()
}
