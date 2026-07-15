/// Split CamelCase and underscored identifiers into space-separated tokens.
/// Matches `creation_lib.db.tokenizer.tokenize` byte-for-byte.
///
///   `WorkbenchChemistry` -> `workbench chemistry workbenchchemistry`
///   `DLC01_Weapon_Radium` -> `dlc 01 weapon radium dlc01weaponradium`
///   `HTTPSProxy` -> `https proxy httpsproxy`
pub fn tokenize_str(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut out: Vec<String> = Vec::new();
    for part in split_separators(text) {
        split_camel_into(&part, &mut out);
    }
    // Append full lowercased text with separators stripped.
    let full: String = text
        .chars()
        .filter(|c| *c != '_' && *c != '-' && *c != ' ')
        .flat_map(|c| c.to_lowercase())
        .collect();
    out.push(full);
    out.join(" ")
}

/// Split on one or more of `_`, `-`, ` `.
fn split_separators(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c == '_' || c == '-' || c == ' ' {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

/// Replicates the Python regex `[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+|[0-9]+`
/// without lookaround: emit runs of digits, caps, and lower-with-optional-leading-cap.
/// A run of caps followed by a cap+lower sequence yields two tokens where the
/// last cap starts the lower-run (matches the Python lookahead semantics).
fn split_camel_into(part: &str, out: &mut Vec<String>) {
    let chars: Vec<char> = part.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            out.push(lowercase_slice(&chars[start..i]));
        } else if c.is_ascii_uppercase() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_uppercase() {
                i += 1;
            }
            let run_len = i - start;
            let has_lower_next = i < chars.len() && chars[i].is_ascii_lowercase();
            if has_lower_next && run_len > 1 {
                // Caps run of length > 1 followed by lowercase:
                // caps[0..run_len-1] is a token, caps[run_len-1] joins the lowers.
                out.push(lowercase_slice(&chars[start..i - 1]));
                let lower_start = i - 1;
                while i < chars.len() && chars[i].is_ascii_lowercase() {
                    i += 1;
                }
                out.push(lowercase_slice(&chars[lower_start..i]));
            } else if has_lower_next {
                // Single cap followed by lowers — one token.
                while i < chars.len() && chars[i].is_ascii_lowercase() {
                    i += 1;
                }
                out.push(lowercase_slice(&chars[start..i]));
            } else {
                // All-caps run with no lower after.
                out.push(lowercase_slice(&chars[start..i]));
            }
        } else if c.is_ascii_lowercase() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_lowercase() {
                i += 1;
            }
            out.push(lowercase_slice(&chars[start..i]));
        } else {
            // Skip unrecognized punctuation or non-ascii.
            i += 1;
        }
    }
}

fn lowercase_slice(chars: &[char]) -> String {
    chars.iter().flat_map(|c| c.to_lowercase()).collect()
}

pub fn tokenize(text: &str) -> String {
    tokenize_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case() {
        assert_eq!(
            tokenize_str("WorkbenchChemistry"),
            "workbench chemistry workbenchchemistry"
        );
    }

    #[test]
    fn underscored_with_digits() {
        assert_eq!(
            tokenize_str("DLC01_Weapon_Radium"),
            "dlc 01 weapon radium dlc01weaponradium"
        );
    }

    #[test]
    fn all_caps_then_camel() {
        assert_eq!(tokenize_str("HTTPSProxy"), "https proxy httpsproxy");
    }

    #[test]
    fn digits_between() {
        assert_eq!(tokenize_str("abc123DEF"), "abc 123 def abc123def");
    }

    #[test]
    fn empty() {
        assert_eq!(tokenize_str(""), "");
    }
}
