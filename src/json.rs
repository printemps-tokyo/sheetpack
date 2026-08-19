//! The small amount of JSON writing sheetpack needs.
//!
//! Manifests are written, never generically parsed -- `--check` only pulls a
//! couple of known string fields back out of a file this tool wrote itself
//! (see `check.rs`). So this is a handful of helpers rather than `serde` plus
//! `serde_json` in the dependency tree.

use std::fmt::Write as _;

/// Escape a string and wrap it in double quotes, ready to splice into output.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            // Control characters have no short escape; the \u form is required.
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `",\n"` between array elements and `"\n"` after the last one.
pub fn sep(index: usize, len: usize) -> &'static str {
    if index + 1 < len {
        ",\n"
    } else {
        "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_escapes_the_json_specials() {
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(quote("a\nb\tc\r"), r#""a\nb\tc\r""#);
        assert_eq!(quote("\u{1}"), "\"\\u0001\"");
        assert_eq!(quote("sprites/走る.png"), "\"sprites/走る.png\"");
    }

    #[test]
    fn separators() {
        assert_eq!(sep(0, 2), ",\n");
        assert_eq!(sep(1, 2), "\n");
    }
}
