// .lod byte-exact gate: lodsettings::encode must produce the exact 16-byte LE layout.

use lodgen_native::output::lodsettings;

#[test]
fn lod_bytes_exact_for_known_bounds() {
    // DLC03FarHarbor-like bounds: SW (-41,-27), span -> stride 64
    let bytes = lodsettings::encode((-41, -27), 64, 4, 32);
    let mut expected = Vec::new();
    expected.extend_from_slice(&(-41i16).to_le_bytes());
    expected.extend_from_slice(&(-27i16).to_le_bytes());
    expected.extend_from_slice(&64i32.to_le_bytes());
    expected.extend_from_slice(&4i32.to_le_bytes());
    expected.extend_from_slice(&32i32.to_le_bytes());
    assert_eq!(bytes.to_vec(), expected);
}

#[test]
fn lod_bytes_exact_for_commonwealth_like_bounds() {
    // Commonwealth-like: SW (-56,-56), stride 128, lod4..32
    let bytes = lodsettings::encode((-56, -56), 128, 4, 32);
    assert_eq!(bytes.len(), 16);
    assert_eq!(&bytes[0..2], &(-56i16).to_le_bytes());
    assert_eq!(&bytes[2..4], &(-56i16).to_le_bytes());
    assert_eq!(&bytes[4..8], &128i32.to_le_bytes());
    assert_eq!(&bytes[8..12], &4i32.to_le_bytes());
    assert_eq!(&bytes[12..16], &32i32.to_le_bytes());
}
