/// Per-game profile — semantic feature gates for the Papyrus compiler.
///
/// Every game-specific semantic decision routes through `GameProfile`; no
/// scattered `game == Fo4` literals outside this module.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Game {
    SkyrimSe,
    Fo4,
    Starfield,
}

/// Feature-gate table for one target game.
#[derive(Debug, Clone, Copy)]
pub struct GameProfile {
    pub game_id: u16,
    pub major_version: u8,
    pub minor_version: u8,
    /// Whether the game's Papyrus supports struct types.
    pub allow_structs: bool,
    /// Whether the game's Papyrus supports property groups.
    pub allow_groups: bool,
    /// Whether the game's Papyrus supports guard statements.
    pub allow_guards: bool,
}

impl GameProfile {
    pub fn for_game(game: Game) -> Self {
        match game {
            Game::SkyrimSe => Self {
                game_id: 1,
                major_version: 3,
                minor_version: 9,
                allow_structs: false,
                allow_groups: false,
                allow_guards: false,
            },
            Game::Fo4 => Self {
                game_id: 2,
                // PapyrusCompiler 2.8.0.4 stamps FO4 .pex headers as version 3.9.
                major_version: 3,
                minor_version: 9,
                allow_structs: true,
                allow_groups: true,
                allow_guards: false,
            },
            Game::Starfield => Self {
                game_id: 4,
                major_version: 3,
                minor_version: 2,
                allow_structs: true,
                allow_groups: true,
                allow_guards: true,
            },
        }
    }
}
