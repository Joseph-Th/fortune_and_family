//! Deterministic simulation kernel for Civic Dynasty — one city, one persistent `AppState`.
//!
//! Rivergate is modeled as a deterministic political economy:
//! `Registry` (immutable authored definitions) plus `AppState` (every mutable
//! value needed for continuation, including RNG) flows through canonical
//! `systems/*` mutations and is observed through read-only `projection` or
//! HTML. Every adapter — CLI, persistence, gameplay harness, art — reuses
//! the same production path rather than reimplementing rules.
//!
//! Profiles: **Universal · Stateful Application · Deterministic System ·
//! Automated Behavior Evaluation · Artifact Generation**. Related authorities:
//! `README.md` (run/surface), `AGENTS.md` (execution card), `ARCHITECTURE.md`
//! (ownership), `STATUS.md` (current scope/schemas), `TESTING.md`
//! (verification), `DESIGN.md` (intent), `GAMEPLAY_HARNESS.md` (report).
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
//!
//! Determinism contract: given the same registry, state seed, command
//! sequence, and day count, execution reproduces bit-identically (state-owned
//! RNG, `BTreeMap`-ordered iteration, typed-ID tie-breakers, fixed-point
//! `Money`/`Quantity`).
//! Failure semantics: consequential operations validate before mutation,
//! preserve state on rejection, and report a typed
//! `CommandError`/`SimulationError`/`PersistenceError` variant.
//!
//! Ownership map (see `ARCHITECTURE.md`):
//! - `Registry` owns immutable Rivergate definitions.
//! - `AppState` owns every mutable value required for continuation.
//! - `systems/*` own canonical validation → commit pipelines.
//! - `persistence` / `projection` / `gameplay` / `art` / `main.rs` are
//!   adapters that reuse the canonical path and own no domain rules.
//!
//! Verification: `bash scripts/test.sh <lane>` per `TESTING.md`; no hosted CI.

pub mod art;
pub mod core;
pub mod gameplay;
pub mod ids;
pub mod money;
pub mod persistence;
pub mod projection;
pub mod registry;
pub mod rng;
mod systems;

pub use art::{
    ART_REVIEW_SCHEMA_VERSION, ArtReview, ArtReviewConfig, ArtReviewError, ArtReviewReport,
    ArtSeverity, CharacterSpec, SpriteRole, build_art_review, build_art_review_report,
    render_art_review_html,
};
pub use core::{AppState, HistoryLog, NewGameConfig};
pub use gameplay::{
    GAMEPLAY_REPORT_SCHEMA_VERSION, GameplayAggregate, GameplayCampaignReport,
    GameplayCandidateRanking, GameplayCommandKind, GameplayCommandStats,
    GameplayConsequenceProfile, GameplayDomain, GameplayFantasyArc, GameplayFeedbackEvent,
    GameplayFeedbackSource, GameplayFinding, GameplayFindingSeverity, GameplayHarnessConfig,
    GameplayHarnessError, GameplayHarnessReport, GameplayInteractionEdge, GameplayMeasure,
    GameplayMeasureChange, GameplayPersona, GameplayPhase, GameplayPhaseStats,
    GameplayQuietDiagnostic, GameplayScores, GameplaySnapshot, GameplaySuccessionTransition,
    GameplayTraceStep, GameplayViableOption, render_gameplay_report, run_gameplay_harness,
};
pub use persistence::{
    PersistenceError, SaveOutcome, SaveRevision, StateValidationKind, load_state,
    load_state_with_revision, save_state, save_state_cas, save_state_new, write_generated_file,
};
pub use projection::{
    AttentionItem, AttentionTone, CampaignProjection, StateSummary, build_campaign_projection,
    build_state_summary, render_campaign_html,
};
pub use registry::{Registry, build_rivergate_registry};
pub use systems::{
    BusinessAcquisitionQuote, CommandError, CommandOutcome, CrisisResponse, EducationFocus,
    InformationFocus, LaborResponse, LoanTerms, NewGameError, PlayerCommand,
    PropertyLiquidationQuote, PublicWorkFundingError, SimulationError, StrategicError,
    SupplyContractTerms, TimelineError, advance_days, apply_player_command, build_new_game,
    quote_business_acquisition, quote_property_liquidation, validate_invariants,
};

#[cfg(test)]
mod test_support;
