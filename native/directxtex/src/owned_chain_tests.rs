use super::*;
use std::sync::Arc;

fn pixels() -> Rgba8MipChain {
    rgba8_box_mip_chain(8, 4, &(0..128).map(|i| (i * 13) as u8).collect::<Vec<_>>()).unwrap()
}

#[test]
fn owned_chain_preserves_formats_headers_and_cpu_fallback() {
    for format in [
        "BC1_UNORM",
        "BC1_UNORM_SRGB",
        "BC3_UNORM",
        "BC3_UNORM_SRGB",
        "BC4_UNORM",
        "BC5_UNORM",
        "BC7_UNORM",
        "BC7_UNORM_SRGB",
        "R8G8B8A8_UNORM",
        "R8G8B8A8_UNORM_SRGB",
    ] {
        let chain = pixels();
        let expected = encode_dds_from_rgba8_chain(&chain, format, false, None).unwrap();
        assert_eq!(
            encode_dds_from_owned_rgba8_chain(chain.clone(), format, false, None).unwrap(),
            expected
        );
        let failure = |_: Arc<Rgba8MipChain>, _: bool| Err("fixture GPU failure".to_string());
        assert_eq!(
            encode_dds_from_owned_rgba8_chain(chain.clone(), format, false, Some(&failure))
                .unwrap(),
            expected
        );
        let incomplete = |_: Arc<Rgba8MipChain>, _: bool| Ok(Vec::new());
        assert_eq!(
            encode_dds_from_owned_rgba8_chain(chain, format, false, Some(&incomplete)).unwrap(),
            expected
        );
    }
}

#[test]
fn owned_submission_keeps_original_pixel_allocations() {
    let chain = pixels();
    let addresses: Vec<usize> = chain.iter().map(|(_, _, p)| p.as_ptr() as usize).collect();
    let reference = encode_dds_from_rgba8_chain(&chain, "BC7_UNORM", false, None).unwrap();
    let encode = move |shared: Arc<Rgba8MipChain>, srgb: bool| {
        assert!(!srgb);
        assert_eq!(
            shared
                .iter()
                .map(|(_, _, p)| p.as_ptr() as usize)
                .collect::<Vec<_>>(),
            addresses
        );
        shared
            .iter()
            .map(|(w, h, pixels)| {
                compressed_payload_from_rgba(
                    *w as usize,
                    *h as usize,
                    pixels,
                    DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM,
                    false,
                    false,
                )
            })
            .collect()
    };
    assert_eq!(
        encode_dds_from_owned_rgba8_chain(chain, "BC7_UNORM", false, Some(&encode)).unwrap(),
        reference
    );
}

#[test]
fn owned_chain_retains_input_validation() {
    assert!(encode_dds_from_owned_rgba8_chain(Vec::new(), "BC7_UNORM", false, None).is_err());
    assert!(
        encode_dds_from_owned_rgba8_chain(vec![(2, 2, vec![0; 3])], "BC7_UNORM", false, None)
            .is_err()
    );
}
