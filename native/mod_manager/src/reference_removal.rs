use crate::{
    check_cancel,
    database::{Database, error, records},
    encoding,
    executables::text,
    invalid, organization,
};
use pyo3::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn state(
    db: &Database,
    profile: &str,
    selected: &Value,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<Value> {
    let profile_row = records(
        &*db.connection()?,
        "SELECT id,name,source FROM profiles WHERE id=?",
        [profile],
    )?
    .into_iter()
    .next();
    let before = organization::entries(&*db.connection()?, profile)?;
    let selected: BTreeSet<_> = selected
        .as_array()
        .ok_or_else(|| invalid("Select referenced mods or separators in one local profile."))?
        .iter()
        .map(|v| {
            v.as_str()
                .ok_or_else(|| invalid("Selected entries no longer belong to this profile."))
        })
        .collect::<PyResult<_>>()?;
    if profile_row.is_none() || selected.is_empty() || selected.len() > 100000 {
        return Err(invalid(
            "Select referenced mods or separators in one local profile.",
        ));
    }
    let available: BTreeSet<_> = before.iter().map(|v| text(v, "mod_id")).collect();
    if !selected.is_subset(&available) {
        return Err(invalid(
            "Selected entries no longer belong to this profile.",
        ));
    }
    let mut mods = Vec::new();
    for row in &before {
        check_cancel(Some(cancel))?;
        if selected.contains(text(row, "mod_id")) {
            let record = records(
                &*db.connection()?,
                "SELECT id,name,source,kind FROM mods WHERE id=?",
                [text(row, "mod_id")],
            )?
            .into_iter()
            .next()
            .ok_or_else(|| invalid("Selected entries no longer belong to this profile."))?;
            if !matches!(text(&record, "kind"), "external" | "separator") {
                return Err(invalid(
                    "Metadata removal accepts references and separators. Owned mods use Remove owned mod with recovery.",
                ));
            }
            mods.push(record);
        }
    }
    Ok(json!([profile_row.unwrap(), before, mods]))
}

#[pyfunction]
fn reference_removal_review(
    db: &Database,
    profile: &str,
    selected: &str,
    cancel: &Bound<'_, PyAny>,
) -> PyResult<String> {
    check_cancel(Some(cancel))?;
    let value = state(db, profile, &encoding::parse(selected)?, cancel)?;
    check_cancel(Some(cancel))?;
    let mods = value[2].as_array().unwrap();
    Ok(json!({"profile_id":profile,"profile_name":value[0]["name"],"mod_ids":mods.iter().map(|v| &v["id"]).collect::<Vec<_>>(),"names":mods.iter().map(|v| &v["name"]).collect::<Vec<_>>(),"kinds":mods.iter().map(|v| &v["kind"]).collect::<Vec<_>>(),"expected":encoding::digest(&value,true,true)}).to_string())
}

#[pyfunction]
fn reference_removal_apply(
    db: &Database,
    library: &Bound<'_, PyAny>,
    plan: &str,
    cancel: &Bound<'_, PyAny>,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if library
        .py()
        .import("creation_lib.mod_manager.runtime")?
        .call_method1("active_sessions", (library.getattr("root")?,))?
        .is_truthy()?
    {
        return Err(invalid(
            "Close the recorded runtime before removing local profile entries.",
        ));
    }
    let plan = encoding::parse(plan)?;
    let profile = text(&plan, "profile_id");
    db.connection()?
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(error)?;
    let value = state(db, profile, &plan["mod_ids"], cancel)?;
    if encoding::digest(&value, true, true) != text(&plan, "expected") {
        return Err(invalid(
            "Profile entries or selected metadata changed. Review removal again.",
        ));
    }
    check_cancel(Some(cancel))?;
    for id in plan["mod_ids"].as_array().unwrap() {
        let id = id.as_str().unwrap();
        db.connection()?
            .execute(
                "INSERT OR IGNORE INTO reference_exclusions(profile_id,mod_id) VALUES (?,?)",
                params![profile, id],
            )
            .map_err(error)?;
        db.connection()?
            .execute(
                "DELETE FROM entries WHERE profile_id=? AND mod_id=?",
                params![profile, id],
            )
            .map_err(error)?;
    }
    let after = organization::entries(&*db.connection()?, profile)?;
    for (index, row) in after.iter().enumerate() {
        db.connection()?
            .execute(
                "UPDATE entries SET priority=? WHERE profile_id=? AND mod_id=?",
                params![index as i64, profile, text(row, "mod_id")],
            )
            .map_err(error)?;
    }
    let mut snapshot = value[1].as_array().unwrap().clone();
    for row in &mut snapshot {
        if let Some(record) = value[2]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == row["mod_id"])
        {
            row["removed_metadata"] = json!({"source":record["source"],"kind":record["kind"]});
        }
    }
    organization::history(&*db.connection()?, profile, &snapshot, "reference-removal")?;
    checkpoint.call0()?;
    check_cancel(Some(cancel))?;
    Ok(())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(reference_removal_review, m)?)?;
    m.add_function(wrap_pyfunction!(reference_removal_apply, m)?)?;
    Ok(())
}
