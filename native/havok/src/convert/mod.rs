mod corpus;
mod corpus_generated;
pub mod fo76;
pub mod hooks;
pub mod manager;
pub mod ops;
pub mod templates;
pub mod version;

use crate::error::HavokError;

pub use corpus::{NativePatchCorpusManifest, native_patch_corpus_manifest};
pub use hooks::{ConversionContext, CustomHookRegistry};
pub use manager::{PatchDirection, PatchManager, PatchRoute, PatchStep};
pub use ops::{ClassVersion, Patch, PatchOperation, PatchValue};
pub use templates::fo4_weapon_psd_object_template;
pub use version::{
    HavokVersion, all_versions, detect_version_id, get_version, get_version_by_name,
    get_version_chain, parse_target_version,
};

pub fn route_name(source: u8, target: u8, source_format: &str) -> &'static str {
    if source == target {
        "noop"
    } else if source_format == "tagfile" && source == 56 && target == 53 {
        "tag0-fo76-to-fo4"
    } else if source_format == "packfile" {
        "packfile-version-patch-chain"
    } else {
        "tagfile-version-patch-chain"
    }
}

pub fn conversion_not_implemented(source: u8, target: u8, source_format: &str) -> HavokError {
    let route = route_name(source, target, source_format).to_string();
    HavokError::ConversionNotImplemented {
        source_version: source,
        target_version: target,
        route,
        reason: "Havok semantic rewrite and serialization for changed conversion output has not been ported to Rust".to_string(),
    }
}
