//! Shared analysis vocabulary: ids, severities, evidence values.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Finding severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// A programming mistake that will likely damage the part or the machine.
    Error,
    /// A suspicious condition worth reviewing.
    Warning,
    /// Context with no implied defect.
    Info,
}

impl Severity {
    /// Sort rank used by the report: errors first.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Info => 2,
        }
    }

    /// Lowercase label used in JSON and filters.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Whether a finding follows directly from the program text or from a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Derived from program text or exact geometry.
    Exact,
    /// Derived from an estimated stock, engagement, or material model.
    Model,
}

impl Basis {
    /// Lowercase label used in JSON and UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Model => "model",
        }
    }
}

/// A stable rule identifier.
///
/// It serializes as a plain string and deserializes to an owned value, so a
/// report can be loaded without leaking a `'static` lifetime requirement.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleId(pub Cow<'static, str>);

impl RuleId {
    /// Creates a rule id from a static string.
    #[must_use]
    pub const fn new(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    /// Borrows the id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Creates an owned id.
    #[must_use]
    pub fn owned(id: impl Into<String>) -> Self {
        Self(Cow::Owned(id.into()))
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&'static str> for RuleId {
    fn from(value: &'static str) -> Self {
        Self::new(value)
    }
}

impl Serialize for RuleId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RuleId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Self(Cow::Owned(text)))
    }
}

/// One typed evidence value attached to a finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EvidenceValue {
    /// An integer count or index.
    Int(i64),
    /// A floating-point value; quantized when serialized in a report.
    Float(f64),
    /// Free text.
    Text(String),
    /// A boolean state.
    Bool(bool),
    /// A point in machine millimetres.
    Point3([f64; 3]),
}

impl EvidenceValue {
    /// Quantizes a float to `decimals` places; other variants are unchanged.
    ///
    /// Rounding and the decimal factor go through [`nc_core::math`], so the
    /// result does not depend on the platform libm. At most 15 decimals are
    /// used, the largest power of ten that is exactly representable in `f64`.
    #[must_use]
    pub fn quantized(self, decimals: u32) -> Self {
        match self {
            Self::Float(value) => {
                let factor = decimal_factor(decimals);
                if !factor.is_finite() || factor == 0.0 {
                    return Self::Float(value);
                }
                Self::Float(nc_core::math::round(value * factor) / factor)
            }
            other => other,
        }
    }

    /// Returns the text form used by markdown exports.
    #[must_use]
    pub fn display_text(&self) -> String {
        match self {
            Self::Int(value) => value.to_string(),
            Self::Float(value) => format!("{value:.3}"),
            Self::Text(value) => value.clone(),
            Self::Bool(value) => value.to_string(),
            Self::Point3(point) => {
                format!("[{:.3}, {:.3}, {:.3}]", point[0], point[1], point[2])
            }
        }
    }
}

/// Returns `10^decimals` as an exactly representable `f64` factor.
///
/// A small multiplication loop avoids `f64::powi`, and the cap keeps the factor
/// exact: `10^15` is below `2^53`, while larger powers lose integer precision.
fn decimal_factor(decimals: u32) -> f64 {
    let mut factor = 1.0;
    for _ in 0..decimals.min(15) {
        factor *= 10.0;
    }
    factor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_id_round_trips_through_json() {
        let id = RuleId::new("clearance.rapid_at_depth");
        let text = serde_json::to_string(&id).expect("serialize");
        assert_eq!(text, "\"clearance.rapid_at_depth\"");
        let read: RuleId = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(read, id);
    }

    #[test]
    fn severity_ranks_errors_first() {
        assert!(Severity::Error.rank() < Severity::Warning.rank());
        assert!(Severity::Warning.rank() < Severity::Info.rank());
    }

    #[test]
    fn evidence_quantizes_floats_only() {
        assert_eq!(
            EvidenceValue::Float(1.23456).quantized(3),
            EvidenceValue::Float(1.235)
        );
        assert_eq!(EvidenceValue::Int(3).quantized(3), EvidenceValue::Int(3));
    }

    #[test]
    fn evidence_serializes_untagged() {
        let value =
            serde_json::to_value(EvidenceValue::Point3([1.0, 2.0, 3.0])).expect("serialize");
        assert_eq!(value, serde_json::json!([1.0, 2.0, 3.0]));
    }
}
