//! SWF container: (de)compression, header parse, and a byte-exact tag-stream
//! splitter.
//!
//! The splitter never decodes tag bodies — it returns each top-level tag as an
//! opaque byte span into the decompressed movie body, so untouched tags survive
//! byte-for-byte when we splice. This is the foundation for marker-symbol
//! injection: the existing pure-Python codec re-minimizes shape bit widths and
//! is therefore byte-lossy, which is unusable for editing real menu SWFs.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signature {
    /// "FWS" — uncompressed.
    Uncompressed,
    /// "CWS" — zlib (DEFLATE) compressed from byte 8 onward.
    Zlib,
    /// "ZWS" — LZMA compressed (not produced by Bethesda menu SWFs).
    Lzma,
}

/// A decompressed SWF movie: the bytes from file offset 8 onward (FrameSize
/// RECT + frame rate/count + tag stream), with the original signature/version
/// retained so the file can be re-emitted.
#[derive(Debug, Clone)]
pub struct Movie {
    pub signature: Signature,
    pub version: u8,
    /// Decompressed bytes after the 8-byte file header.
    pub body: Vec<u8>,
}

/// One top-level tag, addressed as a byte span into [`Movie::body`].
#[derive(Debug, Clone, Copy)]
pub struct TagSpan {
    pub code: u16,
    /// Offset of the tag header in the movie body.
    pub start: usize,
    /// 2 (short header) or 6 (long header: 0x3F marker + u32 length).
    pub header_len: usize,
    pub body_len: usize,
}

impl TagSpan {
    pub fn end(&self) -> usize {
        self.start + self.header_len + self.body_len
    }

    /// Range of the tag *body* (excludes the header) within the movie body.
    pub fn body_range(&self) -> std::ops::Range<usize> {
        let s = self.start + self.header_len;
        s..s + self.body_len
    }
}

pub fn decompress(raw: &[u8]) -> Result<Movie, String> {
    if raw.len() < 8 {
        return Err("SWF too short".into());
    }
    let signature = match &raw[0..3] {
        b"FWS" => Signature::Uncompressed,
        b"CWS" => Signature::Zlib,
        b"ZWS" => Signature::Lzma,
        other => return Err(format!("not a SWF (bad signature {other:?})")),
    };
    let version = raw[3];
    let file_length = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
    let body = match signature {
        Signature::Uncompressed => raw[8..].to_vec(),
        Signature::Zlib => {
            let mut out = Vec::with_capacity(file_length.saturating_sub(8));
            ZlibDecoder::new(&raw[8..])
                .read_to_end(&mut out)
                .map_err(|e| format!("zlib inflate failed: {e}"))?;
            out
        }
        Signature::Lzma => return Err("ZWS (LZMA) SWF not supported".into()),
    };
    Ok(Movie {
        signature,
        version,
        body,
    })
}

/// Byte length of the leading FrameSize RECT in a movie body. The RECT is
/// `Nbits` (5 bits) followed by four `Nbits`-wide signed fields, byte-aligned.
fn rect_byte_len(body: &[u8]) -> Result<usize, String> {
    let first = *body.first().ok_or("empty movie body")?;
    let nbits = (first >> 3) as usize; // top 5 bits
    let total_bits = 5 + 4 * nbits;
    Ok(total_bits.div_ceil(8))
}

/// Offset in the movie body where the tag stream begins (after the RECT plus a
/// u16 frame rate and a u16 frame count).
pub fn tags_offset(body: &[u8]) -> Result<usize, String> {
    Ok(rect_byte_len(body)? + 4)
}

/// Split the tag stream into opaque byte spans. Stops after the End tag (code
/// 0). Returns an error if any tag header or body overruns the movie.
pub fn split_tags(body: &[u8]) -> Result<Vec<TagSpan>, String> {
    split_tags_at(body, tags_offset(body)?)
}

/// Like [`split_tags`] but starting at an explicit offset — used to walk the
/// control-tag stream inside a DefineSprite body (which begins after a u16
/// sprite id and u16 frame count, i.e. offset 4 within the sprite body).
pub fn split_tags_at(body: &[u8], start: usize) -> Result<Vec<TagSpan>, String> {
    let mut p = start;
    let mut spans = Vec::new();
    while p + 2 <= body.len() {
        let code_and_len = u16::from_le_bytes([body[p], body[p + 1]]);
        let code = code_and_len >> 6;
        let short_len = (code_and_len & 0x3F) as usize;
        let (header_len, body_len) = if short_len == 0x3F {
            if p + 6 > body.len() {
                return Err("truncated long tag header".into());
            }
            let long =
                u32::from_le_bytes([body[p + 2], body[p + 3], body[p + 4], body[p + 5]]) as usize;
            (6, long)
        } else {
            (2, short_len)
        };
        if p + header_len + body_len > body.len() {
            return Err(format!("tag {code} body overruns movie at offset {p}"));
        }
        spans.push(TagSpan {
            code,
            start: p,
            header_len,
            body_len,
        });
        p += header_len + body_len;
        if code == 0 {
            break; // End tag terminates the top-level stream
        }
    }
    Ok(spans)
}

/// Encode a tag header. `force_long` preserves a long header on a body that
/// would fit in short form (some authoring tools emit long headers); callers
/// splicing untouched tags must copy the original header bytes instead, since
/// this minimizes by default.
pub fn write_tag_header(code: u16, body_len: usize, force_long: bool) -> Vec<u8> {
    if body_len >= 0x3F || force_long {
        let code_and_len = (code << 6) | 0x3F;
        let mut out = code_and_len.to_le_bytes().to_vec();
        out.extend_from_slice(&(body_len as u32).to_le_bytes());
        out
    } else {
        let code_and_len = (code << 6) | (body_len as u16);
        code_and_len.to_le_bytes().to_vec()
    }
}

/// Re-assemble a SWF file from a (possibly edited) movie body. The 8-byte file
/// header is rewritten (FileLength = 8 + body.len(), the UNcompressed total per
/// spec). CWS bodies are re-deflated at a fixed level so the output is
/// deterministic; the in-game loader decompresses, so byte-exact recompression
/// is unnecessary (untouched tags are preserved exactly in the decompressed
/// body). ZWS is not produced.
pub fn assemble(signature: Signature, version: u8, body: &[u8]) -> Result<Vec<u8>, String> {
    let sig = match signature {
        Signature::Uncompressed => b"FWS",
        Signature::Zlib => b"CWS",
        Signature::Lzma => return Err("cannot assemble ZWS (LZMA) SWF".into()),
    };
    let file_length = (8 + body.len()) as u32;
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(sig);
    out.push(version);
    out.extend_from_slice(&file_length.to_le_bytes());
    match signature {
        Signature::Uncompressed => out.extend_from_slice(body),
        Signature::Zlib => {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(6));
            enc.write_all(body)
                .map_err(|e| format!("zlib deflate failed: {e}"))?;
            out.extend_from_slice(&enc.finish().map_err(|e| format!("zlib finish: {e}"))?);
        }
        Signature::Lzma => unreachable!(),
    }
    Ok(out)
}
