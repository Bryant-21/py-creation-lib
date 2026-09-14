pub mod bto;
pub mod btr;
pub mod btt;
pub mod lodsettings;

fn triangle_values<T: Copy + Into<i64>>(triangles: &[[T; 3]]) -> Vec<nif_core_native::model::NifValue> {
    use nif_core_native::model::NifValue;
    triangles
        .iter()
        .map(|triangle| {
            NifValue::Array(
                triangle
                    .iter()
                    .map(|&index| NifValue::Int(index.into()))
                    .collect(),
            )
        })
        .collect()
}

#[cfg(test)]
fn assert_triangle_bytes_match_legacy(nif: &mut nif_core_native::model::NifFile) {
    use nif_core_native::model::NifValue;
    let compact = nif.to_bytes().unwrap();
    let mut triangles_checked = 0;
    for block in &mut nif.blocks {
        let Some(NifValue::Array(triangles)) = block.fields.get_mut("Triangles") else {
            continue;
        };
        for triangle in triangles {
            let NifValue::Array(indices) = triangle else {
                panic!("expected positional triangle")
            };
            let fields = ["v1", "v2", "v3"]
                .into_iter()
                .zip(indices.iter().cloned())
                .map(|(key, value)| (key.to_string(), value))
                .collect();
            *triangle = NifValue::Struct(fields);
            triangles_checked += 1;
        }
    }
    assert!(triangles_checked > 0);
    assert!(nif.to_bytes().unwrap() == compact);
}
