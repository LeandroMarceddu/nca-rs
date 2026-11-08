//! `modal.units_missing` and `modal.distance_missing`.
//!
//! Both rules report the first motion that runs before the corresponding modal
//! group was ever programmed. Later motions are not reported separately: one
//! finding names the line a reader must fix.

use std::collections::BTreeMap;

use nc_gcode::MotionRecord;

use crate::config::RuleSettings;
use crate::rule::{AnalysisModel, Requirements, Rule, RuleOutcome};
use crate::types::{Basis, Severity};

use super::{diagnostic, insert_motion_evidence};

/// Stable id of the units rule.
const UNITS_ID: &str = "modal.units_missing";

/// Stable id of the distance rule.
const DISTANCE_ID: &str = "modal.distance_missing";

/// Returns the rules in this module.
#[must_use]
pub fn rules() -> Vec<Box<dyn Rule>> {
    vec![Box::new(UnitsMissing), Box::new(DistanceMissing)]
}

/// Which modal group a rule inspects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    /// `G20`/`G21` length units.
    Units,
    /// `G90`/`G91` distance mode.
    Distance,
}

/// Checks that the first motion has an explicit unit mode.
struct UnitsMissing;

impl Rule for UnitsMissing {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(UNITS_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        evaluate_group(model, Group::Units)
    }

    fn why_it_matters(&self) -> &'static str {
        "Without G20/G21 the controller keeps its previous length unit. An inch \
         program read as millimetres, or the reverse, cuts the part 25.4 times \
         too small or too large."
    }
}

/// Checks that the first motion has an explicit distance mode.
struct DistanceMissing;

impl Rule for DistanceMissing {
    fn id(&self) -> crate::types::RuleId {
        crate::types::RuleId::new(DISTANCE_ID)
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn basis(&self) -> Basis {
        Basis::Exact
    }

    fn requirements(&self) -> Requirements {
        Requirements::NONE
    }

    fn evaluate(&self, model: &AnalysisModel<'_>, _cfg: &RuleSettings) -> RuleOutcome {
        evaluate_group(model, Group::Distance)
    }

    fn why_it_matters(&self) -> &'static str {
        "Without G90/G91 the controller keeps its previous distance mode. An \
         incremental move read as absolute can drive the tool to an unexpected \
         position before the operator notices."
    }
}

/// Reports the first motion that lacks the requested modal group.
fn evaluate_group(model: &AnalysisModel<'_>, group: Group) -> RuleOutcome {
    let Some(motion) = first_motion_without(model, group) else {
        return RuleOutcome::Findings(Vec::new());
    };
    RuleOutcome::Findings(vec![finding(motion, group)])
}

/// Finds the first motion whose modal state lacks `group`.
fn first_motion_without<'a>(model: &AnalysisModel<'a>, group: Group) -> Option<&'a MotionRecord> {
    model.program.motions.iter().find(|motion| {
        let modal = model.program.modal(motion.modal);
        match group {
            Group::Units => modal.units.is_none(),
            Group::Distance => modal.distance.is_none(),
        }
    })
}

/// Builds the one finding for `motion`.
fn finding(motion: &MotionRecord, group: Group) -> crate::report::Diagnostic {
    let (rule, message) = match group {
        Group::Units => (
            UNITS_ID,
            "first motion runs before an explicit G20/G21; the length unit is undefined",
        ),
        Group::Distance => (
            DISTANCE_ID,
            "first motion runs before an explicit G90/G91; the distance mode is undefined",
        ),
    };
    let mut evidence = BTreeMap::new();
    insert_motion_evidence(&mut evidence, motion);
    diagnostic(
        rule,
        Severity::Warning,
        Basis::Exact,
        motion.line,
        message,
        evidence,
    )
}
