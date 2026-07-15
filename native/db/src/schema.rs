use std::collections::{HashMap, HashSet};

use once_cell::sync::Lazy;

use crate::error::{DbError, DbResult};

/// Whitelisted filter columns per table (passed into FTS queries).
pub static FILTER_COLUMNS: Lazy<HashMap<&'static str, HashSet<&'static str>>> = Lazy::new(|| {
    let mut m: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
    m.insert("records", ["record_type", "source"].into_iter().collect());
    m.insert(
        "pages",
        ["category", "parent_script", "function_name"]
            .into_iter()
            .collect(),
    );
    m.insert(
        "scripts",
        ["source", "category", "extends"].into_iter().collect(),
    );
    m.insert(
        "ext_records",
        ["mod_name", "record_type"].into_iter().collect(),
    );
    m.insert("ext_scripts", ["mod_name", "extends"].into_iter().collect());
    m.insert("ext_readmes", ["mod_name"].into_iter().collect());
    m.insert("behaviors", ["category", "source"].into_iter().collect());
    m.insert("nifs", ["category", "source"].into_iter().collect());
    m.insert(
        "havok_behaviors",
        ["category", "source"].into_iter().collect(),
    );
    m.insert(
        "havok_projects",
        ["category", "source"].into_iter().collect(),
    );
    m.insert(
        "havok_animations",
        ["category", "source", "actor"].into_iter().collect(),
    );
    m.insert("havok_skeletons", ["source"].into_iter().collect());
    m.insert(
        "havok_manifests",
        ["manifest_type", "source"].into_iter().collect(),
    );
    m.insert(
        "nif_material_textures",
        ["material_path", "material_type"].into_iter().collect(),
    );
    m
});

/// Whitelisted primary-key columns per table.
pub static ALLOWED_KEYS: Lazy<HashMap<&'static str, HashSet<&'static str>>> = Lazy::new(|| {
    let mut m: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
    m.insert("pages", ["filename"].into_iter().collect());
    m.insert("records", ["form_key"].into_iter().collect());
    m.insert("scripts", ["script_name"].into_iter().collect());
    m.insert("ext_records", ["form_key"].into_iter().collect());
    m.insert(
        "ext_scripts",
        ["script_name", "script_id"].into_iter().collect(),
    );
    m.insert("ext_readmes", ["mod_name"].into_iter().collect());
    m.insert("behaviors", ["id"].into_iter().collect());
    m.insert("nifs", ["id"].into_iter().collect());
    m.insert("havok_behaviors", ["id"].into_iter().collect());
    m.insert("havok_projects", ["id"].into_iter().collect());
    m.insert("havok_animations", ["id"].into_iter().collect());
    m.insert("havok_skeletons", ["id"].into_iter().collect());
    m.insert("havok_manifests", ["id"].into_iter().collect());
    m
});

/// Whitelisted GROUP BY columns per table.
pub static ALLOWED_GROUP: Lazy<HashMap<&'static str, HashSet<&'static str>>> = Lazy::new(|| {
    let mut m: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
    m.insert("pages", ["category"].into_iter().collect());
    m.insert("records", ["record_type", "source"].into_iter().collect());
    m.insert(
        "scripts",
        ["source", "category", "extends"].into_iter().collect(),
    );
    m.insert(
        "ext_records",
        ["mod_name", "record_type"].into_iter().collect(),
    );
    m.insert("ext_scripts", ["mod_name", "extends"].into_iter().collect());
    m.insert("behaviors", ["category", "source"].into_iter().collect());
    m.insert("nifs", ["category", "source"].into_iter().collect());
    m
});

pub fn check_filter(table: &str, column: &str) -> DbResult<()> {
    let set = FILTER_COLUMNS
        .get(table)
        .ok_or_else(|| DbError::BadFilter {
            table: table.into(),
            column: column.into(),
        })?;
    if !set.contains(column) {
        return Err(DbError::BadFilter {
            table: table.into(),
            column: column.into(),
        });
    }
    Ok(())
}

pub fn check_key(table: &str, column: &str) -> DbResult<()> {
    let set = ALLOWED_KEYS.get(table).ok_or_else(|| DbError::BadKey {
        table: table.into(),
        column: column.into(),
    })?;
    if !set.contains(column) {
        return Err(DbError::BadKey {
            table: table.into(),
            column: column.into(),
        });
    }
    Ok(())
}

pub fn check_group(table: &str, column: &str) -> DbResult<()> {
    let set = ALLOWED_GROUP.get(table).ok_or_else(|| DbError::BadGroup {
        table: table.into(),
        column: column.into(),
    })?;
    if !set.contains(column) {
        return Err(DbError::BadGroup {
            table: table.into(),
            column: column.into(),
        });
    }
    Ok(())
}

/// Validate that a bare identifier is safe to interpolate into SQL. We accept
/// only `[A-Za-z0-9_]+` plus an optional single `.` for qualified forms.
pub fn validate_ident(ident: &str) -> DbResult<()> {
    if ident.is_empty() {
        return Err(DbError::Schema("empty identifier".into()));
    }
    let ok = ident
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    if !ok {
        return Err(DbError::Schema(format!("invalid identifier '{ident}'")));
    }
    Ok(())
}
