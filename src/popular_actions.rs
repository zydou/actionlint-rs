//! Data set of well-known popular actions.
//!
//! Rust counterpart of the original generated `popular_actions.go`. It holds the metadata of
//! popular actions so the [`crate::rule_action`] rule can validate `uses:` specifications without
//! network access.
//!
//! This data is generated from
//! <https://github.com/actions/starter-workflows> and the list of popular actions on the GitHub
//! Marketplace.

use std::collections::HashMap;

use crate::action_metadata::{ActionMetadata, ActionMetadataInputs, ActionMetadataOutputs};

/// Returns the data set of known popular actions. Keys are specs (`owner/repo@ref`) of actions and
/// values are their metadata.
pub fn popular_actions() -> &'static HashMap<&'static str, ActionMetadata> {
    static MAP: std::sync::OnceLock<HashMap<&'static str, ActionMetadata>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(build_popular_actions)
}

/// Returns the set of outdated popular action specs (e.g. `actions/checkout@v2`). Using these
/// actions is reported by the `action` rule.
pub fn outdated_popular_action_specs() -> &'static std::collections::HashSet<&'static str> {
    static SET: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
        std::sync::OnceLock::new();
    SET.get_or_init(std::collections::HashSet::new)
}

fn build_popular_actions() -> HashMap<&'static str, ActionMetadata> {
    // Populated by the port of the generated popular-actions data.
    HashMap::new()
}

// Keep the type aliases referenced so they are not flagged as unused when the data is filled in.
#[allow(dead_code)]
fn _type_assertions(_i: &ActionMetadataInputs, _o: &ActionMetadataOutputs) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popular_actions_load() {
        // The table is ported separately; this only ensures it initializes without panicking.
        let _ = popular_actions();
        let _ = outdated_popular_action_specs();
    }
}
