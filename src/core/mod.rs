//! Core facade: persistent records, state container, and checksum folding.
//!
//! Purpose: re-export the durable types that systems and adapters need
//! without exposing internal module layout.
//! Owns: module wiring only (`records`, `state`, `extended`, `checksum`).
//! Reads/Mutates: as its submodules.
//! Does not own: business rules, persistence, or projection.
//! Focused tests: as submodules (`state_tests.rs`, persistence, invariants).

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
