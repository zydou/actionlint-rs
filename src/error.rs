//! Errors detected by actionlint rules and their formatting.
//!
//! Rust counterpart of the original `error.go`. An [`Error`] is a single problem found in a
//! workflow file. Errors can be pretty-printed with a source snippet, or rendered through a Go
//! template via [`ErrorFormatter`].

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use colored::Colorize;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::rule::Rule;

/// An error detected by an actionlint rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// The error message.
    pub message: String,
    /// The file path where the error occurred. Populated by the linter.
    pub filepath: String,
    /// The 1-based line number where the error occurred.
    pub line: usize,
    /// The 1-based column number where the error occurred.
    pub column: usize,
    /// A string representing the kind of the error, usually the rule name that found it.
    pub kind: String,
}

impl Error {
    /// Creates a new error located at the given position.
    pub fn at(line: usize, column: usize, kind: &str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            filepath: String::new(),
            line,
            column,
            kind: kind.to_string(),
        }
    }

    /// Creates a new error with a formatted message located at the given position.
    #[must_use]
    pub fn format_at(line: usize, column: usize, kind: &str, message: fmt::Arguments<'_>) -> Self {
        Self::at(line, column, kind, message.to_string())
    }

    /// Returns a summary of the error as a single line, e.g.
    /// `foo.yaml:1:2: message [kind]`.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{}:{}:{}: {} [{}]",
            self.filepath, self.line, self.column, self.message, self.kind
        )
    }

    /// Returns the source line (1-based) that this error refers to, if it can be found.
    fn get_line<'a>(&self, source: &'a [u8]) -> Option<&'a str> {
        let source = std::str::from_utf8(source).ok()?;
        source.lines().nth(self.line.checked_sub(1)?)
    }

    /// Builds the `^~~~~` indicator string for the error position within `line`.
    ///
    /// Column is 1-based. Returns `None` when the column is out of range or zero.
    fn get_indicator(&self, line: &str) -> Option<String> {
        if self.column == 0 {
            return None;
        }
        let start = self.column - 1;
        let before = &line[..start.min(line.len())];
        let after = &line[start.min(line.len())..];

        // Width of the non-space characters after '^' to underline.
        let mut uw: usize = 0;
        for c in after.chars() {
            if c.is_whitespace() {
                break;
            }
            uw += c.width().unwrap_or(1);
        }
        uw = uw.saturating_sub(1);

        let sw = before.width();
        Some(format!("{}^{}", " ".repeat(sw), "~".repeat(uw)))
    }

    /// Converts the error into the structured fields used by the template formatter and JSON
    /// serialization.
    #[must_use]
    pub fn template_fields(&self, source: &[u8]) -> ErrorTemplateFields {
        let mut snippet = String::new();
        let mut end = self.column;

        if !source.is_empty()
            && self.line > 0
            && let Some(l) = self.get_line(source)
        {
            snippet.push_str(l);
            if l.width() >= self.column.saturating_sub(1)
                && let Some(indicator) = self.get_indicator(l)
            {
                snippet.push('\n');
                snippet.push_str(&indicator);
                // The indicator line is pure ASCII so byte length is fine here.
                end = indicator.len();
            }
        }

        ErrorTemplateFields {
            message: self.message.clone(),
            filepath: self.filepath.clone(),
            line: self.line,
            column: self.column,
            kind: self.kind.clone(),
            snippet,
            end_column: end,
        }
    }

    /// Pretty-prints the error with file name, source position, colorful message and a source
    /// snippet with an indicator. When `source` is empty, no snippet is printed.
    pub fn pretty_print(&self, w: &mut dyn Write, source: &[u8]) {
        let _ = write!(w, "{}", self.filepath.yellow());
        let _ = write!(w, "{}", ":".bright_black());
        let _ = write!(w, "{}", self.line);
        let _ = write!(w, "{}", ":".bright_black());
        let _ = write!(w, "{}", self.column);
        let _ = write!(w, "{}", ": ".bright_black());
        let _ = write!(w, "{}", self.message.bold());
        let _ = writeln!(w, "{}", format!(" [{}]", self.kind).bright_black());

        if source.is_empty() || self.line == 0 {
            return;
        }
        let Some(line) = self.get_line(source) else {
            return;
        };
        if line.width() < self.column.saturating_sub(1) {
            return;
        }

        let lnum = format!("{} | ", self.line);
        let indent = " ".repeat(lnum.len().saturating_sub(2));
        let _ = writeln!(w, "{}|", indent.bright_black());
        // Go prints the line-number gutter with `Fprint` (no trailing newline) so the source line
        // shares the same physical line as the gutter. Using `writeln!` here would split them.
        let _ = write!(w, "{}", lnum.bright_black());
        let _ = writeln!(w, "{line}");
        let indicator = self.get_indicator(line).unwrap_or_default();
        let _ = write!(w, "{}| ", indent.bright_black());
        let _ = writeln!(w, "{}", indicator.green());
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

/// Compares two errors for deterministic ordering: by file path, line, column, then message.
#[must_use]
pub fn compare_errors(a: &Error, b: &Error) -> std::cmp::Ordering {
    a.filepath
        .cmp(&b.filepath)
        .then(a.line.cmp(&b.line))
        .then(a.column.cmp(&b.column))
        .then(a.message.cmp(&b.message))
}

/// Returns whether two errors are equal in the fields used for de-duplication.
#[must_use]
pub fn equals_errors(a: &Error, b: &Error) -> bool {
    a.filepath == b.filepath && a.line == b.line && a.column == b.column && a.message == b.message
}

/// All fields used to format a single error message through a template.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ErrorTemplateFields {
    /// The error message body.
    pub message: String,
    /// The canonical relative file path. Empty when the input was read from stdin.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub filepath: String,
    /// The line number of the error position.
    pub line: usize,
    /// The column number of the error position.
    pub column: usize,
    /// The rule name the error belongs to.
    pub kind: String,
    /// The code snippet and indicator showing where the error occurred.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub snippet: String,
    /// The column number where the error indicator (`^~~~~~~`) ends. Equals `column` when no
    /// indicator could be shown.
    pub end_column: usize,
}

/// Unescapes backslash escape sequences (`\n`, `\t`, ...) in a format string.
///
/// See <https://golang.org/ref/spec#Rune_literals> for the set of recognized escapes.
#[must_use]
pub fn unescape_backslash(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('a') => out.push('\x07'),
            Some('b') => out.push('\x08'),
            Some('f') => out.push('\x0c'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('v') => out.push('\x0b'),
            Some('\\') => out.push('\\'),
            // Unknown escape: keep both characters as-is.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Converts a string to `PascalCase`, dropping non-alphanumeric separators.
#[must_use]
pub fn to_pascal_case(s: &str) -> String {
    let mut out = String::new();
    for word in s.split(|c: char| !c.is_ascii_alphanumeric()) {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            if first.is_ascii_lowercase() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            } else {
                out.push_str(word);
            }
        }
    }
    out
}

/// Fields describing a rule for use in error format templates.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuleTemplateFields {
    /// The rule name.
    pub name: String,
    /// The rule description.
    pub description: String,
}

/// A formatter that renders a slice of [`ErrorTemplateFields`] with a Go template.
///
/// Used to implement the `--format` command line option.
pub struct ErrorFormatter {
    template: gtmpl::Template,
    rules: std::sync::Mutex<std::collections::BTreeMap<String, RuleTemplateFields>>,
}

impl ErrorFormatter {
    /// Creates a new formatter from the given Go template format string.
    ///
    /// The format must contain at least one `{{ }}` placeholder. Escaped characters such as `\n`
    /// in the format string are unescaped before parsing.
    pub fn new(format: &str) -> Result<Self, String> {
        if !format.contains("{{") {
            return Err(format!(
                "template to format error messages must contain at least one {{{{ }}}} placeholder: {format}"
            ));
        }

        let unescaped = unescape_backslash(format);
        let mut template = gtmpl::Template::default();
        template.add_funcs(&[
            ("json", json_func as gtmpl::Func),
            ("replace", replace_func as gtmpl::Func),
            ("toPascalCase", to_pascal_case_func as gtmpl::Func),
            ("getVersion", get_version_func as gtmpl::Func),
            ("allKinds", all_kinds_func as gtmpl::Func),
        ]);
        template.parse(&unescaped).map_err(|e| {
            format!("template {format:?} to format error messages could not be parsed: {e}")
        })?;

        // The formatter starts with only the built-in `syntax-check` kind registered, matching the
        // original `NewErrorFormatter`. Every other rule is registered later, when the linter
        // constructs and runs its rules ([`ErrorFormatter::register_rule`]): the built-in rules are
        // always added, while the external `pyflakes` / `shellcheck` kinds appear only when those
        // commands are enabled and resolvable — exactly mirroring the original.
        let mut rules = std::collections::BTreeMap::new();
        rules.insert(
            "syntax-check".to_string(),
            RuleTemplateFields {
                name: "syntax-check".to_string(),
                description: "Checks for GitHub Actions workflow syntax".to_string(),
            },
        );
        set_registered_kinds(&rules);

        Ok(Self {
            template,
            rules: std::sync::Mutex::new(rules),
        })
    }

    /// Renders the given template fields and writes the result to `out`.
    ///
    /// Mirrors the original's behavior on template *execution* failures: the error is swallowed and
    /// whatever the template wrote before failing is kept. The original's `ErrorFormatter.Print`
    /// returns the `text/template` execution error, but the linter discards it and still exits with
    /// the "problems found" status, so a bad expression inside an otherwise parseable template
    /// never turns into a fatal error.
    pub fn print(&self, out: &mut dyn Write, fields: &[ErrorTemplateFields]) -> Result<(), String> {
        // Build the template context with PascalCase keys so templates can access fields as
        // `{{.Filepath}}`, matching the original Go struct field names.
        let arr: Vec<gtmpl::Value> = fields.iter().map(field_to_gtmpl).collect();
        let ctx = gtmpl::Context::from(gtmpl::Value::Array(arr));
        let mut buf = Vec::new();
        // Template *execution* errors are non-fatal, matching the original: `text/template` writes
        // whatever was rendered before the error and keeps the error internal to the writer, so the
        // partial output is printed and the exit status still reflects the lint result. Only
        // template *parse* errors (caught in `ErrorFormatter::new`) are fatal.
        let _ = self.template.execute(&mut buf, &ctx);
        out.write_all(&buf).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Converts the errors into template fields and renders them to `out`.
    pub fn print_errors(
        &self,
        out: &mut dyn Write,
        errors: &[Error],
        source: &[u8],
    ) -> Result<(), String> {
        let fields: Vec<_> = errors.iter().map(|e| e.template_fields(source)).collect();
        self.print(out, &fields)
    }

    /// Registers a rule so its name and description are available to the `allKinds` format helper.
    pub fn register_rule(&self, rule: &dyn Rule) {
        let mut rules = self.rules.lock().expect("rules mutex poisoned");
        rules
            .entry(rule.name().to_string())
            .or_insert_with(|| RuleTemplateFields {
                name: rule.name().to_string(),
                description: rule.description().to_string(),
            });
        set_registered_kinds(&rules);
    }
}

/// The error template fields in their canonical (struct declaration) order, matching the json tags
/// of the original Go `ErrorTemplateFields` struct. `encoding/json` emits struct fields in this
/// order, so the `json` template function must serialize them the same way. The map keys are the
/// exported Go field names, since templates reach them as `{{.Filepath}}`; the `json` function
/// lowers them back to the json tag names.
const CANONICAL_ERROR_FIELDS: [(&str, &str); 7] = [
    ("Message", "message"),
    ("Filepath", "filepath"),
    ("Line", "line"),
    ("Column", "column"),
    ("Kind", "kind"),
    ("Snippet", "snippet"),
    ("EndColumn", "end_column"),
];

/// Converts a [`gtmpl::Value`] into a [`serde_json::Value`], applying the `omitempty` semantics of
/// the original json tags for the known error fields.
fn gtmpl_to_json(v: &gtmpl::Value, omit_empty: bool) -> serde_json::Value {
    use gtmpl::Value;
    match v {
        Value::Nil => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Number(n) => {
            // gtmpl_value stores integral numbers as `Num::U`/`Num::I` and only floats as
            // `Num::F`, so `as_f64()` returns `None` for integers (e.g. line/column). Try the
            // integer forms first and only fall back to a float, otherwise positions become 0.
            if let Some(i) = n.as_i64() {
                serde_json::Value::from(i)
            } else if let Some(u) = n.as_u64() {
                serde_json::Value::from(u)
            } else {
                serde_json::Value::from(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Array(a) => {
            serde_json::Value::Array(a.iter().map(|v| gtmpl_to_json(v, omit_empty)).collect())
        }
        Value::Object(o) => {
            let mut map = serde_json::Map::new();
            // Go's encoding/json emits struct fields in declaration order. gtmpl's object is a
            // HashMap with nondeterministic order, so walk the canonical field order explicitly
            // and insert each present field in that order (serde_json's `preserve_order` feature
            // keeps the resulting Map ordered).
            for (exported, lower) in CANONICAL_ERROR_FIELDS {
                let Some(val) = o.get(exported) else {
                    continue;
                };
                if omit_empty
                    && let Value::String(s) = val
                    && s.is_empty()
                {
                    continue;
                }
                map.insert((*lower).to_string(), gtmpl_to_json(val, omit_empty));
            }
            serde_json::Value::Object(map)
        }
        other => serde_json::Value::String(other.to_string()),
    }
}

/// The `json` template function: serializes the given value to a JSON string, mirroring the
/// original's `json` helper (including `omitempty` for the known error fields).
fn json_func(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let arg = args.first().unwrap_or(&gtmpl::Value::Nil);
    let mut json = serde_json::to_string(&gtmpl_to_json(arg, true)).map_err(|e| {
        gtmpl::FuncError::Generic(format!("could not encode template value into JSON: {e}"))
    })?;
    // The original encodes through a json.Encoder, which appends a newline after the document.
    json.push('\n');
    Ok(gtmpl::Value::String(json))
}

/// The `replace` template function: replaces occurrences of `old` with `new` in a string. Accepts
/// any number of `old`, `new` pairs, mirroring Go's `strings.NewReplacer`.
fn replace_func(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let mut it = args.iter();
    let Some(first) = it.next() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("replace".into(), 1));
    };
    let mut s = first.to_string();
    let mut pairs = it.map(std::string::ToString::to_string);
    while let Some(old) = pairs.next() {
        let new = pairs.next().unwrap_or_default();
        s = s.replace(&old, &new);
    }
    Ok(gtmpl::Value::String(s))
}

/// The `toPascalCase` template function: converts a string to `PascalCase`.
fn to_pascal_case_func(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("toPascalCase".into(), 1));
    };
    Ok(gtmpl::Value::String(to_pascal_case(&first.to_string())))
}

/// The `getVersion` template function: returns the actionlint version.
fn get_version_func(_args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    Ok(gtmpl::Value::String(env!("CARGO_PKG_VERSION").to_string()))
}

/// The registry of rule kinds published by [`ErrorFormatter`] for the `allKinds` format helper.
///
/// `gtmpl` format helpers are plain function pointers with no access to the formatter, so the
/// registered kinds are published through this process-global mirror (guarded by a mutex) that
/// [`all_kinds_func`] reads at render time. Only one formatter renders per process, matching the
/// original's single CLI process.
static REGISTERED_KINDS: OnceLock<Mutex<BTreeMap<String, RuleTemplateFields>>> = OnceLock::new();

/// Replaces the globally published rule kinds with the formatter's current set.
fn set_registered_kinds(rules: &BTreeMap<String, RuleTemplateFields>) {
    let registry = REGISTERED_KINDS.get_or_init(|| Mutex::new(BTreeMap::new()));
    *registry.lock().expect("kinds mutex poisoned") = rules.clone();
}

/// The `allKinds` template function: returns the list of registered rule kinds, sorted by name.
fn all_kinds_func(_args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let kinds: Vec<RuleTemplateFields> = match REGISTERED_KINDS.get() {
        Some(registry) => {
            let map = registry.lock().expect("kinds mutex poisoned");
            map.values().cloned().collect()
        }
        None => Vec::new(),
    };
    Ok(gtmpl::Value::Array(
        kinds
            .into_iter()
            .map(|kind| {
                gtmpl::Value::Object(
                    [
                        ("Name".to_string(), gtmpl::Value::String(kind.name)),
                        (
                            "Description".to_string(),
                            gtmpl::Value::String(kind.description),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                )
            })
            .collect(),
    ))
}

/// Converts an [`ErrorTemplateFields`] into a [`gtmpl::Value`] object.
///
/// Each field is exposed only under its exported `PascalCase` name, matching the original Go
/// struct: `text/template` can only reach exported fields, so a lowercase access such as
/// `{{.filepath}}` resolves to nothing. The `json` function reads these `PascalCase` keys and
/// lowers them back to the json tag names.
fn field_to_gtmpl(f: &ErrorTemplateFields) -> gtmpl::Value {
    use gtmpl::Value;
    let pairs: [(&str, Value); 7] = [
        ("Message", Value::String(f.message.clone())),
        ("Filepath", Value::String(f.filepath.clone())),
        ("Line", Value::Number((f.line as f64).into())),
        ("Column", Value::Number((f.column as f64).into())),
        ("Kind", Value::String(f.kind.clone())),
        ("Snippet", Value::String(f.snippet.clone())),
        ("EndColumn", Value::Number((f.end_column as f64).into())),
    ];
    Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "on: push\njobs:\n  test:\n";

    #[test]
    fn summary() {
        let e = Error::at(1, 2, "kind", "msg");
        assert_eq!(e.summary(), ":1:2: msg [kind]");
    }

    #[test]
    fn indicator() {
        let e = Error::at(1, 5, "k", "m");
        // "on: push" column 5 is 'p'; underline "push" -> '^' + 3 tildes.
        let ind = e.get_indicator("on: push").unwrap();
        assert_eq!(ind, "    ^~~~");
    }

    #[test]
    fn template_fields_snippet() {
        let e = Error::at(1, 5, "k", "m");
        let f = e.template_fields(SRC.as_bytes());
        assert_eq!(f.snippet, "on: push\n    ^~~~");
        assert_eq!(f.line, 1);
    }

    #[test]
    fn unescape() {
        assert_eq!(unescape_backslash("a\\nb"), "a\nb");
        assert_eq!(unescape_backslash("a\\tb"), "a\tb");
        assert_eq!(unescape_backslash("a\\\\b"), "a\\b");
        assert_eq!(unescape_backslash("abc"), "abc");
    }

    #[test]
    fn pascal_case() {
        assert_eq!(to_pascal_case("hello world"), "HelloWorld");
        assert_eq!(to_pascal_case("foo-bar_baz"), "FooBarBaz");
    }

    #[test]
    fn formatter_rejects_no_placeholder() {
        assert!(ErrorFormatter::new("no placeholder").is_err());
    }

    #[test]
    fn formatter_range() {
        let f = ErrorFormatter::new("{{range .}}{{.Filepath}}:{{.Line}}:{{.Message}}\n{{end}}")
            .unwrap();
        let errors = vec![
            Error {
                message: "m1".into(),
                filepath: "foo.yaml".into(),
                line: 1,
                column: 4,
                kind: "k".into(),
            },
            Error {
                message: "m2".into(),
                filepath: "foo.yaml".into(),
                line: 3,
                column: 1,
                kind: "k".into(),
            },
        ];
        let mut out = Vec::new();
        f.print_errors(&mut out, &errors, b"").unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "foo.yaml:1:m1\nfoo.yaml:3:m2\n"
        );
    }

    #[test]
    fn compare_and_equals() {
        let a = Error::at(1, 2, "k", "a");
        let b = Error::at(1, 3, "k", "b");
        assert_eq!(compare_errors(&a, &b), std::cmp::Ordering::Less);
        assert!(!equals_errors(&a, &b));
    }
}
