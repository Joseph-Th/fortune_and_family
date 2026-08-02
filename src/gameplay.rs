//! Deterministic gameplay harness that drives the public player-command and simulation pipelines.

use crate::core::{
    AppState, AuditKind, BusinessStatus, CharacterStatus, ContractStatus, CrisisStatus,
    EmploymentStatus, HouseGovernance, LawKind, LegalCaseKind, LegalCaseStatus, LoanStatus,
    NewGameConfig, ObjectiveStatus, PublicWorkKind, PublicWorkStatus, StartingBackground,
};
use crate::ids::{BusinessId, DynastyId};
use crate::money::{Money, Quantity, cost_for};
use crate::registry::{GoodCategory, RecipeDef, Registry};
use crate::systems::{
    BUSINESS_POLICY_CHANGE_INTERVAL_DAYS, CommandError, CrisisResponse,
    LEGAL_CASE_FILING_INTERVAL_DAYS, LaborResponse, LoanTerms, MAX_ACTIVE_SPONSORED_PUBLIC_WORKS,
    NewGameError, PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS, PlayerCommand, SimulationError,
    StrategicError, SupplyContractTerms, advance_days, apply_player_command, build_new_game,
    quote_business_acquisition, validate_invariants,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use thiserror::Error;

const ALL_COMMAND_KINDS: [GameplayCommandKind; 15] = [
    GameplayCommandKind::TransferBusinessCash,
    GameplayCommandKind::AcquireBusiness,
    GameplayCommandKind::InvestInBusiness,
    GameplayCommandKind::SetBusinessPolicy,
    GameplayCommandKind::CreateSupplyContract,
    GameplayCommandKind::IssueLoan,
    GameplayCommandKind::BuyProperty,
    GameplayCommandKind::EnactLaw,
    GameplayCommandKind::StartPublicWork,
    GameplayCommandKind::FileLegalCase,
    GameplayCommandKind::SetHouseGovernance,
    GameplayCommandKind::NominateForOffice,
    GameplayCommandKind::RespondToCrisis,
    GameplayCommandKind::ResolveLaborDispute,
    GameplayCommandKind::AcknowledgeNotification,
];

const ALL_DOMAINS: [GameplayDomain; 15] = [
    GameplayDomain::Economy,
    GameplayDomain::Business,
    GameplayDomain::Market,
    GameplayDomain::Contracts,
    GameplayDomain::Loans,
    GameplayDomain::Property,
    GameplayDomain::Labor,
    GameplayDomain::Dynasty,
    GameplayDomain::Family,
    GameplayDomain::Institutions,
    GameplayDomain::Law,
    GameplayDomain::Districts,
    GameplayDomain::Legal,
    GameplayDomain::Crises,
    GameplayDomain::Information,
];

/// Version of the serialized gameplay-harness report contract.
pub const GAMEPLAY_REPORT_SCHEMA_VERSION: u16 = 7;
const NOTIFICATION_BATCH_THRESHOLD: usize = 8;

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
            decision_interval_days: 7,
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
    CreateSupplyContract,
    IssueLoan,
    BuyProperty,
    EnactLaw,
    StartPublicWork,
    FileLegalCase,
    SetHouseGovernance,
    NominateForOffice,
    RespondToCrisis,
    ResolveLaborDispute,
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
            Self::CreateSupplyContract => "supply-contract",
            Self::IssueLoan => "issue-loan",
            Self::BuyProperty => "buy-property",
            Self::EnactLaw => "enact-law",
            Self::StartPublicWork => "public-work",
            Self::FileLegalCase => "legal-case",
            Self::SetHouseGovernance => "house-governance",
            Self::NominateForOffice => "office-nomination",
            Self::RespondToCrisis => "crisis-response",
            Self::ResolveLaborDispute => "labor-response",
            Self::AcknowledgeNotification => "acknowledge",
        }
    }

    const fn expected_activation_days(self) -> u32 {
        match self {
            Self::TransferBusinessCash
            | Self::AcquireBusiness
            | Self::InvestInBusiness
            | Self::CreateSupplyContract
            | Self::FileLegalCase
            | Self::RespondToCrisis => 360,
            Self::ResolveLaborDispute => 720,
            Self::SetBusinessPolicy
            | Self::IssueLoan
            | Self::BuyProperty
            | Self::EnactLaw
            | Self::StartPublicWork
            | Self::SetHouseGovernance
            | Self::NominateForOffice
            | Self::AcknowledgeNotification => 1,
        }
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
    Dynasty,
    Family,
    Institutions,
    Law,
    Districts,
    Legal,
    Crises,
    Information,
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
            Self::Dynasty => "dynasty",
            Self::Family => "family",
            Self::Institutions => "institutions",
            Self::Law => "law",
            Self::Districts => "districts",
            Self::Legal => "legal",
            Self::Crises => "crises",
            Self::Information => "information",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayCommandStats {
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
    pub player_treasury: Money,
    pub player_business_cash: Money,
    pub active_businesses: u16,
    pub distressed_businesses: u16,
    pub insolvent_businesses: u16,
    pub average_business_condition: u16,
    pub average_business_quality: u16,
    pub business_policy_checksum: i64,
    pub market_price_total: Money,
    pub market_stock_total: Quantity,
    pub active_contracts: u16,
    pub fulfilled_contracts: u16,
    pub breached_contracts: u16,
    pub contract_failures: u32,
    pub player_active_contracts: u16,
    pub player_fulfilled_contracts: u16,
    pub player_breached_contracts: u16,
    pub player_contract_failures: u32,
    pub current_loans: u16,
    pub delinquent_loans: u16,
    pub defaulted_loans: u16,
    pub repaid_loans: u16,
    pub total_loan_balance: Money,
    pub player_properties: u16,
    pub occupied_properties: u16,
    pub active_employment: u16,
    pub disputed_employment: u16,
    pub average_labor_loyalty: u16,
    pub legitimacy: u16,
    pub quality_reputation: u16,
    pub reliability_reputation: u16,
    pub generation: u16,
    pub family_unity: u16,
    pub family_charter_version: u64,
    pub offices_held: u16,
    pub available_offices: u16,
    pub institution_memberships: u16,
    pub institution_budget_total: Money,
    pub active_laws: u16,
    pub law_value_checksum: i64,
    pub public_work_progress_total: u32,
    pub building_public_works: u16,
    pub completed_public_works: u16,
    pub suspended_public_works: u16,
    pub average_food_satisfaction: u16,
    pub average_district_unrest: u16,
    pub open_legal_cases: u16,
    pub decided_legal_cases: u16,
    pub active_crises: u16,
    pub escalated_crises: u16,
    pub resolved_crises: u16,
    pub crisis_severity_total: u32,
    pub information_reports: u16,
    pub achieved_ai_objectives: u16,
    pub unread_notifications: u16,
    pub outbox_messages: u32,
    pub chronicle_entries: u32,
}

#[derive(Debug)]
struct BusinessSnapshotPart {
    player_treasury: Money,
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
    current_loans: u16,
    delinquent_loans: u16,
    defaulted_loans: u16,
    repaid_loans: u16,
    total_loan_balance: Money,
    player_properties: u16,
    occupied_properties: u16,
    active_employment: u16,
    disputed_employment: u16,
    average_labor_loyalty: u16,
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
            current_loans: count_loan_status(state, LoanStatus::Current),
            delinquent_loans: count_loan_status(state, LoanStatus::Delinquent),
            defaulted_loans: count_loan_status(state, LoanStatus::Defaulted),
            repaid_loans: count_loan_status(state, LoanStatus::Repaid),
            total_loan_balance: state.loans.values().fold(Money::ZERO, |total, loan| {
                total.saturating_add(loan.balance)
            }),
            player_properties: usize_to_u16(
                state
                    .properties
                    .values()
                    .filter(|property| property.owner_dynasty_id == Some(player_id))
                    .count(),
            ),
            occupied_properties: usize_to_u16(
                state
                    .properties
                    .values()
                    .filter(|property| property.occupant_business_id.is_some())
                    .count(),
            ),
            active_employment: count_employment_status(state, EmploymentStatus::Active),
            disputed_employment: count_employment_status(state, EmploymentStatus::Disputed),
            average_labor_loyalty: average_u16(
                state
                    .employment
                    .values()
                    .map(|agreement| agreement.loyalty_basis_points),
            ),
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
    offices_held: u16,
    available_offices: u16,
    institution_memberships: u16,
    institution_budget_total: Money,
    active_laws: u16,
    law_value_checksum: i64,
    public_work_progress_total: u32,
    building_public_works: u16,
    completed_public_works: u16,
    suspended_public_works: u16,
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
            offices_held: count_player_offices(state, player_id),
            available_offices: usize_to_u16(state.institutions.len()),
            institution_memberships: count_player_memberships(state, player_id),
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
        }
    }
}

#[derive(Debug)]
struct WorldSnapshotPart {
    average_food_satisfaction: u16,
    average_district_unrest: u16,
    open_legal_cases: u16,
    decided_legal_cases: u16,
    active_crises: u16,
    escalated_crises: u16,
    resolved_crises: u16,
    crisis_severity_total: u32,
    information_reports: u16,
    achieved_ai_objectives: u16,
    unread_notifications: u16,
    outbox_messages: u32,
    chronicle_entries: u32,
}

impl WorldSnapshotPart {
    fn capture(state: &AppState) -> Self {
        Self {
            average_food_satisfaction: average_u16(
                state
                    .households
                    .iter()
                    .map(crate::core::Household::food_satisfaction_basis_points),
            ),
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
            information_reports: usize_to_u16(state.information_reports.len()),
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

impl GameplaySnapshot {
    fn capture(state: &AppState) -> Self {
        let player_id = state.player_dynasty_id;
        let business = BusinessSnapshotPart::capture(state, player_id);
        let strategic = StrategicSnapshotPart::capture(state, player_id);
        let civic = CivicSnapshotPart::capture(state, player_id);
        let world = WorldSnapshotPart::capture(state);
        Self {
            day: state.clock.day(),
            player_treasury: business.player_treasury,
            player_business_cash: business.player_business_cash,
            active_businesses: business.active_businesses,
            distressed_businesses: business.distressed_businesses,
            insolvent_businesses: business.insolvent_businesses,
            average_business_condition: business.average_business_condition,
            average_business_quality: business.average_business_quality,
            business_policy_checksum: business.business_policy_checksum,
            market_price_total: business.market_price_total,
            market_stock_total: business.market_stock_total,
            active_contracts: strategic.active_contracts,
            fulfilled_contracts: strategic.fulfilled_contracts,
            breached_contracts: strategic.breached_contracts,
            contract_failures: strategic.contract_failures,
            player_active_contracts: strategic.player_active_contracts,
            player_fulfilled_contracts: strategic.player_fulfilled_contracts,
            player_breached_contracts: strategic.player_breached_contracts,
            player_contract_failures: strategic.player_contract_failures,
            current_loans: strategic.current_loans,
            delinquent_loans: strategic.delinquent_loans,
            defaulted_loans: strategic.defaulted_loans,
            repaid_loans: strategic.repaid_loans,
            total_loan_balance: strategic.total_loan_balance,
            player_properties: strategic.player_properties,
            occupied_properties: strategic.occupied_properties,
            active_employment: strategic.active_employment,
            disputed_employment: strategic.disputed_employment,
            average_labor_loyalty: strategic.average_labor_loyalty,
            legitimacy: civic.legitimacy,
            quality_reputation: civic.quality_reputation,
            reliability_reputation: civic.reliability_reputation,
            generation: civic.generation,
            family_unity: civic.family_unity,
            family_charter_version: civic.family_charter_version,
            offices_held: civic.offices_held,
            available_offices: civic.available_offices,
            institution_memberships: civic.institution_memberships,
            institution_budget_total: civic.institution_budget_total,
            active_laws: civic.active_laws,
            law_value_checksum: civic.law_value_checksum,
            public_work_progress_total: civic.public_work_progress_total,
            building_public_works: civic.building_public_works,
            completed_public_works: civic.completed_public_works,
            suspended_public_works: civic.suspended_public_works,
            average_food_satisfaction: world.average_food_satisfaction,
            average_district_unrest: world.average_district_unrest,
            open_legal_cases: world.open_legal_cases,
            decided_legal_cases: world.decided_legal_cases,
            active_crises: world.active_crises,
            escalated_crises: world.escalated_crises,
            resolved_crises: world.resolved_crises,
            crisis_severity_total: world.crisis_severity_total,
            information_reports: world.information_reports,
            achieved_ai_objectives: world.achieved_ai_objectives,
            unread_notifications: world.unread_notifications,
            outbox_messages: world.outbox_messages,
            chronicle_entries: world.chronicle_entries,
        }
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
pub struct GameplayTraceStep {
    pub day: i64,
    pub considered_candidates: u16,
    pub viable_candidates: u16,
    pub substantive_viable_candidates: u16,
    pub viable_command_kinds: BTreeSet<GameplayCommandKind>,
    pub selected_command: Option<GameplayCommandKind>,
    pub command_description: Option<String>,
    pub outcome: Option<String>,
    pub rejection_summary: Vec<String>,
    pub immediate_domains: BTreeSet<GameplayDomain>,
    pub delayed_domains: BTreeSet<GameplayDomain>,
    pub persistent_domains: BTreeSet<GameplayDomain>,
    pub ambient_domains: BTreeSet<GameplayDomain>,
    pub immediate_world_feedback: bool,
    pub delayed_world_feedback: bool,
    pub ambient_world_feedback: bool,
}

impl GameplayTraceStep {
    fn consequence_breadth(&self) -> usize {
        self.immediate_domains.union(&self.delayed_domains).count()
    }
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
    pub no_action_cycles: u32,
    pub quiet_cycles: u32,
    pub blocked_cycles: u32,
    pub total_viable_choices: u32,
    pub total_viable_command_kinds: u32,
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
    pub minimum_operating_businesses: u16,
    pub maximum_disputed_employment: u16,
    pub maximum_offices_held: u16,
    pub maximum_unfinished_public_works: u16,
    pub maximum_active_crises: u16,
    pub maximum_unread_notifications: u16,
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
    pub no_action_cycles: u64,
    pub quiet_cycles: u64,
    pub blocked_cycles: u64,
    pub cycles_with_multiple_viable_command_kinds: u64,
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
}

#[derive(Clone, Debug)]
struct Candidate {
    kind: GameplayCommandKind,
    command: PlayerCommand,
    description: String,
    score: i64,
}

#[derive(Debug)]
struct CampaignAccumulator {
    commands: BTreeMap<GameplayCommandKind, GameplayCommandStats>,
    rejection_reasons: BTreeMap<String, u32>,
    domain_changes: BTreeMap<GameplayDomain, u32>,
    causal_domain_changes: BTreeMap<GameplayDomain, u32>,
    ambient_domain_changes: BTreeMap<GameplayDomain, u32>,
    interactions: BTreeMap<(GameplayCommandKind, GameplayDomain), u32>,
    trace: Vec<GameplayTraceStep>,
    decision_cycles: u32,
    cycles_with_viable_choices: u32,
    cycles_with_multiple_viable_command_kinds: u32,
    no_action_cycles: u32,
    quiet_cycles: u32,
    blocked_cycles: u32,
    total_viable_choices: u32,
    total_viable_command_kinds: u32,
    minimum_food_satisfaction: u16,
    minimum_operating_businesses: u16,
    maximum_disputed_employment: u16,
    maximum_offices_held: u16,
    maximum_unfinished_public_works: u16,
    maximum_active_crises: u16,
    maximum_unread_notifications: u16,
    last_command: Option<GameplayCommandKind>,
}

impl CampaignAccumulator {
    fn new() -> Self {
        Self {
            commands: initialized_command_stats(),
            rejection_reasons: BTreeMap::new(),
            domain_changes: initialized_domain_counts(),
            causal_domain_changes: initialized_domain_counts(),
            ambient_domain_changes: initialized_domain_counts(),
            interactions: BTreeMap::new(),
            trace: Vec::new(),
            decision_cycles: 0,
            cycles_with_viable_choices: 0,
            cycles_with_multiple_viable_command_kinds: 0,
            no_action_cycles: 0,
            quiet_cycles: 0,
            blocked_cycles: 0,
            total_viable_choices: 0,
            total_viable_command_kinds: 0,
            minimum_food_satisfaction: u16::MAX,
            minimum_operating_businesses: u16::MAX,
            maximum_disputed_employment: 0,
            maximum_offices_held: 0,
            maximum_unfinished_public_works: 0,
            maximum_active_crises: 0,
            maximum_unread_notifications: 0,
            last_command: None,
        }
    }

    fn observe_initial_snapshot(&mut self, snapshot: &GameplaySnapshot) {
        self.observe_non_food_snapshot(snapshot);
    }

    fn observe_snapshot(&mut self, snapshot: &GameplaySnapshot) {
        self.minimum_food_satisfaction = self
            .minimum_food_satisfaction
            .min(snapshot.average_food_satisfaction);
        self.observe_non_food_snapshot(snapshot);
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
        let seed = config.start_seed.saturating_add(u64::from(seed_offset));
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
    })
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
        no_action_cycles: accumulator.no_action_cycles,
        quiet_cycles: accumulator.quiet_cycles,
        blocked_cycles: accumulator.blocked_cycles,
        total_viable_choices: accumulator.total_viable_choices,
        total_viable_command_kinds: accumulator.total_viable_command_kinds,
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
        minimum_operating_businesses: accumulator.minimum_operating_businesses,
        maximum_disputed_employment: accumulator.maximum_disputed_employment,
        maximum_offices_held: accumulator.maximum_offices_held,
        maximum_unfinished_public_works: accumulator.maximum_unfinished_public_works,
        maximum_active_crises: accumulator.maximum_active_crises,
        maximum_unread_notifications: accumulator.maximum_unread_notifications,
        trace,
    })
}

fn run_decision_cycle(
    registry: &Registry,
    config: &GameplayHarnessConfig,
    persona: GameplayPersona,
    state: &mut AppState,
    step_days: u32,
    accumulator: &mut CampaignAccumulator,
) -> Result<(), GameplayHarnessError> {
    accumulator.decision_cycles = accumulator.decision_cycles.saturating_add(1);
    let mut baseline_state = state.clone();
    let before = GameplaySnapshot::capture(state);
    let candidates = ranked_candidates(registry, state, persona, accumulator);
    let substantive_candidate_count = candidates
        .iter()
        .filter(|candidate| candidate.kind != GameplayCommandKind::AcknowledgeNotification)
        .count();
    record_offered_command_kinds(&candidates, accumulator);
    record_generated_candidates(&candidates, accumulator);
    let probe_limit = usize::from(config.max_candidate_probes).min(candidates.len());
    let (selected, viable_count, substantive_viable_count, viable_command_kinds, rejections) =
        probe_candidates(
            registry,
            state,
            candidates.into_iter().take(probe_limit),
            accumulator,
        );
    accumulator.total_viable_choices = accumulator
        .total_viable_choices
        .saturating_add(usize_to_u32(substantive_viable_count));
    accumulator.total_viable_command_kinds = accumulator
        .total_viable_command_kinds
        .saturating_add(usize_to_u32(viable_command_kinds.len()));
    if substantive_viable_count > 0 {
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
    if viable_command_kinds.len() >= 2 {
        accumulator.cycles_with_multiple_viable_command_kinds = accumulator
            .cycles_with_multiple_viable_command_kinds
            .saturating_add(1);
    }
    let action = execute_selected(registry, state, selected, accumulator)?;
    let after_command = GameplaySnapshot::capture(state);
    let consequence_horizon = consequence_horizon_days(
        action.as_ref().map(|action| action.kind),
        state,
        step_days,
        config.max_consequence_horizon_days,
    );
    let mut consequence_state = (consequence_horizon > step_days).then(|| state.clone());
    advance_days(registry, state, step_days)?;
    let campaign_after_time = GameplaySnapshot::capture(state);
    accumulator.observe_snapshot(&campaign_after_time);
    let after_time = if let Some(consequence_state) = consequence_state.as_mut() {
        advance_days(registry, consequence_state, consequence_horizon)?;
        GameplaySnapshot::capture(consequence_state)
    } else {
        campaign_after_time
    };
    advance_days(registry, &mut baseline_state, consequence_horizon)?;
    let baseline_after_time = GameplaySnapshot::capture(&baseline_state);
    record_cycle(
        CycleObservation {
            before: &before,
            after_command: &after_command,
            after_time: &after_time,
            baseline_after_time: &baseline_after_time,
            considered: probe_limit,
            viable: viable_count,
            substantive_viable: substantive_viable_count,
            viable_command_kinds,
            rejections,
            action,
        },
        accumulator,
    );
    Ok(())
}

fn consequence_horizon_days(
    command: Option<GameplayCommandKind>,
    state: &AppState,
    step_days: u32,
    maximum: u16,
) -> u32 {
    let desired = match command {
        Some(GameplayCommandKind::SetHouseGovernance) => 360,
        Some(GameplayCommandKind::NominateForOffice) => state
            .institutions
            .values()
            .map(|institution| {
                institution
                    .next_selection_day
                    .saturating_sub(state.clock.day())
                    .max(1)
            })
            .min()
            .and_then(|days| u32::try_from(days).ok())
            .unwrap_or(90),
        Some(GameplayCommandKind::StartPublicWork | GameplayCommandKind::FileLegalCase) => 60,
        Some(
            GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::CreateSupplyContract
            | GameplayCommandKind::IssueLoan
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::EnactLaw,
        ) => 30,
        Some(
            GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::ResolveLaborDispute
            | GameplayCommandKind::AcknowledgeNotification,
        )
        | None => step_days,
    };
    desired.min(u32::from(maximum)).max(step_days)
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

fn execute_selected(
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
    accumulator.last_command = Some(candidate.kind);
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
    accumulator: &mut CampaignAccumulator,
) -> (
    Option<Candidate>,
    usize,
    usize,
    BTreeSet<GameplayCommandKind>,
    Vec<String>,
) {
    let mut selected = None;
    let mut viable_count = 0_usize;
    let mut substantive_viable_count = 0_usize;
    let mut viable_command_kinds = BTreeSet::new();
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
                    viable_command_kinds.insert(candidate.kind);
                }
                if selected.is_none() {
                    selected = Some(candidate);
                }
            }
            Err(error) => {
                command_stats.rejected = command_stats.rejected.saturating_add(1);
                let category = command_error_category(&error);
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
    (
        selected,
        viable_count,
        substantive_viable_count,
        viable_command_kinds,
        rejections,
    )
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
        rejections,
        action,
    } = observation;
    let immediate_domains = before.changed_domains(after_command);
    let total_causal_domains = baseline_after_time.changed_domains(after_time);
    let persistent_domains: BTreeSet<_> = immediate_domains
        .intersection(&total_causal_domains)
        .copied()
        .collect();
    let delayed_domains: BTreeSet<_> = total_causal_domains
        .difference(&immediate_domains)
        .copied()
        .collect();
    let ambient_domains = before.changed_domains(baseline_after_time);
    let immediate_feedback = !immediate_domains.is_empty()
        || after_command.outbox_messages > before.outbox_messages
        || after_command.chronicle_entries > before.chronicle_entries;
    let action_delayed_outbox = after_time
        .outbox_messages
        .saturating_sub(after_command.outbox_messages);
    let ambient_outbox = baseline_after_time
        .outbox_messages
        .saturating_sub(before.outbox_messages);
    let action_delayed_chronicle = after_time
        .chronicle_entries
        .saturating_sub(after_command.chronicle_entries);
    let ambient_chronicle = baseline_after_time
        .chronicle_entries
        .saturating_sub(before.chronicle_entries);
    let delayed_feedback =
        action_delayed_outbox != ambient_outbox || action_delayed_chronicle != ambient_chronicle;
    let ambient_feedback = baseline_after_time.outbox_messages > before.outbox_messages
        || baseline_after_time.chronicle_entries > before.chronicle_entries;
    let observed_domains: BTreeSet<_> = immediate_domains
        .union(&delayed_domains)
        .copied()
        .chain(ambient_domains.iter().copied())
        .collect();
    for domain in observed_domains {
        *accumulator.domain_changes.entry(domain).or_default() += 1;
    }
    for domain in immediate_domains.union(&delayed_domains) {
        *accumulator
            .causal_domain_changes
            .entry(*domain)
            .or_default() += 1;
    }
    for domain in &ambient_domains {
        *accumulator
            .ambient_domain_changes
            .entry(*domain)
            .or_default() += 1;
    }
    if let Some(action) = &action {
        record_action_consequences(
            action.kind,
            &immediate_domains,
            &persistent_domains,
            &delayed_domains,
            immediate_feedback,
            delayed_feedback,
            accumulator,
        );
    }
    accumulator.trace.push(GameplayTraceStep {
        day: before.day,
        considered_candidates: usize_to_u16(considered),
        viable_candidates: usize_to_u16(viable),
        substantive_viable_candidates: usize_to_u16(substantive_viable),
        viable_command_kinds,
        selected_command: action.as_ref().map(|action| action.kind),
        command_description: action.as_ref().map(|action| action.description.clone()),
        outcome: action.map(|action| action.outcome),
        rejection_summary: rejections,
        immediate_domains,
        delayed_domains,
        persistent_domains,
        ambient_domains,
        immediate_world_feedback: immediate_feedback,
        delayed_world_feedback: delayed_feedback,
        ambient_world_feedback: ambient_feedback,
    });
}

fn record_action_consequences(
    kind: GameplayCommandKind,
    immediate: &BTreeSet<GameplayDomain>,
    persistent: &BTreeSet<GameplayDomain>,
    delayed: &BTreeSet<GameplayDomain>,
    immediate_feedback: bool,
    delayed_feedback: bool,
    accumulator: &mut CampaignAccumulator,
) {
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
    if !persistent.is_empty() {
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
    generate_finance_candidates(state, persona, &mut candidates);
    generate_civic_candidates(registry, state, persona, &mut candidates);
    generate_family_candidates(state, persona, &mut candidates);
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

fn generate_reactive_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    for crisis in state.crises.values().filter(|crisis| {
        crisis.status.is_active()
            && !state.audit_log.iter().rev().any(|record| {
                record.kind() == AuditKind::CrisisResponse
                    && record.subject() == format!("crisis:{}", crisis.id)
                    && record.day().saturating_add(30) > state.clock.day()
            })
    }) {
        for response in crisis_responses(persona) {
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
        for response in labor_responses(persona) {
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
    generate_cash_rebalance_candidate(&player_businesses, candidates);
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
    for template in policy_templates(persona) {
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

fn generate_cash_rebalance_candidate(
    player_businesses: &[&crate::core::Business],
    candidates: &mut Vec<Candidate>,
) {
    if player_businesses.len() < 2 {
        return;
    }
    let richest = player_businesses
        .iter()
        .max_by_key(|business| business.cash());
    let poorest = player_businesses
        .iter()
        .min_by_key(|business| business.cash());
    let (Some(richest), Some(poorest)) = (richest, poorest) else {
        return;
    };
    if richest.id() == poorest.id() || richest.cash() <= poorest.cash() {
        return;
    }
    let amount = Money::from_copper(
        richest
            .cash()
            .copper()
            .saturating_sub(poorest.cash().copper())
            / 2,
    );
    if amount <= Money::ZERO {
        return;
    }
    push_candidate(
        candidates,
        GameplayCommandKind::TransferBusinessCash,
        PlayerCommand::TransferBusinessCash {
            from_business_id: richest.id(),
            to_business_id: poorest.id(),
            amount,
        },
        format!(
            "rebalance {amount} from business {} to {}",
            richest.id(),
            poorest.id()
        ),
        100,
    );
}

fn generate_business_investment_candidate(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    business: &crate::core::Business,
    candidates: &mut Vec<Candidate>,
) {
    if !matches!(
        business.status(),
        BusinessStatus::Distressed | BusinessStatus::Insolvent
    ) {
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
    let dynasty_reserve = recapitalization_dynasty_reserve(persona, staple_emergency);
    let spendable = Money::from_copper(
        player_treasury
            .copper()
            .saturating_sub(dynasty_reserve.copper())
            .max(0),
    );
    if spendable <= Money::ZERO {
        return;
    }
    let target_cash = business_recapitalization_target(state, business, recipe);
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
    let emergency_bonus = if staple_emergency { 3_000 } else { 0 };
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

fn average_household_food_satisfaction(state: &AppState) -> u16 {
    if state.households.records().is_empty() {
        return 10_000;
    }
    let total: u64 = state
        .households
        .iter()
        .map(|household| u64::from(household.food_satisfaction_basis_points()))
        .sum();
    let count = u64::try_from(state.households.records().len()).unwrap_or(u64::MAX);
    u16::try_from(total / count).unwrap_or(10_000)
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

fn business_recapitalization_target(
    state: &AppState,
    business: &crate::core::Business,
    recipe: &RecipeDef,
) -> Money {
    let payroll_buffer = state
        .employment
        .values()
        .filter(|agreement| {
            agreement.business_id == business.id() && agreement.status != EmploymentStatus::Ended
        })
        .fold(Money::ZERO, |total, agreement| {
            total.saturating_add(agreement.weekly_wage.saturating_mul(2))
        });
    let input_buffer = recipe.inputs().iter().fold(Money::ZERO, |total, input| {
        let price = state
            .market
            .get_quote(input.good_id())
            .expect("recipe input good must have a market quote")
            .price();
        let quantity = input.quantity().saturating_mul_ratio(
            i64::from(business.operations.capacity_batches_per_day).saturating_mul(7),
            1,
        );
        total.saturating_add(cost_for(quantity, price))
    });
    business
        .policy
        .minimum_cash_reserve
        .saturating_add(recipe.daily_operating_cost().saturating_mul(14))
        .saturating_add(payroll_buffer)
        .saturating_add(input_buffer)
}

fn generate_business_acquisition_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    player_businesses: &[&crate::core::Business],
    candidates: &mut Vec<Candidate>,
) {
    let portfolio_limit = match persona {
        GameplayPersona::Entrepreneur => 4,
        GameplayPersona::Opportunist => 3,
        GameplayPersona::Steward | GameplayPersona::PowerBroker => 2,
    };
    if player_businesses.len() >= portfolio_limit {
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
            maintenance_basis_points: 800,
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
    let contract_bonus = match persona {
        GameplayPersona::Entrepreneur => 500,
        GameplayPersona::Steward => 250,
        GameplayPersona::PowerBroker => 100,
        GameplayPersona::Opportunist => 180,
    };
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
            if let Some(seller) = find_contract_seller(registry, state, input.good_id(), player_id)
            {
                add_contract_candidate(
                    registry,
                    state,
                    candidates,
                    ContractCandidateInput {
                        buyer_business_id: business.id(),
                        seller_business_id: seller,
                        good_id: input.good_id(),
                        quantity_per_week: input.quantity().saturating_mul_ratio(4, 1),
                        bonus: contract_bonus,
                    },
                );
            }
        }
        if let Some(buyer) =
            find_contract_buyer(registry, state, recipe.output_good_id(), player_id)
        {
            add_contract_candidate(
                registry,
                state,
                candidates,
                ContractCandidateInput {
                    buyer_business_id: buyer,
                    seller_business_id: business.id(),
                    good_id: recipe.output_good_id(),
                    quantity_per_week: recipe.output_quantity().saturating_mul_ratio(4, 1),
                    bonus: contract_bonus.saturating_sub(30),
                },
            );
        }
    }
}

fn find_contract_seller(
    registry: &Registry,
    state: &AppState,
    good_id: crate::ids::GoodId,
    excluded_owner: DynastyId,
) -> Option<BusinessId> {
    state.businesses.iter().find_map(|business| {
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

fn find_contract_buyer(
    registry: &Registry,
    state: &AppState,
    good_id: crate::ids::GoodId,
    excluded_owner: DynastyId,
) -> Option<BusinessId> {
    state.businesses.iter().find_map(|business| {
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
    if !contract_terms_are_operationally_supported(
        registry,
        state,
        buyer_business_id,
        seller_business_id,
        good_id,
        quantity_per_week,
    ) {
        return;
    }
    let Some(quote) = state.market.quotes.get(&good_id) else {
        return;
    };
    let penalty = cost_for(quantity_per_week, quote.price).saturating_mul(2);
    push_candidate(
        candidates,
        GameplayCommandKind::CreateSupplyContract,
        PlayerCommand::CreateSupplyContract {
            terms: SupplyContractTerms {
                buyer_business_id,
                seller_business_id,
                good_id,
                quantity_per_week,
                unit_price: quote.price,
                penalty,
                duration_weeks: 26,
            },
        },
        format!(
            "contract good {good_id} from business {seller_business_id} to {buyer_business_id}"
        ),
        bonus,
    );
}

fn contract_terms_are_operationally_supported(
    registry: &Registry,
    state: &AppState,
    buyer_business_id: BusinessId,
    seller_business_id: BusinessId,
    good_id: crate::ids::GoodId,
    quantity_per_week: Quantity,
) -> bool {
    let Some(buyer) = state.businesses.get(buyer_business_id) else {
        return false;
    };
    let Some(seller) = state.businesses.get(seller_business_id) else {
        return false;
    };
    let Some(buyer_recipe) = registry.get_recipe(buyer.recipe_id()) else {
        return false;
    };
    let Some(seller_recipe) = registry.get_recipe(seller.recipe_id()) else {
        return false;
    };
    let Some(input_per_batch) = buyer_recipe
        .inputs()
        .iter()
        .find(|input| input.good_id() == good_id)
        .map(crate::registry::RecipeInput::quantity)
    else {
        return false;
    };
    let seller_capacity = seller_recipe.output_quantity().saturating_mul_ratio(
        i64::from(seller.operations.capacity_batches_per_day).saturating_mul(5),
        1,
    );
    let buyer_capacity = input_per_batch.saturating_mul_ratio(
        i64::from(buyer.operations.capacity_batches_per_day).saturating_mul(5),
        1,
    );
    let existing_outgoing = state
        .contracts
        .values()
        .filter(|contract| {
            contract.status == ContractStatus::Active
                && contract.seller_business_id == seller_business_id
                && contract.good_id == good_id
        })
        .fold(Quantity::ZERO, |total, contract| {
            total.saturating_add(contract.quantity_per_week)
        });
    let existing_incoming = state
        .contracts
        .values()
        .filter(|contract| {
            contract.status == ContractStatus::Active
                && contract.buyer_business_id == buyer_business_id
                && contract.good_id == good_id
        })
        .fold(Quantity::ZERO, |total, contract| {
            total.saturating_add(contract.quantity_per_week)
        });
    if existing_outgoing.saturating_add(quantity_per_week) > seller_capacity
        || existing_incoming.saturating_add(quantity_per_week) > buyer_capacity
    {
        return false;
    }
    let Some(quote) = state.market.quotes.get(&good_id) else {
        return false;
    };
    let weekly_payment = cost_for(quantity_per_week, quote.price);
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
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    add_borrow_candidate(state, persona, candidates);
    add_lend_candidate(state, persona, candidates);
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

fn add_borrow_candidate(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player_id = state.player_dynasty_id;
    let lender = state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != player_id)
        .max_by_key(|dynasty| dynasty.treasury());
    let Some(lender) = lender else {
        return;
    };
    if state.loans.values().any(|loan| {
        loan.lender_dynasty_id == lender.id()
            && loan.borrower_dynasty_id == player_id
            && matches!(
                loan.status,
                LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured
            )
    }) {
        return;
    }
    let principal = Money::from_copper((lender.treasury().copper() / 8).clamp(1_000, 12_000));
    let collateral = state.properties.values().find(|property| {
        property.owner_dynasty_id == Some(player_id) && property.collateral_loan_id.is_none()
    });
    let bonus = match persona {
        GameplayPersona::Opportunist => 520,
        GameplayPersona::Entrepreneur => 380,
        GameplayPersona::Steward => 80,
        GameplayPersona::PowerBroker => 120,
    };
    push_candidate(
        candidates,
        GameplayCommandKind::IssueLoan,
        PlayerCommand::IssueLoan {
            terms: LoanTerms {
                lender_dynasty_id: lender.id(),
                borrower_dynasty_id: player_id,
                principal,
                weekly_payment: Money::from_copper((principal.copper() / 26).max(1)),
                interest_basis_points: 700,
                collateral_property_id: collateral.map(|property| property.id),
            },
        },
        format!("borrow {principal} from dynasty {}", lender.id()),
        bonus,
    );
}

fn add_lend_candidate(state: &AppState, persona: GameplayPersona, candidates: &mut Vec<Candidate>) {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    if player.treasury() < Money::from_copper(4_000) {
        return;
    }
    let borrower = state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .min_by_key(|dynasty| dynasty.treasury());
    let Some(borrower) = borrower else {
        return;
    };
    if state.loans.values().any(|loan| {
        loan.lender_dynasty_id == state.player_dynasty_id
            && loan.borrower_dynasty_id == borrower.id()
            && matches!(
                loan.status,
                LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured
            )
    }) {
        return;
    }
    let principal = Money::from_copper((player.treasury().copper() / 10).clamp(1_000, 8_000));
    let collateral = state.properties.values().find(|property| {
        property.owner_dynasty_id == Some(borrower.id()) && property.collateral_loan_id.is_none()
    });
    let bonus = match persona {
        GameplayPersona::PowerBroker => 430,
        GameplayPersona::Entrepreneur => 300,
        GameplayPersona::Opportunist => 260,
        GameplayPersona::Steward => 100,
    };
    push_candidate(
        candidates,
        GameplayCommandKind::IssueLoan,
        PlayerCommand::IssueLoan {
            terms: LoanTerms {
                lender_dynasty_id: state.player_dynasty_id,
                borrower_dynasty_id: borrower.id(),
                principal,
                weekly_payment: Money::from_copper((principal.copper() / 20).max(1)),
                interest_basis_points: 900,
                collateral_property_id: collateral.map(|property| property.id),
            },
        },
        format!("lend {principal} to dynasty {}", borrower.id()),
        bonus,
    );
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
    let law_bonus: i64 = match persona {
        GameplayPersona::PowerBroker => 560,
        GameplayPersona::Steward => 260,
        GameplayPersona::Entrepreneur => 180,
        GameplayPersona::Opportunist => 140,
    };
    for (kind, value) in law_candidates(registry, state) {
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

fn law_candidates(registry: &Registry, state: &AppState) -> [(LawKind, i64); 7] {
    let bread_price = registry
        .get_good_id("bread")
        .and_then(|good_id| state.market.quotes.get(&good_id))
        .map_or(1, |quote| quote.price.copper())
        .max(1);
    [
        (LawKind::BreadPriceCeiling, bread_price),
        (LawKind::ForeignMerchantToll, 600),
        (LawKind::InterestLimit, 800),
        (LawKind::FireCode, 7_000),
        (LawKind::RentRestriction, 900),
        (LawKind::GuildEntryRestriction, 1_200),
        (LawKind::EmergencyImports, 250),
    ]
}

fn law_persona_bonus(persona: GameplayPersona, kind: LawKind) -> i64 {
    match persona {
        GameplayPersona::Steward => match kind {
            LawKind::BreadPriceCeiling | LawKind::EmergencyImports => 220,
            LawKind::FireCode | LawKind::RentRestriction => 180,
            LawKind::ForeignMerchantToll
            | LawKind::InterestLimit
            | LawKind::GuildEntryRestriction
            | LawKind::PublicDebtAuthorization => 0,
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
        GameplayPersona::PowerBroker => 120,
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

fn generate_legal_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
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
    for defendant in state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .take(3)
    {
        let kind = if state.relationships.values().any(|relationship| {
            (relationship.pair.first == defendant.id()
                || relationship.pair.second == defendant.id())
                && relationship.resentment_basis_points > 4_000
        }) {
            LegalCaseKind::Fraud
        } else {
            LegalCaseKind::ContractBreach
        };
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

fn generate_family_candidates(
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
        .is_none_or(|record| record.day().saturating_add(360) <= state.clock.day());
    if governance_available {
        for governance in [
            HouseGovernance::Primogeniture,
            HouseGovernance::FamilyPartnership,
            HouseGovernance::BranchFederation,
        ] {
            if governance == council.governance {
                continue;
            }
            push_candidate(
                candidates,
                GameplayCommandKind::SetHouseGovernance,
                PlayerCommand::SetHouseGovernance { governance },
                format!("adopt {governance:?} governance"),
                governance_bonus(persona, governance),
            );
        }
    }
    let nomination_bonus = match persona {
        GameplayPersona::PowerBroker => 620,
        GameplayPersona::Steward => 170,
        GameplayPersona::Entrepreneur => 130,
        GameplayPersona::Opportunist => 260,
    };
    let mut characters: Vec<_> = state
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
        .collect();
    characters.sort_by_key(|character| {
        std::cmp::Reverse((character.capabilities.social, character.id()))
    });
    for institution in state.institutions.values() {
        if let Some(character) = characters
            .iter()
            .find(|character| !institution.members.contains(&character.id()))
        {
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
                nomination_bonus,
            );
        }
    }
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
    let coverage = if command_stats.executed == 0 { 900 } else { 0 };
    let repetition = i64::from(command_stats.executed).saturating_mul(35);
    let repeat_last = if accumulator.last_command == Some(kind) {
        260
    } else {
        0
    };
    persona_weight(persona, kind)
        .saturating_add(coverage)
        .saturating_add(urgency_weight(state, kind))
        .saturating_sub(repetition)
        .saturating_sub(repeat_last)
}

fn persona_weight(persona: GameplayPersona, kind: GameplayCommandKind) -> i64 {
    match persona {
        GameplayPersona::Steward => match kind {
            GameplayCommandKind::RespondToCrisis | GameplayCommandKind::ResolveLaborDispute => 900,
            GameplayCommandKind::InvestInBusiness => 800,
            GameplayCommandKind::SetBusinessPolicy | GameplayCommandKind::StartPublicWork => 600,
            GameplayCommandKind::AcknowledgeNotification => 300,
            GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::CreateSupplyContract
            | GameplayCommandKind::IssueLoan
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::EnactLaw
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::SetHouseGovernance
            | GameplayCommandKind::NominateForOffice => 180,
        },
        GameplayPersona::Entrepreneur => match kind {
            GameplayCommandKind::CreateSupplyContract
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::TransferBusinessCash => 850,
            GameplayCommandKind::IssueLoan => 650,
            GameplayCommandKind::EnactLaw
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::SetHouseGovernance
            | GameplayCommandKind::NominateForOffice
            | GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::ResolveLaborDispute
            | GameplayCommandKind::AcknowledgeNotification => 140,
        },
        GameplayPersona::PowerBroker => match kind {
            GameplayCommandKind::EnactLaw
            | GameplayCommandKind::NominateForOffice
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::FileLegalCase => 900,
            GameplayCommandKind::SetHouseGovernance => 700,
            GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::CreateSupplyContract
            | GameplayCommandKind::IssueLoan
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::ResolveLaborDispute
            | GameplayCommandKind::AcknowledgeNotification => 120,
        },
        GameplayPersona::Opportunist => match kind {
            GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::IssueLoan
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::FileLegalCase => 850,
            GameplayCommandKind::CreateSupplyContract => 500,
            GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::EnactLaw
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::SetHouseGovernance
            | GameplayCommandKind::NominateForOffice
            | GameplayCommandKind::ResolveLaborDispute
            | GameplayCommandKind::AcknowledgeNotification => 100,
        },
    }
}

fn urgency_weight(state: &AppState, kind: GameplayCommandKind) -> i64 {
    match kind {
        GameplayCommandKind::RespondToCrisis => {
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
        GameplayCommandKind::ResolveLaborDispute => {
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
        GameplayCommandKind::SetBusinessPolicy => {
            if state.businesses.iter().any(|business| {
                business.owner_dynasty_id() == state.player_dynasty_id
                    && business.status() == BusinessStatus::Distressed
            }) {
                1_000
            } else {
                0
            }
        }
        GameplayCommandKind::InvestInBusiness => {
            if state.businesses.iter().any(|business| {
                business.owner_dynasty_id() == state.player_dynasty_id
                    && matches!(
                        business.status(),
                        BusinessStatus::Distressed | BusinessStatus::Insolvent
                    )
            }) {
                2_400
            } else {
                0
            }
        }
        GameplayCommandKind::AcquireBusiness => {
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
        GameplayCommandKind::AcknowledgeNotification => {
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
        GameplayCommandKind::IssueLoan => {
            if state
                .dynasties
                .get(&state.player_dynasty_id)
                .is_some_and(|dynasty| dynasty.treasury() < Money::from_copper(8_000))
            {
                700
            } else {
                0
            }
        }
        GameplayCommandKind::TransferBusinessCash
        | GameplayCommandKind::CreateSupplyContract
        | GameplayCommandKind::BuyProperty
        | GameplayCommandKind::EnactLaw
        | GameplayCommandKind::StartPublicWork
        | GameplayCommandKind::FileLegalCase
        | GameplayCommandKind::SetHouseGovernance
        | GameplayCommandKind::NominateForOffice => 0,
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

fn command_error_category(error: &CommandError) -> String {
    match error {
        CommandError::Strategic(source) => strategic_error_category(source).to_owned(),
        CommandError::Simulation(source) => simulation_error_category(source).to_owned(),
        CommandError::MissingBusiness { .. } => "missing business".to_owned(),
        CommandError::BusinessNotOwned { .. } => "business not owned".to_owned(),
        CommandError::PlayerNotParty => "player not party".to_owned(),
        CommandError::InvalidBusinessPolicy => "invalid business policy".to_owned(),
        CommandError::UnchangedBusinessPolicy { .. } => "unchanged business policy".to_owned(),
        CommandError::BusinessPolicyCooldown { .. } => "business policy cooldown".to_owned(),
        CommandError::InvalidBusinessInvestment => "invalid business investment".to_owned(),
        CommandError::InvalidLawValue { .. } => "invalid law value".to_owned(),
        CommandError::UnchangedLaw { .. } => "unchanged law".to_owned(),
        CommandError::UnsupportedLaw { .. } => "unsupported law".to_owned(),
        CommandError::MissingDistrict { .. } => "missing district".to_owned(),
        CommandError::MissingDynasty { .. } => "missing dynasty".to_owned(),
        CommandError::InsufficientPlayerFunds { .. } => "insufficient player funds".to_owned(),
        CommandError::InsufficientPlayerLegitimacy { .. } => {
            "insufficient player legitimacy".to_owned()
        }
        CommandError::InsufficientBusinessFunds { .. } => "insufficient business funds".to_owned(),
        CommandError::InvalidPublicWorkBudget => "invalid public-work budget".to_owned(),
        CommandError::DuplicateActivePublicWork { .. } => "duplicate active public work".to_owned(),
        CommandError::PublicWorkCooldown { .. } => "public-work cooldown".to_owned(),
        CommandError::PublicWorkCapacity { .. } => "public-work capacity".to_owned(),
        CommandError::SameLegalParty => "same legal party".to_owned(),
        CommandError::InvalidLegalTerms => "invalid legal terms".to_owned(),
        CommandError::DuplicateActiveLegalCase { .. } => "duplicate active legal case".to_owned(),
        CommandError::LegalCaseCooldown { .. } => "legal-case cooldown".to_owned(),
        CommandError::MissingFamilyCouncil { .. } => "missing family council".to_owned(),
        CommandError::UnchangedHouseGovernance { .. } => "unchanged governance".to_owned(),
        CommandError::HouseGovernanceCooldown { .. } => "governance cooldown".to_owned(),
        CommandError::MissingInstitution { .. } => "missing institution".to_owned(),
        CommandError::AlreadyInstitutionMember { .. } => "already institution member".to_owned(),
        CommandError::InvalidNominee { .. } => "invalid nominee".to_owned(),
        CommandError::NomineeAlreadyHoldsOffice { .. } => "nominee already holds office".to_owned(),
        CommandError::MissingCrisis { .. } => "missing crisis".to_owned(),
        CommandError::InactiveCrisis { .. } => "inactive crisis".to_owned(),
        CommandError::CrisisResponseCooldown { .. } => "crisis response cooldown".to_owned(),
        CommandError::MissingEmployment { .. } => "missing employment".to_owned(),
        CommandError::InvalidLaborDispute { .. } => "invalid labor dispute".to_owned(),
        CommandError::NoReplacementLaborAvailable { .. } => {
            "no replacement labor available".to_owned()
        }
        CommandError::MissingNotification { .. } => "missing notification".to_owned(),
    }
}

const fn strategic_error_category(error: &StrategicError) -> &'static str {
    match error {
        StrategicError::MissingBusiness { .. } => "strategic: missing business",
        StrategicError::BusinessInactive { .. } => "strategic: inactive business",
        StrategicError::MissingDynasty { .. } => "strategic: missing dynasty",
        StrategicError::MissingProperty { .. } => "strategic: missing property",
        StrategicError::SameContractParty => "strategic: same contract party",
        StrategicError::SameLoanParty => "strategic: same loan party",
        StrategicError::NonPositiveAmount => "strategic: nonpositive amount",
        StrategicError::NonPositiveQuantity => "strategic: nonpositive quantity",
        StrategicError::EmptyContractDuration => "strategic: empty contract duration",
        StrategicError::SellerCannotProduce { .. } => "strategic: seller cannot produce",
        StrategicError::BuyerDoesNotConsume { .. } => "strategic: buyer does not consume",
        StrategicError::InsufficientDynastyFunds { .. } => "strategic: insufficient dynasty funds",
        StrategicError::InterestOutOfRange { .. } => "strategic: interest out of range",
        StrategicError::CollateralNotOwned { .. } => "strategic: collateral not owned",
        StrategicError::PropertyAlreadyPledged { .. } => "strategic: property already pledged",
        StrategicError::PropertyAlreadyOwned { .. } => "strategic: property already owned",
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
        SimulationError::RegistryMismatch { .. } => "simulation: registry mismatch",
        SimulationError::BusinessNotFound { .. } => "simulation: business not found",
        SimulationError::BusinessInactive { .. } => "simulation: inactive business",
        SimulationError::SameBusiness { .. } => "simulation: same business",
        SimulationError::NonPositiveAmount { .. } => "simulation: nonpositive amount",
        SimulationError::InsufficientBusinessCash { .. } => {
            "simulation: insufficient business cash"
        }
        SimulationError::MarketQuoteMissing { .. } => "simulation: missing market quote",
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
    let actionability = if opportunity_cycles == 0 {
        100
    } else {
        ratio_score(accumulator.cycles_with_viable_choices, opportunity_cycles)
    };
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
    let business = if end.active_businesses > 0 {
        100
    } else if end.distressed_businesses > 0 {
        65
    } else {
        0
    };
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
        if accumulator.minimum_operating_businesses > 0 {
            100
        } else {
            0
        },
        100_u16.saturating_sub(accumulator.maximum_disputed_employment.saturating_mul(8)),
        100_u16.saturating_sub(accumulator.maximum_active_crises.saturating_mul(15)),
    ]);
    average_scores(&[business, food.min(100), treasury, crisis, trajectory])
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

fn aggregate_campaigns(campaigns: &[GameplayCampaignReport]) -> GameplayAggregate {
    let mut commands = initialized_command_stats();
    let mut rejection_reasons = BTreeMap::new();
    let mut domain_changes = initialized_domain_counts();
    let mut causal_domain_changes = initialized_domain_counts();
    let mut ambient_domain_changes = initialized_domain_counts();
    let mut interactions = BTreeMap::new();
    let mut simulated_days = 0_u64;
    let mut decision_cycles = 0_u64;
    let mut viable_choices = 0_u64;
    let mut viable_command_kinds = 0_u64;
    let mut no_action_cycles = 0_u64;
    let mut quiet_cycles = 0_u64;
    let mut blocked_cycles = 0_u64;
    let mut cycles_with_multiple_viable_command_kinds = 0_u64;
    for campaign in campaigns {
        merge_campaign(
            campaign,
            &mut commands,
            &mut rejection_reasons,
            &mut domain_changes,
            &mut causal_domain_changes,
            &mut ambient_domain_changes,
            &mut interactions,
        );
        simulated_days = simulated_days.saturating_add(u64::from(campaign.simulated_days));
        decision_cycles = decision_cycles.saturating_add(u64::from(campaign.decision_cycles));
        viable_choices = viable_choices.saturating_add(u64::from(campaign.total_viable_choices));
        viable_command_kinds =
            viable_command_kinds.saturating_add(u64::from(campaign.total_viable_command_kinds));
        no_action_cycles = no_action_cycles.saturating_add(u64::from(campaign.no_action_cycles));
        quiet_cycles = quiet_cycles.saturating_add(u64::from(campaign.quiet_cycles));
        blocked_cycles = blocked_cycles.saturating_add(u64::from(campaign.blocked_cycles));
        cycles_with_multiple_viable_command_kinds = cycles_with_multiple_viable_command_kinds
            .saturating_add(u64::from(
                campaign.cycles_with_multiple_viable_command_kinds,
            ));
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
        simulated_days,
        decision_cycles,
        successful_actions,
        substantive_actions,
        candidate_probes,
        viable_choices,
        viable_command_kinds,
        no_action_cycles,
        quiet_cycles,
        blocked_cycles,
        cycles_with_multiple_viable_command_kinds,
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
    add_business_survival_finding(campaigns, &mut findings);
    add_system_health_findings(aggregate, campaigns, &mut findings);
    add_choice_quality_finding(aggregate, &mut findings);
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
                if campaign_days >= u64::from(kind.expected_activation_days()) {
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
                    "offered_cycles={}, generated={}, considered={}, viable={}, rejected={}; no configured agent executed it",
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
            .any(|kind| campaign_days >= u64::from(kind.expected_activation_days()));
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
            GameplayCommandKind::IssueLoan,
            GameplayCommandKind::BuyProperty,
        ],
        GameplayDomain::Business => &[
            GameplayCommandKind::AcquireBusiness,
            GameplayCommandKind::InvestInBusiness,
            GameplayCommandKind::SetBusinessPolicy,
        ],
        GameplayDomain::Market => &[
            GameplayCommandKind::SetBusinessPolicy,
            GameplayCommandKind::CreateSupplyContract,
            GameplayCommandKind::EnactLaw,
        ],
        GameplayDomain::Contracts => &[GameplayCommandKind::CreateSupplyContract],
        GameplayDomain::Loans => &[GameplayCommandKind::IssueLoan],
        GameplayDomain::Property => &[GameplayCommandKind::BuyProperty],
        GameplayDomain::Labor => &[
            GameplayCommandKind::InvestInBusiness,
            GameplayCommandKind::SetBusinessPolicy,
            GameplayCommandKind::ResolveLaborDispute,
        ],
        GameplayDomain::Dynasty => &[
            GameplayCommandKind::IssueLoan,
            GameplayCommandKind::EnactLaw,
            GameplayCommandKind::NominateForOffice,
            GameplayCommandKind::RespondToCrisis,
        ],
        GameplayDomain::Family => &[GameplayCommandKind::SetHouseGovernance],
        GameplayDomain::Institutions => &[
            GameplayCommandKind::StartPublicWork,
            GameplayCommandKind::NominateForOffice,
        ],
        GameplayDomain::Law => &[GameplayCommandKind::EnactLaw],
        GameplayDomain::Districts => &[
            GameplayCommandKind::StartPublicWork,
            GameplayCommandKind::RespondToCrisis,
            GameplayCommandKind::ResolveLaborDispute,
        ],
        GameplayDomain::Legal => &[GameplayCommandKind::FileLegalCase],
        GameplayDomain::Crises => &[GameplayCommandKind::RespondToCrisis],
        GameplayDomain::Information => &[GameplayCommandKind::AcknowledgeNotification],
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
    let share = u64::from(stats.executed).saturating_mul(100) / aggregate.substantive_actions;
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
        let share = non_operational.saturating_mul(100) / campaigns.len();
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
    let stressed_share = fully_stressed.saturating_mul(100) / campaigns.len();
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
        aggregate.viable_command_kinds.saturating_mul(10) / opportunity_cycles;
    let multiple_share = aggregate
        .cycles_with_multiple_viable_command_kinds
        .saturating_mul(100)
        / opportunity_cycles;
    if average_kinds_tenths < 20 || multiple_share < 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Decision breadth is narrower than raw choice counts suggest".to_owned(),
            evidence: format!(
                "The average cycle exposed {} viable command families and {multiple_share}% offered at least two substantive families.",
                format_tenths(average_kinds_tenths)
            ),
        });
    }
    let blocked_share = aggregate.blocked_cycles.saturating_mul(100) / opportunity_cycles;
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
        let share = collapsed_food.saturating_mul(100) / campaigns.len();
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
    if low_food.saturating_mul(100) / campaigns.len() >= 25 && low_food > collapsed_food {
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
        .map(|campaign| u64::from(campaign.end.defaulted_loans))
        .sum();
    let repaid: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.repaid_loans))
        .sum();
    if defaults > repaid && defaults > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Credit defaults outnumber successful repayments".to_owned(),
            evidence: format!("Observed {defaults} defaulted and {repaid} repaid loans."),
        });
    }
    let disputed: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.disputed_employment))
        .sum();
    let active: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.active_employment))
        .sum();
    if disputed > active && disputed > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Labor disputes dominate active employment".to_owned(),
            evidence: format!(
                "Endpoints contained {disputed} disputed and {active} active agreements."
            ),
        });
    }
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
    if overloaded.saturating_mul(100) / campaigns.len() < 25 {
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
    if complete_capture.saturating_mul(100) / campaigns.len() >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Player captures every political office".to_owned(),
            evidence: format!(
                "{complete_capture} of {} campaigns held every available office at some point.",
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
    if overloaded.saturating_mul(100) / campaigns.len() >= 25 {
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
    if aggregate.substantive_actions > 0
        && crisis_actions.saturating_mul(100) / aggregate.substantive_actions >= 35
    {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Crisis response crowds out strategic play".to_owned(),
            evidence: format!(
                "Crisis responses accounted for {}% of executed actions.",
                crisis_actions.saturating_mul(100) / aggregate.substantive_actions
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
    render_health_summary(report, &mut output);
    render_command_table(report, &mut output);
    render_domain_table(report, &mut output);
    render_interactions(report, &mut output);
    render_rejections(report, &mut output);
    render_findings(report, &mut output);
    render_campaign_summaries(report, &mut output);
    render_trace_samples(report, &mut output);
    output
}

fn render_health_summary(report: &GameplayHarnessReport, output: &mut String) {
    let Some(first) = report.campaigns.first() else {
        return;
    };
    let mut minimum_food = (
        first.minimum_food_satisfaction,
        first.minimum_food_satisfaction,
    );
    let mut operating_businesses = (
        first.minimum_operating_businesses,
        first.minimum_operating_businesses,
    );
    let mut peak_offices = (first.maximum_offices_held, first.maximum_offices_held);
    let mut peak_unread = (
        first.maximum_unread_notifications,
        first.maximum_unread_notifications,
    );
    let mut available_offices = first.end.available_offices;
    let mut fulfilled_contracts = 0_u64;
    let mut breached_contracts = 0_u64;
    let mut repaid_loans = 0_u64;
    let mut defaulted_loans = 0_u64;
    let mut completed_works = 0_u64;
    let mut suspended_works = 0_u64;
    for campaign in &report.campaigns {
        minimum_food.0 = minimum_food.0.min(campaign.minimum_food_satisfaction);
        minimum_food.1 = minimum_food.1.max(campaign.minimum_food_satisfaction);
        operating_businesses.0 = operating_businesses
            .0
            .min(campaign.minimum_operating_businesses);
        operating_businesses.1 = operating_businesses
            .1
            .max(campaign.minimum_operating_businesses);
        peak_offices.0 = peak_offices.0.min(campaign.maximum_offices_held);
        peak_offices.1 = peak_offices.1.max(campaign.maximum_offices_held);
        peak_unread.0 = peak_unread.0.min(campaign.maximum_unread_notifications);
        peak_unread.1 = peak_unread.1.max(campaign.maximum_unread_notifications);
        available_offices = available_offices.max(campaign.end.available_offices);
        fulfilled_contracts =
            fulfilled_contracts.saturating_add(u64::from(campaign.end.player_fulfilled_contracts));
        breached_contracts =
            breached_contracts.saturating_add(u64::from(campaign.end.player_breached_contracts));
        repaid_loans = repaid_loans.saturating_add(u64::from(campaign.end.repaid_loans));
        defaulted_loans = defaulted_loans.saturating_add(u64::from(campaign.end.defaulted_loans));
        completed_works =
            completed_works.saturating_add(u64::from(campaign.end.completed_public_works));
        suspended_works =
            suspended_works.saturating_add(u64::from(campaign.end.suspended_public_works));
    }
    let _ = writeln!(output, "Experience health");
    let _ = writeln!(
        output,
        "  trajectory ranges: food {:.2}-{:.2}% | operating businesses {}-{} | peak offices {}-{}/{} | peak unread {}-{}",
        f64::from(minimum_food.0) / 100.0,
        f64::from(minimum_food.1) / 100.0,
        operating_businesses.0,
        operating_businesses.1,
        peak_offices.0,
        peak_offices.1,
        available_offices,
        peak_unread.0,
        peak_unread.1
    );
    let _ = writeln!(
        output,
        "  outcomes: player contracts {fulfilled_contracts} fulfilled / {breached_contracts} breached | loans {repaid_loans} repaid / {defaulted_loans} defaulted | public works {completed_works} completed / {suspended_works} suspended\n"
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
        "coverage: {}/{} command kinds | causal domains {}/{} | ambient domains {}/{} | {} command-domain edges | {} quiet / {} blocked cycles",
        aggregate.command_coverage,
        ALL_COMMAND_KINDS.len(),
        aggregate.causal_domain_coverage,
        ALL_DOMAINS.len(),
        aggregate.ambient_domain_coverage,
        ALL_DOMAINS.len(),
        aggregate.interactions.len(),
        aggregate.quiet_cycles,
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
    let average_families = format_tenths(average_families_tenths);
    let _ = writeln!(
        output,
        "choice quality: {average_families} viable command families/cycle | {} cycles with multiple families\n",
        aggregate.cycles_with_multiple_viable_command_kinds
    );
}

fn render_command_table(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Command coverage");
    let _ = writeln!(
        output,
        "  {:<20} {:>7} {:>9} {:>8} {:>8} {:>8} {:>8} {:>10} {:>8} {:>8}",
        "command",
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
            "  {:<20} {:>7} {:>9} {:>8} {:>8} {:>8} {:>8} {:>10} {:>8} {:>8}",
            kind.label(),
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
            "  seed {} {:<12} {:?} day {:>4}: {:<18} immediate [{}] delayed [{}] ambient [{}]",
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
            step.day,
            command,
            domain_labels(&step.immediate_domains),
            domain_labels(&step.delayed_domains),
            domain_labels(&step.ambient_domains)
        );
    }
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

fn compare_economy_and_business(
    earlier: &GameplaySnapshot,
    later: &GameplaySnapshot,
    domains: &mut BTreeSet<GameplayDomain>,
) {
    if earlier.player_treasury != later.player_treasury
        || earlier.player_business_cash != later.player_business_cash
    {
        domains.insert(GameplayDomain::Economy);
    }
    if earlier.active_businesses != later.active_businesses
        || earlier.distressed_businesses != later.distressed_businesses
        || earlier.insolvent_businesses != later.insolvent_businesses
        || earlier.average_business_condition != later.average_business_condition
        || earlier.average_business_quality != later.average_business_quality
        || earlier.business_policy_checksum != later.business_policy_checksum
    {
        domains.insert(GameplayDomain::Business);
    }
    if earlier.market_price_total != later.market_price_total
        || earlier.market_stock_total != later.market_stock_total
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
    {
        domains.insert(GameplayDomain::Contracts);
    }
    if earlier.current_loans != later.current_loans
        || earlier.delinquent_loans != later.delinquent_loans
        || earlier.defaulted_loans != later.defaulted_loans
        || earlier.repaid_loans != later.repaid_loans
        || earlier.total_loan_balance != later.total_loan_balance
    {
        domains.insert(GameplayDomain::Loans);
    }
    if earlier.player_properties != later.player_properties
        || earlier.occupied_properties != later.occupied_properties
    {
        domains.insert(GameplayDomain::Property);
    }
    if earlier.active_employment != later.active_employment
        || earlier.disputed_employment != later.disputed_employment
        || earlier.average_labor_loyalty != later.average_labor_loyalty
    {
        domains.insert(GameplayDomain::Labor);
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
        || earlier.generation != later.generation
    {
        domains.insert(GameplayDomain::Dynasty);
    }
    if earlier.family_unity != later.family_unity
        || earlier.family_charter_version != later.family_charter_version
    {
        domains.insert(GameplayDomain::Family);
    }
    if earlier.offices_held != later.offices_held
        || earlier.institution_memberships != later.institution_memberships
        || earlier.institution_budget_total != later.institution_budget_total
    {
        domains.insert(GameplayDomain::Institutions);
    }
    if earlier.active_laws != later.active_laws
        || earlier.law_value_checksum != later.law_value_checksum
    {
        domains.insert(GameplayDomain::Law);
    }
    if earlier.average_food_satisfaction != later.average_food_satisfaction
        || earlier.average_district_unrest != later.average_district_unrest
        || earlier.public_work_progress_total != later.public_work_progress_total
        || earlier.building_public_works != later.building_public_works
        || earlier.completed_public_works != later.completed_public_works
        || earlier.suspended_public_works != later.suspended_public_works
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
    {
        domains.insert(GameplayDomain::Legal);
    }
    if earlier.active_crises != later.active_crises
        || earlier.escalated_crises != later.escalated_crises
        || earlier.resolved_crises != later.resolved_crises
        || earlier.crisis_severity_total != later.crisis_severity_total
    {
        domains.insert(GameplayDomain::Crises);
    }
    if earlier.information_reports != later.information_reports
        || earlier.achieved_ai_objectives != later.achieved_ai_objectives
        || earlier.unread_notifications != later.unread_notifications
        || earlier.outbox_messages != later.outbox_messages
        || earlier.chronicle_entries != later.chronicle_entries
    {
        domains.insert(GameplayDomain::Information);
    }
}

fn initialized_command_stats() -> BTreeMap<GameplayCommandKind, GameplayCommandStats> {
    ALL_COMMAND_KINDS
        .into_iter()
        .map(|kind| (kind, GameplayCommandStats::default()))
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

fn ratio_score(numerator: u32, denominator: u32) -> u16 {
    if denominator == 0 {
        return 0;
    }
    u16::try_from(numerator.saturating_mul(100) / denominator)
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
