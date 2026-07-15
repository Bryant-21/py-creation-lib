use serde::{Deserialize, Serialize};

use crate::error::{MaterialError, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BaseHeader {
    pub signature: u32,
    pub version: u32,
    pub tile_u: bool,
    pub tile_v: bool,
    pub u_offset: f32,
    pub v_offset: f32,
    pub u_scale: f32,
    pub v_scale: f32,
    pub alpha: f32,
    pub alpha_blend_mode0: u8,
    pub alpha_blend_mode1: u32,
    pub alpha_blend_mode2: u32,
    pub alpha_test_ref: u8,
    pub alpha_test: bool,
    pub zbuffer_write: bool,
    pub zbuffer_test: bool,
    pub ssr: bool,
    pub wet_ssr: bool,
    pub decal: bool,
    pub two_sided: bool,
    pub decal_nofade: bool,
    pub non_occluder: bool,
    pub refraction: bool,
    pub refraction_falloff: bool,
    pub refraction_power: f32,
    pub env_mapping: Option<bool>,
    pub env_mapping_mask_scale: Option<f32>,
    pub depth_bias: Option<bool>,
    pub grayscale_to_palette_color: bool,
    pub mask_writes: Option<u8>,
}

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| MaterialError::invalid("unexpected EOF"))?;
        if end > self.data.len() {
            return Err(MaterialError::invalid("unexpected EOF"));
        }
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_exact(1)?[0])
    }

    pub fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_u8()? != 0)
    }

    pub fn read_u32(&mut self) -> Result<u32> {
        let bytes: [u8; 4] = self
            .read_exact(4)?
            .try_into()
            .map_err(|_| MaterialError::invalid("unexpected EOF"))?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn read_f32(&mut self) -> Result<f32> {
        let bytes: [u8; 4] = self
            .read_exact(4)?
            .try_into()
            .map_err(|_| MaterialError::invalid("unexpected EOF"))?;
        Ok(f32::from_le_bytes(bytes))
    }

    pub fn read_string(&mut self) -> Result<String> {
        let len = self.read_u32()? as usize;
        if len == 0 {
            return Ok(String::new());
        }
        let bytes = self.read_exact(len)?;
        std::str::from_utf8(bytes)
            .map(|s| s.to_owned())
            .map_err(|err| MaterialError::invalid(err.to_string()))
    }
}

pub struct Writer {
    data: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn write_u8(&mut self, value: u8) {
        self.data.push(value);
    }

    pub fn write_bool(&mut self, value: bool) {
        self.write_u8(if value { 1 } else { 0 });
    }

    pub fn write_u32(&mut self, value: u32) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_f32(&mut self, value: f32) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_string(&mut self, value: &str) {
        if value.is_empty() {
            self.write_u32(0);
            return;
        }
        self.write_u32(value.len() as u32);
        self.data.extend_from_slice(value.as_bytes());
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }
}

pub fn read_color3(reader: &mut Reader<'_>) -> Result<[f32; 3]> {
    Ok([reader.read_f32()?, reader.read_f32()?, reader.read_f32()?])
}

pub fn write_color3(writer: &mut Writer, color: [f32; 3]) {
    writer.write_f32(color[0]);
    writer.write_f32(color[1]);
    writer.write_f32(color[2]);
}

pub fn read_header(reader: &mut Reader<'_>, expected_signature: u32) -> Result<BaseHeader> {
    let signature = reader.read_u32()?;
    if signature != expected_signature {
        return Err(MaterialError::invalid(format!(
            "Invalid signature: 0x{signature:08X}"
        )));
    }
    let version = reader.read_u32()?;
    let tile_flags = reader.read_u32()?;
    let tile_u = (tile_flags & 2) != 0;
    let tile_v = (tile_flags & 1) != 0;
    let u_offset = reader.read_f32()?;
    let v_offset = reader.read_f32()?;
    let u_scale = reader.read_f32()?;
    let v_scale = reader.read_f32()?;
    let alpha = reader.read_f32()?;
    let alpha_blend_mode0 = reader.read_u8()?;
    let alpha_blend_mode1 = reader.read_u32()?;
    let alpha_blend_mode2 = reader.read_u32()?;
    let alpha_test_ref = reader.read_u8()?;
    let alpha_test = reader.read_bool()?;
    let zbuffer_write = reader.read_bool()?;
    let zbuffer_test = reader.read_bool()?;
    let ssr = reader.read_bool()?;
    let wet_ssr = reader.read_bool()?;
    let decal = reader.read_bool()?;
    let two_sided = reader.read_bool()?;
    let decal_nofade = reader.read_bool()?;
    let non_occluder = reader.read_bool()?;
    let refraction = reader.read_bool()?;
    let refraction_falloff = reader.read_bool()?;
    let refraction_power = reader.read_f32()?;
    let (env_mapping, env_mapping_mask_scale, depth_bias) = if version < 10 {
        (Some(reader.read_bool()?), Some(reader.read_f32()?), None)
    } else {
        (None, None, Some(reader.read_bool()?))
    };
    let grayscale_to_palette_color = reader.read_bool()?;
    let mask_writes = if version >= 6 {
        Some(reader.read_u8()?)
    } else {
        None
    };

    Ok(BaseHeader {
        signature,
        version,
        tile_u,
        tile_v,
        u_offset,
        v_offset,
        u_scale,
        v_scale,
        alpha,
        alpha_blend_mode0,
        alpha_blend_mode1,
        alpha_blend_mode2,
        alpha_test_ref,
        alpha_test,
        zbuffer_write,
        zbuffer_test,
        ssr,
        wet_ssr,
        decal,
        two_sided,
        decal_nofade,
        non_occluder,
        refraction,
        refraction_falloff,
        refraction_power,
        env_mapping,
        env_mapping_mask_scale,
        depth_bias,
        grayscale_to_palette_color,
        mask_writes,
    })
}

pub fn write_header(writer: &mut Writer, header: &BaseHeader) {
    writer.write_u32(header.signature);
    writer.write_u32(header.version);
    let tile_flags = (if header.tile_u { 2 } else { 0 }) | (if header.tile_v { 1 } else { 0 });
    writer.write_u32(tile_flags);
    writer.write_f32(header.u_offset);
    writer.write_f32(header.v_offset);
    writer.write_f32(header.u_scale);
    writer.write_f32(header.v_scale);
    writer.write_f32(header.alpha);
    writer.write_u8(header.alpha_blend_mode0);
    writer.write_u32(header.alpha_blend_mode1);
    writer.write_u32(header.alpha_blend_mode2);
    writer.write_u8(header.alpha_test_ref);
    writer.write_bool(header.alpha_test);
    writer.write_bool(header.zbuffer_write);
    writer.write_bool(header.zbuffer_test);
    writer.write_bool(header.ssr);
    writer.write_bool(header.wet_ssr);
    writer.write_bool(header.decal);
    writer.write_bool(header.two_sided);
    writer.write_bool(header.decal_nofade);
    writer.write_bool(header.non_occluder);
    writer.write_bool(header.refraction);
    writer.write_bool(header.refraction_falloff);
    writer.write_f32(header.refraction_power);
    if header.version < 10 {
        writer.write_bool(header.env_mapping.unwrap_or(false));
        writer.write_f32(header.env_mapping_mask_scale.unwrap_or(0.0));
    } else {
        writer.write_bool(header.depth_bias.unwrap_or(false));
    }
    writer.write_bool(header.grayscale_to_palette_color);
    if header.version >= 6 {
        writer.write_u8(header.mask_writes.unwrap_or(0));
    }
}
