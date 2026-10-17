//! Byte offsets of the bitmap character ids inside DefineShape 1-4 and
//! DefineMorphShape 1-2 bodies. Shapes name bitmap fills by id instead of
//! placing them, so a character closure must follow these ids and a remap must
//! rewrite them in place.

use std::collections::HashSet;

use crate::container::split_tags;

/// The id Flash writes for a bitmap fill style that names no bitmap.
pub const NO_BITMAP: u16 = 0xFFFF;

/// `(shape id, named id)` for every shape fill style in a movie body that names a
/// character which is not a bitmap. Scaleform builds such a fill with no image
/// and dereferences it when the shape first draws.
pub fn dangling_bitmap_fills(body: &[u8]) -> Result<Vec<(u16, u16)>, String> {
    let spans = split_tags(body)?;
    let id = |at: usize| u16::from_le_bytes([body[at], body[at + 1]]);
    let bitmaps: HashSet<u16> = spans
        .iter()
        .filter(|s| matches!(s.code, 6 | 20 | 21 | 35 | 36 | 90))
        .map(|s| id(s.body_range().start))
        .collect();
    let mut dangling = Vec::new();
    for span in &spans {
        let start = span.body_range().start;
        for at in bitmap_id_offsets(span.code, &body[span.body_range()])? {
            let named = id(start + at);
            if !bitmaps.contains(&named) {
                dangling.push((id(start), named));
            }
        }
    }
    Ok(dangling)
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u8,
}

impl Bits<'_> {
    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }

    fn ub(&mut self, n: u32) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..n {
            let byte = *self.data.get(self.pos).ok_or("shape overruns its tag")?;
            value = (value << 1) | u32::from((byte >> (7 - self.bit)) & 1);
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, String> {
        self.align();
        let byte = *self.data.get(self.pos).ok_or("shape overruns its tag")?;
        self.pos += 1;
        Ok(byte)
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from(self.u8()?) | u16::from(self.u8()?) << 8)
    }

    fn skip(&mut self, n: usize) -> Result<(), String> {
        self.align();
        if self.pos + n > self.data.len() {
            return Err("shape overruns its tag".into());
        }
        self.pos += n;
        Ok(())
    }

    fn rect(&mut self) -> Result<(), String> {
        self.align();
        let n = self.ub(5)?;
        self.ub(4 * n).map(drop)
    }

    fn matrix(&mut self) -> Result<(), String> {
        self.align();
        for _ in 0..2 {
            if self.ub(1)? == 1 {
                let n = self.ub(5)?;
                self.ub(2 * n)?;
            }
        }
        let n = self.ub(5)?;
        self.ub(2 * n).map(drop)
    }

    fn bitmap_id(&mut self, out: &mut Vec<usize>) -> Result<(), String> {
        let at = self.pos;
        if self.u16()? != NO_BITMAP {
            out.push(at);
        }
        Ok(())
    }

    fn count(&mut self, extended: bool) -> Result<usize, String> {
        let count = self.u8()?;
        Ok(if count == 0xFF && extended { usize::from(self.u16()?) } else { usize::from(count) })
    }
}

fn fill_style(b: &mut Bits, version: u8, out: &mut Vec<usize>) -> Result<(), String> {
    let color = if version >= 3 { 4 } else { 3 };
    match b.u8()? {
        0x00 => b.skip(color),
        kind @ (0x10 | 0x12 | 0x13) => {
            b.matrix()?;
            let records = usize::from(b.u8()? & 0x0F);
            b.skip(records * (1 + color))?;
            if kind == 0x13 { b.skip(2) } else { Ok(()) }
        }
        0x40..=0x43 => {
            b.bitmap_id(out)?;
            b.matrix()
        }
        kind => Err(format!("unknown fill style type {kind:#04x}")),
    }
}

fn fill_styles(b: &mut Bits, version: u8, out: &mut Vec<usize>) -> Result<(), String> {
    for _ in 0..b.count(version >= 2)? {
        fill_style(b, version, out)?;
    }
    Ok(())
}

/// Reads LINESTYLE2's two flag bytes (JoinStyle is bits 5-4 of the first, HasFillFlag
/// bit 3) and the miter limit a miter join adds; returns HasFillFlag.
fn line_style2_has_fill(b: &mut Bits) -> Result<bool, String> {
    let first = b.u8()?;
    b.u8()?;
    if (first >> 4) & 3 == 2 {
        b.u16()?;
    }
    Ok(first & 0x08 != 0)
}

fn line_styles(b: &mut Bits, version: u8, out: &mut Vec<usize>) -> Result<(), String> {
    for _ in 0..b.count(true)? {
        b.u16()?;
        if version < 4 {
            b.skip(if version == 3 { 4 } else { 3 })?;
        } else if line_style2_has_fill(b)? {
            fill_style(b, version, out)?;
        } else {
            b.skip(4)?;
        }
    }
    Ok(())
}

fn shape_records(b: &mut Bits, version: u8, out: &mut Vec<usize>) -> Result<(), String> {
    b.align();
    let (mut fill_bits, mut line_bits) = (b.ub(4)?, b.ub(4)?);
    loop {
        if b.ub(1)? == 1 {
            let straight = b.ub(1)? == 1;
            let n = b.ub(4)? + 2;
            let fields = if !straight { 4 * n } else if b.ub(1)? == 1 { 2 * n } else { 1 + n };
            b.ub(fields)?;
            continue;
        }
        let flags = b.ub(5)?;
        if flags == 0 {
            return Ok(());
        }
        if flags & 1 != 0 {
            let n = b.ub(5)?;
            b.ub(2 * n)?;
        }
        let fills = u32::from(flags & 2 != 0) + u32::from(flags & 4 != 0);
        b.ub(fills * fill_bits + if flags & 8 != 0 { line_bits } else { 0 })?;
        if flags & 16 != 0 {
            fill_styles(b, version, out)?;
            line_styles(b, version, out)?;
            b.align();
            (fill_bits, line_bits) = (b.ub(4)?, b.ub(4)?);
        }
    }
}

fn morph_fill_style(b: &mut Bits, out: &mut Vec<usize>) -> Result<(), String> {
    match b.u8()? {
        0x00 => b.skip(8),
        0x10 | 0x12 => {
            b.matrix()?;
            b.matrix()?;
            let records = usize::from(b.u8()? & 0x0F);
            b.skip(records * 10)
        }
        0x40..=0x43 => {
            b.bitmap_id(out)?;
            b.matrix()?;
            b.matrix()
        }
        kind => Err(format!("unknown morph fill style type {kind:#04x}")),
    }
}

/// Offsets of every bitmap id (other than [`NO_BITMAP`]) a shape or morph
/// shape body names; empty for any other tag.
pub fn bitmap_id_offsets(code: u16, body: &[u8]) -> Result<Vec<usize>, String> {
    let mut b = Bits { data: body, pos: 2, bit: 0 };
    let mut out = Vec::new();
    match code {
        2 | 22 | 32 | 83 => {
            let version = match code {
                2 => 1,
                22 => 2,
                32 => 3,
                _ => 4,
            };
            b.rect()?;
            if version == 4 {
                b.rect()?;
                b.u8()?;
            }
            fill_styles(&mut b, version, &mut out)?;
            line_styles(&mut b, version, &mut out)?;
            shape_records(&mut b, version, &mut out)?;
        }
        46 | 84 => {
            b.rect()?;
            b.rect()?;
            if code == 84 {
                b.rect()?;
                b.rect()?;
                b.u8()?;
            }
            b.skip(4)?; // offset to the end edges, which name styles only by index
            for _ in 0..b.count(true)? {
                morph_fill_style(&mut b, &mut out)?;
            }
            if code == 84 {
                for _ in 0..b.count(true)? {
                    b.skip(4)?;
                    if line_style2_has_fill(&mut b)? {
                        morph_fill_style(&mut b, &mut out)?;
                    } else {
                        b.skip(8)?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(out)
}
