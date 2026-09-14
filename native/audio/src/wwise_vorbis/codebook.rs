use super::bits::{BitReader, BitWriter, ilog};
use super::executable::PeImage;

const CODEBOOK_SYNC: u32 = 0x564342;

/// Wwise stores codebooks with narrower fields than a Vorbis setup header:
/// 4-bit dimensions, 14-bit entry count, codeword lengths at a per-book width
/// and a 1-bit lookup type. Everything else is bit-identical to Vorbis.
pub(crate) fn expand_codebook(packed: &[u8], out: &mut BitWriter) -> Result<usize, String> {
    let mut input = BitReader::new(packed);
    let dimensions = input.read(4)?;
    let entries = input.read(14)?;
    out.write(CODEBOOK_SYNC, 24);
    out.write(dimensions, 16);
    out.write(entries, 24);

    let ordered = input.read(1)?;
    out.write(ordered, 1);
    if ordered == 1 {
        out.write(input.read(5)?, 5);
        let mut assigned = 0;
        while assigned < entries {
            let width = ilog(entries - assigned);
            let run = input.read(width)?;
            out.write(run, width);
            assigned += run;
        }
        if assigned != entries {
            return Err(format!(
                "ordered codebook assigns {assigned} of {entries} entries"
            ));
        }
    } else {
        let length_width = input.read(3)?;
        let sparse = input.read(1)?;
        if !(1..=5).contains(&length_width) {
            return Err(format!(
                "codeword length width {length_width} is outside 1..=5"
            ));
        }
        out.write(sparse, 1);
        for _ in 0..entries {
            let present = if sparse == 1 {
                let flag = input.read(1)?;
                out.write(flag, 1);
                flag == 1
            } else {
                true
            };
            if present {
                out.write(input.read(length_width)?, 5);
            }
        }
    }

    let lookup_type = input.read(1)?;
    out.write(lookup_type, 4);
    if lookup_type == 1 {
        if dimensions == 0 {
            return Err("lookup codebook has zero dimensions".to_owned());
        }
        out.write(input.read(32)?, 32);
        out.write(input.read(32)?, 32);
        let value_width_less_one = input.read(4)?;
        out.write(value_width_less_one, 4);
        out.write(input.read(1)?, 1);
        let value_width = value_width_less_one + 1;
        for _ in 0..lookup1_values(entries, dimensions) {
            out.write(input.read(value_width)?, value_width);
        }
    }
    Ok(input.bits_read())
}

/// Wwise's table spaces books at whole bits / 8 + 1 bytes, so a book ending
/// exactly on a byte boundary still owns the following byte.
pub(crate) fn packed_codebook_len(packed: &[u8]) -> Result<usize, String> {
    Ok(expand_codebook(packed, &mut BitWriter::new())? / 8 + 1)
}

pub struct CodebookTable {
    pub(super) books: Vec<Vec<u8>>,
}

/// Wwise's table holds 598 books; far shorter validated runs are coincidence.
const MIN_TABLE_BOOKS: usize = 256;
const MAX_PACKED_BOOK_BYTES: u64 = 0x10000;

impl CodebookTable {
    /// Locates the codebook table a Wwise runtime links into a game executable:
    /// a pointer array whose every gap is exactly one packed codebook long.
    pub fn from_executable(bytes: &[u8]) -> Result<Self, String> {
        let image = PeImage::parse(bytes)?;
        let books = image
            .climbing_pointer_runs(MIN_TABLE_BOOKS + 1, MAX_PACKED_BOOK_BYTES)
            .iter()
            .map(|run| longest_codebook_stretch(image.bytes(), run))
            .max_by_key(Vec::len)
            .unwrap_or_default();
        if books.len() < MIN_TABLE_BOOKS {
            return Err("no Wwise Vorbis codebook table found in executable".to_owned());
        }
        Ok(Self {
            books: books.into_iter().map(<[u8]>::to_vec).collect(),
        })
    }

    pub fn book_count(&self) -> usize {
        self.books.len()
    }

    pub(crate) fn book(&self, id: u32) -> Result<&[u8], String> {
        self.books
            .get(id as usize)
            .map(Vec::as_slice)
            .ok_or_else(|| {
                format!(
                    "codebook {id} is past the end of a {}-book table",
                    self.books.len()
                )
            })
    }
}

fn longest_codebook_stretch<'a>(bytes: &'a [u8], offsets: &[usize]) -> Vec<&'a [u8]> {
    let mut best: Vec<&[u8]> = Vec::new();
    let mut current: Vec<&[u8]> = Vec::new();
    for pair in offsets.windows(2) {
        let book = bytes
            .get(pair[0]..pair[1])
            .filter(|book| packed_codebook_len(book) == Ok(book.len()));
        match book {
            Some(book) => current.push(book),
            None => {
                if current.len() > best.len() {
                    best = std::mem::take(&mut current);
                }
                current.clear();
            }
        }
    }
    if current.len() > best.len() {
        best = current;
    }
    best
}

fn lookup1_values(entries: u32, dimensions: u32) -> u32 {
    let fits = |root: u32| {
        root.checked_pow(dimensions)
            .is_some_and(|power| power <= entries)
    };
    let mut root = f64::from(entries).powf(1.0 / f64::from(dimensions)) as u32;
    while fits(root + 1) {
        root += 1;
    }
    while root > 0 && !fits(root) {
        root -= 1;
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wwise_vorbis::executable::tests::synthetic_image;

    const SYNC: u32 = 0x564342;

    fn bits(fields: &[(u32, u32)]) -> Vec<u8> {
        let mut writer = BitWriter::new();
        for &(value, count) in fields {
            writer.write(value, count);
        }
        writer.into_bytes()
    }

    fn expand(packed: &[u8]) -> Vec<u8> {
        let mut out = BitWriter::new();
        expand_codebook(packed, &mut out).unwrap();
        out.into_bytes()
    }

    #[test]
    fn sparse_lengths_widen_to_five_bits() {
        let packed = bits(&[
            (1, 4),
            (3, 14),
            (0, 1),
            (3, 3),
            (1, 1),
            (1, 1),
            (4, 3),
            (0, 1),
            (1, 1),
            (2, 3),
            (0, 1),
        ]);
        let full = bits(&[
            (SYNC, 24),
            (1, 16),
            (3, 24),
            (0, 1),
            (1, 1),
            (1, 1),
            (4, 5),
            (0, 1),
            (1, 1),
            (2, 5),
            (0, 4),
        ]);
        assert_eq!(expand(&packed), full);
    }

    #[test]
    fn ordered_runs_copy_through() {
        let packed = bits(&[(2, 4), (5, 14), (1, 1), (3, 5), (2, 3), (3, 2), (0, 1)]);
        let full = bits(&[
            (SYNC, 24),
            (2, 16),
            (5, 24),
            (1, 1),
            (3, 5),
            (2, 3),
            (3, 2),
            (0, 4),
        ]);
        assert_eq!(expand(&packed), full);
    }

    #[test]
    fn lookup_table_values_copy_through() {
        let mut packed = vec![(2, 4), (9, 14), (0, 1), (2, 3), (0, 1)];
        let mut full = vec![(SYNC, 24), (2, 16), (9, 24), (0, 1), (0, 1)];
        for length in 0..9 {
            packed.push((length % 4, 2));
            full.push((length % 4, 5));
        }
        let lookup = [
            (1, 1),
            (0x1122_3344, 32),
            (0x5566_7788, 32),
            (2, 4),
            (1, 1),
            (5, 3),
            (6, 3),
            (7, 3),
        ];
        packed.extend_from_slice(&lookup);
        full.push((1, 4));
        full.extend_from_slice(&lookup[1..]);
        assert_eq!(expand(&bits(&packed)), bits(&full));
    }

    #[test]
    fn packed_length_counts_a_byte_past_the_last_whole_byte() {
        let mut exactly_32_bits = vec![(1, 4), (8, 14), (0, 1), (1, 3), (0, 1)];
        exactly_32_bits.extend(std::iter::repeat_n((1, 1), 8));
        exactly_32_bits.push((0, 1));
        let mut packed = bits(&exactly_32_bits);
        assert_eq!(packed.len(), 4);
        packed.push(0);
        assert_eq!(packed_codebook_len(&packed).unwrap(), 5);

        let thirty_three_bits = bits(&[
            (1, 4),
            (3, 14),
            (0, 1),
            (3, 3),
            (1, 1),
            (1, 1),
            (4, 3),
            (0, 1),
            (1, 1),
            (2, 3),
            (0, 1),
        ]);
        assert_eq!(packed_codebook_len(&thirty_three_bits).unwrap(), 5);
    }

    #[test]
    fn zero_width_codeword_lengths_are_rejected() {
        let packed = bits(&[(1, 4), (3, 14), (0, 1), (0, 3), (0, 1), (0, 8)]);
        assert!(expand_codebook(&packed, &mut BitWriter::new()).is_err());
    }

    fn sample_books(count: u32) -> Vec<Vec<u8>> {
        (0..count)
            .map(|index| {
                let entries = 1 + index % 40;
                let mut fields = vec![(1, 4), (entries, 14), (0, 1), (3, 3), (0, 1)];
                fields.extend((0..entries).map(|entry| ((index + entry) % 8, 3)));
                fields.push((0, 1));
                let mut book = bits(&fields);
                book.resize(packed_codebook_len(&book).unwrap(), 0);
                book
            })
            .collect()
    }

    fn table_offsets(books: &[Vec<u8>]) -> Vec<u64> {
        let mut offset = 0;
        let mut offsets = vec![0];
        for book in books {
            offset += book.len() as u64;
            offsets.push(offset);
        }
        offsets
    }

    #[test]
    fn table_is_found_past_pointer_runs_that_are_not_codebooks() {
        let books = sample_books(300);
        let decoy = (0..300).map(|index| index * 3).collect();
        let image = synthetic_image(&books, &[decoy, table_offsets(&books)]);
        let table = CodebookTable::from_executable(&image).unwrap();
        assert_eq!(table.book_count(), 300);
        for (id, book) in books.iter().enumerate() {
            assert_eq!(table.book(id as u32).unwrap(), book.as_slice(), "book {id}");
        }
        assert!(table.book(300).is_err());
    }

    #[test]
    fn executable_without_a_table_is_reported() {
        let image = synthetic_image(
            &[vec![0x55; 4096]],
            &[(0..400).map(|index| index * 8).collect()],
        );
        let error = CodebookTable::from_executable(&image).err().unwrap();
        assert!(error.contains("codebook"), "{error}");
    }

    #[test]
    fn lookup1_values_is_the_largest_whole_root() {
        for (entries, dimensions, expected) in [
            (9, 2, 3),
            (8, 2, 2),
            (64, 3, 4),
            (63, 3, 3),
            (256, 1, 256),
            (10, 4, 1),
        ] {
            assert_eq!(
                lookup1_values(entries, dimensions),
                expected,
                "{entries} entries, {dimensions} dims"
            );
        }
    }
}
