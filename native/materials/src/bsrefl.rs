#![allow(dead_code)]

use std::collections::HashMap;

use serde::Serialize;

use crate::error::{MaterialError, Result};
use crate::string_table::STRING_TABLE;

pub mod chunk_type {
    pub const NONE: u32 = 0;
    pub const BETH: u32 = 0x4854_4542;
    pub const STRT: u32 = 0x5452_5453;
    pub const TYPE: u32 = 0x4550_5954;
    pub const CLAS: u32 = 0x5341_4C43;
    pub const LIST: u32 = 0x5453_494C;
    pub const MAPC: u32 = 0x4350_414D;
    pub const OBJT: u32 = 0x544A_424F;
    pub const DIFF: u32 = 0x4646_4944;
    pub const USER: u32 = 0x5245_5355;
    pub const USRD: u32 = 0x4452_5355;
}

const BETH_MAGIC: u64 = 0x0000_0008_4854_4542;

#[derive(Debug, Clone, Serialize)]
pub struct BsreflSummary {
    pub chunks_remaining: u32,
    pub chunks: Vec<ChunkSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkSummary {
    #[serde(rename = "type")]
    pub chunk_type: u32,
    pub size: usize,
}

pub fn find_master_string(value: &str) -> i32 {
    let mut n0 = 19usize;
    let mut n2 = STRING_TABLE.len();
    while n2 > n0 + 1 {
        let n1 = (n0 + n2) >> 1;
        if value < STRING_TABLE[n1] {
            n2 = n1;
        } else {
            n0 = n1;
        }
    }
    if n2 > n0 && STRING_TABLE[n0] == value {
        n0 as i32
    } else {
        -1
    }
}

pub struct Chunk<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Chunk<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }

    fn read_exact(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        if end > self.data.len() {
            self.pos = self.data.len();
            return None;
        }
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Some(bytes)
    }

    pub fn read_u8(&mut self) -> Option<u8> {
        Some(self.read_exact(1)?[0])
    }

    pub fn read_bool(&mut self) -> Option<bool> {
        Some(self.read_u8()? != 0)
    }

    pub fn read_u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.read_exact(2)?.try_into().ok()?))
    }

    pub fn read_u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.read_exact(4)?.try_into().ok()?))
    }

    pub fn read_i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.read_exact(4)?.try_into().ok()?))
    }

    pub fn read_u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.read_exact(8)?.try_into().ok()?))
    }

    pub fn read_i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.read_exact(8)?.try_into().ok()?))
    }

    pub fn read_float(&mut self) -> Option<f32> {
        let mut raw = self.read_u32()?;
        if ((raw.wrapping_add(0x0080_0000)) & 0x7F00_0000) == 0 {
            raw = 0;
        }
        Some(f32::from_bits(raw))
    }

    pub fn read_float_0_to_1(&mut self) -> Option<f32> {
        Some(self.read_float()?.clamp(0.0, 1.0))
    }

    pub fn read_double(&mut self) -> Option<f64> {
        Some(f64::from_bits(self.read_u64()?))
    }

    pub fn read_string(&mut self) -> Option<String> {
        let len = self.read_u16()? as usize;
        let raw = self.read_exact(len)?;
        let trimmed_len = raw.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        let raw = &raw[..trimmed_len];
        Some(String::from_utf8_lossy(raw).into_owned())
    }

    pub fn get_field_number(&mut self, mut n: u16, n_max: u16, is_diff: bool) -> Option<u16> {
        if !is_diff {
            n = n.wrapping_add(1);
            return (n <= n_max).then_some(n);
        }

        n = self.read_u16()?;
        let signed_n = n as i16;
        let signed_max = n_max as i16;
        if signed_n <= signed_max {
            return (signed_n >= 0).then_some(n);
        }
        self.pos = self.data.len();
        None
    }
}

pub struct Stream<'a> {
    data: &'a [u8],
    pos: usize,
    pub chunks_remaining: u32,
    string_map: HashMap<u32, i32>,
}

impl<'a> Stream<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self> {
        let mut stream = Self {
            data,
            pos: 0,
            chunks_remaining: 0,
            string_map: HashMap::new(),
        };
        stream.read_string_table()?;
        Ok(stream)
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| MaterialError::invalid("unexpected end of reflection stream"))?;
        if end > self.data.len() {
            return Err(MaterialError::invalid(
                "unexpected end of reflection stream",
            ));
        }
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn read_u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read_exact(4)?.try_into().map_err(
            |_| MaterialError::invalid("unexpected end of reflection stream"),
        )?))
    }

    fn read_u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.read_exact(8)?.try_into().map_err(
            |_| MaterialError::invalid("unexpected end of reflection stream"),
        )?))
    }

    fn read_string_table(&mut self) -> Result<()> {
        if self.data.len() < 24 || self.read_u64()? != BETH_MAGIC {
            return Err(MaterialError::invalid("invalid reflection stream header"));
        }
        if self.read_u32()? != 4 {
            return Err(MaterialError::invalid(
                "unsupported reflection stream version",
            ));
        }

        self.chunks_remaining = self.read_u32()?;
        if self.chunks_remaining < 2 || self.read_u32()? != chunk_type::STRT {
            return Err(MaterialError::invalid(
                "missing string table in reflection stream",
            ));
        }
        self.chunks_remaining -= 2;

        let string_table_len = self.read_u32()? as usize;
        let end = self
            .pos
            .checked_add(string_table_len)
            .ok_or_else(|| MaterialError::invalid("unexpected end of reflection stream"))?;
        if end > self.data.len() {
            return Err(MaterialError::invalid(
                "unexpected end of reflection stream",
            ));
        }

        while self.pos < end {
            let entry_offset = (self.pos - 24) as u32;
            let string_start = self.pos;
            while self.pos < end && self.data[self.pos] != 0 {
                self.pos += 1;
            }
            if self.pos >= end {
                return Err(MaterialError::invalid(
                    "string table is not terminated in reflection stream",
                ));
            }
            let value = String::from_utf8_lossy(&self.data[string_start..self.pos]);
            self.pos += 1;
            self.string_map
                .insert(entry_offset, find_master_string(value.as_ref()));
        }

        Ok(())
    }

    pub fn read_chunk(&mut self) -> Result<Option<(u32, Chunk<'a>)>> {
        if self.chunks_remaining == 0 {
            return Ok(None);
        }
        self.chunks_remaining -= 1;

        let chunk_type = self.read_u32()?;
        let chunk_size = self.read_u32()? as usize;
        let body = self.read_exact(chunk_size)?;
        Ok(Some((chunk_type, Chunk::new(body))))
    }

    pub fn find_string_by_offset(&self, strt_offs: u32) -> u32 {
        if let Some(index) = self.string_map.get(&strt_offs) {
            if *index >= 0 {
                return *index as u32;
            }
        }
        strt_offs.wrapping_sub(0xFFFF_FF01).min(18)
    }

    pub fn get_string(&self, strt_offs: u32) -> &'static str {
        let index = self.find_string_by_offset(strt_offs) as usize;
        STRING_TABLE.get(index).copied().unwrap_or(STRING_TABLE[18])
    }
}

pub fn inspect(data: &[u8]) -> Result<BsreflSummary> {
    let mut stream = Stream::new(data)?;
    let chunks_remaining = stream.chunks_remaining;
    let mut chunks = Vec::new();
    while let Some((chunk_type, chunk)) = stream.read_chunk()? {
        chunks.push(ChunkSummary {
            chunk_type,
            size: chunk.size(),
        });
    }
    Ok(BsreflSummary {
        chunks_remaining,
        chunks,
    })
}

#[cfg(test)]
mod tests {
    use super::Chunk;

    #[test]
    fn chunk_read_string_trims_all_trailing_nul_padding() {
        let mut body = Vec::new();
        body.extend_from_slice(&5u16.to_le_bytes());
        body.extend_from_slice(b"hi\0\0\0");

        let mut chunk = Chunk::new(&body);

        assert_eq!(chunk.read_string().as_deref(), Some("hi"));
    }
}
