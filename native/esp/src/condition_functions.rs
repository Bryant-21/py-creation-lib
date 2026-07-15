#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CtdaParamKey {
    ParameterOneRecord,
    FirstParameter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CtdaFunctionMeta {
    pub(crate) game: &'static str,
    pub(crate) id: u16,
    pub(crate) name: &'static str,
    pub(crate) parameter_one_formkey: Option<CtdaParamKey>,
}

const CTDA_FUNCTIONS: &[CtdaFunctionMeta] = &[
    CtdaFunctionMeta {
        game: "fo4",
        id: 74,
        name: "GetGlobalValue",
        parameter_one_formkey: Some(CtdaParamKey::ParameterOneRecord),
    },
    CtdaFunctionMeta {
        game: "fo76",
        id: 74,
        name: "GetGlobalValue",
        parameter_one_formkey: Some(CtdaParamKey::ParameterOneRecord),
    },
    CtdaFunctionMeta {
        game: "fo4",
        id: 277,
        name: "GetBaseValue",
        parameter_one_formkey: Some(CtdaParamKey::ParameterOneRecord),
    },
    CtdaFunctionMeta {
        game: "fo76",
        id: 277,
        name: "GetBaseValue",
        parameter_one_formkey: Some(CtdaParamKey::ParameterOneRecord),
    },
    CtdaFunctionMeta {
        game: "starfield",
        id: 56,
        name: "GetQuestCompletedConditionData",
        parameter_one_formkey: Some(CtdaParamKey::FirstParameter),
    },
    CtdaFunctionMeta {
        game: "starfield",
        id: 543,
        name: "GetQuestCompletedConditionData",
        parameter_one_formkey: Some(CtdaParamKey::FirstParameter),
    },
];

pub(crate) fn infer_game_from_plugins<'a>(
    masters: impl IntoIterator<Item = &'a String>,
    plugin_name: &'a str,
) -> Option<&'static str> {
    std::iter::once(plugin_name)
        .chain(masters.into_iter().map(String::as_str))
        .find_map(|name| {
            let lower = name.to_ascii_lowercase();
            match lower.as_str() {
                "fallout4.esm" => Some("fo4"),
                "seventysix.esm" => Some("fo76"),
                "starfield.esm" => Some("starfield"),
                _ => None,
            }
        })
}

pub(crate) fn lookup_ctda_function(
    game: Option<&str>,
    function_id: u16,
) -> Option<&'static CtdaFunctionMeta> {
    let game = game?;
    CTDA_FUNCTIONS
        .iter()
        .find(|entry| entry.game == game && entry.id == function_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_reported_high_value_condition_functions() {
        assert_eq!(
            lookup_ctda_function(Some("fo4"), 277).map(|entry| entry.name),
            Some("GetBaseValue")
        );
        assert_eq!(
            lookup_ctda_function(Some("fo76"), 74).map(|entry| entry.name),
            Some("GetGlobalValue")
        );
        assert_eq!(
            lookup_ctda_function(Some("starfield"), 56).map(|entry| entry.name),
            Some("GetQuestCompletedConditionData")
        );
        assert_eq!(
            lookup_ctda_function(Some("starfield"), 543).map(|entry| entry.name),
            Some("GetQuestCompletedConditionData")
        );
    }

    #[test]
    fn infers_game_from_master_or_plugin_name() {
        assert_eq!(
            infer_game_from_plugins(["Fallout4.esm".to_string()].iter(), "Patch.esp"),
            Some("fo4")
        );
        assert_eq!(
            infer_game_from_plugins([].iter(), "Starfield.esm"),
            Some("starfield")
        );
    }
}
