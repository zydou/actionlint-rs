//! Validation of glob patterns used in workflow filters.
//!
//! Rust counterpart of the original `glob.go`. It validates filter patterns for Git ref names
//! (branches, tags) and file paths according to the GitHub Actions syntax.

use std::fmt;

/// An error on an invalid glob pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidGlobPattern {
    /// A human readable error message.
    pub message: String,
    /// The 1-based column number of the error in the pattern (0 means before the first char).
    pub column: usize,
}

impl fmt::Display for InvalidGlobPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.column, self.message)
    }
}

struct GlobValidator {
    is_ref: bool,
    prec: bool,
    errs: Vec<InvalidGlobPattern>,
    /// The pattern as a char vector with the current index.
    chars: Vec<char>,
    idx: usize,
}

impl GlobValidator {
    fn error(&mut self, msg: String) {
        // Character at the error position is already consumed, so column is idx (0-based -> use as is).
        let c = if self.line_gt1() { 0 } else { self.idx };
        self.errs.push(InvalidGlobPattern {
            message: msg,
            column: c,
        });
    }

    fn line_gt1(&self) -> bool {
        self.chars.contains(&'\n')
    }

    fn unexpected(&mut self, ch: Option<char>, what: &str, why: &str) {
        let unexpected = match ch {
            None => "unexpected EOF".to_string(),
            Some(c) => format!("unexpected character {c:?}"),
        };
        let while_ = if what.is_empty() {
            String::new()
        } else {
            format!(" while checking {what}")
        };
        self.error(format!("invalid glob pattern. {unexpected}{while_}. {why}"));
    }

    fn invalid_ref_char(&mut self, c: char, why: &str) {
        let cfmt = if c.is_control() {
            format!("{c:?}")
        } else {
            format!("'{c}'")
        };
        self.error(format!(
            "character {cfmt} is invalid for branch and tag names. {why}. see `man git-check-ref-format` for more details. note that regular expression is unavailable"
        ));
    }

    /// Peeks the current character without consuming.
    fn peek(&self) -> Option<char> {
        self.chars.get(self.idx).copied()
    }

    /// Consumes and returns the current character.
    fn next(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.idx += 1;
        }
        c
    }

    fn validate_next(&mut self) -> bool {
        let Some(mut c) = self.next() else {
            return false;
        };
        let mut prec = true;

        match c {
            '\\' => match self.peek() {
                Some('[' | '?' | '*') => {
                    c = self.next().unwrap();
                    if self.is_ref
                        && let Some(n) = self.peek()
                    {
                        self.invalid_ref_char(
                            n,
                            "ref name cannot contain spaces, ~, ^, :, [, ?, *",
                        );
                    }
                }
                Some('+' | '\\' | '!') => {
                    c = self.next().unwrap();
                }
                _ => {
                    if self.is_ref {
                        self.invalid_ref_char(
                            '\\',
                            "only special characters [, ?, +, *, \\, ! can be escaped with \\",
                        );
                        c = self.next().unwrap_or(c);
                    }
                }
            },
            '?' => {
                if !self.prec {
                    self.unexpected(
                        Some('?'),
                        "special character ? (zero or one)",
                        "the preceding character must not be special character",
                    );
                }
                prec = false;
            }
            '+' => {
                if !self.prec {
                    self.unexpected(
                        Some('+'),
                        "special character + (one or more)",
                        "the preceding character must not be special character",
                    );
                }
                prec = false;
            }
            '*' => {
                prec = false;
            }
            '[' => self.validate_char_match(),
            '\r' => {
                if self.peek() == Some('\n') {
                    c = self.next().unwrap();
                }
                self.unexpected(Some(c), "", "newline cannot be contained");
            }
            '\n' => {
                self.unexpected(Some('\n'), "", "newline cannot be contained");
            }
            ' ' | '\t' | '~' | '^' | ':' if self.is_ref => {
                self.invalid_ref_char(c, "ref name cannot contain spaces, ~, ^, :, [, ?, *");
            }
            _ => {}
        }
        self.prec = prec;

        if self.peek().is_none() {
            if self.is_ref && (c == '/' || c == '.') {
                self.invalid_ref_char(c, "ref name must not end with / and .");
            }
            return false;
        }

        true
    }

    fn validate_char_match(&mut self) {
        if self.peek() == Some(']') {
            let c = self.next().unwrap();
            self.unexpected(
                Some(c),
                "content of character match []",
                "character match must not be empty",
            );
            return;
        }

        let mut chars = 0;
        loop {
            let Some(mut c) = self.next() else {
                self.unexpected(None, "end of character match []", "missing ]");
                return;
            };
            if c == ']' {
                break;
            } else {
                if self.peek() != Some('-') {
                    chars += 1;
                    continue;
                }
                // Range of characters like 0-9.
                chars += 2;
                let s = c;
                c = self.next().unwrap_or(c); // eat '-'
                match self.peek() {
                    Some(']') => {
                        c = self.next().unwrap();
                        self.unexpected(
                            Some(c),
                            "character range in []",
                            "end of range is missing",
                        );
                        break;
                    }
                    None => {}
                    _ => {
                        c = self.next().unwrap_or(c); // eat end of range
                        if s > c {
                            self.unexpected(
                                Some(c),
                                "character range in []",
                                &format!(
                                    "start of range {s:?} ({}) is larger than end of range {c:?} ({})",
                                    s as u32, c as u32
                                ),
                            );
                        }
                    }
                }
            }
        }

        if chars == 1 {
            self.unexpected(
                None,
                "character match []",
                "character match with single character is useless. simply use x instead of [x]",
            );
        }
    }

    fn validate(&mut self, pat: &str) {
        self.errs.clear();
        self.prec = false;
        self.chars = pat.chars().collect();
        self.idx = 0;

        if pat.is_empty() {
            self.error("glob pattern cannot be empty".to_string());
            return;
        }

        // Handle first character.
        match self.peek() {
            Some('/') => {
                if self.is_ref {
                    self.next();
                    self.invalid_ref_char('/', "ref name must not start with /");
                    self.prec = true;
                }
            }
            Some('!') => {
                self.next();
                if self.peek().is_none() {
                    self.unexpected(
                        Some('!'),
                        "! at first character (negate pattern)",
                        "at least one character must follow !",
                    );
                    return;
                }
                self.prec = false;
            }
            _ => {}
        }

        while self.validate_next() {}
    }
}

fn validate_glob(pat: &str, is_ref: bool) -> Vec<InvalidGlobPattern> {
    let mut v = GlobValidator {
        is_ref,
        prec: false,
        errs: Vec::new(),
        chars: Vec::new(),
        idx: 0,
    };
    v.validate(pat);
    v.errs
}

/// Checks a pattern as a glob for Git ref names (branches, tags).
#[must_use]
pub fn validate_ref_glob(pat: &str) -> Vec<InvalidGlobPattern> {
    validate_glob(pat, true)
}

/// Checks a pattern as a glob for file paths.
#[must_use]
pub fn validate_path_glob(pat: &str) -> Vec<InvalidGlobPattern> {
    let p = pat.trim();
    let mut errs = Vec::new();
    if pat != p {
        errs.push(InvalidGlobPattern {
            message: "leading and trailing spaces are not allowed in glob path".to_string(),
            column: 0,
        });
    }

    // '.' is not handled by path filter.
    let p = p.strip_prefix('!').unwrap_or(p);
    if p == "." || p == ".." || p.starts_with("./") || p.starts_with("../") {
        errs.push(InvalidGlobPattern {
            message: "'.' and '..' are not allowed in glob path".to_string(),
            column: 0,
        });
    }

    if !errs.is_empty() {
        return errs;
    }

    validate_glob(pat, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_ref_glob() {
        assert!(validate_ref_glob("refs/heads/main").is_empty());
        assert!(validate_ref_glob("feature/*").is_empty());
        assert!(validate_ref_glob("v[0-9]+").is_empty());
    }

    #[test]
    fn empty_pattern() {
        let errs = validate_ref_glob("");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].message, "glob pattern cannot be empty");
    }

    #[test]
    fn leading_slash_ref() {
        let errs = validate_ref_glob("/foo");
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("must not start with /"));
    }

    #[test]
    fn space_in_ref() {
        let errs = validate_ref_glob("foo bar");
        assert!(!errs.is_empty());
        assert!(
            errs.iter()
                .any(|e| e.message.contains("invalid for branch and tag names"))
        );
    }

    #[test]
    fn missing_bracket() {
        let errs = validate_path_glob("[0-9");
        assert!(!errs.is_empty());
        assert!(errs.iter().any(|e| e.message.contains("missing ]")));
    }

    #[test]
    fn path_leading_space() {
        let errs = validate_path_glob(" foo ");
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("leading and trailing spaces"));
    }

    #[test]
    fn path_dot() {
        let errs = validate_path_glob("./foo");
        assert!(!errs.is_empty());
        assert!(
            errs.iter()
                .any(|e| e.message.contains("'.' and '..' are not allowed"))
        );
    }

    #[test]
    fn valid_path_glob() {
        assert!(validate_path_glob("path/to/file.txt").is_empty());
        assert!(validate_path_glob("**/*.yml").is_empty());
    }

    #[test]
    fn single_char_match() {
        let errs = validate_path_glob("[x]");
        assert!(!errs.is_empty());
        assert!(errs.iter().any(|e| e.message.contains("single character")));
    }
}
