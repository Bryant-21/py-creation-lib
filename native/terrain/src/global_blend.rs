use crate::btd::{BtdError, BtdFile, CellTextureSet};
use std::collections::{BTreeSet, HashMap, HashSet};

pub const SOURCE_CELL_SAMPLES: usize = 128;
pub const SOURCE_QUADRANT_SAMPLES: usize = 64;
pub const LAND_CELL_INTERVALS: usize = 32;
pub const LAND_QUADRANT_INTERVALS: usize = 16;
pub const LAND_QUADRANT_VERTICES: usize = 17;
pub const LAND_ALPHA_DOWNSAMPLE_WIDTH: i32 = 4;
/// FO4's land renderer packs base + 5 blend weights per vertex; quads carrying more
/// textures render black wherever an over-budget layer locally dominates (the engine
/// drops the overflow without renormalizing — CK-confirmed: removing any two textures
/// from a 7+-texture quad clears the black). Vanilla records ship up to 10/quad but
/// only with near-zero tail weights; FO76 source quads never exceed base+5, so anything
/// above 6 here is downsample bleed, not content.
pub const MAX_TEXTURES_PER_QUADRANT: usize = 6;

type QuadrantKey = (i32, i32, u8);

pub trait SourceAlphaLookup {
    fn sample_alpha(&self, source_ltex_object_id: u32, u: i32, v: i32) -> u8;
}

#[derive(Debug, Clone)]
pub struct GlobalLandscapeBlend {
    min_cell_x: i32,
    min_cell_y: i32,
    width: usize,
    height: usize,
    vertices: Vec<HashMap<u32, f32>>,
    quadrant_base_source_ltex_object_ids: HashMap<QuadrantKey, u32>,
    edge_retained_source_ltex_object_ids: HashMap<QuadrantKey, HashSet<u32>>,
}

#[derive(Debug, Clone)]
pub struct QuadrantBlend {
    pub base_source_ltex_object_id: u32,
    pub alpha_source_ltex_object_ids: Vec<u32>,
    pub alpha_vtxt: Vec<Vec<u8>>,
    pub dropped_source_ltex_object_ids: Vec<u32>,
}

impl GlobalLandscapeBlend {
    pub fn build(
        btd: &mut BtdFile,
        min_cell_x: i32,
        min_cell_y: i32,
        cells_x: usize,
        cells_y: usize,
        alpha_lookup: &impl SourceAlphaLookup,
    ) -> Result<Self, BtdError> {
        let width = cells_x * LAND_CELL_INTERVALS + 1;
        let height = cells_y * LAND_CELL_INTERVALS + 1;
        let mut vertices = Vec::with_capacity(width * height);
        for_each_global_blend_vertex(
            btd,
            min_cell_x,
            min_cell_y,
            cells_x,
            cells_y,
            alpha_lookup,
            |_, _, weights| vertices.push(weights.clone()),
        )?;

        let quadrant_base_source_ltex_object_ids =
            quadrant_base_source_ltex_object_ids(btd, min_cell_x, min_cell_y, cells_x, cells_y)?;

        let edge_retained_source_ltex_object_ids = compute_edge_retention_plan(
            min_cell_x,
            min_cell_y,
            cells_x,
            cells_y,
            width,
            &vertices,
            &quadrant_base_source_ltex_object_ids,
        );

        Ok(Self {
            min_cell_x,
            min_cell_y,
            width,
            height,
            vertices,
            quadrant_base_source_ltex_object_ids,
            edge_retained_source_ltex_object_ids,
        })
    }

    #[cfg(test)]
    pub fn from_vertices(
        min_cell_x: i32,
        min_cell_y: i32,
        cells_x: usize,
        cells_y: usize,
        vertices: Vec<HashMap<u32, f32>>,
    ) -> Self {
        let width = cells_x * LAND_CELL_INTERVALS + 1;
        let height = cells_y * LAND_CELL_INTERVALS + 1;
        assert_eq!(vertices.len(), width * height);
        let quadrant_base_source_ltex_object_ids = HashMap::new();
        let edge_retained_source_ltex_object_ids = compute_edge_retention_plan(
            min_cell_x,
            min_cell_y,
            cells_x,
            cells_y,
            width,
            &vertices,
            &quadrant_base_source_ltex_object_ids,
        );
        Self {
            min_cell_x,
            min_cell_y,
            width,
            height,
            vertices,
            quadrant_base_source_ltex_object_ids,
            edge_retained_source_ltex_object_ids,
        }
    }

    #[cfg(test)]
    fn from_vertices_with_bases(
        min_cell_x: i32,
        min_cell_y: i32,
        cells_x: usize,
        cells_y: usize,
        vertices: Vec<HashMap<u32, f32>>,
        quadrant_base_source_ltex_object_ids: HashMap<QuadrantKey, u32>,
    ) -> Self {
        let width = cells_x * LAND_CELL_INTERVALS + 1;
        let height = cells_y * LAND_CELL_INTERVALS + 1;
        assert_eq!(vertices.len(), width * height);
        let edge_retained_source_ltex_object_ids = compute_edge_retention_plan(
            min_cell_x,
            min_cell_y,
            cells_x,
            cells_y,
            width,
            &vertices,
            &quadrant_base_source_ltex_object_ids,
        );
        Self {
            min_cell_x,
            min_cell_y,
            width,
            height,
            vertices,
            quadrant_base_source_ltex_object_ids,
            edge_retained_source_ltex_object_ids,
        }
    }

    pub fn source_ltex_object_ids(&self) -> BTreeSet<u32> {
        let mut ids = BTreeSet::new();
        for weights in &self.vertices {
            ids.extend(weights.keys().copied());
        }
        ids
    }

    pub fn serialize_quadrant(
        &self,
        cell_x: i32,
        cell_y: i32,
        quadrant: u8,
    ) -> Result<Option<QuadrantBlend>, String> {
        let mut totals = HashMap::<u32, u32>::new();
        let mut edge_ids = HashSet::<u32>::new();
        let mut has_empty_edge_vertex = false;
        let mut quadrant_weights =
            Vec::with_capacity(LAND_QUADRANT_VERTICES * LAND_QUADRANT_VERTICES);
        for row in 0..LAND_QUADRANT_VERTICES {
            for column in 0..LAND_QUADRANT_VERTICES {
                let quantized = quantize_vertex_weights_to_bytes(
                    self.quadrant_vertex_weights(cell_x, cell_y, quadrant, row, column)?,
                );
                let is_edge = row == 0
                    || row == LAND_QUADRANT_VERTICES - 1
                    || column == 0
                    || column == LAND_QUADRANT_VERTICES - 1;
                if is_edge && quantized.is_empty() {
                    has_empty_edge_vertex = true;
                }
                for (&id, &byte) in &quantized {
                    *totals.entry(id).or_insert(0) += u32::from(byte);
                    if is_edge {
                        edge_ids.insert(id);
                    }
                }
                quadrant_weights.push(quantized);
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
            has_empty_edge_vertex,
            preferred_base_source_ltex_object_id,
            &retained_edge_ids,
        ) else {
            return Ok(None);
        };
        let alpha_source_ltex_object_ids = retained
            .source_ltex_object_ids
            .iter()
            .copied()
            .filter(|id| *id != retained.base_source_ltex_object_id)
            .collect::<Vec<_>>();
        let alpha_vtxt = encode_alpha_vtxt(
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

    pub fn quadrant_vertex_weights(
        &self,
        cell_x: i32,
        cell_y: i32,
        quadrant: u8,
        row: usize,
        column: usize,
    ) -> Result<&HashMap<u32, f32>, String> {
        if row >= LAND_QUADRANT_VERTICES || column >= LAND_QUADRANT_VERTICES || quadrant > 3 {
            return Err("quadrant vertex coordinate out of range".to_string());
        }
        let cell_offset_x = usize::try_from(cell_x - self.min_cell_x)
            .map_err(|_| format!("cell x {cell_x} is outside global blend"))?;
        let cell_offset_y = usize::try_from(cell_y - self.min_cell_y)
            .map_err(|_| format!("cell y {cell_y} is outside global blend"))?;
        let qx = usize::from(quadrant & 1);
        let qy = usize::from((quadrant >> 1) & 1);
        let global_x = cell_offset_x * LAND_CELL_INTERVALS + qx * LAND_QUADRANT_INTERVALS + column;
        let global_y = cell_offset_y * LAND_CELL_INTERVALS + qy * LAND_QUADRANT_INTERVALS + row;
        if global_x >= self.width || global_y >= self.height {
            return Err(format!(
                "cell ({cell_x},{cell_y}) quadrant {quadrant} vertex ({column},{row}) is outside global blend"
            ));
        }
        Ok(&self.vertices[global_y * self.width + global_x])
    }
}

struct RetainedQuadrantTextures {
    source_ltex_object_ids: Vec<u32>,
    base_source_ltex_object_id: u32,
    dropped_source_ltex_object_ids: Vec<u32>,
}

fn retained_quadrant_textures(
    totals: &HashMap<u32, u32>,
    edge_ids: &HashSet<u32>,
    has_empty_edge_vertex: bool,
    preferred_base_source_ltex_object_id: Option<u32>,
    retained_edge_ids: &HashSet<u32>,
) -> Option<RetainedQuadrantTextures> {
    if totals.is_empty() {
        return None;
    }
    let blocked_edge_ids = edge_ids
        .difference(retained_edge_ids)
        .copied()
        .collect::<HashSet<_>>();
    let preferred_base_required = preferred_base_source_ltex_object_id
        .is_some_and(|id| !edge_ids.contains(&id) && has_empty_edge_vertex);
    let mut retained = Vec::new();
    if let Some(id) = preferred_base_source_ltex_object_id {
        if preferred_base_required {
            retained.push(id);
        }
    }

    let mut edge_candidates: Vec<u32> = retained_edge_ids.iter().copied().collect();
    edge_candidates.sort_by(|left, right| {
        totals
            .get(right)
            .unwrap_or(&0)
            .cmp(totals.get(left).unwrap_or(&0))
            .then_with(|| left.cmp(right))
    });
    for id in edge_candidates {
        if retained.contains(&id) {
            continue;
        }
        if retained.len() >= MAX_TEXTURES_PER_QUADRANT {
            break;
        }
        retained.push(id);
    }

    retained.sort_by(|left, right| {
        totals
            .get(right)
            .unwrap_or(&0)
            .cmp(totals.get(left).unwrap_or(&0))
            .then_with(|| left.cmp(right))
    });
    if let Some(id) = preferred_base_source_ltex_object_id {
        if !retained.contains(&id)
            && retained.len() < MAX_TEXTURES_PER_QUADRANT
            && !blocked_edge_ids.contains(&id)
            && (totals.contains_key(&id) || has_empty_edge_vertex)
        {
            retained.push(id);
        }
    }

    let mut interior: Vec<u32> = totals
        .keys()
        .copied()
        .filter(|id| !retained.contains(id) && !blocked_edge_ids.contains(id))
        .collect();
    interior.sort_by(|left, right| {
        totals[right]
            .cmp(&totals[left])
            .then_with(|| left.cmp(right))
    });
    for id in interior {
        if retained.len() >= MAX_TEXTURES_PER_QUADRANT {
            break;
        }
        retained.push(id);
    }

    if retained.is_empty() {
        if let Some(id) = totals
            .iter()
            .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(left.0)))
            .map(|(id, _)| *id)
        {
            retained.push(id);
        }
    }

    let mut dropped_source_ltex_object_ids: Vec<u32> = totals
        .keys()
        .copied()
        .filter(|id| !retained.contains(id))
        .collect();
    dropped_source_ltex_object_ids.sort_unstable();

    let base_source_ltex_object_id = preferred_base_source_ltex_object_id
        .filter(|id| retained.contains(id))
        .unwrap_or_else(|| {
            retained
                .iter()
                .copied()
                .max_by(|left, right| {
                    totals
                        .get(left)
                        .unwrap_or(&0)
                        .cmp(totals.get(right).unwrap_or(&0))
                        .then_with(|| right.cmp(left))
                })
                .expect("retained has at least one texture")
        });

    Some(RetainedQuadrantTextures {
        source_ltex_object_ids: retained,
        base_source_ltex_object_id,
        dropped_source_ltex_object_ids,
    })
}

#[derive(Default)]
struct QuadrantTextureUsageSummary {
    totals: HashMap<u32, u32>,
    edge_weights: HashMap<u32, u32>,
    has_empty_edge_vertex: bool,
}

#[derive(Default)]
struct EdgeConnectivity {
    node_by_quadrant_id: HashMap<(usize, u32), usize>,
    nodes: Vec<(usize, u32)>,
    parents: Vec<usize>,
    ranks: Vec<u8>,
}

impl EdgeConnectivity {
    fn ensure_node(&mut self, quadrant_index: usize, source_ltex_object_id: u32) -> usize {
        if let Some(index) = self
            .node_by_quadrant_id
            .get(&(quadrant_index, source_ltex_object_id))
            .copied()
        {
            return index;
        }
        let index = self.nodes.len();
        self.node_by_quadrant_id
            .insert((quadrant_index, source_ltex_object_id), index);
        self.nodes.push((quadrant_index, source_ltex_object_id));
        self.parents.push(index);
        self.ranks.push(0);
        index
    }

    fn find(&mut self, index: usize) -> usize {
        let parent = self.parents[index];
        if parent == index {
            return index;
        }
        let root = self.find(parent);
        self.parents[index] = root;
        root
    }

    fn union(
        &mut self,
        left_quadrant_index: usize,
        right_quadrant_index: usize,
        source_ltex_object_id: u32,
    ) {
        let left = self.ensure_node(left_quadrant_index, source_ltex_object_id);
        let right = self.ensure_node(right_quadrant_index, source_ltex_object_id);
        let mut left_root = self.find(left);
        let mut right_root = self.find(right);
        if left_root == right_root {
            return;
        }
        if self.ranks[left_root] < self.ranks[right_root] {
            std::mem::swap(&mut left_root, &mut right_root);
        }
        self.parents[right_root] = left_root;
        if self.ranks[left_root] == self.ranks[right_root] {
            self.ranks[left_root] += 1;
        }
    }

    fn into_components(
        mut self,
        summaries: &[QuadrantTextureUsageSummary],
        quadrant_keys: &[QuadrantKey],
    ) -> Vec<EdgeComponent> {
        let mut component_index_by_root = HashMap::new();
        let mut components: Vec<EdgeComponent> = Vec::new();
        for node_index in 0..self.nodes.len() {
            let root = self.find(node_index);
            let (quadrant_index, source_ltex_object_id) = self.nodes[node_index];
            let component_index = *component_index_by_root.entry(root).or_insert_with(|| {
                let index = components.len();
                components.push(EdgeComponent {
                    source_ltex_object_id,
                    quadrants: Vec::new(),
                    weight: 0,
                });
                index
            });
            let component = &mut components[component_index];
            component.quadrants.push(quadrant_keys[quadrant_index]);
            component.weight += u64::from(
                *summaries[quadrant_index]
                    .edge_weights
                    .get(&source_ltex_object_id)
                    .unwrap_or(&0),
            );
        }
        for component in &mut components {
            component.quadrants.sort_unstable();
        }
        components
    }
}

pub fn collect_required_source_ltex_object_ids(
    btd: &mut BtdFile,
    min_cell_x: i32,
    min_cell_y: i32,
    cells_x: usize,
    cells_y: usize,
    alpha_lookup: &impl SourceAlphaLookup,
) -> Result<BTreeSet<u32>, BtdError> {
    let quadrant_grid_width = cells_x * 2;
    let quadrant_grid_height = cells_y * 2;
    let quadrant_count = quadrant_grid_width * quadrant_grid_height;
    let mut summaries = (0..quadrant_count)
        .map(|_| QuadrantTextureUsageSummary::default())
        .collect::<Vec<_>>();
    let quadrant_keys = (0..quadrant_grid_height)
        .flat_map(|quadrant_y| {
            (0..quadrant_grid_width).map(move |quadrant_x| {
                quadrant_key_from_grid(min_cell_x, min_cell_y, quadrant_x, quadrant_y)
            })
        })
        .collect::<Vec<_>>();
    let mut connectivity = EdgeConnectivity::default();
    let mut quantized = Vec::new();

    for_each_global_blend_vertex(
        btd,
        min_cell_x,
        min_cell_y,
        cells_x,
        cells_y,
        alpha_lookup,
        |vertex_x, vertex_y, weights| {
            quantize_vertex_weights(weights, &mut quantized);
            let (quadrant_xs, quadrant_x_count) =
                covering_quadrant_axis(vertex_x, quadrant_grid_width);
            let (quadrant_ys, quadrant_y_count) =
                covering_quadrant_axis(vertex_y, quadrant_grid_height);
            let mut covered = [(0usize, 0usize, 0usize); 4];
            let mut covered_count = 0;
            for &quadrant_y in &quadrant_ys[..quadrant_y_count] {
                for &quadrant_x in &quadrant_xs[..quadrant_x_count] {
                    let quadrant_index = quadrant_y * quadrant_grid_width + quadrant_x;
                    covered[covered_count] = (quadrant_index, quadrant_x, quadrant_y);
                    covered_count += 1;
                }
            }
            let is_edge =
                vertex_x % LAND_QUADRANT_INTERVALS == 0 || vertex_y % LAND_QUADRANT_INTERVALS == 0;

            for &(quadrant_index, _, _) in &covered[..covered_count] {
                let summary = &mut summaries[quadrant_index];
                if is_edge && quantized.is_empty() {
                    summary.has_empty_edge_vertex = true;
                }
                for &(source_ltex_object_id, byte, _) in &quantized {
                    *summary.totals.entry(source_ltex_object_id).or_insert(0) += u32::from(byte);
                    if is_edge {
                        *summary
                            .edge_weights
                            .entry(source_ltex_object_id)
                            .or_insert(0) += u32::from(byte);
                        connectivity.ensure_node(quadrant_index, source_ltex_object_id);
                    }
                }
            }

            if is_edge {
                for left in 0..covered_count {
                    for right in left + 1..covered_count {
                        let (left_index, left_x, left_y) = covered[left];
                        let (right_index, right_x, right_y) = covered[right];
                        if left_x.abs_diff(right_x) + left_y.abs_diff(right_y) != 1 {
                            continue;
                        }
                        for &(source_ltex_object_id, _, _) in &quantized {
                            connectivity.union(left_index, right_index, source_ltex_object_id);
                        }
                    }
                }
            }
        },
    )?;

    let quadrant_bases =
        quadrant_base_source_ltex_object_ids(btd, min_cell_x, min_cell_y, cells_x, cells_y)?;
    let quadrants = all_quadrant_keys(min_cell_x, min_cell_y, cells_x, cells_y);
    let mut retained = HashMap::with_capacity(quadrant_count);
    let mut empty_edge_vertices = HashSet::new();
    for (quadrant_index, summary) in summaries.iter().enumerate() {
        let key = quadrant_keys[quadrant_index];
        retained.insert(key, summary.edge_weights.keys().copied().collect());
        if summary.has_empty_edge_vertex {
            empty_edge_vertices.insert(key);
        }
    }
    let components = connectivity.into_components(&summaries, &quadrant_keys);
    let retained = trim_edge_retention(
        &quadrants,
        retained,
        &empty_edge_vertices,
        &quadrant_bases,
        &components,
    );

    let mut required = BTreeSet::new();
    for key in quadrants {
        let quadrant_index = quadrant_grid_index(min_cell_x, min_cell_y, quadrant_grid_width, key);
        let summary = &summaries[quadrant_index];
        let edge_ids = summary.edge_weights.keys().copied().collect::<HashSet<_>>();
        let retained_edge_ids = retained
            .get(&key)
            .cloned()
            .unwrap_or_else(|| edge_ids.clone());
        if let Some(retained) = retained_quadrant_textures(
            &summary.totals,
            &edge_ids,
            summary.has_empty_edge_vertex,
            quadrant_bases.get(&key).copied(),
            &retained_edge_ids,
        ) {
            required.extend(retained.source_ltex_object_ids);
        }
    }
    Ok(required)
}

fn covering_quadrant_axis(vertex: usize, quadrant_count: usize) -> ([usize; 2], usize) {
    if vertex % LAND_QUADRANT_INTERVALS != 0 {
        return ([vertex / LAND_QUADRANT_INTERVALS, 0], 1);
    }
    let boundary = vertex / LAND_QUADRANT_INTERVALS;
    let mut quadrants = [0; 2];
    let mut count = 0;
    if boundary > 0 {
        quadrants[count] = boundary - 1;
        count += 1;
    }
    if boundary < quadrant_count {
        quadrants[count] = boundary;
        count += 1;
    }
    (quadrants, count)
}

fn quadrant_key_from_grid(
    min_cell_x: i32,
    min_cell_y: i32,
    quadrant_x: usize,
    quadrant_y: usize,
) -> QuadrantKey {
    (
        min_cell_x + (quadrant_x / 2) as i32,
        min_cell_y + (quadrant_y / 2) as i32,
        ((quadrant_y % 2) * 2 + quadrant_x % 2) as u8,
    )
}

fn quadrant_grid_index(
    min_cell_x: i32,
    min_cell_y: i32,
    quadrant_grid_width: usize,
    (cell_x, cell_y, quadrant): QuadrantKey,
) -> usize {
    let quadrant_x = (cell_x - min_cell_x) as usize * 2 + usize::from(quadrant & 1);
    let quadrant_y = (cell_y - min_cell_y) as usize * 2 + usize::from(quadrant >> 1);
    quadrant_y * quadrant_grid_width + quadrant_x
}

#[derive(Debug, Clone)]
struct EdgeComponent {
    source_ltex_object_id: u32,
    quadrants: Vec<QuadrantKey>,
    weight: u64,
}

fn compute_edge_retention_plan(
    min_cell_x: i32,
    min_cell_y: i32,
    cells_x: usize,
    cells_y: usize,
    width: usize,
    vertices: &[HashMap<u32, f32>],
    quadrant_base_source_ltex_object_ids: &HashMap<QuadrantKey, u32>,
) -> HashMap<QuadrantKey, HashSet<u32>> {
    let mut retained = HashMap::<QuadrantKey, HashSet<u32>>::new();
    let mut edge_weights = HashMap::<(QuadrantKey, u32), u32>::new();
    let mut empty_edge_vertices = HashSet::<QuadrantKey>::new();
    let quadrants = all_quadrant_keys(min_cell_x, min_cell_y, cells_x, cells_y);

    for key in &quadrants {
        let mut ids = HashSet::<u32>::new();
        for (global_x, global_y) in quadrant_edge_global_vertices(min_cell_x, min_cell_y, *key) {
            let quantized = quantized_global_vertex(vertices, width, global_x, global_y);
            if quantized.is_empty() {
                empty_edge_vertices.insert(*key);
            }
            for (id, byte) in quantized {
                ids.insert(id);
                *edge_weights.entry((*key, id)).or_insert(0) += u32::from(byte);
            }
        }
        retained.insert(*key, ids);
    }

    let components = edge_components(
        min_cell_x,
        min_cell_y,
        cells_x,
        cells_y,
        width,
        vertices,
        &retained,
        &edge_weights,
    );
    trim_edge_retention(
        &quadrants,
        retained,
        &empty_edge_vertices,
        quadrant_base_source_ltex_object_ids,
        &components,
    )
}

fn trim_edge_retention(
    quadrants: &[QuadrantKey],
    mut retained: HashMap<QuadrantKey, HashSet<u32>>,
    empty_edge_vertices: &HashSet<QuadrantKey>,
    quadrant_base_source_ltex_object_ids: &HashMap<QuadrantKey, u32>,
    components: &[EdgeComponent],
) -> HashMap<QuadrantKey, HashSet<u32>> {
    let mut component_by_quadrant_id = HashMap::<(QuadrantKey, u32), usize>::new();
    for (component_index, component) in components.iter().enumerate() {
        for quadrant in &component.quadrants {
            component_by_quadrant_id.insert(
                (*quadrant, component.source_ltex_object_id),
                component_index,
            );
        }
    }

    loop {
        let Some(overfull_key) = quadrants
            .iter()
            .copied()
            .filter(|key| {
                retained_texture_count(
                    *key,
                    &retained,
                    &empty_edge_vertices,
                    quadrant_base_source_ltex_object_ids,
                ) > MAX_TEXTURES_PER_QUADRANT
            })
            .max_by_key(|key| {
                retained_texture_count(
                    *key,
                    &retained,
                    &empty_edge_vertices,
                    quadrant_base_source_ltex_object_ids,
                )
            })
        else {
            break;
        };

        // Never trim a component whose removal would leave a member quadrant with no
        // retained edge textures — serialize_quadrant blocks non-retained edge ids, so
        // an emptied quadrant could end up with nothing to serve as BTXT.
        let would_empty_a_member = |component: &EdgeComponent| {
            component.quadrants.iter().any(|quadrant| {
                retained.get(quadrant).is_some_and(|ids| {
                    ids.len() <= 1 && ids.contains(&component.source_ltex_object_id)
                })
            })
        };
        let Some(component_index) = retained
            .get(&overfull_key)
            .into_iter()
            .flat_map(|ids| ids.iter().copied())
            .filter_map(|id| component_by_quadrant_id.get(&(overfull_key, id)).copied())
            .filter(|index| !would_empty_a_member(&components[*index]))
            .min_by(|left, right| {
                let left_component = &components[*left];
                let right_component = &components[*right];
                left_component
                    .weight
                    .cmp(&right_component.weight)
                    .then_with(|| {
                        left_component
                            .quadrants
                            .len()
                            .cmp(&right_component.quadrants.len())
                    })
                    .then_with(|| {
                        right_component
                            .source_ltex_object_id
                            .cmp(&left_component.source_ltex_object_id)
                    })
            })
        else {
            break;
        };

        let component = &components[component_index];
        for quadrant in &component.quadrants {
            if let Some(ids) = retained.get_mut(quadrant) {
                ids.remove(&component.source_ltex_object_id);
            }
        }
    }

    retained
}

fn retained_texture_count(
    key: QuadrantKey,
    retained: &HashMap<QuadrantKey, HashSet<u32>>,
    empty_edge_vertices: &HashSet<QuadrantKey>,
    quadrant_base_source_ltex_object_ids: &HashMap<QuadrantKey, u32>,
) -> usize {
    let edge_count = retained.get(&key).map(|ids| ids.len()).unwrap_or(0);
    let source_base_required = quadrant_base_source_ltex_object_ids
        .get(&key)
        .is_some_and(|id| {
            empty_edge_vertices.contains(&key)
                && !retained
                    .get(&key)
                    .is_some_and(|edge_ids| edge_ids.contains(id))
        });
    edge_count + usize::from(source_base_required)
}

fn edge_components(
    min_cell_x: i32,
    min_cell_y: i32,
    cells_x: usize,
    cells_y: usize,
    width: usize,
    vertices: &[HashMap<u32, f32>],
    retained: &HashMap<QuadrantKey, HashSet<u32>>,
    edge_weights: &HashMap<(QuadrantKey, u32), u32>,
) -> Vec<EdgeComponent> {
    let mut adjacency = HashMap::<u32, HashMap<QuadrantKey, HashSet<QuadrantKey>>>::new();
    for key in all_quadrant_keys(min_cell_x, min_cell_y, cells_x, cells_y) {
        for neighbor in right_and_top_neighbors(key, min_cell_x, min_cell_y, cells_x, cells_y) {
            for (global_x, global_y) in
                shared_edge_global_vertices(min_cell_x, min_cell_y, key, neighbor)
            {
                for id in quantized_global_vertex(vertices, width, global_x, global_y).into_keys() {
                    if !retained.get(&key).is_some_and(|ids| ids.contains(&id))
                        || !retained.get(&neighbor).is_some_and(|ids| ids.contains(&id))
                    {
                        continue;
                    }
                    adjacency
                        .entry(id)
                        .or_default()
                        .entry(key)
                        .or_default()
                        .insert(neighbor);
                    adjacency
                        .entry(id)
                        .or_default()
                        .entry(neighbor)
                        .or_default()
                        .insert(key);
                }
            }
        }
    }

    let mut component_by_quadrant_id = HashSet::<(QuadrantKey, u32)>::new();
    let mut components = Vec::<EdgeComponent>::new();
    for (&key, ids) in retained {
        for &id in ids {
            if component_by_quadrant_id.contains(&(key, id)) {
                continue;
            }
            let mut stack = vec![key];
            let mut quadrants = Vec::<QuadrantKey>::new();
            component_by_quadrant_id.insert((key, id));
            while let Some(current) = stack.pop() {
                quadrants.push(current);
                if let Some(neighbors) =
                    adjacency.get(&id).and_then(|by_quad| by_quad.get(&current))
                {
                    for &neighbor in neighbors {
                        if component_by_quadrant_id.insert((neighbor, id)) {
                            stack.push(neighbor);
                        }
                    }
                }
            }
            quadrants.sort_unstable();
            let weight = quadrants
                .iter()
                .map(|quadrant| u64::from(*edge_weights.get(&(*quadrant, id)).unwrap_or(&0)))
                .sum();
            components.push(EdgeComponent {
                source_ltex_object_id: id,
                quadrants,
                weight,
            });
        }
    }
    components
}

fn all_quadrant_keys(
    min_cell_x: i32,
    min_cell_y: i32,
    cells_x: usize,
    cells_y: usize,
) -> Vec<QuadrantKey> {
    let mut keys = Vec::with_capacity(cells_x * cells_y * 4);
    for cell_y_offset in 0..cells_y {
        for cell_x_offset in 0..cells_x {
            let cell_x = min_cell_x + cell_x_offset as i32;
            let cell_y = min_cell_y + cell_y_offset as i32;
            for quadrant in 0..4 {
                keys.push((cell_x, cell_y, quadrant));
            }
        }
    }
    keys
}

fn right_and_top_neighbors(
    key: QuadrantKey,
    min_cell_x: i32,
    min_cell_y: i32,
    cells_x: usize,
    cells_y: usize,
) -> Vec<QuadrantKey> {
    let (cell_x, cell_y, quadrant) = key;
    let qx = quadrant & 1;
    let qy = (quadrant >> 1) & 1;
    let max_cell_x = min_cell_x + cells_x as i32 - 1;
    let max_cell_y = min_cell_y + cells_y as i32 - 1;
    let mut neighbors = Vec::with_capacity(2);
    if qx == 0 {
        neighbors.push((cell_x, cell_y, quadrant + 1));
    } else if cell_x < max_cell_x {
        neighbors.push((cell_x + 1, cell_y, quadrant - 1));
    }
    if qy == 0 {
        neighbors.push((cell_x, cell_y, quadrant + 2));
    } else if cell_y < max_cell_y {
        neighbors.push((cell_x, cell_y + 1, quadrant - 2));
    }
    neighbors
}

fn quadrant_edge_global_vertices(
    min_cell_x: i32,
    min_cell_y: i32,
    key: QuadrantKey,
) -> Vec<(usize, usize)> {
    let (origin_x, origin_y) = quadrant_global_origin(min_cell_x, min_cell_y, key);
    let mut vertices = Vec::with_capacity((LAND_QUADRANT_VERTICES * 4) - 4);
    for offset in 0..LAND_QUADRANT_VERTICES {
        vertices.push((origin_x + offset, origin_y));
        vertices.push((origin_x + offset, origin_y + LAND_QUADRANT_INTERVALS));
    }
    for offset in 1..LAND_QUADRANT_INTERVALS {
        vertices.push((origin_x, origin_y + offset));
        vertices.push((origin_x + LAND_QUADRANT_INTERVALS, origin_y + offset));
    }
    vertices
}

fn shared_edge_global_vertices(
    min_cell_x: i32,
    min_cell_y: i32,
    left: QuadrantKey,
    right: QuadrantKey,
) -> Vec<(usize, usize)> {
    let (left_x, left_y) = quadrant_global_origin(min_cell_x, min_cell_y, left);
    let (right_x, right_y) = quadrant_global_origin(min_cell_x, min_cell_y, right);
    if left_x + LAND_QUADRANT_INTERVALS == right_x && left_y == right_y {
        return (0..LAND_QUADRANT_VERTICES)
            .map(|offset| (right_x, right_y + offset))
            .collect();
    }
    if left_y + LAND_QUADRANT_INTERVALS == right_y && left_x == right_x {
        return (0..LAND_QUADRANT_VERTICES)
            .map(|offset| (right_x + offset, right_y))
            .collect();
    }
    Vec::new()
}

fn quadrant_global_origin(min_cell_x: i32, min_cell_y: i32, key: QuadrantKey) -> (usize, usize) {
    let (cell_x, cell_y, quadrant) = key;
    let cell_offset_x = usize::try_from(cell_x - min_cell_x).expect("quadrant x in range");
    let cell_offset_y = usize::try_from(cell_y - min_cell_y).expect("quadrant y in range");
    let qx = usize::from(quadrant & 1);
    let qy = usize::from((quadrant >> 1) & 1);
    (
        cell_offset_x * LAND_CELL_INTERVALS + qx * LAND_QUADRANT_INTERVALS,
        cell_offset_y * LAND_CELL_INTERVALS + qy * LAND_QUADRANT_INTERVALS,
    )
}

fn quantized_global_vertex(
    vertices: &[HashMap<u32, f32>],
    width: usize,
    global_x: usize,
    global_y: usize,
) -> HashMap<u32, u8> {
    vertices
        .get(global_y * width + global_x)
        .map(quantize_vertex_weights_to_bytes)
        .unwrap_or_default()
}

fn encode_alpha_vtxt(
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
            values.sort_by_key(|(retained_index, _, _, _)| *retained_index);

            let position = position as u16;
            for (retained_index, _id, byte, _frac) in values {
                if retained_index == 0 {
                    continue;
                }
                if byte == 0 {
                    continue;
                }
                let slot = retained_index - 1;
                per_slot[slot].extend_from_slice(&position.to_le_bytes());
                per_slot[slot].push(0);
                per_slot[slot].push(0);
                per_slot[slot].extend_from_slice(&(f32::from(byte) / 255.0).to_le_bytes());
            }
        }
    }
    Ok(per_slot)
}

fn quantize_vertex_weights_to_bytes(weights: &HashMap<u32, f32>) -> HashMap<u32, u8> {
    let mut values = Vec::new();
    quantize_vertex_weights(weights, &mut values);
    values
        .into_iter()
        .map(|(id, byte, _fraction)| (id, byte))
        .collect()
}

fn quantize_vertex_weights(weights: &HashMap<u32, f32>, values: &mut Vec<(u32, u8, f32)>) {
    values.clear();
    let total = weights
        .values()
        .copied()
        .filter(|weight| *weight > 0.0)
        .sum::<f32>();
    if total <= 0.0 {
        return;
    }

    values.extend(weights.iter().filter_map(|(&id, &weight)| {
        (weight > 0.0).then(|| {
            let scaled = (weight / total).clamp(0.0, 1.0) * 255.0;
            let floor = scaled.floor() as u8;
            (id, floor, scaled - f32::from(floor))
        })
    }));
    let floor_sum: u16 = values.iter().map(|(_, floor, _)| u16::from(*floor)).sum();
    let mut remaining = (255 - floor_sum.min(255)) as usize;
    values.sort_by(|left, right| {
        right
            .2
            .total_cmp(&left.2)
            .then_with(|| left.0.cmp(&right.0))
    });
    for (_id, byte, _frac) in values.iter_mut() {
        if remaining == 0 {
            break;
        }
        if *byte < u8::MAX {
            *byte += 1;
            remaining -= 1;
        }
    }

    values.retain(|(_, byte, _fraction)| *byte > 0);
}

struct SourceCellBlendData {
    textures: CellTextureSet,
    alphas: Vec<u16>,
}

#[derive(Clone, Copy)]
struct SourceSampleCoordinate {
    cell: i32,
    local: usize,
    world: i32,
}

fn for_each_global_blend_vertex(
    btd: &mut BtdFile,
    min_cell_x: i32,
    min_cell_y: i32,
    cells_x: usize,
    cells_y: usize,
    alpha_lookup: &impl SourceAlphaLookup,
    mut visit: impl FnMut(usize, usize, &HashMap<u32, f32>),
) -> Result<(), BtdError> {
    let width = cells_x * LAND_CELL_INTERVALS + 1;
    let height = cells_y * LAND_CELL_INTERVALS + 1;
    let header = btd.header();
    let source_columns = blend_sample_axis(min_cell_x, width, header.cell_min_x, header.cell_max_x);
    let source_rows = blend_sample_axis(min_cell_y, height, header.cell_min_y, header.cell_max_y);
    let first_source_cell_x = source_columns[0][0].cell;
    let last_source_cell_x =
        source_columns[width - 1][LAND_ALPHA_DOWNSAMPLE_WIDTH as usize - 1].cell;
    let mut loaded_rows: Vec<(i32, Vec<SourceCellBlendData>)> = Vec::with_capacity(2);
    let mut weights = HashMap::<u32, f32>::new();
    let sample_weight = 1.0 / (LAND_ALPHA_DOWNSAMPLE_WIDTH * LAND_ALPHA_DOWNSAMPLE_WIDTH) as f32;

    for vertex_y in 0..height {
        let row_coordinates = &source_rows[vertex_y];
        loaded_rows.retain(|(cell_y, _)| {
            row_coordinates
                .iter()
                .any(|coordinate| coordinate.cell == *cell_y)
        });
        for coordinate in row_coordinates {
            if loaded_rows
                .iter()
                .any(|(cell_y, _)| *cell_y == coordinate.cell)
            {
                continue;
            }
            let mut cells = Vec::with_capacity(
                usize::try_from(last_source_cell_x - first_source_cell_x + 1).unwrap(),
            );
            for cell_x in first_source_cell_x..=last_source_cell_x {
                cells.push(SourceCellBlendData {
                    textures: btd.cell_texture_set(cell_x, coordinate.cell)?,
                    alphas: btd.cell_land_alpha_u16(cell_x, coordinate.cell, 0)?,
                });
            }
            loaded_rows.push((coordinate.cell, cells));
        }

        for vertex_x in 0..width {
            weights.clear();
            for source_y in row_coordinates {
                let source_cell_row = loaded_rows
                    .iter()
                    .find(|(cell_y, _)| *cell_y == source_y.cell)
                    .map(|(_, cells)| cells)
                    .expect("source row loaded");
                for source_x in &source_columns[vertex_x] {
                    let source_cell = &source_cell_row
                        [usize::try_from(source_x.cell - first_source_cell_x).unwrap()];
                    let Some(source_ltex_object_id) = source_sample_texture_object_id(
                        btd,
                        source_cell,
                        alpha_lookup,
                        *source_x,
                        *source_y,
                    ) else {
                        continue;
                    };
                    *weights.entry(source_ltex_object_id).or_insert(0.0) += sample_weight;
                }
            }
            visit(vertex_x, vertex_y, &weights);
        }
    }
    Ok(())
}

fn blend_sample_axis(
    min_cell: i32,
    vertex_count: usize,
    header_min_cell: i32,
    header_max_cell: i32,
) -> Vec<[SourceSampleCoordinate; LAND_ALPHA_DOWNSAMPLE_WIDTH as usize]> {
    let min_sample = header_min_cell * SOURCE_CELL_SAMPLES as i32;
    let max_sample = (header_max_cell + 1) * SOURCE_CELL_SAMPLES as i32 - 1;
    (0..vertex_count)
        .map(|vertex| {
            let center = min_cell * SOURCE_CELL_SAMPLES as i32
                + (SOURCE_CELL_SAMPLES / 2) as i32
                + (vertex * 4) as i32;
            std::array::from_fn(|offset| {
                let world = (center - LAND_ALPHA_DOWNSAMPLE_WIDTH / 2 + offset as i32)
                    .clamp(min_sample, max_sample);
                SourceSampleCoordinate {
                    cell: world.div_euclid(SOURCE_CELL_SAMPLES as i32),
                    local: world.rem_euclid(SOURCE_CELL_SAMPLES as i32) as usize,
                    world,
                }
            })
        })
        .collect()
}

fn quadrant_base_source_ltex_object_ids(
    btd: &BtdFile,
    min_cell_x: i32,
    min_cell_y: i32,
    cells_x: usize,
    cells_y: usize,
) -> Result<HashMap<QuadrantKey, u32>, BtdError> {
    let mut bases = HashMap::new();
    for cell_y_offset in 0..cells_y {
        for cell_x_offset in 0..cells_x {
            let cell_x = min_cell_x + cell_x_offset as i32;
            let cell_y = min_cell_y + cell_y_offset as i32;
            let set = btd.cell_texture_set(cell_x, cell_y)?;
            for (quadrant, quad) in set.quadrants.iter().enumerate() {
                let Some(texture_index) = quad.base else {
                    continue;
                };
                let Some(source_ltex_object_id) = btd
                    .land_texture_form_id(texture_index as usize)
                    .map(|form_id| form_id & 0x00FF_FFFF)
                else {
                    continue;
                };
                bases.insert((cell_x, cell_y, quadrant as u8), source_ltex_object_id);
            }
        }
    }
    Ok(bases)
}

fn source_sample_texture_object_id(
    btd: &BtdFile,
    source_cell: &SourceCellBlendData,
    alpha_lookup: &impl SourceAlphaLookup,
    source_x: SourceSampleCoordinate,
    source_y: SourceSampleCoordinate,
) -> Option<u32> {
    let quadrant = ((source_y.local / SOURCE_QUADRANT_SAMPLES) << 1)
        | (source_x.local / SOURCE_QUADRANT_SAMPLES);
    let Some(quad) = source_cell.textures.quadrants.get(quadrant) else {
        return None;
    };
    let packed = source_cell.alphas[source_y.local * SOURCE_CELL_SAMPLES + source_x.local];
    let u = source_x.world - source_y.world;
    let v = source_x.world + source_y.world;
    for layer in (0..5usize).rev() {
        let value = ((packed >> (layer * 3)) & 0x7) as u8;
        if value == 0 {
            continue;
        }
        let Some(texture_index) = quad.additional[layer] else {
            continue;
        };
        let Some(source_ltex_object_id) = btd
            .land_texture_form_id(texture_index as usize)
            .map(|form_id| form_id & 0x00FF_FFFF)
        else {
            continue;
        };
        if !fo76_layer_alpha_passes(
            value,
            alpha_lookup.sample_alpha(source_ltex_object_id, u, v),
        ) {
            continue;
        }
        return Some(source_ltex_object_id);
    }

    quad.base
        .and_then(|texture_index| btd.land_texture_form_id(texture_index as usize))
        .map(|form_id| form_id & 0x00FF_FFFF)
}

fn fo76_layer_alpha_passes(layer_value: u8, source_alpha: u8) -> bool {
    if layer_value == 0 {
        return false;
    }
    let threshold = f32::from(7 - layer_value.min(7)) * (255.5 / 7.0);
    f32::from(source_alpha) >= threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights(entries: &[(u32, f32)]) -> HashMap<u32, f32> {
        entries.iter().copied().collect()
    }

    #[test]
    fn retained_quadrant_textures_applies_layer_cap_and_reports_drop() {
        let totals = HashMap::from([(1, 7), (2, 6), (3, 5), (4, 4), (5, 3), (6, 2), (7, 1)]);
        let edge_ids = totals.keys().copied().collect::<HashSet<_>>();
        let retained =
            retained_quadrant_textures(&totals, &edge_ids, false, None, &edge_ids).unwrap();

        assert_eq!(
            retained.source_ltex_object_ids.len(),
            MAX_TEXTURES_PER_QUADRANT
        );
        assert_eq!(retained.dropped_source_ltex_object_ids, [7]);
    }

    #[test]
    fn fo76_layer_alpha_passes_fo76utils_threshold() {
        assert!(!fo76_layer_alpha_passes(0, 255));
        assert!(fo76_layer_alpha_passes(7, 0));
        assert!(fo76_layer_alpha_passes(6, 37));
        assert!(!fo76_layer_alpha_passes(6, 36));
        assert!(fo76_layer_alpha_passes(1, 219));
        assert!(!fo76_layer_alpha_passes(1, 218));
    }

    #[test]
    fn shared_cell_edge_reads_same_global_vertex_weights() {
        let width = 2 * LAND_CELL_INTERVALS + 1;
        let height = LAND_CELL_INTERVALS + 1;
        let mut vertices = vec![HashMap::new(); width * height];
        vertices[8 * width + LAND_CELL_INTERVALS] = weights(&[(0x111111, 0.25), (0x222222, 0.75)]);
        let blend = GlobalLandscapeBlend::from_vertices(0, 0, 2, 1, vertices);

        let left = blend
            .quadrant_vertex_weights(0, 0, 1, 8, LAND_QUADRANT_VERTICES - 1)
            .unwrap();
        let right = blend.quadrant_vertex_weights(1, 0, 0, 8, 0).unwrap();

        assert_eq!(left, right);
    }

    fn effective_bytes(quad: &QuadrantBlend, position: usize) -> HashMap<u32, u8> {
        let mut result = HashMap::<u32, u8>::new();
        let mut alpha_sum = 0u16;
        for (source_id, vtxt) in quad
            .alpha_source_ltex_object_ids
            .iter()
            .copied()
            .zip(quad.alpha_vtxt.iter())
        {
            let byte = vtxt_alpha_byte(vtxt, position);
            if byte > 0 {
                result.insert(source_id, byte);
                alpha_sum += u16::from(byte);
            }
        }
        let base = 255u16.saturating_sub(alpha_sum).min(255) as u8;
        if base > 0 {
            result.insert(quad.base_source_ltex_object_id, base);
        }
        result
    }

    fn vtxt_alpha_byte(vtxt: &[u8], position: usize) -> u8 {
        for chunk in vtxt.chunks_exact(8) {
            let pos = u16::from_le_bytes([chunk[0], chunk[1]]) as usize;
            if pos == position {
                let opacity = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                return (opacity * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        0
    }

    #[test]
    fn quadrant_serialization_preserves_effective_weights_across_different_bases() {
        let width = 2 * LAND_CELL_INTERVALS + 1;
        let height = LAND_CELL_INTERVALS + 1;
        let mut vertices = vec![HashMap::new(); width * height];
        for y in 0..LAND_QUADRANT_VERTICES {
            for x in LAND_QUADRANT_INTERVALS..=LAND_CELL_INTERVALS {
                vertices[y * width + x] = weights(&[(0x111111, 1.0)]);
            }
            for x in LAND_CELL_INTERVALS..=LAND_CELL_INTERVALS + LAND_QUADRANT_INTERVALS {
                vertices[y * width + x] = weights(&[(0x222222, 1.0)]);
            }
            vertices[y * width + LAND_CELL_INTERVALS] =
                weights(&[(0x111111, 0.50), (0x222222, 0.50)]);
        }
        let blend = GlobalLandscapeBlend::from_vertices(0, 0, 2, 1, vertices);

        let left = blend.serialize_quadrant(0, 0, 1).unwrap().unwrap();
        let right = blend.serialize_quadrant(1, 0, 0).unwrap().unwrap();

        assert_eq!(left.base_source_ltex_object_id, 0x111111);
        assert_eq!(right.base_source_ltex_object_id, 0x222222);
        let left_position = 8 * LAND_QUADRANT_VERTICES + LAND_QUADRANT_VERTICES - 1;
        let right_position = 8 * LAND_QUADRANT_VERTICES;
        assert_eq!(
            effective_bytes(&left, left_position),
            effective_bytes(&right, right_position)
        );
    }

    #[test]
    fn quadrant_serialization_prefers_source_base_over_dominant_texture() {
        let width = LAND_CELL_INTERVALS + 1;
        let height = LAND_CELL_INTERVALS + 1;
        let mut vertices = vec![HashMap::new(); width * height];
        vertices[0] = weights(&[(0x111111, 0.20), (0x222222, 0.80)]);
        let blend = GlobalLandscapeBlend::from_vertices_with_bases(
            0,
            0,
            1,
            1,
            vertices,
            HashMap::from([((0, 0, 0), 0x111111)]),
        );

        let quad = blend.serialize_quadrant(0, 0, 0).unwrap().unwrap();

        assert_eq!(quad.base_source_ltex_object_id, 0x111111);
        assert!(quad.alpha_source_ltex_object_ids.contains(&0x222222));
    }

    #[test]
    fn capacity_keeps_edge_textures_before_interior_textures() {
        let width = LAND_CELL_INTERVALS + 1;
        let height = LAND_CELL_INTERVALS + 1;
        let mut vertices = vec![HashMap::new(); width * height];
        let edge_count = MAX_TEXTURES_PER_QUADRANT as u32;
        for id in 1..=edge_count {
            vertices[id as usize] = weights(&[(id, 1.0)]);
        }
        vertices[8 * width + 8] = weights(&[(99, 1.0)]);
        let blend = GlobalLandscapeBlend::from_vertices(0, 0, 1, 1, vertices);

        let quad = blend.serialize_quadrant(0, 0, 0).unwrap().unwrap();
        assert!(quad.dropped_source_ltex_object_ids.contains(&99));
        for id in 1..=edge_count {
            assert!(
                quad.base_source_ltex_object_id == id
                    || quad.alpha_source_ltex_object_ids.contains(&id)
            );
        }
    }

    #[test]
    fn capacity_trim_never_empties_a_single_texture_neighbor() {
        let width = LAND_CELL_INTERVALS + 1;
        let height = LAND_CELL_INTERVALS + 1;
        let mut vertices = vec![HashMap::new(); width * height];
        // Overfull BL quadrant: seven strong single-quad edge textures...
        for id in 1..=7u32 {
            vertices[id as usize] = weights(&[(id, 1.0)]);
        }
        // ...plus a weak texture on the BL/BR shared edge that is the BR quadrant's
        // ONLY texture. The old trimmer picked it (weakest component) and emptied BR,
        // which panicked serialize_quadrant's base selection.
        vertices[8 * width + LAND_QUADRANT_INTERVALS] = weights(&[(99, 0.2)]);
        let blend = GlobalLandscapeBlend::from_vertices(0, 0, 1, 1, vertices);

        let bottom_left = blend.serialize_quadrant(0, 0, 0).unwrap().unwrap();
        let bottom_right = blend.serialize_quadrant(0, 0, 1).unwrap().unwrap();

        assert_eq!(bottom_right.base_source_ltex_object_id, 99);
        assert!(
            bottom_left.base_source_ltex_object_id == 99
                || bottom_left.alpha_source_ltex_object_ids.contains(&99)
        );
        assert!(1 + bottom_left.alpha_source_ltex_object_ids.len() <= MAX_TEXTURES_PER_QUADRANT);
    }

    #[test]
    fn capacity_trims_lowest_edge_texture_when_fo4_limit_is_exceeded() {
        let width = LAND_CELL_INTERVALS + 1;
        let height = LAND_CELL_INTERVALS + 1;
        let mut vertices = vec![HashMap::new(); width * height];
        for id in 1..=11u32 {
            vertices[id as usize] = weights(&[(id, 1.0)]);
        }
        let blend = GlobalLandscapeBlend::from_vertices(0, 0, 1, 1, vertices);

        let quad = blend.serialize_quadrant(0, 0, 0).unwrap().unwrap();

        assert!(quad.dropped_source_ltex_object_ids.contains(&11));
        assert_eq!(
            quad.alpha_source_ltex_object_ids.len(),
            MAX_TEXTURES_PER_QUADRANT - 1
        );
        assert_eq!(
            quad.dropped_source_ltex_object_ids.len(),
            11 - MAX_TEXTURES_PER_QUADRANT
        );
    }

    #[test]
    fn capacity_trim_propagates_across_shared_quadrant_edge() {
        let width = LAND_CELL_INTERVALS + 1;
        let height = LAND_CELL_INTERVALS + 1;
        let mut vertices = vec![HashMap::new(); width * height];
        for id in 1..=9u32 {
            vertices[LAND_CELL_INTERVALS * width + id as usize] = weights(&[(id, 1.0)]);
        }
        vertices[LAND_QUADRANT_INTERVALS * width] = weights(&[(10, 1.0), (11, 0.10)]);
        let blend = GlobalLandscapeBlend::from_vertices(0, 0, 1, 1, vertices);

        let bottom_left = blend.serialize_quadrant(0, 0, 0).unwrap().unwrap();
        let top_left = blend.serialize_quadrant(0, 0, 2).unwrap().unwrap();
        let bottom_left_top_position = (LAND_QUADRANT_VERTICES - 1) * LAND_QUADRANT_VERTICES;
        let top_left_bottom_position = 0;

        assert!(bottom_left.dropped_source_ltex_object_ids.contains(&11));
        assert!(top_left.dropped_source_ltex_object_ids.contains(&11));
        assert_eq!(
            effective_bytes(&bottom_left, bottom_left_top_position),
            effective_bytes(&top_left, top_left_bottom_position)
        );
    }

    #[test]
    fn capacity_ignores_edge_textures_that_quantize_to_zero() {
        let width = LAND_CELL_INTERVALS + 1;
        let height = LAND_CELL_INTERVALS + 1;
        let mut vertices = vec![HashMap::new(); width * height];
        for id in 1..=10u32 {
            vertices[id as usize] = weights(&[(id, 1.0)]);
        }
        vertices[0] = weights(&[(1, 1.0), (11, 0.001)]);
        let blend = GlobalLandscapeBlend::from_vertices(0, 0, 1, 1, vertices);

        let quad = blend.serialize_quadrant(0, 0, 0).unwrap().unwrap();

        assert!(!quad.dropped_source_ltex_object_ids.contains(&11));
        assert!(
            quad.base_source_ltex_object_id != 11
                && !quad.alpha_source_ltex_object_ids.contains(&11)
        );
    }

    #[test]
    fn capacity_keeps_required_source_base_by_trimming_edge_texture() {
        let width = LAND_CELL_INTERVALS + 1;
        let height = LAND_CELL_INTERVALS + 1;
        let mut vertices = vec![HashMap::new(); width * height];
        for id in 1..=10u32 {
            vertices[id as usize] = weights(&[(id, 1.0)]);
        }
        let blend = GlobalLandscapeBlend::from_vertices_with_bases(
            0,
            0,
            1,
            1,
            vertices,
            HashMap::from([((0, 0, 0), 99)]),
        );

        let quad = blend.serialize_quadrant(0, 0, 0).unwrap().unwrap();

        assert_eq!(quad.base_source_ltex_object_id, 99);
        assert_eq!(
            quad.alpha_source_ltex_object_ids.len(),
            MAX_TEXTURES_PER_QUADRANT - 1
        );
        assert_eq!(
            quad.dropped_source_ltex_object_ids.len(),
            11 - MAX_TEXTURES_PER_QUADRANT
        );
    }

    #[test]
    fn retained_interior_texture_with_subbyte_edge_trace_stays_off_edge() {
        let width = LAND_CELL_INTERVALS + 1;
        let height = LAND_CELL_INTERVALS + 1;
        let mut vertices = vec![HashMap::new(); width * height];
        vertices[0] = weights(&[(1, 1.0), (2, 0.001)]);
        vertices[8 * width + 8] = weights(&[(2, 1.0)]);
        let blend = GlobalLandscapeBlend::from_vertices(0, 0, 1, 1, vertices);

        let quad = blend.serialize_quadrant(0, 0, 0).unwrap().unwrap();

        assert!(quad.alpha_source_ltex_object_ids.contains(&2));
        assert!(!effective_bytes(&quad, 0).contains_key(&2));
    }

    #[test]
    fn quantized_alpha_bytes_form_valid_partition() {
        let width = LAND_CELL_INTERVALS + 1;
        let height = LAND_CELL_INTERVALS + 1;
        let mut vertices = vec![HashMap::new(); width * height];
        vertices[0] = weights(&[(1, 0.20), (2, 0.30), (3, 0.50)]);
        let blend = GlobalLandscapeBlend::from_vertices(0, 0, 1, 1, vertices);

        let quad = blend.serialize_quadrant(0, 0, 0).unwrap().unwrap();
        let mut sum = 0u16;
        for bytes in &quad.alpha_vtxt {
            if bytes.is_empty() {
                continue;
            }
            let opacity = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            sum += (opacity * 255.0).round() as u16;
        }
        assert!(sum <= 255);
    }
}
