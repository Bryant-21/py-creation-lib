//! Process-wide memo caches for the AnimTextData emitters.
//!
//! Bucket writers run per subgraph, but their inputs are shared: ~13 subgraphs resolve to
//! the same core behavior, a race's subgraphs share one skeleton, and neighbours share most
//! of their clip closure. Re-parsing per subgraph dominates wall-clock on a full conversion.
//!
//! Two levels are cached:
//! * whole parsed packfiles for behavior files (a few hundred; the parse and string-index
//!   derivation are the expensive part);
//! * small derived values for animation clips (reference frame, annotations, stance
//!   bodies), since retaining ~10k parsed clip graphs would cost gigabytes.
//!
//! Keys are normalized on-disk paths, so different roots or letter case hit one entry.
//! [`clear_all`] frees everything once generation finishes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

use havok_native::hkx::{HkxFile, read_packfile};

/// Path-keyed memo. `T` is the (cheaply cloneable) cached value — an `Arc`, an
/// `Option<Arc<_>>`, or a small owned value.
///
/// Only the map lookup is locked; the value is built through a per-entry `OnceLock`, so
/// concurrent rayon workers building DIFFERENT files never serialize on each other, and
/// two workers racing the SAME file build it once.
pub(super) struct FileMemo<T: Clone> {
    entries: OnceLock<RwLock<HashMap<String, Arc<OnceLock<T>>>>>,
}

impl<T: Clone> FileMemo<T> {
    pub(super) const fn new() -> Self {
        Self {
            entries: OnceLock::new(),
        }
    }

    pub(super) fn get_or_init(&self, key: &str, build: impl FnOnce() -> T) -> T {
        let entries = self.entries.get_or_init(Default::default);
        // Hits dominate (that is the point of the memo) and the callers are heavily
        // threaded, so the hit path must not serialize: look the slot up under a shared
        // read lock and take the write lock only to insert a new one.
        let existing = entries
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned();
        let slot = match existing {
            Some(slot) => slot,
            None => entries
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .entry(key.to_owned())
                .or_insert_with(|| Arc::new(OnceLock::new()))
                .clone(),
        };
        slot.get_or_init(build).clone()
    }

    pub(super) fn clear(&self) {
        self.entries
            .get_or_init(Default::default)
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

/// Normalized cache key for an on-disk path (case- and separator-insensitive).
pub(super) fn path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

static BEHAVIORS: FileMemo<Option<Arc<HkxFile>>> = FileMemo::new();

/// Read + parse a behavior `.hkx`, memoized. `None` for an unreadable file or one that is
/// not a packfile — cached too, so a missing file is not re-probed per subgraph.
pub(super) fn behavior_packfile(path: &Path) -> Option<Arc<HkxFile>> {
    BEHAVIORS.get_or_init(&path_key(path), || {
        let data = std::fs::read(path).ok()?;
        read_packfile(&data).ok().map(Arc::new)
    })
}

/// Drop every memoized entry. Called once AnimTextData generation completes so the
/// retained packfiles and clip data do not follow the run into later conversion phases.
pub fn clear_all() {
    BEHAVIORS.clear();
    super::offsets::clear_clip_memo();
    super::speed::clear_behavior_memo();
    super::stance::clear_stance_memo();
}
