pub(crate) struct BitReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub(crate) fn read(&mut self, bit_count: u32) -> Result<u32, String> {
        if self.position + bit_count as usize > self.bytes.len() * 8 {
            return Err(format!(
                "bitstream ended: wanted {bit_count} bits at bit {} of {}",
                self.position,
                self.bytes.len() * 8
            ));
        }
        let mut value = 0_u32;
        for index in 0..bit_count {
            let bit = (self.bytes[self.position / 8] >> (self.position % 8)) & 1;
            value |= u32::from(bit) << index;
            self.position += 1;
        }
        Ok(value)
    }

    pub(crate) fn bits_read(&self) -> usize {
        self.position
    }
}

pub(crate) struct BitWriter {
    bytes: Vec<u8>,
    position: usize,
}

impl BitWriter {
    pub(crate) fn new() -> Self {
        Self {
            bytes: Vec::new(),
            position: 0,
        }
    }

    pub(crate) fn write(&mut self, value: u32, bit_count: u32) {
        for index in 0..bit_count {
            if self.position.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let bit = ((value >> index) & 1) as u8;
            *self.bytes.last_mut().expect("byte pushed above") |= bit << (self.position % 8);
            self.position += 1;
        }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

pub(crate) fn ilog(value: u32) -> u32 {
    u32::BITS - value.leading_zeros()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ilog_counts_significant_bits() {
        for (value, expected) in [
            (0, 0),
            (1, 1),
            (2, 2),
            (3, 2),
            (4, 3),
            (7, 3),
            (8, 4),
            (597, 10),
        ] {
            assert_eq!(ilog(value), expected, "ilog({value})");
        }
    }

    #[test]
    fn writer_packs_least_significant_bit_first() {
        let mut writer = BitWriter::new();
        writer.write(0b101, 3);
        writer.write(0b11111, 5);
        writer.write(1, 1);
        assert_eq!(writer.into_bytes(), vec![0xFD, 0x01]);
    }

    #[test]
    fn reader_returns_what_writer_wrote() {
        let fields = [
            (1, 1),
            (0x3F, 6),
            (0x564342, 24),
            (0xDEADBEEF, 32),
            (0, 7),
            (0x1234, 16),
        ];
        let mut writer = BitWriter::new();
        for (value, bits) in fields {
            writer.write(value, bits);
        }
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes);
        for (value, bits) in fields {
            assert_eq!(reader.read(bits).unwrap(), value, "{bits}-bit field");
        }
        assert_eq!(reader.bits_read(), 86);
    }

    #[test]
    fn reader_refuses_to_run_past_the_end() {
        let mut reader = BitReader::new(&[0xFF]);
        assert_eq!(reader.read(7).unwrap(), 0x7F);
        assert!(reader.read(2).is_err());
    }
}
