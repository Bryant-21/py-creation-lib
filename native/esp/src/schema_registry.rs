#[path = "../generated/fnv.rs"]
mod fnv;
#[path = "../generated/fo3.rs"]
mod fo3;
#[path = "../generated/fo4.rs"]
mod fo4;
#[path = "../generated/fo76.rs"]
mod fo76;
#[path = "../generated/oblivion.rs"]
mod oblivion;
#[path = "../generated/skyrimse.rs"]
mod skyrimse;
#[path = "../generated/starfield.rs"]
mod starfield;

#[derive(Clone, Copy)]
pub struct SchemaModule {
    pub game: &'static str,
    pub authoring_schema_json: &'static str,
}

impl SchemaModule {
    const fn new(game: &'static str, authoring_schema_json: &'static str) -> Self {
        Self {
            game,
            authoring_schema_json,
        }
    }
}

const SUPPORTED_SCHEMAS: &[SchemaModule] = &[
    SchemaModule::new(oblivion::GAME, oblivion::AUTHORING_SCHEMA_JSON),
    SchemaModule::new(fo3::GAME, fo3::AUTHORING_SCHEMA_JSON),
    SchemaModule::new(fnv::GAME, fnv::AUTHORING_SCHEMA_JSON),
    SchemaModule::new(fo4::GAME, fo4::AUTHORING_SCHEMA_JSON),
    SchemaModule::new(skyrimse::GAME, skyrimse::AUTHORING_SCHEMA_JSON),
    SchemaModule::new(fo76::GAME, fo76::AUTHORING_SCHEMA_JSON),
    SchemaModule::new(starfield::GAME, starfield::AUTHORING_SCHEMA_JSON),
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
        .map(|schema| schema.authoring_schema_json)
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
}
