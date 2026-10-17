use crate::{encoding, invalid, relative_path};
use pyo3::prelude::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use unicode_casefold::UnicodeCaseFold;

const TYPES: &[&str] = &[
    "Required",
    "Optional",
    "Recommended",
    "NotUsable",
    "CouldBeUsable",
];
const GROUPS: &[&str] = &[
    "SelectAny",
    "SelectAll",
    "SelectExactlyOne",
    "SelectAtMostOne",
    "SelectAtLeastOne",
];
const DEPS: &[&str] = &[
    "dependencies",
    "fileDependency",
    "flagDependency",
    "gameDependency",
    "fommDependency",
    "foseDependency",
    "nvseDependency",
    "skseDependency",
    "f4seDependency",
];

fn tag(node: &Bound<'_, PyAny>) -> PyResult<String> {
    node.getattr("tag")?.extract()
}
fn get(node: &Bound<'_, PyAny>, key: &str, default: &str) -> PyResult<String> {
    node.call_method1("get", (key, default))?.extract()
}
fn find<'py>(node: &Bound<'py, PyAny>, key: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
    let child = node.call_method1("find", (key,))?;
    Ok(if child.is_none() { None } else { Some(child) })
}
fn children<'py>(node: &Bound<'py, PyAny>) -> PyResult<Vec<Bound<'py, PyAny>>> {
    node.try_iter()?.collect()
}
fn required(node: &Bound<'_, PyAny>, key: &str) -> PyResult<String> {
    if !node.getattr("attrib")?.contains(key)? {
        return Err(invalid(format!("FOMOD <{}> requires '{key}'.", tag(node)?)));
    }
    get(node, key, "")
}
fn one<'py>(
    node: &Bound<'py, PyAny>,
    key: &str,
    required: bool,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let nodes = children(&node.call_method1("findall", (key,))?)?;
    if nodes.len() > 1 || (required && nodes.is_empty()) {
        return Err(invalid(format!(
            "FOMOD <{}> requires {} one <{key}>.",
            tag(node)?,
            if required { "exactly" } else { "at most" }
        )));
    }
    Ok(nodes.into_iter().next())
}
fn choice(value: &str, choices: &[&str], label: &str) -> PyResult<()> {
    if !choices.contains(&value) {
        return Err(invalid(format!("Unsupported FOMOD {label}: {value:?}")));
    }
    Ok(())
}
fn boolean(value: &str) -> PyResult<bool> {
    let value = value.trim();
    choice(value, &["true", "false", "1", "0"], "boolean")?;
    Ok(matches!(value, "true" | "1"))
}
fn version(value: &str) -> PyResult<Vec<String>> {
    if value.len() > 128
        || value.is_empty()
        || value
            .split('.')
            .any(|v| v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid(format!("Unsupported version value: {value:?}")));
    }
    Ok(value
        .split('.')
        .map(|v| {
            let v = v.trim_start_matches('0');
            if v.is_empty() { "0".into() } else { v.into() }
        })
        .collect())
}

fn grammar(tag: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    Some(match tag {
        "config" => (
            &[
                "moduleName",
                "moduleImage",
                "moduleDependencies",
                "requiredInstallFiles",
                "installSteps",
                "conditionalFileInstalls",
            ],
            &[],
        ),
        "moduleName" => (&[], &["position", "colour"]),
        "moduleImage" => (&[], &["path", "showImage", "showFade", "height"]),
        "moduleDependencies" | "dependencies" | "visible" => (DEPS, &["operator"]),
        "fileDependency" => (&[], &["file", "state"]),
        "flagDependency" => (&[], &["flag", "value"]),
        "gameDependency" | "fommDependency" | "foseDependency" | "nvseDependency"
        | "skseDependency" | "f4seDependency" => (&[], &["version"]),
        "requiredInstallFiles" | "files" => (&["file", "folder"], &[]),
        "file" | "folder" => (
            &[],
            &[
                "source",
                "destination",
                "alwaysInstall",
                "installIfUsable",
                "priority",
            ],
        ),
        "installSteps" => (&["installStep"], &["order"]),
        "installStep" => (&["visible", "optionalFileGroups"], &["name"]),
        "optionalFileGroups" => (&["group"], &["order"]),
        "group" => (&["plugins"], &["name", "type"]),
        "plugins" => (&["plugin"], &["order"]),
        "plugin" => (
            &[
                "description",
                "image",
                "files",
                "conditionFlags",
                "typeDescriptor",
            ],
            &["name"],
        ),
        "description" => (&[], &[]),
        "image" => (&[], &["path"]),
        "conditionFlags" => (&["flag"], &[]),
        "flag" | "type" | "defaultType" => (&[], &["name"]),
        "typeDescriptor" => (&["type", "dependencyType"], &[]),
        "dependencyType" => (&["defaultType", "patterns"], &[]),
        "patterns" => (&["pattern"], &[]),
        "pattern" => (&["dependencies", "type", "files"], &[]),
        "conditionalFileInstalls" => (&["patterns"], &[]),
        _ => return None,
    })
}

fn normalize(root: &Bound<'_, PyAny>) -> PyResult<u8> {
    let schema_key = "{http://www.w3.org/2001/XMLSchema-instance}noNamespaceSchemaLocation";
    let schema = get(root, schema_key, "")?;
    let mut version = if find(root, "moduleDependancies")?.is_some() {
        1
    } else if find(root, "optionalFileGroups")?.is_some() {
        3
    } else {
        5
    };
    if !schema.is_empty() {
        let lower = schema.to_ascii_lowercase();
        let stem = lower
            .strip_suffix(".xsd")
            .ok_or_else(|| invalid(format!("Unsupported FOMOD schema: {schema}")))?;
        let mut valid = false;
        for suffix in ["modconfig", "moduleconfig", "xmlscript"] {
            if stem.ends_with(suffix) {
                valid = true;
                break;
            }
            for value in ["1.0", "2.0", "3.0", "4.0", "5.0", "5.1"] {
                if stem.ends_with(&format!("{suffix}{value}")) {
                    version = value.as_bytes()[0] - b'0';
                    valid = true;
                    break;
                }
            }
            if valid {
                break;
            }
        }
        if !valid {
            return Err(invalid(format!("Unsupported FOMOD schema: {schema}")));
        }
        root.getattr("attrib")?.del_item(schema_key)?;
    }
    if version <= 3 {
        let mut allowed = BTreeSet::from([
            "moduleName",
            "moduleDependencies",
            "moduleDependancies",
            "requiredInstallFiles",
            "optionalFileGroups",
        ]);
        if version >= 2 {
            allowed.insert("conditionalFileInstalls");
        }
        if version == 3 {
            allowed.insert("moduleImage");
        }
        for child in children(root)? {
            if !allowed.contains(tag(&child)?.as_str()) {
                return Err(invalid(format!(
                    "Unsupported root construct for FOMOD {version}.0."
                )));
            }
        }
    }
    for node in root.call_method0("iter")?.try_iter()? {
        let node = node?;
        let name = tag(&node)?;
        if version == 1
            && [
                "conditionFlags",
                "conditionalFileInstalls",
                "flagDependency",
            ]
            .contains(&name.as_str())
        {
            return Err(invalid(
                "Condition flags and conditional installs require FOMOD 2.0 or later.",
            ));
        }
        let alias = match name.as_str() {
            "moduleDependancies" => "moduleDependencies",
            "dependancies" => "dependencies",
            "dependancy" | "fileDependancy" => "fileDependency",
            "dependancyType" => "dependencyType",
            "falloutDependancy" | "falloutDependency" => "gameDependency",
            "fommDependancy" => "fommDependency",
            "foseDependancy" => "foseDependency",
            _ => &name,
        };
        if alias != name {
            node.setattr("tag", alias)?;
        }
        if version <= 2
            && ["plugins", "optionalFileGroups"].contains(&alias)
            && node.getattr("attrib")?.contains("order")?
        {
            return Err(invalid("Ordering attributes require FOMOD 3.0 or later."));
        }
    }
    if version <= 2 {
        if let Some(dependencies) = find(root, "moduleDependencies")? {
            if dependencies.getattr("attrib")?.is_truthy()? {
                return Err(invalid(
                    "Legacy module dependencies use an implicit And operator.",
                ));
            }
            for child in children(&dependencies)? {
                if tag(&child)? == "fileDependency" {
                    if child.getattr("attrib")?.contains("state")? {
                        return Err(invalid(
                            "Legacy module file dependencies implicitly require Active state.",
                        ));
                    }
                    child.call_method1("set", ("state", "Active"))?;
                }
            }
        }
    }
    if let Some(groups) = find(root, "optionalFileGroups")? {
        if version >= 4
            || root
                .call_method1("findall", ("optionalFileGroups",))?
                .len()?
                != 1
        {
            return Err(invalid(
                "Flat optionalFileGroups are only supported for FOMOD 1–3.",
            ));
        }
        root.call_method1("remove", (&groups,))?;
        if version <= 2 {
            groups.call_method1("set", ("order", "Explicit"))?;
        }
        let et = root.py().import("xml.etree.ElementTree")?;
        let steps = et.call_method1("SubElement", (root, "installSteps"))?;
        steps.call_method1("set", ("order", "Explicit"))?;
        let step = et.call_method1("SubElement", (&steps, "installStep"))?;
        step.call_method1("set", ("name", "Installation options"))?;
        step.call_method1("append", (groups,))?;
    }
    Ok(version)
}

fn validate(node: &Bound<'_, PyAny>, depth: usize, count: &mut usize) -> PyResult<()> {
    *count += 1;
    if depth > 40 || *count > 30000 {
        return Err(invalid(
            "FOMOD XML nesting or element count exceeds the safety limit.",
        ));
    }
    let name = tag(node)?;
    let (allowed, attributes) =
        grammar(&name).ok_or_else(|| invalid(format!("Unsupported FOMOD element: <{name}>")))?;
    for attribute in node.getattr("attrib")?.try_iter()? {
        let attribute = attribute?.extract::<String>()?;
        if !attributes.contains(&attribute.as_str()) {
            return Err(invalid(format!(
                "Unsupported FOMOD attribute: {name}.{attribute}"
            )));
        }
    }
    for child in children(node)? {
        let child_name = tag(&child)?;
        if !allowed.contains(&child_name.as_str()) {
            return Err(invalid(format!(
                "Unexpected FOMOD <{child_name}> inside <{name}>."
            )));
        }
        validate(&child, depth + 1, count)?;
    }
    if [
        "config",
        "installStep",
        "group",
        "plugin",
        "dependencyType",
        "conditionalFileInstalls",
    ]
    .contains(&name.as_str())
    {
        for child in allowed {
            one(node, child, false)?;
        }
    }
    if node.getattr("attrib")?.contains("order")? {
        choice(
            &get(node, "order", "")?,
            &["Explicit", "Ascending", "Descending"],
            "order",
        )?;
    }
    if ["moduleDependencies", "dependencies", "visible"].contains(&name.as_str()) {
        choice(
            &get(node, "operator", "And")?,
            &["And", "Or"],
            "dependency operator",
        )?;
    }
    if ["group", "plugin", "installStep", "flag"].contains(&name.as_str()) {
        required(node, "name")?;
    }
    match name.as_str() {
        "group" => {
            choice(&required(node, "type")?, GROUPS, "group type")?;
            one(node, "plugins", true)?;
        }
        "plugin" => {
            one(node, "typeDescriptor", true)?;
        }
        "typeDescriptor" => {
            if node.len()? != 1 {
                return Err(invalid(
                    "A typeDescriptor must contain exactly one type or dependencyType.",
                ));
            }
        }
        "type" | "defaultType" => choice(&required(node, "name")?, TYPES, "plugin type")?,
        "dependencyType" | "conditionalFileInstalls" => {
            let dynamic = name == "dependencyType";
            if dynamic {
                one(node, "defaultType", true)?;
            }
            let patterns = one(node, "patterns", true)?.unwrap();
            for pattern in children(&patterns)? {
                one(&pattern, "dependencies", true)?;
                one(&pattern, if dynamic { "type" } else { "files" }, true)?;
                if find(&pattern, if dynamic { "files" } else { "type" })?.is_some() {
                    return Err(invalid(if dynamic {
                        "Type patterns cannot contain install files."
                    } else {
                        "Conditional file patterns cannot contain plugin types."
                    }));
                }
            }
        }
        "fileDependency" => {
            relative_path(&required(node, "file")?, false)?;
            choice(
                &required(node, "state")?,
                &["Active", "Inactive", "Missing"],
                "dependency state",
            )?;
        }
        "flagDependency" => {
            required(node, "flag")?;
            required(node, "value")?;
        }
        "gameDependency" | "fommDependency" | "foseDependency" | "nvseDependency"
        | "skseDependency" | "f4seDependency" => {
            version(&required(node, "version")?)?;
        }
        "file" | "folder" => {
            relative_path(&required(node, "source")?, true)?;
            relative_path(&get(node, "destination", "")?, true)?;
            for key in ["alwaysInstall", "installIfUsable"] {
                boolean(&get(node, key, "false")?)?;
            }
            node.py()
                .get_type::<pyo3::types::PyInt>()
                .call1((get(node, "priority", "0")?,))
                .map_err(|_| invalid("FOMOD file priority must be an integer."))?;
        }
        "image" | "moduleImage" => {
            relative_path(&get(node, "path", "")?, true)?;
            if name == "moduleImage" {
                for key in ["showImage", "showFade"] {
                    boolean(&get(node, key, "true")?)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[pyfunction]
fn fomod_prepare(root: &Bound<'_, PyAny>) -> PyResult<(u8, String, String)> {
    if tag(root)? != "config" {
        return Err(invalid(
            "Only declarative ModuleConfig XML is supported; executable installers cannot run here.",
        ));
    }
    let version = normalize(root)?;
    validate(root, 0, &mut 0)?;
    let name = one(root, "moduleName", true)?
        .unwrap()
        .getattr("text")?
        .extract::<Option<String>>()?
        .unwrap_or_default();
    let name = if name.trim().is_empty() {
        "Untitled mod"
    } else {
        name.trim()
    };
    let image = if let Some(image) = one(root, "moduleImage", false)? {
        if boolean(&get(&image, "showImage", "true")?)? {
            get(&image, "path", "")?
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    Ok((version, name.into(), image))
}

fn matches(
    node: Option<&Bound<'_, PyAny>>,
    environment: &Bound<'_, PyAny>,
    flags: &Value,
) -> PyResult<bool> {
    let Some(node) = node else { return Ok(true) };
    let name = tag(node)?;
    match name.as_str() {
        "dependencies" | "visible" | "moduleDependencies" => {
            let conjunction = get(node, "operator", "And")? == "And";
            let mut unresolved = None;
            for child in children(node)? {
                match matches(Some(&child), environment, flags) {
                    Ok(value) if value != conjunction => return Ok(value),
                    Ok(_) => {}
                    Err(e) => unresolved = Some(e),
                }
            }
            if let Some(error) = unresolved {
                return Err(error);
            }
            Ok(conjunction)
        }
        "fileDependency" => {
            let file = get(node, "file", "")?;
            let key = relative_path(&file, false)?.case_fold().collect::<String>();
            let state = environment.getattr("files")?.call_method1("get", (&key,))?;
            if state.is_none() && !environment.getattr("complete")?.is_truthy()? {
                return Err(invalid(format!(
                    "File state is unknown: {file}. Set it explicitly in Dependency environment."
                )));
            }
            Ok(state
                .extract::<String>()
                .unwrap_or_else(|_| "Missing".into())
                == get(node, "state", "")?)
        }
        "flagDependency" => {
            Ok(flags[get(node, "flag", "")?].as_str().unwrap_or("") == get(node, "value", "")?)
        }
        "gameDependency" | "fommDependency" | "foseDependency" | "nvseDependency"
        | "skseDependency" | "f4seDependency" => {
            let available = match name.as_str() {
                "gameDependency" => environment.getattr("game_version")?,
                "fommDependency" => environment.getattr("fomm_version")?,
                _ => environment
                    .getattr("extender_versions")?
                    .call_method1("get", (name.trim_end_matches("Dependency"), ""))?,
            };
            let available = available.extract::<String>().unwrap_or_default();
            if available.is_empty() {
                return Err(invalid(format!(
                    "Cannot evaluate {name}; supply an explicit version in the installer environment."
                )));
            }
            let actual = version(&available)?;
            let required = version(&get(node, "version", "")?)?;
            for index in 0..actual.len().max(required.len()) {
                let a = actual.get(index).map(String::as_str).unwrap_or("0");
                let b = required.get(index).map(String::as_str).unwrap_or("0");
                let order = a.len().cmp(&b.len()).then_with(|| a.cmp(b));
                if !order.is_eq() {
                    return Ok(order.is_gt());
                }
            }
            Ok(true)
        }
        _ => Err(invalid(format!("Unsupported dependency: {name}"))),
    }
}

#[pyfunction]
fn fomod_matches(
    node: &Bound<'_, PyAny>,
    environment: &Bound<'_, PyAny>,
    flags: &str,
) -> PyResult<bool> {
    matches(
        if node.is_none() { None } else { Some(node) },
        environment,
        &encoding::parse(flags)?,
    )
}

fn ordered<'py>(
    node: Option<&Bound<'py, PyAny>>,
    child: &str,
    sensitive: bool,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    let Some(node) = node else { return Ok(vec![]) };
    let values = children(&node.call_method1("findall", (child,))?)?;
    let order = get(node, "order", "Ascending")?;
    if order == "Explicit" {
        return Ok(values);
    }
    let mut values = values
        .into_iter()
        .map(|v| {
            let name = get(&v, "name", "")?;
            Ok((
                if sensitive {
                    name
                } else {
                    name.case_fold().collect()
                },
                v,
            ))
        })
        .collect::<PyResult<Vec<_>>>()?;
    values.sort_by(|a, b| {
        if order == "Descending" {
            b.0.cmp(&a.0)
        } else {
            a.0.cmp(&b.0)
        }
    });
    Ok(values.into_iter().map(|(_, v)| v).collect())
}

fn option_type(
    plugin: &Bound<'_, PyAny>,
    environment: &Bound<'_, PyAny>,
    flags: &Value,
) -> PyResult<String> {
    let descriptor = find(plugin, "typeDescriptor")?.unwrap();
    if let Some(value) = find(&descriptor, "type")? {
        return get(&value, "name", "");
    }
    let dynamic = find(&descriptor, "dependencyType")?.unwrap();
    for pattern in children(&find(&dynamic, "patterns")?.unwrap())? {
        if matches(find(&pattern, "dependencies")?.as_ref(), environment, flags)? {
            return get(&find(&pattern, "type")?.unwrap(), "name", "");
        }
    }
    get(&find(&dynamic, "defaultType")?.unwrap(), "name", "")
}

#[derive(Default)]
struct GroupResult {
    views: Vec<Value>,
    flags: serde_json::Map<String, Value>,
    rules: Vec<Py<PyAny>>,
    errors: Vec<String>,
    unresolved: Vec<PyErr>,
}

fn groups(
    step: &Bound<'_, PyAny>,
    step_index: usize,
    choices: &Value,
    environment: &Bound<'_, PyAny>,
    flags: &Value,
    mo2: bool,
    cancel: Option<&Bound<'_, PyAny>>,
    checkpoint: Option<&Bound<'_, PyAny>>,
) -> PyResult<GroupResult> {
    let mut result = GroupResult::default();
    for (group_index, group) in ordered(find(step, "optionalFileGroups")?.as_ref(), "group", mo2)?
        .into_iter()
        .enumerate()
    {
        crate::check_cancel(cancel)?;
        let group_id = format!("{step_index}:{group_index}");
        let plugins = ordered(find(&group, "plugins")?.as_ref(), "plugin", mo2)?;
        let mut kind = get(&group, "type", "")?;
        if mo2 && plugins.len() == 1 {
            if ["SelectExactlyOne", "SelectAtLeastOne"].contains(&kind.as_str()) {
                kind = "SelectAll".into();
            } else if kind == "SelectAtMostOne" {
                kind = "SelectAny".into();
            }
        }
        let mut options = vec![];
        for (index, plugin) in plugins.into_iter().enumerate() {
            crate::check_cancel(cancel)?;
            if let Some(checkpoint) = checkpoint {
                checkpoint.call1((&plugin,))?;
            }
            let option = if mo2 && kind == "SelectAll" {
                "Required".into()
            } else {
                match option_type(&plugin, environment, flags) {
                    Ok(value) => value,
                    Err(error) => {
                        result.unresolved.push(error);
                        "NotUsable".into()
                    }
                }
            };
            options.push((format!("{group_id}:{index}"), plugin, option));
        }
        let required: BTreeSet<_> = options
            .iter()
            .filter(|v| v.2 == "Required")
            .map(|v| v.0.clone())
            .collect();
        let usable: BTreeSet<_> = options
            .iter()
            .filter(|v| v.2 != "NotUsable")
            .map(|v| v.0.clone())
            .collect();
        let mut selected: BTreeSet<String> = choices[&group_id]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        selected.extend(required);
        let name = get(&group, "name", "")?;
        if kind == "SelectAll" {
            selected = usable.clone();
            if usable.len() != options.len() {
                result
                    .errors
                    .push(format!("{name}: SelectAll contains an unavailable option."));
            }
        } else if choices.get(&group_id).is_none() {
            let recommended: Vec<_> = options
                .iter()
                .filter(|v| v.2 == "Recommended")
                .map(|v| v.0.clone())
                .collect();
            if mo2 {
                if kind == "SelectAtMostOne" && selected.is_empty() {
                    selected.extend(recommended.last().cloned());
                } else if ["SelectExactlyOne", "SelectAtLeastOne"].contains(&kind.as_str())
                    && selected.is_empty()
                {
                    let fallback = options
                        .iter()
                        .find(|v| v.2 == "Optional")
                        .or_else(|| options.iter().find(|v| v.2 == "CouldBeUsable"))
                        .map(|v| v.0.clone());
                    selected.extend(recommended.first().cloned().or(fallback));
                } else if kind == "SelectAny" {
                    selected.extend(recommended);
                }
            } else {
                if ["SelectExactlyOne", "SelectAtMostOne"].contains(&kind.as_str())
                    && selected.is_empty()
                {
                    selected.extend(recommended.first().cloned());
                } else if ["SelectAny", "SelectAtLeastOne"].contains(&kind.as_str()) {
                    selected.extend(recommended);
                }
                if ["SelectExactlyOne", "SelectAtLeastOne"].contains(&kind.as_str())
                    && selected.is_empty()
                {
                    selected.extend(
                        options
                            .iter()
                            .find(|v| v.2 != "NotUsable")
                            .map(|v| v.0.clone()),
                    );
                }
            }
        }
        if !selected.is_subset(&usable) {
            result.errors.push(format!(
                "{name}: selection contains an unavailable or unknown option."
            ));
        }
        if (kind == "SelectExactlyOne" && selected.len() != 1)
            || (kind == "SelectAtMostOne" && selected.len() > 1)
            || (kind == "SelectAtLeastOne" && selected.is_empty())
        {
            result.errors.push(format!(
                "{name}: {kind} selection requirement is not satisfied."
            ));
        }
        let mut views = vec![];
        for (id, plugin, option) in options {
            crate::check_cancel(cancel)?;
            let chosen = selected.contains(&id);
            let image = if let Some(image) = find(&plugin, "image")? {
                get(&image, "path", "")?
            } else {
                String::new()
            };
            views.push(json!({"id":id,"name":get(&plugin,"name","")?,"description":plugin.call_method1("findtext",("description",""))?.extract::<String>()?,"type":option,"selected":chosen,"image":image}));
            if let Some(files) = find(&plugin, "files")? {
                for rule in children(&files)? {
                    crate::check_cancel(cancel)?;
                    if chosen
                        || boolean(&get(&rule, "alwaysInstall", "false")?)?
                        || (option != "NotUsable"
                            && boolean(&get(&rule, "installIfUsable", "false")?)?)
                    {
                        result.rules.push(rule.unbind());
                    }
                }
            }
            if chosen {
                if let Some(conditions) = find(&plugin, "conditionFlags")? {
                    for flag in children(&conditions)? {
                        let name = get(&flag, "name", "")?;
                        let value = flag
                            .getattr("text")?
                            .extract::<Option<String>>()?
                            .unwrap_or_default();
                        if mo2 {
                            if !name.is_empty() {
                                result.flags.entry(name).or_insert(json!(value.trim()));
                            }
                        } else {
                            result.flags.insert(name, json!(value));
                        }
                    }
                }
            }
        }
        result
            .views
            .push(json!({"id":group_id,"name":name,"type":kind,"options":views}));
    }
    Ok(result)
}

#[pyfunction]
#[pyo3(signature=(root,mode,choices,environment,cancel=None,checkpoint=None))]
fn fomod_evaluate(
    root: &Bound<'_, PyAny>,
    mode: &str,
    choices: &str,
    environment: &Bound<'_, PyAny>,
    cancel: Option<&Bound<'_, PyAny>>,
    checkpoint: Option<&Bound<'_, PyAny>>,
) -> PyResult<(String, String, Vec<String>, Vec<Py<PyAny>>)> {
    crate::check_cancel(cancel)?;
    let choices = encoding::parse(choices)?;
    let mut flags = json!({});
    let mut views = vec![];
    let mut errors = vec![];
    let mo2 = mode == "mo2";
    if !matches(
        find(root, "moduleDependencies")?.as_ref(),
        &environment,
        &flags,
    )? {
        return Err(invalid(
            "This package's module dependencies are not satisfied.",
        ));
    }
    let mut rules = if let Some(required) = find(root, "requiredInstallFiles")? {
        children(&required)?
            .into_iter()
            .map(Bound::unbind)
            .collect()
    } else {
        vec![]
    };
    for (index, step) in ordered(find(root, "installSteps")?.as_ref(), "installStep", mo2)?
        .into_iter()
        .enumerate()
    {
        crate::check_cancel(cancel)?;
        if !matches(find(&step, "visible")?.as_ref(), &environment, &flags)? {
            continue;
        }
        let mut seed = serde_json::Map::new();
        let mut seen = BTreeSet::new();
        let mut stable = None;
        for _ in 0..64 {
            crate::check_cancel(cancel)?;
            let mut context = flags.clone();
            if !mo2 {
                context.as_object_mut().unwrap().extend(seed.clone());
            }
            let mut evaluated = groups(
                &step,
                index,
                &choices,
                &environment,
                &context,
                mo2,
                cancel,
                checkpoint,
            )?;
            if mo2 || evaluated.flags == seed {
                if !evaluated.unresolved.is_empty() {
                    return Err(evaluated.unresolved.remove(0));
                }
                stable = Some(evaluated);
                break;
            }
            let state = json!(evaluated.flags).to_string();
            if !seen.insert(state) {
                return Err(invalid(
                    "FOMOD contains cyclic option-type/flag dependencies; no deterministic installation can be planned.",
                ));
            }
            seed = evaluated.flags;
        }
        let evaluated = stable.ok_or_else(|| {
            invalid("FOMOD option-type/flag resolution exceeds the 64-pass limit.")
        })?;
        rules.extend(evaluated.rules);
        errors.extend(evaluated.errors);
        flags.as_object_mut().unwrap().extend(evaluated.flags);
        views.push(json!({"name":get(&step,"name","")?,"groups":evaluated.views}));
    }
    if let Some(conditional) = find(root, "conditionalFileInstalls/patterns")? {
        for pattern in children(&conditional)? {
            crate::check_cancel(cancel)?;
            if matches(
                find(&pattern, "dependencies")?.as_ref(),
                &environment,
                &flags,
            )? {
                rules.extend(
                    children(&find(&pattern, "files")?.unwrap())?
                        .into_iter()
                        .map(Bound::unbind),
                );
            }
        }
    }
    Ok((json!(views).to_string(), flags.to_string(), errors, rules))
}

#[pyfunction]
#[pyo3(signature=(rules,errors,sequence,prefix,files,bindings,cancel=None))]
fn fomod_plan(
    rules: &Bound<'_, PyAny>,
    errors: Vec<String>,
    sequence: &Bound<'_, PyAny>,
    prefix: &str,
    files: &str,
    bindings: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    use std::collections::BTreeMap;
    if !errors.is_empty() {
        return Err(invalid(errors.join("\n")));
    }
    let files = encoding::parse(files)?;
    let files = files
        .as_object()
        .ok_or_else(|| invalid("Invalid installer file inventory."))?;
    let bindings = encoding::parse(bindings)?;
    let mut sorted = vec![];
    for rule in rules.try_iter()? {
        let rule = rule?;
        let priority = rule
            .py()
            .get_type::<pyo3::types::PyInt>()
            .call1((get(&rule, "priority", "0")?,))?
            .str()?
            .extract::<String>()?;
        let index: usize = sequence.get_item(&rule)?.extract()?;
        sorted.push((priority, index, rule));
    }
    sorted.sort_by(|a, b| {
        let negative_a = a.0.starts_with('-');
        let negative_b = b.0.starts_with('-');
        let order = negative_b.cmp(&negative_a).then_with(|| {
            let order = a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0));
            if negative_a { order.reverse() } else { order }
        });
        order.then_with(|| a.1.cmp(&b.1))
    });
    let mut result = BTreeMap::new();
    for (_, _, rule) in sorted {
        crate::check_cancel(cancel)?;
        let source = get(&rule, "source", "")?;
        if source.is_empty() {
            continue;
        }
        let original = format!("{prefix}{}", relative_path(&source, false)?);
        let key = original.case_fold().collect::<String>();
        let source = bindings[&key].as_str().unwrap_or(&original);
        let raw = get(&rule, "destination", &get(&rule, "source", "")?)?;
        let mut destination = relative_path(&raw, true)?;
        let mut pairs = vec![];
        if tag(&rule)? == "folder" {
            let prefix = if source.is_empty() {
                String::new()
            } else {
                format!("{}/", source.trim_end_matches('/'))
            };
            for name in files.values() {
                crate::check_cancel(cancel)?;
                let name = name.as_str().unwrap();
                if name
                    .case_fold()
                    .collect::<String>()
                    .starts_with(&prefix.case_fold().collect::<String>())
                {
                    let suffix = name
                        .chars()
                        .skip(prefix.chars().count())
                        .collect::<String>();
                    pairs.push((
                        name.to_string(),
                        format!(
                            "{}{suffix}",
                            if destination.is_empty() {
                                String::new()
                            } else {
                                format!("{destination}/")
                            }
                        ),
                    ));
                }
            }
            if pairs.is_empty() {
                return Err(invalid(format!(
                    "Installer folder is missing or empty: {source}"
                )));
            }
        } else {
            let key = source.case_fold().collect::<String>();
            let name = files
                .get(&key)
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(format!("Installer file is missing: {source}")))?;
            if destination.is_empty() || raw.ends_with(['/', '\\']) {
                destination = format!(
                    "{}{}",
                    if destination.is_empty() {
                        String::new()
                    } else {
                        format!("{destination}/")
                    },
                    original.rsplit('/').next().unwrap()
                );
            }
            pairs.push((name.into(), destination));
        }
        for (name, target) in pairs {
            crate::check_cancel(cancel)?;
            let target = relative_path(&target, false)?;
            result.insert(target.case_fold().collect::<String>(), (target, name));
        }
    }
    let plan: BTreeMap<_, _> = result.into_values().collect();
    crate::paths::validate_destinations(plan.keys().cloned().collect())?;
    if plan.is_empty() {
        return Err(invalid("The selected installer options produce no files."));
    }
    Ok(json!(plan).to_string())
}

#[pyfunction]
fn fomod_decode(
    data: &Bound<'_, pyo3::types::PyBytes>,
    mode: &str,
    checkpoint: &Bound<'_, PyAny>,
) -> PyResult<(String, String, Vec<String>)> {
    if !["mo2", "schema"].contains(&mode) {
        return Err(invalid("Choose a supported FOMOD compatibility policy."));
    }
    let bytes = data.as_bytes();
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(invalid("FOMOD XML must be at most 2 MiB."));
    }
    let mut notices = vec![];
    let (codec, mut label) = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff])
    {
        ("utf-16", "UTF-16")
    } else if mode == "mo2" && (bytes.starts_with(b"<\0") || bytes.starts_with(b"\0<")) {
        notices.push(
            "BOM-less UTF-16 compatibility decoding was used. Verify names and paths in Review."
                .into(),
        );
        if bytes.starts_with(b"<\0") {
            ("utf-16-le", "UTF-16 LE")
        } else {
            ("utf-16-be", "UTF-16 BE")
        }
    } else {
        ("utf-8-sig", "UTF-8")
    };
    let mut text = match data.call_method1("decode", (codec,)) {
        Ok(value) => value.extract::<String>()?,
        Err(e) if e.is_instance_of::<pyo3::exceptions::PyUnicodeDecodeError>(data.py()) => {
            if mode != "mo2" || codec != "utf-8-sig" {
                return Err(invalid(
                    "FOMOD XML must use UTF-8 or BOM-marked UTF-16 encoding.",
                ));
            }
            label = "Latin-1";
            notices.push(
                "Legacy Latin-1 compatibility decoding was used. Verify names and paths in Review."
                    .into(),
            );
            data.call_method1("decode", ("latin-1",))?.extract()?
        }
        Err(e) => return Err(e),
    };
    if text.contains('\0') {
        return Err(invalid("FOMOD XML contains invalid null characters."));
    }
    let re = data.py().import("re")?;
    if !re
        .call_method1("search", (r"<!\s*(DOCTYPE|ENTITY)", &text, 2))?
        .is_none()
    {
        return Err(invalid(
            "DTD and entity declarations are forbidden in installers.",
        ));
    }
    let declaration = re.call_method1("match", (r"\s*<\?xml\s+[^?]*\?>", &text))?;
    if !declaration.is_none() {
        let raw: String = declaration.get_item(0)?.extract()?;
        let encoding =
            re.call_method1("search", (r#"\bencoding\s*=\s*(['"])([^'"]+)\1"#, &raw, 2))?;
        if !encoding.is_none() {
            let name = encoding
                .get_item(2)?
                .extract::<String>()?
                .case_fold()
                .collect::<String>();
            if !["utf-8", "utf-16", "utf-16le", "utf-16be"].contains(&name.as_str()) {
                if mode == "schema" {
                    return Err(invalid(
                        "Only UTF-8 and BOM-marked UTF-16 XML declarations are supported.",
                    ));
                }
                notices.push("The XML encoding declaration was normalized for compatibility. Verify names and paths in Review.".into());
            }
        }
        let normalized: String = re
            .call_method1(
                "sub",
                (
                    r#"\bencoding\s*=\s*(['"])[^'"]+\1"#,
                    "encoding=\"utf-8\"",
                    raw,
                    0,
                    2,
                ),
            )?
            .extract()?;
        let end: usize = declaration.call_method0("end")?.extract()?;
        text = normalized + &text.chars().skip(end).collect::<String>();
    }
    checkpoint.call0()?;
    Ok((text, label.into(), notices))
}

#[pyfunction]
fn fomod_decoration(root: &Bound<'_, PyAny>) -> PyResult<Py<pyo3::types::PyDict>> {
    let result = pyo3::types::PyDict::new(root.py());
    let title = find(root, "moduleName")?.unwrap();
    let position = get(&title, "position", "Left")?;
    if !["Left", "Right", "RightOfImage"].contains(&position.as_str()) {
        return Err(invalid(
            "Installer title position must be Left, Right or RightOfImage.",
        ));
    }
    result.set_item("position", position)?;
    if title.getattr("attrib")?.contains("colour")? {
        let colour = get(&title, "colour", "")?;
        if ![6, 8].contains(&colour.len()) || !colour.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid(
                "Installer title colour must contain RGB or ARGB hexadecimal bytes.",
            ));
        }
        let mut values = vec![];
        for index in (0..colour.len()).step_by(2) {
            values.push(u8::from_str_radix(&colour[index..index + 2], 16).unwrap() as f64 / 255.0);
        }
        if values.len() == 3 {
            values.push(1.0);
        } else {
            values.rotate_left(1);
        }
        result.set_item("colour", pyo3::types::PyTuple::new(root.py(), values)?)?;
    }
    if let Some(image) = find(root, "moduleImage")? {
        let height = root
            .py()
            .get_type::<pyo3::types::PyInt>()
            .call1((get(&image, "height", "-1")?,))
            .map_err(|_| invalid("Installer header image height must be an integer."))?;
        result.set_item("height", height)?;
        let visible = ["true", "1"].contains(&get(&image, "showImage", "true")?.as_str());
        let source = get(&image, "path", "")?;
        let source = if visible {
            relative_path(
                if source.is_empty() {
                    "fomod/screenshot.png"
                } else {
                    &source
                },
                false,
            )?
        } else {
            String::new()
        };
        result.set_item("image", source)?;
        result.set_item(
            "fade",
            ["true", "1"].contains(&get(&image, "showFade", "true")?.as_str()),
        )?;
    }
    Ok(result.unbind())
}

#[pyfunction]
fn fomod_file_state(name: &str, state: &Bound<'_, PyAny>) -> PyResult<(String, String)> {
    let state = state
        .extract::<String>()
        .map_err(|_| invalid("Unsupported FOMOD file state."))?;
    choice(&state, &["Active", "Inactive", "Missing"], "file state")?;
    Ok((
        relative_path(name, false)?.case_fold().collect(),
        state.into(),
    ))
}

#[pyfunction]
fn fomod_file_key(name: &str) -> PyResult<String> {
    Ok(relative_path(name, false)?.case_fold().collect())
}

#[pyfunction]
#[pyo3(signature=(package,evaluation,sequence,repairs,validator,cancel=None))]
fn fomod_install_plan(
    package: &Bound<'_, PyAny>,
    evaluation: &Bound<'_, PyAny>,
    sequence: &Bound<'_, PyAny>,
    repairs: &Bound<'_, PyAny>,
    validator: &Bound<'_, PyAny>,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    crate::check_cancel(cancel)?;
    let errors: Vec<String> = evaluation.getattr("errors")?.extract()?;
    if !errors.is_empty() {
        return Err(invalid(errors.join("\n")));
    }
    let json = package.py().import("json")?;
    let bindings = if repairs.is_truthy()? {
        json.call_method1(
            "dumps",
            (validator.call1((package, evaluation, repairs, cancel))?,),
        )?
        .extract::<String>()?
    } else {
        "{}".into()
    };
    fomod_plan(
        &evaluation.getattr("rules")?,
        errors,
        sequence,
        &package.getattr("prefix")?.extract::<String>()?,
        &json
            .call_method1("dumps", (package.getattr("files")?,))?
            .extract::<String>()?,
        &bindings,
        cancel,
    )
}

fn missing_sources(
    package: &crate::package::PackageIndex,
    prefix: &str,
    rules: &Bound<'_, PyAny>,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<Vec<Value>> {
    let mut result = std::collections::BTreeMap::<(String, bool), Vec<String>>::new();
    for rule in rules.try_iter()? {
        crate::check_cancel(cancel)?;
        let rule = rule?;
        let raw = get(&rule, "source", "")?;
        if raw.is_empty() {
            continue;
        }
        let source = format!("{prefix}{}", relative_path(&raw, false)?);
        let folder = tag(&rule)? == "folder";
        if !package.available(&source, folder) {
            result
                .entry((source, folder))
                .or_default()
                .push(get(&rule, "destination", &raw)?);
        }
    }
    Ok(result.into_iter().map(|((source,folder),destinations)|json!({"source":source,"folder":folder,"destinations":destinations})).collect())
}

#[pyfunction]
#[pyo3(signature=(package,prefix,rules,cancel=None))]
fn fomod_missing(
    package: &crate::package::PackageIndex,
    prefix: &str,
    rules: &Bound<'_, PyAny>,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    Ok(json!(missing_sources(package, prefix, rules, cancel)?).to_string())
}

#[pyfunction]
#[pyo3(signature=(package,prefix,rules,bindings,cancel=None))]
fn fomod_repair(
    package: &crate::package::PackageIndex,
    prefix: &str,
    rules: &Bound<'_, PyAny>,
    bindings: &str,
    cancel: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let known: std::collections::BTreeMap<String, bool> =
        missing_sources(package, prefix, rules, cancel)?
            .into_iter()
            .map(|v| {
                (
                    v["source"].as_str().unwrap().case_fold().collect(),
                    v["folder"].as_bool().unwrap(),
                )
            })
            .collect();
    let bindings = encoding::parse(bindings)?;
    let bindings = bindings
        .as_object()
        .ok_or_else(|| invalid("Invalid installer source repairs."))?;
    if bindings.len() > 10000 {
        return Err(invalid(
            "Installer source repair exceeds 10,000 explicit bindings.",
        ));
    }
    let mut result = std::collections::BTreeMap::new();
    for (original, replacement) in bindings {
        crate::check_cancel(cancel)?;
        let key = relative_path(original, false)?
            .case_fold()
            .collect::<String>();
        let folder=known.get(&key).ok_or_else(||invalid("A source repair refers to a changed or available declaration. Remove it and review again."))?;
        let replacement = relative_path(
            replacement
                .as_str()
                .ok_or_else(|| invalid("Invalid installer replacement path."))?,
            false,
        )?;
        if crate::package::installer_metadata(&replacement)
            || !package.available(&replacement, *folder)
        {
            return Err(invalid(
                "Replacement must be an existing payload of the same file/folder kind inside this package.",
            ));
        }
        if result.insert(key, replacement).is_some() {
            return Err(invalid(
                "Duplicate source repair ignoring Windows filename case.",
            ));
        }
    }
    Ok(json!(result).to_string())
}

#[pyfunction]
fn fomod_can_advance(steps: &str, index: i64) -> PyResult<bool> {
    let steps = encoding::parse(steps)?;
    let Some(step) = usize::try_from(index).ok().and_then(|i| steps.get(i)) else {
        return Ok(false);
    };
    for group in step["groups"].as_array().unwrap() {
        let options = group["options"].as_array().unwrap();
        let selected: Vec<_> = options.iter().filter(|v| v["selected"] == true).collect();
        if selected.iter().any(|v| v["type"] == "NotUsable") {
            return Ok(false);
        }
        let count = selected.len();
        if (group["type"] == "SelectExactlyOne" && count != 1)
            || (group["type"] == "SelectAtMostOne" && count > 1)
            || (group["type"] == "SelectAtLeastOne" && count == 0)
            || (group["type"] == "SelectAll" && count != options.len())
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[pyfunction]
fn fomod_states_action(
    states: &Bound<'_, PyAny>,
    action: &str,
    args: &Bound<'_, pyo3::types::PyTuple>,
    kwargs: &Bound<'_, pyo3::types::PyDict>,
) -> PyResult<Py<PyAny>> {
    use pyo3::types::PyTuple;
    let py = states.py();
    let dictionary = py.import("builtins")?.getattr("dict")?;
    let mut changed = false;
    let result = match action {
        "set" => {
            let (key, value) =
                fomod_file_state(&args.get_item(0)?.extract::<String>()?, &args.get_item(1)?)?;
            if !states.call_method1("get", (&key,))?.eq(&value)? {
                dictionary.call_method1("__setitem__", (states, key, value))?;
                changed = true;
            }
            py.None().into_bound(py)
        }
        "delete" => {
            dictionary.call_method1(
                "__delitem__",
                (
                    states,
                    fomod_file_key(&args.get_item(0)?.extract::<String>()?)?,
                ),
            )?;
            changed = true;
            py.None().into_bound(py)
        }
        "update" => {
            let values = dictionary.call((args.get_item(0)?,), Some(kwargs))?;
            for pair in values.call_method0("items")?.try_iter()? {
                let pair = pair?;
                fomod_states_action(
                    states,
                    "set",
                    &PyTuple::new(py, [pair.get_item(0)?, pair.get_item(1)?])?,
                    kwargs,
                )?;
            }
            py.None().into_bound(py)
        }
        "clear" => {
            changed = states.is_truthy()?;
            dictionary.call_method1("clear", (states,))?
        }
        "pop" => {
            let key = fomod_file_key(&args.get_item(0)?.extract::<String>()?)?;
            changed = states.contains(&key)?;
            let mut inputs = vec![states.clone(), key.into_pyobject(py)?.into_any()];
            inputs.extend(args.iter().skip(1));
            dictionary
                .getattr("pop")?
                .call(PyTuple::new(py, inputs)?, None)?
        }
        "popitem" => {
            let result = dictionary.call_method1("popitem", (states,))?;
            changed = true;
            result
        }
        "setdefault" => {
            let key = fomod_file_key(&args.get_item(0)?.extract::<String>()?)?;
            if !states.contains(&key)? {
                fomod_states_action(
                    states,
                    "set",
                    &PyTuple::new(
                        py,
                        [key.clone().into_pyobject(py)?.into_any(), args.get_item(1)?],
                    )?,
                    kwargs,
                )?;
            }
            states.get_item(key)?
        }
        _ => return Err(invalid("Unknown FOMOD state operation.")),
    };
    if changed {
        states.setattr(
            "revision",
            states.getattr("revision")?.extract::<u64>()? + 1,
        )?;
    }
    Ok(result.unbind())
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(fomod_states_action, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_prepare, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_matches, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_evaluate, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_plan, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_decode, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_decoration, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_file_state, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_file_key, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_install_plan, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_missing, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_repair, m)?)?;
    m.add_function(wrap_pyfunction!(fomod_can_advance, m)?)?;
    Ok(())
}
