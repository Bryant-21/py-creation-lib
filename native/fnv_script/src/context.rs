use crate::function_map::FunctionMap;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptTarget {
    Quest,
    Actor,
    ObjectReference,
    ReferenceAlias,
    MagicEffect,
    Other,
}

impl ScriptTarget {
    pub fn from_papyrus_extends(extends: &str) -> Self {
        match extends.to_ascii_lowercase().as_str() {
            "quest" => Self::Quest,
            "actor" => Self::Actor,
            "objectreference" => Self::ObjectReference,
            "referencealias" => Self::ReferenceAlias,
            "activemagiceffect" => Self::MagicEffect,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SymbolMetadata {
    pub papyrus_name: String,
    pub papyrus_type: String,
    pub property_kind: PropertyKind,
    static_record_kind: Option<String>,
    members: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyKind {
    MutableState,
    ExternalBinding,
    Intrinsic,
}

impl SymbolMetadata {
    pub fn new(papyrus_name: impl Into<String>, papyrus_type: impl Into<String>) -> Self {
        Self {
            papyrus_name: papyrus_name.into(),
            papyrus_type: papyrus_type.into(),
            property_kind: PropertyKind::ExternalBinding,
            static_record_kind: None,
            members: HashMap::new(),
        }
    }

    pub fn with_member(mut self, source_name: &str, papyrus_name: &str) -> Self {
        self.members
            .insert(source_name.to_ascii_lowercase(), papyrus_name.to_string());
        self
    }

    pub fn intrinsic(mut self) -> Self {
        self.property_kind = PropertyKind::Intrinsic;
        self
    }

    pub fn with_static_record_kind(mut self, record_kind: &str) -> Self {
        self.static_record_kind = Some(record_kind.to_ascii_lowercase());
        self
    }

    pub fn static_record_kind(&self) -> Option<&str> {
        self.static_record_kind.as_deref()
    }

    pub fn member(&self, source_name: &str) -> Option<&str> {
        self.members
            .get(&source_name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

#[derive(Debug, Clone)]
pub struct TargetMetadata {
    pub base_class: ScriptTarget,
    pub game_mode_timer_seconds: f64,
    pub game_mode_timer_id: i64,
    symbols: HashMap<String, SymbolMetadata>,
}

impl TargetMetadata {
    pub fn for_extends(extends: &str) -> Self {
        Self {
            base_class: ScriptTarget::from_papyrus_extends(extends),
            game_mode_timer_seconds: 1.0,
            game_mode_timer_id: 0,
            symbols: HashMap::new(),
        }
    }

    pub fn insert_symbol(&mut self, source_name: &str, metadata: SymbolMetadata) {
        self.symbols
            .insert(source_name.to_ascii_lowercase(), metadata);
    }

    pub fn symbol(&self, source_name: &str) -> Option<&SymbolMetadata> {
        self.symbols.get(&source_name.to_ascii_lowercase())
    }

    pub fn symbols(&self) -> impl Iterator<Item = &SymbolMetadata> {
        self.symbols.values()
    }
}

#[derive(Debug, Clone)]
pub struct FnvScriptContext {
    pub function_map: FunctionMap,
    pub actor_value_map: HashMap<String, String>,
    pub mod_prefix: String,
    pub strict: bool,
    pub script_class_name: String,
    pub papyrus_extends: String,
    pub target: TargetMetadata,
}
