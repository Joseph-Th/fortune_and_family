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
pub mod gameplay;
pub mod ids;
pub mod money;
pub mod persistence;
pub mod projection;
pub mod registry;
pub mod rng;
pub mod systems;

pub use core::{AppState, NewGameConfig};
pub use gameplay::{
    GAMEPLAY_REPORT_SCHEMA_VERSION, GameplayAggregate, GameplayCampaignReport,
    GameplayCandidateRanking, GameplayCommandKind, GameplayCommandStats,
    GameplayConsequenceProfile, GameplayDomain, GameplayFantasyArc, GameplayFinding,
    GameplayFindingSeverity, GameplayHarnessConfig, GameplayHarnessError, GameplayHarnessReport,
    GameplayInteractionEdge, GameplayMeasure, GameplayPersona, GameplayPhase, GameplayPhaseStats,
    GameplayScores, GameplaySnapshot, GameplayTraceStep, GameplayViableOption,
    render_gameplay_report, run_gameplay_harness,
};
pub use persistence::{PersistenceError, StateValidationKind, load_state, save_state};
pub use projection::{
    CampaignProjection, StateSummary, build_campaign_projection, build_state_summary,
    render_campaign_html,
};
pub use registry::{Registry, build_rivergate_registry};
pub use systems::{
    BusinessAcquisitionQuote, CommandError, CommandOutcome, CrisisResponse, EducationFocus,
    InformationFocus, LaborResponse, NewGameError, PlayerCommand, PropertyLiquidationQuote,
    SimulationError, advance_days, apply_player_command, build_new_game,
    quote_business_acquisition, quote_property_liquidation, validate_invariants,
};

#[cfg(test)]
mod test_support;
