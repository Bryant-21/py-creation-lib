/// Resolved Data directories and output directory for a LOD generation run.
#[derive(Clone, Debug)]
pub struct LodPaths {
    pub data_dirs: Vec<std::path::PathBuf>,
    pub output_dir: std::path::PathBuf,
    pub source_data_dir: Option<std::path::PathBuf>,
}

/// Callback trait for progress reporting during LOD generation.
pub trait Progress {
    fn report(&mut self, msg: &str, frac: f32);
}

/// Accumulated output statistics for a completed LOD generation run.
#[derive(Default, Debug, Clone)]
pub struct LodGenStats {
    pub btr: u32,
    pub bto: u32,
    pub btt: u32,
    pub dds: u32,
    pub lod_written: bool,
    pub warnings: Vec<String>,
}

/// Output file lists produced by a single quad generation pass.
#[derive(Default, Debug)]
pub struct QuadOutputs {
    pub meshes: Vec<std::path::PathBuf>,
    pub textures: Vec<std::path::PathBuf>,
    pub object_lod: Option<ObjectQuadTelemetry>,
}

/// Mesh simplification counters for one object quad or aggregate.
#[derive(Default, Debug, Clone)]
pub struct ObjectSimplifyStats {
    pub shapes_considered: u64,
    pub shapes_skipped: u64,
    pub shapes_simplified: u64,
    pub attr_simplifier_count: u64,
    pub sloppy_count: u64,
    pub budget_clamp_count: u64,
    pub triangles_before: u64,
    pub triangles_after: u64,
}

impl ObjectSimplifyStats {
    pub fn add(&mut self, other: &ObjectSimplifyStats) {
        self.shapes_considered += other.shapes_considered;
        self.shapes_skipped += other.shapes_skipped;
        self.shapes_simplified += other.shapes_simplified;
        self.attr_simplifier_count += other.attr_simplifier_count;
        self.sloppy_count += other.sloppy_count;
        self.budget_clamp_count += other.budget_clamp_count;
        self.triangles_before += other.triangles_before;
        self.triangles_after += other.triangles_after;
    }

    pub fn triangles_saved(&self) -> u64 {
        self.triangles_before.saturating_sub(self.triangles_after)
    }
}

/// Per-model object LOD contribution used for diagnostics.
#[derive(Default, Debug, Clone)]
pub struct ObjectModelTelemetry {
    pub model: String,
    pub shape_count: u64,
    pub triangles_before: u64,
    pub triangles_after: u64,
}

impl ObjectModelTelemetry {
    pub fn triangles_saved(&self) -> u64 {
        self.triangles_before.saturating_sub(self.triangles_after)
    }
}

/// Object LOD diagnostics for one BTO quad.
#[derive(Default, Debug, Clone)]
pub struct ObjectQuadTelemetry {
    pub level: i32,
    pub x: i32,
    pub y: i32,
    pub bto_path: Option<std::path::PathBuf>,
    pub bto_bytes: u64,
    pub output_shape_count: u64,
    pub shape_build_secs: f64,
    pub write_report: crate::output::bto::BtoWriteReport,
    pub simplify: ObjectSimplifyStats,
    pub models: Vec<ObjectModelTelemetry>,
}

/// Per-quad execution context passed to each terrain/object/tree generator.
pub struct QuadCtx<'a> {
    pub world: &'a crate::input::WorldspaceInput,
    pub settings: &'a crate::settings::LodSettings,
    pub game: &'a crate::game::Game,
    pub paths: &'a LodPaths,
    pub level: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CollectProgress {
        events: Vec<(String, f32)>,
    }
    impl Progress for CollectProgress {
        fn report(&mut self, msg: &str, frac: f32) {
            self.events.push((msg.to_string(), frac));
        }
    }

    #[test]
    fn progress_trait_collects() {
        let mut p = CollectProgress { events: Vec::new() };
        p.report("terrain 16/-25/-11", 0.5);
        assert_eq!(p.events.len(), 1);
        assert_eq!(p.events[0].1, 0.5);
    }

    #[test]
    fn stats_default_is_empty() {
        let s = LodGenStats::default();
        assert_eq!(s.btr, 0);
        assert!(!s.lod_written);
        assert!(s.warnings.is_empty());
    }
}
