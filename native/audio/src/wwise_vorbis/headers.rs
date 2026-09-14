use super::bits::{BitReader, BitWriter, ilog};
use super::codebook::{CodebookTable, expand_codebook};

const SIGNATURE: &[u8; 6] = b"vorbis";

pub(crate) fn identification_header(
    channels: u8,
    sample_rate: u32,
    nominal_bitrate: u32,
    blocksize_exponents: (u8, u8),
) -> Vec<u8> {
    let mut header = Vec::with_capacity(30);
    header.push(1);
    header.extend_from_slice(SIGNATURE);
    header.extend_from_slice(&0_u32.to_le_bytes());
    header.push(channels);
    header.extend_from_slice(&sample_rate.to_le_bytes());
    header.extend_from_slice(&0_u32.to_le_bytes());
    header.extend_from_slice(&nominal_bitrate.to_le_bytes());
    header.extend_from_slice(&0_u32.to_le_bytes());
    header.push(blocksize_exponents.0 | blocksize_exponents.1 << 4);
    header.push(1);
    header
}

pub(crate) fn comment_header(vendor: &str) -> Vec<u8> {
    let mut header = vec![3];
    header.extend_from_slice(SIGNATURE);
    header.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    header.extend_from_slice(vendor.as_bytes());
    header.extend_from_slice(&0_u32.to_le_bytes());
    header.push(1);
    header
}

pub(crate) struct Setup {
    pub(crate) header: Vec<u8>,
    pub(crate) long_block_modes: Vec<bool>,
}

/// Wwise's setup packet is a Vorbis setup header with codebooks replaced by
/// 10-bit table ids and every field that has only one legal value removed:
/// time-domain transforms, floor type, mapping type, mode window and
/// transform types. The residue type survives, narrowed to 2 bits.
pub(crate) fn rebuild_setup(
    stripped: &[u8],
    channels: u32,
    codebooks: &CodebookTable,
) -> Result<Setup, String> {
    let mut setup = Translator {
        input: BitReader::new(stripped),
        out: BitWriter::new(),
    };
    setup.out.write(5, 8);
    for &byte in SIGNATURE {
        setup.out.write(u32::from(byte), 8);
    }

    let codebook_count = setup.copy(8)? + 1;
    for _ in 0..codebook_count {
        let id = setup.input.read(10)?;
        expand_codebook(codebooks.book(id)?, &mut setup.out)?;
    }

    setup.out.write(0, 6);
    setup.out.write(0, 16);

    let floor_count = setup.copy(6)? + 1;
    for _ in 0..floor_count {
        setup.out.write(1, 16);
        setup.floor1()?;
    }

    let residue_count = setup.copy(6)? + 1;
    for _ in 0..residue_count {
        let residue_type = setup.input.read(2)?;
        setup.out.write(residue_type, 16);
        setup.residue()?;
    }

    let mapping_count = setup.copy(6)? + 1;
    for _ in 0..mapping_count {
        setup.out.write(0, 16);
        setup.mapping(channels)?;
    }

    let mode_count = setup.copy(6)? + 1;
    let mut long_block_modes = Vec::with_capacity(mode_count as usize);
    for _ in 0..mode_count {
        long_block_modes.push(setup.copy(1)? == 1);
        setup.out.write(0, 16);
        setup.out.write(0, 16);
        setup.copy(8)?;
    }
    setup.out.write(1, 1);

    let consumed = setup.input.bits_read().div_ceil(8);
    if consumed != stripped.len() {
        return Err(format!(
            "setup packet is {} bytes but its fields end after {consumed}",
            stripped.len()
        ));
    }
    Ok(Setup {
        header: setup.out.into_bytes(),
        long_block_modes,
    })
}

struct Translator<'a> {
    input: BitReader<'a>,
    out: BitWriter,
}

impl Translator<'_> {
    fn copy(&mut self, bit_count: u32) -> Result<u32, String> {
        let value = self.input.read(bit_count)?;
        self.out.write(value, bit_count);
        Ok(value)
    }

    fn floor1(&mut self) -> Result<(), String> {
        let partitions = self.copy(5)?;
        let partition_classes = (0..partitions)
            .map(|_| self.copy(4))
            .collect::<Result<Vec<_>, _>>()?;
        let class_count = partition_classes.iter().max().map_or(0, |&class| class + 1);
        let mut class_dimensions = Vec::with_capacity(class_count as usize);
        for _ in 0..class_count {
            class_dimensions.push(self.copy(3)? + 1);
            let subclass_bits = self.copy(2)?;
            if subclass_bits != 0 {
                self.copy(8)?;
            }
            for _ in 0..1 << subclass_bits {
                self.copy(8)?;
            }
        }
        self.copy(2)?;
        let range_bits = self.copy(4)?;
        for class in partition_classes {
            for _ in 0..class_dimensions[class as usize] {
                self.copy(range_bits)?;
            }
        }
        Ok(())
    }

    fn residue(&mut self) -> Result<(), String> {
        self.copy(24)?;
        self.copy(24)?;
        self.copy(24)?;
        let classifications = self.copy(6)? + 1;
        self.copy(8)?;
        let mut cascades = Vec::with_capacity(classifications as usize);
        for _ in 0..classifications {
            let low_bits = self.copy(3)?;
            let high_bits = if self.copy(1)? == 1 { self.copy(5)? } else { 0 };
            cascades.push(high_bits << 3 | low_bits);
        }
        for cascade in cascades {
            for _ in 0..cascade.count_ones() {
                self.copy(8)?;
            }
        }
        Ok(())
    }

    fn mapping(&mut self, channels: u32) -> Result<(), String> {
        let submaps = if self.copy(1)? == 1 {
            self.copy(4)? + 1
        } else {
            1
        };
        if self.copy(1)? == 1 {
            let coupling_steps = self.copy(8)? + 1;
            let channel_bits = ilog(channels.saturating_sub(1));
            for _ in 0..coupling_steps {
                self.copy(channel_bits)?;
                self.copy(channel_bits)?;
            }
        }
        if self.copy(2)? != 0 {
            return Err("mapping reserved bits are set".to_owned());
        }
        if submaps > 1 {
            for _ in 0..channels {
                self.copy(4)?;
            }
        }
        for _ in 0..submaps {
            self.copy(8)?;
            self.copy(8)?;
            self.copy(8)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Fields = Vec<(u32, u32)>;

    fn pack(fields: &[(u32, u32)]) -> Vec<u8> {
        let mut writer = BitWriter::new();
        for &(value, count) in fields {
            writer.write(value, count);
        }
        writer.into_bytes()
    }

    fn book(entries: u32) -> Vec<u8> {
        let mut fields = vec![(1, 4), (entries, 14), (0, 1), (2, 3), (0, 1)];
        fields.extend((0..entries).map(|entry| (entry % 4, 2)));
        fields.push((0, 1));
        pack(&fields)
    }

    fn table() -> CodebookTable {
        CodebookTable {
            books: vec![book(3), book(5)],
        }
    }

    fn vorbis_prefix(packet_type: u32) -> Fields {
        let mut fields = vec![(packet_type, 8)];
        fields.extend(b"vorbis".iter().map(|&byte| (u32::from(byte), 8)));
        fields
    }

    /// Codebook ids [1, 0], one floor, one residue.
    fn stripped_front() -> Fields {
        vec![
            (1, 8),
            (1, 10),
            (0, 10),
            (0, 6),
            (1, 5),
            (0, 4),
            (1, 3),
            (1, 2),
            (1, 8),
            (0, 8),
            (2, 8),
            (1, 2),
            (7, 4),
            (10, 7),
            (90, 7),
            (0, 6),
            (2, 2),
            (0, 24),
            (256, 24),
            (15, 24),
            (1, 6),
            (0, 8),
            (0b011, 3),
            (0, 1),
            (0b001, 3),
            (1, 1),
            (1, 5),
            (1, 8),
            (0, 8),
            (1, 8),
            (1, 8),
        ]
    }

    fn full_front(out: &mut BitWriter) {
        for (value, count) in vorbis_prefix(5) {
            out.write(value, count);
        }
        out.write(1, 8);
        expand_codebook(&book(5), out).unwrap();
        expand_codebook(&book(3), out).unwrap();
        for (value, count) in [
            (0, 6),
            (0, 16),
            (0, 6),
            (1, 16),
            (1, 5),
            (0, 4),
            (1, 3),
            (1, 2),
            (1, 8),
            (0, 8),
            (2, 8),
            (1, 2),
            (7, 4),
            (10, 7),
            (90, 7),
            (0, 6),
            (2, 16),
            (0, 24),
            (256, 24),
            (15, 24),
            (1, 6),
            (0, 8),
            (0b011, 3),
            (0, 1),
            (0b001, 3),
            (1, 1),
            (1, 5),
            (1, 8),
            (0, 8),
            (1, 8),
            (1, 8),
        ] {
            out.write(value, count);
        }
    }

    fn short_and_long_modes() -> (Fields, Fields) {
        let stripped = vec![(1, 6), (0, 1), (0, 8), (1, 1), (0, 8)];
        let full = vec![
            (1, 6),
            (0, 1),
            (0, 16),
            (0, 16),
            (0, 8),
            (1, 1),
            (0, 16),
            (0, 16),
            (0, 8),
            (1, 1),
        ];
        (stripped, full)
    }

    #[test]
    fn identification_header_is_thirty_bytes_of_stream_parameters() {
        let header = identification_header(2, 44_100, 61_000, (8, 11));
        let mut expected = vec![1];
        expected.extend_from_slice(b"vorbis");
        expected.extend_from_slice(&0_u32.to_le_bytes());
        expected.push(2);
        expected.extend_from_slice(&44_100_u32.to_le_bytes());
        expected.extend_from_slice(&0_u32.to_le_bytes());
        expected.extend_from_slice(&61_000_u32.to_le_bytes());
        expected.extend_from_slice(&0_u32.to_le_bytes());
        expected.extend_from_slice(&[0xB8, 0x01]);
        assert_eq!(header, expected);
    }

    #[test]
    fn comment_header_carries_vendor_and_no_comments() {
        let mut expected = vec![3];
        expected.extend_from_slice(b"vorbis");
        expected.extend_from_slice(&3_u32.to_le_bytes());
        expected.extend_from_slice(b"abc");
        expected.extend_from_slice(&0_u32.to_le_bytes());
        expected.push(1);
        assert_eq!(comment_header("abc"), expected);
    }

    #[test]
    fn mono_setup_regains_the_fields_wwise_strips() {
        let (stripped_modes, full_modes) = short_and_long_modes();
        let mut stripped = stripped_front();
        stripped.extend([(0, 6), (0, 1), (0, 1), (0, 2), (0, 8), (0, 8), (0, 8)]);
        stripped.extend(stripped_modes);

        let mut full = BitWriter::new();
        full_front(&mut full);
        for (value, count) in [
            (0, 6),
            (0, 16),
            (0, 1),
            (0, 1),
            (0, 2),
            (0, 8),
            (0, 8),
            (0, 8),
        ] {
            full.write(value, count);
        }
        for (value, count) in full_modes {
            full.write(value, count);
        }

        let setup = rebuild_setup(&pack(&stripped), 1, &table()).unwrap();
        assert_eq!(setup.header, full.into_bytes());
        assert_eq!(setup.long_block_modes, vec![false, true]);
    }

    #[test]
    fn stereo_mapping_keeps_coupling_and_channel_mux() {
        let (stripped_modes, full_modes) = short_and_long_modes();
        let mapping = [
            (1, 1),
            (1, 4),
            (1, 1),
            (0, 8),
            (0, 1),
            (1, 1),
            (0, 2),
            (0, 4),
            (1, 4),
            (0, 8),
            (0, 8),
            (0, 8),
            (0, 8),
            (0, 8),
            (0, 8),
        ];
        let mut stripped = stripped_front();
        stripped.push((0, 6));
        stripped.extend(mapping);
        stripped.extend(stripped_modes);

        let mut full = BitWriter::new();
        full_front(&mut full);
        full.write(0, 6);
        full.write(0, 16);
        for (value, count) in mapping.into_iter().chain(full_modes) {
            full.write(value, count);
        }

        let setup = rebuild_setup(&pack(&stripped), 2, &table()).unwrap();
        assert_eq!(setup.header, full.into_bytes());
    }

    #[test]
    fn setup_with_unread_trailing_bytes_is_rejected() {
        let (stripped_modes, _) = short_and_long_modes();
        let mut stripped = stripped_front();
        stripped.extend([(0, 6), (0, 1), (0, 1), (0, 2), (0, 8), (0, 8), (0, 8)]);
        stripped.extend(stripped_modes);
        let mut bytes = pack(&stripped);
        bytes.push(0);
        assert!(rebuild_setup(&bytes, 1, &table()).is_err());
    }
}
