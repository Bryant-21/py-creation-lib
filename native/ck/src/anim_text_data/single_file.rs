//! Parser/emitter/merger for the AnimTextData aggregate
//! `behaviorclipinformationandsubgraphanimationoffsetssinglefile.txt` (format "V4").
//! Grammar: two blocks back-to-back.
//! Block A: count line, then N x (u32 name_id decimal line + ClipGeneratorData body).
//! Block B: count line, then N x (u64 subgraph-id decimal line + AnimationOffsets body).
//! Bodies begin with the ASCII line "V4" and are self-delimiting.
//! The engine loads clip-generator data ONLY from this file (no per-file fallback), so
//! the shipped file must carry vanilla's entries verbatim -- hence the raw-bytes model.

pub struct SingleFileEntry {
    pub key: u64,
    pub body: Vec<u8>,
}

pub struct SingleFile {
    pub block_a: Vec<SingleFileEntry>,
    pub block_b: Vec<SingleFileEntry>,
}

fn read_decimal_line(data: &[u8], pos: &mut usize) -> Result<u64, String> {
    let start = *pos;
    let end = data[start..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|offset| start + offset)
        .ok_or_else(|| format!("unterminated decimal line at offset {start}"))?;
    let text = std::str::from_utf8(&data[start..end])
        .map_err(|_| format!("non-ASCII decimal line at offset {start}"))?;
    let value = text
        .parse::<u64>()
        .map_err(|_| format!("invalid decimal {text:?} at offset {start}"))?;
    *pos = end + 1;
    Ok(value)
}

fn parse_block(
    data: &[u8],
    pos: &mut usize,
    body_len: fn(&[u8]) -> Result<usize, String>,
) -> Result<Vec<SingleFileEntry>, String> {
    let count = read_decimal_line(data, pos)? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let key = read_decimal_line(data, pos)?;
        let len = body_len(&data[*pos..])?;
        entries.push(SingleFileEntry {
            key,
            body: data[*pos..*pos + len].to_vec(),
        });
        *pos += len;
    }
    Ok(entries)
}

pub fn parse_single_file(data: &[u8]) -> Result<SingleFile, String> {
    let mut pos = 0usize;
    let block_a = parse_block(data, &mut pos, clip_generator_body_len)?;
    let block_b = parse_block(data, &mut pos, offsets_body_len)?;
    if pos != data.len() {
        return Err(format!(
            "trailing bytes after block B: consumed {pos} of {}",
            data.len()
        ));
    }
    Ok(SingleFile { block_a, block_b })
}

pub fn emit_single_file(file: &SingleFile) -> Vec<u8> {
    let mut out = Vec::new();
    for block in [&file.block_a, &file.block_b] {
        out.extend_from_slice(block.len().to_string().as_bytes());
        out.push(b'\n');
        for entry in block {
            out.extend_from_slice(entry.key.to_string().as_bytes());
            out.push(b'\n');
            out.extend_from_slice(&entry.body);
        }
    }
    out
}

pub fn compose_merged_single_file(
    vanilla: &[u8],
    block_a_additions: &[(u32, Vec<u8>)],
) -> Result<(Vec<u8>, u32), String> {
    let mut merged = parse_single_file(vanilla)?;
    let existing: std::collections::BTreeSet<u64> =
        merged.block_a.iter().map(|entry| entry.key).collect();
    let mut applied = 0u32;
    for (key, body) in block_a_additions {
        let key = *key as u64;
        if existing.contains(&key) {
            continue;
        }
        merged.block_a.push(SingleFileEntry {
            key,
            body: body.clone(),
        });
        applied += 1;
    }
    Ok((emit_single_file(&merged), applied))
}

// --------------------------------------------------------------------------------- //
// Body-length walkers (raw-bytes model: consume-and-count, no field decoding). They
// follow each block's field order exactly so `body` slices land on exact
// boundaries; the writer primitives they mirror live in `bucket_files.rs`
// (`push_pstr`, `clip_generator_data_body`, `animation_offsets_populated_body`).
// --------------------------------------------------------------------------------- //

fn read_u8(data: &[u8], pos: &mut usize) -> Result<u8, String> {
    let byte = *data
        .get(*pos)
        .ok_or_else(|| format!("unexpected EOF reading u8 at offset {pos}"))?;
    *pos += 1;
    Ok(byte)
}

fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, String> {
    let bytes = data
        .get(*pos..*pos + 4)
        .ok_or_else(|| format!("unexpected EOF reading u32 at offset {pos}"))?;
    let value = u32::from_le_bytes(bytes.try_into().unwrap());
    *pos += 4;
    Ok(value)
}

fn skip_f32(data: &[u8], pos: &mut usize) -> Result<(), String> {
    if *pos + 4 > data.len() {
        return Err(format!("unexpected EOF reading f32 at offset {pos}"));
    }
    *pos += 4;
    Ok(())
}

/// Length-prefixed, NUL-terminated Pascal string (`bucket_files::push_pstr`'s reader
/// counterpart): one u8 length = strlen+1 (NUL counted), then that many raw bytes
/// ending in a NUL. Empty string is a lone `0x00` length byte (no body, no NUL).
fn skip_pstr(data: &[u8], pos: &mut usize) -> Result<(), String> {
    let n = read_u8(data, pos)? as usize;
    if n == 0 {
        return Ok(());
    }
    let end = pos
        .checked_add(n)
        .ok_or_else(|| format!("pstr length overflow at offset {pos}"))?;
    let raw = data
        .get(*pos..end)
        .ok_or_else(|| format!("unexpected EOF reading pstr body at offset {pos}"))?;
    if raw[n - 1] != 0 {
        return Err(format!("pstr not NUL-terminated at offset {pos}"));
    }
    *pos = end;
    Ok(())
}

/// A bare-LF line matching an expected literal (used only for the `"V4"` body marker).
fn skip_expected_line(data: &[u8], pos: &mut usize, expected: &str) -> Result<(), String> {
    let start = *pos;
    let end = data[start..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|offset| start + offset)
        .ok_or_else(|| format!("unterminated line at offset {start}"))?;
    let text = std::str::from_utf8(&data[start..end])
        .map_err(|_| format!("non-ASCII line at offset {start}"))?;
    if text != expected {
        return Err(format!(
            "expected {expected:?} line, got {text:?} at offset {start}"
        ));
    }
    *pos = end + 1;
    Ok(())
}

/// Block A body length: `V4` line, `pstr(behavior_path)`,
/// `u32(clip_count)`, then per clip `pstr pstr f32x3 u8 u8 u32(trigger_count)` and per
/// trigger `pstr f32 u8`.
fn clip_generator_body_len(data: &[u8]) -> Result<usize, String> {
    let mut pos = 0usize;
    skip_expected_line(data, &mut pos, "V4")?;
    skip_pstr(data, &mut pos)?; // behavior_path
    let clip_count = read_u32(data, &mut pos)?;
    for _ in 0..clip_count {
        skip_pstr(data, &mut pos)?; // clip_name
        skip_pstr(data, &mut pos)?; // anim_name
        skip_f32(data, &mut pos)?; // playback_speed
        skip_f32(data, &mut pos)?; // crop_start
        skip_f32(data, &mut pos)?; // crop_end
        read_u8(data, &mut pos)?; // mirror (x0)
        read_u8(data, &mut pos)?; // dynamic (x1)
        let trigger_count = read_u32(data, &mut pos)?;
        for _ in 0..trigger_count {
            skip_pstr(data, &mut pos)?; // event_name
            skip_f32(data, &mut pos)?; // local_time
            read_u8(data, &mut pos)?; // flag
        }
    }
    Ok(pos)
}

/// Block B body length: `V4` line, `pstr(core_behavior)`,
/// `u32(n1)`, n1 x `(pstr, pstr)` section-1 entries, `u32(n2)`, then per section-2 entry
/// `pstr f32 u32(nTrans) trans×[f32x4] u32(nRot) rot×[f32x5] u32(nAnn) ann×[f32,pstr]`.
fn offsets_body_len(data: &[u8]) -> Result<usize, String> {
    let mut pos = 0usize;
    skip_expected_line(data, &mut pos, "V4")?;
    skip_pstr(data, &mut pos)?; // core_behavior
    let n1 = read_u32(data, &mut pos)?;
    for _ in 0..n1 {
        skip_pstr(data, &mut pos)?; // clip_name
        skip_pstr(data, &mut pos)?; // anim_path
    }
    let n2 = read_u32(data, &mut pos)?;
    for _ in 0..n2 {
        skip_pstr(data, &mut pos)?; // anim_path
        skip_f32(data, &mut pos)?; // duration
        let n_trans = read_u32(data, &mut pos)?;
        for _ in 0..n_trans {
            skip_f32(data, &mut pos)?; // time
            skip_f32(data, &mut pos)?; // X
            skip_f32(data, &mut pos)?; // Y
            skip_f32(data, &mut pos)?; // Z
        }
        let n_rot = read_u32(data, &mut pos)?;
        for _ in 0..n_rot {
            skip_f32(data, &mut pos)?; // time
            skip_f32(data, &mut pos)?; // qx
            skip_f32(data, &mut pos)?; // qy
            skip_f32(data, &mut pos)?; // qz
            skip_f32(data, &mut pos)?; // qw
        }
        let n_ann = read_u32(data, &mut pos)?;
        for _ in 0..n_ann {
            skip_f32(data, &mut pos)?; // time
            skip_pstr(data, &mut pos)?; // event_name
        }
    }
    Ok(pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn vanilla_bytes() -> Option<Vec<u8>> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../../extracted/fo4/meshes/animtextdata/\
             behaviorclipinformationandsubgraphanimationoffsetssinglefile.txt",
        );
        if !path.is_file() {
            eprintln!("skipping: oracle missing {}", path.display());
            return None;
        }
        Some(std::fs::read(path).unwrap())
    }

    #[test]
    fn vanilla_single_file_round_trips_byte_identical() {
        let Some(raw) = vanilla_bytes() else { return };
        let parsed = parse_single_file(&raw).unwrap();
        assert_eq!(parsed.block_a.len(), 122);
        assert_eq!(parsed.block_b.len(), 333);
        assert_eq!(emit_single_file(&parsed), raw);
    }

    #[test]
    fn compose_appends_new_block_a_entries_and_skips_collisions() {
        let Some(raw) = vanilla_bytes() else { return };
        let parsed = parse_single_file(&raw).unwrap();
        let colliding_key = parsed.block_a[0].key as u32;
        // A minimal valid ClipGeneratorData body: reuse the bucket writer.
        let body = crate::anim_text_data::bucket_files::clip_generator_data_body(
            r"Actors\Test\Behaviors\TestBehavior.hkx",
            &[],
        );
        let additions = vec![(colliding_key, body.clone()), (0xDEAD_BEEF_u32, body.clone())];
        let (merged, applied) = compose_merged_single_file(&raw, &additions).unwrap();
        assert_eq!(applied, 1);
        let reparsed = parse_single_file(&merged).unwrap();
        assert_eq!(reparsed.block_a.len(), 123);
        assert_eq!(reparsed.block_a.last().unwrap().key, 0xDEAD_BEEF_u64);
        assert_eq!(reparsed.block_a.last().unwrap().body, body);
        assert_eq!(reparsed.block_b.len(), 333);
        // vanilla region byte-preserved: re-emitting without the addition equals vanilla
        let (unchanged, zero) = compose_merged_single_file(&raw, &[]).unwrap();
        assert_eq!(zero, 0);
        assert_eq!(unchanged, raw);
    }
}
