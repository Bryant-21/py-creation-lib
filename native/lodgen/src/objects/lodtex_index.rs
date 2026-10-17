//! `Textures\Terrain\<W>\Objects\<W>.lodtex.txt`: per-object LOD textures for instanced groups (plan Task 28),
//! read by Tales' `LodTextureIndex`. Each row carries the group's rect on the world atlas, since the shapes keep
//! atlas uvs and Tales maps them back to the object's own texture space.

#[derive(Clone, Debug, PartialEq)]
pub struct LodTexRow {
    pub object: u32,
    pub diffuse: String,
    pub normal: String,
    pub specular: String,
    /// atlas uv = model uv * scale + offset.
    pub atlas_offset: [f32; 2],
    pub atlas_scale: [f32; 2],
}

const HEADER: &str = "lodtex 1";

pub fn to_text(rows: &[LodTexRow]) -> String {
    let mut text = format!("{HEADER}\n");
    for r in rows {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            r.object, r.diffuse, r.normal, r.specular, r.atlas_offset[0], r.atlas_offset[1], r.atlas_scale[0],
            r.atlas_scale[1]
        ));
    }
    text
}

/// Strict, like Tales: a bad header or any malformed row rejects the whole index.
pub fn parse(text: &str) -> anyhow::Result<Vec<LodTexRow>> {
    let mut lines = text.lines();
    anyhow::ensure!(lines.next() == Some(HEADER), "lodtex: expected header {HEADER:?}");
    let mut rows = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let fields: Vec<&str> = line.split('\t').collect();
        anyhow::ensure!(fields.len() == 8 && fields[1..4].iter().all(|f| !f.is_empty()), "lodtex: bad row {line:?}");
        let float = |i: usize| -> anyhow::Result<f32> { Ok(fields[i].parse::<f32>()?) };
        let (offset, scale) = ([float(4)?, float(5)?], [float(6)?, float(7)?]);
        anyhow::ensure!(
            offset.iter().chain(&scale).all(|v| v.is_finite()) && scale.iter().all(|s| *s > 0.0),
            "lodtex: bad atlas rect in {line:?}"
        );
        rows.push(LodTexRow {
            object: fields[0].parse()?,
            diffuse: fields[1].into(),
            normal: fields[2].into(),
            specular: fields[3].into(),
            atlas_offset: offset,
            atlas_scale: scale,
        });
    }
    Ok(rows)
}

pub fn write(output_dir: &std::path::Path, world: &str, rows: &[LodTexRow]) -> anyhow::Result<std::path::PathBuf> {
    let dir = output_dir.join(format!("Textures/Terrain/{world}/Objects"));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{world}.lodtex.txt"));
    std::fs::write(&path, to_text(rows))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> LodTexRow {
        LodTexRow {
            object: 7,
            diffuse: "terrain\\w\\objects\\groups\\7.d.dds".into(),
            normal: "terrain\\w\\objects\\groups\\7.n.dds".into(),
            specular: "terrain\\w\\objects\\groups\\7.s.dds".into(),
            atlas_offset: [0.5, 0.25],
            atlas_scale: [0.0625, 0.125],
        }
    }

    #[test]
    fn lodtex_index_round_trip_and_rejects_bad_lines() {
        let rows = vec![row()];
        let text = to_text(&rows);
        assert!(text.starts_with("lodtex 1\n"));
        assert_eq!(parse(&text).unwrap(), rows);
        assert!(parse("lodtex 2\n").is_err());
        assert!(parse("lodtex 1\nnot\ta\trow\n").is_err());
        assert!(parse("lodtex 1\n7\ta\tb\tc\t0\t0\t0\t1\n").is_err());
    }

    #[test]
    fn written_index_lands_next_to_the_world_atlas() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "Appalachia", &[row()]).unwrap();
        assert!(path.ends_with("Textures/Terrain/Appalachia/Objects/Appalachia.lodtex.txt"));
        assert_eq!(parse(&std::fs::read_to_string(&path).unwrap()).unwrap(), vec![row()]);
    }
}
