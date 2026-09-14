pub(crate) struct PeImage<'a> {
    bytes: &'a [u8],
    image_base: u64,
    sections: Vec<Section>,
}

struct Section {
    virtual_address: u64,
    raw_offset: usize,
    raw_size: usize,
}

const PE32_PLUS_MAGIC: u16 = 0x20B;

impl<'a> PeImage<'a> {
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        let not_pe = || "not a 64-bit PE executable".to_owned();
        if bytes.get(..2) != Some(b"MZ") {
            return Err(not_pe());
        }
        let pe = read_u32(bytes, 0x3C).ok_or_else(not_pe)? as usize;
        if bytes.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(not_pe());
        }
        let section_count = read_u16(bytes, pe + 6).ok_or_else(not_pe)? as usize;
        let optional_size = read_u16(bytes, pe + 20).ok_or_else(not_pe)? as usize;
        let optional = pe + 24;
        if read_u16(bytes, optional) != Some(PE32_PLUS_MAGIC) {
            return Err(not_pe());
        }
        let image_base = read_u64(bytes, optional + 24).ok_or_else(not_pe)?;
        let sections = (0..section_count)
            .map(|index| {
                let header = optional + optional_size + 40 * index;
                Some(Section {
                    virtual_address: u64::from(read_u32(bytes, header + 12)?),
                    raw_size: read_u32(bytes, header + 16)? as usize,
                    raw_offset: read_u32(bytes, header + 20)? as usize,
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(not_pe)?;
        Ok(Self {
            bytes,
            image_base,
            sections,
        })
    }

    pub(crate) fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// File offsets addressed by runs of 64-bit pointers that climb in steps
    /// of at most `max_step` bytes, each run at least `min_len` long.
    pub(crate) fn climbing_pointer_runs(&self, min_len: usize, max_step: u64) -> Vec<Vec<usize>> {
        let mut runs = Vec::new();
        for section in &self.sections {
            let raw_end = (section.raw_offset + section.raw_size).min(self.bytes.len());
            let raw = self
                .bytes
                .get(section.raw_offset..raw_end)
                .unwrap_or_default();
            let mut run: Vec<usize> = Vec::new();
            let mut previous = 0_u64;
            for word in raw.chunks_exact(8) {
                let address = u64::from_le_bytes(word.try_into().expect("chunk is 8 bytes"));
                let offset = self.file_offset(address);
                let climbs = offset.is_some()
                    && !run.is_empty()
                    && address > previous
                    && address - previous <= max_step;
                if !climbs && run.len() >= min_len {
                    runs.push(std::mem::take(&mut run));
                } else if !climbs {
                    run.clear();
                }
                if let Some(offset) = offset {
                    run.push(offset);
                    previous = address;
                }
            }
            if run.len() >= min_len {
                runs.push(run);
            }
        }
        runs
    }

    /// One past a section's end still resolves: tables often close with an
    /// end pointer that lands exactly there.
    fn file_offset(&self, address: u64) -> Option<usize> {
        let rva = address.checked_sub(self.image_base)?;
        self.sections.iter().find_map(|section| {
            let within = rva.checked_sub(section.virtual_address)?;
            (within <= section.raw_size as u64)
                .then(|| section.raw_offset + within as usize)
                .filter(|&offset| offset <= self.bytes.len())
        })
    }
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const IMAGE_BASE: u64 = 0x1_4000_0000;
    const RDATA_RVA: u64 = 0x1000;
    const RDATA_RAW: usize = 0x400;
    const SECTION_TABLE: usize = 0x148;

    fn put(image: &mut [u8], at: usize, bytes: &[u8]) {
        image[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// A PE32+ image whose .rdata holds `blobs` back to back and whose .data
    /// holds `pointer_tables`, each a list of .rdata offsets, zero-terminated.
    pub(crate) fn synthetic_image(blobs: &[Vec<u8>], pointer_tables: &[Vec<u64>]) -> Vec<u8> {
        let rdata = blobs.concat();
        let mut data = Vec::new();
        for table in pointer_tables {
            for offset in table {
                data.extend_from_slice(&(IMAGE_BASE + RDATA_RVA + offset).to_le_bytes());
            }
            data.extend_from_slice(&0_u64.to_le_bytes());
        }
        let data_rva = RDATA_RVA + (rdata.len() as u64).next_multiple_of(0x1000);
        let data_raw = RDATA_RAW + rdata.len().next_multiple_of(0x200);

        let mut image = vec![0_u8; data_raw + data.len()];
        put(&mut image, 0, b"MZ");
        put(&mut image, 0x3C, &0x40_u32.to_le_bytes());
        put(&mut image, 0x40, b"PE\0\0");
        put(&mut image, 0x44, &0x8664_u16.to_le_bytes());
        put(&mut image, 0x46, &2_u16.to_le_bytes());
        put(&mut image, 0x54, &0xF0_u16.to_le_bytes());
        put(&mut image, 0x58, &0x20B_u16.to_le_bytes());
        put(&mut image, 0x70, &IMAGE_BASE.to_le_bytes());
        for (index, (name, rva, raw, len)) in [
            (*b".rdata\0\0", RDATA_RVA, RDATA_RAW, rdata.len()),
            (*b".data\0\0\0", data_rva, data_raw, data.len()),
        ]
        .into_iter()
        .enumerate()
        {
            let header = SECTION_TABLE + 40 * index;
            put(&mut image, header, &name);
            put(&mut image, header + 8, &(len as u32).to_le_bytes());
            put(&mut image, header + 12, &(rva as u32).to_le_bytes());
            put(&mut image, header + 16, &(len as u32).to_le_bytes());
            put(&mut image, header + 20, &(raw as u32).to_le_bytes());
        }
        put(&mut image, RDATA_RAW, &rdata);
        put(&mut image, data_raw, &data);
        image
    }

    #[test]
    fn pointer_runs_resolve_to_file_offsets() {
        let blob = vec![0xAA; 64];
        let climbing: Vec<u64> = (0..6).map(|index| index * 8).collect();
        let image = synthetic_image(&[blob], &[climbing, vec![0, 40]]);
        let pe = PeImage::parse(&image).unwrap();
        let runs = pe.climbing_pointer_runs(4, 16);
        let expected: Vec<usize> = (0..6).map(|index| RDATA_RAW + index * 8).collect();
        assert_eq!(runs, vec![expected]);
    }

    #[test]
    fn pointer_runs_break_on_large_steps() {
        let image = synthetic_image(&[vec![0; 512]], &[vec![0, 8, 16, 300, 308, 316]]);
        let pe = PeImage::parse(&image).unwrap();
        assert_eq!(pe.climbing_pointer_runs(3, 16).len(), 2);
        assert!(pe.climbing_pointer_runs(4, 16).is_empty());
    }

    #[test]
    fn non_pe_input_is_rejected() {
        assert!(PeImage::parse(b"RIFF....WAVE").is_err());
    }
}
