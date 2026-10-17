//! Per-tile object LOD report: what was written and what was dropped.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileReport {
    pub level: u32,
    pub x: i32,
    pub y: i32,
    pub shapes_out: usize,
    pub shapes_cap_dropped: u64,
    pub shapes_atlas_dropped: u64,
    pub bto_bytes: u64,
    /// Instanced (shape, data) pairs and the instances they place (objects.source = fo76_bto_instanced).
    #[serde(default)]
    pub instanced_groups: usize,
    #[serde(default)]
    pub instanced_instances: u64,
    /// Instance transforms with zero or non-finite scale, skipped.
    #[serde(default)]
    pub degenerate: u64,
    /// Instances with no placed ref within 1 unit, or whose ref the engine can't hide by id; at L4 they stay merged
    /// geometry.
    #[serde(default)]
    pub unmatched_ids: u64,
}

/// A source tile that failed to convert: none of its objects reached the output.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedTile {
    pub level: i32,
    pub x: i32,
    pub y: i32,
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectReport {
    pub tiles: Vec<TileReport>,
    #[serde(default)]
    pub failed_tiles: Vec<FailedTile>,
}

impl ObjectReport {
    pub fn total_dropped(&self) -> u64 {
        self.tiles.iter().map(|t| t.shapes_cap_dropped + t.shapes_atlas_dropped).sum()
    }

    pub fn total_instanced(&self) -> (usize, u64) {
        self.tiles.iter().fold((0, 0), |(groups, instances), t| (groups + t.instanced_groups, instances + t.instanced_instances))
    }
}

pub fn write_object_report(out_dir: &Path, world: &str, report: &ObjectReport) -> anyhow::Result<PathBuf> {
    let dir = out_dir.join("LODGen");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{world}.objects.report.json"));
    std::fs::write(&path, serde_json::to_vec_pretty(report)?)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_round_trips_and_sums_drops() {
        let dir = tempfile::tempdir().unwrap();
        let report = ObjectReport {
            tiles: vec![
                TileReport { level: 4, x: 0, y: 0, shapes_out: 10, shapes_cap_dropped: 2, shapes_atlas_dropped: 1, bto_bytes: 100, ..TileReport::default() },
                TileReport { level: 8, x: 0, y: 0, shapes_out: 5, shapes_cap_dropped: 0, shapes_atlas_dropped: 3, bto_bytes: 50, ..TileReport::default() },
            ],
            failed_tiles: vec![FailedTile { level: 4, x: 1, y: 0, error: "bad nif".into() }],
        };
        let path = write_object_report(dir.path(), "APPALACHIA", &report).unwrap();
        assert!(path.ends_with("LODGen/APPALACHIA.objects.report.json"));
        let text = std::fs::read_to_string(&path).unwrap();
        let back: ObjectReport = serde_json::from_str(&text).unwrap();
        assert_eq!(back, report);
        assert_eq!(report.total_dropped(), 6);
    }

    #[test]
    fn reports_written_before_failed_tiles_still_parse() {
        let back: ObjectReport = serde_json::from_str(r#"{"tiles": []}"#).unwrap();
        assert!(back.failed_tiles.is_empty());
    }

    #[test]
    fn tiles_written_before_instancing_still_parse_and_instances_sum() {
        let old = r#"{"tiles": [{"level": 4, "x": 0, "y": 0, "shapes_out": 1, "shapes_cap_dropped": 0,
            "shapes_atlas_dropped": 0, "bto_bytes": 5}]}"#;
        let mut report: ObjectReport = serde_json::from_str(old).unwrap();
        assert_eq!(report.tiles[0].instanced_groups, 0);
        report.tiles[0].instanced_groups = 2;
        report.tiles[0].instanced_instances = 9;
        report.tiles.push(TileReport { instanced_groups: 1, instanced_instances: 3, ..TileReport::default() });
        assert_eq!(report.total_instanced(), (3, 12));
    }
}
