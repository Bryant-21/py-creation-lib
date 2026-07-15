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
            let shape = if let Some(template) = entry.papyrus {
                EntryShape::Papyrus { template }
            } else if let Some(template) = entry.expansion {
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
                    arg_kinds: entry.arg_kinds,
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
