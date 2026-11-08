//! Untrusted input checker for GitHub Actions expressions.
//!
//! Rust counterpart of the original `expr_insecure.go`. It detects potentially untrusted inputs in
//! `run:` scripts (e.g. `github.event.issue.title`) to mitigate script injection attacks. See
//! <https://docs.github.com/en/actions/reference/security/secure-use#good-practices-for-mitigating-script-injection-attacks>.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

use crate::expr::ExprError;
use crate::expr_ast::{ExprNode, ExprNodeKind, FuncCallNode, VariableNode};
use crate::expr_lexer::Token;

fn is_safe_func_call(call: &FuncCallNode) -> bool {
    matches!(
        call.callee.to_lowercase().as_str(),
        "contains" | "startswith" | "endswith"
    )
}

/// A node of the untrusted input search tree. It matches recursive context object property
/// dereferences. The root of the tree represents a context name and its descendants represent
/// recursive properties.
pub struct UntrustedInputMap {
    /// The name of this node (a context name at the root, or a property name below).
    pub name: String,
    parent: RefCell<Weak<UntrustedInputMap>>,
    /// The child properties. `None` means this node is a leaf.
    children: Option<BTreeMap<String, Rc<UntrustedInputMap>>>,
}

impl UntrustedInputMap {
    /// Creates a new untrusted input map node with the given name and children.
    ///
    /// The children's parent pointers are set to this node.
    #[must_use]
    pub fn new(name: &str, children: Vec<Rc<UntrustedInputMap>>) -> Rc<Self> {
        let mut m = BTreeMap::new();
        for c in children {
            m.insert(c.name.clone(), c);
        }
        let node = Rc::new(Self {
            name: name.to_string(),
            parent: RefCell::new(Weak::new()),
            children: if m.is_empty() { None } else { Some(m) },
        });
        // Set each child's parent pointer now that `node` exists.
        if let Some(map) = &node.children {
            for c in map.values() {
                *c.parent.borrow_mut() = Rc::downgrade(&node);
            }
        }
        node
    }

    fn find_object_prop(&self, name: &str) -> Option<Rc<UntrustedInputMap>> {
        self.children.as_ref()?.get(name).cloned()
    }

    fn find_array_elem(&self) -> Option<Rc<UntrustedInputMap>> {
        self.find_object_prop("*")
    }

    /// Builds a path like `github.event.commits.*.body` by following parent pointers.
    pub fn build_path(&self) -> String {
        let mut b = String::new();
        self.build_path_inner(&mut b);
        b
    }

    fn build_path_inner(&self, b: &mut String) {
        if let Some(p) = self.parent.borrow().upgrade()
            && !p.name.is_empty()
        {
            p.build_path_inner(b);
            b.push('.');
        }
        b.push_str(&self.name);
    }
}

impl std::fmt::Display for UntrustedInputMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.build_path())
    }
}

/// A list of untrusted inputs forming a tree structure to efficiently detect untrusted inputs in
/// nested object property access, array index access, and object filters. Each value is a root of
/// the search, so the names are context names.
pub type UntrustedInputSearchRoots = BTreeMap<String, Rc<UntrustedInputMap>>;

/// The list of builtin untrusted inputs. These inputs are detected as untrusted in `run:` scripts.
///
/// See:
/// - <https://securitylab.github.com/research/github-actions-untrusted-input/>
/// - <https://docs.github.com/en/actions/reference/security/secure-use#good-practices-for-mitigating-script-injection-attacks>
/// - <https://github.com/github/codeql/blob/main/javascript/ql/src/experimental/Security/CWE-094/ExpressionInjection.ql>
#[must_use]
pub fn builtin_untrusted_inputs() -> UntrustedInputSearchRoots {
    let mut roots = UntrustedInputSearchRoots::new();
    let github = UntrustedInputMap::new(
        "github",
        vec![
            UntrustedInputMap::new(
                "event",
                vec![
                    UntrustedInputMap::new(
                        "issue",
                        vec![
                            UntrustedInputMap::new("title", vec![]),
                            UntrustedInputMap::new("body", vec![]),
                        ],
                    ),
                    UntrustedInputMap::new(
                        "pull_request",
                        vec![
                            UntrustedInputMap::new("title", vec![]),
                            UntrustedInputMap::new("body", vec![]),
                            UntrustedInputMap::new(
                                "head",
                                vec![
                                    UntrustedInputMap::new("ref", vec![]),
                                    UntrustedInputMap::new("label", vec![]),
                                    UntrustedInputMap::new(
                                        "repo",
                                        vec![UntrustedInputMap::new("default_branch", vec![])],
                                    ),
                                ],
                            ),
                        ],
                    ),
                    UntrustedInputMap::new("comment", vec![UntrustedInputMap::new("body", vec![])]),
                    UntrustedInputMap::new("review", vec![UntrustedInputMap::new("body", vec![])]),
                    UntrustedInputMap::new(
                        "review_comment",
                        vec![UntrustedInputMap::new("body", vec![])],
                    ),
                    UntrustedInputMap::new(
                        "pages",
                        vec![UntrustedInputMap::new(
                            "*",
                            vec![UntrustedInputMap::new("page_name", vec![])],
                        )],
                    ),
                    UntrustedInputMap::new(
                        "commits",
                        vec![UntrustedInputMap::new(
                            "*",
                            vec![
                                UntrustedInputMap::new("message", vec![]),
                                UntrustedInputMap::new(
                                    "author",
                                    vec![
                                        UntrustedInputMap::new("email", vec![]),
                                        UntrustedInputMap::new("name", vec![]),
                                    ],
                                ),
                            ],
                        )],
                    ),
                    UntrustedInputMap::new(
                        "head_commit",
                        vec![
                            UntrustedInputMap::new("message", vec![]),
                            UntrustedInputMap::new(
                                "author",
                                vec![
                                    UntrustedInputMap::new("email", vec![]),
                                    UntrustedInputMap::new("name", vec![]),
                                ],
                            ),
                        ],
                    ),
                    UntrustedInputMap::new(
                        "discussion",
                        vec![
                            UntrustedInputMap::new("title", vec![]),
                            UntrustedInputMap::new("body", vec![]),
                        ],
                    ),
                ],
            ),
            UntrustedInputMap::new("head_ref", vec![]),
        ],
    );
    roots.insert("github".to_string(), github);
    roots
}

fn errorf_at_expr(tok: &Token, msg: String) -> ExprError {
    ExprError {
        message: msg,
        offset: tok.offset,
        line: tok.line,
        column: tok.column,
    }
}

fn sorted_quotes(mut inputs: Vec<String>) -> String {
    inputs.sort();
    inputs.dedup();
    inputs
        .iter()
        .map(|s| format!("{s:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A checker to detect untrusted inputs in an expression syntax tree. It checks object property
/// accesses, array index accesses, and object filters, and detects paths to untrusted inputs.
///
/// Note: To avoid breaking the state of checking property accesses on nested property accesses like
/// `foo[aaa.bbb].bar`, `IndexAccessNode.index` is visited before `IndexAccessNode.operand`.
pub struct UntrustedInputChecker {
    roots: UntrustedInputSearchRoots,
    filtering_object: bool,
    cur: Vec<Option<Rc<UntrustedInputMap>>>,
    start: Option<Token>,
    errs: Vec<ExprError>,
    safe_calls: usize,
}

impl UntrustedInputChecker {
    /// Creates a new checker. The `roots` argument is a search tree which defines untrusted input
    /// paths.
    #[must_use]
    pub fn new(roots: UntrustedInputSearchRoots) -> Self {
        Self {
            roots,
            filtering_object: false,
            cur: Vec::new(),
            start: None,
            errs: Vec::new(),
            safe_calls: 0,
        }
    }

    fn reset(&mut self) {
        self.start = None;
        self.filtering_object = false;
        self.cur.clear();
    }

    fn compact(&mut self) {
        self.cur.retain(Option::is_some);
    }

    fn on_var(&mut self, v: &VariableNode) {
        // Find root context (currently only "github" exists).
        if let Some(c) = self.roots.get(&v.name) {
            self.start = Some(v.tok.clone());
            self.cur.push(Some(Rc::clone(c)));
        }
    }

    fn on_prop_access(&mut self, name: &str) {
        let mut changed = false;
        for c in &mut self.cur {
            let Some(cur) = c else { continue };
            if let Some(next) = cur.find_object_prop(name) {
                *c = Some(next)
            } else {
                *c = None;
                changed = true;
            }
        }
        if changed {
            self.compact();
        }
    }

    fn on_index_access(&mut self) {
        if self.filtering_object {
            self.filtering_object = false;
            return; // e.g. match `github.event.*.body[0]` as `github.event.commits[0].body`
        }
        let mut changed = false;
        for c in &mut self.cur {
            let Some(cur) = c else { continue };
            if let Some(next) = cur.find_array_elem() {
                *c = Some(next)
            } else {
                *c = None;
                changed = true;
            }
        }
        if changed {
            self.compact();
        }
    }

    fn on_object_filter(&mut self) {
        self.filtering_object = true;
        let mut changed = false;
        let mut appended: Vec<Option<Rc<UntrustedInputMap>>> = Vec::new();
        for c in &mut self.cur {
            // Decide the replacement values before assigning back to `c` so the borrow on `*c`
            // is released.
            let replacement: Vec<Rc<UntrustedInputMap>> = match c {
                Some(cur) => {
                    // Object filter for arrays
                    if let Some(next) = cur.find_array_elem() {
                        vec![next]
                    } else {
                        match &cur.children {
                            None => {
                                changed = true;
                                Vec::new()
                            }
                            // Object filter for objects: expand to all children.
                            Some(children) => children.values().cloned().collect(),
                        }
                    }
                }
                None => continue,
            };
            if replacement.is_empty() {
                *c = None;
            } else {
                *c = Some(replacement[0].clone());
                for child in &replacement[1..] {
                    appended.push(Some(child.clone()));
                }
            }
        }
        self.cur.append(&mut appended);
        if changed {
            self.compact();
        }
    }

    fn end(&mut self) {
        let mut inputs: Vec<String> = Vec::new();
        for cur in self.cur.iter().flatten() {
            if cur.children.is_some() {
                continue; // A leaf has no children
            }
            inputs.push(cur.build_path());
        }

        if inputs.len() == 1 {
            let err = errorf_at_expr(
                self.start.as_ref().unwrap(),
                format!(
                    "{:?} is potentially untrusted. avoid using it directly in inline scripts. instead, pass it through an environment variable. see https://docs.github.com/en/actions/reference/security/secure-use#good-practices-for-mitigating-script-injection-attacks for more details",
                    inputs[0]
                ),
            );
            self.errs.push(err);
        } else if inputs.len() > 1 {
            // When multiple untrusted inputs are detected, the expression extracts multiple
            // properties with object filter syntax. Show all properties in the error message.
            let err = errorf_at_expr(
                self.start.as_ref().unwrap(),
                format!(
                    "object filter extracts potentially untrusted properties {}. avoid using the value directly in inline scripts. instead, pass the value through an environment variable. see https://docs.github.com/en/actions/reference/security/secure-use#good-practices-for-mitigating-script-injection-attacks for more details",
                    sorted_quotes(inputs)
                ),
            );
            self.errs.push(err);
        }

        self.reset();
    }

    /// Callback invoked when visiting a node before visiting its children.
    pub fn on_visit_node_enter(&mut self, n: &ExprNode) {
        if let ExprNodeKind::FuncCall(f) = n.as_ref()
            && is_safe_func_call(f)
        {
            self.safe_calls += 1;
        }
    }

    /// Callback invoked when visiting a node after visiting its children.
    pub fn on_visit_node_leave(&mut self, n: &ExprNode) {
        // Skip unsafe checks if we are inside of a safe function call expression.
        if self.safe_calls > 0 {
            if let ExprNodeKind::FuncCall(f) = n.as_ref()
                && is_safe_func_call(f)
            {
                self.safe_calls -= 1;
            }
            return;
        }

        match n.as_ref() {
            ExprNodeKind::Variable(v) => {
                self.end();
                self.on_var(v);
            }
            ExprNodeKind::ObjectDeref(d) => {
                self.on_prop_access(&d.property);
            }
            ExprNodeKind::IndexAccess(i) => {
                if let Some(ExprNodeKind::String(lit)) =
                    i.index.as_ref().map(std::convert::AsRef::as_ref)
                {
                    // Special case like github['event']['issue']['title']
                    self.on_prop_access(&lit.value);
                } else {
                    self.on_index_access();
                }
            }
            ExprNodeKind::ArrayDeref(_) => {
                self.on_object_filter();
            }
            _ => {
                self.end();
            }
        }
    }

    /// Callback invoked after visiting the whole syntax tree. This is necessary to handle the case
    /// where an untrusted input access is at the root of the expression.
    pub fn on_visit_end(&mut self) {
        self.end();
    }

    /// Returns errors detected by this checker. This should be called after visiting all nodes in a
    /// syntax tree.
    #[must_use]
    pub fn errs(&self) -> &[ExprError] {
        &self.errs
    }

    /// Takes the detected errors, leaving this checker empty.
    pub fn take_errs(&mut self) -> Vec<ExprError> {
        std::mem::take(&mut self.errs)
    }

    /// Initializes the state of the checker.
    pub fn init(&mut self) {
        self.errs.clear();
        self.safe_calls = 0;
        self.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr_ast::visit_expr_node;
    use crate::expr_parser;

    fn check(src: &str) -> Vec<String> {
        let (node, _) = expr_parser::parse_expression(src).unwrap();
        let mut c = UntrustedInputChecker::new(builtin_untrusted_inputs());
        c.init();
        visit_expr_node(&node, &mut |n, _, entering| {
            if entering {
                c.on_visit_node_enter(n);
            } else {
                c.on_visit_node_leave(n);
            }
        });
        c.on_visit_end();
        c.take_errs().into_iter().map(|e| e.message).collect()
    }

    #[test]
    fn detects_untrusted_input() {
        let errs = check("github.event.issue.title }}");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("\"github.event.issue.title\" is potentially untrusted"));
    }

    #[test]
    fn detects_nested_array_filter() {
        let errs = check("github.event.commits.*.message }}");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("github.event.commits.*.message"));
    }

    #[test]
    fn safe_func_call_suppresses() {
        let errs = check("contains(github.event.issue.title, 'x') }}");
        assert!(errs.is_empty());
    }

    #[test]
    fn trusted_input_not_reported() {
        let errs = check("github.event.action }}");
        assert!(errs.is_empty());
    }

    #[test]
    fn index_string_access() {
        let errs = check("github['event']['issue']['title'] }}");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("github.event.issue.title"));
    }

    #[test]
    fn object_filter_multiple() {
        // `.*` on `event` expands to all its children; `.body` reaches multiple leaves.
        let errs = check("github.event.*.body }}");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].starts_with("object filter extracts potentially untrusted properties"));
    }

    #[test]
    fn map_display() {
        let m = builtin_untrusted_inputs();
        let github = m.get("github").unwrap();
        assert_eq!(github.build_path(), "github");
    }
}
