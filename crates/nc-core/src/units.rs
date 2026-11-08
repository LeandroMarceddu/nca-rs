//! Length units and conversion helpers.
//!
//! Millimetres are canonical in the domain. Inch mode is a protocol and
//! display concern: convert at the edges with [`Units::to_mm`] and
//! [`Units::from_mm`].

use std::fmt;

use serde::{Deserialize, Serialize};

/// Length unit used for display and protocol values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Units {
    /// Millimetres (`G21`).
    #[default]
    Mm,
    /// Inches (`G20`).
    Inch,
}

impl Units {
    /// Exact number of millimetres in one inch.
    pub const MM_PER_INCH: f64 = 25.4;

    /// Convert a value expressed in this unit into millimetres.
    #[must_use]
    pub fn to_mm(self, value: f64) -> f64 {
        match self {
            Units::Mm => value,
            Units::Inch => value * Self::MM_PER_INCH,
        }
    }

    /// Convert a millimetre value into this unit.
    #[must_use]
    pub fn from_mm(self, value_mm: f64) -> f64 {
        match self {
            Units::Mm => value_mm,
            Units::Inch => value_mm / Self::MM_PER_INCH,
        }
    }

    /// Millimetres represented by one unit.
    #[must_use]
    pub const fn mm_per_unit(self) -> f64 {
        match self {
            Units::Mm => 1.0,
            Units::Inch => Self::MM_PER_INCH,
        }
    }

    /// `true` for metric units.
    #[must_use]
    pub const fn is_metric(self) -> bool {
        matches!(self, Units::Mm)
    }

    /// Short unit label: `"mm"` or `"in"`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Units::Mm => "mm",
            Units::Inch => "in",
        }
    }

    /// Parse a unit from common spellings, G-codes included.
    ///
    /// Accepts `mm`, `metric`, `G21`, `in`, `inch`, `imperial` and `G20`
    /// (case-insensitive).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "mm" | "metric" | "g21" => Some(Units::Mm),
            "in" | "inch" | "inches" | "imperial" | "g20" => Some(Units::Inch),
            _ => None,
        }
    }
}

impl fmt::Display for Units {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inch_conversion_is_exact_and_invertible() {
        assert_eq!(Units::Mm.to_mm(12.5), 12.5);
        assert_eq!(Units::Inch.to_mm(1.0), 25.4);
        assert_eq!(Units::Inch.from_mm(25.4), 1.0);
        assert!((Units::Inch.from_mm(Units::Inch.to_mm(3.75)) - 3.75).abs() < 1e-12);
        assert_eq!(Units::Mm.mm_per_unit(), 1.0);
        assert_eq!(Units::Inch.mm_per_unit(), 25.4);
    }

    #[test]
    fn units_parse_and_display() {
        assert_eq!(Units::parse("G21"), Some(Units::Mm));
        assert_eq!(Units::parse(" G20 "), Some(Units::Inch));
        assert_eq!(Units::parse("inch"), Some(Units::Inch));
        assert_eq!(Units::parse("furlong"), None);
        assert_eq!(Units::Mm.to_string(), "mm");
        assert_eq!(Units::Inch.to_string(), "in");
        assert!(Units::Mm.is_metric());
        assert!(!Units::Inch.is_metric());
    }
}
