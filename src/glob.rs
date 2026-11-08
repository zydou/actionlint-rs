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

/// The error message returned by [`path_match`] when the pattern is syntactically invalid.
/// This mirrors Go's `path.ErrBadPattern`.
const ERR_BAD_PATTERN: &str = "syntax error in pattern";

/// Splits the next chunk off the pattern. A chunk is either a single leading `*` (with an empty
/// chunk) or a sequence of non-star characters. Characters inside a `[...]` class are scanned
/// verbatim, so a `*` inside a class does not terminate the chunk.
///
/// Returns whether the chunk was preceded by a star, the chunk itself, and the rest of the pattern.
fn scan_chunk(mut pattern: &[char]) -> (bool, &[char], &[char]) {
    let mut star = false;
    while !pattern.is_empty() && pattern[0] == '*' {
        pattern = &pattern[1..];
        star = true;
    }

    let mut in_range = false;
    let mut i = 0;
    while i < pattern.len() {
        match pattern[i] {
            '\\' => {
                // A backslash is scanned as a two-character unit. Whether it is valid is decided
                // later by `match_chunk`.
                if i + 1 < pattern.len() {
                    i += 1;
                }
            }
            '[' => in_range = true,
            ']' => in_range = false,
            '*' if !in_range => break,
            _ => {}
        }
        i += 1;
    }

    (star, &pattern[0..i], &pattern[i..])
}

/// Reads one escape/range endpoint out of a character class. The endpoint may not be `-`, `]`, or
/// empty, and a backslash is not a valid escape inside a class in `path.Match`.
fn get_esc(chunk: &[char]) -> Result<(char, &[char]), String> {
    if chunk.is_empty() || chunk[0] == '-' || chunk[0] == ']' {
        return Err(ERR_BAD_PATTERN.to_string());
    }
    let r = chunk[0];
    if r == '\\' {
        // Unlike `filepath.Match`, `path.Match` does not support backslash escapes in classes.
        return Err(ERR_BAD_PATTERN.to_string());
    }
    if r == '\0' {
        return Err(ERR_BAD_PATTERN.to_string());
    }
    let rest = &chunk[1..];
    if rest.is_empty() {
        return Err(ERR_BAD_PATTERN.to_string());
    }
    Ok((r, rest))
}

/// Checks whether `chunk` begins the match of `s`. On success it returns the remainder of `s`
/// (not the remainder of the chunk). Returns `Ok(None)` when the chunk does not match, and an
/// error when the chunk itself is a malformed pattern.
fn match_chunk<'a>(mut chunk: &[char], mut s: &'a [char]) -> Result<Option<&'a [char]>, String> {
    // Once the match fails we keep scanning the rest of the chunk purely to surface syntax errors.
    let mut failed = false;

    while !chunk.is_empty() {
        if !failed && s.is_empty() {
            failed = true;
        }
        match chunk[0] {
            '[' => {
                // Character class. When the match already failed we still parse the class for
                // syntax errors, but `r` stays at the NUL sentinel and `s` is left untouched.
                let mut r = '\0';
                if !failed {
                    r = s[0];
                    s = &s[1..];
                }
                chunk = &chunk[1..];

                let mut negated = false;
                if !chunk.is_empty() && chunk[0] == '^' {
                    negated = true;
                    chunk = &chunk[1..];
                }

                let mut matched = false;
                let mut n_range = 0;
                loop {
                    if !chunk.is_empty() && chunk[0] == ']' && n_range > 0 {
                        chunk = &chunk[1..];
                        break;
                    }
                    let (lo, rest) = get_esc(chunk)?;
                    chunk = rest;
                    let mut hi = lo;
                    if chunk[0] == '-' {
                        let (h, rest) = get_esc(&chunk[1..])?;
                        hi = h;
                        chunk = rest;
                    }
                    if lo <= r && r <= hi {
                        matched = true;
                    }
                    n_range += 1;
                }
                if matched == negated {
                    failed = true;
                }
            }
            '?' => {
                if !failed {
                    if s[0] == '/' {
                        failed = true;
                    }
                    // `?` matches exactly one rune, so advance by one char.
                    s = &s[1..];
                }
                chunk = &chunk[1..];
            }
            '\\' => {
                chunk = &chunk[1..];
                if chunk.is_empty() {
                    return Err(ERR_BAD_PATTERN.to_string());
                }
            }
            c => {
                if !failed {
                    if c == s[0] {
                        s = &s[1..];
                    } else {
                        failed = true;
                    }
                }
                chunk = &chunk[1..];
            }
        }
    }

    if failed { Ok(None) } else { Ok(Some(s)) }
}

/// Matches `name` against the given glob `pattern` using the semantics of Go's standard library
/// `path.Match` (the slash-aware variant, not `filepath.Match`).
///
/// Supported syntax:
/// - `?` matches any single character except `/`
/// - `*` matches any sequence of non-`/` characters (including the empty sequence)
/// - `[class]` matches any single character in the class; ranges (`a-z`) and negation (`^` as the
///   first character) are supported.
///
/// Unlike `filepath.Match`, backslash escapes are not supported: a trailing backslash, or a
/// backslash inside a character class, is a bad pattern. Also unlike the common glob convention,
/// a `]` at the start of a class or a `-` at the start/end of a class is *not* treated as a
/// literal — Go's `path.Match` reports such a class as a bad pattern.
///
/// Returns `Ok(true)`/`Ok(false)` for a (non-)match, or `Err` with a descriptive message when the
/// pattern is syntactically invalid.
pub fn path_match(pattern: &str, name: &str) -> Result<bool, String> {
    let pat_chars: Vec<char> = pattern.chars().collect();
    let name_chars: Vec<char> = name.chars().collect();
    let mut pat: &[char] = &pat_chars;
    let mut name: &[char] = &name_chars;

    'pattern: while !pat.is_empty() {
        let (star, chunk, rest) = scan_chunk(pat);
        pat = rest;

        if star && chunk.is_empty() {
            // Trailing `*` matches the rest of the string unless it contains a `/`.
            return Ok(!name.contains(&'/'));
        }

        // Look for a match at the current position. If this is the last chunk we require the whole
        // remaining name to be consumed, otherwise a star later on could still match.
        match match_chunk(chunk, name)? {
            Some(t) if t.is_empty() || !pat.is_empty() => {
                name = t;
                continue 'pattern;
            }
            _ => {}
        }

        if star {
            // Look for a match while skipping characters. A `/` cannot be skipped.
            for i in 0..name.len() {
                if name[i] == '/' {
                    break;
                }
                if let Some(t) = match_chunk(chunk, &name[i + 1..])? {
                    // If we are the last chunk, make sure we exhausted the name.
                    if pat.is_empty() && !t.is_empty() {
                        continue;
                    }
                    name = t;
                    continue 'pattern;
                }
            }
        }

        // Before returning no-match, validate the remainder of the pattern so that a syntactically
        // invalid pattern still reports an error.
        while !pat.is_empty() {
            let (_, chunk, rest) = scan_chunk(pat);
            pat = rest;
            match_chunk(chunk, &[])?;
        }
        return Ok(false);
    }

    Ok(name.is_empty())
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

    // ---- path_match (port of Go's path.Match) ----

    /// Asserts `path_match` yields the expected boolean result (no error).
    fn assert_match(pat: &str, name: &str, expected: bool) {
        assert_eq!(
            path_match(pat, name),
            Ok(expected),
            "path_match({pat:?}, {name:?}) should be {expected}"
        );
    }

    /// Asserts `path_match` reports a bad pattern.
    fn assert_bad_pattern(pat: &str, name: &str) {
        let r = path_match(pat, name);
        assert!(
            r.is_err(),
            "path_match({pat:?}, {name:?}) should be a bad pattern"
        );
    }

    #[test]
    fn path_match_exact() {
        assert_match("", "", true);
        assert_match("abc", "abc", true);
        assert_match("a/b/c", "a/b/c", true);
        assert_match("abc", "abd", false);
        assert_match("abc", "ab", false);
        assert_match("abc", "", false);
        assert_match("", "a", false);
    }

    #[test]
    fn path_match_star() {
        // `*` matches any sequence of non-`/` characters, including the empty sequence.
        assert_match("*", "", true);
        assert_match("*", "a", true);
        assert_match("*", "abc", true);
        // `*` never matches across a `/`.
        assert_match("*", "a/b", false);
        assert_match("*/", "a/", true);
        assert_match("*/", "a/b", false);
        // A trailing `*` matches the rest only if there is no `/`.
        assert_match("a/*", "a/", true);
        assert_match("a/*", "a/b", true);
        assert_match("a/*", "a/b/c", false);
        assert_match("a/*/c", "a/b/c", true);
        assert_match("a/*/c", "a/b/b/c", false);
        assert_match("*/x", "a/x", true);
        assert_match("*/x", "a/b/x", false);
        assert_match("a*b", "ab", true);
        assert_match("a*b", "axxxb", true);
        assert_match("a*b", "a/b", false);
        assert_match("a*b*c", "abc", true);
        assert_match("a*b*c", "axxbxxc", true);
        assert_match("*/*", "a/b", true);
        assert_match("*/*", "a/b/c", false);
    }

    #[test]
    fn path_match_question() {
        // `?` matches exactly one character that is not `/`.
        assert_match("?", "a", true);
        assert_match("?", "/", false);
        assert_match("?", "", false);
        assert_match("a?c", "abc", true);
        assert_match("a?c", "a/c", false);
        assert_match("a?c", "ac", false);
        assert_match("a?c", "abbc", false);
    }

    #[test]
    fn path_match_class() {
        assert_match("[a]", "a", true);
        assert_match("[a]", "b", false);
        assert_match("[abc]", "b", true);
        assert_match("[abc]", "d", false);
        // Ranges.
        assert_match("[a-c]", "b", true);
        assert_match("[a-c]", "d", false);
        assert_match("[0-9]", "5", true);
        assert_match("[0-9]", "x", false);
        // Negation with leading `^`.
        assert_match("[^a-c]", "d", true);
        assert_match("[^a-c]", "b", false);
        assert_match("[^/]", "x", true);
        assert_match("[^/]", "/", false);
        // Class combined with stars.
        assert_match("[a-c]*", "bXYZ", true);
        assert_match("[a-c]*", "d", false);
        // A class explicitly containing `/` does match it.
        assert_match("[/]", "/", true);
    }

    #[test]
    fn path_match_class_multi_char() {
        // A class consumes exactly one character of the name.
        assert_match("[a-c][a-c]", "ab", true);
        assert_match("[a-c][a-c]", "abc", false);
    }

    #[test]
    fn path_match_bad_patterns() {
        // Unterminated character class.
        assert_bad_pattern("[", "a");
        assert_bad_pattern("[a", "a");
        assert_bad_pattern("[a-", "a");
        assert_bad_pattern("[^", "a");
        // `]` as the only content is invalid because a class must be non-empty.
        assert_bad_pattern("[]", "a");
        // A `]` at the start of a class is not a literal in `path.Match`; it is a bad pattern.
        assert_bad_pattern("[]a]", "a");
        assert_bad_pattern("[]]", "]");
        // A `-` at the start or end of a class is not a literal in `path.Match`; it is a bad
        // pattern.
        assert_bad_pattern("[-a]", "a");
        assert_bad_pattern("[a-]", "a");
        // Trailing backslash.
        assert_bad_pattern("\\", "a");
        assert_bad_pattern("a\\", "a");
        // Backslash inside a class is not a valid escape in `path.Match`.
        assert_bad_pattern("[a\\", "a");
        assert_bad_pattern("[\\]", "a");
    }

    #[test]
    fn path_match_backslash_outside_class() {
        // Outside a class, a backslash simply escapes the next literal character (no error), so
        // `a\b` behaves like the literal `ab`.
        assert_match("a\\b", "ab", true);
        assert_match("a\\b", "aXb", false);
    }

    #[test]
    fn path_match_non_ascii() {
        // `?` and `*` operate on runes, not bytes.
        assert_match("?", "é", true);
        assert_match("*", "é", true);
        assert_match("?", "éé", false);
        assert_match("[à-ÿ]", "é", true);
        assert_match("[à-ÿ]", "a", false);
        assert_match("é?", "éà", true);
    }

    #[test]
    fn path_match_error_message_is_descriptive() {
        let err = path_match("[a", "a").unwrap_err();
        assert!(!err.is_empty());
        assert!(err.contains("pattern"));
    }
}
