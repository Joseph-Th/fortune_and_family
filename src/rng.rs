//! Serializable deterministic randomness owned by `AppState`.
//!
//! Purpose: supply reproducible entropy for simulation, bootstrap, and
//! gameplay-harness variation without touching OS or thread-local RNG.
//! Owns: `DeterministicRng` state, its SplitMix-derived `next_u64`, and
//! bounded helpers `range_u32` / `is_chance_success`.
//! Reads: nothing.
//! Mutates: its own `state` (owned by `AppState.rng`).
//! Does not own: clocks, registries, or domain decisions.
//! Determinism: given the same seed, sequence of calls, and persisted
//! state, every campaign reproduces bit-identically; `AppState` persists
//! the RNG so continuation is exact.
//! Focused tests: `src/rng.rs::tests` distinct streams, `src/simulation/*` determinism.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    /// Creates an RNG whose entire future stream is determined by `seed`.
    /// `AppState.rng` persists this value so continuation is exact.
    #[must_use]
    pub const fn seeded(seed: u64) -> Self {
        Self { state: seed }
    }

    #[must_use]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    #[must_use]
    /// Returns a value in `0..upper_exclusive`.
    ///
    /// # Panics
    ///
    /// Panics when `upper_exclusive` is zero.
    pub fn range_u32(&mut self, upper_exclusive: u32) -> u32 {
        assert!(upper_exclusive > 0, "random range must not be empty");
        u32::try_from(self.next_u64() % u64::from(upper_exclusive))
            .expect("reduced random value must fit u32")
    }

    #[must_use]
    /// Returns whether a deterministic basis-point probability succeeds.
    ///
    /// # Panics
    ///
    /// Panics when `basis_points` is greater than 10,000.
    pub fn is_chance_success(&mut self, basis_points: u16) -> bool {
        assert!(
            basis_points <= 10_000,
            "chance must be at most 10,000 basis points"
        );
        self.range_u32(10_000) < u32::from(basis_points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_seed_values_do_not_share_the_same_initial_stream() {
        let mut zero = DeterministicRng::seeded(0);
        let mut golden_ratio = DeterministicRng::seeded(0x9E37_79B9_7F4A_7C15);

        assert_ne!(zero.next_u64(), golden_ratio.next_u64());
    }

    #[test]
    fn identical_seed_values_reproduce_the_same_stream() {
        let mut first = DeterministicRng::seeded(0);
        let mut second = DeterministicRng::seeded(0);

        for _ in 0..16 {
            assert_eq!(first.next_u64(), second.next_u64());
        }
    }
}
