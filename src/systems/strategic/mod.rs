//! Strategic scheduling entries, shared relationship plumbing, law appliers,
//! information reports, and annual family systems.

pub(crate) use super::SimulationError;
pub(crate) use super::commands::CrisisResponse;
pub(crate) use super::transactions::{
    TimelineError, add_market_supply, checked_future_day, checked_next_business_finance_version,
    credit_market_clearing_account, debit_market_clearing_account, next_business_finance_version,
    next_family_charter_version,
};
pub(crate) use crate::core::{
    AiObjective, AppState, AuditKind, AuditRecord, BusinessStatus, CharacterRole, CharacterStatus,
    ChronicleEntry, ChronicleKind, CivicDebtStatus, ContractStatus, Crisis, CrisisKind,
    CrisisStatus, DistrictRuntime, DynastyPair, EmploymentAgreement, EmploymentStatus, EnactedLaw,
    ExternalRoute, FamilyCouncilState, FamilyLink, FamilyLinkKind, HouseGovernance,
    InformationConfidence, InformationReport, InformationTarget, InstitutionRuntime, LawKind,
    LegalCase, LegalCaseKind, LegalCaseStatus, LegalClaimSource, Loan, LoanStatus, ObjectiveKind,
    ObjectiveStatus, OfficePower, OutboxKind, OutboxMessage, Property, PropertyKind, PublicWork,
    PublicWorkKind, PublicWorkStatus, RelationshipState, SupplyContract,
};
pub(crate) use crate::ids::{
    BusinessId, CharacterId, CivicDebtId, DistrictId, DynastyId, EmploymentId, GoodId, HouseholdId,
    IdentifierAllocationError, InstitutionId, PropertyId,
};
pub(crate) use crate::money::{
    Money, Quantity, affordable_quantity, checked_cost_for, cost_for, rounded_cost_copper_wide,
};
pub(crate) use crate::registry::{InstitutionKind, Registry};
pub(crate) use crate::systems::INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS;
pub(crate) use serde::{Deserialize, Serialize};
pub(crate) use std::collections::{BTreeMap, BTreeSet};
pub(crate) use std::fmt::Write as _;
pub(crate) use thiserror::Error;

pub(crate) const OFFICE_ADMINISTRATIVE_LOAD_PER_POWER: u16 = 10;
pub(crate) const OFFICE_DUTY_COST_PER_POWER: Money = Money::from_copper(100);
pub(crate) const OFFICE_DUTY_PORTFOLIO_SURCHARGE_PER_ADDITIONAL_OFFICE: Money =
    Money::from_copper(50);
/// Monthly fees of office paid out of the holding institution's budget to the
/// officeholder's dynasty. Offices stay a net cost (duties exceed the stipend),
/// but service should materially compensate the house that performs it.
const OFFICE_STIPEND_PER_POWER: Money = Money::from_copper(40);
const AI_DYNASTY_HOUSEHOLD_UPKEEP_MONTHLY: Money = Money::from_copper(500);
const AI_DYNASTY_UPKEEP_PER_FAMILY_MEMBER: Money = Money::from_copper(250);
const AI_DYNASTY_UPKEEP_PER_BUSINESS: Money = Money::from_copper(400);
/// Standing costs of great-house display grow with stored wealth: retainers,
/// obligations, and civic expectation scale off everything above this
/// threshold each month. Hoarded treasure therefore bleeds instead of
/// compounding without limit, keeping rivals reachably mortal and the city's
/// wealth circulating through the same economy everyone else uses.
const AI_DYNASTY_WEALTH_UPKEEP_THRESHOLD: Money = Money::from_copper(40_000);
const AI_DYNASTY_WEALTH_UPKEEP_BASIS_POINTS: i64 = 150;
const AI_DYNASTY_UPKEEP_SHORTFALL_LEGITIMACY_PENALTY: u16 = 60;
const AI_DYNASTY_UPKEEP_SHORTFALL_RELIABILITY_PENALTY: u16 = 120;
const OFFICE_DUTY_FAILURE_NOTIFICATION_INTERVAL_DAYS: i64 = 90;
const OFFICE_DUTY_FORFEITURE_WINDOW_DAYS: i64 = 90;
const OFFICE_DUTY_REELECTION_BAN_DAYS: i64 = 180;
const OFFICE_DUTY_FORFEITURE_THRESHOLD: usize = 3;
const OFFICE_NOMINATION_CAMPAIGN_BONUS: u32 = 2_000;

/// How much extra victory legitimacy each point of institutional standing
/// grants: winning a default (7,000) guild office pays 150 + 140, and a fully
/// endowed one up to 150 + 200. The reward lands on the winner's dynasty,
/// whose legitimacy then feeds every future election score, so endowments and
/// office stewardship translate into durable political weight.
const INSTITUTION_STANDING_VICTORY_DIVISOR: u32 = 50;
const OFFICE_CONCENTRATION_BACKLASH_PER_ADDITIONAL_OFFICE: i16 = 120;
const MAX_OFFICE_CONCENTRATION_BACKLASH: i16 = 600;
pub(crate) const DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS: i64 = 180;
pub(crate) const PROPERTY_LIQUIDATION_BASIS_POINTS: i64 = 5_000;
const PROPERTY_AUCTION_DISTRESS_TREASURY_LIMIT: Money = Money::from_copper(2_000);

/// Contract commitments are sized against a five-day operating week even
/// though the simulation week has seven days: the margin keeps both parties
/// free to keep trading the good on the open market alongside the contract.
const CONTRACT_CAPACITY_COMMITMENT_DAYS: i64 = 5;

const UNADDRESSED_CRISIS_MONTHLY_ESCALATION_BASIS_POINTS: u16 = 240;
const ADDRESSED_CRISIS_MONTHLY_RECOVERY_BASIS_POINTS: u16 = 360;
/// Route disruption at or above this level spawns a trade-disruption crisis;
/// a tracked disruption also holds at this condition until every route heals.
pub(crate) const TRADE_DISRUPTION_ROUTE_DISRUPTION_THRESHOLD: u16 = 7_000;
/// A resolved banking panic raises the default bar for a follow-up panic for
/// three years; older panics stop counting so confidence can rebuild.
const BANKING_PANIC_MEMORY_DAYS: i64 = 3 * 360;
/// Resolved crises stay visible in state for three years, then are pruned.
const CRISIS_HISTORY_RETENTION_DAYS: i64 = 3 * 360;
const EPIDEMIC_ONSET_WELFARE_DIVISOR: u16 = 7;
const EPIDEMIC_DAILY_WELFARE_DIVISOR: u16 = 60;
const DISTRICT_BACKGROUND_EMPLOYMENT_BASIS_POINTS: u16 = 4_500;
const DISTRICT_FORMAL_EMPLOYMENT_BASIS_POINTS_PER_WORKER: u32 = 100;
const DISTRICT_MAX_FORMAL_EMPLOYMENT_BONUS_BASIS_POINTS: u32 = 4_500;
const PUBLIC_WORK_TOOL_SHARE_BASIS_POINTS: i64 = 2_500;

/// Speculative credit terms: risk capital carries a punishing rate on a short,
/// heavy book, is capped near the working-capital ceiling, and is secured by
/// whatever unpledged property the borrower can offer, so a failed speculation
/// costs the borrower real assets instead of only reputation. The installment
/// is deliberately large relative to a losing firm's distribution stream:
/// some of these loans rescue the borrower, others miss installments within
/// months, fall delinquent, default, and ground the enforcement claims that
/// keep courts, seizure, and banking panics reachable inside one session.
const SPECULATIVE_LOAN_INTEREST_BASIS_POINTS: u16 = 2_500;
const SPECULATIVE_LOAN_TERM_WEEKS: i64 = 22;
const SPECULATIVE_LOAN_MAX_PRINCIPAL: Money = Money::from_copper(10_000);
/// Monthly risk-appetite draw per liquid house: speculative offers stay a
/// minority of the lending book while still arriving several times per
/// campaign instead of roughly once per session.
const SPECULATIVE_LOAN_MONTHLY_CHANCE_BASIS_POINTS: u16 = 4_500;

mod ai;
mod businesses;
mod contracts;
mod credit;
mod crises;
mod households;
mod initialization;
mod labor;
mod legal_cases;
mod offices;
mod property;

// Each submodule pulls this module's shared plumbing in wholesale; local
// definitions shadow the glob, so no ambiguity arises.
pub(crate) use ai::*;
#[allow(clippy::wildcard_imports)]
pub use businesses::*;
pub use contracts::*;
pub use credit::*;
pub(crate) use crises::*;
pub(crate) use households::*;
pub(crate) use initialization::*;
pub(crate) use labor::*;
pub(crate) use legal_cases::*;
pub(crate) use offices::*;
pub use property::*;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum StrategicError {
    #[error(transparent)]
    IdentifierAllocation(#[from] IdentifierAllocationError),
    #[error(transparent)]
    Timeline(#[from] TimelineError),
    /// Simulation-level failure propagated from a shared transaction primitive.
    #[error(transparent)]
    Simulation(#[from] super::SimulationError),
    #[error(
        "state scenario {state_scenario:?} does not match registry scenario {registry_scenario:?}"
    )]
    RegistryMismatch {
        state_scenario: String,
        registry_scenario: String,
    },
    #[error("business {business_id} does not exist")]
    MissingBusiness { business_id: BusinessId },
    #[error("business {business_id} is not active")]
    BusinessInactive { business_id: BusinessId },
    #[error("business {business_id} is not owned by dynasty {dynasty_id}")]
    BusinessNotOwnedByDynasty {
        business_id: BusinessId,
        dynasty_id: DynastyId,
    },
    #[error("dynasty {dynasty_id} does not exist")]
    MissingDynasty { dynasty_id: DynastyId },
    #[error("property {property_id} does not exist")]
    MissingProperty { property_id: PropertyId },
    #[error("contract parties must be different businesses")]
    SameContractParty,
    #[error("contract businesses must belong to different dynasties, both belong to {dynasty_id}")]
    SameContractOwner { dynasty_id: DynastyId },
    #[error("loan parties must be different dynasties")]
    SameLoanParty,
    #[error(
        "loan {loan_id} already represents unsettled credit from dynasty {lender_dynasty_id} to dynasty {borrower_dynasty_id}"
    )]
    ExistingUnsettledLoan {
        lender_dynasty_id: DynastyId,
        borrower_dynasty_id: DynastyId,
        loan_id: crate::ids::LoanId,
    },
    #[error("defaulted loan {loan_id} cannot be restructured before day {available_day}")]
    DefaultedLoanRestructuringCooldown {
        loan_id: crate::ids::LoanId,
        available_day: i64,
    },
    #[error(
        "loan {loan_id} cannot add {incoming}; current balance {current} would exceed the supported money range"
    )]
    LoanBalanceOverflow {
        loan_id: crate::ids::LoanId,
        current: Money,
        incoming: Money,
    },
    #[error("amount must be positive")]
    NonPositiveAmount,
    #[error("quantity must be positive")]
    NonPositiveQuantity,
    #[error("contract duration must contain at least one week")]
    EmptyContractDuration,
    #[error(
        "contract payment for quantity {quantity} at unit price {unit_price} exceeds the supported money range"
    )]
    ContractPaymentOverflow {
        quantity: Quantity,
        unit_price: Money,
    },
    #[error("seller business {seller_business_id} cannot produce good {good_id}")]
    SellerCannotProduce {
        seller_business_id: BusinessId,
        good_id: GoodId,
    },
    #[error("buyer business {buyer_business_id} does not consume good {good_id}")]
    BuyerDoesNotConsume {
        buyer_business_id: BusinessId,
        good_id: GoodId,
    },
    #[error("dynasty {dynasty_id} has only {available} available, requires {required}")]
    InsufficientDynastyFunds {
        dynasty_id: DynastyId,
        available: Money,
        required: Money,
    },
    #[error(
        "dynasty {dynasty_id} cannot receive {incoming}; current treasury {current} would exceed the supported money range"
    )]
    DynastyTreasuryOverflow {
        dynasty_id: DynastyId,
        current: Money,
        incoming: Money,
    },
    #[error(
        "business {business_id} cannot receive {incoming}; current cash {current} would exceed the supported money range"
    )]
    BusinessCashOverflow {
        business_id: BusinessId,
        current: Money,
        incoming: Money,
    },
    #[error("business {business_id} finance version is exhausted")]
    BusinessFinanceVersionExhausted { business_id: BusinessId },
    #[error(
        "business {business_id} has only {available} distributable cash after preserving reserve {required_reserve}; requested {requested}"
    )]
    BusinessDistributionExceedsSurplus {
        business_id: BusinessId,
        available: Money,
        required_reserve: Money,
        requested: Money,
    },
    #[error(
        "dynasty {dynasty_id} cannot remove administrative load {outgoing}; current load is {current}"
    )]
    DynastyAdministrativeLoadUnderflow {
        dynasty_id: DynastyId,
        current: u16,
        outgoing: u16,
    },
    #[error(
        "dynasty {dynasty_id} cannot add administrative load {incoming}; current load {current} exceeds the supported range"
    )]
    DynastyAdministrativeLoadOverflow {
        dynasty_id: DynastyId,
        current: u16,
        incoming: u16,
    },
    #[error(
        "business acquisition cost overflows the supported money range: price {purchase_price}, recapitalization {recapitalization}"
    )]
    AcquisitionCostOverflow {
        purchase_price: Money,
        recapitalization: Money,
    },
    #[error(
        "business {business_id} valuation exceeds the supported money range after applying the acquisition discount"
    )]
    BusinessValuationOverflow { business_id: BusinessId },
    #[error("loan interest {interest_basis_points} is outside the 0..=10000 basis-point range")]
    InterestOutOfRange { interest_basis_points: u16 },
    #[error("property {property_id} is not owned by borrower dynasty {borrower_dynasty_id}")]
    CollateralNotOwned {
        property_id: PropertyId,
        borrower_dynasty_id: DynastyId,
    },
    #[error("property {property_id} is already pledged to loan {loan_id}")]
    PropertyAlreadyPledged {
        property_id: PropertyId,
        loan_id: crate::ids::LoanId,
    },
    #[error("property {property_id} is already owned")]
    PropertyAlreadyOwned { property_id: PropertyId },
    #[error("property {property_id} is not owned by dynasty {seller_dynasty_id}")]
    PropertyNotOwnedBySeller {
        property_id: PropertyId,
        seller_dynasty_id: DynastyId,
    },
    #[error("property buyer and seller must differ")]
    SamePropertyParty,
    #[error("the civic treasury is not available for a property auction guarantee")]
    MissingCivicTreasury,
    #[error(
        "property auction has only {buyer_available} private and {civic_available} civic liquidity, requires {required}"
    )]
    InsufficientPropertyAuctionLiquidity {
        buyer_available: Money,
        civic_available: Money,
        required: Money,
    },
    #[error("property collateral references missing loan {loan_id}")]
    MissingCollateralLoan { loan_id: crate::ids::LoanId },
    #[error(
        "property {property_id} lien loan {loan_id} belongs to borrower {borrower_dynasty_id}, not seller {seller_dynasty_id}"
    )]
    PropertyLienBorrowerMismatch {
        property_id: PropertyId,
        loan_id: crate::ids::LoanId,
        borrower_dynasty_id: DynastyId,
        seller_dynasty_id: DynastyId,
    },
    #[error(
        "property {property_id} sale price {price} cannot settle lien loan {loan_id} balance {balance}"
    )]
    PropertySaleCannotSettleLien {
        property_id: PropertyId,
        loan_id: crate::ids::LoanId,
        price: Money,
        balance: Money,
    },
    #[error("business {business_id} is already owned by dynasty {buyer_dynasty_id}")]
    BusinessAlreadyOwned {
        business_id: BusinessId,
        buyer_dynasty_id: DynastyId,
    },
    #[error("character {manager_id} is not an active member of buyer dynasty {buyer_dynasty_id}")]
    InvalidAcquisitionManager {
        manager_id: CharacterId,
        buyer_dynasty_id: DynastyId,
    },
    #[error(
        "business {business_id} requires at least {required} recapitalization, but {provided} was provided"
    )]
    InsufficientBusinessRecapitalization {
        business_id: BusinessId,
        provided: Money,
        required: Money,
    },
}

fn ensure_registry_matches(registry: &Registry, state: &AppState) -> Result<(), StrategicError> {
    if state.scenario_key() != registry.scenario().key() {
        return Err(StrategicError::RegistryMismatch {
            state_scenario: state.scenario_key().to_owned(),
            registry_scenario: registry.scenario().key().to_owned(),
        });
    }
    Ok(())
}

/// A player-facing counterparty report a commit will emit.
#[derive(Clone, Copy)]
struct ReservedCounterpartyReport {
    id: crate::ids::InformationReportId,
    expires_day: i64,
}

const COUNTERPARTY_REPORT_EXPIRY_DAYS: i64 = 180;

fn grant_maturing_institution_support(state: &mut AppState) -> Result<(), SimulationError> {
    let day = state.clock.day();
    let Some(establishment_day) = day.checked_sub(INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS) else {
        return Ok(());
    };
    // Patronage records mature exactly 90 days after they are appended, and
    // the audit log is day-ordered: skip to the single establishment day
    // instead of scanning the whole history every day.
    let start = state
        .audit_log
        .partition_point(|record| record.day() < establishment_day);
    let mut matured: Vec<(InstitutionId, CharacterId)> = Vec::new();
    for record in state.audit_log.iter().skip(start) {
        if record.day() > establishment_day {
            break;
        }
        if record.kind() != AuditKind::InstitutionPatronage {
            continue;
        }
        if let Some((institution_id, character_id)) =
            record.audit_subject().institution_character_ids()
        {
            matured.push((institution_id, character_id));
        }
    }
    matured.sort_unstable();
    matured.dedup();
    for (institution_id, character_id) in matured {
        let Some(character) = state.characters.get(character_id) else {
            continue;
        };
        let dynasty_id = character.dynasty_id();
        let Some(dynasty) = state.dynasties.get_mut(&dynasty_id) else {
            continue;
        };
        dynasty.resources.legitimacy_basis_points = dynasty
            .resources
            .legitimacy_basis_points
            .saturating_add(250)
            .min(10_000);
        try_push_outbox(
            state,
            OutboxKind::Politics,
            format!("Institutional support established for character {character_id}"),
            format!(
                "The house's patronage of institution {institution_id} has matured into established standing; the dynasty's legitimacy grows."
            ),
        )?;
    }
    Ok(())
}

pub(crate) fn expire_time_limited_state(state: &mut AppState) {
    let day = state.clock.day();
    state
        .information_reports
        .retain(|_, report| report.expires_day >= day);
    for institution in state.institutions.values_mut() {
        if institution
            .active_directive
            .is_some_and(|directive| directive.expires_day < day)
        {
            institution.active_directive = None;
        }
    }
}

pub(crate) fn active_law_value(state: &AppState, kind: LawKind) -> Option<i64> {
    state
        .laws
        .values()
        .find(|law| law.active && law.kind == kind)
        .map(|law| law.value)
}

fn apply_route_laws(state: &mut AppState) {
    let Some(toll) = active_law_value(state, LawKind::ForeignMerchantToll) else {
        return;
    };
    let toll = u16::try_from(toll.clamp(0, 10_000)).unwrap_or(10_000);
    for route in state.external_routes.values_mut() {
        route.toll_basis_points = toll;
    }
}

fn apply_external_route_supply(state: &mut AppState) -> Result<(), SimulationError> {
    let routes: Vec<_> = state
        .external_routes
        .values()
        .filter(|route| route.active)
        .map(|route| {
            let disruption_availability = 10_000_u16.saturating_sub(route.disruption_basis_points);
            let toll_availability = 10_000_u16.saturating_sub(route.toll_basis_points);
            (
                route.good_id,
                route
                    .daily_capacity
                    .saturating_mul_ratio(i64::from(disruption_availability), 10_000)
                    .saturating_mul_ratio(i64::from(toll_availability), 10_000),
            )
        })
        .collect();
    for (good_id, quantity) in routes {
        add_market_supply(state, good_id, quantity)?;
    }
    Ok(())
}

pub(crate) fn apply_law_price_controls(registry: &Registry, state: &mut AppState) {
    let ceiling = active_law_value(state, LawKind::BreadPriceCeiling);
    let Some(ceiling) = ceiling else {
        return;
    };
    let Some(bread_id) = registry.get_good_id("bread") else {
        return;
    };
    let quote = state
        .market
        .quotes
        .get_mut(&bread_id)
        .expect("bread quote must exist");
    quote.price = quote.price.min(Money::from_copper(ceiling));
}

pub(crate) fn run_daily_strategic_systems(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    apply_route_laws(state);
    apply_crisis_daily_effects(registry, state)?;
    grant_maturing_institution_support(state)?;
    recover_ai_businesses(registry, state);
    apply_external_route_supply(state)?;
    Ok(())
}

pub(crate) fn run_weekly_strategic_systems(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    settle_contracts(state)?;
    settle_loans(state)?;
    settle_civic_debts(registry, state)?;
    settle_property_rents(state)?;
    settle_employment(registry, state)?;
    distribute_business_dividends(registry, state)?;
    progress_public_works(registry, state)?;
    update_relationships_from_obligations(state);
    update_quality_reputations(state);
    apply_law_economic_effects(registry, state)?;
    Ok(())
}

fn update_relationships_from_obligations(state: &mut AppState) {
    for relationship in state.relationships.values_mut() {
        if relationship.obligation > 0 {
            relationship.trust_basis_points = relationship
                .trust_basis_points
                .saturating_add(5)
                .min(10_000);
            // An outstanding favor is gradually worked off as the grateful
            // house repays it through cooperation, so its weekly trust
            // influence is bounded instead of accruing forever.
            relationship.obligation -= 1;
        } else if relationship.obligation < 0 {
            relationship.resentment_basis_points = relationship
                .resentment_basis_points
                .saturating_add(5)
                .min(10_000);
            relationship.obligation += 1;
        }
    }
}

fn update_quality_reputations(state: &mut AppState) {
    let dynasty_ids: Vec<_> = state.dynasties.keys().copied().collect();
    for dynasty_id in dynasty_ids {
        let mut total_quality = 0_u64;
        let mut business_count = 0_u64;
        let mut lifetime_revenue_copper = 0_i128;
        let mut lifetime_costs_copper = 0_i128;
        for business in state.businesses.iter().filter(|business| {
            business.owner_dynasty_id() == dynasty_id
                && business.status() != crate::core::BusinessStatus::Closed
        }) {
            total_quality =
                total_quality.saturating_add(u64::from(business.operations.quality_basis_points));
            business_count = business_count.saturating_add(1);
            lifetime_revenue_copper += i128::from(business.finance.lifetime_revenue.copper());
            lifetime_costs_copper += i128::from(business.finance.lifetime_costs.copper());
        }
        if business_count == 0 {
            continue;
        }
        let target = u16::try_from(total_quality / business_count).unwrap_or(10_000);
        let dynasty = state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("reputation dynasty must exist");
        let maximum_step = quality_reputation_step(
            dynasty.resources.reputation_quality_basis_points,
            target,
            lifetime_revenue_copper,
            lifetime_costs_copper,
        );
        dynasty.resources.reputation_quality_basis_points = move_basis_points_toward(
            dynasty.resources.reputation_quality_basis_points,
            target,
            maximum_step,
        );
    }
}

fn quality_reputation_step(
    current: u16,
    target: u16,
    lifetime_revenue_copper: i128,
    lifetime_costs_copper: i128,
) -> u16 {
    if current >= target {
        return 50;
    }
    let has_trade_history = lifetime_revenue_copper > 0 || lifetime_costs_copper > 0;
    if has_trade_history && lifetime_revenue_copper >= lifetime_costs_copper {
        50
    } else {
        25
    }
}

fn move_basis_points_toward(current: u16, target: u16, maximum_step: u16) -> u16 {
    if current < target {
        current.saturating_add(target.saturating_sub(current).min(maximum_step))
    } else {
        current.saturating_sub(current.saturating_sub(target).min(maximum_step))
    }
}

fn adjust_reliability_reputation(state: &mut AppState, dynasty_id: DynastyId, delta: i16) {
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("reputation dynasty must exist");
    let adjusted = i32::from(dynasty.resources.reputation_reliability_basis_points)
        .saturating_add(i32::from(delta))
        .clamp(0, 10_000);
    dynasty.resources.reputation_reliability_basis_points =
        u16::try_from(adjusted).expect("clamped reputation must fit u16");
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RelationshipDelta {
    trust: i16,
    respect: i16,
    fear: i16,
    resentment: i16,
    obligation: i32,
}

impl RelationshipDelta {
    pub(crate) const fn new(
        trust: i16,
        respect: i16,
        fear: i16,
        resentment: i16,
        obligation: i32,
    ) -> Self {
        Self {
            trust,
            respect,
            fear,
            resentment,
            obligation,
        }
    }
}

pub(crate) fn adjust_dynasty_relationship(
    state: &mut AppState,
    left_dynasty_id: DynastyId,
    right_dynasty_id: DynastyId,
    delta: RelationshipDelta,
) {
    if left_dynasty_id == right_dynasty_id {
        return;
    }
    let pair = DynastyPair::new(left_dynasty_id, right_dynasty_id);
    let day = state.clock.day();
    let relationship = state
        .relationships
        .get_mut(&pair)
        .expect("every dynasty pair must have a relationship record");
    relationship.trust_basis_points =
        adjust_basis_points(relationship.trust_basis_points, delta.trust);
    relationship.respect_basis_points =
        adjust_basis_points(relationship.respect_basis_points, delta.respect);
    relationship.fear_basis_points =
        adjust_basis_points(relationship.fear_basis_points, delta.fear);
    relationship.resentment_basis_points =
        adjust_basis_points(relationship.resentment_basis_points, delta.resentment);
    relationship.obligation = relationship.obligation.saturating_add(delta.obligation);
    relationship.last_interaction_day = day;
}

pub(crate) const MAX_RELATIONSHIP_MEMORIES: usize = 12;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub(crate) enum DurableFeedbackError {
    #[error(transparent)]
    IdentifierAllocation(#[from] IdentifierAllocationError),
    #[error(transparent)]
    Timeline(#[from] TimelineError),
}

impl From<DurableFeedbackError> for StrategicError {
    fn from(error: DurableFeedbackError) -> Self {
        match error {
            DurableFeedbackError::IdentifierAllocation(error) => Self::IdentifierAllocation(error),
            DurableFeedbackError::Timeline(error) => Self::Timeline(error),
        }
    }
}

impl From<DurableFeedbackError> for SimulationError {
    fn from(error: DurableFeedbackError) -> Self {
        match error {
            DurableFeedbackError::IdentifierAllocation(error) => Self::IdentifierAllocation(error),
            DurableFeedbackError::Timeline(error) => Self::Timeline(error),
        }
    }
}

pub(crate) fn remember_dynasty_interaction(
    state: &mut AppState,
    left_dynasty_id: DynastyId,
    right_dynasty_id: DynastyId,
    memory: &str,
) {
    if left_dynasty_id == right_dynasty_id {
        return;
    }
    let pair = DynastyPair::new(left_dynasty_id, right_dynasty_id);
    let day = state.clock.day();
    let relationship = state
        .relationships
        .get_mut(&pair)
        .expect("every dynasty pair must have a relationship record");
    if relationship.memories.len() >= MAX_RELATIONSHIP_MEMORIES {
        relationship.memories.remove(0);
    }
    relationship.memories.push(format!("Day {day}: {memory}"));
    relationship.last_interaction_day = day;
}

pub(crate) fn try_record_counterparty_information(
    state: &mut AppState,
    first_dynasty_id: DynastyId,
    second_dynasty_id: DynastyId,
    source: &str,
) -> Result<(), DurableFeedbackError> {
    let Some(reservation) =
        reserve_counterparty_report(state, first_dynasty_id, second_dynasty_id)?
    else {
        return Ok(());
    };
    emit_counterparty_report(
        state,
        reservation,
        first_dynasty_id,
        second_dynasty_id,
        source,
    );
    Ok(())
}

/// Reserves the player-facing counterparty report for a pair when exactly one
/// party is the player; AI-to-AI pairs consume no identifier and reserve
/// nothing.
fn reserve_counterparty_report(
    state: &mut AppState,
    first_dynasty_id: DynastyId,
    second_dynasty_id: DynastyId,
) -> Result<Option<ReservedCounterpartyReport>, DurableFeedbackError> {
    let player_dynasty_id = state.player_dynasty_id;
    let counterparty_is_player_adjacent =
        (first_dynasty_id == player_dynasty_id) != (second_dynasty_id == player_dynasty_id);
    if !counterparty_is_player_adjacent {
        return Ok(None);
    }
    let expires_day = checked_future_day(state.clock.day(), COUNTERPARTY_REPORT_EXPIRY_DAYS)?;
    let id = state.next_ids.try_information_report()?;
    Ok(Some(ReservedCounterpartyReport { id, expires_day }))
}

/// Emits a previously reserved counterparty report. Infallible: every
/// fallible step ran during reservation, and the report text is derived from
/// current state at emit time exactly as before.
fn emit_counterparty_report(
    state: &mut AppState,
    reservation: ReservedCounterpartyReport,
    first_dynasty_id: DynastyId,
    second_dynasty_id: DynastyId,
    source: &str,
) {
    let ReservedCounterpartyReport { id, expires_day } = reservation;
    let player_dynasty_id = state.player_dynasty_id;
    let counterparty_id =
        if first_dynasty_id == player_dynasty_id && second_dynasty_id != player_dynasty_id {
            second_dynasty_id
        } else {
            first_dynasty_id
        };
    let counterparty = state
        .dynasties
        .get(&counterparty_id)
        .expect("counterparty dynasty must exist");
    let target = InformationTarget::Counterparty {
        dynasty_id: counterparty_id,
    };
    let subject = format!("Counterparty report: House {}", counterparty.name());
    let reliability = counterparty.resources.reputation_reliability_basis_points;
    let pair = DynastyPair::new(player_dynasty_id, counterparty_id);
    let relationship = state
        .relationships
        .get(&pair)
        .expect("counterparty relationship must exist");
    let summary = format!(
        "Reliability {}.{}%; trust {}.{}%; respect {}.{}%; resentment {}.{}%; obligation {}.",
        reliability / 100,
        (reliability % 100) / 10,
        relationship.trust_basis_points / 100,
        (relationship.trust_basis_points % 100) / 10,
        relationship.respect_basis_points / 100,
        (relationship.respect_basis_points % 100) / 10,
        relationship.resentment_basis_points / 100,
        (relationship.resentment_basis_points % 100) / 10,
        relationship.obligation
    );
    state.information_reports.retain(|_, report| {
        report.owner_dynasty_id != player_dynasty_id || report.target != Some(target)
    });
    state.information_reports.insert(
        id,
        InformationReport {
            id,
            owner_dynasty_id: player_dynasty_id,
            target: Some(target),
            subject,
            confidence: InformationConfidence::Probable,
            created_day: state.clock.day(),
            expires_day,
            source: source.to_owned(),
            summary,
        },
    );
}

fn adjust_basis_points(current: u16, delta: i16) -> u16 {
    u16::try_from(
        i32::from(current)
            .saturating_add(i32::from(delta))
            .clamp(0, 10_000),
    )
    .expect("clamped basis-point value must fit u16")
}

fn apply_law_economic_effects(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let emergency_imports = active_law_value(state, LawKind::EmergencyImports)
        .map_or(Quantity::ZERO, |value| Quantity::from_units(value.max(0)));
    if emergency_imports > Quantity::ZERO
        && let Some(grain_id) = registry.get_good_id("grain")
    {
        add_market_supply(state, grain_id, emergency_imports)?;
    }
    Ok(())
}

pub(crate) fn run_monthly_strategic_systems(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    update_district_conditions(state);
    apply_household_living_costs(state)?;
    resolve_institution_selections(registry, state)?;
    apply_office_duties(state)?;
    apply_office_stipends(state)?;
    apply_office_power_effects(registry, state)?;
    apply_active_office_directives(registry, state)?;
    advance_ai_objectives(registry, state)?;
    apply_ai_dynasty_upkeep(state)?;
    advance_ai_credit_participation(registry, state)?;
    update_information_reports(registry, state)?;
    file_grounded_ai_legal_cases(registry, state)?;
    advance_legal_case_hearings(state)?;
    resolve_legal_cases(state)?;
    update_external_route_risk(state);
    detect_and_advance_crises(registry, state)?;
    recover_external_routes(state);
    prune_acknowledged_outbox(state);
    Ok(())
}

/// Acknowledged notices leave the outbox after this long: once flagged they
/// have no reader, and an append-only notification log would grow without
/// bound on multi-generation campaigns. Unacknowledged notices are always
/// retained so nothing actionable is ever dropped.
const ACKNOWLEDGED_OUTBOX_RETENTION_DAYS: i64 = 360;

fn prune_acknowledged_outbox(state: &mut AppState) {
    let day = state.clock.day();
    // Messages are appended in day order, so everything eligible for pruning
    // sits in an aged prefix. When nothing qualifies, skip the mutation
    // entirely: `retain` folds the log's tail and invalidates its incremental
    // checksum, so an unconditional monthly prune would deep-copy the whole
    // outbox every month even when not a single message aged out.
    let has_prunable_prefix = state.outbox.iter().any(|message| {
        message.acknowledged && day.saturating_sub(message.day) > ACKNOWLEDGED_OUTBOX_RETENTION_DAYS
    });
    if !has_prunable_prefix {
        return;
    }
    // Dropping a prefix of the append-only log preserves the strictly
    // increasing ID and day ordering every reader and validator relies on.
    state.outbox.retain(|message| {
        !message.acknowledged
            || day.saturating_sub(message.day) <= ACKNOWLEDGED_OUTBOX_RETENTION_DAYS
    });
}

fn update_information_reports(
    registry: &Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let day = state.clock.day();
    // Movement is measured against each good's price at the last monthly
    // boundary, so the report describes the whole month; the references then
    // roll forward to today's prices for next month's report.
    let most_changed = registry.goods().iter().filter_map(|good| {
        let quote = state.market.get_quote(good.id())?;
        let month_start = state
            .market
            .month_start_prices
            .get(&good.id())
            .copied()
            .unwrap_or_else(|| quote.price());
        let change = (quote.price().copper() - month_start.copper()).unsigned_abs();
        // A month with no net price movement anywhere is a non-event:
        // publishing "identified causes" for it would manufacture
        // intelligence noise.
        (change > 0).then_some((
            change,
            good.id(),
            good.name().to_owned(),
            quote.price(),
            quote.causes().to_vec(),
        ))
    });
    let Some((_, good_id, name, price, causes)) = most_changed.max_by_key(|item| item.0) else {
        state.market.month_start_prices = state
            .market
            .quotes
            .iter()
            .map(|(good_id, quote)| (*good_id, quote.price()))
            .collect();
        return Ok(());
    };
    let expires_day = checked_future_day(day, 120)?;
    let id = state.next_ids.try_information_report()?;
    // Expired reports are removed by the canonical daily
    // `expire_time_limited_state` pass, which runs before monthly systems.
    state.information_reports.insert(
        id,
        InformationReport {
            id,
            owner_dynasty_id: state.player_dynasty_id,
            target: Some(InformationTarget::Market { good_id }),
            subject: format!("Monthly market report: {name}"),
            confidence: InformationConfidence::Confirmed,
            created_day: day,
            expires_day,
            source: "House ledgers, guild correspondence, and market inspection".to_owned(),
            summary: format!("{name} is priced at {price}; identified causes: {causes:?}."),
        },
    );
    state.market.month_start_prices = state
        .market
        .quotes
        .iter()
        .map(|(good_id, quote)| (*good_id, quote.price()))
        .collect();
    Ok(())
}

pub(crate) fn run_annual_strategic_systems(state: &mut AppState) -> Result<(), SimulationError> {
    educate_family_members(state);
    form_dynastic_marriage(state)?;
    update_family_councils(state)?;
    Ok(())
}

fn educate_family_members(state: &mut AppState) {
    for character in state.characters.iter_mut() {
        if character.status() != crate::core::CharacterStatus::Active {
            continue;
        }
        match character.role() {
            CharacterRole::Heir | CharacterRole::Clerk => {
                character.capabilities.administration = character
                    .capabilities
                    .administration
                    .saturating_add(2)
                    .min(100);
                character.capabilities.commerce =
                    character.capabilities.commerce.saturating_add(1).min(100);
            }
            CharacterRole::HeadOfHouse => {}
        }
    }
}

fn form_dynastic_marriage(state: &mut AppState) -> Result<(), SimulationError> {
    if state.clock.day() % 1_800 != 0 {
        return Ok(());
    }
    let heirs: Vec<_> = state
        .dynasties
        .values()
        .filter_map(|dynasty| Some((dynasty.id(), dynasty.heir_id()?)))
        .filter(|(_, heir_id)| {
            state.characters.get(*heir_id).is_some_and(|character| {
                character.status() == crate::core::CharacterStatus::Active
                    // A marriage compact requires adults, matching the
                    // minimum age every other family mechanic enforces.
                    && state.clock.day().saturating_sub(character.birth_day())
                        >= crate::systems::commands::HEIR_MINIMUM_AGE_DAYS
            })
        })
        .collect();
    let is_married = |character_id| {
        state.family_links.values().any(|link| {
            link.active
                && link.kind == FamilyLinkKind::Marriage
                && (link.first_character_id == character_id
                    || link.second_character_id == character_id)
        })
    };
    let selected_pair = heirs.iter().enumerate().find_map(|(index, left)| {
        if is_married(left.1) {
            return None;
        }
        heirs
            .iter()
            .skip(index + 1)
            .find(|right| !is_married(right.1))
            .map(|right| (*left, *right))
    });
    let Some(((left_dynasty, left_heir), (right_dynasty, right_heir))) = selected_pair else {
        return Ok(());
    };
    let id = state.next_ids.try_family_link()?;
    state.family_links.insert(
        id,
        FamilyLink {
            id,
            first_character_id: left_heir,
            second_character_id: right_heir,
            kind: FamilyLinkKind::Marriage,
            active: true,
        },
    );
    let pair = DynastyPair::new(left_dynasty, right_dynasty);
    if let Some(relationship) = state.relationships.get_mut(&pair) {
        relationship.trust_basis_points = relationship
            .trust_basis_points
            .saturating_add(1_000)
            .min(10_000);
        relationship.obligation = relationship.obligation.saturating_add(2);
    }
    remember_dynasty_interaction(
        state,
        left_dynasty,
        right_dynasty,
        "A dynastic marriage joined the two houses.",
    );
    try_push_outbox(
        state,
        OutboxKind::Family,
        "Dynastic marriage concluded".to_owned(),
        format!(
            "The heirs of dynasties {left_dynasty} and {right_dynasty} entered a marriage compact."
        ),
    )?;
    Ok(())
}

fn update_family_councils(state: &mut AppState) -> Result<(), SimulationError> {
    let loyalty_adjustments: Vec<_> = state
        .family_councils
        .values()
        .map(|council| {
            let mut total_loyalty = 0_u64;
            let mut active_members = 0_u64;
            for character_id in &council.members {
                let character = state
                    .characters
                    .get(*character_id)
                    .expect("family council member must exist");
                if character.status() == crate::core::CharacterStatus::Active {
                    total_loyalty = total_loyalty
                        .saturating_add(u64::from(character.runtime.loyalty_basis_points));
                    active_members = active_members.saturating_add(1);
                }
            }
            let average_loyalty = total_loyalty
                .checked_div(active_members)
                .and_then(|average| u16::try_from(average).ok())
                .unwrap_or(5_000);
            let adjustment = (i32::from(average_loyalty) - 5_000) / 50;
            (council.dynasty_id, adjustment)
        })
        .collect();

    let mut updates = Vec::new();
    for (dynasty_id, loyalty_adjustment) in loyalty_adjustments {
        let council = state
            .family_councils
            .get(&dynasty_id)
            .expect("family council must exist");
        let members = u16::try_from(council.members.len()).unwrap_or(u16::MAX);
        let branch_pressure = i32::from(members.saturating_sub(2).saturating_mul(80));
        let governance_adjustment = match council.governance {
            HouseGovernance::HeadCommand => -200,
            HouseGovernance::Primogeniture => 50,
            HouseGovernance::FamilyPartnership => 250,
            HouseGovernance::BranchFederation => 120,
            HouseGovernance::ElectedHead => -50,
        };
        let unity_basis_points = i32::from(council.unity_basis_points)
            .saturating_sub(branch_pressure)
            .saturating_add(50)
            .saturating_add(loyalty_adjustment)
            .saturating_add(governance_adjustment)
            .clamp(0, 10_000)
            .try_into()
            .expect("clamped family unity must fit u16");
        let governance_change =
            if unity_basis_points < 3_000 && council.governance == HouseGovernance::Primogeniture {
                Some((
                    council.governance,
                    HouseGovernance::FamilyPartnership,
                    next_family_charter_version(dynasty_id, council.charter_version)?,
                ))
            } else {
                None
            };
        updates.push((dynasty_id, unity_basis_points, governance_change));
    }

    let mut governance_changes = Vec::new();
    for (dynasty_id, unity_basis_points, governance_change) in updates {
        let council = state
            .family_councils
            .get_mut(&dynasty_id)
            .expect("family council must exist");
        council.unity_basis_points = unity_basis_points;
        if let Some((prior, governance, next_charter_version)) = governance_change {
            council.governance = governance;
            council.charter_version = next_charter_version;
            governance_changes.push((dynasty_id, prior, governance));
        }
    }
    for (dynasty_id, prior, governance) in governance_changes {
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::HouseGovernanceChange,
            subject: format!("dynasty:{dynasty_id}").into(),
            detail: format!(
                "automatic=true;from={prior:?};governance={governance:?};reason=low_unity"
            )
            .into(),
        });
        try_push_outbox(
            state,
            OutboxKind::Family,
            format!("House {dynasty_id} charter changed under pressure"),
            format!(
                "Low family unity forced a transition from {prior:?} to {governance:?} governance."
            ),
        )?;
    }
    Ok(())
}

pub(crate) fn try_push_outbox(
    state: &mut AppState,
    kind: OutboxKind,
    subject: String,
    body: String,
) -> Result<(), IdentifierAllocationError> {
    let id = state.next_ids.try_outbox()?;
    state.outbox.push(OutboxMessage {
        id,
        day: state.clock.day(),
        kind,
        subject,
        body,
        acknowledged: false,
    });
    Ok(())
}

pub(crate) fn push_outbox(state: &mut AppState, kind: OutboxKind, subject: String, body: String) {
    try_push_outbox(state, kind, subject, body)
        .expect("bootstrap identifier space must be available");
}

#[cfg(test)]
#[path = "strategic_tests.rs"]
mod tests;
