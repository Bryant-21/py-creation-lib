pub enum GameMode {
    Fo4,
}

pub struct Game {
    pub mode: GameMode,
}

impl Game {
    pub fn fo4() -> Self {
        Game {
            mode: GameMode::Fo4,
        }
    }

    /// Cell size in game units (4096)
    pub fn cell_size(&self) -> i32 {
        4096
    }

    /// Number of height posts per cell edge (33)
    pub fn posts_per_cell(&self) -> i32 {
        33
    }

    /// Game units per height post (128)
    pub fn units_per_post(&self) -> i32 {
        128
    }

    /// LOD levels [4, 8, 16, 32] (Game.cs:48)
    pub fn lod_levels(&self) -> [i32; 4] {
        [4, 8, 16, 32]
    }

    /// Maximum vertex count per LOD mesh (Game.cs:48)
    pub fn max_vertices(&self) -> u32 {
        65535
    }

    /// Maximum triangle count for FO4 (Game.cs:53-60)
    pub fn max_triangles(&self) -> u64 {
        4_294_967_295
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fo4_profile_constants() {
        let g = Game::fo4();
        assert!(matches!(g.mode, GameMode::Fo4));
        assert_eq!(g.cell_size(), 4096);
        assert_eq!(g.posts_per_cell(), 33);
        assert_eq!(g.units_per_post(), 128);
        assert_eq!(g.lod_levels(), [4, 8, 16, 32]);
        assert_eq!(g.max_vertices(), 65535);
        assert_eq!(g.max_triangles(), 4_294_967_295);
    }
}
