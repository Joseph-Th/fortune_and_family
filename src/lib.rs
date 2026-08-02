//! Deterministic simulation kernel for Civic Dynasty.
//!
//! ```
//! use civic_dynasty::{NewGameConfig, advance_days, build_new_game, build_rivergate_registry};
//!
//! let registry = build_rivergate_registry();
//! let mut state = build_new_game(&registry, NewGameConfig::default())
//!     .expect("default campaign must build");
//! advance_days(&registry, &mut state, 1).expect("campaign must advance");
//!
//! assert_eq!(state.clock().day(), 1);
//! ```

pub mod core;
pub mod ids;
pub mod money;
pub mod persistence;
pub mod projection;
pub mod registry;
pub mod rng;
pub mod systems;

pub use core::{AppState, NewGameConfig, StateSummary};
pub use persistence::{PersistenceError, StateValidationKind, load_state, save_state};
pub use projection::{CampaignProjection, build_campaign_projection, render_campaign_html};
pub use registry::{Registry, build_rivergate_registry};
pub use systems::{
    CommandError, CommandOutcome, CrisisResponse, LaborResponse, NewGameError, PlayerCommand,
    SimulationError, advance_days, apply_player_command, build_new_game, validate_invariants,
};

#[cfg(test)]
mod test_support;
