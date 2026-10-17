use super::*;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ArmorFamilyModel {
    pub armor_form_key: String,
    pub owner_form_key: String,
    pub source_subrecord_sig: String,
    pub source_path: String,
}

#[derive(Default)]
pub(super) struct ArmorFamilyIndex {
    addons: HashMap<String, Vec<String>>,
    models: HashMap<(String, String), Vec<String>>,
    seen_models: HashSet<String>,
}

impl ArmorFamilyIndex {
    pub fn add_plugin(&mut self, plugin: &ParsedPlugin) {
        if plugin.game.as_deref() != Some("starfield") {
            return;
        }
        let own: Arc<str> = Arc::from(plugin.plugin_name.as_str());
        self.add_items(&plugin.root_items, &own, &plugin.header.masters);
    }

    fn add_items(&mut self, items: &[ParsedItem], own: &Arc<str>, masters: &[String]) {
        for item in items {
            let record = match item {
                ParsedItem::Group(group) => {
                    self.add_items(&group.children, own, masters);
                    continue;
                }
                ParsedItem::Record(record) => record,
            };
            if !matches!(record.signature.as_str(), "ARMO" | "ARMA") {
                continue;
            }
            let key = resolve_form_id_to_form_key(record.form_id, own, masters).to_string();
            let fields = effective_subrecords_for_record(record);
            if record.signature.as_str() == "ARMO" {
                let addons = fields
                    .windows(2)
                    .filter(|pair| pair[0].signature == "INDX" && pair[1].signature == "MODL")
                    .filter_map(|pair| {
                        let raw = u32::from_le_bytes(pair[1].data.as_ref().try_into().ok()?);
                        (raw != 0)
                            .then(|| resolve_form_id_to_form_key(raw, own, masters).to_string())
                    })
                    .collect();
                self.addons.entry(key).or_insert(addons);
            } else {
                if !self.seen_models.insert(key.clone()) {
                    continue;
                }
                let mut models: HashMap<String, Vec<String>> = HashMap::new();
                for field in fields.iter() {
                    if !matches!(field.signature.as_str(), "MOD2" | "MOD3" | "MOD4" | "MOD5") {
                        continue;
                    }
                    let data = field.data.as_ref();
                    let end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
                    let Ok(path) = std::str::from_utf8(&data[..end]) else {
                        continue;
                    };
                    if !path.to_ascii_lowercase().ends_with(".nif") {
                        continue;
                    }
                    models
                        .entry(field.signature.to_string())
                        .or_default()
                        .push(path.to_string());
                }
                for (role, paths) in models {
                    self.models.entry((key.clone(), role)).or_insert(paths);
                }
            }
        }
    }

    pub fn companions(&self, consumer: &str, role: &str) -> Vec<ArmorFamilyModel> {
        if !matches!(role, "MOD2" | "MOD3" | "MOD4" | "MOD5") {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (armor, addons) in &self.addons {
            if !addons.iter().any(|addon| addon == consumer) {
                continue;
            }
            for owner in addons.iter().filter(|addon| addon.as_str() != consumer) {
                if let Some(paths) = self.models.get(&(owner.clone(), role.to_string())) {
                    out.extend(paths.iter().map(|path| ArmorFamilyModel {
                        armor_form_key: armor.clone(),
                        owner_form_key: owner.clone(),
                        source_subrecord_sig: role.to_string(),
                        source_path: path.clone(),
                    }));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(signature: &str, bytes: Vec<u8>) -> ParsedSubrecord {
        ParsedSubrecord {
            signature: signature.into(),
            data: Bytes::from(bytes),
            semantic_type: None,
        }
    }

    fn record(signature: &str, id: u32, subrecords: Vec<ParsedSubrecord>) -> ParsedItem {
        ParsedItem::Record(ParsedRecord {
            signature: signature.into(),
            form_id: id,
            flags: 0,
            version_control: 0,
            form_version: None,
            version2: None,
            subrecords,
            raw_payload: None,
            parse_error: None,
        })
    }

    #[test]
    fn starfield_armor_families_require_indexed_addons_and_exact_model_role() {
        let plugin = ParsedPlugin {
            plugin_name: "Starfield.esm".into(),
            file_path: String::new(),
            header_size: 0,
            header: ParsedPluginHeader::default_for_test(),
            game: Some("starfield".into()),
            root_items: vec![
                record(
                    "ARMO",
                    0x800,
                    vec![
                        field("INDX", 0u16.to_le_bytes().to_vec()),
                        field("MODL", 0x801u32.to_le_bytes().to_vec()),
                        field("INDX", 1u16.to_le_bytes().to_vec()),
                        field("MODL", 0x802u32.to_le_bytes().to_vec()),
                        field("MODL", 0x803u32.to_le_bytes().to_vec()),
                    ],
                ),
                record(
                    "ARMA",
                    0x802,
                    vec![
                        field("MOD3", b"Clothes/body_f.nif\0".to_vec()),
                        field("MOD5", b"Clothes/body_f_1st.nif\0".to_vec()),
                        field("MODL", 0x347Du32.to_le_bytes().to_vec()),
                    ],
                ),
                record(
                    "ARMA",
                    0x803,
                    vec![field("MOD3", b"Clothes/unrelated.nif\0".to_vec())],
                ),
            ],
        };
        let mut index = ArmorFamilyIndex::default();
        index.add_plugin(&plugin);
        let third = index.companions("Starfield.esm:000801", "MOD3");
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].source_path, "Clothes/body_f.nif");
        assert_eq!(third[0].armor_form_key, "Starfield.esm:000800");
        assert_eq!(
            index.companions("Starfield.esm:000801", "MOD5")[0].source_path,
            "Clothes/body_f_1st.nif"
        );
        assert!(index.companions("Starfield.esm:000801", "MOD2").is_empty());
        assert!(index.companions("Starfield.esm:000803", "MOD3").is_empty());
        assert!(index.companions("Starfield.esm:000801", "MODL").is_empty());
    }
}
