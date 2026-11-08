//! Workflow syntax tree.
//!
//! Rust counterpart of the original `ast.go`. It defines the root [`Workflow`] node and all the
//! nodes that make up a GitHub Actions workflow configuration file.
//!
//! String values that appear in the workflow source are represented by [`String`], which carries
//! the source position and whether the value was quoted. This mirrors the original design where
//! most string fields are `*String` so that rules can report precise error locations.

use std::collections::BTreeMap;
use std::sync::Arc;

/// A position in a file. Line and column are 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    /// The 1-based line number.
    pub line: usize,
    /// The 1-based column number.
    pub col: usize,
}

impl Pos {
    /// Creates a new position.
    #[must_use]
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }

    /// Returns whether this position is strictly before `other`.
    #[must_use]
    pub fn is_before(&self, other: &Pos) -> bool {
        (self.line, self.col) < (other.line, other.col)
    }
}

impl std::fmt::Display for Pos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line:{},col:{}", self.line, self.col)
    }
}

/// A generic string value in a YAML file, carrying its source position.
///
/// This type is intentionally named `String` to mirror the original AST. It is distinct from
/// [`std::string::String`]; reference the `value` field (or use [`String::as_str`]) to obtain the
/// raw text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct String {
    /// The raw value of the string.
    pub value: std::string::String,
    /// Whether the string was quoted with `'` or `"` in the YAML source.
    pub quoted: bool,
    /// The position of the string in the source.
    pub pos: Pos,
}

impl String {
    /// Creates a new positioned string.
    pub fn new(value: impl Into<std::string::String>, quoted: bool, pos: Pos) -> Self {
        Self {
            value: value.into(),
            quoted,
            pos,
        }
    }

    /// Creates a positioned, unquoted string at the given position.
    pub fn at(value: impl Into<std::string::String>, pos: Pos) -> Self {
        Self::new(value, false, pos)
    }

    /// Returns the raw text of the string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Returns whether the string contains at least one `${{ }}` expression.
    #[must_use]
    pub fn contains_expression(&self) -> bool {
        contains_expression(&self.value)
    }

    /// Returns whether a single expression is assigned to the string.
    #[must_use]
    pub fn is_expression_assigned(&self) -> bool {
        is_expr_assigned(&self.value)
    }
}

impl std::fmt::Display for String {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value)
    }
}

impl From<&'static str> for String {
    fn from(s: &'static str) -> Self {
        String::at(s, Pos::default())
    }
}

/// Checks whether the given string contains a `${{ }}` placeholder.
#[must_use]
pub fn contains_expression(s: &str) -> bool {
    match (s.find("${{"), s.find("}}")) {
        (Some(i), Some(j)) => i < j,
        _ => false,
    }
}

/// Returns whether a single expression is assigned to the string.
fn is_expr_assigned(s: &str) -> bool {
    let v = s.trim();
    // Do not check that "}}" occurs exactly once because it might appear inside a JSON string,
    // e.g. `if: ${{ env.foo == '{"foo": {"bar": true}}' }}`.
    v.starts_with("${{") && v.ends_with("}}") && v.matches("${{").count() == 1
}

/// A generic boolean value in a YAML file, carrying its source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bool {
    /// The raw value.
    pub value: bool,
    /// The expression string when `${{ }}` syntax is used for this section.
    pub expression: Option<String>,
    /// The position in the source.
    pub pos: Pos,
}

impl std::fmt::Display for Bool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(e) = &self.expression {
            return f.write_str(&e.value);
        }
        f.write_str(if self.value { "true" } else { "false" })
    }
}

/// A generic integer value in a YAML file, carrying its source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Int {
    /// The raw value.
    pub value: i64,
    /// The expression string when `${{ }}` syntax is used for this section.
    pub expression: Option<String>,
    /// The position in the source.
    pub pos: Pos,
}

/// A generic float value in a YAML file, carrying its source position.
#[derive(Debug, Clone, PartialEq)]
pub struct Float {
    /// The raw value.
    pub value: f64,
    /// The expression string when `${{ }}` syntax is used for this section.
    pub expression: Option<String>,
    /// The position in the source.
    pub pos: Pos,
}

/// A webhook event filter such as `branches`, `paths-ignore`, etc.
///
/// See <https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions#using-filters>.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookEventFilter {
    /// The filter name such as `branches`, `tags`.
    pub name: String,
    /// The list of filter values.
    pub values: Vec<String>,
}

impl WebhookEventFilter {
    /// Returns whether the filter has no values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// An event based on a webhook event.
///
/// Some events cannot have a `types` field. Only `push` and `pull` events can have `tags`,
/// `tags-ignore`, `paths` and `paths-ignore` fields. Only `workflow_run` can have a `workflows`
/// field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookEvent {
    /// The name of the webhook event.
    pub hook: String,
    /// The activity types that trigger the workflow.
    pub types: Vec<String>,
    /// The `branches` filter.
    pub branches: Option<WebhookEventFilter>,
    /// The `branches-ignore` filter.
    pub branches_ignore: Option<WebhookEventFilter>,
    /// The `tags` filter.
    pub tags: Option<WebhookEventFilter>,
    /// The `tags-ignore` filter.
    pub tags_ignore: Option<WebhookEventFilter>,
    /// The `paths` filter.
    pub paths: Option<WebhookEventFilter>,
    /// The `paths-ignore` filter.
    pub paths_ignore: Option<WebhookEventFilter>,
    /// Workflow names that trigger this workflow via `workflow_run`.
    pub workflows: Vec<String>,
    /// The position in the source.
    pub pos: Pos,
}

impl WebhookEvent {
    /// The name of the event.
    #[must_use]
    pub fn event_name(&self) -> &str {
        self.hook.as_str()
    }
}

/// A single entry in a `schedule` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleEntry {
    /// The cron string for the schedule.
    pub cron: String,
    /// The optional IANA timezone for the schedule.
    pub timezone: Option<String>,
}

/// An event scheduled by the `schedule` trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledEvent {
    /// The list of schedule entries.
    pub schedules: Vec<ScheduleEntry>,
    /// The position in the source.
    pub pos: Pos,
}

impl ScheduledEvent {
    /// The name of the event.
    #[must_use]
    pub fn event_name(&self) -> &'static str {
        "schedule"
    }
}

/// The type of an input of a `workflow_dispatch` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkflowDispatchEventInputType {
    /// No type is specified.
    #[default]
    None,
    /// A string input.
    String,
    /// A number input.
    Number,
    /// A boolean input.
    Boolean,
    /// A choice input.
    Choice,
    /// An environment input.
    Environment,
}

/// An input specified when dispatching a workflow manually.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchInput {
    /// The name of the input.
    pub name: String,
    /// The description of the input.
    pub description: Option<String>,
    /// Whether the input is mandatory.
    pub required: Option<Bool>,
    /// The default value of the input.
    pub default: Option<String>,
    /// The type of the input.
    pub input_type: WorkflowDispatchEventInputType,
    /// The options of a `choice` type input.
    pub options: Vec<String>,
}

/// An event on dispatching a workflow manually.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDispatchEvent {
    /// Map from input names (lower cased) to their attributes.
    pub inputs: BTreeMap<std::string::String, DispatchInput>,
    /// The position in the source.
    pub pos: Pos,
}

impl WorkflowDispatchEvent {
    /// The name of the event.
    #[must_use]
    pub fn event_name(&self) -> &'static str {
        "workflow_dispatch"
    }
}

/// The `repository_dispatch` event configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryDispatchEvent {
    /// The activity types that can trigger the workflow.
    pub types: Vec<String>,
    /// The position in the source.
    pub pos: Pos,
}

impl RepositoryDispatchEvent {
    /// The name of the event.
    #[must_use]
    pub fn event_name(&self) -> &'static str {
        "repository_dispatch"
    }
}

/// The type of an input of a `workflow_call` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkflowCallEventInputType {
    /// An invalid type was given.
    #[default]
    Invalid,
    /// A boolean input.
    Boolean,
    /// A number input.
    Number,
    /// A string input.
    String,
}

/// An input configuration of a `workflow_call` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCallEventInput {
    /// The name of the input.
    pub name: String,
    /// The description of the input.
    pub description: Option<String>,
    /// The default value of the input. `None` means no default.
    pub default: Option<String>,
    /// Whether the input is required. `None` means not explicitly specified (defaults to optional).
    pub required: Option<Bool>,
    /// The input type; must be one of `boolean`, `number` or `string`.
    pub input_type: WorkflowCallEventInputType,
    /// The lower-cased ID of the input.
    pub id: std::string::String,
}

impl WorkflowCallEventInput {
    /// Returns whether the input is marked as required.
    #[must_use]
    pub fn is_required(&self) -> bool {
        self.required.as_ref().is_some_and(|b| b.value)
    }
}

/// A secret configuration of a `workflow_call` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCallEventSecret {
    /// The name of the secret.
    pub name: String,
    /// The description of the secret.
    pub description: Option<String>,
    /// Whether the secret is required. `None` means optional.
    pub required: Option<Bool>,
}

/// An output configuration of a `workflow_call` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCallEventOutput {
    /// The name of the output.
    pub name: String,
    /// The description of the output.
    pub description: Option<String>,
    /// The expression for the value of the output.
    pub value: Option<String>,
}

/// The `workflow_call` event configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCallEvent {
    /// The inputs of the event. This is a list (not a map) because the order matters when
    /// checking default values.
    pub inputs: Vec<WorkflowCallEventInput>,
    /// Map from secret name (lower cased) to its configuration.
    pub secrets: BTreeMap<std::string::String, WorkflowCallEventSecret>,
    /// Map from output name (lower cased) to its configuration.
    pub outputs: BTreeMap<std::string::String, WorkflowCallEventOutput>,
    /// The position in the source.
    pub pos: Pos,
}

impl WorkflowCallEvent {
    /// The name of the event.
    #[must_use]
    pub fn event_name(&self) -> &'static str {
        "workflow_call"
    }
}

/// The `image_version` event configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageVersionEvent {
    /// Names that match the created image names.
    pub names: Vec<String>,
    /// Names that match the created image versions. Glob patterns are available.
    pub versions: Vec<String>,
    /// The position in the source.
    pub pos: Pos,
}

impl ImageVersionEvent {
    /// The name of the event.
    #[must_use]
    pub fn event_name(&self) -> &'static str {
        "image_version"
    }
}

/// Any of the events that can trigger a workflow in the `on:` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A webhook-based event.
    Webhook(WebhookEvent),
    /// A `schedule` event.
    Scheduled(ScheduledEvent),
    /// A `workflow_dispatch` event.
    WorkflowDispatch(WorkflowDispatchEvent),
    /// A `repository_dispatch` event.
    RepositoryDispatch(RepositoryDispatchEvent),
    /// A `workflow_call` event.
    WorkflowCall(WorkflowCallEvent),
    /// An `image_version` event.
    ImageVersion(ImageVersionEvent),
}

impl Event {
    /// The name of the event that triggers the workflow.
    #[must_use]
    pub fn event_name(&self) -> &str {
        match self {
            Event::Webhook(e) => e.event_name(),
            Event::Scheduled(e) => e.event_name(),
            Event::WorkflowDispatch(e) => e.event_name(),
            Event::RepositoryDispatch(e) => e.event_name(),
            Event::WorkflowCall(e) => e.event_name(),
            Event::ImageVersion(e) => e.event_name(),
        }
    }
}

/// A single permission scope such as `issues`, `checks`, etc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionScope {
    /// The scope name.
    pub name: String,
    /// The permission value of the scope.
    pub value: String,
}

/// The permissions configuration of a workflow or job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permissions {
    /// A permission value applied to all scopes at once.
    pub all: Option<String>,
    /// Map from scope name to its permission configuration.
    pub scopes: BTreeMap<std::string::String, PermissionScope>,
    /// The position in the source.
    pub pos: Pos,
}

/// The `run` defaults configuration (shell and working directory).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultsRun {
    /// The shell name to run.
    pub shell: Option<String>,
    /// The default working directory path.
    pub working_directory: Option<String>,
    /// The position in the source.
    pub pos: Pos,
}

/// The defaults configuration for running shell scripts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    /// The `run` configuration.
    pub run: Option<DefaultsRun>,
    /// The position in the source.
    pub pos: Pos,
}

/// The concurrency configuration of a workflow or job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Concurrency {
    /// The name of the concurrency group.
    pub group: String,
    /// Whether canceling this workflow cancels other in-progress jobs.
    pub cancel_in_progress: Option<Bool>,
    /// The position in the source.
    pub pos: Pos,
}

/// The environment configuration of a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    /// The name of the environment.
    pub name: String,
    /// The URL mapped to `environment_url` in the deployments API. Empty means unspecified.
    pub url: Option<String>,
    /// Whether GitHub should create a deployment for this job.
    pub deployment: Option<Bool>,
    /// The position in the source.
    pub pos: Pos,
}

/// The kind of how a step is executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecKind {
    /// The step runs an action.
    Action,
    /// The step runs a shell script.
    Run,
}

/// How a step is executed: either an action (`uses:`) or a shell script (`run:`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exec {
    /// Configuration for running a shell script.
    Run(ExecRun),
    /// Configuration for running an action.
    Action(ExecAction),
}

impl Exec {
    /// The kind of the step execution.
    #[must_use]
    pub fn kind(&self) -> ExecKind {
        match self {
            Exec::Run(_) => ExecKind::Run,
            Exec::Action(_) => ExecKind::Action,
        }
    }
}

/// Configuration for running a shell script at a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRun {
    /// The script to run.
    pub run: String,
    /// The optional `shell` field.
    pub shell: Option<String>,
    /// The optional `working-directory` field.
    pub working_directory: Option<String>,
    /// The position of the `run` section.
    pub run_pos: Pos,
}

/// An input field for running an action via `with:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    /// The name of the input.
    pub name: String,
    /// The value of the input.
    pub value: String,
}

/// Configuration for running an action at a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecAction {
    /// The action specification from `uses:`.
    pub uses: String,
    /// The inputs to the action from `with:`. Keys are lower cased.
    pub inputs: BTreeMap<std::string::String, Input>,
    /// The optional `entrypoint` field in `with:`.
    pub entrypoint: Option<String>,
    /// The optional `args` field in `with:`.
    pub args: Option<String>,
}

/// The kind of a raw YAML value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawYAMLValueKind {
    /// A mapping value.
    Object,
    /// A sequence value.
    Array,
    /// A scalar value.
    String,
}

/// A value at a matrix variation. Any value can be placed at matrix variations, including
/// mappings and arrays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawYAMLValue {
    /// A raw YAML mapping. Keys are lower cased since they are case-insensitive.
    Object(RawYAMLObject),
    /// A raw YAML sequence.
    Array(RawYAMLArray),
    /// A raw YAML scalar.
    String(RawYAMLString),
}

impl RawYAMLValue {
    /// The kind of the raw YAML value.
    #[must_use]
    pub fn kind(&self) -> RawYAMLValueKind {
        match self {
            RawYAMLValue::Object(_) => RawYAMLValueKind::Object,
            RawYAMLValue::Array(_) => RawYAMLValueKind::Array,
            RawYAMLValue::String(_) => RawYAMLValueKind::String,
        }
    }

    /// The start position of the value in the source file.
    #[must_use]
    pub fn pos(&self) -> Pos {
        match self {
            RawYAMLValue::Object(o) => o.pos,
            RawYAMLValue::Array(a) => a.pos,
            RawYAMLValue::String(s) => s.pos,
        }
    }

    /// Returns whether this value equals `other` structurally.
    #[must_use]
    pub fn equals(&self, other: &RawYAMLValue) -> bool {
        match (self, other) {
            (RawYAMLValue::Object(a), RawYAMLValue::Object(b)) => a == b,
            (RawYAMLValue::Array(a), RawYAMLValue::Array(b)) => a == b,
            (RawYAMLValue::String(a), RawYAMLValue::String(b)) => a == b,
            _ => false,
        }
    }
}

impl std::fmt::Display for RawYAMLValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RawYAMLValue::Object(o) => o.fmt(f),
            RawYAMLValue::Array(a) => a.fmt(f),
            RawYAMLValue::String(s) => s.fmt(f),
        }
    }
}

/// A raw YAML mapping value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawYAMLObject {
    /// Map from property names (lower cased) to their values.
    pub props: BTreeMap<std::string::String, RawYAMLValue>,
    /// The start position in the source.
    pub pos: Pos,
}

impl std::fmt::Display for RawYAMLObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, (n, p)) in self.props.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{n:?}: {p}")?;
        }
        write!(f, "}}")
    }
}

/// A raw YAML sequence value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawYAMLArray {
    /// The elements of the array.
    pub elems: Vec<RawYAMLValue>,
    /// The start position in the source.
    pub pos: Pos,
}

impl std::fmt::Display for RawYAMLArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        for (i, e) in self.elems.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{e}")?;
        }
        write!(f, "]")
    }
}

/// A raw YAML scalar value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawYAMLString {
    /// The string representation of the scalar node.
    pub value: std::string::String,
    /// The start position in the source.
    pub pos: Pos,
}

impl std::fmt::Display for RawYAMLString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.value)
    }
}

/// One row of a matrix; a named value that can take multiple variations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixRow {
    /// The name of the matrix value.
    pub name: String,
    /// The variations the value can take.
    pub values: Vec<RawYAMLValue>,
    /// The expression string when `${{ }}` syntax is used for this section.
    pub expression: Option<String>,
}

/// Which value should be taken in a row of the matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixAssign {
    /// The name of the matrix value.
    pub key: String,
    /// The selected value.
    pub value: RawYAMLValue,
}

/// A combination of matrix value assignments defining one matrix variation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixCombination {
    /// Map from matrix value name to its assignment.
    pub assigns: BTreeMap<std::string::String, MatrixAssign>,
    /// The expression string when `${{ }}` syntax is used for this section.
    pub expression: Option<String>,
}

/// A list of matrix combinations used by `include` and `exclude` sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixCombinations {
    /// The combinations.
    pub combinations: Vec<MatrixCombination>,
    /// The expression string when `${{ }}` syntax is used for this section.
    pub expression: Option<String>,
}

impl MatrixCombinations {
    /// Returns whether the combinations include at least one expression node.
    #[must_use]
    pub fn contains_expression(&self) -> bool {
        self.expression.is_some() || self.combinations.iter().any(|c| c.expression.is_some())
    }
}

/// The matrix variations configuration of a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matrix {
    /// Map from name (lower cased) to a row of values.
    pub rows: BTreeMap<std::string::String, MatrixRow>,
    /// Additional combinations to include.
    pub include: Option<MatrixCombinations>,
    /// Combinations to exclude.
    pub exclude: Option<MatrixCombinations>,
    /// The expression string when `${{ }}` syntax is used for this section.
    pub expression: Option<String>,
    /// The position in the source.
    pub pos: Pos,
}

/// The strategy configuration of how a job is run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strategy {
    /// The matrix of combinations.
    pub matrix: Option<Matrix>,
    /// Whether other jobs should stop when one job fails.
    pub fail_fast: Option<Bool>,
    /// How many jobs should run at once.
    pub max_parallel: Option<Int>,
    /// The position in the source.
    pub pos: Pos,
}

/// A key-value pair of an environment variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVar {
    /// The name of the environment variable.
    pub name: String,
    /// The string value of the environment variable.
    pub value: String,
}

/// A set of environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    /// Map from env var name to its value.
    pub vars: BTreeMap<std::string::String, EnvVar>,
    /// An expression string containing `${{ ... }}`. When set, `vars` must be empty.
    pub expression: Option<String>,
}

/// A step configuration. A step runs one action or one shell script.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// The step ID.
    pub id: Option<String>,
    /// The `if:` condition.
    pub if_condition: Option<String>,
    /// The step name.
    pub name: Option<String>,
    /// How the step is executed.
    pub exec: Exec,
    /// The environment variables set while running the step.
    pub env: Option<Env>,
    /// Whether execution should continue on error.
    pub continue_on_error: Option<Bool>,
    /// The timeout value in minutes.
    pub timeout_minutes: Option<Float>,
    /// The position in the source.
    pub pos: Pos,
}

/// The credentials configuration of a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    /// The username for authentication.
    pub username: Option<String>,
    /// The password for authentication.
    pub password: Option<String>,
    /// The expression string when `${{ }}` syntax is used for this section.
    pub expression: Option<String>,
    /// The position in the source.
    pub pos: Pos,
}

/// The configuration of how to run a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    /// The Docker image specification.
    pub image: String,
    /// The credentials configuration of the container.
    pub credentials: Option<Credentials>,
    /// The environment variables set in the container.
    pub env: Option<Env>,
    /// The list of port number mappings.
    pub ports: Vec<String>,
    /// The list of volumes mounted to the container.
    pub volumes: Vec<String>,
    /// The options string for running the container.
    pub options: Option<String>,
    /// Command overriding the image's default command for service containers.
    pub command: Option<String>,
    /// Entrypoint overriding the image's default entrypoint for service containers.
    pub entrypoint: Option<String>,
    /// The position in the source.
    pub pos: Pos,
}

/// The configuration to run a service such as a database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// The name of the service.
    pub name: String,
    /// The container configuration of the service.
    pub container: Container,
}

/// A mapping from service ID to its configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Services {
    /// Map from service ID (lower cased) to its configuration.
    pub value: BTreeMap<std::string::String, Service>,
    /// An expression assigned to the whole services mapping via `${{ }}`.
    pub expression: Option<String>,
    /// The position in the source.
    pub pos: Pos,
}

/// An output entry of a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The name of the output.
    pub name: String,
    /// The value of the output.
    pub value: String,
}

/// The runner configuration (`runs-on:`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runner {
    /// The label names to select a runner.
    pub labels: Vec<String>,
    /// The expression string when `${{ }}` syntax is used for this section.
    pub labels_expr: Option<String>,
    /// The group of runners specified in `runs-on:`.
    pub group: Option<String>,
}

/// A normal input for a workflow call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCallInput {
    /// The name of the input.
    pub name: String,
    /// The value of the input.
    pub value: String,
}

/// A secret input for a workflow call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCallSecret {
    /// The name of the secret.
    pub name: String,
    /// The value of the secret.
    pub value: String,
}

/// A workflow call at `jobs.<job_id>.uses`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCall {
    /// The workflow specification to be called (mandatory).
    pub uses: String,
    /// Map from input name (lower cased) to its value at `with:`.
    pub inputs: BTreeMap<std::string::String, WorkflowCallInput>,
    /// Map from secret name (lower cased) to its value at `secrets:`.
    pub secrets: BTreeMap<std::string::String, WorkflowCallSecret>,
    /// Whether `secrets: inherit` was specified. When true, `secrets` must be empty.
    pub inherit_secrets: bool,
}

/// A custom image snapshot at `jobs.<job_id>.snapshot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The name of the custom image.
    pub image_name: String,
    /// The version of the custom image.
    pub version: Option<String>,
    /// The condition whether the custom image is used.
    pub if_condition: Option<String>,
}

/// The configuration of how to run a job.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    /// The ID of the job (the key of the job configuration object).
    pub id: String,
    /// The user-specified job name.
    pub name: Option<String>,
    /// The job IDs that must complete before this job runs.
    pub needs: Vec<String>,
    /// The runner configuration.
    pub runs_on: Option<Runner>,
    /// The permission configuration for the job.
    pub permissions: Option<Permissions>,
    /// The environment where the job runs.
    pub environment: Option<Environment>,
    /// The concurrency configuration for the job.
    pub concurrency: Option<Concurrency>,
    /// Map from output name to its specification.
    pub outputs: BTreeMap<std::string::String, Output>,
    /// The environment variables set while running the job.
    pub env: Option<Env>,
    /// The default configurations for running scripts.
    pub defaults: Option<Defaults>,
    /// Whether this job should run.
    pub if_condition: Option<String>,
    /// The steps to run in the job.
    pub steps: Vec<Step>,
    /// The timeout value in minutes.
    pub timeout_minutes: Option<Float>,
    /// The strategy configuration.
    pub strategy: Option<Strategy>,
    /// Whether execution should continue on error.
    pub continue_on_error: Option<Bool>,
    /// The container configuration for the job.
    pub container: Option<Container>,
    /// The services configuration.
    pub services: Option<Services>,
    /// A workflow call via `uses:`.
    pub workflow_call: Option<WorkflowCall>,
    /// A custom image snapshot.
    pub snapshot: Option<Snapshot>,
    /// The position in the source.
    pub pos: Pos,
}

/// The root of the workflow syntax tree, representing one workflow configuration file.
#[derive(Debug, Clone, PartialEq)]
pub struct Workflow {
    /// The name of the workflow. `None` when not specified.
    pub name: Option<String>,
    /// The name of workflow runs; can be set dynamically via `${{ }}`.
    pub run_name: Option<String>,
    /// The events that can trigger this workflow.
    pub on: Vec<Event>,
    /// The permission configuration of the workflow.
    pub permissions: Option<Permissions>,
    /// The default environment variables for the workflow.
    pub env: Option<Env>,
    /// The default configuration for running scripts.
    pub defaults: Option<Defaults>,
    /// The concurrency configuration of the entire workflow.
    pub concurrency: Option<Concurrency>,
    /// Map from job ID (lower cased) to the job object.
    pub jobs: BTreeMap<std::string::String, Job>,
}

impl Workflow {
    /// Returns the `workflow_call` event node if present.
    #[must_use]
    pub fn find_workflow_call_event(&self) -> Option<&WorkflowCallEvent> {
        self.on.iter().find_map(|e| match e {
            Event::WorkflowCall(e) => Some(e),
            _ => None,
        })
    }
}

/// A cheaply cloneable handle used to share AST nodes across rules.
pub type Shared<T> = Arc<T>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pos_display_and_before() {
        let a = Pos::new(1, 2);
        let b = Pos::new(1, 3);
        let c = Pos::new(2, 1);
        assert_eq!(a.to_string(), "line:1,col:2");
        assert!(a.is_before(&b));
        assert!(b.is_before(&c));
        assert!(!b.is_before(&a));
        assert!(!a.is_before(&a));
    }

    #[test]
    fn string_helpers() {
        let s = String::at("${{ env.foo }}", Pos::new(1, 1));
        assert!(s.is_expression_assigned());
        assert!(s.contains_expression());
        assert_eq!(s.as_str(), "${{ env.foo }}");

        let s2 = String::at("prefix ${{ env.foo }}", Pos::new(1, 1));
        assert!(!s2.is_expression_assigned());
        assert!(s2.contains_expression());

        assert!(contains_expression("a ${{ foo }} b"));
        assert!(!contains_expression("${{ foo }"));
        assert!(!contains_expression("plain"));
    }

    #[test]
    fn quoted_flag() {
        let s = String::new("hello", true, Pos::new(2, 3));
        assert!(s.quoted);
        assert_eq!(s.pos, Pos::new(2, 3));
    }

    #[test]
    fn event_names() {
        let e = Event::Scheduled(ScheduledEvent {
            schedules: vec![],
            pos: Pos::new(1, 1),
        });
        assert_eq!(e.event_name(), "schedule");
    }

    #[test]
    fn raw_value_display() {
        let v = RawYAMLValue::Array(RawYAMLArray {
            elems: vec![
                RawYAMLValue::String(RawYAMLString {
                    value: "a".into(),
                    pos: Pos::new(1, 1),
                }),
                RawYAMLValue::String(RawYAMLString {
                    value: "b".into(),
                    pos: Pos::new(1, 2),
                }),
            ],
            pos: Pos::new(1, 1),
        });
        assert_eq!(v.to_string(), "[\"a\", \"b\"]");
        assert_eq!(v.kind(), RawYAMLValueKind::Array);
    }

    #[test]
    fn workflow_call_event_lookup() {
        let w = Workflow {
            name: None,
            run_name: None,
            on: vec![Event::WorkflowCall(WorkflowCallEvent {
                inputs: vec![],
                secrets: BTreeMap::new(),
                outputs: BTreeMap::new(),
                pos: Pos::new(1, 1),
            })],
            permissions: None,
            env: None,
            defaults: None,
            concurrency: None,
            jobs: BTreeMap::new(),
        };
        assert!(w.find_workflow_call_event().is_some());
    }

    #[test]
    fn bool_display() {
        let b = Bool {
            value: true,
            expression: Some(String::at("${{ x }}", Pos::new(1, 1))),
            pos: Pos::new(1, 1),
        };
        assert_eq!(b.to_string(), "${{ x }}");
        let b2 = Bool {
            value: false,
            expression: None,
            pos: Pos::new(1, 1),
        };
        assert_eq!(b2.to_string(), "false");
    }
}
