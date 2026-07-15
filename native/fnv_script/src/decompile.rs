use crate::error::FnvScriptError;

pub fn decompile_bytecode(bytes: &[u8]) -> Result<String, FnvScriptError> {
    Err(FnvScriptError::Decompile(format!(
        "unsupported SCDA bytecode decompile ({} bytes)",
        bytes.len()
    )))
}
