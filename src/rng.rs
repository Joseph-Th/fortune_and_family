//! Serializable deterministic randomness owned by `AppState` — the sole entropy source.
//!
//! Purpose: supply reproducible entropy for simulation, bootstrap, and
//! gameplay-harness variation without touching OS entropy, thread-local RNG,
//! or wall-clock time. Every decision that needs randomness reads
//! `AppState.rng` so a saved campaign resumes with identical future draws.
//! Owns: `DeterministicRng` state, its SplitMix64-derived `next_u64` step,
//! and bounded helpers `range_u32` / `is_chance_success`.
//! Reads: nothing.
//! Mutates: its own `state` (persisted inside `AppState.rng`; callers hold
//! `&mut AppState` while drawing).
//! Does not own: clocks, registries, market or business rules, or any
//! domain decision that consumes the draw.
//! Canonical operations: `seeded` → `next_u64` → `range_u32` /
//! `is_chance_success` (every call advances the persisted stream).
//! Relevant invariants: given the same seed and the same ordered call
//! sequence, the stream is bit-identical across builds; `AppState`
//! persistence includes the RNG word so continuation is exact; no fallback
//! to OS entropy exists.
//! Focused tests: `src/rng.rs::tests` distinct and identical streams,
//! `src/simulation/*` cross-day determinism, persistence round-trip includes RNG.

use serde::{Deserialize, Serialize};

/// Deterministic, serializable RNG owned by `AppState`.
///
/// Wraps a SplitMix64 step over a single `u64` word. The stream is fully
/// determined by the seed and the ordered call sequence, persists across
/// saves, and never falls back to OS entropy. Every stochastic simulation
/// decision reads `AppState.rng` so a replay from the same save produces
/// bit-identical futures.
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

    /// Advances the stream by one SplitMix64 step and returns the next `u64`.
    ///
    /// # Panics
    ///
    /// Never panics; the step is wrapping arithmetic.
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
