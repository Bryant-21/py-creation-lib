use crate::function_map::FunctionMap;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct FnvScriptContext {
    pub function_map: FunctionMap,
    pub actor_value_map: HashMap<String, String>,
    pub mod_prefix: String,
    pub strict: bool,
    pub script_class_name: String,
    pub papyrus_extends: String,
}
