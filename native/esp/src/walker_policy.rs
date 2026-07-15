//! Deserializes the walker policy JSON sent from Python.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WalkPolicy {
    pub(crate) follow_signatures: Option<Vec<String>>,
    pub(crate) asset_kinds: Option<Vec<String>>,
    pub(crate) reverse_passes: Vec<String>,
    pub(crate) behavior_bundle: bool,
    pub(crate) character_assets: bool,
    pub(crate) animation_lookup: bool,
    pub(crate) max_depth: Option<u32>,
    // Records with these signatures are reached and emitted, but their forward
    // refs are NOT enqueued — they act as graph terminals. Used for REGN/LAYR
    // in the cell-slice flow to avoid pulling in worldspace/location chains.
    #[serde(default)]
    pub(crate) terminal_signatures: Option<Vec<String>>,
}

impl Default for WalkPolicy {
    fn default() -> Self {
        Self {
            follow_signatures: None,
            asset_kinds: None,
            reverse_passes: Vec::new(),
            behavior_bundle: false,
            character_assets: false,
            animation_lookup: false,
            max_depth: None,
            terminal_signatures: None,
        }
    }
}
