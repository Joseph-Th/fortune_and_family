//! Deterministic gameplay harness — the automated behavior-evaluation layer.
//!
//! Purpose: drive exhaustive `build_new_game` → `apply_player_command` →
//! `advance_days` loops exclusively through canonical public pipelines (no
//! direct record patching) to produce a `GameplayHarnessReport` that is the
//! machine contract for reachability/variety/interconnection/feedback/
//! resilience scores and findings. Every campaign is an independent
//! counterfactual experiment.
//! Owns: `GameplayHarnessConfig`/`Report`, `GameplayCommandKind` (32 kinds)
//! / `GameplayDomain` (17 domains), the `ALL_COMMAND_KINDS`/`ALL_DOMAINS`
//! exhaustive catalogs, report schema version, and re-exports of the seven
//! internal submodules (`candidates`, `harness`, `persona`, `findings`,
//! `scoring`, `types`, `render`).
//! Reads: `Registry` (immutable) and `AppState` via `lib.rs` entry points;
//! gameplay tests use `advance_days` through this facade.
//! Mutates: nothing directly; harness mutates cloned working states on
//! isolated branches (action vs baseline) and never patches authoritative
//! records.
//! Does not own: simulation or command policy (it validates through them) or
//! UI prose (render is a presentation layer over structured facts).
//! Canonical operations: `GameplayHarnessConfig::default` →
//! `run_gameplay_harness` → `GameplayHarnessReport` → `render_gameplay_report`
//! / JSON; decision cycle `capture → generate → rank → probe (clone) →
//! select → commit → advance action + baseline → attribute → score`.
//! Relevant invariants: bounded work (`max_candidate_probes`, horizons,
//! `trace_limit`); every report lists both observed and intentionally
//! unobserved state components; findings are derived facts, not prose;
//! persona variation stays behind `apply_player_command` boundaries;
//! `GAMEPLAY_REPORT_SCHEMA_VERSION` bumps on any semantic change.
//! Focused tests: `src/gameplay_tests.rs`, `bash scripts/test.sh gameplay`
//! (36+3 campaigns) and `gameplay-audit`.

use crate::core::{
    AppState, AuditKind, AuditRecord, AuditSubject, BusinessStatus, CharacterStatus,
    CivicDebtStatus, ContractStatus, CrisisKind, CrisisStatus, DynastyPair, EmploymentStatus,
    HouseGovernance, LawKind, LegalCaseKind, LegalCaseStatus, LoanStatus, NewGameConfig,
    ObjectiveStatus, OfficePower, PublicWorkKind, PublicWorkStatus, StartingBackground,
};
use crate::ids::{BusinessId, CharacterId, DistrictId, DynastyId, GoodId, InstitutionId};
use crate::money::{Money, Quantity, checked_cost_for};
use crate::registry::{GoodCategory, InstitutionKind, Registry};
// Test modules under `src/gameplay_tests.rs` reach `advance_days` through
// this module's namespace; production gameplay code uses the scratch entry.
#[cfg(test)]
use crate::systems::DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS;
#[cfg(test)]
use crate::systems::advance_days;
use crate::systems::{
    BUSINESS_POLICY_CHANGE_INTERVAL_DAYS, BUSINESS_WAGE_CHANGE_INTERVAL_DAYS,
    CIVIC_DEBT_CREDITOR_RESERVE, COMMISSIONED_INFORMATION_SOURCE, CRISIS_REFORM_COST,
    CRISIS_SUPPRESS_COST, CRISIS_SUPPRESS_LEGITIMACY_COST, CommandError, CrisisResponse,
    EducationFocus, FAMILY_COUNCIL_MEETING_COST, FAMILY_COUNCIL_MEETING_INTERVAL_DAYS,
    FAMILY_EDUCATION_COST, HEIR_DESIGNATION_INTERVAL_DAYS, HEIR_DESIGNATION_LEGITIMACY_COST,
    HEIR_DESIGNATION_UNITY_COST, HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS,
    HOUSE_GOVERNANCE_UNITY_COST, INFORMATION_COMMISSION_COST, INFORMATION_COMMISSION_INTERVAL_DAYS,
    INFORMATION_LEVERAGE_COST, INSTITUTION_ENDOWMENT_MAX, INSTITUTION_ENDOWMENT_MIN,
    INSTITUTION_SUPPORT_COST, INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS,
    INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT, InformationFocus,
    LABOR_CONDITIONS_IMPROVEMENT_COST, LABOR_NEGOTIATION_COST, LABOR_REPLACEMENT_COST,
    LAW_LEGITIMACY_REQUIREMENT, LAW_SPONSORSHIP_COST, LAW_SPONSORSHIP_INTERVAL_DAYS,
    LEGAL_CASE_FILING_COST, LEGAL_CASE_FILING_INTERVAL_DAYS, LaborResponse, LoanTerms,
    MAX_ACTIVE_SPONSORED_PUBLIC_WORKS, MAX_ACTIVE_WARDS, MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER,
    MAX_WEEKLY_WAGE_PER_WORKER, NewGameError, OFFICE_NOMINATION_CAMPAIGN_COST,
    OFFICE_NOMINATION_DELIVERY_REQUIREMENT, OFFICE_NOMINATION_REPUTATION_REQUIREMENT,
    OFFICE_NOMINATION_RESOLUTION_DAYS, OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS,
    OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST, OFFICE_POWER_ESTABLISHMENT_DAYS,
    PRIVATE_LOAN_COUNTERPARTY_RESERVE, PROPERTY_COUNTERPARTY_BUYER_RESERVE,
    PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS, PlayerCommand, STANDARD_CONTRACT_BATCHES_PER_WEEK,
    SimulationError, StrategicError, SupplyContractTerms, WARD_ADOPTION_COST,
    WARD_ADOPTION_DELIVERY_REQUIREMENT, WARD_ADOPTION_INTERVAL_DAYS,
    WARD_ADOPTION_LEGITIMACY_REQUIREMENT, WARD_ADOPTION_REPUTATION_REQUIREMENT,
    WARD_ADOPTION_UNITY_COST, active_player_ward_count, advance_days_scratch, apply_player_command,
    apply_player_command_scratch, available_household_workers, available_supply_contract_capacity,
    build_new_game, business_operating_spendable_cash, business_owner_distribution_reserve,
    business_recapitalization_target, business_sustainable_unit_cost,
    contract_counterparty_price_bounds, contract_relationship_pressure_basis_points,
    credit_pair_blocks_new_loan, crisis_relief_cost, crisis_response_contains_crisis,
    defaulted_loan_restructuring_available, family_education_next_day,
    has_established_player_institution_membership, has_established_player_office_power,
    has_player_office, institution_capability_score, institution_endowment_next_day,
    institution_membership_count, institution_support_day,
    institution_support_delivery_requirement, institution_support_next_day,
    latest_defaulted_loan_for_pair, market_reference_weekly_wage,
    office_nomination_delivery_requirement, office_nomination_next_day, player_contract_deliveries,
    private_loan_borrower_financing_pressure, projected_dynasty_monthly_office_duty,
    projected_dynasty_monthly_office_duty_with_additional_offices,
    public_work_initial_contribution, quote_business_acquisition, quote_information_leverage,
    quote_player_legal_claim, quote_player_legal_settlement, quote_property_liquidation,
    required_office_power_for_law, unresolved_default_owed_elsewhere, validate_invariants,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use thiserror::Error;

const ALL_COMMAND_KINDS: [GameplayCommandKind; 32] = [
    GameplayCommandKind::TransferBusinessCash,
    GameplayCommandKind::WithdrawBusinessCash,
    GameplayCommandKind::AcquireBusiness,
    GameplayCommandKind::InvestInBusiness,
    GameplayCommandKind::SetBusinessPolicy,
    GameplayCommandKind::SetBusinessWages,
    GameplayCommandKind::SecureSupply,
    GameplayCommandKind::SellOutput,
    GameplayCommandKind::BorrowFunds,
    GameplayCommandKind::ExtendCredit,
    GameplayCommandKind::BuyProperty,
    GameplayCommandKind::SellProperty,
    GameplayCommandKind::EnactLaw,
    GameplayCommandKind::StartPublicWork,
    GameplayCommandKind::FundPublicWork,
    GameplayCommandKind::FileLegalCase,
    GameplayCommandKind::SettleLegalCase,
    GameplayCommandKind::SetHouseGovernance,
    GameplayCommandKind::ConveneFamilyCouncil,
    GameplayCommandKind::DesignateHeir,
    GameplayCommandKind::AdoptWard,
    GameplayCommandKind::EducateFamilyMember,
    GameplayCommandKind::CultivateInstitutionSupport,
    GameplayCommandKind::EndowInstitution,
    GameplayCommandKind::NominateForOffice,
    GameplayCommandKind::ExerciseOfficePower,
    GameplayCommandKind::WithdrawFromInstitution,
    GameplayCommandKind::RespondToCrisis,
    GameplayCommandKind::ResolveLaborDispute,
    GameplayCommandKind::CommissionInformation,
    GameplayCommandKind::LeverageInformation,
    GameplayCommandKind::AcknowledgeNotification,
];

const ALL_DOMAINS: [GameplayDomain; 17] = [
    GameplayDomain::Economy,
    GameplayDomain::Business,
    GameplayDomain::Market,
    GameplayDomain::Contracts,
    GameplayDomain::Loans,
    GameplayDomain::Property,
    GameplayDomain::Labor,
    GameplayDomain::Relationships,
    GameplayDomain::Dynasty,
    GameplayDomain::Family,
    GameplayDomain::Institutions,
    GameplayDomain::Law,
    GameplayDomain::Districts,
    GameplayDomain::Legal,
    GameplayDomain::Crises,
    GameplayDomain::Information,
    GameplayDomain::Feedback,
];

/// Version of the serialized gameplay-harness report contract.
pub const GAMEPLAY_REPORT_SCHEMA_VERSION: u16 = 78;
#[cfg(test)]
const HARNESS_OBSERVED_STATE_COMPONENTS: &[&str] = &[
    "clock",
    "player_dynasty_id",
    "dynasties",
    "characters",
    "households",
    "businesses",
    "institutions",
    "market",
    "contracts",
    "loans",
    "civic_debts",
    "properties",
    "employment",
    "family_links",
    "family_councils",
    "laws",
    "relationships",
    "information_reports",
    "ai_objectives",
    "districts",
    "public_works",
    "legal_cases",
    "external_routes",
    "crises",
    "outbox",
    "chronicle",
    "audit_log",
];
#[cfg(test)]
const HARNESS_INTENTIONALLY_UNOBSERVED_STATE_COMPONENTS: &[&str] = &[
    "schema_version",
    "scenario_key",
    "registry_fingerprint",
    "rng",
    "next_ids",
];

// The harness is organized by concern; every piece is re-exported here so
// `crate::gameplay::*` (the lib.rs facade and the sibling test module) sees
// one flat, stable namespace.

mod candidates;
mod findings;
mod harness;
mod persona;
mod render;
mod scoring;
mod types;

pub(crate) use candidates::*;
pub(crate) use findings::*;
pub use harness::*;
pub use persona::*;
pub use render::*;
pub(crate) use scoring::*;
pub use types::*;

#[cfg(test)]
#[path = "../gameplay_tests.rs"]
mod tests;
