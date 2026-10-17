use serde_json::Value;
use sha2::{Digest, Sha256};

fn quoted(value: &str, ascii: bool) -> String {
    let mut output = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{8}' => output.push_str("\\b"),
            '\u{c}' => output.push_str("\\f"),
            c if c < ' ' || (ascii && c > '~') => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    output.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => output.push(c),
        }
    }
    output.push('"');
    output
}

pub fn encode(value: &Value, ascii: bool, spaced: bool) -> String {
    let comma = if spaced { ", " } else { "," };
    let colon = if spaced { ": " } else { ":" };
    match value {
        Value::String(value) => quoted(value, ascii),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(|v| encode(v, ascii, spaced))
                .collect::<Vec<_>>()
                .join(comma)
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(k, v)| format!("{}{colon}{}", quoted(k, ascii), encode(v, ascii, spaced)))
                .collect::<Vec<_>>()
                .join(comma)
        ),
        value => value.to_string(),
    }
}

pub fn digest(value: &Value, ascii: bool, spaced: bool) -> String {
    format!(
        "{:x}",
        Sha256::digest(encode(value, ascii, spaced).as_bytes())
    )
}

pub fn parse(value: &str) -> pyo3::PyResult<Value> {
    serde_json::from_str(value).map_err(|error| crate::invalid(error.to_string()))
}
