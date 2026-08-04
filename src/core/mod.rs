//! Persistent runtime records and application-state ownership.

mod extended;
mod records;
mod state;

pub use extended::{
    AiObjective, CivicDebt, CivicDebtStatus, ContractStatus, Crisis, CrisisKind, CrisisStatus,
    DistrictRuntime, DynastyPair, EmploymentAgreement, EmploymentStatus, EnactedLaw, ExternalRoute,
    FamilyCouncilState, FamilyLink, FamilyLinkKind, HouseGovernance, InformationConfidence,
    InformationReport, InstitutionRuntime, LawKind, LegalCase, LegalCaseKind, LegalCaseStatus,
    Loan, LoanStatus, ObjectiveKind, ObjectiveStatus, OfficePower, OutboxKind, OutboxMessage,
    Property, PropertyKind, PublicWork, PublicWorkKind, PublicWorkStatus, RelationshipState,
    SupplyContract,
};

pub use records::{
    AuditKind, AuditRecord, Business, BusinessPolicy, BusinessStatus, CampaignPhase, Character,
    CharacterRole, CharacterStatus, ChronicleEntry, ChronicleKind, Dynasty, Household, MarketCause,
    MarketQuote, MarketState, ParseStartingBackgroundError, SocialClass, StartingBackground,
};
pub(crate) use records::{
    BusinessFinance, BusinessIdentity, BusinessOperations, CharacterCapabilities,
    CharacterIdentity, CharacterRuntime, DynastyIdentity, DynastyRelationships, DynastyResources,
    DynastyRuntime,
};
pub use state::{
    AppState, BusinessStore, CURRENT_SCHEMA_VERSION, CharacterStore, HouseholdStore, NewGameConfig,
    SimulationClock,
};
pub(crate) use state::{NextIds, population_weighted_food_satisfaction_basis_points};
