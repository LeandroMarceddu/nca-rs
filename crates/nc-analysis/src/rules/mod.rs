//! Implemented preflight rules.
//!
//! Each rule lives in its own module and exposes `rules()`. The orchestrator's
//! registry aggregates these modules; [`all`] is the stable vertical-slice
//! aggregate used by tests and direct callers.

pub mod feed_missing;
pub mod machine_envelope;
pub mod rapid_at_depth;

use crate::rule::Rule;

/// Every implemented rule, in stable evaluation order.
///
/// The order follows the catalog: modal, travel, then clearance. Report
/// collections are sorted after evaluation, so this order never changes the
/// serialized report bytes.
#[must_use]
pub fn all() -> Vec<Box<dyn Rule>> {
    let mut rules: Vec<Box<dyn Rule>> = Vec::new();
    rules.extend(feed_missing::rules());
    rules.extend(machine_envelope::rules());
    rules.extend(rapid_at_depth::rules());
    rules
}
