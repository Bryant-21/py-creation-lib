use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use indexmap::IndexMap;

pub const FLOAT_NAN_TAG: u64 = 1u64 << 32;

#[derive(Debug, Clone, PartialEq)]
pub enum NifValue {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    FloatNan(u64),
    String(String),
    Ref(i32),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Matrix33([[f32; 3]; 3]),
    Matrix44([[f32; 4]; 4]),
    Color3([f32; 3]),
    Color4([f32; 4]),
    Quaternion([f32; 4]),
    Array(Vec<NifValue>),
    Struct(IndexMap<String, NifValue>),
    Bytes(Vec<u8>),
    Char(String),
}

impl NifValue {
    pub fn as_i64(&self) -> i64 {
        match self {
            NifValue::Int(i) => *i,
            NifValue::UInt(u) => *u as i64,
            NifValue::Bool(b) => {
                if *b {
                    1
                } else {
                    0
                }
            }
            NifValue::Ref(r) => *r as i64,
            NifValue::Float(f) => *f as i64,
            NifValue::FloatNan(_) => 0,
            _ => 0,
        }
    }
    pub fn as_usize(&self) -> usize {
        let v = self.as_i64();
        if v < 0 { 0 } else { v as usize }
    }
}

fn hash_nif_value(value: &NifValue, hasher: &mut DefaultHasher) {
    std::mem::discriminant(value).hash(hasher);
    match value {
        NifValue::Null => {}
        NifValue::Bool(value) => value.hash(hasher),
        NifValue::Int(value) => value.hash(hasher),
        NifValue::UInt(value) => value.hash(hasher),
        NifValue::Float(value) => value.to_bits().hash(hasher),
        NifValue::FloatNan(value) => value.hash(hasher),
        NifValue::String(value) => value.hash(hasher),
        NifValue::Ref(value) => value.hash(hasher),
        NifValue::Vec3(value) => {
            for item in value {
                item.to_bits().hash(hasher);
            }
        }
        NifValue::Vec4(value) | NifValue::Color4(value) | NifValue::Quaternion(value) => {
            for item in value {
                item.to_bits().hash(hasher);
            }
        }
        NifValue::Matrix33(value) => {
            for row in value {
                for item in row {
                    item.to_bits().hash(hasher);
                }
            }
        }
        NifValue::Matrix44(value) => {
            for row in value {
                for item in row {
                    item.to_bits().hash(hasher);
                }
            }
        }
        NifValue::Color3(value) => {
            for item in value {
                item.to_bits().hash(hasher);
            }
        }
        NifValue::Array(items) => {
            items.len().hash(hasher);
            for item in items {
                hash_nif_value(item, hasher);
            }
        }
        NifValue::Struct(fields) => {
            fields.len().hash(hasher);
            for (key, value) in fields {
                key.hash(hasher);
                hash_nif_value(value, hasher);
            }
        }
        NifValue::Bytes(value) => value.hash(hasher),
        NifValue::Char(value) => value.hash(hasher),
    }
}

#[derive(Debug, Clone, Default)]
pub struct NifHeader {
    pub header_string: String,
    pub version: (u8, u8, u8, u8),
    pub version_packed: u32,
    pub endian_type: u8,
    pub user_version: u32,
    pub bs_version: u32,
    pub num_blocks: u32,
    pub creator: String,
    pub export_info: Vec<String>,
    pub sf_export_data: Vec<u8>,
    pub block_type_names: Vec<String>,
    pub block_type_index: Vec<u16>,
    pub block_sizes: Vec<u32>,
    pub strings: Vec<String>,
    pub max_string_length: u32,
    pub num_groups: u32,
    pub groups: Vec<u32>,
    pub footer_roots: Vec<i32>,
}

#[derive(Debug, Clone)]
pub struct NifBlock {
    pub block_id: usize,
    pub type_name: String,
    pub fields: IndexMap<String, NifValue>,
    pub remainder: Vec<u8>,
    pub original_bytes: Option<Vec<u8>>,
    pub original_content_hash: Option<u64>,
}

impl NifBlock {
    pub fn new(block_id: usize, type_name: impl Into<String>) -> Self {
        Self {
            block_id,
            type_name: type_name.into(),
            fields: IndexMap::new(),
            remainder: Vec::new(),
            original_bytes: None,
            original_content_hash: None,
        }
    }

    /// Return a field value by name. Tries exact match first, then falls back to
    /// matching against the "bare" name portion of disambiguated keys
    /// (`"name:suffix"` → `"name"`). Mirrors `NifBlock.get_field` in Python.
    pub fn get_field(&self, name: &str) -> Option<&NifValue> {
        if let Some(v) = self.fields.get(name) {
            return Some(v);
        }
        for (key, val) in self.fields.iter() {
            let bare = bare_name(key);
            if bare == name {
                return Some(val);
            }
        }
        None
    }

    pub fn get_field_mut(&mut self, name: &str) -> Option<&mut NifValue> {
        if self.fields.contains_key(name) {
            return self.fields.get_mut(name);
        }
        let matching_key = self
            .fields
            .keys()
            .find(|key| bare_name(key) == name)
            .cloned()?;
        self.fields.get_mut(&matching_key)
    }

    /// Set a field value. Tries exact match first, then bare-name fallback.
    /// If no existing slot matches, appends a new entry preserving insertion order.
    pub fn set_field(&mut self, name: &str, value: NifValue) {
        if self.fields.contains_key(name) {
            self.fields.insert(name.to_string(), value);
            return;
        }
        let matching_key: Option<String> =
            self.fields.keys().find(|k| bare_name(k) == name).cloned();
        if let Some(key) = matching_key {
            self.fields.insert(key, value);
            return;
        }
        self.fields.insert(name.to_string(), value);
    }

    /// Collect all Ref/Ptr block indices referenced by this block's fields.
    ///
    /// Recurses into `Struct` values (compound types like `NiControllerSequence`
    /// controlled blocks) and arrays of structs, mirroring `NifBlock.get_refs`
    /// in Python.
    pub fn get_refs(&self, schema: &crate::schema::NifSchema) -> Vec<i32> {
        let mut out: Vec<i32> = Vec::new();
        let all_fields = schema.get_all_fields(&self.type_name);
        let fdef_map = build_field_def_map(&all_fields);

        for (name, value) in self.fields.iter() {
            if let Some(fdef) = fdef_map.get(name.as_str()).copied() {
                collect_refs_from_value(value, fdef, schema, &mut out);
            }
        }
        out
    }

    /// Return `[(field_name, [block_ids])]` for all Ref/Ptr fields that contain
    /// at least one valid ref. Preserves insertion order of block fields.
    pub fn get_all_ref_fields(&self, schema: &crate::schema::NifSchema) -> Vec<(String, Vec<i32>)> {
        let mut out: Vec<(String, Vec<i32>)> = Vec::new();
        let all_fields = schema.get_all_fields(&self.type_name);
        let fdef_map = build_field_def_map(&all_fields);

        for (name, value) in self.fields.iter() {
            let fdef = match fdef_map.get(name.as_str()).copied() {
                Some(f) => f,
                None => continue,
            };
            if !is_ref_field(fdef) {
                continue;
            }
            let mut refs: Vec<i32> = Vec::new();
            collect_ref_scalars(value, &mut refs);
            if !refs.is_empty() {
                out.push((name.clone(), refs));
            }
        }
        out
    }

    pub fn content_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.type_name.hash(&mut hasher);
        for (key, value) in self.fields.iter() {
            key.hash(&mut hasher);
            hash_nif_value(value, &mut hasher);
        }
        self.remainder.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBlockContext {
    version_packed: u32,
    endian_type: u8,
    user_version: u32,
    bs_version: u32,
    strings: Vec<String>,
}

impl RawBlockContext {
    pub fn from_header(header: &NifHeader) -> Self {
        Self {
            version_packed: header.version_packed,
            endian_type: header.endian_type,
            user_version: header.user_version,
            bs_version: header.bs_version,
            strings: header.strings.clone(),
        }
    }

    pub fn matches_header(&self, header: &NifHeader) -> bool {
        self.version_packed == header.version_packed
            && self.endian_type == header.endian_type
            && self.user_version == header.user_version
            && self.bs_version == header.bs_version
            && self.strings == header.strings
    }
}

#[derive(Debug, Clone, Default)]
pub struct NifFile {
    pub header: NifHeader,
    pub blocks: Vec<NifBlock>,
    pub path: Option<PathBuf>,
    pub raw_block_context: Option<RawBlockContext>,
}

impl NifFile {
    /// Parse a NIF from an in-memory byte buffer via [`crate::io::NifReader`].
    pub fn from_bytes(bytes: &[u8], path: Option<PathBuf>) -> Result<Self, crate::io::ReadError> {
        let schema = &*crate::schema::SCHEMA;
        let mut nif = crate::io::NifReader::read(bytes, schema)?;
        nif.path = path;
        Ok(nif)
    }

    /// Parse a complete NIF without retaining writer-only original block bytes
    /// and hashes. Use the default loader when raw-block reuse may be required.
    pub fn from_bytes_lean(
        bytes: &[u8],
        path: Option<PathBuf>,
    ) -> Result<Self, crate::io::ReadError> {
        let schema = &*crate::schema::SCHEMA;
        let mut nif = crate::io::NifReader::read_lean(bytes, schema)?;
        nif.path = path;
        Ok(nif)
    }

    /// Read a NIF file from disk via [`crate::io::NifReader`].
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, crate::io::ReadError> {
        let path: PathBuf = path.into();
        let bytes = std::fs::read(&path).map_err(|e| {
            crate::io::ReadError::Other(format!("failed to read {}: {}", path.display(), e))
        })?;
        Self::from_bytes(&bytes, Some(path))
    }

    /// Read and fully decode a NIF without retaining writer-only original block data.
    pub fn load_lean(path: impl Into<PathBuf>) -> Result<Self, crate::io::ReadError> {
        let path: PathBuf = path.into();
        let bytes = std::fs::read(&path).map_err(|e| {
            crate::io::ReadError::Other(format!("failed to read {}: {}", path.display(), e))
        })?;
        Self::from_bytes_lean(&bytes, Some(path))
    }

    pub(crate) fn load_for_header_retarget(
        path: impl Into<PathBuf>,
        target_game: &str,
    ) -> Result<Self, crate::io::ReadError> {
        let path: PathBuf = path.into();
        let bytes = std::fs::read(&path).map_err(|e| {
            crate::io::ReadError::Other(format!("failed to read {}: {}", path.display(), e))
        })?;
        let mut header_reader = crate::io::BasicReader::new(std::io::Cursor::new(bytes.as_slice()));
        let source_header = crate::io::reader::read_header(&mut header_reader)?;
        let target_header = Self::new(target_game).header;
        let will_retarget = source_header.version_packed != target_header.version_packed
            || source_header.user_version != target_header.user_version
            || source_header.bs_version != target_header.bs_version;
        if will_retarget {
            Self::from_bytes_lean(&bytes, Some(path))
        } else {
            Self::from_bytes(&bytes, Some(path))
        }
    }

    /// Read external material and texture references without decoding unrelated
    /// sized blocks. Legacy NIFs retain the full-reader path.
    pub fn load_referenced_asset_paths(
        path: impl Into<PathBuf>,
    ) -> Result<ReferencedAssetPaths, crate::io::ReadError> {
        let path = path.into();
        let file = std::fs::File::open(&path).map_err(|e| {
            crate::io::ReadError::Other(format!("failed to read {}: {}", path.display(), e))
        })?;
        let schema = &*crate::schema::SCHEMA;
        match crate::io::NifReader::read_referenced_asset_paths(file, schema)? {
            Some(refs) => Ok(refs),
            None => Self::load(path).map(|nif| nif.referenced_asset_paths()),
        }
    }

    /// Serialize this NIF to bytes via [`crate::io::NifWriter`].
    pub fn to_bytes(&mut self) -> Result<Vec<u8>, crate::io::WriteError> {
        let schema = &*crate::schema::SCHEMA;
        crate::io::NifWriter::write_to_bytes(self, schema)
    }

    /// Write this NIF to disk via [`crate::io::NifWriter`]. If `path` is
    /// `None`, uses the previously loaded path.
    pub fn save(&mut self, path: Option<PathBuf>) -> Result<(), crate::io::WriteError> {
        let target = path
            .or_else(|| self.path.clone())
            .ok_or_else(|| crate::io::WriteError::Other("no filepath specified".to_string()))?;
        let bytes = self.to_bytes()?;
        std::fs::write(&target, &bytes).map_err(|e| {
            crate::io::WriteError::Other(format!("failed to write {}: {}", target.display(), e))
        })?;
        self.path = Some(target);
        Ok(())
    }

    /// Construct a blank NIF with a game-appropriate header and root block.
    /// Supports the short aliases (`"fo4"`, `"skyrimse"`,
    /// `"fo76"`, `"starfield"`) and falls back to FO4 defaults for unknown
    /// values, mirroring `NifFile.new` in Python.
    pub fn new(game: &str) -> Self {
        let mut nif = NifFile::default();
        let (version, user_version, bs_version) = default_game_versions(game);
        nif.header.header_string = format!(
            "Gamebryo File Format, Version {}.{}.{}.{}",
            version.0, version.1, version.2, version.3
        );
        nif.header.version = version;
        nif.header.version_packed = ((version.0 as u32) << 24)
            | ((version.1 as u32) << 16)
            | ((version.2 as u32) << 8)
            | (version.3 as u32);
        nif.header.user_version = user_version;
        nif.header.bs_version = bs_version;
        nif.header.endian_type = 1;
        nif.header.export_info = vec![String::new(), String::new(), String::new()];

        let root_type = if matches!(
            game.to_ascii_lowercase().as_str(),
            "morrowind" | "tes3" | "oblivion" | "tes4"
        ) {
            "NiNode"
        } else {
            "BSFadeNode"
        };
        let mut root = NifBlock::new(0, root_type);
        root.set_field("Name", NifValue::String(String::new()));
        root.set_field("Num Extra Data List", NifValue::UInt(0));
        root.set_field("Extra Data List", NifValue::Array(Vec::new()));
        root.set_field("Controller", NifValue::Ref(-1));
        root.set_field("Flags", NifValue::UInt(14));
        root.set_field("Translation", NifValue::Vec3([0.0, 0.0, 0.0]));
        root.set_field(
            "Rotation",
            NifValue::Matrix33([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        );
        root.set_field("Scale", NifValue::Float(1.0));
        root.set_field("Collision Object", NifValue::Ref(-1));
        root.set_field("Num Children", NifValue::UInt(0));
        root.set_field("Children", NifValue::Array(Vec::new()));
        nif.blocks.push(root);
        nif.header.num_blocks = 1;
        nif.header.block_type_names = vec![root_type.to_string()];
        nif.header.block_type_index = vec![0];
        nif.header.block_sizes = vec![0];
        nif.header.footer_roots = vec![0];
        nif
    }

    /// Append a new block and update header bookkeeping. Returns the new
    /// block's id (its index in `self.blocks`).
    pub fn add_block(
        &mut self,
        type_name: impl Into<String>,
        fields: Option<IndexMap<String, NifValue>>,
    ) -> usize {
        let type_name: String = type_name.into();
        let bid = self.blocks.len();
        let mut block = NifBlock::new(bid, type_name.clone());
        for fdef in crate::schema::SCHEMA.get_all_fields(&type_name) {
            if fdef.is_abstract {
                continue;
            }
            let key = field_key(fdef);
            block
                .fields
                .insert(key, default_value_for_field(fdef, &crate::schema::SCHEMA));
        }
        if let Some(initial) = fields {
            for (k, v) in initial {
                block.fields.insert(k, v);
            }
        }
        self.blocks.push(block);

        if !self.header.block_type_names.iter().any(|n| n == &type_name) {
            self.header.block_type_names.push(type_name.clone());
        }
        let type_idx = self
            .header
            .block_type_names
            .iter()
            .position(|n| n == &type_name)
            .unwrap_or(0) as u16;
        self.header.block_type_index.push(type_idx);
        self.header.block_sizes.push(0);
        self.header.num_blocks = self.blocks.len() as u32;
        bid
    }

    pub fn insert_block(&mut self, index: usize, type_name: impl Into<String>) -> usize {
        let index = index.min(self.blocks.len());
        let block_id = self.add_block(type_name, None);
        let block = self.blocks.pop().unwrap();
        let type_index = self.header.block_type_index.pop().unwrap();
        let block_size = self.header.block_sizes.pop().unwrap();
        for existing in &mut self.blocks {
            for value in existing.fields.values_mut() {
                shift_refs_at_or_after(value, index);
            }
        }
        for root in &mut self.header.footer_roots {
            if *root >= index as i32 {
                *root += 1;
            }
        }
        self.blocks.insert(index, block);
        self.header.block_type_index.insert(index, type_index);
        self.header.block_sizes.insert(index, block_size);
        for (new_id, block) in self.blocks.iter_mut().enumerate() {
            block.block_id = new_id;
        }
        self.header.num_blocks = self.blocks.len() as u32;
        debug_assert_eq!(block_id, self.blocks.len() - 1);
        index
    }

    /// Remove blocks by id and remap every remaining Ref/Ptr to the new index
    /// space. Refs targeting removed blocks are rewritten to `-1`.
    pub fn remove_blocks(&mut self, block_ids: &[usize]) {
        if block_ids.is_empty() {
            return;
        }
        let remove: HashSet<usize> = block_ids.iter().copied().collect();

        let mut id_map: HashMap<i32, i32> = HashMap::with_capacity(self.blocks.len());
        let mut new_id: i32 = 0;
        for old_id in 0..self.blocks.len() {
            if remove.contains(&old_id) {
                id_map.insert(old_id as i32, -1);
            } else {
                id_map.insert(old_id as i32, new_id);
                new_id += 1;
            }
        }

        let mut new_blocks: Vec<NifBlock> = Vec::with_capacity(self.blocks.len() - remove.len());
        for mut block in std::mem::take(&mut self.blocks) {
            if remove.contains(&block.block_id) {
                continue;
            }
            block.block_id = *id_map.get(&(block.block_id as i32)).unwrap_or(&-1) as usize;
            new_blocks.push(block);
        }
        self.blocks = new_blocks;
        self.header.footer_roots = self
            .header
            .footer_roots
            .iter()
            .filter_map(|root| id_map.get(root).copied())
            .filter(|root| *root >= 0)
            .collect();
        self.remap_refs(&id_map);
        self.rebuild_header();
    }

    pub fn convert_block_type(&mut self, block_id: usize, new_type: &str) -> Result<bool, String> {
        let Some(definition) = crate::schema::SCHEMA.get_niobject(new_type) else {
            return Err(format!("Unknown NIF block type: {new_type}"));
        };
        if definition.abstract_ {
            return Err(format!("Cannot convert to abstract block type: {new_type}"));
        }
        let Some(block) = self.blocks.get(block_id) else {
            return Err(format!("Block {block_id} does not exist"));
        };
        if block.type_name == new_type {
            return Ok(false);
        }
        let old_hierarchy = crate::schema::SCHEMA
            .get_type_hierarchy(&block.type_name)
            .into_iter()
            .collect::<HashSet<_>>();
        if !crate::schema::SCHEMA
            .get_type_hierarchy(new_type)
            .iter()
            .any(|type_name| old_hierarchy.contains(type_name))
        {
            return Err(format!(
                "{} and {new_type} do not share a NIF base type",
                block.type_name
            ));
        }
        let old_fields = block
            .fields
            .iter()
            .map(|(name, value)| (bare_name(name).to_string(), value.clone()))
            .collect::<HashMap<_, _>>();
        let mut new_fields = IndexMap::new();
        for field in crate::schema::SCHEMA.get_all_fields(new_type) {
            if field.is_abstract {
                continue;
            }
            new_fields.insert(
                field_key(field),
                old_fields
                    .get(field.name)
                    .cloned()
                    .unwrap_or_else(|| default_value_for_field(field, &crate::schema::SCHEMA)),
            );
        }
        let block = &mut self.blocks[block_id];
        block.type_name = new_type.to_string();
        block.fields = new_fields;
        block.original_bytes = None;
        block.original_content_hash = None;
        self.rebuild_header();
        Ok(true)
    }

    /// Rewrite Ref/Ptr values in every block according to `id_map`. Refs not
    /// present in the map are rewritten to `-1` (matching Python's
    /// `missing_default=-1` semantics used by `remove_blocks`). Negative
    /// refs (-1 sentinels) are preserved as-is.
    pub fn remap_refs(&mut self, id_map: &HashMap<i32, i32>) {
        let schema = &*crate::schema::SCHEMA;
        for block in self.blocks.iter_mut() {
            remap_block_refs(block, id_map, schema);
        }
    }

    /// Return `{block_id → [child_block_ids]}` built from each block's
    /// Ref/Ptr fields. Useful for downstream tree UIs without forcing a full
    /// recursive walk.
    pub fn get_hierarchy(&self) -> HashMap<i32, Vec<i32>> {
        let schema = &*crate::schema::SCHEMA;
        let mut out: HashMap<i32, Vec<i32>> = HashMap::with_capacity(self.blocks.len());
        for block in self.blocks.iter() {
            out.insert(block.block_id as i32, block.get_refs(schema));
        }
        out
    }

    /// Return indices of all blocks whose `type_name` is `type_name` or a
    /// subtype thereof.
    pub fn find_blocks(&self, type_name: &str) -> Vec<usize> {
        let schema = &*crate::schema::SCHEMA;
        let mut out: Vec<usize> = Vec::new();
        for (i, block) in self.blocks.iter().enumerate() {
            if schema.is_subtype_of(&block.type_name, type_name) {
                out.push(i);
            }
        }
        out
    }

    /// Return a reference to the block at `block_id`, or `None` if out of range.
    pub fn get_block(&self, block_id: usize) -> Option<&NifBlock> {
        self.blocks.get(block_id)
    }

    /// Rebuild `block_type_names`, `block_type_index`, `block_sizes`, and
    /// `num_blocks` from the current `blocks` list. Used after structural
    /// edits like [`Self::remove_blocks`].
    pub fn rebuild_header(&mut self) {
        let mut names: Vec<String> = Vec::new();
        let mut indices: Vec<u16> = Vec::with_capacity(self.blocks.len());
        for block in self.blocks.iter() {
            let idx = match names.iter().position(|n| n == &block.type_name) {
                Some(i) => i,
                None => {
                    names.push(block.type_name.clone());
                    names.len() - 1
                }
            };
            indices.push(idx as u16);
        }
        self.header.block_type_names = names;
        self.header.block_type_index = indices;
        self.header.block_sizes = vec![0; self.blocks.len()];
        self.header.num_blocks = self.blocks.len() as u32;
    }
}

fn shift_refs_at_or_after(value: &mut NifValue, index: usize) {
    match value {
        NifValue::Ref(reference) if *reference >= index as i32 => *reference += 1,
        NifValue::Array(values) => {
            for value in values {
                shift_refs_at_or_after(value, index);
            }
        }
        NifValue::Struct(fields) => {
            for value in fields.values_mut() {
                shift_refs_at_or_after(value, index);
            }
        }
        _ => {}
    }
}

// ---------- helpers ----------

fn bare_name(key: &str) -> &str {
    match key.find(':') {
        Some(i) => &key[..i],
        None => key,
    }
}

fn field_key(fdef: &crate::schema::FieldDef) -> String {
    match fdef.suffix {
        Some(suffix) => format!("{}:{}", fdef.name, suffix),
        None => fdef.name.to_string(),
    }
}

fn default_game_versions(game: &str) -> ((u8, u8, u8, u8), u32, u32) {
    let g = game.to_ascii_lowercase();
    match g.as_str() {
        "morrowind" | "tes3" => ((4, 0, 0, 2), 0, 0),
        "oblivion" | "tes4" => ((20, 0, 0, 5), 11, 11),
        "skyrim" | "tes5" => ((20, 2, 0, 7), 12, 83),
        "fo4" => ((20, 2, 0, 7), 12, 130),
        "skyrimse" => ((20, 2, 0, 7), 12, 100),
        "fo76" => ((20, 2, 0, 7), 12, 155),
        "starfield" => ((20, 2, 0, 7), 12, 170),
        "fo3" => ((20, 2, 0, 7), 11, 11),
        "fnv" => ((20, 2, 0, 7), 11, 34),
        _ => ((20, 2, 0, 7), 12, 130),
    }
}

fn default_value_for_field(
    fdef: &crate::schema::FieldDef,
    schema: &crate::schema::NifSchema,
) -> NifValue {
    if fdef.length.is_some() {
        return NifValue::Array(Vec::new());
    }
    if let Some(default) = fdef.default {
        if !default.is_empty() {
            return default_scalar_value(fdef.type_name, default);
        }
    }
    default_value_for_type(fdef.type_name, schema)
}

fn default_scalar_value(type_name: &str, default: &str) -> NifValue {
    let trimmed = default.trim().trim_matches('"');
    match type_name {
        "string" | "SizedString" | "SizedString16" | "HeaderString" | "LineString"
        | "NiFixedString" => NifValue::String(trimmed.to_string()),
        "char" => NifValue::Char(trimmed.chars().next().unwrap_or('\0').to_string()),
        "bool" => NifValue::Bool(matches!(
            trimmed.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        )),
        "float" | "hfloat" | "normbyte" => trimmed
            .parse::<f64>()
            .map(NifValue::Float)
            .unwrap_or(NifValue::Float(0.0)),
        "Ref" | "Ptr" => trimmed
            .parse::<i32>()
            .map(NifValue::Ref)
            .unwrap_or(NifValue::Ref(-1)),
        _ => trimmed
            .parse::<i64>()
            .map(NifValue::Int)
            .unwrap_or_else(|_| default_value_for_type(type_name, &crate::schema::SCHEMA)),
    }
}

fn default_value_for_type(type_name: &str, schema: &crate::schema::NifSchema) -> NifValue {
    match type_name {
        "byte" | "ushort" | "uint" | "uint64" | "ulittle32" | "BlockTypeIndex" | "FileVersion"
        | "StringOffset" => NifValue::UInt(0),
        "sbyte" | "short" | "int" | "int64" => NifValue::Int(0),
        "float" | "hfloat" | "normbyte" => NifValue::Float(0.0),
        "bool" => NifValue::Bool(false),
        "Ref" | "Ptr" => NifValue::Ref(-1),
        "string" | "SizedString" | "SizedString16" | "HeaderString" | "LineString"
        | "NiFixedString" => NifValue::String(String::new()),
        "char" => NifValue::Char(String::new()),
        _ if schema.get_enum(type_name).is_some()
            || schema.get_bitflag(type_name).is_some()
            || schema.get_bitfield(type_name).is_some() =>
        {
            NifValue::UInt(0)
        }
        _ => match schema.get_struct(type_name) {
            Some(struct_def) => {
                let mut fields = IndexMap::new();
                for sf in struct_def.fields.iter() {
                    if sf.is_abstract || sf.length.is_some() {
                        continue;
                    }
                    fields.insert(field_key(sf), default_value_for_field(sf, schema));
                }
                NifValue::Struct(fields)
            }
            None => NifValue::Int(0),
        },
    }
}

fn build_field_def_map<'a>(
    all_fields: &[&'a crate::schema::FieldDef],
) -> HashMap<&'a str, &'a crate::schema::FieldDef> {
    let mut map: HashMap<&'a str, &'a crate::schema::FieldDef> =
        HashMap::with_capacity(all_fields.len() * 2);
    for f in all_fields.iter().copied() {
        map.insert(f.name, f);
        // Note: keys in IndexMap are stored with `:suffix` when present, so
        // we also need a lookup by the decorated form. We rebuild that here
        // using a static concat would require owned strings; callers already
        // do bare-name lookup via `name`, so we only register the bare name.
    }
    map
}

fn is_ref_field(fdef: &crate::schema::FieldDef) -> bool {
    matches!(fdef.type_name, "Ref" | "Ptr") || matches!(fdef.template, Some("Ref") | Some("Ptr"))
}

fn collect_ref_scalars(value: &NifValue, out: &mut Vec<i32>) {
    match value {
        NifValue::Ref(r) if *r >= 0 => out.push(*r),
        NifValue::Int(i) if *i >= 0 => out.push(*i as i32),
        NifValue::UInt(u) => out.push(*u as i32),
        NifValue::Array(arr) => {
            for v in arr {
                match v {
                    NifValue::Ref(r) if *r >= 0 => out.push(*r),
                    NifValue::Int(i) if *i >= 0 => out.push(*i as i32),
                    NifValue::UInt(u) => out.push(*u as i32),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn collect_refs_from_value(
    value: &NifValue,
    fdef: &crate::schema::FieldDef,
    schema: &crate::schema::NifSchema,
    out: &mut Vec<i32>,
) {
    if is_ref_field(fdef) {
        collect_ref_scalars(value, out);
        return;
    }
    match value {
        NifValue::Struct(inner) => {
            collect_refs_from_struct(inner, fdef.type_name, schema, out);
        }
        NifValue::Array(arr) => {
            for item in arr {
                if let NifValue::Struct(inner) = item {
                    collect_refs_from_struct(inner, fdef.type_name, schema, out);
                }
            }
        }
        _ => {}
    }
}

fn collect_refs_from_struct(
    struct_val: &IndexMap<String, NifValue>,
    struct_type: &str,
    schema: &crate::schema::NifSchema,
    out: &mut Vec<i32>,
) {
    let sdef = match schema.get_struct(struct_type) {
        Some(s) => s,
        None => return,
    };
    let fdef_map: HashMap<&str, &crate::schema::FieldDef> =
        sdef.fields.iter().map(|f| (f.name, f)).collect();
    for (key, val) in struct_val.iter() {
        let sfdef = match fdef_map.get(bare_name(key)) {
            Some(&f) => f,
            None => continue,
        };
        collect_refs_from_value(val, sfdef, schema, out);
    }
}

fn lookup_ref(v: i32, id_map: &HashMap<i32, i32>) -> i32 {
    if let Some(&mapped) = id_map.get(&v) {
        return mapped;
    }
    if v < 0 {
        return v;
    }
    -1
}

fn remap_block_refs(
    block: &mut NifBlock,
    id_map: &HashMap<i32, i32>,
    schema: &crate::schema::NifSchema,
) {
    let all_fields = schema.get_all_fields(&block.type_name);
    let fdef_map = build_field_def_map(&all_fields);

    let keys: Vec<String> = block.fields.keys().cloned().collect();
    for key in keys {
        let fdef = match fdef_map.get(bare_name(&key)).copied() {
            Some(f) => f,
            None => continue,
        };
        if let Some(current) = block.fields.get(&key).cloned() {
            let new_val = remap_value(&current, fdef, id_map, schema);
            block.fields.insert(key, new_val);
        }
    }
}

fn remap_value(
    value: &NifValue,
    fdef: &crate::schema::FieldDef,
    id_map: &HashMap<i32, i32>,
    schema: &crate::schema::NifSchema,
) -> NifValue {
    if is_ref_field(fdef) {
        return match value {
            NifValue::Ref(r) => NifValue::Ref(lookup_ref(*r, id_map)),
            NifValue::Int(i) => NifValue::Int(lookup_ref(*i as i32, id_map) as i64),
            NifValue::Array(arr) => NifValue::Array(
                arr.iter()
                    .map(|v| match v {
                        NifValue::Ref(r) => NifValue::Ref(lookup_ref(*r, id_map)),
                        NifValue::Int(i) => NifValue::Int(lookup_ref(*i as i32, id_map) as i64),
                        other => other.clone(),
                    })
                    .collect(),
            ),
            other => other.clone(),
        };
    }

    match value {
        NifValue::Struct(inner) => {
            let sdef = match schema.get_struct(fdef.type_name) {
                Some(s) => s,
                None => return value.clone(),
            };
            let sfdef_map: HashMap<&str, &crate::schema::FieldDef> =
                sdef.fields.iter().map(|f| (f.name, f)).collect();
            let mut out: IndexMap<String, NifValue> = IndexMap::with_capacity(inner.len());
            for (k, v) in inner.iter() {
                let new_val = match sfdef_map.get(bare_name(k)).copied() {
                    Some(sfdef) => remap_value(v, sfdef, id_map, schema),
                    None => v.clone(),
                };
                out.insert(k.clone(), new_val);
            }
            NifValue::Struct(out)
        }
        NifValue::Array(arr) => {
            if arr.is_empty() {
                return value.clone();
            }
            if let Some(NifValue::Struct(_)) = arr.first() {
                let sdef = match schema.get_struct(fdef.type_name) {
                    Some(s) => s,
                    None => return value.clone(),
                };
                let sfdef_map: HashMap<&str, &crate::schema::FieldDef> =
                    sdef.fields.iter().map(|f| (f.name, f)).collect();
                let new_arr: Vec<NifValue> = arr
                    .iter()
                    .map(|item| match item {
                        NifValue::Struct(inner) => {
                            let mut out: IndexMap<String, NifValue> =
                                IndexMap::with_capacity(inner.len());
                            for (k, v) in inner.iter() {
                                let nv = match sfdef_map.get(bare_name(k)).copied() {
                                    Some(sfdef) => remap_value(v, sfdef, id_map, schema),
                                    None => v.clone(),
                                };
                                out.insert(k.clone(), nv);
                            }
                            NifValue::Struct(out)
                        }
                        other => other.clone(),
                    })
                    .collect();
                return NifValue::Array(new_arr);
            }
            value.clone()
        }
        _ => value.clone(),
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReferencedAssetPaths {
    pub textures: Vec<String>,
    pub materials: Vec<String>,
}

impl NifFile {
    /// Enumerate texture-slot (`BSShaderTextureSet.Textures`), legacy inline
    /// texture (`File Name`), and external material (`Name` on a shader
    /// property) paths this NIF references, as normalized data-relative
    /// rel-paths (lowercase, forward-slash). Empty strings and duplicates are
    /// dropped; insertion order preserved.
    pub fn referenced_asset_paths(&self) -> ReferencedAssetPaths {
        fn norm(raw: &str, root: &str) -> Option<String> {
            let s = raw.trim().trim_matches('\0').trim();
            if s.is_empty() {
                return None;
            }
            let mut p = s
                .replace('\\', "/")
                .trim_start_matches('/')
                .to_ascii_lowercase();
            if let Some((_, rest)) = p.split_once("/data/") {
                p = rest.to_string();
            } else if let Some(rest) = p.strip_prefix("data/") {
                p = rest.to_string();
            } else if let Some((_, rest)) = p.split_once(&format!("/{root}/")) {
                p = format!("{root}/{rest}");
            } else if p.contains(':') {
                return None;
            }
            if !p.starts_with(&format!("{root}/")) {
                p = format!("{root}/{p}");
            }
            Some(p)
        }
        let mut out = ReferencedAssetPaths::default();
        for block in &self.blocks {
            match block.type_name.as_str() {
                "BSShaderTextureSet" => {
                    if let Some(NifValue::Array(items)) = block.get_field("Textures") {
                        for item in items {
                            if let NifValue::String(p) = item {
                                if let Some(n) = norm(p, "textures") {
                                    if !out.textures.contains(&n) {
                                        out.textures.push(n);
                                    }
                                }
                            }
                        }
                    }
                }
                "TallGrassShaderProperty" | "BSShaderNoLightingProperty" => {
                    if let Some(NifValue::String(path)) = block.get_field("File Name") {
                        if let Some(normalized) = norm(path, "textures") {
                            if !out.textures.contains(&normalized) {
                                out.textures.push(normalized);
                            }
                        }
                    }
                }
                "BSLightingShaderProperty" | "BSEffectShaderProperty" => {
                    let mut has_external_material = false;
                    if let Some(NifValue::String(name)) = block.get_field("Name") {
                        if let Some(n) = norm(name, "materials") {
                            let is_material = n.ends_with(".bgsm") || n.ends_with(".bgem");
                            if is_material && !out.materials.contains(&n) {
                                out.materials.push(n);
                            }
                            has_external_material = is_material;
                        }
                    }
                    if block.type_name == "BSEffectShaderProperty" && !has_external_material {
                        for field in [
                            "Source Texture",
                            "Greyscale Texture",
                            "Env Map Texture",
                            "Normal Texture",
                            "Env Mask Texture",
                        ] {
                            if let Some(NifValue::String(path)) = block.get_field(field) {
                                if let Some(normalized) = norm(path, "textures") {
                                    if !out.textures.contains(&normalized) {
                                        out.textures.push(normalized);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::SCHEMA;

    #[test]
    fn get_field_bare_name_fallback() {
        let mut block = NifBlock::new(0, "NiObjectNET");
        block
            .fields
            .insert("Name:12".to_string(), NifValue::String("foo".to_string()));
        match block.get_field("Name") {
            Some(NifValue::String(s)) => assert_eq!(s, "foo"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn referenced_asset_paths_extracts_absolute_material_without_data_segment() {
        let mut nif = NifFile::new("fo76");
        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.fields.insert(
            "Name".to_string(),
            NifValue::String(
                r"C:\Projects\76\Build\PC\Materials\Landscape\Rocks\MtnTopCliff_Tiled01.BGSM"
                    .to_string(),
            ),
        );
        nif.blocks.push(shader);

        let refs = nif.referenced_asset_paths();

        assert_eq!(
            refs.materials,
            vec!["materials/landscape/rocks/mtntopcliff_tiled01.bgsm"]
        );
    }

    #[test]
    fn set_field_updates_suffixed_slot() {
        let mut block = NifBlock::new(0, "NiObjectNET");
        block
            .fields
            .insert("Name:5".to_string(), NifValue::String("old".to_string()));
        block.set_field("Name", NifValue::String("new".to_string()));
        match block.fields.get("Name:5") {
            Some(NifValue::String(s)) => assert_eq!(s, "new"),
            other => panic!("expected updated slot, got {:?}", other),
        }
        assert!(block.fields.get("Name").is_none());
    }

    #[test]
    fn referenced_asset_paths_collects_textures_and_material_name() {
        let mut nif = NifFile::default();

        let mut shader = NifBlock::new(0, "BSLightingShaderProperty");
        shader.set_field(
            "Name",
            NifValue::String(
                "C:\\Projects\\Fallout4\\Build\\PC\\Data\\Materials\\Landscape\\Rock01.bgsm\0"
                    .to_string(),
            ),
        );
        nif.blocks.push(shader);

        let mut texset = NifBlock::new(1, "BSShaderTextureSet");
        texset.set_field(
            "Textures",
            NifValue::Array(vec![
                NifValue::String("Textures\\Landscape\\Rock01_d.dds".to_string()),
                NifValue::String("Landscape\\Rock01_n.dds".to_string()),
                NifValue::String(
                    "C:\\Projects\\76\\Build\\PC\\Data\\Textures\\Landscape\\Rock01_s.dds"
                        .to_string(),
                ),
                NifValue::String(String::new()), // empty slot ignored
            ]),
        );
        nif.blocks.push(texset);

        let refs = nif.referenced_asset_paths();
        assert_eq!(
            refs.materials,
            vec!["materials/landscape/rock01.bgsm".to_string()]
        );
        assert_eq!(
            refs.textures,
            vec![
                "textures/landscape/rock01_d.dds".to_string(),
                "textures/landscape/rock01_n.dds".to_string(),
                "textures/landscape/rock01_s.dds".to_string(),
            ]
        );
    }

    #[test]
    fn referenced_asset_paths_collects_fnv_tall_grass_texture() {
        let mut nif = NifFile::new("fnv");
        let mut shader = NifBlock::new(0, "TallGrassShaderProperty");
        shader.set_field(
            "File Name",
            NifValue::String("textures\\landscape\\grass\\GrassWastelandComp01.dds".to_string()),
        );
        nif.blocks.push(shader);

        let refs = nif.referenced_asset_paths();

        assert_eq!(
            refs.textures,
            vec!["textures/landscape/grass/grasswastelandcomp01.dds"]
        );
    }

    #[test]
    fn referenced_asset_paths_collects_fnv_no_lighting_texture() {
        let mut nif = NifFile::new("fnv");
        let mut shader = NifBlock::new(0, "BSShaderNoLightingProperty");
        shader.set_field(
            "File Name",
            NifValue::String("textures\\effects\\FXDustSmallGen01.dds".to_string()),
        );
        nif.blocks.push(shader);

        let refs = nif.referenced_asset_paths();

        assert_eq!(refs.textures, vec!["textures/effects/fxdustsmallgen01.dds"]);
    }

    #[test]
    fn add_block_updates_header() {
        let mut nif = NifFile::default();
        nif.header.version_packed = 0x14020007;
        let bid1 = nif.add_block("NiNode", None);
        let bid2 = nif.add_block("NiNode", None);
        let bid3 = nif.add_block("BSTriShape", None);
        assert_eq!(bid1, 0);
        assert_eq!(bid2, 1);
        assert_eq!(bid3, 2);
        assert_eq!(nif.header.num_blocks, 3);
        assert_eq!(
            nif.header.block_type_names,
            vec!["NiNode".to_string(), "BSTriShape".to_string()]
        );
        assert_eq!(nif.header.block_type_index, vec![0u16, 0u16, 1u16]);
    }

    #[test]
    fn add_block_populates_schema_defaults_before_overrides() {
        let mut nif = NifFile::default();
        let mut overrides = IndexMap::new();
        overrides.insert("Name".to_string(), NifValue::String("Custom".to_string()));

        let bid = nif.add_block("NiNode", Some(overrides));
        let block = &nif.blocks[bid];

        match block.get_field("Name") {
            Some(NifValue::String(name)) => assert_eq!(name, "Custom"),
            other => panic!("expected overridden Name, got {:?}", other),
        }
        match block.get_field("Controller") {
            Some(NifValue::Ref(r)) => assert_eq!(*r, -1),
            other => panic!("expected default Controller Ref(-1), got {:?}", other),
        }
        match block.get_field("Children") {
            Some(NifValue::Array(children)) => assert!(children.is_empty()),
            other => panic!("expected default empty Children array, got {:?}", other),
        }
    }

    #[test]
    fn remove_blocks_remaps_refs() {
        let mut nif = NifFile::default();
        nif.add_block("NiNode", None); // 0 - keep
        nif.add_block("NiNode", None); // 1 - remove
        nif.add_block("NiNode", None); // 2 - keep

        // Block 0 has a Ref pointing to block 2
        nif.blocks[0]
            .fields
            .insert("Controller".to_string(), NifValue::Ref(2));

        nif.remove_blocks(&[1]);
        assert_eq!(nif.blocks.len(), 2);
        // Block 0 unchanged id, ref to old 2 becomes new 1
        match nif.blocks[0].fields.get("Controller") {
            Some(NifValue::Ref(r)) => assert_eq!(*r, 1),
            other => panic!("expected Ref(1), got {:?}", other),
        }
    }

    #[test]
    fn remap_refs_rewrites_removed_to_neg1() {
        let mut nif = NifFile::default();
        nif.add_block("NiNode", None);
        nif.add_block("NiNode", None);
        nif.add_block("NiNode", None);
        nif.blocks[0]
            .fields
            .insert("Controller".to_string(), NifValue::Ref(1));

        nif.remove_blocks(&[1]);
        match nif.blocks[0].fields.get("Controller") {
            Some(NifValue::Ref(r)) => assert_eq!(*r, -1),
            other => panic!("expected Ref(-1), got {:?}", other),
        }
    }

    #[test]
    fn find_blocks_honors_subtypes() {
        let mut nif = NifFile::default();
        nif.add_block("BSFadeNode", None); // subtype of NiNode
        nif.add_block("NiNode", None);
        nif.add_block("BSTriShape", None);

        let hits = nif.find_blocks("NiNode");
        // BSFadeNode inherits NiNode; NiNode itself too. BSTriShape is not a NiNode.
        assert!(hits.contains(&0));
        assert!(hits.contains(&1));
        assert!(!hits.contains(&2));
    }

    #[test]
    fn get_block_in_range() {
        let mut nif = NifFile::default();
        nif.add_block("NiNode", None);
        assert!(nif.get_block(0).is_some());
        assert!(nif.get_block(99).is_none());
    }

    #[test]
    fn new_fo4_creates_fade_node_root() {
        let nif = NifFile::new("fo4");
        assert_eq!(nif.blocks.len(), 1);
        assert_eq!(nif.blocks[0].type_name, "BSFadeNode");
        assert_eq!(nif.header.version, (20, 2, 0, 7));
        assert_eq!(nif.header.user_version, 12);
        assert_eq!(nif.header.bs_version, 130);
        assert_eq!(nif.header.endian_type, 1);
    }

    #[test]
    fn new_uses_profile_default_versions() {
        let cases = [
            ("fo4", 12, 130),
            ("skyrimse", 12, 100),
            ("fo76", 12, 155),
            ("starfield", 12, 170),
            ("fo3", 11, 11),
            ("fnv", 11, 34),
        ];
        for (game, user_version, bs_version) in cases {
            let nif = NifFile::new(game);
            assert_eq!(nif.header.version, (20, 2, 0, 7), "{game}");
            assert_eq!(nif.header.user_version, user_version, "{game}");
            assert_eq!(nif.header.bs_version, bs_version, "{game}");
        }
    }

    #[test]
    fn to_bytes_and_from_bytes_roundtrip_without_python_payload() {
        let mut nif = NifFile::new("fo4");
        let bytes = nif.to_bytes().expect("serialize new nif");
        let parsed = NifFile::from_bytes(&bytes, None).expect("parse serialized nif");

        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.blocks[0].type_name, "BSFadeNode");
        assert_eq!(parsed.header.version, (20, 2, 0, 7));
        assert!(parsed.path.is_none());
    }

    #[test]
    fn retarget_loader_keeps_raw_metadata_when_header_already_matches() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fo4_donor.nif");
        let mut source = NifFile::new("fo4");
        source.blocks[0].set_field("Name", NifValue::String("Nonempty donor root".into()));
        let source_bytes = source.to_bytes().unwrap();
        std::fs::write(&path, &source_bytes).unwrap();

        let mut parsed = NifFile::load_for_header_retarget(&path, "fo4").unwrap();
        assert!(parsed.blocks.iter().all(|block| block.original_bytes.is_some()));
        assert!(
            parsed
                .blocks
                .iter()
                .all(|block| block.original_content_hash.is_some())
        );
        assert!(!parsed.header.strings.is_empty());
        assert_eq!(parsed.to_bytes().unwrap(), source_bytes);
    }

    #[test]
    fn retarget_loader_uses_lean_decode_when_header_will_change() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fo76_source.nif");
        let mut source = NifFile::new("fo76");
        std::fs::write(&path, source.to_bytes().unwrap()).unwrap();

        let parsed = NifFile::load_for_header_retarget(&path, "fo4").unwrap();
        assert!(parsed.blocks.iter().all(|block| block.original_bytes.is_none()));
        assert!(
            parsed
                .blocks
                .iter()
                .all(|block| block.original_content_hash.is_none())
        );
    }

    #[test]
    fn get_hierarchy_returns_refs_per_block() {
        let mut nif = NifFile::default();
        nif.add_block("NiNode", None);
        nif.add_block("NiNode", None);
        nif.blocks[0]
            .fields
            .insert("Controller".to_string(), NifValue::Ref(1));
        let h = nif.get_hierarchy();
        assert_eq!(h.get(&0).map(|v| v.clone()).unwrap_or_default(), vec![1]);
        assert_eq!(
            h.get(&1).map(|v| v.clone()).unwrap_or_default(),
            Vec::<i32>::new()
        );
    }

    #[test]
    fn get_all_ref_fields_lists_only_refs() {
        let mut block = NifBlock::new(0, "NiNode");
        block
            .fields
            .insert("Controller".to_string(), NifValue::Ref(5));
        block
            .fields
            .insert("Name".to_string(), NifValue::String("x".to_string()));
        block.fields.insert(
            "Children".to_string(),
            NifValue::Array(vec![NifValue::Ref(2), NifValue::Ref(3)]),
        );
        let schema = &*SCHEMA;
        let refs = block.get_all_ref_fields(schema);
        let names: Vec<&str> = refs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"Controller"));
        assert!(names.contains(&"Children"));
        assert!(!names.contains(&"Name"));
    }

    #[test]
    fn lean_load_preserves_filesystem_error_interface() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.nif");
        assert_eq!(
            format!("{:?}", NifFile::load(&path).unwrap_err()),
            format!("{:?}", NifFile::load_lean(&path).unwrap_err())
        );
    }
}
