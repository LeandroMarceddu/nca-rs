//! Implemented preflight rules.
//!
//! Each rule lives in its own module and exposes `rules()`. The orchestrator's
//! registry aggregates these modules; [`all`] is the stable vertical-slice
//! aggregate used by tests and direct callers.

pub mod clearance;
mod extents;
pub mod feed_missing;
pub mod geometry;
pub mod machine_envelope;
pub mod modal;
pub mod parse;
pub mod process;
pub mod rapid_at_depth;
pub mod travel;

use crate::rule::Rule;

/// Every implemented rule, in stable evaluation order.
///
/// The order follows the catalog: modal, travel, then clearance. Report
/// collections are sorted after evaluation, so this order never changes the
/// serialized report bytes.
#[must_use]
pub fn all() -> Vec<Box<dyn Rule>> {
    let mut rules: Vec<Box<dyn Rule>> = Vec::new();
    rules.extend(modal::rules());
    rules.extend(parse::rules());
    rules.extend(feed_missing::rules());
    rules.extend(machine_envelope::rules());
    rules.extend(travel::rules());
    rules.extend(rapid_at_depth::rules());
    rules.extend(clearance::rules());
    rules.extend(geometry::rules());
    rules.extend(process::rules());
    rules
}
