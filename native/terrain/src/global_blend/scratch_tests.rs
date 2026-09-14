use super::*;
use std::time::Instant;

fn compare_quadrant(
    blend: &GlobalLandscapeBlend,
    x: i32,
    y: i32,
    quadrant: u8,
    reverse: bool,
) -> (f64, f64, usize) {
    let legacy = || {
        let start = Instant::now();
        let result = blend.serialize_quadrant_legacy(x, y, quadrant);
        (result, start.elapsed().as_secs_f64())
    };
    let current = || {
        let start = Instant::now();
        let result = blend.serialize_quadrant(x, y, quadrant);
        (result, start.elapsed().as_secs_f64())
    };
    let ((before, old_secs), (after, new_secs)) = if reverse {
        let after = current();
        (legacy(), after)
    } else {
        let before = legacy();
        (before, current())
    };
    let bytes = match (before, after) {
        (Ok(Some(before)), Ok(Some(after))) => {
            assert_eq!(
                before.base_source_ltex_object_id,
                after.base_source_ltex_object_id
            );
            assert_eq!(
                before.alpha_source_ltex_object_ids,
                after.alpha_source_ltex_object_ids
            );
            assert_eq!(
                before.dropped_source_ltex_object_ids,
                after.dropped_source_ltex_object_ids
            );
            assert_eq!(
                before.alpha_vtxt, after.alpha_vtxt,
                "cell {x},{y} quadrant {quadrant}"
            );
            before.alpha_vtxt.iter().map(Vec::len).sum()
        }
        (Ok(None), Ok(None)) => 0,
        (Err(before), Err(after)) => {
            assert_eq!(before, after);
            0
        }
        other => panic!("quadrant result differs: {other:?}"),
    };
    (old_secs, new_secs, bytes)
}

#[test]
fn scratch_buffers_preserve_sparse_dense_tied_and_empty_quadrants() {
    let width = 2 * LAND_CELL_INTERVALS + 1;
    let vertices = (0..width * width)
        .map(|position| {
            (1..=(position % 10) as u32)
                .map(|id| {
                    let weight = match position % 7 {
                        0 => 0.0,
                        1 => 1.0,
                        2 => -1.0,
                        _ => ((position * id as usize) % 37) as f32 / 37.0,
                    };
                    (id, weight)
                })
                .collect()
        })
        .collect();
    let blend = GlobalLandscapeBlend::from_vertices(-1, -1, 2, 2, vertices);
    for y in -1..=0 {
        for x in -1..=0 {
            for q in 0..4 {
                compare_quadrant(&blend, x, y, q, q % 2 == 0);
            }
        }
    }
    compare_quadrant(&blend, -2, 0, 0, false);
    compare_quadrant(&blend, 1, 0, 0, false);
    compare_quadrant(&blend, 0, 0, 4, false);
}

struct AlphaImages(HashMap<u32, (usize, usize, Vec<u8>)>);
impl SourceAlphaLookup for AlphaImages {
    fn sample_alpha(&self, id: u32, u: i32, v: i32) -> u8 {
        self.0
            .get(&id)
            .map(|(width, height, alpha)| {
                alpha[v.rem_euclid(*height as i32) as usize * width
                    + u.rem_euclid(*width as i32) as usize]
            })
            .unwrap_or(255)
    }
}

#[test]
#[ignore = "requires TERRAIN_SCRATCH_BTD, TERRAIN_SCRATCH_MANIFEST and TERRAIN_SCRATCH_REPORT"]
fn scratch_buffers_match_complete_btd_corpus() {
    let path = std::env::var("TERRAIN_SCRATCH_BTD").unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("TERRAIN_SCRATCH_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let mut images = AlphaImages(HashMap::new());
    for texture in manifest["textures"].as_array().unwrap() {
        let key = texture["source_ltex_form_key"].as_str().unwrap();
        let id = u32::from_str_radix(key.rsplit(':').next().unwrap(), 16).unwrap();
        if images.0.contains_key(&id) {
            continue;
        }
        let image = directxtex_native::read_dds_rgba_image(std::path::Path::new(
            texture["diffuse_path"].as_str().unwrap(),
        ))
        .unwrap();
        images.0.insert(
            id,
            (
                image.width as usize,
                image.height as usize,
                image.rgba.chunks_exact(4).map(|pixel| pixel[3]).collect(),
            ),
        );
    }
    let mut btd = BtdFile::open(&path).unwrap();
    let header = btd.header().clone();
    let started = Instant::now();
    let blend = GlobalLandscapeBlend::build(
        &mut btd,
        header.cell_min_x,
        header.cell_min_y,
        header.cells_x,
        header.cells_y,
        &images,
    )
    .unwrap();
    let build_secs = started.elapsed().as_secs_f64();
    eprintln!(
        "built {} cells in {build_secs:.3}s",
        header.cells_x * header.cells_y
    );
    let (mut legacy_secs, mut optimized_secs, mut output_bytes, mut quadrants) =
        (0.0, 0.0, 0usize, 0usize);
    for y in header.cell_min_y..=header.cell_max_y {
        for x in header.cell_min_x..=header.cell_max_x {
            for q in 0..4 {
                let (old, new, bytes) = compare_quadrant(&blend, x, y, q, quadrants % 2 == 0);
                legacy_secs += old;
                optimized_secs += new;
                output_bytes += bytes;
                quadrants += 1;
            }
        }
        if (y - header.cell_min_y) % 25 == 0 {
            eprintln!("checked {quadrants} quadrants");
        }
    }
    let report = serde_json::json!({"btd":path,"cells":header.cells_x*header.cells_y,"quadrants":quadrants,"alpha_bytes":output_bytes,"all_fields_equal":true,"alpha_textures":images.0.len(),"build_seconds":build_secs,"legacy_serialize_seconds":legacy_secs,"optimized_serialize_seconds":optimized_secs});
    std::fs::write(
        std::env::var("TERRAIN_SCRATCH_REPORT").unwrap(),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    eprintln!("{report}");
}

// Retain the pre-optimization encoder as a byte-equivalence oracle.
impl GlobalLandscapeBlend {
    fn serialize_quadrant_legacy(
        &self,
        cell_x: i32,
        cell_y: i32,
        quadrant: u8,
    ) -> Result<Option<QuadrantBlend>, String> {
        let mut totals = HashMap::<u32, u32>::new();
        let mut edge_ids = HashSet::<u32>::new();
        let mut quadrant_weights =
            Vec::with_capacity(LAND_QUADRANT_VERTICES * LAND_QUADRANT_VERTICES);
        let mut shared_edge_vertices =
            Vec::with_capacity(LAND_QUADRANT_VERTICES * LAND_QUADRANT_VERTICES);
        let cell_offset_x = usize::try_from(cell_x - self.min_cell_x)
            .map_err(|_| format!("cell x {cell_x} is outside global blend"))?;
        let cell_offset_y = usize::try_from(cell_y - self.min_cell_y)
            .map_err(|_| format!("cell y {cell_y} is outside global blend"))?;
        let quadrant_origin_x = cell_offset_x * LAND_CELL_INTERVALS
            + usize::from(quadrant & 1) * LAND_QUADRANT_INTERVALS;
        let quadrant_origin_y = cell_offset_y * LAND_CELL_INTERVALS
            + usize::from((quadrant >> 1) & 1) * LAND_QUADRANT_INTERVALS;
        for row in 0..LAND_QUADRANT_VERTICES {
            for column in 0..LAND_QUADRANT_VERTICES {
                let quantized = quantize_vertex_weights_to_bytes(
                    self.quadrant_vertex_weights(cell_x, cell_y, quadrant, row, column)?,
                );
                let is_edge = row == 0
                    || row == LAND_QUADRANT_VERTICES - 1
                    || column == 0
                    || column == LAND_QUADRANT_VERTICES - 1;
                for (&id, &byte) in &quantized {
                    *totals.entry(id).or_insert(0) += u32::from(byte);
                    if is_edge {
                        edge_ids.insert(id);
                    }
                }
                quadrant_weights.push(quantized);
                let global_x = quadrant_origin_x + column;
                let global_y = quadrant_origin_y + row;
                shared_edge_vertices.push(
                    (column == 0 && global_x > 0)
                        || (column == LAND_QUADRANT_VERTICES - 1 && global_x + 1 < self.width)
                        || (row == 0 && global_y > 0)
                        || (row == LAND_QUADRANT_VERTICES - 1 && global_y + 1 < self.height),
                );
            }
        }

        let preferred_base_source_ltex_object_id = self
            .quadrant_base_source_ltex_object_ids
            .get(&(cell_x, cell_y, quadrant))
            .copied();
        let retained_edge_ids = self
            .edge_retained_source_ltex_object_ids
            .get(&(cell_x, cell_y, quadrant))
            .cloned()
            .unwrap_or_else(|| edge_ids.clone());
        let Some(retained) = retained_quadrant_textures(
            &totals,
            &edge_ids,
            preferred_base_source_ltex_object_id,
            &retained_edge_ids,
        ) else {
            return Ok(None);
        };
        let mut shared_edge_saturated_vertex_counts = HashMap::<u32, u16>::new();
        for (weights, is_shared_edge) in quadrant_weights.iter().zip(shared_edge_vertices) {
            if !is_shared_edge {
                continue;
            }
            let retained_weight_total = retained
                .source_ltex_object_ids
                .iter()
                .map(|id| u16::from(*weights.get(id).unwrap_or(&0)))
                .sum::<u16>();
            if retained_weight_total == 0 {
                continue;
            }
            for id in &retained.source_ltex_object_ids {
                if u16::from(*weights.get(id).unwrap_or(&0)) == retained_weight_total {
                    *shared_edge_saturated_vertex_counts.entry(*id).or_insert(0) += 1;
                    break;
                }
            }
        }
        let mut alpha_source_ltex_object_ids = retained
            .source_ltex_object_ids
            .iter()
            .copied()
            .filter(|id| *id != retained.base_source_ltex_object_id)
            .collect::<Vec<_>>();
        alpha_source_ltex_object_ids.sort_by(|left, right| {
            shared_edge_saturated_vertex_counts
                .get(right)
                .unwrap_or(&0)
                .cmp(shared_edge_saturated_vertex_counts.get(left).unwrap_or(&0))
                .then_with(|| {
                    totals
                        .get(right)
                        .unwrap_or(&0)
                        .cmp(totals.get(left).unwrap_or(&0))
                })
                .then_with(|| left.cmp(right))
        });
        let alpha_vtxt = encode_alpha_vtxt_legacy(
            &quadrant_weights,
            retained.base_source_ltex_object_id,
            &alpha_source_ltex_object_ids,
        )?;

        Ok(Some(QuadrantBlend {
            base_source_ltex_object_id: retained.base_source_ltex_object_id,
            alpha_source_ltex_object_ids,
            alpha_vtxt,
            dropped_source_ltex_object_ids: retained.dropped_source_ltex_object_ids,
        }))
    }
}

fn encode_alpha_vtxt_legacy(
    quadrant_weights: &[HashMap<u32, u8>],
    base_source_ltex_object_id: u32,
    alpha_source_ltex_object_ids: &[u32],
) -> Result<Vec<Vec<u8>>, String> {
    let mut per_slot = vec![Vec::new(); alpha_source_ltex_object_ids.len()];
    let mut retained_source_ltex_object_ids =
        Vec::with_capacity(alpha_source_ltex_object_ids.len() + 1);
    retained_source_ltex_object_ids.push(base_source_ltex_object_id);
    retained_source_ltex_object_ids.extend_from_slice(alpha_source_ltex_object_ids);

    for row in 0..LAND_QUADRANT_VERTICES {
        for column in 0..LAND_QUADRANT_VERTICES {
            let position = row * LAND_QUADRANT_VERTICES + column;
            let weights = &quadrant_weights[position];
            let retained_weight_total = retained_source_ltex_object_ids
                .iter()
                .map(|id| u16::from(*weights.get(id).unwrap_or(&0)))
                .sum::<u16>();
            if retained_weight_total == 0 {
                continue;
            }

            let mut values = retained_source_ltex_object_ids
                .iter()
                .enumerate()
                .filter_map(|(retained_index, id)| {
                    let byte = *weights.get(id).unwrap_or(&0);
                    (byte > 0).then(|| {
                        let numerator = u16::from(byte) * 255;
                        let floor = (numerator / retained_weight_total) as u8;
                        let remainder = numerator % retained_weight_total;
                        (retained_index, *id, floor, remainder)
                    })
                })
                .collect::<Vec<_>>();
            if values.is_empty() {
                continue;
            }

            let floor_sum: u16 = values
                .iter()
                .map(|(_, _, floor, _)| u16::from(*floor))
                .sum();
            let mut remaining = (255 - floor_sum.min(255)) as usize;
            values.sort_by(|left, right| right.3.cmp(&left.3).then_with(|| left.1.cmp(&right.1)));
            for (_retained_index, _id, byte, _remainder) in values.iter_mut() {
                if remaining == 0 {
                    break;
                }
                if *byte < u8::MAX {
                    *byte += 1;
                    remaining -= 1;
                }
            }
            let mut normalized_weights = vec![0u8; retained_source_ltex_object_ids.len()];
            for (retained_index, _id, byte, _frac) in values {
                normalized_weights[retained_index] = byte;
            }

            let position = position as u16;
            let mut prefix_weight = u16::from(normalized_weights[0]);
            for (retained_index, &weight) in normalized_weights.iter().enumerate().skip(1) {
                prefix_weight += u16::from(weight);
                if weight == 0 {
                    continue;
                }
                let slot = retained_index - 1;
                let numerator = u32::from(weight) * 255;
                let mut alpha_byte =
                    ((numerator + u32::from(prefix_weight) / 2) / u32::from(prefix_weight)) as u8;
                if slot >= FIRST_ATXT_SLOT_REQUIRING_HEADROOM {
                    alpha_byte = alpha_byte.min(MAX_LATE_ATXT_ALPHA_WITH_BASE_HEADROOM);
                }
                if alpha_byte == 0 {
                    continue;
                }
                per_slot[slot].extend_from_slice(&position.to_le_bytes());
                per_slot[slot].push(0);
                per_slot[slot].push(0);
                per_slot[slot].extend_from_slice(&(f32::from(alpha_byte) / 255.0).to_le_bytes());
            }
        }
    }
    Ok(per_slot)
}
