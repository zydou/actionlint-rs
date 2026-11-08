//! Lexer for GitHub Actions expression syntax.
//!
//! Rust counterpart of the original `expr_lexer.go`. It converts a `${{ ... }}` expression source
//! into a sequence of [`Token`] values. See
//! <https://docs.github.com/en/actions/learn-github-actions/expressions>.

use std::fmt;

use crate::expr::ExprError;

/// The kind of a lexed token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A default value representing an unknown token.
    Unknown,
    /// The end of the token sequence. A sequence without this token is invalid.
    End,
    /// An identifier.
    Ident,
    /// A string literal.
    String,
    /// An integer (including hex integers).
    Int,
    /// A float number.
    Float,
    /// The `(` token.
    LeftParen,
    /// The `)` token.
    RightParen,
    /// The `[` token.
    LeftBracket,
    /// The `]` token.
    RightBracket,
    /// The `.` token.
    Dot,
    /// The `!` token.
    Not,
    /// The `<` token.
    Less,
    /// The `<=` token.
    LessEq,
    /// The `>` token.
    Greater,
    /// The `>=` token.
    GreaterEq,
    /// The `==` token.
    Eq,
    /// The `!=` token.
    NotEq,
    /// The `&&` token.
    And,
    /// The `||` token.
    Or,
    /// The `*` token.
    Star,
    /// The `,` token.
    Comma,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TokenKind::Unknown => "UNKNOWN",
            TokenKind::End => "END",
            TokenKind::Ident => "IDENT",
            TokenKind::String => "STRING",
            TokenKind::Int => "INTEGER",
            TokenKind::Float => "FLOAT",
            TokenKind::LeftParen => "(",
            TokenKind::RightParen => ")",
            TokenKind::LeftBracket => "[",
            TokenKind::RightBracket => "]",
            TokenKind::Dot => ".",
            TokenKind::Not => "!",
            TokenKind::Less => "<",
            TokenKind::LessEq => "<=",
            TokenKind::Greater => ">",
            TokenKind::GreaterEq => ">=",
            TokenKind::Eq => "==",
            TokenKind::NotEq => "!=",
            TokenKind::And => "&&",
            TokenKind::Or => "||",
            TokenKind::Star => "*",
            TokenKind::Comma => ",",
        };
        f.write_str(s)
    }
}

/// A token lexed from expression syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The kind of the token.
    pub kind: TokenKind,
    /// The string representation of the token.
    pub value: String,
    /// The byte offset of the start of the token string.
    pub offset: usize,
    /// The 1-based line number of the start position of the token.
    pub line: usize,
    /// The 1-based column number of the start position of the token.
    pub column: usize,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}:{}",
            self.kind, self.line, self.column, self.offset
        )
    }
}

fn is_whitespace(r: char) -> bool {
    matches!(r, ' ' | '\n' | '\r' | '\t')
}

fn is_alpha(r: char) -> bool {
    r.is_ascii_alphabetic()
}

fn is_num(r: char) -> bool {
    r.is_ascii_digit()
}

fn is_hex_num(r: char) -> bool {
    r.is_ascii_hexdigit()
}

fn is_alnum(r: char) -> bool {
    r.is_ascii_alphanumeric()
}

const EXPECTED_PUNCT_CHARS: &str =
    "''', '}', '(', ')', '[', ']', '.', '!', '<', '>', '=', '&', '|', '*', ',', ' '";
const EXPECTED_DIGIT_CHARS: &str = "'0'..'9'";
const EXPECTED_ALL_CHARS: &str = concat!(
    "'a'..'z', 'A'..'Z', '_', ",
    "'0'..'9', ",
    "''', '}', '(', ')', '[', ']', '.', '!', '<', '>', '=', '&', '|', '*', ',', ' '"
);

/// A lexer for expression syntax.
#[derive(Debug)]
pub struct ExprLexer {
    src: Vec<char>,
    /// Byte offsets of each char index, for accurate offset reporting.
    offsets: Vec<usize>,
    idx: usize,
    start: usize,
    line: usize,
    column: usize,
    start_line: usize,
    start_column: usize,
    lex_err: Option<ExprError>,
}

impl ExprLexer {
    /// Creates a new lexer for the given expression source.
    #[must_use]
    pub fn new(src: &str) -> Self {
        let mut offsets = Vec::with_capacity(src.len() + 1);
        let mut off = 0usize;
        for c in src.chars() {
            offsets.push(off);
            off += c.len_utf8();
        }
        offsets.push(off);
        Self {
            src: src.chars().collect(),
            offsets,
            idx: 0,
            start: 0,
            line: 1,
            column: 1,
            start_line: 1,
            start_column: 1,
            lex_err: None,
        }
    }

    fn error(&mut self, msg: String) {
        if self.lex_err.is_none() {
            self.lex_err = Some(ExprError {
                message: msg,
                offset: self.offset(),
                line: self.line,
                column: self.column,
            });
        }
    }

    /// Returns the current byte offset (scanning position).
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offsets[self.idx.min(self.offsets.len() - 1)]
    }

    fn peek(&self) -> Option<char> {
        self.src.get(self.idx).copied()
    }

    fn token(&mut self, kind: TokenKind) -> Token {
        let end = self.idx.min(self.src.len());
        let value: String = self.src[self.start..end].iter().collect();
        let t = Token {
            kind,
            value,
            offset: self.offsets[self.start.min(self.offsets.len() - 1)],
            line: self.start_line,
            column: self.start_column,
        };
        self.start = self.idx;
        self.start_line = self.line;
        self.start_column = self.column;
        t
    }

    fn eof(&self) -> Token {
        Token {
            kind: TokenKind::End,
            value: String::new(),
            offset: self.offsets[self.start.min(self.offsets.len() - 1)],
            line: self.start_line,
            column: self.start_column,
        }
    }

    /// Consumes the current char, advancing line/column, and returns the next char (peek after
    /// eating).
    fn eat(&mut self) -> Option<char> {
        if let Some(c) = self.src.get(self.idx) {
            let c = *c;
            if c == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
            self.idx += 1;
        }
        self.peek()
    }

    fn skip_white(&mut self) {
        while let Some(r) = self.peek() {
            if !is_whitespace(r) {
                return;
            }
            self.eat();
            self.start = self.idx;
            self.start_line = self.line;
            self.start_column = self.column;
        }
    }

    fn unexpected(&mut self, r: Option<char>, where_: &str, expected: &str) -> Token {
        let what = match r {
            None => "EOF".to_string(),
            Some(c) => format!("character {c:?}"),
        };
        let note = if r == Some('"') {
            ". do you mean string literals? only single quotes are available for string delimiter"
        } else {
            ""
        };
        self.error(format!(
            "got unexpected {what} while lexing {where_}, expecting {expected}{note}"
        ));
        self.eof()
    }

    fn unexpected_eof(&mut self) -> Token {
        self.error("unexpected EOF while lexing expression".to_string());
        self.eof()
    }

    fn lex_ident(&mut self) -> Token {
        loop {
            match self.eat() {
                Some(r) if is_alnum(r) || r == '_' || r == '-' => {}
                _ => return self.token(TokenKind::Ident),
            }
        }
    }

    fn lex_num(&mut self) -> Token {
        let mut r = self.peek().unwrap();

        if r == '-' {
            r = self.eat().unwrap_or(r);
        }

        if r == '0' {
            r = self.eat().unwrap_or(r);
            if r == 'x' {
                self.eat();
                return self.lex_hex_int();
            }
        } else {
            if !is_num(r) {
                return self.unexpected(Some(r), "integer part of number", EXPECTED_DIGIT_CHARS);
            }
            loop {
                r = self.eat().unwrap_or(r);
                if !is_num(r) {
                    break;
                }
            }
        }

        let mut kind = TokenKind::Int;

        if r == '.' {
            r = self.eat().unwrap_or(r);
            if !is_num(r) {
                return self.unexpected(
                    Some(r),
                    "fraction part of float number",
                    EXPECTED_DIGIT_CHARS,
                );
            }
            loop {
                r = self.eat().unwrap_or(r);
                if !is_num(r) {
                    break;
                }
            }
            kind = TokenKind::Float;
        }

        if r == 'e' || r == 'E' {
            r = self.eat().unwrap_or(r);
            if r == '-' {
                r = self.eat().unwrap_or(r);
            }
            if r == '0' {
                r = self.eat().unwrap_or(r);
            } else {
                if !is_num(r) {
                    return self.unexpected(
                        Some(r),
                        "exponent part of float number",
                        EXPECTED_DIGIT_CHARS,
                    );
                }
                loop {
                    r = self.eat().unwrap_or(r);
                    if !is_num(r) {
                        break;
                    }
                }
            }
            kind = TokenKind::Float;
        }

        if is_alnum(r) {
            let s: String = self.src[self.start..self.idx].iter().collect();
            return self.unexpected(
                Some(r),
                &format!("character following number {s}"),
                EXPECTED_PUNCT_CHARS,
            );
        }

        self.token(kind)
    }

    fn lex_hex_int(&mut self) -> Token {
        let mut r = self.peek().unwrap();

        if r == '0' {
            r = self.eat().unwrap_or(r);
        } else {
            if !is_hex_num(r) {
                let e = "'0'..'9', 'a'..'f', 'A'..'F'";
                return self.unexpected(Some(r), "hex integer", e);
            }
            loop {
                r = self.eat().unwrap_or(r);
                if !is_hex_num(r) {
                    break;
                }
            }
        }

        // GitHub Actions does not support exponent part like 0x1f2p-a8.
        if is_alnum(r) {
            let s: String = self.src[self.start..self.idx].iter().collect();
            return self.unexpected(
                Some(r),
                &format!("character following hex integer {s}"),
                EXPECTED_PUNCT_CHARS,
            );
        }

        self.token(TokenKind::Int)
    }

    fn lex_string(&mut self) -> Token {
        loop {
            match self.eat() {
                Some('\'') if self.eat() != Some('\'') => {
                    return self.token(TokenKind::String);
                }
                None => {
                    return self.unexpected(None, "end of string literal", "'''");
                }
                _ => {}
            }
        }
    }

    fn lex_end(&mut self) -> Token {
        let r = self.eat(); // eat the first '}'
        if r != Some('}') {
            return self.unexpected(r, "end marker }}", "'}'");
        }
        self.eat(); // eat the second '}'
        self.token(TokenKind::End)
    }

    fn lex_less(&mut self) -> Token {
        let mut kind = TokenKind::Less;
        if self.eat() == Some('=') {
            kind = TokenKind::LessEq;
            self.eat();
        }
        self.token(kind)
    }

    fn lex_greater(&mut self) -> Token {
        let mut kind = TokenKind::Greater;
        if self.eat() == Some('=') {
            kind = TokenKind::GreaterEq;
            self.eat();
        }
        self.token(kind)
    }

    fn lex_eq(&mut self) -> Token {
        let r = self.eat();
        if r != Some('=') {
            return self.unexpected(r, "== operator", "'='");
        }
        self.eat();
        self.token(TokenKind::Eq)
    }

    fn lex_bang(&mut self) -> Token {
        let mut kind = TokenKind::Not;
        if self.eat() == Some('=') {
            self.eat();
            kind = TokenKind::NotEq;
        }
        self.token(kind)
    }

    fn lex_and(&mut self) -> Token {
        let r = self.eat();
        if r != Some('&') {
            return self.unexpected(r, "&& operator", "'&'");
        }
        self.eat();
        self.token(TokenKind::And)
    }

    fn lex_or(&mut self) -> Token {
        let r = self.eat();
        if r != Some('|') {
            return self.unexpected(r, "|| operator", "'|'");
        }
        self.eat();
        self.token(TokenKind::Or)
    }

    fn lex_char(&mut self, kind: TokenKind) -> Token {
        self.eat();
        self.token(kind)
    }

    /// Lexes the next token incrementally.
    pub fn next_token(&mut self) -> Token {
        self.skip_white();

        let r = match self.peek() {
            Some(c) => c,
            None => return self.unexpected_eof(),
        };

        if is_alpha(r) || r == '_' {
            return self.lex_ident();
        }
        if is_num(r) || r == '-' {
            return self.lex_num();
        }

        match r {
            '\'' => self.lex_string(),
            '}' => self.lex_end(),
            '!' => self.lex_bang(),
            '<' => self.lex_less(),
            '>' => self.lex_greater(),
            '=' => self.lex_eq(),
            '&' => self.lex_and(),
            '|' => self.lex_or(),
            '(' => self.lex_char(TokenKind::LeftParen),
            ')' => self.lex_char(TokenKind::RightParen),
            '[' => self.lex_char(TokenKind::LeftBracket),
            ']' => self.lex_char(TokenKind::RightBracket),
            '.' => self.lex_char(TokenKind::Dot),
            '*' => self.lex_char(TokenKind::Star),
            ',' => self.lex_char(TokenKind::Comma),
            _ => self.unexpected(Some(r), "expression", EXPECTED_ALL_CHARS),
        }
    }

    /// Returns an error encountered while lexing, if any.
    #[must_use]
    pub fn err(&self) -> Option<&ExprError> {
        self.lex_err.as_ref()
    }
}

/// Lexes the given string as expression syntax. The source must contain `}}` which marks the end of
/// the expression; otherwise an unexpected-EOF error is reported.
///
/// Returns the tokens, the final byte offset, and an optional error.
#[must_use]
pub fn lex_expression(src: &str) -> (Option<Vec<Token>>, usize, Option<ExprError>) {
    let mut l = ExprLexer::new(src);
    let mut ts = Vec::new();
    loop {
        let t = l.next_token();
        if let Some(e) = &l.lex_err {
            let off = l.offset();
            let e = e.clone();
            let _ = t;
            return (None, off, Some(e));
        }
        let end = t.kind == TokenKind::End;
        ts.push(t);
        if end {
            return (Some(ts), l.offset(), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        let (ts, _, _) = lex_expression(src);
        ts.unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lex_simple_ident() {
        // The lexer receives the inner expression source (no `${{` prefix) ending with `}}`.
        assert_eq!(kinds("foo }}"), vec![TokenKind::Ident, TokenKind::End]);
    }

    #[test]
    fn lex_dotted() {
        let (ts, _, _) = lex_expression("github.event.name }}");
        let ts = ts.unwrap();
        assert_eq!(
            ts.iter().map(|t| t.kind).collect::<Vec<_>>(),
            vec![
                TokenKind::Ident,
                TokenKind::Dot,
                TokenKind::Ident,
                TokenKind::Dot,
                TokenKind::Ident,
                TokenKind::End,
            ]
        );
        assert_eq!(ts[0].value, "github");
        assert_eq!(ts[0].line, 1);
        assert_eq!(ts[0].column, 1);
    }

    #[test]
    fn lex_string_literal() {
        let (ts, _, _) = lex_expression("'hello' }}");
        let ts = ts.unwrap();
        assert_eq!(ts[0].kind, TokenKind::String);
        assert_eq!(ts[0].value, "'hello'");
    }

    #[test]
    fn lex_escaped_quote() {
        let (ts, _, _) = lex_expression("'it''s' }}");
        let ts = ts.unwrap();
        assert_eq!(ts[0].kind, TokenKind::String);
        assert_eq!(ts[0].value, "'it''s'");
    }

    #[test]
    fn lex_operators() {
        assert_eq!(
            kinds("a == b }}"),
            vec![
                TokenKind::Ident,
                TokenKind::Eq,
                TokenKind::Ident,
                TokenKind::End
            ]
        );
        assert_eq!(
            kinds("a != b }}"),
            vec![
                TokenKind::Ident,
                TokenKind::NotEq,
                TokenKind::Ident,
                TokenKind::End
            ]
        );
        assert_eq!(
            kinds("a && b || c }}"),
            vec![
                TokenKind::Ident,
                TokenKind::And,
                TokenKind::Ident,
                TokenKind::Or,
                TokenKind::Ident,
                TokenKind::End,
            ]
        );
    }

    #[test]
    fn lex_numbers() {
        let (ts, _, _) = lex_expression("42 }}");
        assert_eq!(ts.unwrap()[0].kind, TokenKind::Int);
        let (ts, _, _) = lex_expression("3.14 }}");
        assert_eq!(ts.unwrap()[0].kind, TokenKind::Float);
        let (ts, _, _) = lex_expression("0x1f }}");
        assert_eq!(ts.unwrap()[0].kind, TokenKind::Int);
        let (ts, _, _) = lex_expression("-5 }}");
        assert_eq!(ts.unwrap()[0].kind, TokenKind::Int);
    }

    #[test]
    fn lex_hex_value() {
        let (ts, _, _) = lex_expression("0xff }}");
        assert_eq!(ts.unwrap()[0].value, "0xff");
    }

    #[test]
    fn lex_function_call() {
        assert_eq!(
            kinds("contains(a, 'x') }}"),
            vec![
                TokenKind::Ident,
                TokenKind::LeftParen,
                TokenKind::Ident,
                TokenKind::Comma,
                TokenKind::String,
                TokenKind::RightParen,
                TokenKind::End,
            ]
        );
    }

    #[test]
    fn lex_array_deref() {
        assert_eq!(
            kinds("a.b.* }}"),
            vec![
                TokenKind::Ident,
                TokenKind::Dot,
                TokenKind::Ident,
                TokenKind::Dot,
                TokenKind::Star,
                TokenKind::End,
            ]
        );
    }

    #[test]
    fn lex_index_access() {
        assert_eq!(
            kinds("a[0] }}"),
            vec![
                TokenKind::Ident,
                TokenKind::LeftBracket,
                TokenKind::Int,
                TokenKind::RightBracket,
                TokenKind::End,
            ]
        );
    }

    #[test]
    fn lex_error_unterminated_string() {
        let (_, _, err) = lex_expression("'abc }}");
        assert!(err.is_some());
    }

    #[test]
    fn lex_error_double_quote() {
        let (_, _, err) = lex_expression("\"abc\" }}");
        assert!(err.is_some());
        assert!(err.unwrap().message.contains("only single quotes"));
    }

    #[test]
    fn token_display() {
        let t = Token {
            kind: TokenKind::Ident,
            value: "foo".into(),
            offset: 0,
            line: 1,
            column: 1,
        };
        assert_eq!(t.to_string(), "IDENT:1:1:0");
    }
}
