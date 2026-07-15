use fnv_script_native::decompile::decompile_bytecode;

#[test]
fn decompile_fails_when_unsupported() {
    let bytes = vec![0u8; 16];
    let err = decompile_bytecode(&bytes).unwrap_err();
    assert!(
        err.to_string()
            .contains("unsupported SCDA bytecode decompile")
    );
}
