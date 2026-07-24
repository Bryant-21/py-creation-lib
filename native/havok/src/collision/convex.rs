use super::compressed_mesh::BuildOptions;
use super::mass_properties::CompressedMassProperties;
use super::polytope::{SourcePolytopeShape, build_fo4_source_polytope_collision};
use crate::error::{HavokError, HavokResult};
use crate::hkx::model::HkxFile;
use crate::hkx::types::HkxValue;

#[derive(Debug, Clone, PartialEq)]
pub struct SourceConvexShape {
    pub vertices: Vec<[f32; 4]>,
    pub convex_radius: f32,
    pub mass_properties: Option<CompressedMassProperties>,
}

impl SourceConvexShape {
    pub fn validate(&self) -> HavokResult<()> {
        if self.vertices.is_empty() {
            return Err(HavokError::InvalidInput(
                "source convex shape has no vertices".to_string(),
            ));
        }
        if self.vertices.len() > 252 {
            return Err(HavokError::InvalidInput(format!(
                "source convex shape has {} vertices; Havok supports at most 252",
                self.vertices.len()
            )));
        }
        if self
            .vertices
            .iter()
            .any(|vertex| !vertex.iter().all(|value| value.is_finite()))
        {
            return Err(HavokError::InvalidInput(
                "source convex shape has a non-finite vertex".to_string(),
            ));
        }
        if !self.convex_radius.is_finite() || self.convex_radius < 0.0 {
            return Err(HavokError::InvalidInput(format!(
                "source convex shape has invalid convex radius {}",
                self.convex_radius
            )));
        }
        Ok(())
    }
}

pub fn build_fo4_source_convex_collision(
    shape: &SourceConvexShape,
    opts: &BuildOptions,
) -> HavokResult<Vec<u8>> {
    shape.validate()?;

    let mut template_opts = opts.clone();
    template_opts.convex_radius = shape.convex_radius;
    let template_shape = source_convex_template(shape.mass_properties.clone());
    let template = build_fo4_source_polytope_collision(&template_shape, &template_opts)?;
    let file = HkxFile::read(&template)?;
    let mut objects = file.objects().to_vec();
    let object = objects
        .iter_mut()
        .find(|object| object.class_name == "hknpConvexPolytopeShape")
        .ok_or_else(|| {
            HavokError::InvalidInput(
                "polytope template missing hknpConvexPolytopeShape".to_string(),
            )
        })?;

    object.class_name = "hknpConvexShape".to_string();
    object.signature = 0xC8F7_C10D;
    object.members.retain(|member| {
        matches!(
            member.name.as_str(),
            "flags"
                | "numShapeKeyBits"
                | "dispatchType"
                | "convexRadius"
                | "userData"
                | "properties"
                | "vertices"
        )
    });
    let flags = if shape.vertices.len() == 1 { 17 } else { 1 };
    set_int_member(&mut object.members, "flags", flags);
    set_int_member(&mut object.members, "dispatchType", 1);
    if let Some(member) = object
        .members
        .iter_mut()
        .find(|member| member.name == "convexRadius")
    {
        member.value = HkxValue::F32(shape.convex_radius);
    }
    if let Some(member) = object
        .members
        .iter_mut()
        .find(|member| member.name == "vertices")
    {
        let mut vertices = shape.vertices.clone();
        while vertices.len() % 4 != 0 {
            vertices.push(*vertices.last().expect("validated non-empty vertices"));
        }
        member.value = HkxValue::Array(
            vertices
                .iter()
                .map(|vertex| HkxValue::F32List(vertex.to_vec()))
                .collect(),
        );
    }

    Ok(HkxFile::from_tagxml(11, "hk_2014.1.0-r1", objects).save())
}

fn set_int_member(members: &mut [crate::hkx::model::HkxMember], name: &str, value: u64) {
    if let Some(member) = members.iter_mut().find(|member| member.name == name) {
        member.value = match member.value {
            HkxValue::I8(_) => HkxValue::I8(value as i8),
            HkxValue::U8(_) => HkxValue::U8(value as u8),
            HkxValue::I16(_) => HkxValue::I16(value as i16),
            HkxValue::U16(_) => HkxValue::U16(value as u16),
            HkxValue::I32(_) => HkxValue::I32(value as i32),
            HkxValue::U64(_) => HkxValue::U64(value),
            _ => HkxValue::U32(value as u32),
        };
    }
}

fn source_convex_template(
    mass_properties: Option<CompressedMassProperties>,
) -> SourcePolytopeShape {
    SourcePolytopeShape {
        vertices: vec![
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ],
        planes: vec![
            [0.0, 0.0, -1.0, -1.0],
            [0.0, 0.0, 1.0, -1.0],
            [0.0, -1.0, 0.0, -1.0],
            [1.0, 0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0, -1.0],
            [-1.0, 0.0, 0.0, -1.0],
        ],
        faces: (0..6).map(|index| (index * 4, 4, 128)).collect(),
        indices: vec![
            0, 1, 2, 3, 4, 7, 6, 5, 0, 4, 5, 1, 1, 5, 6, 2, 2, 6, 7, 3, 3, 7, 4, 0,
        ],
        convex_radius: 0.0,
        mass_properties,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_convex_round_trip_keeps_native_class_vertices_and_radius() {
        let shape = SourceConvexShape {
            vertices: vec![
                [-1.0, 0.0, 0.0, 0.5],
                [1.0, 0.0, 0.0, f32::from_bits(0x3F00_0001)],
                [1.0, 2.0, 0.0, f32::from_bits(0x3F00_0002)],
                [-1.0, 2.0, 0.0, f32::from_bits(0x3F00_0003)],
            ],
            convex_radius: 0.025,
            mass_properties: None,
        };
        let blob = build_fo4_source_convex_collision(&shape, &BuildOptions::default())
            .expect("build source convex");
        let file = HkxFile::read(&blob).expect("parse source convex output");
        let output = file
            .objects()
            .iter()
            .find(|object| object.class_name == "hknpConvexShape")
            .expect("native convex output");
        assert!(
            file.objects()
                .iter()
                .all(|object| object.class_name != "hknpConvexPolytopeShape")
        );
        let vertices = output
            .members
            .iter()
            .find(|member| member.name == "vertices")
            .and_then(|member| match &member.value {
                HkxValue::Array(values) => Some(values),
                _ => None,
            })
            .expect("vertices");
        assert_eq!(vertices.len(), shape.vertices.len());
        assert_eq!(vertices[3], HkxValue::F32List(shape.vertices[3].to_vec()));
        assert_eq!(
            output
                .members
                .iter()
                .find(|member| member.name == "convexRadius")
                .map(|member| &member.value),
            Some(&HkxValue::F32(shape.convex_radius))
        );
    }

    #[test]
    fn single_vertex_convex_uses_havok_sphere_support_layout() {
        let shape = SourceConvexShape {
            vertices: vec![[1.0, 2.0, 3.0, 0.5]],
            convex_radius: 0.25,
            mass_properties: None,
        };
        let blob = build_fo4_source_convex_collision(&shape, &BuildOptions::default())
            .expect("build single-vertex convex");
        let file = HkxFile::read(&blob).expect("parse single-vertex convex");
        let output = file
            .objects()
            .iter()
            .find(|object| object.class_name == "hknpConvexShape")
            .expect("native convex output");
        let vertices = output
            .members
            .iter()
            .find(|member| member.name == "vertices")
            .and_then(|member| match &member.value {
                HkxValue::Array(values) => Some(values),
                _ => None,
            })
            .expect("vertices");
        assert_eq!(
            vertices.len(),
            4,
            "support vertices are padded to SIMD width"
        );
        assert!(
            vertices
                .iter()
                .all(|vertex| { vertex == &HkxValue::F32List(shape.vertices[0].to_vec()) })
        );
        assert_eq!(
            output
                .members
                .iter()
                .find(|member| member.name == "flags")
                .map(|member| &member.value),
            Some(&HkxValue::U16(17))
        );
    }
}
