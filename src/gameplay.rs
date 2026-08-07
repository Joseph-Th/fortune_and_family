//! Deterministic gameplay harness that drives the public player-command and simulation pipelines.

use crate::core::{
    AppState, AuditKind, AuditRecord, AuditSubject, BusinessStatus, CharacterStatus,
    CivicDebtStatus, ContractStatus, CrisisKind, CrisisStatus, EmploymentStatus, FamilyLinkKind,
    HouseGovernance, LawKind, LegalCaseKind, LegalCaseStatus, LoanStatus, NewGameConfig,
    ObjectiveStatus, OfficePower, PublicWorkKind, PublicWorkStatus, StartingBackground,
};
use crate::ids::{BusinessId, CharacterId, DistrictId, DynastyId, InstitutionId};
use crate::money::{Money, Quantity, cost_for};
use crate::registry::{GoodCategory, Registry};
use crate::systems::{
    BUSINESS_POLICY_CHANGE_INTERVAL_DAYS, CIVIC_DEBT_CREDITOR_RESERVE,
    COMMISSIONED_INFORMATION_SOURCE, CommandError, CrisisResponse,
    DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS, EducationFocus, FAMILY_COUNCIL_MEETING_COST,
    FAMILY_COUNCIL_MEETING_INTERVAL_DAYS, FAMILY_EDUCATION_COST, FAMILY_EDUCATION_INTERVAL_DAYS,
    HEIR_DESIGNATION_INTERVAL_DAYS, HEIR_DESIGNATION_LEGITIMACY_COST,
    HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS, INFORMATION_COMMISSION_COST,
    INFORMATION_COMMISSION_INTERVAL_DAYS, INFORMATION_LEVERAGE_COST, INSTITUTION_SUPPORT_COST,
    INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT, INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS,
    INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT, InformationFocus, LABOR_REPLACEMENT_COST,
    LAW_LEGITIMACY_REQUIREMENT, LAW_SPONSORSHIP_INTERVAL_DAYS, LEGAL_CASE_FILING_COST,
    LEGAL_CASE_FILING_INTERVAL_DAYS, LaborResponse, LoanTerms, MAX_ACTIVE_SPONSORED_PUBLIC_WORKS,
    MAX_ACTIVE_WARDS, MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER, NewGameError,
    OFFICE_NOMINATION_DELIVERY_REQUIREMENT, OFFICE_NOMINATION_REPUTATION_REQUIREMENT,
    OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS, OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST,
    OFFICE_POWER_ESTABLISHMENT_DAYS, PRIVATE_LOAN_COUNTERPARTY_RESERVE,
    PROPERTY_COUNTERPARTY_BUYER_RESERVE, PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS, PlayerCommand,
    STANDARD_CONTRACT_BATCHES_PER_WEEK, SimulationError, StrategicError, SupplyContractTerms,
    WARD_ADOPTION_COST, WARD_ADOPTION_DELIVERY_REQUIREMENT, WARD_ADOPTION_INTERVAL_DAYS,
    WARD_ADOPTION_LEGITIMACY_REQUIREMENT, WARD_ADOPTION_REPUTATION_REQUIREMENT, advance_days,
    apply_player_command, available_household_workers, available_supply_contract_capacity,
    build_new_game, business_recapitalization_target, contract_counterparty_price_bounds,
    contract_relationship_pressure_basis_points, crisis_response_contains_crisis,
    has_established_player_office_power, institution_capability_score,
    institution_membership_count, institution_support_day, institution_support_next_day,
    office_nomination_next_day, player_contract_deliveries, projected_dynasty_monthly_office_duty,
    quote_business_acquisition, quote_information_leverage, quote_property_liquidation,
    required_office_power_for_law, validate_invariants,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use thiserror::Error;

const ALL_COMMAND_KINDS: [GameplayCommandKind; 27] = [
    GameplayCommandKind::TransferBusinessCash,
    GameplayCommandKind::AcquireBusiness,
    GameplayCommandKind::InvestInBusiness,
    GameplayCommandKind::SetBusinessPolicy,
    GameplayCommandKind::SecureSupply,
    GameplayCommandKind::SellOutput,
    GameplayCommandKind::BorrowFunds,
    GameplayCommandKind::ExtendCredit,
    GameplayCommandKind::BuyProperty,
    GameplayCommandKind::SellProperty,
    GameplayCommandKind::EnactLaw,
    GameplayCommandKind::StartPublicWork,
    GameplayCommandKind::FileLegalCase,
    GameplayCommandKind::SetHouseGovernance,
    GameplayCommandKind::ConveneFamilyCouncil,
    GameplayCommandKind::DesignateHeir,
    GameplayCommandKind::AdoptWard,
    GameplayCommandKind::EducateFamilyMember,
    GameplayCommandKind::CultivateInstitutionSupport,
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
pub const GAMEPLAY_REPORT_SCHEMA_VERSION: u16 = 38;
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
const HARNESS_INTENTIONALLY_UNOBSERVED_STATE_COMPONENTS: &[&str] =
    &["schema_version", "scenario_key", "rng", "next_ids"];
const CLOSE_CHOICE_SCORE_GAP: i64 = 300;
const HEIR_CONFIRMATION_HEAD_AGE_YEARS: i64 = 52;
const HEIR_CONFIRMATION_HEALTH_THRESHOLD: u16 = 5_000;
const COMMERCIAL_STANDING_REPUTATION_REQUIREMENT: u16 = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
const NOTIFICATION_BATCH_THRESHOLD: usize = 8;
const AGENT_LOAN_AMORTIZATION_WEEKS: i64 = 104;
const AGENT_OPPORTUNIST_LOAN_AMORTIZATION_WEEKS: i64 = 26;
const AGENT_OPPORTUNIST_LOAN_INTEREST_BASIS_POINTS: u16 = 1_800;
const AGENT_EXTERNAL_CREDIT_TREASURY_PRESSURE: Money = Money::from_copper(25_000);
const AGENT_OFFICE_DUTY_RESERVE_MONTHS: i64 = 12;
const AGENT_OFFICE_LIQUIDITY_BUFFER: Money = Money::from_copper(5_000);
const AGENT_FAMILY_COUNCIL_DUTY_RESERVE_MONTHS: i64 = 6;
const AGENT_FAMILY_COUNCIL_LIQUIDITY_BUFFER: Money = Money::from_copper(2_500);
const FAMILY_COUNCIL_INTERVENTION_UNITY_THRESHOLD: u16 = 7_000;
const AGENT_CONTRACT_DURATION_WEEKS: u16 = 104;
const AGENT_CASH_REBALANCE_TRIGGER: Money = Money::from_copper(1_000);
const AGENT_CASH_REBALANCE_BUFFER: Money = Money::from_copper(2_000);
const AGENT_CASH_REBALANCE_INTERVAL_DAYS: i64 = 28;
const AGENT_PLANNED_CAPITALIZATION_INTERVAL_DAYS: i64 = 360;
const AGENT_PLANNED_CAPITALIZATION_MAX: Money = Money::from_copper(8_000);
const AGENT_INFORMATION_COMMISSION_INTERVAL_DAYS: i64 = 720;
const AGENT_INFORMATION_SEVERE_COUNTERPARTY_PRESSURE_BASIS_POINTS: u16 = 1_500;
const AGENT_INFORMATION_LEVERAGE_DELAY_DAYS: i64 = 90;
const INFORMATION_ROUTINE_PAIR_WINDOW_DAYS: i64 = 180;
const AGENT_INFORMATION_MARKET_PRICE_CHANGE_BASIS_POINTS: u64 = 1_000;
const AGENT_INFORMATION_MARKET_SHORTAGE_BASIS_POINTS: u64 = 2_500;
const AGENT_INFORMATION_MARKET_CONTRACT_GAP_BASIS_POINTS: u64 = 500;
const AGENT_INFORMATION_DISTRICT_CONDITION_THRESHOLD: u16 = 4_500;
const AGENT_INFORMATION_DISTRICT_UNREST_THRESHOLD: u16 = 3_500;
const AGENT_INFORMATION_COUNTERPARTY_TRUST_THRESHOLD: u16 = 4_000;
const AGENT_INFORMATION_COUNTERPARTY_RESENTMENT_THRESHOLD: u16 = 2_500;
const SUBSTANTIVE_STREAK_MAX_GAP_DAYS: i64 = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GameplayPersona {
    Steward,
    Entrepreneur,
    PowerBroker,
    Opportunist,
}

impl GameplayPersona {
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [
            Self::Steward,
            Self::Entrepreneur,
            Self::PowerBroker,
            Self::Opportunist,
        ]
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Steward => "steward",
            Self::Entrepreneur => "entrepreneur",
            Self::PowerBroker => "power-broker",
            Self::Opportunist => "opportunist",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayHarnessConfig {
    pub start_seed: u64,
    pub seed_count: u16,
    pub days_per_campaign: u32,
    pub decision_interval_days: u16,
    pub max_candidate_probes: u16,
    pub max_consequence_horizon_days: u16,
    pub trace_limit_per_campaign: u16,
    pub personas: Vec<GameplayPersona>,
    pub backgrounds: Vec<StartingBackground>,
}

impl Default for GameplayHarnessConfig {
    fn default() -> Self {
        Self {
            start_seed: 1,
            seed_count: 1,
            days_per_campaign: 1_080,
            decision_interval_days: 30,
            max_candidate_probes: 24,
            max_consequence_horizon_days: 360,
            trace_limit_per_campaign: 40,
            personas: GameplayPersona::all().to_vec(),
            backgrounds: vec![
                StartingBackground::Baker,
                StartingBackground::ClothTrader,
                StartingBackground::Blacksmith,
            ],
        }
    }
}

impl GameplayHarnessConfig {
    fn validate(&self) -> Result<(), GameplayHarnessError> {
        if self.seed_count == 0 {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "seed_count must be positive".to_owned(),
            });
        }
        if self
            .start_seed
            .checked_add(u64::from(self.seed_count.saturating_sub(1)))
            .is_none()
        {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "configured seed range exceeds u64::MAX".to_owned(),
            });
        }
        if self.days_per_campaign == 0 {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "days_per_campaign must be positive".to_owned(),
            });
        }
        if self.decision_interval_days == 0 {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "decision_interval_days must be positive".to_owned(),
            });
        }
        if self.max_candidate_probes == 0 {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "max_candidate_probes must be positive".to_owned(),
            });
        }
        if self.max_consequence_horizon_days == 0 {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "max_consequence_horizon_days must be positive".to_owned(),
            });
        }
        if self.personas.is_empty() || self.backgrounds.is_empty() {
            return Err(GameplayHarnessError::InvalidConfig {
                reason: "at least one persona and background are required".to_owned(),
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn campaign_count(&self) -> usize {
        usize::from(self.seed_count)
            .saturating_mul(self.personas.len())
            .saturating_mul(self.backgrounds.len())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GameplayCommandKind {
    TransferBusinessCash,
    AcquireBusiness,
    InvestInBusiness,
    SetBusinessPolicy,
    SecureSupply,
    SellOutput,
    BorrowFunds,
    ExtendCredit,
    BuyProperty,
    SellProperty,
    EnactLaw,
    StartPublicWork,
    FileLegalCase,
    SetHouseGovernance,
    ConveneFamilyCouncil,
    DesignateHeir,
    AdoptWard,
    EducateFamilyMember,
    CultivateInstitutionSupport,
    NominateForOffice,
    ExerciseOfficePower,
    WithdrawFromInstitution,
    RespondToCrisis,
    ResolveLaborDispute,
    CommissionInformation,
    LeverageInformation,
    AcknowledgeNotification,
}

impl GameplayCommandKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::TransferBusinessCash => "transfer-cash",
            Self::AcquireBusiness => "acquire-business",
            Self::InvestInBusiness => "invest-business",
            Self::SetBusinessPolicy => "set-policy",
            Self::SecureSupply => "secure-supply",
            Self::SellOutput => "sell-output",
            Self::BorrowFunds => "borrow-funds",
            Self::ExtendCredit => "extend-credit",
            Self::BuyProperty => "buy-property",
            Self::SellProperty => "sell-property",
            Self::EnactLaw => "enact-law",
            Self::StartPublicWork => "public-work",
            Self::FileLegalCase => "legal-case",
            Self::SetHouseGovernance => "house-governance",
            Self::ConveneFamilyCouncil => "family-council",
            Self::DesignateHeir => "designate-heir",
            Self::AdoptWard => "adopt-ward",
            Self::EducateFamilyMember => "family-education",
            Self::CultivateInstitutionSupport => "institution-support",
            Self::NominateForOffice => "office-nomination",
            Self::ExerciseOfficePower => "office-power",
            Self::WithdrawFromInstitution => "institution-withdrawal",
            Self::RespondToCrisis => "crisis-response",
            Self::ResolveLaborDispute => "labor-response",
            Self::CommissionInformation => "commission-intelligence",
            Self::LeverageInformation => "leverage-intelligence",
            Self::AcknowledgeNotification => "acknowledge",
        }
    }

    const fn expected_activation_days(self) -> u32 {
        match self {
            Self::TransferBusinessCash
            | Self::AcquireBusiness
            | Self::InvestInBusiness
            | Self::SecureSupply
            | Self::SellOutput
            | Self::FileLegalCase
            | Self::RespondToCrisis
            | Self::ConveneFamilyCouncil
            | Self::AdoptWard
            | Self::EducateFamilyMember
            | Self::CultivateInstitutionSupport
            | Self::NominateForOffice
            | Self::ExerciseOfficePower
            | Self::LeverageInformation
            | Self::WithdrawFromInstitution => 360,
            Self::DesignateHeir => 7_200,
            Self::ResolveLaborDispute | Self::EnactLaw | Self::StartPublicWork => 720,
            Self::SetBusinessPolicy
            | Self::BorrowFunds
            | Self::ExtendCredit
            | Self::BuyProperty
            | Self::SellProperty
            | Self::SetHouseGovernance
            | Self::CommissionInformation
            | Self::AcknowledgeNotification => 1,
        }
    }

    const fn is_activation_dependent(self) -> bool {
        matches!(
            self,
            Self::TransferBusinessCash
                | Self::AcquireBusiness
                | Self::InvestInBusiness
                | Self::SecureSupply
                | Self::SellOutput
                | Self::FileLegalCase
                | Self::RespondToCrisis
                | Self::ResolveLaborDispute
                | Self::SellProperty
                | Self::BorrowFunds
                | Self::ExtendCredit
                | Self::SetHouseGovernance
                | Self::ConveneFamilyCouncil
                | Self::EnactLaw
                | Self::StartPublicWork
                | Self::ExerciseOfficePower
                | Self::WithdrawFromInstitution
                | Self::CommissionInformation
                | Self::LeverageInformation
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GameplayDomain {
    Economy,
    Business,
    Market,
    Contracts,
    Loans,
    Property,
    Labor,
    Relationships,
    Dynasty,
    Family,
    Institutions,
    Law,
    Districts,
    Legal,
    Crises,
    Information,
    Feedback,
}

impl GameplayDomain {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Economy => "economy",
            Self::Business => "business",
            Self::Market => "market",
            Self::Contracts => "contracts",
            Self::Loans => "loans",
            Self::Property => "property",
            Self::Labor => "labor",
            Self::Relationships => "relationships",
            Self::Dynasty => "dynasty",
            Self::Family => "family",
            Self::Institutions => "institutions",
            Self::Law => "law",
            Self::Districts => "districts",
            Self::Legal => "legal",
            Self::Crises => "crises",
            Self::Information => "information",
            Self::Feedback => "feedback",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayCommandStats {
    pub activation_opportunities: u32,
    pub offered_cycles: u32,
    pub generated: u32,
    pub considered: u32,
    pub viable: u32,
    pub executed: u32,
    pub rejected: u32,
    pub immediate_world_feedback: u32,
    pub delayed_world_feedback: u32,
    pub actions_with_feedback: u32,
    pub actions_with_persistent_consequences: u32,
    pub actions_with_delayed_consequences: u32,
    pub changed_domains: BTreeSet<GameplayDomain>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayInteractionEdge {
    pub command: GameplayCommandKind,
    pub domain: GameplayDomain,
    pub observations: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayCandidateRanking {
    pub command: GameplayCommandKind,
    pub score: i64,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayScores {
    pub actionability: u16,
    pub variety: u16,
    pub interconnection: u16,
    pub feedback: u16,
    pub resilience: u16,
    pub overall: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplaySnapshot {
    pub day: i64,
    pub dynasty_state_checksum: u64,
    pub player_treasury: Money,
    pub player_civic_contributions: Money,
    pub player_unmet_office_duties: u32,
    pub player_business_cash: Money,
    pub active_businesses: u16,
    pub distressed_businesses: u16,
    pub insolvent_businesses: u16,
    pub average_business_condition: u16,
    pub average_business_quality: u16,
    pub business_policy_checksum: i64,
    pub business_state_checksum: u64,
    pub household_state_checksum: u64,
    pub market_price_total: Money,
    pub market_stock_total: Quantity,
    pub market_state_checksum: u64,
    pub external_route_state_checksum: u64,
    pub active_contracts: u16,
    pub fulfilled_contracts: u16,
    pub breached_contracts: u16,
    pub contract_failures: u32,
    pub player_active_contracts: u16,
    pub player_fulfilled_contracts: u16,
    pub player_breached_contracts: u16,
    pub player_contract_failures: u32,
    pub player_contract_deliveries: u32,
    pub contract_state_checksum: u64,
    pub current_loans: u16,
    pub delinquent_loans: u16,
    pub restructured_loans: u16,
    pub defaulted_loans: u16,
    pub repaid_loans: u16,
    pub total_loan_balance: Money,
    pub loan_state_checksum: u64,
    pub current_civic_debts: u16,
    pub delinquent_civic_debts: u16,
    pub defaulted_civic_debts: u16,
    pub repaid_civic_debts: u16,
    pub total_civic_debt_balance: Money,
    pub civic_debt_state_checksum: u64,
    pub player_properties: u16,
    pub player_pledged_properties: u16,
    pub player_collateral_balance: Money,
    pub occupied_properties: u16,
    pub property_state_checksum: u64,
    pub active_employment: u16,
    pub disputed_employment: u16,
    pub player_active_employment: u16,
    pub player_disputed_employment: u16,
    pub average_labor_loyalty: u16,
    pub employment_state_checksum: u64,
    pub average_relationship_trust: u16,
    pub average_relationship_respect: u16,
    pub average_relationship_fear: u16,
    pub average_relationship_resentment: u16,
    pub maximum_contract_relationship_pressure_basis_points: u16,
    pub relationship_obligation_total: i64,
    pub relationship_memory_count: u16,
    pub relationship_state_checksum: u64,
    pub legitimacy: u16,
    pub quality_reputation: u16,
    pub reliability_reputation: u16,
    pub generation: u16,
    pub family_unity: u16,
    pub family_charter_version: u64,
    pub house_governance: HouseGovernance,
    pub offices_held: u16,
    pub available_offices: u16,
    pub eligible_officeholders: u16,
    pub active_wards: u16,
    pub player_family_capability_checksum: u32,
    pub character_state_checksum: u64,
    pub family_state_checksum: u64,
    pub player_office_checksum: i64,
    pub institution_memberships: u16,
    pub player_institutions_represented: u16,
    pub institution_budget_total: Money,
    pub institution_state_checksum: u64,
    pub active_laws: u16,
    pub law_value_checksum: i64,
    pub active_law_checksum: i64,
    pub law_state_checksum: u64,
    pub public_work_progress_total: u32,
    pub building_public_works: u16,
    pub completed_public_works: u16,
    pub suspended_public_works: u16,
    pub player_completed_public_work_checksum: i64,
    pub public_work_state_checksum: u64,
    pub average_food_satisfaction: u16,
    pub minimum_district_food_satisfaction: u16,
    pub average_district_unrest: u16,
    pub district_state_checksum: u64,
    pub open_legal_cases: u16,
    pub decided_legal_cases: u16,
    pub legal_case_state_checksum: u64,
    pub active_crises: u16,
    pub escalated_crises: u16,
    pub resolved_crises: u16,
    pub crisis_severity_total: u32,
    pub crisis_state_checksum: u64,
    pub information_reports: u16,
    pub information_report_checksum: i64,
    pub information_state_checksum: u64,
    pub achieved_ai_objectives: u16,
    pub ai_objective_state_checksum: u64,
    pub unread_notifications: u16,
    pub outbox_messages: u32,
    pub chronicle_entries: u32,
    pub outbox_state_checksum: u64,
    pub chronicle_state_checksum: u64,
    pub audit_state_checksum: u64,
}

#[derive(Debug)]
struct BusinessSnapshotPart {
    player_treasury: Money,
    player_civic_contributions: Money,
    player_unmet_office_duties: u32,
    player_business_cash: Money,
    active_businesses: u16,
    distressed_businesses: u16,
    insolvent_businesses: u16,
    average_business_condition: u16,
    average_business_quality: u16,
    business_policy_checksum: i64,
    market_price_total: Money,
    market_stock_total: Quantity,
}

impl BusinessSnapshotPart {
    fn capture(state: &AppState, player_id: DynastyId) -> Self {
        let player = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist");
        let businesses: Vec<_> = state
            .businesses
            .ids_for_owner(player_id)
            .into_iter()
            .flatten()
            .filter_map(|id| state.businesses.get(*id))
            .collect();
        Self {
            player_treasury: player.treasury(),
            player_civic_contributions: player.civic_contributions(),
            player_unmet_office_duties: player.unmet_office_duties(),
            player_business_cash: businesses.iter().fold(Money::ZERO, |total, business| {
                total.saturating_add(business.cash())
            }),
            active_businesses: count_business_status(&businesses, BusinessStatus::Active),
            distressed_businesses: count_business_status(&businesses, BusinessStatus::Distressed),
            insolvent_businesses: count_business_status(&businesses, BusinessStatus::Insolvent),
            average_business_condition: average_u16(
                businesses
                    .iter()
                    .map(|business| business.operations.condition_basis_points),
            ),
            average_business_quality: average_u16(
                businesses
                    .iter()
                    .map(|business| business.operations.quality_basis_points),
            ),
            business_policy_checksum: businesses.iter().fold(0_i64, |total, business| {
                total
                    .saturating_add(i64::from(business.policy.target_input_days) * 11)
                    .saturating_add(i64::from(business.policy.target_output_days) * 13)
                    .saturating_add(business.policy.minimum_cash_reserve.copper())
                    .saturating_add(i64::from(business.policy.maintenance_basis_points) * 17)
                    .saturating_add(i64::from(business.policy.quality_target_basis_points) * 19)
            }),
            market_price_total: state
                .market
                .quotes
                .values()
                .fold(Money::ZERO, |total, quote| {
                    total.saturating_add(quote.price)
                }),
            market_stock_total: state
                .market
                .quotes
                .values()
                .fold(Quantity::ZERO, |total, quote| {
                    total.saturating_add(quote.stock)
                }),
        }
    }
}

#[derive(Debug)]
struct StrategicSnapshotPart {
    active_contracts: u16,
    fulfilled_contracts: u16,
    breached_contracts: u16,
    contract_failures: u32,
    player_active_contracts: u16,
    player_fulfilled_contracts: u16,
    player_breached_contracts: u16,
    player_contract_failures: u32,
    player_contract_deliveries: u32,
    current_loans: u16,
    delinquent_loans: u16,
    restructured_loans: u16,
    defaulted_loans: u16,
    repaid_loans: u16,
    total_loan_balance: Money,
    civic_debt: CivicDebtSnapshotPart,
    player_properties: u16,
    player_pledged_properties: u16,
    player_collateral_balance: Money,
    occupied_properties: u16,
    active_employment: u16,
    disputed_employment: u16,
    player_active_employment: u16,
    player_disputed_employment: u16,
    average_labor_loyalty: u16,
    average_relationship_trust: u16,
    average_relationship_respect: u16,
    average_relationship_fear: u16,
    average_relationship_resentment: u16,
    maximum_contract_relationship_pressure_basis_points: u16,
    relationship_obligation_total: i64,
    relationship_memory_count: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PropertySnapshotPart {
    player_properties: u16,
    player_pledged_properties: u16,
    player_collateral_balance: Money,
    occupied_properties: u16,
}

impl PropertySnapshotPart {
    fn capture(state: &AppState, player_id: DynastyId) -> Self {
        Self {
            player_properties: usize_to_u16(
                state
                    .properties
                    .values()
                    .filter(|property| property.owner_dynasty_id == Some(player_id))
                    .count(),
            ),
            player_pledged_properties: usize_to_u16(
                state
                    .properties
                    .values()
                    .filter(|property| {
                        property.owner_dynasty_id == Some(player_id)
                            && property.collateral_loan_id.is_some()
                    })
                    .count(),
            ),
            player_collateral_balance: state
                .loans
                .values()
                .filter(|loan| {
                    loan.collateral_property_id.is_some_and(|property_id| {
                        state
                            .properties
                            .get(&property_id)
                            .is_some_and(|property| property.owner_dynasty_id == Some(player_id))
                    })
                })
                .fold(Money::ZERO, |total, loan| {
                    total.saturating_add(loan.balance)
                }),
            occupied_properties: usize_to_u16(
                state
                    .properties
                    .values()
                    .filter(|property| property.occupant_business_id.is_some())
                    .count(),
            ),
        }
    }
}

#[derive(Debug)]
struct CivicDebtSnapshotPart {
    current: u16,
    delinquent: u16,
    defaulted: u16,
    repaid: u16,
    total_balance: Money,
}

impl CivicDebtSnapshotPart {
    fn capture(state: &AppState) -> Self {
        Self {
            current: count_civic_debt_status(state, CivicDebtStatus::Current),
            delinquent: count_civic_debt_status(state, CivicDebtStatus::Delinquent),
            defaulted: count_civic_debt_status(state, CivicDebtStatus::Defaulted),
            repaid: count_civic_debt_status(state, CivicDebtStatus::Repaid),
            total_balance: state.civic_debts.values().fold(Money::ZERO, |total, debt| {
                total.saturating_add(debt.balance)
            }),
        }
    }
}

#[derive(Debug)]
struct RelationshipSnapshotPart {
    average_trust: u16,
    average_respect: u16,
    average_fear: u16,
    average_resentment: u16,
    maximum_contract_pressure: u16,
    obligation_total: i64,
    memory_count: u16,
}

impl RelationshipSnapshotPart {
    fn capture(state: &AppState, player_id: DynastyId) -> Self {
        let relationships: Vec<_> = state
            .relationships
            .values()
            .filter(|relationship| {
                relationship.pair.first == player_id || relationship.pair.second == player_id
            })
            .collect();
        Self {
            average_trust: average_u16(
                relationships
                    .iter()
                    .map(|relationship| relationship.trust_basis_points),
            ),
            average_respect: average_u16(
                relationships
                    .iter()
                    .map(|relationship| relationship.respect_basis_points),
            ),
            average_fear: average_u16(
                relationships
                    .iter()
                    .map(|relationship| relationship.fear_basis_points),
            ),
            average_resentment: average_u16(
                relationships
                    .iter()
                    .map(|relationship| relationship.resentment_basis_points),
            ),
            maximum_contract_pressure: maximum_player_contract_relationship_pressure_basis_points(
                state, player_id,
            ),
            obligation_total: relationships.iter().fold(0_i64, |total, relationship| {
                total.saturating_add(i64::from(relationship.obligation))
            }),
            memory_count: usize_to_u16(
                relationships
                    .iter()
                    .map(|relationship| relationship.memories.len())
                    .sum(),
            ),
        }
    }
}

impl StrategicSnapshotPart {
    fn capture(state: &AppState, player_id: DynastyId) -> Self {
        let player_business_ids: BTreeSet<_> = state
            .businesses
            .ids_for_owner(player_id)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let player_contracts = || {
            state.contracts.values().filter(|contract| {
                player_business_ids.contains(&contract.buyer_business_id)
                    || player_business_ids.contains(&contract.seller_business_id)
            })
        };
        let relationships = RelationshipSnapshotPart::capture(state, player_id);
        let properties = PropertySnapshotPart::capture(state, player_id);
        Self {
            active_contracts: count_contract_status(state, ContractStatus::Active),
            fulfilled_contracts: count_contract_status(state, ContractStatus::Fulfilled),
            breached_contracts: count_contract_status(state, ContractStatus::Breached),
            contract_failures: state
                .contracts
                .values()
                .map(|contract| u32::from(contract.missed_deliveries))
                .sum(),
            player_active_contracts: usize_to_u16(
                player_contracts()
                    .filter(|contract| contract.status == ContractStatus::Active)
                    .count(),
            ),
            player_fulfilled_contracts: usize_to_u16(
                player_contracts()
                    .filter(|contract| contract.status == ContractStatus::Fulfilled)
                    .count(),
            ),
            player_breached_contracts: usize_to_u16(
                player_contracts()
                    .filter(|contract| contract.status == ContractStatus::Breached)
                    .count(),
            ),
            player_contract_failures: player_contracts()
                .map(|contract| u32::from(contract.missed_deliveries))
                .sum(),
            player_contract_deliveries: player_contract_deliveries(state),
            current_loans: count_loan_status(state, LoanStatus::Current),
            delinquent_loans: count_loan_status(state, LoanStatus::Delinquent),
            restructured_loans: count_loan_status(state, LoanStatus::Restructured),
            defaulted_loans: count_loan_status(state, LoanStatus::Defaulted),
            repaid_loans: count_loan_status(state, LoanStatus::Repaid),
            total_loan_balance: state.loans.values().fold(Money::ZERO, |total, loan| {
                total.saturating_add(loan.balance)
            }),
            civic_debt: CivicDebtSnapshotPart::capture(state),
            player_properties: properties.player_properties,
            player_pledged_properties: properties.player_pledged_properties,
            player_collateral_balance: properties.player_collateral_balance,
            occupied_properties: properties.occupied_properties,
            active_employment: count_employment_status(state, EmploymentStatus::Active),
            disputed_employment: count_employment_status(state, EmploymentStatus::Disputed),
            player_active_employment: usize_to_u16(
                state
                    .employment
                    .values()
                    .filter(|agreement| {
                        player_business_ids.contains(&agreement.business_id)
                            && agreement.status == EmploymentStatus::Active
                    })
                    .count(),
            ),
            player_disputed_employment: usize_to_u16(
                state
                    .employment
                    .values()
                    .filter(|agreement| {
                        player_business_ids.contains(&agreement.business_id)
                            && agreement.status == EmploymentStatus::Disputed
                    })
                    .count(),
            ),
            average_labor_loyalty: average_u16(
                state
                    .employment
                    .values()
                    .map(|agreement| agreement.loyalty_basis_points),
            ),
            average_relationship_trust: relationships.average_trust,
            average_relationship_respect: relationships.average_respect,
            average_relationship_fear: relationships.average_fear,
            average_relationship_resentment: relationships.average_resentment,
            maximum_contract_relationship_pressure_basis_points: relationships
                .maximum_contract_pressure,
            relationship_obligation_total: relationships.obligation_total,
            relationship_memory_count: relationships.memory_count,
        }
    }
}

#[derive(Debug)]
struct CivicSnapshotPart {
    legitimacy: u16,
    quality_reputation: u16,
    reliability_reputation: u16,
    generation: u16,
    family_unity: u16,
    family_charter_version: u64,
    house_governance: HouseGovernance,
    offices_held: u16,
    available_offices: u16,
    eligible_officeholders: u16,
    active_wards: u16,
    player_family_capability_checksum: u32,
    player_office_checksum: i64,
    institution_memberships: u16,
    player_institutions_represented: u16,
    institution_budget_total: Money,
    active_laws: u16,
    law_value_checksum: i64,
    active_law_checksum: i64,
    public_work_progress_total: u32,
    building_public_works: u16,
    completed_public_works: u16,
    suspended_public_works: u16,
    player_completed_public_work_checksum: i64,
}

impl CivicSnapshotPart {
    fn capture(state: &AppState, player_id: DynastyId) -> Self {
        let player = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist");
        let council = state
            .family_councils
            .get(&player_id)
            .expect("player family council must exist");
        Self {
            legitimacy: player.resources.legitimacy_basis_points,
            quality_reputation: player.resources.reputation_quality_basis_points,
            reliability_reputation: player.resources.reputation_reliability_basis_points,
            generation: player.runtime.generation,
            family_unity: council.unity_basis_points,
            family_charter_version: council.charter_version,
            house_governance: council.governance,
            offices_held: count_player_offices(state, player_id),
            available_offices: usize_to_u16(state.institutions.len()),
            eligible_officeholders: count_eligible_officeholders(state, player_id),
            active_wards: count_active_player_wards(state, player_id),
            player_family_capability_checksum: player_family_capability_checksum(state, player_id),
            player_office_checksum: state
                .institutions
                .values()
                .filter(|institution| {
                    institution
                        .office_holder_id
                        .and_then(|character_id| state.characters.get(character_id))
                        .is_some_and(|character| character.dynasty_id() == player_id)
                })
                .fold(0_i64, |total, institution| {
                    total.saturating_add(i64::from(institution.institution_id.value()))
                }),
            institution_memberships: count_player_memberships(state, player_id),
            player_institutions_represented: count_player_institutions_represented(
                state, player_id,
            ),
            institution_budget_total: state
                .institutions
                .values()
                .fold(Money::ZERO, |total, institution| {
                    total.saturating_add(institution.budget)
                }),
            active_laws: usize_to_u16(state.laws.values().filter(|law| law.active).count()),
            law_value_checksum: state
                .laws
                .values()
                .filter(|law| law.active)
                .map(|law| law.value)
                .sum(),
            active_law_checksum: state.laws.values().filter(|law| law.active).fold(
                0_i64,
                |total, law| {
                    total
                        .saturating_add((law.kind as i64).saturating_mul(10_007))
                        .saturating_add(law.value)
                },
            ),
            public_work_progress_total: state
                .public_works
                .values()
                .map(|work| u32::from(work.progress_basis_points))
                .sum(),
            building_public_works: usize_to_u16(
                state
                    .public_works
                    .values()
                    .filter(|work| work.status == PublicWorkStatus::Building)
                    .count(),
            ),
            completed_public_works: usize_to_u16(
                state
                    .public_works
                    .values()
                    .filter(|work| work.status == PublicWorkStatus::Completed)
                    .count(),
            ),
            suspended_public_works: usize_to_u16(
                state
                    .public_works
                    .values()
                    .filter(|work| work.status == PublicWorkStatus::Suspended)
                    .count(),
            ),
            player_completed_public_work_checksum: state
                .public_works
                .values()
                .filter(|work| {
                    work.sponsor_dynasty_id == Some(player_id)
                        && work.status == PublicWorkStatus::Completed
                })
                .fold(0_i64, |total, work| {
                    total
                        .saturating_add(i64::from(work.district_id.value()).saturating_mul(101))
                        .saturating_add(work.kind as i64)
                }),
        }
    }
}

#[derive(Debug)]
struct WorldSnapshotPart {
    average_food_satisfaction: u16,
    minimum_district_food_satisfaction: u16,
    average_district_unrest: u16,
    open_legal_cases: u16,
    decided_legal_cases: u16,
    active_crises: u16,
    escalated_crises: u16,
    resolved_crises: u16,
    crisis_severity_total: u32,
    information_reports: u16,
    information_report_checksum: i64,
    achieved_ai_objectives: u16,
    unread_notifications: u16,
    outbox_messages: u32,
    chronicle_entries: u32,
}

impl WorldSnapshotPart {
    fn capture(state: &AppState) -> Self {
        Self {
            average_food_satisfaction:
                crate::core::population_weighted_food_satisfaction_basis_points(
                    state.households.iter(),
                )
                .unwrap_or(0),
            minimum_district_food_satisfaction: minimum_district_food_satisfaction(state),
            average_district_unrest: average_u16(
                state
                    .districts
                    .values()
                    .map(|district| district.unrest_basis_points),
            ),
            open_legal_cases: usize_to_u16(
                state
                    .legal_cases
                    .values()
                    .filter(|case| {
                        matches!(
                            case.status,
                            LegalCaseStatus::Filed | LegalCaseStatus::Hearing
                        )
                    })
                    .count(),
            ),
            decided_legal_cases: usize_to_u16(
                state
                    .legal_cases
                    .values()
                    .filter(|case| {
                        matches!(
                            case.status,
                            LegalCaseStatus::DecidedForPlaintiff
                                | LegalCaseStatus::DecidedForDefendant
                                | LegalCaseStatus::Settled
                        )
                    })
                    .count(),
            ),
            active_crises: usize_to_u16(
                state
                    .crises
                    .values()
                    .filter(|crisis| crisis.status.is_active())
                    .count(),
            ),
            escalated_crises: usize_to_u16(
                state
                    .crises
                    .values()
                    .filter(|crisis| crisis.status == CrisisStatus::Escalated)
                    .count(),
            ),
            resolved_crises: usize_to_u16(
                state
                    .crises
                    .values()
                    .filter(|crisis| crisis.status == CrisisStatus::Resolved)
                    .count(),
            ),
            crisis_severity_total: state
                .crises
                .values()
                .map(|crisis| u32::from(crisis.severity_basis_points))
                .sum(),
            information_reports: usize_to_u16(
                state
                    .information_reports
                    .values()
                    .filter(|report| report.owner_dynasty_id == state.player_dynasty_id)
                    .count(),
            ),
            information_report_checksum: state
                .information_reports
                .values()
                .filter(|report| report.owner_dynasty_id == state.player_dynasty_id)
                .fold(0_i64, |total, report| {
                    total
                        .saturating_add(i64::from(report.id().value()).saturating_mul(101))
                        .saturating_add(report.created_day.saturating_mul(17))
                        .saturating_add(report.expires_day)
                        .saturating_add((report.confidence as i64).saturating_mul(10_007))
                }),
            achieved_ai_objectives: usize_to_u16(
                state
                    .ai_objectives
                    .values()
                    .filter(|objective| objective.status == ObjectiveStatus::Achieved)
                    .count(),
            ),
            unread_notifications: usize_to_u16(
                state
                    .outbox
                    .iter()
                    .filter(|message| !message.acknowledged)
                    .count(),
            ),
            outbox_messages: usize_to_u32(state.outbox.len()),
            chronicle_entries: usize_to_u32(state.chronicle.len()),
        }
    }
}

fn minimum_district_food_satisfaction(state: &AppState) -> u16 {
    state
        .districts
        .keys()
        .filter_map(|district_id| {
            crate::core::population_weighted_food_satisfaction_basis_points(
                state
                    .households
                    .iter()
                    .filter(|household| household.district_id() == *district_id),
            )
        })
        .min()
        .unwrap_or(0)
}

macro_rules! assemble_gameplay_snapshot {
    ($state:ident, $business:ident, $strategic:ident, $civic:ident, $world:ident) => {
        GameplaySnapshot {
            day: $state.clock.day(),
            dynasty_state_checksum: dynasty_state_checksum($state),
            player_treasury: $business.player_treasury,
            player_civic_contributions: $business.player_civic_contributions,
            player_unmet_office_duties: $business.player_unmet_office_duties,
            player_business_cash: $business.player_business_cash,
            active_businesses: $business.active_businesses,
            distressed_businesses: $business.distressed_businesses,
            insolvent_businesses: $business.insolvent_businesses,
            average_business_condition: $business.average_business_condition,
            average_business_quality: $business.average_business_quality,
            business_policy_checksum: $business.business_policy_checksum,
            business_state_checksum: stable_serialized_checksum(&$state.businesses),
            household_state_checksum: stable_serialized_checksum(&$state.households),
            market_price_total: $business.market_price_total,
            market_stock_total: $business.market_stock_total,
            market_state_checksum: stable_serialized_checksum(&$state.market),
            external_route_state_checksum: stable_serialized_checksum(&$state.external_routes),
            active_contracts: $strategic.active_contracts,
            fulfilled_contracts: $strategic.fulfilled_contracts,
            breached_contracts: $strategic.breached_contracts,
            contract_failures: $strategic.contract_failures,
            player_active_contracts: $strategic.player_active_contracts,
            player_fulfilled_contracts: $strategic.player_fulfilled_contracts,
            player_breached_contracts: $strategic.player_breached_contracts,
            player_contract_failures: $strategic.player_contract_failures,
            player_contract_deliveries: $strategic.player_contract_deliveries,
            contract_state_checksum: stable_serialized_checksum(&$state.contracts),
            current_loans: $strategic.current_loans,
            delinquent_loans: $strategic.delinquent_loans,
            restructured_loans: $strategic.restructured_loans,
            defaulted_loans: $strategic.defaulted_loans,
            repaid_loans: $strategic.repaid_loans,
            total_loan_balance: $strategic.total_loan_balance,
            loan_state_checksum: stable_serialized_checksum(&$state.loans),
            current_civic_debts: $strategic.civic_debt.current,
            delinquent_civic_debts: $strategic.civic_debt.delinquent,
            defaulted_civic_debts: $strategic.civic_debt.defaulted,
            repaid_civic_debts: $strategic.civic_debt.repaid,
            total_civic_debt_balance: $strategic.civic_debt.total_balance,
            civic_debt_state_checksum: stable_serialized_checksum(&$state.civic_debts),
            player_properties: $strategic.player_properties,
            player_pledged_properties: $strategic.player_pledged_properties,
            player_collateral_balance: $strategic.player_collateral_balance,
            occupied_properties: $strategic.occupied_properties,
            property_state_checksum: stable_serialized_checksum(&$state.properties),
            active_employment: $strategic.active_employment,
            disputed_employment: $strategic.disputed_employment,
            player_active_employment: $strategic.player_active_employment,
            player_disputed_employment: $strategic.player_disputed_employment,
            average_labor_loyalty: $strategic.average_labor_loyalty,
            employment_state_checksum: stable_serialized_checksum(&$state.employment),
            average_relationship_trust: $strategic.average_relationship_trust,
            average_relationship_respect: $strategic.average_relationship_respect,
            average_relationship_fear: $strategic.average_relationship_fear,
            average_relationship_resentment: $strategic.average_relationship_resentment,
            maximum_contract_relationship_pressure_basis_points: $strategic
                .maximum_contract_relationship_pressure_basis_points,
            relationship_obligation_total: $strategic.relationship_obligation_total,
            relationship_memory_count: $strategic.relationship_memory_count,
            relationship_state_checksum: stable_serialized_checksum(&$state.relationships),
            legitimacy: $civic.legitimacy,
            quality_reputation: $civic.quality_reputation,
            reliability_reputation: $civic.reliability_reputation,
            generation: $civic.generation,
            family_unity: $civic.family_unity,
            family_charter_version: $civic.family_charter_version,
            house_governance: $civic.house_governance,
            offices_held: $civic.offices_held,
            available_offices: $civic.available_offices,
            eligible_officeholders: $civic.eligible_officeholders,
            active_wards: $civic.active_wards,
            player_family_capability_checksum: $civic.player_family_capability_checksum,
            character_state_checksum: stable_serialized_checksum(&$state.characters),
            family_state_checksum: stable_serialized_checksum(&(
                &$state.family_links,
                &$state.family_councils,
            )),
            player_office_checksum: $civic.player_office_checksum,
            institution_memberships: $civic.institution_memberships,
            player_institutions_represented: $civic.player_institutions_represented,
            institution_budget_total: $civic.institution_budget_total,
            institution_state_checksum: stable_serialized_checksum(&$state.institutions),
            active_laws: $civic.active_laws,
            law_value_checksum: $civic.law_value_checksum,
            active_law_checksum: $civic.active_law_checksum,
            law_state_checksum: stable_serialized_checksum(&$state.laws),
            public_work_progress_total: $civic.public_work_progress_total,
            building_public_works: $civic.building_public_works,
            completed_public_works: $civic.completed_public_works,
            suspended_public_works: $civic.suspended_public_works,
            player_completed_public_work_checksum: $civic.player_completed_public_work_checksum,
            public_work_state_checksum: stable_serialized_checksum(&$state.public_works),
            average_food_satisfaction: $world.average_food_satisfaction,
            minimum_district_food_satisfaction: $world.minimum_district_food_satisfaction,
            average_district_unrest: $world.average_district_unrest,
            district_state_checksum: stable_serialized_checksum(&$state.districts),
            open_legal_cases: $world.open_legal_cases,
            decided_legal_cases: $world.decided_legal_cases,
            legal_case_state_checksum: stable_serialized_checksum(&$state.legal_cases),
            active_crises: $world.active_crises,
            escalated_crises: $world.escalated_crises,
            resolved_crises: $world.resolved_crises,
            crisis_severity_total: $world.crisis_severity_total,
            crisis_state_checksum: stable_serialized_checksum(&$state.crises),
            information_reports: $world.information_reports,
            information_report_checksum: $world.information_report_checksum,
            information_state_checksum: stable_serialized_checksum(&$state.information_reports),
            achieved_ai_objectives: $world.achieved_ai_objectives,
            ai_objective_state_checksum: stable_serialized_checksum(&$state.ai_objectives),
            unread_notifications: $world.unread_notifications,
            outbox_messages: $world.outbox_messages,
            chronicle_entries: $world.chronicle_entries,
            outbox_state_checksum: stable_serialized_checksum(&$state.outbox),
            chronicle_state_checksum: stable_serialized_checksum(&$state.chronicle),
            audit_state_checksum: stable_serialized_checksum(&$state.audit_log),
        }
    };
}

impl GameplaySnapshot {
    fn capture(state: &AppState) -> Self {
        let player_id = state.player_dynasty_id;
        let business = BusinessSnapshotPart::capture(state, player_id);
        let strategic = StrategicSnapshotPart::capture(state, player_id);
        let civic = CivicSnapshotPart::capture(state, player_id);
        let world = WorldSnapshotPart::capture(state);
        assemble_gameplay_snapshot!(state, business, strategic, civic, world)
    }

    #[must_use]
    pub fn changed_domains(&self, later: &Self) -> BTreeSet<GameplayDomain> {
        let mut domains = BTreeSet::new();
        compare_economy_and_business(self, later, &mut domains);
        compare_contracts_and_finance(self, later, &mut domains);
        compare_dynasty_and_civic(self, later, &mut domains);
        compare_world_and_information(self, later, &mut domains);
        domains
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayDecisionContext {
    pub player_treasury: Money,
    pub player_business_cash: Money,
    pub active_businesses: u16,
    pub distressed_businesses: u16,
    pub insolvent_businesses: u16,
    pub average_business_condition: u16,
    pub player_contract_deliveries: u32,
    pub current_loans: u16,
    pub delinquent_loans: u16,
    pub restructured_loans: u16,
    pub defaulted_loans: u16,
    pub player_properties: u16,
    pub player_pledged_properties: u16,
    pub player_collateral_balance: Money,
    pub legitimacy: u16,
    pub quality_reputation: u16,
    pub reliability_reputation: u16,
    pub offices_held: u16,
    pub eligible_officeholders: u16,
    pub active_laws: u16,
    pub building_public_works: u16,
    pub suspended_public_works: u16,
    pub active_wards: u16,
    pub family_unity: u16,
    pub generation: u16,
    pub player_disputed_employment: u16,
    pub maximum_contract_relationship_pressure_basis_points: u16,
    pub active_crises: u16,
    pub unread_notifications: u16,
}

impl From<&GameplaySnapshot> for GameplayDecisionContext {
    fn from(snapshot: &GameplaySnapshot) -> Self {
        Self {
            player_treasury: snapshot.player_treasury,
            player_business_cash: snapshot.player_business_cash,
            active_businesses: snapshot.active_businesses,
            distressed_businesses: snapshot.distressed_businesses,
            insolvent_businesses: snapshot.insolvent_businesses,
            average_business_condition: snapshot.average_business_condition,
            player_contract_deliveries: snapshot.player_contract_deliveries,
            current_loans: snapshot.current_loans,
            delinquent_loans: snapshot.delinquent_loans,
            restructured_loans: snapshot.restructured_loans,
            defaulted_loans: snapshot.defaulted_loans,
            player_properties: snapshot.player_properties,
            player_pledged_properties: snapshot.player_pledged_properties,
            player_collateral_balance: snapshot.player_collateral_balance,
            legitimacy: snapshot.legitimacy,
            quality_reputation: snapshot.quality_reputation,
            reliability_reputation: snapshot.reliability_reputation,
            offices_held: snapshot.offices_held,
            eligible_officeholders: snapshot.eligible_officeholders,
            active_laws: snapshot.active_laws,
            building_public_works: snapshot.building_public_works,
            suspended_public_works: snapshot.suspended_public_works,
            active_wards: snapshot.active_wards,
            family_unity: snapshot.family_unity,
            generation: snapshot.generation,
            player_disputed_employment: snapshot.player_disputed_employment,
            maximum_contract_relationship_pressure_basis_points: snapshot
                .maximum_contract_relationship_pressure_basis_points,
            active_crises: snapshot.active_crises,
            unread_notifications: snapshot.unread_notifications,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayViableOption {
    pub command: GameplayCommandKind,
    pub score: i64,
    pub description: String,
    pub immediate_domains: BTreeSet<GameplayDomain>,
    pub projected_domains: BTreeSet<GameplayDomain>,
    pub immediate_history_change: bool,
    pub projected_history_change: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GameplayTraceSignal {
    ImmediateWorldFeedback,
    DelayedWorldFeedback,
    AmbientWorldFeedback,
    PersistentHistoryChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayTraceStep {
    pub day: i64,
    pub context: GameplayDecisionContext,
    pub considered_candidates: u16,
    pub viable_candidates: u16,
    pub substantive_viable_candidates: u16,
    pub viable_command_kinds: BTreeSet<GameplayCommandKind>,
    pub ranked_candidates: Vec<GameplayCandidateRanking>,
    pub viable_options: Vec<GameplayViableOption>,
    pub close_choice_score_gap: Option<i64>,
    pub distinct_immediate_choice_profiles: u16,
    pub distinct_projected_choice_profiles: u16,
    pub selected_command: Option<GameplayCommandKind>,
    pub command_description: Option<String>,
    pub outcome: Option<String>,
    pub rejection_summary: Vec<String>,
    pub immediate_domains: BTreeSet<GameplayDomain>,
    pub delayed_domains: BTreeSet<GameplayDomain>,
    pub persistent_domains: BTreeSet<GameplayDomain>,
    pub ambient_domains: BTreeSet<GameplayDomain>,
    pub signals: BTreeSet<GameplayTraceSignal>,
}

impl GameplayTraceStep {
    fn consequence_breadth(&self) -> usize {
        self.immediate_domains
            .union(&self.delayed_domains)
            .count()
            .saturating_add(usize::from(
                self.signals
                    .contains(&GameplayTraceSignal::PersistentHistoryChange),
            ))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayFantasyArc {
    pub first_reputation_standing_day: Option<i64>,
    pub first_commercial_standing_day: Option<i64>,
    pub first_institution_support_day: Option<i64>,
    pub first_office_campaign_day: Option<i64>,
    pub first_office_day: Option<i64>,
    pub first_city_shaping_action_day: Option<i64>,
    pub first_player_labor_dispute_day: Option<i64>,
    pub first_heir_designation_day: Option<i64>,
    pub first_succession_day: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GameplayPhase {
    Foundation,
    Establishment,
    InstitutionalAscent,
    DynasticGovernance,
    SuccessionLegacy,
}

impl GameplayPhase {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Foundation => "foundation",
            Self::Establishment => "establishment",
            Self::InstitutionalAscent => "institutional-ascent",
            Self::DynasticGovernance => "dynastic-governance",
            Self::SuccessionLegacy => "succession-legacy",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayPhaseStats {
    pub decision_cycles: u32,
    pub substantive_actions: u32,
    pub institutional_campaign_actions: u32,
    pub quiet_cycles: u32,
    pub quiet_cycles_with_ambient_change: u32,
    pub longest_quiet_streak_cycles: u32,
    pub blocked_cycles: u32,
    pub cycles_with_multiple_viable_command_kinds: u32,
    pub cycles_with_close_viable_command_kinds: u32,
    pub cycles_with_distinct_immediate_consequences: u32,
    pub cycles_with_distinct_projected_consequences: u32,
    pub total_viable_choices: u32,
    pub total_viable_command_kinds: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayCampaignReport {
    pub seed: u64,
    pub persona: GameplayPersona,
    pub background: StartingBackground,
    pub simulated_days: u32,
    pub decision_cycles: u32,
    pub cycles_with_viable_choices: u32,
    pub cycles_with_multiple_viable_command_kinds: u32,
    pub cycles_with_close_viable_command_kinds: u32,
    pub cycles_with_distinct_immediate_consequences: u32,
    pub cycles_with_distinct_projected_consequences: u32,
    pub no_action_cycles: u32,
    pub quiet_cycles: u32,
    pub quiet_cycles_with_ambient_change: u32,
    pub blocked_cycles: u32,
    pub total_viable_choices: u32,
    pub total_viable_command_kinds: u32,
    pub phase_stats: BTreeMap<GameplayPhase, GameplayPhaseStats>,
    pub commands: BTreeMap<GameplayCommandKind, GameplayCommandStats>,
    pub rejection_reasons: BTreeMap<String, u32>,
    pub domain_changes: BTreeMap<GameplayDomain, u32>,
    pub causal_domain_changes: BTreeMap<GameplayDomain, u32>,
    pub ambient_domain_changes: BTreeMap<GameplayDomain, u32>,
    pub interactions: Vec<GameplayInteractionEdge>,
    pub start: GameplaySnapshot,
    pub end: GameplaySnapshot,
    pub scores: GameplayScores,
    pub minimum_food_satisfaction: u16,
    pub minimum_district_food_satisfaction: u16,
    pub minimum_operating_businesses: u16,
    pub maximum_disputed_employment: u16,
    pub maximum_player_disputed_employment: u16,
    pub maximum_delinquent_loans: u16,
    pub maximum_defaulted_loans: u16,
    pub maximum_delinquent_civic_debts: u16,
    pub maximum_defaulted_civic_debts: u16,
    pub maximum_offices_held: u16,
    pub maximum_unfinished_public_works: u16,
    pub maximum_active_crises: u16,
    pub maximum_unread_notifications: u16,
    pub maximum_contract_relationship_pressure_basis_points: u16,
    pub minimum_post_succession_family_unity: Option<u16>,
    pub longest_substantive_command_streak: u16,
    pub longest_substantive_streak_command: Option<GameplayCommandKind>,
    pub longest_substantive_action_gap_days: u32,
    pub longest_asset_rich_quiet_gap_days: u32,
    pub longest_recovery_pressure_days: u32,
    pub terminal_recovery_pressure_days: u32,
    pub commission_leverage_pairs: u16,
    pub fantasy_arc: GameplayFantasyArc,
    pub trace: Vec<GameplayTraceStep>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameplayFindingSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayFinding {
    pub severity: GameplayFindingSeverity,
    pub title: String,
    pub evidence: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayAggregate {
    pub campaigns: u32,
    pub simulated_days: u64,
    pub decision_cycles: u64,
    pub successful_actions: u64,
    pub substantive_actions: u64,
    pub candidate_probes: u64,
    pub viable_choices: u64,
    pub viable_command_kinds: u64,
    pub phase_stats: BTreeMap<GameplayPhase, GameplayPhaseStats>,
    pub no_action_cycles: u64,
    pub quiet_cycles: u64,
    pub quiet_cycles_with_ambient_change: u64,
    pub blocked_cycles: u64,
    pub cycles_with_multiple_viable_command_kinds: u64,
    pub cycles_with_close_viable_command_kinds: u64,
    pub cycles_with_distinct_immediate_consequences: u64,
    pub cycles_with_distinct_projected_consequences: u64,
    pub command_coverage: u16,
    pub domain_coverage: u16,
    pub commands: BTreeMap<GameplayCommandKind, GameplayCommandStats>,
    pub rejection_reasons: BTreeMap<String, u32>,
    pub domain_changes: BTreeMap<GameplayDomain, u32>,
    pub causal_domain_changes: BTreeMap<GameplayDomain, u32>,
    pub ambient_domain_changes: BTreeMap<GameplayDomain, u32>,
    pub causal_domain_coverage: u16,
    pub ambient_domain_coverage: u16,
    pub interactions: Vec<GameplayInteractionEdge>,
    pub scores: GameplayScores,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayHarnessReport {
    pub schema_version: u16,
    pub config: GameplayHarnessConfig,
    pub aggregate: GameplayAggregate,
    pub campaigns: Vec<GameplayCampaignReport>,
    pub findings: Vec<GameplayFinding>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Error)]
pub enum GameplayHarnessError {
    #[error("invalid gameplay harness configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error(transparent)]
    NewGame(#[from] NewGameError),
    #[error(transparent)]
    Simulation(#[from] SimulationError),
    #[error("selected command became invalid after a successful probe: {description}: {source}")]
    SelectedCommandRejected {
        description: String,
        #[source]
        source: CommandError,
    },
    #[error("gameplay candidate {description:?} does not map to a player command route")]
    UnclassifiedCandidate { description: String },
    #[error(
        "gameplay candidate {description:?} is labeled {declared:?}, but its player command maps to {actual:?}"
    )]
    CandidateKindMismatch {
        description: String,
        declared: GameplayCommandKind,
        actual: GameplayCommandKind,
    },
}

#[derive(Clone, Debug)]
struct Candidate {
    kind: GameplayCommandKind,
    command: PlayerCommand,
    description: String,
    score: i64,
}

fn classify_player_command(
    state: &AppState,
    command: &PlayerCommand,
) -> Option<GameplayCommandKind> {
    let player_id = state.player_dynasty_id;
    match command {
        PlayerCommand::TransferBusinessCash { .. } => {
            Some(GameplayCommandKind::TransferBusinessCash)
        }
        PlayerCommand::AcquireBusiness { .. } => Some(GameplayCommandKind::AcquireBusiness),
        PlayerCommand::InvestInBusiness { .. } => Some(GameplayCommandKind::InvestInBusiness),
        PlayerCommand::SetBusinessPolicy { .. } => Some(GameplayCommandKind::SetBusinessPolicy),
        PlayerCommand::CreateSupplyContract { terms } => {
            let buyer_is_player = state
                .businesses
                .get(terms.buyer_business_id)
                .is_some_and(|business| business.owner_dynasty_id() == player_id);
            let seller_is_player = state
                .businesses
                .get(terms.seller_business_id)
                .is_some_and(|business| business.owner_dynasty_id() == player_id);
            match (buyer_is_player, seller_is_player) {
                (true, false) => Some(GameplayCommandKind::SecureSupply),
                (false, true) => Some(GameplayCommandKind::SellOutput),
                (false, false) | (true, true) => None,
            }
        }
        PlayerCommand::IssueLoan { terms } => {
            match (
                terms.borrower_dynasty_id == player_id,
                terms.lender_dynasty_id == player_id,
            ) {
                (true, false) => Some(GameplayCommandKind::BorrowFunds),
                (false, true) => Some(GameplayCommandKind::ExtendCredit),
                (false, false) | (true, true) => None,
            }
        }
        PlayerCommand::BuyProperty { .. } => Some(GameplayCommandKind::BuyProperty),
        PlayerCommand::SellProperty { .. } => Some(GameplayCommandKind::SellProperty),
        PlayerCommand::EnactLaw { .. } => Some(GameplayCommandKind::EnactLaw),
        PlayerCommand::StartPublicWork { .. } => Some(GameplayCommandKind::StartPublicWork),
        PlayerCommand::FileLegalCase { .. } => Some(GameplayCommandKind::FileLegalCase),
        PlayerCommand::SetHouseGovernance { .. } => Some(GameplayCommandKind::SetHouseGovernance),
        PlayerCommand::ConveneFamilyCouncil => Some(GameplayCommandKind::ConveneFamilyCouncil),
        PlayerCommand::DesignateHeir { .. } => Some(GameplayCommandKind::DesignateHeir),
        PlayerCommand::AdoptWard { .. } => Some(GameplayCommandKind::AdoptWard),
        PlayerCommand::EducateFamilyMember { .. } => Some(GameplayCommandKind::EducateFamilyMember),
        PlayerCommand::CultivateInstitutionSupport { .. } => {
            Some(GameplayCommandKind::CultivateInstitutionSupport)
        }
        PlayerCommand::NominateForOffice { .. } => Some(GameplayCommandKind::NominateForOffice),
        PlayerCommand::ExerciseOfficePower { .. } => Some(GameplayCommandKind::ExerciseOfficePower),
        PlayerCommand::WithdrawFromInstitution { .. } => {
            Some(GameplayCommandKind::WithdrawFromInstitution)
        }
        PlayerCommand::RespondToCrisis { .. } => Some(GameplayCommandKind::RespondToCrisis),
        PlayerCommand::ResolveLaborDispute { .. } => Some(GameplayCommandKind::ResolveLaborDispute),
        PlayerCommand::CommissionInformation { .. } => {
            Some(GameplayCommandKind::CommissionInformation)
        }
        PlayerCommand::LeverageInformation { .. } => Some(GameplayCommandKind::LeverageInformation),
        PlayerCommand::AcknowledgeNotification { .. } => {
            Some(GameplayCommandKind::AcknowledgeNotification)
        }
    }
}

fn validate_candidate_classifications(
    state: &AppState,
    candidates: &[Candidate],
) -> Result<(), GameplayHarnessError> {
    for candidate in candidates {
        let Some(actual) = classify_player_command(state, &candidate.command) else {
            return Err(GameplayHarnessError::UnclassifiedCandidate {
                description: candidate.description.clone(),
            });
        };
        if actual != candidate.kind {
            return Err(GameplayHarnessError::CandidateKindMismatch {
                description: candidate.description.clone(),
                declared: candidate.kind,
                actual,
            });
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ProbeResult {
    selected: Option<Candidate>,
    viable_count: usize,
    substantive_viable_count: usize,
    viable_command_kinds: BTreeSet<GameplayCommandKind>,
    viable_options: Vec<GameplayViableOption>,
    close_choice_score_gap: Option<i64>,
    distinct_immediate_choice_profiles: usize,
    distinct_projected_choice_profiles: usize,
    rejections: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
struct ChoiceCycleMetrics {
    substantive_candidate_count: usize,
    substantive_viable_count: usize,
    viable_command_kind_count: usize,
    close_choice: bool,
    distinct_immediate_choices: bool,
    distinct_projected_choices: bool,
}

#[derive(Clone, Copy, Debug)]
struct PhaseCycleObservation {
    action: Option<GameplayCommandKind>,
    choices: ChoiceCycleMetrics,
    ambient_change: bool,
}

#[derive(Debug)]
struct CampaignAccumulator {
    commands: BTreeMap<GameplayCommandKind, GameplayCommandStats>,
    phase_stats: BTreeMap<GameplayPhase, GameplayPhaseStats>,
    current_phase_quiet_streaks: BTreeMap<GameplayPhase, u32>,
    rejection_reasons: BTreeMap<String, u32>,
    domain_changes: BTreeMap<GameplayDomain, u32>,
    causal_domain_changes: BTreeMap<GameplayDomain, u32>,
    ambient_domain_changes: BTreeMap<GameplayDomain, u32>,
    interactions: BTreeMap<(GameplayCommandKind, GameplayDomain), u32>,
    trace: Vec<GameplayTraceStep>,
    decision_cycles: u32,
    cycles_with_viable_choices: u32,
    cycles_with_multiple_viable_command_kinds: u32,
    cycles_with_close_viable_command_kinds: u32,
    cycles_with_distinct_immediate_consequences: u32,
    cycles_with_distinct_projected_consequences: u32,
    no_action_cycles: u32,
    quiet_cycles: u32,
    quiet_cycles_with_ambient_change: u32,
    blocked_cycles: u32,
    total_viable_choices: u32,
    total_viable_command_kinds: u32,
    minimum_food_satisfaction: u16,
    minimum_district_food_satisfaction: u16,
    minimum_operating_businesses: u16,
    maximum_disputed_employment: u16,
    maximum_player_disputed_employment: u16,
    maximum_delinquent_loans: u16,
    maximum_defaulted_loans: u16,
    maximum_delinquent_civic_debts: u16,
    maximum_defaulted_civic_debts: u16,
    maximum_offices_held: u16,
    maximum_unfinished_public_works: u16,
    maximum_active_crises: u16,
    maximum_unread_notifications: u16,
    maximum_contract_relationship_pressure_basis_points: u16,
    minimum_post_succession_family_unity: Option<u16>,
    last_command: Option<GameplayCommandKind>,
    last_substantive_command: Option<GameplayCommandKind>,
    last_substantive_command_day: Option<i64>,
    current_substantive_command_streak: u16,
    longest_substantive_command_streak: u16,
    longest_substantive_streak_command: Option<GameplayCommandKind>,
    current_substantive_action_gap_days: u32,
    longest_substantive_action_gap_days: u32,
    current_asset_rich_quiet_gap_days: u32,
    longest_asset_rich_quiet_gap_days: u32,
    current_recovery_pressure_days: u32,
    longest_recovery_pressure_days: u32,
    commission_leverage_pairs: u16,
    last_information_commission_day: Option<i64>,
    starting_generation: Option<u16>,
    fantasy_arc: GameplayFantasyArc,
}

impl CampaignAccumulator {
    fn new() -> Self {
        Self {
            commands: initialized_command_stats(),
            phase_stats: initialized_phase_stats(),
            current_phase_quiet_streaks: initialized_phase_counts(),
            rejection_reasons: BTreeMap::new(),
            domain_changes: initialized_domain_counts(),
            causal_domain_changes: initialized_domain_counts(),
            ambient_domain_changes: initialized_domain_counts(),
            interactions: BTreeMap::new(),
            trace: Vec::new(),
            decision_cycles: 0,
            cycles_with_viable_choices: 0,
            cycles_with_multiple_viable_command_kinds: 0,
            cycles_with_close_viable_command_kinds: 0,
            cycles_with_distinct_immediate_consequences: 0,
            cycles_with_distinct_projected_consequences: 0,
            no_action_cycles: 0,
            quiet_cycles: 0,
            quiet_cycles_with_ambient_change: 0,
            blocked_cycles: 0,
            total_viable_choices: 0,
            total_viable_command_kinds: 0,
            minimum_food_satisfaction: u16::MAX,
            minimum_district_food_satisfaction: u16::MAX,
            minimum_operating_businesses: u16::MAX,
            maximum_disputed_employment: 0,
            maximum_player_disputed_employment: 0,
            maximum_delinquent_loans: 0,
            maximum_defaulted_loans: 0,
            maximum_delinquent_civic_debts: 0,
            maximum_defaulted_civic_debts: 0,
            maximum_offices_held: 0,
            maximum_unfinished_public_works: 0,
            maximum_active_crises: 0,
            maximum_unread_notifications: 0,
            maximum_contract_relationship_pressure_basis_points: 0,
            minimum_post_succession_family_unity: None,
            last_command: None,
            last_substantive_command: None,
            last_substantive_command_day: None,
            current_substantive_command_streak: 0,
            longest_substantive_command_streak: 0,
            longest_substantive_streak_command: None,
            current_substantive_action_gap_days: 0,
            longest_substantive_action_gap_days: 0,
            current_asset_rich_quiet_gap_days: 0,
            longest_asset_rich_quiet_gap_days: 0,
            current_recovery_pressure_days: 0,
            longest_recovery_pressure_days: 0,
            commission_leverage_pairs: 0,
            last_information_commission_day: None,
            starting_generation: None,
            fantasy_arc: GameplayFantasyArc::default(),
        }
    }

    fn record_executed_command(&mut self, kind: GameplayCommandKind, day: i64) {
        self.last_command = Some(kind);
        if kind == GameplayCommandKind::AcknowledgeNotification {
            return;
        }
        if kind == GameplayCommandKind::NominateForOffice {
            self.fantasy_arc
                .first_office_campaign_day
                .get_or_insert(day);
        }
        if kind == GameplayCommandKind::CultivateInstitutionSupport {
            self.fantasy_arc
                .first_institution_support_day
                .get_or_insert(day);
        }
        if kind == GameplayCommandKind::DesignateHeir {
            self.fantasy_arc
                .first_heir_designation_day
                .get_or_insert(day);
        }
        if matches!(
            kind,
            GameplayCommandKind::EnactLaw
                | GameplayCommandKind::StartPublicWork
                | GameplayCommandKind::ExerciseOfficePower
        ) {
            self.fantasy_arc
                .first_city_shaping_action_day
                .get_or_insert(day);
        }
        if kind == GameplayCommandKind::CommissionInformation {
            self.last_information_commission_day = Some(day);
        } else if kind == GameplayCommandKind::LeverageInformation
            && self
                .last_information_commission_day
                .is_some_and(|commission_day| {
                    day.saturating_sub(commission_day) <= INFORMATION_ROUTINE_PAIR_WINDOW_DAYS
                })
        {
            self.commission_leverage_pairs = self.commission_leverage_pairs.saturating_add(1);
            self.last_information_commission_day = None;
        }
        let follows_recent_same_command = self.last_substantive_command == Some(kind)
            && self
                .last_substantive_command_day
                .is_some_and(|previous_day| {
                    day.saturating_sub(previous_day) <= SUBSTANTIVE_STREAK_MAX_GAP_DAYS
                });
        if follows_recent_same_command {
            self.current_substantive_command_streak =
                self.current_substantive_command_streak.saturating_add(1);
        } else {
            self.last_substantive_command = Some(kind);
            self.current_substantive_command_streak = 1;
        }
        self.last_substantive_command_day = Some(day);
        if self.current_substantive_command_streak > self.longest_substantive_command_streak {
            self.longest_substantive_command_streak = self.current_substantive_command_streak;
            self.longest_substantive_streak_command = Some(kind);
        }
    }

    fn record_action_gap(
        &mut self,
        action: Option<GameplayCommandKind>,
        step_days: u32,
        snapshot: &GameplaySnapshot,
    ) {
        if action.is_some_and(|kind| kind != GameplayCommandKind::AcknowledgeNotification) {
            self.current_substantive_action_gap_days = 0;
            self.current_asset_rich_quiet_gap_days = 0;
            return;
        }
        self.current_substantive_action_gap_days = self
            .current_substantive_action_gap_days
            .saturating_add(step_days);
        self.longest_substantive_action_gap_days = self
            .longest_substantive_action_gap_days
            .max(self.current_substantive_action_gap_days);
        let asset_rich_and_cash_poor = snapshot.player_treasury < Money::from_copper(4_000)
            && snapshot.player_properties >= 2
            && snapshot.player_business_cash >= Money::from_copper(10_000)
            && snapshot.active_businesses > 0;
        if asset_rich_and_cash_poor {
            self.current_asset_rich_quiet_gap_days = self
                .current_asset_rich_quiet_gap_days
                .saturating_add(step_days);
            self.longest_asset_rich_quiet_gap_days = self
                .longest_asset_rich_quiet_gap_days
                .max(self.current_asset_rich_quiet_gap_days);
        } else {
            self.current_asset_rich_quiet_gap_days = 0;
        }
    }

    fn record_recovery_pressure(&mut self, step_days: u32, snapshot: &GameplaySnapshot) {
        let under_recovery_pressure = snapshot.player_treasury <= Money::ZERO
            && snapshot.active_businesses == 0
            && snapshot
                .distressed_businesses
                .saturating_add(snapshot.insolvent_businesses)
                > 0
            && snapshot.player_properties == 0
            && snapshot.defaulted_loans > 0;
        if under_recovery_pressure {
            self.current_recovery_pressure_days = self
                .current_recovery_pressure_days
                .saturating_add(step_days);
            self.longest_recovery_pressure_days = self
                .longest_recovery_pressure_days
                .max(self.current_recovery_pressure_days);
        } else {
            self.current_recovery_pressure_days = 0;
        }
    }

    fn record_phase_cycle(&mut self, phase: GameplayPhase, observation: PhaseCycleObservation) {
        let PhaseCycleObservation {
            action,
            choices,
            ambient_change,
        } = observation;
        let quiet_cycle = action
            .is_none_or(|kind| kind == GameplayCommandKind::AcknowledgeNotification)
            && choices.substantive_viable_count == 0
            && choices.substantive_candidate_count == 0;
        let current_quiet_streak = self
            .current_phase_quiet_streaks
            .get_mut(&phase)
            .expect("every gameplay phase must have quiet-streak state");
        if quiet_cycle {
            *current_quiet_streak = current_quiet_streak.saturating_add(1);
        } else {
            *current_quiet_streak = 0;
        }
        let stats = self
            .phase_stats
            .get_mut(&phase)
            .expect("every gameplay phase must have statistics");
        stats.decision_cycles = stats.decision_cycles.saturating_add(1);
        stats.total_viable_command_kinds = stats
            .total_viable_command_kinds
            .saturating_add(usize_to_u32(choices.viable_command_kind_count));
        stats.total_viable_choices = stats
            .total_viable_choices
            .saturating_add(usize_to_u32(choices.substantive_viable_count));
        if action.is_some_and(|kind| kind != GameplayCommandKind::AcknowledgeNotification) {
            stats.substantive_actions = stats.substantive_actions.saturating_add(1);
            if action.is_some_and(|kind| {
                matches!(
                    kind,
                    GameplayCommandKind::CultivateInstitutionSupport
                        | GameplayCommandKind::NominateForOffice
                )
            }) {
                stats.institutional_campaign_actions =
                    stats.institutional_campaign_actions.saturating_add(1);
            }
        } else if choices.substantive_viable_count == 0 {
            if choices.substantive_candidate_count == 0 {
                stats.quiet_cycles = stats.quiet_cycles.saturating_add(1);
                stats.longest_quiet_streak_cycles =
                    stats.longest_quiet_streak_cycles.max(*current_quiet_streak);
                if ambient_change {
                    stats.quiet_cycles_with_ambient_change =
                        stats.quiet_cycles_with_ambient_change.saturating_add(1);
                    self.quiet_cycles_with_ambient_change =
                        self.quiet_cycles_with_ambient_change.saturating_add(1);
                }
            } else {
                stats.blocked_cycles = stats.blocked_cycles.saturating_add(1);
            }
        }
        if choices.viable_command_kind_count >= 2 {
            stats.cycles_with_multiple_viable_command_kinds = stats
                .cycles_with_multiple_viable_command_kinds
                .saturating_add(1);
        }
        if choices.close_choice {
            stats.cycles_with_close_viable_command_kinds = stats
                .cycles_with_close_viable_command_kinds
                .saturating_add(1);
        }
        if choices.distinct_immediate_choices {
            stats.cycles_with_distinct_immediate_consequences = stats
                .cycles_with_distinct_immediate_consequences
                .saturating_add(1);
        }
        if choices.distinct_projected_choices {
            stats.cycles_with_distinct_projected_consequences = stats
                .cycles_with_distinct_projected_consequences
                .saturating_add(1);
        }
    }

    fn observe_initial_snapshot(&mut self, snapshot: &GameplaySnapshot) {
        self.starting_generation = Some(snapshot.generation);
        self.observe_fantasy_arc(snapshot);
        self.observe_non_food_snapshot(snapshot);
    }

    fn observe_snapshot(&mut self, snapshot: &GameplaySnapshot) {
        self.minimum_food_satisfaction = self
            .minimum_food_satisfaction
            .min(snapshot.average_food_satisfaction);
        self.minimum_district_food_satisfaction = self
            .minimum_district_food_satisfaction
            .min(snapshot.minimum_district_food_satisfaction);
        self.observe_fantasy_arc(snapshot);
        self.observe_non_food_snapshot(snapshot);
    }

    fn observe_fantasy_arc(&mut self, snapshot: &GameplaySnapshot) {
        let has_reputation = snapshot
            .quality_reputation
            .max(snapshot.reliability_reputation)
            >= COMMERCIAL_STANDING_REPUTATION_REQUIREMENT;
        if has_reputation {
            self.fantasy_arc
                .first_reputation_standing_day
                .get_or_insert(snapshot.day);
        }
        if has_reputation
            && snapshot.player_contract_deliveries >= OFFICE_NOMINATION_DELIVERY_REQUIREMENT
        {
            self.fantasy_arc
                .first_commercial_standing_day
                .get_or_insert(snapshot.day);
        }
        if snapshot.offices_held > 0 {
            self.fantasy_arc
                .first_office_day
                .get_or_insert(snapshot.day);
        }
        if snapshot.player_disputed_employment > 0 {
            self.fantasy_arc
                .first_player_labor_dispute_day
                .get_or_insert(snapshot.day);
        }
        if self
            .starting_generation
            .is_some_and(|generation| snapshot.generation > generation)
        {
            self.fantasy_arc
                .first_succession_day
                .get_or_insert(snapshot.day);
            self.minimum_post_succession_family_unity = Some(
                self.minimum_post_succession_family_unity
                    .map_or(snapshot.family_unity, |minimum| {
                        minimum.min(snapshot.family_unity)
                    }),
            );
        }
    }

    fn observe_non_food_snapshot(&mut self, snapshot: &GameplaySnapshot) {
        self.minimum_operating_businesses = self.minimum_operating_businesses.min(
            snapshot
                .active_businesses
                .saturating_add(snapshot.distressed_businesses),
        );
        self.maximum_disputed_employment = self
            .maximum_disputed_employment
            .max(snapshot.disputed_employment);
        self.maximum_player_disputed_employment = self
            .maximum_player_disputed_employment
            .max(snapshot.player_disputed_employment);
        self.maximum_delinquent_loans =
            self.maximum_delinquent_loans.max(snapshot.delinquent_loans);
        self.maximum_defaulted_loans = self.maximum_defaulted_loans.max(snapshot.defaulted_loans);
        self.maximum_delinquent_civic_debts = self
            .maximum_delinquent_civic_debts
            .max(snapshot.delinquent_civic_debts);
        self.maximum_defaulted_civic_debts = self
            .maximum_defaulted_civic_debts
            .max(snapshot.defaulted_civic_debts);
        self.maximum_offices_held = self.maximum_offices_held.max(snapshot.offices_held);
        self.maximum_unfinished_public_works = self.maximum_unfinished_public_works.max(
            snapshot
                .building_public_works
                .saturating_add(snapshot.suspended_public_works),
        );
        self.maximum_active_crises = self.maximum_active_crises.max(snapshot.active_crises);
        self.maximum_unread_notifications = self
            .maximum_unread_notifications
            .max(snapshot.unread_notifications);
        self.maximum_contract_relationship_pressure_basis_points = self
            .maximum_contract_relationship_pressure_basis_points
            .max(snapshot.maximum_contract_relationship_pressure_basis_points);
    }
}

/// Runs deterministic player agents across configured backgrounds, personas, and seeds.
///
/// Each agent derives legal candidates from visible campaign state, probes those candidates through
/// the canonical command API on cloned state, commits the highest-ranked viable action, advances
/// through the canonical simulation pipeline, and records immediate and delayed system reactions.
///
/// # Errors
///
/// Returns an error for invalid configuration, campaign creation failure, simulation failure, or a
/// command that unexpectedly fails after succeeding against an identical probe state.
pub fn run_gameplay_harness(
    registry: &Registry,
    config: GameplayHarnessConfig,
) -> Result<GameplayHarnessReport, GameplayHarnessError> {
    config.validate()?;
    let mut campaigns = Vec::with_capacity(config.campaign_count());
    for seed_offset in 0..config.seed_count {
        let seed = config
            .start_seed
            .checked_add(u64::from(seed_offset))
            .ok_or_else(|| GameplayHarnessError::InvalidConfig {
                reason: "configured seed range exceeds u64::MAX".to_owned(),
            })?;
        for background in &config.backgrounds {
            for persona in &config.personas {
                campaigns.push(run_campaign(
                    registry,
                    &config,
                    seed,
                    *background,
                    *persona,
                )?);
            }
        }
    }
    let aggregate = aggregate_campaigns(&campaigns);
    let findings = derive_findings(&aggregate, &campaigns);
    Ok(GameplayHarnessReport {
        schema_version: GAMEPLAY_REPORT_SCHEMA_VERSION,
        config,
        aggregate,
        campaigns,
        findings,
        limitations: gameplay_harness_limitations(),
    })
}

fn gameplay_harness_limitations() -> Vec<String> {
    vec![
        "Automated agents measure reachability and systemic outcomes, not whether a human understands the interface or enjoys the decisions.".to_owned(),
        "The report cannot measure emotional investment, narrative quality, or the cognitive burden of comparing choices.".to_owned(),
        "Agents inspect authoritative simulation state when choosing what to investigate; commissioned reports can unlock traceable follow-up actions, but the harness does not measure whether a human can interpret the report or identify the best use.".to_owned(),
        "Alternative-choice profiles compare immediate effects and one decision interval of projected divergence; the harness does not advance every unchosen branch through its full delayed consequence horizon.".to_owned(),
        "Deterministic personas follow fixed priorities and do not model experimentation, misunderstanding, changing preferences, or interface friction.".to_owned(),
        "Stress personas can prove that risky legal, labor, and financial routes exist, but they cannot prove that those risks are legible or attractive to a human player.".to_owned(),
        "AI-objective progress measures rival activity, but the harness cannot prove that a human recognizes which house caused a setback or understands that rival's intent.".to_owned(),
        "Counterfactual attribution can only detect consequences represented by the report snapshot and configured consequence horizon.".to_owned(),
        "Persistent state and chronicle changes approximate historical imprint; the harness cannot judge whether the game presents that legacy as a coherent remembered story.".to_owned(),
    ]
}

fn run_campaign(
    registry: &Registry,
    config: &GameplayHarnessConfig,
    seed: u64,
    background: StartingBackground,
    persona: GameplayPersona,
) -> Result<GameplayCampaignReport, GameplayHarnessError> {
    let mut state = build_new_game(
        registry,
        NewGameConfig {
            seed,
            dynasty_name: format!("Harness-{}-{seed}", persona.label()),
            founder_name: format!("Agent {}", persona.label()),
            background,
        },
    )?;
    let start = GameplaySnapshot::capture(&state);
    let mut accumulator = CampaignAccumulator::new();
    accumulator.observe_initial_snapshot(&start);
    let mut remaining = config.days_per_campaign;
    while remaining > 0 {
        let step_days = u32::from(config.decision_interval_days).min(remaining);
        run_decision_cycle(
            registry,
            config,
            persona,
            &mut state,
            step_days,
            &mut accumulator,
        )?;
        remaining = remaining.saturating_sub(step_days);
    }
    if terminal_phase_needs_decision(&accumulator) {
        run_terminal_decision_cycle(registry, config, persona, &mut state, &mut accumulator)?;
    }
    validate_invariants(registry, &state);
    let end = GameplaySnapshot::capture(&state);
    let scores = score_campaign(&accumulator, &start, &end);
    let interactions = interaction_vec(&accumulator.interactions);
    let trace = select_trace(
        accumulator.trace,
        usize::from(config.trace_limit_per_campaign),
    );
    Ok(GameplayCampaignReport {
        seed,
        persona,
        background,
        simulated_days: config.days_per_campaign,
        decision_cycles: accumulator.decision_cycles,
        cycles_with_viable_choices: accumulator.cycles_with_viable_choices,
        cycles_with_multiple_viable_command_kinds: accumulator
            .cycles_with_multiple_viable_command_kinds,
        cycles_with_close_viable_command_kinds: accumulator.cycles_with_close_viable_command_kinds,
        cycles_with_distinct_immediate_consequences: accumulator
            .cycles_with_distinct_immediate_consequences,
        cycles_with_distinct_projected_consequences: accumulator
            .cycles_with_distinct_projected_consequences,
        no_action_cycles: accumulator.no_action_cycles,
        quiet_cycles: accumulator.quiet_cycles,
        quiet_cycles_with_ambient_change: accumulator.quiet_cycles_with_ambient_change,
        blocked_cycles: accumulator.blocked_cycles,
        total_viable_choices: accumulator.total_viable_choices,
        total_viable_command_kinds: accumulator.total_viable_command_kinds,
        phase_stats: accumulator.phase_stats,
        commands: accumulator.commands,
        rejection_reasons: accumulator.rejection_reasons,
        domain_changes: accumulator.domain_changes,
        causal_domain_changes: accumulator.causal_domain_changes,
        ambient_domain_changes: accumulator.ambient_domain_changes,
        interactions,
        start,
        end,
        scores,
        minimum_food_satisfaction: accumulator.minimum_food_satisfaction,
        minimum_district_food_satisfaction: accumulator.minimum_district_food_satisfaction,
        minimum_operating_businesses: accumulator.minimum_operating_businesses,
        maximum_disputed_employment: accumulator.maximum_disputed_employment,
        maximum_player_disputed_employment: accumulator.maximum_player_disputed_employment,
        maximum_delinquent_loans: accumulator.maximum_delinquent_loans,
        maximum_defaulted_loans: accumulator.maximum_defaulted_loans,
        maximum_delinquent_civic_debts: accumulator.maximum_delinquent_civic_debts,
        maximum_defaulted_civic_debts: accumulator.maximum_defaulted_civic_debts,
        maximum_offices_held: accumulator.maximum_offices_held,
        maximum_unfinished_public_works: accumulator.maximum_unfinished_public_works,
        maximum_active_crises: accumulator.maximum_active_crises,
        maximum_unread_notifications: accumulator.maximum_unread_notifications,
        maximum_contract_relationship_pressure_basis_points: accumulator
            .maximum_contract_relationship_pressure_basis_points,
        minimum_post_succession_family_unity: accumulator.minimum_post_succession_family_unity,
        longest_substantive_command_streak: accumulator.longest_substantive_command_streak,
        longest_substantive_streak_command: accumulator.longest_substantive_streak_command,
        longest_substantive_action_gap_days: accumulator.longest_substantive_action_gap_days,
        longest_asset_rich_quiet_gap_days: accumulator.longest_asset_rich_quiet_gap_days,
        longest_recovery_pressure_days: accumulator.longest_recovery_pressure_days,
        terminal_recovery_pressure_days: accumulator.current_recovery_pressure_days,
        commission_leverage_pairs: accumulator.commission_leverage_pairs,
        fantasy_arc: accumulator.fantasy_arc,
        trace,
    })
}

fn terminal_phase_needs_decision(accumulator: &CampaignAccumulator) -> bool {
    accumulator
        .phase_stats
        .get(&gameplay_phase(&accumulator.fantasy_arc))
        .is_some_and(|stats| stats.decision_cycles == 0)
}

#[derive(Clone, Copy)]
enum DecisionCycleMode {
    AdvanceCampaign { step_days: u32 },
    Terminal,
}

fn run_terminal_decision_cycle(
    registry: &Registry,
    config: &GameplayHarnessConfig,
    persona: GameplayPersona,
    state: &mut AppState,
    accumulator: &mut CampaignAccumulator,
) -> Result<(), GameplayHarnessError> {
    run_decision_cycle_internal(
        registry,
        config,
        persona,
        state,
        DecisionCycleMode::Terminal,
        accumulator,
    )
}

fn run_decision_cycle(
    registry: &Registry,
    config: &GameplayHarnessConfig,
    persona: GameplayPersona,
    state: &mut AppState,
    step_days: u32,
    accumulator: &mut CampaignAccumulator,
) -> Result<(), GameplayHarnessError> {
    run_decision_cycle_internal(
        registry,
        config,
        persona,
        state,
        DecisionCycleMode::AdvanceCampaign { step_days },
        accumulator,
    )
}

fn run_decision_cycle_internal(
    registry: &Registry,
    config: &GameplayHarnessConfig,
    persona: GameplayPersona,
    state: &mut AppState,
    mode: DecisionCycleMode,
    accumulator: &mut CampaignAccumulator,
) -> Result<(), GameplayHarnessError> {
    accumulator.decision_cycles = accumulator.decision_cycles.saturating_add(1);
    apply_notification_housekeeping(registry, state, accumulator)?;
    let phase = gameplay_phase(&accumulator.fantasy_arc);
    let mut baseline_state = state.clone();
    let before = GameplaySnapshot::capture(state);
    record_activation_opportunities(registry, state, persona, accumulator);
    let candidates = ranked_candidates(registry, state, persona, accumulator);
    validate_candidate_classifications(state, &candidates)?;
    let ranked_candidates = summarize_ranked_candidates(&candidates);
    let substantive_candidate_count = candidates
        .iter()
        .filter(|candidate| candidate.kind != GameplayCommandKind::AcknowledgeNotification)
        .count();
    record_offered_command_kinds(&candidates, accumulator);
    record_generated_candidates(&candidates, accumulator);
    let candidates_to_probe =
        select_probe_candidates(candidates, usize::from(config.max_candidate_probes));
    let probe_limit = candidates_to_probe.len();
    let projection_step_days = match mode {
        DecisionCycleMode::AdvanceCampaign { step_days } => step_days,
        DecisionCycleMode::Terminal => u32::from(config.decision_interval_days),
    };
    let probe = probe_candidates(
        registry,
        state,
        candidates_to_probe.into_iter(),
        projection_step_days,
        accumulator,
    )?;
    let choice_metrics =
        record_choice_cycle_metrics(accumulator, substantive_candidate_count, &probe);
    let action = apply_selected_candidate(registry, state, probe.selected, accumulator)?;
    let action_kind = action.as_ref().map(|action| action.kind);
    let action_gap_days = match mode {
        DecisionCycleMode::AdvanceCampaign { step_days } => step_days,
        DecisionCycleMode::Terminal => 0,
    };
    accumulator.record_action_gap(action_kind, action_gap_days, &before);
    let after_command = GameplaySnapshot::capture(state);
    let consequence_horizon = consequence_horizon_days(
        action.as_ref().map(|action| action.kind),
        projection_step_days,
        config.max_consequence_horizon_days,
    );
    let after_time = match mode {
        DecisionCycleMode::AdvanceCampaign { step_days } => {
            let mut consequence_state = (consequence_horizon > step_days).then(|| state.clone());
            advance_days(registry, state, step_days)?;
            let campaign_after_time = GameplaySnapshot::capture(state);
            accumulator.observe_snapshot(&campaign_after_time);
            accumulator.record_recovery_pressure(step_days, &campaign_after_time);
            if let Some(consequence_state) = consequence_state.as_mut() {
                advance_days(registry, consequence_state, consequence_horizon)?;
                GameplaySnapshot::capture(consequence_state)
            } else {
                campaign_after_time
            }
        }
        DecisionCycleMode::Terminal => {
            accumulator.observe_snapshot(&after_command);
            let mut consequence_state = state.clone();
            advance_days(registry, &mut consequence_state, consequence_horizon)?;
            GameplaySnapshot::capture(&consequence_state)
        }
    };
    advance_days(registry, &mut baseline_state, consequence_horizon)?;
    let baseline_after_time = GameplaySnapshot::capture(&baseline_state);
    let ambient_change = !before.changed_domains(&baseline_after_time).is_empty()
        || baseline_after_time.outbox_messages > before.outbox_messages
        || baseline_after_time.chronicle_entries > before.chronicle_entries;
    accumulator.record_phase_cycle(
        phase,
        PhaseCycleObservation {
            action: action_kind,
            choices: choice_metrics,
            ambient_change,
        },
    );
    record_cycle(
        CycleObservation {
            before: &before,
            after_command: &after_command,
            after_time: &after_time,
            baseline_after_time: &baseline_after_time,
            considered: probe_limit,
            viable: probe.viable_count,
            substantive_viable: probe.substantive_viable_count,
            viable_command_kinds: probe.viable_command_kinds,
            ranked_candidates,
            viable_options: probe.viable_options,
            close_choice_score_gap: probe.close_choice_score_gap,
            distinct_immediate_choice_profiles: probe.distinct_immediate_choice_profiles,
            distinct_projected_choice_profiles: probe.distinct_projected_choice_profiles,
            rejections: probe.rejections,
            action,
        },
        accumulator,
    );
    Ok(())
}

fn record_choice_cycle_metrics(
    accumulator: &mut CampaignAccumulator,
    substantive_candidate_count: usize,
    probe: &ProbeResult,
) -> ChoiceCycleMetrics {
    let close_choice = probe
        .close_choice_score_gap
        .is_some_and(|gap| gap <= CLOSE_CHOICE_SCORE_GAP);
    let distinct_immediate_choices = probe.distinct_immediate_choice_profiles >= 2;
    let distinct_projected_choices = probe.distinct_projected_choice_profiles >= 2;
    accumulator.total_viable_choices = accumulator
        .total_viable_choices
        .saturating_add(usize_to_u32(probe.substantive_viable_count));
    accumulator.total_viable_command_kinds = accumulator
        .total_viable_command_kinds
        .saturating_add(usize_to_u32(probe.viable_command_kinds.len()));
    if probe.substantive_viable_count > 0 {
        accumulator.cycles_with_viable_choices =
            accumulator.cycles_with_viable_choices.saturating_add(1);
    } else {
        accumulator.no_action_cycles = accumulator.no_action_cycles.saturating_add(1);
        if substantive_candidate_count == 0 {
            accumulator.quiet_cycles = accumulator.quiet_cycles.saturating_add(1);
        } else {
            accumulator.blocked_cycles = accumulator.blocked_cycles.saturating_add(1);
        }
    }
    if probe.viable_command_kinds.len() >= 2 {
        accumulator.cycles_with_multiple_viable_command_kinds = accumulator
            .cycles_with_multiple_viable_command_kinds
            .saturating_add(1);
    }
    if close_choice {
        accumulator.cycles_with_close_viable_command_kinds = accumulator
            .cycles_with_close_viable_command_kinds
            .saturating_add(1);
    }
    if distinct_immediate_choices {
        accumulator.cycles_with_distinct_immediate_consequences = accumulator
            .cycles_with_distinct_immediate_consequences
            .saturating_add(1);
    }
    if distinct_projected_choices {
        accumulator.cycles_with_distinct_projected_consequences = accumulator
            .cycles_with_distinct_projected_consequences
            .saturating_add(1);
    }
    ChoiceCycleMetrics {
        substantive_candidate_count,
        substantive_viable_count: probe.substantive_viable_count,
        viable_command_kind_count: probe.viable_command_kinds.len(),
        close_choice,
        distinct_immediate_choices,
        distinct_projected_choices,
    }
}

fn apply_notification_housekeeping(
    registry: &Registry,
    state: &mut AppState,
    accumulator: &mut CampaignAccumulator,
) -> Result<(), GameplayHarnessError> {
    let unread = state
        .outbox
        .iter()
        .filter(|message| !message.acknowledged)
        .count();
    if unread < NOTIFICATION_BATCH_THRESHOLD {
        return Ok(());
    }
    let message_id = state
        .outbox
        .iter()
        .rev()
        .find(|message| !message.acknowledged)
        .expect("an unread backlog must contain a latest message")
        .id;
    apply_player_command(
        registry,
        state,
        PlayerCommand::AcknowledgeNotification { message_id },
    )
    .map_err(|source| GameplayHarnessError::SelectedCommandRejected {
        description: format!(
            "acknowledge {unread} notifications through notification {message_id}"
        ),
        source,
    })?;
    let command_stats = accumulator
        .commands
        .get_mut(&GameplayCommandKind::AcknowledgeNotification)
        .expect("acknowledgement statistics must exist");
    command_stats.offered_cycles = command_stats.offered_cycles.saturating_add(1);
    command_stats.generated = command_stats.generated.saturating_add(1);
    command_stats.considered = command_stats.considered.saturating_add(1);
    command_stats.viable = command_stats.viable.saturating_add(1);
    command_stats.executed = command_stats.executed.saturating_add(1);
    command_stats.immediate_world_feedback =
        command_stats.immediate_world_feedback.saturating_add(1);
    command_stats.actions_with_feedback = command_stats.actions_with_feedback.saturating_add(1);
    command_stats.actions_with_persistent_consequences = command_stats
        .actions_with_persistent_consequences
        .saturating_add(1);
    command_stats
        .changed_domains
        .insert(GameplayDomain::Feedback);
    accumulator.record_executed_command(
        GameplayCommandKind::AcknowledgeNotification,
        state.clock.day(),
    );
    Ok(())
}

fn gameplay_phase(arc: &GameplayFantasyArc) -> GameplayPhase {
    if arc.first_succession_day.is_some() {
        GameplayPhase::SuccessionLegacy
    } else if arc.first_city_shaping_action_day.is_some() {
        GameplayPhase::DynasticGovernance
    } else if arc.first_commercial_standing_day.is_some() {
        GameplayPhase::InstitutionalAscent
    } else if arc.first_reputation_standing_day.is_some() {
        GameplayPhase::Establishment
    } else {
        GameplayPhase::Foundation
    }
}

fn consequence_horizon_days(
    command: Option<GameplayCommandKind>,
    step_days: u32,
    maximum: u16,
) -> u32 {
    let desired = match command {
        Some(
            GameplayCommandKind::SetHouseGovernance
            | GameplayCommandKind::ConveneFamilyCouncil
            | GameplayCommandKind::DesignateHeir
            | GameplayCommandKind::AdoptWard
            | GameplayCommandKind::EducateFamilyMember
            | GameplayCommandKind::StartPublicWork,
        ) => 360,
        Some(GameplayCommandKind::NominateForOffice) => 120,
        Some(
            GameplayCommandKind::CultivateInstitutionSupport
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::EnactLaw
            | GameplayCommandKind::ExerciseOfficePower
            | GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::WithdrawFromInstitution,
        ) => 180,
        Some(
            GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::SecureSupply
            | GameplayCommandKind::SellOutput
            | GameplayCommandKind::BorrowFunds
            | GameplayCommandKind::ExtendCredit
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::SellProperty
            | GameplayCommandKind::CommissionInformation
            | GameplayCommandKind::LeverageInformation,
        ) => 30,
        Some(
            GameplayCommandKind::ResolveLaborDispute | GameplayCommandKind::AcknowledgeNotification,
        )
        | None => step_days,
    };
    desired.min(u32::from(maximum)).max(step_days)
}

fn select_probe_candidates(candidates: Vec<Candidate>, limit: usize) -> Vec<Candidate> {
    if limit == 0 {
        return Vec::new();
    }
    let mut seen_kinds = BTreeSet::new();
    let mut family_leaders = Vec::new();
    let mut additional_variants = Vec::new();
    for candidate in candidates {
        if seen_kinds.insert(candidate.kind) {
            family_leaders.push(candidate);
        } else {
            additional_variants.push(candidate);
        }
    }
    family_leaders
        .into_iter()
        .chain(additional_variants)
        .take(limit)
        .collect()
}

fn summarize_ranked_candidates(candidates: &[Candidate]) -> Vec<GameplayCandidateRanking> {
    let mut seen = BTreeSet::new();
    candidates
        .iter()
        .filter(|candidate| seen.insert(candidate.kind))
        .take(5)
        .map(|candidate| GameplayCandidateRanking {
            command: candidate.kind,
            score: candidate.score,
            description: candidate.description.clone(),
        })
        .collect()
}

#[derive(Debug)]
struct ExecutedAction {
    kind: GameplayCommandKind,
    description: String,
    outcome: String,
}

fn record_generated_candidates(candidates: &[Candidate], accumulator: &mut CampaignAccumulator) {
    for candidate in candidates {
        let command_stats = accumulator
            .commands
            .get_mut(&candidate.kind)
            .expect("every command kind must have statistics");
        command_stats.generated = command_stats.generated.saturating_add(1);
    }
}

fn record_activation_opportunities(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    accumulator: &mut CampaignAccumulator,
) {
    let crisis_opportunity = state.crises.values().any(|crisis| {
        crisis.status.is_active()
            && !state.audit_log.iter().any(|record| {
                record.kind() == AuditKind::CrisisResponse
                    && record.subject() == format!("crisis:{}", crisis.id)
            })
    });
    let labor_opportunity = state.employment.values().any(|agreement| {
        agreement.status == EmploymentStatus::Disputed
            && state
                .businesses
                .get(agreement.business_id)
                .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id)
    });
    let legal_opportunity = state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .any(|dynasty| legal_grievance_kind(state, dynasty.id()).is_some());
    let property_liquidation_opportunity = has_property_liquidation_opportunity(registry, state);
    let institution_withdrawal_opportunity = has_institution_withdrawal_opportunity(state);
    let extend_credit_opportunity = has_extend_credit_opportunity(state, persona);
    let mut family_candidates = Vec::new();
    generate_family_candidates(registry, state, persona, &mut family_candidates);
    let governance_opportunity = family_candidates
        .iter()
        .any(|candidate| candidate.kind == GameplayCommandKind::SetHouseGovernance);
    let family_council_opportunity = family_candidates
        .iter()
        .any(|candidate| candidate.kind == GameplayCommandKind::ConveneFamilyCouncil);
    let mut civic_candidates = Vec::new();
    generate_law_candidates(registry, state, persona, &mut civic_candidates);
    generate_public_work_candidates(registry, state, persona, &mut civic_candidates);
    generate_office_power_directive_candidates(registry, state, persona, &mut civic_candidates);
    let law_opportunity = civic_candidates
        .iter()
        .any(|candidate| candidate.kind == GameplayCommandKind::EnactLaw);
    let public_work_opportunity = civic_candidates
        .iter()
        .any(|candidate| candidate.kind == GameplayCommandKind::StartPublicWork);
    let office_power_opportunity = civic_candidates
        .iter()
        .any(|candidate| candidate.kind == GameplayCommandKind::ExerciseOfficePower);
    let mut information_candidates = Vec::new();
    generate_information_candidates(registry, state, persona, &mut information_candidates);
    let information_commission_opportunity = information_candidates
        .iter()
        .any(|candidate| candidate.kind == GameplayCommandKind::CommissionInformation);
    let information_leverage_opportunity = information_candidates
        .iter()
        .any(|candidate| candidate.kind == GameplayCommandKind::LeverageInformation);
    for (kind, available) in [
        (GameplayCommandKind::RespondToCrisis, crisis_opportunity),
        (GameplayCommandKind::ResolveLaborDispute, labor_opportunity),
        (GameplayCommandKind::FileLegalCase, legal_opportunity),
        (
            GameplayCommandKind::SellProperty,
            property_liquidation_opportunity,
        ),
        (
            GameplayCommandKind::WithdrawFromInstitution,
            institution_withdrawal_opportunity,
        ),
        (GameplayCommandKind::ExtendCredit, extend_credit_opportunity),
        (
            GameplayCommandKind::SetHouseGovernance,
            governance_opportunity,
        ),
        (
            GameplayCommandKind::ConveneFamilyCouncil,
            family_council_opportunity,
        ),
        (GameplayCommandKind::EnactLaw, law_opportunity),
        (
            GameplayCommandKind::StartPublicWork,
            public_work_opportunity,
        ),
        (
            GameplayCommandKind::ExerciseOfficePower,
            office_power_opportunity,
        ),
        (
            GameplayCommandKind::CommissionInformation,
            information_commission_opportunity,
        ),
        (
            GameplayCommandKind::LeverageInformation,
            information_leverage_opportunity,
        ),
    ] {
        if available {
            let command_stats = accumulator
                .commands
                .get_mut(&kind)
                .expect("every command kind must have statistics");
            command_stats.activation_opportunities =
                command_stats.activation_opportunities.saturating_add(1);
        }
    }
}

fn record_offered_command_kinds(candidates: &[Candidate], accumulator: &mut CampaignAccumulator) {
    let offered: BTreeSet<_> = candidates.iter().map(|candidate| candidate.kind).collect();
    for kind in offered {
        let command_stats = accumulator
            .commands
            .get_mut(&kind)
            .expect("every command kind must have statistics");
        command_stats.offered_cycles = command_stats.offered_cycles.saturating_add(1);
    }
}

fn apply_selected_candidate(
    registry: &Registry,
    state: &mut AppState,
    selected: Option<Candidate>,
    accumulator: &mut CampaignAccumulator,
) -> Result<Option<ExecutedAction>, GameplayHarnessError> {
    let Some(candidate) = selected else {
        return Ok(None);
    };
    let outcome = apply_player_command(registry, state, candidate.command).map_err(|source| {
        GameplayHarnessError::SelectedCommandRejected {
            description: candidate.description.clone(),
            source,
        }
    })?;
    accumulator
        .commands
        .get_mut(&candidate.kind)
        .expect("every command kind must have statistics")
        .executed = accumulator
        .commands
        .get(&candidate.kind)
        .expect("every command kind must have statistics")
        .executed
        .saturating_add(1);
    accumulator.record_executed_command(candidate.kind, state.clock.day());
    Ok(Some(ExecutedAction {
        kind: candidate.kind,
        description: candidate.description,
        outcome: outcome.summary,
    }))
}

fn probe_candidates(
    registry: &Registry,
    state: &AppState,
    candidates: impl Iterator<Item = Candidate>,
    projection_days: u32,
    accumulator: &mut CampaignAccumulator,
) -> Result<ProbeResult, GameplayHarnessError> {
    let baseline = GameplaySnapshot::capture(state);
    let mut projected_baseline_state = state.clone();
    advance_days(registry, &mut projected_baseline_state, projection_days)?;
    let projected_baseline = GameplaySnapshot::capture(&projected_baseline_state);
    let mut selected = None;
    let mut housekeeping_fallback = None;
    let mut viable_count = 0_usize;
    let mut substantive_viable_count = 0_usize;
    let mut viable_command_kinds = BTreeSet::new();
    let mut viable_options = Vec::new();
    let mut immediate_profiles = BTreeSet::new();
    let mut projected_profiles = BTreeSet::new();
    let mut rejections = Vec::new();
    for candidate in candidates {
        let command_stats = accumulator
            .commands
            .get_mut(&candidate.kind)
            .expect("every command kind must have statistics");
        command_stats.considered = command_stats.considered.saturating_add(1);
        let mut probe = state.clone();
        match apply_player_command(registry, &mut probe, candidate.command.clone()) {
            Ok(_) => {
                command_stats.viable = command_stats.viable.saturating_add(1);
                viable_count = viable_count.saturating_add(1);
                if candidate.kind != GameplayCommandKind::AcknowledgeNotification {
                    substantive_viable_count = substantive_viable_count.saturating_add(1);
                    if viable_command_kinds.insert(candidate.kind) {
                        let immediate_snapshot = GameplaySnapshot::capture(&probe);
                        let immediate_domains = baseline.changed_domains(&immediate_snapshot);
                        let immediate_history_change = baseline.audit_state_checksum
                            != immediate_snapshot.audit_state_checksum;
                        let mut projected_state = probe.clone();
                        advance_days(registry, &mut projected_state, projection_days)?;
                        let projected_snapshot = GameplaySnapshot::capture(&projected_state);
                        let projected_domains =
                            projected_baseline.changed_domains(&projected_snapshot);
                        let projected_history_change = projected_baseline.audit_state_checksum
                            != projected_snapshot.audit_state_checksum;
                        immediate_profiles
                            .insert((immediate_domains.clone(), immediate_history_change));
                        projected_profiles
                            .insert((projected_domains.clone(), projected_history_change));
                        viable_options.push(GameplayViableOption {
                            command: candidate.kind,
                            score: candidate.score,
                            description: candidate.description.clone(),
                            immediate_domains,
                            projected_domains,
                            immediate_history_change,
                            projected_history_change,
                        });
                    }
                    if selected.is_none() {
                        selected = Some(candidate);
                    }
                } else if housekeeping_fallback.is_none() {
                    housekeeping_fallback = Some(candidate);
                }
            }
            Err(error) => {
                command_stats.rejected = command_stats.rejected.saturating_add(1);
                let category = command_error_category(&error).to_owned();
                *accumulator
                    .rejection_reasons
                    .entry(category.clone())
                    .or_default() += 1;
                if rejections.len() < 4 {
                    rejections.push(category);
                }
            }
        }
    }
    let close_choice_score_gap = viable_options
        .first()
        .zip(viable_options.get(1))
        .map(|(first, second)| first.score.saturating_sub(second.score));
    Ok(ProbeResult {
        selected: selected.or(housekeeping_fallback),
        viable_count,
        substantive_viable_count,
        viable_command_kinds,
        viable_options,
        close_choice_score_gap,
        distinct_immediate_choice_profiles: immediate_profiles.len(),
        distinct_projected_choice_profiles: projected_profiles.len(),
        rejections,
    })
}

struct CycleObservation<'a> {
    before: &'a GameplaySnapshot,
    after_command: &'a GameplaySnapshot,
    after_time: &'a GameplaySnapshot,
    baseline_after_time: &'a GameplaySnapshot,
    considered: usize,
    viable: usize,
    substantive_viable: usize,
    viable_command_kinds: BTreeSet<GameplayCommandKind>,
    ranked_candidates: Vec<GameplayCandidateRanking>,
    viable_options: Vec<GameplayViableOption>,
    close_choice_score_gap: Option<i64>,
    distinct_immediate_choice_profiles: usize,
    distinct_projected_choice_profiles: usize,
    rejections: Vec<String>,
    action: Option<ExecutedAction>,
}

fn record_cycle(observation: CycleObservation<'_>, accumulator: &mut CampaignAccumulator) {
    let CycleObservation {
        before,
        after_command,
        after_time,
        baseline_after_time,
        considered,
        viable,
        substantive_viable,
        viable_command_kinds,
        ranked_candidates,
        viable_options,
        close_choice_score_gap,
        distinct_immediate_choice_profiles,
        distinct_projected_choice_profiles,
        rejections,
        action,
    } = observation;
    let immediate_domains = before.changed_domains(after_command);
    let total_causal_domains = baseline_after_time.changed_domains(after_time);
    let persistent_domains: BTreeSet<_> = immediate_domains
        .intersection(&total_causal_domains)
        .copied()
        .collect();
    let persistent_history_change =
        persistent_history_changed(before, after_command, after_time, baseline_after_time);
    let delayed_domains: BTreeSet<_> = total_causal_domains
        .difference(&immediate_domains)
        .copied()
        .collect();
    let ambient_domains = before.changed_domains(baseline_after_time);
    let signals = cycle_trace_signals(
        before,
        after_command,
        after_time,
        baseline_after_time,
        persistent_history_change,
    );
    let observed_domains: BTreeSet<_> = immediate_domains
        .union(&delayed_domains)
        .copied()
        .chain(ambient_domains.iter().copied())
        .collect();
    record_cycle_domain_changes(
        &observed_domains,
        &immediate_domains,
        &delayed_domains,
        &ambient_domains,
        accumulator,
    );
    if let Some(action) = &action {
        record_action_consequences(
            action.kind,
            ActionConsequenceObservation {
                immediate: &immediate_domains,
                persistent: &persistent_domains,
                delayed: &delayed_domains,
                signals: &signals,
            },
            accumulator,
        );
    }
    accumulator.trace.push(GameplayTraceStep {
        day: before.day,
        context: GameplayDecisionContext::from(before),
        considered_candidates: usize_to_u16(considered),
        viable_candidates: usize_to_u16(viable),
        substantive_viable_candidates: usize_to_u16(substantive_viable),
        viable_command_kinds,
        ranked_candidates,
        viable_options,
        close_choice_score_gap,
        distinct_immediate_choice_profiles: usize_to_u16(distinct_immediate_choice_profiles),
        distinct_projected_choice_profiles: usize_to_u16(distinct_projected_choice_profiles),
        selected_command: action.as_ref().map(|action| action.kind),
        command_description: action.as_ref().map(|action| action.description.clone()),
        outcome: action.map(|action| action.outcome),
        rejection_summary: rejections,
        immediate_domains,
        delayed_domains,
        persistent_domains,
        ambient_domains,
        signals,
    });
}

fn cycle_trace_signals(
    before: &GameplaySnapshot,
    after_command: &GameplaySnapshot,
    after_time: &GameplaySnapshot,
    baseline_after_time: &GameplaySnapshot,
    persistent_history_change: bool,
) -> BTreeSet<GameplayTraceSignal> {
    let immediate_feedback = !before.changed_domains(after_command).is_empty()
        || after_command.outbox_messages > before.outbox_messages
        || after_command.chronicle_entries > before.chronicle_entries;
    let delayed_feedback = after_time
        .outbox_messages
        .saturating_sub(after_command.outbox_messages)
        != baseline_after_time
            .outbox_messages
            .saturating_sub(before.outbox_messages)
        || after_time
            .chronicle_entries
            .saturating_sub(after_command.chronicle_entries)
            != baseline_after_time
                .chronicle_entries
                .saturating_sub(before.chronicle_entries);
    let ambient_feedback = baseline_after_time.outbox_messages > before.outbox_messages
        || baseline_after_time.chronicle_entries > before.chronicle_entries;
    [
        immediate_feedback.then_some(GameplayTraceSignal::ImmediateWorldFeedback),
        delayed_feedback.then_some(GameplayTraceSignal::DelayedWorldFeedback),
        ambient_feedback.then_some(GameplayTraceSignal::AmbientWorldFeedback),
        persistent_history_change.then_some(GameplayTraceSignal::PersistentHistoryChange),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn record_cycle_domain_changes(
    observed: &BTreeSet<GameplayDomain>,
    immediate: &BTreeSet<GameplayDomain>,
    delayed: &BTreeSet<GameplayDomain>,
    ambient: &BTreeSet<GameplayDomain>,
    accumulator: &mut CampaignAccumulator,
) {
    for domain in observed {
        *accumulator.domain_changes.entry(*domain).or_default() += 1;
    }
    for domain in immediate.union(delayed) {
        *accumulator
            .causal_domain_changes
            .entry(*domain)
            .or_default() += 1;
    }
    for domain in ambient {
        *accumulator
            .ambient_domain_changes
            .entry(*domain)
            .or_default() += 1;
    }
}

#[derive(Clone, Copy)]
struct ActionConsequenceObservation<'a> {
    immediate: &'a BTreeSet<GameplayDomain>,
    persistent: &'a BTreeSet<GameplayDomain>,
    delayed: &'a BTreeSet<GameplayDomain>,
    signals: &'a BTreeSet<GameplayTraceSignal>,
}

fn record_action_consequences(
    kind: GameplayCommandKind,
    observation: ActionConsequenceObservation<'_>,
    accumulator: &mut CampaignAccumulator,
) {
    let ActionConsequenceObservation {
        immediate,
        persistent,
        delayed,
        signals,
    } = observation;
    let immediate_feedback = signals.contains(&GameplayTraceSignal::ImmediateWorldFeedback);
    let delayed_feedback = signals.contains(&GameplayTraceSignal::DelayedWorldFeedback);
    let persistent_history_change = signals.contains(&GameplayTraceSignal::PersistentHistoryChange);
    let command_stats = accumulator
        .commands
        .get_mut(&kind)
        .expect("every command kind must have statistics");
    if immediate_feedback {
        command_stats.immediate_world_feedback =
            command_stats.immediate_world_feedback.saturating_add(1);
    }
    if delayed_feedback {
        command_stats.delayed_world_feedback =
            command_stats.delayed_world_feedback.saturating_add(1);
    }
    if immediate_feedback || delayed_feedback {
        command_stats.actions_with_feedback = command_stats.actions_with_feedback.saturating_add(1);
    }
    if !persistent.is_empty() || persistent_history_change {
        command_stats.actions_with_persistent_consequences = command_stats
            .actions_with_persistent_consequences
            .saturating_add(1);
    }
    if !delayed.is_empty() {
        command_stats.actions_with_delayed_consequences = command_stats
            .actions_with_delayed_consequences
            .saturating_add(1);
    }
    for domain in immediate.union(delayed) {
        command_stats.changed_domains.insert(*domain);
        *accumulator.interactions.entry((kind, *domain)).or_default() += 1;
    }
}

fn ranked_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    accumulator: &CampaignAccumulator,
) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    generate_reactive_candidates(state, persona, &mut candidates);
    generate_business_candidates(registry, state, persona, &mut candidates);
    generate_contract_candidates(registry, state, persona, &mut candidates);
    generate_finance_candidates(registry, state, persona, &mut candidates);
    generate_information_candidates(registry, state, persona, &mut candidates);
    generate_civic_candidates(registry, state, persona, &mut candidates);
    generate_family_candidates(registry, state, persona, &mut candidates);
    candidates
        .retain(|candidate| candidate_preserves_office_duty_reserve(registry, state, candidate));
    for candidate in &mut candidates {
        candidate.score = candidate.score.saturating_add(rank_adjustment(
            candidate.kind,
            state,
            persona,
            accumulator,
        ));
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.description.cmp(&right.description))
    });
    candidates
}

fn candidate_preserves_office_duty_reserve(
    registry: &Registry,
    state: &AppState,
    candidate: &Candidate,
) -> bool {
    if candidate_is_emergency_spending(state, candidate) {
        return true;
    }
    let additional_powers = match &candidate.command {
        PlayerCommand::NominateForOffice { institution_id, .. } => state
            .institutions
            .get(institution_id)
            .map_or(0, |institution| institution.powers.len()),
        PlayerCommand::TransferBusinessCash { .. }
        | PlayerCommand::AcquireBusiness { .. }
        | PlayerCommand::InvestInBusiness { .. }
        | PlayerCommand::SetBusinessPolicy { .. }
        | PlayerCommand::CreateSupplyContract { .. }
        | PlayerCommand::IssueLoan { .. }
        | PlayerCommand::BuyProperty { .. }
        | PlayerCommand::SellProperty { .. }
        | PlayerCommand::EnactLaw { .. }
        | PlayerCommand::StartPublicWork { .. }
        | PlayerCommand::FileLegalCase { .. }
        | PlayerCommand::SetHouseGovernance { .. }
        | PlayerCommand::ConveneFamilyCouncil
        | PlayerCommand::DesignateHeir { .. }
        | PlayerCommand::AdoptWard { .. }
        | PlayerCommand::EducateFamilyMember { .. }
        | PlayerCommand::CultivateInstitutionSupport { .. }
        | PlayerCommand::ExerciseOfficePower { .. }
        | PlayerCommand::WithdrawFromInstitution { .. }
        | PlayerCommand::RespondToCrisis { .. }
        | PlayerCommand::ResolveLaborDispute { .. }
        | PlayerCommand::CommissionInformation { .. }
        | PlayerCommand::LeverageInformation { .. }
        | PlayerCommand::AcknowledgeNotification { .. } => 0,
    };
    let reserve = if matches!(candidate.command, PlayerCommand::ConveneFamilyCouncil)
        && state
            .family_councils
            .get(&state.player_dynasty_id)
            .is_some_and(|council| {
                council.unity_basis_points < FAMILY_COUNCIL_INTERVENTION_UNITY_THRESHOLD
            }) {
        player_family_recovery_office_duty_reserve(state)
    } else {
        player_office_duty_reserve(state, additional_powers)
    };
    let reserve = if additional_powers > 0 && player_has_office_duty_forfeiture(state) {
        reserve.saturating_mul(3)
    } else {
        reserve
    };
    if reserve == Money::ZERO {
        return true;
    }
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let cost = candidate_player_treasury_cost(registry, state, candidate);
    cost == Money::ZERO || treasury.saturating_sub(cost) >= reserve
}

fn player_has_office_duty_forfeiture(state: &AppState) -> bool {
    state.audit_log.iter().any(|record| {
        record.kind() == AuditKind::OfficeDutyForfeiture
            && audit_subject_has_dynasty(record.audit_subject(), state.player_dynasty_id)
    })
}

fn audit_subject_has_dynasty(subject: &AuditSubject, dynasty_id: DynastyId) -> bool {
    subject.references_dynasty(dynasty_id)
}

fn candidate_is_emergency_spending(state: &AppState, candidate: &Candidate) -> bool {
    match &candidate.command {
        PlayerCommand::RespondToCrisis { crisis_id, .. } => {
            state.crises.get(crisis_id).is_some_and(|crisis| {
                crisis.status == CrisisStatus::Escalated || crisis.severity_basis_points >= 8_000
            })
        }
        PlayerCommand::InvestInBusiness { business_id, .. } => {
            state.businesses.get(*business_id).is_some_and(|business| {
                matches!(
                    business.status(),
                    BusinessStatus::Distressed | BusinessStatus::Insolvent
                ) || business.operations.condition_basis_points < 2_000
            })
        }
        PlayerCommand::TransferBusinessCash { .. }
        | PlayerCommand::AcquireBusiness { .. }
        | PlayerCommand::SetBusinessPolicy { .. }
        | PlayerCommand::CreateSupplyContract { .. }
        | PlayerCommand::IssueLoan { .. }
        | PlayerCommand::BuyProperty { .. }
        | PlayerCommand::SellProperty { .. }
        | PlayerCommand::EnactLaw { .. }
        | PlayerCommand::StartPublicWork { .. }
        | PlayerCommand::FileLegalCase { .. }
        | PlayerCommand::SetHouseGovernance { .. }
        | PlayerCommand::ConveneFamilyCouncil
        | PlayerCommand::DesignateHeir { .. }
        | PlayerCommand::AdoptWard { .. }
        | PlayerCommand::EducateFamilyMember { .. }
        | PlayerCommand::CultivateInstitutionSupport { .. }
        | PlayerCommand::NominateForOffice { .. }
        | PlayerCommand::ExerciseOfficePower { .. }
        | PlayerCommand::WithdrawFromInstitution { .. }
        | PlayerCommand::ResolveLaborDispute { .. }
        | PlayerCommand::CommissionInformation { .. }
        | PlayerCommand::LeverageInformation { .. }
        | PlayerCommand::AcknowledgeNotification { .. } => false,
    }
}

fn player_office_duty_reserve(state: &AppState, additional_powers: usize) -> Money {
    let monthly_duty =
        projected_dynasty_monthly_office_duty(state, state.player_dynasty_id, additional_powers);
    if monthly_duty == Money::ZERO {
        return Money::ZERO;
    }
    monthly_duty
        .saturating_mul(AGENT_OFFICE_DUTY_RESERVE_MONTHS)
        .saturating_add(AGENT_OFFICE_LIQUIDITY_BUFFER)
}

fn player_family_recovery_office_duty_reserve(state: &AppState) -> Money {
    let monthly_duty = projected_dynasty_monthly_office_duty(state, state.player_dynasty_id, 0);
    if monthly_duty == Money::ZERO {
        return Money::ZERO;
    }
    monthly_duty
        .saturating_mul(AGENT_FAMILY_COUNCIL_DUTY_RESERVE_MONTHS)
        .saturating_add(AGENT_FAMILY_COUNCIL_LIQUIDITY_BUFFER)
}

fn candidate_player_treasury_cost(
    registry: &Registry,
    state: &AppState,
    candidate: &Candidate,
) -> Money {
    match &candidate.command {
        PlayerCommand::AcquireBusiness {
            business_id,
            recapitalization,
            ..
        } => quote_business_acquisition(registry, state, state.player_dynasty_id, *business_id)
            .map_or(Money::ZERO, |quote| {
                quote.purchase_price.saturating_add(*recapitalization)
            }),
        PlayerCommand::InvestInBusiness { amount, .. } => *amount,
        PlayerCommand::IssueLoan { terms }
            if terms.lender_dynasty_id == state.player_dynasty_id =>
        {
            terms.principal
        }
        PlayerCommand::BuyProperty { property_id } => state
            .properties
            .get(property_id)
            .map_or(Money::ZERO, |property| property.value),
        PlayerCommand::EnactLaw { .. } => Money::from_copper(2_000),
        PlayerCommand::StartPublicWork { budget, .. } => {
            Money::from_copper((budget.copper() / 10).max(1)).min(*budget)
        }
        PlayerCommand::FileLegalCase { .. } => LEGAL_CASE_FILING_COST,
        PlayerCommand::ConveneFamilyCouncil => FAMILY_COUNCIL_MEETING_COST,
        PlayerCommand::AdoptWard { .. } => WARD_ADOPTION_COST,
        PlayerCommand::EducateFamilyMember { .. } => FAMILY_EDUCATION_COST,
        PlayerCommand::CultivateInstitutionSupport { .. } => INSTITUTION_SUPPORT_COST,
        PlayerCommand::NominateForOffice { .. } => Money::from_copper(300),
        PlayerCommand::CommissionInformation { .. } => INFORMATION_COMMISSION_COST,
        PlayerCommand::LeverageInformation { .. } => INFORMATION_LEVERAGE_COST,
        PlayerCommand::RespondToCrisis {
            crisis_id,
            response,
        } => match response {
            CrisisResponse::Relief => state.crises.get(crisis_id).map_or(Money::ZERO, |crisis| {
                Money::from_copper(i64::from(crisis.severity_basis_points).saturating_mul(2))
            }),
            CrisisResponse::Reform => Money::from_copper(1_500),
            CrisisResponse::Suppress => Money::from_copper(900),
            CrisisResponse::Exploit => Money::ZERO,
        },
        PlayerCommand::TransferBusinessCash { .. }
        | PlayerCommand::SetBusinessPolicy { .. }
        | PlayerCommand::CreateSupplyContract { .. }
        | PlayerCommand::IssueLoan { .. }
        | PlayerCommand::SellProperty { .. }
        | PlayerCommand::WithdrawFromInstitution { .. }
        | PlayerCommand::ExerciseOfficePower { .. }
        | PlayerCommand::SetHouseGovernance { .. }
        | PlayerCommand::DesignateHeir { .. }
        | PlayerCommand::ResolveLaborDispute { .. }
        | PlayerCommand::AcknowledgeNotification { .. } => Money::ZERO,
    }
}

fn generate_reactive_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    for crisis in state.crises.values().filter(|crisis| {
        crisis.status.is_active() && !crisis_has_containment_response(state, crisis.id)
    }) {
        let was_exploited = crisis_was_exploited(state, crisis.id);
        for response in crisis_responses(persona) {
            if response == CrisisResponse::Exploit && was_exploited {
                continue;
            }
            if !can_afford_crisis_response(state, crisis, response) {
                continue;
            }
            push_candidate(
                candidates,
                GameplayCommandKind::RespondToCrisis,
                PlayerCommand::RespondToCrisis {
                    crisis_id: crisis.id,
                    response,
                },
                format!("respond {response:?} to crisis {}", crisis.id),
                crisis_response_bonus(persona, response),
            );
        }
    }
    for agreement in state.employment.values().filter(|agreement| {
        agreement.status == EmploymentStatus::Disputed
            && state
                .businesses
                .get(agreement.business_id)
                .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id)
    }) {
        if let Some(response) = preferred_labor_response(state, agreement, persona) {
            push_candidate(
                candidates,
                GameplayCommandKind::ResolveLaborDispute,
                PlayerCommand::ResolveLaborDispute {
                    employment_id: agreement.id,
                    response,
                },
                format!("resolve labor dispute {} with {response:?}", agreement.id),
                labor_response_bonus(persona, response),
            );
        }
    }
    let unread_notifications = state
        .outbox
        .iter()
        .filter(|message| !message.acknowledged)
        .count();
    if unread_notifications >= NOTIFICATION_BATCH_THRESHOLD
        && let Some(message) = state
            .outbox
            .iter()
            .rev()
            .find(|message| !message.acknowledged)
    {
        push_candidate(
            candidates,
            GameplayCommandKind::AcknowledgeNotification,
            PlayerCommand::AcknowledgeNotification {
                message_id: message.id,
            },
            format!(
                "acknowledge {unread_notifications} notifications through notification {}",
                message.id
            ),
            0,
        );
    }
}

fn crisis_has_containment_response(state: &AppState, crisis_id: crate::ids::CrisisId) -> bool {
    let subject = format!("crisis:{crisis_id}");
    state
        .audit_log
        .iter()
        .rev()
        .any(|record| record.subject() == subject && crisis_response_contains_crisis(record))
}

fn crisis_was_exploited(state: &AppState, crisis_id: crate::ids::CrisisId) -> bool {
    let subject = format!("crisis:{crisis_id}");
    state.audit_log.iter().rev().any(|record| {
        record.kind() == AuditKind::CrisisResponse
            && record.subject() == subject
            && record.detail() == "response=Exploit"
    })
}

fn can_afford_crisis_response(
    state: &AppState,
    crisis: &crate::core::Crisis,
    response: CrisisResponse,
) -> bool {
    let dynasty = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    match response {
        CrisisResponse::Relief => {
            dynasty.treasury()
                >= Money::from_copper(i64::from(crisis.severity_basis_points).saturating_mul(2))
        }
        CrisisResponse::Reform => dynasty.treasury() >= Money::from_copper(1_500),
        CrisisResponse::Suppress => dynasty.treasury() >= Money::from_copper(900),
        CrisisResponse::Exploit => dynasty.resources.legitimacy_basis_points >= 600,
    }
}

fn preferred_labor_response(
    state: &AppState,
    agreement: &crate::core::EmploymentAgreement,
    persona: GameplayPersona,
) -> Option<LaborResponse> {
    if agreement.conditions_basis_points < 5_000 {
        return can_execute_labor_response(state, agreement, LaborResponse::ImproveConditions)
            .then_some(LaborResponse::ImproveConditions);
    }
    labor_responses(persona)
        .into_iter()
        .find(|response| can_execute_labor_response(state, agreement, *response))
}

fn can_execute_labor_response(
    state: &AppState,
    agreement: &crate::core::EmploymentAgreement,
    response: LaborResponse,
) -> bool {
    let Some(business) = state.businesses.get(agreement.business_id) else {
        return false;
    };
    match response {
        LaborResponse::ImproveConditions => business.cash() >= Money::from_copper(1_000),
        LaborResponse::Negotiate => business.cash() >= Money::from_copper(500),
        LaborResponse::ReplaceWorkers => {
            business.cash() >= LABOR_REPLACEMENT_COST
                && state
                    .households
                    .ids_for_district(business.district_id())
                    .is_some_and(|ids| {
                        ids.iter().any(|household_id| {
                            *household_id != agreement.household_id
                                && available_household_workers(state, *household_id, None)
                                    >= u32::from(agreement.workers)
                        })
                    })
        }
    }
}

fn generate_business_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player_businesses: Vec<_> = state
        .businesses
        .ids_for_owner(state.player_dynasty_id)
        .into_iter()
        .flatten()
        .filter_map(|id| state.businesses.get(*id))
        .collect();
    generate_business_acquisition_candidates(
        registry,
        state,
        persona,
        &player_businesses,
        candidates,
    );
    for business in &player_businesses {
        generate_business_investment_candidate(registry, state, persona, business, candidates);
        generate_business_policy_candidates(state, persona, business, candidates);
    }
    generate_cash_rebalance_candidate(registry, state, &player_businesses, candidates);
}

fn generate_business_policy_candidates(
    state: &AppState,
    persona: GameplayPersona,
    business: &crate::core::Business,
    candidates: &mut Vec<Candidate>,
) {
    let policy_subject = format!("business:{}", business.id());
    let policy_change_available = state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::BusinessPolicyChange && record.subject() == policy_subject
        })
        .is_none_or(|record| {
            state.clock.day()
                >= record
                    .day()
                    .saturating_add(BUSINESS_POLICY_CHANGE_INTERVAL_DAYS)
        });
    if !policy_change_available {
        return;
    }
    let desired_label = preferred_policy_label(persona, business);
    for template in policy_templates(persona)
        .into_iter()
        .filter(|template| template.label == desired_label)
    {
        if business.policy.target_input_days == template.target_input_days
            && business.policy.target_output_days == template.target_output_days
            && business.policy.minimum_cash_reserve == template.minimum_cash_reserve
            && business.policy.maintenance_basis_points == template.maintenance_basis_points
            && business.policy.quality_target_basis_points == template.quality_target_basis_points
        {
            continue;
        }
        push_candidate(
            candidates,
            GameplayCommandKind::SetBusinessPolicy,
            PlayerCommand::SetBusinessPolicy {
                business_id: business.id(),
                target_input_days: template.target_input_days,
                target_output_days: template.target_output_days,
                minimum_cash_reserve: template.minimum_cash_reserve,
                maintenance_basis_points: template.maintenance_basis_points,
                quality_target_basis_points: template.quality_target_basis_points,
            },
            format!(
                "set {} policy on business {}",
                template.label,
                business.id()
            ),
            template.bonus,
        );
    }
}

fn preferred_policy_label(
    persona: GameplayPersona,
    business: &crate::core::Business,
) -> &'static str {
    let stressed = matches!(
        business.status(),
        BusinessStatus::Distressed | BusinessStatus::Insolvent
    ) || business.operations.condition_basis_points < 6_000
        || business.cash() < business.policy.minimum_cash_reserve;
    if stressed {
        return "defensive";
    }
    match persona {
        GameplayPersona::Steward => {
            if business.operations.quality_basis_points < 8_500
                && business.cash() >= Money::from_copper(6_000)
            {
                "premium"
            } else {
                "defensive"
            }
        }
        GameplayPersona::Entrepreneur => {
            if business.operations.quality_basis_points < 8_000 {
                "premium"
            } else {
                "growth"
            }
        }
        GameplayPersona::PowerBroker => "defensive",
        GameplayPersona::Opportunist => "growth",
    }
}

fn generate_cash_rebalance_candidate(
    registry: &Registry,
    state: &AppState,
    player_businesses: &[&crate::core::Business],
    candidates: &mut Vec<Candidate>,
) {
    if player_businesses.len() < 2 {
        return;
    }
    if state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::CashTransfer)
        .is_some_and(|record| {
            state.clock.day()
                < record
                    .day()
                    .saturating_add(AGENT_CASH_REBALANCE_INTERVAL_DAYS)
        })
    {
        return;
    }
    let source = player_businesses.iter().max_by_key(|business| {
        business
            .cash()
            .copper()
            .saturating_sub(business_cash_target(registry, state, business).copper())
    });
    let target = player_businesses.iter().max_by_key(|business| {
        business_cash_target(registry, state, business)
            .copper()
            .saturating_sub(business.cash().copper())
    });
    let (Some(source), Some(target)) = (source, target) else {
        return;
    };
    if source.id() == target.id() {
        return;
    }
    let source_surplus = source
        .cash()
        .copper()
        .saturating_sub(business_cash_target(registry, state, source).copper())
        .max(0);
    let target_deficit = business_cash_target(registry, state, target)
        .copper()
        .saturating_sub(target.cash().copper())
        .max(0);
    if target_deficit < AGENT_CASH_REBALANCE_TRIGGER.copper() {
        return;
    }
    let buffered_deficit = business_cash_target(registry, state, target)
        .saturating_add(AGENT_CASH_REBALANCE_BUFFER)
        .copper()
        .saturating_sub(target.cash().copper())
        .max(0);
    let amount = Money::from_copper(source_surplus.min(buffered_deficit));
    if amount < AGENT_CASH_REBALANCE_TRIGGER {
        return;
    }
    let urgency = if matches!(
        target.status(),
        BusinessStatus::Distressed | BusinessStatus::Insolvent
    ) {
        900
    } else {
        250
    };
    push_candidate(
        candidates,
        GameplayCommandKind::TransferBusinessCash,
        PlayerCommand::TransferBusinessCash {
            from_business_id: source.id(),
            to_business_id: target.id(),
            amount,
        },
        format!(
            "cover a {amount} liquidity shortfall from business {} to {}",
            source.id(),
            target.id()
        ),
        urgency,
    );
}

fn business_cash_target(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
) -> Money {
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe must exist");
    let payroll_buffer = state
        .employment
        .values()
        .filter(|agreement| {
            agreement.business_id == business.id()
                && matches!(
                    agreement.status,
                    EmploymentStatus::Active | EmploymentStatus::Disputed
                )
        })
        .fold(Money::ZERO, |total, agreement| {
            total.saturating_add(agreement.weekly_wage)
        });
    let recovery_buffer = if matches!(
        business.status(),
        BusinessStatus::Distressed | BusinessStatus::Insolvent
    ) {
        Money::from_copper(2_000)
    } else {
        Money::ZERO
    };
    business
        .policy
        .minimum_cash_reserve
        .saturating_add(recipe.daily_operating_cost().saturating_mul(7))
        .saturating_add(payroll_buffer)
        .saturating_add(recovery_buffer)
}

fn generate_business_investment_candidate(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    business: &crate::core::Business,
    candidates: &mut Vec<Candidate>,
) {
    if business.status() == BusinessStatus::Active {
        generate_planned_business_investment(state, persona, business, candidates);
        return;
    }
    if !matches!(
        business.status(),
        BusinessStatus::Distressed | BusinessStatus::Insolvent
    ) {
        return;
    }
    if has_internal_cash_recovery(registry, state, business) {
        return;
    }
    let player_treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe must exist");
    let average_food_satisfaction = average_household_food_satisfaction(state);
    let staple_emergency = registry
        .get_good(recipe.output_good_id())
        .is_some_and(|good| good.category() == GoodCategory::Staple)
        && average_food_satisfaction < 5_000;
    let severe_rehabilitation = business.operations.condition_basis_points < 2_000;
    let portfolio_emergency = player_has_no_active_business(state);
    let dynasty_reserve = if portfolio_emergency {
        Money::ZERO
    } else if severe_rehabilitation {
        Money::from_copper(2_000)
    } else {
        recapitalization_dynasty_reserve(persona, staple_emergency)
    };
    let spendable = Money::from_copper(
        player_treasury
            .copper()
            .saturating_sub(dynasty_reserve.copper())
            .max(0),
    );
    if spendable <= Money::ZERO {
        return;
    }
    let target_cash = business_recapitalization_target(registry, state, business);
    let shortfall = Money::from_copper(
        target_cash
            .copper()
            .saturating_sub(business.cash().copper())
            .max(0),
    );
    let amount = shortfall.min(spendable);
    let minimum_meaningful = recipe.daily_operating_cost().saturating_mul(7);
    if amount <= Money::ZERO
        || (!staple_emergency && amount < minimum_meaningful && amount < shortfall)
    {
        return;
    }
    let persona_bonus: i64 = match persona {
        GameplayPersona::Steward => 760,
        GameplayPersona::Entrepreneur => 700,
        GameplayPersona::PowerBroker => 260,
        GameplayPersona::Opportunist => 180,
    };
    let emergency_bonus = if portfolio_emergency {
        4_500
    } else if staple_emergency {
        3_000
    } else if severe_rehabilitation {
        2_600
    } else {
        0
    };
    push_candidate(
        candidates,
        GameplayCommandKind::InvestInBusiness,
        PlayerCommand::InvestInBusiness {
            business_id: business.id(),
            amount,
        },
        format!("invest {amount} in business {}", business.id()),
        persona_bonus
            .saturating_add(1_700)
            .saturating_add(emergency_bonus),
    );
}

fn generate_planned_business_investment(
    state: &AppState,
    persona: GameplayPersona,
    business: &crate::core::Business,
    candidates: &mut Vec<Candidate>,
) {
    let subject = format!("business:{}", business.id());
    if state.audit_log.iter().rev().any(|record| {
        record.kind() == AuditKind::BusinessCapitalization
            && record.subject() == subject
            && state.clock.day().saturating_sub(record.day())
                < AGENT_PLANNED_CAPITALIZATION_INTERVAL_DAYS
    }) {
        return;
    }
    let target_condition = 9_000_u16;
    let target_quality = business.policy.quality_target_basis_points.max(7_500);
    let condition_investment =
        i64::from(target_condition.saturating_sub(business.operations.condition_basis_points))
            .saturating_mul(2);
    let quality_investment =
        i64::from(target_quality.saturating_sub(business.operations.quality_basis_points))
            .saturating_mul(4);
    let desired = Money::from_copper(condition_investment.max(quality_investment));
    if desired < Money::from_copper(1_000) {
        return;
    }
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let reserve = recapitalization_dynasty_reserve(persona, false);
    let spendable = Money::from_copper(treasury.copper().saturating_sub(reserve.copper()).max(0));
    let amount = desired.min(AGENT_PLANNED_CAPITALIZATION_MAX).min(spendable);
    if amount < Money::from_copper(1_000) {
        return;
    }
    let bonus = match persona {
        GameplayPersona::Entrepreneur => 900,
        GameplayPersona::Steward => 650,
        GameplayPersona::Opportunist => 300,
        GameplayPersona::PowerBroker => 120,
    };
    push_candidate(
        candidates,
        GameplayCommandKind::InvestInBusiness,
        PlayerCommand::InvestInBusiness {
            business_id: business.id(),
            amount,
        },
        format!(
            "modernize business {} with {amount} of condition and quality investment",
            business.id()
        ),
        bonus,
    );
}

fn has_internal_cash_recovery(
    registry: &Registry,
    state: &AppState,
    target: &crate::core::Business,
) -> bool {
    let target_deficit = business_cash_target(registry, state, target)
        .copper()
        .saturating_sub(target.cash().copper())
        .max(0);
    if target_deficit < AGENT_CASH_REBALANCE_TRIGGER.copper() {
        return false;
    }
    state
        .businesses
        .ids_for_owner(state.player_dynasty_id)
        .into_iter()
        .flatten()
        .filter_map(|business_id| state.businesses.get(*business_id))
        .filter(|business| business.id() != target.id())
        .any(|business| {
            business
                .cash()
                .copper()
                .saturating_sub(business_cash_target(registry, state, business).copper())
                >= AGENT_CASH_REBALANCE_TRIGGER.copper()
        })
}

fn average_household_food_satisfaction(state: &AppState) -> u16 {
    crate::core::population_weighted_food_satisfaction_basis_points(state.households.iter())
        .unwrap_or(10_000)
}

const fn recapitalization_dynasty_reserve(
    persona: GameplayPersona,
    staple_emergency: bool,
) -> Money {
    if staple_emergency {
        return Money::ZERO;
    }
    match persona {
        GameplayPersona::Steward => Money::from_copper(15_000),
        GameplayPersona::Entrepreneur => Money::from_copper(10_000),
        GameplayPersona::PowerBroker => Money::from_copper(20_000),
        GameplayPersona::Opportunist => Money::from_copper(8_000),
    }
}

fn generate_business_acquisition_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    player_businesses: &[&crate::core::Business],
    candidates: &mut Vec<Candidate>,
) {
    let portfolio_limit = match persona {
        GameplayPersona::Entrepreneur | GameplayPersona::Opportunist => 3,
        GameplayPersona::Steward | GameplayPersona::PowerBroker => 2,
    };
    if player_businesses.len() >= portfolio_limit {
        return;
    }
    if !portfolio_ready_for_acquisition(state, player_businesses) {
        return;
    }
    let has_financially_stressed_business = player_businesses.iter().any(|business| {
        matches!(
            business.status(),
            BusinessStatus::Distressed | BusinessStatus::Insolvent
        )
    });
    let Some(manager_id) = acquisition_manager_id(state, player_businesses) else {
        return;
    };
    let operating_businesses = player_businesses
        .iter()
        .filter(|business| {
            matches!(
                business.status(),
                BusinessStatus::Active | BusinessStatus::Distressed
            )
        })
        .count();
    if operating_businesses > 0 && has_financially_stressed_business {
        return;
    }
    let persona_bonus: i64 = match persona {
        GameplayPersona::Entrepreneur => 720,
        GameplayPersona::Opportunist => 520,
        GameplayPersona::Steward => 320,
        GameplayPersona::PowerBroker => 280,
    };
    let recovery_bonus = if operating_businesses == 0 { 1_000 } else { 0 };
    for business in state.businesses.iter().filter(|business| {
        business.owner_dynasty_id() != state.player_dynasty_id
            && matches!(
                business.status(),
                BusinessStatus::Distressed | BusinessStatus::Insolvent | BusinessStatus::Closed
            )
    }) {
        let Ok(quote) =
            quote_business_acquisition(registry, state, state.player_dynasty_id, business.id())
        else {
            continue;
        };
        let recapitalization = acquisition_recapitalization(registry, state, business, quote);
        let required = quote.purchase_price.saturating_add(recapitalization);
        let player_treasury = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .treasury();
        let expansion_reserve =
            recapitalization_dynasty_reserve(persona, false).saturating_add(Money::from_copper(
                i64::try_from(player_businesses.len())
                    .unwrap_or(i64::MAX)
                    .saturating_mul(2_000),
            ));
        if player_treasury < required.saturating_add(expansion_reserve) {
            continue;
        }
        push_candidate(
            candidates,
            GameplayCommandKind::AcquireBusiness,
            PlayerCommand::AcquireBusiness {
                business_id: business.id(),
                manager_id,
                recapitalization,
            },
            format!(
                "acquire business {} for {} with {} working capital",
                business.id(),
                quote.purchase_price,
                recapitalization
            ),
            persona_bonus.saturating_add(recovery_bonus),
        );
    }
}

fn portfolio_ready_for_acquisition(
    state: &AppState,
    player_businesses: &[&crate::core::Business],
) -> bool {
    player_businesses.iter().all(|business| {
        business.status() == BusinessStatus::Active
            && business.operations.condition_basis_points >= 7_000
            && business.cash() >= business.policy.minimum_cash_reserve
            && business.finance.lifetime_revenue >= business.finance.lifetime_costs
            && !state.employment.values().any(|agreement| {
                agreement.business_id == business.id()
                    && agreement.status == EmploymentStatus::Disputed
            })
    })
}

fn acquisition_manager_id(
    state: &AppState,
    player_businesses: &[&crate::core::Business],
) -> Option<crate::ids::CharacterId> {
    let assigned_managers: BTreeSet<_> = player_businesses
        .iter()
        .map(|business| business.manager_id())
        .collect();
    let active_characters = || {
        state
            .characters
            .ids_for_dynasty(state.player_dynasty_id)
            .into_iter()
            .flatten()
            .filter_map(|character_id| state.characters.get(*character_id))
            .filter(|character| character.status() == CharacterStatus::Active)
    };
    active_characters()
        .filter(|character| !assigned_managers.contains(&character.id()))
        .max_by_key(|character| {
            u32::from(character.capabilities.craft)
                .saturating_add(u32::from(character.capabilities.commerce))
        })
        .or_else(|| {
            active_characters().max_by_key(|character| {
                u32::from(character.capabilities.craft)
                    .saturating_add(u32::from(character.capabilities.commerce))
            })
        })
        .map(crate::core::Character::id)
}

fn acquisition_recapitalization(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
    quote: crate::systems::BusinessAcquisitionQuote,
) -> Money {
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe must exist");
    let desired = quote
        .minimum_recapitalization
        .saturating_add(recipe.daily_operating_cost().saturating_mul(14));
    let player_treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let available = player_treasury
        .copper()
        .saturating_sub(quote.purchase_price.copper())
        .max(0);
    if available >= quote.minimum_recapitalization.copper() {
        Money::from_copper(desired.copper().min(available))
    } else {
        quote.minimum_recapitalization
    }
}

#[derive(Clone, Copy, Debug)]
struct PolicyTemplate {
    label: &'static str,
    target_input_days: u16,
    target_output_days: u16,
    minimum_cash_reserve: Money,
    maintenance_basis_points: u16,
    quality_target_basis_points: u16,
    bonus: i64,
}

fn policy_templates(persona: GameplayPersona) -> [PolicyTemplate; 3] {
    let premium_bonus = match persona {
        GameplayPersona::Entrepreneur => 260,
        GameplayPersona::Steward => 160,
        GameplayPersona::PowerBroker | GameplayPersona::Opportunist => 40,
    };
    let growth_bonus = match persona {
        GameplayPersona::Entrepreneur | GameplayPersona::Opportunist => 240,
        GameplayPersona::Steward => 80,
        GameplayPersona::PowerBroker => 20,
    };
    let defensive_bonus = match persona {
        GameplayPersona::Steward => 300,
        GameplayPersona::PowerBroker => 100,
        GameplayPersona::Entrepreneur => 60,
        GameplayPersona::Opportunist => 10,
    };
    let growth_maintenance_basis_points = if persona == GameplayPersona::Opportunist {
        400
    } else {
        800
    };
    [
        PolicyTemplate {
            label: "premium",
            target_input_days: 7,
            target_output_days: 3,
            minimum_cash_reserve: Money::from_copper(4_000),
            maintenance_basis_points: 1_800,
            quality_target_basis_points: 9_000,
            bonus: premium_bonus,
        },
        PolicyTemplate {
            label: "growth",
            target_input_days: 12,
            target_output_days: 1,
            minimum_cash_reserve: Money::from_copper(1_000),
            maintenance_basis_points: growth_maintenance_basis_points,
            quality_target_basis_points: 7_000,
            bonus: growth_bonus,
        },
        PolicyTemplate {
            label: "defensive",
            target_input_days: 5,
            target_output_days: 4,
            minimum_cash_reserve: Money::from_copper(8_000),
            maintenance_basis_points: 1_300,
            quality_target_basis_points: 7_800,
            bonus: defensive_bonus,
        },
    ]
}

fn generate_contract_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player_id = state.player_dynasty_id;
    for business in state.businesses.iter().filter(|business| {
        business.owner_dynasty_id() == player_id
            && !matches!(
                business.status(),
                BusinessStatus::Closed | BusinessStatus::Insolvent
            )
    }) {
        let recipe = registry
            .get_recipe(business.recipe_id())
            .expect("business recipe must exist");
        for input in recipe.inputs() {
            for seller in contract_sellers(registry, state, input.good_id(), player_id) {
                add_contract_candidate(
                    registry,
                    state,
                    candidates,
                    ContractCandidateInput {
                        kind: GameplayCommandKind::SecureSupply,
                        buyer_business_id: business.id(),
                        seller_business_id: seller,
                        good_id: input.good_id(),
                        quantity_per_week: input
                            .quantity()
                            .saturating_mul_ratio(secure_supply_batches(business), 1),
                        bonus: secure_supply_bonus(persona),
                    },
                );
            }
        }
        for buyer in contract_buyers(registry, state, recipe.output_good_id(), player_id) {
            add_contract_candidate(
                registry,
                state,
                candidates,
                ContractCandidateInput {
                    kind: GameplayCommandKind::SellOutput,
                    buyer_business_id: buyer,
                    seller_business_id: business.id(),
                    good_id: recipe.output_good_id(),
                    quantity_per_week: recipe
                        .output_quantity()
                        .saturating_mul_ratio(STANDARD_CONTRACT_BATCHES_PER_WEEK, 1),
                    bonus: sell_output_bonus(persona),
                },
            );
        }
    }
}

fn secure_supply_batches(business: &crate::core::Business) -> i64 {
    let has_trade_history = business.finance.lifetime_revenue > Money::ZERO
        || business.finance.lifetime_costs > Money::ZERO;
    if has_trade_history && business.finance.lifetime_revenue >= business.finance.lifetime_costs {
        STANDARD_CONTRACT_BATCHES_PER_WEEK
    } else {
        1
    }
}

const fn secure_supply_bonus(persona: GameplayPersona) -> i64 {
    match persona {
        GameplayPersona::Steward => 420,
        GameplayPersona::Entrepreneur => 520,
        GameplayPersona::PowerBroker => 80,
        GameplayPersona::Opportunist => 120,
    }
}

const fn sell_output_bonus(persona: GameplayPersona) -> i64 {
    match persona {
        GameplayPersona::Steward | GameplayPersona::PowerBroker => 100,
        GameplayPersona::Entrepreneur => 620,
        GameplayPersona::Opportunist => 560,
    }
}

fn contract_sellers<'a>(
    registry: &'a Registry,
    state: &'a AppState,
    good_id: crate::ids::GoodId,
    excluded_owner: DynastyId,
) -> impl Iterator<Item = BusinessId> + 'a {
    state.businesses.iter().filter_map(move |business| {
        let recipe = registry.get_recipe(business.recipe_id())?;
        (business.owner_dynasty_id() != excluded_owner
            && !matches!(
                business.status(),
                BusinessStatus::Closed | BusinessStatus::Insolvent
            )
            && recipe.output_good_id() == good_id)
            .then_some(business.id())
    })
}

fn contract_buyers<'a>(
    registry: &'a Registry,
    state: &'a AppState,
    good_id: crate::ids::GoodId,
    excluded_owner: DynastyId,
) -> impl Iterator<Item = BusinessId> + 'a {
    state.businesses.iter().filter_map(move |business| {
        let recipe = registry.get_recipe(business.recipe_id())?;
        (business.owner_dynasty_id() != excluded_owner
            && !matches!(
                business.status(),
                BusinessStatus::Closed | BusinessStatus::Insolvent
            )
            && recipe
                .inputs()
                .iter()
                .any(|input| input.good_id() == good_id))
        .then_some(business.id())
    })
}

#[derive(Clone, Copy, Debug)]
struct ContractCandidateInput {
    kind: GameplayCommandKind,
    buyer_business_id: BusinessId,
    seller_business_id: BusinessId,
    good_id: crate::ids::GoodId,
    quantity_per_week: Quantity,
    bonus: i64,
}

fn add_contract_candidate(
    registry: &Registry,
    state: &AppState,
    candidates: &mut Vec<Candidate>,
    input: ContractCandidateInput,
) {
    let ContractCandidateInput {
        kind,
        buyer_business_id,
        seller_business_id,
        good_id,
        quantity_per_week,
        bonus,
    } = input;
    if state.contracts.values().any(|contract| {
        contract.status == ContractStatus::Active
            && contract.buyer_business_id == buyer_business_id
            && contract.seller_business_id == seller_business_id
            && contract.good_id == good_id
    }) {
        return;
    }
    let Some(quote) = state.market.quotes.get(&good_id) else {
        return;
    };
    let price_bounds = contract_counterparty_price_bounds(
        state,
        buyer_business_id,
        seller_business_id,
        quote.price,
    );
    let buyer_is_player = state
        .businesses
        .get(buyer_business_id)
        .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id);
    let unit_price = if buyer_is_player {
        quote.price.max(price_bounds.minimum_seller_price)
    } else {
        quote.price.min(price_bounds.maximum_buyer_price)
    };
    if !can_support_contract_terms(
        registry,
        state,
        buyer_business_id,
        seller_business_id,
        good_id,
        quantity_per_week,
        unit_price,
    ) {
        return;
    }
    let penalty = cost_for(quantity_per_week, unit_price).saturating_mul(2);
    let relationship_note = if price_bounds.relationship_pressure_basis_points > 0 {
        format!(
            " under {} bp of counterparty pressure",
            price_bounds.relationship_pressure_basis_points
        )
    } else {
        String::new()
    };
    push_candidate(
        candidates,
        kind,
        PlayerCommand::CreateSupplyContract {
            terms: SupplyContractTerms {
                buyer_business_id,
                seller_business_id,
                good_id,
                quantity_per_week,
                unit_price,
                penalty,
                duration_weeks: AGENT_CONTRACT_DURATION_WEEKS,
            },
        },
        format!(
            "contract good {good_id} from business {seller_business_id} to {buyer_business_id} at {unit_price}{relationship_note}"
        ),
        bonus,
    );
}

fn can_support_contract_terms(
    registry: &Registry,
    state: &AppState,
    buyer_business_id: BusinessId,
    seller_business_id: BusinessId,
    good_id: crate::ids::GoodId,
    quantity_per_week: Quantity,
    unit_price: Money,
) -> bool {
    let Some(buyer) = state.businesses.get(buyer_business_id) else {
        return false;
    };
    let Some(seller) = state.businesses.get(seller_business_id) else {
        return false;
    };
    let Some(seller_recipe) = registry.get_recipe(seller.recipe_id()) else {
        return false;
    };
    let Some(capacity) = available_supply_contract_capacity(
        registry,
        state,
        buyer_business_id,
        seller_business_id,
        good_id,
    ) else {
        return false;
    };
    if quantity_per_week > capacity.seller || quantity_per_week > capacity.buyer {
        return false;
    }
    let weekly_payment = cost_for(quantity_per_week, unit_price);
    let buyer_working_cash = buyer
        .cash()
        .saturating_sub(buyer.policy.minimum_cash_reserve);
    if buyer_working_cash < weekly_payment.saturating_mul(4) {
        return false;
    }
    seller.inventory_quantity(good_id) >= quantity_per_week
        || seller.cash() >= seller_recipe.daily_operating_cost().saturating_mul(7)
}

fn generate_finance_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    add_borrow_candidate(state, persona, candidates);
    add_lend_candidate(state, persona, candidates);
    add_property_liquidation_candidates(registry, state, persona, candidates);
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let property_bonus = match persona {
        GameplayPersona::Entrepreneur => 430,
        GameplayPersona::Opportunist => 520,
        GameplayPersona::PowerBroker => 230,
        GameplayPersona::Steward => 160,
    };
    let mut properties: Vec<_> = state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id.is_none() && property.value <= treasury)
        .collect();
    properties.sort_by_key(|property| (property.value, property.id));
    for property in properties.into_iter().take(4) {
        push_candidate(
            candidates,
            GameplayCommandKind::BuyProperty,
            PlayerCommand::BuyProperty {
                property_id: property.id,
            },
            format!(
                "buy {:?} property {} for {}",
                property.kind, property.id, property.value
            ),
            property_bonus,
        );
    }
}

fn add_property_liquidation_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player_id = state.player_dynasty_id;
    if !player_needs_property_liquidation(state) {
        return;
    }
    let mut properties: Vec<_> = state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id == Some(player_id))
        .collect();
    properties.sort_by_key(|property| {
        (
            property.occupant_business_id.is_some(),
            property.value,
            property.id,
        )
    });
    let buyers: Vec<_> = state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != player_id)
        .collect();
    let persona_bonus = match persona {
        GameplayPersona::Steward => 6_000,
        GameplayPersona::Entrepreneur => 5_600,
        GameplayPersona::PowerBroker => 5_200,
        GameplayPersona::Opportunist => 6_400,
    };
    for property in properties.into_iter().take(2) {
        let buyer = buyers
            .iter()
            .filter_map(|buyer| {
                accepted_property_liquidation_quote(registry, state, buyer.id(), property.id)
                    .map(|quote| (*buyer, quote))
            })
            .max_by_key(|(buyer, quote)| (quote.buyer_contribution, buyer.treasury(), buyer.id()));
        let Some((buyer, quote)) = buyer else {
            continue;
        };
        push_candidate(
            candidates,
            GameplayCommandKind::SellProperty,
            PlayerCommand::SellProperty {
                property_id: property.id,
                buyer_dynasty_id: buyer.id(),
            },
            format!(
                "liquidate {:?} property {} to dynasty {} for {} net {}; lien payoff {}; civic guarantee {}",
                property.kind,
                property.id,
                buyer.id(),
                quote.price,
                quote.seller_proceeds,
                quote.lien_payoff,
                quote.civic_guarantee
            ),
            persona_bonus,
        );
    }
}

fn player_needs_property_liquidation(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    let player = state
        .dynasties
        .get(&player_id)
        .expect("player dynasty must exist");
    if player.treasury() >= Money::from_copper(2_000) {
        return false;
    }
    let business_rescue_needed = state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == player_id
            && (matches!(
                business.status(),
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            ) || business.cash() == Money::ZERO
                || business.operations.condition_basis_points < 2_000)
    });
    let owned_properties = state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id == Some(player_id))
        .count();
    business_rescue_needed || owned_properties >= 2
}

fn accepted_property_liquidation_quote(
    registry: &Registry,
    state: &AppState,
    buyer_dynasty_id: DynastyId,
    property_id: crate::ids::PropertyId,
) -> Option<crate::systems::PropertyLiquidationQuote> {
    let quote = quote_property_liquidation(
        registry,
        state,
        state.player_dynasty_id,
        buyer_dynasty_id,
        property_id,
    )
    .ok()?;
    let buyer = state.dynasties.get(&buyer_dynasty_id)?;
    buyer
        .treasury()
        .checked_sub(quote.buyer_contribution)
        .filter(|remaining| *remaining >= PROPERTY_COUNTERPARTY_BUYER_RESERVE)?;
    Some(quote)
}

fn has_property_liquidation_opportunity(registry: &Registry, state: &AppState) -> bool {
    if !player_needs_property_liquidation(state) {
        return false;
    }
    let player_id = state.player_dynasty_id;
    state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id == Some(player_id))
        .any(|property| {
            state
                .dynasties
                .keys()
                .copied()
                .filter(|dynasty_id| *dynasty_id != player_id)
                .any(|buyer_dynasty_id| {
                    accepted_property_liquidation_quote(
                        registry,
                        state,
                        buyer_dynasty_id,
                        property.id,
                    )
                    .is_some()
                })
        })
}

fn add_borrow_candidate(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player_id = state.player_dynasty_id;
    let player = state
        .dynasties
        .get(&player_id)
        .expect("player dynasty must exist");
    let base_borrowing_trigger = match persona {
        GameplayPersona::Steward => Money::from_copper(4_000),
        GameplayPersona::Entrepreneur => Money::from_copper(12_000),
        GameplayPersona::PowerBroker => Money::from_copper(3_000),
        GameplayPersona::Opportunist => Money::from_copper(8_000),
    };
    let office_reserve = player_office_duty_reserve(state, 0);
    let borrowing_trigger = if office_reserve > base_borrowing_trigger {
        office_reserve
    } else {
        base_borrowing_trigger
    };
    if player.treasury() >= borrowing_trigger
        || state.loans.values().any(|loan| {
            loan.borrower_dynasty_id == player_id
                && matches!(
                    loan.status,
                    LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured
                )
        })
    {
        return;
    }
    let lender = state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != player_id)
        .filter(|dynasty| !same_pair_credit_blocks_new_loan(state, dynasty.id(), player_id))
        .filter(|dynasty| {
            dynasty
                .treasury()
                .checked_sub(PRIVATE_LOAN_COUNTERPARTY_RESERVE)
                .is_some_and(|available| available >= Money::from_copper(1_000))
        })
        .max_by_key(|dynasty| dynasty.treasury());
    let Some(lender) = lender else {
        return;
    };
    let defaulted_loan = latest_defaulted_loan(state, lender.id(), player_id);
    let desired_principal = if defaulted_loan.is_some() {
        Money::from_copper((lender.treasury().copper() / 12).clamp(1_000, 6_000))
    } else {
        Money::from_copper((lender.treasury().copper() / 8).clamp(1_000, 12_000))
    };
    let lender_available = lender
        .treasury()
        .checked_sub(PRIVATE_LOAN_COUNTERPARTY_RESERVE)
        .expect("eligible lender must retain the negotiated reserve");
    let principal = desired_principal.min(lender_available);
    let repayment_balance =
        defaulted_loan.map_or(principal, |loan| loan.balance.saturating_add(principal));
    let amortization_weeks = if defaulted_loan.is_some() {
        AGENT_LOAN_AMORTIZATION_WEEKS.saturating_mul(2)
    } else {
        AGENT_LOAN_AMORTIZATION_WEEKS
    };
    let collateral = state.properties.values().find(|property| {
        property.owner_dynasty_id == Some(player_id) && property.collateral_loan_id.is_none()
    });
    let base_bonus: i64 = match persona {
        GameplayPersona::Opportunist => 520,
        GameplayPersona::Entrepreneur => 380,
        GameplayPersona::Steward => 80,
        GameplayPersona::PowerBroker => 120,
    };
    let bonus = base_bonus.saturating_add(if defaulted_loan.is_some() { 1_800 } else { 0 });
    push_candidate(
        candidates,
        GameplayCommandKind::BorrowFunds,
        PlayerCommand::IssueLoan {
            terms: LoanTerms {
                lender_dynasty_id: lender.id(),
                borrower_dynasty_id: player_id,
                principal,
                weekly_payment: positive_money_ceil_div(repayment_balance, amortization_weeks),
                interest_basis_points: if defaulted_loan.is_some() { 1_000 } else { 700 },
                collateral_property_id: collateral.map(|property| property.id),
            },
        },
        defaulted_loan.map_or_else(
            || format!("borrow {principal} from dynasty {}", lender.id()),
            |loan| {
                format!(
                    "restructure defaulted loan {} with a {principal} recovery advance from dynasty {}",
                    loan.id,
                    lender.id()
                )
            },
        ),
        bonus,
    );
}

fn same_pair_credit_blocks_new_loan(
    state: &AppState,
    lender_id: DynastyId,
    borrower_id: DynastyId,
) -> bool {
    state.loans.values().any(|loan| {
        loan.lender_dynasty_id == lender_id
            && loan.borrower_dynasty_id == borrower_id
            && (matches!(
                loan.status,
                LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured
            ) || (loan.status == LoanStatus::Defaulted
                && state.clock.day()
                    < loan
                        .next_due_day
                        .saturating_add(DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS)))
    })
}

fn latest_defaulted_loan(
    state: &AppState,
    lender_id: DynastyId,
    borrower_id: DynastyId,
) -> Option<&crate::core::Loan> {
    state
        .loans
        .values()
        .filter(|loan| {
            loan.lender_dynasty_id == lender_id
                && loan.borrower_dynasty_id == borrower_id
                && loan.status == LoanStatus::Defaulted
        })
        .max_by_key(|loan| (loan.next_due_day, loan.id))
}

fn lending_limits(persona: GameplayPersona) -> (Money, usize) {
    match persona {
        GameplayPersona::Steward => (Money::from_copper(40_000), 1),
        GameplayPersona::Entrepreneur => (Money::from_copper(30_000), 2),
        GameplayPersona::PowerBroker => (Money::from_copper(50_000), 1),
        GameplayPersona::Opportunist => (Money::from_copper(25_000), 2),
    }
}

fn positive_money_ceil_div(value: Money, denominator: i64) -> Money {
    debug_assert!(value > Money::ZERO);
    debug_assert!(denominator > 0);
    let copper = value.copper();
    Money::from_copper(copper / denominator + i64::from(copper % denominator != 0))
}

fn active_player_lending(state: &AppState) -> usize {
    state
        .loans
        .values()
        .filter(|loan| {
            loan.lender_dynasty_id == state.player_dynasty_id
                && matches!(
                    loan.status,
                    LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured
                )
        })
        .count()
}

fn eligible_lending_borrower(state: &AppState) -> Option<&crate::core::Dynasty> {
    state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .filter(|dynasty| {
            !same_pair_credit_blocks_new_loan(state, state.player_dynasty_id, dynasty.id())
        })
        .filter(|dynasty| lending_pressure(state, dynasty.id()) > 0)
        .min_by_key(|dynasty| {
            (
                std::cmp::Reverse(lending_pressure(state, dynasty.id())),
                dynasty.treasury(),
                dynasty.id(),
            )
        })
}

fn lending_pressure(state: &AppState, dynasty_id: DynastyId) -> u8 {
    let distressed_credit = state.loans.values().any(|loan| {
        loan.borrower_dynasty_id == dynasty_id
            && matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted)
    });
    if distressed_credit {
        return 2;
    }
    u8::from(
        state
            .dynasties
            .get(&dynasty_id)
            .is_some_and(|dynasty| dynasty.treasury() < AGENT_EXTERNAL_CREDIT_TREASURY_PRESSURE),
    )
}

fn eligible_lending_restructuring_borrower(state: &AppState) -> Option<&crate::core::Dynasty> {
    state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .filter(|dynasty| {
            latest_defaulted_loan(state, state.player_dynasty_id, dynasty.id()).is_some_and(
                |loan| {
                    state.clock.day()
                        >= loan
                            .next_due_day
                            .saturating_add(DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS)
                },
            )
        })
        .min_by_key(|dynasty| dynasty.treasury())
}

fn has_extend_credit_opportunity(state: &AppState, persona: GameplayPersona) -> bool {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let (lending_reserve, lending_limit) = lending_limits(persona);
    let can_restructure = player.treasury() >= Money::from_copper(1_000)
        && eligible_lending_restructuring_borrower(state).is_some();
    can_restructure
        || (player.treasury() >= lending_reserve
            && active_player_lending(state) < lending_limit
            && eligible_lending_borrower(state).is_some())
}

fn add_lend_candidate(state: &AppState, persona: GameplayPersona, candidates: &mut Vec<Candidate>) {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let (lending_reserve, lending_limit) = lending_limits(persona);
    let restructuring_borrower = eligible_lending_restructuring_borrower(state);
    let borrower = if let Some(borrower) = restructuring_borrower {
        if player.treasury() < Money::from_copper(1_000) {
            return;
        }
        borrower
    } else {
        if player.treasury() < lending_reserve || active_player_lending(state) >= lending_limit {
            return;
        }
        let Some(borrower) = eligible_lending_borrower(state) else {
            return;
        };
        borrower
    };
    let defaulted_loan = latest_defaulted_loan(state, state.player_dynasty_id, borrower.id());
    let opportunistic_new_credit =
        defaulted_loan.is_none() && persona == GameplayPersona::Opportunist;
    let principal = if defaulted_loan.is_some() {
        Money::from_copper((player.treasury().copper() / 14).clamp(1_000, 5_000))
    } else if opportunistic_new_credit {
        Money::from_copper((player.treasury().copper() / 8).clamp(1_500, 10_000))
    } else {
        Money::from_copper((player.treasury().copper() / 10).clamp(1_000, 8_000))
    };
    let repayment_balance =
        defaulted_loan.map_or(principal, |loan| loan.balance.saturating_add(principal));
    let amortization_weeks = if defaulted_loan.is_some() {
        AGENT_LOAN_AMORTIZATION_WEEKS.saturating_mul(2)
    } else if opportunistic_new_credit {
        AGENT_OPPORTUNIST_LOAN_AMORTIZATION_WEEKS
    } else {
        AGENT_LOAN_AMORTIZATION_WEEKS
    };
    let interest_basis_points = if defaulted_loan.is_some() {
        1_100
    } else if opportunistic_new_credit {
        AGENT_OPPORTUNIST_LOAN_INTEREST_BASIS_POINTS
    } else {
        900
    };
    let collateral = state.properties.values().find(|property| {
        property.owner_dynasty_id == Some(borrower.id())
            && property.collateral_loan_id.is_none()
            && repayment_balance >= property.value.saturating_mul_ratio(1, 5)
    });
    let base_bonus: i64 = match persona {
        GameplayPersona::PowerBroker => 430,
        GameplayPersona::Entrepreneur => 300,
        GameplayPersona::Opportunist => 520,
        GameplayPersona::Steward => 100,
    };
    let bonus = base_bonus.saturating_add(if defaulted_loan.is_some() { 1_400 } else { 0 });
    push_candidate(
        candidates,
        GameplayCommandKind::ExtendCredit,
        PlayerCommand::IssueLoan {
            terms: LoanTerms {
                lender_dynasty_id: state.player_dynasty_id,
                borrower_dynasty_id: borrower.id(),
                principal,
                weekly_payment: positive_money_ceil_div(repayment_balance, amortization_weeks),
                interest_basis_points,
                collateral_property_id: collateral.map(|property| property.id),
            },
        },
        defaulted_loan.map_or_else(
            || {
                if opportunistic_new_credit {
                    format!(
                        "offer a high-yield short-term loan of {principal} to dynasty {}",
                        borrower.id()
                    )
                } else {
                    format!("lend {principal} to dynasty {}", borrower.id())
                }
            },
            |loan| {
                format!(
                    "restructure defaulted loan {} with a {principal} recovery advance to dynasty {}",
                    loan.id,
                    borrower.id()
                )
            },
        ),
        bonus,
    );
}

fn generate_information_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let mut leverage_available = false;
    for report in state.information_reports.values().filter(|report| {
        report.owner_dynasty_id == state.player_dynasty_id
            && report.source == COMMISSIONED_INFORMATION_SOURCE
            && state.clock.day()
                >= report
                    .created_day
                    .saturating_add(AGENT_INFORMATION_LEVERAGE_DELAY_DAYS)
    }) {
        let Ok(quote) = quote_information_leverage(registry, state, report.id()) else {
            continue;
        };
        leverage_available = true;
        let bonus = match persona {
            GameplayPersona::Steward => 780,
            GameplayPersona::Entrepreneur => 860,
            GameplayPersona::PowerBroker => 900,
            GameplayPersona::Opportunist => 920,
        };
        push_candidate(
            candidates,
            GameplayCommandKind::LeverageInformation,
            PlayerCommand::LeverageInformation {
                report_id: quote.report_id,
            },
            quote.description,
            bonus,
        );
    }
    if leverage_available {
        return;
    }
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    if state.clock.day() < 180 || player.treasury() < INFORMATION_COMMISSION_COST {
        return;
    }
    let report_commission_day = state
        .information_reports
        .values()
        .filter(|report| {
            report.owner_dynasty_id == state.player_dynasty_id
                && report.source == COMMISSIONED_INFORMATION_SOURCE
        })
        .map(|report| report.created_day)
        .max();
    let audit_subject = format!("dynasty:{}", state.player_dynasty_id);
    let audit_commission_day = state
        .audit_log
        .iter()
        .filter(|record| {
            record.kind() == AuditKind::InformationCommission && record.subject() == audit_subject
        })
        .map(AuditRecord::day)
        .max();
    let commission_interval = if matches!(
        persona,
        GameplayPersona::PowerBroker | GameplayPersona::Opportunist
    ) && maximum_player_contract_relationship_pressure_basis_points(
        state,
        state.player_dynasty_id,
    )
        >= AGENT_INFORMATION_SEVERE_COUNTERPARTY_PRESSURE_BASIS_POINTS
    {
        INFORMATION_COMMISSION_INTERVAL_DAYS
    } else {
        AGENT_INFORMATION_COMMISSION_INTERVAL_DAYS
    };
    let available = report_commission_day
        .max(audit_commission_day)
        .is_none_or(|day| state.clock.day() >= day.saturating_add(commission_interval));
    if !available {
        return;
    }
    let Some((focus, description)) = preferred_information_focus(registry, state, persona) else {
        return;
    };
    let bonus: i64 = match persona {
        GameplayPersona::Steward => 420,
        GameplayPersona::Entrepreneur => 520,
        GameplayPersona::PowerBroker => 560,
        GameplayPersona::Opportunist => 480,
    };
    push_candidate(
        candidates,
        GameplayCommandKind::CommissionInformation,
        PlayerCommand::CommissionInformation { focus },
        description,
        bonus,
    );
}

fn preferred_information_focus(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
) -> Option<(InformationFocus, String)> {
    match persona {
        GameplayPersona::Entrepreneur => preferred_market_information_focus(registry, state),
        GameplayPersona::Steward => preferred_district_information_focus(registry, state),
        GameplayPersona::PowerBroker | GameplayPersona::Opportunist => {
            preferred_counterparty_information_focus(state, persona)
        }
    }
}

fn preferred_market_information_focus(
    registry: &Registry,
    state: &AppState,
) -> Option<(InformationFocus, String)> {
    let contract = state
        .contracts
        .values()
        .filter(|contract| player_external_contract(state, contract))
        .filter(|contract| market_information_is_material(state, contract))
        .max_by_key(|contract| market_information_priority(state, contract))?;
    let good = registry.get_good(contract.good_id)?;
    Some((
        InformationFocus::Market {
            good_id: contract.good_id,
        },
        format!("commission a market brief on {}", good.name()),
    ))
}

fn player_external_contract(state: &AppState, contract: &crate::core::SupplyContract) -> bool {
    if contract.status != ContractStatus::Active
        || contract.end_day < state.clock.day().saturating_add(60)
    {
        return false;
    }
    let buyer_is_player = state
        .businesses
        .get(contract.buyer_business_id)
        .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id);
    let seller_is_player = state
        .businesses
        .get(contract.seller_business_id)
        .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id);
    buyer_is_player != seller_is_player
}

fn market_information_priority(
    state: &AppState,
    contract: &crate::core::SupplyContract,
) -> (u64, i64, std::cmp::Reverse<crate::ids::GoodId>) {
    let (price_change, shortage) =
        state
            .market
            .quotes
            .get(&contract.good_id)
            .map_or((0, 0), |quote| {
                (
                    quote
                        .price
                        .copper()
                        .saturating_sub(quote.previous_price.copper())
                        .unsigned_abs(),
                    quote
                        .target_stock
                        .milliunits()
                        .saturating_sub(quote.stock.milliunits())
                        .max(0),
                )
            });
    (price_change, shortage, std::cmp::Reverse(contract.good_id))
}

fn market_information_is_material(
    state: &AppState,
    contract: &crate::core::SupplyContract,
) -> bool {
    state
        .market
        .quotes
        .get(&contract.good_id)
        .is_some_and(|quote| {
            let previous_price = quote.previous_price.copper().max(1).unsigned_abs();
            let price_change = quote
                .price
                .copper()
                .saturating_sub(quote.previous_price.copper())
                .unsigned_abs();
            let price_change_basis_points = price_change
                .saturating_mul(10_000)
                .checked_div(previous_price)
                .unwrap_or(u64::MAX);
            let target_stock = quote.target_stock.milliunits().max(1).unsigned_abs();
            let shortage = quote
                .target_stock
                .milliunits()
                .saturating_sub(quote.stock.milliunits())
                .max(0)
                .unsigned_abs();
            let shortage_basis_points = shortage
                .saturating_mul(10_000)
                .checked_div(target_stock)
                .unwrap_or(u64::MAX);
            let buyer_is_player = state
                .businesses
                .get(contract.buyer_business_id)
                .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id);
            let seller_is_player = state
                .businesses
                .get(contract.seller_business_id)
                .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id);
            let current_market_price = quote.price.copper().max(1);
            let adverse_contract_gap = if buyer_is_player && !seller_is_player {
                contract
                    .unit_price
                    .copper()
                    .saturating_sub(current_market_price)
            } else if seller_is_player && !buyer_is_player {
                current_market_price.saturating_sub(contract.unit_price.copper())
            } else {
                0
            }
            .max(0)
            .unsigned_abs();
            let adverse_contract_gap_basis_points = adverse_contract_gap
                .saturating_mul(10_000)
                .checked_div(current_market_price.unsigned_abs())
                .unwrap_or(u64::MAX);
            price_change_basis_points >= AGENT_INFORMATION_MARKET_PRICE_CHANGE_BASIS_POINTS
                || shortage_basis_points >= AGENT_INFORMATION_MARKET_SHORTAGE_BASIS_POINTS
                || adverse_contract_gap_basis_points
                    >= AGENT_INFORMATION_MARKET_CONTRACT_GAP_BASIS_POINTS
        })
}

fn preferred_district_information_focus(
    registry: &Registry,
    state: &AppState,
) -> Option<(InformationFocus, String)> {
    let (district_id, _) = state
        .districts
        .iter()
        .filter(|(_, district)| district_information_is_material(district))
        .max_by_key(|(district_id, district)| {
            (
                district_hardship(district),
                std::cmp::Reverse(**district_id),
            )
        })?;
    let district = registry.get_district(*district_id)?;
    Some((
        InformationFocus::District {
            district_id: *district_id,
        },
        format!("commission a district brief on {}", district.name()),
    ))
}

fn district_hardship(district: &crate::core::DistrictRuntime) -> u32 {
    u32::from(district.unrest_basis_points)
        .saturating_add(u32::from(
            10_000_u16.saturating_sub(district.employment_basis_points),
        ))
        .saturating_add(u32::from(
            10_000_u16.saturating_sub(district.sanitation_basis_points),
        ))
        .saturating_add(u32::from(
            10_000_u16.saturating_sub(district.safety_basis_points),
        ))
}

fn district_information_is_material(district: &crate::core::DistrictRuntime) -> bool {
    district.employment_basis_points < AGENT_INFORMATION_DISTRICT_CONDITION_THRESHOLD
        || district.sanitation_basis_points < AGENT_INFORMATION_DISTRICT_CONDITION_THRESHOLD
        || district.safety_basis_points < AGENT_INFORMATION_DISTRICT_CONDITION_THRESHOLD
        || district.unrest_basis_points >= AGENT_INFORMATION_DISTRICT_UNREST_THRESHOLD
}

fn preferred_counterparty_information_focus(
    state: &AppState,
    persona: GameplayPersona,
) -> Option<(InformationFocus, String)> {
    let relationship = state
        .relationships
        .values()
        .filter(|relationship| {
            relationship.pair.first == state.player_dynasty_id
                || relationship.pair.second == state.player_dynasty_id
        })
        .filter(|relationship| counterparty_information_is_material(state, relationship, persona))
        .max_by_key(|relationship| {
            counterparty_information_priority(state, relationship, persona)
        })?;
    let dynasty_id = relationship_counterparty_id(relationship, state.player_dynasty_id)?;
    let dynasty = state.dynasties.get(&dynasty_id)?;
    Some((
        InformationFocus::Counterparty { dynasty_id },
        format!("commission a house brief on House {}", dynasty.name()),
    ))
}

fn counterparty_information_is_material(
    state: &AppState,
    relationship: &crate::core::RelationshipState,
    persona: GameplayPersona,
) -> bool {
    let strained = relationship.trust_basis_points
        <= AGENT_INFORMATION_COUNTERPARTY_TRUST_THRESHOLD
        || relationship.resentment_basis_points
            >= AGENT_INFORMATION_COUNTERPARTY_RESENTMENT_THRESHOLD;
    if strained || persona != GameplayPersona::Opportunist {
        return strained;
    }
    let Some(counterparty_id) = relationship_counterparty_id(relationship, state.player_dynasty_id)
    else {
        return false;
    };
    let player_treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .map_or(Money::ZERO, crate::core::Dynasty::treasury);
    state
        .dynasties
        .get(&counterparty_id)
        .is_some_and(|dynasty| dynasty.treasury() >= player_treasury.saturating_mul(2))
}

fn counterparty_information_priority(
    state: &AppState,
    relationship: &crate::core::RelationshipState,
    persona: GameplayPersona,
) -> (u32, std::cmp::Reverse<DynastyId>) {
    let counterparty_id = relationship_counterparty_id(relationship, state.player_dynasty_id)
        .expect("filtered relationship must contain the player dynasty");
    let score = match persona {
        GameplayPersona::PowerBroker => u32::from(
            relationship
                .resentment_basis_points
                .saturating_add(10_000_u16.saturating_sub(relationship.trust_basis_points)),
        ),
        GameplayPersona::Opportunist => u32::try_from(
            state
                .dynasties
                .get(&counterparty_id)
                .map_or(0_i64, |dynasty| dynasty.treasury().copper())
                .max(0),
        )
        .unwrap_or(u32::MAX),
        GameplayPersona::Steward | GameplayPersona::Entrepreneur => 0,
    };
    (score, std::cmp::Reverse(counterparty_id))
}

fn relationship_counterparty_id(
    relationship: &crate::core::RelationshipState,
    player_id: DynastyId,
) -> Option<DynastyId> {
    if relationship.pair.first == player_id {
        Some(relationship.pair.second)
    } else if relationship.pair.second == player_id {
        Some(relationship.pair.first)
    } else {
        None
    }
}

fn maximum_player_contract_relationship_pressure_basis_points(
    state: &AppState,
    player_id: DynastyId,
) -> u16 {
    state
        .relationships
        .values()
        .filter_map(|relationship| {
            relationship_counterparty_id(relationship, player_id)
                .map(|dynasty_id| contract_relationship_pressure_basis_points(state, dynasty_id))
        })
        .max()
        .unwrap_or(0)
}

fn generate_civic_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    generate_law_candidates(registry, state, persona, candidates);
    generate_public_work_candidates(registry, state, persona, candidates);
    generate_legal_candidates(state, persona, candidates);
}

fn generate_law_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    if !has_player_office(state) {
        return;
    }
    let sponsorship_available = state
        .laws
        .values()
        .filter(|law| law.sponsor_dynasty_id == Some(state.player_dynasty_id))
        .map(|law| law.enacted_day)
        .max()
        .is_none_or(|day| state.clock.day() >= day.saturating_add(LAW_SPONSORSHIP_INTERVAL_DAYS));
    let has_legitimacy = state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_some_and(|dynasty| {
            dynasty.resources.legitimacy_basis_points >= LAW_LEGITIMACY_REQUIREMENT
        });
    if !sponsorship_available || !has_legitimacy {
        return;
    }
    if state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_none_or(|dynasty| dynasty.treasury() < Money::from_copper(2_000))
    {
        return;
    }
    let law_bonus: i64 = match persona {
        GameplayPersona::PowerBroker => 560,
        GameplayPersona::Steward => 260,
        GameplayPersona::Entrepreneur => 180,
        GameplayPersona::Opportunist => 140,
    };
    for (kind, value) in law_candidates(registry, state) {
        if !has_established_player_office_power(state, required_office_power_for_law(kind)) {
            continue;
        }
        if state
            .laws
            .values()
            .any(|law| law.active && law.kind == kind && law.value == value)
        {
            continue;
        }
        push_candidate(
            candidates,
            GameplayCommandKind::EnactLaw,
            PlayerCommand::EnactLaw { kind, value },
            format!("enact {kind:?} with value {value}"),
            law_bonus.saturating_add(law_persona_bonus(persona, kind)),
        );
    }
}

fn law_candidates(registry: &Registry, state: &AppState) -> Vec<(LawKind, i64)> {
    let bread_price = registry
        .get_good_id("bread")
        .and_then(|good_id| state.market.quotes.get(&good_id))
        .map_or(1, |quote| quote.price.copper())
        .max(1);
    let mut candidates = vec![
        (LawKind::BreadPriceCeiling, bread_price),
        (LawKind::ForeignMerchantToll, 600),
        (LawKind::InterestLimit, 800),
        (LawKind::FireCode, 7_000),
        (LawKind::RentRestriction, 900),
        (LawKind::GuildEntryRestriction, 1_200),
        (LawKind::EmergencyImports, 250),
    ];
    if let Some(principal) = civic_debt_candidate_principal(registry, state) {
        candidates.push((LawKind::PublicDebtAuthorization, principal.copper()));
    }
    candidates
}

fn civic_debt_candidate_principal(registry: &Registry, state: &AppState) -> Option<Money> {
    let treasury_id = registry.get_institution_id("treasury")?;
    let treasury = state.institutions.get(&treasury_id)?;
    let unsettled = state
        .civic_debts
        .values()
        .filter(|debt| debt.status != CivicDebtStatus::Repaid)
        .count();
    if treasury.budget >= Money::from_copper(50_000) || unsettled >= 2 {
        return None;
    }
    let principal = Money::from_copper(
        Money::from_copper(50_000)
            .saturating_sub(treasury.budget)
            .copper()
            .clamp(10_000, 100_000),
    );
    state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .any(|dynasty| {
            dynasty
                .treasury()
                .saturating_sub(CIVIC_DEBT_CREDITOR_RESERVE)
                >= principal
        })
        .then_some(principal)
}

fn law_persona_bonus(persona: GameplayPersona, kind: LawKind) -> i64 {
    match persona {
        GameplayPersona::Steward => match kind {
            LawKind::BreadPriceCeiling | LawKind::EmergencyImports => 220,
            LawKind::FireCode | LawKind::RentRestriction => 180,
            LawKind::PublicDebtAuthorization => 100,
            LawKind::ForeignMerchantToll
            | LawKind::InterestLimit
            | LawKind::GuildEntryRestriction => 0,
        },
        GameplayPersona::Entrepreneur => match kind {
            LawKind::ForeignMerchantToll => 180,
            LawKind::GuildEntryRestriction => -80,
            LawKind::BreadPriceCeiling
            | LawKind::InterestLimit
            | LawKind::FireCode
            | LawKind::RentRestriction
            | LawKind::EmergencyImports
            | LawKind::PublicDebtAuthorization => 0,
        },
        GameplayPersona::PowerBroker => match kind {
            LawKind::PublicDebtAuthorization => 260,
            LawKind::BreadPriceCeiling
            | LawKind::ForeignMerchantToll
            | LawKind::InterestLimit
            | LawKind::FireCode
            | LawKind::RentRestriction
            | LawKind::GuildEntryRestriction
            | LawKind::EmergencyImports => 120,
        },
        GameplayPersona::Opportunist => match kind {
            LawKind::InterestLimit => -100,
            LawKind::ForeignMerchantToll => 160,
            LawKind::BreadPriceCeiling
            | LawKind::FireCode
            | LawKind::RentRestriction
            | LawKind::GuildEntryRestriction
            | LawKind::EmergencyImports
            | LawKind::PublicDebtAuthorization => 0,
        },
    }
}

fn generate_public_work_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    if !has_established_player_office_power(state, OfficePower::PublicWorks) {
        return;
    }
    let active_sponsored = state
        .public_works
        .values()
        .filter(|work| {
            work.sponsor_dynasty_id == Some(state.player_dynasty_id)
                && matches!(
                    work.status,
                    PublicWorkStatus::Building | PublicWorkStatus::Suspended
                )
        })
        .count();
    if active_sponsored >= MAX_ACTIVE_SPONSORED_PUBLIC_WORKS {
        return;
    }
    let subject = format!("dynasty:{}", state.player_dynasty_id);
    let sponsorship_available = state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::PublicWorkStarted && record.subject() == subject)
        .is_none_or(|record| {
            state.clock.day()
                >= record
                    .day()
                    .saturating_add(PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS)
        });
    if !sponsorship_available {
        return;
    }
    if state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_none_or(|dynasty| dynasty.treasury() < Money::from_copper(1_200))
    {
        return;
    }
    let bonus = match persona {
        GameplayPersona::PowerBroker => 520,
        GameplayPersona::Steward => 440,
        GameplayPersona::Entrepreneur => 180,
        GameplayPersona::Opportunist => 60,
    };
    for district in registry.districts() {
        let runtime = state
            .districts
            .get(&district.id())
            .expect("district runtime must exist");
        let kind = weakest_district_work(runtime);
        if state.public_works.values().any(|work| {
            work.district_id == district.id()
                && work.kind == kind
                && matches!(
                    work.status,
                    PublicWorkStatus::Building | PublicWorkStatus::Suspended
                )
        }) {
            continue;
        }
        push_candidate(
            candidates,
            GameplayCommandKind::StartPublicWork,
            PlayerCommand::StartPublicWork {
                district_id: district.id(),
                kind,
                budget: Money::from_copper(12_000),
            },
            format!("start {kind:?} in {}", district.name()),
            bonus,
        );
    }
}

fn weakest_district_work(district: &crate::core::DistrictRuntime) -> PublicWorkKind {
    if district.sanitation_basis_points <= district.safety_basis_points
        && district.sanitation_basis_points <= district.employment_basis_points
    {
        PublicWorkKind::Drainage
    } else if district.safety_basis_points <= district.employment_basis_points {
        PublicWorkKind::WatchStation
    } else {
        PublicWorkKind::Market
    }
}

fn has_player_office(state: &AppState) -> bool {
    state.institutions.values().any(|institution| {
        institution.office_holder_id.is_some_and(|character_id| {
            state
                .characters
                .get(character_id)
                .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
        })
    })
}

fn generate_legal_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    if state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_none_or(|dynasty| dynasty.treasury() < LEGAL_CASE_FILING_COST)
    {
        return;
    }
    let filing_available = state
        .legal_cases
        .values()
        .filter(|legal_case| legal_case.plaintiff_dynasty_id == state.player_dynasty_id)
        .map(|legal_case| legal_case.filed_day)
        .max()
        .is_none_or(|last_filing_day| {
            state.clock.day() >= last_filing_day.saturating_add(LEGAL_CASE_FILING_INTERVAL_DAYS)
        });
    if !filing_available {
        return;
    }
    let bonus = match persona {
        GameplayPersona::PowerBroker => 480,
        GameplayPersona::Opportunist => 420,
        GameplayPersona::Entrepreneur => 180,
        GameplayPersona::Steward => 80,
    };
    for (defendant, kind) in state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .filter_map(|defendant| {
            legal_grievance_kind(state, defendant.id()).map(|kind| (defendant, kind))
        })
        .take(3)
    {
        if state.legal_cases.values().any(|case| {
            case.plaintiff_dynasty_id == state.player_dynasty_id
                && case.defendant_dynasty_id == defendant.id()
                && case.kind == kind
                && matches!(
                    case.status,
                    LegalCaseStatus::Filed | LegalCaseStatus::Hearing
                )
        }) {
            continue;
        }
        push_candidate(
            candidates,
            GameplayCommandKind::FileLegalCase,
            PlayerCommand::FileLegalCase {
                defendant_dynasty_id: defendant.id(),
                kind,
                evidence_basis_points: 7_200,
                damages: Money::from_copper(3_000),
            },
            format!("file {kind:?} case against dynasty {}", defendant.id()),
            bonus,
        );
    }
}

fn legal_grievance_kind(state: &AppState, defendant_id: DynastyId) -> Option<LegalCaseKind> {
    let player_id = state.player_dynasty_id;
    let already_litigated = |kind| {
        state.legal_cases.values().any(|legal_case| {
            legal_case.plaintiff_dynasty_id == player_id
                && legal_case.defendant_dynasty_id == defendant_id
                && legal_case.kind == kind
        })
    };
    if !already_litigated(LegalCaseKind::Debt)
        && state.loans.values().any(|loan| {
            loan.lender_dynasty_id == player_id
                && loan.borrower_dynasty_id == defendant_id
                && matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted)
        })
    {
        return Some(LegalCaseKind::Debt);
    }
    let player_businesses: BTreeSet<_> = state
        .businesses
        .ids_for_owner(player_id)
        .into_iter()
        .flatten()
        .copied()
        .collect();
    if !already_litigated(LegalCaseKind::ContractBreach)
        && state.contracts.values().any(|contract| {
            contract.status == ContractStatus::Breached
                && contract.breaching_dynasty_id == Some(defendant_id)
                && (player_businesses.contains(&contract.buyer_business_id)
                    || player_businesses.contains(&contract.seller_business_id))
        })
    {
        return Some(LegalCaseKind::ContractBreach);
    }
    None
}

fn generate_family_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let council = state
        .family_councils
        .get(&state.player_dynasty_id)
        .expect("player family council must exist");
    let governance_subject = format!("dynasty:{}", state.player_dynasty_id);
    let governance_available = state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::HouseGovernanceChange
                && record.subject() == governance_subject
        })
        .is_none_or(|record| {
            record
                .day()
                .saturating_add(HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS)
                <= state.clock.day()
        });
    if governance_available
        && let Some(governance) = preferred_house_governance(state, persona)
        && governance != council.governance
    {
        push_candidate(
            candidates,
            GameplayCommandKind::SetHouseGovernance,
            PlayerCommand::SetHouseGovernance { governance },
            format!("adopt {governance:?} governance to address current family pressure"),
            governance_bonus(persona, governance),
        );
    }
    generate_family_council_candidate(state, persona, candidates);
    generate_heir_designation_candidates(state, persona, candidates);
    generate_ward_adoption_candidates(state, persona, candidates);
    generate_family_education_candidates(state, persona, candidates);
    generate_institution_withdrawal_candidates(state, persona, candidates);
    generate_office_power_directive_candidates(registry, state, persona, candidates);
    generate_institution_ascent_candidates(registry, state, persona, candidates);
}

fn generate_family_council_candidate(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let dynasty_id = state.player_dynasty_id;
    let Some(council) = state.family_councils.get(&dynasty_id) else {
        return;
    };
    if council.unity_basis_points >= FAMILY_COUNCIL_INTERVENTION_UNITY_THRESHOLD
        || state
            .dynasties
            .get(&dynasty_id)
            .is_none_or(|dynasty| dynasty.treasury() < FAMILY_COUNCIL_MEETING_COST)
    {
        return;
    }
    let subject = format!("dynasty:{dynasty_id};council-meeting");
    let available = state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::HouseGovernanceChange && record.subject() == subject
        })
        .is_none_or(|record| {
            record
                .day()
                .saturating_add(FAMILY_COUNCIL_MEETING_INTERVAL_DAYS)
                <= state.clock.day()
        });
    if !available {
        return;
    }
    let pressure_bonus = i64::from(
        FAMILY_COUNCIL_INTERVENTION_UNITY_THRESHOLD.saturating_sub(council.unity_basis_points) / 50,
    );
    let persona_bonus = match persona {
        GameplayPersona::Steward => 30,
        GameplayPersona::PowerBroker => 20,
        GameplayPersona::Entrepreneur => 15,
        GameplayPersona::Opportunist => 10,
    };
    push_candidate(
        candidates,
        GameplayCommandKind::ConveneFamilyCouncil,
        PlayerCommand::ConveneFamilyCouncil,
        format!(
            "convene the family council at {} bp unity to reconcile claims and obligations",
            council.unity_basis_points
        ),
        55_i64
            .saturating_add(pressure_bonus)
            .saturating_add(persona_bonus),
    );
}

fn preferred_house_governance(
    state: &AppState,
    persona: GameplayPersona,
) -> Option<HouseGovernance> {
    let dynasty = state.dynasties.get(&state.player_dynasty_id)?;
    let council = state.family_councils.get(&state.player_dynasty_id)?;
    let active_members = council
        .members
        .iter()
        .filter(|character_id| {
            state
                .characters
                .get(**character_id)
                .is_some_and(|character| character.status() == CharacterStatus::Active)
        })
        .count();
    let administrative_load = dynasty.administrative_load().saturating_add(
        crate::systems::dynasty_office_administrative_load(state, dynasty.id()),
    );
    let overextended = administrative_load > dynasty.administrative_capacity();
    let head_age = state.characters.get(dynasty.head_id()).map_or(0, |head| {
        state.clock.day().saturating_sub(head.birth_day()) / 360
    });
    if council.unity_basis_points < 5_500 {
        return Some(HouseGovernance::FamilyPartnership);
    }
    if overextended && active_members >= 4 {
        return Some(HouseGovernance::BranchFederation);
    }
    if head_age >= 50 || dynasty.runtime.succession_risk_basis_points >= 2_500 {
        return Some(HouseGovernance::Primogeniture);
    }
    Some(match persona {
        GameplayPersona::Entrepreneur if active_members >= 4 => HouseGovernance::BranchFederation,
        GameplayPersona::Steward | GameplayPersona::Entrepreneur => {
            HouseGovernance::FamilyPartnership
        }
        GameplayPersona::PowerBroker => HouseGovernance::Primogeniture,
        GameplayPersona::Opportunist if overextended => HouseGovernance::BranchFederation,
        GameplayPersona::Opportunist => HouseGovernance::HeadCommand,
    })
}

fn generate_heir_designation_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let dynasty = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    if dynasty.resources.legitimacy_basis_points < HEIR_DESIGNATION_LEGITIMACY_COST {
        return;
    }
    let designation_subject = format!("dynasty:{}", state.player_dynasty_id);
    let last_designation_day = state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::HeirDesignation && record.subject() == designation_subject
        })
        .map(AuditRecord::day);
    let designation_available = last_designation_day.is_none_or(|last_day| {
        state.clock.day() >= last_day.saturating_add(HEIR_DESIGNATION_INTERVAL_DAYS)
    });
    if !designation_available {
        return;
    }
    let Some(current_heir_id) = dynasty.heir_id() else {
        return;
    };
    let Some(current_heir) = state.characters.get(current_heir_id) else {
        return;
    };
    let (head_age, head_health) = character_age_and_health(state, dynasty.head_id());
    if head_age < 48 && dynasty.runtime.succession_risk_basis_points < 2_000 {
        return;
    }
    let current_score = successor_score(current_heir, persona);
    let current_primary = successor_primary_capability(current_heir, persona);
    let council = state
        .family_councils
        .get(&state.player_dynasty_id)
        .expect("player family council must exist");
    let replacement = council
        .members
        .iter()
        .filter_map(|character_id| state.characters.get(*character_id))
        .filter(|character| {
            character.id() != dynasty.head_id()
                && character.id() != current_heir_id
                && character.status() == CharacterStatus::Active
                && state.clock.day().saturating_sub(character.birth_day()) >= 18 * 360
        })
        .max_by_key(|character| {
            (
                successor_primary_capability(character, persona),
                successor_score(character, persona),
                character.id(),
            )
        });
    if let Some(replacement) = replacement {
        let replacement_score = successor_score(replacement, persona);
        let replacement_primary = successor_primary_capability(replacement, persona);
        let broadly_superior = replacement_score >= current_score.saturating_add(20);
        let strategically_specialized = replacement_primary >= current_primary.saturating_add(5);
        if broadly_superior || strategically_specialized {
            push_candidate(
                candidates,
                GameplayCommandKind::DesignateHeir,
                PlayerCommand::DesignateHeir {
                    character_id: replacement.id(),
                },
                format!(
                    "designate character {} as heir for the {persona:?} succession strategy",
                    replacement.id()
                ),
                1_000_i64.saturating_add(head_age.saturating_sub(47).saturating_mul(20)),
            );
            return;
        }
    }
    let current_heir_is_eligible = current_heir.status() == CharacterStatus::Active
        && state.clock.day().saturating_sub(current_heir.birth_day()) >= 18 * 360
        && council.members.contains(&current_heir_id);
    let confirmation_pressure = head_age >= HEIR_CONFIRMATION_HEAD_AGE_YEARS
        || head_health <= HEIR_CONFIRMATION_HEALTH_THRESHOLD;
    if last_designation_day.is_some() || !current_heir_is_eligible || !confirmation_pressure {
        return;
    }
    push_candidate(
        candidates,
        GameplayCommandKind::DesignateHeir,
        PlayerCommand::DesignateHeir {
            character_id: current_heir_id,
        },
        format!(
            "formally confirm character {current_heir_id} as heir for the {persona:?} succession strategy"
        ),
        900_i64.saturating_add(head_age.saturating_sub(47).saturating_mul(20)),
    );
}

fn character_age_and_health(state: &AppState, character_id: CharacterId) -> (i64, u16) {
    state
        .characters
        .get(character_id)
        .map_or((0, 10_000), |character| {
            (
                state.clock.day().saturating_sub(character.birth_day()) / 360,
                character.runtime.health_basis_points,
            )
        })
}

const fn successor_primary_capability(
    character: &crate::core::Character,
    persona: GameplayPersona,
) -> u16 {
    match persona {
        GameplayPersona::Steward => character.capabilities.administration,
        GameplayPersona::Entrepreneur => character.capabilities.commerce,
        GameplayPersona::PowerBroker => character.capabilities.social,
        GameplayPersona::Opportunist => character.capabilities.craft,
    }
}

fn successor_score(character: &crate::core::Character, persona: GameplayPersona) -> i64 {
    let capabilities = &character.capabilities;
    let loyalty = i64::from(character.runtime.loyalty_basis_points / 50);
    match persona {
        GameplayPersona::Steward => {
            i64::from(capabilities.administration) * 4
                + i64::from(capabilities.social) * 2
                + i64::from(capabilities.commerce)
                + loyalty
        }
        GameplayPersona::Entrepreneur => {
            i64::from(capabilities.commerce) * 4
                + i64::from(capabilities.administration) * 2
                + i64::from(capabilities.craft)
                + loyalty
        }
        GameplayPersona::PowerBroker => {
            i64::from(capabilities.social) * 4
                + i64::from(capabilities.administration) * 2
                + i64::from(capabilities.commerce)
                + loyalty
        }
        GameplayPersona::Opportunist => {
            i64::from(capabilities.administration) * 2
                + i64::from(capabilities.commerce) * 2
                + i64::from(capabilities.social) * 2
                + i64::from(capabilities.craft) * 2
                + loyalty
        }
    }
}

fn eligible_office_characters(state: &AppState) -> Vec<&crate::core::Character> {
    state
        .characters
        .iter()
        .filter(|character| {
            character.dynasty_id() == state.player_dynasty_id
                && character.status() == CharacterStatus::Active
                && !state
                    .institutions
                    .values()
                    .any(|institution| institution.office_holder_id == Some(character.id()))
        })
        .collect()
}

fn player_controlled_office_powers(state: &AppState) -> BTreeSet<OfficePower> {
    state
        .institutions
        .values()
        .filter(|institution| {
            institution.office_holder_id.is_some_and(|character_id| {
                state
                    .characters
                    .get(character_id)
                    .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
            })
        })
        .flat_map(|institution| institution.powers.iter().copied())
        .collect()
}

fn institution_is_strategic_target(
    state: &AppState,
    institution: &crate::core::InstitutionRuntime,
    controlled_powers: &BTreeSet<OfficePower>,
    player_has_institutional_foothold: bool,
    persona: GameplayPersona,
) -> bool {
    let held_by_player = institution.office_holder_id.is_some_and(|character_id| {
        state
            .characters
            .get(character_id)
            .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
    });
    let represented_by_player = institution.members.iter().any(|character_id| {
        state
            .characters
            .get(*character_id)
            .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
    });
    !held_by_player
        && (represented_by_player
            || !player_has_institutional_foothold
            || institution.powers.iter().any(|power| {
                !controlled_powers.contains(power)
                    && office_power_persona_bonus(persona, *power) > 0
            }))
}

fn office_power_directive_available(state: &AppState, institution_id: InstitutionId) -> bool {
    let subject = format!("institution:{institution_id}");
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::OfficeDirective && record.subject() == subject)
        .is_none_or(|record| {
            state.clock.day()
                >= record
                    .day()
                    .saturating_add(OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS)
        })
}

fn district_food_satisfaction(state: &AppState, district_id: DistrictId) -> u16 {
    let (total, count) = state
        .households
        .ids_for_district(district_id)
        .into_iter()
        .flatten()
        .filter_map(|household_id| state.households.get(*household_id))
        .fold((0_u64, 0_u64), |(total, count), household| {
            (
                total.saturating_add(u64::from(household.food_satisfaction_basis_points())),
                count.saturating_add(1),
            )
        });
    total
        .checked_div(count)
        .and_then(|average| u16::try_from(average).ok())
        .unwrap_or(10_000)
}

fn office_power_need_bonus(
    state: &AppState,
    institution_id: InstitutionId,
    district_id: DistrictId,
    power: OfficePower,
) -> i64 {
    let district = state
        .districts
        .get(&district_id)
        .expect("institution district must exist");
    match power {
        OfficePower::Licenses => {
            i64::from(6_500_u16.saturating_sub(district.employment_basis_points))
        }
        OfficePower::Inspections => {
            i64::from(6_500_u16.saturating_sub(district.sanitation_basis_points))
        }
        OfficePower::MarketTolls | OfficePower::Taxation => state
            .institutions
            .get(&institution_id)
            .map_or(0, |institution| {
                i64::from(6_500_u16.saturating_sub(institution.legitimacy_basis_points))
            }),
        OfficePower::DebtEnforcement => {
            if state.loans.values().any(|loan| {
                (loan.lender_dynasty_id == state.player_dynasty_id
                    || loan.borrower_dynasty_id == state.player_dynasty_id)
                    && matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted)
            }) {
                1_800
            } else {
                0
            }
        }
        OfficePower::CityContracts => {
            if state.businesses.iter().any(|business| {
                business.owner_dynasty_id() == state.player_dynasty_id
                    && (business.status() == BusinessStatus::Distressed
                        || business.cash() < Money::from_copper(5_000))
            }) {
                1_500
            } else {
                0
            }
        }
        OfficePower::PublicWorks => {
            i64::from(6_500_u16.saturating_sub(district.employment_basis_points)).saturating_add(
                i64::from(6_500_u16.saturating_sub(district.sanitation_basis_points)),
            )
        }
        OfficePower::WatchPriorities => {
            i64::from(6_500_u16.saturating_sub(district.safety_basis_points))
                .saturating_add(i64::from(district.unrest_basis_points / 2))
        }
        OfficePower::EmergencyImports => {
            let crisis_pressure = state.crises.values().any(|crisis| {
                crisis.status.is_active()
                    && matches!(
                        crisis.kind,
                        CrisisKind::GrainShortage | CrisisKind::Epidemic
                    )
            });
            let food_pressure =
                7_000_u16.saturating_sub(district_food_satisfaction(state, district_id));
            i64::from(food_pressure).saturating_add(if crisis_pressure { 2_000 } else { 0 })
        }
    }
}

fn generate_office_power_directive_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    if player.resources.legitimacy_basis_points < OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST {
        return;
    }
    for institution in state.institutions.values() {
        let held_by_player = institution.office_holder_id.is_some_and(|character_id| {
            state.characters.get(character_id).is_some_and(|character| {
                character.status() == CharacterStatus::Active
                    && character.dynasty_id() == state.player_dynasty_id
            })
        });
        if !held_by_player || !office_power_directive_available(state, institution.institution_id) {
            continue;
        }
        if state.clock.day()
            < institution
                .term_started_day
                .saturating_add(OFFICE_POWER_ESTABLISHMENT_DAYS)
        {
            continue;
        }
        let district_id = registry
            .get_institution(institution.institution_id)
            .expect("runtime institution must have a definition")
            .district_id();
        let selected = institution
            .powers
            .iter()
            .copied()
            .map(|power| {
                let need =
                    office_power_need_bonus(state, institution.institution_id, district_id, power);
                let priority = office_power_persona_bonus(persona, power).saturating_add(need);
                (need, priority, power)
            })
            .filter(|(need, priority, _)| *need > 0 && *priority > 0)
            .max_by_key(|(_, priority, power)| (*priority, *power));
        let Some((_, priority, power)) = selected else {
            continue;
        };
        push_candidate(
            candidates,
            GameplayCommandKind::ExerciseOfficePower,
            PlayerCommand::ExerciseOfficePower {
                institution_id: institution.institution_id,
                power,
            },
            format!(
                "exercise {power:?} through institution {} to shape district {district_id}",
                institution.institution_id
            ),
            priority,
        );
    }
}

fn generate_institution_ascent_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let (support_bonus, nomination_bonus) = institution_ascent_bonuses(persona);
    let characters = eligible_office_characters(state);
    let controlled_powers = player_controlled_office_powers(state);
    let player_has_institutional_foothold = has_player_institutional_foothold(state);
    for institution in state.institutions.values() {
        if !institution_is_strategic_target(
            state,
            institution,
            &controlled_powers,
            player_has_institutional_foothold,
            persona,
        ) {
            continue;
        }
        let institution_kind = registry
            .get_institution(institution.institution_id)
            .expect("runtime institution must have a registry definition")
            .kind();
        let has_available_player_member = characters
            .iter()
            .any(|character| institution.members.contains(&character.id()));
        let strongest_character = (!has_available_player_member)
            .then(|| {
                characters
                    .iter()
                    .copied()
                    .filter(|character| {
                        is_institution_support_available(state, character.id())
                            && institution_membership_count(state, character.id())
                                < MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER
                            && institution_support_day(
                                state,
                                institution.institution_id,
                                character.id(),
                            )
                            .is_none()
                    })
                    .max_by_key(|character| {
                        (
                            institution_capability_score(character, institution_kind),
                            std::cmp::Reverse(character.id()),
                        )
                    })
            })
            .flatten();
        let power_bonus = institution_power_bonus(persona, &institution.powers);
        if let Some(character) = strongest_character {
            push_candidate(
                candidates,
                GameplayCommandKind::CultivateInstitutionSupport,
                PlayerCommand::CultivateInstitutionSupport {
                    institution_id: institution.institution_id,
                    character_id: character.id(),
                },
                format!(
                    "cultivate support for character {} in institution {}",
                    character.id(),
                    institution.institution_id
                ),
                support_bonus.saturating_add(power_bonus),
            );
        }
        let nominee = characters
            .iter()
            .copied()
            .filter(|character| is_office_nomination_available(state, character.id()))
            .filter(|character| institution.members.contains(&character.id()))
            .filter(|character| {
                institution_support_day(state, institution.institution_id, character.id())
                    .is_some_and(|day| {
                        state.clock.day()
                            >= day.saturating_add(INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS)
                    })
            })
            .max_by_key(|character| {
                (
                    institution_capability_score(character, institution_kind),
                    std::cmp::Reverse(character.id()),
                )
            });
        if let Some(character) = nominee {
            push_candidate(
                candidates,
                GameplayCommandKind::NominateForOffice,
                PlayerCommand::NominateForOffice {
                    institution_id: institution.institution_id,
                    character_id: character.id(),
                },
                format!(
                    "nominate character {} to institution {}",
                    character.id(),
                    institution.institution_id
                ),
                nomination_bonus.saturating_add(power_bonus),
            );
        }
    }
}

const fn institution_ascent_bonuses(persona: GameplayPersona) -> (i64, i64) {
    match persona {
        GameplayPersona::PowerBroker => (850, 620),
        GameplayPersona::Steward => (420, 170),
        GameplayPersona::Entrepreneur => (260, 130),
        GameplayPersona::Opportunist => (540, 260),
    }
}

fn has_player_institutional_foothold(state: &AppState) -> bool {
    state.institutions.values().any(|institution| {
        institution.members.iter().any(|character_id| {
            state
                .characters
                .get(*character_id)
                .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
        })
    })
}

fn generate_institution_withdrawal_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    if !has_institution_withdrawal_opportunity(state) {
        return;
    }
    let recent_shortfall = has_recent_player_office_duty_shortfall(state);
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let monthly_cost = player_current_office_duty_cost(state);
    let severe_liquidity = treasury < monthly_cost.saturating_mul(3);
    let business_distress = player_has_severe_business_distress(state);
    let persona_bonus: i64 = match persona {
        GameplayPersona::Steward => -100,
        GameplayPersona::Entrepreneur => 200,
        GameplayPersona::PowerBroker => -200,
        GameplayPersona::Opportunist => 350,
    };
    let urgency: i64 = if recent_shortfall {
        2_400
    } else if severe_liquidity {
        1_800
    } else if business_distress {
        1_200
    } else {
        700
    };
    for institution in state.institutions.values() {
        let Some(character_id) = institution.office_holder_id else {
            continue;
        };
        if state
            .characters
            .get(character_id)
            .is_none_or(|character| character.dynasty_id() != state.player_dynasty_id)
        {
            continue;
        }
        push_candidate(
            candidates,
            GameplayCommandKind::WithdrawFromInstitution,
            PlayerCommand::WithdrawFromInstitution {
                institution_id: institution.institution_id,
                character_id,
            },
            format!(
                "withdraw character {character_id} from institution {} and surrender its office",
                institution.institution_id
            ),
            urgency.saturating_add(persona_bonus),
        );
    }
}

fn player_current_office_duty_cost(state: &AppState) -> Money {
    projected_dynasty_monthly_office_duty(state, state.player_dynasty_id, 0)
}

fn player_has_severe_business_distress(state: &AppState) -> bool {
    state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == state.player_dynasty_id
            && (matches!(
                business.status(),
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            ) || business.operations.condition_basis_points < 2_000)
    })
}

fn has_recent_player_office_duty_shortfall(state: &AppState) -> bool {
    state.audit_log.iter().rev().any(|record| {
        record.kind() == AuditKind::OfficeDutyShortfall
            && audit_subject_has_dynasty(record.audit_subject(), state.player_dynasty_id)
            && state.clock.day().saturating_sub(record.day()) <= 180
    })
}

fn has_institution_withdrawal_opportunity(state: &AppState) -> bool {
    let monthly_cost = player_current_office_duty_cost(state);
    if monthly_cost == Money::ZERO {
        return false;
    }
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    has_recent_player_office_duty_shortfall(state)
        || treasury < monthly_cost.saturating_mul(6)
        || (player_has_severe_business_distress(state)
            && treasury
                < monthly_cost
                    .saturating_mul(AGENT_OFFICE_DUTY_RESERVE_MONTHS)
                    .saturating_add(AGENT_OFFICE_LIQUIDITY_BUFFER))
}

fn generate_ward_adoption_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let adoption_available = player.treasury() >= WARD_ADOPTION_COST
        && player.resources.legitimacy_basis_points >= WARD_ADOPTION_LEGITIMACY_REQUIREMENT
        && player
            .resources
            .reputation_quality_basis_points
            .max(player.resources.reputation_reliability_basis_points)
            >= WARD_ADOPTION_REPUTATION_REQUIREMENT
        && player_contract_deliveries(state) >= WARD_ADOPTION_DELIVERY_REQUIREMENT
        && usize::from(count_active_player_wards(state, state.player_dynasty_id))
            < MAX_ACTIVE_WARDS
        && state
            .audit_log
            .iter()
            .rev()
            .find(|record| record.kind() == AuditKind::WardAdoption)
            .is_none_or(|record| {
                state.clock.day() >= record.day().saturating_add(WARD_ADOPTION_INTERVAL_DAYS)
            });
    if !adoption_available {
        return;
    }
    let base_bonus: i64 = match persona {
        GameplayPersona::PowerBroker => 620,
        GameplayPersona::Steward => 500,
        GameplayPersona::Opportunist => 420,
        GameplayPersona::Entrepreneur => 360,
    };
    for focus in education_focus_order(persona) {
        push_candidate(
            candidates,
            GameplayCommandKind::AdoptWard,
            PlayerCommand::AdoptWard { focus },
            format!("adopt a {focus:?}-focused ward into the dynasty"),
            base_bonus.saturating_add(education_focus_persona_bonus(persona, focus)),
        );
    }
}

fn generate_family_education_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let education_available = player.treasury() >= FAMILY_EDUCATION_COST
        && player
            .resources
            .reputation_quality_basis_points
            .max(player.resources.reputation_reliability_basis_points)
            >= COMMERCIAL_STANDING_REPUTATION_REQUIREMENT
        && player_contract_deliveries(state) >= OFFICE_NOMINATION_DELIVERY_REQUIREMENT
        && state
            .audit_log
            .iter()
            .rev()
            .find(|record| record.kind() == AuditKind::FamilyEducation)
            .is_none_or(|record| {
                state.clock.day() >= record.day().saturating_add(FAMILY_EDUCATION_INTERVAL_DAYS)
            });
    if !education_available {
        return;
    }
    let base_bonus: i64 = match persona {
        GameplayPersona::PowerBroker => 480,
        GameplayPersona::Entrepreneur => 430,
        GameplayPersona::Steward => 400,
        GameplayPersona::Opportunist => 320,
    };
    let active_characters: Vec<_> = state
        .characters
        .iter()
        .filter(|character| {
            character.dynasty_id() == state.player_dynasty_id
                && character.status() == CharacterStatus::Active
        })
        .collect();
    for focus in education_focus_order(persona) {
        let student = active_characters
            .iter()
            .filter(|character| character_focus_value(character, focus) < 100)
            .min_by_key(|character| (character_focus_value(character, focus), character.id()));
        let Some(student) = student else {
            continue;
        };
        push_candidate(
            candidates,
            GameplayCommandKind::EducateFamilyMember,
            PlayerCommand::EducateFamilyMember {
                character_id: student.id(),
                focus,
            },
            format!("educate character {} in {focus:?}", student.id()),
            base_bonus.saturating_add(education_focus_persona_bonus(persona, focus)),
        );
    }
}

const fn education_focus_order(persona: GameplayPersona) -> [EducationFocus; 4] {
    match persona {
        GameplayPersona::Steward => [
            EducationFocus::Administration,
            EducationFocus::Social,
            EducationFocus::Commerce,
            EducationFocus::Craft,
        ],
        GameplayPersona::Entrepreneur => [
            EducationFocus::Commerce,
            EducationFocus::Administration,
            EducationFocus::Craft,
            EducationFocus::Social,
        ],
        GameplayPersona::PowerBroker => [
            EducationFocus::Social,
            EducationFocus::Administration,
            EducationFocus::Commerce,
            EducationFocus::Craft,
        ],
        GameplayPersona::Opportunist => [
            EducationFocus::Commerce,
            EducationFocus::Social,
            EducationFocus::Administration,
            EducationFocus::Craft,
        ],
    }
}

const fn education_focus_persona_bonus(persona: GameplayPersona, focus: EducationFocus) -> i64 {
    match persona {
        GameplayPersona::Steward => match focus {
            EducationFocus::Administration => 260,
            EducationFocus::Social => 140,
            EducationFocus::Commerce | EducationFocus::Craft => 0,
        },
        GameplayPersona::Entrepreneur => match focus {
            EducationFocus::Commerce => 260,
            EducationFocus::Administration => 140,
            EducationFocus::Social | EducationFocus::Craft => 0,
        },
        GameplayPersona::PowerBroker => match focus {
            EducationFocus::Social => 260,
            EducationFocus::Administration => 140,
            EducationFocus::Commerce | EducationFocus::Craft => 0,
        },
        GameplayPersona::Opportunist => match focus {
            EducationFocus::Commerce => 260,
            EducationFocus::Social => 140,
            EducationFocus::Administration | EducationFocus::Craft => 0,
        },
    }
}

const fn character_focus_value(character: &crate::core::Character, focus: EducationFocus) -> u16 {
    match focus {
        EducationFocus::Administration => character.capabilities.administration,
        EducationFocus::Commerce => character.capabilities.commerce,
        EducationFocus::Social => character.capabilities.social,
        EducationFocus::Craft => character.capabilities.craft,
    }
}

const fn office_power_persona_bonus(persona: GameplayPersona, power: OfficePower) -> i64 {
    match persona {
        GameplayPersona::Steward => match power {
            OfficePower::PublicWorks => 500,
            OfficePower::EmergencyImports => 420,
            OfficePower::Inspections => 300,
            OfficePower::Licenses
            | OfficePower::MarketTolls
            | OfficePower::DebtEnforcement
            | OfficePower::CityContracts
            | OfficePower::WatchPriorities
            | OfficePower::Taxation => 0,
        },
        GameplayPersona::Entrepreneur => match power {
            OfficePower::MarketTolls => 500,
            OfficePower::Licenses => 420,
            OfficePower::CityContracts => 360,
            OfficePower::Inspections
            | OfficePower::DebtEnforcement
            | OfficePower::PublicWorks
            | OfficePower::WatchPriorities
            | OfficePower::Taxation
            | OfficePower::EmergencyImports => 0,
        },
        GameplayPersona::PowerBroker => match power {
            OfficePower::Taxation => 500,
            OfficePower::PublicWorks => 440,
            OfficePower::DebtEnforcement => 400,
            OfficePower::Licenses
            | OfficePower::Inspections
            | OfficePower::MarketTolls
            | OfficePower::CityContracts
            | OfficePower::WatchPriorities
            | OfficePower::EmergencyImports => 0,
        },
        GameplayPersona::Opportunist => match power {
            OfficePower::DebtEnforcement => 500,
            OfficePower::MarketTolls => 420,
            OfficePower::WatchPriorities => 360,
            OfficePower::Licenses
            | OfficePower::Inspections
            | OfficePower::CityContracts
            | OfficePower::PublicWorks
            | OfficePower::Taxation
            | OfficePower::EmergencyImports => 0,
        },
    }
}

fn institution_power_bonus(persona: GameplayPersona, powers: &BTreeSet<OfficePower>) -> i64 {
    powers
        .iter()
        .map(|power| office_power_persona_bonus(persona, *power))
        .max()
        .unwrap_or(0)
}

fn is_office_nomination_available(state: &AppState, character_id: CharacterId) -> bool {
    let Some(player) = state.dynasties.get(&state.player_dynasty_id) else {
        return false;
    };
    if player.treasury() < Money::from_copper(300)
        || player
            .resources
            .reputation_quality_basis_points
            .max(player.resources.reputation_reliability_basis_points)
            < OFFICE_NOMINATION_REPUTATION_REQUIREMENT
        || player_contract_deliveries(state) < OFFICE_NOMINATION_DELIVERY_REQUIREMENT
    {
        return false;
    }
    office_nomination_next_day(state, character_id)
        .is_none_or(|next_day| state.clock.day() >= next_day)
}

fn is_institution_support_available(state: &AppState, character_id: CharacterId) -> bool {
    let Some(player) = state.dynasties.get(&state.player_dynasty_id) else {
        return false;
    };
    if player.treasury() < INSTITUTION_SUPPORT_COST
        || player
            .resources
            .reputation_quality_basis_points
            .max(player.resources.reputation_reliability_basis_points)
            < INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT
        || player_contract_deliveries(state) < INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT
    {
        return false;
    }
    institution_support_next_day(state, character_id)
        .is_none_or(|next_day| state.clock.day() >= next_day)
}

fn governance_bonus(persona: GameplayPersona, governance: HouseGovernance) -> i64 {
    match persona {
        GameplayPersona::Steward => match governance {
            HouseGovernance::FamilyPartnership => 420,
            HouseGovernance::HeadCommand
            | HouseGovernance::Primogeniture
            | HouseGovernance::BranchFederation
            | HouseGovernance::ElectedHead => 80,
        },
        GameplayPersona::Entrepreneur => match governance {
            HouseGovernance::FamilyPartnership => 240,
            HouseGovernance::HeadCommand
            | HouseGovernance::Primogeniture
            | HouseGovernance::BranchFederation
            | HouseGovernance::ElectedHead => 80,
        },
        GameplayPersona::PowerBroker => match governance {
            HouseGovernance::Primogeniture => 360,
            HouseGovernance::HeadCommand
            | HouseGovernance::FamilyPartnership
            | HouseGovernance::BranchFederation
            | HouseGovernance::ElectedHead => 80,
        },
        GameplayPersona::Opportunist => match governance {
            HouseGovernance::BranchFederation => 340,
            HouseGovernance::HeadCommand
            | HouseGovernance::Primogeniture
            | HouseGovernance::FamilyPartnership
            | HouseGovernance::ElectedHead => 80,
        },
    }
}

fn rank_adjustment(
    kind: GameplayCommandKind,
    state: &AppState,
    persona: GameplayPersona,
    accumulator: &CampaignAccumulator,
) -> i64 {
    let command_stats = accumulator
        .commands
        .get(&kind)
        .expect("every command kind must have statistics");
    let coverage = if command_stats.executed == 0 { 250 } else { 0 };
    let repetition = i64::from(command_stats.executed).saturating_mul(35);
    let repeat_last = if accumulator.last_command == Some(kind) {
        260
    } else {
        0
    };
    persona_weight(persona, kind)
        .saturating_add(coverage)
        .saturating_add(urgency_weight(state, kind))
        .saturating_add(recovery_priority_adjustment(state, kind))
        .saturating_sub(repetition)
        .saturating_sub(repeat_last)
}

fn player_has_no_active_business(state: &AppState) -> bool {
    let mut owns_business = false;
    let mut has_active = false;
    let mut has_recoverable = false;
    for business in state
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id() == state.player_dynasty_id)
    {
        owns_business = true;
        has_active |= business.status() == BusinessStatus::Active;
        has_recoverable |= matches!(
            business.status(),
            BusinessStatus::Distressed | BusinessStatus::Insolvent
        );
    }
    owns_business && !has_active && has_recoverable
}

fn recovery_priority_adjustment(state: &AppState, kind: GameplayCommandKind) -> i64 {
    if !player_has_no_active_business(state) {
        return 0;
    }
    match kind {
        GameplayCommandKind::TransferBusinessCash
        | GameplayCommandKind::InvestInBusiness
        | GameplayCommandKind::BorrowFunds
        | GameplayCommandKind::AcquireBusiness
        | GameplayCommandKind::SellProperty => 3_500,
        GameplayCommandKind::SecureSupply
        | GameplayCommandKind::SellOutput
        | GameplayCommandKind::ResolveLaborDispute
        | GameplayCommandKind::RespondToCrisis => 500,
        GameplayCommandKind::BuyProperty
        | GameplayCommandKind::ExtendCredit
        | GameplayCommandKind::CommissionInformation
        | GameplayCommandKind::LeverageInformation
        | GameplayCommandKind::CultivateInstitutionSupport
        | GameplayCommandKind::NominateForOffice
        | GameplayCommandKind::EnactLaw
        | GameplayCommandKind::StartPublicWork
        | GameplayCommandKind::FileLegalCase
        | GameplayCommandKind::SetHouseGovernance
        | GameplayCommandKind::ConveneFamilyCouncil
        | GameplayCommandKind::DesignateHeir
        | GameplayCommandKind::AdoptWard
        | GameplayCommandKind::EducateFamilyMember
        | GameplayCommandKind::ExerciseOfficePower => -2_500,
        GameplayCommandKind::SetBusinessPolicy
        | GameplayCommandKind::WithdrawFromInstitution
        | GameplayCommandKind::AcknowledgeNotification => 0,
    }
}

fn steward_weight(kind: GameplayCommandKind) -> i64 {
    match kind {
        GameplayCommandKind::RespondToCrisis | GameplayCommandKind::ResolveLaborDispute => 900,
        GameplayCommandKind::InvestInBusiness | GameplayCommandKind::ExerciseOfficePower => 800,
        GameplayCommandKind::ConveneFamilyCouncil => 850,
        GameplayCommandKind::DesignateHeir | GameplayCommandKind::EducateFamilyMember => 650,
        GameplayCommandKind::SetBusinessPolicy | GameplayCommandKind::StartPublicWork => 600,
        GameplayCommandKind::AdoptWard => 520,
        GameplayCommandKind::CommissionInformation => 480,
        GameplayCommandKind::LeverageInformation => 700,
        GameplayCommandKind::CultivateInstitutionSupport | GameplayCommandKind::SecureSupply => 420,
        GameplayCommandKind::AcknowledgeNotification
        | GameplayCommandKind::WithdrawFromInstitution => 300,
        GameplayCommandKind::TransferBusinessCash
        | GameplayCommandKind::AcquireBusiness
        | GameplayCommandKind::SellOutput
        | GameplayCommandKind::BorrowFunds
        | GameplayCommandKind::ExtendCredit
        | GameplayCommandKind::BuyProperty
        | GameplayCommandKind::SellProperty
        | GameplayCommandKind::EnactLaw
        | GameplayCommandKind::FileLegalCase
        | GameplayCommandKind::SetHouseGovernance
        | GameplayCommandKind::NominateForOffice => 180,
    }
}

fn persona_weight(persona: GameplayPersona, kind: GameplayCommandKind) -> i64 {
    match persona {
        GameplayPersona::Steward => steward_weight(kind),
        GameplayPersona::Entrepreneur => match kind {
            GameplayCommandKind::SellOutput | GameplayCommandKind::LeverageInformation => 950,
            GameplayCommandKind::SecureSupply
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::SellProperty
            | GameplayCommandKind::TransferBusinessCash => 850,
            GameplayCommandKind::BorrowFunds
            | GameplayCommandKind::CommissionInformation
            | GameplayCommandKind::DesignateHeir => 700,
            GameplayCommandKind::EducateFamilyMember => 600,
            GameplayCommandKind::ConveneFamilyCouncil => 320,
            GameplayCommandKind::ExtendCredit => 420,
            GameplayCommandKind::AdoptWard => 360,
            GameplayCommandKind::CultivateInstitutionSupport => 300,
            GameplayCommandKind::EnactLaw
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::SetHouseGovernance
            | GameplayCommandKind::NominateForOffice
            | GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::ResolveLaborDispute
            | GameplayCommandKind::AcknowledgeNotification => 140,
            GameplayCommandKind::ExerciseOfficePower
            | GameplayCommandKind::WithdrawFromInstitution => 500,
        },
        GameplayPersona::PowerBroker => match kind {
            GameplayCommandKind::EnactLaw
            | GameplayCommandKind::CultivateInstitutionSupport
            | GameplayCommandKind::NominateForOffice
            | GameplayCommandKind::ExerciseOfficePower
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::LeverageInformation => 900,
            GameplayCommandKind::ExtendCredit => 820,
            GameplayCommandKind::CommissionInformation => 760,
            GameplayCommandKind::DesignateHeir | GameplayCommandKind::AdoptWard => 780,
            GameplayCommandKind::EducateFamilyMember => 720,
            GameplayCommandKind::ConveneFamilyCouncil => 800,
            GameplayCommandKind::SetHouseGovernance => 700,
            GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::SecureSupply
            | GameplayCommandKind::SellOutput
            | GameplayCommandKind::BorrowFunds
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::SellProperty
            | GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::ResolveLaborDispute
            | GameplayCommandKind::AcknowledgeNotification => 120,
            GameplayCommandKind::WithdrawFromInstitution => 50,
        },
        GameplayPersona::Opportunist => match kind {
            GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::BorrowFunds
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::SellProperty
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::LeverageInformation => 850,
            GameplayCommandKind::SellOutput | GameplayCommandKind::ExerciseOfficePower => 700,
            GameplayCommandKind::CommissionInformation
            | GameplayCommandKind::ExtendCredit
            | GameplayCommandKind::DesignateHeir => 620,
            GameplayCommandKind::CultivateInstitutionSupport
            | GameplayCommandKind::WithdrawFromInstitution => 650,
            GameplayCommandKind::AdoptWard => 500,
            GameplayCommandKind::EducateFamilyMember => 420,
            GameplayCommandKind::ConveneFamilyCouncil => 350,
            GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::SecureSupply
            | GameplayCommandKind::EnactLaw
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::SetHouseGovernance
            | GameplayCommandKind::NominateForOffice
            | GameplayCommandKind::ResolveLaborDispute
            | GameplayCommandKind::AcknowledgeNotification => 100,
        },
    }
}

fn active_crisis_urgency(state: &AppState) -> i64 {
    if state
        .crises
        .values()
        .any(|crisis| crisis.status.is_active())
    {
        2_500
    } else {
        0
    }
}

fn urgency_weight(state: &AppState, kind: GameplayCommandKind) -> i64 {
    match kind {
        GameplayCommandKind::RespondToCrisis => active_crisis_urgency(state),
        GameplayCommandKind::ResolveLaborDispute => labor_dispute_urgency(state),
        GameplayCommandKind::SetBusinessPolicy => business_policy_urgency(state),
        GameplayCommandKind::InvestInBusiness => impaired_business_urgency(state, 2_400),
        GameplayCommandKind::AcquireBusiness => acquisition_urgency(state),
        GameplayCommandKind::AcknowledgeNotification => notification_urgency(state),
        GameplayCommandKind::BorrowFunds => borrowing_urgency(state),
        GameplayCommandKind::SellProperty => 3_500,
        GameplayCommandKind::TransferBusinessCash => impaired_business_urgency(state, 2_800),
        GameplayCommandKind::LeverageInformation => 600,
        GameplayCommandKind::WithdrawFromInstitution => institution_withdrawal_urgency(state),
        GameplayCommandKind::FileLegalCase => legal_case_urgency(state),
        GameplayCommandKind::ConveneFamilyCouncil => family_council_urgency(state),
        GameplayCommandKind::SecureSupply
        | GameplayCommandKind::SellOutput
        | GameplayCommandKind::ExtendCredit
        | GameplayCommandKind::BuyProperty
        | GameplayCommandKind::EnactLaw
        | GameplayCommandKind::StartPublicWork
        | GameplayCommandKind::SetHouseGovernance
        | GameplayCommandKind::DesignateHeir
        | GameplayCommandKind::AdoptWard
        | GameplayCommandKind::EducateFamilyMember
        | GameplayCommandKind::CultivateInstitutionSupport
        | GameplayCommandKind::CommissionInformation
        | GameplayCommandKind::ExerciseOfficePower
        | GameplayCommandKind::NominateForOffice => 0,
    }
}

fn family_council_urgency(state: &AppState) -> i64 {
    let unity = state
        .family_councils
        .get(&state.player_dynasty_id)
        .map_or(10_000, |council| council.unity_basis_points);
    match unity {
        0..=3_499 => 2_400,
        3_500..=5_499 => 1_600,
        5_500..=6_999 => 800,
        _ => 0,
    }
}

fn legal_case_urgency(state: &AppState) -> i64 {
    let player_id = state.player_dynasty_id;
    let has_defaulted_debt = state.loans.values().any(|loan| {
        loan.lender_dynasty_id == player_id
            && loan.status == LoanStatus::Defaulted
            && legal_grievance_kind(state, loan.borrower_dynasty_id) == Some(LegalCaseKind::Debt)
    });
    if has_defaulted_debt {
        return 1_200;
    }
    let has_unresolved_grievance = state
        .dynasties
        .keys()
        .copied()
        .filter(|dynasty_id| *dynasty_id != player_id)
        .any(|dynasty_id| legal_grievance_kind(state, dynasty_id).is_some());
    if has_unresolved_grievance { 800 } else { 0 }
}

fn labor_dispute_urgency(state: &AppState) -> i64 {
    if state
        .employment
        .values()
        .any(|agreement| agreement.status == EmploymentStatus::Disputed)
    {
        2_100
    } else {
        0
    }
}

fn business_policy_urgency(state: &AppState) -> i64 {
    if state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == state.player_dynasty_id
            && business.status() == BusinessStatus::Distressed
    }) {
        1_000
    } else {
        0
    }
}

fn impaired_business_urgency(state: &AppState, urgency: i64) -> i64 {
    if state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == state.player_dynasty_id
            && matches!(
                business.status(),
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            )
    }) {
        urgency
    } else {
        0
    }
}

fn acquisition_urgency(state: &AppState) -> i64 {
    if state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == state.player_dynasty_id
            && matches!(
                business.status(),
                BusinessStatus::Active | BusinessStatus::Distressed
            )
    }) {
        0
    } else {
        2_300
    }
}

fn notification_urgency(state: &AppState) -> i64 {
    if state
        .outbox
        .iter()
        .filter(|message| !message.acknowledged)
        .count()
        > 8
    {
        650
    } else {
        0
    }
}

fn borrowing_urgency(state: &AppState) -> i64 {
    if player_has_no_active_business(state) {
        3_000
    } else if state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_some_and(|dynasty| dynasty.treasury() < Money::from_copper(8_000))
    {
        700
    } else {
        0
    }
}

fn institution_withdrawal_urgency(state: &AppState) -> i64 {
    if has_institution_withdrawal_opportunity(state) {
        1_500
    } else {
        0
    }
}

fn push_candidate(
    candidates: &mut Vec<Candidate>,
    kind: GameplayCommandKind,
    command: PlayerCommand,
    description: String,
    score: i64,
) {
    candidates.push(Candidate {
        kind,
        command,
        description,
        score,
    });
}

fn crisis_responses(persona: GameplayPersona) -> [CrisisResponse; 4] {
    match persona {
        GameplayPersona::Steward => [
            CrisisResponse::Relief,
            CrisisResponse::Reform,
            CrisisResponse::Suppress,
            CrisisResponse::Exploit,
        ],
        GameplayPersona::Entrepreneur => [
            CrisisResponse::Reform,
            CrisisResponse::Relief,
            CrisisResponse::Exploit,
            CrisisResponse::Suppress,
        ],
        GameplayPersona::PowerBroker => [
            CrisisResponse::Suppress,
            CrisisResponse::Reform,
            CrisisResponse::Relief,
            CrisisResponse::Exploit,
        ],
        GameplayPersona::Opportunist => [
            CrisisResponse::Exploit,
            CrisisResponse::Suppress,
            CrisisResponse::Reform,
            CrisisResponse::Relief,
        ],
    }
}

fn crisis_response_bonus(persona: GameplayPersona, response: CrisisResponse) -> i64 {
    match persona {
        GameplayPersona::Steward => match response {
            CrisisResponse::Relief => 600,
            CrisisResponse::Exploit => -200,
            CrisisResponse::Reform | CrisisResponse::Suppress => 100,
        },
        GameplayPersona::Entrepreneur => match response {
            CrisisResponse::Reform => 500,
            CrisisResponse::Exploit => -200,
            CrisisResponse::Relief | CrisisResponse::Suppress => 100,
        },
        GameplayPersona::PowerBroker => match response {
            CrisisResponse::Suppress => 520,
            CrisisResponse::Exploit => -200,
            CrisisResponse::Relief | CrisisResponse::Reform => 100,
        },
        GameplayPersona::Opportunist => match response {
            CrisisResponse::Exploit => 700,
            CrisisResponse::Relief | CrisisResponse::Reform | CrisisResponse::Suppress => 100,
        },
    }
}

fn labor_responses(persona: GameplayPersona) -> [LaborResponse; 3] {
    match persona {
        GameplayPersona::Steward => [
            LaborResponse::ImproveConditions,
            LaborResponse::Negotiate,
            LaborResponse::ReplaceWorkers,
        ],
        GameplayPersona::Entrepreneur | GameplayPersona::PowerBroker => [
            LaborResponse::Negotiate,
            LaborResponse::ImproveConditions,
            LaborResponse::ReplaceWorkers,
        ],
        GameplayPersona::Opportunist => [
            LaborResponse::ReplaceWorkers,
            LaborResponse::Negotiate,
            LaborResponse::ImproveConditions,
        ],
    }
}

fn labor_response_bonus(persona: GameplayPersona, response: LaborResponse) -> i64 {
    match persona {
        GameplayPersona::Steward => match response {
            LaborResponse::ImproveConditions => 500,
            LaborResponse::Negotiate | LaborResponse::ReplaceWorkers => 80,
        },
        GameplayPersona::Entrepreneur => match response {
            LaborResponse::Negotiate => 450,
            LaborResponse::ImproveConditions | LaborResponse::ReplaceWorkers => 80,
        },
        GameplayPersona::PowerBroker => match response {
            LaborResponse::Negotiate => 400,
            LaborResponse::ImproveConditions | LaborResponse::ReplaceWorkers => 80,
        },
        GameplayPersona::Opportunist => match response {
            LaborResponse::ReplaceWorkers => 550,
            LaborResponse::ImproveConditions | LaborResponse::Negotiate => 80,
        },
    }
}

const LAW_POWER_PENDING: &str = "law sponsorship office power not established";
const WORK_REQUIRES_OFFICE: &str = "public-work sponsorship requires office";
const WORK_REQUIRES_POWER: &str = "public-work sponsorship requires office power";
const WORK_POWER_PENDING: &str = "public-work office power not established";
const OFFICE_RECORD_SHORT: &str = "insufficient office commercial record";
const WARD_RECORD_SHORT: &str = "insufficient ward commercial record";
const OFFICE_DIRECTIVE_PENDING: &str = "office power directive not established";
const SUPPORT_REPUTATION_SHORT: &str = "insufficient institution-support reputation";
const SUPPORT_RECORD_SHORT: &str = "insufficient institution-support commercial record";
const SUPPORT_EXISTS: &str = "institution support already established";
const SUPPORT_MISSING: &str = "institution support not established";
const REPORT_UNCOMMISSIONED: &str = "intelligence report not commissioned";
const REPORT_NO_LEVERAGE: &str = "intelligence report has no leverage";

const fn command_error_category(error: &CommandError) -> &'static str {
    match error {
        CommandError::Strategic(source) => strategic_error_category(source),
        CommandError::Simulation(source) => simulation_error_category(source),
        CommandError::MissingBusiness { .. } => "missing business",
        CommandError::BusinessNotOwned { .. } => "business not owned",
        CommandError::PlayerNotParty => "player not party",
        CommandError::LoanCounterpartyLenderReserve { .. } => "loan counterparty lender reserve",
        CommandError::LoanCounterpartyInterestTooLow { .. } => "loan counterparty interest low",
        CommandError::LoanCounterpartyInterestTooHigh { .. } => "loan counterparty interest high",
        CommandError::LoanCounterpartyPaymentTooLow { .. } => "loan counterparty payment low",
        CommandError::LoanCounterpartyPaymentTooHigh { .. } => "loan counterparty payment high",
        CommandError::LoanCounterpartyCollateralTooLarge { .. } => {
            "loan counterparty collateral too large"
        }
        CommandError::ContractCounterpartyPriceTooLow { .. } => "contract counterparty price low",
        CommandError::ContractCounterpartyPriceTooHigh { .. } => "contract counterparty price high",
        CommandError::ContractCounterpartyPenaltyOutOfRange { .. } => {
            "contract counterparty penalty"
        }
        CommandError::ContractCounterpartyCapacity { .. } => "contract counterparty capacity",
        CommandError::PropertyCounterpartyBuyerReserve { .. } => "property buyer reserve",
        CommandError::InvalidBusinessPolicy => "invalid business policy",
        CommandError::UnchangedBusinessPolicy { .. } => "unchanged business policy",
        CommandError::BusinessPolicyCooldown { .. } => "business policy cooldown",
        CommandError::InvalidBusinessInvestment => "invalid business investment",
        CommandError::InvalidLawValue { .. } => "invalid law value",
        CommandError::UnchangedLaw { .. } => "unchanged law",
        CommandError::MissingCivicTreasury => "missing civic treasury",
        CommandError::NoCivicDebtCreditor { .. } => "no civic debt creditor",
        CommandError::CivicTreasuryOverflow { .. } => "civic treasury overflow",
        CommandError::LawSponsorshipRequiresOffice => "law sponsorship requires office",
        CommandError::LawSponsorshipRequiresPower { .. } => "law sponsorship requires office power",
        CommandError::LawSponsorshipPowerNotEstablished { .. } => LAW_POWER_PENDING,
        CommandError::LawCooldown { .. } => "law cooldown",
        CommandError::MissingDistrict { .. } => "missing district",
        CommandError::MissingDynasty { .. } => "missing dynasty",
        CommandError::InsufficientPlayerFunds { .. } => "insufficient player funds",
        CommandError::InsufficientPlayerLegitimacy { .. } => "insufficient player legitimacy",
        CommandError::InsufficientBusinessFunds { .. } => "insufficient business funds",
        CommandError::InvalidPublicWorkBudget => "invalid public-work budget",
        CommandError::PublicWorkSponsorshipRequiresOffice => WORK_REQUIRES_OFFICE,
        CommandError::PublicWorkSponsorshipRequiresPower => WORK_REQUIRES_POWER,
        CommandError::PublicWorkPowerNotEstablished { .. } => WORK_POWER_PENDING,
        CommandError::DuplicateActivePublicWork { .. } => "duplicate active public work",
        CommandError::PublicWorkCooldown { .. } => "public-work cooldown",
        CommandError::PublicWorkCapacity { .. } => "public-work capacity",
        CommandError::SameLegalParty => "same legal party",
        CommandError::InvalidLegalTerms => "invalid legal terms",
        CommandError::DuplicateActiveLegalCase { .. } => "duplicate active legal case",
        CommandError::LegalCaseCooldown { .. } => "legal-case cooldown",
        CommandError::MissingFamilyCouncil { .. } => "missing family council",
        CommandError::UnchangedHouseGovernance { .. } => "unchanged governance",
        CommandError::HouseGovernanceCooldown { .. } => "governance cooldown",
        CommandError::FamilyCouncilMeetingCooldown { .. } => "family council cooldown",
        CommandError::InvalidHeirCandidate { .. } => "invalid heir candidate",
        CommandError::UnchangedHeir { .. } => "unchanged heir",
        CommandError::HeirDesignationCooldown { .. } => "heir designation cooldown",
        CommandError::InsufficientOfficeReputation { .. } => "insufficient office reputation",
        CommandError::InsufficientOfficeCommercialRecord { .. } => OFFICE_RECORD_SHORT,
        CommandError::OfficeNominationCooldown { .. } => "office nomination cooldown",
        CommandError::WardAdoptionCooldown { .. } => "ward adoption cooldown",
        CommandError::WardCapacity { .. } => "ward capacity",
        CommandError::InsufficientWardReputation { .. } => "insufficient ward reputation",
        CommandError::InsufficientWardCommercialRecord { .. } => WARD_RECORD_SHORT,
        CommandError::InvalidFamilyStudent { .. } => "invalid family student",
        CommandError::FamilyEducationAtMaximum { .. } => "family education at maximum",
        CommandError::FamilyEducationCooldown { .. } => "family education cooldown",
        CommandError::MissingInstitution { .. } => "missing institution",
        CommandError::OfficePowerUnavailable { .. } => "office power unavailable",
        CommandError::OfficePowerDirectiveNotEstablished { .. } => OFFICE_DIRECTIVE_PENDING,
        CommandError::OfficePowerDirectiveCooldown { .. } => "office power directive cooldown",
        CommandError::InsufficientInstitutionSupportReputation { .. } => SUPPORT_REPUTATION_SHORT,
        CommandError::InsufficientInstitutionSupportCommercialRecord { .. } => SUPPORT_RECORD_SHORT,
        CommandError::InstitutionSupportAlreadyEstablished { .. } => SUPPORT_EXISTS,
        CommandError::InstitutionMembershipCapacity { .. } => "institution membership capacity",
        CommandError::InstitutionSupportCooldown { .. } => "institution support cooldown",
        CommandError::InstitutionBudgetOverflow { .. } => "institution budget overflow",
        CommandError::MissingInstitutionSupport { .. } => "missing institution support",
        CommandError::InstitutionSupportNotEstablished { .. } => SUPPORT_MISSING,
        CommandError::InvalidNominee { .. } => "invalid nominee",
        CommandError::NomineeAlreadyHoldsOffice { .. } => "nominee already holds office",
        CommandError::InvalidInstitutionWithdrawal { .. } => "invalid institution withdrawal",
        CommandError::MissingCrisis { .. } => "missing crisis",
        CommandError::InactiveCrisis { .. } => "inactive crisis",
        CommandError::CrisisAlreadyAddressed { .. } => "crisis already addressed",
        CommandError::MissingEmployment { .. } => "missing employment",
        CommandError::InvalidLaborDispute { .. } => "invalid labor dispute",
        CommandError::LaborWageOverflow { .. } => "labor wage overflow",
        CommandError::NoReplacementLaborAvailable { .. } => "no replacement labor available",
        CommandError::MissingGood { .. } => "missing good",
        CommandError::MissingMarketQuote { .. } => "missing market quote",
        CommandError::InformationCannotTargetPlayer => "invalid intelligence target",
        CommandError::InformationCommissionCooldown { .. } => "intelligence commission cooldown",
        CommandError::MissingInformationReport { .. } => "missing intelligence report",
        CommandError::InformationReportNotOwned { .. } => "intelligence report not owned",
        CommandError::InformationReportNotCommissioned { .. } => REPORT_UNCOMMISSIONED,
        CommandError::InformationReportExpired { .. } => "intelligence report expired",
        CommandError::InformationReportHasNoLeverage { .. } => REPORT_NO_LEVERAGE,
        CommandError::MissingNotification { .. } => "missing notification",
    }
}

const fn strategic_error_category(error: &StrategicError) -> &'static str {
    match error {
        StrategicError::RegistryMismatch { .. } => "strategic: registry mismatch",
        StrategicError::MissingBusiness { .. } => "strategic: missing business",
        StrategicError::BusinessInactive { .. } => "strategic: inactive business",
        StrategicError::BusinessNotOwnedByDynasty { .. } => {
            "strategic: business ownership mismatch"
        }
        StrategicError::MissingDynasty { .. } => "strategic: missing dynasty",
        StrategicError::MissingProperty { .. } => "strategic: missing property",
        StrategicError::SameContractParty => "strategic: same contract party",
        StrategicError::SameContractOwner { .. } => "strategic: same contract owner",
        StrategicError::SameLoanParty => "strategic: same loan party",
        StrategicError::ExistingUnsettledLoan { .. } => "strategic: existing unsettled loan",
        StrategicError::DefaultedLoanRestructuringCooldown { .. } => {
            "strategic: restructuring cooldown"
        }
        StrategicError::LoanBalanceOverflow { .. } => "strategic: loan balance overflow",
        StrategicError::NonPositiveAmount => "strategic: nonpositive amount",
        StrategicError::NonPositiveQuantity => "strategic: nonpositive quantity",
        StrategicError::EmptyContractDuration => "strategic: empty contract duration",
        StrategicError::ContractPaymentOverflow { .. } => "strategic: contract payment overflow",
        StrategicError::SellerCannotProduce { .. } => "strategic: seller cannot produce",
        StrategicError::BuyerDoesNotConsume { .. } => "strategic: buyer does not consume",
        StrategicError::InsufficientDynastyFunds { .. } => "strategic: insufficient dynasty funds",
        StrategicError::DynastyTreasuryOverflow { .. } => "strategic: dynasty treasury overflow",
        StrategicError::BusinessCashOverflow { .. } => "strategic: business cash overflow",
        StrategicError::BusinessFinanceVersionExhausted { .. } => {
            "strategic: business finance version exhausted"
        }
        StrategicError::DynastyAdministrativeLoadUnderflow { .. } => {
            "strategic: administrative load underflow"
        }
        StrategicError::DynastyAdministrativeLoadOverflow { .. } => {
            "strategic: administrative load overflow"
        }
        StrategicError::AcquisitionCostOverflow { .. } => "strategic: acquisition cost overflow",
        StrategicError::BusinessValuationOverflow { .. } => {
            "strategic: business valuation overflow"
        }
        StrategicError::InterestOutOfRange { .. } => "strategic: interest out of range",
        StrategicError::CollateralNotOwned { .. } => "strategic: collateral not owned",
        StrategicError::PropertyAlreadyPledged { .. } => "strategic: property already pledged",
        StrategicError::PropertyAlreadyOwned { .. } => "strategic: property already owned",
        StrategicError::PropertyNotOwnedBySeller { .. } => {
            "strategic: property not owned by seller"
        }
        StrategicError::SamePropertyParty => "strategic: same property party",
        StrategicError::MissingCivicTreasury => "strategic: missing civic treasury",
        StrategicError::InsufficientPropertyAuctionLiquidity { .. } => {
            "strategic: insufficient property auction liquidity"
        }
        StrategicError::MissingCollateralLoan { .. } => "strategic: missing collateral loan",
        StrategicError::PropertyLienBorrowerMismatch { .. } => {
            "strategic: property lien borrower mismatch"
        }
        StrategicError::PropertySaleCannotSettleLien { .. } => {
            "strategic: property sale cannot settle lien"
        }
        StrategicError::BusinessAlreadyOwned { .. } => "strategic: business already owned",
        StrategicError::BusinessNotAcquirable { .. } => "strategic: business not acquirable",
        StrategicError::InvalidAcquisitionManager { .. } => {
            "strategic: invalid acquisition manager"
        }
        StrategicError::InsufficientBusinessRecapitalization { .. } => {
            "strategic: insufficient business recapitalization"
        }
    }
}

const fn simulation_error_category(error: &SimulationError) -> &'static str {
    match error {
        SimulationError::InvalidDayCount { .. } => "simulation: invalid day count",
        SimulationError::DayRangeExhausted { .. } => "simulation: day range exhausted",
        SimulationError::RegistryMismatch { .. } => "simulation: registry mismatch",
        SimulationError::BusinessNotFound { .. } => "simulation: business not found",
        SimulationError::BusinessInactive { .. } => "simulation: inactive business",
        SimulationError::SameBusiness { .. } => "simulation: same business",
        SimulationError::NonPositiveAmount { .. } => "simulation: nonpositive amount",
        SimulationError::InsufficientBusinessCash { .. } => {
            "simulation: insufficient business cash"
        }
        SimulationError::BusinessCashOverflow { .. } => "simulation: business cash overflow",
        SimulationError::BusinessLifetimeCostsOverflow { .. } => {
            "simulation: business lifetime costs overflow"
        }
        SimulationError::BusinessLifetimeRevenueOverflow { .. } => {
            "simulation: business lifetime revenue overflow"
        }
        SimulationError::StaleBusinessFinance { .. } => "simulation: stale business finance",
        SimulationError::BusinessFinanceVersionExhausted { .. } => {
            "simulation: business finance version exhausted"
        }
        SimulationError::FamilyCharterVersionExhausted { .. } => {
            "simulation: family charter version exhausted"
        }
        SimulationError::DynastyGenerationExhausted { .. } => {
            "simulation: dynasty generation exhausted"
        }
        SimulationError::DynastyCivicContributionsOverflow { .. } => {
            "simulation: civic contributions overflow"
        }
        SimulationError::InstitutionTermNumberExhausted { .. } => {
            "simulation: institution term number exhausted"
        }
        SimulationError::MarketQuoteMissing { .. } => "simulation: missing market quote",
        SimulationError::MarketStockOverflow { .. } => "simulation: market stock overflow",
        SimulationError::MarketSupplyOverflow { .. } => "simulation: market supply overflow",
        SimulationError::WeeklyExternalIncomeOverflow { .. } => {
            "simulation: weekly external income overflow"
        }
        SimulationError::LoanBalanceOverflow { .. } => "simulation: loan balance overflow",
        SimulationError::CivicDebtBalanceOverflow { .. } => {
            "simulation: civic debt balance overflow"
        }
        SimulationError::MarketClearingAccountOverflow { .. } => {
            "simulation: market clearing account overflow"
        }
    }
}

fn score_campaign(
    accumulator: &CampaignAccumulator,
    start: &GameplaySnapshot,
    end: &GameplaySnapshot,
) -> GameplayScores {
    let executed: u32 = accumulator
        .commands
        .iter()
        .filter(|(kind, _)| **kind != GameplayCommandKind::AcknowledgeNotification)
        .map(|(_, stats)| stats.executed)
        .sum();
    let opportunity_cycles = accumulator
        .decision_cycles
        .saturating_sub(accumulator.quiet_cycles);
    let opportunity_conversion = if opportunity_cycles == 0 {
        100
    } else {
        ratio_score(accumulator.cycles_with_viable_choices, opportunity_cycles)
    };
    let strategic_cadence = ratio_score(
        accumulator.cycles_with_viable_choices,
        accumulator.decision_cycles,
    );
    let actionability = average_scores(&[opportunity_conversion, strategic_cadence]);
    let command_coverage = usize_to_u16(
        accumulator
            .commands
            .iter()
            .filter(|(kind, stats)| {
                **kind != GameplayCommandKind::AcknowledgeNotification && stats.executed > 0
            })
            .count(),
    );
    let coverage_score = ratio_score(
        u32::from(command_coverage),
        usize_to_u32(ALL_COMMAND_KINDS.len().saturating_sub(1)),
    );
    let dominant_actions = accumulator
        .commands
        .values()
        .map(|stats| stats.executed)
        .max()
        .unwrap_or(0);
    let distribution_score = 100_u16.saturating_sub(ratio_score(dominant_actions, executed));
    let choice_richness = ratio_score(
        accumulator.total_viable_command_kinds,
        opportunity_cycles.saturating_mul(3),
    );
    let variety = average_scores(&[coverage_score, distribution_score, choice_richness]);
    let edge_count = usize_to_u32(accumulator.interactions.len());
    let interaction_observations: u32 = accumulator.interactions.values().copied().sum();
    let interconnection = interconnection_score(
        edge_count,
        interaction_observations,
        executed,
        u32::from(command_coverage),
    );
    let feedback_actions: u32 = accumulator
        .commands
        .iter()
        .filter(|(kind, _)| **kind != GameplayCommandKind::AcknowledgeNotification)
        .map(|(_, stats)| stats.actions_with_feedback)
        .sum();
    let feedback = ratio_score(feedback_actions, executed);
    let resilience = resilience_score(accumulator, start, end);
    let overall = weighted_overall(
        actionability,
        variety,
        interconnection,
        feedback,
        resilience,
    );
    GameplayScores {
        actionability,
        variety,
        interconnection,
        feedback,
        resilience,
        overall,
    }
}

fn interconnection_score(
    edge_count: u32,
    observations: u32,
    executed: u32,
    executed_kinds: u32,
) -> u16 {
    if executed == 0 || executed_kinds == 0 {
        return 0;
    }
    let target_edges = executed_kinds.saturating_mul(6);
    let edge_coverage = ratio_score(edge_count, target_edges);
    let target_observations = executed.saturating_mul(4);
    let breadth = ratio_score(observations, target_observations);
    average_scores(&[edge_coverage, breadth])
}

fn resilience_score(
    accumulator: &CampaignAccumulator,
    start: &GameplaySnapshot,
    end: &GameplaySnapshot,
) -> u16 {
    let business = if end.active_businesses > 0 && end.distressed_businesses == 0 {
        100
    } else if end.active_businesses > 0 {
        80
    } else if end.distressed_businesses > 0 {
        25
    } else {
        0
    };
    let condition = (end.average_business_condition / 100).min(100);
    let food = end.average_food_satisfaction / 100;
    let treasury = if end.player_treasury >= start.player_treasury {
        100
    } else if end.player_treasury > Money::ZERO {
        60
    } else {
        0
    };
    let crisis = if end.escalated_crises == 0 { 100 } else { 35 };
    let trajectory = average_scores(&[
        accumulator.minimum_food_satisfaction / 100,
        accumulator.minimum_district_food_satisfaction / 100,
        if accumulator.minimum_operating_businesses > 0 {
            100
        } else {
            0
        },
        100_u16.saturating_sub(accumulator.maximum_disputed_employment.saturating_mul(8)),
        100_u16.saturating_sub(accumulator.maximum_active_crises.saturating_mul(15)),
    ]);
    average_scores(&[
        business,
        business,
        condition,
        food.min(100),
        treasury,
        crisis,
        trajectory,
    ])
}

fn weighted_overall(
    actionability: u16,
    variety: u16,
    interconnection: u16,
    feedback: u16,
    resilience: u16,
) -> u16 {
    let total = u32::from(actionability) * 20
        + u32::from(variety) * 20
        + u32::from(interconnection) * 20
        + u32::from(feedback) * 15
        + u32::from(resilience) * 25;
    u16::try_from(total / 100).unwrap_or(100).min(100)
}

#[derive(Clone, Copy, Debug, Default)]
struct AggregateCycleTotals {
    simulated_days: u64,
    decision_cycles: u64,
    viable_choices: u64,
    viable_command_kinds: u64,
    no_action_cycles: u64,
    quiet_cycles: u64,
    quiet_cycles_with_ambient_change: u64,
    blocked_cycles: u64,
    multiple_viable_command_kinds: u64,
    close_viable_command_kinds: u64,
    distinct_immediate_consequences: u64,
    distinct_projected_consequences: u64,
}

impl AggregateCycleTotals {
    fn add_campaign(&mut self, campaign: &GameplayCampaignReport) {
        self.simulated_days = self
            .simulated_days
            .saturating_add(u64::from(campaign.simulated_days));
        self.decision_cycles = self
            .decision_cycles
            .saturating_add(u64::from(campaign.decision_cycles));
        self.viable_choices = self
            .viable_choices
            .saturating_add(u64::from(campaign.total_viable_choices));
        self.viable_command_kinds = self
            .viable_command_kinds
            .saturating_add(u64::from(campaign.total_viable_command_kinds));
        self.no_action_cycles = self
            .no_action_cycles
            .saturating_add(u64::from(campaign.no_action_cycles));
        self.quiet_cycles = self
            .quiet_cycles
            .saturating_add(u64::from(campaign.quiet_cycles));
        self.quiet_cycles_with_ambient_change = self
            .quiet_cycles_with_ambient_change
            .saturating_add(u64::from(campaign.quiet_cycles_with_ambient_change));
        self.blocked_cycles = self
            .blocked_cycles
            .saturating_add(u64::from(campaign.blocked_cycles));
        self.multiple_viable_command_kinds = self.multiple_viable_command_kinds.saturating_add(
            u64::from(campaign.cycles_with_multiple_viable_command_kinds),
        );
        self.close_viable_command_kinds = self
            .close_viable_command_kinds
            .saturating_add(u64::from(campaign.cycles_with_close_viable_command_kinds));
        self.distinct_immediate_consequences =
            self.distinct_immediate_consequences
                .saturating_add(u64::from(
                    campaign.cycles_with_distinct_immediate_consequences,
                ));
        self.distinct_projected_consequences =
            self.distinct_projected_consequences
                .saturating_add(u64::from(
                    campaign.cycles_with_distinct_projected_consequences,
                ));
    }
}

fn aggregate_campaigns(campaigns: &[GameplayCampaignReport]) -> GameplayAggregate {
    let mut commands = initialized_command_stats();
    let mut phase_stats = initialized_phase_stats();
    let mut rejection_reasons = BTreeMap::new();
    let mut domain_changes = initialized_domain_counts();
    let mut causal_domain_changes = initialized_domain_counts();
    let mut ambient_domain_changes = initialized_domain_counts();
    let mut interactions = BTreeMap::new();
    let mut totals = AggregateCycleTotals::default();
    for campaign in campaigns {
        merge_phase_stats(campaign, &mut phase_stats);
        merge_campaign(
            campaign,
            &mut commands,
            &mut rejection_reasons,
            &mut domain_changes,
            &mut causal_domain_changes,
            &mut ambient_domain_changes,
            &mut interactions,
        );
        totals.add_campaign(campaign);
    }
    let successful_actions = commands
        .values()
        .map(|stats| u64::from(stats.executed))
        .sum();
    let substantive_actions = commands
        .iter()
        .filter(|(kind, _)| **kind != GameplayCommandKind::AcknowledgeNotification)
        .map(|(_, stats)| u64::from(stats.executed))
        .sum();
    let candidate_probes = commands
        .values()
        .map(|stats| u64::from(stats.considered))
        .sum();
    let command_coverage =
        usize_to_u16(commands.values().filter(|stats| stats.executed > 0).count());
    let domain_coverage = usize_to_u16(domain_changes.values().filter(|count| **count > 0).count());
    let causal_domain_coverage = usize_to_u16(
        causal_domain_changes
            .values()
            .filter(|count| **count > 0)
            .count(),
    );
    let ambient_domain_coverage = usize_to_u16(
        ambient_domain_changes
            .values()
            .filter(|count| **count > 0)
            .count(),
    );
    let scores = aggregate_scores(campaigns);
    GameplayAggregate {
        campaigns: usize_to_u32(campaigns.len()),
        simulated_days: totals.simulated_days,
        decision_cycles: totals.decision_cycles,
        successful_actions,
        substantive_actions,
        candidate_probes,
        viable_choices: totals.viable_choices,
        viable_command_kinds: totals.viable_command_kinds,
        phase_stats,
        no_action_cycles: totals.no_action_cycles,
        quiet_cycles: totals.quiet_cycles,
        quiet_cycles_with_ambient_change: totals.quiet_cycles_with_ambient_change,
        blocked_cycles: totals.blocked_cycles,
        cycles_with_multiple_viable_command_kinds: totals.multiple_viable_command_kinds,
        cycles_with_close_viable_command_kinds: totals.close_viable_command_kinds,
        cycles_with_distinct_immediate_consequences: totals.distinct_immediate_consequences,
        cycles_with_distinct_projected_consequences: totals.distinct_projected_consequences,
        command_coverage,
        domain_coverage,
        commands,
        rejection_reasons,
        domain_changes,
        causal_domain_changes,
        ambient_domain_changes,
        causal_domain_coverage,
        ambient_domain_coverage,
        interactions: interaction_vec(&interactions),
        scores,
    }
}

fn merge_phase_stats(
    campaign: &GameplayCampaignReport,
    phase_stats: &mut BTreeMap<GameplayPhase, GameplayPhaseStats>,
) {
    for (phase, source) in &campaign.phase_stats {
        let target = phase_stats
            .get_mut(phase)
            .expect("every gameplay phase must have aggregate statistics");
        target.decision_cycles = target
            .decision_cycles
            .saturating_add(source.decision_cycles);
        target.substantive_actions = target
            .substantive_actions
            .saturating_add(source.substantive_actions);
        target.institutional_campaign_actions = target
            .institutional_campaign_actions
            .saturating_add(source.institutional_campaign_actions);
        target.quiet_cycles = target.quiet_cycles.saturating_add(source.quiet_cycles);
        target.quiet_cycles_with_ambient_change = target
            .quiet_cycles_with_ambient_change
            .saturating_add(source.quiet_cycles_with_ambient_change);
        target.longest_quiet_streak_cycles = target
            .longest_quiet_streak_cycles
            .max(source.longest_quiet_streak_cycles);
        target.blocked_cycles = target.blocked_cycles.saturating_add(source.blocked_cycles);
        target.cycles_with_multiple_viable_command_kinds = target
            .cycles_with_multiple_viable_command_kinds
            .saturating_add(source.cycles_with_multiple_viable_command_kinds);
        target.cycles_with_close_viable_command_kinds = target
            .cycles_with_close_viable_command_kinds
            .saturating_add(source.cycles_with_close_viable_command_kinds);
        target.cycles_with_distinct_immediate_consequences = target
            .cycles_with_distinct_immediate_consequences
            .saturating_add(source.cycles_with_distinct_immediate_consequences);
        target.cycles_with_distinct_projected_consequences = target
            .cycles_with_distinct_projected_consequences
            .saturating_add(source.cycles_with_distinct_projected_consequences);
        target.total_viable_choices = target
            .total_viable_choices
            .saturating_add(source.total_viable_choices);
        target.total_viable_command_kinds = target
            .total_viable_command_kinds
            .saturating_add(source.total_viable_command_kinds);
    }
}

fn merge_campaign(
    campaign: &GameplayCampaignReport,
    commands: &mut BTreeMap<GameplayCommandKind, GameplayCommandStats>,
    rejections: &mut BTreeMap<String, u32>,
    domains: &mut BTreeMap<GameplayDomain, u32>,
    causal_domains: &mut BTreeMap<GameplayDomain, u32>,
    ambient_domains: &mut BTreeMap<GameplayDomain, u32>,
    interactions: &mut BTreeMap<(GameplayCommandKind, GameplayDomain), u32>,
) {
    for (kind, source) in &campaign.commands {
        let target = commands
            .get_mut(kind)
            .expect("every command kind must have aggregate statistics");
        target.activation_opportunities = target
            .activation_opportunities
            .saturating_add(source.activation_opportunities);
        target.offered_cycles = target.offered_cycles.saturating_add(source.offered_cycles);
        target.generated = target.generated.saturating_add(source.generated);
        target.considered = target.considered.saturating_add(source.considered);
        target.viable = target.viable.saturating_add(source.viable);
        target.executed = target.executed.saturating_add(source.executed);
        target.rejected = target.rejected.saturating_add(source.rejected);
        target.immediate_world_feedback = target
            .immediate_world_feedback
            .saturating_add(source.immediate_world_feedback);
        target.delayed_world_feedback = target
            .delayed_world_feedback
            .saturating_add(source.delayed_world_feedback);
        target.actions_with_feedback = target
            .actions_with_feedback
            .saturating_add(source.actions_with_feedback);
        target.actions_with_persistent_consequences = target
            .actions_with_persistent_consequences
            .saturating_add(source.actions_with_persistent_consequences);
        target.actions_with_delayed_consequences = target
            .actions_with_delayed_consequences
            .saturating_add(source.actions_with_delayed_consequences);
        target.changed_domains.extend(&source.changed_domains);
    }
    for (reason, count) in &campaign.rejection_reasons {
        *rejections.entry(reason.clone()).or_default() += count;
    }
    for (domain, count) in &campaign.domain_changes {
        *domains.entry(*domain).or_default() += count;
    }
    for (domain, count) in &campaign.causal_domain_changes {
        *causal_domains.entry(*domain).or_default() += count;
    }
    for (domain, count) in &campaign.ambient_domain_changes {
        *ambient_domains.entry(*domain).or_default() += count;
    }
    for edge in &campaign.interactions {
        *interactions.entry((edge.command, edge.domain)).or_default() += edge.observations;
    }
}

fn aggregate_scores(campaigns: &[GameplayCampaignReport]) -> GameplayScores {
    if campaigns.is_empty() {
        return GameplayScores {
            actionability: 0,
            variety: 0,
            interconnection: 0,
            feedback: 0,
            resilience: 0,
            overall: 0,
        };
    }
    GameplayScores {
        actionability: average_u16(
            campaigns
                .iter()
                .map(|campaign| campaign.scores.actionability),
        ),
        variety: average_u16(campaigns.iter().map(|campaign| campaign.scores.variety)),
        interconnection: average_u16(
            campaigns
                .iter()
                .map(|campaign| campaign.scores.interconnection),
        ),
        feedback: average_u16(campaigns.iter().map(|campaign| campaign.scores.feedback)),
        resilience: average_u16(campaigns.iter().map(|campaign| campaign.scores.resilience)),
        overall: average_u16(campaigns.iter().map(|campaign| campaign.scores.overall)),
    }
}

fn derive_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
) -> Vec<GameplayFinding> {
    let mut findings = Vec::new();
    add_score_findings(aggregate, &mut findings);
    add_command_findings(aggregate, &mut findings);
    add_domain_findings(aggregate, &mut findings);
    add_action_concentration_finding(aggregate, &mut findings);
    add_institutional_campaign_concentration_finding(aggregate, &mut findings);
    add_phase_institutional_campaign_concentration_finding(aggregate, &mut findings);
    add_repetitive_command_streak_finding(campaigns, &mut findings);
    add_information_routine_finding(campaigns, &mut findings);
    add_crisis_trajectory_finding(aggregate, &mut findings);
    add_office_directive_trajectory_finding(aggregate, &mut findings);
    add_welfare_dynamism_finding(aggregate, campaigns, &mut findings);
    add_long_horizon_risk_findings(aggregate, campaigns, &mut findings);
    add_rival_commercial_pressure_finding(aggregate, campaigns, &mut findings);
    add_succession_cohesion_finding(campaigns, &mut findings);
    add_long_substantive_gap_finding(campaigns, &mut findings);
    add_asset_liquidity_drought_finding(campaigns, &mut findings);
    add_economic_recovery_dead_end_finding(campaigns, &mut findings);
    add_campaign_blocking_finding(campaigns, &mut findings);
    add_business_survival_finding(campaigns, &mut findings);
    add_system_health_findings(aggregate, campaigns, &mut findings);
    add_choice_quality_finding(aggregate, &mut findings);
    add_institutional_reach_finding(campaigns, &mut findings);
    add_strategic_cadence_finding(aggregate, campaigns, &mut findings);
    add_phase_quality_findings(aggregate, &mut findings);
    add_core_fantasy_findings(aggregate, campaigns, &mut findings);
    add_variance_finding(campaigns, &mut findings);
    if findings.is_empty() {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Info,
            title: "No material gameplay harness concerns".to_owned(),
            evidence: "All configured command and system thresholds were satisfied.".to_owned(),
        });
    }
    findings
}

fn add_rival_commercial_pressure_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 3_600 || campaigns.len() < 4 {
        return;
    }
    let pressured = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_contract_relationship_pressure_basis_points >= 1_000)
        .count();
    if scaled_ratio_usize(pressured, campaigns.len(), 100) >= 50 {
        return;
    }
    let maximum = campaigns
        .iter()
        .map(|campaign| campaign.maximum_contract_relationship_pressure_basis_points)
        .max()
        .unwrap_or(0);
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Rivalry rarely changes commercial leverage".to_owned(),
        evidence: format!(
            "Only {pressured} of {} mature campaigns ever reached 1,000 bp of relationship-driven contract pressure; the maximum observed pressure was {maximum} bp. Rival houses may dislike the player, but that hostility is not consistently changing the price of doing business with them.",
            campaigns.len()
        ),
    });
}

fn add_succession_cohesion_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let succession_campaigns: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.fantasy_arc.first_succession_day.is_some())
        .collect();
    if succession_campaigns.len() < 4 {
        return;
    }
    let highly_stable = succession_campaigns
        .iter()
        .filter(|campaign| {
            campaign
                .minimum_post_succession_family_unity
                .is_some_and(|unity| unity >= 7_000)
        })
        .count();
    if scaled_ratio_usize(highly_stable, succession_campaigns.len(), 100) < 75 {
        return;
    }
    let minimum = succession_campaigns
        .iter()
        .filter_map(|campaign| campaign.minimum_post_succession_family_unity)
        .min()
        .unwrap_or(10_000);
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Succession rarely destabilizes family cohesion".to_owned(),
        evidence: format!(
            "{highly_stable} of {} succession campaigns never fell below 7,000 bp of family unity after transition; the lowest observed post-succession unity was {minimum} bp. Inheritance changes the officeholder, but the family order is usually too stable to demand a new internal strategy.",
            succession_campaigns.len()
        ),
    });
}

fn add_strategic_cadence_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if aggregate.decision_cycles == 0 {
        return;
    }
    let static_quiet_cycles = aggregate
        .quiet_cycles
        .saturating_sub(aggregate.quiet_cycles_with_ambient_change);
    let static_quiet_share = scaled_ratio_u64(static_quiet_cycles, aggregate.decision_cycles, 100);
    if static_quiet_share < 25 {
        return;
    }
    let worst = campaigns.iter().max_by_key(|campaign| {
        let static_quiet = campaign
            .quiet_cycles
            .saturating_sub(campaign.quiet_cycles_with_ambient_change);
        scaled_ratio_u64(
            u64::from(static_quiet),
            u64::from(campaign.decision_cycles),
            100,
        )
    });
    let worst_evidence = worst.map_or_else(String::new, |campaign| {
        let campaign_static_quiet = campaign
            .quiet_cycles
            .saturating_sub(campaign.quiet_cycles_with_ambient_change);
        let campaign_static_quiet_share = scaled_ratio_u64(
            u64::from(campaign_static_quiet),
            u64::from(campaign.decision_cycles),
            100,
        );
        format!(
            " The most static campaign was seed {}, {} {:?}, at {campaign_static_quiet_share}%.",
            campaign.seed,
            campaign.persona.label(),
            campaign.background
        )
    });
    findings.push(GameplayFinding {
        severity: if static_quiet_share >= 40 {
            GameplayFindingSeverity::Critical
        } else {
            GameplayFindingSeverity::Warning
        },
        title: "Strategic cadence leaves too many static decision cycles".to_owned(),
        evidence: format!(
            "{} of {} decision cycles were quiet, but {} still contained ambient world change. The remaining {static_quiet_cycles} static cycles were {static_quiet_share}% of all decisions.{worst_evidence}",
            aggregate.quiet_cycles,
            aggregate.decision_cycles,
            aggregate.quiet_cycles_with_ambient_change,
        ),
    });
}

#[derive(Clone, Copy)]
struct PhaseQualityThresholds {
    minimum_action_share: u64,
    maximum_static_quiet_share: u64,
    maximum_quiet_streak_cycles: u32,
    minimum_multi_family_share: u64,
    minimum_average_choices_tenths: u64,
    minimum_average_families_tenths: u64,
}

fn add_phase_quality_findings(aggregate: &GameplayAggregate, findings: &mut Vec<GameplayFinding>) {
    add_phase_quality_finding(
        aggregate,
        findings,
        GameplayPhase::Establishment,
        "establishment",
        "Establishment becomes a waiting phase",
        PhaseQualityThresholds {
            minimum_action_share: 60,
            maximum_static_quiet_share: 40,
            maximum_quiet_streak_cycles: 6,
            minimum_multi_family_share: 25,
            minimum_average_choices_tenths: 25,
            minimum_average_families_tenths: 16,
        },
    );
    add_phase_quality_finding(
        aggregate,
        findings,
        GameplayPhase::InstitutionalAscent,
        "ascent",
        "Institutional ascent lacks parallel political work",
        PhaseQualityThresholds {
            minimum_action_share: 60,
            maximum_static_quiet_share: 35,
            maximum_quiet_streak_cycles: 6,
            minimum_multi_family_share: 25,
            minimum_average_choices_tenths: 25,
            minimum_average_families_tenths: 15,
        },
    );
    add_phase_quality_finding(
        aggregate,
        findings,
        GameplayPhase::DynasticGovernance,
        "governance",
        "Dynastic governance remains intermittent and strategically narrow",
        PhaseQualityThresholds {
            minimum_action_share: 0,
            maximum_static_quiet_share: 30,
            maximum_quiet_streak_cycles: 10,
            minimum_multi_family_share: 30,
            minimum_average_choices_tenths: 30,
            minimum_average_families_tenths: 20,
        },
    );
    add_phase_quality_finding(
        aggregate,
        findings,
        GameplayPhase::SuccessionLegacy,
        "succession and legacy",
        "Succession and legacy lack post-transition strategy",
        PhaseQualityThresholds {
            minimum_action_share: 55,
            maximum_static_quiet_share: 35,
            maximum_quiet_streak_cycles: 8,
            minimum_multi_family_share: 30,
            minimum_average_choices_tenths: 25,
            minimum_average_families_tenths: 18,
        },
    );
}

fn add_phase_quality_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
    phase: GameplayPhase,
    phase_label: &str,
    title: &str,
    thresholds: PhaseQualityThresholds,
) {
    let stats = aggregate
        .phase_stats
        .get(&phase)
        .copied()
        .unwrap_or_default();
    if stats.decision_cycles < 20 {
        return;
    }
    let action_share = scaled_ratio_u64(
        u64::from(stats.substantive_actions),
        u64::from(stats.decision_cycles),
        100,
    );
    let quiet_share = scaled_ratio_u64(
        u64::from(stats.quiet_cycles),
        u64::from(stats.decision_cycles),
        100,
    );
    let static_quiet_cycles = stats
        .quiet_cycles
        .saturating_sub(stats.quiet_cycles_with_ambient_change);
    let static_quiet_share = scaled_ratio_u64(
        u64::from(static_quiet_cycles),
        u64::from(stats.decision_cycles),
        100,
    );
    let multi_family_share = scaled_ratio_u64(
        u64::from(stats.cycles_with_multiple_viable_command_kinds),
        u64::from(stats.decision_cycles),
        100,
    );
    let opportunity_cycles = stats.decision_cycles.saturating_sub(stats.quiet_cycles);
    let average_choices_tenths = scaled_ratio_u64(
        u64::from(stats.total_viable_choices),
        u64::from(opportunity_cycles),
        10,
    );
    let average_families_tenths = scaled_ratio_u64(
        u64::from(stats.total_viable_command_kinds),
        u64::from(opportunity_cycles),
        10,
    );
    let choice_depth_is_sufficient = average_families_tenths
        >= thresholds.minimum_average_families_tenths
        || average_choices_tenths >= thresholds.minimum_average_choices_tenths;
    let mut missed_thresholds = Vec::new();
    if action_share < thresholds.minimum_action_share {
        missed_thresholds.push(format!(
            "action share {action_share}% < {}%",
            thresholds.minimum_action_share
        ));
    }
    if static_quiet_share >= thresholds.maximum_static_quiet_share {
        missed_thresholds.push(format!(
            "static quiet share {static_quiet_share}% >= {}%",
            thresholds.maximum_static_quiet_share
        ));
    }
    if stats.longest_quiet_streak_cycles > thresholds.maximum_quiet_streak_cycles {
        missed_thresholds.push(format!(
            "longest quiet streak {} > {} cycles",
            stats.longest_quiet_streak_cycles, thresholds.maximum_quiet_streak_cycles
        ));
    }
    if multi_family_share < thresholds.minimum_multi_family_share {
        missed_thresholds.push(format!(
            "multi-family share {multi_family_share}% < {}%",
            thresholds.minimum_multi_family_share
        ));
    }
    if !choice_depth_is_sufficient {
        missed_thresholds.push(format!(
            "choice depth {} choices / {} families < {} choices or {} families",
            format_tenths(average_choices_tenths),
            format_tenths(average_families_tenths),
            format_tenths(thresholds.minimum_average_choices_tenths),
            format_tenths(thresholds.minimum_average_families_tenths)
        ));
    }
    if missed_thresholds.is_empty() {
        return;
    }
    let threshold_evidence = missed_thresholds.join("; ");
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: title.to_owned(),
        evidence: format!(
            "Across {} {phase_label} cycles, substantive actions occurred in {action_share}%, {quiet_share}% were quiet, {}% were quiet while the world still changed, {static_quiet_share}% were static, the longest quiet streak lasted {} cycles, multiple command families were viable in {multi_family_share}%, and actionable cycles averaged {} viable choices across {} families. Thresholds missed: {threshold_evidence}.",
            stats.decision_cycles,
            scaled_ratio_u64(
                u64::from(stats.quiet_cycles_with_ambient_change),
                u64::from(stats.decision_cycles),
                100,
            ),
            stats.longest_quiet_streak_cycles,
            format_tenths(average_choices_tenths),
            format_tenths(average_families_tenths)
        ),
    });
}

fn add_repetitive_command_streak_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let Some(campaign) = campaigns
        .iter()
        .max_by_key(|campaign| campaign.longest_substantive_command_streak)
    else {
        return;
    };
    if campaign.longest_substantive_command_streak < 8 {
        return;
    }
    let command = campaign
        .longest_substantive_streak_command
        .map_or("unknown", GameplayCommandKind::label);
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Repeated command streak resembles routine micromanagement".to_owned(),
        evidence: format!(
            "The longest streak was {} consecutive {command} actions for seed {}, {} {:?}.",
            campaign.longest_substantive_command_streak,
            campaign.seed,
            campaign.persona.label(),
            campaign.background
        ),
    });
}

fn add_information_routine_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let routine_campaigns: Vec<_> = campaigns
        .iter()
        .filter(|campaign| {
            campaign.maximum_contract_relationship_pressure_basis_points
                < AGENT_INFORMATION_SEVERE_COUNTERPARTY_PRESSURE_BASIS_POINTS
        })
        .collect();
    if routine_campaigns.is_empty() {
        return;
    }
    let commissions: u32 = routine_campaigns
        .iter()
        .map(|campaign| {
            campaign
                .commands
                .get(&GameplayCommandKind::CommissionInformation)
                .map_or(0, |stats| stats.executed)
        })
        .sum();
    let pairs: u32 = routine_campaigns
        .iter()
        .map(|campaign| u32::from(campaign.commission_leverage_pairs))
        .sum();
    let simulated_days = routine_campaigns.iter().fold(0_u64, |total, campaign| {
        total.saturating_add(u64::from(campaign.simulated_days))
    });
    let commissions_per_hundred_campaign_years = scaled_ratio_u64(
        u64::from(commissions).saturating_mul(360),
        simulated_days.max(1),
        100,
    );
    if commissions < 20
        || pairs.saturating_mul(100) < commissions.saturating_mul(75)
        || commissions_per_hundred_campaign_years < 50
    {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Commissioned intelligence becomes a routine two-step ritual".to_owned(),
        evidence: format!(
            "{pairs} of {commissions} commissioned reports in non-severe-pressure campaigns were leveraged within {INFORMATION_ROUTINE_PAIR_WINDOW_DAYS} days, at a rate of {commissions_per_hundred_campaign_years} commissions per 100 campaign-years. Intelligence is functioning, but the repeated commission-then-spend sequence risks becoming scheduled maintenance rather than a response to uncertainty. Campaigns that reached at least {AGENT_INFORMATION_SEVERE_COUNTERPARTY_PRESSURE_BASIS_POINTS} bp of relationship-driven contract pressure are excluded because their faster political intelligence cadence is an explicit response to material exposure."
        ),
    });
}

fn add_crisis_trajectory_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let stats = aggregate
        .commands
        .get(&GameplayCommandKind::RespondToCrisis)
        .expect("crisis response statistics must exist");
    if stats.executed < 20 {
        return;
    }
    let future_consequences = stats
        .actions_with_persistent_consequences
        .max(stats.actions_with_delayed_consequences);
    let future_share = scaled_ratio_u64(
        u64::from(future_consequences),
        u64::from(stats.executed),
        100,
    );
    if future_share >= 50 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Crisis responses rarely change the future trajectory".to_owned(),
        evidence: format!(
            "At least {future_consequences} of {} crisis responses produced an action-attributable consequence that persisted or emerged after time advanced ({future_share}%). Crises are visible and actionable, but intervention seldom changes what happens after the immediate resolution step.",
            stats.executed,
        ),
    });
}

fn add_office_directive_trajectory_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 1_800 {
        return;
    }
    let stats = aggregate
        .commands
        .get(&GameplayCommandKind::ExerciseOfficePower)
        .expect("office-power statistics must exist");
    if stats.executed < 20 {
        return;
    }
    let delayed_share = scaled_ratio_u64(
        u64::from(stats.actions_with_delayed_consequences),
        u64::from(stats.executed),
        100,
    );
    if delayed_share >= 15 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Office directives rarely alter the later trajectory".to_owned(),
        evidence: format!(
            "Only {} of {} office directives produced a newly attributable consequence after time advanced ({delayed_share}%). Directives create immediate visible effects, but mature political power is not consistently changing later system behavior.",
            stats.actions_with_delayed_consequences,
            stats.executed,
        ),
    });
}

fn add_welfare_dynamism_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 1_800 || campaigns.is_empty() {
        return;
    }
    let crisis_exposed: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_active_crises > 0)
        .collect();
    if crisis_exposed.len() < 4 {
        return;
    }
    let mechanically_stable = crisis_exposed
        .iter()
        .filter(|campaign| campaign.minimum_district_food_satisfaction >= 9_500)
        .count();
    if scaled_ratio_usize(mechanically_stable, crisis_exposed.len(), 100) < 75 {
        return;
    }
    let minimum = crisis_exposed
        .iter()
        .map(|campaign| campaign.minimum_district_food_satisfaction)
        .min()
        .unwrap_or(10_000);
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Crises leave household welfare almost mechanically flat".to_owned(),
        evidence: format!(
            "{mechanically_stable} of {} crisis-exposed campaigns kept their worst district at or above 95% food satisfaction; the lowest observed district value was {:.2}%. The city reacts in logs and crisis counters, but ordinary households experience little material disruption.",
            crisis_exposed.len(),
            f64::from(minimum) / 100.0,
        ),
    });
}

fn add_long_horizon_risk_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 3_600 || campaigns.is_empty() {
        return;
    }
    let credit_actions = aggregate
        .commands
        .get(&GameplayCommandKind::ExtendCredit)
        .map_or(0, |stats| stats.executed);
    let credit_distress = campaigns.iter().any(|campaign| {
        campaign.maximum_delinquent_loans > 0 || campaign.maximum_defaulted_loans > 0
    });
    if credit_actions >= 50 && !credit_distress {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Long-horizon lending never encounters credit distress".to_owned(),
            evidence: format!(
                "Agents extended credit {credit_actions} times, but no campaign ever recorded a delinquent or defaulted private loan. Credit is profitable and relational, but its downside remained untested."
            ),
        });
    }

    let civic_actions = aggregate
        .commands
        .get(&GameplayCommandKind::StartPublicWork)
        .map_or(0, |stats| stats.executed)
        .saturating_add(
            aggregate
                .commands
                .get(&GameplayCommandKind::EnactLaw)
                .map_or(0, |stats| stats.executed),
        );
    let civic_debt_activity = campaigns.iter().any(|campaign| {
        campaign.maximum_delinquent_civic_debts > 0
            || campaign.maximum_defaulted_civic_debts > 0
            || campaign.end.current_civic_debts > 0
            || campaign.end.repaid_civic_debts > 0
    });
    if civic_actions >= 20 && !civic_debt_activity {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Mature civic ambition never activates municipal finance".to_owned(),
            evidence: format!(
                "Agents enacted laws or sponsored public works {civic_actions} times without issuing, repaying, or distressing civic debt. City-shaping expenditure is not testing the municipal financing layer."
            ),
        });
    }
}

fn add_long_substantive_gap_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let eligible: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.simulated_days >= 720)
        .collect();
    if eligible.is_empty() {
        return;
    }
    let long_gaps = eligible
        .iter()
        .filter(|campaign| campaign.longest_substantive_action_gap_days >= 360)
        .count();
    let worst = eligible
        .iter()
        .max_by_key(|campaign| campaign.longest_substantive_action_gap_days)
        .expect("eligible campaigns must have a longest gap");
    if scaled_ratio_usize(long_gaps, eligible.len(), 100) >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Long stretches pass without a substantive player decision".to_owned(),
            evidence: format!(
                "{long_gaps} of {} campaigns had a decision gap of at least one year; the worst gap was {} days for seed {}, {} {:?}.",
                eligible.len(),
                worst.longest_substantive_action_gap_days,
                worst.seed,
                worst.persona.label(),
                worst.background
            ),
        });
    } else if worst.longest_substantive_action_gap_days >= 540 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "An individual campaign has a prolonged strategic drought".to_owned(),
            evidence: format!(
                "Seed {}, {} {:?} passed {} days without a substantive action even though the aggregate drought rate remained below 25%.",
                worst.seed,
                worst.persona.label(),
                worst.background,
                worst.longest_substantive_action_gap_days
            ),
        });
    }
}

fn add_asset_liquidity_drought_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let trapped = campaigns
        .iter()
        .find(|campaign| campaign.longest_asset_rich_quiet_gap_days >= 360);
    let Some(campaign) = trapped else {
        return;
    };
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Asset-rich dynasties can become decision-poor".to_owned(),
        evidence: format!(
            "Seed {}, {} {:?} spent {} consecutive days without a substantive action while treasury cash was below 40 cr, at least two properties were owned, and active businesses retained at least 100 cr. The harness should surface costly liquidity routes instead of treating locked wealth as unusable.",
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
            campaign.longest_asset_rich_quiet_gap_days
        ),
    });
}

fn add_economic_recovery_dead_end_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let trapped: Vec<_> = campaigns
        .iter()
        .filter(|campaign| {
            campaign.longest_substantive_action_gap_days >= 360
                && campaign.end.player_treasury <= Money::ZERO
                && campaign.end.active_businesses == 0
                && campaign
                    .end
                    .distressed_businesses
                    .saturating_add(campaign.end.insolvent_businesses)
                    > 0
                && campaign.end.player_properties == 0
                && campaign
                    .end
                    .current_loans
                    .saturating_add(campaign.end.delinquent_loans)
                    .saturating_add(campaign.end.restructured_loans)
                    == 0
        })
        .collect();
    if let Some(worst) = trapped
        .iter()
        .max_by_key(|campaign| campaign.longest_substantive_action_gap_days)
    {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Critical,
            title: "Economic failure can become an unrecoverable campaign state".to_owned(),
            evidence: format!(
                "{} campaign(s) ended with no treasury, no healthy business, no property, no active or restructured credit, and a year-scale decision drought. The worst was seed {}, {} {:?}, with {} days without a substantive route.",
                trapped.len(),
                worst.seed,
                worst.persona.label(),
                worst.background,
                worst.longest_substantive_action_gap_days
            ),
        });
    }

    if let Some(campaign) = campaigns
        .iter()
        .find(|campaign| campaign.terminal_recovery_pressure_days >= 360)
    {
        let borrowing = campaign
            .commands
            .get(&GameplayCommandKind::BorrowFunds)
            .map_or(0, |stats| stats.executed);
        let investment = campaign
            .commands
            .get(&GameplayCommandKind::InvestInBusiness)
            .map_or(0, |stats| stats.executed);
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "An individual dynasty remains trapped in recovery churn".to_owned(),
            evidence: format!(
                "Seed {}, {} {:?} remained under recovery pressure for {} consecutive days through the campaign endpoint, with no treasury, property, or active business and {} defaulted loans despite {borrowing} borrowing or restructuring actions and {investment} recapitalizations. Activity continued, but it did not produce a credible recovery path.",
                campaign.seed,
                campaign.persona.label(),
                campaign.background,
                campaign.terminal_recovery_pressure_days,
                campaign.end.defaulted_loans
            ),
        });
    }
}

fn add_campaign_blocking_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let Some((campaign, opportunity_cycles, blocked_share)) = campaigns
        .iter()
        .filter_map(|campaign| {
            let opportunity_cycles = campaign
                .decision_cycles
                .saturating_sub(campaign.quiet_cycles);
            (opportunity_cycles > 0).then_some((
                campaign,
                opportunity_cycles,
                scaled_ratio_u64(
                    u64::from(campaign.blocked_cycles),
                    u64::from(opportunity_cycles),
                    100,
                ),
            ))
        })
        .max_by_key(|(_, _, blocked_share)| *blocked_share)
    else {
        return;
    };
    if campaign.blocked_cycles < 4 || blocked_share < 25 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "An individual campaign becomes strategically blocked".to_owned(),
        evidence: format!(
            "{} of {opportunity_cycles} actionable cycles in seed {}, {} {:?} ended with no viable command ({blocked_share}%). Aggregate averages can hide this start-specific failure mode.",
            campaign.blocked_cycles,
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
        ),
    });
}

fn add_score_findings(aggregate: &GameplayAggregate, findings: &mut Vec<GameplayFinding>) {
    for (label, score) in [
        ("actionability", aggregate.scores.actionability),
        ("variety", aggregate.scores.variety),
        ("interconnection", aggregate.scores.interconnection),
        ("feedback", aggregate.scores.feedback),
        ("resilience", aggregate.scores.resilience),
    ] {
        let severity = if score < 30 {
            GameplayFindingSeverity::Critical
        } else if score < 60 {
            GameplayFindingSeverity::Warning
        } else {
            continue;
        };
        findings.push(GameplayFinding {
            severity,
            title: format!("Low {label} score"),
            evidence: format!("The aggregate {label} score was {score}/100."),
        });
    }
}

fn add_command_findings(aggregate: &GameplayAggregate, findings: &mut Vec<GameplayFinding>) {
    let campaign_days = average_campaign_days(aggregate);
    for kind in ALL_COMMAND_KINDS {
        let stats = aggregate
            .commands
            .get(&kind)
            .expect("every command kind must have aggregate statistics");
        if stats.executed == 0 {
            let (severity, title) = if stats.generated == 0 {
                if command_route_expected(aggregate, kind, campaign_days) {
                    (
                        GameplayFindingSeverity::Critical,
                        format!("{} had no reachable candidate", kind.label()),
                    )
                } else {
                    (
                        GameplayFindingSeverity::Info,
                        format!("{} was not exercised in this horizon", kind.label()),
                    )
                }
            } else if stats.considered == 0 {
                (
                    GameplayFindingSeverity::Warning,
                    format!("{} candidates were never probed", kind.label()),
                )
            } else if stats.viable == 0 {
                (
                    GameplayFindingSeverity::Critical,
                    format!("{} was always rejected", kind.label()),
                )
            } else if stats.offered_cycles < 3 {
                (
                    GameplayFindingSeverity::Info,
                    format!(
                        "{} appeared only as a rare unselected alternative",
                        kind.label()
                    ),
                )
            } else {
                (
                    GameplayFindingSeverity::Warning,
                    format!("{} was viable but never selected", kind.label()),
                )
            };
            findings.push(GameplayFinding {
                severity,
                title,
                evidence: format!(
                    "activation_opportunities={}, offered_cycles={}, generated={}, considered={}, viable={}, rejected={}; no configured agent executed it",
                    stats.activation_opportunities,
                    stats.offered_cycles,
                    stats.generated,
                    stats.considered,
                    stats.viable,
                    stats.rejected
                ),
            });
        } else if stats.changed_domains.is_empty() {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Warning,
                title: format!("{} produced no observed system change", kind.label()),
                evidence: format!("The command executed {} times.", stats.executed),
            });
        }
    }
}

fn add_domain_findings(aggregate: &GameplayAggregate, findings: &mut Vec<GameplayFinding>) {
    let campaign_days = average_campaign_days(aggregate);
    for domain in ALL_DOMAINS {
        let causal = aggregate
            .causal_domain_changes
            .get(&domain)
            .copied()
            .unwrap_or(0);
        let ambient = aggregate
            .ambient_domain_changes
            .get(&domain)
            .copied()
            .unwrap_or(0);
        let player_route_expected = domain_player_commands(domain)
            .iter()
            .any(|kind| command_route_expected(aggregate, *kind, campaign_days));
        if causal == 0 && ambient == 0 {
            findings.push(GameplayFinding {
                severity: if player_route_expected {
                    GameplayFindingSeverity::Warning
                } else {
                    GameplayFindingSeverity::Info
                },
                title: if player_route_expected {
                    format!("{} domain remained static", domain.label())
                } else {
                    format!("{} domain was inactive in this horizon", domain.label())
                },
                evidence: format!(
                    "No snapshot transition touched this domain across {campaign_days} days per campaign."
                ),
            });
        } else if causal == 0 {
            let player_route_offered = domain_player_commands(domain).iter().any(|kind| {
                aggregate
                    .commands
                    .get(kind)
                    .is_some_and(|stats| stats.offered_cycles > 0)
            });
            findings.push(GameplayFinding {
                severity: if player_route_offered || player_route_expected {
                    GameplayFindingSeverity::Warning
                } else {
                    GameplayFindingSeverity::Info
                },
                title: if player_route_offered || player_route_expected {
                    format!("{} domain is autonomous but not player-responsive", domain.label())
                } else {
                    format!(
                        "{} domain changed before a player route became available",
                        domain.label()
                    )
                },
                evidence: if player_route_offered {
                    format!(
                        "It changed in {ambient} baseline observations but no offered command produced an attributable transition."
                    )
                } else {
                    format!(
                        "It changed in {ambient} baseline observations, but no command associated with this domain was offered during the configured horizon."
                    )
                },
            });
        }
    }
}

fn command_route_expected(
    aggregate: &GameplayAggregate,
    kind: GameplayCommandKind,
    campaign_days: u64,
) -> bool {
    if kind.is_activation_dependent() {
        aggregate
            .commands
            .get(&kind)
            .is_some_and(|stats| stats.activation_opportunities > 0 || stats.offered_cycles > 0)
    } else {
        campaign_days >= u64::from(kind.expected_activation_days())
    }
}

fn average_campaign_days(aggregate: &GameplayAggregate) -> u64 {
    if aggregate.campaigns == 0 {
        0
    } else {
        aggregate.simulated_days / u64::from(aggregate.campaigns)
    }
}

const fn domain_player_commands(domain: GameplayDomain) -> &'static [GameplayCommandKind] {
    match domain {
        GameplayDomain::Economy => &[
            GameplayCommandKind::TransferBusinessCash,
            GameplayCommandKind::AcquireBusiness,
            GameplayCommandKind::InvestInBusiness,
            GameplayCommandKind::BorrowFunds,
            GameplayCommandKind::ExtendCredit,
            GameplayCommandKind::BuyProperty,
            GameplayCommandKind::SellProperty,
            GameplayCommandKind::CultivateInstitutionSupport,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Business => &[
            GameplayCommandKind::AcquireBusiness,
            GameplayCommandKind::InvestInBusiness,
            GameplayCommandKind::SetBusinessPolicy,
        ],
        GameplayDomain::Market => &[
            GameplayCommandKind::SetBusinessPolicy,
            GameplayCommandKind::SecureSupply,
            GameplayCommandKind::SellOutput,
            GameplayCommandKind::EnactLaw,
        ],
        GameplayDomain::Contracts => &[
            GameplayCommandKind::SecureSupply,
            GameplayCommandKind::SellOutput,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Loans => &[
            GameplayCommandKind::BorrowFunds,
            GameplayCommandKind::ExtendCredit,
        ],
        GameplayDomain::Property => &[
            GameplayCommandKind::BuyProperty,
            GameplayCommandKind::SellProperty,
        ],
        GameplayDomain::Labor => &[
            GameplayCommandKind::InvestInBusiness,
            GameplayCommandKind::SetBusinessPolicy,
            GameplayCommandKind::ResolveLaborDispute,
        ],
        GameplayDomain::Relationships => &[
            GameplayCommandKind::SecureSupply,
            GameplayCommandKind::SellOutput,
            GameplayCommandKind::BorrowFunds,
            GameplayCommandKind::ExtendCredit,
            GameplayCommandKind::SellProperty,
            GameplayCommandKind::FileLegalCase,
            GameplayCommandKind::CultivateInstitutionSupport,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Dynasty => &[
            GameplayCommandKind::BorrowFunds,
            GameplayCommandKind::ExtendCredit,
            GameplayCommandKind::SellProperty,
            GameplayCommandKind::EnactLaw,
            GameplayCommandKind::DesignateHeir,
            GameplayCommandKind::AdoptWard,
            GameplayCommandKind::EducateFamilyMember,
            GameplayCommandKind::CultivateInstitutionSupport,
            GameplayCommandKind::NominateForOffice,
            GameplayCommandKind::ExerciseOfficePower,
            GameplayCommandKind::WithdrawFromInstitution,
            GameplayCommandKind::RespondToCrisis,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Family => &[
            GameplayCommandKind::SetHouseGovernance,
            GameplayCommandKind::DesignateHeir,
            GameplayCommandKind::AdoptWard,
            GameplayCommandKind::EducateFamilyMember,
            GameplayCommandKind::WithdrawFromInstitution,
        ],
        GameplayDomain::Institutions => &[
            GameplayCommandKind::StartPublicWork,
            GameplayCommandKind::CultivateInstitutionSupport,
            GameplayCommandKind::NominateForOffice,
            GameplayCommandKind::ExerciseOfficePower,
            GameplayCommandKind::WithdrawFromInstitution,
        ],
        GameplayDomain::Law => &[GameplayCommandKind::EnactLaw],
        GameplayDomain::Districts => &[
            GameplayCommandKind::StartPublicWork,
            GameplayCommandKind::ExerciseOfficePower,
            GameplayCommandKind::RespondToCrisis,
            GameplayCommandKind::ResolveLaborDispute,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Legal => &[GameplayCommandKind::FileLegalCase],
        GameplayDomain::Crises => &[GameplayCommandKind::RespondToCrisis],
        GameplayDomain::Information => &[
            GameplayCommandKind::SecureSupply,
            GameplayCommandKind::SellOutput,
            GameplayCommandKind::BorrowFunds,
            GameplayCommandKind::ExtendCredit,
            GameplayCommandKind::CommissionInformation,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Feedback => &ALL_COMMAND_KINDS,
    }
}

fn add_action_concentration_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let Some((kind, stats)) = aggregate
        .commands
        .iter()
        .filter(|(kind, _)| **kind != GameplayCommandKind::AcknowledgeNotification)
        .max_by_key(|(_, stats)| stats.executed)
    else {
        return;
    };
    if aggregate.substantive_actions == 0 {
        return;
    }
    let share = scaled_ratio_u64(
        u64::from(stats.executed),
        aggregate.substantive_actions,
        100,
    );
    if share < 35 {
        return;
    }
    findings.push(GameplayFinding {
        severity: if share >= 60 {
            GameplayFindingSeverity::Critical
        } else {
            GameplayFindingSeverity::Warning
        },
        title: format!("{} dominates player decisions", kind.label()),
        evidence: format!(
            "It accounted for {share}% of {} executed actions.",
            aggregate.substantive_actions
        ),
    });
}

fn add_institutional_campaign_concentration_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    if aggregate.substantive_actions == 0 {
        return;
    }
    let support_actions = aggregate
        .commands
        .get(&GameplayCommandKind::CultivateInstitutionSupport)
        .map_or(0, |stats| stats.executed);
    let nomination_actions = aggregate
        .commands
        .get(&GameplayCommandKind::NominateForOffice)
        .map_or(0, |stats| stats.executed);
    let campaign_actions = support_actions.saturating_add(nomination_actions);
    if campaign_actions < 20 {
        return;
    }
    let share = scaled_ratio_u64(
        u64::from(campaign_actions),
        aggregate.substantive_actions,
        100,
    );
    if share < 35 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Institutional campaigning dominates the decision loop".to_owned(),
        evidence: format!(
            "Patronage and nominations accounted for {campaign_actions} of {} substantive actions ({share}%). Family political capacity should create strategic reach without becoming recurring campaign administration.",
            aggregate.substantive_actions
        ),
    });
}

fn add_phase_institutional_campaign_concentration_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let stats = aggregate
        .phase_stats
        .get(&GameplayPhase::InstitutionalAscent)
        .copied()
        .unwrap_or_default();
    if stats.substantive_actions < 20 || stats.institutional_campaign_actions < 20 {
        return;
    }
    let share = scaled_ratio_u64(
        u64::from(stats.institutional_campaign_actions),
        u64::from(stats.substantive_actions),
        100,
    );
    if share < 65 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Institutional ascent becomes campaign administration".to_owned(),
        evidence: format!(
            "Patronage and nominations accounted for {} of {} substantive institutional-ascent actions ({share}%). Political ascent should still leave room for commercial, family, information, and civic decisions while support and campaigns mature.",
            stats.institutional_campaign_actions, stats.substantive_actions
        ),
    });
}

fn add_business_survival_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.is_empty() {
        return;
    }
    let non_operational = campaigns
        .iter()
        .filter(|campaign| {
            campaign
                .end
                .active_businesses
                .saturating_add(campaign.end.distressed_businesses)
                == 0
        })
        .count();
    if non_operational > 0 {
        let share = scaled_ratio_usize(non_operational, campaigns.len(), 100);
        findings.push(GameplayFinding {
            severity: if share >= 50 {
                GameplayFindingSeverity::Critical
            } else {
                GameplayFindingSeverity::Warning
            },
            title: "Player businesses become non-operational".to_owned(),
            evidence: format!(
                "{non_operational} of {} campaigns ended with every player business insolvent or closed ({share}%).",
                campaigns.len()
            ),
        });
    }
    let fully_stressed = campaigns
        .iter()
        .filter(|campaign| {
            campaign.end.active_businesses == 0
                && campaign
                    .end
                    .distressed_businesses
                    .saturating_add(campaign.end.insolvent_businesses)
                    > 0
        })
        .count();
    let stressed_share = scaled_ratio_usize(fully_stressed, campaigns.len(), 100);
    if stressed_share >= 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Player portfolios frequently lack a healthy active business".to_owned(),
            evidence: format!(
                "{fully_stressed} of {} campaigns ended with every player business distressed or insolvent ({stressed_share}%).",
                campaigns.len()
            ),
        });
    }
}

fn add_choice_quality_finding(aggregate: &GameplayAggregate, findings: &mut Vec<GameplayFinding>) {
    let opportunity_cycles = aggregate
        .decision_cycles
        .saturating_sub(aggregate.quiet_cycles);
    if opportunity_cycles == 0 {
        return;
    }
    let average_kinds_tenths =
        scaled_ratio_u64(aggregate.viable_command_kinds, opportunity_cycles, 10);
    let average_choices_tenths = scaled_ratio_u64(aggregate.viable_choices, opportunity_cycles, 10);
    let multiple_share = scaled_ratio_u64(
        aggregate.cycles_with_multiple_viable_command_kinds,
        opportunity_cycles,
        100,
    );
    add_choice_breadth_finding(
        average_choices_tenths,
        average_kinds_tenths,
        multiple_share,
        findings,
    );
    add_choice_tradeoff_findings(aggregate, findings);
    add_blocked_choice_finding(aggregate, opportunity_cycles, findings);
}

fn add_choice_breadth_finding(
    average_choices_tenths: u64,
    average_kinds_tenths: u64,
    multiple_share: u64,
    findings: &mut Vec<GameplayFinding>,
) {
    if average_choices_tenths < 20 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Actionable cycles offer too few meaningful alternatives".to_owned(),
            evidence: format!(
                "The average actionable cycle exposed {} viable choices across {} command families; {multiple_share}% offered at least two substantive families.",
                format_tenths(average_choices_tenths),
                format_tenths(average_kinds_tenths)
            ),
        });
    } else if average_kinds_tenths < 15 || multiple_share < 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Actionable cycles are usually single-track".to_owned(),
            evidence: format!(
                "The average actionable cycle exposed {} viable choices but only {} command families; just {multiple_share}% offered at least two substantive families. Mature play risks becoming a sequence of predetermined task categories rather than competing plans.",
                format_tenths(average_choices_tenths),
                format_tenths(average_kinds_tenths)
            ),
        });
    } else if average_kinds_tenths < 20 || multiple_share < 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Info,
            title: "Strategic alternatives concentrate within command families".to_owned(),
            evidence: format!(
                "The average actionable cycle exposed {} viable choices but only {} command families; {multiple_share}% offered at least two families. Policy templates, targets, projects, and counterparties provide choice depth even when the strategic category is focused.",
                format_tenths(average_choices_tenths),
                format_tenths(average_kinds_tenths)
            ),
        });
    }
}

fn add_choice_tradeoff_findings(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    if aggregate.cycles_with_multiple_viable_command_kinds < 20 {
        return;
    }
    let denominator = aggregate.cycles_with_multiple_viable_command_kinds;
    let close_share = scaled_ratio_u64(
        aggregate.cycles_with_close_viable_command_kinds,
        denominator,
        100,
    );
    let immediate_share = scaled_ratio_u64(
        aggregate.cycles_with_distinct_immediate_consequences,
        denominator,
        100,
    );
    let projected_share = scaled_ratio_u64(
        aggregate.cycles_with_distinct_projected_consequences,
        denominator,
        100,
    );
    if close_share < 20 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Most multi-option cycles still have an obvious winner".to_owned(),
            evidence: format!(
                "Only {} of {} cycles with multiple viable command families placed the two highest-ranked viable families within {CLOSE_CHOICE_SCORE_GAP} score points ({close_share}%). The harness sees breadth, but the agent rarely faces a close strategic tradeoff.",
                aggregate.cycles_with_close_viable_command_kinds,
                denominator
            ),
        });
    }
    if immediate_share < 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Viable alternatives often share the same immediate consequence profile"
                .to_owned(),
            evidence: format!(
                "Only {} of {} multi-family cycles exposed at least two distinct immediate domain-change profiles ({immediate_share}%). Delayed effects may still diverge, but the first-order feedback risks making different commands feel interchangeable.",
                aggregate.cycles_with_distinct_immediate_consequences,
                denominator
            ),
        });
    }
    if projected_share < 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Strategic alternatives converge after one decision interval".to_owned(),
            evidence: format!(
                "Only {} of {} multi-family cycles produced at least two distinct projected domain-change profiles after one decision interval ({projected_share}%). Immediate feedback may differ while the simulated trajectories still converge.",
                aggregate.cycles_with_distinct_projected_consequences,
                denominator
            ),
        });
    }
}

fn add_blocked_choice_finding(
    aggregate: &GameplayAggregate,
    opportunity_cycles: u64,
    findings: &mut Vec<GameplayFinding>,
) {
    let blocked_share = scaled_ratio_u64(aggregate.blocked_cycles, opportunity_cycles, 100);
    if blocked_share >= 10 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Substantive choices are frequently blocked".to_owned(),
            evidence: format!(
                "{} of {opportunity_cycles} cycles with substantive candidates ended without a viable action ({blocked_share}%).",
                aggregate.blocked_cycles
            ),
        });
    }
}

fn add_institutional_reach_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let eligible: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.simulated_days >= 1_800 && campaign.end.available_offices >= 5)
        .collect();
    if eligible.is_empty() {
        return;
    }
    let near_universal = eligible
        .iter()
        .filter(|campaign| {
            u32::from(campaign.end.player_institutions_represented).saturating_mul(100)
                >= u32::from(campaign.end.available_offices).saturating_mul(80)
        })
        .count();
    let share = scaled_ratio_usize(near_universal, eligible.len(), 100);
    if share >= 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Dynasty networks become institutionally universal".to_owned(),
            evidence: format!(
                "{near_universal} of {} mature campaigns ended with player representation in at least 80% of institutions ({share}%). Family growth should create parallel strategies, but near-universal access weakens specialization, coalition choice, and the cost of succession.",
                eligible.len()
            ),
        });
    }
}

fn format_tenths(value: u64) -> String {
    format!("{}.{:01}", value / 10, value % 10)
}

fn add_system_health_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.is_empty() {
        return;
    }
    add_food_health_findings(campaigns, findings);
    add_economic_health_findings(campaigns, findings);
    add_business_condition_finding(campaigns, findings);
    add_public_work_health_finding(aggregate, campaigns, findings);
    add_political_health_finding(aggregate, campaigns, findings);
    add_feed_health_findings(aggregate, campaigns, findings);
}

fn add_food_health_findings(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let collapsed_food = campaigns
        .iter()
        .filter(|campaign| {
            campaign.end.average_food_satisfaction < 1_000
                || campaign.minimum_food_satisfaction < 1_000
        })
        .count();
    if collapsed_food > 0 {
        let share = scaled_ratio_usize(collapsed_food, campaigns.len(), 100);
        findings.push(GameplayFinding {
            severity: if share >= 25 {
                GameplayFindingSeverity::Critical
            } else {
                GameplayFindingSeverity::Warning
            },
            title: "At least one campaign experiences complete food collapse".to_owned(),
            evidence: format!(
                "{collapsed_food} of {} campaigns fell below 10% food satisfaction at an endpoint or during the simulated trajectory ({share}%).",
                campaigns.len()
            ),
        });
    }
    let low_food = campaigns
        .iter()
        .filter(|campaign| campaign.end.average_food_satisfaction < 3_000)
        .count();
    if scaled_ratio_usize(low_food, campaigns.len(), 100) >= 25 && low_food > collapsed_food {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Critical,
            title: "Household food access collapses in many campaigns".to_owned(),
            evidence: format!(
                "{low_food} of {} campaigns ended below 30% food satisfaction.",
                campaigns.len()
            ),
        });
    }
}

fn add_economic_health_findings(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let player_fulfilled: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.player_fulfilled_contracts))
        .sum();
    let player_breached: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.player_breached_contracts))
        .sum();
    if player_breached > player_fulfilled && player_breached > 0 {
        let player_failures: u64 = campaigns
            .iter()
            .map(|campaign| u64::from(campaign.end.player_contract_failures))
            .sum();
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Player contracts breach more often than they complete".to_owned(),
            evidence: format!(
                "Player businesses ended with {player_breached} breached and {player_fulfilled} fulfilled contracts after {player_failures} missed deliveries."
            ),
        });
    }
    let fulfilled: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.fulfilled_contracts))
        .sum();
    let breached: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.breached_contracts))
        .sum();
    if breached > fulfilled.saturating_mul(2) && breached > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Contracts fail more often than they complete".to_owned(),
            evidence: format!("Observed {breached} breached and {fulfilled} fulfilled contracts."),
        });
    }
    let defaults: u64 = campaigns
        .iter()
        .map(|campaign| {
            u64::from(campaign.end.defaulted_loans)
                .saturating_add(u64::from(campaign.end.defaulted_civic_debts))
        })
        .sum();
    let repaid: u64 = campaigns
        .iter()
        .map(|campaign| {
            u64::from(campaign.end.repaid_loans)
                .saturating_add(u64::from(campaign.end.repaid_civic_debts))
        })
        .sum();
    if defaults > repaid && defaults > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Credit defaults outnumber successful repayments".to_owned(),
            evidence: format!(
                "Observed {defaults} defaulted and {repaid} repaid private or municipal obligations."
            ),
        });
    }
    let disputed: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.player_disputed_employment))
        .sum();
    let active: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.player_active_employment))
        .sum();
    if disputed > active && disputed > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Labor disputes dominate the player workforce".to_owned(),
            evidence: format!(
                "Player endpoints contained {disputed} disputed and {active} active agreements."
            ),
        });
    }
}

fn add_business_condition_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let collapsed = campaigns
        .iter()
        .filter(|campaign| campaign.end.average_business_condition < 1_000)
        .count();
    if collapsed == 0 {
        return;
    }
    let share = scaled_ratio_usize(collapsed, campaigns.len(), 100);
    findings.push(GameplayFinding {
        severity: if share >= 50 {
            GameplayFindingSeverity::Critical
        } else {
            GameplayFindingSeverity::Warning
        },
        title: "Business condition collapses over the campaign".to_owned(),
        evidence: format!(
            "{collapsed} of {} campaigns ended below 10% average business condition ({share}%).",
            campaigns.len()
        ),
    });
}

fn add_public_work_health_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let starts = aggregate
        .commands
        .get(&GameplayCommandKind::StartPublicWork)
        .map_or(0_u64, |stats| u64::from(stats.executed));
    if starts == 0 {
        return;
    }
    let overloaded = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_unfinished_public_works > 4)
        .count();
    if scaled_ratio_usize(overloaded, campaigns.len(), 100) < 25 {
        return;
    }
    let completed: u64 = campaigns
        .iter()
        .map(|campaign| {
            u64::from(
                campaign
                    .end
                    .completed_public_works
                    .saturating_sub(campaign.start.completed_public_works),
            )
        })
        .sum();
    let suspended: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.suspended_public_works))
        .sum();
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Public works accumulate faster than the city can execute them".to_owned(),
        evidence: format!(
            "{overloaded} of {} campaigns exceeded four unfinished projects; agents started {starts}, completed {completed}, and ended with {suspended} suspended projects.",
            campaigns.len()
        ),
    });
}

fn add_political_health_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let nominations = aggregate
        .commands
        .get(&GameplayCommandKind::NominateForOffice)
        .map_or(0_u64, |stats| u64::from(stats.executed));
    let offices_ever_held: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.maximum_offices_held))
        .sum();
    if nominations > 0 && offices_ever_held == 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Critical,
            title: "Office nominations never produce political power".to_owned(),
            evidence: format!(
                "The harness executed {nominations} nominations without any campaign ever producing a player officeholder."
            ),
        });
    }
    let complete_capture = campaigns
        .iter()
        .filter(|campaign| {
            campaign.end.available_offices > 1
                && campaign.maximum_offices_held >= campaign.end.available_offices
        })
        .count();
    if scaled_ratio_usize(complete_capture, campaigns.len(), 100) >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Player captures every political office".to_owned(),
            evidence: format!(
                "{complete_capture} of {} campaigns held every available office at some point.",
                campaigns.len()
            ),
        });
    }
    let officeholder_capacity_capture = campaigns
        .iter()
        .filter(|campaign| {
            let effective_capacity = campaign
                .end
                .available_offices
                .min(campaign.end.eligible_officeholders);
            effective_capacity > 1
                && campaign.end.active_wards == 0
                && campaign.maximum_offices_held >= effective_capacity
        })
        .count();
    if scaled_ratio_usize(officeholder_capacity_capture, campaigns.len(), 100) >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Dynasty fills every available officeholder slot".to_owned(),
            evidence: format!(
                "{officeholder_capacity_capture} of {} campaigns filled every office slot their active family members could legally occupy without adopting a ward, so political growth stalled at the founding household.",
                campaigns.len()
            ),
        });
    }
}

fn add_feed_health_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let overloaded = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_unread_notifications > 100)
        .count();
    if scaled_ratio_usize(overloaded, campaigns.len(), 100) >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Notification volume exceeds a usable decision feed".to_owned(),
            evidence: format!(
                "{overloaded} of {} campaigns accumulated more than 100 unread notifications.",
                campaigns.len()
            ),
        });
    } else if overloaded > 0 {
        let worst = campaigns
            .iter()
            .max_by_key(|campaign| campaign.maximum_unread_notifications)
            .expect("non-empty campaigns must have a maximum notification backlog");
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Individual campaigns experience notification overload".to_owned(),
            evidence: format!(
                "{overloaded} of {} campaigns exceeded 100 unread notifications; the worst reached {} ({:?}, {:?}, seed {}).",
                campaigns.len(),
                worst.maximum_unread_notifications,
                worst.persona,
                worst.background,
                worst.seed
            ),
        });
    }
    let crisis_actions = aggregate
        .commands
        .get(&GameplayCommandKind::RespondToCrisis)
        .map_or(0_u64, |stats| u64::from(stats.executed));
    let crisis_share = scaled_ratio_u64(crisis_actions, aggregate.substantive_actions, 100);
    if aggregate.substantive_actions > 0 && crisis_share >= 35 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Crisis response crowds out strategic play".to_owned(),
            evidence: format!(
                "Crisis responses accounted for {crisis_share}% of executed actions."
            ),
        });
    }
}

fn add_core_fantasy_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    add_fantasy_arc_findings(aggregate, campaigns, findings);
    add_information_agency_finding(aggregate, findings);
    add_power_conversion_finding(aggregate, findings);
    add_player_labor_agency_finding(aggregate, campaigns, findings);
    add_persona_convergence_finding(campaigns, findings);
    add_civic_convergence_finding(aggregate, campaigns, findings);
    add_power_exposure_finding(aggregate, campaigns, findings);
    add_office_duty_failure_finding(aggregate, campaigns, findings);
    add_dynastic_continuity_finding(aggregate, campaigns, findings);
}

fn add_fantasy_arc_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.is_empty() {
        return;
    }
    add_fantasy_arc_order_finding(campaigns, findings);
    add_fantasy_arc_compression_findings(campaigns, findings);
    add_absolute_fantasy_pacing_finding(campaigns, findings);
    add_synchronized_fantasy_timing_finding(campaigns, findings);
    add_fantasy_arc_completion_findings(average_campaign_days(aggregate), campaigns, findings);
}

fn add_fantasy_arc_order_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let political_before_commercial = campaigns
        .iter()
        .filter(|campaign| {
            matches!(
                (
                    campaign.fantasy_arc.first_commercial_standing_day,
                    campaign.fantasy_arc.first_office_campaign_day,
                ),
                (Some(commercial), Some(political)) if political < commercial
            )
        })
        .count();
    if political_before_commercial > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Political ascent precedes commercial standing".to_owned(),
            evidence: format!(
                "{political_before_commercial} of {} campaigns launched an office campaign before establishing both the required reputation and delivery record.",
                campaigns.len()
            ),
        });
    }
}

fn add_fantasy_arc_compression_findings(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let immediate_political_access = campaigns
        .iter()
        .filter(|campaign| {
            matches!(
                (
                    campaign.fantasy_arc.first_institution_support_day,
                    campaign.fantasy_arc.first_office_campaign_day,
                ),
                (Some(support), Some(campaign_day)) if campaign_day <= support.saturating_add(60)
            )
        })
        .count();
    if scaled_ratio_usize(immediate_political_access, campaigns.len(), 100) >= 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Institutional support immediately becomes candidacy".to_owned(),
            evidence: format!(
                "{immediate_political_access} of {} campaigns launched an office campaign within 60 days of first cultivating institutional support, leaving little distinct coalition-building phase.",
                campaigns.len()
            ),
        });
    }

    let immediate_city_power = campaigns
        .iter()
        .filter(|campaign| {
            matches!(
                (
                    campaign.fantasy_arc.first_office_day,
                    campaign.fantasy_arc.first_city_shaping_action_day,
                ),
                (Some(office), Some(city_action)) if city_action <= office.saturating_add(90)
            )
        })
        .count();
    if scaled_ratio_usize(immediate_city_power, campaigns.len(), 100) >= 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Officeholding immediately becomes city-shaping power".to_owned(),
            evidence: format!(
                "{immediate_city_power} of {} campaigns sponsored a law, started a public work, or issued an office directive within 90 days of first taking office, leaving little time for office-specific duties or coalition building.",
                campaigns.len()
            ),
        });
    }
}

fn add_absolute_fantasy_pacing_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let eligible: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.simulated_days >= 720)
        .collect();
    if eligible.is_empty() {
        return;
    }
    let compressed = eligible
        .iter()
        .filter(|campaign| {
            matches!(
                (
                    campaign.fantasy_arc.first_commercial_standing_day,
                    campaign.fantasy_arc.first_institution_support_day,
                    campaign.fantasy_arc.first_office_campaign_day,
                    campaign.fantasy_arc.first_city_shaping_action_day,
                ),
                (Some(standing), Some(support), Some(campaign_day), Some(city_day))
                    if standing <= 420 && support <= 480 && campaign_day <= 600 && city_day <= 900
            )
        })
        .count();
    if scaled_ratio_usize(compressed, eligible.len(), 100) < 50 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "The core fantasy arc is compressed into the opening establishment cycle".to_owned(),
        evidence: format!(
            "{compressed} of {} campaigns established a commercial record within 420 days, cultivated institutional support within 480 days, began an office campaign within 600 days, and exercised city-shaping power within 900 days. Foundation, social ascent, and institutional authority may not be receiving distinct enough phases for a multi-generation campaign.",
            eligible.len()
        ),
    });
}

fn add_synchronized_fantasy_timing_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let mut campaigns_by_seed: BTreeMap<u64, Vec<&GameplayCampaignReport>> = BTreeMap::new();
    for campaign in campaigns {
        campaigns_by_seed
            .entry(campaign.seed)
            .or_default()
            .push(campaign);
    }
    let eligible_seed_cohorts = campaigns_by_seed
        .values()
        .filter(|cohort| cohort.len() >= 4)
        .count();
    let synchronized_seed_cohorts = campaigns_by_seed
        .values()
        .filter(|cohort| cohort.len() >= 4)
        .filter(|cohort| {
            milestone_span(
                cohort
                    .iter()
                    .filter_map(|campaign| campaign.fantasy_arc.first_commercial_standing_day),
            ) <= Some(7)
                && milestone_span(
                    cohort
                        .iter()
                        .filter_map(|campaign| campaign.fantasy_arc.first_institution_support_day),
                ) <= Some(7)
                && milestone_span(
                    cohort
                        .iter()
                        .filter_map(|campaign| campaign.fantasy_arc.first_office_campaign_day),
                ) <= Some(7)
                && milestone_span(
                    cohort
                        .iter()
                        .filter_map(|campaign| campaign.fantasy_arc.first_office_day),
                ) <= Some(7)
                && milestone_span(
                    cohort
                        .iter()
                        .filter_map(|campaign| campaign.fantasy_arc.first_city_shaping_action_day),
                ) <= Some(7)
        })
        .count();
    if eligible_seed_cohorts > 0 && synchronized_seed_cohorts == eligible_seed_cohorts {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Core fantasy timing is highly synchronized".to_owned(),
            evidence: format!(
                "Within every one of {eligible_seed_cohorts} seed cohorts, all personas and backgrounds reached commercial standing, institutional support, office campaigning, officeholding, and city-shaping action within seven days of each other."
            ),
        });
    }
}

fn add_fantasy_arc_completion_findings(
    average_days: u64,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_days >= 1_080 {
        let incomplete = campaigns
            .iter()
            .filter(|campaign| {
                campaign.fantasy_arc.first_commercial_standing_day.is_none()
                    || campaign.fantasy_arc.first_institution_support_day.is_none()
                    || campaign.fantasy_arc.first_office_campaign_day.is_none()
                    || campaign.fantasy_arc.first_office_day.is_none()
            })
            .count();
        if scaled_ratio_usize(incomplete, campaigns.len(), 100) >= 25 {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Warning,
                title: "The early commercial-to-political arc is incomplete".to_owned(),
                evidence: format!(
                    "{incomplete} of {} campaigns did not reach commercial standing, cultivate institutional support, launch an office campaign, and obtain office within the measured horizon.",
                    campaigns.len()
                ),
            });
        }
    }
    if average_days >= 1_080 {
        let established_in_time: Vec<_> = campaigns
            .iter()
            .filter(|campaign| {
                campaign.fantasy_arc.first_office_day.is_some_and(|day| {
                    day <= i64::try_from(average_days)
                        .unwrap_or(i64::MAX)
                        .saturating_sub(OFFICE_POWER_ESTABLISHMENT_DAYS)
                })
            })
            .collect();
        let without_city_shaping = established_in_time
            .iter()
            .filter(|campaign| campaign.fantasy_arc.first_city_shaping_action_day.is_none())
            .count();
        if !established_in_time.is_empty()
            && scaled_ratio_usize(without_city_shaping, established_in_time.len(), 100) >= 25
        {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Warning,
                title: "Institutional power does not become city-shaping action".to_owned(),
                evidence: format!(
                    "{without_city_shaping} of {} campaigns whose office powers had time to establish never sponsored a law, started a public work, or issued an office directive.",
                    established_in_time.len()
                ),
            });
        }
    }
    if average_days >= 7_200 {
        let without_succession = campaigns
            .iter()
            .filter(|campaign| campaign.fantasy_arc.first_succession_day.is_none())
            .count();
        let missing_share = scaled_ratio_usize(without_succession, campaigns.len(), 100);
        if missing_share >= 25 {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Warning,
                title: "The dynastic arc does not reach succession".to_owned(),
                evidence: format!(
                    "{without_succession} of {} generation-length campaigns did not transfer leadership to a successor ({missing_share}%).",
                    campaigns.len(),
                ),
            });
        }
    }
}

fn milestone_span(days: impl Iterator<Item = i64>) -> Option<i64> {
    let mut minimum = None;
    let mut maximum = None;
    for day in days {
        minimum = Some(minimum.map_or(day, |current: i64| current.min(day)));
        maximum = Some(maximum.map_or(day, |current: i64| current.max(day)));
    }
    minimum
        .zip(maximum)
        .map(|(minimum, maximum)| maximum - minimum)
}

fn add_player_labor_agency_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.is_empty() || average_campaign_days(aggregate) < 720 {
        return;
    }
    let player_dispute_campaigns = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_player_disputed_employment > 0)
        .count();
    let ambient_labor_changes = aggregate
        .ambient_domain_changes
        .get(&GameplayDomain::Labor)
        .copied()
        .unwrap_or(0);
    if player_dispute_campaigns == 0 && ambient_labor_changes > 0 {
        findings.push(GameplayFinding {
            severity: if campaigns.len() >= 3 {
                GameplayFindingSeverity::Warning
            } else {
                GameplayFindingSeverity::Info
            },
            title: "Labor conflict remains ambient to the player".to_owned(),
            evidence: format!(
                "Labor changed in {ambient_labor_changes} baseline observations, but none of {} campaigns produced a dispute in a player-owned business. A single campaign is insufficient to distinguish successful dispute avoidance from a systemic exposure gap.",
                campaigns.len()
            ),
        });
    }
}

fn add_information_agency_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let information_changes = aggregate
        .ambient_domain_changes
        .get(&GameplayDomain::Information)
        .copied()
        .unwrap_or(0);
    let player_information_changes = aggregate
        .causal_domain_changes
        .get(&GameplayDomain::Information)
        .copied()
        .unwrap_or(0);
    let commissions = aggregate
        .commands
        .get(&GameplayCommandKind::CommissionInformation)
        .map_or(0, |stats| stats.executed);
    let commission_opportunities = aggregate
        .commands
        .get(&GameplayCommandKind::CommissionInformation)
        .map_or(0, |stats| stats.activation_opportunities);
    let leverage_actions = aggregate
        .commands
        .get(&GameplayCommandKind::LeverageInformation)
        .map_or(0, |stats| stats.executed);
    let leverage_opportunities = aggregate
        .commands
        .get(&GameplayCommandKind::LeverageInformation)
        .map_or(0, |stats| stats.activation_opportunities);
    if commission_opportunities > 0 && commissions == 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Commercial intelligence is not player-directed".to_owned(),
            evidence: format!(
                "The harness observed {commission_opportunities} material intelligence opportunities and {information_changes} baseline information changes, but agents commissioned {commissions} reports and produced {player_information_changes} causally attributed information changes."
            ),
        });
    }
    if leverage_opportunities > 0 && leverage_actions == 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Commissioned intelligence does not become action".to_owned(),
            evidence: format!(
                "Agents commissioned {commissions} reports and observed {leverage_opportunities} actionable leverage opportunities, but never converted one into a contract renegotiation, targeted outreach, or district initiative."
            ),
        });
    }
}

fn add_power_conversion_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    const ECONOMIC_COMMANDS: [GameplayCommandKind; 8] = [
        GameplayCommandKind::AcquireBusiness,
        GameplayCommandKind::InvestInBusiness,
        GameplayCommandKind::SetBusinessPolicy,
        GameplayCommandKind::SecureSupply,
        GameplayCommandKind::SellOutput,
        GameplayCommandKind::BorrowFunds,
        GameplayCommandKind::ExtendCredit,
        GameplayCommandKind::BuyProperty,
    ];
    const INSTITUTIONAL_COMMANDS: [GameplayCommandKind; 6] = [
        GameplayCommandKind::EnactLaw,
        GameplayCommandKind::StartPublicWork,
        GameplayCommandKind::FileLegalCase,
        GameplayCommandKind::SetHouseGovernance,
        GameplayCommandKind::NominateForOffice,
        GameplayCommandKind::ExerciseOfficePower,
    ];
    let economic_to_social = aggregate.interactions.iter().any(|edge| {
        ECONOMIC_COMMANDS.contains(&edge.command)
            && matches!(
                edge.domain,
                GameplayDomain::Relationships | GameplayDomain::Institutions | GameplayDomain::Law
            )
    });
    let institutional_to_material = aggregate.interactions.iter().any(|edge| {
        INSTITUTIONAL_COMMANDS.contains(&edge.command)
            && matches!(
                edge.domain,
                GameplayDomain::Economy
                    | GameplayDomain::Business
                    | GameplayDomain::Market
                    | GameplayDomain::Districts
            )
    });
    if !economic_to_social || !institutional_to_material {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "The power-conversion loop is incomplete".to_owned(),
            evidence: format!(
                "economic_to_social={economic_to_social}; institutional_to_material={institutional_to_material}. The core fantasy requires commercial power to create social or institutional leverage and institutional power to reshape material conditions."
            ),
        });
    }
}

fn add_persona_convergence_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let personas: BTreeSet<_> = campaigns.iter().map(|campaign| campaign.persona).collect();
    if personas.len() < 3 {
        return;
    }
    let top_sets: Vec<BTreeSet<GameplayCommandKind>> = personas
        .iter()
        .map(|persona| {
            let mut totals = BTreeMap::<GameplayCommandKind, u32>::new();
            for campaign in campaigns
                .iter()
                .filter(|campaign| campaign.persona == *persona)
            {
                for (kind, stats) in &campaign.commands {
                    if is_persona_identity_command(*kind) && !is_cross_persona_enabler(*kind) {
                        let total = totals.entry(*kind).or_default();
                        *total = total.saturating_add(stats.executed);
                    }
                }
            }
            let mut ranked: Vec<_> = totals.into_iter().collect();
            ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
            ranked.into_iter().take(3).map(|(kind, _)| kind).collect()
        })
        .collect();
    let mut common = top_sets
        .iter()
        .skip(1)
        .fold(top_sets[0].clone(), |common, next| {
            common.intersection(next).copied().collect()
        });
    if common.contains(&GameplayCommandKind::NominateForOffice)
        && persona_outcomes_diverge(campaigns, |campaign| campaign.end.player_office_checksum)
    {
        common.remove(&GameplayCommandKind::NominateForOffice);
    }
    if persona_outcomes_diverge(campaigns, |campaign| {
        campaign.end.player_family_capability_checksum
    }) {
        common.remove(&GameplayCommandKind::AdoptWard);
        common.remove(&GameplayCommandKind::EducateFamilyMember);
    }
    if common.len() >= 2 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Distinct personas converge on the same action priorities".to_owned(),
            evidence: format!(
                "At least {} of the three most-used substantive command families were shared by every configured persona: {}.",
                common.len(),
                common
                    .iter()
                    .map(|kind| kind.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
}

fn persona_outcomes_diverge<T: Copy + Ord>(
    campaigns: &[GameplayCampaignReport],
    outcome: impl Fn(&GameplayCampaignReport) -> T,
) -> bool {
    let personas: BTreeSet<_> = campaigns.iter().map(|campaign| campaign.persona).collect();
    let outcome_sets: Vec<BTreeSet<T>> = personas
        .iter()
        .map(|persona| {
            campaigns
                .iter()
                .filter(|campaign| campaign.persona == *persona)
                .map(&outcome)
                .collect()
        })
        .collect();
    let mut comparisons = 0_u64;
    let mut overlap_total = 0_u64;
    for (index, left) in outcome_sets.iter().enumerate() {
        for right in outcome_sets.iter().skip(index + 1) {
            let union = left.union(right).count();
            if union == 0 {
                continue;
            }
            let intersection = left.intersection(right).count();
            overlap_total =
                overlap_total.saturating_add(scaled_ratio_usize(intersection, union, 100));
            comparisons = comparisons.saturating_add(1);
        }
    }
    comparisons > 0 && overlap_total / comparisons < 75
}

const fn is_cross_persona_enabler(kind: GameplayCommandKind) -> bool {
    matches!(
        kind,
        GameplayCommandKind::CommissionInformation
            | GameplayCommandKind::CultivateInstitutionSupport
    )
}

const fn is_persona_identity_command(kind: GameplayCommandKind) -> bool {
    matches!(
        kind,
        GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::SellOutput
            | GameplayCommandKind::ExtendCredit
            | GameplayCommandKind::SellProperty
            | GameplayCommandKind::EnactLaw
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::SetHouseGovernance
            | GameplayCommandKind::AdoptWard
            | GameplayCommandKind::EducateFamilyMember
            | GameplayCommandKind::CommissionInformation
            | GameplayCommandKind::CultivateInstitutionSupport
            | GameplayCommandKind::NominateForOffice
            | GameplayCommandKind::WithdrawFromInstitution
    )
}

fn add_civic_convergence_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.len() < 8 || average_campaign_days(aggregate) < 720 {
        return;
    }
    let fixed_dimensions = [
        campaigns
            .iter()
            .map(|campaign| campaign.end.active_law_checksum)
            .collect::<BTreeSet<_>>()
            .len(),
        campaigns
            .iter()
            .map(|campaign| campaign.end.player_completed_public_work_checksum)
            .collect::<BTreeSet<_>>()
            .len(),
        campaigns
            .iter()
            .map(|campaign| campaign.end.player_office_checksum)
            .collect::<BTreeSet<_>>()
            .len(),
        campaigns
            .iter()
            .map(|campaign| campaign.end.house_governance as u8)
            .collect::<BTreeSet<_>>()
            .len(),
    ]
    .into_iter()
    .filter(|unique_values| *unique_values == 1)
    .count();
    if fixed_dimensions >= 3 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Civic progression converges despite different strategies".to_owned(),
            evidence: format!(
                "{fixed_dimensions} of four identity-sensitive civic outcome dimensions had no variation across {} campaigns: active law mix, sponsored works, offices held, and house governance.",
                campaigns.len()
            ),
        });
    }
}

fn add_power_exposure_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 3_600 {
        return;
    }
    let established: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_offices_held > 0)
        .collect();
    if established.len() < 4 {
        return;
    }
    let sheltered = established
        .iter()
        .filter(|campaign| {
            let unmet_duties = campaign
                .end
                .player_unmet_office_duties
                .saturating_sub(campaign.start.player_unmet_office_duties);
            campaign.maximum_player_disputed_employment == 0
                && campaign.end.player_contract_failures == 0
                && campaign.end.distressed_businesses == 0
                && campaign.end.insolvent_businesses == 0
                && campaign.end.player_treasury.copper()
                    >= campaign.start.player_treasury.copper().saturating_div(2)
                && campaign.maximum_contract_relationship_pressure_basis_points < 1_500
                && unmet_duties == 0
        })
        .count();
    if scaled_ratio_usize(sheltered, established.len(), 100) < 50 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Established dynasties often avoid measured power exposure".to_owned(),
        evidence: format!(
            "{sheltered} of {} officeholding campaigns reached the endpoint without a player labor dispute, contract failure, distressed business, insolvent business, major treasury drawdown, at least 1,500 basis points of relationship-driven contract pressure, or unmet office duty. Routine civic payments no longer count as meaningful exposure by themselves; political backlash does count once it materially worsens commercial bargaining. The design calls for greater power to create consequential obligations and vulnerability, not only additional tools.",
            established.len()
        ),
    });
}

fn add_office_duty_failure_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 1_080 || campaigns.is_empty() {
        return;
    }
    let chronic_failures: Vec<_> = campaigns
        .iter()
        .filter_map(|campaign| {
            let failures = campaign
                .end
                .player_unmet_office_duties
                .saturating_sub(campaign.start.player_unmet_office_duties);
            (failures >= 12).then_some((campaign, failures))
        })
        .collect();
    if chronic_failures.is_empty() {
        return;
    }
    let (worst, failures) = chronic_failures
        .into_iter()
        .max_by_key(|(_, failures)| *failures)
        .expect("non-empty chronic office-duty failures must have a maximum");
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Office obligations repeatedly exceed dynasty liquidity".to_owned(),
        evidence: format!(
            "At least one campaign accumulated twelve or more unmet monthly office duties; the worst was {failures} for seed {}, {} {:?}. Political service is creating a recurring liquidity trap rather than a manageable strategic liability.",
            worst.seed,
            worst.persona.label(),
            worst.background,
        ),
    });
}

fn add_dynastic_continuity_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 7_200 || campaigns.is_empty() {
        return;
    }
    let successions = campaigns
        .iter()
        .filter(|campaign| campaign.end.generation > campaign.start.generation)
        .count();
    if successions == 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Long campaigns do not exercise dynastic continuity".to_owned(),
            evidence: format!(
                "None of {} campaigns advanced beyond their starting generation over {} days per campaign.",
                campaigns.len(),
                average_campaign_days(aggregate)
            ),
        });
        return;
    }
    if aggregate
        .commands
        .get(&GameplayCommandKind::DesignateHeir)
        .is_none_or(|stats| stats.executed == 0)
    {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Succession occurs without player preparation".to_owned(),
            evidence: format!(
                "{successions} of {} long campaigns reached a new generation, but none designated an heir. The continuity system is functioning as simulation, not yet as a player-authored dynasty strategy.",
                campaigns.len()
            ),
        });
    }
}

fn add_variance_finding(campaigns: &[GameplayCampaignReport], findings: &mut Vec<GameplayFinding>) {
    let Some(minimum) = campaigns
        .iter()
        .min_by_key(|campaign| campaign.scores.overall)
    else {
        return;
    };
    let Some(maximum) = campaigns
        .iter()
        .max_by_key(|campaign| campaign.scores.overall)
    else {
        return;
    };
    let spread = maximum
        .scores
        .overall
        .saturating_sub(minimum.scores.overall);
    if spread >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Large experience variance between starts".to_owned(),
            evidence: format!(
                "Scores ranged from {}/100 ({:?}, {:?}, seed {}) to {}/100 ({:?}, {:?}, seed {}).",
                minimum.scores.overall,
                minimum.persona,
                minimum.background,
                minimum.seed,
                maximum.scores.overall,
                maximum.persona,
                maximum.background,
                maximum.seed
            ),
        });
    }
}

/// Renders a compact, human-readable gameplay report suitable for CI logs and design review.
#[must_use]
pub fn render_gameplay_report(report: &GameplayHarnessReport) -> String {
    let mut output = String::new();
    render_report_header(report, &mut output);
    render_phase_summary(report, &mut output);
    render_health_summary(report, &mut output);
    render_command_table(report, &mut output);
    render_domain_table(report, &mut output);
    render_interactions(report, &mut output);
    render_rejections(report, &mut output);
    render_findings(report, &mut output);
    render_limitations(report, &mut output);
    render_fantasy_arcs(report, &mut output);
    render_campaign_summaries(report, &mut output);
    render_trace_samples(report, &mut output);
    output
}

fn render_phase_summary(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Phase quality");
    for phase in [
        GameplayPhase::Foundation,
        GameplayPhase::Establishment,
        GameplayPhase::InstitutionalAscent,
        GameplayPhase::DynasticGovernance,
        GameplayPhase::SuccessionLegacy,
    ] {
        let stats = report
            .aggregate
            .phase_stats
            .get(&phase)
            .copied()
            .unwrap_or_default();
        let action_share = scaled_ratio_u64(
            u64::from(stats.substantive_actions),
            u64::from(stats.decision_cycles),
            100,
        );
        let campaign_admin_share = scaled_ratio_u64(
            u64::from(stats.institutional_campaign_actions),
            u64::from(stats.substantive_actions),
            100,
        );
        let multi_family_share = scaled_ratio_u64(
            u64::from(stats.cycles_with_multiple_viable_command_kinds),
            u64::from(stats.decision_cycles),
            100,
        );
        let close_choice_share = scaled_ratio_u64(
            u64::from(stats.cycles_with_close_viable_command_kinds),
            u64::from(stats.cycles_with_multiple_viable_command_kinds),
            100,
        );
        let distinct_choice_share = scaled_ratio_u64(
            u64::from(stats.cycles_with_distinct_immediate_consequences),
            u64::from(stats.cycles_with_multiple_viable_command_kinds),
            100,
        );
        let projected_choice_share = scaled_ratio_u64(
            u64::from(stats.cycles_with_distinct_projected_consequences),
            u64::from(stats.cycles_with_multiple_viable_command_kinds),
            100,
        );
        let opportunity_cycles = stats.decision_cycles.saturating_sub(stats.quiet_cycles);
        let average_choices_tenths = scaled_ratio_u64(
            u64::from(stats.total_viable_choices),
            u64::from(opportunity_cycles),
            10,
        );
        let average_families_tenths = scaled_ratio_u64(
            u64::from(stats.total_viable_command_kinds),
            u64::from(opportunity_cycles),
            10,
        );
        let _ = writeln!(
            output,
            "  {:<22} cycles {:>5} | action {:>3}% | campaign admin {:>3}% | multi {:>3}% | close {:>3}% | distinct now {:>3}% / next {:>3}% | choices {}.{} / families {}.{} | quiet {:>5} (ambient {:>5}, longest {:>2}) | blocked {:>5}",
            phase.label(),
            stats.decision_cycles,
            action_share,
            campaign_admin_share,
            multi_family_share,
            close_choice_share,
            distinct_choice_share,
            projected_choice_share,
            average_choices_tenths / 10,
            average_choices_tenths % 10,
            average_families_tenths / 10,
            average_families_tenths % 10,
            stats.quiet_cycles,
            stats.quiet_cycles_with_ambient_change,
            stats.longest_quiet_streak_cycles,
            stats.blocked_cycles
        );
    }
    let _ = writeln!(output);
}

fn render_limitations(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Harness limits");
    for limitation in &report.limitations {
        let _ = writeln!(output, "  - {limitation}");
    }
    let _ = writeln!(output);
}

fn render_fantasy_arcs(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Core fantasy milestones");
    for campaign in &report.campaigns {
        let arc = campaign.fantasy_arc;
        let _ = writeln!(
            output,
            "  seed {:>3} {:<12} {:?}: reputation {} | commercial record {} | institutional support {} | campaign {} | office {} | city-shaping {} | labor conflict {} | heir designated {} | succession {}",
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
            milestone_day(arc.first_reputation_standing_day),
            milestone_day(arc.first_commercial_standing_day),
            milestone_day(arc.first_institution_support_day),
            milestone_day(arc.first_office_campaign_day),
            milestone_day(arc.first_office_day),
            milestone_day(arc.first_city_shaping_action_day),
            milestone_day(arc.first_player_labor_dispute_day),
            milestone_day(arc.first_heir_designation_day),
            milestone_day(arc.first_succession_day),
        );
    }
    let _ = writeln!(output);
}

fn milestone_day(day: Option<i64>) -> String {
    day.map_or_else(|| "not reached".to_owned(), |day| format!("day {day}"))
}

#[derive(Clone, Copy, Debug)]
struct HealthSummary {
    minimum_food: (u16, u16),
    minimum_district_food: (u16, u16),
    operating_businesses: (u16, u16),
    peak_offices: (u16, u16),
    peak_unread: (u16, u16),
    peak_private_credit_distress: (u16, u16),
    peak_civic_credit_distress: (u16, u16),
    available_offices: u16,
    represented_institutions: (u16, u16),
    fulfilled_contracts: u64,
    breached_contracts: u64,
    repaid_loans: u64,
    defaulted_loans: u64,
    repaid_civic_debts: u64,
    defaulted_civic_debts: u64,
    completed_works: u64,
    suspended_works: u64,
}

impl HealthSummary {
    fn new(first: &GameplayCampaignReport) -> Self {
        Self {
            minimum_food: (
                first.minimum_food_satisfaction,
                first.minimum_food_satisfaction,
            ),
            minimum_district_food: (
                first.minimum_district_food_satisfaction,
                first.minimum_district_food_satisfaction,
            ),
            operating_businesses: (
                first.minimum_operating_businesses,
                first.minimum_operating_businesses,
            ),
            peak_offices: (first.maximum_offices_held, first.maximum_offices_held),
            peak_unread: (
                first.maximum_unread_notifications,
                first.maximum_unread_notifications,
            ),
            peak_private_credit_distress: (0, 0),
            peak_civic_credit_distress: (0, 0),
            available_offices: first.end.available_offices,
            represented_institutions: (
                first.end.player_institutions_represented,
                first.end.player_institutions_represented,
            ),
            fulfilled_contracts: 0,
            breached_contracts: 0,
            repaid_loans: 0,
            defaulted_loans: 0,
            repaid_civic_debts: 0,
            defaulted_civic_debts: 0,
            completed_works: 0,
            suspended_works: 0,
        }
    }

    fn observe(&mut self, campaign: &GameplayCampaignReport) {
        self.minimum_food.0 = self.minimum_food.0.min(campaign.minimum_food_satisfaction);
        self.minimum_food.1 = self.minimum_food.1.max(campaign.minimum_food_satisfaction);
        self.minimum_district_food.0 = self
            .minimum_district_food
            .0
            .min(campaign.minimum_district_food_satisfaction);
        self.minimum_district_food.1 = self
            .minimum_district_food
            .1
            .max(campaign.minimum_district_food_satisfaction);
        self.operating_businesses.0 = self
            .operating_businesses
            .0
            .min(campaign.minimum_operating_businesses);
        self.operating_businesses.1 = self
            .operating_businesses
            .1
            .max(campaign.minimum_operating_businesses);
        self.peak_offices.0 = self.peak_offices.0.min(campaign.maximum_offices_held);
        self.peak_offices.1 = self.peak_offices.1.max(campaign.maximum_offices_held);
        self.peak_unread.0 = self
            .peak_unread
            .0
            .min(campaign.maximum_unread_notifications);
        self.peak_unread.1 = self
            .peak_unread
            .1
            .max(campaign.maximum_unread_notifications);
        self.peak_private_credit_distress.0 = self
            .peak_private_credit_distress
            .0
            .max(campaign.maximum_delinquent_loans);
        self.peak_private_credit_distress.1 = self
            .peak_private_credit_distress
            .1
            .max(campaign.maximum_defaulted_loans);
        self.peak_civic_credit_distress.0 = self
            .peak_civic_credit_distress
            .0
            .max(campaign.maximum_delinquent_civic_debts);
        self.peak_civic_credit_distress.1 = self
            .peak_civic_credit_distress
            .1
            .max(campaign.maximum_defaulted_civic_debts);
        self.available_offices = self.available_offices.max(campaign.end.available_offices);
        self.represented_institutions.0 = self
            .represented_institutions
            .0
            .min(campaign.end.player_institutions_represented);
        self.represented_institutions.1 = self
            .represented_institutions
            .1
            .max(campaign.end.player_institutions_represented);
        self.fulfilled_contracts = self
            .fulfilled_contracts
            .saturating_add(u64::from(campaign.end.player_fulfilled_contracts));
        self.breached_contracts = self
            .breached_contracts
            .saturating_add(u64::from(campaign.end.player_breached_contracts));
        self.repaid_loans = self
            .repaid_loans
            .saturating_add(u64::from(campaign.end.repaid_loans));
        self.defaulted_loans = self
            .defaulted_loans
            .saturating_add(u64::from(campaign.end.defaulted_loans));
        self.repaid_civic_debts = self
            .repaid_civic_debts
            .saturating_add(u64::from(campaign.end.repaid_civic_debts));
        self.defaulted_civic_debts = self
            .defaulted_civic_debts
            .saturating_add(u64::from(campaign.end.defaulted_civic_debts));
        self.completed_works = self
            .completed_works
            .saturating_add(u64::from(campaign.end.completed_public_works));
        self.suspended_works = self
            .suspended_works
            .saturating_add(u64::from(campaign.end.suspended_public_works));
    }
}

fn summarize_health(campaigns: &[GameplayCampaignReport]) -> Option<HealthSummary> {
    let mut summary = HealthSummary::new(campaigns.first()?);
    for campaign in campaigns {
        summary.observe(campaign);
    }
    Some(summary)
}

fn render_health_summary(report: &GameplayHarnessReport, output: &mut String) {
    let Some(summary) = summarize_health(&report.campaigns) else {
        return;
    };
    let _ = writeln!(output, "Experience health");
    let _ = writeln!(
        output,
        "  trajectory ranges: city food {:.2}-{:.2}% | worst district food {:.2}-{:.2}% | operating businesses {}-{} | peak offices {}-{}/{} | represented institutions {}-{}/{} | peak unread {}-{}",
        f64::from(summary.minimum_food.0) / 100.0,
        f64::from(summary.minimum_food.1) / 100.0,
        f64::from(summary.minimum_district_food.0) / 100.0,
        f64::from(summary.minimum_district_food.1) / 100.0,
        summary.operating_businesses.0,
        summary.operating_businesses.1,
        summary.peak_offices.0,
        summary.peak_offices.1,
        summary.available_offices,
        summary.represented_institutions.0,
        summary.represented_institutions.1,
        summary.available_offices,
        summary.peak_unread.0,
        summary.peak_unread.1
    );
    let _ = writeln!(
        output,
        "  outcomes: player contracts {} fulfilled / {} breached | private loans {} repaid / {} defaulted | civic debts {} repaid / {} defaulted | public works {} completed / {} suspended\n",
        summary.fulfilled_contracts,
        summary.breached_contracts,
        summary.repaid_loans,
        summary.defaulted_loans,
        summary.repaid_civic_debts,
        summary.defaulted_civic_debts,
        summary.completed_works,
        summary.suspended_works,
    );
    let _ = writeln!(
        output,
        "  peak credit distress in one campaign: private {} delinquent / {} defaulted | civic {} delinquent / {} defaulted\n",
        summary.peak_private_credit_distress.0,
        summary.peak_private_credit_distress.1,
        summary.peak_civic_credit_distress.0,
        summary.peak_civic_credit_distress.1,
    );
}

fn render_report_header(report: &GameplayHarnessReport, output: &mut String) {
    let aggregate = &report.aggregate;
    let _ = writeln!(output, "Civic Dynasty gameplay harness");
    let _ = writeln!(
        output,
        "{} campaigns | {} simulated days | {} substantive actions ({} total) | {} candidate probes",
        aggregate.campaigns,
        aggregate.simulated_days,
        aggregate.substantive_actions,
        aggregate.successful_actions,
        aggregate.candidate_probes
    );
    let _ = writeln!(
        output,
        "scores: overall {:>3} | actionability {:>3} | variety {:>3} | interconnection {:>3} | feedback {:>3} | resilience {:>3}",
        aggregate.scores.overall,
        aggregate.scores.actionability,
        aggregate.scores.variety,
        aggregate.scores.interconnection,
        aggregate.scores.feedback,
        aggregate.scores.resilience
    );
    let _ = writeln!(
        output,
        "coverage: {}/{} command kinds | causal domains {}/{} | ambient domains {}/{} | {} command-domain edges | {} quiet ({} with ambient change) / {} blocked cycles",
        aggregate.command_coverage,
        ALL_COMMAND_KINDS.len(),
        aggregate.causal_domain_coverage,
        ALL_DOMAINS.len(),
        aggregate.ambient_domain_coverage,
        ALL_DOMAINS.len(),
        aggregate.interactions.len(),
        aggregate.quiet_cycles,
        aggregate.quiet_cycles_with_ambient_change,
        aggregate.blocked_cycles
    );
    let opportunity_cycles = aggregate
        .decision_cycles
        .saturating_sub(aggregate.quiet_cycles);
    let average_families_tenths = aggregate
        .viable_command_kinds
        .saturating_mul(10)
        .checked_div(opportunity_cycles)
        .unwrap_or(0);
    let average_choices_tenths = aggregate
        .viable_choices
        .saturating_mul(10)
        .checked_div(opportunity_cycles)
        .unwrap_or(0);
    let average_families = format_tenths(average_families_tenths);
    let average_choices = format_tenths(average_choices_tenths);
    let _ = writeln!(
        output,
        "choice quality: {average_choices} viable choices / {average_families} command families per actionable cycle | {} multi-family | {} close-ranked | {} distinct immediate profiles | {} distinct projected profiles\n",
        aggregate.cycles_with_multiple_viable_command_kinds,
        aggregate.cycles_with_close_viable_command_kinds,
        aggregate.cycles_with_distinct_immediate_consequences,
        aggregate.cycles_with_distinct_projected_consequences
    );
}

fn render_command_table(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Command coverage");
    let _ = writeln!(
        output,
        "  {:<20} {:>8} {:>7} {:>9} {:>8} {:>8} {:>8} {:>8} {:>10} {:>8} {:>8}",
        "command",
        "triggers",
        "offered",
        "generated",
        "probed",
        "viable",
        "used",
        "feedback",
        "persistent",
        "delayed",
        "domains"
    );
    for kind in ALL_COMMAND_KINDS {
        let stats = report
            .aggregate
            .commands
            .get(&kind)
            .expect("every command kind must have aggregate statistics");
        let _ = writeln!(
            output,
            "  {:<20} {:>8} {:>7} {:>9} {:>8} {:>8} {:>8} {:>8} {:>10} {:>8} {:>8}",
            kind.label(),
            stats.activation_opportunities,
            stats.offered_cycles,
            stats.generated,
            stats.considered,
            stats.viable,
            stats.executed,
            stats.actions_with_feedback,
            stats.actions_with_persistent_consequences,
            stats.actions_with_delayed_consequences,
            stats.changed_domains.len()
        );
    }
    let _ = writeln!(output);
}

fn render_domain_table(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Observed domain transitions (causal / ambient)");
    for row in ALL_DOMAINS.chunks(3) {
        let mut line = String::new();
        for domain in row {
            let causal = report
                .aggregate
                .causal_domain_changes
                .get(domain)
                .copied()
                .unwrap_or(0);
            let ambient = report
                .aggregate
                .ambient_domain_changes
                .get(domain)
                .copied()
                .unwrap_or(0);
            let _ = write!(
                line,
                "  {:<14} {:>5}/{:<5}",
                domain.label(),
                causal,
                ambient
            );
        }
        let _ = writeln!(output, "{line}");
    }
    let _ = writeln!(output);
}

fn render_interactions(report: &GameplayHarnessReport, output: &mut String) {
    let mut edges = report.aggregate.interactions.clone();
    edges.sort_by(|left, right| {
        right
            .observations
            .cmp(&left.observations)
            .then_with(|| left.command.cmp(&right.command))
            .then_with(|| left.domain.cmp(&right.domain))
    });
    let _ = writeln!(output, "Strongest observed command consequences");
    for edge in edges.into_iter().take(10) {
        let _ = writeln!(
            output,
            "  {:<20} -> {:<14} {:>6} observations",
            edge.command.label(),
            edge.domain.label(),
            edge.observations
        );
    }
    let _ = writeln!(output);
}

fn render_rejections(report: &GameplayHarnessReport, output: &mut String) {
    if report.aggregate.rejection_reasons.is_empty() {
        return;
    }
    let mut reasons: Vec<_> = report.aggregate.rejection_reasons.iter().collect();
    reasons.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
    let _ = writeln!(output, "Most common blocked choices");
    for (reason, count) in reasons.into_iter().take(8) {
        let _ = writeln!(output, "  {count:>6}  {reason}");
    }
    let _ = writeln!(output);
}

fn render_findings(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Findings");
    for finding in &report.findings {
        let _ = writeln!(
            output,
            "  [{:?}] {}: {}",
            finding.severity, finding.title, finding.evidence
        );
    }
    let _ = writeln!(output);
}

fn render_campaign_summaries(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Campaign summaries");
    for campaign in &report.campaigns {
        let actions: u32 = campaign.commands.values().map(|stats| stats.executed).sum();
        let _ = writeln!(
            output,
            "  seed {:>3} | {:<12} | {:<11?} | score {:>3} | actions {:>3} | choices {:>4} | treasury {} | businesses A:{} D:{} I:{}",
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
            campaign.scores.overall,
            actions,
            campaign.total_viable_choices,
            campaign.end.player_treasury,
            campaign.end.active_businesses,
            campaign.end.distressed_businesses,
            campaign.end.insolvent_businesses
        );
    }
    let _ = writeln!(output);
}

fn render_trace_samples(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Representative decisions");
    for campaign in &report.campaigns {
        let Some(step) = campaign
            .trace
            .iter()
            .max_by_key(|step| step.consequence_breadth())
        else {
            continue;
        };
        let command = step
            .selected_command
            .map_or("none", GameplayCommandKind::label);
        let _ = writeln!(
            output,
            "  seed {} {:<12} {:?} day {:>4}: {:<18} viable [{}] gap {:?} profiles immediate:{} projected:{} immediate [{}] delayed [{}] ambient [{}] signals [{}]",
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
            step.day,
            command,
            step.viable_options
                .iter()
                .take(3)
                .map(|candidate| format!(
                    "{}:{}:now={}{}:next={}{}",
                    candidate.command.label(),
                    candidate.score,
                    domain_labels(&candidate.immediate_domains),
                    history_suffix(candidate.immediate_history_change),
                    domain_labels(&candidate.projected_domains),
                    history_suffix(candidate.projected_history_change)
                ))
                .collect::<Vec<_>>()
                .join(","),
            step.close_choice_score_gap,
            step.distinct_immediate_choice_profiles,
            step.distinct_projected_choice_profiles,
            domain_labels(&step.immediate_domains),
            domain_labels(&step.delayed_domains),
            domain_labels(&step.ambient_domains),
            trace_signal_labels(&step.signals)
        );
    }
}

fn history_suffix(changed: bool) -> &'static str {
    if changed { "+history" } else { "" }
}

fn trace_signal_labels(signals: &BTreeSet<GameplayTraceSignal>) -> String {
    if signals.is_empty() {
        return "none".to_owned();
    }
    signals
        .iter()
        .map(|signal| match signal {
            GameplayTraceSignal::ImmediateWorldFeedback => "immediate-feedback",
            GameplayTraceSignal::DelayedWorldFeedback => "delayed-feedback",
            GameplayTraceSignal::AmbientWorldFeedback => "ambient-feedback",
            GameplayTraceSignal::PersistentHistoryChange => "persistent-history",
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn domain_labels(domains: &BTreeSet<GameplayDomain>) -> String {
    if domains.is_empty() {
        return "none".to_owned();
    }
    domains
        .iter()
        .map(|domain| domain.label())
        .collect::<Vec<_>>()
        .join(",")
}

struct StableChecksumWriter(u64);

impl StableChecksumWriter {
    const OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
    const PRIME: u64 = 1_099_511_628_211;
}

impl std::io::Write for StableChecksumWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for byte in bytes {
            self.0 = self.0.wrapping_mul(Self::PRIME) ^ u64::from(*byte);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn stable_serialized_checksum<T: Serialize + ?Sized>(value: &T) -> u64 {
    let mut writer = StableChecksumWriter(StableChecksumWriter::OFFSET_BASIS);
    serde_json::to_writer(&mut writer, value)
        .expect("gameplay observation state must serialize into its checksum");
    writer.0
}

fn persistent_history_changed(
    before: &GameplaySnapshot,
    after_command: &GameplaySnapshot,
    after_time: &GameplaySnapshot,
    baseline_after_time: &GameplaySnapshot,
) -> bool {
    before.audit_state_checksum != after_command.audit_state_checksum
        && baseline_after_time.audit_state_checksum != after_time.audit_state_checksum
}

fn dynasty_state_checksum(state: &AppState) -> u64 {
    let observations: Vec<_> = state
        .dynasties
        .values()
        .map(|dynasty| {
            (
                dynasty.id(),
                dynasty.head_id(),
                dynasty.heir_id(),
                (
                    dynasty.runtime.phase,
                    dynasty.runtime.generation,
                    dynasty.runtime.succession_risk_basis_points,
                ),
                (
                    dynasty.resources.unmet_office_duties,
                    dynasty.resources.legitimacy_basis_points,
                    dynasty.resources.administrative_capacity,
                    dynasty.resources.administrative_load,
                    dynasty.resources.reputation_quality_basis_points,
                    dynasty.resources.reputation_reliability_basis_points,
                ),
            )
        })
        .collect();
    stable_serialized_checksum(&observations)
}

fn compare_economy_and_business(
    earlier: &GameplaySnapshot,
    later: &GameplaySnapshot,
    domains: &mut BTreeSet<GameplayDomain>,
) {
    if earlier.player_treasury != later.player_treasury
        || earlier.player_civic_contributions != later.player_civic_contributions
        || earlier.player_business_cash != later.player_business_cash
        || earlier.household_state_checksum != later.household_state_checksum
    {
        domains.insert(GameplayDomain::Economy);
    }
    if earlier.active_businesses != later.active_businesses
        || earlier.distressed_businesses != later.distressed_businesses
        || earlier.insolvent_businesses != later.insolvent_businesses
        || earlier.average_business_condition != later.average_business_condition
        || earlier.average_business_quality != later.average_business_quality
        || earlier.business_policy_checksum != later.business_policy_checksum
        || earlier.business_state_checksum != later.business_state_checksum
    {
        domains.insert(GameplayDomain::Business);
    }
    if earlier.market_price_total != later.market_price_total
        || earlier.market_stock_total != later.market_stock_total
        || earlier.market_state_checksum != later.market_state_checksum
        || earlier.external_route_state_checksum != later.external_route_state_checksum
    {
        domains.insert(GameplayDomain::Market);
    }
}

fn compare_contracts_and_finance(
    earlier: &GameplaySnapshot,
    later: &GameplaySnapshot,
    domains: &mut BTreeSet<GameplayDomain>,
) {
    if earlier.active_contracts != later.active_contracts
        || earlier.fulfilled_contracts != later.fulfilled_contracts
        || earlier.breached_contracts != later.breached_contracts
        || earlier.contract_failures != later.contract_failures
        || earlier.player_active_contracts != later.player_active_contracts
        || earlier.player_fulfilled_contracts != later.player_fulfilled_contracts
        || earlier.player_breached_contracts != later.player_breached_contracts
        || earlier.player_contract_failures != later.player_contract_failures
        || earlier.player_contract_deliveries != later.player_contract_deliveries
        || earlier.contract_state_checksum != later.contract_state_checksum
    {
        domains.insert(GameplayDomain::Contracts);
    }
    if earlier.current_loans != later.current_loans
        || earlier.delinquent_loans != later.delinquent_loans
        || earlier.restructured_loans != later.restructured_loans
        || earlier.defaulted_loans != later.defaulted_loans
        || earlier.repaid_loans != later.repaid_loans
        || earlier.total_loan_balance != later.total_loan_balance
        || earlier.current_civic_debts != later.current_civic_debts
        || earlier.delinquent_civic_debts != later.delinquent_civic_debts
        || earlier.defaulted_civic_debts != later.defaulted_civic_debts
        || earlier.repaid_civic_debts != later.repaid_civic_debts
        || earlier.total_civic_debt_balance != later.total_civic_debt_balance
        || earlier.loan_state_checksum != later.loan_state_checksum
        || earlier.civic_debt_state_checksum != later.civic_debt_state_checksum
    {
        domains.insert(GameplayDomain::Loans);
    }
    if earlier.player_properties != later.player_properties
        || earlier.player_pledged_properties != later.player_pledged_properties
        || earlier.player_collateral_balance != later.player_collateral_balance
        || earlier.occupied_properties != later.occupied_properties
        || earlier.property_state_checksum != later.property_state_checksum
    {
        domains.insert(GameplayDomain::Property);
    }
    if earlier.active_employment != later.active_employment
        || earlier.disputed_employment != later.disputed_employment
        || earlier.player_active_employment != later.player_active_employment
        || earlier.player_disputed_employment != later.player_disputed_employment
        || earlier.average_labor_loyalty != later.average_labor_loyalty
        || earlier.employment_state_checksum != later.employment_state_checksum
    {
        domains.insert(GameplayDomain::Labor);
    }
    if earlier.average_relationship_trust != later.average_relationship_trust
        || earlier.average_relationship_respect != later.average_relationship_respect
        || earlier.average_relationship_fear != later.average_relationship_fear
        || earlier.average_relationship_resentment != later.average_relationship_resentment
        || earlier.relationship_obligation_total != later.relationship_obligation_total
        || earlier.relationship_memory_count != later.relationship_memory_count
        || earlier.relationship_state_checksum != later.relationship_state_checksum
    {
        domains.insert(GameplayDomain::Relationships);
    }
}

fn compare_dynasty_and_civic(
    earlier: &GameplaySnapshot,
    later: &GameplaySnapshot,
    domains: &mut BTreeSet<GameplayDomain>,
) {
    if earlier.legitimacy != later.legitimacy
        || earlier.quality_reputation != later.quality_reputation
        || earlier.reliability_reputation != later.reliability_reputation
        || earlier.player_unmet_office_duties != later.player_unmet_office_duties
        || earlier.generation != later.generation
        || earlier.achieved_ai_objectives != later.achieved_ai_objectives
        || earlier.dynasty_state_checksum != later.dynasty_state_checksum
        || earlier.ai_objective_state_checksum != later.ai_objective_state_checksum
    {
        domains.insert(GameplayDomain::Dynasty);
    }
    if earlier.family_unity != later.family_unity
        || earlier.family_charter_version != later.family_charter_version
        || earlier.house_governance != later.house_governance
        || earlier.active_wards != later.active_wards
        || earlier.player_family_capability_checksum != later.player_family_capability_checksum
        || earlier.character_state_checksum != later.character_state_checksum
        || earlier.family_state_checksum != later.family_state_checksum
    {
        domains.insert(GameplayDomain::Family);
    }
    if earlier.offices_held != later.offices_held
        || earlier.eligible_officeholders != later.eligible_officeholders
        || earlier.player_office_checksum != later.player_office_checksum
        || earlier.institution_memberships != later.institution_memberships
        || earlier.player_institutions_represented != later.player_institutions_represented
        || earlier.institution_budget_total != later.institution_budget_total
        || earlier.player_civic_contributions != later.player_civic_contributions
        || earlier.player_unmet_office_duties != later.player_unmet_office_duties
        || earlier.institution_state_checksum != later.institution_state_checksum
    {
        domains.insert(GameplayDomain::Institutions);
    }
    if earlier.active_laws != later.active_laws
        || earlier.law_value_checksum != later.law_value_checksum
        || earlier.active_law_checksum != later.active_law_checksum
        || earlier.law_state_checksum != later.law_state_checksum
    {
        domains.insert(GameplayDomain::Law);
    }
    if earlier.average_food_satisfaction != later.average_food_satisfaction
        || earlier.minimum_district_food_satisfaction != later.minimum_district_food_satisfaction
        || earlier.average_district_unrest != later.average_district_unrest
        || earlier.public_work_progress_total != later.public_work_progress_total
        || earlier.building_public_works != later.building_public_works
        || earlier.completed_public_works != later.completed_public_works
        || earlier.suspended_public_works != later.suspended_public_works
        || earlier.player_completed_public_work_checksum
            != later.player_completed_public_work_checksum
        || earlier.public_work_state_checksum != later.public_work_state_checksum
        || earlier.district_state_checksum != later.district_state_checksum
    {
        domains.insert(GameplayDomain::Districts);
    }
}

fn compare_world_and_information(
    earlier: &GameplaySnapshot,
    later: &GameplaySnapshot,
    domains: &mut BTreeSet<GameplayDomain>,
) {
    if earlier.open_legal_cases != later.open_legal_cases
        || earlier.decided_legal_cases != later.decided_legal_cases
        || earlier.legal_case_state_checksum != later.legal_case_state_checksum
    {
        domains.insert(GameplayDomain::Legal);
    }
    if earlier.active_crises != later.active_crises
        || earlier.escalated_crises != later.escalated_crises
        || earlier.resolved_crises != later.resolved_crises
        || earlier.crisis_severity_total != later.crisis_severity_total
        || earlier.crisis_state_checksum != later.crisis_state_checksum
    {
        domains.insert(GameplayDomain::Crises);
    }
    if earlier.information_reports != later.information_reports
        || earlier.information_report_checksum != later.information_report_checksum
        || earlier.information_state_checksum != later.information_state_checksum
    {
        domains.insert(GameplayDomain::Information);
    }
    if earlier.unread_notifications != later.unread_notifications
        || earlier.outbox_messages != later.outbox_messages
        || earlier.chronicle_entries != later.chronicle_entries
        || earlier.outbox_state_checksum != later.outbox_state_checksum
        || earlier.chronicle_state_checksum != later.chronicle_state_checksum
    {
        domains.insert(GameplayDomain::Feedback);
    }
}

fn initialized_command_stats() -> BTreeMap<GameplayCommandKind, GameplayCommandStats> {
    ALL_COMMAND_KINDS
        .into_iter()
        .map(|kind| (kind, GameplayCommandStats::default()))
        .collect()
}

fn initialized_phase_stats() -> BTreeMap<GameplayPhase, GameplayPhaseStats> {
    [
        GameplayPhase::Foundation,
        GameplayPhase::Establishment,
        GameplayPhase::InstitutionalAscent,
        GameplayPhase::DynasticGovernance,
        GameplayPhase::SuccessionLegacy,
    ]
    .into_iter()
    .map(|phase| (phase, GameplayPhaseStats::default()))
    .collect()
}

fn initialized_phase_counts() -> BTreeMap<GameplayPhase, u32> {
    initialized_phase_stats()
        .into_keys()
        .map(|phase| (phase, 0))
        .collect()
}

fn initialized_domain_counts() -> BTreeMap<GameplayDomain, u32> {
    ALL_DOMAINS.into_iter().map(|domain| (domain, 0)).collect()
}

fn interaction_vec(
    interactions: &BTreeMap<(GameplayCommandKind, GameplayDomain), u32>,
) -> Vec<GameplayInteractionEdge> {
    interactions
        .iter()
        .map(
            |((command, domain), observations)| GameplayInteractionEdge {
                command: *command,
                domain: *domain,
                observations: *observations,
            },
        )
        .collect()
}

fn select_trace(mut trace: Vec<GameplayTraceStep>, limit: usize) -> Vec<GameplayTraceStep> {
    if limit == 0 {
        return Vec::new();
    }
    if trace.len() <= limit {
        return trace;
    }
    let edge_count = (limit / 4).max(1);
    let mut indices = BTreeSet::new();
    indices.extend(0..edge_count.min(trace.len()));
    indices.extend(trace.len().saturating_sub(edge_count)..trace.len());
    let mut ranked: Vec<_> = trace
        .iter()
        .enumerate()
        .map(|(index, step)| (step.consequence_breadth(), step.viable_candidates, index))
        .collect();
    ranked.sort_by(|left, right| right.cmp(left));
    for (_, _, index) in ranked {
        if indices.len() >= limit {
            break;
        }
        indices.insert(index);
    }
    trace
        .drain(..)
        .enumerate()
        .filter_map(|(index, step)| indices.contains(&index).then_some(step))
        .collect()
}

fn count_business_status(businesses: &[&crate::core::Business], status: BusinessStatus) -> u16 {
    usize_to_u16(
        businesses
            .iter()
            .filter(|business| business.status() == status)
            .count(),
    )
}

fn count_contract_status(state: &AppState, status: ContractStatus) -> u16 {
    usize_to_u16(
        state
            .contracts
            .values()
            .filter(|contract| contract.status == status)
            .count(),
    )
}

fn count_loan_status(state: &AppState, status: LoanStatus) -> u16 {
    usize_to_u16(
        state
            .loans
            .values()
            .filter(|loan| loan.status == status)
            .count(),
    )
}

fn count_civic_debt_status(state: &AppState, status: CivicDebtStatus) -> u16 {
    usize_to_u16(
        state
            .civic_debts
            .values()
            .filter(|debt| debt.status == status)
            .count(),
    )
}

fn count_employment_status(state: &AppState, status: EmploymentStatus) -> u16 {
    usize_to_u16(
        state
            .employment
            .values()
            .filter(|agreement| agreement.status == status)
            .count(),
    )
}

fn count_player_offices(state: &AppState, player_id: DynastyId) -> u16 {
    usize_to_u16(
        state
            .institutions
            .values()
            .filter(|institution| {
                institution.office_holder_id.is_some_and(|character_id| {
                    state
                        .characters
                        .get(character_id)
                        .is_some_and(|character| character.dynasty_id() == player_id)
                })
            })
            .count(),
    )
}

fn count_eligible_officeholders(state: &AppState, player_id: DynastyId) -> u16 {
    usize_to_u16(
        state
            .characters
            .iter()
            .filter(|character| {
                character.dynasty_id() == player_id && character.status() == CharacterStatus::Active
            })
            .count(),
    )
}

fn count_active_player_wards(state: &AppState, player_id: DynastyId) -> u16 {
    usize_to_u16(
        state
            .family_links
            .values()
            .filter(|link| link.active && link.kind == FamilyLinkKind::Ward)
            .filter(|link| {
                state
                    .characters
                    .get(link.second_character_id)
                    .is_some_and(|character| {
                        character.dynasty_id() == player_id
                            && character.status() == CharacterStatus::Active
                    })
            })
            .count(),
    )
}

fn player_family_capability_checksum(state: &AppState, player_id: DynastyId) -> u32 {
    state
        .characters
        .iter()
        .filter(|character| {
            character.dynasty_id() == player_id && character.status() == CharacterStatus::Active
        })
        .fold(0_u32, |total, character| {
            total
                .saturating_add(u32::from(character.capabilities.administration) * 11)
                .saturating_add(u32::from(character.capabilities.commerce) * 13)
                .saturating_add(u32::from(character.capabilities.social) * 17)
                .saturating_add(u32::from(character.capabilities.craft) * 19)
        })
}

fn count_player_memberships(state: &AppState, player_id: DynastyId) -> u16 {
    usize_to_u16(
        state
            .institutions
            .values()
            .map(|institution| {
                institution
                    .members
                    .iter()
                    .filter(|character_id| {
                        state
                            .characters
                            .get(**character_id)
                            .is_some_and(|character| character.dynasty_id() == player_id)
                    })
                    .count()
            })
            .sum(),
    )
}

fn count_player_institutions_represented(state: &AppState, player_id: DynastyId) -> u16 {
    usize_to_u16(
        state
            .institutions
            .values()
            .filter(|institution| {
                institution.members.iter().any(|character_id| {
                    state
                        .characters
                        .get(*character_id)
                        .is_some_and(|character| character.dynasty_id() == player_id)
                })
            })
            .count(),
    )
}

fn average_u16(values: impl Iterator<Item = u16>) -> u16 {
    let (total, count) = values.fold((0_u64, 0_u64), |(total, count), value| {
        (
            total.saturating_add(u64::from(value)),
            count.saturating_add(1),
        )
    });
    u16::try_from(total.checked_div(count).unwrap_or(0)).unwrap_or(u16::MAX)
}

fn average_scores(values: &[u16]) -> u16 {
    average_u16(values.iter().copied())
}

fn scaled_ratio_u64(numerator: u64, denominator: u64, scale: u64) -> u64 {
    if denominator == 0 {
        return 0;
    }
    let result = u128::from(numerator).saturating_mul(u128::from(scale)) / u128::from(denominator);
    u64::try_from(result).unwrap_or(u64::MAX)
}

fn scaled_ratio_usize(numerator: usize, denominator: usize, scale: u64) -> u64 {
    scaled_ratio_u64(
        u64::try_from(numerator).unwrap_or(u64::MAX),
        u64::try_from(denominator).unwrap_or(u64::MAX),
        scale,
    )
}

fn ratio_score(numerator: u32, denominator: u32) -> u16 {
    u16::try_from(scaled_ratio_u64(
        u64::from(numerator),
        u64::from(denominator),
        100,
    ))
    .unwrap_or(100)
    .min(100)
}

fn usize_to_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

fn usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[path = "gameplay_tests.rs"]
mod tests;
