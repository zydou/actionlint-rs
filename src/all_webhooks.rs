//! Table of all webhooks with their activity types.
//!
//! Rust counterpart of the original generated `all_webhooks.go`, based on
//! <https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows>.
//!
//! A value of `None` means the activity types cannot be determined from the document (e.g.
//! `repository_dispatch` can contain arbitrary user-customized types).

use std::collections::HashMap;

/// Returns the table of all webhooks and their activity types.
///
/// `None` means the activity types are arbitrary and cannot be determined statically.
pub fn all_webhook_types() -> &'static HashMap<&'static str, Option<&'static [&'static str]>> {
    static MAP: std::sync::OnceLock<HashMap<&'static str, Option<&'static [&'static str]>>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(|| {
        HashMap::from([
            (
                "branch_protection_rule",
                Some(&["created", "edited", "deleted"][..]),
            ),
            (
                "check_run",
                Some(&["created", "rerequested", "completed", "requested_action"][..]),
            ),
            ("check_suite", Some(&["completed"][..])),
            ("create", Some(&[][..])),
            ("delete", Some(&[][..])),
            ("deployment", Some(&[][..])),
            ("deployment_status", Some(&[][..])),
            (
                "discussion",
                Some(
                    &[
                        "created",
                        "edited",
                        "deleted",
                        "transferred",
                        "pinned",
                        "unpinned",
                        "labeled",
                        "unlabeled",
                        "locked",
                        "unlocked",
                        "category_changed",
                        "answered",
                        "unanswered",
                    ][..],
                ),
            ),
            (
                "discussion_comment",
                Some(&["created", "edited", "deleted"][..]),
            ),
            ("fork", Some(&[][..])),
            ("gollum", Some(&[][..])),
            ("image_version", Some(&[][..])),
            ("issue_comment", Some(&["created", "edited", "deleted"][..])),
            (
                "issues",
                Some(
                    &[
                        "opened",
                        "edited",
                        "deleted",
                        "transferred",
                        "pinned",
                        "unpinned",
                        "closed",
                        "reopened",
                        "assigned",
                        "unassigned",
                        "labeled",
                        "unlabeled",
                        "locked",
                        "unlocked",
                        "milestoned",
                        "demilestoned",
                        "typed",
                        "untyped",
                    ][..],
                ),
            ),
            ("label", Some(&["created", "edited", "deleted"][..])),
            ("merge_group", Some(&["checks_requested"][..])),
            (
                "milestone",
                Some(&["created", "closed", "opened", "edited", "deleted"][..]),
            ),
            ("page_build", Some(&[][..])),
            ("public", Some(&[][..])),
            (
                "pull_request",
                Some(
                    &[
                        "assigned",
                        "unassigned",
                        "labeled",
                        "unlabeled",
                        "opened",
                        "edited",
                        "closed",
                        "reopened",
                        "synchronize",
                        "converted_to_draft",
                        "locked",
                        "unlocked",
                        "enqueued",
                        "dequeued",
                        "milestoned",
                        "demilestoned",
                        "ready_for_review",
                        "review_requested",
                        "review_request_removed",
                        "auto_merge_enabled",
                        "auto_merge_disabled",
                    ][..],
                ),
            ),
            (
                "pull_request_review",
                Some(&["submitted", "edited", "dismissed"][..]),
            ),
            (
                "pull_request_review_comment",
                Some(&["created", "edited", "deleted"][..]),
            ),
            (
                "pull_request_target",
                Some(
                    &[
                        "assigned",
                        "unassigned",
                        "labeled",
                        "unlabeled",
                        "opened",
                        "edited",
                        "closed",
                        "reopened",
                        "synchronize",
                        "converted_to_draft",
                        "locked",
                        "unlocked",
                        "enqueued",
                        "dequeued",
                        "milestoned",
                        "demilestoned",
                        "ready_for_review",
                        "review_requested",
                        "review_request_removed",
                        "auto_merge_enabled",
                        "auto_merge_disabled",
                    ][..],
                ),
            ),
            ("push", Some(&[][..])),
            ("registry_package", Some(&["published", "updated"][..])),
            (
                "release",
                Some(
                    &[
                        "published",
                        "unpublished",
                        "created",
                        "edited",
                        "deleted",
                        "prereleased",
                        "released",
                    ][..],
                ),
            ),
            // repository_dispatch can contain arbitrary user-customized types.
            ("repository_dispatch", None),
            ("schedule", Some(&[][..])),
            ("status", Some(&[][..])),
            ("watch", Some(&["started"][..])),
            ("workflow_call", Some(&[][..])),
            ("workflow_dispatch", Some(&[][..])),
            (
                "workflow_run",
                Some(&["completed", "requested", "in_progress"][..]),
            ),
        ])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_webhook_with_types() {
        let m = all_webhook_types();
        assert_eq!(
            m.get("check_run"),
            Some(&Some(
                &["created", "rerequested", "completed", "requested_action"][..]
            ))
        );
    }

    #[test]
    fn repository_dispatch_is_none() {
        let m = all_webhook_types();
        assert_eq!(m.get("repository_dispatch"), Some(&None));
    }

    #[test]
    fn push_has_no_types() {
        let m = all_webhook_types();
        assert_eq!(m.get("push"), Some(&Some(&[][..])));
    }

    #[test]
    fn unknown_webhook_absent() {
        let m = all_webhook_types();
        assert!(!m.contains_key("not_a_webhook"));
    }
}
