use crate::error::FnvScriptError;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct FunctionMap {
    entries: HashMap<String, FunctionEntry>,
}

#[derive(Debug, Clone)]
pub struct FunctionEntry {
    pub name: String,
    pub shape: EntryShape,
    pub arg_kinds: Vec<String>,
    pub return_kind: String,
}

#[derive(Debug, Clone)]
pub enum EntryShape {
    Papyrus {
        template: String,
    },
    Expansion {
        template: String,
    },
    Drop {
        reason: String,
        strict_failure: bool,
    },
}

#[derive(Debug, Deserialize)]
struct RawEntry {
    papyrus: Option<String>,
    expansion: Option<String>,
    rewrite: Option<String>,
    reason: Option<String>,
    strict_failure: Option<bool>,
    #[serde(default)]
    arg_kinds: Vec<String>,
    return_kind: Option<String>,
}

impl FunctionMap {
    pub fn from_yaml(s: &str) -> Result<Self, FnvScriptError> {
        if s.trim().is_empty() {
            return Ok(Self {
                entries: HashMap::new(),
            });
        }

        let raw: HashMap<String, RawEntry> =
            serde_yaml::from_str(s).map_err(|err| FnvScriptError::Parse {
                line: 0,
                col: 0,
                msg: format!("function_map: {err}"),
            })?;

        let mut entries = HashMap::new();
        for (name, entry) in raw {
            let arg_kinds = entry
                .arg_kinds
                .into_iter()
                .map(|kind| normalize_arg_kind(&name, &kind))
                .collect::<Result<Vec<_>, _>>()?;
            let shape = if let Some(template) = entry.papyrus {
                validate_template_arguments(&name, &template, arg_kinds.len())?;
                EntryShape::Papyrus { template }
            } else if let Some(template) = entry.expansion {
                validate_template_arguments(&name, &template, arg_kinds.len())?;
                EntryShape::Expansion { template }
            } else if entry.rewrite.as_deref() == Some("drop_with_warning") {
                EntryShape::Drop {
                    reason: entry.reason.unwrap_or_else(|| "no FO4 equivalent".into()),
                    strict_failure: entry.strict_failure.unwrap_or(true),
                }
            } else {
                return Err(FnvScriptError::Parse {
                    line: 0,
                    col: 0,
                    msg: format!("function_map: '{name}' missing papyrus/expansion/rewrite"),
                });
            };

            entries.insert(
                normalize_name(&name),
                FunctionEntry {
                    name,
                    shape,
                    arg_kinds,
                    return_kind: entry.return_kind.unwrap_or_else(|| "void".into()),
                },
            );
        }

        Ok(Self { entries })
    }

    pub fn get(&self, name: &str) -> Option<&FunctionEntry> {
        self.entries.get(&normalize_name(name))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

fn normalize_name(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn normalize_arg_kind(function_name: &str, kind: &str) -> Result<String, FnvScriptError> {
    let normalized = kind.trim().to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        "actor"
            | "actor_value"
            | "bool"
            | "faction"
            | "float"
            | "formkey"
            | "int"
            | "message"
            | "object"
            | "object_reference"
            | "package"
            | "perk"
            | "quest"
            | "reputation"
            | "string"
            | "topic"
    ) {
        Ok(normalized)
    } else {
        Err(FnvScriptError::Parse {
            line: 0,
            col: 0,
            msg: format!("function_map: '{function_name}' has unsupported arg kind '{kind}'"),
        })
    }
}

fn validate_template_arguments(
    function_name: &str,
    template: &str,
    arg_count: usize,
) -> Result<(), FnvScriptError> {
    let mut referenced = vec![false; arg_count];
    let mut remainder = template;
    while let Some(start) = remainder.find("{arg") {
        remainder = &remainder[start + 4..];
        let Some(end) = remainder.find('}') else {
            return Err(template_error(
                function_name,
                "contains an unterminated argument placeholder",
            ));
        };
        let index_text = &remainder[..end];
        let index = index_text.parse::<usize>().map_err(|_| {
            template_error(
                function_name,
                &format!("contains malformed argument placeholder '{{arg{index_text}}}'"),
            )
        })?;
        let Some(slot) = referenced.get_mut(index) else {
            return Err(template_error(
                function_name,
                &format!(
                    "references argument {index}, but arg_kinds declares {arg_count} argument(s)"
                ),
            ));
        };
        *slot = true;
        remainder = &remainder[end + 1..];
    }

    if let Some(index) = referenced.iter().position(|seen| !seen) {
        return Err(template_error(
            function_name,
            &format!("declares argument {index}, but the template does not consume it"),
        ));
    }
    Ok(())
}

fn template_error(function_name: &str, detail: &str) -> FnvScriptError {
    FnvScriptError::Parse {
        line: 0,
        col: 0,
        msg: format!("function_map: '{function_name}' {detail}"),
    }
}
