use once_cell::sync::Lazy;
use regex::Regex;

static FTS5_SPECIAL: Lazy<Regex> = Lazy::new(|| Regex::new(r#"["\(\)\*\:\^\{\}]"#).unwrap());

/// Escape a user query for safe FTS5 MATCH by wrapping each word in double
/// quotes. Mirrors `creation_lib.db.db.fts5_escape`.
pub fn fts5_escape_str(query: &str) -> String {
    if query.is_empty() {
        return "\"\"".into();
    }
    let cleaned = FTS5_SPECIAL.replace_all(query, " ");
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    if words.is_empty() {
        return "\"\"".into();
    }
    let mut out = String::with_capacity(query.len() + 2 * words.len());
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push('"');
        out.push_str(w);
        out.push('"');
    }
    out
}

/// Cleaned-word list with only words of length >= `min_len`. Used by the
/// OR-fallback path.
pub fn cleaned_words(query: &str, min_len: usize) -> Vec<String> {
    FTS5_SPECIAL
        .replace_all(query, " ")
        .split_whitespace()
        .filter(|w| w.chars().count() >= min_len)
        .map(|w| w.to_string())
        .collect()
}

/// Wrap an FTS5 MATCH expression with a column-scope prefix:
///   `{col1 col2}: "foo" "bar"`
pub fn column_scope_prefix(cols: &[String]) -> String {
    let mut s = String::from("{");
    for (i, c) in cols.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(c);
    }
    s.push_str("} : ");
    s
}

pub fn fts5_escape(query: &str) -> String {
    fts5_escape_str(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query() {
        assert_eq!(fts5_escape_str(""), "\"\"");
    }

    #[test]
    fn multiword() {
        assert_eq!(fts5_escape_str("combat shotgun"), "\"combat\" \"shotgun\"");
    }

    #[test]
    fn strips_special() {
        assert_eq!(
            fts5_escape_str(r#"laser*(gun):"rifle""#),
            "\"laser\" \"gun\" \"rifle\""
        );
    }

    #[test]
    fn column_scope() {
        let p = column_scope_prefix(&["name".into(), "category".into()]);
        assert_eq!(p, "{name category} : ");
    }
}
