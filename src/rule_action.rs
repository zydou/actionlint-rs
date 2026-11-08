//! The `action` rule.
//!
//! Checks: Checks for popular actions released on GitHub, local actions, and action calls at
//! `uses:`. Rust counterpart of the original `rule_action.go`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use regex::Regex;

use crate::action_metadata::{ActionMetadata, ActionMetadataRuns, LocalActionsCache};
use crate::ast::{Exec, ExecAction, Pos, Step};
use crate::popular_actions::{outdated_popular_action_specs, popular_actions};
use crate::quotes::sorted_quotes;
use crate::rule::{Pass, Rule, RuleBase};

/// Returns the set of colors allowed at `branding.color` in `action.yaml`.
///
/// See <https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#brandingcolor>.
#[must_use]
pub fn branding_colors() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        HashSet::from([
            "white",
            "black",
            "yellow",
            "blue",
            "green",
            "orange",
            "red",
            "purple",
            "gray-dark",
        ])
    })
}

/// Returns the set of icon names allowed at `branding.icon` in `action.yaml`.
///
/// See <https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#brandingicon>.
#[must_use]
pub fn branding_icons() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        HashSet::from([
            "activity",
            "airplay",
            "alert-circle",
            "alert-octagon",
            "alert-triangle",
            "align-center",
            "align-justify",
            "align-left",
            "align-right",
            "anchor",
            "aperture",
            "archive",
            "arrow-down-circle",
            "arrow-down-left",
            "arrow-down-right",
            "arrow-down",
            "arrow-left-circle",
            "arrow-left",
            "arrow-right-circle",
            "arrow-right",
            "arrow-up-circle",
            "arrow-up-left",
            "arrow-up-right",
            "arrow-up",
            "at-sign",
            "award",
            "bar-chart-2",
            "bar-chart",
            "battery-charging",
            "battery",
            "bell-off",
            "bell",
            "bluetooth",
            "bold",
            "book-open",
            "book",
            "bookmark",
            "box",
            "briefcase",
            "calendar",
            "camera-off",
            "camera",
            "cast",
            "check-circle",
            "check-square",
            "check",
            "chevron-down",
            "chevron-left",
            "chevron-right",
            "chevron-up",
            "chevrons-down",
            "chevrons-left",
            "chevrons-right",
            "chevrons-up",
            "circle",
            "clipboard",
            "clock",
            "cloud-drizzle",
            "cloud-lightning",
            "cloud-off",
            "cloud-rain",
            "cloud-snow",
            "cloud",
            "code",
            "command",
            "compass",
            "copy",
            "corner-down-left",
            "corner-down-right",
            "corner-left-down",
            "corner-left-up",
            "corner-right-down",
            "corner-right-up",
            "corner-up-left",
            "corner-up-right",
            "cpu",
            "credit-card",
            "crop",
            "crosshair",
            "database",
            "delete",
            "disc",
            "dollar-sign",
            "download-cloud",
            "download",
            "droplet",
            "edit-2",
            "edit-3",
            "edit",
            "external-link",
            "eye-off",
            "eye",
            "fast-forward",
            "feather",
            "file-minus",
            "file-plus",
            "file-text",
            "file",
            "film",
            "filter",
            "flag",
            "folder-minus",
            "folder-plus",
            "folder",
            "gift",
            "git-branch",
            "git-commit",
            "git-merge",
            "git-pull-request",
            "globe",
            "grid",
            "hard-drive",
            "hash",
            "headphones",
            "heart",
            "help-circle",
            "home",
            "image",
            "inbox",
            "info",
            "italic",
            "layers",
            "layout",
            "life-buoy",
            "link-2",
            "link",
            "list",
            "loader",
            "lock",
            "log-in",
            "log-out",
            "mail",
            "map-pin",
            "map",
            "maximize-2",
            "maximize",
            "menu",
            "message-circle",
            "message-square",
            "mic-off",
            "mic",
            "minimize-2",
            "minimize",
            "minus-circle",
            "minus-square",
            "minus",
            "monitor",
            "moon",
            "more-horizontal",
            "more-vertical",
            "move",
            "music",
            "navigation-2",
            "navigation",
            "octagon",
            "package",
            "paperclip",
            "pause-circle",
            "pause",
            "percent",
            "phone-call",
            "phone-forwarded",
            "phone-incoming",
            "phone-missed",
            "phone-off",
            "phone-outgoing",
            "phone",
            "pie-chart",
            "play-circle",
            "play",
            "plus-circle",
            "plus-square",
            "plus",
            "pocket",
            "power",
            "printer",
            "radio",
            "refresh-ccw",
            "refresh-cw",
            "repeat",
            "rewind",
            "rotate-ccw",
            "rotate-cw",
            "rss",
            "save",
            "scissors",
            "search",
            "send",
            "server",
            "settings",
            "share-2",
            "share",
            "shield-off",
            "shield",
            "shopping-bag",
            "shopping-cart",
            "shuffle",
            "sidebar",
            "skip-back",
            "skip-forward",
            "slash",
            "sliders",
            "smartphone",
            "speaker",
            "square",
            "star",
            "stop-circle",
            "sun",
            "sunrise",
            "sunset",
            "table",
            "tablet",
            "tag",
            "target",
            "terminal",
            "thermometer",
            "thumbs-down",
            "thumbs-up",
            "toggle-left",
            "toggle-right",
            "trash-2",
            "trash",
            "trending-down",
            "trending-up",
            "triangle",
            "truck",
            "tv",
            "type",
            "umbrella",
            "underline",
            "unlock",
            "upload-cloud",
            "upload",
            "user-check",
            "user-minus",
            "user-plus",
            "user-x",
            "user",
            "users",
            "video-off",
            "video",
            "voicemail",
            "volume-1",
            "volume-2",
            "volume-x",
            "volume",
            "watch",
            "wifi-off",
            "wifi",
            "wind",
            "x-circle",
            "x-square",
            "x",
            "zap-off",
            "zap",
            "zoom-in",
            "zoom-out",
        ])
    })
}

/// Returns whether the given Docker `image` refers to an image hosted on a well-known Docker
/// registry (as opposed to a local `Dockerfile`).
///
/// See <https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#runsimage>.
fn is_image_on_docker_registry(image: &str) -> bool {
    image.starts_with("docker://")
        || image.starts_with("gcr.io/")
        || image.starts_with("pkg.dev/")
        || image.starts_with("ghcr.io/")
        || image.starts_with("docker.io/")
}

/// The `action` rule.
#[derive(Debug)]
pub struct RuleAction {
    base: RuleBase,
    /// Cache of local action metadata for the repository being linted.
    cache: Arc<LocalActionsCache>,
    /// Local action specs whose metadata has already been checked, so that a local action
    /// referenced by multiple steps is only validated once (mirrors the Go implementation, which
    /// relies on the cache's `cached` flag).
    checked_local_actions: HashSet<String>,
}

impl RuleAction {
    /// Creates a new `action` rule. `cache` is the local action metadata cache for the repository
    /// being linted.
    #[must_use]
    pub fn new(cache: Arc<LocalActionsCache>) -> Self {
        Self {
            base: RuleBase::new(
                "action",
                "Checks for popular actions released on GitHub, local actions, and action calls at \"uses:\"",
            ),
            cache,
            checked_local_actions: HashSet::new(),
        }
    }

    fn visit_step_impl(&mut self, n: &Step) {
        let Exec::Action(e) = &n.exec else {
            return;
        };

        // A step with no `uses:` key (e.g. only `with:`, or an otherwise malformed step) parses to
        // an empty `uses` value here. The original implementation models a missing `uses` as a nil
        // pointer and skips it in `VisitStep`; mirror that by skipping empty specs so the action
        // rule stays silent and the missing `uses`/`run` is left to the syntax check.
        if e.uses.as_str().is_empty() {
            return;
        }

        if e.uses.contains_expression() {
            // Cannot parse specification made with interpolation. Give up.
            return;
        }

        let spec = e.uses.as_str();

        if spec.starts_with("./") {
            // Relative to repository root.
            self.check_local_action(spec, e);
            return;
        }

        if spec.starts_with("docker://") {
            self.check_docker_action(spec, e);
            return;
        }

        self.check_repo_action(spec, e);
    }

    /// Parses `{owner}/{repo}@{ref}` or `{owner}/{repo}/{path}@{ref}` and validates it against the
    /// popular actions data set.
    fn check_repo_action(&mut self, spec: &str, exec: &ExecAction) {
        let uses = &exec.uses;
        let Some(idx) = spec.find('@') else {
            self.invalid_action_format(uses.pos, spec, "ref is missing");
            return;
        };
        let r#ref = &spec[idx + 1..];
        let s = &spec[..idx]; // remove {ref}

        let Some(idx) = s.find('/') else {
            self.invalid_action_format(uses.pos, spec, "owner is missing");
            return;
        };

        let owner = &s[..idx];
        let s = &s[idx + 1..]; // eat {owner}

        let repo = match s.find('/') {
            Some(i) => &s[..i],
            None => s,
        };

        if owner.is_empty() || repo.is_empty() || r#ref.is_empty() {
            self.invalid_action_format(
                uses.pos,
                spec,
                "owner and repo and ref should not be empty",
            );
        }

        let Some(meta) = popular_actions().get(spec) else {
            if outdated_popular_action_specs().contains(spec) {
                self.base.error_at(
                    uses.pos,
                    format!(
                        "the runner of {spec:?} action is too old to run on GitHub Actions. update the action's version to fix this issue"
                    ),
                );
                return;
            }
            self.base.debug(format_args!(
                "This action is not found in popular actions data set: {spec}"
            ));
            return;
        };

        if meta.skip_inputs {
            self.base
                .debug(format_args!("This action skips to check inputs: {spec}"));
            return;
        }

        let describe = |_m: &ActionMetadata| format!("{spec:?}");
        self.check_action(meta, exec, describe);
    }

    fn invalid_action_format(&mut self, pos: Pos, spec: &str, why: &str) {
        self.base.error_at(
            pos,
            format!(
                "specifying action {spec:?} in invalid format because {why}. available formats are \"{{owner}}/{{repo}}@{{ref}}\" or \"{{owner}}/{{repo}}/{{path}}@{{ref}}\""
            ),
        );
    }

    fn missing_runs_prop(&mut self, pos: Pos, prop: &str, ty: &str, action: &str, path: &str) {
        self.base.error_at(
            pos,
            format!(
                "{prop:?} is required in \"runs\" section because {action:?} is a {ty} action. the action is defined at {path:?}"
            ),
        );
    }

    /// Reports `props` that are not allowed in the `runs` section for the given action type.
    fn check_invalid_runs_props(
        &mut self,
        pos: Pos,
        r: &ActionMetadataRuns,
        ty: &str,
        action: &str,
        path: &str,
        props: &[&str],
    ) {
        for prop in props {
            let invalid = match *prop {
                "main" => !r.main.is_empty(),
                "pre" => !r.pre.is_empty(),
                "pre-if" => !r.pre_if.is_empty(),
                "post" => !r.post.is_empty(),
                "post-if" => !r.post_if.is_empty(),
                "steps" => !r.steps.is_empty(),
                "image" => !r.image.is_empty(),
                "pre-entrypoint" => !r.pre_entrypoint.is_empty(),
                "entrypoint" => !r.entrypoint.is_empty(),
                "post-entrypoint" => !r.post_entrypoint.is_empty(),
                // Note: Go checks `r.Args != nil` (presence, not emptiness). Since an empty
                // `args:` key deserializes to an empty vec in both cases, length is equivalent
                // here.
                "args" => !r.args.is_empty(),
                // Note: Go checks `r.Env != nil` (presence). A present-but-empty `env:` map is
                // indistinguishable from an absent one after deserialization.
                "env" => !r.env.is_empty(),
                _ => false,
            };

            if invalid {
                self.base.error_at(
                    pos,
                    format!(
                        "{prop:?} is not allowed in \"runs\" section because {action:?} is a {ty} action. the action is defined at {path:?}"
                    ),
                );
            }
        }
    }

    fn check_runs_file_exists(&mut self, file: &str, dir: &Path, prop: &str, name: &str, pos: Pos) {
        if file.is_empty() {
            return;
        }
        let p = dir.join(file);
        if std::fs::metadata(&p).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
            self.base.error_at(
                pos,
                format!(
                    "file {file:?} does not exist in \"{dir}\". it is specified at {prop:?} key in \"runs\" section in {name:?} action",
                    dir = dir.display()
                ),
            );
        }
    }

    /// Checks the `runs` section of a local Docker action.
    ///
    /// See <https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#runs-for-docker-container-actions>.
    fn check_local_docker_action_runs(
        &mut self,
        r: &ActionMetadataRuns,
        dir: &Path,
        name: &str,
        pos: Pos,
    ) {
        if r.image.is_empty() {
            self.missing_runs_prop(pos, "image", "Docker", name, &dir.to_string_lossy());
        } else if !is_image_on_docker_registry(&r.image) {
            self.check_runs_file_exists(&r.image, dir, "image", name, pos);
            if Path::new(&r.image)
                .file_name()
                .is_none_or(|f| f != "Dockerfile")
            {
                self.base.error_at(
                    pos,
                    format!(
                        "the local file {:?} referenced from \"image\" key must be named \"Dockerfile\" in {name:?} action. the action is defined at \"{dir}\"",
                        r.image,
                        dir = dir.display()
                    ),
                );
            }
        }
        self.check_runs_file_exists(&r.pre_entrypoint, dir, "pre-entrypoint", name, pos);
        self.check_runs_file_exists(&r.entrypoint, dir, "entrypoint", name, pos);
        self.check_runs_file_exists(&r.post_entrypoint, dir, "post-entrypoint", name, pos);
        self.check_invalid_runs_props(
            pos,
            r,
            "Docker",
            name,
            &dir.to_string_lossy(),
            &["main", "pre", "pre-if", "post", "post-if", "steps"],
        );
    }

    /// Checks the `runs` section of a local composite action.
    ///
    /// See <https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#runs-for-composite-actions>.
    fn check_local_composite_action_runs(
        &mut self,
        r: &ActionMetadataRuns,
        dir: &Path,
        name: &str,
        pos: Pos,
    ) {
        if r.steps.is_empty() {
            self.missing_runs_prop(pos, "steps", "Composite", name, &dir.to_string_lossy());
        }
        self.check_invalid_runs_props(
            pos,
            r,
            "Composite",
            name,
            &dir.to_string_lossy(),
            &[
                "main",
                "pre",
                "pre-if",
                "post",
                "post-if",
                "image",
                "pre-entrypoint",
                "entrypoint",
                "post-entrypoint",
                "args",
                "env",
            ],
        );
    }

    /// Checks the `runs` section of a local JavaScript action.
    ///
    /// See <https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#runs-for-javascript-actions>.
    fn check_local_javascript_action_runs(
        &mut self,
        r: &ActionMetadataRuns,
        dir: &Path,
        name: &str,
        pos: Pos,
    ) {
        if r.main.is_empty() {
            self.missing_runs_prop(pos, "main", "JavaScript", name, &dir.to_string_lossy());
        } else {
            self.check_runs_file_exists(&r.main, dir, "main", name, pos);
        }

        self.check_runs_file_exists(&r.pre, dir, "pre", name, pos);
        if r.pre.is_empty() && !r.pre_if.is_empty() {
            self.base.error_at(
                pos,
                format!(
                    "\"pre\" is required when \"pre-if\" is specified in \"runs\" section in {name:?} action. the action is defined at \"{dir}\"",
                    dir = dir.display()
                ),
            );
        }

        self.check_runs_file_exists(&r.post, dir, "post", name, pos);
        if r.post.is_empty() && !r.post_if.is_empty() {
            self.base.error_at(
                pos,
                format!(
                    "\"post\" is required when \"post-if\" is specified in \"runs\" section in {name:?} action. the action is defined at \"{dir}\"",
                    dir = dir.display()
                ),
            );
        }

        self.check_invalid_runs_props(
            pos,
            r,
            "JavaScript",
            name,
            &dir.to_string_lossy(),
            &[
                "steps",
                "image",
                "pre-entrypoint",
                "entrypoint",
                "post-entrypoint",
                "args",
                "env",
            ],
        );
    }

    fn check_local_action_inputs(&mut self, meta: &ActionMetadata, pos: Pos) {
        for i in meta.inputs.values() {
            if i.deprecated && i.deprecation_message.is_empty() {
                self.base.error_at(
                    pos,
                    format!(
                        "input {:?} is deprecated but \"deprecationMessage\" is empty in metadata of {:?} action at \"{}\"",
                        i.name, meta.name, meta.path().display()
                    ),
                );
            }
        }
    }

    /// Checks the `runs` section of a local action.
    ///
    /// See <https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#runs>.
    fn check_local_action_runs(&mut self, meta: &ActionMetadata, pos: Pos) {
        let r = &meta.runs;
        match r.using.as_str() {
            "" => self.base.error_at(
                pos,
                format!(
                    "\"runs.using\" is missing in local action {:?} defined at \"{dir}\"",
                    meta.name,
                    dir = meta.dir().display()
                ),
            ),
            "docker" => {
                self.check_local_docker_action_runs(r, meta.dir(), &meta.name, pos);
            }
            "composite" => {
                self.check_local_composite_action_runs(r, meta.dir(), &meta.name, pos);
            }
            "node20" | "node24" => {
                self.check_local_javascript_action_runs(r, meta.dir(), &meta.name, pos);
            }
            _ => {
                self.base.error_at(
                    pos,
                    format!(
                        "invalid runner name {:?} at runs.using in {:?} action defined at \"{dir}\". valid runners are \"composite\", \"docker\", \"node20\", and \"node24\". see https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#runs",
                        r.using, meta.name, dir = meta.dir().display()
                    ),
                );

                // Probably invalid version of Node.js runner. Assume it is a JavaScript action to
                // find as many errors as possible.
                if r.using.starts_with("node") {
                    self.check_local_javascript_action_runs(r, meta.dir(), &meta.name, pos);
                }
            }
        }
    }

    /// Validates a `docker://` action reference.
    ///
    /// See <https://docs.github.com/en/actions/learn-github-actions/workflow-syntax-for-github-actions#example-using-the-github-packages-container-registry>.
    fn check_docker_action(&mut self, uri_full: &str, exec: &ExecAction) {
        let mut uri = uri_full;
        let mut tag = "";
        let mut tag_exists = false;
        let rest = &uri_full["docker://".len()..];
        if let Some(idx) = rest.find(':') {
            let idx = idx + "docker://".len();
            if idx < uri_full.len() {
                tag = &uri_full[idx + 1..];
                uri = &uri_full[..idx];
                tag_exists = true;
            }
        }

        // The Go implementation parses the URI with `net/url.Parse`. The `url` crate is not a
        // dependency of this project, and `url.Parse` almost never fails for these strings, so a
        // lightweight validation is done instead: reject control characters (which `url.Parse`
        // rejects) and otherwise accept the URI.
        if let Err(e) = validate_docker_uri(uri) {
            self.base.error_at(
                exec.uses.pos,
                format!("URI for Docker container {uri:?} is invalid: {e} (tag={tag})"),
            );
        }

        if tag_exists && tag.is_empty() {
            self.base.error_at(
                exec.uses.pos,
                format!("tag of Docker action should not be empty: {uri:?}"),
            );
        }
    }

    fn check_local_action_metadata(&mut self, meta: &ActionMetadata, action: &ExecAction) {
        let pos = action.uses.pos;
        let path = meta.path();

        if meta.name.is_empty() {
            self.base.error_at(
                pos,
                format!("name is required in action metadata \"{}\"", path.display()),
            );
        }
        if meta.description.is_empty() {
            self.base.error_at(
                pos,
                format!(
                    "description is required in metadata of {:?} action at \"{}\"",
                    meta.name,
                    path.display()
                ),
            );
        }
        if !meta.branding.icon.is_empty()
            && !branding_icons().contains(meta.branding.icon.to_lowercase().as_str())
        {
            self.base.error_at(
                pos,
                format!(
                    "incorrect icon name {:?} at branding.icon in metadata of {:?} action at \"{}\". see the official document to know the exhaustive list of supported icons: https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#brandingicon",
                    meta.branding.icon, meta.name, path.display()
                ),
            );
        }
        if !meta.branding.color.is_empty()
            && !branding_colors().contains(meta.branding.color.to_lowercase().as_str())
        {
            self.base.error_at(
                pos,
                format!(
                    "incorrect color {:?} at branding.icon in metadata of {:?} action at \"{}\". see the official document to know the exhaustive list of supported colors: https://docs.github.com/en/actions/creating-actions/metadata-syntax-for-github-actions#brandingcolor",
                    meta.branding.color, meta.name, path.display()
                ),
            );
        }
        self.check_local_action_inputs(meta, pos);
        self.check_local_action_runs(meta, pos);
    }

    /// Validates a local action (an action in the same repository as the workflow).
    ///
    /// See <https://docs.github.com/en/actions/learn-github-actions/workflow-syntax-for-github-actions#example-using-action-in-the-same-repository-as-the-workflow>.
    fn check_local_action(&mut self, spec: &str, action: &ExecAction) {
        let meta = match self.cache.find_metadata(spec) {
            Err(e) => {
                self.base.error_at(action.uses.pos, e);
                return;
            }
            Ok(None) => return,
            Ok(Some(m)) => m,
        };

        // Only check the metadata the first time this local action is referenced so that the same
        // errors are not reported for every step using it.
        if self.checked_local_actions.insert(spec.to_string()) {
            self.base.debug(format_args!(
                "Checking metadata of {:?} action {:?} at {spec:?}",
                meta.runs, meta.name
            ));
            self.check_local_action_metadata(&meta, action);
        }

        let describe = |m: &ActionMetadata| format!("{:?} defined at {:?}", m.name, spec);
        self.check_action(&meta, action, describe);
    }

    fn check_action(
        &mut self,
        meta: &ActionMetadata,
        exec: &ExecAction,
        describe: impl Fn(&ActionMetadata) -> std::string::String,
    ) {
        // Check specified inputs are defined in the action's inputs spec.
        for (id, i) in &exec.inputs {
            match meta.inputs.get(id) {
                None => {
                    let ns: Vec<&str> = meta.inputs.values().map(|i| i.name.as_str()).collect();
                    self.base.error_at(
                        i.name.pos,
                        format!(
                            "input {:?} is not defined in action {}. available inputs are {}",
                            i.name.value,
                            describe(meta),
                            sorted_quotes(ns)
                        ),
                    );
                }
                Some(m) if m.deprecated && !m.required => {
                    // Note: Using required inputs cannot be avoided. So we don't report it as an
                    // error (though this should not happen normally).
                    let mut msg = format!(
                        "avoid using deprecated input {:?} in action {}",
                        i.name.value,
                        describe(meta)
                    );
                    let d = re_newline_with_indent()
                        .replace_all(m.deprecation_message.trim_end_matches(['.', ' ']), " ");
                    if !d.is_empty() {
                        msg += ": ";
                        msg += &d;
                    }
                    self.base.error_at(i.name.pos, msg);
                }
                Some(_) => {}
            }
        }

        // Check mandatory inputs are specified.
        for (id, i) in &meta.inputs {
            if i.required && !exec.inputs.contains_key(id) {
                let ns: Vec<&str> = meta
                    .inputs
                    .values()
                    .filter(|i| i.required)
                    .map(|i| i.name.as_str())
                    .collect();
                self.base.error_at(
                    exec.uses.pos,
                    format!(
                        "missing input {:?} which is required by action {}. all required inputs are {}",
                        i.name,
                        describe(meta),
                        sorted_quotes(ns)
                    ),
                );
            }
        }
    }
}

/// A regular expression matching a newline and its surrounding indentation. Used to collapse
/// multi-line deprecation messages onto a single line.
fn re_newline_with_indent() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s*\r?\n\s*").expect("valid regex"))
}

/// Lightweight validation of a Docker image URI.
///
/// This mirrors the failure cases of Go's `net/url.Parse` that are reachable for a
/// `docker://<image>` reference. `net/url.Parse` rejects URIs containing ASCII control characters
/// or spaces; it accepts virtually every other image reference, so this check only rejects those.
fn validate_docker_uri(uri: &str) -> Result<(), String> {
    if let Some(c) = uri.chars().find(|c| c.is_control() || *c == ' ') {
        return Err(format!("invalid character {c:?} in URI"));
    }
    Ok(())
}

impl Pass for RuleAction {
    fn visit_step(&mut self, n: &Step) -> Result<(), String> {
        self.visit_step_impl(n);
        Ok(())
    }
}

impl Default for RuleAction {
    fn default() -> Self {
        // A null cache (no project root) never hits, so local actions are not resolved. This is
        // used when no project is available (e.g. in tests).
        Self::new(Arc::new(LocalActionsCache::new(None)))
    }
}

impl Rule for RuleAction {
    fn take_errors(&mut self) -> Vec<crate::error::Error> {
        self.base.take_errors()
    }

    fn name(&self) -> &'static str {
        self.base.name()
    }

    fn description(&self) -> &'static str {
        self.base.description()
    }

    fn enable_debug(&mut self, out: Box<dyn std::io::Write + Send>) {
        self.base.enable_debug(out);
    }

    fn set_config(&mut self, config: crate::config::Config) {
        self.base.set_config(config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::String;
    use std::path::PathBuf;

    /// Builds a step whose `uses:` is `spec` with the given `with:` input names.
    fn action_step(spec: &str, inputs: &[&str]) -> Step {
        let mut map = std::collections::BTreeMap::new();
        for (n, name) in inputs.iter().enumerate() {
            map.insert(
                (*name).to_string(),
                crate::ast::Input {
                    name: String::at(*name, Pos::new(3, 7 + n)),
                    value: String::at("v", Pos::new(3, 7 + n)),
                },
            );
        }
        Step {
            id: None,
            if_condition: None,
            name: None,
            exec: Exec::Action(ExecAction {
                uses: String::at(spec, Pos::new(2, 5)),
                inputs: map,
                entrypoint: None,
                args: None,
            }),
            env: None,
            continue_on_error: None,
            timeout_minutes: None,
            pos: Pos::new(2, 3),
        }
    }

    #[test]
    fn name_and_description() {
        let r = RuleAction::default();
        assert_eq!(r.name(), "action");
        assert!(r.description().contains("popular actions"));
    }

    #[test]
    fn invalid_action_format_missing_ref() {
        let mut r = RuleAction::default();
        r.visit_step(&action_step("actions/checkout", &[])).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("ref is missing"), "{:?}", errs[0]);
        assert_eq!(errs[0].line, 2);
        assert_eq!(errs[0].column, 5);
    }

    #[test]
    fn invalid_action_format_missing_owner() {
        let mut r = RuleAction::default();
        // No `/` at all, so the owner cannot be parsed.
        r.visit_step(&action_step("checkout@v4", &[])).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0].message.contains("owner is missing"),
            "{:?}",
            errs[0]
        );
    }

    #[test]
    fn invalid_action_format_empty_owner() {
        let mut r = RuleAction::default();
        // A leading `/` yields an empty owner after parsing.
        r.visit_step(&action_step("/checkout@v4", &[])).unwrap();
        let errs = r.take_errors();
        assert!(
            errs[0]
                .message
                .contains("owner and repo and ref should not be empty"),
            "{:?}",
            errs[0]
        );
    }

    #[test]
    fn popular_action_valid_input() {
        // `actions/checkout@v4` is a known popular action with no required inputs.
        let mut r = RuleAction::default();
        r.visit_step(&action_step("actions/checkout@v4", &["token"]))
            .unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn popular_action_undefined_input() {
        let mut r = RuleAction::default();
        r.visit_step(&action_step("actions/checkout@v4", &["no-such-input"]))
            .unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0].message.contains("is not defined in action"),
            "{:?}",
            errs[0]
        );
        assert!(errs[0].message.contains("available inputs are"));
        // Error is reported at the input's position.
        assert_eq!(errs[0].line, 3);
    }

    #[test]
    fn popular_action_missing_required_input() {
        // `actions/add-to-project@v1.0.1` requires `github-token` and `project-url`.
        let mut r = RuleAction::default();
        r.visit_step(&action_step("actions/add-to-project@v1.0.1", &[]))
            .unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 2);
        for e in &errs {
            assert!(
                e.message.contains("which is required by action"),
                "{:?}",
                e.message
            );
            assert!(e.message.contains("all required inputs are"));
        }
        let joined = errs.iter().map(|e| e.message.as_str()).collect::<Vec<_>>();
        assert!(joined.iter().any(|m| m.contains("github-token")));
        assert!(joined.iter().any(|m| m.contains("project-url")));
    }

    #[test]
    fn outdated_action_spec() {
        // `actions/checkout@v2` is a known outdated popular action spec.
        let mut r = RuleAction::default();
        r.visit_step(&action_step("actions/checkout@v2", &[]))
            .unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .contains("is too old to run on GitHub Actions"),
            "{:?}",
            errs[0]
        );
    }

    #[test]
    fn unknown_repo_action_is_ignored() {
        let mut r = RuleAction::default();
        r.visit_step(&action_step("some-owner/some-repo@v1", &[]))
            .unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn expression_uses_is_skipped() {
        let mut r = RuleAction::default();
        r.visit_step(&action_step("${{ matrix.action }}", &[]))
            .unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn docker_action_valid() {
        let mut r = RuleAction::default();
        r.visit_step(&action_step("docker://alpine:3.18", &[]))
            .unwrap();
        assert!(r.take_errors().is_empty());
    }

    #[test]
    fn docker_action_empty_tag() {
        let mut r = RuleAction::default();
        r.visit_step(&action_step("docker://alpine:", &[])).unwrap();
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0]
                .message
                .contains("tag of Docker action should not be empty"),
            "{:?}",
            errs[0]
        );
    }

    #[test]
    fn branding_tables_are_complete() {
        assert_eq!(branding_colors().len(), 9);
        assert!(branding_colors().contains("gray-dark"));
        // A generous lower bound; the exact icon count is large (~200).
        assert!(branding_icons().len() > 190, "{}", branding_icons().len());
        assert!(branding_icons().contains("zap"));
        assert!(branding_icons().contains("git-pull-request"));
    }

    #[test]
    fn is_image_on_docker_registry_prefixes() {
        assert!(is_image_on_docker_registry("docker://alpine:3"));
        assert!(is_image_on_docker_registry("gcr.io/foo/bar"));
        assert!(is_image_on_docker_registry("pkg.dev/proj/repo/img"));
        assert!(is_image_on_docker_registry("ghcr.io/owner/img"));
        assert!(is_image_on_docker_registry("docker.io/library/alpine"));
        assert!(!is_image_on_docker_registry("./Dockerfile"));
        assert!(!is_image_on_docker_registry("Dockerfile"));
    }

    #[test]
    fn docker_uri_validation() {
        assert!(validate_docker_uri("alpine:3.18").is_ok());
        assert!(validate_docker_uri("ghcr.io/owner/img").is_ok());
        assert!(validate_docker_uri("bad\u{0}uri").is_err());
        assert!(validate_docker_uri("bad uri").is_err());
    }

    /// Builds a local action directory with the given `action.yml` contents and returns a cache
    /// rooted at the temp directory plus the root path. `name` must be unique per test so that
    /// tests running in parallel do not share a directory.
    fn local_action_cache(name: &str, contents: &str) -> (Arc<LocalActionsCache>, PathBuf) {
        let root = std::env::temp_dir().join(format!("actionlint_rule_action_test_{name}"));
        let dir = root.join("my-action");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("action.yml"), contents).unwrap();
        let cache = Arc::new(LocalActionsCache::new(Some(root.clone())));
        (cache, root)
    }

    #[test]
    fn local_action_missing_name_and_description_and_bad_branding() {
        let contents = r#"
name: ""
description: ""
branding:
  icon: "not-an-icon"
  color: "not-a-color"
runs:
  using: composite
"#;
        let (cache, root) = local_action_cache("bad_meta", contents);
        let mut r = RuleAction::new(cache);
        r.visit_step(&action_step("./my-action", &[])).unwrap();
        let errs = r.take_errors();
        let msgs: Vec<&str> = errs.iter().map(|e| e.message.as_str()).collect();
        assert!(
            msgs.iter().any(|m| m.contains("name is required")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("description is required")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("incorrect icon name")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("incorrect color")),
            "{msgs:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn local_action_valid_metadata_has_no_metadata_errors() {
        let contents = r#"
name: My Action
description: Does things
branding:
  icon: "zap"
  color: "green"
runs:
  using: composite
  steps:
    - run: echo hi
"#;
        let (cache, root) = local_action_cache("good_meta", contents);
        let mut r = RuleAction::new(cache);
        r.visit_step(&action_step("./my-action", &[])).unwrap();
        let errs = r.take_errors();
        assert!(errs.is_empty(), "{errs:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn local_action_missing_runs_using() {
        let contents = "name: A\ndescription: D\nruns: {}\n";
        let (cache, root) = local_action_cache("missing_using", contents);
        let mut r = RuleAction::new(cache);
        r.visit_step(&action_step("./my-action", &[])).unwrap();
        let errs = r.take_errors();
        assert!(
            errs.iter()
                .any(|e| e.message.contains("\"runs.using\" is missing")),
            "{errs:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn local_action_invalid_runner() {
        let contents = "name: A\ndescription: D\nruns:\n  using: node99\n";
        let (cache, root) = local_action_cache("bad_runner", contents);
        let mut r = RuleAction::new(cache);
        r.visit_step(&action_step("./my-action", &[])).unwrap();
        let errs = r.take_errors();
        assert!(
            errs.iter()
                .any(|e| e.message.contains("invalid runner name")),
            "{errs:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn local_action_metadata_checked_once() {
        let contents = "description: D\nruns:\n  using: composite\n";
        let (cache, root) = local_action_cache("checked_once", contents);
        let mut r = RuleAction::new(cache);
        // The same local action referenced twice should report the missing-name error once.
        r.visit_step(&action_step("./my-action", &[])).unwrap();
        r.visit_step(&action_step("./my-action", &[])).unwrap();
        let errs = r.take_errors();
        let name_errs = errs
            .iter()
            .filter(|e| e.message.contains("name is required"))
            .count();
        assert_eq!(name_errs, 1, "{errs:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deprecated_input_warns() {
        // `check_action` warns when a deprecated, non-required input is specified.
        let mut md = ActionMetadata::default();
        md.name = "My Action".to_string();
        let mut inputs = crate::action_metadata::ActionMetadataInputs::new();
        inputs.insert(
            "old-input".to_string(),
            crate::action_metadata::ActionMetadataInput {
                name: "old-input".to_string(),
                required: false,
                deprecated: true,
                deprecation_message: "use new-input instead.".to_string(),
            },
        );
        inputs.insert(
            "new-input".to_string(),
            crate::action_metadata::ActionMetadataInput {
                name: "new-input".to_string(),
                required: false,
                deprecated: false,
                deprecation_message: std::string::String::new(),
            },
        );
        md.inputs = inputs;

        let mut r = RuleAction::default();
        let exec = ExecAction {
            uses: String::at("owner/repo@v1", Pos::new(2, 5)),
            inputs: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(
                    "old-input".to_string(),
                    crate::ast::Input {
                        name: String::at("old-input", Pos::new(3, 9)),
                        value: String::at("v", Pos::new(3, 9)),
                    },
                );
                m
            },
            entrypoint: None,
            args: None,
        };
        let describe = |m: &ActionMetadata| format!("{:?}", m.name);
        r.check_action(&md, &exec, describe);
        let errs = r.take_errors();
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0].message.contains("avoid using deprecated input"),
            "{:?}",
            errs[0]
        );
        assert!(errs[0].message.contains("use new-input instead"));
        assert_eq!(errs[0].line, 3);
    }
}
