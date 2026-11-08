//! Expression syntax tree.
//!
//! Rust counterpart of the original `expr_ast.go`. It defines the nodes of the GitHub Actions
//! expression syntax tree (see
//! <https://docs.github.com/en/actions/learn-github-actions/expressions>).

use std::fmt;
use std::rc::Rc;

use crate::expr_lexer::Token;

/// A node of the expression syntax tree.
pub type ExprNode = Rc<ExprNodeKind>;

/// The concrete kinds of expression syntax tree nodes.
#[derive(Debug, Clone)]
pub enum ExprNodeKind {
    /// Variable access, e.g. `github`.
    Variable(VariableNode),
    /// The `null` literal.
    Null(NullNode),
    /// A boolean literal, `true` or `false`.
    Bool(BoolNode),
    /// An integer literal.
    Int(IntNode),
    /// A float literal.
    Float(FloatNode),
    /// A string literal with escapes resolved and quotes removed.
    String(StringNode),
    /// Property dereference of an object, e.g. `foo.bar`.
    ObjectDeref(ObjectDerefNode),
    /// Elements dereference of an array, e.g. `*` in `foo.bar.*.piyo`.
    ArrayDeref(ArrayDerefNode),
    /// Index access, representing dynamic property or array index access.
    IndexAccess(IndexAccessNode),
    /// The unary `!` operator.
    NotOp(NotOpNode),
    /// A binary comparison; `==`, `!=`, `<`, `<=`, `>` or `>=`.
    CompareOp(CompareOpNode),
    /// A logical binary operator; `&&` or `||`.
    LogicalOp(LogicalOpNode),
    /// A function call. Currently only builtin functions can be called.
    FuncCall(FuncCallNode),
}

impl ExprNodeKind {
    /// Returns the first token of the node, used to obtain its source position.
    #[must_use]
    pub fn token(&self) -> Option<&Token> {
        match self {
            ExprNodeKind::Variable(n) => Some(&n.tok),
            ExprNodeKind::Null(n) => Some(&n.tok),
            ExprNodeKind::Bool(n) => Some(&n.tok),
            ExprNodeKind::Int(n) => Some(&n.tok),
            ExprNodeKind::Float(n) => Some(&n.tok),
            ExprNodeKind::String(n) => Some(&n.tok),
            ExprNodeKind::ObjectDeref(n) => n.receiver.as_ref().and_then(|r| r.token()),
            ExprNodeKind::ArrayDeref(n) => n.receiver.as_ref().and_then(|r| r.token()),
            ExprNodeKind::IndexAccess(n) => n.operand.as_ref().and_then(|o| o.token()),
            ExprNodeKind::NotOp(n) => Some(&n.tok),
            ExprNodeKind::CompareOp(n) => n.left.as_ref().and_then(|l| l.token()),
            ExprNodeKind::LogicalOp(n) => n.left.as_ref().and_then(|l| l.token()),
            ExprNodeKind::FuncCall(n) => Some(&n.tok),
        }
    }
}

/// A node for variable access.
#[derive(Debug, Clone)]
pub struct VariableNode {
    /// The name of the variable.
    pub name: String,
    /// The first token of the node.
    pub tok: Token,
}

/// A node for the `null` literal.
#[derive(Debug, Clone)]
pub struct NullNode {
    /// The first token of the node.
    pub tok: Token,
}

/// A node for a boolean literal.
#[derive(Debug, Clone)]
pub struct BoolNode {
    /// The value of the boolean literal.
    pub value: bool,
    /// The first token of the node.
    pub tok: Token,
}

/// A node for an integer literal.
#[derive(Debug, Clone)]
pub struct IntNode {
    /// The value of the integer literal.
    pub value: i64,
    /// The first token of the node.
    pub tok: Token,
}

/// A node for a float literal.
#[derive(Debug, Clone)]
pub struct FloatNode {
    /// The value of the float literal.
    pub value: f64,
    /// The first token of the node.
    pub tok: Token,
}

/// A node for a string literal.
#[derive(Debug, Clone)]
pub struct StringNode {
    /// The value of the string literal with escapes resolved and quotes removed.
    pub value: String,
    /// The first token of the node.
    pub tok: Token,
}

/// A node representing property dereference of an object like `foo.bar`.
#[derive(Debug, Clone)]
pub struct ObjectDerefNode {
    /// The receiver expression of the property dereference.
    pub receiver: Option<ExprNode>,
    /// The name of the property to access.
    pub property: String,
}

/// A node representing element dereference of an array like `*` in `foo.bar.*.piyo`.
#[derive(Debug, Clone)]
pub struct ArrayDerefNode {
    /// The receiver expression of the array element dereference.
    pub receiver: Option<ExprNode>,
}

/// A node for index access, representing dynamic object property access or array index access.
#[derive(Debug, Clone)]
pub struct IndexAccessNode {
    /// The operand expression, which should be an array or object.
    pub operand: Option<ExprNode>,
    /// The index expression, which should be an integer or string.
    pub index: Option<ExprNode>,
}

/// A node for the unary `!` operator.
#[derive(Debug, Clone)]
pub struct NotOpNode {
    /// The operand expression.
    pub operand: Option<ExprNode>,
    /// The first token of the node.
    pub tok: Token,
}

/// The kind of a comparison operator; `==`, `!=`, `<`, `<=`, `>` or `>=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOpKind {
    /// Invalid and initial value.
    Invalid,
    /// The `<` operator.
    Less,
    /// The `<=` operator.
    LessEq,
    /// The `>` operator.
    Greater,
    /// The `>=` operator.
    GreaterEq,
    /// The `==` operator.
    Eq,
    /// The `!=` operator.
    NotEq,
}

impl CompareOpKind {
    /// Returns whether this is `==` or `!=`.
    #[must_use]
    pub fn is_equality_op(&self) -> bool {
        matches!(self, CompareOpKind::Eq | CompareOpKind::NotEq)
    }
}

impl fmt::Display for CompareOpKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompareOpKind::Less => write!(f, "<"),
            CompareOpKind::LessEq => write!(f, "<="),
            CompareOpKind::Greater => write!(f, ">"),
            CompareOpKind::GreaterEq => write!(f, ">="),
            CompareOpKind::Eq => write!(f, "=="),
            CompareOpKind::NotEq => write!(f, "!="),
            CompareOpKind::Invalid => write!(f, ""),
        }
    }
}

/// A node for a binary comparison; `==`, `!=`, `<`, `<=`, `>` or `>=`.
#[derive(Debug, Clone)]
pub struct CompareOpNode {
    /// Which operator is used.
    pub kind: CompareOpKind,
    /// The left hand side expression.
    pub left: Option<ExprNode>,
    /// The right hand side expression.
    pub right: Option<ExprNode>,
}

/// The kind of a logical operator; `&&` or `||`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOpKind {
    /// Invalid and initial value.
    Invalid,
    /// The `&&` operator.
    And,
    /// The `||` operator.
    Or,
}

impl fmt::Display for LogicalOpKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogicalOpKind::And => write!(f, "&&"),
            LogicalOpKind::Or => write!(f, "||"),
            LogicalOpKind::Invalid => write!(f, "INVALID LOGICAL OPERATOR"),
        }
    }
}

/// A node for a logical binary operator; `&&` or `||`.
#[derive(Debug, Clone)]
pub struct LogicalOpNode {
    /// Which operator is used.
    pub kind: LogicalOpKind,
    /// The left hand side expression.
    pub left: Option<ExprNode>,
    /// The right hand side expression.
    pub right: Option<ExprNode>,
}

/// A node representing a function call in an expression.
#[derive(Debug, Clone)]
pub struct FuncCallNode {
    /// The name of the called function. Currently only builtin functions can be called.
    pub callee: String,
    /// The arguments of the function call.
    pub args: Vec<ExprNode>,
    /// The first token of the node.
    pub tok: Token,
}

/// A visitor function for [`visit_expr_node`]. The `entering` argument is `true` when called before
/// visiting children and `false` after. The `parent` is `None` for the root node.
pub type VisitExprNodeFunc<'a> = dyn FnMut(&ExprNode, Option<&ExprNode>, bool) + 'a;

/// Visits the given expression syntax tree in depth-first order, calling `f` before and after each
/// node.
pub fn visit_expr_node(n: &ExprNode, f: &mut VisitExprNodeFunc<'_>) {
    visit_expr_node_inner(n, None, f);
}

fn visit_expr_node_inner(n: &ExprNode, parent: Option<&ExprNode>, f: &mut VisitExprNodeFunc<'_>) {
    f(n, parent, true);
    match n.as_ref() {
        ExprNodeKind::ObjectDeref(x) => {
            if let Some(r) = &x.receiver {
                visit_expr_node_inner(r, Some(n), f);
            }
        }
        ExprNodeKind::ArrayDeref(x) => {
            if let Some(r) = &x.receiver {
                visit_expr_node_inner(r, Some(n), f);
            }
        }
        ExprNodeKind::IndexAccess(x) => {
            // Index must be visited before Operand so UntrustedInputChecker works correctly.
            if let Some(i) = &x.index {
                visit_expr_node_inner(i, Some(n), f);
            }
            if let Some(o) = &x.operand {
                visit_expr_node_inner(o, Some(n), f);
            }
        }
        ExprNodeKind::NotOp(x) => {
            if let Some(o) = &x.operand {
                visit_expr_node_inner(o, Some(n), f);
            }
        }
        ExprNodeKind::CompareOp(x) => {
            if let Some(l) = &x.left {
                visit_expr_node_inner(l, Some(n), f);
            }
            if let Some(r) = &x.right {
                visit_expr_node_inner(r, Some(n), f);
            }
        }
        ExprNodeKind::LogicalOp(x) => {
            if let Some(l) = &x.left {
                visit_expr_node_inner(l, Some(n), f);
            }
            if let Some(r) = &x.right {
                visit_expr_node_inner(r, Some(n), f);
            }
        }
        ExprNodeKind::FuncCall(x) => {
            for a in &x.args {
                visit_expr_node_inner(a, Some(n), f);
            }
        }
        _ => {}
    }
    f(n, parent, false);
}
