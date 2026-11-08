//! Root of the `actionlint` library.
//!
//! `actionlint` is a static checker for GitHub Actions workflow files. This crate is a Rust
//! reimplementation of the original Go implementation hosted at
//! <https://github.com/rhysd/actionlint>.
//!
//! The central entry point is [`linter::Linter`], which manages the entire linting lifecycle.
//! Workflow files are parsed into an [`ast::Workflow`] tree by [`parse`], and a set of
//! [`rule::Rule`] implementations traverse the tree through [`pass::Visitor`] to collect
//! [`error::Error`] values.

pub mod action_metadata;
pub mod all_webhooks;
pub mod ast;
pub mod availability;
pub mod command;
pub mod config;
pub mod error;
pub mod expr;
pub mod expr_ast;
pub mod expr_insecure;
pub mod expr_lexer;
pub mod expr_parser;
pub mod expr_sema;
pub mod expr_type;
pub mod glob;
pub mod linter;
pub mod parse;
pub mod pass;
pub mod popular_actions;
pub mod process;
pub mod project;
pub mod reusable_workflow;
pub mod rule;
pub mod rule_action;
pub mod rule_credentials;
pub mod rule_deprecated_commands;
pub mod rule_env_var;
pub mod rule_events;
pub mod rule_expression;
pub mod rule_glob;
pub mod rule_id;
pub mod rule_if_cond;
pub mod rule_job_needs;
pub mod rule_matrix;
pub mod rule_permissions;
pub mod rule_pyflakes;
pub mod rule_runner_label;
pub mod rule_shell_name;
pub mod rule_shellcheck;
pub mod rule_workflow_call;
