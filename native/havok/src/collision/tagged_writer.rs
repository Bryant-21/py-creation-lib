//! Havok 2019 tagged binary format (TAG0) writer.
//!
//! Assembles a complete tagged binary blob from pre-built components.
//! Domain-agnostic — knows about container structure, not physics.
//!
//! Direct port of `py_creation_lib/python/creation_lib/havok/tagged_writer.py`.

/// One ITEM entry in the tagged format index.
#[derive(Debug, Clone, PartialEq)]
pub struct TaggedItem {
    /// 0x10 = VAR0 (single object), 0x20 = VARN (array)
    pub kind: u32,
    /// Index into TYPE section's type table
    pub type_idx: u32,
    /// Byte offset into DATA section
    pub data_offset: u32,
    /// Element count (1 for VAR0, N for VARN)
    pub count: u32,
}

impl TaggedItem {
    /// Compute packed u32: (kind << 24) | (type_idx & 0x00FF_FFFF)
    pub fn packed(&self) -> u32 {
        (self.kind << 24) | (self.type_idx & 0x00FF_FFFF)
    }
}

/// One PTCH entry — pointer fixup within the DATA section.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchEntry {
    /// Source: ITEM index or DATA byte offset
    pub src: u32,
    /// Patch type flag
    pub flag: u32,
    /// Target: ITEM index or DATA byte offset
    pub target: u32,
}

/// Builds a Havok 2019 tagged format binary blob (TAG0 container).
///
/// Direct port of `TaggedBlobBuilder` in `py_creation_lib/python/creation_lib/havok/tagged_writer.py`.
pub struct TaggedBlobBuilder {
    sdk_version: Vec<u8>,
    type_section: Vec<u8>,
    data: Vec<u8>,
    items: Vec<TaggedItem>,
    patches: Vec<PatchEntry>,
    ptch_trailer: Vec<u8>,
}

impl TaggedBlobBuilder {
    pub fn new(sdk_version: &str) -> Self {
        Self {
            sdk_version: sdk_version.as_bytes().to_vec(),
            type_section: Vec::new(),
            data: Vec::new(),
            items: Vec::new(),
            patches: Vec::new(),
            ptch_trailer: Vec::new(),
        }
    }

    pub fn set_type_section(&mut self, type_bytes: Vec<u8>) {
        self.type_section = type_bytes;
    }

    pub fn set_data(&mut self, data_bytes: Vec<u8>) {
        self.data = data_bytes;
    }

    pub fn set_items(&mut self, items: Vec<TaggedItem>) {
        self.items = items;
    }

    pub fn set_patches(&mut self, patches: Vec<PatchEntry>) {
        self.patches = patches;
    }

    /// Set raw trailing bytes appended after PTCH entries (before alignment pad).
    pub fn set_ptch_trailer(&mut self, trailer: Vec<u8>) {
        self.ptch_trailer = trailer;
    }

    /// Assemble and return the complete tagged binary blob.
    pub fn build(&self) -> Vec<u8> {
        // SDKV section (leaf)
        let sdkv = leaf_tag(b"SDKV", &self.sdk_version);

        // DATA section (leaf)
        let data_sec = leaf_tag(b"DATA", &self.data);

        // TYPE section (container — already includes its own sub-tags; pass through verbatim)
        let type_sec = &self.type_section;

        // ITEM content: each entry 12 bytes, little-endian
        let mut item_content = Vec::new();
        for item in &self.items {
            item_content.extend_from_slice(&item.packed().to_le_bytes());
            item_content.extend_from_slice(&item.data_offset.to_le_bytes());
            item_content.extend_from_slice(&item.count.to_le_bytes());
        }
        let item_sec = leaf_tag(b"ITEM", &item_content);

        // PTCH content (with optional trailer, padded to 8-byte alignment)
        let mut ptch_content = Vec::new();
        for p in &self.patches {
            ptch_content.extend_from_slice(&p.src.to_le_bytes());
            ptch_content.extend_from_slice(&p.flag.to_le_bytes());
            ptch_content.extend_from_slice(&p.target.to_le_bytes());
        }
        ptch_content.extend_from_slice(&self.ptch_trailer);
        let pad_needed = ptch_content.len().wrapping_neg() % 8;
        ptch_content.resize(ptch_content.len() + pad_needed, 0u8);
        let ptch_sec = leaf_tag(b"PTCH", &ptch_content);

        // INDX container wraps ITEM + PTCH
        let indx_inner: Vec<u8> = [item_sec.as_slice(), ptch_sec.as_slice()].concat();
        let indx_size = 8 + indx_inner.len();
        let indx_sec: Vec<u8> = [
            container_start(b"INDX", indx_size).as_slice(),
            indx_inner.as_slice(),
        ]
        .concat();

        // Assemble body: SDKV + DATA + TYPE + INDX
        let body: Vec<u8> = [
            sdkv.as_slice(),
            data_sec.as_slice(),
            type_sec.as_slice(),
            indx_sec.as_slice(),
        ]
        .concat();

        // TAG0 container wraps everything
        let total_size = 8 + body.len();
        let tag0_header = container_start(b"TAG0", total_size);
        [tag0_header.as_slice(), body.as_slice()].concat()
    }
}

// ---------------------------------------------------------------------------
// Internal section-assembly helpers
// ---------------------------------------------------------------------------

/// Build a leaf section: 0x40 type byte, size includes 8-byte header.
fn leaf_tag(name: &[u8; 4], content: &[u8]) -> Vec<u8> {
    let size = 8 + content.len();
    let header = ((0x40u32 << 24) | (size as u32 & 0x00FF_FFFF)).to_be_bytes();
    let mut out = Vec::with_capacity(8 + content.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(name);
    out.extend_from_slice(content);
    out
}

/// Build a container header: 0x00 type byte, size spans all children.
fn container_start(name: &[u8; 4], total_size: usize) -> Vec<u8> {
    let header = ((0x00u32 << 24) | (total_size as u32 & 0x00FF_FFFF)).to_be_bytes();
    let mut out = Vec::with_capacity(8);
    out.extend_from_slice(&header);
    out.extend_from_slice(name);
    out
}
