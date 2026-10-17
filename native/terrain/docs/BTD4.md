# BTD4: dense terrain sidecar

A `.btd4` carries an FO76 worldspace's terrain at full BTD density next to the converted ESM, for
FO4CS dense terrain: heights, the dense layer weights, ground cover, vertex colours, the raw FO76
material selectors with the material bank they index, and FO76's ground-cover grass points.
There is one format. A reader rejects any file that does not match it exactly.

The writer, the reader and the shared validators are in `src/btd4.rs`; the bank and the raw patches
are in `src/raw_terrain.rs`; the bounded-memory verifier is in `src/btd4_verify.rs`. A synthetic
file with every channel is written by `cargo run -p terrain_native --example write_btd4_fixture -- <out>`.

## 1. When a file is written

- The converter writes a `.btd4` only for an FO76 identity-frame source with at least one LTEX,
  because every file needs a material bank. Starfield sources write none.
- The ESM is unchanged: LAND, LTEX, TXST and GRAS records are byte-identical with and without the
  sidecar (tests `btd4_sidecar_leaves_every_emitted_record_unchanged`,
  `btd4_grass_points_leave_every_emitted_record_unchanged`).
- The FO4 material upgrade (`material_upgrade/src/terrain_btd4.rs`) writes the same format for FO4
  worldspaces, with RAWT patches built from LAND and HGTS only to satisfy the contract.

## 2. File layout (all integers little-endian)

```
char[4] "BTD4"
u32     density                    128
f32     height_min
f32     height_scale               > 0
u16+N   worldspace EditorID        u16 byte length, then UTF-8 bytes, no terminator; non-empty
u16     plugin_count               >= 1
plugin_count x (u16+N string)      plugin names; index 0 is the converted output plugin
i32     cell_min_x, cell_min_y, cell_max_x, cell_max_y
u32     cell_count                 <= 1,000,000
u32     bank_length, then bank_length bytes: the raw material bank (§3)
cell_count x index row (120 bytes), strictly ascending by (y, x):
  i32 x, i32 y
  7 x (u64 offset, u64 length)
channel chunks
```

- Channel order is 0 HGTS, 1 ALPH, 2 LAYR, 3 GCVR, 4 CLRS, 5 RAWT, 6 GRSP.
- `offset` is absolute from the start of the file. `length` is the compressed byte count. A
  length of 0 means the channel is absent, and its offset is then 0. HGTS and RAWT are present in
  every cell.
- Each present chunk is exactly one zlib stream (RFC 1950, with its Adler-32). Reject a chunk if the
  stream is truncated, has trailing bytes after the stream end, inflates to more than the
  channel maximum, or is longer than `compressed_bound(max) = max + (max >> 12) + (max >> 14) +
  (max >> 25) + 13` bytes.

### 2.1 Channels (inflated)

| # | Tag | Size | Content |
|---|---|---|---|
| 0 | HGTS | 129*129 u16 | heights, row 0 = south; `height_min + h * height_scale` |
| 1 | ALPH | 1 + 28*4225 | u8 28, then 7 final-coverage planes (65x65 u8) per quadrant in LAYR order; base = 255 - sum |
| 2 | LAYR | 1 + 32*6 | u8 32, then 8 slots per quadrant: u8 plugin (0xFF empty), u32 LTEX object id (0 empty), u8 0 |
| 3 | GCVR | 1 + n*(5 + 16384) | u8 n, then n x (u8 plugin, u32 GRAS object id, 128x128 u8 mask) |
| 4 | CLRS | 129*129*3 | vertex colours, RGB u8 |
| 5 | RAWT | 4 patches | raw FO76 selectors, palettes and colours per quadrant (§3.2) |
| 6 | GRSP | <= 4,266,202 | FO76 ground-cover grass points (§4) |

ALPH needs LAYR. A plugin index must be below `plugin_count`.

## 3. Raw terrain

### 3.1 Material bank

```
u32 magic 'FRBK' (0x4B425246), u32 byte length (multiple of 4, <= 112 KiB), u32 count 1..=255
count x 72-byte entry, in reverse source-ordinal order (palette id n names entry n - 1):
  u32 source ordinal (count - 1 - entry index), u32 source LTEX form id (non-zero),
  u32 x 4 offsets of the _d, _n, _l, _r texture paths (each past the table),
  f32 x 4 blend row: softness (>= 0.0001), sin, cos of the rotation, repeats per world unit,
  f32 x 4 emission row: linear colour, override flag (> 0.1),
  f32 x 4 exposure row: luminous emittance, exposure offset, min, max
NUL-terminated relative `textures/...` paths, zero-padded to 4 bytes
```

Every row value is finite. Two entries can carry the same LTEX id; readers key on the ordinal.

The four maps of a material, staged as `<prefix>_source_{d,n,l,r}.dds`:

| Map | Format | Content |
|---|---|---|
| `_d` | BC3_UNORM_SRGB | albedo RGB, height in A |
| `_n` | BC5_SNORM | tangent-space normal XY |
| `_l` | BC3_UNORM | R gloss, G AO, A emissive mask |
| `_r` | BC4_UNORM, 256x256, full mips | grey F0 |

Each of `_d`, `_n` and `_l` has one size and mip count across the bank; `_r` is 256x256 for every material so one texture
array holds them (FO76 ships 4x4 and 256x256 `_r`).

### 3.2 RAWT patch

A cell's RAWT is four patches, quadrant `q` at storage origin
`(cell_x*128 + 64 + (q & 1)*64, cell_y*128 + 64 + (q >> 1)*64)`:

```
i32 x 2  storage origin
f32 x 2  native grid origin (multiples of 64, continuous across the four quadrants)
67*67 u16 selector words (one-sample border)
9 x 8 u8 palettes (3x3 64-sample quadrants around the patch; bytes 5 and 7 are 0,
         the others are bank palette ids <= count, 0 = none)
19*19 u16 RGB555 colours (every 4 samples, one-sample border)
```

## 4. GRSP: FO76 ground-cover grass points

GRSP carries the grass positions FO76's BTD grass path (`AddCellGrass` →
`GetGroundCoverParameters` → `CreateGrassPoissonDiskSample`) would generate for each cell. Those
positions do not depend on terrain height, so the converter computes them once
(`src/fo76_grass.rs`). The runtime applies only the height, water and slope tests and the
per-instance random draws (spec §5).

Algorithm reference: `mods/B21_TalesFromAppalachia/docs/re/FO76_GROUND_COVER.md` (cited below as
"spec §n"). A cell where FO76 places no grass has a zero-length GRSP entry, as does every cell when
the texture manifest has no `ground_covers`. The GRSP maximum inflated size is
`1 + 255*41 + 2 + 6144*10 + 4*1048576 = 4,266,202` bytes.

### 4.1 Payload

```
u8   grass_count                    1..=255
grass_count x 41-byte grass entry
u16  run_count                      0..=6144
run_count x run
```

There must be no bytes after the last run.

#### Grass entry (41 bytes)

| Offset | Type | Field | Meaning |
|---|---|---|---|
| 0 | u8 | plugin_index | index into the header plugin table (< plugin_count) |
| 1 | u32 | object_id | FO4 GRAS to instance, plugin-local, non-zero |
| 5 | u8 | density | raw FO76 GRAS DNAM density % (already folded into the points) |
| 6 | u8 | min_slope | degrees |
| 7 | u8 | max_slope | degrees |
| 8 | u8 | flags | FO76 GRAS flags (0x04 FitToSlope, 0x08 invert water test, 0x10 LOD blend; spec §2.1) |
| 9 | u16 | water_above | FO76 water distance A; 0xFFFF = none |
| 11 | u16 | water_below | FO76 water distance B; 0xFFFF = none |
| 13 | f32 | height_range | per-instance scale offset range |
| 17 | f32 | color_range | shader constant |
| 21 | f32 | wave_period | shader constant |
| 25 | f32 | freq_base | raw file f4 (clump-noise feature size; already applied) |
| 29 | f32 | sharp | raw file f0 (clump sharpness; already applied) |
| 33 | f32 | cover | raw file f28 (clump coverage; already applied) |
| 37 | f32 | lod_blend | raw file f32 (`+228`) |

All values are the raw FO76 DNAM file values (spec §2.1), not the converted FO4 GRAS fields. The
referenced FO4 GRAS is the record the conversion emitted for that FO76 grass. When the terrain import
replaced that record with a target-game GRAS of the same EditorID, the entry names the target record
instead (`plugin_index` > 0). This matches the GCVR channel.

The table lists grasses in order of first use by the runs below. An entry is not unique per FO76 GRAS
across cells: each cell has its own table.

#### Run (10-byte header + points)

| Offset | Type | Field | Range |
|---|---|---|---|
| 0 | u8 | block_x | 0..=15 |
| 1 | u8 | block_y | 0..=15 |
| 2 | u8 | param_index | 0..=23 |
| 3 | u8 | grass | < grass_count |
| 4 | i32 | seed_after | any |
| 8 | u16 | point_count | 1..=4096 |
| 10 | point_count x (i16 x, i16 y) | points | each coordinate -512..=4096 |

- One run is one FO76 grass param (spec §3) that produced at least one point. Runs are strictly
  ascending by `(block_x, block_y, param_index)`. That is FO76's processing order: block x outer,
  block y inner, then params in slot-then-GNAM order.
- `param_index` is FO76's `k`, the param's position in the block's list. A missing `k` is normal. Its
  param had all-zero densities, its grass has no FO4 record or FO76 DNAM, or its seeding found no
  open sample. Such params still use up their `k`.
- Block `(bx, by)` covers FO4-cell-local `[bx*256, bx*256+256) x [by*256, by*256+256)`. It is the 8x8
  BTD samples whose first sample is `(bx*8, by*8)`.
- **Point encoding.** A point is block-local, in 1/16 world unit:
  `local = (bx*256 + x/16, by*256 + y/16)` relative to the FO4 cell origin `(cell_x*4096,
  cell_y*4096)`. The world position is the cell origin plus `local`. Grown points lie strictly inside
  the block, at 0..=4096. The first point (the seeding point) is not bounds-tested by FO76 and can
  sit up to one sample (32 units) before the block, so it can be as low as -512 and its cell-local
  value can be negative. The converter rounds to nearest: `round((world - block_origin) * 16)`.
- **Point order** is FO76's Poisson list order, and it is significant (§4.2). The first point is the
  seeding point.
- **`seed_after`** is the BSRandom seed left after Poisson sampling for this param. It is
  `31*cellX + 29*cellY + 23*lx + 17*ly + 13*k` (lx = bx*8, ly = by*8, i32 wrapping arithmetic),
  plus 2 draws for every positive-density sample tried while seeding, plus 32 draws per point.
  The runtime continues drawing from it (§4.2).

### 4.2 Runtime contract (spec §5)

For each run, set `seed = seed_after`. Then, for each point in list order:

1. Build the cell-local position (§4.1). FO76 converts it to half precision relative to the cell
   origin and back, then samples terrain height `z` and normal at that position. The rounding mode of
   that conversion is not confirmed. Round to nearest-even unless RE shows otherwise.
2. **Water test.** `W` is the cell's water height, or -FLT_MAX when the cell has none.
   `above = (water_above == 0xFFFF) ? FLT_MAX : W + water_above`.
   `below = (water_below == 0xFFFF) ? FLT_MIN : W - water_below`.
   - If flag 0x08 is set, keep the point only if `below <= z <= above`.
   - If flag 0x08 is clear, keep it if `(water_above == 0xFFFF || z < W || z > above) &&
     (water_below == 0xFFFF || z < below || z > W)`.
3. **Slope test.** Keep the point if `cos(max_slope) <= normal.z <= cos(min_slope)`. FO76 reads the
   cosines from a 512-entry table indexed by `(int)(deg*512/360) & 0x1FF`.
4. **Draws.** Only kept points consume randoms. First the rotation `a = R(seed++) * 2*pi` about Z;
   with FitToSlope (0x04) the instance is aligned to the terrain normal. Then the scale offset
   `(2*R(seed++) - 1) * height_range`. A rejected point consumes nothing.
5. **Instance.** FO76 writes 16 halfs: `x, y, z, lodBlend, 9 rotation terms, scaleOffset,
   z - half(z), 0`. Slot 4 holds the LOD blend: `(1 - lod_blend)/modelBound` with flag 0x10,
   otherwise -1 [I]. FO4 puts a colour value in that slot. FO76 has no per-instance colour, so write 1.0.

`R(seed) = TBL[(u32)seed & 0x7FF]`. TBL is 2048 floats: MT19937 `init_genrand(1)`, then
`TBL[i] = (float)u32 / 2^32 * 0.99999f` for i = 0..2047. The test vectors for `TBL[0..7]` are `0.4170178,
0.9971748, 0.7203173, 0.932548, 0.0001144, 0.1281232, 0.3023295, 0.9990305`. FO4's own BSRandom table
has only 512 entries, so the runtime needs its own 2048-entry table.

### 4.3 How the converter fills it

- **Frame.** FO76 grass cells are world cells (RE-confirmed): block centre `cellX*4096 + (lx+4)*32`.
  So the seeds use the FO4 cell coordinates and FO4-cell-local sample offsets, and blocks never
  straddle cells. The inputs are the FO4-frame assembled grids (`fo4_frame::assemble_cell_grid`):
  FO4 quadrant (qx, qy) comes from BTD cell (X+qx, Y+qy) quadrant (1-qx, 1-qy).
- **Inputs.**
  - Raw LOD0 ground-cover bytes, where raw bit k = GCVR slot k. `BtdFile::cell_ground_cover_mask_u8`
    returns them bit-reversed, so the converter reverses them back.
  - Raw LTEX alpha words (`cell_raw_land_alpha_u16`).
  - Quadrant texture bytes 0,1,2,3,4,6 (`ltexIndex = ltexCount - byte`; the list ends at the first 0
    byte or at a byte above ltexCount).
  - The quadrant GCVR slots.
- **GCVR data** comes from the manifest's `ground_covers`, which covers every GCVR in the BTD table.
  - Each grass is a GNAM followed by a DNAM, in file order. A repeated GNAM keeps its first entry.
  - The LNAM LTEX set: an empty set gives zero texture weight.
  - YNAM, default 0.286, unclamped.
- **Grass data** is each GRAS entry's `fo76_dnam` (raw DNAM).
  - A GCVR grass that is not an emitted GRAS gets no points but still uses its `k`.
  - The count of such params is the operation count `btd4.grass_points.unresolved_grass_params`.
- **Order within one GCVR.** It is GNAM file order. FO76 orders by pointer hash, so results match
  FO76 statistically, not bit for bit (spec §3 caveat).
- **Precision.** sin/cos come from Rust `f32::sin_cos`, while FO76 uses `__libm_sse2_sincosf_`. A
  last-ulp difference can rarely flip a boundary or distance test.
- **Limits.**
  - A density above 100% (a GCVR u16 > 100, or a 0xFFFF entry whose GRAS density is > 100) is
    rejected at conversion. Above 1.0 the sampler's `cbrt` radius goes through zero and the point
    count explodes. FO76 data peaks at 100 (GRAS) and 50 (GCVR).
  - Caps: 4096 points per run, 1,048,576 per cell, 255 grasses per cell. Real data peaks at about
    70 points per run and 25k per cell.

## 5. Verification and size

- `Btd4Reader::open` and `verify_btd4` decode every GRSP chunk with the same validator as the writer.
  The verifier reports `cells_with_grass_points` and `grass_points_total`, and adds GRSP grass to
  `referenced_grass_ids` and `unresolved_grass_ids`.
- The real Appalachia estimate assumes every grass is emitted, which is the upper bound
  (`sample_real_fo76_grass_points`, ignored test, at strides 16 and 7):
  - Every sampled cell carries points: about 5.6k points per cell and about 225-236M points world-wide.
  - About 4.5 bytes per point after zlib, so about 1.0 GB of GRSP chunks for the whole world. The
    densest sampled cell has 25k points in 92 KB.
  - Conversion time is about 10-12 minutes in release, with blocks sampled in parallel.
