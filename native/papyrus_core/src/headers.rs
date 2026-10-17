use crate::pex::{parse_pex_file, PexObjectPayload};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use unicode_casefold::UnicodeCaseFold;

pub const DEFAULT_MANIFEST: &str =
    include_str!("../../../python/creation_lib/pex/data/fo4_papyrus_defaults.json");
pub const FLAGS: &str =
    include_str!("../../../python/creation_lib/pex/data/Institute_Papyrus_Flags.flg");
pub const FLAGS_FILE_NAME: &str = "Institute_Papyrus_Flags.flg";
pub const EMITTER_SOURCE: &str = include_str!("headers.rs");

#[derive(Deserialize)]
pub struct HeaderObject {
    name: String,
    parent: String,
    structs: Vec<HeaderStruct>,
    properties: Vec<HeaderProperty>,
    states: Vec<HeaderState>,
    #[serde(default)]
    custom_events: Vec<String>,
}

// PEX keeps no CustomEvent declarations; a send leaves "<script>_<Event>" in the string table.
fn custom_events(script: &str, strings: &[String]) -> Vec<String> {
    let prefix = format!("{}_", script.to_lowercase());
    let mut events: Vec<String> = strings
        .iter()
        .filter(|s| s.len() > prefix.len() && s.is_char_boundary(prefix.len()))
        .filter(|s| s[..prefix.len()].to_lowercase() == prefix)
        .map(|s| s[prefix.len()..].to_owned())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .collect();
    events.sort_by_key(|e| e.to_lowercase());
    events.dedup_by_key(|e| e.to_lowercase());
    events
}

const PROPERTY_READ: u8 = 1;
const PROPERTY_WRITE: u8 = 2;
const PROPERTY_AUTO: u8 = 4;

#[derive(Deserialize)]
struct HeaderProperty {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    // 0 (absent) keeps the old "always Auto" output for callers that pass no PEX flags.
    #[serde(default)]
    flags: u8,
    #[serde(default)]
    auto_const: bool,
}

#[derive(Deserialize)]
struct HeaderStruct {
    name: String,
    members: Vec<HeaderParameter>,
}

#[derive(Deserialize)]
struct HeaderParameter {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

#[derive(Deserialize)]
struct HeaderState {
    name: String,
    functions: Vec<HeaderFunction>,
}

#[derive(Deserialize)]
struct HeaderFunction {
    name: String,
    return_type: Option<String>,
    is_global: bool,
    params: Vec<HeaderParameter>,
}

fn source_type(ty: &str, owner: &str) -> String {
    let Some((script, structure)) = ty.split_once('#') else {
        return ty.to_owned();
    };
    if script.case_fold().eq(owner.case_fold()) {
        structure.to_owned()
    } else {
        format!("{script}:{structure}")
    }
}

pub fn emit_header_object(obj: &HeaderObject, manifest: &Value) -> String {
    let mut declaration = format!("Scriptname {}", obj.name);
    if !obj.parent.is_empty() {
        declaration.push_str(&format!(" Extends {}", obj.parent));
    }
    let mut lines = vec![declaration, String::new()];
    for structure in &obj.structs {
        lines.push(format!("Struct {}", structure.name));
        for member in &structure.members {
            lines.push(format!(
                "    {} {}",
                source_type(&member.ty, &obj.name),
                member.name
            ));
        }
        lines.extend(["EndStruct".into(), String::new()]);
    }
    let mut properties = false;
    for property in &obj.properties {
        if property.name.starts_with("::") {
            continue;
        }
        let ty = source_type(&property.ty, &obj.name);
        // An Auto declaration adds a non-const backing variable, which a Const child script
        // rejects; getter/setter-only PEX properties (X, AutoReadOnly) must not get one.
        if property.flags == 0 || property.flags & PROPERTY_AUTO != 0 {
            let konst = if property.auto_const { " Const" } else { "" };
            lines.push(format!("{ty} Property {} Auto{konst}", property.name));
        } else {
            lines.push(format!("{ty} Property {}", property.name));
            if property.flags & PROPERTY_READ != 0 {
                lines.extend([format!("    {ty} Function Get()"), "    EndFunction".into()]);
            }
            if property.flags & PROPERTY_WRITE != 0 {
                lines.extend([format!("    Function Set({ty} value)"), "    EndFunction".into()]);
            }
            lines.push("EndProperty".into());
        }
        properties = true;
    }
    if properties {
        lines.push(String::new());
    }
    for event in &obj.custom_events {
        lines.push(format!("CustomEvent {event}"));
    }
    if !obj.custom_events.is_empty() {
        lines.push(String::new());
    }
    let key = obj.name.to_lowercase();
    let defaults = &manifest["defaults"][&key];
    let events = manifest["events"][&key].as_array();
    let mut functions = BTreeMap::new();
    for empty in [true, false] {
        for state in &obj.states {
            if state.name.is_empty() != empty {
                continue;
            }
            for function in &state.functions {
                if !function.name.starts_with("::") {
                    functions
                        .entry(function.name.to_lowercase())
                        .or_insert(function);
                }
            }
        }
    }
    for (key, function) in functions {
        let params = function
            .params
            .iter()
            .enumerate()
            .map(|(index, param)| {
                let mut value = format!("{} {}", source_type(&param.ty, &obj.name), param.name);
                if let Some(default) = defaults[&key][index.to_string()].as_str() {
                    value.push_str(&format!(" = {default}"));
                }
                value
            })
            .collect::<Vec<_>>()
            .join(", ");
        if events.is_some_and(|events| events.iter().any(|event| event.as_str() == Some(&key))) {
            lines.extend([
                format!("Event {}({params})", function.name),
                "EndEvent".into(),
                String::new(),
            ]);
        } else {
            let prefix = match function.return_type.as_deref() {
                None | Some("") | Some("None") => String::new(),
                Some(ty) => format!("{} ", source_type(ty, &obj.name)),
            };
            let global = if function.is_global { " Global" } else { "" };
            lines.push(format!(
                "{prefix}Function {}({params}){global} Native",
                function.name
            ));
        }
    }
    format!("{}\n", lines.join("\n").trim_end())
}

pub fn emit_header(obj: &PexObjectPayload, strings: &[String], manifest: &Value) -> String {
    let header = HeaderObject {
        name: obj.name.clone(),
        parent: obj.parent.clone(),
        structs: obj
            .structs
            .iter()
            .map(|s| HeaderStruct {
                name: s.name.clone(),
                members: s
                    .members
                    .iter()
                    .map(|m| HeaderParameter {
                        name: m.name.clone(),
                        ty: m.ty.clone(),
                    })
                    .collect(),
            })
            .collect(),
        properties: obj
            .properties
            .iter()
            .map(|p| HeaderProperty {
                name: p.name.clone(),
                ty: p.ty.clone(),
                flags: p.flags,
                auto_const: obj
                    .variables
                    .iter()
                    .any(|v| v.is_const && v.name.eq_ignore_ascii_case(&p.auto_var)),
            })
            .collect(),
        states: obj
            .states
            .iter()
            .map(|s| HeaderState {
                name: s.name.clone(),
                functions: s
                    .functions
                    .iter()
                    .map(|f| HeaderFunction {
                        name: f.name.clone(),
                        return_type: Some(f.return_type.clone()),
                        is_global: f.is_global,
                        params: f
                            .params
                            .iter()
                            .map(|p| HeaderParameter {
                                name: p.name.clone(),
                                ty: p.ty.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
        custom_events: custom_events(&obj.name, strings),
    };
    emit_header_object(&header, manifest)
}

pub fn emit_headers_for_file(
    path: &Path,
    manifest: &Value,
) -> Result<Vec<(String, String)>, String> {
    let file = parse_pex_file(&path.to_string_lossy())?;
    Ok(file
        .objects
        .iter()
        .map(|obj| (obj.name.clone(), emit_header(obj, &file.string_table, manifest)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    #[ignore = "requires a captured local PEX header corpus"]
    fn captured_header_corpus_matches() {
        let baseline =
            std::fs::read_to_string(std::env::var("PAPYRUS_HEADER_BASELINE").unwrap()).unwrap();
        let manifest = serde_json::from_str(DEFAULT_MANIFEST).unwrap();
        let mut count = 0;
        for line in baseline.lines() {
            let row: Value = serde_json::from_str(line).unwrap();
            let path = Path::new(row["path"].as_str().unwrap());
            let actual = emit_headers_for_file(path, &manifest);
            if let Some(expected) = row.get("headers") {
                assert_eq!(
                    serde_json::to_value(actual.unwrap()).unwrap(),
                    *expected,
                    "{}",
                    path.display()
                );
            } else {
                assert!(actual.is_err(), "{}", path.display());
            }
            count += 1;
        }
        eprintln!("Matched {count} captured PEX headers");
    }

    #[test]
    fn source_declarations_preserve_defaults_events_and_default_state_precedence() {
        let object: HeaderObject = serde_json::from_value(json!({
            "name":"Widget", "parent":"Form", "structs":[],
            "properties":[{"name":"Value","type":"Widget#Entry[]"},{"name":"::private","type":"Int"}],
            "states":[
                {"name":"Busy","functions":[{"name":"Read","return_type":"Int","params":[],"is_global":false}]},
                {"name":"","functions":[
                    {"name":"Read","return_type":"Other#Entry[]","params":[],"is_global":true},
                    {"name":"OnInit","return_type":"None","params":[{"name":"value","type":"Int"}],"is_global":false},
                    {"name":"::remote_x","return_type":"None","params":[],"is_global":false}
                ]}
            ]
        })).unwrap();
        let manifest =
            json!({"defaults":{"widget":{"oninit":{"0":"3"}}},"events":{"widget":["oninit"]}});
        assert_eq!(
            emit_header_object(&object, &manifest),
            "Scriptname Widget Extends Form\n\nEntry[] Property Value Auto\n\nEvent OnInit(Int value = 3)\nEndEvent\n\nOther:Entry[] Function Read() Global Native\n"
        );
    }

    #[test]
    fn only_auto_properties_declare_a_backing_variable() {
        let object: HeaderObject = serde_json::from_value(json!({
            "name":"Ref", "parent":"Form", "structs":[], "states":[],
            "properties":[
                {"name":"X","type":"Float","flags":1},
                {"name":"Owner","type":"Actor","flags":3},
                {"name":"Count","type":"Int","flags":7},
                {"name":"Link","type":"Form","flags":7,"auto_const":true}
            ]
        })).unwrap();
        assert_eq!(
            emit_header_object(&object, &json!({})),
            "Scriptname Ref Extends Form\n\n\
             Float Property X\n    Float Function Get()\n    EndFunction\nEndProperty\n\
             Actor Property Owner\n    Actor Function Get()\n    EndFunction\n    Function Set(Actor value)\n    EndFunction\nEndProperty\n\
             Int Property Count Auto\n\
             Form Property Link Auto Const\n"
        );
    }

    #[test]
    fn custom_events_come_back_from_their_send_strings() {
        let strings = [
            "followersscript_CompanionChange",
            "FollowersScript_Bad Name",
            "otherscript_Elsewhere",
            "followersscript_",
        ]
        .map(String::from);
        assert_eq!(
            custom_events("FollowersScript", &strings),
            vec!["CompanionChange".to_string()]
        );
        let object: HeaderObject = serde_json::from_value(json!({
            "name":"Q", "parent":"Quest", "structs":[], "properties":[], "states":[],
            "custom_events":["Changed"]
        })).unwrap();
        assert_eq!(
            emit_header_object(&object, &json!({})),
            "Scriptname Q Extends Quest\n\nCustomEvent Changed\n"
        );
    }
}
