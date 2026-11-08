//! Axis identifiers and axis bitmasks.

use std::fmt;
use std::ops::{
    BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not, Sub, SubAssign,
};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Number of axes modelled by the domain.
///
/// grblHAL supports up to nine axes; the reference FlexiHAL profile reports
/// three (X, Y, Z) but the domain stays nine-axis capable.
pub const AXIS_COUNT: usize = 9;

/// A machine axis.
///
/// The declaration order is the wire order used by grblHAL for coordinate
/// arrays, `Axes` bitmasks and the `$I+` `[AXS:...]` report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Axis {
    /// X axis.
    X = 0,
    /// Y axis.
    Y = 1,
    /// Z axis.
    Z = 2,
    /// A axis (rotary).
    A = 3,
    /// B axis (rotary).
    B = 4,
    /// C axis (rotary).
    C = 5,
    /// U axis (linear, usually a duplicate of X).
    U = 6,
    /// V axis (linear, usually a duplicate of Y).
    V = 7,
    /// W axis (linear, usually a duplicate of Z).
    W = 8,
}

impl Axis {
    /// All axes in wire order.
    pub const ALL: [Axis; AXIS_COUNT] = [
        Axis::X,
        Axis::Y,
        Axis::Z,
        Axis::A,
        Axis::B,
        Axis::C,
        Axis::U,
        Axis::V,
        Axis::W,
    ];

    /// Zero-based index in the wire order, always in `0..=8`.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Upper-case axis letter used by G-code and grblHAL reports.
    #[must_use]
    pub const fn letter(self) -> char {
        match self {
            Axis::X => 'X',
            Axis::Y => 'Y',
            Axis::Z => 'Z',
            Axis::A => 'A',
            Axis::B => 'B',
            Axis::C => 'C',
            Axis::U => 'U',
            Axis::V => 'V',
            Axis::W => 'W',
        }
    }

    /// Resolve an axis from a letter, accepting either case.
    #[must_use]
    pub fn from_letter(letter: char) -> Option<Self> {
        match letter.to_ascii_uppercase() {
            'X' => Some(Axis::X),
            'Y' => Some(Axis::Y),
            'Z' => Some(Axis::Z),
            'A' => Some(Axis::A),
            'B' => Some(Axis::B),
            'C' => Some(Axis::C),
            'U' => Some(Axis::U),
            'V' => Some(Axis::V),
            'W' => Some(Axis::W),
            _ => None,
        }
    }

    /// Resolve an axis from its wire-order index.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Axis::ALL.get(index).copied()
    }

    /// Human-readable name, for example `"X"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
            Axis::A => "A",
            Axis::B => "B",
            Axis::C => "C",
            Axis::U => "U",
            Axis::V => "V",
            Axis::W => "W",
        }
    }
}

impl fmt::Display for Axis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when an axis letter is not one of `X Y Z A B C U V W`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid axis letter '{0}'")]
pub struct InvalidAxisLetter(pub char);

/// A bitmask over [`Axis`] values.
///
/// Bits 0..=8 correspond to [`Axis::ALL`]; higher bits are preserved but do not
/// map to an axis. The type is a plain value type so it can be stored in
/// snapshots and persisted profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Axes(u16);

/// Mask of all nine modelled axes.
const ALL_AXES_BITS: u16 = (1 << AXIS_COUNT) - 1;

impl Axes {
    /// No axes.
    #[must_use]
    pub const fn empty() -> Self {
        Axes(0)
    }

    /// All nine modelled axes.
    #[must_use]
    pub const fn all() -> Self {
        Axes(ALL_AXES_BITS)
    }

    /// The X, Y and Z axes.
    #[must_use]
    pub const fn xyz() -> Self {
        Axes(0b0000_0111)
    }

    /// Build a mask from raw bits, dropping bits that are not modelled axes.
    #[must_use]
    pub const fn from_bits(bits: u16) -> Self {
        Axes(bits & ALL_AXES_BITS)
    }

    /// Build a mask from raw bits, preserving bits above the modelled axes.
    ///
    /// Used for controller bitmasks (for example the `H:` homed-axis mask)
    /// where unknown bits must round-trip even though they do not map to an
    /// [`Axis`].
    #[must_use]
    pub const fn from_bits_retain(bits: u16) -> Self {
        Axes(bits)
    }

    /// Raw bits, including any preserved bits above the modelled axes.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Number of modelled axes in the mask.
    #[must_use]
    pub const fn len(self) -> usize {
        (self.0 & ALL_AXES_BITS).count_ones() as usize
    }

    /// Returns `true` when no modelled axis is present.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 & ALL_AXES_BITS == 0
    }

    /// Returns `true` when `axis` is present.
    #[must_use]
    pub const fn contains(self, axis: Axis) -> bool {
        self.0 & (1 << (axis as u16)) != 0
    }

    /// Add an axis.
    pub fn insert(&mut self, axis: Axis) {
        self.0 |= 1 << (axis as u16);
    }

    /// Remove an axis.
    pub fn remove(&mut self, axis: Axis) {
        self.0 &= !(1 << (axis as u16));
    }

    /// Toggle an axis.
    pub fn toggle(&mut self, axis: Axis) {
        self.0 ^= 1 << (axis as u16);
    }

    /// Iterate the modelled axes present in the mask, in wire order.
    pub fn iter(self) -> impl Iterator<Item = Axis> {
        Axis::ALL
            .into_iter()
            .filter(move |axis| self.contains(*axis))
    }

    /// Union of two masks.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Axes(self.0 | other.0)
    }

    /// Intersection of two masks.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Axes(self.0 & other.0)
    }

    /// Difference of two masks.
    #[must_use]
    pub const fn difference(self, other: Self) -> Self {
        Axes(self.0 & !other.0)
    }

    /// Parse an axis mask such as `"XYZ"`, `"X Y Z"` or `"x,y,z"`.
    ///
    /// Whitespace, commas and `|` are accepted as separators. An empty string
    /// yields an empty mask. Any other character is rejected.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidAxisLetter`] for the first character that is neither an
    /// axis letter nor a separator.
    pub fn parse(text: &str) -> Result<Self, InvalidAxisLetter> {
        let mut axes = Axes::empty();
        for ch in text.chars() {
            if ch.is_whitespace() || ch == ',' || ch == '|' {
                continue;
            }
            match Axis::from_letter(ch) {
                Some(axis) => axes.insert(axis),
                None => return Err(InvalidAxisLetter(ch)),
            }
        }
        Ok(axes)
    }
}

impl From<Axis> for Axes {
    fn from(axis: Axis) -> Self {
        let mut axes = Axes::empty();
        axes.insert(axis);
        axes
    }
}

impl From<Axes> for u16 {
    fn from(axes: Axes) -> Self {
        axes.bits()
    }
}

impl From<u16> for Axes {
    fn from(bits: u16) -> Self {
        Axes::from_bits(bits)
    }
}

impl FromIterator<Axis> for Axes {
    fn from_iter<T: IntoIterator<Item = Axis>>(iter: T) -> Self {
        let mut axes = Axes::empty();
        for axis in iter {
            axes.insert(axis);
        }
        axes
    }
}

impl FromStr for Axes {
    type Err = InvalidAxisLetter;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Axes::parse(text)
    }
}

impl fmt::Display for Axes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for axis in self.iter() {
            f.write_str(axis.as_str())?;
        }
        Ok(())
    }
}

impl BitOr for Axes {
    type Output = Axes;

    fn bitor(self, rhs: Axes) -> Axes {
        Axes(self.0 | rhs.0)
    }
}

impl BitOrAssign for Axes {
    fn bitor_assign(&mut self, rhs: Axes) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for Axes {
    type Output = Axes;

    fn bitand(self, rhs: Axes) -> Axes {
        Axes(self.0 & rhs.0)
    }
}

impl BitAndAssign for Axes {
    fn bitand_assign(&mut self, rhs: Axes) {
        self.0 &= rhs.0;
    }
}

impl BitXor for Axes {
    type Output = Axes;

    fn bitxor(self, rhs: Axes) -> Axes {
        Axes(self.0 ^ rhs.0)
    }
}

impl BitXorAssign for Axes {
    fn bitxor_assign(&mut self, rhs: Axes) {
        self.0 ^= rhs.0;
    }
}

impl Sub for Axes {
    type Output = Axes;

    fn sub(self, rhs: Axes) -> Axes {
        self.difference(rhs)
    }
}

impl SubAssign for Axes {
    fn sub_assign(&mut self, rhs: Axes) {
        *self = self.difference(rhs);
    }
}

impl Not for Axes {
    type Output = Axes;

    fn not(self) -> Axes {
        Axes(!self.0 & ALL_AXES_BITS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_indices_and_letters_round_trip() {
        for (index, axis) in Axis::ALL.into_iter().enumerate() {
            assert_eq!(axis.index(), index);
            assert_eq!(Axis::from_index(index), Some(axis));
            assert_eq!(Axis::from_letter(axis.letter()), Some(axis));
            assert_eq!(
                Axis::from_letter(axis.letter().to_ascii_lowercase()),
                Some(axis)
            );
            assert_eq!(axis.as_str(), axis.letter().to_string());
        }
        assert_eq!(Axis::from_index(AXIS_COUNT), None);
        assert_eq!(Axis::from_letter('Q'), None);
    }

    #[test]
    fn axes_parse_and_display() {
        let axes = Axes::parse("XYZ").expect("valid");
        assert_eq!(axes, Axes::xyz());
        assert_eq!(axes.to_string(), "XYZ");
        assert_eq!(axes.len(), 3);
        assert!(!axes.is_empty());

        let spaced = Axes::parse("x y, z|A").expect("valid separators");
        assert_eq!(spaced.to_string(), "XYZA");

        assert_eq!(Axes::parse("").expect("empty"), Axes::empty());
        assert_eq!(Axes::parse("   ").expect("blank"), Axes::empty());
        assert_eq!(Axes::parse("XYQ"), Err(InvalidAxisLetter('Q')));
    }

    #[test]
    fn axes_bit_operations() {
        let xyz = Axes::xyz();
        let z = Axes::from(Axis::Z);
        assert!(xyz.contains(Axis::Z));
        assert!(!z.contains(Axis::X));
        assert_eq!(xyz.difference(z).to_string(), "XY");
        assert_eq!((xyz & z).to_string(), "Z");
        assert_eq!((Axes::empty() | z).to_string(), "Z");
        assert_eq!((!Axes::empty()).to_string(), "XYZABCUVW");
        assert_eq!(xyz.len(), 3);

        let mut axes = Axes::empty();
        axes.insert(Axis::X);
        axes.toggle(Axis::Y);
        axes.toggle(Axis::Y);
        axes.remove(Axis::X);
        assert!(axes.is_empty());

        let collected: Axes = Axis::ALL.into_iter().collect();
        assert_eq!(collected, Axes::all());
    }

    #[test]
    fn axes_preserve_unknown_bits_only_when_retaining() {
        let retained = Axes::from_bits_retain(0xFFFF);
        assert_eq!(retained.bits(), 0xFFFF);
        assert_eq!(retained.len(), AXIS_COUNT);
        assert_eq!(retained.to_string(), "XYZABCUVW");
        assert_eq!(Axes::from_bits(0xFFFF).bits(), 0x01FF);
    }
}
