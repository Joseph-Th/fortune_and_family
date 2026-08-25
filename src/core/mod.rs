//! Persistent runtime records and application-state ownership.

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
