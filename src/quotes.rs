//! Helpers for formatting quoted, comma-separated lists of strings.
//!
//! Rust counterpart of the original `quotes.go`. These produce Go-style quoted strings (using
//! [`escape_default`]-like quoting) joined by `, ` and are used in error messages that list
//! available values.

/// Returns the string quoted Go-style (double quotes with escapes), matching `strconv.Quote`.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        // Go's strconv.Quote escapes these control characters and the quote/backslash.
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x07' => out.push_str("\\a"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\x0b' => out.push_str("\\v"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Returns the strings each quoted Go-style and joined by `, `.
///
/// Counterpart of the Go `quotes` function.
pub fn quotes<S: AsRef<str>>(ss: &[S]) -> String {
    ss.iter()
        .map(|s| quote(s.as_ref()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Returns the strings sorted and then formatted like [`quotes`].
///
/// Counterpart of the Go `sortedQuotes` function.
pub fn sorted_quotes<S: AsRef<str>>(mut ss: Vec<S>) -> String {
    ss.sort_by(|a, b| a.as_ref().cmp(b.as_ref()));
    quotes(&ss)
}

/// Returns all the strings from the given lists formatted like [`quotes`] and joined by `, `.
///
/// Counterpart of the Go `quotesAll` variadic function. Each list is appended in order.
pub fn quotes_all<S: AsRef<str>>(sss: &[&[S]]) -> String {
    let mut out = String::new();
    let mut comma = false;
    for ss in sss {
        for s in *ss {
            if comma {
                out.push_str(", ");
            } else {
                comma = true;
            }
            out.push_str(&quote(s.as_ref()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        assert_eq!(quotes::<&str>(&[]), "");
    }

    #[test]
    fn single() {
        assert_eq!(quotes(&["foo"]), "\"foo\"");
    }

    #[test]
    fn multiple() {
        assert_eq!(quotes(&["a", "b", "c"]), "\"a\", \"b\", \"c\"");
    }

    #[test]
    fn escapes() {
        assert_eq!(quotes(&["a\"b"]), "\"a\\\"b\"");
        assert_eq!(quotes(&["a\nb"]), "\"a\\nb\"");
    }

    #[test]
    fn sorted() {
        assert_eq!(sorted_quotes(vec!["c", "a", "b"]), "\"a\", \"b\", \"c\"");
    }
}
