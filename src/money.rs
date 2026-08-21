//! Fixed-point economic value types used by every simulation subsystem.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Money(i64);

impl Money {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn from_copper(copper: i64) -> Self {
        Self(copper)
    }

    #[must_use]
    pub const fn copper(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the largest nonnegative amount that can be added without overflowing.
    #[must_use]
    pub const fn max_nonnegative_addend(self) -> Self {
        let headroom = i64::MAX.saturating_sub(self.0);
        if headroom > 0 {
            Self(headroom)
        } else {
            Self::ZERO
        }
    }

    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[must_use]
    pub const fn saturating_mul(self, factor: i64) -> Self {
        Self(self.0.saturating_mul(factor))
    }

    #[must_use]
    /// Multiplies this value by a rational number using a wide intermediate and saturates only the
    /// final result.
    ///
    /// # Panics
    ///
    /// Panics when `denominator` is zero.
    pub const fn saturating_mul_ratio(self, numerator: i64, denominator: i64) -> Self {
        Self(saturating_mul_ratio_i64(self.0, numerator, denominator))
    }

    #[must_use]
    /// Divides this positive value by a positive denominator and rounds any fractional copper
    /// upward.
    ///
    /// # Panics
    ///
    /// Panics when this value is not positive or `denominator` is not positive.
    pub const fn ceil_div_positive(self, denominator: i64) -> Self {
        assert!(self.0 > 0, "ceiling division dividend must be positive");
        assert!(
            denominator > 0,
            "ceiling division denominator must be positive"
        );
        Self(self.0 / denominator + (self.0 % denominator != 0) as i64)
    }
    /// Multiplies this value by a rational number using a wide intermediate.
    ///
    /// Returns `None` when the final value is outside the supported money range.
    ///
    /// # Panics
    ///
    /// Panics when `denominator` is zero.
    #[must_use]
    pub const fn checked_mul_ratio(self, numerator: i64, denominator: i64) -> Option<Self> {
        assert!(denominator != 0, "ratio denominator must not be zero");
        let result = (self.0 as i128) * (numerator as i128) / (denominator as i128);
        match checked_i128_to_i64(result) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[must_use]
    /// Multiplies nonnegative values by a nonnegative rational number and rounds any fractional
    /// copper upward.
    ///
    /// # Panics
    ///
    /// Panics when this value or `numerator` is negative, or when `denominator` is not positive.
    pub const fn saturating_mul_ratio_ceil_nonnegative(
        self,
        numerator: i64,
        denominator: i64,
    ) -> Self {
        assert!(self.0 >= 0, "rounded money ratio value must be nonnegative");
        assert!(
            numerator >= 0,
            "rounded money ratio numerator must be nonnegative"
        );
        assert!(
            denominator > 0,
            "rounded money ratio denominator must be positive"
        );
        let product = (self.0 as i128).saturating_mul(numerator as i128);
        let denominator = denominator as i128;
        let quotient = product / denominator;
        let rounded = quotient.saturating_add((product % denominator != 0) as i128);
        Self(saturating_i128_to_i64(rounded))
    }

    #[must_use]
    pub const fn min(self, other: Self) -> Self {
        if self.0 <= other.0 { self } else { other }
    }
}

impl fmt::Display for Money {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let absolute = self.0.unsigned_abs();
        write!(
            formatter,
            "{sign}{}.{:02} cr",
            absolute / 100,
            absolute % 100
        )
    }
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Quantity(i64);

impl Quantity {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1_000);

    #[must_use]
    pub const fn from_milliunits(milliunits: i64) -> Self {
        Self(milliunits)
    }

    #[must_use]
    pub const fn from_units(units: i64) -> Self {
        Self(units.saturating_mul(1_000))
    }

    #[must_use]
    pub const fn milliunits(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the largest nonnegative amount that can be added without overflowing.
    #[must_use]
    pub const fn max_nonnegative_addend(self) -> Self {
        let headroom = i64::MAX.saturating_sub(self.0);
        if headroom > 0 {
            Self(headroom)
        } else {
            Self::ZERO
        }
    }

    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[must_use]
    /// Multiplies this quantity by a rational number using a wide intermediate and saturates only
    /// the final result.
    ///
    /// # Panics
    ///
    /// Panics when `denominator` is zero.
    pub const fn saturating_mul_ratio(self, numerator: i64, denominator: i64) -> Self {
        Self(saturating_mul_ratio_i64(self.0, numerator, denominator))
    }

    #[must_use]
    pub const fn min(self, other: Self) -> Self {
        if self.0 <= other.0 { self } else { other }
    }

    #[must_use]
    pub const fn max(self, other: Self) -> Self {
        if self.0 >= other.0 { self } else { other }
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let absolute = self.0.unsigned_abs();
        write!(
            formatter,
            "{sign}{}.{:03}",
            absolute / 1_000,
            absolute % 1_000
        )
    }
}

#[must_use]
pub fn cost_for(quantity: Quantity, unit_price: Money) -> Money {
    Money::from_copper(saturating_i128_to_i64(rounded_cost_copper_wide(
        quantity, unit_price,
    )))
}

/// Returns the rounded transaction cost when it fits the supported money range.
///
/// Positive fractional copper is rounded upward, matching [`cost_for`].
#[must_use]
pub fn checked_cost_for(quantity: Quantity, unit_price: Money) -> Option<Money> {
    checked_i128_to_i64(rounded_cost_copper_wide(quantity, unit_price)).map(Money::from_copper)
}

pub(crate) fn rounded_cost_copper_wide(quantity: Quantity, unit_price: Money) -> i128 {
    let product = i128::from(quantity.milliunits()) * i128::from(unit_price.copper());
    let whole_copper = product / 1_000;
    let positive_remainder = product > 0 && product % 1_000 != 0;
    whole_copper + i128::from(positive_remainder)
}

#[must_use]
pub fn affordable_quantity(cash: Money, unit_price: Money) -> Quantity {
    if cash.copper() <= 0 || unit_price.copper() <= 0 {
        return Quantity::ZERO;
    }

    Quantity::from_milliunits(saturating_mul_ratio_i64(
        cash.copper(),
        1_000,
        unit_price.copper(),
    ))
}

const fn saturating_mul_ratio_i64(value: i64, numerator: i64, denominator: i64) -> i64 {
    assert!(denominator != 0, "ratio denominator must not be zero");
    let result = (value as i128).saturating_mul(numerator as i128) / denominator as i128;
    saturating_i128_to_i64(result)
}

/// Ceil-divides a nonnegative numerator by a positive denominator.
pub(crate) const fn ceil_div_nonnegative(numerator: i64, denominator: i64) -> i64 {
    assert!(
        numerator >= 0,
        "ceiling division numerator must be nonnegative"
    );
    assert!(
        denominator > 0,
        "ceiling division denominator must be positive"
    );
    numerator / denominator + (numerator % denominator != 0) as i64
}

// The explicit bounds checks make this final narrowing conversion lossless.
#[allow(clippy::cast_possible_truncation)]
const fn saturating_i128_to_i64(value: i128) -> i64 {
    if value > i64::MAX as i128 {
        i64::MAX
    } else if value < i64::MIN as i128 {
        i64::MIN
    } else {
        value as i64
    }
}

const fn checked_i128_to_i64(value: i128) -> Option<i64> {
    if value > i64::MAX as i128 || value < i64::MIN as i128 {
        None
    } else {
        // The explicit bounds check makes this narrowing conversion lossless.
        #[allow(clippy::cast_possible_truncation)]
        Some(value as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_fractional_cost_rounds_up_to_one_copper() {
        assert_eq!(
            cost_for(Quantity::from_milliunits(1), Money::from_copper(1)),
            Money::from_copper(1),
            "a positive transfer must never become free through fixed-point truncation"
        );
    }

    #[test]
    fn affordable_quantity_remains_affordable_with_rounded_costs() {
        let cash = Money::from_copper(1);
        let price = Money::from_copper(300);
        let quantity = affordable_quantity(cash, price);

        assert!(
            cost_for(quantity, price) <= cash,
            "rounded transaction cost must not exceed the cash used to derive affordability"
        );
    }

    #[test]
    fn cost_uses_a_wide_intermediate_before_rounding() {
        let quantity = Quantity::from_milliunits(i64::MAX);
        let price = Money::from_copper(2);
        let expected = Money::from_copper(
            i64::try_from((i128::from(i64::MAX) * 2 + 999) / 1_000)
                .expect("expected cost must fit the supported money range"),
        );

        assert_eq!(cost_for(quantity, price), expected);
    }

    #[test]
    fn checked_cost_rejects_a_final_money_overflow() {
        let quantity = Quantity::from_milliunits(i64::MAX);
        let price = Money::from_copper(i64::MAX);

        assert_eq!(checked_cost_for(quantity, price), None);
        assert_eq!(cost_for(quantity, price), Money::from_copper(i64::MAX));
    }

    #[test]
    fn affordability_uses_a_wide_intermediate_before_division() {
        let cash = Money::from_copper(i64::MAX / 2);

        assert_eq!(
            affordable_quantity(cash, Money::from_copper(1_000)),
            Quantity::from_milliunits(cash.copper())
        );
    }

    #[test]
    fn quantity_ratio_saturates_only_the_final_result() {
        assert_eq!(
            Quantity::from_milliunits(i64::MAX).saturating_mul_ratio(2, 2),
            Quantity::from_milliunits(i64::MAX)
        );
        assert_eq!(
            Quantity::from_milliunits(i64::MIN).saturating_mul_ratio(1, -1),
            Quantity::from_milliunits(i64::MAX)
        );
    }

    #[test]
    fn money_ratio_saturates_only_the_final_result() {
        assert_eq!(
            Money::from_copper(i64::MAX).saturating_mul_ratio(2, 2),
            Money::from_copper(i64::MAX)
        );
    }

    #[test]
    fn checked_money_ratio_rejects_final_overflow() {
        assert_eq!(Money::from_copper(i64::MAX).checked_mul_ratio(11, 10), None);
        assert_eq!(
            Money::from_copper(100).checked_mul_ratio(11, 10),
            Some(Money::from_copper(110))
        );
    }

    #[test]
    fn nonnegative_money_ratio_rounds_up_after_wide_multiplication() {
        assert_eq!(
            Money::from_copper(1).saturating_mul_ratio_ceil_nonnegative(1, 2),
            Money::from_copper(1)
        );
        assert_eq!(
            Money::from_copper(i64::MAX).saturating_mul_ratio_ceil_nonnegative(2, 2),
            Money::from_copper(i64::MAX)
        );
    }

    #[test]
    fn nonnegative_addend_capacity_preserves_exact_boundary_addition() {
        let money = Money::from_copper(i64::MAX - 7);
        let quantity = Quantity::from_milliunits(i64::MAX - 11);

        assert_eq!(money.max_nonnegative_addend(), Money::from_copper(7));
        assert_eq!(
            money.checked_add(money.max_nonnegative_addend()),
            Some(Money::from_copper(i64::MAX))
        );
        assert_eq!(
            quantity.max_nonnegative_addend(),
            Quantity::from_milliunits(11)
        );
        assert_eq!(
            quantity.checked_add(quantity.max_nonnegative_addend()),
            Some(Quantity::from_milliunits(i64::MAX))
        );
    }

    #[test]
    fn checked_subtraction_preserves_exact_results_and_rejects_overflow() {
        assert_eq!(
            Money::from_copper(11).checked_sub(Money::from_copper(7)),
            Some(Money::from_copper(4))
        );
        assert_eq!(
            Quantity::from_milliunits(11).checked_sub(Quantity::from_milliunits(7)),
            Some(Quantity::from_milliunits(4))
        );
        assert_eq!(
            Money::from_copper(i64::MIN).checked_sub(Money::from_copper(1)),
            None
        );
        assert_eq!(
            Quantity::from_milliunits(i64::MAX).checked_sub(Quantity::from_milliunits(-1)),
            None
        );
    }
}
