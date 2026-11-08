//! `parse.*` rules: unsupported blocks, assumed dialect, ambiguous tool hints.
//!
//! These rules report what the parser had to assume or skip. They never change
//! geometry and never fail the run: every one is `Exact` and needs no
//! configuration.

pub mod dialect;
pub mod tool_hint;
pub mod unsupported;

use crate::rule::Rule;

/// Every rule in this module, in catalog order.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    let mut rules: Vec<Box<dyn Rule>> = Vec::new();
    rules.extend(unsupported::rules());
    rules.extend(dialect::rules());
    rules.extend(tool_hint::rules());
    rules
}
