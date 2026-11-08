//! Work coordinate systems and the machine/work coordinate transform.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::axis::Axis;
use crate::position::Position;

/// A selectable work coordinate system, `G54` through `G59.3`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum Wcs {
    /// `G54`, the default work coordinate system.
    #[default]
    #[serde(rename = "G54")]
    G54,
    /// `G55`.
    #[serde(rename = "G55")]
    G55,
    /// `G56`.
    #[serde(rename = "G56")]
    G56,
    /// `G57`.
    #[serde(rename = "G57")]
    G57,
    /// `G58`.
    #[serde(rename = "G58")]
    G58,
    /// `G59`.
    #[serde(rename = "G59")]
    G59,
    /// `G59.1`.
    #[serde(rename = "G59.1")]
    G59_1,
    /// `G59.2`.
    #[serde(rename = "G59.2")]
    G59_2,
    /// `G59.3`.
    #[serde(rename = "G59.3")]
    G59_3,
}

impl Wcs {
    /// Every work coordinate system in report order.
    pub const ALL: [Wcs; 9] = [
        Wcs::G54,
        Wcs::G55,
        Wcs::G56,
        Wcs::G57,
        Wcs::G58,
        Wcs::G59,
        Wcs::G59_1,
        Wcs::G59_2,
        Wcs::G59_3,
    ];

    /// Zero-based index in report order.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Wcs::G54 => 0,
            Wcs::G55 => 1,
            Wcs::G56 => 2,
            Wcs::G57 => 3,
            Wcs::G58 => 4,
            Wcs::G59 => 5,
            Wcs::G59_1 => 6,
            Wcs::G59_2 => 7,
            Wcs::G59_3 => 8,
        }
    }

    /// Resolve a work coordinate system from its report index.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Wcs::ALL.get(index).copied()
    }

    /// Canonical G-code word, for example `"G59.3"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Wcs::G54 => "G54",
            Wcs::G55 => "G55",
            Wcs::G56 => "G56",
            Wcs::G57 => "G57",
            Wcs::G58 => "G58",
            Wcs::G59 => "G59",
            Wcs::G59_1 => "G59.1",
            Wcs::G59_2 => "G59.2",
            Wcs::G59_3 => "G59.3",
        }
    }

    /// Parse `"G54"` .. `"G59.3"`, with or without the `G` prefix and in any
    /// case.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut word = text.trim().to_ascii_uppercase();
        if let Some(stripped) = word.strip_prefix('G') {
            word = stripped.to_string();
        }
        match word.as_str() {
            "54" => Some(Wcs::G54),
            "55" => Some(Wcs::G55),
            "56" => Some(Wcs::G56),
            "57" => Some(Wcs::G57),
            "58" => Some(Wcs::G58),
            "59" => Some(Wcs::G59),
            "59.1" => Some(Wcs::G59_1),
            "59.2" => Some(Wcs::G59_2),
            "59.3" => Some(Wcs::G59_3),
            _ => None,
        }
    }
}

impl fmt::Display for Wcs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Offsets used to convert between machine and work coordinates.
///
/// The active work offset is `offsets[wcs] + g92`, matching grblHAL's `WCO`
/// report. Tool length offsets are stored separately because grbl applies them
/// through the G-code parser rather than through `WCO`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordinateSystem {
    /// Currently selected work coordinate system.
    pub wcs: Wcs,
    /// Stored offsets for `G54` .. `G59.3`, in [`Wcs::ALL`] order.
    pub offsets: [Position; 9],
    /// `G92` offset, applied in addition to the work offset.
    pub g92: Position,
    /// Active tool length offset; informational for the coordinate transform.
    pub tool_length_offset: Position,
}

impl Default for CoordinateSystem {
    fn default() -> Self {
        CoordinateSystem {
            wcs: Wcs::G54,
            offsets: [Position::zero(); 9],
            g92: Position::zero(),
            tool_length_offset: Position::zero(),
        }
    }
}

impl CoordinateSystem {
    /// Stored offset for one work coordinate system.
    #[must_use]
    pub fn offset(&self, wcs: Wcs) -> Position {
        self.offsets[wcs.index()]
    }

    /// Store an offset for one work coordinate system.
    pub fn set_offset(&mut self, wcs: Wcs, offset: Position) {
        self.offsets[wcs.index()] = offset;
    }

    /// Total work coordinate offset: the active work offset plus `G92`.
    #[must_use]
    pub fn work_offset(&self) -> Position {
        self.offset(self.wcs) + self.g92
    }

    /// Convert a machine position into the active work coordinate system.
    #[must_use]
    pub fn to_work(&self, machine_position: Position) -> Position {
        machine_position - self.work_offset()
    }

    /// Convert a work position into machine coordinates.
    #[must_use]
    pub fn to_machine(&self, work_position: Position) -> Position {
        work_position + self.work_offset()
    }

    /// Current X coordinate in machine space.
    #[must_use]
    pub fn machine_x(&self) -> f64 {
        self.offsets[self.wcs.index()].get(Axis::X) + self.g92.get(Axis::X)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wcs_round_trips() {
        for (index, wcs) in Wcs::ALL.into_iter().enumerate() {
            assert_eq!(wcs.index(), index);
            assert_eq!(Wcs::from_index(index), Some(wcs));
            assert_eq!(Wcs::parse(wcs.as_str()), Some(wcs));
            assert_eq!(Wcs::parse(&wcs.as_str().to_ascii_lowercase()), Some(wcs));
            assert_eq!(wcs.to_string(), wcs.as_str());
            let without_prefix = wcs.as_str().trim_start_matches('G');
            assert_eq!(Wcs::parse(without_prefix), Some(wcs));
        }
        assert_eq!(Wcs::from_index(9), None);
        assert_eq!(Wcs::parse("G60"), None);
        assert_eq!(Wcs::default(), Wcs::G54);
    }

    #[test]
    fn coordinate_transform_uses_wcs_and_g92() {
        let mut cs = CoordinateSystem::default();
        assert_eq!(cs.work_offset(), Position::zero());

        let offset = Position::xyz(10.0, 20.0, 30.0);
        cs.set_offset(Wcs::G55, offset);
        cs.wcs = Wcs::G55;
        cs.g92 = Position::xyz(1.0, 2.0, 3.0);
        assert_eq!(cs.work_offset(), Position::xyz(11.0, 22.0, 33.0));

        let machine = Position::xyz(100.0, 200.0, 300.0);
        let work = cs.to_work(machine);
        assert_eq!(work, Position::xyz(89.0, 178.0, 267.0));
        assert_eq!(cs.to_machine(work), machine);
        assert_eq!(cs.machine_x(), 11.0);
    }
}
