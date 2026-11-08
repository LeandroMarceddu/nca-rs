//! Axis positions in millimetres.

use std::fmt;
use std::ops::{Add, Index, IndexMut, Sub};

use serde::{Deserialize, Serialize};

use crate::axis::{AXIS_COUNT, Axes, Axis};
use crate::units::Units;

/// A position over a subset of the nine modelled axes.
///
/// Every stored coordinate is in millimetres. An inch-mode controller reports
/// inch values on the wire; convert with [`Position::from_units`] at the
/// protocol edge and with [`Position::to_units`] when displaying.
///
/// The default position is the machine origin on X, Y and Z; it is not an
/// empty position.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    coords: [f64; AXIS_COUNT],
    axes: Axes,
}

impl Default for Position {
    fn default() -> Self {
        Position::new(Axes::xyz())
    }
}

impl Position {
    /// A zero position with the given axes present.
    #[must_use]
    pub fn new(axes: Axes) -> Self {
        Position {
            coords: [0.0; AXIS_COUNT],
            axes,
        }
    }

    /// The machine origin, expressed as X0 Y0 Z0.
    #[must_use]
    pub fn zero() -> Self {
        Position::default()
    }

    /// A position with explicit X, Y and Z values.
    #[must_use]
    pub fn xyz(x: f64, y: f64, z: f64) -> Self {
        let mut position = Position::default();
        position.set(Axis::X, x);
        position.set(Axis::Y, y);
        position.set(Axis::Z, z);
        position
    }

    /// Build a position from one value per axis in `axes`, in wire order.
    ///
    /// Returns `None` when the number of values does not match the mask.
    #[must_use]
    pub fn from_values(axes: Axes, values: &[f64]) -> Option<Self> {
        if values.len() != axes.len() {
            return None;
        }
        let mut position = Position::new(axes);
        for (axis, value) in axes.iter().zip(values.iter().copied()) {
            position.set(axis, value);
        }
        Some(position)
    }

    /// Axes present in this position.
    #[must_use]
    pub const fn axes(self) -> Axes {
        self.axes
    }

    /// Replace the axis mask. Values for axes that were not present read as
    /// zero when the axis is added back.
    pub fn set_axes(&mut self, axes: Axes) {
        self.axes = axes;
    }

    /// Coordinate for one axis; zero when the axis is not present.
    #[must_use]
    pub fn get(self, axis: Axis) -> f64 {
        self.coords[axis.index()]
    }

    /// Mutable coordinate for one axis.
    pub fn get_mut(&mut self, axis: Axis) -> &mut f64 {
        &mut self.coords[axis.index()]
    }

    /// Set one coordinate without changing the axis mask.
    pub fn set(&mut self, axis: Axis, value: f64) {
        self.coords[axis.index()] = value;
    }

    /// Coordinates for every axis, including axes not present in the mask.
    #[must_use]
    pub const fn as_array(&self) -> &[f64; AXIS_COUNT] {
        &self.coords
    }

    /// Iterate `(axis, value)` for the axes present in the mask, in wire order.
    pub fn iter(self) -> impl Iterator<Item = (Axis, f64)> {
        self.axes.iter().map(move |axis| (axis, self.get(axis)))
    }

    /// Returns `true` when every present coordinate is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.iter().all(|(_, value)| value.is_finite())
    }

    /// Largest absolute coordinate, or `0.0` for an empty position.
    #[must_use]
    pub fn abs_max(self) -> f64 {
        self.iter()
            .map(|(_, value)| value.abs())
            .fold(0.0_f64, f64::max)
    }

    /// Euclidean distance to `other` over the union of both axis masks.
    #[must_use]
    pub fn distance_to(self, other: Position) -> f64 {
        let mut sum = 0.0;
        for axis in (self.axes | other.axes).iter() {
            let delta = self.get(axis) - other.get(axis);
            sum += delta * delta;
        }
        sum.sqrt()
    }

    /// Convert this position into `units` for display.
    #[must_use]
    pub fn to_units(self, units: Units) -> Self {
        let mut converted = self;
        for axis in self.axes.iter() {
            converted.set(axis, units.from_mm(self.get(axis)));
        }
        converted
    }

    /// Interpret this position as being expressed in `units` and convert it to
    /// millimetres.
    #[must_use]
    pub fn from_units(self, units: Units) -> Self {
        let mut converted = self;
        for axis in self.axes.iter() {
            converted.set(axis, units.to_mm(self.get(axis)));
        }
        converted
    }
}

impl Index<Axis> for Position {
    type Output = f64;

    fn index(&self, axis: Axis) -> &Self::Output {
        &self.coords[axis.index()]
    }
}

impl IndexMut<Axis> for Position {
    fn index_mut(&mut self, axis: Axis) -> &mut Self::Output {
        &mut self.coords[axis.index()]
    }
}

impl Add for Position {
    type Output = Position;

    fn add(mut self, rhs: Position) -> Position {
        let axes = self.axes | rhs.axes;
        for axis in axes.iter() {
            self.set(axis, self.get(axis) + rhs.get(axis));
        }
        self.axes = axes;
        self
    }
}

impl Sub for Position {
    type Output = Position;

    fn sub(mut self, rhs: Position) -> Position {
        let axes = self.axes | rhs.axes;
        for axis in axes.iter() {
            self.set(axis, self.get(axis) - rhs.get(axis));
        }
        self.axes = axes;
        self
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (axis, value) in self.iter() {
            if !first {
                f.write_str(" ")?;
            }
            first = false;
            write!(f, "{axis}{value:.3}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_position_is_xyz_origin() {
        let position = Position::default();
        assert_eq!(position.axes(), Axes::xyz());
        assert_eq!(position.get(Axis::X), 0.0);
        assert_eq!(position.get(Axis::W), 0.0);
        assert_eq!(position.to_string(), "X0.000 Y0.000 Z0.000");
    }

    #[test]
    fn get_set_index_and_iteration() {
        let mut position = Position::xyz(1.0, -2.0, 3.5);
        assert_eq!(position[Axis::Y], -2.0);
        position[Axis::Z] = 4.25;
        position.set(Axis::A, 12.0);
        position.set_axes(Axes::parse("XYZA").expect("valid"));

        assert_eq!(position.get(Axis::Z), 4.25);
        assert_eq!(position.get(Axis::A), 12.0);
        let collected: Vec<(Axis, f64)> = position.iter().collect();
        assert_eq!(
            collected,
            vec![
                (Axis::X, 1.0),
                (Axis::Y, -2.0),
                (Axis::Z, 4.25),
                (Axis::A, 12.0)
            ]
        );
        assert!(!position.get(Axis::B).is_nan());
        assert_eq!(position.to_string(), "X1.000 Y-2.000 Z4.250 A12.000");
    }

    #[test]
    fn empty_position_displays_and_iterates_nothing() {
        let position = Position::new(Axes::empty());
        assert_eq!(position.to_string(), "");
        assert_eq!(position.iter().count(), 0);
        assert!(position.is_finite());
        assert_eq!(position.abs_max(), 0.0);
    }

    #[test]
    fn arithmetic_uses_the_union_of_axes() {
        let xyz = Position::xyz(1.0, 2.0, 3.0);
        let mut a = Position::new(Axes::from(Axis::A));
        a.set(Axis::A, 4.0);
        let sum = xyz + a;
        assert_eq!(sum.axes().to_string(), "XYZA");
        assert_eq!(sum.get(Axis::A), 4.0);
        assert_eq!(sum.get(Axis::Z), 3.0);

        let difference = sum - xyz;
        assert_eq!(difference.get(Axis::A), 4.0);
        assert_eq!(difference.get(Axis::X), 0.0);
    }

    #[test]
    fn unit_conversion_round_trips() {
        let metric = Position::xyz(25.4, -50.8, 0.0);
        let imperial = metric.to_units(Units::Inch);
        assert!((imperial.get(Axis::X) - 1.0).abs() < 1e-12);
        assert!((imperial.get(Axis::Y) + 2.0).abs() < 1e-12);
        let round_trip = imperial.from_units(Units::Inch);
        assert!((round_trip.get(Axis::X) - metric.get(Axis::X)).abs() < 1e-12);
        assert_eq!(metric.to_units(Units::Mm), metric);
    }

    #[test]
    fn from_values_validates_length() {
        let axes = Axes::parse("XYZ").expect("valid");
        assert_eq!(
            Position::from_values(axes, &[1.0, 2.0, 3.0]),
            Some(Position::xyz(1.0, 2.0, 3.0))
        );
        assert_eq!(Position::from_values(axes, &[1.0, 2.0]), None);
    }

    #[test]
    fn distance_uses_common_axes() {
        let a = Position::xyz(0.0, 0.0, 0.0);
        let b = Position::xyz(3.0, 4.0, 0.0);
        assert!((a.distance_to(b) - 5.0).abs() < 1e-12);
        assert_eq!(a.distance_to(a), 0.0);
    }

    #[test]
    fn abs_max_and_finiteness() {
        let position = Position::xyz(-10.0, 2.0, 3.0);
        assert_eq!(position.abs_max(), 10.0);
        assert!(position.is_finite());
        let bad = Position::xyz(f64::NAN, 0.0, 0.0);
        assert!(!bad.is_finite());
    }
}
