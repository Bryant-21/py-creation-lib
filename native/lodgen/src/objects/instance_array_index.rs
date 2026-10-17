//! `Textures\Terrain\<W>\Objects\<W>.InstanceArray.txt`: the instanced-atlas rect of each texture-array layer.
//! FO4CS maps a layered group's atlas uvs back to model uvs with `model = (atlas - offset) / scale`.
use std::path::{Path, PathBuf};

use crate::objects::instance_layers::UvRect;

const HEADER: &str = "lodarrays 1";
/// Width and height of every array slice.
pub const SLICE_SIZE: u32 = 256;

/// `<W>.InstanceArray<suffix>` next to the world atlas: `.dds`, `_n.dds`, `_s.dds` or `.txt`.
pub fn file(output_dir: &Path, world: &str, suffix: &str) -> PathBuf {
    output_dir.join(format!("Textures/Terrain/{world}/Objects/{world}.InstanceArray{suffix}"))
}

/// One row per layer, in layer order; floats print with `{}`, which round-trips an f32.
pub fn to_text(rects: &[UvRect]) -> String {
    let mut text = format!("{HEADER}\nsize {SLICE_SIZE}\nslices {}\n", rects.len());
    for (layer, r) in rects.iter().enumerate() {
        text.push_str(&format!("{layer}\t{}\t{}\t{}\t{}\n", r.offset[0], r.offset[1], r.scale[0], r.scale[1]));
    }
    text
}

/// Strict: the three header lines, then exactly `slices` rows numbered 0.., nothing else.
pub fn parse(text: &str) -> anyhow::Result<Vec<UvRect>> {
    let mut lines = text.split_terminator('\n');
    anyhow::ensure!(lines.next() == Some(HEADER), "lodarrays: expected header {HEADER:?}");
    anyhow::ensure!(lines.next() == Some(&format!("size {SLICE_SIZE}")[..]), "lodarrays: expected size {SLICE_SIZE}");
    let count: usize = lines
        .next()
        .and_then(|line| line.strip_prefix("slices "))
        .ok_or_else(|| anyhow::anyhow!("lodarrays: expected a slices line"))?
        .parse()?;
    let mut rects = Vec::with_capacity(count);
    for line in lines {
        let fields: Vec<&str> = line.split('\t').collect();
        anyhow::ensure!(fields.len() == 5, "lodarrays: bad row {line:?}");
        anyhow::ensure!(fields[0].parse::<usize>().ok() == Some(rects.len()), "lodarrays: row out of order {line:?}");
        let v: Vec<f32> = fields[1..].iter().map(|f| f.parse::<f32>()).collect::<Result<_, _>>()?;
        rects.push(UvRect { offset: [v[0], v[1]], scale: [v[2], v[3]] });
    }
    anyhow::ensure!(rects.len() == count, "lodarrays: {} rows for {count} slices", rects.len());
    Ok(rects)
}

pub fn write(output_dir: &Path, world: &str, rects: &[UvRect]) -> anyhow::Result<PathBuf> {
    let path = file(output_dir, world, ".txt");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, to_text(rects))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rects() -> Vec<UvRect> {
        vec![
            UvRect { offset: [0.25, 0.0625], scale: [0.0625, 0.03125] },
            UvRect { offset: [0.1, 1.0 / 3.0], scale: [1.0 / 4096.0, 0.7] },
        ]
    }

    #[test]
    fn index_text_is_exactly_the_contract_format() {
        let text = to_text(&rects()[..1]);
        assert_eq!(text, "lodarrays 1\nsize 256\nslices 1\n0\t0.25\t0.0625\t0.0625\t0.03125\n");
        assert_eq!(to_text(&[]), "lodarrays 1\nsize 256\nslices 0\n");
    }

    #[test]
    fn index_round_trips_every_float_and_rejects_anything_else() {
        let text = to_text(&rects());
        assert!(text.contains("\n1\t0.1\t0.33333334\t0.00024414063\t0.7\n"), "{text}");
        assert_eq!(parse(&text).unwrap(), rects());
        for bad in [
            "lodarrays 2\nsize 256\nslices 0\n",
            "lodarrays 1\nsize 128\nslices 0\n",
            "lodarrays 1\nsize 256\nslices 2\n0\t0\t0\t1\t1\n",
            "lodarrays 1\nsize 256\nslices 1\n1\t0\t0\t1\t1\n",
            "lodarrays 1\nsize 256\nslices 1\n0\t0\t0\t1\n",
            "lodarrays 1\nsize 256\nslices 1\n0\t0\t0\t1\t1\nextra\n",
            "lodarrays 1\r\nsize 256\r\nslices 0\r\n",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn index_and_arrays_land_next_to_the_world_atlas() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "Appalachia", &rects()).unwrap();
        assert!(path.ends_with("Textures/Terrain/Appalachia/Objects/Appalachia.InstanceArray.txt"));
        assert_eq!(parse(&std::fs::read_to_string(&path).unwrap()).unwrap(), rects());
        let normal = file(dir.path(), "Appalachia", "_n.dds");
        assert!(normal.ends_with("Textures/Terrain/Appalachia/Objects/Appalachia.InstanceArray_n.dds"));
    }
}
