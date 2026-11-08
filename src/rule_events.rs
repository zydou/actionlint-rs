//! The `events` rule.
//!
//! Checks: Checks for workflow trigger events at `on:`. Rust counterpart of the original
//! `rule_events.go`. See
//! <https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows>.

use croner::parser::{CronParser, Seconds, Year};

use crate::all_webhooks::all_webhook_types;
use crate::ast::{
    Event, Pos, String, WebhookEvent, WebhookEventFilter, Workflow, WorkflowCallEvent,
    WorkflowCallEventInputType, WorkflowDispatchEvent, WorkflowDispatchEventInputType,
};
use crate::quotes::{quote, quotes, sorted_quotes};
use crate::rule::{Pass, Rule, RuleBase};

/// The `events` rule.
#[derive(Debug)]
pub struct RuleEvents {
    base: RuleBase,
}

impl RuleEvents {
    /// Creates a new `events` rule.
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: RuleBase::new("events", "Checks for workflow trigger events at \"on:\""),
        }
    }

    fn check_event(&mut self, event: &Event) {
        match event {
            Event::Scheduled(e) => {
                for s in &e.schedules {
                    self.check_cron(&s.cron);
                    if let Some(tz) = &s.timezone {
                        self.check_timezone(tz);
                    }
                }
            }
            Event::WorkflowDispatch(e) => self.check_workflow_dispatch_event(e),
            // Nothing to check for `repository_dispatch`.
            Event::RepositoryDispatch(_) => {}
            Event::WorkflowCall(e) => self.check_workflow_call_event(e),
            Event::Webhook(e) => self.check_webhook_event(e),
            // Nothing to check for `image_version`.
            Event::ImageVersion(_) => {}
        }
    }

    /// Checks the CRON format of a `schedule` entry and that it does not run more often than
    /// every 5 minutes.
    ///
    /// The original uses `robfig/cron` configured for exactly the 5 fields minute, hour,
    /// day-of-month, month and day-of-week, which also rejects `@descriptor` nicknames. The
    /// [`croner`] parser used here silently expands those nicknames (e.g. `@daily`) into a valid
    /// 5-field spec, so descriptors are rejected up front to keep the behavior aligned.
    fn check_cron(&mut self, spec: &String) {
        if spec.value.contains('@') {
            self.base.error_at(
                spec.pos,
                format!(
                    "invalid CRON format {} in schedule event: parser does not accept descriptors",
                    quotes(&[&spec.value])
                ),
            );
            return;
        }

        // Go's `robfig/cron` validates the field count before any other check and phrases the
        // failure as `expected exactly 5 fields, found M: [spec]`. croner phrases the same failure
        // differently, so the count is checked up front to keep the message byte-aligned.
        let field_count = spec.value.split_whitespace().count();
        if field_count != 5 {
            self.base.error_at(
                spec.pos,
                format!(
                    "invalid CRON format {} in schedule event: expected exactly 5 fields, found {field_count}: [{}]",
                    quotes(&[&spec.value]),
                    spec.value
                ),
            );
            return;
        }

        let parser = CronParser::builder()
            .seconds(Seconds::Disallowed)
            .year(Year::Disallowed)
            .build();
        let cron = match parser.parse(&spec.value) {
            Ok(c) => c,
            Err(err) => {
                self.base.error_at(
                    spec.pos,
                    format!(
                        "invalid CRON format {} in schedule event: {err}",
                        quotes(&[&spec.value])
                    ),
                );
                return;
            }
        };

        // Compute the interval between the first two fire times after the Unix epoch. Both the
        // original (`sched.Next(time.Unix(0, 0))`) and croner (given a UTC `DateTime`) evaluate
        // the schedule in UTC, so the resulting interval is the same.
        let start = chrono::DateTime::UNIX_EPOCH;
        let Ok(first) = cron.find_next_occurrence(&start, false) else {
            return;
        };
        let Ok(second) = cron.find_next_occurrence(&first, false) else {
            return;
        };
        let diff = (second - first).num_seconds() as f64;

        // (#14) <https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#scheduled-events>
        //
        // > The shortest interval you can run scheduled workflows is once every 5 minutes.
        if diff < 60.0 * 5.0 {
            self.base.error_at(
                spec.pos,
                format!(
                    "scheduled job runs too frequently. it runs once per {diff} seconds. the shortest interval is once every 5 minutes"
                ),
            );
        }
    }

    /// Checks that the timezone of a `schedule` entry is a valid IANA timezone name.
    ///
    /// The original validates the name against the full IANA database via Go's `time.LoadLocation`
    /// and separately rejects the special values `""`, `"UTC"` and `"Local"`, which `LoadLocation`
    /// accepts but which are not correct IANA timezone names.
    ///
    /// NOTE: This port does not link the IANA tz database (that would require the heavy
    /// `chrono-tz` dependency, which is not part of this project), so only the *format* of the
    /// name is validated here. A well-formed but nonexistent zone such as `America/Nowhere` is
    /// therefore not reported by this rule. This is a known, intentional limitation.
    fn check_timezone(&mut self, tz: &String) {
        let v = tz.value.as_str();
        // Go's `time.LoadLocation` accepts the special names "", "UTC" and "Local", but the
        // original explicitly rejects them because they are not valid IANA zone names. Otherwise a
        // name is valid only if it resolves in the IANA tz database, which `chrono_tz::Tz`
        // validates against its embedded copy of the database.
        let special =
            v.is_empty() || v.eq_ignore_ascii_case("utc") || v.eq_ignore_ascii_case("local");
        if special || v.parse::<chrono_tz::Tz>().is_err() {
            self.base.error_at(
                tz.pos,
                format!(
                    "invalid timezone {} in schedule event. it must be a valid IANA timezone name",
                    quotes(&[v])
                ),
            );
        }
    }

    fn filter_not_available(&mut self, pos: Pos, filter: &str, hook: &str, available: &[&str]) {
        let e = if available.len() < 2 {
            "event"
        } else {
            "events"
        };
        self.base.error_at(
            pos,
            format!(
                "{} filter is not available for {hook} event. it is only for {} {e}",
                quotes(&[filter]),
                available.join(", ")
            ),
        );
    }

    fn check_exclusive_filters(
        &mut self,
        filter: Option<&WebhookEventFilter>,
        ignore: Option<&WebhookEventFilter>,
        hook: &str,
        available: &[&str],
    ) {
        if !available.contains(&hook) {
            if let Some(f) = filter
                && !f.is_empty()
            {
                self.filter_not_available(f.name.pos, &f.name.value, hook, available);
            }
            if let Some(i) = ignore
                && !i.is_empty()
            {
                self.filter_not_available(i.name.pos, &i.name.value, hook, available);
            }
            return;
        }

        // The filters are only available for this hook, so report a conflict when both are set.
        if let (Some(f), Some(i)) = (filter, ignore)
            && !f.is_empty()
            && !i.is_empty()
        {
            let mut p = f.name.pos;
            if p.is_before(&i.name.pos) {
                p = i.name.pos;
            }
            self.base.error_at(
                p,
                format!(
                    "both {} and {} filters cannot be used for the same event {}. note: use '!' to negate patterns",
                    quotes(&[&f.name.value]),
                    quotes(&[&i.name.value]),
                    quotes(&[hook])
                ),
            );
        }
    }

    fn check_webhook_event(&mut self, event: &WebhookEvent) {
        let hook = event.hook.value.as_str();

        let Some(types) = all_webhook_types().get(hook) else {
            self.base.error_at(
                event.pos,
                format!(
                    "unknown Webhook event {}. see https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#webhook-events for list of all Webhook event names",
                    quotes(&[hook])
                ),
            );
            return;
        };

        self.check_types(&event.hook, &event.types, *types);

        if hook == "workflow_run" {
            if event.workflows.is_empty() {
                self.base.error_at(
                    event.pos,
                    "no workflow is configured for \"workflow_run\" event",
                );
            }
        } else if !event.workflows.is_empty() {
            self.base.error_at(
                event.pos,
                format!(
                    "\"workflows\" cannot be configured for {} event. it is only for workflow_run event",
                    quotes(&[hook])
                ),
            );
        }

        // Some filters are available with specific events and exclusive
        // - on.merge_group.<branches|branches-ignore>
        // - on.<push|pull_request|pull_request_target>.<paths|paths-ignore>
        // - on.push.<branches|tags|branches-ignore|tags-ignore>
        // - on.<pull_request|pull_request_target>.<branches|branches-ignore>
        // - on.workflow_run.<branches|branches-ignore>
        self.check_exclusive_filters(
            event.paths.as_ref(),
            event.paths_ignore.as_ref(),
            hook,
            &["push", "pull_request", "pull_request_target"],
        );
        self.check_exclusive_filters(
            event.branches.as_ref(),
            event.branches_ignore.as_ref(),
            hook,
            &[
                "merge_group",
                "push",
                "pull_request",
                "pull_request_target",
                "workflow_run",
            ],
        );
        self.check_exclusive_filters(
            event.tags.as_ref(),
            event.tags_ignore.as_ref(),
            hook,
            &["push"],
        );
    }

    fn check_types(&mut self, hook: &String, types: &[String], expected: Option<&[&str]>) {
        // A value of `None` means the activity types are arbitrary (Go's `nil` entry in the table).
        let expected = expected.unwrap_or(&[]);

        if expected.is_empty() && !types.is_empty() {
            self.base.error_at(
                hook.pos,
                format!(
                    "\"types\" cannot be specified for {} Webhook event",
                    quotes(&[&hook.value])
                ),
            );
            return;
        }

        for ty in types {
            if !expected.contains(&ty.value.as_str()) {
                self.base.error_at(
                    ty.pos,
                    format!(
                        "invalid activity type {} for {} Webhook event. available types are {}",
                        quotes(&[&ty.value]),
                        quotes(&[&hook.value]),
                        sorted_quotes(expected.to_vec())
                    ),
                );
            }
        }
    }

    fn check_workflow_call_event(&mut self, event: &WorkflowCallEvent) {
        for i in &event.inputs {
            let Some(default) = &i.default else {
                continue;
            };
            // A `${{ }}` expression is allowed in the default value, so nothing can be checked.
            if !default.contains_expression() {
                match i.input_type {
                    WorkflowCallEventInputType::Number => {
                        if let Err(err) = go_parse_float(&default.value) {
                            self.base.error_at(
                                default.pos,
                                format!(
                                    "input of workflow_call event {} is typed as number but its default value {} cannot be parsed as a float number: {err}",
                                    quotes(&[&i.name.value]),
                                    quotes(&[&default.value])
                                ),
                            );
                        }
                    }
                    WorkflowCallEventInputType::Boolean => {
                        let d = default.value.to_lowercase();
                        if d != "true" && d != "false" {
                            self.base.error_at(
                                default.pos,
                                format!(
                                    "input of workflow_call event {} is typed as boolean. its default value must be true or false but got {}",
                                    quotes(&[&i.name.value]),
                                    quotes(&[&default.value])
                                ),
                            );
                        }
                    }
                    _ => {}
                }
            }
            if i.is_required() {
                self.base.error_at(
                    default.pos,
                    format!(
                        "input {} of workflow_call event has the default value {}, but it is also required. if an input is marked as required, its default value will never be used",
                        quotes(&[&i.name.value]),
                        quotes(&[&default.value])
                    ),
                );
            }
        }
    }

    fn check_workflow_dispatch_event(&mut self, event: &WorkflowDispatchEvent) {
        for (n, i) in &event.inputs {
            let n = n.as_str();
            if i.input_type == WorkflowDispatchEventInputType::Choice {
                if i.options.is_empty() {
                    self.base.error_at(
                        i.name.pos,
                        format!(
                            "input type of {} is \"choice\" but \"options\" is not set",
                            quotes(&[n])
                        ),
                    );
                    continue;
                }
                let mut seen = std::collections::HashSet::with_capacity(i.options.len());
                for o in &i.options {
                    if !seen.insert(o.value.as_str()) {
                        self.base.error_at(
                            o.pos,
                            format!(
                                "option {} is duplicated in options of {} input",
                                quotes(&[&o.value]),
                                quotes(&[n])
                            ),
                        );
                    }
                }
                if let Some(default) = &i.default {
                    let options = i
                        .options
                        .iter()
                        .map(|o| o.value.as_str())
                        .collect::<Vec<_>>();
                    if !seen.contains(default.value.as_str()) {
                        // The options are each quoted and joined, then the whole list is quoted
                        // again (Go wraps the joined string in `%q`), escaping the inner quotes.
                        let options = quote(&quotes(&options));
                        self.base.error_at(
                            default.pos,
                            format!(
                                "default value {} of {} input is not included in its options {options}",
                                quotes(&[&default.value]),
                                quotes(&[n]),
                            ),
                        );
                    }
                }
            } else {
                if !i.options.is_empty() {
                    self.base.error_at(
                        i.name.pos,
                        format!(
                            "\"options\" can not be set to {} input because its input type is not \"choice\"",
                            quotes(&[n])
                        ),
                    );
                }
                if let Some(default) = &i.default {
                    match i.input_type {
                        WorkflowDispatchEventInputType::Number => {
                            if let Err(err) = go_parse_float(&default.value) {
                                self.base.error_at(
                                    default.pos,
                                    format!(
                                        "type of {} input is \"number\" but its default value {} cannot be parsed as a float number: {err}",
                                        quotes(&[&i.name.value]),
                                        quotes(&[&default.value])
                                    ),
                                );
                            }
                        }
                        WorkflowDispatchEventInputType::Boolean => {
                            let d = default.value.to_lowercase();
                            if d != "true" && d != "false" {
                                self.base.error_at(
                                    default.pos,
                                    format!(
                                        "type of {} input is \"boolean\". its default value {} must be \"true\" or \"false\"",
                                        quotes(&[n]),
                                        quotes(&[&default.value])
                                    ),
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // Maximum number of inputs is 25
        // <https://docs.github.com/en/actions/using-workflows/events-that-trigger-workflows#providing-inputs>
        if event.inputs.len() > 25 {
            self.base.error_at(
                event.pos,
                format!(
                    "maximum number of inputs for \"workflow_dispatch\" event is 25 but {} inputs are provided. see https://docs.github.com/en/actions/using-workflows/events-that-trigger-workflows#providing-inputs",
                    event.inputs.len()
                ),
            );
        }
    }
}

impl Default for RuleEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl Pass for RuleEvents {
    fn visit_workflow_pre(&mut self, n: &Workflow) -> Result<(), std::string::String> {
        for e in &n.on {
            self.check_event(e);
        }
        Ok(())
    }
}

impl Rule for RuleEvents {
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

/// Parses `s` as a 64-bit float the way Go's `strconv.ParseFloat` does, returning a Go-formatted
/// error message on failure.
///
/// The original reports the raw `strconv.ParseFloat` error, which reads
/// `strconv.ParseFloat: parsing "X": invalid syntax` for malformed input or
/// `... value out of range` when a finite-looking literal overflows. Rust's `f64::from_str` only
/// errors on malformed input (it yields infinity on overflow), so the overflow case is detected by
/// checking whether a non-infinity spelling parsed to an infinite value.
fn go_parse_float(s: &str) -> Result<f64, std::string::String> {
    const PREFIX: &str = "strconv.ParseFloat: parsing";
    match s.parse::<f64>() {
        Ok(v) => {
            let is_inf_spelling = {
                let t = s.trim().trim_start_matches(['+', '-']).to_ascii_lowercase();
                t == "inf" || t == "infinity"
            };
            if v.is_infinite() && !is_inf_spelling {
                Err(format!("{PREFIX} {}: value out of range", quotes(&[s])))
            } else {
                Ok(v)
            }
        }
        Err(_) => Err(format!("{PREFIX} {}: invalid syntax", quotes(&[s]))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        Bool, DispatchInput, ScheduleEntry, ScheduledEvent, WorkflowCallEventInput,
        WorkflowDispatchEventInputType,
    };
    use std::collections::BTreeMap;

    fn at(value: &str, pos: Pos) -> String {
        String::at(value, pos)
    }

    fn workflow(on: Vec<Event>) -> Workflow {
        Workflow {
            name: None,
            run_name: None,
            on,
            permissions: None,
            env: None,
            defaults: None,
            concurrency: None,
            jobs: BTreeMap::default(),
        }
    }

    fn run(on: Vec<Event>) -> Vec<crate::error::Error> {
        let mut r = RuleEvents::new();
        let wf = workflow(on);
        r.visit_workflow_pre(&wf).unwrap();
        r.take_errors()
    }

    fn scheduled(cron: &str, tz: Option<&str>) -> Event {
        Event::Scheduled(ScheduledEvent {
            schedules: vec![ScheduleEntry {
                cron: at(cron, Pos::new(2, 5)),
                timezone: tz.map(|t| at(t, Pos::new(2, 30))),
            }],
            pos: Pos::new(2, 3),
        })
    }

    fn webhook(hook: &str, pos: Pos) -> WebhookEvent {
        WebhookEvent {
            hook: at(hook, pos),
            types: vec![],
            branches: None,
            branches_ignore: None,
            tags: None,
            tags_ignore: None,
            paths: None,
            paths_ignore: None,
            workflows: vec![],
            pos,
        }
    }

    #[test]
    fn valid_cron() {
        let errs = run(vec![scheduled("*/30 * * * *", None)]);
        assert!(errs.is_empty(), "unexpected: {errs:?}");
    }

    #[test]
    fn too_frequent_cron() {
        // Runs every 2 minutes -> below the 5 minute minimum.
        let errs = run(vec![scheduled("*/2 * * * *", None)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("runs too frequently"));
        assert!(errs[0].message.contains("once per 120 seconds"));
    }

    #[test]
    fn invalid_cron() {
        let errs = run(vec![scheduled("not a cron", None)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("invalid CRON format"));
    }

    #[test]
    fn descriptor_cron_rejected() {
        // croner would silently accept `@daily`; the original rejects descriptors.
        let errs = run(vec![scheduled("@daily", None)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("invalid CRON format"));
    }

    #[test]
    fn valid_timezone() {
        let errs = run(vec![scheduled("0 0 * * *", Some("America/New_York"))]);
        assert!(errs.is_empty(), "unexpected: {errs:?}");
    }

    #[test]
    fn invalid_timezone_special_values() {
        for tz in ["", "UTC", "utc", "Local", "LOCAL"] {
            let errs = run(vec![scheduled("0 0 * * *", Some(tz))]);
            assert_eq!(errs.len(), 1, "tz={tz}: {errs:?}");
            assert!(errs[0].message.contains("invalid timezone"), "tz={tz}");
        }
    }

    #[test]
    fn invalid_timezone_bad_format() {
        for tz in ["/UTC", "America//New_York", "has space"] {
            let errs = run(vec![scheduled("0 0 * * *", Some(tz))]);
            assert_eq!(errs.len(), 1, "tz={tz}: {errs:?}");
        }
    }

    #[test]
    fn unknown_webhook_event() {
        let errs = run(vec![Event::Webhook(webhook(
            "no_such_hook",
            Pos::new(2, 3),
        ))]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("unknown Webhook event"));
    }

    #[test]
    fn invalid_activity_type() {
        let mut e = webhook("check_run", Pos::new(2, 3));
        e.types = vec![at("bogus", Pos::new(3, 7))];
        let errs = run(vec![Event::Webhook(e)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("invalid activity type"));
        assert!(errs[0].message.contains("available types are"));
    }

    #[test]
    fn types_cannot_be_specified() {
        // `push` has no activity types.
        let mut e = webhook("push", Pos::new(2, 3));
        e.types = vec![at("created", Pos::new(3, 7))];
        let errs = run(vec![Event::Webhook(e)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("\"types\" cannot be specified"));
    }

    #[test]
    fn arbitrary_types_allowed_for_repository_dispatch() {
        let errs = run(vec![Event::Webhook(webhook(
            "repository_dispatch",
            Pos::new(2, 3),
        ))]);
        // `repository_dispatch` is a known hook with arbitrary types; no `types` error.
        assert!(errs.is_empty(), "unexpected: {errs:?}");
    }

    #[test]
    fn workflow_run_requires_workflows() {
        let errs = run(vec![Event::Webhook(webhook(
            "workflow_run",
            Pos::new(2, 3),
        ))]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("no workflow is configured"));
    }

    #[test]
    fn workflows_only_for_workflow_run() {
        let mut e = webhook("push", Pos::new(2, 3));
        e.workflows = vec![at("other", Pos::new(3, 7))];
        let errs = run(vec![Event::Webhook(e)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(
            errs[0]
                .message
                .contains("\"workflows\" cannot be configured")
        );
    }

    #[test]
    fn exclusive_filters_conflict_on_push() {
        let mut e = webhook("push", Pos::new(2, 3));
        e.paths = Some(WebhookEventFilter {
            name: at("paths", Pos::new(3, 3)),
            values: vec![at("a/**", Pos::new(3, 9))],
        });
        e.paths_ignore = Some(WebhookEventFilter {
            name: at("paths-ignore", Pos::new(4, 3)),
            values: vec![at("b/**", Pos::new(4, 16))],
        });
        let errs = run(vec![Event::Webhook(e)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(
            errs[0]
                .message
                .contains("cannot be used for the same event")
        );
        // The reported position is the later of the two filters.
        assert_eq!(errs[0].line, 4);
    }

    #[test]
    fn tags_filter_not_available_for_issues() {
        let mut e = webhook("issues", Pos::new(2, 3));
        e.tags = Some(WebhookEventFilter {
            name: at("tags", Pos::new(3, 3)),
            values: vec![at("v1", Pos::new(3, 8))],
        });
        let errs = run(vec![Event::Webhook(e)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("filter is not available"));
        // Only one available event ("push"), so the singular "event" is used.
        assert!(
            errs[0]
                .message
                .contains("for issues event. it is only for push event")
        );
    }

    #[test]
    fn branches_filter_available_for_push_no_conflict() {
        let mut e = webhook("push", Pos::new(2, 3));
        e.branches = Some(WebhookEventFilter {
            name: at("branches", Pos::new(3, 3)),
            values: vec![at("main", Pos::new(3, 12))],
        });
        let errs = run(vec![Event::Webhook(e)]);
        assert!(errs.is_empty(), "unexpected: {errs:?}");
    }

    fn call_input(
        name: &str,
        input_type: WorkflowCallEventInputType,
        default: Option<&str>,
    ) -> WorkflowCallEventInput {
        WorkflowCallEventInput {
            name: at(name, Pos::new(3, 5)),
            description: None,
            default: default.map(|d| at(d, Pos::new(5, 7))),
            required: None,
            input_type,
            id: name.to_lowercase(),
        }
    }

    #[test]
    fn workflow_call_invalid_number_default() {
        let e = WorkflowCallEvent {
            inputs: vec![call_input(
                "num",
                WorkflowCallEventInputType::Number,
                Some("notanumber"),
            )],
            ..Default::default()
        };
        let errs = run(vec![Event::WorkflowCall(e)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(
            errs[0]
                .message
                .contains("cannot be parsed as a float number")
        );
    }

    #[test]
    fn workflow_call_valid_number_default() {
        let e = WorkflowCallEvent {
            inputs: vec![call_input(
                "num",
                WorkflowCallEventInputType::Number,
                Some("3.14"),
            )],
            ..Default::default()
        };
        let errs = run(vec![Event::WorkflowCall(e)]);
        assert!(errs.is_empty(), "unexpected: {errs:?}");
    }

    #[test]
    fn workflow_call_invalid_boolean_default() {
        let e = WorkflowCallEvent {
            inputs: vec![call_input(
                "flag",
                WorkflowCallEventInputType::Boolean,
                Some("yes"),
            )],
            ..Default::default()
        };
        let errs = run(vec![Event::WorkflowCall(e)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("typed as boolean"));
    }

    #[test]
    fn workflow_call_expression_default_skipped() {
        let e = WorkflowCallEvent {
            inputs: vec![call_input(
                "num",
                WorkflowCallEventInputType::Number,
                Some("${{ github.sha }}"),
            )],
            ..Default::default()
        };
        let errs = run(vec![Event::WorkflowCall(e)]);
        assert!(errs.is_empty(), "unexpected: {errs:?}");
    }

    #[test]
    fn workflow_call_required_with_default() {
        let mut i = call_input("num", WorkflowCallEventInputType::String, Some("x"));
        i.required = Some(Bool {
            value: true,
            expression: None,
            pos: Pos::new(4, 7),
        });
        let e = WorkflowCallEvent {
            inputs: vec![i],
            ..Default::default()
        };
        let errs = run(vec![Event::WorkflowCall(e)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("it is also required"));
    }

    fn dispatch_input(
        name: &str,
        input_type: WorkflowDispatchEventInputType,
        default: Option<&str>,
        options: Vec<&str>,
    ) -> DispatchInput {
        DispatchInput {
            name: at(name, Pos::new(3, 5)),
            description: None,
            required: None,
            default: default.map(|d| at(d, Pos::new(5, 7))),
            input_type,
            options: options.iter().map(|o| at(o, Pos::new(6, 9))).collect(),
        }
    }

    fn dispatch<I, K>(inputs: I) -> Event
    where
        I: IntoIterator<Item = (K, DispatchInput)>,
        K: Into<std::string::String>,
    {
        Event::WorkflowDispatch(WorkflowDispatchEvent {
            inputs: inputs.into_iter().map(|(k, v)| (k.into(), v)).collect(),
            pos: Pos::new(2, 3),
        })
    }

    #[test]
    fn choice_without_options() {
        let e = dispatch(vec![(
            "env",
            dispatch_input("env", WorkflowDispatchEventInputType::Choice, None, vec![]),
        )]);
        let errs = run(vec![e]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("\"options\" is not set"));
    }

    #[test]
    fn choice_duplicated_options() {
        let e = dispatch(vec![(
            "env",
            dispatch_input(
                "env",
                WorkflowDispatchEventInputType::Choice,
                None,
                vec!["a", "a", "b"],
            ),
        )]);
        let errs = run(vec![e]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("is duplicated in options"));
    }

    #[test]
    fn choice_default_not_in_options() {
        let e = dispatch(vec![(
            "env",
            dispatch_input(
                "env",
                WorkflowDispatchEventInputType::Choice,
                Some("z"),
                vec!["a", "b"],
            ),
        )]);
        let errs = run(vec![e]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("is not included in its options"));
    }

    #[test]
    fn choice_default_in_options_ok() {
        let e = dispatch(vec![(
            "env",
            dispatch_input(
                "env",
                WorkflowDispatchEventInputType::Choice,
                Some("a"),
                vec!["a", "b"],
            ),
        )]);
        let errs = run(vec![e]);
        assert!(errs.is_empty(), "unexpected: {errs:?}");
    }

    #[test]
    fn non_choice_with_options() {
        let e = dispatch(vec![(
            "num",
            dispatch_input(
                "num",
                WorkflowDispatchEventInputType::String,
                None,
                vec!["a", "b"],
            ),
        )]);
        let errs = run(vec![e]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("its input type is not \"choice\""));
    }

    #[test]
    fn dispatch_invalid_number_default() {
        let e = dispatch(vec![(
            "num",
            dispatch_input(
                "num",
                WorkflowDispatchEventInputType::Number,
                Some("abc"),
                vec![],
            ),
        )]);
        let errs = run(vec![e]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(
            errs[0]
                .message
                .contains("cannot be parsed as a float number")
        );
    }

    #[test]
    fn dispatch_invalid_boolean_default() {
        let e = dispatch(vec![(
            "flag",
            dispatch_input(
                "flag",
                WorkflowDispatchEventInputType::Boolean,
                Some("nope"),
                vec![],
            ),
        )]);
        let errs = run(vec![e]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("must be \"true\" or \"false\""));
    }

    #[test]
    fn dispatch_too_many_inputs() {
        let inputs = (0..26).map(|i| {
            let name = format!("in{i}");
            let input = dispatch_input(&name, WorkflowDispatchEventInputType::String, None, vec![]);
            (name, input)
        });
        let errs = run(vec![dispatch(inputs)]);
        assert_eq!(errs.len(), 1, "unexpected: {errs:?}");
        assert!(errs[0].message.contains("maximum number of inputs"));
        assert!(errs[0].message.contains("26 inputs are provided"));
    }

    #[test]
    fn dispatch_exactly_25_inputs_ok() {
        let inputs = (0..25).map(|i| {
            let name = format!("in{i}");
            let input = dispatch_input(&name, WorkflowDispatchEventInputType::String, None, vec![]);
            (name, input)
        });
        let errs = run(vec![dispatch(inputs)]);
        assert!(errs.is_empty(), "unexpected: {errs:?}");
    }
}
