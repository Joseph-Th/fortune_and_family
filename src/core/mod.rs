//! Core facade: persistent records, state container, and checksum folding.
//!
//! Purpose: re-export the durable types that systems and adapters need
//! without exposing internal module layout, so callers import `crate::core::*`
//! rather than reaching into `records`/`state`/`extended` directly.
//! Owns: module wiring only (`records`, `state`, `extended`, `checksum`).
//! Reads/Mutates: as its submodules (this file itself holds no state).
//! Does not own: business rules, persistence, or projection — it only
//! types the state those layers operate on.
//! Canonical operations: re-export only; callers use `AppState`, store
//! accessors, `HistoryLog`, and record getters defined in submodules.
//! Relevant invariants: none directly; enforces facade boundary so internal
//! layout can change without touching callers.
//! Focused tests: as submodules (`state_tests.rs`, persistence round-trip,
//! invariant batteries).

mod checksum;
mod extended;
mod records;
mod state;

pub(crate) use checksum::ChecksumFolder;

pub use extended::{
    AiObjective, CivicDebt, CivicDebtStatus, ContractStatus, Crisis, CrisisKind, CrisisStatus,
    DistrictRuntime, DynastyPair, EmploymentAgreement, EmploymentStatus, EnactedLaw, ExternalRoute,
    FamilyCouncilState, FamilyLink, FamilyLinkKind, HouseGovernance, InformationConfidence,
    InformationReport, InformationTarget, InstitutionRuntime, LawKind, LegalCase, LegalCaseKind,
    LegalCaseStatus, LegalClaimSource, Loan, LoanStatus, ObjectiveKind, ObjectiveStatus,
    OfficeDirectiveState, OfficePower, OutboxKind, OutboxMessage, Property, PropertyKind,
    PublicWork, PublicWorkKind, PublicWorkStatus, RelationshipState, SupplyContract,
};

pub(crate) use extended::MIN_PARENT_CHILD_AGE_GAP_DAYS;
pub use records::{
    AuditKind, AuditRecord, AuditSubject, Business, BusinessPolicy, BusinessStatus, CampaignPhase,
    Character, CharacterRole, CharacterStatus, ChronicleEntry, ChronicleKind, Dynasty, Household,
    MarketCause, MarketQuote, MarketState, ParseStartingBackgroundError, SocialClass,
    StartingBackground,
};
pub(crate) use records::{
    BusinessFinance, BusinessIdentity, BusinessOperations, CharacterCapabilities,
    CharacterIdentity, CharacterRuntime, DynastyIdentity, DynastyRelationships, DynastyResources,
    DynastyRuntime,
};
pub use state::{
    AppState, BusinessStore, CURRENT_SCHEMA_VERSION, CharacterStore, HistoryLog, HouseholdStore,
    NEUTRAL_FOOD_SATISFACTION_BASIS_POINTS, NewGameConfig, SimulationClock,
};
pub(crate) use state::{
    CampaignEvidenceMemo, NextIds, population_weighted_food_satisfaction_basis_points,
};
