use flate2::read::GzDecoder;
use std::io::Read;
use std::sync::OnceLock;

pub struct SchemaModule {
    pub game: &'static str,
    compressed_json: &'static [u8],
    authoring_schema_json: OnceLock<String>,
}

impl SchemaModule {
    const fn new(game: &'static str, compressed_json: &'static [u8]) -> Self {
        Self {
            game,
            compressed_json,
            authoring_schema_json: OnceLock::new(),
        }
    }

    fn json(&self) -> &str {
        self.authoring_schema_json.get_or_init(|| {
            let mut json = String::new();
            GzDecoder::new(self.compressed_json)
                .read_to_string(&mut json)
                .expect("build-generated schema must decompress to UTF-8 JSON");
            json
        })
    }
}

static SUPPORTED_SCHEMAS: [SchemaModule; 7] = [
    SchemaModule::new(
        "oblivion",
        include_bytes!(concat!(env!("OUT_DIR"), "/oblivion.json.gz")),
    ),
    SchemaModule::new(
        "fo3",
        include_bytes!(concat!(env!("OUT_DIR"), "/fo3.json.gz")),
    ),
    SchemaModule::new(
        "fnv",
        include_bytes!(concat!(env!("OUT_DIR"), "/fnv.json.gz")),
    ),
    SchemaModule::new(
        "fo4",
        include_bytes!(concat!(env!("OUT_DIR"), "/fo4.json.gz")),
    ),
    SchemaModule::new(
        "skyrimse",
        include_bytes!(concat!(env!("OUT_DIR"), "/skyrimse.json.gz")),
    ),
    SchemaModule::new(
        "fo76",
        include_bytes!(concat!(env!("OUT_DIR"), "/fo76.json.gz")),
    ),
    SchemaModule::new(
        "starfield",
        include_bytes!(concat!(env!("OUT_DIR"), "/starfield.json.gz")),
    ),
];

pub fn supported_games() -> Vec<&'static str> {
    SUPPORTED_SCHEMAS.iter().map(|schema| schema.game).collect()
}

pub fn schema_json_for_game(game: &str) -> Option<&'static str> {
    let normalized = game.trim();
    if normalized.is_empty() {
        return None;
    }

    SUPPORTED_SCHEMAS
        .iter()
        .find(|schema| schema.game.eq_ignore_ascii_case(normalized))
        .map(SchemaModule::json)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{schema_json_for_game, supported_games};

    #[test]
    fn supported_games_are_unique() {
        let games = supported_games();
        let unique: BTreeSet<_> = games.iter().copied().collect();
        assert_eq!(games.len(), unique.len());
    }

    #[test]
    fn schema_json_lookup_is_case_insensitive_and_trimmed() {
        let fo4 = schema_json_for_game(" FO4 ").expect("fo4 schema should exist");
        assert!(fo4.contains(r#""game": "fo4""#));
    }

    #[test]
    fn schema_json_lookup_rejects_unsupported_games() {
        assert!(schema_json_for_game("morrowind").is_none());
        assert!(schema_json_for_game("   ").is_none());
    }

    #[test]
    fn compressed_schemas_match_generated_sources() {
        for (game, source) in [
            ("oblivion", include_str!("../generated/oblivion.rs")),
            ("fo3", include_str!("../generated/fo3.rs")),
            ("fnv", include_str!("../generated/fnv.rs")),
            ("fo4", include_str!("../generated/fo4.rs")),
            ("skyrimse", include_str!("../generated/skyrimse.rs")),
            ("fo76", include_str!("../generated/fo76.rs")),
            ("starfield", include_str!("../generated/starfield.rs")),
        ] {
            let literal = source
                .split_once("AUTHORING_SCHEMA_JSON: &str = ")
                .unwrap()
                .1;
            let json = literal
                .split_once('"')
                .unwrap()
                .1
                .rsplit_once('"')
                .unwrap()
                .0;
            let expected = json.replace("\r\n", "\n");
            let actual = schema_json_for_game(game).unwrap();
            assert_eq!(actual, expected, "schema changed for {game}");
            assert!(std::ptr::eq(actual, schema_json_for_game(game).unwrap()));
        }
    }
}
