const CONTINUED: u8 = 0x01;
const BEGINNING_OF_STREAM: u8 = 0x02;
const END_OF_STREAM: u8 = 0x04;
const MAX_SEGMENTS: usize = 255;
const TARGET_PAGE_BODY: usize = 4096;
/// Granule of a page on which no packet ends.
const NO_PACKET_ENDS: u64 = u64::MAX;

pub(crate) struct OggWriter {
    bytes: Vec<u8>,
    serial: u32,
    sequence: u32,
    lacing: Vec<u8>,
    body: Vec<u8>,
    page_granule: Option<u64>,
    page_continued: bool,
    last_granule: u64,
}

impl OggWriter {
    pub(crate) fn new(serial: u32) -> Self {
        Self {
            bytes: Vec::new(),
            serial,
            sequence: 0,
            lacing: Vec::with_capacity(MAX_SEGMENTS),
            body: Vec::new(),
            page_granule: None,
            page_continued: false,
            last_granule: 0,
        }
    }

    /// A full page is only written once another packet arrives, so the last
    /// packet always shares the end-of-stream page. Decoders trim trailing
    /// padding from the granule on the page holding the final packet, and do
    /// nothing with an empty end-of-stream page.
    pub(crate) fn packet(&mut self, data: &[u8], granule: u64) {
        if self.body.len() >= TARGET_PAGE_BODY {
            self.flush();
        }
        let mut remaining = data;
        let mut started = false;
        loop {
            if self.lacing.len() == MAX_SEGMENTS {
                self.write_page(0);
                self.page_continued = started;
            }
            let segment = remaining.len().min(255);
            self.lacing.push(segment as u8);
            self.body.extend_from_slice(&remaining[..segment]);
            remaining = &remaining[segment..];
            started = true;
            if segment < 255 {
                break;
            }
        }
        self.page_granule = Some(granule);
        self.last_granule = granule;
    }

    pub(crate) fn flush(&mut self) {
        if !self.lacing.is_empty() {
            self.write_page(0);
            self.page_continued = false;
        }
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        if self.page_granule.is_none() && self.lacing.is_empty() {
            self.page_granule = Some(self.last_granule);
        }
        self.write_page(END_OF_STREAM);
        self.bytes
    }

    fn write_page(&mut self, extra_flags: u8) {
        let mut flags = extra_flags;
        if self.page_continued {
            flags |= CONTINUED;
        }
        if self.sequence == 0 {
            flags |= BEGINNING_OF_STREAM;
        }
        let start = self.bytes.len();
        self.bytes.extend_from_slice(b"OggS");
        self.bytes.push(0);
        self.bytes.push(flags);
        let granule = self.page_granule.take().unwrap_or(NO_PACKET_ENDS);
        self.bytes.extend_from_slice(&granule.to_le_bytes());
        self.bytes.extend_from_slice(&self.serial.to_le_bytes());
        self.bytes.extend_from_slice(&self.sequence.to_le_bytes());
        self.bytes.extend_from_slice(&[0; 4]);
        self.bytes.push(self.lacing.len() as u8);
        self.bytes.append(&mut self.lacing);
        self.bytes.append(&mut self.body);
        let crc = ogg_crc(&self.bytes[start..]);
        self.bytes[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
        self.sequence += 1;
    }
}

pub(crate) fn ogg_crc(bytes: &[u8]) -> u32 {
    const POLYNOMIAL: u32 = 0x04C1_1DB7;
    let mut crc = 0_u32;
    for &byte in bytes {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                crc << 1 ^ POLYNOMIAL
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct Page {
        pub(crate) flags: u8,
        pub(crate) granule: u64,
        pub(crate) sequence: u32,
        pub(crate) lacing: Vec<u8>,
        pub(crate) body: Vec<u8>,
    }

    pub(crate) fn read_pages(mut bytes: &[u8]) -> Vec<Page> {
        let mut pages = Vec::new();
        while !bytes.is_empty() {
            assert_eq!(&bytes[..4], b"OggS");
            let segments = bytes[26] as usize;
            let lacing = bytes[27..27 + segments].to_vec();
            let length = 27 + segments + lacing.iter().map(|&value| value as usize).sum::<usize>();
            let mut unsigned = bytes[..length].to_vec();
            unsigned[22..26].fill(0);
            assert_eq!(ogg_crc(&unsigned).to_le_bytes(), bytes[22..26], "page CRC");
            pages.push(Page {
                flags: bytes[5],
                granule: u64::from_le_bytes(bytes[6..14].try_into().unwrap()),
                sequence: u32::from_le_bytes(bytes[18..22].try_into().unwrap()),
                body: bytes[27 + segments..length].to_vec(),
                lacing,
            });
            bytes = &bytes[length..];
        }
        pages
    }

    #[test]
    fn crc_uses_the_ogg_polynomial_without_reflection() {
        assert_eq!(ogg_crc(b"123456789"), 0x89A1_897F);
    }

    #[test]
    fn first_page_begins_the_stream_with_lacing_and_granule() {
        let mut writer = OggWriter::new(0x1234);
        writer.packet(&[7; 300], 5);
        writer.flush();
        let pages = read_pages(&writer.finish());
        assert_eq!(pages[0].flags, 0x02);
        assert_eq!(pages[0].granule, 5);
        assert_eq!(pages[0].sequence, 0);
        assert_eq!(pages[0].lacing, vec![255, 45]);
        assert_eq!(pages[0].body, vec![7; 300]);
    }

    #[test]
    fn packet_longer_than_a_page_continues_onto_the_next() {
        let mut writer = OggWriter::new(1);
        writer.packet(&vec![1; 255 * 255 + 10], 9);
        let pages = read_pages(&writer.finish());
        assert_eq!(pages.len(), 2);
        assert_eq!(
            (pages[0].flags, pages[0].granule, pages[0].lacing.len()),
            (0x02, u64::MAX, 255)
        );
        assert_eq!(
            (pages[1].flags, pages[1].granule, pages[1].sequence),
            (0x05, 9, 1)
        );
        assert_eq!(pages[1].lacing, vec![10]);
    }

    #[test]
    fn final_packet_filling_a_page_still_ends_the_stream_on_that_page() {
        let mut writer = OggWriter::new(1);
        writer.packet(&[0; 100], 3);
        writer.packet(&[0; 5000], 7);
        let pages = read_pages(&writer.finish());
        assert_eq!(pages.len(), 1);
        assert_eq!((pages[0].flags, pages[0].granule), (0x06, 7));
    }

    #[test]
    fn packet_ending_on_a_full_page_does_not_mark_the_next_page_continued() {
        let mut writer = OggWriter::new(1);
        for _ in 0..255 {
            writer.packet(&[1], 3);
        }
        writer.packet(&[2], 4);
        let pages = read_pages(&writer.finish());
        assert_eq!((pages[0].flags, pages[0].granule), (0x02, 3));
        assert_eq!((pages[1].flags, pages[1].granule), (0x04, 4));
    }
}
