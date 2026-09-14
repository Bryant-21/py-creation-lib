use std::io::{Read, Seek, SeekFrom, Write};

use byteorder::{BigEndian, LittleEndian, ReadBytesExt, WriteBytesExt};
use thiserror::Error;

use crate::model::{FLOAT_NAN_TAG, NifValue};

#[derive(Debug, Error)]
pub enum IoError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("short read: expected {expected}, got {got}")]
    ShortRead { expected: usize, got: usize },
    #[error("unknown basic type: {0}")]
    UnknownType(String),
}

pub struct BasicReader<R: Read + Seek> {
    pub reader: R,
    pub big_endian: bool,
}

impl<R: Read + Seek> BasicReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            big_endian: false,
        }
    }

    pub fn pos(&mut self) -> u64 {
        self.reader.stream_position().unwrap_or(0)
    }

    pub fn seek(&mut self, pos: u64) -> Result<(), IoError> {
        self.reader.seek(SeekFrom::Start(pos))?;
        Ok(())
    }

    pub fn read_n_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
        let mut buf = vec![0u8; n];
        self.reader.read_exact(&mut buf)?;
        Ok(buf)
    }

    // --- Integers ---

    pub fn read_byte(&mut self) -> Result<u8, IoError> {
        Ok(self.reader.read_u8()?)
    }
    pub fn read_sbyte(&mut self) -> Result<i8, IoError> {
        Ok(self.reader.read_i8()?)
    }
    pub fn read_ushort(&mut self) -> Result<u16, IoError> {
        Ok(if self.big_endian {
            self.reader.read_u16::<BigEndian>()?
        } else {
            self.reader.read_u16::<LittleEndian>()?
        })
    }
    pub fn read_short(&mut self) -> Result<i16, IoError> {
        Ok(if self.big_endian {
            self.reader.read_i16::<BigEndian>()?
        } else {
            self.reader.read_i16::<LittleEndian>()?
        })
    }
    pub fn read_uint(&mut self) -> Result<u32, IoError> {
        Ok(if self.big_endian {
            self.reader.read_u32::<BigEndian>()?
        } else {
            self.reader.read_u32::<LittleEndian>()?
        })
    }
    pub fn read_int(&mut self) -> Result<i32, IoError> {
        Ok(if self.big_endian {
            self.reader.read_i32::<BigEndian>()?
        } else {
            self.reader.read_i32::<LittleEndian>()?
        })
    }
    pub fn read_ulittle32(&mut self) -> Result<u32, IoError> {
        Ok(self.reader.read_u32::<LittleEndian>()?)
    }
    pub fn read_uint64(&mut self) -> Result<u64, IoError> {
        Ok(if self.big_endian {
            self.reader.read_u64::<BigEndian>()?
        } else {
            self.reader.read_u64::<LittleEndian>()?
        })
    }
    pub fn read_int64(&mut self) -> Result<i64, IoError> {
        Ok(if self.big_endian {
            self.reader.read_i64::<BigEndian>()?
        } else {
            self.reader.read_i64::<LittleEndian>()?
        })
    }

    // --- Floats ---

    pub fn read_float(&mut self) -> Result<NifValue, IoError> {
        let raw = if self.big_endian {
            self.reader.read_u32::<BigEndian>()?
        } else {
            self.reader.read_u32::<LittleEndian>()?
        };
        let f = f32::from_bits(raw);
        if f.is_nan() {
            Ok(NifValue::FloatNan(raw as u64 | FLOAT_NAN_TAG))
        } else {
            Ok(NifValue::Float(f as f64))
        }
    }

    pub fn read_hfloat(&mut self) -> Result<NifValue, IoError> {
        let raw = if self.big_endian {
            self.reader.read_u16::<BigEndian>()?
        } else {
            self.reader.read_u16::<LittleEndian>()?
        };
        let f = half_to_f32(raw);
        if f.is_nan() {
            Ok(NifValue::Int(raw as i64))
        } else {
            Ok(NifValue::Float(f as f64))
        }
    }

    pub fn read_normbyte(&mut self) -> Result<f64, IoError> {
        let b = self.read_byte()?;
        Ok((b as f64 / 127.5) - 1.0)
    }

    // --- Bool (version-dependent) ---

    pub fn read_bool(&mut self, version_packed: u32) -> Result<u32, IoError> {
        if version_packed <= 0x04000002 {
            Ok(self.read_uint()?)
        } else {
            Ok(self.read_byte()? as u32)
        }
    }

    // --- Strings ---

    pub fn read_sized_string(&mut self) -> Result<String, IoError> {
        let len = self.read_uint()? as usize;
        if len == 0 {
            return Ok(String::new());
        }
        let bytes = self.read_n_bytes(len)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn read_sized_string16(&mut self) -> Result<String, IoError> {
        let len = self.read_ushort()? as usize;
        if len == 0 {
            return Ok(String::new());
        }
        let bytes = self.read_n_bytes(len)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn read_header_string(&mut self) -> Result<String, IoError> {
        let mut buf = Vec::with_capacity(64);
        loop {
            let mut b = [0u8; 1];
            let n = self.reader.read(&mut b)?;
            if n == 0 || b[0] == b'\n' {
                break;
            }
            buf.push(b[0]);
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    pub fn read_export_string(&mut self) -> Result<String, IoError> {
        let len = self.read_byte()? as usize;
        if len == 0 {
            return Ok(String::new());
        }
        let bytes = self.read_n_bytes(len)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn read_ni_fixed_string(
        &mut self,
        version_packed: u32,
        strings: &[String],
    ) -> Result<Option<String>, IoError> {
        // version < 20.1.0.3 (0x14010003) -> SizedString
        if version_packed < 0x14010003 {
            return Ok(Some(self.read_sized_string()?));
        }
        let idx = self.read_int()?;
        if idx < 0 {
            return Ok(None);
        }
        let i = idx as usize;
        if i < strings.len() {
            Ok(Some(strings[i].clone()))
        } else {
            Ok(None)
        }
    }

    pub fn read_ref(&mut self) -> Result<i32, IoError> {
        self.read_int()
    }
    pub fn read_ptr(&mut self) -> Result<i32, IoError> {
        self.read_int()
    }
    pub fn read_block_type_index(&mut self) -> Result<i16, IoError> {
        self.read_short()
    }
    pub fn read_file_version(&mut self) -> Result<u32, IoError> {
        self.read_ulittle32()
    }
    pub fn read_string_offset(&mut self) -> Result<u32, IoError> {
        self.read_uint()
    }

    pub fn read_char(&mut self) -> Result<String, IoError> {
        let b = self.read_byte()?;
        // ASCII only; use replacement for non-ASCII to match Python errors="replace"
        if b.is_ascii() {
            Ok((b as char).to_string())
        } else {
            Ok("\u{FFFD}".to_string())
        }
    }

    // --- Bulk array reads (fast path) ---

    pub fn read_bulk_u8(&mut self, count: usize) -> Result<Vec<NifValue>, IoError> {
        let bytes = self.read_n_bytes(count)?;
        Ok(bytes
            .into_iter()
            .map(|b| NifValue::UInt(b as u64))
            .collect())
    }
    pub fn read_bulk_i8(&mut self, count: usize) -> Result<Vec<NifValue>, IoError> {
        let bytes = self.read_n_bytes(count)?;
        Ok(bytes
            .into_iter()
            .map(|b| NifValue::Int(b as i8 as i64))
            .collect())
    }
    pub fn read_bulk_u16(&mut self, count: usize) -> Result<Vec<NifValue>, IoError> {
        let raw = self.read_n_bytes(count * 2)?;
        let be = self.big_endian;
        Ok(raw
            .chunks_exact(2)
            .map(|c| {
                let v = if be {
                    u16::from_be_bytes([c[0], c[1]])
                } else {
                    u16::from_le_bytes([c[0], c[1]])
                };
                NifValue::UInt(v as u64)
            })
            .collect())
    }
    pub fn read_bulk_i16(&mut self, count: usize) -> Result<Vec<NifValue>, IoError> {
        let raw = self.read_n_bytes(count * 2)?;
        let be = self.big_endian;
        Ok(raw
            .chunks_exact(2)
            .map(|c| {
                let v = if be {
                    i16::from_be_bytes([c[0], c[1]])
                } else {
                    i16::from_le_bytes([c[0], c[1]])
                };
                NifValue::Int(v as i64)
            })
            .collect())
    }
    pub fn read_bulk_u32(&mut self, count: usize) -> Result<Vec<NifValue>, IoError> {
        let raw = self.read_n_bytes(count * 4)?;
        let be = self.big_endian;
        Ok(raw
            .chunks_exact(4)
            .map(|c| {
                let arr = [c[0], c[1], c[2], c[3]];
                let v = if be {
                    u32::from_be_bytes(arr)
                } else {
                    u32::from_le_bytes(arr)
                };
                NifValue::UInt(v as u64)
            })
            .collect())
    }
    pub fn read_bulk_i32(&mut self, count: usize) -> Result<Vec<NifValue>, IoError> {
        let raw = self.read_n_bytes(count * 4)?;
        let be = self.big_endian;
        Ok(raw
            .chunks_exact(4)
            .map(|c| {
                let arr = [c[0], c[1], c[2], c[3]];
                let v = if be {
                    i32::from_be_bytes(arr)
                } else {
                    i32::from_le_bytes(arr)
                };
                NifValue::Int(v as i64)
            })
            .collect())
    }
    pub fn read_bulk_f32(&mut self, count: usize) -> Result<Vec<NifValue>, IoError> {
        // NaN-tagged: preserve raw bits when NaN
        let raw = self.read_n_bytes(count * 4)?;
        let be = self.big_endian;
        Ok(raw
            .chunks_exact(4)
            .map(|c| {
                let arr = [c[0], c[1], c[2], c[3]];
                let bits = if be {
                    u32::from_be_bytes(arr)
                } else {
                    u32::from_le_bytes(arr)
                };
                let f = f32::from_bits(bits);
                if f.is_nan() {
                    NifValue::FloatNan(bits as u64 | FLOAT_NAN_TAG)
                } else {
                    NifValue::Float(f as f64)
                }
            })
            .collect())
    }

    // --- Dispatch ---

    pub fn read_basic(
        &mut self,
        type_name: &str,
        version_packed: u32,
        strings: &[String],
    ) -> Result<NifValue, IoError> {
        match type_name {
            "byte" => Ok(NifValue::UInt(self.read_byte()? as u64)),
            "sbyte" => Ok(NifValue::Int(self.read_sbyte()? as i64)),
            "ushort" => Ok(NifValue::UInt(self.read_ushort()? as u64)),
            "short" => Ok(NifValue::Int(self.read_short()? as i64)),
            "uint" => Ok(NifValue::UInt(self.read_uint()? as u64)),
            "int" => Ok(NifValue::Int(self.read_int()? as i64)),
            "ulittle32" => Ok(NifValue::UInt(self.read_ulittle32()? as u64)),
            "uint64" => Ok(NifValue::UInt(self.read_uint64()?)),
            "int64" => Ok(NifValue::Int(self.read_int64()?)),
            "float" => self.read_float(),
            "hfloat" => self.read_hfloat(),
            "normbyte" => Ok(NifValue::Float(self.read_normbyte()?)),
            "char" => Ok(NifValue::Char(self.read_char()?)),
            "bool" => Ok(NifValue::UInt(self.read_bool(version_packed)? as u64)),
            "Ref" => Ok(NifValue::Ref(self.read_ref()?)),
            "Ptr" => Ok(NifValue::Ref(self.read_ptr()?)),
            "BlockTypeIndex" => Ok(NifValue::Int(self.read_block_type_index()? as i64)),
            "FileVersion" => Ok(NifValue::UInt(self.read_file_version()? as u64)),
            "StringOffset" => Ok(NifValue::UInt(self.read_string_offset()? as u64)),
            "HeaderString" | "LineString" => Ok(NifValue::String(self.read_header_string()?)),
            "SizedString" => Ok(NifValue::String(self.read_sized_string()?)),
            "SizedString16" => Ok(NifValue::String(self.read_sized_string16()?)),
            "string" | "NiFixedString" => {
                if version_packed < 0x14010003 {
                    return Ok(NifValue::String(self.read_sized_string()?));
                }
                let index = self.read_int()?;
                if index < 0 {
                    Ok(NifValue::Null)
                } else if let Some(value) = strings.get(index as usize) {
                    Ok(NifValue::String(value.clone()))
                } else {
                    Ok(NifValue::Int(index as i64))
                }
            }
            other => Err(IoError::UnknownType(other.to_string())),
        }
    }
}

// --- Writer ---

pub struct BasicWriter<W: Write> {
    pub writer: W,
    pub big_endian: bool,
}

impl<W: Write> BasicWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            big_endian: false,
        }
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), IoError> {
        self.writer.write_all(bytes)?;
        Ok(())
    }

    // --- Integers ---

    pub fn write_byte(&mut self, v: u8) -> Result<(), IoError> {
        self.writer.write_u8(v)?;
        Ok(())
    }
    pub fn write_sbyte(&mut self, v: i8) -> Result<(), IoError> {
        self.writer.write_i8(v)?;
        Ok(())
    }
    pub fn write_ushort(&mut self, v: u16) -> Result<(), IoError> {
        if self.big_endian {
            self.writer.write_u16::<BigEndian>(v)?;
        } else {
            self.writer.write_u16::<LittleEndian>(v)?;
        }
        Ok(())
    }
    pub fn write_short(&mut self, v: i16) -> Result<(), IoError> {
        if self.big_endian {
            self.writer.write_i16::<BigEndian>(v)?;
        } else {
            self.writer.write_i16::<LittleEndian>(v)?;
        }
        Ok(())
    }
    pub fn write_uint(&mut self, v: u32) -> Result<(), IoError> {
        if self.big_endian {
            self.writer.write_u32::<BigEndian>(v)?;
        } else {
            self.writer.write_u32::<LittleEndian>(v)?;
        }
        Ok(())
    }
    pub fn write_int(&mut self, v: i32) -> Result<(), IoError> {
        if self.big_endian {
            self.writer.write_i32::<BigEndian>(v)?;
        } else {
            self.writer.write_i32::<LittleEndian>(v)?;
        }
        Ok(())
    }
    pub fn write_ulittle32(&mut self, v: u32) -> Result<(), IoError> {
        self.writer.write_u32::<LittleEndian>(v)?;
        Ok(())
    }
    pub fn write_uint64(&mut self, v: u64) -> Result<(), IoError> {
        if self.big_endian {
            self.writer.write_u64::<BigEndian>(v)?;
        } else {
            self.writer.write_u64::<LittleEndian>(v)?;
        }
        Ok(())
    }
    pub fn write_int64(&mut self, v: i64) -> Result<(), IoError> {
        if self.big_endian {
            self.writer.write_i64::<BigEndian>(v)?;
        } else {
            self.writer.write_i64::<LittleEndian>(v)?;
        }
        Ok(())
    }

    // --- Floats ---

    pub fn write_float(&mut self, v: &NifValue) -> Result<(), IoError> {
        let bits: u32 = match v {
            NifValue::FloatNan(tagged) => (*tagged & 0xFFFF_FFFF) as u32,
            NifValue::Float(f) => (*f as f32).to_bits(),
            NifValue::Int(i) => (*i as f32).to_bits(),
            NifValue::UInt(u) => (*u as f32).to_bits(),
            NifValue::Null => 0u32,
            _ => 0u32,
        };
        if self.big_endian {
            self.writer.write_u32::<BigEndian>(bits)?;
        } else {
            self.writer.write_u32::<LittleEndian>(bits)?;
        }
        Ok(())
    }

    pub fn write_hfloat(&mut self, v: &NifValue) -> Result<(), IoError> {
        let bits: u16 = match v {
            // Int carries raw u16 bits for NaN preservation.
            NifValue::Int(i) => (*i as i64 & 0xFFFF) as u16,
            NifValue::UInt(u) => (*u & 0xFFFF) as u16,
            NifValue::Float(f) => f32_to_half(*f as f32),
            NifValue::FloatNan(tagged) => (*tagged & 0xFFFF) as u16,
            NifValue::Null => 0u16,
            _ => 0u16,
        };
        if self.big_endian {
            self.writer.write_u16::<BigEndian>(bits)?;
        } else {
            self.writer.write_u16::<LittleEndian>(bits)?;
        }
        Ok(())
    }

    pub fn write_normbyte(&mut self, v: f64) -> Result<(), IoError> {
        let scaled = ((v + 1.0) * 127.5).round();
        let b = scaled.max(0.0).min(255.0) as i64;
        self.write_byte(b as u8)
    }

    pub fn write_bool(&mut self, v: &NifValue, version_packed: u32) -> Result<(), IoError> {
        let raw = v.as_i64();
        if version_packed <= 0x04000002 {
            self.write_uint(raw as u32)
        } else {
            self.write_byte(raw as u8)
        }
    }

    // --- Strings ---

    pub fn write_sized_string(&mut self, s: &str) -> Result<(), IoError> {
        let bytes = s.as_bytes();
        self.write_uint(bytes.len() as u32)?;
        self.write_bytes(bytes)
    }

    pub fn write_sized_string16(&mut self, s: &str) -> Result<(), IoError> {
        let bytes = s.as_bytes();
        self.write_ushort(bytes.len() as u16)?;
        self.write_bytes(bytes)
    }

    pub fn write_header_string(&mut self, s: &str) -> Result<(), IoError> {
        self.write_bytes(s.as_bytes())?;
        self.write_byte(b'\n')
    }

    pub fn write_export_string(&mut self, bytes: &[u8]) -> Result<(), IoError> {
        self.write_byte(bytes.len() as u8)?;
        if !bytes.is_empty() {
            self.write_bytes(bytes)?;
        }
        Ok(())
    }

    pub fn write_char(&mut self, s: &str) -> Result<(), IoError> {
        // ASCII only. Empty -> 0 byte; multi-byte -> take first byte.
        let bytes = s.as_bytes();
        let b = if bytes.is_empty() { 0u8 } else { bytes[0] };
        self.write_byte(b)
    }

    pub fn write_ref(&mut self, v: i32) -> Result<(), IoError> {
        self.write_int(v)
    }
    pub fn write_ptr(&mut self, v: i32) -> Result<(), IoError> {
        self.write_int(v)
    }
    pub fn write_block_type_index(&mut self, v: i16) -> Result<(), IoError> {
        self.write_short(v)
    }
    pub fn write_file_version(&mut self, v: u32) -> Result<(), IoError> {
        self.write_ulittle32(v)
    }
    pub fn write_string_offset(&mut self, v: u32) -> Result<(), IoError> {
        self.write_uint(v)
    }

    pub fn write_ni_fixed_string(
        &mut self,
        s: Option<&str>,
        version_packed: u32,
        string_index_map: &std::collections::HashMap<String, i32>,
    ) -> Result<(), IoError> {
        if version_packed < 0x14010003 {
            self.write_sized_string(s.unwrap_or(""))
        } else {
            let idx: i32 = match s {
                None => -1,
                Some(v) => string_index_map.get(v).copied().unwrap_or(-1),
            };
            self.write_int(idx)
        }
    }

    // --- Dispatch ---

    pub fn write_basic(
        &mut self,
        type_name: &str,
        val: &NifValue,
        version_packed: u32,
        string_index_map: &std::collections::HashMap<String, i32>,
    ) -> Result<(), IoError> {
        match type_name {
            "byte" => self.write_byte(nif_val_to_u8(val)),
            "sbyte" => self.write_sbyte(nif_val_to_i8(val)),
            "ushort" => self.write_ushort(nif_val_to_u16(val)),
            "short" => self.write_short(nif_val_to_i16(val)),
            "uint" => self.write_uint(nif_val_to_u32(val)),
            "int" => self.write_int(nif_val_to_i32(val)),
            "ulittle32" => self.write_ulittle32(nif_val_to_u32(val)),
            "uint64" => self.write_uint64(nif_val_to_u64(val)),
            "int64" => self.write_int64(nif_val_to_i64(val)),
            "float" => self.write_float(val),
            "hfloat" => self.write_hfloat(val),
            "normbyte" => self.write_normbyte(match val {
                NifValue::Float(f) => *f,
                _ => val.as_i64() as f64,
            }),
            "char" => self.write_char(match val {
                NifValue::Char(s) | NifValue::String(s) => s.as_str(),
                _ => "",
            }),
            "bool" => self.write_bool(val, version_packed),
            "Ref" | "Ptr" => self.write_ref(match val {
                NifValue::Ref(r) => *r,
                _ => val.as_i64() as i32,
            }),
            "BlockTypeIndex" => self.write_block_type_index(nif_val_to_i16(val)),
            "FileVersion" => self.write_file_version(nif_val_to_u32(val)),
            "StringOffset" => self.write_string_offset(nif_val_to_u32(val)),
            "HeaderString" | "LineString" => self.write_header_string(match val {
                NifValue::String(s) => s.as_str(),
                _ => "",
            }),
            "SizedString" => self.write_sized_string(match val {
                NifValue::String(s) => s.as_str(),
                NifValue::Null => "",
                _ => "",
            }),
            "SizedString16" => self.write_sized_string16(match val {
                NifValue::String(s) => s.as_str(),
                NifValue::Null => "",
                _ => "",
            }),
            "string" | "NiFixedString" => {
                if version_packed >= 0x14010003 {
                    if let NifValue::Int(index) = val {
                        return self.write_int(*index as i32);
                    }
                }
                let s: Option<&str> = match val {
                    NifValue::Null => None,
                    NifValue::String(s) => Some(s.as_str()),
                    _ => Some(""),
                };
                self.write_ni_fixed_string(s, version_packed, string_index_map)
            }
            other => Err(IoError::UnknownType(other.to_string())),
        }
    }
}

// --- NifValue -> scalar helpers ---

pub fn nif_val_to_u8(v: &NifValue) -> u8 {
    (v.as_i64() & 0xFF) as u8
}
pub fn nif_val_to_i8(v: &NifValue) -> i8 {
    (v.as_i64() & 0xFF) as i8
}
pub fn nif_val_to_u16(v: &NifValue) -> u16 {
    (v.as_i64() & 0xFFFF) as u16
}
pub fn nif_val_to_i16(v: &NifValue) -> i16 {
    (v.as_i64() & 0xFFFF) as i16
}
pub fn nif_val_to_u32(v: &NifValue) -> u32 {
    (v.as_i64() & 0xFFFF_FFFF) as u32
}
pub fn nif_val_to_i32(v: &NifValue) -> i32 {
    v.as_i64() as i32
}
pub fn nif_val_to_u64(v: &NifValue) -> u64 {
    match v {
        NifValue::UInt(u) => *u,
        _ => v.as_i64() as u64,
    }
}
pub fn nif_val_to_i64(v: &NifValue) -> i64 {
    v.as_i64()
}

// IEEE 754 half-precision -> f32 conversion.
fn half_to_f32(h: u16) -> f32 {
    let sign = (h >> 15) & 0x1;
    let exp = (h >> 10) & 0x1F;
    let mant = h & 0x3FF;

    let f_bits: u32 = if exp == 0 {
        if mant == 0 {
            (sign as u32) << 31
        } else {
            // subnormal
            let mut m = mant as u32;
            let mut e: i32 = -14;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            m &= 0x3FF;
            let e_biased = (e + 127) as u32;
            ((sign as u32) << 31) | (e_biased << 23) | (m << 13)
        }
    } else if exp == 31 {
        // inf / nan
        ((sign as u32) << 31) | (0xFFu32 << 23) | ((mant as u32) << 13)
    } else {
        let e = (exp as i32) - 15 + 127;
        ((sign as u32) << 31) | ((e as u32) << 23) | ((mant as u32) << 13)
    };
    f32::from_bits(f_bits)
}

// IEEE 754 f32 -> half-precision u16 conversion (round-to-nearest-even).
fn f32_to_half(f: f32) -> u16 {
    let bits = f.to_bits();
    let sign = ((bits >> 31) & 0x1) as u16;
    let exp32 = ((bits >> 23) & 0xFF) as i32;
    let mant32 = bits & 0x7F_FFFF;

    if exp32 == 0xFF {
        // inf / nan
        let mant_h: u16 = if mant32 != 0 {
            // NaN — preserve top mantissa bits, ensure non-zero
            let m = (mant32 >> 13) as u16 & 0x3FF;
            if m == 0 { 0x200 } else { m }
        } else {
            0
        };
        return (sign << 15) | (0x1F << 10) | mant_h;
    }
    // Unbiased exponent.
    let e = exp32 - 127;
    if e > 15 {
        // Overflow -> inf
        return (sign << 15) | (0x1F << 10);
    }
    if e < -24 {
        // Underflow -> zero (with sign)
        return sign << 15;
    }
    if e < -14 {
        // Subnormal half
        let shift = -e - 14;
        let mant_full = mant32 | 0x0080_0000; // add implicit leading 1
        let shift_amt = 13 + shift as u32;
        if shift_amt >= 32 {
            return sign << 15;
        }
        let half_round = 1u32 << (shift_amt - 1);
        let m = (mant_full + half_round) >> shift_amt;
        return (sign << 15) | (m as u16 & 0x3FF);
    }
    let exp_h = (e + 15) as u16;
    // Round to nearest even.
    let round_bit = 1u32 << 12;
    let lsb = (mant32 >> 13) & 1;
    let rem = mant32 & 0x1FFF;
    let mut mant_h = (mant32 >> 13) as u16;
    if rem > round_bit || (rem == round_bit && lsb != 0) {
        mant_h += 1;
    }
    let mut exp_h = exp_h;
    if mant_h & 0x400 != 0 {
        mant_h = 0;
        exp_h += 1;
        if exp_h >= 0x1F {
            return (sign << 15) | (0x1F << 10);
        }
    }
    (sign << 15) | ((exp_h & 0x1F) << 10) | (mant_h & 0x3FF)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn invalid_string_table_index_is_preserved_for_validation() {
        let mut reader = BasicReader::new(Cursor::new(5i32.to_le_bytes()));
        let value = reader
            .read_basic("NiFixedString", 0x1402_0007, &["Only".to_string()])
            .unwrap();
        assert_eq!(value, NifValue::Int(5));
    }
}
