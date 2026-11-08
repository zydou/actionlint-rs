//! Parser for GitHub Actions expression syntax.
//!
//! Rust counterpart of the original `expr_parser.go`. It parses a token sequence produced by the
//! [`ExprLexer`](crate::expr_lexer::ExprLexer) into an [`ExprNode`] syntax tree. See
//! <https://docs.github.com/en/actions/learn-github-actions/expressions>.

use std::rc::Rc;

use crate::expr::ExprError;
use crate::expr_ast::{
    ArrayDerefNode, BoolNode, CompareOpKind, CompareOpNode, ExprNode, ExprNodeKind, FloatNode,
    FuncCallNode, IndexAccessNode, IntNode, LogicalOpKind, LogicalOpNode, NotOpNode, NullNode,
    ObjectDerefNode, StringNode, VariableNode,
};
use crate::expr_lexer::{ExprLexer, Token, TokenKind};

/// A parser for expression syntax.
pub struct ExprParser {
    cur: Token,
    lexer: ExprLexer,
    err: Option<ExprError>,
}

fn error_at_token(t: &Token, msg: String) -> ExprError {
    ExprError {
        message: msg,
        offset: t.offset,
        line: t.line,
        column: t.column,
    }
}

impl ExprParser {
    /// Creates a new parser.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cur: Token {
                kind: TokenKind::Unknown,
                value: String::new(),
                offset: 0,
                line: 0,
                column: 0,
            },
            lexer: ExprLexer::new(""),
            err: None,
        }
    }

    fn error(&mut self, msg: String) {
        if self.err.is_none() {
            self.err = Some(error_at_token(&self.cur, msg));
        }
    }

    fn unexpected(&mut self, where_: &str, expected: &[TokenKind]) {
        if self.err.is_some() {
            return;
        }
        let quoted: Vec<String> = expected.iter().map(|e| format!("'{e}'")).collect();
        let what = if self.cur.kind == TokenKind::End {
            "end of input".to_string()
        } else {
            format!("token {:?}", self.cur.kind.to_string())
        };
        let msg = format!(
            "unexpected {} while parsing {}. expecting {}",
            what,
            where_,
            quoted.join(", ")
        );
        self.error(msg);
    }

    fn next(&mut self) -> Token {
        let ret = self.cur.clone();
        self.cur = self.lexer.next_token();
        ret
    }

    fn peek(&self) -> &Token {
        &self.cur
    }

    fn parse_ident(&mut self) -> Option<ExprNode> {
        let ident = self.next(); // eat ident
        match self.peek().kind {
            TokenKind::LeftParen => {
                // Parse function call as a primary expression: only builtin function calls are
                // allowed, so the callee is always a builtin function name.
                self.next(); // eat '('
                let mut args = Vec::new();
                if self.peek().kind == TokenKind::RightParen {
                    self.next(); // eat ')'
                } else {
                    loop {
                        let arg = self.parse_logical_or()?;
                        args.push(arg);
                        match self.peek().kind {
                            TokenKind::Comma => {
                                self.next(); // eat ','
                            }
                            TokenKind::RightParen => {
                                self.next(); // eat ')'
                                break;
                            }
                            _ => {
                                self.unexpected(
                                    "arguments of function call",
                                    &[TokenKind::Comma, TokenKind::RightParen],
                                );
                                return None;
                            }
                        }
                    }
                }
                Some(Rc::new(ExprNodeKind::FuncCall(FuncCallNode {
                    callee: ident.value.clone(),
                    args,
                    tok: ident,
                })))
            }
            _ => {
                // Keywords are case sensitive; TRUE, FALSE, NULL are invalid named values.
                match ident.value.as_str() {
                    "null" => Some(Rc::new(ExprNodeKind::Null(NullNode { tok: ident }))),
                    "true" => Some(Rc::new(ExprNodeKind::Bool(BoolNode {
                        value: true,
                        tok: ident,
                    }))),
                    "false" => Some(Rc::new(ExprNodeKind::Bool(BoolNode {
                        value: false,
                        tok: ident,
                    }))),
                    // Variable name access is case insensitive.
                    _ => Some(Rc::new(ExprNodeKind::Variable(VariableNode {
                        name: ident.value.to_lowercase(),
                        tok: ident,
                    }))),
                }
            }
        }
    }

    fn parse_nested_expr(&mut self) -> Option<ExprNode> {
        self.next(); // eat '('
        let nested = self.parse_logical_or()?;
        if self.peek().kind == TokenKind::RightParen {
            self.next(); // eat ')'
        } else {
            self.unexpected(
                "closing ')' of nested expression (...)",
                &[TokenKind::RightParen],
            );
            return None;
        }
        Some(nested)
    }

    fn parse_int(&mut self) -> Option<ExprNode> {
        let t = self.peek().clone();
        let v = t.value.trim_start_matches('-');
        let parsed = if let Some(hex) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
            i64::from_str_radix(hex, 16).ok()
        } else {
            t.value.parse::<i64>().ok()
        };
        let Some(i) = parsed else {
            self.error(format!(
                "parsing invalid integer literal {:?}: invalid value",
                t.value
            ));
            return None;
        };
        self.next(); // eat int
        Some(Rc::new(ExprNodeKind::Int(IntNode { value: i, tok: t })))
    }

    fn parse_float(&mut self) -> Option<ExprNode> {
        let t = self.peek().clone();
        let Ok(f) = t.value.parse::<f64>() else {
            self.error(format!(
                "parsing invalid float literal {:?}: invalid value",
                t.value
            ));
            return None;
        };
        self.next(); // eat float
        Some(Rc::new(ExprNodeKind::Float(FloatNode { value: f, tok: t })))
    }

    fn parse_string(&mut self) -> Option<ExprNode> {
        let t = self.next(); // eat string
        let s = &t.value[1..t.value.len() - 1]; // strip first and last single quotes
        let s = s.replace("''", "'"); // unescape ''
        Some(Rc::new(ExprNodeKind::String(StringNode {
            value: s,
            tok: t,
        })))
    }

    fn parse_primary_expr(&mut self) -> Option<ExprNode> {
        match self.peek().kind {
            TokenKind::Ident => self.parse_ident(),
            TokenKind::LeftParen => self.parse_nested_expr(),
            TokenKind::Int => self.parse_int(),
            TokenKind::Float => self.parse_float(),
            TokenKind::String => self.parse_string(),
            _ => {
                self.unexpected(
                    "variable access, function call, null, bool, int, float or string",
                    &[
                        TokenKind::Ident,
                        TokenKind::LeftParen,
                        TokenKind::Int,
                        TokenKind::Float,
                        TokenKind::String,
                    ],
                );
                None
            }
        }
    }

    fn parse_postfix_op(&mut self) -> Option<ExprNode> {
        let mut ret = self.parse_primary_expr()?;

        loop {
            match self.peek().kind {
                TokenKind::Dot => {
                    self.next(); // eat '.'
                    match self.peek().kind {
                        TokenKind::Star => {
                            self.next(); // eat '*'
                            ret = Rc::new(ExprNodeKind::ArrayDeref(ArrayDerefNode {
                                receiver: Some(ret),
                            }));
                        }
                        TokenKind::Ident => {
                            let t = self.next(); // eat 'b' of 'a.b'
                            // Property name is case insensitive.
                            ret = Rc::new(ExprNodeKind::ObjectDeref(ObjectDerefNode {
                                receiver: Some(ret),
                                property: t.value.to_lowercase(),
                            }));
                        }
                        _ => {
                            self.unexpected(
                                "object property dereference like 'a.b' or array element dereference like 'a.*'",
                                &[TokenKind::Ident, TokenKind::Star],
                            );
                            return None;
                        }
                    }
                }
                TokenKind::LeftBracket => {
                    self.next(); // eat '['
                    let idx = self.parse_logical_or()?;
                    ret = Rc::new(ExprNodeKind::IndexAccess(IndexAccessNode {
                        operand: Some(ret),
                        index: Some(idx),
                    }));
                    if self.peek().kind != TokenKind::RightBracket {
                        self.unexpected(
                            "closing bracket ']' for index access",
                            &[TokenKind::RightBracket],
                        );
                        return None;
                    }
                    self.next(); // eat ']'
                }
                _ => return Some(ret),
            }
        }
    }

    fn parse_prefix_op(&mut self) -> Option<ExprNode> {
        let t = self.peek().clone();
        if t.kind != TokenKind::Not {
            return self.parse_postfix_op();
        }
        self.next(); // eat '!' token
        let o = self.parse_prefix_op()?;
        Some(Rc::new(ExprNodeKind::NotOp(NotOpNode {
            operand: Some(o),
            tok: t,
        })))
    }

    fn parse_compare_bin_op(&mut self) -> Option<ExprNode> {
        let l = self.parse_prefix_op()?;

        let kind = match self.peek().kind {
            TokenKind::Less => CompareOpKind::Less,
            TokenKind::LessEq => CompareOpKind::LessEq,
            TokenKind::Greater => CompareOpKind::Greater,
            TokenKind::GreaterEq => CompareOpKind::GreaterEq,
            TokenKind::Eq => CompareOpKind::Eq,
            TokenKind::NotEq => CompareOpKind::NotEq,
            _ => return Some(l),
        };
        self.next(); // eat the operator token
        let r = self.parse_compare_bin_op()?;
        Some(Rc::new(ExprNodeKind::CompareOp(CompareOpNode {
            kind,
            left: Some(l),
            right: Some(r),
        })))
    }

    fn parse_logical_and(&mut self) -> Option<ExprNode> {
        let l = self.parse_compare_bin_op()?;
        if self.peek().kind != TokenKind::And {
            return Some(l);
        }
        self.next(); // eat &&
        let r = self.parse_logical_and()?;
        Some(Rc::new(ExprNodeKind::LogicalOp(LogicalOpNode {
            kind: LogicalOpKind::And,
            left: Some(l),
            right: Some(r),
        })))
    }

    fn parse_logical_or(&mut self) -> Option<ExprNode> {
        let l = self.parse_logical_and()?;
        if self.peek().kind != TokenKind::Or {
            return Some(l);
        }
        self.next(); // eat ||
        let r = self.parse_logical_or()?;
        Some(Rc::new(ExprNodeKind::LogicalOp(LogicalOpNode {
            kind: LogicalOpKind::Or,
            left: Some(l),
            right: Some(r),
        })))
    }

    /// Returns an error caused during the previous parsing.
    #[must_use]
    pub fn err(&self) -> Option<&ExprError> {
        if let Some(e) = self.lexer.err() {
            return Some(e);
        }
        self.err.as_ref()
    }

    /// Parses the token sequence from the given lexer into a syntax tree.
    pub fn parse(mut self, lexer: ExprLexer) -> Result<ExprNode, ExprError> {
        self.err = None;
        self.lexer = lexer;
        self.cur = self.lexer.next_token();

        let root = match self.parse_logical_or() {
            Some(r) => r,
            None => {
                return Err(self.err().cloned().unwrap_or_else(|| ExprError {
                    message: "parse error".to_string(),
                    offset: 0,
                    line: 1,
                    column: 1,
                }));
            }
        };
        if let Some(e) = self.err().cloned() {
            return Err(e);
        }

        if self.peek().kind != TokenKind::End {
            // It did not reach the end of the sequence.
            let mut kinds = vec![self.peek().kind.to_string()];
            let mut c = 1;
            loop {
                let t = self.lexer.next_token();
                if t.kind == TokenKind::End {
                    break;
                }
                kinds.push(t.kind.to_string());
                c += 1;
            }
            let msg = format!(
                "parser did not reach end of input after parsing the expression. {c} remaining token(s) in the input: {}",
                kinds.join(", ")
            );
            self.error(msg);
            return Err(self.err.clone().unwrap());
        }

        Ok(root)
    }
}

impl Default for ExprParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Parses the given expression source into a syntax tree.
///
/// The source is the inner expression (without the `${{` prefix) and must contain `}}` marking the
/// end of the expression. Returns the parsed node and the byte offset of the end, or an error.
pub fn parse_expression(src: &str) -> Result<(ExprNode, usize), ExprError> {
    let lexer = ExprLexer::new(src);
    let node = ExprParser::new().parse(lexer)?;
    Ok((node, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The parser receives the inner expression source (no `${{` prefix) ending with `}}`.
    fn parse_ok(src: &str) -> ExprNode {
        parse_expression(src).unwrap().0
    }

    #[test]
    fn parse_variable() {
        let n = parse_ok("github }}");
        match n.as_ref() {
            ExprNodeKind::Variable(v) => assert_eq!(v.name, "github"),
            _ => panic!("expected variable"),
        }
    }

    #[test]
    fn parse_variable_case_insensitive() {
        let n = parse_ok("GITHUB }}");
        match n.as_ref() {
            ExprNodeKind::Variable(v) => assert_eq!(v.name, "github"),
            _ => panic!("expected variable"),
        }
    }

    #[test]
    fn parse_property_case_insensitive() {
        let n = parse_ok("github.EVENT.NAME }}");
        match n.as_ref() {
            ExprNodeKind::ObjectDeref(d) => assert_eq!(d.property, "name"),
            _ => panic!("expected object deref"),
        }
    }

    #[test]
    fn parse_object_deref() {
        let n = parse_ok("github.event.name }}");
        match n.as_ref() {
            ExprNodeKind::ObjectDeref(d) => {
                assert_eq!(d.property, "name");
                match d.receiver.as_ref().unwrap().as_ref() {
                    ExprNodeKind::ObjectDeref(d2) => assert_eq!(d2.property, "event"),
                    _ => panic!("expected nested deref"),
                }
            }
            _ => panic!("expected object deref"),
        }
    }

    #[test]
    fn parse_literals() {
        match parse_ok("null }}").as_ref() {
            ExprNodeKind::Null(_) => {}
            _ => panic!("expected null"),
        }
        match parse_ok("true }}").as_ref() {
            ExprNodeKind::Bool(b) => assert!(b.value),
            _ => panic!("expected bool"),
        }
        match parse_ok("false }}").as_ref() {
            ExprNodeKind::Bool(b) => assert!(!b.value),
            _ => panic!("expected bool"),
        }
        match parse_ok("42 }}").as_ref() {
            ExprNodeKind::Int(i) => assert_eq!(i.value, 42),
            _ => panic!("expected int"),
        }
        match parse_ok("3.5 }}").as_ref() {
            ExprNodeKind::Float(f) => assert!((f.value - 3.5).abs() < 1e-9),
            _ => panic!("expected float"),
        }
        match parse_ok("'hi' }}").as_ref() {
            ExprNodeKind::String(s) => assert_eq!(s.value, "hi"),
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn parse_keywords_case_sensitive() {
        // TRUE is not a keyword, it is a variable.
        let n = parse_ok("TRUE }}");
        match n.as_ref() {
            ExprNodeKind::Variable(v) => assert_eq!(v.name, "true"),
            _ => panic!("expected variable"),
        }
    }

    #[test]
    fn parse_string_escape() {
        let n = parse_ok("'it''s' }}");
        match n.as_ref() {
            ExprNodeKind::String(s) => assert_eq!(s.value, "it's"),
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn parse_function_call() {
        let n = parse_ok("contains(github.event.head_commit.message, 'fix') }}");
        match n.as_ref() {
            ExprNodeKind::FuncCall(f) => {
                assert_eq!(f.callee, "contains");
                assert_eq!(f.args.len(), 2);
            }
            _ => panic!("expected function call"),
        }
    }

    #[test]
    fn parse_array_deref() {
        let n = parse_ok("github.event.commits.*.message }}");
        match n.as_ref() {
            ExprNodeKind::ObjectDeref(d) => {
                assert_eq!(d.property, "message");
                match d.receiver.as_ref().unwrap().as_ref() {
                    ExprNodeKind::ArrayDeref(_) => {}
                    _ => panic!("expected array deref"),
                }
            }
            _ => panic!("expected object deref"),
        }
    }

    #[test]
    fn parse_index_access() {
        let n = parse_ok("github.event.commits[0] }}");
        match n.as_ref() {
            ExprNodeKind::IndexAccess(i) => match i.index.as_ref().unwrap().as_ref() {
                ExprNodeKind::Int(x) => assert_eq!(x.value, 0),
                _ => panic!("expected int index"),
            },
            _ => panic!("expected index access"),
        }
    }

    #[test]
    fn parse_operators() {
        let n = parse_ok("a == b }}");
        match n.as_ref() {
            ExprNodeKind::CompareOp(c) => assert_eq!(c.kind, CompareOpKind::Eq),
            _ => panic!("expected compare"),
        }

        let n = parse_ok("a && b || c }}");
        match n.as_ref() {
            ExprNodeKind::LogicalOp(l) => assert_eq!(l.kind, LogicalOpKind::Or),
            _ => panic!("expected logical or"),
        }
    }

    #[test]
    fn parse_not() {
        let n = parse_ok("!github }}");
        match n.as_ref() {
            ExprNodeKind::NotOp(_) => {}
            _ => panic!("expected not"),
        }
    }

    #[test]
    fn parse_nested() {
        let n = parse_ok("(a || b) && c }}");
        match n.as_ref() {
            ExprNodeKind::LogicalOp(l) => assert_eq!(l.kind, LogicalOpKind::And),
            _ => panic!("expected logical and"),
        }
    }

    #[test]
    fn parse_error_missing_paren() {
        let e = parse_expression("(a || b }}").unwrap_err();
        assert!(!e.message.is_empty());
    }

    #[test]
    fn parse_error_trailing_tokens() {
        let e = parse_expression("a b }}").unwrap_err();
        assert!(e.message.contains("remaining token"));
    }

    #[test]
    fn parse_error_bad_string() {
        assert!(parse_expression("\"x\" }}").is_err());
    }
}
