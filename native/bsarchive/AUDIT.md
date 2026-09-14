# Format coverage spec

Format coverage for the project-owned `py_creation_lib/native/bsarchive/` fork, which was
reset from an internal reference snapshot.

## Format coverage matrix

| Format | Games | Read | Write | Notes |
|---|---|---|---|---|
| TES4 BSA v103 | Oblivion | yes | yes (`Version::v103`) | Zlib. |
| TES4 BSA v104 | FO3/FNV/TES5 | yes | yes (`Version::v104`) | Zlib; embedded filenames flag honored (`ARCHIVE_EMBEDNAME`). |
| TES4 BSA v105 | Skyrim SE | yes | yes (`Version::v105`) | LZ4 frame via `lzzzz::lz4f`. |
| FO4 BA2 GNRL v1 | Fallout 4 old-gen | yes | yes (`fo4og`) | Zlib. Crate uses `Compression::default()` (≈level 6). |
| FO4 BA2 DX10 v1 | Fallout 4 old-gen | yes | yes (`fo4ogdds`) | DX10 mip chunk splitting implemented in `file.rs::make_chunks` (≤4 chunks, default 512×512 pitch). |
| FO4 BA2 GNRL v8 | FO4 next-gen | yes | yes (`fo4`) | Header layout identical to v1; version literal differs. |
| FO4 BA2 DX10 v8 | FO4 next-gen | yes | yes (`fo4dds`) | Header layout identical to v1; version literal differs. |
| Starfield BA2 GNRL v2 | Starfield | yes | yes | Adds 8-byte sentinel after file count (see `archive.rs` line 370). Zlib default. |
| Starfield BA2 DX10 v3 | Starfield | yes | yes | Adds sentinel + compression-format discriminator (0=Zip, 3=LZ4). Crate uses `lz4_flex` raw block codec for the LZ4 path. |
| FO4 BA2 GNMF (Sony GNF) | PS4 content | partial | no (NotImplemented) | `read_dx10` handles ascii DDS input; `read_gnmf` returns `Error::NotImplemented`. Not needed for PC mod workflow. |

## Native archive parity features to check in the fork

Cross-referenced against xEdit archive behavior.

1. **Per-call zlib compression level override.** Implemented in the fork for FO4/Starfield chunk compression and TES4 file compression via the native pack wrapper.
2. **Packed-data MD5 deduplication.** Legacy archive packing keys compressed payloads by `(size, hash)`; when two files compress to identical bytes, they share disk offset. Crate writes every chunk/file verbatim. -> **patch required** (optimization; not required for game-load correctness).
3. **"Compression must save ≥32 bytes" rule.** Implemented in the native pack wrapper for TES4 files and FO4/Starfield chunks.
4. **DDS-archive compression enforcement.** Implemented in the native pack wrapper for FO4/FO76/Starfield texture archives.
5. **TES4 file/archive flag inference.** Implemented in the native pack wrapper for the project-supported TES4-family outputs.
6. **FO4 file-order preservation.** Legacy archive packing stores FO4 files in caller's submission order. `ba2::fo4::Archive` is a `BTreeMap` keyed by `ArchiveKey`, so output order is hash-sorted. Byte-exact output versus legacy tools is a non-goal, but the listing order diverges. -> **accept divergence**; validation compares file sets, not ordered sequences.

## Format-coverage gaps

None blocking the project-supported scope. TES3 has been intentionally dropped from the ModBox21 wrapper. The supported matrix is:

- BSA v103 / v104 / v105 — ✓
- FO4 GNRL v1 / v7 / v8 — ✓
- FO4 DX10 v1 / v2 / v3 / v7 / v8 — ✓
- Starfield GNRL v2 / v3 + LZ4 frame — ✓ (v3 DX10 uses `lz4_flex` block; matches `baSFdds → ctLZ4Block`)
- BSA v104/v105 `ARCHIVE_EMBEDNAME` edge cases — ✓ (read side in the crate; write side via the TES4 flag inference above)