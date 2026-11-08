//! Parser for GitHub Actions workflow configuration files.
//!
//! Rust counterpart of the original `parse.go`. It parses a YAML workflow file into an
//! [`ast::Workflow`] syntax tree, reporting syntax errors as [`error::Error`] values with the
//! `syntax-check` kind. See
//! <https://docs.github.com/en/actions/reference/workflow-syntax-for-github-actions>.
//!
//! The original parses YAML into `yaml.Node` (a position-aware node tree that preserves duplicate
//! mapping keys) via `go.yaml.in/yaml/v4`. To mirror that faithfully we build the same kind of node
//! tree from the low-level [`saphyr_parser`] event stream. The higher-level `saphyr::Yaml` mapping
//! is unsuitable because it silently deduplicates mapping keys, which would break duplicate-key and
//! merge-key detection.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use saphyr_parser::{Event, Parser as YamlParser, ScalarStyle};

use crate::ast;
use crate::ast::{
    Bool, Concurrency, Container, Credentials, Defaults, DefaultsRun, DispatchInput, Env, EnvVar,
    Environment, Event as AstEvent, Exec, ExecAction, ExecRun, Float, ImageVersionEvent, Input,
    Int, Job, Matrix, MatrixAssign, MatrixCombination, MatrixCombinations, MatrixRow, Output,
    PermissionScope, Permissions, RawYAMLArray, RawYAMLObject, RawYAMLString, RawYAMLValue,
    RepositoryDispatchEvent, Runner, ScheduleEntry, ScheduledEvent, Service, Services, Snapshot,
    Step, Strategy, String as AstString, WebhookEvent, WebhookEventFilter, Workflow, WorkflowCall,
    WorkflowCallEvent, WorkflowCallEventInput, WorkflowCallEventInputType, WorkflowCallEventOutput,
    WorkflowCallEventSecret, WorkflowCallInput, WorkflowCallSecret, WorkflowDispatchEvent,
    WorkflowDispatchEventInputType,
};
use crate::error::Error;
use crate::quotes::sorted_quotes;

// ---------------------------------------------------------------------------
// YAML node tree (counterpart of `yaml.Node`)
// ---------------------------------------------------------------------------

/// The kind of a [`Node`] in the YAML node tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Scalar,
    Sequence,
    Mapping,
    Alias,
}

impl NodeKind {
    /// The human-readable name used in error messages (counterpart of `nodeKindName`).
    fn name(self) -> &'static str {
        match self {
            NodeKind::Scalar => "scalar",
            NodeKind::Sequence => "sequence",
            NodeKind::Mapping => "mapping",
            NodeKind::Alias => "alias",
        }
    }
}

/// Whether a scalar was quoted (or a block scalar) in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    Quoted,
    Block,
}

/// A node in the YAML node tree, mirroring `yaml.Node` from `go.yaml.in/yaml/v4`.
///
/// Mapping nodes store their content **flattened** as `[key, value, key, value, ...]`, exactly like
/// `yaml.Node.Content`, so the parser can iterate pairs with a stride of two.
#[derive(Debug, Clone)]
struct Node {
    kind: NodeKind,
    /// The resolved tag such as `!!str`, `!!int`, `!!null`, `!!merge`, `!!seq`, `!!map`.
    tag: String,
    style: Style,
    value: String,
    line: usize,
    col: usize,
    /// The anchor name declared on this node, if any.
    anchor: Option<String>,
    /// The numeric anchor ID linking aliases to their target (0 means "no anchor").
    anchor_id: usize,
    /// Container content. Mappings are flattened `[k, v, k, v, ...]`.
    content: Vec<Rc<Node>>,
    /// For alias nodes, the ID of the anchor they reference.
    alias_target: usize,
}

impl Node {
    fn pos(&self) -> ast::Pos {
        ast::Pos::new(self.line, self.col)
    }
}

/// Converts a `saphyr_parser` tag into the short tag form used by the parser (`!!str`, ...).
fn short_tag(handle: &str, suffix: &str) -> String {
    if handle == "tag:yaml.org,2002:" {
        format!("!!{suffix}")
    } else {
        // Custom tag handles (`!foo`) are irrelevant to every parser check; use the suffix.
        suffix.to_string()
    }
}

// ---------------------------------------------------------------------------
// Implicit tag resolution (counterpart of go-yaml's `resolve.go`)
// ---------------------------------------------------------------------------

/// Resolves an implicit (untagged, plain) scalar value to its YAML tag, mirroring go-yaml's
/// `resolve("", value)` for the core schema.
fn resolve_plain(value: &str) -> String {
    // The hint table maps a leading byte to a resolution strategy; bytes not listed resolve to the
    // default (0), which skips the map lookup. An empty value keeps the `N` default (never 0), so
    // the empty string still resolves to `!!null` like go-yaml.
    let hint = match value.as_bytes().first() {
        None => b'N',
        Some(&b) => match b {
            b'+' | b'-' => b'S',
            b'0'..=b'9' => b'D',
            b'y' | b'Y' | b'n' | b'N' | b't' | b'T' | b'f' | b'F' | b'o' | b'O' | b'~' => b'M',
            b'.' => b'.',
            _ => 0,
        },
    };

    if hint != 0 {
        match value {
            "true" | "True" | "TRUE" | "false" | "False" | "FALSE" => return "!!bool".into(),
            "" | "~" | "null" | "Null" | "NULL" => return "!!null".into(),
            ".nan" | ".NaN" | ".NAN" => return "!!float".into(),
            ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" | "-.inf" | "-.Inf"
            | "-.INF" | "-0" | "-0.0" => return "!!float".into(),
            "<<" => return "!!merge".into(),
            _ => {}
        }

        match hint {
            b'M' => {}
            b'.' => {
                if parse_go_float(value).is_some() {
                    return "!!float".into();
                }
            }
            b'D' | b'S' => {
                if is_timestamp(value) {
                    return "!!timestamp".into();
                }
                let plain = value.replace('_', "");
                if parse_go_int(&plain).is_some() {
                    return "!!int".into();
                }
                if is_yaml_style_float(&plain) && parse_go_float(&plain).is_some() {
                    return "!!float".into();
                }
            }
            _ => {}
        }
    }

    "!!str".into()
}

/// Parses an integer using Go's `strconv.ParseInt(plain, 0, 64)` semantics (base inferred from a
/// `0x`/`0o`/`0b` prefix, or a leading `0` for octal).
fn parse_go_int(plain: &str) -> Option<i64> {
    let (neg, rest) = match plain.as_bytes().first() {
        Some(b'-') => (true, &plain[1..]),
        Some(b'+') => (false, &plain[1..]),
        _ => (false, plain),
    };
    if rest.is_empty() {
        return None;
    }

    let (digits, radix) = if let Some(r) = rest.strip_prefix("0x").or(rest.strip_prefix("0X")) {
        (r, 16)
    } else if let Some(r) = rest.strip_prefix("0o").or(rest.strip_prefix("0O")) {
        (r, 8)
    } else if let Some(r) = rest.strip_prefix("0b").or(rest.strip_prefix("0B")) {
        (r, 2)
    } else if rest.len() > 1 && rest.starts_with('0') {
        (&rest[1..], 8)
    } else {
        (rest, 10)
    };
    if digits.is_empty() {
        return None;
    }

    let mag = i128::from_str_radix(digits, radix).ok()?;
    let value = if neg { -mag } else { mag };
    i64::try_from(value).ok()
}

/// Parses a float using `f64::from_str` plus the `inf`/`nan` spellings Go's `strconv.ParseFloat`
/// accepts.
fn parse_go_float(value: &str) -> Option<f64> {
    let lower = value.to_ascii_lowercase();
    let lower = lower.strip_prefix('+').unwrap_or(&lower);
    match lower {
        "inf" | "infinity" => return Some(f64::INFINITY),
        "-inf" | "-infinity" => return Some(f64::NEG_INFINITY),
        "nan" => return Some(f64::NAN),
        _ => {}
    }
    value.parse::<f64>().ok()
}

/// Whether the string matches go-yaml's `yamlStyleFloat` regular expression
/// `^[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?$`.
fn is_yaml_style_float(value: &str) -> bool {
    let b = value.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        i += 1;
    }
    // mantissa: (\.[0-9]+|[0-9]+(\.[0-9]*)?)
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    } else {
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    // exponent: ([eE][-+]?[0-9]+)?
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == b.len()
}

/// A minimal check that a value looks like a YAML timestamp, mirroring go-yaml's `parseTimestamp`.
/// Only the resulting *tag* matters to the parser, so the value itself is not produced.
fn is_timestamp(value: &str) -> bool {
    let b = value.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i != 4 || i == b.len() || b[i] != b'-' {
        return false;
    }
    let rest = &value[5..];
    // Date only: "2006-1-2".
    if let Some((y, m, d)) = parse_ymd(rest)
        && chrono::NaiveDate::from_ymd_opt(y, m, d).is_some()
    {
        return true;
    }
    // Space separated with no timezone: "2006-1-2 15:4:5.999999999".
    if let Some((date, time)) = rest.split_once(' ')
        && let Some((y, m, d)) = parse_ymd(date)
        && parse_hms(time).is_some()
        && chrono::NaiveDate::from_ymd_opt(y, m, d).is_some()
    {
        return true;
    }
    // RFC3339 with a 'T'/'t' separator and a timezone.
    if let Some(t) = rest.find(['T', 't']) {
        let (date, time) = rest.split_at(t);
        let time = &time[1..];
        if let Some((y, m, d)) = parse_ymd(date)
            && parse_hms_tz(time)
            && chrono::NaiveDate::from_ymd_opt(y, m, d).is_some()
        {
            return true;
        }
    }
    false
}

/// Parses `Y-M-D` returning `(year, month, day)`.
fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    let (y, rest) = s.split_once('-')?;
    let (m, d) = rest.split_once('-')?;
    if y.is_empty() || m.is_empty() || d.is_empty() {
        return None;
    }
    Some((y.parse().ok()?, m.parse().ok()?, d.parse().ok()?))
}

/// Validates an `H:M:S[.fraction]` time component.
fn parse_hms(s: &str) -> Option<()> {
    let (h, rest) = s.split_once(':')?;
    let (m, sec) = rest.split_once(':')?;
    let sec = sec.split('.').next()?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    let sec: u32 = sec.parse().ok()?;
    (h <= 23 && m <= 59 && sec <= 59).then_some(())
}

/// Validates an `H:M:S[.fraction]Z` or `H:M:S[.fraction][+-]HH:MM` time with a timezone.
fn parse_hms_tz(s: &str) -> bool {
    let (time, tz) = if let Some(z) = s.find(['Z', 'z']) {
        (&s[..z], "")
    } else {
        let Some(p) = s.rfind(['+', '-']) else {
            return false;
        };
        let tz = &s[p + 1..];
        if !tz.contains(':') {
            return false;
        }
        (&s[..p], tz)
    };
    if parse_hms(time).is_none() {
        return false;
    }
    if tz.is_empty() {
        return true; // 'Z' form
    }
    let Some((oh, om)) = tz.split_once(':') else {
        return false;
    };
    oh.parse::<u32>().is_ok() && om.parse::<u32>().is_ok()
}

// ---------------------------------------------------------------------------
// Node tree construction from the saphyr event stream
// ---------------------------------------------------------------------------

/// Extracts the anchor name declared at `index` in the source. saphyr reports aliases only as
/// numeric IDs, so anchor *names* are recovered from the source text at each anchored node's start
/// index, which points at the `&` (or at a tag handle that precedes it).
fn anchor_name_at(source: &str, index: usize) -> Option<String> {
    let bytes = source.as_bytes();
    let mut i = index;
    while i < bytes.len() && bytes[i] != b'&' {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    i += 1; // skip '&'
    let start = i;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() || matches!(c, b',' | b'[' | b']' | b'{' | b'}') {
            break;
        }
        i += 1;
    }
    if i == start {
        return None;
    }
    Some(source[start..i].to_string())
}

/// A container being assembled while walking the event stream.
enum Frame {
    /// The document root, holding at most one child (the root node).
    Document(Option<Rc<Node>>),
    Sequence(Node),
    /// A mapping holds a key in `pending` until its value arrives.
    Mapping(Node, Option<Node>),
}

/// Whether the scalar about to be added is the *value* of a mapping entry (its key is pending).
///
/// go-yaml positions an empty implicit-null value of a `key:` entry one column past the `:`, but
/// an empty sequence item (`-` with no value) at the dash itself. saphyr reports both the same way,
/// so the empty-scalar column shift must only apply to mapping values.
fn is_mapping_value(stack: &[Frame]) -> bool {
    matches!(stack.last(), Some(Frame::Mapping(_, Some(_))))
}

/// Computes the position of a block scalar's `|`/`>` indicator.
///
/// go-yaml records a block scalar node's position at the indicator that introduces it (e.g. the
/// `|` in `if: |`), which sits at the end of the previous non-blank line. saphyr instead points the
/// scalar event at the first content character. Given the content-start `marker`, this walks back to
/// the last non-blank line and returns the position just past its last character (the indicator).
fn block_scalar_indicator_pos(source: &str, marker: saphyr_parser::Marker) -> (usize, usize) {
    let content_idx = marker.index();
    let before = &source[..content_idx.min(source.len())];
    // The last non-blank line before the content holds the indicator as its final character.
    let Some(last_line_start) = before.trim_end().rfind('\n').map(|i| i + 1) else {
        return (marker.line(), marker.col() + 1);
    };
    let last_line = &before[last_line_start..before.trim_end().len()];
    if last_line.trim().is_empty() {
        return (marker.line(), marker.col() + 1);
    }
    let line = marker.line().saturating_sub(1);
    // The indicator is the last character of that line, so its column is the line's char length.
    let col = last_line.chars().count();
    (line, col)
}

/// Builds a [`Node`] tree for the first document in the source, attaching anchor names.
fn build_tree(source: &str) -> Result<Node, saphyr_parser::ScanError> {
    let parser = YamlParser::new_from_str(source);
    let mut stack: Vec<Frame> = vec![Frame::Document(None)];
    let mut done = false;

    for ev in parser {
        let (event, span) = ev?;
        if done {
            break;
        }
        let line = span.start.line();
        let col = span.start.col() + 1;

        match event {
            Event::StreamStart | Event::StreamEnd | Event::Nothing | Event::DocumentStart(_) => {}
            Event::DocumentEnd => done = true,
            Event::Alias(id) => {
                let node = Node {
                    kind: NodeKind::Alias,
                    tag: String::new(),
                    style: Style::Plain,
                    value: String::new(),
                    line,
                    col,
                    anchor: None,
                    anchor_id: 0,
                    content: Vec::new(),
                    alias_target: id,
                };
                add_child(&mut stack, node);
            }
            Event::Scalar(value, style, anchor_id, tag) => {
                let s = value.into_owned();
                // Block scalars (`|`/`>`) get their own style so they are not treated as quoted:
                // go-yaml positions them at the indicator and they carry no surrounding quotes.
                let style = match style {
                    ScalarStyle::Plain => Style::Plain,
                    ScalarStyle::Literal | ScalarStyle::Folded => Style::Block,
                    _ => Style::Quoted,
                };
                let is_block = style == Style::Block;
                // Only plain (unquoted) scalars receive an implicit tag; quoted and block
                // scalars are always `!!str`, matching go-yaml's resolver.
                let tag = match &tag {
                    Some(t) => short_tag(&t.handle, &t.suffix),
                    None if style == Style::Plain => resolve_plain(&s),
                    None => "!!str".to_string(),
                };
                // go-yaml positions a block scalar node at its `|`/`>` indicator, but saphyr
                // points the event at the first content character on a following line. Walk back
                // to the indicator so positions match the original.
                let (line, col) = if is_block {
                    block_scalar_indicator_pos(source, span.start)
                } else if style == Style::Plain
                    && s.is_empty()
                    && anchor_id == 0
                    && is_mapping_value(&stack)
                {
                    // An empty plain scalar with no anchor is the implicit null value of a `key:`
                    // with no value. saphyr gives it a zero-width span at the `:` while go-yaml
                    // places the node just after it, so shift one column to match. An *anchored*
                    // empty scalar (`key: &name`) is positioned at its `&` indicator, which saphyr
                    // already reports correctly, so it is excluded here.
                    (line, col + 1)
                } else {
                    (line, col)
                };
                let node = Node {
                    kind: NodeKind::Scalar,
                    tag,
                    style,
                    value: s,
                    line,
                    col,
                    anchor: anchor_name_at(source, span.start.index()),
                    anchor_id,
                    content: Vec::new(),
                    alias_target: 0,
                };
                add_child(&mut stack, node);
            }
            Event::SequenceStart(anchor_id, tag) => {
                let tag = match &tag {
                    Some(t) => short_tag(&t.handle, &t.suffix),
                    None => "!!seq".to_string(),
                };
                let node = Node {
                    kind: NodeKind::Sequence,
                    tag,
                    style: Style::Plain,
                    value: String::new(),
                    line,
                    col,
                    anchor: anchor_name_at(source, span.start.index()),
                    anchor_id,
                    content: Vec::new(),
                    alias_target: 0,
                };
                stack.push(Frame::Sequence(node));
            }
            Event::MappingStart(anchor_id, tag) => {
                let tag = match &tag {
                    Some(t) => short_tag(&t.handle, &t.suffix),
                    None => "!!map".to_string(),
                };
                let node = Node {
                    kind: NodeKind::Mapping,
                    tag,
                    style: Style::Plain,
                    value: String::new(),
                    line,
                    col,
                    anchor: anchor_name_at(source, span.start.index()),
                    anchor_id,
                    content: Vec::new(),
                    alias_target: 0,
                };
                stack.push(Frame::Mapping(node, None));
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let finished = match stack.pop().expect("container frame") {
                    Frame::Sequence(n) => n,
                    Frame::Mapping(mut n, pending) => {
                        // A trailing key with no value gets an implicit null value, like go-yaml.
                        if let Some(key) = pending {
                            let null = Node {
                                kind: NodeKind::Scalar,
                                tag: "!!null".into(),
                                style: Style::Plain,
                                value: String::new(),
                                line: n.line,
                                col: n.col,
                                anchor: None,
                                anchor_id: 0,
                                content: Vec::new(),
                                alias_target: 0,
                            };
                            n.content.push(Rc::new(key));
                            n.content.push(Rc::new(null));
                        }
                        n
                    }
                    Frame::Document(_) => unreachable!("unbalanced document frame"),
                };
                add_child(&mut stack, finished);
            }
        }
    }

    match stack.into_iter().next() {
        Some(Frame::Document(Some(root))) => Ok((*root).clone()),
        _ => Ok(empty_node()),
    }
}

/// Attaches a finished node to the current container frame.
#[allow(clippy::ptr_arg)] // `Vec` is required for `last_mut`; a slice would not support it
fn add_child(stack: &mut Vec<Frame>, node: Node) {
    let rc = Rc::new(node);
    match stack.last_mut().expect("document frame always present") {
        Frame::Document(slot) => *slot = Some(rc),
        Frame::Sequence(n) => n.content.push(rc),
        Frame::Mapping(n, pending) => match pending.take() {
            Some(key) => {
                n.content.push(Rc::new(key));
                n.content.push(rc);
            }
            None => *pending = Some((*rc).clone()),
        },
    }
}

/// Creates an empty scalar node used when a document has no content.
fn empty_node() -> Node {
    Node {
        kind: NodeKind::Scalar,
        tag: "!!null".into(),
        style: Style::Plain,
        value: String::new(),
        line: 1,
        col: 1,
        anchor: None,
        anchor_id: 0,
        content: Vec::new(),
        alias_target: 0,
    }
}

/// Resolves alias nodes to their target anchor nodes, reporting recursive and unused anchors.
/// Mirrors `parser.resolveAliases`. Unknown anchors are detected by the YAML parser itself.
///
/// The tree is rebuilt bottom-up so that each anchor entry stores the *resolved* node. Storing a
/// pre-resolution clone (and mutating via `Rc::make_mut`) would leave the stored target stale,
/// causing aliases to substitute an unresolved subtree.
fn resolve_aliases(root: &mut Rc<Node>, errors: &mut Vec<Error>) {
    #[derive(Default)]
    struct AnchorEntry {
        used: bool,
        /// Whether the anchored node's own content has been fully resolved. Aliases encountered
        /// before this is set (i.e. from within the node's own subtree) are recursive.
        defined: bool,
        /// The fully resolved anchored node, available once `defined` is set.
        resolved: Option<Rc<Node>>,
        anchor_name: String,
        line: usize,
        col: usize,
    }

    fn resolve(
        node: &Node,
        anchors: &mut HashMap<usize, AnchorEntry>,
        errors: &mut Vec<Error>,
    ) -> Rc<Node> {
        match node.kind {
            NodeKind::Alias => {
                let Some(entry) = anchors.get_mut(&node.alias_target) else {
                    // Unknown anchor: go-yaml detects it, so treat it as an unresolved alias.
                    return Rc::new(node.clone());
                };
                entry.used = true;
                if entry.defined
                    && let Some(resolved) = entry.resolved.clone()
                {
                    return resolved; // Resolved
                }
                // Don't resolve the recursive alias because it would cause infinite recursion on
                // parsing the tree as `RawYAMLValue`. (#610)
                let name = entry.anchor_name.clone();
                let (tl, tc) = (entry.line, entry.col);
                errors.push(Error::format_at(
                    node.line,
                    node.col,
                    "syntax-check",
                    format_args!(
                        "recursive alias {name:?} is found. anchor was declared at line:{tl}, column:{tc}"
                    ),
                ));
                Rc::new(node.clone())
            }
            NodeKind::Scalar => {
                // A scalar has no content, so its anchor is available immediately.
                let rc = Rc::new(node.clone());
                if node.anchor_id > 0 {
                    anchors.insert(
                        node.anchor_id,
                        AnchorEntry {
                            used: false,
                            defined: true,
                            resolved: Some(rc.clone()),
                            anchor_name: node.anchor.clone().unwrap_or_default(),
                            line: node.line,
                            col: node.col,
                        },
                    );
                }
                rc
            }
            NodeKind::Mapping | NodeKind::Sequence => {
                // Register the anchor before resolving children so that aliases that reference it
                // from within its own subtree are detected as recursive.
                if node.anchor_id > 0 {
                    anchors.insert(
                        node.anchor_id,
                        AnchorEntry {
                            anchor_name: node.anchor.clone().unwrap_or_default(),
                            line: node.line,
                            col: node.col,
                            ..AnchorEntry::default()
                        },
                    );
                }

                let content = node
                    .content
                    .iter()
                    .map(|c| resolve(c, anchors, errors))
                    .collect();
                let rc = Rc::new(Node {
                    content,
                    ..node.clone()
                });

                if node.anchor_id > 0
                    && let Some(entry) = anchors.get_mut(&node.anchor_id)
                {
                    entry.resolved = Some(rc.clone());
                    entry.defined = true;
                }
                rc
            }
        }
    }

    // Keyed by anchor ID; holds the usage state and the resolved anchored node for substitution.
    let mut anchors: HashMap<usize, AnchorEntry> = HashMap::new();
    *root = resolve(root, &mut anchors, errors);

    // Report anchors that were declared but never referenced by any alias.
    let mut unused: Vec<&AnchorEntry> = anchors.values().filter(|e| !e.used).collect();
    unused.sort_by_key(|e| (e.line, e.col));
    for e in unused {
        errors.push(Error::format_at(
            e.line,
            e.col,
            "syntax-check",
            format_args!("anchor {:?} is defined but not used", e.anchor_name),
        ));
    }
}

// ---------------------------------------------------------------------------
// Parser (counterpart of the `parser` struct and its methods)
// ---------------------------------------------------------------------------

/// A key-value entry of a YAML mapping (counterpart of `workflowMappingEntry`).
struct MappingEntry {
    /// The key in lower case for case-insensitive comparisons.
    id: String,
    /// The original key string with its position.
    key: AstString,
    /// The value node.
    val: Rc<Node>,
}

/// The workflow parser, accumulating syntax errors as it walks the node tree.
struct Parser {
    errors: Vec<Error>,
}

impl Parser {
    fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Records an error positioned at the given node.
    fn error(&mut self, n: &Node, m: impl Into<String>) {
        self.errors
            .push(Error::at(n.line, n.col, "syntax-check", m));
    }

    /// Records an error positioned at the given coordinates.
    fn error_at(&mut self, pos: ast::Pos, m: impl Into<String>) {
        self.errors
            .push(Error::at(pos.line, pos.col, "syntax-check", m));
    }

    /// Records a formatted error positioned at the given node.
    fn errorf(&mut self, n: &Node, args: std::fmt::Arguments<'_>) {
        self.errors
            .push(Error::format_at(n.line, n.col, "syntax-check", args));
    }

    /// Records a formatted error positioned at the given coordinates.
    fn errorf_at(&mut self, pos: ast::Pos, args: std::fmt::Arguments<'_>) {
        self.errors
            .push(Error::format_at(pos.line, pos.col, "syntax-check", args));
    }

    /// Resolves all aliases in the tree, recording recursive/unused anchor errors.
    fn resolve_aliases(&mut self, root: &mut Rc<Node>) {
        let mut errors = std::mem::take(&mut self.errors);
        resolve_aliases(root, &mut errors);
        self.errors = errors;
    }

    /// Reports a key that is not allowed in the given section.
    fn unexpected_key(&mut self, s: &AstString, sec: &str, expected: &[&str]) {
        let sec = if sec.contains(' ') {
            sec.to_string()
        } else {
            format!("{sec:?} section")
        };
        let m = match expected.len() {
            1 => format!(
                "expected {:?} key for {sec} but got {:?}",
                expected[0], s.value
            ),
            0 => format!("unexpected key {:?} for {sec}", s.value),
            _ => format!(
                "unexpected key {:?} for {sec}. expected one of {}",
                s.value,
                sorted_quotes(expected.to_vec())
            ),
        };
        self.error_at(s.pos, m);
    }

    /// Checks that a section (a mapping with `len` entries) is not empty.
    fn check_not_empty(&mut self, sec: &str, len: usize, n: &Node) -> bool {
        if len == 0 {
            self.errorf(n, format_args!("{sec:?} section should not be empty"));
            return false;
        }
        true
    }

    /// Checks that `n` is a sequence node, reporting an error otherwise.
    fn check_sequence(&mut self, sec: &str, n: &Node, allow_empty: bool) -> bool {
        if n.kind != NodeKind::Sequence {
            self.errorf(
                n,
                format_args!(
                    "{sec:?} section must be sequence node but got {} node with {:?} tag",
                    n.kind.name(),
                    n.tag
                ),
            );
            return false;
        }
        allow_empty || self.check_not_empty(sec, n.content.len(), n)
    }

    /// Checks that `n` is a scalar node usable as a string value.
    fn check_string(&mut self, n: &Node, allow_empty: bool) -> bool {
        // Do not check the tag is `!!str`: other scalars are handled via their string form.
        if n.kind != NodeKind::Scalar {
            self.errorf(
                n,
                format_args!(
                    "expected scalar node for string value but found {} node with {:?} tag",
                    n.kind.name(),
                    n.tag
                ),
            );
            return false;
        }
        if !allow_empty && n.value.is_empty() {
            self.error(n, "string should not be empty");
            return false;
        }
        true
    }

    /// Reports that a plain text node was found where an expression was expected.
    fn missing_expression(&mut self, n: &Node, expecting: &str) {
        self.errorf(
            n,
            format_args!(
                "expecting a single ${{{{...}}}} expression or {expecting}, but found plain text node"
            ),
        );
    }

    /// Parses `n` as a single `${{ ... }}` expression assignment.
    fn parse_expression(&mut self, n: &Node, expecting: &str) -> Option<AstString> {
        if !is_expr_assigned(&n.value) {
            self.missing_expression(n, expecting);
            return None;
        }
        Some(new_string(n))
    }

    /// Parses `n` as an expression only when it is a string node.
    fn may_parse_expression(n: &Node) -> Option<AstString> {
        if n.tag != "!!str" || !is_expr_assigned(&n.value) {
            return None;
        }
        Some(new_string(n))
    }

    /// Parses `n` as a string value.
    fn parse_string(&mut self, n: &Node, allow_empty: bool) -> AstString {
        if !self.check_string(n, allow_empty) {
            return AstString::new("", false, n.pos());
        }
        new_string(n)
    }

    /// Parses `n` as a sequence of string values.
    fn parse_string_sequence(
        &mut self,
        sec: &str,
        n: &Node,
        allow_empty: bool,
        allow_elem_empty: bool,
    ) -> Vec<AstString> {
        if !self.check_sequence(sec, n, allow_empty) {
            return Vec::new();
        }
        n.content
            .iter()
            .map(|c| self.parse_string(c, allow_elem_empty))
            .collect()
    }

    /// Parses `n` as either a single string or a sequence of strings.
    fn parse_string_or_string_sequence(
        &mut self,
        sec: &str,
        n: &Node,
        allow_empty: bool,
        allow_elem_empty: bool,
    ) -> Vec<AstString> {
        if n.kind == NodeKind::Scalar {
            if allow_empty && n.tag == "!!null" {
                return Vec::new(); // In the case of 'foo:'
            }
            return vec![self.parse_string(n, allow_elem_empty)];
        }
        self.parse_string_sequence(sec, n, allow_empty, allow_elem_empty)
    }

    /// Parses `n` as a boolean value.
    fn parse_bool(&mut self, n: &Node) -> Option<Bool> {
        if n.kind != NodeKind::Scalar || (n.tag != "!!bool" && n.tag != "!!str") {
            self.errorf(
                n,
                format_args!(
                    "expected bool value but found {} node with {:?} tag",
                    n.kind.name(),
                    n.tag
                ),
            );
            return None;
        }

        if n.tag == "!!str" {
            let expression = self.parse_expression(n, "boolean literal \"true\" or \"false\"");
            return Some(Bool {
                value: false,
                expression,
                pos: n.pos(),
            });
        }

        Some(Bool {
            value: n.value == "true",
            expression: None,
            pos: n.pos(),
        })
    }

    /// Parses `n` as an integer value.
    fn parse_int(&mut self, n: &Node) -> Option<Int> {
        if n.kind != NodeKind::Scalar || (n.tag != "!!int" && n.tag != "!!str") {
            self.errorf(
                n,
                format_args!(
                    "expected scalar node for integer value but found {} node with {:?} tag",
                    n.kind.name(),
                    n.tag
                ),
            );
            return None;
        }

        if n.tag == "!!str" {
            let expression = self.parse_expression(n, "integer literal")?;
            return Some(Int {
                value: 0,
                expression: Some(expression),
                pos: n.pos(),
            });
        }

        if let Some(value) = parse_go_int(&n.value) {
            Some(Int {
                value,
                expression: None,
                pos: n.pos(),
            })
        } else {
            {
                self.errorf(
                    n,
                    format_args!(
                        "invalid integer value: {:?}: strconv.Atoi: parsing {:?}: invalid syntax",
                        n.value, n.value
                    ),
                );
                None
            }
        }
    }

    /// Parses `n` as a floating point value.
    fn parse_float(&mut self, n: &Node) -> Option<Float> {
        if n.kind != NodeKind::Scalar
            || (n.tag != "!!float" && n.tag != "!!int" && n.tag != "!!str")
        {
            self.errorf(
                n,
                format_args!(
                    "expected scalar node for float value but found {} node with {:?} tag",
                    n.kind.name(),
                    n.tag
                ),
            );
            return None;
        }

        if n.tag == "!!str" {
            let expression = self.parse_expression(n, "float number literal")?;
            return Some(Float {
                value: 0.0,
                expression: Some(expression),
                pos: n.pos(),
            });
        }

        match parse_go_float(&n.value) {
            Some(value) if !value.is_nan() => Some(Float {
                value,
                expression: None,
                pos: n.pos(),
            }),
            _ => {
                self.errorf(
                    n,
                    format_args!(
                        "invalid float value: {:?}: strconv.ParseFloat: parsing {:?}: invalid syntax",
                        n.value,
                        n.value
                    ),
                );
                None
            }
        }
    }

    /// Parses `n` as a mapping, returning key-value entries.
    fn parse_mapping(
        &mut self,
        where_: &str,
        n: &Node,
        allow_empty: bool,
        case_sensitive: bool,
    ) -> Vec<MappingEntry> {
        if n.kind == NodeKind::Scalar && n.tag == "!!null" {
            if !allow_empty {
                self.errorf(
                    n,
                    format_args!(
                        "{where_} should not be empty. please remove this section if it's unnecessary"
                    ),
                );
            }
            return Vec::new();
        }

        if n.kind != NodeKind::Mapping {
            self.errorf(
                n,
                format_args!(
                    "{where_} is {} node but mapping node is expected",
                    n.kind.name()
                ),
            );
            return Vec::new();
        }

        let mut entries = Vec::new();
        let mut keys: HashMap<String, ast::Pos> = HashMap::new();
        let mut empty = true;
        let mut i = 0;
        while i + 1 < n.content.len() {
            let k_node = &n.content[i];
            let v_node = n.content[i + 1].clone();
            i += 2;
            let k = self.parse_string(k_node, false);

            if k.value == "<<" {
                self.error_at(
                    k.pos,
                    "GitHub Actions does not support YAML merge key \"<<\"",
                );
                continue;
            }

            let id = if case_sensitive {
                k.value.clone()
            } else {
                k.value.to_lowercase()
            };

            if let Some(prev) = keys.get(&id) {
                let note = if case_sensitive {
                    ""
                } else {
                    ". note that this key is case insensitive"
                };
                self.errorf_at(
                    k.pos,
                    format_args!(
                        "key {:?} is duplicated in {where_}. previously defined at {}{note}",
                        k.value, prev
                    ),
                );
                continue;
            }

            keys.insert(id.clone(), k.pos);
            empty = false;
            entries.push(MappingEntry {
                id,
                key: k,
                val: v_node,
            });
        }

        if !allow_empty && empty {
            self.errorf(
                n,
                format_args!(
                    "{where_} should not be empty. please remove this section if it's unnecessary"
                ),
            );
        }

        entries
    }

    /// Parses `n` as a named section mapping.
    fn parse_section_mapping(
        &mut self,
        section: &str,
        n: &Node,
        allow_empty: bool,
        case_sensitive: bool,
    ) -> Vec<MappingEntry> {
        let where_ = format!("{section:?} section");
        self.parse_mapping(&where_, n, allow_empty, case_sensitive)
    }

    /// Parses `n` as a mapping with a custom description.
    fn parse_mapping_at(
        &mut self,
        where_: &str,
        n: &Node,
        allow_empty: bool,
        case_sensitive: bool,
    ) -> Vec<MappingEntry> {
        self.parse_mapping(where_, n, allow_empty, case_sensitive)
    }

    // -- Events -------------------------------------------------------------

    fn parse_schedule_event(&mut self, pos: ast::Pos, n: &Node) -> Option<ScheduledEvent> {
        if !self.check_sequence("schedule", n, false) {
            return None;
        }

        let mut schedules = Vec::with_capacity(n.content.len());
        for c in &n.content {
            let mut cron = None;
            let mut timezone = None;
            for e in self.parse_mapping_at("element of \"schedule\" section", c, false, true) {
                match e.id.as_str() {
                    "cron" => {
                        let s = self.parse_string(&e.val, false);
                        if !s.value.is_empty() {
                            cron = Some(s);
                        }
                    }
                    "timezone" => {
                        let s = self.parse_string(&e.val, false);
                        if !s.value.is_empty() {
                            timezone = Some(s);
                        }
                    }
                    _ => self.unexpected_key(
                        &e.key,
                        "element of \"schedule\" section",
                        &["cron", "timezone"],
                    ),
                }
            }
            if let Some(cron) = cron {
                schedules.push(ScheduleEntry { cron, timezone });
            }
        }

        Some(ScheduledEvent { schedules, pos })
    }

    fn parse_workflow_dispatch_event_input(&mut self, name: AstString, n: &Node) -> DispatchInput {
        let mut ret = DispatchInput {
            name,
            description: None,
            required: None,
            default: None,
            input_type: WorkflowDispatchEventInputType::None,
            options: Vec::new(),
        };

        for e in self.parse_mapping_at("input settings of workflow_dispatch event", n, true, true) {
            match e.id.as_str() {
                "description" => ret.description = Some(self.parse_string(&e.val, true)),
                "required" => ret.required = self.parse_bool(&e.val),
                "default" => ret.default = Some(self.parse_string(&e.val, true)),
                "type" => {
                    if !self.check_string(&e.val, false) {
                        continue;
                    }
                    ret.input_type = match e.val.value.as_str() {
                        "string" => WorkflowDispatchEventInputType::String,
                        "number" => WorkflowDispatchEventInputType::Number,
                        "boolean" => WorkflowDispatchEventInputType::Boolean,
                        "choice" => WorkflowDispatchEventInputType::Choice,
                        "environment" => WorkflowDispatchEventInputType::Environment,
                        _ => {
                            self.errorf(
                                &e.val,
                                format_args!(
                                    "input type of workflow_dispatch event must be one of \"string\", \"number\", \"boolean\", \"choice\", \"environment\" but got {:?}",
                                    e.val.value
                                ),
                            );
                            continue;
                        }
                    };
                }
                "options" => {
                    ret.options = self.parse_string_sequence("options", &e.val, false, false);
                }
                _ => self.unexpected_key(&e.key, "inputs", &["description", "required", "default"]),
            }
        }

        ret
    }

    fn parse_workflow_dispatch_event(&mut self, pos: ast::Pos, n: &Node) -> WorkflowDispatchEvent {
        let mut ret = WorkflowDispatchEvent {
            inputs: BTreeMap::new(),
            pos,
        };

        for e in self.parse_section_mapping("workflow_dispatch", n, true, true) {
            if e.id != "inputs" {
                self.unexpected_key(&e.key, "workflow_dispatch", &["inputs"]);
                continue;
            }
            for ie in self.parse_section_mapping("inputs", &e.val, true, false) {
                let input = self.parse_workflow_dispatch_event_input(ie.key.clone(), &ie.val);
                ret.inputs.insert(ie.id.clone(), input);
            }
        }

        ret
    }

    fn parse_repository_dispatch_event(
        &mut self,
        pos: ast::Pos,
        n: &Node,
    ) -> RepositoryDispatchEvent {
        let mut ret = RepositoryDispatchEvent {
            types: Vec::new(),
            pos,
        };

        // Note: Omitting 'types' is ok. In the case, all types trigger the workflow.
        for e in self.parse_section_mapping("repository_dispatch", n, true, true) {
            if e.id == "types" {
                ret.types = self.parse_string_or_string_sequence("types", &e.val, false, false);
            } else {
                self.unexpected_key(&e.key, "repository_dispatch", &["types"]);
            }
        }

        ret
    }

    fn parse_webhook_event_filter(&mut self, name: AstString, n: &Node) -> WebhookEventFilter {
        let values = self.parse_string_or_string_sequence(&name.value, n, false, false);
        WebhookEventFilter { name, values }
    }

    fn parse_webhook_event(&mut self, name: AstString, n: &Node) -> WebhookEvent {
        let mut ret = WebhookEvent {
            hook: name.clone(),
            types: Vec::new(),
            branches: None,
            branches_ignore: None,
            tags: None,
            tags_ignore: None,
            paths: None,
            paths_ignore: None,
            workflows: Vec::new(),
            pos: name.pos,
        };

        for e in self.parse_section_mapping(&name.value, n, true, true) {
            match e.id.as_str() {
                "types" => {
                    ret.types =
                        self.parse_string_or_string_sequence(&e.key.value, &e.val, false, false);
                }
                "branches" => {
                    ret.branches = Some(self.parse_webhook_event_filter(e.key.clone(), &e.val))
                }
                "branches-ignore" => {
                    ret.branches_ignore =
                        Some(self.parse_webhook_event_filter(e.key.clone(), &e.val))
                }
                "tags" => ret.tags = Some(self.parse_webhook_event_filter(e.key.clone(), &e.val)),
                "tags-ignore" => {
                    ret.tags_ignore = Some(self.parse_webhook_event_filter(e.key.clone(), &e.val))
                }
                "paths" => ret.paths = Some(self.parse_webhook_event_filter(e.key.clone(), &e.val)),
                "paths-ignore" => {
                    ret.paths_ignore = Some(self.parse_webhook_event_filter(e.key.clone(), &e.val))
                }
                "workflows" => {
                    ret.workflows =
                        self.parse_string_or_string_sequence(&e.key.value, &e.val, false, false);
                }
                _ => self.unexpected_key(
                    &e.key,
                    &name.value,
                    &[
                        "types",
                        "branches",
                        "branches-ignore",
                        "tags",
                        "tags-ignore",
                        "paths",
                        "paths-ignore",
                        "workflows",
                    ],
                ),
            }
        }

        ret
    }

    fn parse_workflow_call_event_input(
        &mut self,
        id: String,
        name: AstString,
        n: &Node,
    ) -> WorkflowCallEventInput {
        let mut ret = WorkflowCallEventInput {
            name: name.clone(),
            description: None,
            default: None,
            required: None,
            input_type: WorkflowCallEventInputType::Invalid,
            id,
        };
        let mut typed = false;

        for e in self.parse_mapping_at("input of workflow_call event", n, true, true) {
            match e.id.as_str() {
                "description" => ret.description = Some(self.parse_string(&e.val, true)),
                "required" => ret.required = self.parse_bool(&e.val),
                "default" => ret.default = Some(self.parse_string(&e.val, true)),
                "type" => {
                    typed = true;
                    if !self.check_string(&e.val, false) {
                        continue;
                    }
                    ret.input_type = match e.val.value.as_str() {
                        "boolean" => WorkflowCallEventInputType::Boolean,
                        "number" => WorkflowCallEventInputType::Number,
                        "string" => WorkflowCallEventInputType::String,
                        _ => {
                            self.errorf(
                                &e.val,
                                format_args!(
                                    "invalid value {:?} for input type of workflow_call event. it must be one of \"boolean\", \"number\", or \"string\"",
                                    e.val.value
                                ),
                            );
                            continue;
                        }
                    };
                }
                _ => self.unexpected_key(
                    &e.key,
                    "inputs at workflow_call event",
                    &["description", "required", "default", "type"],
                ),
            }
        }

        if !typed {
            self.errorf_at(
                name.pos,
                format_args!(
                    "\"type\" is missing at {:?} input of workflow_call event",
                    name.value
                ),
            );
        }

        ret
    }

    fn parse_workflow_call_event_secret(
        &mut self,
        name: AstString,
        n: &Node,
    ) -> WorkflowCallEventSecret {
        let mut ret = WorkflowCallEventSecret {
            name,
            description: None,
            required: None,
        };

        for e in self.parse_mapping_at("secret of workflow_call event", n, true, true) {
            match e.id.as_str() {
                "description" => ret.description = Some(self.parse_string(&e.val, true)),
                "required" => ret.required = self.parse_bool(&e.val),
                _ => self.unexpected_key(&e.key, "secrets", &["description", "required"]),
            }
        }

        ret
    }

    fn parse_workflow_call_event_output(
        &mut self,
        name: AstString,
        n: &Node,
    ) -> WorkflowCallEventOutput {
        let mut ret = WorkflowCallEventOutput {
            name: name.clone(),
            description: None,
            value: None,
        };

        for e in self.parse_mapping_at("output of workflow_call event", n, true, true) {
            match e.id.as_str() {
                "description" => ret.description = Some(self.parse_string(&e.val, true)),
                "value" => ret.value = Some(self.parse_string(&e.val, false)),
                _ => self.unexpected_key(
                    &e.key,
                    "outputs at workflow_call event",
                    &["description", "value"],
                ),
            }
        }

        if ret.value.is_none() {
            self.errorf_at(
                name.pos,
                format_args!(
                    "\"value\" is missing at {:?} output of workflow_call event",
                    name.value
                ),
            );
        }

        ret
    }

    fn parse_workflow_call_event(&mut self, pos: ast::Pos, n: &Node) -> WorkflowCallEvent {
        let mut ret = WorkflowCallEvent {
            inputs: Vec::new(),
            secrets: BTreeMap::new(),
            secrets_present: false,
            outputs: BTreeMap::new(),
            pos,
        };

        for e in self.parse_section_mapping("workflow_call", n, true, true) {
            match e.id.as_str() {
                "inputs" => {
                    for ie in self.parse_section_mapping("inputs", &e.val, true, false) {
                        let input = self.parse_workflow_call_event_input(
                            ie.id.clone(),
                            ie.key.clone(),
                            &ie.val,
                        );
                        ret.inputs.push(input);
                    }
                }
                "secrets" => {
                    // Remember `secrets:` was present even when it has no child (null). The
                    // original distinguishes a nil `Secrets` map ("`secrets:` omitted", secrets
                    // may be inherited) from an empty non-nil map ("`secrets:` present but no
                    // child", so the `secrets` context is a strict empty object).
                    ret.secrets_present = true;
                    for se in self.parse_section_mapping("secrets", &e.val, true, false) {
                        let secret = self.parse_workflow_call_event_secret(se.key.clone(), &se.val);
                        ret.secrets.insert(se.id.clone(), secret);
                    }
                }
                "outputs" => {
                    for oe in self.parse_section_mapping("outputs", &e.val, true, false) {
                        let output = self.parse_workflow_call_event_output(oe.key.clone(), &oe.val);
                        ret.outputs.insert(oe.id.clone(), output);
                    }
                }
                _ => {
                    self.unexpected_key(&e.key, "workflow_call", &["inputs", "secrets", "outputs"])
                }
            }
        }

        ret
    }

    fn parse_image_version_event(&mut self, pos: ast::Pos, n: &Node) -> ImageVersionEvent {
        let mut ret = ImageVersionEvent {
            names: Vec::new(),
            versions: Vec::new(),
            pos,
        };

        for e in self.parse_section_mapping("image_version", n, true, true) {
            match e.id.as_str() {
                "names" => ret.names = self.parse_string_sequence("names", &e.val, false, false),
                "versions" => {
                    ret.versions = self.parse_string_sequence("versions", &e.val, false, false)
                }
                _ => self.unexpected_key(&e.key, "image_version", &["names", "versions"]),
            }
        }

        ret
    }

    fn parse_event_with_no_config(&mut self, n: &Node) -> Option<AstEvent> {
        let s = self.parse_string(n, false);
        match s.value.as_str() {
            "" => None,
            "schedule" => {
                self.error(n, "schedule event must be configured with mapping");
                None
            }
            "repository_dispatch" => Some(AstEvent::RepositoryDispatch(RepositoryDispatchEvent {
                types: Vec::new(),
                pos: n.pos(),
            })),
            "workflow_dispatch" => Some(AstEvent::WorkflowDispatch(WorkflowDispatchEvent {
                inputs: BTreeMap::new(),
                pos: n.pos(),
            })),
            "workflow_call" => Some(AstEvent::WorkflowCall(WorkflowCallEvent {
                inputs: Vec::new(),
                secrets: BTreeMap::new(),
                secrets_present: false,
                outputs: BTreeMap::new(),
                pos: n.pos(),
            })),
            "image_version" => Some(AstEvent::ImageVersion(ImageVersionEvent {
                names: Vec::new(),
                versions: Vec::new(),
                pos: n.pos(),
            })),
            _ => Some(AstEvent::Webhook(WebhookEvent {
                hook: s,
                types: Vec::new(),
                branches: None,
                branches_ignore: None,
                tags: None,
                tags_ignore: None,
                paths: None,
                paths_ignore: None,
                workflows: Vec::new(),
                pos: n.pos(),
            })),
        }
    }

    fn parse_events(&mut self, n: &Node) -> Vec<AstEvent> {
        match n.kind {
            NodeKind::Scalar => self.parse_event_with_no_config(n).into_iter().collect(),
            NodeKind::Mapping => {
                let mut ret = Vec::new();
                for e in self.parse_section_mapping("on", n, false, true) {
                    let pos = e.key.pos;
                    match e.id.as_str() {
                        "schedule" => {
                            if let Some(ev) = self.parse_schedule_event(pos, &e.val) {
                                ret.push(AstEvent::Scheduled(ev));
                            }
                        }
                        "workflow_dispatch" => ret.push(AstEvent::WorkflowDispatch(
                            self.parse_workflow_dispatch_event(pos, &e.val),
                        )),
                        "repository_dispatch" => ret.push(AstEvent::RepositoryDispatch(
                            self.parse_repository_dispatch_event(pos, &e.val),
                        )),
                        "workflow_call" => ret.push(AstEvent::WorkflowCall(
                            self.parse_workflow_call_event(pos, &e.val),
                        )),
                        "image_version" => ret.push(AstEvent::ImageVersion(
                            self.parse_image_version_event(pos, &e.val),
                        )),
                        _ => ret.push(AstEvent::Webhook(self.parse_webhook_event(e.key, &e.val))),
                    }
                }
                ret
            }
            NodeKind::Sequence => {
                self.check_not_empty("on", n.content.len(), n);
                let mut ret = Vec::new();
                for c in &n.content {
                    if let Some(ev) = self.parse_event_with_no_config(c) {
                        ret.push(ev);
                    }
                }
                ret
            }
            NodeKind::Alias => {
                self.errorf(
                    n,
                    format_args!(
                        "\"on\" section value is expected to be mapping or sequence but found {} node",
                        n.kind.name()
                    ),
                );
                Vec::new()
            }
        }
    }

    // -- Shared sections ----------------------------------------------------

    fn parse_permissions(&mut self, pos: ast::Pos, n: &Node) -> Permissions {
        let mut ret = Permissions {
            all: None,
            scopes: BTreeMap::new(),
            pos,
        };

        if n.kind == NodeKind::Scalar {
            ret.all = Some(self.parse_string(n, false));
        } else {
            for e in self.parse_section_mapping("permissions", n, true, false) {
                let value = self.parse_string(&e.val, false);
                ret.scopes
                    .insert(e.id, PermissionScope { name: e.key, value });
            }
        }

        ret
    }

    fn parse_env(&mut self, n: &Node) -> Env {
        if n.kind == NodeKind::Scalar {
            return Env {
                vars: BTreeMap::new(),
                expression: self.parse_expression(n, "mapping value for \"env\" section"),
            };
        }

        let mut vars = BTreeMap::new();
        for e in self.parse_section_mapping("env", n, false, false) {
            let value = self.parse_string(&e.val, true);
            vars.insert(e.id, EnvVar { name: e.key, value });
        }

        Env {
            vars,
            expression: None,
        }
    }

    fn parse_defaults(&mut self, pos: ast::Pos, n: &Node) -> Defaults {
        let mut ret = Defaults { run: None, pos };

        for e in self.parse_section_mapping("defaults", n, false, true) {
            if e.id != "run" {
                self.unexpected_key(&e.key, "defaults", &["run"]);
                continue;
            }
            let mut run = DefaultsRun {
                shell: None,
                working_directory: None,
                pos: e.key.pos,
            };
            for re in self.parse_section_mapping("run", &e.val, false, true) {
                match re.id.as_str() {
                    "shell" => run.shell = Some(self.parse_string(&re.val, false)),
                    "working-directory" => {
                        run.working_directory = Some(self.parse_string(&re.val, false))
                    }
                    _ => self.unexpected_key(&re.key, "run", &["shell", "working-directory"]),
                }
            }
            ret.run = Some(run);
        }

        if ret.run.is_none() {
            self.error(n, "\"defaults\" section should have \"run\" section");
        }

        ret
    }

    fn parse_concurrency(&mut self, pos: ast::Pos, n: &Node) -> Concurrency {
        let mut ret = Concurrency {
            group: AstString::new("", false, pos),
            cancel_in_progress: None,
            pos,
        };

        if n.kind == NodeKind::Scalar {
            ret.group = self.parse_string(n, false);
            return ret;
        }

        for e in self.parse_section_mapping("concurrency", n, false, true) {
            match e.id.as_str() {
                "group" => ret.group = self.parse_string(&e.val, false),
                "cancel-in-progress" => ret.cancel_in_progress = self.parse_bool(&e.val),
                _ => self.unexpected_key(&e.key, "concurrency", &["group", "cancel-in-progress"]),
            }
        }
        if ret.group.value.is_empty() {
            self.error_at(pos, "group name is missing in \"concurrency\" section");
        }
        ret
    }

    fn parse_environment(&mut self, pos: ast::Pos, n: &Node) -> Environment {
        let mut ret = Environment {
            name: AstString::new("", false, pos),
            url: None,
            deployment: None,
            pos,
        };

        if n.kind == NodeKind::Scalar {
            ret.name = self.parse_string(n, false);
            return ret;
        }

        for e in self.parse_section_mapping("environment", n, false, true) {
            match e.id.as_str() {
                "name" => ret.name = self.parse_string(&e.val, false),
                "url" => ret.url = Some(self.parse_string(&e.val, false)),
                "deployment" => ret.deployment = self.parse_bool(&e.val),
                _ => self.unexpected_key(&e.key, "environment", &["deployment", "name", "url"]),
            }
        }
        if ret.name.value.is_empty() {
            self.error_at(pos, "name is missing in \"environment\" section");
        }
        ret
    }

    fn parse_outputs(&mut self, n: &Node) -> BTreeMap<String, Output> {
        let mut ret = BTreeMap::new();
        for e in self.parse_section_mapping("outputs", n, false, false) {
            let value = self.parse_string(&e.val, true);
            ret.insert(e.id, Output { name: e.key, value });
        }
        self.check_not_empty("outputs", ret.len(), n);
        ret
    }

    // -- Matrix / strategy --------------------------------------------------

    fn parse_raw_yaml_value(&mut self, n: &Node) -> Option<RawYAMLValue> {
        match n.kind {
            NodeKind::Scalar => Some(RawYAMLValue::String(RawYAMLString {
                value: n.value.clone(),
                pos: n.pos(),
            })),
            NodeKind::Sequence => {
                let mut elems = Vec::with_capacity(n.content.len());
                for c in &n.content {
                    if let Some(v) = self.parse_raw_yaml_value(c) {
                        elems.push(v);
                    }
                }
                Some(RawYAMLValue::Array(RawYAMLArray {
                    elems,
                    pos: n.pos(),
                }))
            }
            NodeKind::Mapping => {
                let mut props = BTreeMap::new();
                for e in self.parse_mapping_at("matrix row value", n, true, false) {
                    if let Some(v) = self.parse_raw_yaml_value(&e.val) {
                        props.insert(e.id, v);
                    }
                }
                Some(RawYAMLValue::Object(RawYAMLObject {
                    props,
                    pos: n.pos(),
                }))
            }
            NodeKind::Alias => {
                self.errorf(
                    n,
                    format_args!(
                        "unexpected {} node on parsing value in matrix row",
                        n.kind.name()
                    ),
                );
                None
            }
        }
    }

    fn parse_matrix_combinations(&mut self, sec: &str, n: &Node) -> Option<MatrixCombinations> {
        if n.kind == NodeKind::Scalar {
            return Some(MatrixCombinations {
                combinations: Vec::new(),
                expression: self.parse_expression(n, "array of matrix combination"),
            });
        }

        if !self.check_sequence(sec, n, false) {
            return None;
        }

        let where_ = format!("element in {sec:?} section");
        let mut combinations = Vec::with_capacity(n.content.len());
        for c in &n.content {
            if c.kind == NodeKind::Scalar {
                let expression = self.parse_expression(c, "mapping of matrix combination");
                combinations.push(MatrixCombination {
                    assigns: BTreeMap::new(),
                    expression,
                });
                continue;
            }

            let mut assigns = BTreeMap::new();
            for e in self.parse_mapping(&where_, c, false, false) {
                if let Some(v) = self.parse_raw_yaml_value(&e.val) {
                    assigns.insert(
                        e.id,
                        MatrixAssign {
                            key: e.key,
                            value: v,
                        },
                    );
                }
            }
            combinations.push(MatrixCombination {
                assigns,
                expression: None,
            });
        }
        Some(MatrixCombinations {
            combinations,
            expression: None,
        })
    }

    fn parse_matrix(&mut self, pos: ast::Pos, n: &Node) -> Matrix {
        if n.kind == NodeKind::Scalar {
            return Matrix {
                rows: BTreeMap::new(),
                include: None,
                exclude: None,
                expression: self.parse_expression(n, "matrix"),
                pos: n.pos(),
            };
        }

        let mut ret = Matrix {
            rows: BTreeMap::new(),
            include: None,
            exclude: None,
            expression: None,
            pos,
        };

        for e in self.parse_section_mapping("matrix", n, false, false) {
            match e.id.as_str() {
                "include" => ret.include = self.parse_matrix_combinations("include", &e.val),
                "exclude" => ret.exclude = self.parse_matrix_combinations("exclude", &e.val),
                _ => {
                    if e.val.kind == NodeKind::Scalar {
                        let expression =
                            self.parse_expression(&e.val, "array value for matrix variations");
                        ret.rows.insert(
                            e.id,
                            MatrixRow {
                                name: e.key,
                                values: Vec::new(),
                                expression,
                            },
                        );
                        continue;
                    }

                    if !self.check_sequence("matrix values", &e.val, false) {
                        continue;
                    }

                    let mut values = Vec::with_capacity(e.val.content.len());
                    for c in &e.val.content {
                        if let Some(v) = self.parse_raw_yaml_value(c) {
                            values.push(v);
                        }
                    }
                    ret.rows.insert(
                        e.id,
                        MatrixRow {
                            name: e.key,
                            values,
                            expression: None,
                        },
                    );
                }
            }
        }

        ret
    }

    fn parse_max_parallel(&mut self, n: &Node) -> Option<Int> {
        let i = self.parse_int(n);
        if let Some(i) = &i
            && i.expression.is_none()
            && i.value <= 0
        {
            self.errorf(
                n,
                format_args!(
                    "value at \"max-parallel\" must be greater than zero: {}",
                    i.value
                ),
            );
        }
        i
    }

    fn parse_strategy(&mut self, pos: ast::Pos, n: &Node) -> Strategy {
        let mut ret = Strategy {
            matrix: None,
            fail_fast: None,
            max_parallel: None,
            pos,
        };

        for e in self.parse_section_mapping("strategy", n, false, true) {
            match e.id.as_str() {
                "matrix" => ret.matrix = Some(self.parse_matrix(e.key.pos, &e.val)),
                "fail-fast" => ret.fail_fast = self.parse_bool(&e.val),
                "max-parallel" => ret.max_parallel = self.parse_max_parallel(&e.val),
                _ => self.unexpected_key(
                    &e.key,
                    "strategy",
                    &["matrix", "fail-fast", "max-parallel"],
                ),
            }
        }

        ret
    }

    fn parse_credentials(&mut self, pos: ast::Pos, n: &Node) -> Option<Credentials> {
        let mut ret = Credentials {
            username: None,
            password: None,
            expression: None,
            pos,
        };

        if let Some(e) = Self::may_parse_expression(n) {
            ret.expression = Some(e);
            return Some(ret);
        }

        for e in self.parse_section_mapping("credentials", n, false, true) {
            match e.id.as_str() {
                "username" => ret.username = Some(self.parse_string(&e.val, false)),
                "password" => ret.password = Some(self.parse_string(&e.val, false)),
                _ => self.unexpected_key(&e.key, "credentials", &["username", "password"]),
            }
        }

        if ret.username.is_none() || ret.password.is_none() {
            self.error_at(
                pos,
                "both \"username\" and \"password\" must be specified in \"credentials\" section",
            );
            return None;
        }

        Some(ret)
    }

    /// Parses a container section. Note that the original assigns the parsed `volumes` list to the
    /// `Ports` field by mistake; this port fixes that bug and assigns to `volumes`.
    fn parse_container(&mut self, sec: &str, pos: ast::Pos, n: &Node) -> Container {
        let mut ret = Container {
            image: AstString::new("", false, pos),
            credentials: None,
            env: None,
            ports: Vec::new(),
            volumes: Vec::new(),
            options: None,
            command: None,
            entrypoint: None,
            pos,
        };
        let mut keys = vec!["image", "credentials", "env", "ports", "volumes", "options"];
        let is_services = sec == "services";
        let mut image_set = false;
        if is_services {
            keys.push("command");
            keys.push("entrypoint");
        }

        if n.kind == NodeKind::Scalar {
            // When you only specify a container image, you can omit the image keyword.
            ret.image = self.parse_string(n, false);
            return ret;
        }

        for e in self.parse_section_mapping(sec, n, false, true) {
            match e.id.as_str() {
                "image" => {
                    ret.image = self.parse_string(&e.val, false);
                    image_set = true;
                }
                "credentials" => ret.credentials = self.parse_credentials(e.key.pos, &e.val),
                "env" => ret.env = Some(self.parse_env(&e.val)),
                "ports" => {
                    ret.ports = self.parse_string_sequence("ports", &e.val, true, false);
                }
                // Fix of the original bug that assigned this to `Ports`.
                "volumes" => {
                    ret.volumes = self.parse_string_sequence("volumes", &e.val, true, false);
                }
                "options" => ret.options = Some(self.parse_string(&e.val, true)),
                "command" => {
                    if is_services {
                        ret.command = Some(self.parse_string(&e.val, true));
                    } else {
                        self.unexpected_key(&e.key, sec, &keys);
                    }
                }
                "entrypoint" => {
                    if is_services {
                        ret.entrypoint = Some(self.parse_string(&e.val, true));
                    } else {
                        self.unexpected_key(&e.key, sec, &keys);
                    }
                }
                _ => self.unexpected_key(&e.key, sec, &keys),
            }
        }

        // Go checks `ret.Image == nil`, i.e. whether the `image` key was present at all. An image
        // that is present but empty (`image: ''`) is reported by `parse_string` instead.
        if !image_set {
            self.errorf_at(pos, format_args!("\"image\" is missing in {sec:?} section"));
        }

        ret
    }

    fn parse_services(&mut self, n: &Node) -> Services {
        let mut ret = Services {
            value: BTreeMap::new(),
            expression: None,
            pos: n.pos(),
        };

        if let Some(e) = Self::may_parse_expression(n) {
            ret.expression = Some(e);
            return ret;
        }

        for e in self.parse_section_mapping("services", n, false, false) {
            let container = self.parse_container("services", e.key.pos, &e.val);
            ret.value.insert(
                e.id,
                Service {
                    name: e.key,
                    container,
                },
            );
        }
        ret
    }

    fn parse_timeout_minutes(&mut self, n: &Node) -> Option<Float> {
        let f = self.parse_float(n);
        if let Some(f) = &f
            && f.expression.is_none()
            && f.value <= 0.0
        {
            self.errorf(
                n,
                format_args!(
                    "value at \"timeout-minutes\" must be greater than zero: {}",
                    format_float(f.value)
                ),
            );
        }
        f
    }

    // -- Steps --------------------------------------------------------------

    fn parse_step_exec_action(&mut self, entries: &[MappingEntry], is_docker: bool) -> ExecAction {
        let mut ret = ExecAction {
            uses: AstString::new("", false, ast::Pos::default()),
            inputs: BTreeMap::new(),
            entrypoint: None,
            args: None,
        };

        for e in entries {
            match e.id.as_str() {
                "uses" => ret.uses = self.parse_string(&e.val, false),
                "with" => {
                    for we in self.parse_section_mapping("with", &e.val, false, false) {
                        match (is_docker, we.id.as_str()) {
                            (true, "entrypoint") => {
                                ret.entrypoint = Some(self.parse_string(&we.val, false))
                            }
                            (true, "args") => ret.args = Some(self.parse_string(&we.val, true)),
                            _ => {
                                let value = self.parse_string(&we.val, true);
                                ret.inputs.insert(
                                    we.id,
                                    Input {
                                        name: we.key,
                                        value,
                                    },
                                );
                            }
                        }
                    }
                }
                "id" | "if" | "name" | "env" | "continue-on-error" | "timeout-minutes" => {}
                _ => self.unexpected_key(
                    &e.key,
                    "step to execute action",
                    &[
                        "id",
                        "if",
                        "name",
                        "env",
                        "continue-on-error",
                        "timeout-minutes",
                        "uses",
                        "with",
                    ],
                ),
            }
        }

        ret
    }

    fn parse_step_exec_run(&mut self, entries: &[MappingEntry]) -> ExecRun {
        let mut ret = ExecRun {
            run: AstString::new("", false, ast::Pos::default()),
            shell: None,
            working_directory: None,
            run_pos: ast::Pos::default(),
        };

        for e in entries {
            match e.id.as_str() {
                "run" => {
                    ret.run = self.parse_string(&e.val, false);
                    ret.run_pos = e.key.pos;
                }
                "shell" => ret.shell = Some(self.parse_string(&e.val, false)),
                "working-directory" => {
                    ret.working_directory = Some(self.parse_string(&e.val, false))
                }
                "id" | "if" | "name" | "env" | "continue-on-error" | "timeout-minutes" => {}
                _ => self.unexpected_key(
                    &e.key,
                    "step to run shell command",
                    &[
                        "id",
                        "if",
                        "name",
                        "env",
                        "continue-on-error",
                        "timeout-minutes",
                        "run",
                        "shell",
                        "working-directory",
                    ],
                ),
            }
        }

        ret
    }

    fn parse_step(&mut self, n: &Node) -> Step {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Kind {
            Unknown,
            Action,
            Docker,
            Run,
        }

        let mut ret = Step {
            id: None,
            if_condition: None,
            name: None,
            exec: Exec::Action(ExecAction {
                uses: AstString::new("", false, ast::Pos::default()),
                inputs: BTreeMap::new(),
                entrypoint: None,
                args: None,
            }),
            env: None,
            continue_on_error: None,
            timeout_minutes: None,
            pos: n.pos(),
        };

        let mut kind = Kind::Unknown;
        let entries = self.parse_mapping_at("element of \"steps\" section", n, false, true);
        for e in &entries {
            match e.id.as_str() {
                "id" => ret.id = Some(self.parse_string(&e.val, false)),
                "if" => ret.if_condition = Some(self.parse_string(&e.val, false)),
                "name" => ret.name = Some(self.parse_string(&e.val, true)),
                "env" => ret.env = Some(self.parse_env(&e.val)),
                "continue-on-error" => ret.continue_on_error = self.parse_bool(&e.val),
                "timeout-minutes" => ret.timeout_minutes = self.parse_timeout_minutes(&e.val),
                "uses" => {
                    kind = if e.val.value.starts_with("docker://") {
                        Kind::Docker
                    } else {
                        Kind::Action
                    };
                }
                "run" => kind = Kind::Run,
                _ => {}
            }
        }

        match kind {
            Kind::Action | Kind::Docker => {
                ret.exec =
                    Exec::Action(self.parse_step_exec_action(&entries, kind == Kind::Docker));
            }
            Kind::Run => ret.exec = Exec::Run(self.parse_step_exec_run(&entries)),
            Kind::Unknown => {
                self.error(
                    n,
                    "step must run script with \"run\" section or run action with \"uses\" section",
                );
            }
        }

        ret
    }

    fn parse_steps(&mut self, n: &Node) -> Vec<Step> {
        if !self.check_sequence("steps", n, false) {
            return Vec::new();
        }
        n.content.iter().map(|c| self.parse_step(c)).collect()
    }

    fn parse_runs_on(&mut self, n: &Node) -> Runner {
        if let Some(expr) = Self::may_parse_expression(n) {
            return Runner {
                labels: Vec::new(),
                labels_expr: Some(expr),
                group: None,
            };
        }

        if n.kind == NodeKind::Scalar || n.kind == NodeKind::Sequence {
            let labels = self.parse_string_or_string_sequence("runs-on", n, false, false);
            return Runner {
                labels,
                labels_expr: None,
                group: None,
            };
        }

        let mut r = Runner {
            labels: Vec::new(),
            labels_expr: None,
            group: None,
        };
        for e in self.parse_section_mapping("runs-on", n, false, true) {
            match e.id.as_str() {
                "labels" => {
                    if let Some(expr) = Self::may_parse_expression(&e.val) {
                        r.labels_expr = Some(expr);
                        continue;
                    }
                    r.labels = self.parse_string_or_string_sequence("labels", &e.val, false, false);
                }
                "group" => r.group = Some(self.parse_string(&e.val, false)),
                _ => self.unexpected_key(&e.key, "runs-on", &["labels", "group"]),
            }
        }

        r
    }

    fn parse_snapshot(&mut self, pos: ast::Pos, n: &Node) -> Option<Snapshot> {
        match n.kind {
            NodeKind::Scalar => Some(Snapshot {
                image_name: self.parse_string(n, false),
                version: None,
                if_condition: None,
            }),
            NodeKind::Mapping => {
                let mut ret = Snapshot {
                    image_name: AstString::new("", false, pos),
                    version: None,
                    if_condition: None,
                };
                for e in self.parse_section_mapping("on", n, false, true) {
                    match e.id.as_str() {
                        "image-name" => ret.image_name = self.parse_string(&e.val, false),
                        "version" => ret.version = Some(self.parse_string(&e.val, false)),
                        "if" => ret.if_condition = Some(self.parse_string(&e.val, false)),
                        _ => self.unexpected_key(
                            &e.key,
                            "snapshot",
                            &["image-name", "version", "if"],
                        ),
                    }
                }
                if ret.image_name.value.is_empty() {
                    self.error_at(
                        pos,
                        "\"snapshot\" section must have \"image-name\" configuration",
                    );
                }
                Some(ret)
            }
            _ => {
                self.errorf(
                    n,
                    format_args!(
                        "\"snapshot\" section value must be string or mapping but found {} node",
                        n.kind.name()
                    ),
                );
                None
            }
        }
    }

    // -- Jobs ---------------------------------------------------------------

    fn parse_job(&mut self, id: AstString, n: &Node) -> Job {
        let mut ret = Job {
            id: id.clone(),
            name: None,
            needs: Vec::new(),
            runs_on: None,
            permissions: None,
            environment: None,
            concurrency: None,
            outputs: BTreeMap::new(),
            env: None,
            defaults: None,
            if_condition: None,
            steps: Vec::new(),
            timeout_minutes: None,
            strategy: None,
            continue_on_error: None,
            container: None,
            services: None,
            workflow_call: None,
            snapshot: None,
            pos: id.pos,
        };
        let mut call = WorkflowCall {
            uses: AstString::new("", false, ast::Pos::default()),
            uses_present: false,
            inputs: BTreeMap::new(),
            secrets: BTreeMap::new(),
            inherit_secrets: false,
        };
        let mut steps_only_key: Option<AstString> = None;
        let mut call_only_key: Option<AstString> = None;

        let where_ = format!("{:?} job", id.value);
        for e in self.parse_mapping(&where_, n, false, true) {
            let k = e.key.clone();
            let v = e.val.clone();
            match e.id.as_str() {
                "name" => ret.name = Some(self.parse_string(&v, true)),
                "needs" => {
                    ret.needs = if v.kind == NodeKind::Scalar {
                        vec![self.parse_string(&v, false)]
                    } else {
                        self.parse_string_sequence("needs", &v, false, false)
                    };
                }
                "runs-on" => {
                    ret.runs_on = Some(self.parse_runs_on(&v));
                    steps_only_key = Some(k);
                }
                "permissions" => {
                    ret.permissions = Some(self.parse_permissions(k.pos, &v));
                }
                "environment" => {
                    ret.environment = Some(self.parse_environment(k.pos, &v));
                    steps_only_key = Some(k);
                }
                "concurrency" => {
                    ret.concurrency = Some(self.parse_concurrency(k.pos, &v));
                }
                "outputs" => {
                    ret.outputs = self.parse_outputs(&v);
                    steps_only_key = Some(k);
                }
                "env" => {
                    ret.env = Some(self.parse_env(&v));
                    steps_only_key = Some(k);
                }
                "defaults" => {
                    ret.defaults = Some(self.parse_defaults(k.pos, &v));
                    steps_only_key = Some(k);
                }
                "if" => ret.if_condition = Some(self.parse_string(&v, false)),
                "steps" => {
                    ret.steps = self.parse_steps(&v);
                    steps_only_key = Some(k);
                }
                "timeout-minutes" => {
                    ret.timeout_minutes = self.parse_timeout_minutes(&v);
                    steps_only_key = Some(k);
                }
                "strategy" => {
                    ret.strategy = Some(self.parse_strategy(k.pos, &v));
                }
                "continue-on-error" => {
                    ret.continue_on_error = self.parse_bool(&v);
                    steps_only_key = Some(k);
                }
                "container" => {
                    ret.container = Some(self.parse_container("container", k.pos, &v));
                    steps_only_key = Some(k);
                }
                "services" => {
                    ret.services = Some(self.parse_services(&v));
                }
                "uses" => {
                    call.uses = self.parse_string(&v, false);
                    // Remember `uses:` was present even when its value is empty (null). The
                    // original tracks this with a non-nil `*String` pointer; without this flag an
                    // empty `uses:` would be mistaken for "no `uses` at all", wrongly requiring
                    // `steps`/`runs-on` and rejecting the call-only key.
                    call.uses_present = true;
                    call_only_key = Some(k);
                }
                "with" => {
                    for we in self.parse_section_mapping("with", &v, false, false) {
                        let value = self.parse_string(&we.val, true);
                        call.inputs.insert(
                            we.id,
                            WorkflowCallInput {
                                name: we.key,
                                value,
                            },
                        );
                    }
                    call_only_key = Some(k);
                }
                "secrets" => {
                    if v.kind == NodeKind::Scalar {
                        // `secrets: inherit` special case.
                        if v.value == "inherit" {
                            call.inherit_secrets = true;
                        } else {
                            self.errorf(
                                &v,
                                format_args!(
                                    "expected mapping node for secrets or \"inherit\" string node but found {:?} node",
                                    v.value
                                ),
                            );
                        }
                    } else {
                        for se in self.parse_section_mapping("secrets", &v, false, false) {
                            let value = self.parse_string(&se.val, true);
                            call.secrets.insert(
                                se.id,
                                WorkflowCallSecret {
                                    name: se.key,
                                    value,
                                },
                            );
                        }
                    }
                    call_only_key = Some(k);
                }
                "snapshot" => {
                    ret.snapshot = self.parse_snapshot(k.pos, &v);
                }
                _ => self.unexpected_key(
                    &k,
                    "job",
                    &[
                        "name",
                        "needs",
                        "runs-on",
                        "permissions",
                        "environment",
                        "concurrency",
                        "outputs",
                        "env",
                        "defaults",
                        "if",
                        "steps",
                        "timeout-minutes",
                        "strategy",
                        "continue-on-error",
                        "container",
                        "services",
                        "uses",
                        "with",
                        "secrets",
                        "snapshot",
                    ],
                ),
            }
        }

        // Gate on whether `uses:` was present, not on whether its value is empty. The original
        // checks `call.Uses != nil`, which is true even for `uses:` with a null value; such a job
        // is still a (malformed) call job, so `steps`/`runs-on` must not be required and the
        // call-only key must not be rejected.
        if call.uses_present {
            if let Some(k) = steps_only_key {
                self.errorf_at(
                    k.pos,
                    format_args!(
                        "when a reusable workflow is called with \"uses\", {:?} is not available. only following keys are allowed: \"name\", \"uses\", \"with\", \"secrets\", \"needs\", \"if\", and \"permissions\" in job {:?}",
                        k.value,
                        id.value
                    ),
                );
            } else {
                ret.workflow_call = Some(call);
            }
        } else {
            // When not a reusable call.
            if ret.steps.is_empty() {
                self.errorf_at(
                    id.pos,
                    format_args!("\"steps\" section is missing in job {:?}", id.value),
                );
            }
            if ret.runs_on.is_none() {
                self.errorf_at(
                    id.pos,
                    format_args!("\"runs-on\" section is missing in job {:?}", id.value),
                );
            }
            if let Some(k) = call_only_key {
                self.errorf_at(
                    k.pos,
                    format_args!(
                        "{:?} is only available for a reusable workflow call with \"uses\" but \"uses\" is not found in job {:?}",
                        k.value,
                        id.value
                    ),
                );
            }
        }

        ret
    }

    fn parse_jobs(&mut self, n: &Node) -> BTreeMap<String, Job> {
        let mut ret = BTreeMap::new();
        for e in self.parse_section_mapping("jobs", n, false, false) {
            let job = self.parse_job(e.key.clone(), &e.val);
            ret.insert(e.id, job);
        }
        ret
    }

    // -- Workflow -----------------------------------------------------------

    fn parse_workflow(&mut self, n: &Node) -> Workflow {
        let mut w = Workflow {
            name: None,
            run_name: None,
            on: Vec::new(),
            permissions: None,
            env: None,
            defaults: None,
            concurrency: None,
            jobs: BTreeMap::new(),
        };
        let mut on_missing = true;
        let mut jobs_missing = true;

        if n.content.is_empty() {
            self.error(n, "workflow is empty");
            return w;
        }

        for e in self.parse_section_mapping("workflow", &n.content[0], false, true) {
            let k = e.key.clone();
            let v = e.val.clone();
            match e.id.as_str() {
                "name" => w.name = Some(self.parse_string(&v, true)),
                "on" => {
                    w.on = self.parse_events(&v);
                    on_missing = false;
                }
                "permissions" => w.permissions = Some(self.parse_permissions(k.pos, &v)),
                "env" => w.env = Some(self.parse_env(&v)),
                "defaults" => w.defaults = Some(self.parse_defaults(k.pos, &v)),
                "concurrency" => w.concurrency = Some(self.parse_concurrency(k.pos, &v)),
                "jobs" => {
                    w.jobs = self.parse_jobs(&v);
                    jobs_missing = false;
                }
                "run-name" => w.run_name = Some(self.parse_string(&v, false)),
                _ => self.unexpected_key(
                    &k,
                    "workflow",
                    &[
                        "name",
                        "run-name",
                        "on",
                        "permissions",
                        "env",
                        "defaults",
                        "concurrency",
                        "jobs",
                    ],
                ),
            }
        }

        if on_missing {
            self.error(n, "\"on\" section is missing in workflow");
        }
        if jobs_missing {
            self.error(n, "\"jobs\" section is missing in workflow");
        }

        w
    }
}

/// Formats a float the way Go's `%v` verb does for the messages that report an invalid value.
fn format_float(f: f64) -> String {
    #[allow(clippy::float_cmp)] // whole-number check against trunc() is intentional
    if f == f.trunc() && f.is_finite() {
        format!("{f:.0}")
    } else {
        format!("{f}")
    }
}

/// Converts a `ScanError` from saphyr into the errors reported for unparseable YAML. This mirrors
/// `handleYAMLUnmarshalError` (which reports parser errors with their position).
fn handle_yaml_error(text: &str, err: &saphyr_parser::ScanError) -> Vec<Error> {
    let marker = err.marker();
    // Go's yaml.v3 reports an unresolved alias as a top-level error at 0:0 with the message
    // `yaml: unknown anchor '<name>' referenced`. saphyr instead reports it as a positioned parse
    // error whose message omits the anchor name, so that case is rewritten to match Go by reading
    // the alias name out of the source at the marker position.
    if err.info().contains("unknown anchor")
        && let Some(name) = alias_name_at(text, marker.line(), marker.col())
    {
        return vec![Error::at(
            0,
            0,
            "syntax-check",
            format!("could not parse as YAML: yaml: unknown anchor '{name}' referenced"),
        )];
    }
    // saphyr's `marker.col()` is 0-based and matches the column that Go's yaml.v3 reports in
    // `ParserError.Column` / `TypeError`, which actionlint passes through unchanged. So no +1.
    vec![Error::format_at(
        marker.line(),
        marker.col(),
        "syntax-check",
        format_args!("could not parse as YAML: {}", err.info()),
    )]
}

/// Extracts the alias (anchor) name referenced by a `*name` node at the given 1-based line and
/// 0-based column in `source`. Used to recover the anchor name that saphyr's unknown-anchor error
/// omits.
fn alias_name_at(source: &str, line: usize, col: usize) -> Option<String> {
    let line_text = source.lines().nth(line.checked_sub(1)?)?;
    let bytes = line_text.as_bytes();
    let mut i = col.min(bytes.len());
    // The marker sits on (or just after) the `*` that introduces the alias; skip it if present.
    if i < bytes.len() && bytes[i] == b'*' {
        i += 1;
    }
    let start = i;
    while i < bytes.len()
        && !bytes[i].is_ascii_whitespace()
        && !matches!(bytes[i], b',' | b'}' | b']')
    {
        i += 1;
    }
    if start == i {
        None
    } else {
        Some(line_text[start..i].to_owned())
    }
}

/// Parses the given workflow YAML source into a workflow syntax tree. Returns the parsed
/// [`Workflow`] (or `None` when the source cannot be parsed into a workflow at all) and the list of
/// detected syntax errors.
#[must_use]
pub fn parse(content: &[u8]) -> (Option<Workflow>, Vec<Error>) {
    let Ok(text) = std::str::from_utf8(content) else {
        return (
            None,
            vec![Error::at(
                0,
                0,
                "syntax-check",
                "could not parse as YAML: invalid UTF-8",
            )],
        );
    };

    let root = match build_tree(text) {
        Ok(root) => root,
        Err(err) => return (None, handle_yaml_error(text, &err)),
    };

    // go-yaml wraps the root node in a `DocumentNode` whose first content entry is the root
    // mapping. `parse` reads `n.Content[0]`, so mirror that with a synthetic document node.
    // An empty (or null-only) document has no content, which makes `parse` report
    // "workflow is empty" just like go-yaml's empty `DocumentNode.Content`.
    let (root_line, root_col) = (root.line.max(1), root.col.max(1));
    let content = if root.kind == NodeKind::Scalar && root.tag == "!!null" {
        Vec::new()
    } else {
        vec![Rc::new(root)]
    };
    let document = Node {
        kind: NodeKind::Mapping,
        tag: String::new(),
        style: Style::Plain,
        value: String::new(),
        line: root_line,
        col: root_col,
        anchor: None,
        anchor_id: 0,
        content,
        alias_target: 0,
    };
    let mut doc = Rc::new(document);

    let mut p = Parser::new();
    p.resolve_aliases(&mut doc);

    let workflow = p.parse_workflow(&doc);
    (Some(workflow), p.errors)
}

/// Creates an [`AstString`] from a scalar node.
fn new_string(n: &Node) -> AstString {
    AstString::new(n.value.clone(), n.style == Style::Quoted, n.pos())
}

/// Whether a single `${{ ... }}` expression is assigned to the string.
fn is_expr_assigned(s: &str) -> bool {
    let v = s.trim();
    v.starts_with("${{") && v.ends_with("}}") && v.matches("${{").count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errs(src: &str) -> Vec<String> {
        let (_, es) = parse(src.as_bytes());
        es.iter().map(std::string::ToString::to_string).collect()
    }

    fn assert_no_err(src: &str) {
        let (w, es) = parse(src.as_bytes());
        assert!(es.is_empty(), "unexpected errors: {es:?}");
        assert!(w.is_some());
    }

    #[test]
    fn minimal_workflow() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo hello
";
        let (w, es) = parse(src.as_bytes());
        assert!(es.is_empty(), "{es:?}");
        let w = w.unwrap();
        assert_eq!(w.on.len(), 1);
        assert_eq!(w.on[0].event_name(), "push");
        assert_eq!(w.jobs.len(), 1);
        let job = w.jobs.get("test").unwrap();
        assert_eq!(
            job.runs_on
                .as_ref()
                .unwrap()
                .labels
                .iter()
                .map(|s| s.value.clone())
                .collect::<Vec<_>>(),
            vec!["ubuntu-latest"]
        );
        assert_eq!(job.steps.len(), 1);
    }

    #[test]
    fn positions_are_correct() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo
";
        let (_, es) = parse(src.as_bytes());
        assert!(es.is_empty(), "{es:?}");
        // `runs-on` is on line 4, column 5.
        let (w, _) = parse(src.as_bytes());
        let w = w.unwrap();
        let job = w.jobs.get("test").unwrap();
        assert_eq!(job.pos, ast::Pos::new(3, 3));
        assert_eq!(
            job.runs_on.as_ref().unwrap().labels[0].value,
            "ubuntu-latest"
        );
    }

    #[test]
    fn empty_workflow() {
        let es = errs("");
        assert_eq!(es.len(), 1, "{es:?}");
        assert!(es[0].contains("workflow is empty"), "{es:?}");
    }

    #[test]
    fn missing_on_and_jobs() {
        let src = "name: foo\n";
        let es = errs(src);
        assert!(es.iter().any(|e| e.contains("\"on\" section is missing")));
        assert!(es.iter().any(|e| e.contains("\"jobs\" section is missing")));
    }

    #[test]
    fn missing_steps_and_runs_on() {
        let src = "\
on: push
jobs:
  test:
    name: test
";
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("\"steps\" section is missing"))
        );
        assert!(
            es.iter()
                .any(|e| e.contains("\"runs-on\" section is missing"))
        );
    }

    #[test]
    fn duplicate_keys() {
        let src = "\
on: push
jobs:
  test:
    strategy:
      matrix:
        foo: [a, b, c]
        FOO: [a, b, c]
    runs-on: ubuntu-latest
    runs-on: macos-latest
    steps:
      - run: echo
";
        let es = errs(src);
        assert!(
            es.iter().any(
                |e| e.contains("key \"FOO\" is duplicated in \"matrix\" section")
                    && e.contains("note that this key is case insensitive")
            ),
            "{es:?}"
        );
        assert!(
            es.iter()
                .any(|e| e.contains("key \"runs-on\" is duplicated in \"test\" job")),
            "{es:?}"
        );
    }

    #[test]
    fn merge_key_unsupported() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo
        env:
          <<: *x
          A: b
";
        // Undefined anchor is a YAML parse error.
        let es = errs(src);
        assert!(
            es.iter().any(|e| e.contains("could not parse as YAML")),
            "{es:?}"
        );
    }

    #[test]
    fn merge_key_with_anchor() {
        let src = "on: push\njobs:\n  test: &t\n    runs-on: ubuntu-latest\n    steps: &s\n      - run: echo\n  test2:\n    <<: *t\n    steps: *s\n";
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("does not support YAML merge key")),
            "{es:?}"
        );
    }

    #[test]
    fn recursive_anchor() {
        let src = "\
on: push
jobs:
  test: &a
    runs-on: ubuntu-latest
    steps:
      - run: *a
";
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("recursive alias \"a\" is found")),
            "{es:?}"
        );
    }

    #[test]
    fn unexpected_key_in_workflow() {
        let src = "\
name: foo
foo: bar
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo
";
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("unexpected key \"foo\" for \"workflow\" section")),
            "{es:?}"
        );
    }

    #[test]
    fn expression_values() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo
        if: ${{ github.event_name == 'push' }}
";
        assert_no_err(src);
        let (w, _) = parse(src.as_bytes());
        let w = w.unwrap();
        let step = &w.jobs.get("test").unwrap().steps[0];
        assert_eq!(
            step.if_condition.as_ref().map(|s| s.value.as_str()),
            Some("${{ github.event_name == 'push' }}")
        );
    }

    #[test]
    fn matrix_and_strategy() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      max-parallel: 2
      matrix:
        os: [ubuntu-latest, macos-latest]
        include:
          - os: ubuntu-latest
            experimental: true
    steps:
      - run: echo
";
        assert_no_err(src);
        let (w, _) = parse(src.as_bytes());
        let w = w.unwrap();
        let job = w.jobs.get("test").unwrap();
        let st = job.strategy.as_ref().unwrap();
        assert!(!st.fail_fast.as_ref().unwrap().value);
        assert_eq!(st.max_parallel.as_ref().unwrap().value, 2);
        let matrix = st.matrix.as_ref().unwrap();
        assert!(matrix.rows.contains_key("os"));
        assert!(matrix.include.is_some());
    }

    #[test]
    fn max_parallel_must_be_positive() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      max-parallel: 0
      matrix:
        os: [ubuntu-latest]
    steps:
      - run: echo
";
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("value at \"max-parallel\" must be greater than zero: 0")),
            "{es:?}"
        );
    }

    #[test]
    fn timeout_minutes_must_be_positive() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    timeout-minutes: -1
    steps:
      - run: echo
";
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("value at \"timeout-minutes\" must be greater than zero: -1")),
            "{es:?}"
        );
    }

    #[test]
    fn step_must_have_run_or_uses() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - name: no run/uses
";
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("step must run script with \"run\" section")),
            "{es:?}"
        );
    }

    #[test]
    fn docker_step_uses() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: docker://alpine:3.18
        with:
          entrypoint: /bin/sh
          args: -c 'echo hi'
";
        assert_no_err(src);
        let (w, _) = parse(src.as_bytes());
        let w = w.unwrap();
        let step = &w.jobs.get("test").unwrap().steps[0];
        let Exec::Action(a) = &step.exec else {
            panic!("expected action exec");
        };
        assert_eq!(a.uses.value, "docker://alpine:3.18");
        assert_eq!(
            a.entrypoint.as_ref().map(|s| s.value.as_str()),
            Some("/bin/sh")
        );
        assert_eq!(
            a.args.as_ref().map(|s| s.value.as_str()),
            Some("-c 'echo hi'")
        );
        assert!(a.inputs.is_empty());
    }

    #[test]
    fn normal_action_step_with_inputs() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          repository: foo/bar
          token: ${{ github.token }}
";
        assert_no_err(src);
        let (w, _) = parse(src.as_bytes());
        let w = w.unwrap();
        let step = &w.jobs.get("test").unwrap().steps[0];
        let Exec::Action(a) = &step.exec else {
            panic!("expected action exec");
        };
        assert_eq!(a.inputs.len(), 2);
        assert_eq!(a.inputs.get("repository").unwrap().value.value, "foo/bar");
        assert!(a.entrypoint.is_none());
    }

    #[test]
    fn reusable_workflow_call() {
        let src = "\
on: push
jobs:
  call:
    uses: owner/repo/.github/workflows/w.yml@v1
    with:
      foo: bar
    secrets: inherit
";
        assert_no_err(src);
        let (w, _) = parse(src.as_bytes());
        let w = w.unwrap();
        let job = w.jobs.get("call").unwrap();
        let wc = job.workflow_call.as_ref().unwrap();
        assert_eq!(wc.uses.value, "owner/repo/.github/workflows/w.yml@v1");
        assert!(wc.inherit_secrets);
    }

    #[test]
    fn reusable_call_with_steps_key_is_error() {
        let src = "\
on: push
jobs:
  call:
    uses: owner/repo/.github/workflows/w.yml@v1
    runs-on: ubuntu-latest
    steps:
      - run: echo
";
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("when a reusable workflow is called with \"uses\"")),
            "{es:?}"
        );
    }

    #[test]
    fn container_and_services() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    container:
      image: node:20
      env:
        FOO: bar
      ports:
        - 80
      volumes:
        - /tmp:/tmp
      options: --cpus 1
    services:
      redis:
        image: redis
        ports:
          - 6379
    steps:
      - run: echo
";
        assert_no_err(src);
        let (w, _) = parse(src.as_bytes());
        let w = w.unwrap();
        let job = w.jobs.get("test").unwrap();
        let c = job.container.as_ref().unwrap();
        assert_eq!(c.image.value, "node:20");
        assert_eq!(
            c.ports.iter().map(|s| s.value.clone()).collect::<Vec<_>>(),
            vec!["80"]
        );
        // Regression test for the original bug that put volumes into ports.
        assert_eq!(
            c.volumes
                .iter()
                .map(|s| s.value.clone())
                .collect::<Vec<_>>(),
            vec!["/tmp:/tmp"]
        );
        let svc = job.services.as_ref().unwrap().value.get("redis").unwrap();
        assert_eq!(svc.container.image.value, "redis");
        assert_eq!(
            svc.container
                .ports
                .iter()
                .map(|s| s.value.clone())
                .collect::<Vec<_>>(),
            vec!["6379"]
        );
    }

    #[test]
    fn credentials_require_both() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    container:
      image: node:20
      credentials:
        username: user
    steps:
      - run: echo
";
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("both \"username\" and \"password\" must be specified")),
            "{es:?}"
        );
    }

    #[test]
    fn permissions_and_env() {
        let src = "\
on: push
permissions:
  contents: read
  issues: write
env:
  GLOBAL: always
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo
";
        assert_no_err(src);
        let (w, _) = parse(src.as_bytes());
        let w = w.unwrap();
        let perms = w.permissions.as_ref().unwrap();
        assert_eq!(perms.scopes.get("contents").unwrap().value.value, "read");
        assert_eq!(
            w.env
                .as_ref()
                .unwrap()
                .vars
                .get("global")
                .unwrap()
                .value
                .value,
            "always"
        );
    }

    #[test]
    fn schedule_event() {
        let src = "\
on:
  schedule:
    - cron: '0 0 * * *'
      timezone: UTC
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo
";
        assert_no_err(src);
        let (w, _) = parse(src.as_bytes());
        let w = w.unwrap();
        let AstEvent::Scheduled(s) = &w.on[0] else {
            panic!("expected schedule event");
        };
        assert_eq!(s.schedules[0].cron.value, "0 0 * * *");
        assert_eq!(
            s.schedules[0].timezone.as_ref().map(|s| s.value.as_str()),
            Some("UTC")
        );
    }

    #[test]
    fn schedule_requires_mapping() {
        let src = "\
on: schedule
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo
";
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("schedule event must be configured with mapping")),
            "{es:?}"
        );
    }

    #[test]
    fn workflow_dispatch_event() {
        let src = "\
on:
  workflow_dispatch:
    inputs:
      name:
        description: 'Your name'
        required: true
        type: string
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo
";
        assert_no_err(src);
        let (w, _) = parse(src.as_bytes());
        let AstEvent::WorkflowDispatch(d) = &w.unwrap().on[0] else {
            panic!("expected workflow_dispatch event");
        };
        let input = d.inputs.get("name").unwrap();
        assert_eq!(input.input_type, WorkflowDispatchEventInputType::String);
        assert!(input.required.as_ref().unwrap().value);
    }

    #[test]
    fn workflow_call_missing_type() {
        let src = "\
on:
  workflow_call:
    inputs:
      foo:
        description: bar
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo
";
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("\"type\" is missing at \"foo\" input")),
            "{es:?}"
        );
    }

    #[test]
    fn quoted_strings_are_marked() {
        let src = "\
on: push
jobs:
  test:
    runs-on: 'ubuntu-latest'
    steps:
      - run: \"echo\"
";
        let (w, es) = parse(src.as_bytes());
        assert!(es.is_empty(), "{es:?}");
        let w = w.unwrap();
        let job = w.jobs.get("test").unwrap();
        assert_eq!(
            job.runs_on.as_ref().unwrap().labels[0].value,
            "ubuntu-latest"
        );
    }

    #[test]
    fn integer_and_bool_tags_resolved() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      max-parallel: 4
      fail-fast: true
      matrix:
        os: [ubuntu-latest]
    steps:
      - run: echo
";
        let (w, es) = parse(src.as_bytes());
        assert!(es.is_empty(), "{es:?}");
        let w = w.unwrap();
        let st = w.jobs.get("test").unwrap().strategy.as_ref().unwrap();
        assert_eq!(st.max_parallel.as_ref().unwrap().value, 4);
        assert!(st.fail_fast.as_ref().unwrap().value);
    }

    #[test]
    fn invalid_integer_value() {
        let src = "\
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      max-parallel: notanumber
      matrix:
        os: [ubuntu-latest]
    steps:
      - run: echo
";
        // A plain non-expression string passes the tag check but must be a single `${{ ... }}`
        // expression, so a bare literal is rejected as an expression.
        let es = errs(src);
        assert!(
            es.iter()
                .any(|e| e.contains("expecting a single ${{...}} expression or integer literal")),
            "{es:?}"
        );
    }
}
