use crate::{encoding, executables::text, invalid, relative_path, saves::fold};
use pyo3::prelude::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn plugin_name(value: &str) -> PyResult<String> {
    let value = relative_path(value, false)?;
    if value.contains('/')
        || ![".esp", ".esm", ".esl"]
            .iter()
            .any(|suffix| fold(&value).ends_with(suffix))
    {
        return Err(invalid(
            "Plugin names must be root-level ESP, ESM or ESL filenames.",
        ));
    }
    Ok(value)
}
fn truth(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::String(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
        Value::Number(v) => v.as_f64().is_some_and(|v| v != 0.),
    }
}
pub fn is_light(plugin: &Value) -> bool {
    truth(&plugin["header"]["flags"]["esl"]) || fold(text(plugin, "name")).ends_with(".esl")
}
pub fn is_master(plugin: &Value) -> bool {
    truth(&plugin["header"]["flags"]["esm"])
        || [".esm", ".esl"]
            .iter()
            .any(|suffix| fold(text(plugin, "name")).ends_with(suffix))
}
pub fn snapshot(files: &Value, name: &str) -> String {
    files
        .as_object()
        .into_iter()
        .flat_map(|v| v.iter())
        .find(|(key, _)| fold(key) == name)
        .and_then(|(_, v)| v.as_str())
        .unwrap_or("")
        .into()
}
pub fn lines(value: &str, stars: bool) -> PyResult<Vec<(String, bool)>> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in value.lines() {
        let raw = raw.trim().trim_start_matches('\u{feff}');
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let active = raw.starts_with('*');
        let name = plugin_name(if active { &raw[1..] } else { raw })?;
        if !seen.insert(fold(&name)) {
            return Err(invalid(format!("Duplicate plugin in local state: {name}")));
        }
        result.push((name, !stars || active));
    }
    Ok(result)
}
pub fn locked_order(value: &str) -> PyResult<Vec<(String, usize)>> {
    if value.len() > 2 * 1024 * 1024 {
        return Err(invalid("Locked plugin order exceeds its 2 MiB limit."));
    }
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in value.lines() {
        let raw = raw.trim().trim_start_matches('\u{feff}');
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let fields = raw.split('|').collect::<Vec<_>>();
        if fields.len() != 2
            || fields[1].trim().is_empty()
            || !fields[1].trim().bytes().all(|v| v.is_ascii_digit())
        {
            return Err(invalid(
                "Locked plugin order needs filename|nonnegative priority lines.",
            ));
        }
        let name = plugin_name(fields[0].trim())?;
        let position = fields[1].trim().parse::<usize>().unwrap_or(usize::MAX);
        if !seen.insert(fold(&name)) || position > 9999 {
            return Err(invalid(
                "Locked plugin order contains a duplicate name or out-of-range priority.",
            ));
        }
        result.push((name, position));
        if result.len() > 10_000 {
            return Err(invalid(
                "Locked plugin order exceeds its 10,000-name limit.",
            ));
        }
    }
    Ok(result)
}
fn strings(value: &Value) -> PyResult<Vec<String>> {
    value
        .as_array()
        .ok_or_else(|| invalid("Expected plugin names."))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid("Expected a plugin name."))
        })
        .collect()
}
pub fn review(catalog: &Value, order: &Value, enabled: &Value, locked: &Value) -> PyResult<Value> {
    let plugins = catalog["plugins"]
        .as_array()
        .ok_or_else(|| invalid("Invalid plugin inventory."))?;
    let by_name = plugins
        .iter()
        .map(|v| (fold(text(v, "name")), v))
        .collect::<BTreeMap<_, _>>();
    let order = if order.is_null() {
        plugins
            .iter()
            .map(|v| text(v, "name").to_string())
            .collect()
    } else {
        strings(order)?
    };
    let order = order
        .iter()
        .map(|v| plugin_name(v))
        .collect::<PyResult<Vec<_>>>()?;
    let lowered = order.iter().map(|v| fold(v)).collect::<Vec<_>>();
    let names = lowered.iter().cloned().collect::<BTreeSet<_>>();
    if lowered.len() != names.len() || names != by_name.keys().cloned().collect() {
        return Err(invalid(
            "Plugin order must contain every inventory name exactly once.",
        ));
    }
    let active = if enabled.is_null() {
        plugins
            .iter()
            .filter(|v| truth(&v["enabled"]))
            .map(|v| fold(text(v, "name")))
            .collect::<BTreeSet<_>>()
    } else {
        strings(enabled)?.iter().map(|v| fold(v)).collect()
    };
    if !active.is_subset(&names) {
        return Err(invalid(
            "Activation contains a plugin outside this inventory.",
        ));
    }
    let positions = lowered
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let primary = plugins
        .iter()
        .filter(|v| truth(&v["forced"]))
        .map(|v| fold(text(v, "name")))
        .collect::<Vec<_>>();
    if !lowered.starts_with(&primary) || primary.iter().any(|v| !active.contains(v)) {
        return Err(invalid(
            "Required game masters must stay active and first in their defined order.",
        ));
    }
    let locks = if locked.is_null() {
        &catalog["locked"]
    } else {
        locked
    };
    let mut raw = String::new();
    for pair in locks
        .as_array()
        .ok_or_else(|| invalid("Invalid plugin locks."))?
    {
        raw.push_str(&format!(
            "{}|{}\n",
            pair[0].as_str().unwrap_or(""),
            pair[1]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| pair[1].to_string())
        ));
    }
    let locks = locked_order(&raw)?;
    let mut occupied = BTreeSet::new();
    for (name, position) in &locks {
        let key = fold(name);
        if primary.contains(&key) {
            return Err(invalid(
                "Required game masters already have fixed positions and cannot be locked.",
            ));
        }
        if by_name.contains_key(&key)
            && (!occupied.insert(*position) || positions[&key] != *position)
        {
            return Err(invalid(format!(
                "Locked plugin {name} must remain at priority {position}. Unlock it to change its position."
            )));
        }
    }
    let mut full = 0;
    let mut light = 0;
    let game = text(catalog, "game");
    for name in &lowered {
        if !active.contains(name) {
            continue;
        }
        let plugin = by_name[name];
        let actual = text(plugin, "name");
        if text(plugin, "path").is_empty() {
            return Err(invalid(format!("Active plugin is missing: {actual}")));
        }
        if truth(&plugin["ghosted"]) {
            return Err(invalid(format!(
                "Active plugin is ghosted: {actual}. Unghost its owned file before activation."
            )));
        }
        if plugin["masters"].is_null() || plugin["header"].is_null() {
            return Err(invalid(
                "Inspect plugin details before applying activation or order; dependencies are not known.",
            ));
        }
        if truth(&plugin["header"]["flags"]["medium"])
            || truth(&plugin["header"]["flags"]["update"])
            || game == "starfield"
        {
            return Err(invalid(
                "Starfield medium/update plugin ordering needs a game adapter; edits are blocked.",
            ));
        }
        if is_light(plugin) {
            light += 1;
        } else {
            full += 1;
        }
        for master in strings(&plugin["masters"])? {
            let key = fold(&plugin_name(&master)?);
            if !active.contains(&key) {
                return Err(invalid(format!(
                    "{actual} requires active master {master}."
                )));
            }
            if positions[&key] >= positions[name] {
                return Err(invalid(format!(
                    "{master} must load before {actual}; dependency cycles are rejected."
                )));
            }
        }
        if is_master(plugin) {
            for preceding in &lowered[..positions[name]] {
                let other = by_name[preceding];
                if active.contains(preceding) && truth(&other["header"]) && !is_master(other) {
                    return Err(invalid(
                        "Master plugins must load before regular plugins in this adapter.",
                    ));
                }
            }
        }
    }
    if full
        > if ["fo4", "skyrimse"].contains(&game) {
            254
        } else {
            255
        }
        || light > 4096
    {
        return Err(invalid(
            "Plugin activation exceeds the adapter's full/light index limits.",
        ));
    }
    if light > 0 && !["fo4", "skyrimse"].contains(&game) {
        return Err(invalid("This game adapter does not support light plugins."));
    }
    Ok(json!({"order":order,"enabled":active,"locked":locks}))
}
#[pyfunction]
fn plugin_state_name(value: &str) -> PyResult<String> {
    plugin_name(value)
}
#[pyfunction]
fn plugin_state_flag(value: &str, light: bool) -> PyResult<bool> {
    let value = encoding::parse(value)?;
    Ok(if light {
        is_light(&value)
    } else {
        is_master(&value)
    })
}
#[pyfunction]
fn plugin_state_snapshot(files: &str, name: &str) -> PyResult<String> {
    Ok(snapshot(&encoding::parse(files)?, name))
}
#[pyfunction]
fn plugin_state_lines(value: &str, stars: bool) -> PyResult<Vec<(String, bool)>> {
    lines(value, stars)
}
#[pyfunction]
fn plugin_state_locks(value: &str) -> PyResult<Vec<(String, usize)>> {
    locked_order(value)
}
#[pyfunction]
fn plugin_state_review(
    catalog: &str,
    order: &str,
    enabled: &str,
    locked: &str,
) -> PyResult<String> {
    Ok(review(
        &encoding::parse(catalog)?,
        &encoding::parse(order)?,
        &encoding::parse(enabled)?,
        &encoding::parse(locked)?,
    )?
    .to_string())
}
#[pyfunction]
fn plugin_state_indices(
    catalog: &str,
    order: Vec<String>,
    enabled: Vec<String>,
) -> PyResult<BTreeMap<String, String>> {
    let catalog = encoding::parse(catalog)?;
    let plugins = catalog["plugins"]
        .as_array()
        .ok_or_else(|| invalid("Invalid plugin inventory."))?;
    let by_name = plugins
        .iter()
        .map(|v| (fold(text(v, "name")), v))
        .collect::<BTreeMap<_, _>>();
    let active = enabled.iter().map(|v| fold(v)).collect::<BTreeSet<_>>();
    let game = text(&catalog, "game");
    let known = game != "starfield"
        && active.iter().all(|v| {
            by_name
                .get(v)
                .is_some_and(|p| !p["header"].is_null() && !text(p, "path").is_empty())
        });
    let mut result = BTreeMap::new();
    let mut full = 0;
    let mut light = 0;
    for name in order {
        let key = fold(&name);
        let label = if !active.contains(&key) {
            "—".into()
        } else if !known {
            "?".into()
        } else if is_light(by_name[&key]) {
            let label = if ["fo4", "skyrimse"].contains(&game) && light < 4096 {
                format!("FE:{light:03X}")
            } else {
                "Invalid".into()
            };
            light += 1;
            label
        } else {
            let label = if full
                < if ["fo4", "skyrimse"].contains(&game) {
                    254
                } else {
                    255
                } {
                format!("{full:02X}")
            } else {
                "Invalid".into()
            };
            full += 1;
            label
        };
        result.insert(key, label);
    }
    Ok(result)
}
fn validate_dependency_order(plugins: &[Value], order: &[String]) -> PyResult<()> {
    let positions = order
        .iter()
        .enumerate()
        .map(|(i, name)| (fold(name), i))
        .collect::<BTreeMap<_, _>>();
    let mut regular = None;
    let by_name = plugins
        .iter()
        .map(|p| (fold(text(p, "name")), p))
        .collect::<BTreeMap<_, _>>();
    for name in order {
        let key = fold(name);
        let p = by_name
            .get(&key)
            .ok_or_else(|| invalid("Plugin inventory changed. Refresh plugins."))?;
        if p["header"].is_null() || text(p, "path").is_empty() || truth(&p["ghosted"]) {
            continue;
        }
        if is_master(p) {
            if let Some(earlier) = regular {
                return Err(invalid(format!(
                    "Master {name} must load before regular plugin {earlier}."
                )));
            }
        } else {
            regular = Some(name);
        }
        for master in p["masters"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if positions
                .get(&fold(master))
                .is_some_and(|position| *position >= positions[&key])
            {
                return Err(invalid(format!(
                    "{name} depends on {master}, which must load first."
                )));
            }
        }
    }
    Ok(())
}

#[pyfunction]
pub fn plugin_state_default_order(catalog: &str) -> PyResult<Vec<String>> {
    let catalog = encoding::parse(catalog)?;
    let plugins = catalog["plugins"]
        .as_array()
        .ok_or_else(|| invalid("Invalid plugin inventory."))?;
    let names = plugins
        .iter()
        .enumerate()
        .map(|(i, p)| (fold(text(p, "name")), i))
        .collect::<BTreeMap<_, _>>();
    let mut incoming = vec![0usize; plugins.len()];
    let mut children = vec![Vec::new(); plugins.len()];
    for (i, p) in plugins.iter().enumerate() {
        if !text(p, "path").is_empty()
            && !truth(&p["ghosted"])
            && (p["header"].is_null() || p["masters"].is_null())
        {
            return Err(invalid(
                "Inspect plugin headers before arranging library order.",
            ));
        }
        let masters = p["masters"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(fold)
            .collect::<BTreeSet<_>>();
        for master in masters {
            if let Some(&parent) = names.get(&master) {
                incoming[i] += 1;
                children[parent].push(i);
            }
        }
    }
    let rank = |i: usize| {
        let p = &plugins[i];
        (
            if truth(&p["forced"]) {
                0
            } else if is_master(p) {
                1
            } else {
                2
            },
            if truth(&p["forced"]) {
                i as u64
            } else {
                p["library_priority"].as_u64().unwrap_or(0)
            },
            fold(text(p, "name")),
            i,
        )
    };
    let mut ready = (0..plugins.len())
        .filter(|&i| incoming[i] == 0)
        .map(rank)
        .collect::<BTreeSet<_>>();
    let mut order = Vec::new();
    while let Some((_, _, _, i)) = ready.pop_first() {
        order.push(text(&plugins[i], "name").to_string());
        for &child in &children[i] {
            incoming[child] -= 1;
            if incoming[child] == 0 {
                ready.insert(rank(child));
            }
        }
    }
    if order.len() != plugins.len() {
        return Err(invalid(
            "Plugin dependencies contain a cycle; library order cannot be resolved.",
        ));
    }
    validate_dependency_order(plugins, &order)?;
    for lock in catalog["locked"].as_array().into_iter().flatten() {
        if let Some(position) = order
            .iter()
            .position(|name| fold(name) == fold(lock[0].as_str().unwrap_or("")))
        {
            if Some(position as u64) != lock[1].as_u64() {
                return Err(invalid(
                    "Locked priorities conflict with library order. Clear draft locks first.",
                ));
            }
        }
    }
    Ok(order)
}

#[pyfunction]
fn plugin_state_move(
    catalog: &str,
    order: Vec<String>,
    selected: Vec<String>,
    destination: i64,
    locks: Vec<(String, usize)>,
) -> PyResult<Vec<String>> {
    let catalog = encoding::parse(catalog)?;
    let selected = selected.into_iter().collect::<BTreeSet<_>>();
    let moving = order
        .iter()
        .filter(|v| selected.contains(*v))
        .cloned()
        .collect::<Vec<_>>();
    if moving.is_empty() || moving.len() != selected.len() {
        return Err(invalid(
            "Selected plugins changed; refresh this draft before moving them.",
        ));
    }
    let names = order
        .iter()
        .map(|v| (fold(v), v.clone()))
        .collect::<BTreeMap<_, _>>();
    let plugins = catalog["plugins"]
        .as_array()
        .ok_or_else(|| invalid("Invalid plugin inventory."))?;
    let mut occupied = plugins
        .iter()
        .enumerate()
        .filter(|(_, v)| truth(&v["forced"]))
        .map(|(i, v)| (i, text(v, "name").to_string()))
        .collect::<BTreeMap<_, _>>();
    for (name, position) in locks {
        let key = fold(&name);
        if !names.contains_key(&key) {
            continue;
        }
        if position >= order.len() || occupied.contains_key(&position) {
            return Err(invalid(
                "Present plugin locks conflict or exceed this inventory. Review locks first.",
            ));
        }
        occupied.insert(position, names[&key].clone());
    }
    let fixed = occupied.values().map(|v| fold(v)).collect::<BTreeSet<_>>();
    if moving.iter().any(|v| fixed.contains(&fold(v))) {
        return Err(invalid(
            "Unlock selected priorities before moving them. Required game masters are fixed.",
        ));
    }
    let mut remaining = order
        .iter()
        .filter(|v| !selected.contains(*v))
        .cloned()
        .collect::<Vec<_>>();
    let mut destination = destination.max(0).min(remaining.len() as i64) as usize;
    let primary = plugins.iter().filter(|v| truth(&v["forced"])).count();
    if destination < primary {
        return Err(invalid("Required game masters must remain first."));
    }
    if occupied.contains_key(&destination) {
        let first = order.iter().position(|v| selected.contains(v)).unwrap();
        let direction = if destination < first { -1 } else { 1 };
        let choices = (primary..order.len())
            .filter(|i| !occupied.contains_key(i))
            .collect::<Vec<_>>();
        let preferred = choices
            .iter()
            .copied()
            .filter(|i| (*i as i64 - destination as i64) * direction > 0)
            .collect::<Vec<_>>();
        destination = *(if preferred.is_empty() {
            &choices
        } else {
            &preferred
        })
        .iter()
        .min_by_key(|i| i.abs_diff(destination))
        .ok_or_else(|| invalid("No movable plugin priority is available."))?;
    }
    let destination = destination.min(remaining.len());
    remaining.splice(destination..destination, moving);
    let mut free = remaining.into_iter().filter(|v| !fixed.contains(&fold(v)));
    let candidate: Vec<String> = (0..order.len())
        .map(|index| {
            occupied
                .get(&index)
                .cloned()
                .or_else(|| free.next())
                .ok_or_else(|| invalid("Plugin priorities no longer match this inventory."))
        })
        .collect::<PyResult<_>>()?;
    validate_dependency_order(plugins, &candidate)?;
    Ok(candidate)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(plugin_state_name, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_state_flag, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_state_snapshot, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_state_lines, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_state_locks, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_state_review, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_state_indices, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_state_move, m)?)?;
    m.add_function(wrap_pyfunction!(plugin_state_default_order, m)?)?;
    Ok(())
}
