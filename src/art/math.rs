//! Fixed-point angles, trigonometry, and vector helpers.
//!
//! Purpose: supply integer `Angle` (binary radians, `65_536` per turn),
//! fixed-point `sin`/`cos` (`ONE = 4096`), `scale`, `ease_in_out`, and
//! `perpendicular_component` so rig/animation/surface stay bit-identical without floats.
//! Owns: `Angle` wrapping/distance, Bhaskara sine, scaled multiply, cubic ease,
//! and isqrt-derived perpendicular.
//! Reads: nothing.
//! Mutates: nothing (pure value math).
//! Does not own: canvas, palette, or skeleton data.
//! Relevant invariants: every wrap stays in `0..65_536`; `sin`/`cos` bounded by `±ONE`;
//! determinism integer-only; `ONE` is the fixed-point unit.
//! Canonical operations: `ease_in_out`, fixed-point math helpers.
//! Focused tests: `src/art/math.rs::tests` sine, wrapping, monotonic ease.

use serde::{Deserialize, Serialize};

/// Fixed-point scale where `ONE` represents the value `1.0`.
pub const ONE: i32 = 4_096;

const TURN: i32 = 65_536;
const MILLI_DEGREES_PER_TURN: i64 = 360_000;
const BHASKARA_CONSTANT: i64 = 40_500_000_000;

/// An angle measured in binary radians, where a full turn is `65_536`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Angle(u16);

impl Angle {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn units(self) -> u16 {
        self.0
    }

    /// Builds an angle from whole degrees, wrapping at a full turn.
    ///
    /// # Panics
    ///
    /// Panics only if the wrapped degree value cannot fit the internal milli-degree conversion,
    /// which its bounded range prevents.
    #[must_use]
    pub fn from_degrees(degrees: i32) -> Self {
        let wrapped = i64::from(degrees).rem_euclid(360);
        let milli_degrees =
            i32::try_from(wrapped * 1_000).expect("wrapped whole degrees must fit milli-degrees");
        Self::from_milli_degrees(milli_degrees)
    }

    /// Builds an angle from thousandths of a degree, wrapping at a full turn.
    ///
    /// # Panics
    ///
    /// Panics when the wrapped unit count does not fit `u16`, which wrapping prevents.
    #[must_use]
    pub fn from_milli_degrees(milli_degrees: i32) -> Self {
        let wrapped = i64::from(milli_degrees).rem_euclid(MILLI_DEGREES_PER_TURN);
        let units = wrapped * i64::from(TURN) / MILLI_DEGREES_PER_TURN;
        Self(u16::try_from(units.rem_euclid(i64::from(TURN))).expect("wrapped units must fit u16"))
    }

    /// Returns this angle in thousandths of a degree.
    ///
    /// # Panics
    ///
    /// Panics when the converted value does not fit `i32`, which the unit range prevents.
    #[must_use]
    pub fn to_milli_degrees(self) -> i32 {
        let value = i64::from(self.0) * MILLI_DEGREES_PER_TURN / i64::from(TURN);
        i32::try_from(value).expect("milli-degrees must fit i32")
    }

    /// Returns this angle rotated by `units` binary radians.
    ///
    /// # Panics
    ///
    /// Panics when the wrapped unit count does not fit `u16`, which wrapping prevents.
    #[must_use]
    pub fn rotated(self, units: i32) -> Self {
        let value = (i32::from(self.0) + units).rem_euclid(TURN);
        Self(u16::try_from(value).expect("wrapped units must fit u16"))
    }

    /// Returns the shortest signed distance from this angle to `other`, in binary radians.
    #[must_use]
    pub fn signed_distance_to(self, other: Self) -> i32 {
        let difference = (i32::from(other.0) - i32::from(self.0)).rem_euclid(TURN);
        if difference > TURN / 2 {
            difference - TURN
        } else {
            difference
        }
    }

    /// Returns the sine of this angle scaled by [`ONE`].
    #[must_use]
    pub fn sin(self) -> i32 {
        let milli_degrees = i64::from(self.to_milli_degrees());
        if milli_degrees <= 180_000 {
            bhaskara_sine(milli_degrees)
        } else {
            -bhaskara_sine(milli_degrees - 180_000)
        }
    }

    /// Returns the cosine of this angle scaled by [`ONE`].
    #[must_use]
    pub fn cos(self) -> i32 {
        self.rotated(TURN / 4).sin()
    }
}

fn bhaskara_sine(milli_degrees: i64) -> i32 {
    let product = milli_degrees * (180_000 - milli_degrees);
    let value = i64::from(ONE) * 4 * product / (BHASKARA_CONSTANT - product);
    i32::try_from(value).expect("fixed-point sine must fit i32")
}

/// Returns `value * numerator / denominator` using a wide intermediate.
///
/// # Panics
///
/// Panics when `denominator` is zero or the result does not fit `i32`.
#[must_use]
pub fn scale(value: i32, numerator: i32, denominator: i32) -> i32 {
    assert!(denominator != 0, "scale denominator must not be zero");
    let result = i64::from(value) * i64::from(numerator) / i64::from(denominator);
    i32::try_from(result).expect("scaled value must fit i32")
}

/// Returns the per-mille position of `weight` eased with a smooth cubic curve.
///
/// # Panics
///
/// Panics when the eased value does not fit `i32`, which clamping prevents.
#[must_use]
pub fn ease_in_out(weight: i32) -> i32 {
    let weight = i64::from(weight.clamp(0, 1_000));
    let value = weight * weight * (3_000 - 2 * weight) / 1_000_000;
    i32::try_from(value).expect("eased weight must fit i32")
}

/// Returns `sqrt(ONE^2 - squared)` scaled by [`ONE`], clamped at zero.
///
/// # Panics
///
/// Panics when the component does not fit `i32`, which the unit range prevents.
#[must_use]
pub fn perpendicular_component(squared: i32) -> i32 {
    let remainder = i64::from(ONE) * i64::from(ONE) - i64::from(squared.max(0));
    if remainder <= 0 {
        return 0;
    }
    i32::try_from(remainder.isqrt()).expect("component must fit i32")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_matches_known_angles_within_fixed_point_tolerance() {
        let cases = [
            (0, 0),
            (30, ONE / 2),
            (90, ONE),
            (150, ONE / 2),
            (180, 0),
            (270, -ONE),
        ];

        for (degrees, expected) in cases {
            let actual = Angle::from_degrees(degrees).sin();
            assert!(
                (actual - expected).abs() <= 20,
                "sin({degrees}) was {actual}, expected about {expected}"
            );
        }
    }

    #[test]
    fn cosine_leads_sine_by_a_quarter_turn() {
        for degrees in (0..360).step_by(15) {
            let cosine = Angle::from_degrees(degrees).cos();
            let shifted = Angle::from_degrees(degrees + 90).sin();

            assert_eq!(cosine, shifted);
        }
    }

    #[test]
    fn angles_wrap_at_a_full_turn() {
        assert_eq!(Angle::from_degrees(370), Angle::from_degrees(10));
        assert_eq!(Angle::from_degrees(-90), Angle::from_degrees(270));
        assert_eq!(
            Angle::from_degrees(i32::MAX),
            Angle::from_degrees(i32::MAX.rem_euclid(360))
        );
        assert_eq!(
            Angle::from_degrees(i32::MIN),
            Angle::from_degrees(i32::MIN.rem_euclid(360))
        );
    }

    #[test]
    fn signed_distance_takes_the_shorter_direction() {
        let from = Angle::from_degrees(350);
        let to = Angle::from_degrees(10);

        assert!(from.signed_distance_to(to) > 0);
        assert!(to.signed_distance_to(from) < 0);
    }

    #[test]
    fn easing_is_monotonic_and_bounded() {
        let mut previous = -1;
        for weight in (0..=1_000).step_by(50) {
            let eased = ease_in_out(weight);
            assert!(eased >= previous, "easing must not decrease");
            assert!((0..=1_000).contains(&eased));
            previous = eased;
        }
    }

    #[test]
    fn perpendicular_component_is_zero_beyond_the_unit_circle() {
        assert_eq!(perpendicular_component(ONE * ONE), 0);
        assert_eq!(perpendicular_component(0), ONE);
    }
}
