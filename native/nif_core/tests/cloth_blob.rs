use nif_core_native::cloth::{extract_cloth_blob, pack_cloth_blob};
use nif_core_native::model::NifFile;

#[test]
fn nif_core_owns_cloth_blob_pack_and_extract() {
    let mut nif = NifFile::new("fo4");
    let nif_bytes = nif.to_bytes().expect("blank NIF serializes");
    let blob = b"raw cloth blob owned by NIF layer";

    let packed = pack_cloth_blob(&nif_bytes, blob).expect("pack cloth blob");
    let extracted = extract_cloth_blob(&packed).expect("extract cloth blob");

    assert_eq!(extracted, blob);
}
