//! G-code dialect identifiers.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// A G-code dialect family.
///
/// The dialect selects a small set of semantics that differ between controls
/// (dwell units, canned-cycle retract behavior, accepted arc formats, and the
/// meaning of machine-coordinate moves). It is never used to guess tool data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dialect {
    /// Conservative least-common-denominator semantics. The default.
    #[default]
    Generic,
    /// Grbl and grblHAL.
    Grbl,
    /// LinuxCNC.
    LinuxCnc,
    /// Mach3/Mach4.
    Mach3,
    /// Fanuc-style controls.
    Fanuc,
    /// Makera Z1/Carvera, whose firmware is Smoothieware-derived.
    ///
    /// Adds the Makera auxiliary-output codes and treats `G28`/`G30` as a
    /// controller-defined clearance move rather than a stored home position.
    Makera,
}

impl Dialect {
    /// Every dialect in display order.
    pub const ALL: [Self; 6] = [
        Self::Generic,
        Self::Grbl,
        Self::LinuxCnc,
        Self::Mach3,
        Self::Fanuc,
        Self::Makera,
    ];

    /// Stable configuration name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Grbl => "grbl",
            Self::LinuxCnc => "linuxcnc",
            Self::Mach3 => "mach3",
            Self::Fanuc => "fanuc",
            Self::Makera => "makera",
        }
    }

    /// `true` when `G4 P` is expressed in milliseconds rather than seconds.
    #[must_use]
    pub const fn dwell_p_is_milliseconds(self) -> bool {
        matches!(self, Self::Fanuc)
    }
}

impl fmt::Display for Dialect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl FromStr for Dialect {
    type Err = CoreError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().to_ascii_lowercase().as_str() {
            "generic" | "" => Ok(Self::Generic),
            "grbl" | "grblhal" => Ok(Self::Grbl),
            "linuxcnc" | "linux-cnc" | "linux_cnc" => Ok(Self::LinuxCnc),
            "mach3" | "mach4" => Ok(Self::Mach3),
            "fanuc" => Ok(Self::Fanuc),
            "makera" | "makera-z1" | "makera_z1" => Ok(Self::Makera),
            other => Err(CoreError::UnknownDialect(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialect_names_round_trip() {
        for dialect in Dialect::ALL {
            assert_eq!(Dialect::from_str(dialect.label()), Ok(dialect));
            assert!(!dialect.label().is_empty());
        }
    }

    #[test]
    fn only_fanuc_dwell_is_milliseconds() {
        assert!(!Dialect::Generic.dwell_p_is_milliseconds());
        assert!(!Dialect::Grbl.dwell_p_is_milliseconds());
        assert!(!Dialect::Makera.dwell_p_is_milliseconds());
        assert!(Dialect::Fanuc.dwell_p_is_milliseconds());
    }

    #[test]
    fn makera_aliases_resolve() {
        assert_eq!(Dialect::from_str("makera"), Ok(Dialect::Makera));
        assert_eq!(Dialect::from_str("Makera-Z1"), Ok(Dialect::Makera));
        assert_eq!(Dialect::from_str("makera_z1"), Ok(Dialect::Makera));
    }

    #[test]
    fn unknown_dialect_is_an_error() {
        assert!(Dialect::from_str("siemens").is_err());
    }
}
