use crate::error::HavokResult;

use super::descriptors::DescriptorRegistry;
use super::packfile::{PackfileHeader, SectionHeader};
use super::packfile::{ParsedPackfile, parse_packfile};
use super::patcher::{PatchRange, apply_patch_range};
use super::reader::{ReadOutcome, read_objects_with_sources};
use super::tagfile::parse_tagfile;
use super::tagfile2014::{is_binary_tagfile_magic, read_tagfile2014};
use super::types::{HkxType, HkxValue};
use super::writer::write_hkx;

const TAG0_MAGIC: &[u8; 4] = b"TAG0";

#[derive(Debug, Clone, PartialEq)]
pub struct HkxObject {
    pub name: Option<String>,
    pub offset: usize,
    pub signature: u32,
    pub class_name: String,
    pub members: Vec<HkxMember>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HkxMember {
    pub name: String,
    pub value: HkxValue,
}

/// Source-byte location of an array's content payload, captured by the reader
/// so the byte-level patcher can rewrite the array in place on save without
/// going through the full binary writer.
///
/// `member_path` identifies the array within `objects[object_index]` as a
/// chain of member names (e.g. `["data"]` for a top-level array, or
/// `["splineParameters", "data"]` for an array inside an inline struct).
/// Each step indexes into the current object's `members` by name; for
/// arrays of inline structs the patcher recurses into elements separately
/// so paths only ever point at the array itself, not at one of its
/// elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArraySource {
    pub object_index: usize,
    pub member_path: Vec<String>,
    pub content_offset: usize,
    pub content_length: usize,
    pub element_subtype: HkxType,
    pub ctype: String,
}

#[derive(Debug, Clone)]
pub struct HkxFile {
    class_version: u32,
    contents_version: String,
    padding_size: usize,
    objects: Vec<HkxObject>,
    packfile: ParsedPackfile,
    source_bytes: Vec<u8>,
    /// Source-byte tracking for arrays in `objects`, populated by the reader
    /// when this file was loaded from a packfile. Empty for `from_tagxml`.
    /// Used by `patcher::patch_hkx` to overlay model edits onto the source
    /// bytes without re-serializing the whole packfile.
    array_sources: Vec<ArraySource>,
    /// Set when *any* mutation has been made (model edit OR byte overlay).
    /// Drives `is_dirty()` for callers that just want a "did anything
    /// change?" signal.
    dirty: bool,
    /// Set only when the in-memory `objects` have diverged from the
    /// source bytes (e.g. via `objects_mut`, `push_object`,
    /// `set_contents_version`). Distinct from `dirty` because byte-overlay
    /// patches (`apply_patch`) update `source_bytes` directly without
    /// touching `objects`, so for those cases `save()` can still echo
    /// `source_bytes` verbatim — only model-level edits need to route
    /// through the writer.
    model_dirty: bool,
}

impl HkxFile {
    pub fn read(data: &[u8]) -> HavokResult<Self> {
        if data.len() >= 8 && &data[4..8] == TAG0_MAGIC {
            return parse_tagfile(data)?.materialize_hkx();
        }
        if is_binary_tagfile_magic(data) {
            return read_tagfile2014(data);
        }
        Self::from_packfile(data, parse_packfile(data)?)
    }

    fn from_packfile(data: &[u8], packfile: ParsedPackfile) -> HavokResult<Self> {
        let mut registry = DescriptorRegistry::for_contents_version(&packfile.header.version_name);
        let ReadOutcome {
            objects,
            array_sources,
        } = read_objects_with_sources(data, &packfile, &mut registry)?;
        Ok(Self {
            class_version: packfile.header.version,
            contents_version: packfile.header.version_name.clone(),
            padding_size: packfile.header.padding_size,
            objects,
            packfile,
            source_bytes: data.to_vec(),
            array_sources,
            dirty: false,
            model_dirty: false,
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn from_tagxml(
        class_version: u32,
        contents_version: impl Into<String>,
        objects: Vec<HkxObject>,
    ) -> Self {
        let contents_version = contents_version.into();
        Self {
            class_version,
            contents_version: contents_version.clone(),
            padding_size: 0,
            objects,
            packfile: empty_packfile(class_version, contents_version),
            source_bytes: Vec::new(),
            array_sources: Vec::new(),
            // No source bytes at all — `save()` *must* run the writer.
            dirty: false,
            model_dirty: true,
        }
    }

    pub fn array_sources(&self) -> &[ArraySource] {
        &self.array_sources
    }

    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }

    pub fn packfile(&self) -> &ParsedPackfile {
        &self.packfile
    }

    pub fn class_version(&self) -> u32 {
        self.class_version
    }

    pub fn contents_version(&self) -> &str {
        &self.contents_version
    }

    pub fn set_contents_version(&mut self, contents_version: impl Into<String>) {
        self.contents_version = contents_version.into();
        self.dirty = true;
        self.model_dirty = true;
    }

    pub(crate) fn set_class_version(&mut self, class_version: u32) {
        self.class_version = class_version;
        self.dirty = true;
        self.model_dirty = true;
    }

    pub fn padding_size(&self) -> usize {
        self.padding_size
    }

    pub fn objects(&self) -> &[HkxObject] {
        &self.objects
    }

    pub fn objects_mut(&mut self) -> &mut [HkxObject] {
        self.dirty = true;
        self.model_dirty = true;
        &mut self.objects
    }

    pub fn push_object(&mut self, object: HkxObject) -> usize {
        let index = self.objects.len();
        self.objects.push(object);
        self.dirty = true;
        self.model_dirty = true;
        index
    }

    pub fn reorder_objects_remap_pointers(&mut self, new_order: Vec<usize>) {
        // new_order is a permutation: new_order[new_index] = old_index.
        // Build remap: remap[old_index] = new_index.
        let mut remap: Vec<Option<usize>> = vec![None; self.objects.len()];
        for (new_index, &old_index) in new_order.iter().enumerate() {
            remap[old_index] = Some(new_index);
        }

        let old_objects = std::mem::take(&mut self.objects);
        self.objects = new_order
            .iter()
            .map(|&old_index| old_objects[old_index].clone())
            .collect();

        for object in &mut self.objects {
            for member in &mut object.members {
                remap_value_pointers(&mut member.value, &remap);
            }
        }
        self.dirty = true;
        self.model_dirty = true;
    }

    pub fn retain_objects_remap_pointers(
        &mut self,
        mut keep: impl FnMut(usize, &HkxObject) -> bool,
    ) {
        let mut remap = vec![None; self.objects.len()];
        let mut next_index = 0;
        for (index, object) in self.objects.iter().enumerate() {
            if keep(index, object) {
                remap[index] = Some(next_index);
                next_index += 1;
            }
        }
        if next_index == self.objects.len() {
            return;
        }

        let old_objects = std::mem::take(&mut self.objects);
        self.objects = old_objects
            .into_iter()
            .enumerate()
            .filter_map(|(index, object)| remap[index].map(|_| object))
            .collect();
        for object in &mut self.objects {
            for member in &mut object.members {
                remap_value_pointers(&mut member.value, &remap);
            }
        }
        self.dirty = true;
        self.model_dirty = true;
    }

    /// Return the raw source bytes without re-serialising the model.
    ///
    /// # Panics
    /// Panics in debug builds if the model has been mutated since load.
    /// Callers that need to persist mutations should use [`Self::save`] instead.
    pub fn save_unchanged(&self) -> Vec<u8> {
        debug_assert!(
            !self.dirty,
            "save_unchanged called on a dirty HkxFile; use save() to persist mutations"
        );
        self.source_bytes.clone()
    }

    /// Serialize this HkxFile back to packfile bytes.
    ///
    /// Three branches mirror Python `py_creation_lib/python/creation_lib/hkxpack.save_hkx`:
    ///   1. No source bytes (e.g. from_tagxml) → run the writer.
    ///   2. Model is unchanged → return source bytes verbatim.
    ///      `apply_patch` falls in this bucket: it byte-overlays
    ///      `source_bytes` directly, so the bytes already reflect the edit
    ///      and the writer would only round-trip noise.
    ///   3. Model has been mutated (`objects_mut`, `set_contents_version`,
    ///      `push_object`, `reorder_objects_remap_pointers`,
    ///      `retain_objects_remap_pointers`) → run the writer over the
    ///      current object list.
    ///
    /// The byte-level patcher fast path (`patcher::patch_hkx`) is not wired
    /// into `save`: branch 3 always runs the full writer rather than trying
    /// a partial overlay first.
    pub fn save(&self) -> Vec<u8> {
        if self.source_bytes.is_empty() {
            let mut registry = DescriptorRegistry::for_contents_version(self.contents_version());
            return write_hkx(self, &mut registry);
        }
        if !self.model_dirty {
            return self.source_bytes.clone();
        }
        let mut registry = DescriptorRegistry::for_contents_version(self.contents_version());
        write_hkx(self, &mut registry)
    }

    pub fn apply_patch(&mut self, patch: PatchRange) -> HavokResult<()> {
        if apply_patch_range(&mut self.source_bytes, patch)? {
            self.dirty = true;
        }
        Ok(())
    }
}

fn remap_value_pointers(value: &mut HkxValue, remap: &[Option<usize>]) {
    match value {
        HkxValue::Pointer(Some(index)) => {
            if let Some(mapped) = remap.get(*index) {
                *value = HkxValue::Pointer(*mapped);
            }
        }
        HkxValue::Array(values) => {
            values.retain(|value| !is_removed_pointer(value, remap));
            for value in values {
                remap_value_pointers(value, remap);
            }
        }
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => {
            for member in members {
                remap_value_pointers(&mut member.value, remap);
            }
        }
        _ => {}
    }
}

fn is_removed_pointer(value: &HkxValue, remap: &[Option<usize>]) -> bool {
    match value {
        HkxValue::Pointer(Some(index)) => remap.get(*index) == Some(&None),
        _ => false,
    }
}

pub fn read_packfile(data: &[u8]) -> HavokResult<HkxFile> {
    HkxFile::read(data)
}

fn empty_packfile(class_version: u32, contents_version: String) -> ParsedPackfile {
    ParsedPackfile {
        header: PackfileHeader {
            version: class_version,
            version_name: contents_version,
            padding_size: 0,
            pointer_size: 8,
            section_header_size: 0,
            contents_section_index: 0,
            contents_section_offset: 0,
            contents_class_name_section_index: 0,
            contents_class_name_section_offset: 0,
        },
        sections: vec![SectionHeader {
            name: "__data__".to_string(),
            offset: 0,
            data1: 0,
            data2: 0,
            data3: 0,
            exports: 0,
            imports: 0,
            end: 0,
        }],
        classnames: Vec::new(),
        local_fixups: Vec::new(),
        global_fixups: Vec::new(),
        virtual_fixups: Vec::new(),
    }
}
