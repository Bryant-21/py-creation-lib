//! Case-insensitive loose-file index built via a parallel directory walk.
//!
//! The renderer resolves NIF texture/material paths against on-disk "loose"
//! directories (extracted game data, mod folders). Those trees can hold ~1M
//! files, and a serial walk to build the lookup stalls the NIF-load thread.
//! [`FsIndex`] walks the tree in parallel (rayon over depth-2 subtrees) and the
//! PyO3 wrapper builds it with the GIL released, so the UI is never starved.
//!
//! Maps `rel_lower` — the path relative to the root, `/`-separated and
//! lowercased — to the absolute path on disk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use walkdir::WalkDir;

pub struct FsIndex {
    lookup: HashMap<String, String>,
}

impl FsIndex {
    #[must_use]
    pub fn build(root: &Path) -> Self {
        let root_str = root.to_string_lossy();
        // Absolute child paths are `<root><sep><rel>`; the separator is one
        // ASCII byte, so this is always a char boundary.
        let prefix_len = root_str.len() + 1;

        // Split the tree into balanced work units: each depth-2 directory is a
        // unit walked fully by one worker (so `textures/architecture`,
        // `textures/actors`, ... parallelize instead of all of `textures/`
        // landing on one thread). Files shallower than depth 2 are collected
        // directly.
        let mut units: Vec<PathBuf> = Vec::new();
        let mut shallow: Vec<PathBuf> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_file() {
                    shallow.push(e.path());
                } else if ft.is_dir() {
                    match std::fs::read_dir(e.path()) {
                        Ok(rd2) => {
                            for e2 in rd2.flatten() {
                                match e2.file_type() {
                                    Ok(ft2) if ft2.is_dir() => units.push(e2.path()),
                                    Ok(ft2) if ft2.is_file() => shallow.push(e2.path()),
                                    _ => {}
                                }
                            }
                        }
                        // Unreadable at depth 1 — fall back to walking it whole.
                        Err(_) => units.push(e.path()),
                    }
                }
            }
        }

        let per_unit: Vec<Vec<(String, String)>> = units
            .par_iter()
            .map(|unit| walk_unit(unit, prefix_len))
            .collect();

        let capacity: usize = per_unit.iter().map(Vec::len).sum::<usize>() + shallow.len();
        let mut lookup: HashMap<String, String> = HashMap::with_capacity(capacity);
        for part in per_unit {
            for (rel, abs) in part {
                lookup.insert(rel, abs);
            }
        }
        for p in shallow {
            if let Some((rel, abs)) = rel_entry(&p, prefix_len) {
                lookup.insert(rel, abs);
            }
        }

        Self { lookup }
    }

    #[must_use]
    pub fn resolve(&self, rel_path: &str) -> Option<String> {
        self.lookup.get(&normalize_key(rel_path)).cloned()
    }

    #[must_use]
    pub fn contains(&self, rel_path: &str) -> bool {
        self.lookup.contains_key(&normalize_key(rel_path))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lookup.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lookup.is_empty()
    }

    #[must_use]
    pub fn list(&self, prefix: &str, suffix: &str) -> Vec<String> {
        let prefix = normalize_key(prefix);
        let suffix = suffix.to_lowercase();
        self.lookup
            .keys()
            .filter(|k| {
                (prefix.is_empty() || k.starts_with(&prefix))
                    && (suffix.is_empty() || k.ends_with(&suffix))
            })
            .cloned()
            .collect()
    }
}

fn normalize_key(rel_path: &str) -> String {
    rel_path.replace('\\', "/").to_lowercase()
}

fn rel_entry(abs: &Path, prefix_len: usize) -> Option<(String, String)> {
    let abs_str = abs.to_string_lossy();
    if abs_str.len() <= prefix_len {
        return None;
    }
    let rel = abs_str[prefix_len..].replace('\\', "/").to_lowercase();
    Some((rel, abs_str.into_owned()))
}

fn walk_unit(unit: &Path, prefix_len: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in WalkDir::new(unit).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            if let Some(pair) = rel_entry(entry.path(), prefix_len) {
                out.push(pair);
            }
        }
    }
    out
}
