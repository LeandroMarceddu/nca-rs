//! Platform-independent math used by every geometric computation.
//!
//! All trigonometric work in `nca-rs` goes through this module. The wrappers
//! call the `libm` crate instead of the platform's libm, so a report or a
//! heightmap hash does not change between operating systems.
//!
//! Only `f64` is exposed: library code does not use `f32` arithmetic. Stored
//! heightmap cells are rounded to `f32` at the storage boundary, which is a
//! deterministic conversion.

/// Archimedes' constant as an `f64`.
pub const PI: f64 = core::f64::consts::PI;

/// Half of [`PI`].
pub const FRAC_PI_2: f64 = core::f64::consts::FRAC_PI_2;

/// Two times [`PI`].
pub const TAU: f64 = core::f64::consts::TAU;

/// Converts degrees to radians.
#[must_use]
pub fn to_radians(degrees: f64) -> f64 {
    degrees * (PI / 180.0)
}

/// Converts radians to degrees.
#[must_use]
pub fn to_degrees(radians: f64) -> f64 {
    radians * (180.0 / PI)
}

/// Platform-independent sine.
#[must_use]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

/// Platform-independent cosine.
#[must_use]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

/// Platform-independent tangent.
#[must_use]
pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}

/// Platform-independent arc sine.
#[must_use]
pub fn asin(x: f64) -> f64 {
    libm::asin(x)
}

/// Platform-independent arc cosine.
#[must_use]
pub fn acos(x: f64) -> f64 {
    libm::acos(x)
}

/// Platform-independent arc tangent.
#[must_use]
pub fn atan(x: f64) -> f64 {
    libm::atan(x)
}

/// Platform-independent two-argument arc tangent.
#[must_use]
pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

/// Platform-independent square root.
#[must_use]
pub fn sqrt(x: f64) -> f64 {
    libm::sqrt(x)
}

/// Platform-independent hypotenuse.
#[must_use]
pub fn hypot(x: f64, y: f64) -> f64 {
    libm::hypot(x, y)
}

/// Platform-independent power.
#[must_use]
pub fn pow(base: f64, exponent: f64) -> f64 {
    libm::pow(base, exponent)
}

/// Platform-independent floor.
#[must_use]
pub fn floor(x: f64) -> f64 {
    libm::floor(x)
}

/// Platform-independent ceiling.
#[must_use]
pub fn ceil(x: f64) -> f64 {
    libm::ceil(x)
}

/// Platform-independent rounding to the nearest integer, ties away from zero.
#[must_use]
pub fn round(x: f64) -> f64 {
    libm::round(x)
}

/// Platform-independent truncation toward zero.
#[must_use]
pub fn trunc(x: f64) -> f64 {
    libm::trunc(x)
}

/// Platform-independent absolute value.
#[must_use]
pub fn abs(x: f64) -> f64 {
    libm::fabs(x)
}

/// Returns the value clamped into `[min, max]`, propagating NaN as `min`.
#[must_use]
pub fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value.is_nan() {
        return min;
    }
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degree_radian_round_trip() {
        assert!((to_radians(180.0) - PI).abs() < 1e-15);
        assert!((to_degrees(PI) - 180.0).abs() < 1e-12);
    }

    #[test]
    fn trig_identities() {
        assert!((sin(PI / 2.0) - 1.0).abs() < 1e-15);
        assert!((cos(0.0) - 1.0).abs() < 1e-15);
        assert!((atan2(1.0, 1.0) - PI / 4.0).abs() < 1e-15);
        assert!((hypot(3.0, 4.0) - 5.0).abs() < 1e-15);
    }

    #[test]
    fn clamp_is_total() {
        assert_eq!(clamp(-1.0, 0.0, 2.0), 0.0);
        assert_eq!(clamp(3.0, 0.0, 2.0), 2.0);
        assert_eq!(clamp(f64::NAN, 1.0, 2.0), 1.0);
    }
}
