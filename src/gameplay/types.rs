//! Gameplay report schema — the machine contract between harness and consumers.
//!
//! Purpose: own every serialized type (`GameplayHarnessReport`,
//! `GameplayCampaignReport`, `GameplayAggregate`, `GameplaySnapshot`,
//! `GameplayTraceStep`, `GameplayFinding`, etc.) and its
//! `GAMEPLAY_REPORT_SCHEMA_VERSION` so reports are reproducible and
//! versioned.
//! Owns: `GameplayCommandKind` (32), `GameplayDomain` (17),
//! `GameplayHarnessConfig`, all projection/score/finding/snapshot structs,
//! and `serde(deny_unknown_fields)` schemas.
//! Reads: nothing at definition time (populated by harness).
//! Mutates: nothing (pure data definitions).
//! Does not own: orchestration or presentation; consumers are CLI and
//! `scripts/check_gameplay.py`.
//! Invariants: exhaustive `ALL_COMMAND_KINDS`/`ALL_DOMAINS`; schema version
//! bumps on shape change; every trace step carries phase + window context.
//! Focused tests: `src/gameplay_tests.rs` catalog exhaustiveness.

#[allow(clippy::wildcard_imports)] // the module tree re-exports one flat namespace
use super::*;

/// Exhaustive catalog of player command families as observed by the harness.
///
/// `ALL_COMMAND_KINDS` enumerates every variant; tests assert coverage so a new
/// `PlayerCommand` variant cannot silently bypass harness generation, viability,
/// and consequence attribution. Operational vs substantive split (`is_substantive`)
/// keeps liquidity plumbing from inflating agency metrics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GameplayCommandKind {
    TransferBusinessCash,
    WithdrawBusinessCash,
    AcquireBusiness,
    InvestInBusiness,
    SetBusinessPolicy,
    SetBusinessWages,
    SecureSupply,
    SellOutput,
    BorrowFunds,
    ExtendCredit,
    BuyProperty,
    SellProperty,
    EnactLaw,
    StartPublicWork,
    FundPublicWork,
    FileLegalCase,
    SettleLegalCase,
    SetHouseGovernance,
    ConveneFamilyCouncil,
    DesignateHeir,
    AdoptWard,
    EducateFamilyMember,
    CultivateInstitutionSupport,
    EndowInstitution,
    NominateForOffice,
    ExerciseOfficePower,
    WithdrawFromInstitution,
    RespondToCrisis,
    ResolveLaborDispute,
    CommissionInformation,
    LeverageInformation,
    AcknowledgeNotification,
}

/// Returns whether a command represents a strategic player commitment.
///
/// Cash rebalancing keeps a portfolio alive, but it is operational support
/// rather than a meaningful change in dynasty direction.  Keeping it out of
/// substantive-action metrics prevents routine liquidity plumbing from
/// disguising a narrow decision loop.
pub(crate) const fn is_substantive_command_kind(kind: GameplayCommandKind) -> bool {
    !matches!(
        kind,
        GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::WithdrawBusinessCash
            | GameplayCommandKind::AcknowledgeNotification
    )
}

impl GameplayCommandKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::TransferBusinessCash => "transfer-cash",
            Self::WithdrawBusinessCash => "withdraw-cash",
            Self::AcquireBusiness => "acquire-business",
            Self::InvestInBusiness => "invest-business",
            Self::SetBusinessPolicy => "set-policy",
            Self::SetBusinessWages => "set-wages",
            Self::SecureSupply => "secure-supply",
            Self::SellOutput => "sell-output",
            Self::BorrowFunds => "borrow-funds",
            Self::ExtendCredit => "extend-credit",
            Self::BuyProperty => "buy-property",
            Self::SellProperty => "sell-property",
            Self::EnactLaw => "enact-law",
            Self::StartPublicWork => "public-work",
            Self::FundPublicWork => "public-work-funding",
            Self::FileLegalCase => "legal-case",
            Self::SettleLegalCase => "legal-settlement",
            Self::SetHouseGovernance => "house-governance",
            Self::ConveneFamilyCouncil => "family-council",
            Self::DesignateHeir => "designate-heir",
            Self::AdoptWard => "adopt-ward",
            Self::EducateFamilyMember => "family-education",
            Self::CultivateInstitutionSupport => "institution-support",
            Self::EndowInstitution => "institution-endowment",
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
}

/// Exhaustive domain attribution set for counterfactual analysis.
///
/// Every command-to-state edge maps to one `GameplayDomain`; `ALL_DOMAINS`
/// catalog and exhaustive match tests guarantee no state component is
/// silently unobserved. `Feedback` covers outbox/chronicle/ audit observation
/// so probe-vs-baseline attribution is complete.
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
    /// For `ExtendCredit`, the accepted loan immediately reached a business
    /// record rather than remaining only in the borrower's dynasty treasury.
    pub productive_financing_actions: u32,
    /// For `ExtendCredit`, the accepted loan did not immediately reach a
    /// business record and therefore needs separate interpretation.
    pub nonproductive_financing_actions: u32,
    /// For `ExtendCredit`, an existing default was worked back onto revised
    /// terms without advancing new principal. Workouts are recovery actions,
    /// not new financing, so they are measured separately from treasury-only
    /// advances.
    pub financing_workout_actions: u32,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayDistrictCondition {
    pub district_id: DistrictId,
    pub employment_basis_points: u16,
    pub sanitation_basis_points: u16,
    pub safety_basis_points: u16,
    pub unrest_basis_points: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplaySnapshot {
    pub day: i64,
    pub dynasty_state_checksum: u64,
    pub player_treasury: Money,
    pub player_civic_contributions: Money,
    pub player_unmet_office_duties: u32,
    pub player_business_cash: Money,
    pub player_business_lifetime_revenue: Money,
    pub player_business_lifetime_costs: Money,
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
    pub player_breach_victim_contracts: u16,
    pub player_contract_failures: u32,
    pub player_contract_deliveries: u32,
    pub contract_state_checksum: u64,
    pub current_loans: u16,
    pub delinquent_loans: u16,
    pub restructured_loans: u16,
    pub defaulted_loans: u16,
    pub repaid_loans: u16,
    pub written_off_loans: u16,
    pub player_current_lending: u16,
    pub player_delinquent_lending: u16,
    pub player_restructured_lending: u16,
    pub player_defaulted_lending: u16,
    pub player_repaid_lending: u16,
    pub player_written_off_lending: u16,
    pub player_current_borrowing: u16,
    pub player_delinquent_borrowing: u16,
    pub player_restructured_borrowing: u16,
    pub player_defaulted_borrowing: u16,
    pub player_repaid_borrowing: u16,
    pub player_written_off_borrowing: u16,
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
    pub minimum_unowned_property_value: Option<Money>,
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
    pub active_law_kinds: Vec<LawKind>,
    pub law_value_checksum: i64,
    pub active_law_checksum: i64,
    pub law_state_checksum: u64,
    pub public_work_progress_total: u32,
    pub building_public_works: u16,
    pub completed_public_works: u16,
    pub suspended_public_works: u16,
    pub player_completed_public_work_kinds: BTreeSet<PublicWorkKind>,
    pub player_completed_public_work_checksum: i64,
    pub public_work_state_checksum: u64,
    pub average_food_satisfaction: u16,
    pub minimum_district_food_satisfaction: u16,
    pub average_district_unrest: u16,
    pub average_district_employment: u16,
    pub average_district_sanitation: u16,
    pub average_district_safety: u16,
    pub district_conditions: Vec<GameplayDistrictCondition>,
    pub district_state_checksum: u64,
    pub open_legal_cases: u16,
    pub player_open_legal_cases_as_defendant: u16,
    pub decided_legal_cases: u16,
    /// Contracts city-wide that ever recorded an attributed breach victim,
    /// whether or not the penalty has since been discharged.
    pub attributed_breach_contracts: u16,
    /// Every legal case ever filed in the city, in any status. Cases are
    /// retained, so this is cumulative filing volume rather than a stock.
    pub legal_cases_filed_total: u16,
    pub maximum_route_disruption_basis_points: u16,
    pub city_distressed_businesses: u16,
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

pub(crate) fn count_player_open_legal_cases_as_defendant(state: &AppState) -> u16 {
    usize_to_u16(
        state
            .legal_cases
            .values()
            .filter(|case| {
                case.defendant_dynasty_id == state.player_dynasty_id
                    && matches!(
                        case.status,
                        LegalCaseStatus::Filed | LegalCaseStatus::Hearing
                    )
            })
            .count(),
    )
}

#[derive(Debug)]
pub(crate) struct BusinessSnapshotPart {
    pub player_treasury: Money,
    pub player_civic_contributions: Money,
    pub player_unmet_office_duties: u32,
    pub player_business_cash: Money,
    pub player_business_lifetime_revenue: Money,
    pub player_business_lifetime_costs: Money,
    pub active_businesses: u16,
    pub distressed_businesses: u16,
    pub insolvent_businesses: u16,
    pub average_business_condition: u16,
    pub average_business_quality: u16,
    pub business_policy_checksum: i64,
    pub market_price_total: Money,
    pub market_stock_total: Quantity,
}

impl BusinessSnapshotPart {
    pub fn capture(state: &AppState, player_id: DynastyId) -> Self {
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
            player_business_lifetime_revenue: businesses
                .iter()
                .fold(Money::ZERO, |total, business| {
                    total.saturating_add(business.finance.lifetime_revenue)
                }),
            player_business_lifetime_costs: businesses
                .iter()
                .fold(Money::ZERO, |total, business| {
                    total.saturating_add(business.finance.lifetime_costs)
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
pub(crate) struct StrategicSnapshotPart {
    pub active_contracts: u16,
    pub fulfilled_contracts: u16,
    pub breached_contracts: u16,
    pub contract_failures: u32,
    pub player_active_contracts: u16,
    pub player_fulfilled_contracts: u16,
    pub player_breached_contracts: u16,
    pub player_breach_victim_contracts: u16,
    pub player_contract_failures: u32,
    pub player_contract_deliveries: u32,
    pub current_loans: u16,
    pub delinquent_loans: u16,
    pub restructured_loans: u16,
    pub defaulted_loans: u16,
    pub repaid_loans: u16,
    pub written_off_loans: u16,
    pub player_current_lending: u16,
    pub player_delinquent_lending: u16,
    pub player_restructured_lending: u16,
    pub player_defaulted_lending: u16,
    pub player_repaid_lending: u16,
    pub player_written_off_lending: u16,
    pub player_current_borrowing: u16,
    pub player_delinquent_borrowing: u16,
    pub player_restructured_borrowing: u16,
    pub player_defaulted_borrowing: u16,
    pub player_repaid_borrowing: u16,
    pub player_written_off_borrowing: u16,
    pub total_loan_balance: Money,
    pub civic_debt: CivicDebtSnapshotPart,
    pub player_properties: u16,
    pub player_pledged_properties: u16,
    pub player_collateral_balance: Money,
    pub occupied_properties: u16,
    pub minimum_unowned_property_value: Option<Money>,
    pub active_employment: u16,
    pub disputed_employment: u16,
    pub player_active_employment: u16,
    pub player_disputed_employment: u16,
    pub average_labor_loyalty: u16,
    pub average_relationship_trust: u16,
    pub average_relationship_respect: u16,
    pub average_relationship_fear: u16,
    pub average_relationship_resentment: u16,
    pub maximum_contract_relationship_pressure_basis_points: u16,
    pub relationship_obligation_total: i64,
    pub relationship_memory_count: u16,
}

#[derive(Debug)]
pub(crate) struct LoanSnapshotPart {
    pub current: u16,
    pub delinquent: u16,
    pub restructured: u16,
    pub defaulted: u16,
    pub repaid: u16,
    pub written_off: u16,
    pub player_current: u16,
    pub player_delinquent: u16,
    pub player_restructured: u16,
    pub player_defaulted: u16,
    pub player_repaid: u16,
    pub player_written_off: u16,
    pub player_borrowing_current: u16,
    pub player_borrowing_delinquent: u16,
    pub player_borrowing_restructured: u16,
    pub player_borrowing_defaulted: u16,
    pub player_borrowing_repaid: u16,
    pub player_borrowing_written_off: u16,
    pub total_balance: Money,
}

impl LoanSnapshotPart {
    pub fn capture(state: &AppState, player_id: DynastyId) -> Self {
        Self {
            current: count_loan_status(state, LoanStatus::Current),
            delinquent: count_loan_status(state, LoanStatus::Delinquent),
            restructured: count_loan_status(state, LoanStatus::Restructured),
            defaulted: count_loan_status(state, LoanStatus::Defaulted),
            repaid: count_loan_status(state, LoanStatus::Repaid),
            written_off: count_loan_status(state, LoanStatus::WrittenOff),
            player_current: count_player_lending_status(state, player_id, LoanStatus::Current),
            player_delinquent: count_player_lending_status(
                state,
                player_id,
                LoanStatus::Delinquent,
            ),
            player_restructured: count_player_lending_status(
                state,
                player_id,
                LoanStatus::Restructured,
            ),
            player_defaulted: count_player_lending_status(state, player_id, LoanStatus::Defaulted),
            player_repaid: count_player_lending_status(state, player_id, LoanStatus::Repaid),
            player_written_off: count_player_lending_status(
                state,
                player_id,
                LoanStatus::WrittenOff,
            ),
            player_borrowing_current: count_player_borrowing_status(
                state,
                player_id,
                LoanStatus::Current,
            ),
            player_borrowing_delinquent: count_player_borrowing_status(
                state,
                player_id,
                LoanStatus::Delinquent,
            ),
            player_borrowing_restructured: count_player_borrowing_status(
                state,
                player_id,
                LoanStatus::Restructured,
            ),
            player_borrowing_defaulted: count_player_borrowing_status(
                state,
                player_id,
                LoanStatus::Defaulted,
            ),
            player_borrowing_repaid: count_player_borrowing_status(
                state,
                player_id,
                LoanStatus::Repaid,
            ),
            player_borrowing_written_off: count_player_borrowing_status(
                state,
                player_id,
                LoanStatus::WrittenOff,
            ),
            total_balance: state.loans.values().fold(Money::ZERO, |total, loan| {
                total.saturating_add(loan.balance)
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PropertySnapshotPart {
    pub player_properties: u16,
    pub player_pledged_properties: u16,
    pub player_collateral_balance: Money,
    pub occupied_properties: u16,
    pub minimum_unowned_property_value: Option<Money>,
}

impl PropertySnapshotPart {
    pub fn capture(state: &AppState, player_id: DynastyId) -> Self {
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
            minimum_unowned_property_value: state
                .properties
                .values()
                .filter(|property| property.owner_dynasty_id.is_none())
                .map(|property| property.value)
                .min(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct CivicDebtSnapshotPart {
    pub current: u16,
    pub delinquent: u16,
    pub defaulted: u16,
    pub repaid: u16,
    pub total_balance: Money,
}

impl CivicDebtSnapshotPart {
    #[must_use]
    pub fn capture(state: &AppState) -> Self {
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
pub(crate) struct RelationshipSnapshotPart {
    pub average_trust: u16,
    pub average_respect: u16,
    pub average_fear: u16,
    pub average_resentment: u16,
    pub maximum_contract_pressure: u16,
    pub obligation_total: i64,
    pub memory_count: u16,
}

impl RelationshipSnapshotPart {
    pub fn capture(state: &AppState, player_id: DynastyId) -> Self {
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
    #[allow(clippy::too_many_lines)]
    pub fn capture(state: &AppState, player_id: DynastyId) -> Self {
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
        let loans = LoanSnapshotPart::capture(state, player_id);
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
            player_breach_victim_contracts: usize_to_u16(
                state
                    .contracts
                    .values()
                    .filter(|contract| contract.breach_victim_dynasty_id == Some(player_id))
                    .count(),
            ),
            player_contract_failures: player_contracts()
                .map(|contract| u32::from(contract.missed_deliveries))
                .sum(),
            player_contract_deliveries: player_contract_deliveries(state),
            current_loans: loans.current,
            delinquent_loans: loans.delinquent,
            restructured_loans: loans.restructured,
            defaulted_loans: loans.defaulted,
            repaid_loans: loans.repaid,
            written_off_loans: loans.written_off,
            player_current_lending: loans.player_current,
            player_delinquent_lending: loans.player_delinquent,
            player_restructured_lending: loans.player_restructured,
            player_defaulted_lending: loans.player_defaulted,
            player_repaid_lending: loans.player_repaid,
            player_written_off_lending: loans.player_written_off,
            player_current_borrowing: loans.player_borrowing_current,
            player_delinquent_borrowing: loans.player_borrowing_delinquent,
            player_restructured_borrowing: loans.player_borrowing_restructured,
            player_defaulted_borrowing: loans.player_borrowing_defaulted,
            player_repaid_borrowing: loans.player_borrowing_repaid,
            player_written_off_borrowing: loans.player_borrowing_written_off,
            total_loan_balance: loans.total_balance,
            civic_debt: CivicDebtSnapshotPart::capture(state),
            player_properties: properties.player_properties,
            player_pledged_properties: properties.player_pledged_properties,
            player_collateral_balance: properties.player_collateral_balance,
            occupied_properties: properties.occupied_properties,
            minimum_unowned_property_value: properties.minimum_unowned_property_value,
            active_employment: count_employment_status(state, EmploymentStatus::Active),
            disputed_employment: count_employment_status(state, EmploymentStatus::Disputed),
            player_active_employment: count_player_employment_status(
                state,
                &player_business_ids,
                EmploymentStatus::Active,
            ),
            player_disputed_employment: count_player_employment_status(
                state,
                &player_business_ids,
                EmploymentStatus::Disputed,
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

pub(crate) fn count_player_employment_status(
    state: &AppState,
    player_business_ids: &BTreeSet<BusinessId>,
    status: EmploymentStatus,
) -> u16 {
    usize_to_u16(
        state
            .employment
            .values()
            .filter(|agreement| {
                player_business_ids.contains(&agreement.business_id) && agreement.status == status
            })
            .count(),
    )
}

#[derive(Debug)]
pub(crate) struct CivicSnapshotPart {
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
    pub player_office_checksum: i64,
    pub institution_memberships: u16,
    pub player_institutions_represented: u16,
    pub institution_budget_total: Money,
    pub active_laws: u16,
    pub active_law_kinds: Vec<LawKind>,
    pub law_value_checksum: i64,
    pub active_law_checksum: i64,
    pub public_work_progress_total: u32,
    pub building_public_works: u16,
    pub completed_public_works: u16,
    pub suspended_public_works: u16,
    pub player_completed_public_work_kinds: BTreeSet<PublicWorkKind>,
    pub player_completed_public_work_checksum: i64,
}

#[derive(Debug)]
pub(crate) struct LawSnapshotPart {
    pub active: u16,
    pub kinds: Vec<LawKind>,
    pub value_checksum: i64,
    pub checksum: i64,
}

impl LawSnapshotPart {
    #[must_use]
    pub fn capture(state: &AppState) -> Self {
        let active = || state.laws.values().filter(|law| law.active);
        Self {
            active: usize_to_u16(active().count()),
            kinds: active().map(|law| law.kind).collect(),
            value_checksum: active().map(|law| law.value).sum(),
            checksum: active().fold(0_i64, |total, law| {
                total
                    .saturating_add((law.kind as i64).saturating_mul(10_007))
                    .saturating_add(law.value)
            }),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PublicWorkSnapshotPart {
    pub progress_total: u32,
    pub building: u16,
    pub completed: u16,
    pub suspended: u16,
    pub player_completed_kinds: BTreeSet<PublicWorkKind>,
    pub player_completed_checksum: i64,
}

impl PublicWorkSnapshotPart {
    pub fn capture(state: &AppState, player_id: DynastyId) -> Self {
        let player_completed = || {
            state.public_works.values().filter(|work| {
                work.sponsor_dynasty_id == Some(player_id)
                    && work.status == PublicWorkStatus::Completed
            })
        };
        Self {
            progress_total: state
                .public_works
                .values()
                .map(|work| u32::from(work.progress_basis_points))
                .sum(),
            building: count_public_work_status(state, PublicWorkStatus::Building),
            completed: count_public_work_status(state, PublicWorkStatus::Completed),
            suspended: count_public_work_status(state, PublicWorkStatus::Suspended),
            player_completed_kinds: player_completed().map(|work| work.kind).collect(),
            player_completed_checksum: player_completed().fold(0_i64, |total, work| {
                total
                    .saturating_add(i64::from(work.district_id.value()).saturating_mul(101))
                    .saturating_add(work.kind as i64)
            }),
        }
    }
}

pub(crate) fn count_public_work_status(state: &AppState, status: PublicWorkStatus) -> u16 {
    usize_to_u16(
        state
            .public_works
            .values()
            .filter(|work| work.status == status)
            .count(),
    )
}

impl CivicSnapshotPart {
    pub fn capture(state: &AppState, player_id: DynastyId) -> Self {
        let player = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist");
        let council = state
            .family_councils
            .get(&player_id)
            .expect("player family council must exist");
        let laws = LawSnapshotPart::capture(state);
        let public_works = PublicWorkSnapshotPart::capture(state, player_id);
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
            active_wards: usize_to_u16(active_player_ward_count(state)),
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
            active_laws: laws.active,
            active_law_kinds: laws.kinds,
            law_value_checksum: laws.value_checksum,
            active_law_checksum: laws.checksum,
            public_work_progress_total: public_works.progress_total,
            building_public_works: public_works.building,
            completed_public_works: public_works.completed,
            suspended_public_works: public_works.suspended,
            player_completed_public_work_kinds: public_works.player_completed_kinds,
            player_completed_public_work_checksum: public_works.player_completed_checksum,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DistrictConditionSnapshot {
    pub employment: u16,
    pub sanitation: u16,
    pub safety: u16,
    pub unrest: u16,
    pub conditions: Vec<GameplayDistrictCondition>,
}

impl DistrictConditionSnapshot {
    #[must_use]
    pub fn capture(state: &AppState) -> Self {
        let conditions: Vec<_> = state
            .districts
            .values()
            .map(|district| GameplayDistrictCondition {
                district_id: district.district_id,
                employment_basis_points: district.employment_basis_points,
                sanitation_basis_points: district.sanitation_basis_points,
                safety_basis_points: district.safety_basis_points,
                unrest_basis_points: district.unrest_basis_points,
            })
            .collect();
        Self {
            employment: average_u16(
                conditions
                    .iter()
                    .map(|district| district.employment_basis_points),
            ),
            sanitation: average_u16(
                conditions
                    .iter()
                    .map(|district| district.sanitation_basis_points),
            ),
            safety: average_u16(
                conditions
                    .iter()
                    .map(|district| district.safety_basis_points),
            ),
            unrest: average_u16(
                conditions
                    .iter()
                    .map(|district| district.unrest_basis_points),
            ),
            conditions,
        }
    }
}

pub(crate) fn count_open_legal_cases(state: &AppState) -> u16 {
    usize_to_u16(
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
    )
}

pub(crate) fn count_decided_legal_cases(state: &AppState) -> u16 {
    usize_to_u16(
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
    )
}

#[derive(Debug)]
pub(crate) struct WorldSnapshotPart {
    pub average_food_satisfaction: u16,
    pub minimum_district_food_satisfaction: u16,
    pub average_district_unrest: u16,
    pub average_district_employment: u16,
    pub average_district_sanitation: u16,
    pub average_district_safety: u16,
    pub district_conditions: Vec<GameplayDistrictCondition>,
    pub open_legal_cases: u16,
    pub player_open_legal_cases_as_defendant: u16,
    pub decided_legal_cases: u16,
    pub attributed_breach_contracts: u16,
    pub legal_cases_filed_total: u16,
    pub maximum_route_disruption_basis_points: u16,
    pub city_distressed_businesses: u16,
    pub active_crises: u16,
    pub escalated_crises: u16,
    pub resolved_crises: u16,
    pub crisis_severity_total: u32,
    pub information_reports: u16,
    pub information_report_checksum: i64,
    pub achieved_ai_objectives: u16,
    pub unread_notifications: u16,
    pub outbox_messages: u32,
    pub chronicle_entries: u32,
}

impl WorldSnapshotPart {
    #[must_use]
    pub fn capture(state: &AppState) -> Self {
        let district = DistrictConditionSnapshot::capture(state);
        Self {
            average_food_satisfaction:
                crate::core::population_weighted_food_satisfaction_basis_points(
                    state.households.iter(),
                )
                .unwrap_or(crate::core::NEUTRAL_FOOD_SATISFACTION_BASIS_POINTS),
            minimum_district_food_satisfaction: minimum_district_food_satisfaction(state),
            average_district_unrest: district.unrest,
            average_district_employment: district.employment,
            average_district_sanitation: district.sanitation,
            average_district_safety: district.safety,
            district_conditions: district.conditions,
            open_legal_cases: count_open_legal_cases(state),
            player_open_legal_cases_as_defendant: count_player_open_legal_cases_as_defendant(state),
            decided_legal_cases: count_decided_legal_cases(state),
            attributed_breach_contracts: usize_to_u16(
                state
                    .contracts
                    .values()
                    .filter(|contract| contract.breach_victim_dynasty_id.is_some())
                    .count(),
            ),
            legal_cases_filed_total: usize_to_u16(state.legal_cases.len()),
            maximum_route_disruption_basis_points: state
                .external_routes
                .values()
                .map(|route| route.disruption_basis_points)
                .max()
                .unwrap_or(0),
            city_distressed_businesses: usize_to_u16(
                state
                    .businesses
                    .iter()
                    .filter(|business| business.status() == BusinessStatus::Distressed)
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

pub(crate) fn minimum_district_food_satisfaction(state: &AppState) -> u16 {
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
            player_business_lifetime_revenue: $business.player_business_lifetime_revenue,
            player_business_lifetime_costs: $business.player_business_lifetime_costs,
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
            player_breach_victim_contracts: $strategic.player_breach_victim_contracts,
            player_contract_failures: $strategic.player_contract_failures,
            player_contract_deliveries: $strategic.player_contract_deliveries,
            contract_state_checksum: stable_serialized_checksum(&$state.contracts),
            current_loans: $strategic.current_loans,
            delinquent_loans: $strategic.delinquent_loans,
            restructured_loans: $strategic.restructured_loans,
            defaulted_loans: $strategic.defaulted_loans,
            repaid_loans: $strategic.repaid_loans,
            written_off_loans: $strategic.written_off_loans,
            player_current_lending: $strategic.player_current_lending,
            player_delinquent_lending: $strategic.player_delinquent_lending,
            player_restructured_lending: $strategic.player_restructured_lending,
            player_defaulted_lending: $strategic.player_defaulted_lending,
            player_repaid_lending: $strategic.player_repaid_lending,
            player_written_off_lending: $strategic.player_written_off_lending,
            player_current_borrowing: $strategic.player_current_borrowing,
            player_delinquent_borrowing: $strategic.player_delinquent_borrowing,
            player_restructured_borrowing: $strategic.player_restructured_borrowing,
            player_defaulted_borrowing: $strategic.player_defaulted_borrowing,
            player_repaid_borrowing: $strategic.player_repaid_borrowing,
            player_written_off_borrowing: $strategic.player_written_off_borrowing,
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
            minimum_unowned_property_value: $strategic.minimum_unowned_property_value,
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
            active_law_kinds: $civic.active_law_kinds,
            law_value_checksum: $civic.law_value_checksum,
            active_law_checksum: $civic.active_law_checksum,
            law_state_checksum: stable_serialized_checksum(&$state.laws),
            public_work_progress_total: $civic.public_work_progress_total,
            building_public_works: $civic.building_public_works,
            completed_public_works: $civic.completed_public_works,
            suspended_public_works: $civic.suspended_public_works,
            player_completed_public_work_kinds: $civic.player_completed_public_work_kinds,
            player_completed_public_work_checksum: $civic.player_completed_public_work_checksum,
            public_work_state_checksum: stable_serialized_checksum(&$state.public_works),
            average_food_satisfaction: $world.average_food_satisfaction,
            minimum_district_food_satisfaction: $world.minimum_district_food_satisfaction,
            average_district_unrest: $world.average_district_unrest,
            average_district_employment: $world.average_district_employment,
            average_district_sanitation: $world.average_district_sanitation,
            average_district_safety: $world.average_district_safety,
            district_conditions: $world.district_conditions,
            district_state_checksum: stable_serialized_checksum(&$state.districts),
            open_legal_cases: $world.open_legal_cases,
            player_open_legal_cases_as_defendant: $world.player_open_legal_cases_as_defendant,
            decided_legal_cases: $world.decided_legal_cases,
            attributed_breach_contracts: $world.attributed_breach_contracts,
            legal_cases_filed_total: $world.legal_cases_filed_total,
            maximum_route_disruption_basis_points: $world.maximum_route_disruption_basis_points,
            city_distressed_businesses: $world.city_distressed_businesses,
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
            outbox_state_checksum: $state.outbox.structural_checksum(),
            chronicle_state_checksum: $state.chronicle.structural_checksum(),
            audit_state_checksum: $state.audit_log.structural_checksum(),
        }
    };
}

impl GameplaySnapshot {
    #[must_use]
    pub fn capture(state: &AppState) -> Self {
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
    pub player_delinquent_lending: u16,
    pub player_defaulted_lending: u16,
    pub player_delinquent_borrowing: u16,
    pub player_defaulted_borrowing: u16,
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
    pub average_district_employment: u16,
    pub average_district_sanitation: u16,
    pub average_district_safety: u16,
    pub average_district_unrest: u16,
    pub active_wards: u16,
    pub family_unity: u16,
    pub generation: u16,
    pub player_disputed_employment: u16,
    pub maximum_contract_relationship_pressure_basis_points: u16,
    pub player_open_legal_cases_as_defendant: u16,
    pub player_breach_victim_contracts: u16,
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
            player_delinquent_lending: snapshot.player_delinquent_lending,
            player_defaulted_lending: snapshot.player_defaulted_lending,
            player_delinquent_borrowing: snapshot.player_delinquent_borrowing,
            player_defaulted_borrowing: snapshot.player_defaulted_borrowing,
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
            average_district_employment: snapshot.average_district_employment,
            average_district_sanitation: snapshot.average_district_sanitation,
            average_district_safety: snapshot.average_district_safety,
            average_district_unrest: snapshot.average_district_unrest,
            active_wards: snapshot.active_wards,
            family_unity: snapshot.family_unity,
            generation: snapshot.generation,
            player_disputed_employment: snapshot.player_disputed_employment,
            maximum_contract_relationship_pressure_basis_points: snapshot
                .maximum_contract_relationship_pressure_basis_points,
            player_open_legal_cases_as_defendant: snapshot.player_open_legal_cases_as_defendant,
            player_breach_victim_contracts: snapshot.player_breach_victim_contracts,
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
    /// All alternatives in a decision cycle are projected over the same
    /// horizon so their delayed consequences are comparable.
    pub projected_horizon_days: u16,
    pub immediate_domains: BTreeSet<GameplayDomain>,
    pub projected_domains: BTreeSet<GameplayDomain>,
    pub immediate_history_change: bool,
    pub projected_history_change: bool,
    pub immediate_profile: GameplayConsequenceProfile,
    pub projected_profile: GameplayConsequenceProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GameplayMeasure {
    PlayerTreasury,
    PlayerBusinessCash,
    PlayerBusinessLifetimeProfit,
    ActiveBusinesses,
    DistressedBusinesses,
    PlayerActiveContracts,
    PlayerFulfilledContracts,
    PlayerBreachedContracts,
    CurrentLoans,
    PlayerCurrentLending,
    PlayerCurrentBorrowing,
    PlayerProperties,
    PlayerPledgedProperties,
    Legitimacy,
    FamilyUnity,
    OfficesHeld,
    InstitutionMemberships,
    InstitutionRepresentation,
    Generation,
    ActiveLaws,
    BuildingPublicWorks,
    CompletedPublicWorks,
    AverageFoodSatisfaction,
    AverageDistrictUnrest,
    AverageDistrictEmployment,
    AverageDistrictSanitation,
    AverageDistrictSafety,
    ActiveCrises,
    ContractRelationshipPressure,
    PlayerDisputedEmployment,
    DefaultedLoans,
    PlayerDelinquentBorrowing,
    PlayerDefaultedBorrowing,
    WrittenOffLoans,
    PlayerWrittenOffLending,
    PlayerWrittenOffBorrowing,
    UnmetOfficeDuties,
    PlayerOpenLegalCasesAsDefendant,
    InformationReports,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GameplayMeasureChange {
    pub before: i64,
    pub after: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GameplayConsequenceProfile {
    pub increases: BTreeSet<GameplayMeasure>,
    pub decreases: BTreeSet<GameplayMeasure>,
    pub changes: BTreeMap<GameplayMeasure, GameplayMeasureChange>,
    pub impact_fingerprint: u64,
    pub strategic_fingerprint: u64,
}

impl GameplayConsequenceProfile {
    #[must_use]
    pub fn between(baseline: &GameplaySnapshot, outcome: &GameplaySnapshot) -> Self {
        let mut profile = Self::default();
        macro_rules! record {
            ($measure:ident, $field:ident) => {
                record_measure_change(
                    &mut profile,
                    GameplayMeasure::$measure,
                    i64::from(baseline.$field),
                    i64::from(outcome.$field),
                );
            };
            ($measure:ident, $field:ident, money) => {
                record_measure_change(
                    &mut profile,
                    GameplayMeasure::$measure,
                    baseline.$field.copper(),
                    outcome.$field.copper(),
                );
            };
        }
        record!(PlayerTreasury, player_treasury, money);
        record!(PlayerBusinessCash, player_business_cash, money);
        // Lifetime profit is the durable commercial-engine signal: it moves
        // only when the dynasty's businesses earn more than they spend across
        // the whole campaign, so a command that merely shuffles cash between
        // treasury and firms never registers here.
        record_measure_change(
            &mut profile,
            GameplayMeasure::PlayerBusinessLifetimeProfit,
            baseline
                .player_business_lifetime_revenue
                .copper()
                .saturating_sub(baseline.player_business_lifetime_costs.copper()),
            outcome
                .player_business_lifetime_revenue
                .copper()
                .saturating_sub(outcome.player_business_lifetime_costs.copper()),
        );
        record!(ActiveBusinesses, active_businesses);
        record!(DistressedBusinesses, distressed_businesses);
        record!(PlayerActiveContracts, player_active_contracts);
        record!(PlayerFulfilledContracts, player_fulfilled_contracts);
        record!(PlayerBreachedContracts, player_breached_contracts);
        record!(CurrentLoans, current_loans);
        record!(PlayerCurrentLending, player_current_lending);
        record!(PlayerCurrentBorrowing, player_current_borrowing);
        record!(PlayerProperties, player_properties);
        record!(PlayerPledgedProperties, player_pledged_properties);
        record!(Legitimacy, legitimacy);
        record!(FamilyUnity, family_unity);
        record!(OfficesHeld, offices_held);
        record!(InstitutionMemberships, institution_memberships);
        record!(InstitutionRepresentation, player_institutions_represented);
        record!(Generation, generation);
        record!(ActiveLaws, active_laws);
        record!(BuildingPublicWorks, building_public_works);
        record!(CompletedPublicWorks, completed_public_works);
        record!(AverageFoodSatisfaction, average_food_satisfaction);
        record!(AverageDistrictUnrest, average_district_unrest);
        record!(AverageDistrictEmployment, average_district_employment);
        record!(AverageDistrictSanitation, average_district_sanitation);
        record!(AverageDistrictSafety, average_district_safety);
        record!(ActiveCrises, active_crises);
        record!(
            ContractRelationshipPressure,
            maximum_contract_relationship_pressure_basis_points
        );
        record!(PlayerDisputedEmployment, player_disputed_employment);
        record!(DefaultedLoans, defaulted_loans);
        record!(PlayerDelinquentBorrowing, player_delinquent_borrowing);
        record!(PlayerDefaultedBorrowing, player_defaulted_borrowing);
        record!(WrittenOffLoans, written_off_loans);
        record!(PlayerWrittenOffLending, player_written_off_lending);
        record!(PlayerWrittenOffBorrowing, player_written_off_borrowing);
        record!(UnmetOfficeDuties, player_unmet_office_duties);
        record!(
            PlayerOpenLegalCasesAsDefendant,
            player_open_legal_cases_as_defendant
        );
        record!(InformationReports, information_reports);
        profile.impact_fingerprint = impact_outcome_fingerprint(outcome);
        profile.strategic_fingerprint = strategic_outcome_fingerprint(outcome);
        profile
    }
}

pub(crate) fn impact_outcome_fingerprint(snapshot: &GameplaySnapshot) -> u64 {
    let money_bits = |money: Money| u64::from_ne_bytes(money.copper().to_ne_bytes());
    let signed_bits = |value: i64| u64::from_ne_bytes(value.to_ne_bytes());
    [
        money_bits(snapshot.player_treasury),
        money_bits(snapshot.player_business_cash),
        u64::from(snapshot.active_businesses),
        u64::from(snapshot.distressed_businesses),
        u64::from(snapshot.player_properties),
        u64::from(snapshot.legitimacy),
        u64::from(snapshot.family_unity),
        u64::from(snapshot.offices_held),
        u64::from(snapshot.player_institutions_represented),
        u64::from(snapshot.active_laws),
        u64::from(snapshot.completed_public_works),
        u64::from(snapshot.average_food_satisfaction),
        u64::from(snapshot.average_district_unrest),
        u64::from(snapshot.average_district_employment),
        u64::from(snapshot.average_district_sanitation),
        u64::from(snapshot.average_district_safety),
        u64::from(snapshot.active_crises),
        u64::from(snapshot.maximum_contract_relationship_pressure_basis_points),
        u64::from(snapshot.player_disputed_employment),
        u64::from(snapshot.defaulted_loans),
        u64::from(snapshot.player_delinquent_borrowing),
        u64::from(snapshot.player_defaulted_borrowing),
        u64::from(snapshot.written_off_loans),
        u64::from(snapshot.player_written_off_lending),
        u64::from(snapshot.player_written_off_borrowing),
        u64::from(snapshot.player_unmet_office_duties),
        u64::from(snapshot.player_open_legal_cases_as_defendant),
        u64::from(snapshot.information_reports),
        u64::from(snapshot.player_family_capability_checksum),
        signed_bits(snapshot.player_office_checksum),
        signed_bits(snapshot.active_law_checksum),
        signed_bits(snapshot.player_completed_public_work_checksum),
    ]
    .into_iter()
    .fold(14_695_981_039_346_656_037_u64, |fingerprint, value| {
        fingerprint.wrapping_mul(1_099_511_628_211) ^ value
    })
}

pub(crate) fn strategic_outcome_fingerprint(snapshot: &GameplaySnapshot) -> u64 {
    [
        snapshot.business_state_checksum,
        snapshot.market_state_checksum,
        snapshot.contract_state_checksum,
        snapshot.loan_state_checksum,
        snapshot.civic_debt_state_checksum,
        snapshot.property_state_checksum,
        snapshot.employment_state_checksum,
        snapshot.relationship_state_checksum,
        snapshot.character_state_checksum,
        snapshot.family_state_checksum,
        snapshot.institution_state_checksum,
        snapshot.law_state_checksum,
        snapshot.public_work_state_checksum,
        snapshot.district_state_checksum,
        snapshot.legal_case_state_checksum,
        snapshot.crisis_state_checksum,
        snapshot.information_state_checksum,
    ]
    .into_iter()
    .fold(14_695_981_039_346_656_037_u64, |fingerprint, value| {
        fingerprint.wrapping_mul(1_099_511_628_211) ^ value
    })
}

pub(crate) fn record_measure_change(
    profile: &mut GameplayConsequenceProfile,
    measure: GameplayMeasure,
    before: i64,
    after: i64,
) {
    match after.cmp(&before) {
        std::cmp::Ordering::Greater => {
            profile.increases.insert(measure);
        }
        std::cmp::Ordering::Less => {
            profile.decreases.insert(measure);
        }
        std::cmp::Ordering::Equal => {}
    }
    if before != after {
        profile
            .changes
            .insert(measure, GameplayMeasureChange { before, after });
    }
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
    pub phase: GameplayPhase,
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
    /// Why the agent took no substantive action this cycle. Presented when the
    /// world offered no viable choice, separating dormant cycles from
    /// generator-gap, spending-policy, and validation-gate causes.
    pub no_action_reason: Option<String>,
    /// Measures changed by the command at commit time.
    pub immediate_consequences: GameplayConsequenceProfile,
    /// Measures that differ from the no-action branch at the attribution horizon.
    pub attributed_consequences: GameplayConsequenceProfile,
    /// Measures that changed in the no-action branch during the same horizon.
    pub ambient_consequences: GameplayConsequenceProfile,
    /// Durable notices and chronicle entries emitted by the selected action's
    /// command path, limited to the first few stable entries for readability.
    pub command_feedback: Vec<GameplayFeedbackEvent>,
    /// Days the campaign advanced after the command commit when
    /// [`Self::simulation_feedback`] was collected. Zero on terminal cycles.
    pub simulation_window_days: u32,
    /// Durable notices and chronicle entries emitted while the campaign branch
    /// advanced to the next decision point.
    pub simulation_feedback: Vec<GameplayFeedbackEvent>,
    /// Days the no-action branch advanced when [`Self::ambient_feedback`] was
    /// collected. This is the attribution horizon for substantive cycles and
    /// the ordinary decision interval for quiet cycles, which never branch.
    pub ambient_window_days: u32,
    /// Durable notices and chronicle entries emitted by the no-action branch at
    /// the attribution horizon. This makes ambient change explainable rather
    /// than leaving it as a checksum-only difference.
    pub ambient_feedback: Vec<GameplayFeedbackEvent>,
    pub immediate_domains: BTreeSet<GameplayDomain>,
    pub delayed_domains: BTreeSet<GameplayDomain>,
    pub persistent_domains: BTreeSet<GameplayDomain>,
    pub ambient_domains: BTreeSet<GameplayDomain>,
    pub signals: BTreeSet<GameplayTraceSignal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameplayFeedbackSource {
    Command,
    Simulation,
    Ambient,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayFeedbackEvent {
    pub source: GameplayFeedbackSource,
    pub day: i64,
    pub channel: String,
    pub kind: String,
    pub subject: String,
    pub text: String,
}

impl GameplayTraceStep {
    #[must_use]
    pub fn consequence_breadth(&self) -> usize {
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
    pub first_institution_support_target: Option<InstitutionId>,
    pub first_office_campaign_day: Option<i64>,
    pub first_office_campaign_target: Option<InstitutionId>,
    pub first_office_day: Option<i64>,
    pub first_city_shaping_action_day: Option<i64>,
    pub first_city_shaping_command: Option<GameplayCommandKind>,
    pub first_player_labor_dispute_day: Option<i64>,
    pub first_heir_designation_day: Option<i64>,
    pub first_succession_day: Option<i64>,
}

/// One dynasty's end-of-campaign standing in the city power ranking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayRivalStanding {
    pub dynasty_id: DynastyId,
    pub name: String,
    pub is_player: bool,
    pub treasury: Money,
    pub legitimacy_basis_points: u16,
    pub offices_held: u16,
    pub operating_businesses: u16,
}

/// Where the player's house stands among the city's dynasties at campaign
/// end, so rivalry pressure (or its absence) is readable instead of implied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayRivalContext {
    pub dynasty_count: u16,
    /// 1-based rank of the player's treasury among all houses.
    pub player_treasury_rank: u16,
    /// 1-based rank of the player's legitimacy among all houses.
    pub player_legitimacy_rank: u16,
    /// The strongest rival houses plus the player, ordered by treasury.
    pub leaders_by_treasury: Vec<GameplayRivalStanding>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplaySuccessionTransition {
    pub day: i64,
    pub family_unity_before: u16,
    pub family_unity_after: u16,
    pub legitimacy_before: u16,
    pub legitimacy_after: u16,
    pub offices_before: u16,
    pub offices_after: u16,
    pub institution_memberships_before: u16,
    pub institution_memberships_after: u16,
    pub represented_institutions_before: u16,
    pub represented_institutions_after: u16,
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayPhaseStats {
    pub decision_cycles: u32,
    pub substantive_actions: u32,
    pub institutional_campaign_actions: u32,
    pub quiet_cycles: u32,
    pub quiet_cycles_with_ambient_change: u32,
    pub longest_quiet_streak_cycles: u32,
    pub blocked_cycles: u32,
    pub generator_gap_cycles: u32,
    pub policy_gate_cycles: u32,
    pub restrained_cycles: u32,
    pub validation_gate_cycles: u32,
    pub budget_gate_cycles: u32,
    pub dormant_cycles: u32,
    pub cycles_with_multiple_viable_command_kinds: u32,
    pub cycles_with_close_viable_command_kinds: u32,
    pub cycles_with_distinct_immediate_consequences: u32,
    pub cycles_with_distinct_projected_consequences: u32,
    pub cycles_with_multiple_viable_options: u32,
    pub cycles_with_close_viable_options: u32,
    pub cycles_with_distinct_immediate_option_consequences: u32,
    pub cycles_with_distinct_projected_option_consequences: u32,
    pub total_viable_choices: u32,
    pub total_viable_command_kinds: u32,
    pub executed_commands: BTreeMap<GameplayCommandKind, u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayQuietDiagnostic {
    /// Command kinds that had a detected activation opportunity but produced
    /// no generated candidate during no-action cycles. Identifies generator
    /// gaps where the game state invited an action the agent could not build.
    pub generator_gaps: BTreeMap<GameplayCommandKind, u32>,
    /// Command kinds whose candidates were all removed by the agent's own
    /// spending-discipline filters during no-action cycles. The game built an
    /// option, but the persona's reserve policy declined it.
    pub policy_gates: BTreeMap<GameplayCommandKind, u32>,
    /// Command kinds where an activation opportunity fired but no candidate was
    /// built because the persona's standing policy deliberately narrows that
    /// route to strategic-need conditions (distress sales, wage-fairness
    /// cadence, succession-pressure designations, and similar thresholds). The
    /// world offered; the agent declined by design, so these are neither true
    /// generator holes nor spending-policy vetoes of built options.
    pub restrained_routes: BTreeMap<GameplayCommandKind, u32>,
    /// Command kinds that generated candidates and passed the agent's filters
    /// where all generated candidates were probed and rejected by canonical validation
    /// during no-action cycles.
    pub validation_gates: BTreeMap<GameplayCommandKind, u32>,
    /// Command kinds where candidate variants remained unprobed due to the probe budget limit
    /// and none of the probed variants were viable during no-action cycles.
    pub budget_gates: BTreeMap<GameplayCommandKind, u32>,
    /// No-action cycles where no generator gap, spending-policy gate, or
    /// validation gate was detected. The world offered no detected opportunity;
    /// the agent was genuinely dormant rather than blocked by its own policy.
    pub dormant_cycles: u32,
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
    pub cycles_with_multiple_viable_options: u32,
    pub cycles_with_close_viable_options: u32,
    pub cycles_with_distinct_immediate_option_consequences: u32,
    pub cycles_with_distinct_projected_option_consequences: u32,
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
    pub peak_player_treasury: Money,
    pub minimum_unowned_property_value: Option<Money>,
    pub maximum_disputed_employment: u16,
    pub maximum_player_disputed_employment: u16,
    pub maximum_delinquent_loans: u16,
    pub maximum_defaulted_loans: u16,
    pub maximum_player_delinquent_lending: u16,
    pub maximum_player_defaulted_lending: u16,
    pub maximum_player_delinquent_borrowing: u16,
    pub maximum_player_defaulted_borrowing: u16,
    pub maximum_delinquent_civic_debts: u16,
    pub maximum_defaulted_civic_debts: u16,
    pub maximum_offices_held: u16,
    pub maximum_unfinished_public_works: u16,
    pub maximum_active_crises: u16,
    pub observed_crisis_kinds: BTreeSet<CrisisKind>,
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
    pub player_debt_enforcement_cases: u16,
    pub peak_route_disruption_basis_points: u16,
    pub peak_city_distressed_businesses: u16,
    pub rival_context: GameplayRivalContext,
    pub fantasy_arc: GameplayFantasyArc,
    pub succession_transition: Option<GameplaySuccessionTransition>,
    pub quiet_diagnostic: GameplayQuietDiagnostic,
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
    pub cycles_with_multiple_viable_options: u64,
    pub cycles_with_close_viable_options: u64,
    pub cycles_with_distinct_immediate_option_consequences: u64,
    pub cycles_with_distinct_projected_option_consequences: u64,
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
    pub quiet_diagnostic: GameplayQuietDiagnostic,
    pub scores: GameplayScores,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayHarnessReport {
    pub schema_version: u16,
    pub config: GameplayHarnessConfig,
    pub aggregate: GameplayAggregate,
    pub persona_aggregates: BTreeMap<GameplayPersona, GameplayAggregate>,
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
    #[error(
        "activation predicates missed canonically viable command kinds {kinds:?}; a world predicate has drifted from its canonical validation route"
    )]
    ActivationPredicateDrift { kinds: Vec<GameplayCommandKind> },
    #[error("gameplay harness counterfactual worker panicked")]
    CounterfactualWorkerPanicked,
    #[error("gameplay harness campaign worker panicked")]
    CampaignWorkerPanicked,
}
