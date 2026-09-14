const ATRAC9_SUBFORMAT_GUID: [u8; 16] = [
    0xd2, 0x42, 0xe1, 0x47, 0xba, 0x36, 0x8d, 0x4d, 0x88, 0xfc, 0x61, 0x65, 0x4f, 0x8c, 0x83, 0x6c,
];

pub(crate) fn is_atrac9_wave(payload: &[u8]) -> bool {
    if payload.len() < 12 || &payload[..4] != b"RIFF" || &payload[8..12] != b"WAVE" {
        return false;
    }
    let mut offset = 12usize;
    while let Some(header_end) = offset.checked_add(8) {
        if header_end > payload.len() {
            return false;
        }
        let Ok(size_bytes) = payload[offset + 4..header_end].try_into() else {
            return false;
        };
        let chunk_size = u32::from_le_bytes(size_bytes) as usize;
        let chunk_data = header_end;
        let Some(chunk_end) = chunk_data.checked_add(chunk_size) else {
            return false;
        };
        if chunk_end > payload.len() {
            return false;
        }
        if &payload[offset..offset + 4] == b"fmt " {
            return chunk_size >= 40
                && payload[chunk_data..chunk_data + 2] == 0xfffe_u16.to_le_bytes()
                && payload[chunk_data + 24..chunk_data + 40] == ATRAC9_SUBFORMAT_GUID;
        }
        let Some(next) = chunk_end.checked_add(chunk_size & 1) else {
            return false;
        };
        offset = next;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atrac9_wave() -> Vec<u8> {
        let mut format = vec![0_u8; 40];
        format[..2].copy_from_slice(&0xfffe_u16.to_le_bytes());
        format[24..40].copy_from_slice(&ATRAC9_SUBFORMAT_GUID);
        let mut wave = b"RIFF".to_vec();
        wave.extend_from_slice(&52_u32.to_le_bytes());
        wave.extend_from_slice(b"WAVEfmt ");
        wave.extend_from_slice(&40_u32.to_le_bytes());
        wave.extend_from_slice(&format);
        wave
    }

    #[test]
    fn recognizes_atrac9_wave_extensible_subformat() {
        assert!(is_atrac9_wave(&atrac9_wave()));
    }

    #[test]
    fn rejects_other_wave_extensible_subformat() {
        let mut wave = atrac9_wave();
        wave[44] ^= 0xff;
        assert!(!is_atrac9_wave(&wave));
    }
}
