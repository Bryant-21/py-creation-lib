//! FO4 `BSDistantObjectInstanceDataBlock`, written as a raw NIF block: nif.xml has no entry and must not
//! change. Layout per LOD_RUNTIME.md Q8.
use nif_core_native::model::NifFile;

pub const BLOCK_TYPE: &str = "BSDistantObjectInstanceDataBlock";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    pub pos: [f32; 3],
    pub scale: f32,
    /// Quaternion x, y, z, w; identity is `[0, 0, 0, 1]`.
    pub rot: [f32; 4],
}

pub fn encode(object: u32, instances: &[Instance], ids: &[u32]) -> Vec<u8> {
    assert_eq!(instances.len(), ids.len(), "one id per instance");
    let mut out = Vec::with_capacity(8 + instances.len() * 36);
    out.extend_from_slice(&object.to_le_bytes());
    out.extend_from_slice(&(instances.len() as u32).to_le_bytes());
    for i in instances {
        for v in i.pos.iter().chain(std::iter::once(&i.scale)).chain(i.rot.iter()) {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    for id in ids {
        out.extend_from_slice(&id.to_le_bytes());
    }
    out
}

pub fn decode(bytes: &[u8]) -> anyhow::Result<(u32, Vec<Instance>, Vec<u32>)> {
    let u32_at = |o: usize| -> anyhow::Result<u32> {
        let slice = bytes.get(o..o + 4).ok_or_else(|| anyhow::anyhow!("short block"))?;
        Ok(u32::from_le_bytes(slice.try_into()?))
    };
    let f32_at = |o: usize| -> anyhow::Result<f32> { Ok(f32::from_bits(u32_at(o)?)) };
    let object = u32_at(0)?;
    let count = u32_at(4)? as usize;
    let mut instances = Vec::with_capacity(count.min(bytes.len() / 36));
    for n in 0..count {
        let b = 8 + n * 32;
        instances.push(Instance {
            pos: [f32_at(b)?, f32_at(b + 4)?, f32_at(b + 8)?],
            scale: f32_at(b + 12)?,
            rot: [f32_at(b + 16)?, f32_at(b + 20)?, f32_at(b + 24)?, f32_at(b + 28)?],
        });
    }
    let ids_at = 8 + count * 32;
    let ids = (0..count).map(|n| u32_at(ids_at + n * 4)).collect::<anyhow::Result<Vec<_>>>()?;
    Ok((object, instances, ids))
}

pub fn add_to_nif(nif: &mut NifFile, object: u32, instances: &[Instance], ids: &[u32]) -> usize {
    let index = nif.add_block(BLOCK_TYPE, None);
    nif.blocks[index].remainder = encode(object, instances, ids);
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (u32, Vec<Instance>, Vec<u32>) {
        let instances = vec![
            Instance { pos: [1.0, 2.0, 3.0], scale: 1.0, rot: [1.0, 0.0, 0.0, 0.0] },
            Instance { pos: [-4.0, 5.5, 0.0], scale: 2.5, rot: [0.0, 0.0, 0.0, 1.0] },
        ];
        (7, instances, vec![0x0010_0001, 0x0010_0002])
    }

    #[test]
    fn encode_decode_round_trip() {
        let (object, instances, ids) = sample();
        let bytes = encode(object, &instances, &ids);
        let (o, i, d) = decode(&bytes).unwrap();
        assert_eq!((o, i, d), (object, instances, ids));
    }

    #[test]
    fn byte_layout_matches_q8() {
        let (object, instances, ids) = sample();
        let bytes = encode(object, &instances, &ids);
        // Q8: u32 object, u32 count, count * 32-byte Instance, count * u32 id.
        assert_eq!(bytes.len(), 4 + 4 + 2 * 32 + 2 * 4);
        assert_eq!(&bytes[0..4], &7u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &2u32.to_le_bytes());
    }

    #[test]
    fn nif_round_trip_keeps_the_raw_block() {
        let (object, instances, ids) = sample();
        let mut nif = nif_core_native::model::NifFile::new("FO4");
        let root = nif.add_block("NiNode", None);
        nif.header.footer_roots = vec![root as i32];
        let index = add_to_nif(&mut nif, object, &instances, &ids);
        let bytes = nif.to_bytes().unwrap();
        let back = nif_core_native::model::NifFile::from_bytes(&bytes, None).unwrap();
        assert_eq!(back.blocks[index].type_name, "BSDistantObjectInstanceDataBlock");
        assert_eq!(decode(&back.blocks[index].remainder).unwrap(), (object, instances, ids));
    }
}

