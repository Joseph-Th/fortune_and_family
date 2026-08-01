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
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    #[must_use]
    pub const fn saturating_mul(self, factor: i64) -> Self {
        Self(self.0.saturating_mul(factor))
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
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    #[must_use]
    /// Multiplies this quantity by a rational number with saturating integer arithmetic.
    ///
    /// # Panics
    ///
    /// Panics when `denominator` is zero.
    pub const fn saturating_mul_ratio(self, numerator: i64, denominator: i64) -> Self {
        assert!(
            denominator != 0,
            "quantity ratio denominator must not be zero"
        );
        Self(self.0.saturating_mul(numerator) / denominator)
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
    Money::from_copper(quantity.milliunits().saturating_mul(unit_price.copper()) / 1_000)
}

#[must_use]
pub fn affordable_quantity(cash: Money, unit_price: Money) -> Quantity {
    if cash.copper() <= 0 || unit_price.copper() <= 0 {
        return Quantity::ZERO;
    }

    Quantity::from_milliunits(cash.copper().saturating_mul(1_000) / unit_price.copper())
}
