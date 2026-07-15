use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;

use super::generated::*;
use super::types::*;

pub struct NifSchema {
    basics: HashMap<&'static str, &'static BasicTypeDef>,
    enums: HashMap<&'static str, &'static EnumTypeDef>,
    bitflags: HashMap<&'static str, &'static BitflagTypeDef>,
    bitfields: HashMap<&'static str, &'static BitfieldTypeDef>,
    structs: HashMap<&'static str, &'static StructTypeDef>,
    niobjects: HashMap<&'static str, &'static NiObjectTypeDef>,
    hierarchy_cache: Mutex<HashMap<String, Vec<String>>>,
    all_fields_cache: Mutex<HashMap<String, Vec<&'static FieldDef>>>,
    all_field_plan_cache: Mutex<HashMap<String, Arc<Vec<FieldPlanEntry>>>>,
    is_subtype_cache: Mutex<HashMap<(String, String), bool>>,
}

#[derive(Debug, Clone)]
pub struct FieldPlanEntry {
    pub fdef: &'static FieldDef,
    pub key: String,
}

impl NifSchema {
    pub fn from_generated() -> Self {
        let mut basics = HashMap::with_capacity(BASIC_TYPES.len());
        for b in BASIC_TYPES {
            basics.insert(b.name, b);
        }
        let mut enums = HashMap::with_capacity(ENUM_TYPES.len());
        for e in ENUM_TYPES {
            enums.insert(e.name, e);
        }
        let mut bitflags = HashMap::with_capacity(BITFLAG_TYPES.len());
        for b in BITFLAG_TYPES {
            bitflags.insert(b.name, b);
        }
        let mut bitfields = HashMap::with_capacity(BITFIELD_TYPES.len());
        for b in BITFIELD_TYPES {
            bitfields.insert(b.name, b);
        }
        let mut structs = HashMap::with_capacity(STRUCT_TYPES.len());
        for s in STRUCT_TYPES {
            structs.insert(s.name, s);
        }
        let mut niobjects = HashMap::with_capacity(NIOBJECT_TYPES.len());
        for n in NIOBJECT_TYPES {
            niobjects.insert(n.name, n);
        }
        Self {
            basics,
            enums,
            bitflags,
            bitfields,
            structs,
            niobjects,
            hierarchy_cache: Mutex::new(HashMap::new()),
            all_fields_cache: Mutex::new(HashMap::new()),
            all_field_plan_cache: Mutex::new(HashMap::new()),
            is_subtype_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn get_basic(&self, name: &str) -> Option<&BasicTypeDef> {
        self.basics.get(name).copied()
    }
    pub fn get_enum(&self, name: &str) -> Option<&EnumTypeDef> {
        self.enums.get(name).copied()
    }
    pub fn get_bitflag(&self, name: &str) -> Option<&BitflagTypeDef> {
        self.bitflags.get(name).copied()
    }
    pub fn get_bitfield(&self, name: &str) -> Option<&BitfieldTypeDef> {
        self.bitfields.get(name).copied()
    }
    pub fn get_struct(&self, name: &str) -> Option<&StructTypeDef> {
        self.structs.get(name).copied()
    }
    pub fn get_niobject(&self, name: &str) -> Option<&NiObjectTypeDef> {
        self.niobjects.get(name).copied()
    }

    pub fn is_known_type(&self, name: &str) -> bool {
        self.basics.contains_key(name)
            || self.enums.contains_key(name)
            || self.bitflags.contains_key(name)
            || self.bitfields.contains_key(name)
            || self.structs.contains_key(name)
            || self.niobjects.contains_key(name)
    }

    pub fn get_type_hierarchy(&self, type_name: &str) -> Vec<String> {
        if let Some(cached) = self.hierarchy_cache.lock().unwrap().get(type_name).cloned() {
            return cached;
        }
        let mut chain: Vec<String> = Vec::new();
        let mut current: Option<&str> = Some(type_name);
        while let Some(cur) = current {
            if chain.iter().any(|s| s == cur) {
                break;
            }
            chain.push(cur.to_string());
            current = self.niobjects.get(cur).and_then(|n| n.parent);
        }
        self.hierarchy_cache
            .lock()
            .unwrap()
            .insert(type_name.to_string(), chain.clone());
        chain
    }

    pub fn is_subtype_of(&self, type_name: &str, base: &str) -> bool {
        let key = (type_name.to_string(), base.to_string());
        if let Some(&cached) = self.is_subtype_cache.lock().unwrap().get(&key) {
            return cached;
        }
        let result = self.get_type_hierarchy(type_name).iter().any(|s| s == base);
        self.is_subtype_cache.lock().unwrap().insert(key, result);
        result
    }

    pub fn get_all_fields(&self, type_name: &str) -> Vec<&'static FieldDef> {
        if let Some(cached) = self
            .all_fields_cache
            .lock()
            .unwrap()
            .get(type_name)
            .cloned()
        {
            return cached;
        }
        let result: Vec<&'static FieldDef> = if let Some(obj) = self.niobjects.get(type_name) {
            let mut fields: Vec<&'static FieldDef> = Vec::new();
            if let Some(parent) = obj.parent {
                fields.extend(self.get_all_fields(parent));
            }
            for f in obj.fields.iter() {
                fields.push(f);
            }
            fields
        } else if let Some(s) = self.structs.get(type_name) {
            s.fields.iter().collect()
        } else {
            Vec::new()
        };
        self.all_fields_cache
            .lock()
            .unwrap()
            .insert(type_name.to_string(), result.clone());
        result
    }

    pub fn get_all_field_plan(&self, type_name: &str) -> Arc<Vec<FieldPlanEntry>> {
        if let Some(cached) = self
            .all_field_plan_cache
            .lock()
            .unwrap()
            .get(type_name)
            .cloned()
        {
            return cached;
        }

        let result: Arc<Vec<FieldPlanEntry>> = Arc::new(
            self.get_all_fields(type_name)
                .into_iter()
                .map(|fdef| {
                    let key = if let Some(sfx) = fdef.suffix {
                        format!("{}:{}", fdef.name, sfx)
                    } else {
                        fdef.name.to_string()
                    };
                    FieldPlanEntry { fdef, key }
                })
                .collect(),
        );
        self.all_field_plan_cache
            .lock()
            .unwrap()
            .insert(type_name.to_string(), result.clone());
        result
    }
}

pub static SCHEMA: Lazy<NifSchema> = Lazy::new(NifSchema::from_generated);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_sane() {
        let s = NifSchema::from_generated();
        assert!(s.basics.len() >= 20, "basics: {}", s.basics.len());
        assert!(s.enums.len() >= 100, "enums: {}", s.enums.len());
        assert!(s.bitflags.len() >= 20, "bitflags: {}", s.bitflags.len());
        assert!(s.bitfields.len() >= 10, "bitfields: {}", s.bitfields.len());
        assert!(s.structs.len() >= 150, "structs: {}", s.structs.len());
        assert!(s.niobjects.len() >= 400, "niobjects: {}", s.niobjects.len());
    }

    #[test]
    fn bstriangle_has_parent() {
        let s = NifSchema::from_generated();
        let obj = s.get_niobject("BSTriShape").expect("BSTriShape not found");
        assert_eq!(obj.parent, Some("NiAVObject"));
    }

    #[test]
    fn all_fields_includes_inherited() {
        let s = NifSchema::from_generated();
        let fields = s.get_all_fields("BSTriShape");
        assert!(fields.iter().any(|f| f.name == "Vertex Desc"));
        // Inherited from NiAVObject / NiObjectNET chain -- must include Name.
        assert!(
            fields.iter().any(|f| f.name == "Name"),
            "expected inherited 'Name' field"
        );
    }

    #[test]
    fn hierarchy_chain() {
        let s = NifSchema::from_generated();
        let chain = s.get_type_hierarchy("BSTriShape");
        assert_eq!(chain.first().map(String::as_str), Some("BSTriShape"));
        assert!(chain.iter().any(|s| s == "NiAVObject"));
        assert!(chain.iter().any(|s| s == "NiObject"));
    }

    #[test]
    fn is_subtype() {
        let s = NifSchema::from_generated();
        assert!(s.is_subtype_of("BSTriShape", "NiObject"));
        assert!(s.is_subtype_of("BSTriShape", "BSTriShape"));
        assert!(!s.is_subtype_of("BSTriShape", "NiAlphaProperty"));
    }

    #[test]
    fn token_expansion_in_fields() {
        // BSTriShape has vercond="#BS_GTE_F76#" on Bounding Box which should be expanded
        let s = NifSchema::from_generated();
        let obj = s.get_niobject("BSTriShape").unwrap();
        let bb = obj
            .fields
            .iter()
            .find(|f| f.name == "Bounding Box")
            .expect("Bounding Box field");
        let vercond = bb.vercond.expect("vercond set");
        assert!(
            !vercond.contains('#'),
            "tokens not expanded in vercond: {}",
            vercond
        );
        assert!(
            vercond.contains(">="),
            "expected >= operator in vercond: {}",
            vercond
        );
    }

    #[test]
    fn basic_uint_present() {
        let s = NifSchema::from_generated();
        let u = s.get_basic("uint").expect("uint basic");
        assert_eq!(u.size, 4);
        assert!(u.integral);
    }
}
