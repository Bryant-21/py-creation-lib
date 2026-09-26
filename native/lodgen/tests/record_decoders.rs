//! LAND / REFR subrecord decoders. Integration-test crate (not lib unittest)
//! because the `real-esp` lib unittest links both directxtex flavors and cannot
//! be built; the integration-test crate links cleanly.

#![cfg(feature = "real-esp")]

use lodgen_native::input::{
    decode_distant_lod, decode_refr_data, decode_refr_scale, decode_vclr, decode_vhgt_heights,
    decode_vtxt_alpha,
};

fn build_vhgt(base: f32, delta: i8) -> Vec<u8> {
    let mut v = Vec::with_capacity(1096);
    v.extend_from_slice(&base.to_le_bytes());
    v.extend(std::iter::repeat_n(delta as u8, 33 * 33));
    v.extend_from_slice(&[0u8; 3]);
    v
}

#[test]
fn land_subrecords_decode() {
    let flat = decode_vhgt_heights(&build_vhgt(10.0, 0)).unwrap();
    assert!(flat.iter().all(|v| (*v - 80.0).abs() < 1e-3));

    // Column 0 accumulates down rows, each row accumulates across columns
    // from its col-0 value (TerrainData.cs:69-86 port).
    let h = decode_vhgt_heights(&build_vhgt(0.0, 1)).unwrap();
    for (idx, want) in [(0, 8.0), (1, 16.0), (33, 16.0), (34, 24.0)] {
        assert!((h[idx] - want).abs() < 1e-3, "h[{idx}]={}", h[idx]);
    }

    let mut vclr = vec![0u8; 33 * 33 * 3];
    vclr[..3].copy_from_slice(&[10, 20, 30]);
    assert_eq!(decode_vclr(&vclr)[0], [10, 20, 30]);
    assert_eq!(decode_vclr(&[1, 2, 3])[0], [255, 255, 255]);

    let mut vtxt = Vec::new();
    vtxt.extend_from_slice(&5u16.to_le_bytes());
    vtxt.extend_from_slice(&[0, 0]);
    vtxt.extend_from_slice(&0.5f32.to_le_bytes());
    let a = decode_vtxt_alpha(&vtxt);
    assert_eq!(a.len(), 17 * 17);
    assert!((a[5] - 0.5).abs() < 1e-6);
    assert_eq!(a[0], 0.0);
}

#[test]
fn refr_subrecords_decode() {
    let mut data = Vec::new();
    for f in [10.0f32, 20.0, 30.0, 0.1, 0.2, 0.3] {
        data.extend_from_slice(&f.to_le_bytes());
    }
    let (pos, rot) = decode_refr_data(&data).unwrap();
    assert_eq!(pos, [10.0, 20.0, 30.0]);
    assert!((rot[0] - 0.1).abs() < 1e-6 && (rot[2] - 0.3).abs() < 1e-6);
    assert!(decode_refr_data(&data[..20]).is_none());

    let s = 2.5f32.to_le_bytes();
    assert_eq!(decode_refr_scale(&s).unwrap(), 2.5);
    assert!(decode_refr_scale(&s[..2]).is_none());

    const SLOT: usize = 260;
    let mut mnam = vec![0u8; SLOT * 4];
    for (idx, s) in [(0, "a_LOD.nif"), (2, "a_LOD_2.nif"), (3, "foo.dds")] {
        mnam[idx * SLOT..idx * SLOT + s.len()].copy_from_slice(s.as_bytes());
    }
    let m = decode_distant_lod(&mnam);
    assert_eq!(
        m.each_ref().map(|s| s.as_deref()),
        [Some("a_LOD.nif"), None, Some("a_LOD_2.nif"), Some("foo.dds")]
    );
    let truncated = decode_distant_lod(&mnam[..SLOT * 2]);
    assert_eq!(
        truncated.each_ref().map(|s| s.as_deref()),
        [Some("a_LOD.nif"), None, None, None]
    );
}
