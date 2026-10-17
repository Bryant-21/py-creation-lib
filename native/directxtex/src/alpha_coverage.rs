/// Rescales the alpha of every mip below the top so the share of texels at or above
/// `reference` matches the top level's. A box filter turns a thin cutout into wide partial
/// alpha, so an alpha-tested card fills in solid a few mips down and flickers as the sampled
/// mip changes. A 1-bit source leaves only a few alpha levels in the first mips, so texels
/// on the boundary level pass or fail by a fixed ordered dither to land on the target.
/// Colour is untouched.
pub fn preserve_alpha_coverage(chain: &mut [(u32, u32, Vec<u8>)], reference: u8) {
    let Some((_, _, top)) = chain.first() else {
        return;
    };
    let reference = reference.max(1);
    let total_top = (top.len() / 4).max(1) as f32;
    let target = histogram(top)[usize::from(reference)..].iter().sum::<u32>() as f32 / total_top;
    for (width, _, pixels) in chain.iter_mut().skip(1) {
        let counts = histogram(pixels);
        let total = (pixels.len() / 4).max(1) as f32;
        let wanted = (target * total).round() as u32;
        if counts[usize::from(reference)..].iter().sum::<u32>() == wanted {
            continue;
        }
        // The boundary level: every texel above it passes, a share of those on it pass.
        let mut above = 0u32;
        let mut boundary = 0usize;
        for level in (1..=255usize).rev() {
            if above + counts[level] >= wanted {
                boundary = level;
                break;
            }
            above += counts[level];
        }
        if boundary == 0 {
            for px in pixels.chunks_exact_mut(4) {
                px[3] = if px[3] > 0 { reference } else { 0 };
            }
            continue;
        }
        let on_boundary = counts[boundary].max(1) as f32;
        let pass_share = (wanted - above) as f32 / on_boundary;
        let scale = f32::from(reference) / boundary as f32;
        let width = (*width).max(1) as usize;
        for (i, px) in pixels.chunks_exact_mut(4).enumerate() {
            let alpha = usize::from(px[3]);
            px[3] = if alpha > boundary {
                ((alpha as f32 * scale).round() as u32).clamp(u32::from(reference), 255) as u8
            } else if alpha == boundary && bayer4(i % width, i / width) < pass_share {
                reference
            } else {
                ((alpha as f32 * scale).round() as u32).min(u32::from(reference) - 1) as u8
            };
        }
    }
}

/// The mip chain for an alpha-tested cutout: colour flooded under the hidden texels so the
/// card doesn't darken as it shrinks, every level holding the top level's coverage at
/// `reference`, and alpha written as 0/255 because BC3's eight alpha steps would move texels
/// across the threshold. Encode it as BC3; BC1 decodes clear texels as black.
pub fn rgba8_cutout_mip_chain(
    width: u32,
    height: u32,
    rgba: &[u8],
    reference: u8,
) -> Result<Vec<(u32, u32, Vec<u8>)>, String> {
    let mut chain = crate::rgba8_mip_flood_chain(width, height, rgba)?;
    preserve_alpha_coverage(&mut chain, reference);
    for (_, _, pixels) in chain.iter_mut() {
        for px in pixels.chunks_exact_mut(4) {
            px[3] = if px[3] >= reference { 255 } else { 0 };
        }
    }
    Ok(chain)
}

fn bayer4(x: usize, y: usize) -> f32 {
    const M: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    (f32::from(M[y % 4][x % 4]) + 0.5) / 16.0
}

fn histogram(pixels: &[u8]) -> [u32; 256] {
    let mut counts = [0u32; 256];
    for px in pixels.chunks_exact(4) {
        counts[usize::from(px[3])] += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage(pixels: &[u8], reference: u8) -> f32 {
        let texels = pixels.len() / 4;
        pixels.chunks_exact(4).filter(|px| px[3] >= reference).count() as f32 / texels as f32
    }

    /// A 1-bit cutout like the shipped branch atlases: thin opaque strokes on a clear field,
    /// so a box filter spreads partial alpha over most of every lower mip.
    fn branch_mip0(size: u32) -> Vec<u8> {
        let mut rgba = vec![0u8; (size * size * 4) as usize];
        for y in 0..size {
            for x in 0..size {
                let opaque = x % 5 == 0 || (x + 2 * y) % 11 == 0;
                if opaque {
                    let i = ((y * size + x) * 4) as usize;
                    rgba[i..i + 4].copy_from_slice(&[90, 60, 30, 255]);
                }
            }
        }
        rgba
    }

    fn box_chain(size: u32, mip0: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
        crate::rgba8_box_mip_chain(size, size, mip0).unwrap()
    }

    #[test]
    fn every_mip_keeps_the_top_level_coverage_at_the_reference() {
        let mip0 = branch_mip0(64);
        let mut chain = box_chain(64, &mip0);
        let target = coverage(&mip0, 128);
        preserve_alpha_coverage(&mut chain, 128);

        for (w, h, pixels) in chain.iter().filter(|(w, _, _)| *w >= 4) {
            let got = coverage(pixels, 128);
            assert!((got - target).abs() <= 0.08, "{w}x{h}: coverage {got} vs top {target}");
        }
    }

    #[test]
    fn a_plain_box_chain_is_the_defect_this_fixes() {
        let mip0 = branch_mip0(64);
        let chain = box_chain(64, &mip0);
        let target = coverage(&mip0, 128);
        let (_, _, eight) = chain.iter().find(|(w, _, _)| *w == 8).unwrap();
        assert!((coverage(eight, 128) - target).abs() > 0.15);
    }

    #[test]
    fn the_top_level_and_colour_are_untouched() {
        let mip0 = branch_mip0(32);
        let mut chain = box_chain(32, &mip0);
        let before = chain.clone();
        preserve_alpha_coverage(&mut chain, 128);

        assert_eq!(chain[0], before[0]);
        for ((_, _, after), (_, _, original)) in chain.iter().zip(before.iter()) {
            for (a, o) in after.chunks_exact(4).zip(original.chunks_exact(4)) {
                assert_eq!(a[..3], o[..3]);
            }
        }
    }

    #[test]
    fn opaque_and_clear_textures_are_left_alone() {
        for value in [0u8, 255] {
            let mip0 = vec![value; 16 * 16 * 4];
            let mut chain = box_chain(16, &mip0);
            let before = chain.clone();
            preserve_alpha_coverage(&mut chain, 128);
            assert_eq!(chain, before);
        }
    }
}
