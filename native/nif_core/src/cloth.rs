use indexmap::IndexMap;
use thiserror::Error;

use crate::model::{NifFile, NifValue};

const CLOTH_BLOCK_TYPE: &str = "BSClothExtraData";

#[derive(Debug, Error)]
pub enum NifClothError {
    #[error("{0}")]
    InvalidInput(String),
}

pub type NifClothResult<T> = Result<T, NifClothError>;

pub fn extract_cloth_blob(nif_bytes: &[u8]) -> NifClothResult<Vec<u8>> {
    let nif = read_nif(nif_bytes)?;
    let block = nif
        .blocks
        .iter()
        .find(|block| block.type_name == CLOTH_BLOCK_TYPE)
        .ok_or_else(|| NifClothError::InvalidInput(format!("No {CLOTH_BLOCK_TYPE} block")))?;
    let binary_data = block.get_field("Binary Data").ok_or_else(|| {
        NifClothError::InvalidInput(format!("{CLOTH_BLOCK_TYPE} block missing Binary Data"))
    })?;
    byte_array_to_bytes(binary_data)
}

pub fn extract_cloth_blobs(nif_bytes: &[u8]) -> NifClothResult<Vec<Vec<u8>>> {
    let nif = read_nif(nif_bytes)?;
    nif.blocks
        .iter()
        .filter(|block| block.type_name == CLOTH_BLOCK_TYPE)
        .map(|block| {
            let binary_data = block.get_field("Binary Data").ok_or_else(|| {
                NifClothError::InvalidInput(format!("{CLOTH_BLOCK_TYPE} block missing Binary Data"))
            })?;
            byte_array_to_bytes(binary_data)
        })
        .collect()
}

pub fn pack_cloth_blob(nif_bytes: &[u8], blob: &[u8]) -> NifClothResult<Vec<u8>> {
    match extract_cloth_blob(nif_bytes) {
        Ok(existing) if existing == blob => return Ok(nif_bytes.to_vec()),
        Ok(_) | Err(NifClothError::InvalidInput(_)) => {}
    }

    let mut nif = read_nif(nif_bytes)?;
    let cloth_block_id = match nif
        .blocks
        .iter()
        .position(|block| block.type_name == CLOTH_BLOCK_TYPE)
    {
        Some(index) => index,
        None => create_cloth_block(&mut nif)?,
    };

    let block = nif.blocks.get_mut(cloth_block_id).ok_or_else(|| {
        NifClothError::InvalidInput(format!("{CLOTH_BLOCK_TYPE} block index out of bounds"))
    })?;
    block.set_field("Binary Data", bytes_to_byte_array(blob));

    nif.to_bytes()
        .map_err(|error| NifClothError::InvalidInput(format!("failed to write NIF: {error}")))
}

pub fn apply_cloth_template(
    name: &str,
    source_nif_bytes: &[u8],
    args_json: &str,
) -> NifClothResult<Vec<u8>> {
    let blob = havok_native::api::cloth_template_blob(name, args_json)
        .map_err(|error| NifClothError::InvalidInput(error.to_string()))?;
    pack_cloth_blob(source_nif_bytes, &blob)
}

fn read_nif(nif_bytes: &[u8]) -> NifClothResult<NifFile> {
    NifFile::from_bytes(nif_bytes, None)
        .map_err(|error| NifClothError::InvalidInput(format!("failed to read NIF: {error}")))
}

fn create_cloth_block(nif: &mut NifFile) -> NifClothResult<usize> {
    let root_id = nif
        .header
        .footer_roots
        .first()
        .copied()
        .filter(|root| *root >= 0)
        .map(|root| root as usize)
        .unwrap_or(0);
    if root_id >= nif.blocks.len() {
        return Err(NifClothError::InvalidInput(format!(
            "cannot add {CLOTH_BLOCK_TYPE}: root block {root_id} missing"
        )));
    }

    let cloth_block_id = nif.add_block(CLOTH_BLOCK_TYPE, None);
    let root = nif
        .blocks
        .get_mut(root_id)
        .expect("root block bounds checked above");
    let mut extra_data = match root.get_field("Extra Data List") {
        Some(NifValue::Array(values)) => values.clone(),
        _ => Vec::new(),
    };
    extra_data.push(NifValue::Ref(cloth_block_id as i32));
    let extra_data_len = extra_data.len();
    root.set_field("Extra Data List", NifValue::Array(extra_data));
    root.set_field("Num Extra Data List", NifValue::UInt(extra_data_len as u64));

    let block = nif
        .blocks
        .get_mut(cloth_block_id)
        .expect("new block id returned by add_block");
    block.set_field("Name", NifValue::String("CES".to_string()));
    Ok(cloth_block_id)
}

pub(crate) fn byte_array_to_bytes(value: &NifValue) -> NifClothResult<Vec<u8>> {
    let NifValue::Struct(fields) = value else {
        return Err(NifClothError::InvalidInput(
            "Binary Data is not a ByteArray struct".to_string(),
        ));
    };
    let size = fields.get("Data Size").map(NifValue::as_usize).unwrap_or(0);
    let data = fields
        .get("Data")
        .ok_or_else(|| NifClothError::InvalidInput("ByteArray missing Data".to_string()))?;
    let mut bytes = match data {
        NifValue::Bytes(bytes) => bytes.clone(),
        NifValue::Array(values) => values
            .iter()
            .map(|value| (value.as_i64() & 0xFF) as u8)
            .collect::<Vec<_>>(),
        _ => {
            return Err(NifClothError::InvalidInput(
                "ByteArray Data is not byte storage".to_string(),
            ));
        }
    };
    if size > bytes.len() {
        return Err(NifClothError::InvalidInput(format!(
            "ByteArray Data Size {size} exceeds stored byte count {}",
            bytes.len()
        )));
    }
    bytes.truncate(size);
    Ok(bytes)
}

pub(crate) fn bytes_to_byte_array(bytes: &[u8]) -> NifValue {
    let mut fields = IndexMap::new();
    fields.insert("Data Size".to_string(), NifValue::UInt(bytes.len() as u64));
    fields.insert("Data".to_string(), NifValue::Bytes(bytes.to_vec()));
    NifValue::Struct(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nif_with_binary_data(binary_data: NifValue) -> NifFile {
        let mut nif = NifFile::new("fo4");
        nif.add_block(
            "bhkPhysicsSystem",
            Some(IndexMap::from([("Binary Data".to_string(), binary_data)])),
        );
        nif
    }

    fn legacy_byte_array(bytes: &[u8], declared_size: usize) -> NifValue {
        NifValue::Struct(IndexMap::from([
            (
                "Data Size".to_string(),
                NifValue::UInt(declared_size as u64),
            ),
            (
                "Data".to_string(),
                NifValue::Array(
                    bytes
                        .iter()
                        .map(|byte| NifValue::UInt(u64::from(*byte)))
                        .collect(),
                ),
            ),
        ]))
    }

    #[test]
    fn compact_byte_array_matches_legacy_array_encoding() {
        for payload in [&[][..], &[0, 1, 127, 128, 254, 255][..]] {
            let compact = bytes_to_byte_array(payload);
            let NifValue::Struct(fields) = &compact else {
                panic!("expected ByteArray struct");
            };
            assert!(matches!(fields.get("Data"), Some(NifValue::Bytes(bytes)) if bytes == payload));

            let compact_bytes = nif_with_binary_data(compact).to_bytes().unwrap();
            let legacy_bytes = nif_with_binary_data(legacy_byte_array(payload, payload.len()))
                .to_bytes()
                .unwrap();
            assert_eq!(compact_bytes, legacy_bytes);

            let decoded = NifFile::from_bytes(&compact_bytes, None).unwrap();
            let binary_data = decoded
                .blocks
                .iter()
                .find(|block| block.type_name == "bhkPhysicsSystem")
                .and_then(|block| block.get_field("Binary Data"))
                .unwrap();
            assert_eq!(byte_array_to_bytes(binary_data).unwrap(), payload);
        }
    }

    #[test]
    fn byte_array_size_mismatch_is_rejected_for_both_representations() {
        let compact = NifValue::Struct(IndexMap::from([
            ("Data Size".to_string(), NifValue::UInt(3)),
            ("Data".to_string(), NifValue::Bytes(vec![1, 2])),
        ]));
        let legacy = legacy_byte_array(&[1, 2], 3);

        for binary_data in [compact, legacy] {
            let error = nif_with_binary_data(binary_data).to_bytes().unwrap_err();
            assert!(error.to_string().contains("ByteArray Data Size mismatch"));
        }
    }
}
