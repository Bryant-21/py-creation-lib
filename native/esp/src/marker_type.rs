//! FO76 → FO4 map-marker icon-type translation (REFR.TNAM).
//!
//! FO76 stores the marker icon type as a `uint16` (values 0–113); FO4 stores it
//! as `struct:B,B` (a type byte + an unknown byte) whose valid icon range is
//! 0–80. The two enums share the first ~64 named icons but with SHIFTED indices:
//! FO76 omits the FO4-only icons (Diamond City, Bunker Hill, Faneuil Hall, Synth
//! Head, Prydwen), so a raw value copy is WRONG — e.g. FO76 16 (Camper) would
//! land on FO4 16 (Airfield). Worse, FO76 values > 80 overrun FO4's compass
//! icon table entirely and crash the game (OOB read in the HUD compass-marker
//! loop). Translation must therefore be by semantic name, with no passthrough.

/// FO4 marker icon used when a FO76 marker type has no faithful FO4 equivalent
/// (runtime/system markers at 106–113, plus any unknown value). 8 = "Natural
/// Landmark", a base-game icon guaranteed to render on any FO4 install. Change
/// this one constant to pick a different generic.
pub const FO4_GENERIC_MARKER_TYPE: u8 = 8;

/// Highest real FO4 marker icon (0–80 are CK icons; 81–99 are generator
/// placeholders with no art; > 80 is out of range and unsafe for the compass).
pub const FO4_MAX_MARKER_TYPE: u8 = 80;

/// Highest FO4 marker byte used by the custom-icon companion. The 42 FO76
/// region icons occupy 81..=122. These are ONLY safe in-game with the
/// `B21_FO76MapMarkers` F4SE plugin loaded — it extends the engine's native
/// marker-icon array (otherwise 81 entries) to cover this range and supplies
/// the injected SWF symbols. Without the companion, conversion MUST use the
/// default [`translate_marker_type_fo76_to_fo4`] (safe stock FO4 fallbacks) to
/// stay crash-safe.
pub const FO4_CUSTOM_MARKER_MAX: u8 = 122;

/// Single source of truth for the FO76 region icons (FO76 TNAM type 64..=105)
/// that have no FO4 equivalent: `(fo76_type, fo4_custom_byte, symbol_name)`.
///
/// The `symbol_name` is the SymbolClass export present in FO76's
/// `interface/mapmarkerlibrary.swf`; the FO4 byte is also the SWF symbol-order
/// index used by the schema enum (`wbDefinitionsFO4.pas`) and the F4SE hook's
/// type→symbol table. Keep all three consumers aligned to THIS table.
/// FO76 106+ (Door/Quest/PlayerSet/Waypoint/Teammate…) are runtime/system
/// markers and are intentionally absent — they fall back to the generic icon.
pub const FO76_CUSTOM_ICONS: &[(u16, u8, &str)] = &[
    (64, 81, "TrainStationMarker"),
    (65, 82, "ElectricalSubstationMarker"),
    (66, 83, "FissureMarker"),
    (67, 84, "Vault63Marker"),
    (68, 85, "Vault76Marker"),
    (69, 86, "Vault94Marker"),
    (70, 87, "Vault96Marker"),
    (71, 88, "AmusementParkMarker"),
    (72, 89, "MansionMarker"),
    (73, 90, "ArktosPharmaMarker"),
    (74, 91, "PowerPlantMarker"),
    (75, 92, "SkiResortMarker"),
    (76, 93, "AppalachianAntiquesMarker"),
    (77, 94, "TeapotMarker"),
    (78, 95, "AgriculturalCenterMarker"),
    (79, 96, "WoodShackMarker"),
    (80, 97, "HouseTrailerMarker"),
    (81, 98, "LookoutTowerMarker"),
    (82, 99, "OverlookMarker"),
    (83, 100, "PumpkinMarker"),
    (84, 101, "CowSpotsCreameryMarker"),
    (85, 102, "CabinMarker"),
    (86, 103, "TrainTrackMark"),
    (87, 104, "CapitalBuildingMarker"),
    (88, 105, "HighTechBuildingMarker"),
    (89, 106, "LighthouseMarker"),
    (90, 107, "ExcavatorMarker"),
    (91, 108, "SpaceStationMarker"),
    (92, 109, "PalaceWindingPathMarker"),
    (93, 110, "TopOfTheWorldMarker"),
    (94, 111, "DamMarker"),
    (95, 112, "MonorailMarker"),
    (96, 113, "WhitespringResort"),
    (97, 114, "NukaColaQuantumPlant"),
    (98, 115, "MysteriousGuidestoneMarker"),
    (99, 116, "SkullRingMarker"),
    (100, 117, "HammerWingMarker"),
    (101, 118, "CultistMarker"),
    (102, 119, "BloodEagleMarker"),
    (103, 120, "Vault79Marker"),
    (104, 121, "BoSBaseMarker"),
    (105, 122, "LegendaryPurveyorMarker"),
];

/// FO4 SymbolClass export name for an injected marker when it must DIFFER from the
/// FO76 source symbol to avoid colliding with an existing FO4 export. Only Monorail
/// collides: FO4 already exports `MonorailMarker` for its Nuka-World monorail (icon
/// 73), so the FO76 Whitespring monorail (icon 112) is injected under a distinct
/// name. Keying the schema label (`Whitespring Monorail`) and the F4SE hook off this
/// export keeps byte 73 and byte 112 rendering independently.
pub const FO4_MARKER_EXPORT_OVERRIDES: &[(u8, &str)] = &[(112, "WhitespringMonorailMarker")];

/// The FO4 SymbolClass export name to inject a marker under: the override for a
/// colliding byte, else the FO76 source symbol unchanged.
pub fn fo4_marker_export_name<'a>(fo4_byte: u8, source_symbol: &'a str) -> &'a str {
    FO4_MARKER_EXPORT_OVERRIDES
        .iter()
        .find(|&&(b, _)| b == fo4_byte)
        .map(|&(_, name)| name)
        .unwrap_or(source_symbol)
}

/// Translate a FO76 REFR map-marker type (TNAM, uint16) to a stock FO4 marker
/// type byte (0–80). FO76 0–63 map by name to their FO4 icon; FO76 64–105
/// (Appalachia-specific location icons) map to the closest safe FO4 stock icon;
/// FO76 106–113 (runtime/system markers) and any unknown value fall back to
/// [`FO4_GENERIC_MARKER_TYPE`]. The result is always a valid FO4 icon, so the
/// game's compass-marker lookup can never index out of bounds.
pub fn translate_marker_type_fo76_to_fo4(fo76_type: u16) -> u8 {
    match fo76_type {
        0 => 0,    // cave
        1 => 1,    // city
        2 => 3,    // encampment
        3 => 4,    // factory / industrial site
        4 => 5,    // gov't building / monument
        5 => 6,    // metro station
        6 => 7,    // military base
        7 => 8,    // natural landmark
        8 => 9,    // office / civic building
        9 => 10,   // ruins (town)
        10 => 11,  // ruins (urban)
        11 => 12,  // sanctuary
        12 => 13,  // settlement
        13 => 14,  // sewer / utility tunnels
        14 => 15,  // vault
        15 => 16,  // airfield
        16 => 18,  // camper
        17 => 19,  // car
        18 => 20,  // church
        19 => 21,  // country club
        20 => 22,  // custom house
        21 => 23,  // drive-in
        22 => 24,  // elevated highway
        23 => 26,  // farm
        24 => 27,  // filling station
        25 => 28,  // forested
        26 => 29,  // goodneighbor
        27 => 30,  // graveyard
        28 => 31,  // hospital
        29 => 32,  // industrial dome
        30 => 33,  // industrial stacks
        31 => 34,  // institute
        32 => 35,  // irish pride
        33 => 36,  // junkyard
        34 => 37,  // observatory
        35 => 38,  // pier
        36 => 39,  // pond / lake
        37 => 40,  // quarry
        38 => 41,  // radioactive area
        39 => 42,  // radio tower
        40 => 43,  // salem
        41 => 44,  // school
        42 => 45,  // shipwreck
        43 => 46,  // submarine
        44 => 47,  // swan pond
        45 => 49,  // town
        46 => 50,  // brotherhood of steel
        47 => 51,  // brownstone townhouse
        48 => 52,  // bunker
        49 => 53,  // castle
        50 => 54,  // skyscraper
        51 => 55,  // libertalia
        52 => 56,  // low-rise building
        53 => 57,  // minutemen
        54 => 58,  // police station
        55 => 60,  // railroad faction
        56 => 61,  // railroad
        57 => 62,  // satellite
        58 => 63,  // sentinel
        59 => 64,  // uss constitution
        60 => 65,  // mechanist lair
        61 => 66,  // raider settlement
        62 => 67,  // vassal settlement
        63 => 68,  // potential vassal settlement
        64 => 6,   // train station -> metro station
        65 => 4,   // electrical substation -> industrial site
        66 => 41,  // fissure -> radioactive area
        67 => 15,  // vault63 -> vault
        68 => 15,  // vault76 -> vault
        69 => 15,  // vault94 -> vault
        70 => 15,  // vault96 -> vault
        71 => 74,  // amusement park -> rides
        72 => 21,  // mansion -> country club
        73 => 4,   // arktos pharma -> industrial site
        74 => 33,  // power plant -> industrial stacks
        75 => 8,   // ski resort -> natural landmark
        76 => 9,   // appalachian antiques -> office/civic building
        77 => 5,   // teapot -> monument
        78 => 26,  // agricultural center -> farm
        79 => 13,  // wood shack -> settlement
        80 => 18,  // house trailer -> camper
        81 => 42,  // lookout tower -> radio tower
        82 => 8,   // overlook -> natural landmark
        83 => 26,  // pumpkin -> farm
        84 => 26,  // cow spots creamery -> farm
        85 => 13,  // cabin -> settlement
        86 => 6,   // train track -> metro station
        87 => 5,   // capital building -> gov't building / monument
        88 => 34,  // high-tech building -> institute
        89 => 8,   // lighthouse -> natural landmark
        90 => 40,  // excavator -> quarry
        91 => 62,  // space station -> satellite
        92 => 5,   // palace winding path -> monument
        93 => 37,  // top of the world -> observatory
        94 => 5,   // dam -> monument
        95 => 73,  // monorail
        96 => 21,  // whitespring resort -> country club
        97 => 69,  // nuka-cola quantum plant -> bottling plant
        98 => 5,   // mysterious guidestone -> monument
        99 => 8,   // skull ring -> natural landmark
        100 => 8,  // hammer wing -> natural landmark
        101 => 12, // cultist -> sanctuary
        102 => 66, // blood eagle -> raider settlement
        103 => 15, // vault79 -> vault
        104 => 50, // bos base -> brotherhood of steel
        105 => 9,  // legendary purveyor -> office/civic building
        // 106–113: runtime/system markers (door, quest, player, teammate,
        // waypoint, etc.) have no stable FO4 location icon -> generic.
        _ => FO4_GENERIC_MARKER_TYPE,
    }
}

/// Custom-icon translation: like [`translate_marker_type_fo76_to_fo4`] for the
/// shared icons (FO76 0–63), but maps the 42 FO76 region icons (64–105) to
/// their assigned FO4 custom bytes (81–122, see [`FO76_CUSTOM_ICONS`]) instead
/// of collapsing them to the generic icon. FO76 106+ and unknown values still
/// fall back to [`FO4_GENERIC_MARKER_TYPE`].
///
/// Outputs in 81–122 are out of range for the stock engine and CTD without the
/// `B21_FO76MapMarkers` companion loaded — use this variant ONLY when the
/// conversion is producing/deploying that companion.
pub fn translate_marker_type_fo76_to_fo4_custom(fo76_type: u16) -> u8 {
    if let Some(&(_, fo4, _)) = FO76_CUSTOM_ICONS
        .iter()
        .find(|&&(src, _, _)| src == fo76_type)
    {
        return fo4;
    }
    translate_marker_type_fo76_to_fo4(fo76_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_icons_map_by_name_not_index() {
        // The whole point: indices are shifted, so these must NOT be identity.
        assert_eq!(translate_marker_type_fo76_to_fo4(2), 3); // encampment
        assert_eq!(translate_marker_type_fo76_to_fo4(16), 18); // camper, not airfield(16)
        assert_eq!(translate_marker_type_fo76_to_fo4(46), 50); // brotherhood of steel
        assert_eq!(translate_marker_type_fo76_to_fo4(59), 64); // uss constitution
        assert_eq!(translate_marker_type_fo76_to_fo4(63), 68); // potential vassal settlement
    }

    #[test]
    fn appalachia_markers_map_to_safe_stock_fallbacks() {
        assert_eq!(translate_marker_type_fo76_to_fo4(64), 6); // TrainStation -> Metro
        assert_eq!(translate_marker_type_fo76_to_fo4(66), 41); // Fissure -> Radioactive
        assert_eq!(translate_marker_type_fo76_to_fo4(67), 15); // Vault63 -> Vault
        assert_eq!(translate_marker_type_fo76_to_fo4(68), 15); // Vault76 -> Vault
        assert_eq!(translate_marker_type_fo76_to_fo4(103), 15); // Vault79 -> Vault
        assert_eq!(translate_marker_type_fo76_to_fo4(104), 50); // BoSBase -> Brotherhood
        for t in 64u16..=105 {
            assert!(translate_marker_type_fo76_to_fo4(t) <= FO4_MAX_MARKER_TYPE);
        }
    }

    #[test]
    fn runtime_markers_fall_back_to_generic() {
        for t in 106u16..=113 {
            assert_eq!(
                translate_marker_type_fo76_to_fo4(t),
                FO4_GENERIC_MARKER_TYPE
            );
        }
        // The crashing values observed in-game (0x55, 0x66, 0x76, …) all land here.
        assert_eq!(
            translate_marker_type_fo76_to_fo4(0x76),
            FO4_GENERIC_MARKER_TYPE
        );
        assert_eq!(
            translate_marker_type_fo76_to_fo4(9999),
            FO4_GENERIC_MARKER_TYPE
        );
    }

    #[test]
    fn every_output_is_a_valid_fo4_icon() {
        // Crash-safety invariant: no output may exceed FO4's icon table.
        for t in 0u16..=300 {
            assert!(translate_marker_type_fo76_to_fo4(t) <= FO4_MAX_MARKER_TYPE);
        }
    }

    #[test]
    fn custom_table_is_well_formed() {
        // 42 region icons, FO76 64..=105 contiguous, FO4 81..=122 contiguous,
        // every output within the companion's extended array bound.
        assert_eq!(FO76_CUSTOM_ICONS.len(), 42);
        for (i, &(src, fo4, _name)) in FO76_CUSTOM_ICONS.iter().enumerate() {
            assert_eq!(src as usize, 64 + i, "fo76 type not contiguous");
            assert_eq!(fo4 as usize, 81 + i, "fo4 byte not contiguous");
            assert!(fo4 <= FO4_CUSTOM_MARKER_MAX);
            assert!(
                fo4 > FO4_MAX_MARKER_TYPE,
                "custom byte must exceed stock range"
            );
        }
    }

    #[test]
    fn custom_mode_renders_region_icons_not_generic() {
        // The whole point of the companion: 64..=105 get real bytes, not generic.
        assert_eq!(translate_marker_type_fo76_to_fo4_custom(64), 81); // TrainStation
        assert_eq!(translate_marker_type_fo76_to_fo4_custom(67), 84); // Vault63
        assert_eq!(translate_marker_type_fo76_to_fo4_custom(96), 113); // Whitespring
        assert_eq!(translate_marker_type_fo76_to_fo4_custom(105), 122); // LegendaryPurveyor
        // Shared icons (0–63) behave identically to the safe translator.
        assert_eq!(
            translate_marker_type_fo76_to_fo4_custom(16),
            translate_marker_type_fo76_to_fo4(16)
        );
        // Runtime/system markers (106+) and unknown still go generic.
        assert_eq!(
            translate_marker_type_fo76_to_fo4_custom(107),
            FO4_GENERIC_MARKER_TYPE
        );
        assert_eq!(
            translate_marker_type_fo76_to_fo4_custom(122),
            FO4_GENERIC_MARKER_TYPE
        );
        assert_eq!(
            translate_marker_type_fo76_to_fo4_custom(9999),
            FO4_GENERIC_MARKER_TYPE
        );
    }

    #[test]
    fn export_overrides_are_unique_and_only_for_collisions() {
        use std::collections::HashSet;
        // Every injected FO4 export name (override or source symbol) is unique, so
        // no two markers ever share a SymbolClass export — the resolved icon is
        // unambiguous regardless of how the engine binds type->icon.
        let mut exports: HashSet<&str> = HashSet::new();
        for &(_src, fo4, symbol) in FO76_CUSTOM_ICONS {
            let export = fo4_marker_export_name(fo4, symbol);
            assert!(exports.insert(export), "duplicate FO4 export name {export}");
        }
        // The Monorail override specifically renames icon 112 away from the FO76
        // source symbol (which collides with FO4's native Nuka-World MonorailMarker).
        assert_eq!(
            fo4_marker_export_name(112, "MonorailMarker"),
            "WhitespringMonorailMarker"
        );
        // Non-colliding bytes pass the source symbol through unchanged.
        assert_eq!(
            fo4_marker_export_name(81, "TrainStationMarker"),
            "TrainStationMarker"
        );
    }

    #[test]
    fn default_translator_stays_crash_safe() {
        // Regression guard: the DEFAULT path must never emit the custom 81+ bytes
        // used by the companion, so conversions without it never read OOB.
        for t in 0u16..=300 {
            assert!(translate_marker_type_fo76_to_fo4(t) <= FO4_MAX_MARKER_TYPE);
        }
    }
}
