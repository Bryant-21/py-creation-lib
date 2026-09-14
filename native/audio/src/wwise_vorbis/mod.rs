mod bits;
mod codebook;
mod executable;
mod headers;
mod ogg;
mod packets;

pub use codebook::CodebookTable;

use bits::ilog;
use headers::{comment_header, identification_header, rebuild_setup};
use ogg::OggWriter;
use packets::{LongWindow, granule_positions, packet_mode, rebuild_audio_packet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

const WWISE_VORBIS: u16 = 0xFFFF;
/// fmt chunk that carries the vorb header inline after 0x18 bytes.
const INLINE_VORB_FMT_SIZE: usize = 0x42;
const VORB: usize = 0x18;
/// Mod-signal values of streams whose audio packets keep standard Vorbis
/// packet headers.
const UNMODIFIED_PACKET_SIGNALS: [u32; 4] = [0x4A, 0x4B, 0x69, 0x70];
const VENDOR: &str = "creation_lib wwise_vorbis";

struct WemHeader<'a> {
    channels: u16,
    sample_rate: u32,
    average_bytes_per_second: u32,
    sample_count: u32,
    setup_offset: usize,
    audio_offset: usize,
    serial: u32,
    blocksize_exponents: (u8, u8),
    data: &'a [u8],
}

impl<'a> WemHeader<'a> {
    fn parse(wem: &'a [u8]) -> Result<Self, String> {
        if wem.get(..4) != Some(b"RIFF") || wem.get(8..12) != Some(b"WAVE") {
            return Err("not a RIFF WAVE file".to_owned());
        }
        let (mut fmt, mut data) = (None, None);
        let mut chunk = 12;
        while let Some(id) = wem.get(chunk..chunk + 4) {
            let size = le_u32(wem, chunk + 4)? as usize;
            let body = wem.get(chunk + 8..chunk + 8 + size).ok_or_else(|| {
                format!(
                    "{} chunk runs past the end of the file",
                    String::from_utf8_lossy(id)
                )
            })?;
            match id {
                b"fmt " => fmt = Some(body),
                b"data" => data = Some(body),
                _ => {}
            }
            chunk += 8 + size + size % 2;
        }
        let fmt = fmt.ok_or("missing fmt chunk")?;
        let data = data.ok_or("missing data chunk")?;

        let format_tag = le_u16(fmt, 0)?;
        if format_tag != WWISE_VORBIS {
            return Err(format!("codec 0x{format_tag:04X} is not Wwise Vorbis"));
        }
        if fmt.len() != INLINE_VORB_FMT_SIZE {
            return Err(format!(
                "unsupported Wwise Vorbis layout: {}-byte fmt chunk",
                fmt.len()
            ));
        }
        let channels = le_u16(fmt, 2)?;
        if channels == 0 {
            return Err("stream has no channels".to_owned());
        }
        if UNMODIFIED_PACKET_SIGNALS.contains(&le_u32(fmt, VORB + 4)?) {
            return Err(
                "Wwise Vorbis with standard audio packet headers is not supported".to_owned(),
            );
        }
        Ok(Self {
            channels,
            sample_rate: le_u32(fmt, 4)?,
            average_bytes_per_second: le_u32(fmt, 8)?,
            sample_count: le_u32(fmt, VORB)?,
            setup_offset: le_u32(fmt, VORB + 0x10)? as usize,
            audio_offset: le_u32(fmt, VORB + 0x14)? as usize,
            serial: le_u32(fmt, VORB + 0x24)?,
            blocksize_exponents: (fmt[VORB + 0x28], fmt[VORB + 0x29]),
            data,
        })
    }
}

static CODEBOOKS: Mutex<Option<(PathBuf, Arc<CodebookTable>)>> = Mutex::new(None);

pub fn convert_wem_file(
    source: &Path,
    output: &Path,
    codebook_executable: &Path,
) -> Result<(), String> {
    let codebooks = codebooks_in(codebook_executable)?;
    let at = |path: &Path, error: String| format!("{}: {error}", path.display());
    let wem = fs::read(source).map_err(|error| at(source, error.to_string()))?;
    let ogg = wem_to_ogg(&wem, &codebooks).map_err(|error| at(source, error))?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| at(parent, error.to_string()))?;
    }
    fs::write(output, ogg).map_err(|error| at(output, error.to_string()))
}

/// Scanning a game executable for its table takes about a second, so the
/// last one found is kept for the life of the process.
fn codebooks_in(executable: &Path) -> Result<Arc<CodebookTable>, String> {
    let mut cached = CODEBOOKS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((path, table)) = cached.as_ref()
        && path == executable
    {
        return Ok(Arc::clone(table));
    }
    let located = |error: String| format!("{}: {error}", executable.display());
    let image = fs::read(executable).map_err(|error| located(error.to_string()))?;
    let table = Arc::new(CodebookTable::from_executable(&image).map_err(located)?);
    *cached = Some((executable.to_path_buf(), Arc::clone(&table)));
    Ok(table)
}

pub fn wem_to_ogg(wem: &[u8], codebooks: &CodebookTable) -> Result<Vec<u8>, String> {
    let header = WemHeader::parse(wem)?;
    let channels = u8::try_from(header.channels)
        .map_err(|_| format!("{} channels is too many", header.channels))?;
    let (setup_packet, _) = sized_packet(header.data, header.setup_offset)?;
    let setup = rebuild_setup(setup_packet, u32::from(header.channels), codebooks)?;

    let mut audio = Vec::new();
    let mut offset = header.audio_offset;
    while offset < header.data.len() {
        let (packet, next) = sized_packet(header.data, offset)?;
        audio.push(packet);
        offset = next;
    }

    let mode_bits = ilog(setup.long_block_modes.len() as u32 - 1);
    let long_blocks = audio
        .iter()
        .map(|packet| {
            let mode = packet_mode(packet, mode_bits)?;
            setup.long_block_modes.get(mode).copied().ok_or_else(|| {
                format!(
                    "audio packet uses mode {mode} of {}",
                    setup.long_block_modes.len()
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (short_exponent, long_exponent) = header.blocksize_exponents;
    let block_sizes: Vec<u32> = long_blocks
        .iter()
        .map(|&long| 1 << if long { long_exponent } else { short_exponent })
        .collect();
    let granules = granule_positions(&block_sizes, u64::from(header.sample_count));

    let mut ogg = OggWriter::new(header.serial);
    ogg.packet(
        &identification_header(
            channels,
            header.sample_rate,
            header.average_bytes_per_second * 8,
            header.blocksize_exponents,
        ),
        0,
    );
    ogg.flush();
    ogg.packet(&comment_header(VENDOR), 0);
    ogg.packet(&setup.header, 0);
    ogg.flush();
    for (index, packet) in audio.iter().enumerate() {
        let window = long_blocks[index].then(|| LongWindow {
            previous_long: index > 0 && long_blocks[index - 1],
            next_long: long_blocks.get(index + 1) == Some(&true),
        });
        ogg.packet(
            &rebuild_audio_packet(packet, mode_bits, window)?,
            granules[index],
        );
        // ffmpeg reads the stream's start offset from the first audio page only
        // when that page is not also the last; otherwise it trims 128 samples
        // too many from short lines. Closing the page here keeps them apart.
        if index == 1 && audio.len() > 2 {
            ogg.flush();
        }
    }
    Ok(ogg.finish())
}

/// Wwise prefixes each packet in the data chunk with its 16-bit size.
fn sized_packet(data: &[u8], offset: usize) -> Result<(&[u8], usize), String> {
    let size = le_u16(data, offset)? as usize;
    let end = offset + 2 + size;
    let packet = data
        .get(offset + 2..end)
        .ok_or_else(|| format!("packet at data offset {offset} runs past the data chunk"))?;
    Ok((packet, end))
}

fn le_u16(bytes: &[u8], at: usize) -> Result<u16, String> {
    bytes
        .get(at..at + 2)
        .map(|field| u16::from_le_bytes([field[0], field[1]]))
        .ok_or_else(|| format!("header ends before offset {at}"))
}

fn le_u32(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|field| u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
        .ok_or_else(|| format!("header ends before offset {at}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wem(format_tag: u16, mod_signal: u32) -> Vec<u8> {
        let mut fmt = vec![0_u8; 0x42];
        fmt[0..2].copy_from_slice(&format_tag.to_le_bytes());
        fmt[2..4].copy_from_slice(&1_u16.to_le_bytes());
        fmt[4..8].copy_from_slice(&44_100_u32.to_le_bytes());
        fmt[0x10..0x12].copy_from_slice(&0x30_u16.to_le_bytes());
        fmt[0x1C..0x20].copy_from_slice(&mod_signal.to_le_bytes());
        fmt[0x40] = 8;
        fmt[0x41] = 11;
        let mut riff = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        riff.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        riff.extend_from_slice(&fmt);
        riff.extend_from_slice(b"data\0\0\0\0");
        riff
    }

    fn empty_table() -> CodebookTable {
        CodebookTable { books: Vec::new() }
    }

    #[test]
    fn non_riff_input_is_rejected() {
        let error = wem_to_ogg(b"OggS", &empty_table()).err().unwrap();
        assert!(error.contains("RIFF"), "{error}");
    }

    #[test]
    fn codecs_other_than_wwise_vorbis_are_rejected() {
        let error = wem_to_ogg(&wem(0xFFFE, 0xB0), &empty_table())
            .err()
            .unwrap();
        assert!(error.contains("0xFFFE"), "{error}");
    }

    #[test]
    fn streams_with_unmodified_packet_headers_are_rejected() {
        let error = wem_to_ogg(&wem(0xFFFF, 0x4A), &empty_table())
            .err()
            .unwrap();
        assert!(error.contains("packet"), "{error}");
    }
}
