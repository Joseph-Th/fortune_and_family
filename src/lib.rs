//! Deterministic simulation kernel for Civic Dynasty.

pub mod core;
pub mod ids;
pub mod money;
pub mod persistence;
pub mod projection;
pub mod registry;
pub mod rng;
pub mod systems;

pub use core::{AppState, NewGameConfig, StateSummary};
pub use persistence::{PersistenceError, load_state, save_state};
pub use projection::{CampaignProjection, build_campaign_projection, render_campaign_html};
pub use registry::{Registry, build_rivergate_registry};
pub use systems::{
    CommandError, CommandOutcome, CrisisResponse, LaborResponse, PlayerCommand, SimulationError,
    advance_days, apply_player_command, build_new_game, validate_invariants,
};
