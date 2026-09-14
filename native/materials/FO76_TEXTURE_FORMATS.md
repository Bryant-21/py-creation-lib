# Fallout 76 texture/material formats (and how we map them to Fallout 4)

Reference for the FO76→FO4 texture converter in `src/texture_convert.rs`
(`convert_fo76_to_fo4_paths` / `fo76_bundle_to_fo4_buffers`). FO76 uses a
PBR-style packed texture set that is **not** a 1:1 match for FO4's slots — in
particular **FO76 `_l` (lighting) and FO4 `_g` (glow) are different textures**,
not a rename.

## FO76 texture suffixes

- **`_d.dds`** — Diffuse / albedo. Usually `BC1_UNORM_SRGB` or `BC3_UNORM_SRGB`.
  FO4 textures are **not** flagged sRGB, which causes the gamma shift seen when
  an FO76 `_d` is dropped onto an FO4 material unconverted. In FO76 this
  texture is **black on pure-metal surfaces** (metalness lives in `_r`).
- **`_n.dds`** — Normal map, `BC5_SNORM` (2 channels). Unlike FO4 the data type
  is **signed** 8-bit; FO4 expects unsigned, so the converter remaps
  `value * 0.5 + 0.5`.
- **`_l.dds`** — Lighting texture, `BC1_UNORM` / `BC3_UNORM`, 3–4 channels.
  This is a **packed multi-channel** map, NOT an emissive map:
  - **R**: smoothness (low = rough, high = glossy)
  - **G**: ambient occlusion (applied to indirect lighting)
  - **B**: subsurface scattering (optional, only if enabled in the material)
  - **A**: grayscale emissive (optional)
- **`_r.dds`** — Reflectance at normal incidence (reference values for common
  materials), usually `BC1_UNORM_SRGB` (3ch) or `BC4_UNORM` (grayscale). Often
  just a 4×4 texture of a solid color like `#0A0A0A` (~4% reflectance in linear).

Optional maps:

- **`grad.dds`** — Grayscale-to-palette map (works like FO4's). On shader
  materials, U/V are derived from the diffuse intensity and the
  vertex-color-intensity × palette-scale parameter.
- **`_g.dds`** — RGB emissive map. (Separate, full-color emissive — distinct
  from the optional grayscale emissive that may sit in `_l`'s alpha.)
- **`_e.dds` / `_m.dds`** — Environment map and mask. Unlike FO4 these are
  normally **not** used on shader materials (unneeded with PBR + dynamic cube
  maps from Enlighten); sometimes used on effect materials like glass.

## How the FO76→FO4 converter maps channels

`fo76_bundle_to_fo4_buffers` consumes `_d` + `_r` + `_l` together and emits FO4
slots — the inputs do not map one-output-per-input:

| FO76 source | FO4 output | What happens |
|---|---|---|
| `_d` + `_r` + `_l` G(AO) | `_d` (diffuse) | metalness derived from `_r` folded into albedo, then AO term applied |
| `_r` + `_l` R(smoothness) | `_s` (specgloss) | R = specular (from reflectance/metalness), G = gloss (from `_l` R), B = 0, A = 1 |
| `_l` RGB × `_l` A(emissive mask) | `_g` (glow) | preserves the source glow color while masking transparent pixels, **only emitted when a glow output is requested** |
| `_n` | `_n` (normal) | signed→unsigned (`* 0.5 + 0.5`) |
| `_l` B(SSS) | — | not consumed on the FO4 shader path |

Because `_g` can be synthesized from `_l`'s optional alpha mask and color, an
FO76 set with an `_l` may produce an FO4 `_g` that the FO4 base game often does
not have for the same asset. That asymmetry is why base-game-dedup must compare
**per output file**, not per texture group — see `phase/textures.rs`.
