//! Serializable deterministic random number generator owned by application state.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    #[must_use]
    pub const fn seeded(seed: u64) -> Self {
        let state = if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        };
        Self { state }
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
    pub fn chance_basis_points(&mut self, basis_points: u16) -> bool {
        assert!(
            basis_points <= 10_000,
            "chance must be at most 10,000 basis points"
        );
        self.range_u32(10_000) < u32::from(basis_points)
    }
}
