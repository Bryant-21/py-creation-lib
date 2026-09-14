use audio_native::wwise_vorbis::{CodebookTable, convert_wem_file, wem_to_ogg};
use std::path::PathBuf;

fn starfield_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("STARFIELD_DIR")?);
    dir.join("Starfield.exe").is_file().then_some(dir)
}

struct Page {
    flags: u8,
    granule: u64,
    packets: Vec<Vec<u8>>,
}

/// Splits a stream into pages, checking each CRC. Only packets that end on a
/// page are collected; none of these streams continue packets across pages.
fn read_pages(mut bytes: &[u8]) -> Vec<Page> {
    let mut pages = Vec::new();
    while !bytes.is_empty() {
        assert_eq!(&bytes[..4], b"OggS");
        let segments = bytes[26] as usize;
        let lacing = &bytes[27..27 + segments];
        let length = 27 + segments + lacing.iter().map(|&value| value as usize).sum::<usize>();
        let mut unsigned = bytes[..length].to_vec();
        unsigned[22..26].fill(0);
        assert_eq!(crc(&unsigned).to_le_bytes(), bytes[22..26], "page CRC");

        let mut packets = Vec::new();
        let mut body = &bytes[27 + segments..length];
        let mut packet = Vec::new();
        for &value in lacing {
            packet.extend_from_slice(&body[..value as usize]);
            body = &body[value as usize..];
            if value < 255 {
                packets.push(std::mem::take(&mut packet));
            }
        }
        pages.push(Page {
            flags: bytes[5],
            granule: u64::from_le_bytes(bytes[6..14].try_into().unwrap()),
            packets,
        });
        bytes = &bytes[length..];
    }
    pages
}

fn crc(bytes: &[u8]) -> u32 {
    let mut crc = 0_u32;
    for &byte in bytes {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                crc << 1 ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[test]
fn starfield_executable_holds_the_wwise_codebook_table() {
    let Some(dir) = starfield_dir() else {
        eprintln!("STARFIELD_DIR not set; skipping");
        return;
    };
    let image = std::fs::read(dir.join("Starfield.exe")).unwrap();
    let table = CodebookTable::from_executable(&image).unwrap();
    assert_eq!(table.book_count(), 598);
}

#[test]
fn wem_file_converts_to_the_same_stream_on_disk() {
    let Some(dir) = starfield_dir() else {
        eprintln!("STARFIELD_DIR not set; skipping");
        return;
    };
    let executable = dir.join("Starfield.exe");
    let wem = bsarchive_native::python::extract_one_impl(
        &dir.join("Data").join("Starfield - Voices02.ba2"),
        "sound/voice/starfield.esm/robotmodelavasco/00c0c1b2.wem",
    )
    .unwrap();
    let work = std::env::temp_dir().join(format!("wwise_vorbis_test_{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let source = work.join("00c0c1b2.wem");
    std::fs::write(&source, &wem).unwrap();

    convert_wem_file(&source, &work.join("out").join("00c0c1b2.ogg"), &executable).unwrap();

    let table = CodebookTable::from_executable(&std::fs::read(&executable).unwrap()).unwrap();
    let written = std::fs::read(work.join("out").join("00c0c1b2.ogg")).unwrap();
    std::fs::remove_dir_all(&work).unwrap();
    assert_eq!(written, wem_to_ogg(&wem, &table).unwrap());
}

#[test]
fn executable_without_codebooks_names_the_file_it_searched() {
    let work = std::env::temp_dir().join(format!("wwise_vorbis_missing_{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let not_an_executable = work.join("NotAGame.exe");
    std::fs::write(&not_an_executable, b"MZ nope").unwrap();

    let error = convert_wem_file(&work.join("a.wem"), &work.join("a.ogg"), &not_an_executable)
        .err()
        .unwrap();
    std::fs::remove_dir_all(&work).unwrap();
    assert!(error.contains("NotAGame.exe"), "{error}");
}

#[test]
fn starfield_voice_line_becomes_a_well_formed_ogg_vorbis_stream() {
    let Some(dir) = starfield_dir() else {
        eprintln!("STARFIELD_DIR not set; skipping");
        return;
    };
    let table =
        CodebookTable::from_executable(&std::fs::read(dir.join("Starfield.exe")).unwrap()).unwrap();
    let wem = bsarchive_native::python::extract_one_impl(
        &dir.join("Data").join("Starfield - Voices02.ba2"),
        "sound/voice/starfield.esm/robotmodelavasco/00c0c1b2.wem",
    )
    .unwrap();

    let pages = read_pages(&wem_to_ogg(&wem, &table).unwrap());

    assert_eq!(pages[0].flags, 0x02);
    assert_eq!(pages[0].packets.len(), 1);
    assert_eq!(pages[0].packets[0].len(), 30);
    assert_eq!(&pages[0].packets[0][..7], b"\x01vorbis");
    assert_eq!(pages[1].packets.len(), 2);
    assert_eq!(&pages[1].packets[0][..7], b"\x03vorbis");
    assert_eq!(&pages[1].packets[1][..7], b"\x05vorbis");
    assert_eq!(
        pages[2].packets.len(),
        2,
        "first audio page holds only the first two packets"
    );
    let last = pages.last().unwrap();
    assert_eq!(last.flags & 0x04, 0x04);
    assert!(!last.packets.is_empty());
    assert_eq!(last.granule, 630_781);
}
