#[derive(Clone)]
struct TriangleInfo {
    output_id: u32,
    minimum: [f32; 3],
    maximum: [f32; 3],
    centroid: [f32; 3],
}

struct BvhNode {
    triangles: Vec<TriangleInfo>,
    left: Option<Box<BvhNode>>,
    right: Option<Box<BvhNode>>,
    axis: usize,
    minimum: [f32; 3],
    maximum: [f32; 3],
}

pub fn compile_mopp(
    vertices: &[[f32; 3]],
    triangles: &[[u32; 3]],
    radius: f32,
) -> Result<(Vec<u8>, [f32; 3], f32), String> {
    if triangles.is_empty() {
        return Ok((Vec::new(), [0.0; 3], 0.0));
    }
    for triangle in triangles {
        if triangle
            .iter()
            .any(|index| *index as usize >= vertices.len())
        {
            return Err("MOPP triangle references a missing vertex".to_string());
        }
    }
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for triangle in triangles {
        for index in triangle {
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(vertices[*index as usize][axis]);
                maximum[axis] = maximum[axis].max(vertices[*index as usize][axis]);
            }
        }
    }
    let origin = minimum.map(|value| value - radius);
    let largest_dimension = (0..3)
        .map(|axis| maximum[axis] - minimum[axis] + 2.0 * radius)
        .fold(0.0f32, f32::max)
        .max(1e-6);
    let scale = 254.0 * 256.0 * 256.0 / largest_dimension;
    let triangle_info = triangles
        .iter()
        .enumerate()
        .map(|(triangle_index, triangle)| {
            let mut triangle_minimum = [f32::INFINITY; 3];
            let mut triangle_maximum = [f32::NEG_INFINITY; 3];
            for index in triangle {
                for axis in 0..3 {
                    triangle_minimum[axis] =
                        triangle_minimum[axis].min(vertices[*index as usize][axis] - radius);
                    triangle_maximum[axis] =
                        triangle_maximum[axis].max(vertices[*index as usize][axis] + radius);
                }
            }
            TriangleInfo {
                output_id: triangle_index as u32,
                minimum: triangle_minimum,
                maximum: triangle_maximum,
                centroid: std::array::from_fn(|axis| {
                    (triangle_minimum[axis] + triangle_maximum[axis]) * 0.5
                }),
            }
        })
        .collect::<Vec<_>>();
    let root = build_bvh(triangle_info, 0);
    let mut code = root_filters(&root, origin, largest_dimension);
    code.extend(encode_node(&root, origin, largest_dimension));
    Ok((code, origin, scale))
}

fn build_bvh(mut triangles: Vec<TriangleInfo>, depth: usize) -> BvhNode {
    let minimum = std::array::from_fn(|axis| {
        triangles
            .iter()
            .map(|triangle| triangle.minimum[axis])
            .fold(f32::INFINITY, f32::min)
    });
    let maximum = std::array::from_fn(|axis| {
        triangles
            .iter()
            .map(|triangle| triangle.maximum[axis])
            .fold(f32::NEG_INFINITY, f32::max)
    });
    if triangles.len() <= 1 || depth > 40 {
        return BvhNode {
            triangles,
            left: None,
            right: None,
            axis: 0,
            minimum,
            maximum,
        };
    }
    let extents = std::array::from_fn::<_, 3, _>(|axis| maximum[axis] - minimum[axis]);
    let maximum_extent = extents.into_iter().fold(f32::NEG_INFINITY, f32::max);
    let tied = (0..3)
        .filter(|axis| (extents[*axis] - maximum_extent).abs() < 1e-9)
        .collect::<Vec<_>>();
    let axis = tied[depth % tied.len()];
    triangles.sort_by(|left, right| left.centroid[axis].total_cmp(&right.centroid[axis]));
    let right_triangles = triangles.split_off(triangles.len() / 2);
    let left = build_bvh(triangles, depth + 1);
    let right = build_bvh(right_triangles, depth + 1);
    BvhNode {
        triangles: Vec::new(),
        left: Some(Box::new(left)),
        right: Some(Box::new(right)),
        axis,
        minimum,
        maximum,
    }
}

fn encode_node(node: &BvhNode, origin: [f32; 3], dimension: f32) -> Vec<u8> {
    let (Some(left), Some(right)) = (&node.left, &node.right) else {
        let mut code = Vec::new();
        for triangle in &node.triangles {
            for axis in 0..3 {
                code.push(0x26 + axis as u8);
                code.push(lower_bound(triangle.minimum[axis], origin[axis], dimension));
                code.push(upper_bound(triangle.maximum[axis], origin[axis], dimension));
            }
            emit_leaf(&mut code, triangle.output_id);
        }
        return code;
    };
    let left_code = encode_node(left, origin, dimension);
    let right_code = encode_node(right, origin, dimension);
    let mut code = Vec::new();
    let upper = upper_bound(left.maximum[node.axis], origin[node.axis], dimension);
    let lower = lower_bound(right.minimum[node.axis], origin[node.axis], dimension);
    if left_code.len() <= u8::MAX as usize {
        code.extend([0x10 + node.axis as u8, upper, lower, left_code.len() as u8]);
    } else if left_code.len() <= u16::MAX as usize {
        let offset = left_code.len() as u16;
        code.extend([
            0x23 + node.axis as u8,
            upper,
            lower,
            0,
            0,
            (offset >> 8) as u8,
            offset as u8,
        ]);
    } else {
        collect_leaves(node, &mut code);
        return code;
    }
    code.extend(left_code);
    code.extend(right_code);
    code
}

fn collect_leaves(node: &BvhNode, code: &mut Vec<u8>) {
    if node.left.is_none() && node.right.is_none() {
        for triangle in &node.triangles {
            emit_leaf(code, triangle.output_id);
        }
        return;
    }
    if let Some(left) = &node.left {
        collect_leaves(left, code);
    }
    if let Some(right) = &node.right {
        collect_leaves(right, code);
    }
}

fn emit_leaf(code: &mut Vec<u8>, output_id: u32) {
    if output_id <= 0x1f {
        code.push(0x30 + output_id as u8);
    } else if output_id <= u8::MAX as u32 {
        code.extend([0x50, output_id as u8]);
    } else if output_id <= u16::MAX as u32 {
        code.extend([0x51, (output_id >> 8) as u8, output_id as u8]);
    } else {
        code.extend([
            0x52,
            (output_id >> 16) as u8,
            (output_id >> 8) as u8,
            output_id as u8,
        ]);
    }
}

fn lower_bound(value: f32, origin: f32, dimension: f32) -> u8 {
    (254.0 * (value - origin) / dimension)
        .floor()
        .clamp(0.0, 255.0) as u8
}

fn upper_bound(value: f32, origin: f32, dimension: f32) -> u8 {
    (1.0 + 254.0 * (value - origin) / dimension)
        .floor()
        .clamp(0.0, 255.0) as u8
}

fn root_filters(node: &BvhNode, origin: [f32; 3], dimension: f32) -> Vec<u8> {
    let mut filters = (0..3)
        .map(|axis| {
            (
                lower_bound(node.minimum[axis], origin[axis], dimension),
                upper_bound(node.maximum[axis], origin[axis], dimension),
            )
        })
        .collect::<Vec<_>>();
    let largest_upper = filters.iter().map(|filter| filter.1).max().unwrap_or(0);
    if largest_upper < u8::MAX {
        if let Some(filter) = filters.iter_mut().find(|filter| filter.1 == largest_upper) {
            filter.1 = u8::MAX;
        }
    }
    let mut code = Vec::with_capacity(9);
    for (axis, (lower, upper)) in filters.into_iter().enumerate() {
        code.extend([0x26 + axis as u8, lower, upper]);
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_stable_spatial_mopp_bytecode() {
        let vertices = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        let triangles = [[0, 1, 2], [2, 1, 3]];
        let (code, origin, scale) = compile_mopp(&vertices, &triangles, 0.1).unwrap();
        assert_eq!(origin, [-0.1, -0.1, -0.1]);
        assert!(scale > 0.0);
        assert!(code.len() > 20);
        assert_eq!(&code[..3], &[0x26, 0, 0xff]);
    }
}
