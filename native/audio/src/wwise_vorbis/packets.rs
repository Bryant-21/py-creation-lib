use super::bits::{BitReader, BitWriter};

pub(crate) struct LongWindow {
    pub(crate) previous_long: bool,
    pub(crate) next_long: bool,
}

pub(crate) fn packet_mode(packet: &[u8], mode_bits: u32) -> Result<usize, String> {
    Ok(BitReader::new(packet).read(mode_bits)? as usize)
}

/// Wwise audio packets open straight on the mode number. Vorbis wants the
/// audio packet type bit before it and, for long blocks, whether each
/// neighbouring block is long, which Wwise leaves to be recovered.
pub(crate) fn rebuild_audio_packet(
    packet: &[u8],
    mode_bits: u32,
    long_window: Option<LongWindow>,
) -> Result<Vec<u8>, String> {
    let mut input = BitReader::new(packet);
    let mut out = BitWriter::new();
    out.write(0, 1);
    out.write(input.read(mode_bits)?, mode_bits);
    if let Some(window) = long_window {
        out.write(u32::from(window.previous_long), 1);
        out.write(u32::from(window.next_long), 1);
    }
    let mut remaining = packet.len() * 8 - mode_bits as usize;
    while remaining > 0 {
        let chunk = remaining.min(32) as u32;
        out.write(input.read(chunk)?, chunk);
        remaining -= chunk as usize;
    }
    Ok(out.into_bytes())
}

/// A Vorbis packet yields the quarter blocks either side of its overlap with
/// the previous packet, so the first packet yields nothing.
pub(crate) fn granule_positions(block_sizes: &[u32], sample_count: u64) -> Vec<u64> {
    let mut position = 0_u64;
    let mut previous: Option<u32> = None;
    block_sizes
        .iter()
        .map(|&size| {
            if let Some(previous) = previous {
                position += u64::from(previous / 4 + size / 4);
            }
            previous = Some(size);
            position.min(sample_count)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_mode_is_the_leading_bits() {
        assert_eq!(packet_mode(&[0b0000_0110], 2).unwrap(), 2);
        assert!(packet_mode(&[], 1).is_err());
    }

    #[test]
    fn short_block_packet_gains_a_leading_audio_type_bit() {
        assert_eq!(
            rebuild_audio_packet(&[0xAA], 1, None).unwrap(),
            vec![0x54, 0x01]
        );
    }

    #[test]
    fn long_block_packet_gains_neighbouring_window_flags() {
        let window = LongWindow {
            previous_long: true,
            next_long: false,
        };
        assert_eq!(
            rebuild_audio_packet(&[0x03], 1, Some(window)).unwrap(),
            vec![0x16, 0x00]
        );
    }

    #[test]
    fn granules_count_overlapped_quarter_blocks_and_stop_at_the_sample_count() {
        let blocks = [256, 2048, 2048, 256];
        assert_eq!(granule_positions(&blocks, 10_000), vec![0, 576, 1600, 2176]);
        assert_eq!(granule_positions(&blocks, 2_000), vec![0, 576, 1600, 2000]);
    }
}
