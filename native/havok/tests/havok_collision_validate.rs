// Blob-level smoke for the report-only collision validator.

#[test]
fn parse_error_is_returned_not_panicked() {
    let result = havok_native::api::validate_collision_blob(b"NOT_A_HAVOK_BLOB", "{}");
    assert!(
        result.is_err(),
        "garbage input should error, not panic or pass"
    );
}
