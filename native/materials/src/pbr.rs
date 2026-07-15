use crate::error::{MaterialError, Result};

const DIELECTRIC_SPECULAR_FILL: f32 = 0.22;

#[derive(Debug, Clone, Copy)]
pub struct PbrToSpecGlossParams {
    pub ao_multiplier: f32,
    pub specular_multiplier: f32,
    pub gloss_multiplier: f32,
    pub spec_offset: f32,
}

pub struct PbrToSpecGlossBuffers {
    pub diffuse: Vec<f32>,
    pub specular: Vec<f32>,
    pub gloss: Vec<f32>,
}

pub fn convert_buffers(
    albedo_bytes: &[u8],
    metallic_bytes: &[u8],
    roughness_bytes: &[u8],
    ao_bytes: Option<&[u8]>,
    pixel_count: usize,
    params: PbrToSpecGlossParams,
) -> Result<PbrToSpecGlossBuffers> {
    let albedo = read_f32_bytes(albedo_bytes, pixel_count * 3, "albedo")?;
    let metallic = read_f32_bytes(metallic_bytes, pixel_count, "metallic")?;
    let roughness = read_f32_bytes(roughness_bytes, pixel_count, "roughness")?;
    let ao = match ao_bytes {
        Some(bytes) => Some(read_f32_bytes(bytes, pixel_count, "ao")?),
        None => None,
    };

    let mut diffuse = Vec::with_capacity(pixel_count * 3);
    let mut specular = Vec::with_capacity(pixel_count * 3);
    let mut gloss = Vec::with_capacity(pixel_count);

    for idx in 0..pixel_count {
        let albedo_idx = idx * 3;
        let converted = convert_pixel(
            [
                albedo[albedo_idx],
                albedo[albedo_idx + 1],
                albedo[albedo_idx + 2],
            ],
            metallic[idx],
            roughness[idx],
            ao.as_ref().map(|values| values[idx]),
            params,
        );
        diffuse.extend_from_slice(&converted.diffuse);
        specular.extend_from_slice(&converted.specular);
        gloss.push(converted.gloss);
    }

    Ok(PbrToSpecGlossBuffers {
        diffuse,
        specular,
        gloss,
    })
}

struct PbrToSpecGlossPixel {
    diffuse: [f32; 3],
    specular: [f32; 3],
    gloss: f32,
}

fn convert_pixel(
    albedo: [f32; 3],
    metallic: f32,
    roughness: f32,
    ao: Option<f32>,
    params: PbrToSpecGlossParams,
) -> PbrToSpecGlossPixel {
    let threshold = 1.0 - params.spec_offset;
    let denom = params.spec_offset.max(1e-6);
    let metal = ((metallic.clamp(0.0, 1.0) - threshold).max(0.0) / denom).clamp(0.0, 1.0);
    let ao_term = match ao {
        Some(value) => (1.0 - params.ao_multiplier) + value.clamp(0.0, 1.0) * params.ao_multiplier,
        None => 1.0,
    };

    let mut diffuse = [0.0; 3];
    let mut specular_sum = 0.0;
    for channel in 0..3 {
        let diffuse_base = albedo[channel].clamp(0.0, 1.0) + metal;
        diffuse[channel] = (diffuse_base * ao_term).clamp(0.0, 1.0);
        specular_sum += DIELECTRIC_SPECULAR_FILL * (1.0 - metal) + diffuse_base * metal;
    }

    let specular_value = ((specular_sum / 3.0) * params.specular_multiplier).clamp(0.0, 1.0);
    let gloss = ((1.0 - roughness.clamp(0.0, 1.0)) * params.gloss_multiplier).clamp(0.0, 1.0);

    PbrToSpecGlossPixel {
        diffuse,
        specular: [specular_value; 3],
        gloss,
    }
}

pub fn f32_vec_to_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn read_f32_bytes(bytes: &[u8], expected_len: usize, name: &str) -> Result<Vec<f32>> {
    let expected_bytes = expected_len
        .checked_mul(4)
        .ok_or_else(|| MaterialError::invalid(format!("{name} buffer is too large")))?;
    if bytes.len() != expected_bytes {
        return Err(MaterialError::invalid(format!(
            "{name} buffer has {} bytes; expected {expected_bytes}",
            bytes.len()
        )));
    }

    let mut values = Vec::with_capacity(expected_len);
    for chunk in bytes.chunks_exact(4) {
        values.push(f32::from_le_bytes(
            chunk.try_into().expect("chunks_exact(4) yields 4 bytes"),
        ));
    }
    Ok(values)
}
