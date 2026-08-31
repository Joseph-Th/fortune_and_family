//! Systems facade: canonical validation → decision → commit pipelines.
//!
//! Purpose: wire the eight subsystem families (`bootstrap | commands |
//! invariants | legal | progression | simulation | strategic | transactions`)
//! behind one import surface, expose shared constants/helpers
//! (`capacity_weighted_route_disruption`, `DailyCapacityScratch`,
//! `OfficePower::*` derivation, scheduling predicates), and re-export the
//! handful of symbols the CLI, gameplay harness, and projection need. This is
//! the single discovery point for system ownership.
//! Owns: subsystem module wiring and the shared scheduling/employment/route
//! math that multiple domains read (weekly settleability, directive expiry,
//! rent index bounds, worker-capacity helpers).
//! Reads/Mutates: as its submodules (this file itself owns no domain state;
//! re-exports are pass-through).
//! Does not own: any single domain's mutation (each subsystem owns its own
//! validation → commit path).
//! Canonical operations: `build_new_game` (bootstrap), `apply_player_command`
//! / `apply_player_command_scratch` (commands), `advance_days` /
//! `advance_days_scratch` (simulation), persistence `validate_invariants`;
//! scheduling helpers `is_settleable_weekly_due_day` and
//! `is_valid_*` used by persistence validation.
//! Relevant invariants: `OFFICE_TERM_DAYS` / `OFFICE_VACANCY_RETRY_DAYS` and
//! weekly settleability are canonical — callers reuse them rather than
//! re-deriving; route disruption weighting is centralized so household income,
//! crises, and trade share one formula.
//! Focused tests: as submodules (`simulation_tests`, `strategic_tests`,
//! `commands_tests`) plus facade unit tests for scheduling predicates.

use crate::ids::{CharacterId, RecipeId};
use crate::registry::Registry;
use std::collections::BTreeMap;

mod bootstrap;
mod commands;
mod invariants;
mod legal;
mod progression;
pub(crate) mod simulation;
mod strategic;
mod transactions;

pub(crate) use strategic::active_law_value;

pub(crate) const WORKERS_PER_BATCH: u16 = 4;
pub(crate) const EMPLOYMENT_RECOVERY_BASIS_POINTS: u16 = 3_000;
pub(crate) const MIN_DISTRICT_RENT_INDEX_BASIS_POINTS: u16 = 7_000;
pub(crate) const MAX_DISTRICT_RENT_INDEX_BASIS_POINTS: u16 = 14_000;
/// Shared tool-share of weekly spend that becomes market tool demand (25%).
/// Production and civic construction share the same material intensity so
/// the economy has one canonical industrial tool ratio.
pub(crate) const TOOL_SHARE_BASIS_POINTS: i64 = 2_500;
pub(crate) const OFFICE_TERM_DAYS: i64 = 360;
/// A temporarily officeless institution retries its election on this cadence
/// instead of locking its office — and its powers and stipend flow — away for
/// a full term.
pub(crate) const OFFICE_VACANCY_RETRY_DAYS: i64 = 30;
pub(crate) const OFFICE_POWER_ESTABLISHMENT_DAYS: i64 = 120;

/// Returns whether a scheduled day can still arrive: the terminal sentinel is
/// never schedulable.
pub(crate) const fn is_schedulable_day(day: i64) -> bool {
    day != i64::MAX
}

/// Returns whether a repayment-active schedule's due day is validly settleable
/// at `current_day`.
///
/// Weekly obligations settle at global week boundaries. A schedule signed
/// mid-week stores its nominal `signed + 7` date and keeps that private
/// cadence: once it is overdue it settles at every weekly boundary and each
/// settlement advances the due day by seven from the settled date, so the
/// schedule's phase stays anchored to its signing instead of snapping to the
/// global boundary. A valid due day may therefore sit up to two weeks ahead of
/// the current week's start. Anything overdue (at or before the current week's
/// start) or beyond the coming fortnight indicates corrupted scheduling.
pub(crate) fn is_settleable_weekly_due_day(current_day: i64, due_day: i64) -> bool {
    let Some(latest_weekly_boundary) = current_day.checked_sub(current_day.rem_euclid(7)) else {
        return false;
    };
    due_day != i64::MAX
        && due_day
            .checked_sub(latest_weekly_boundary)
            .is_some_and(|offset| (1..=14).contains(&offset))
}

pub(crate) fn is_valid_active_directive_expiry(current_day: i64, expires_day: i64) -> bool {
    expires_day
        .checked_sub(current_day)
        .is_some_and(|remaining| (0..=OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS).contains(&remaining))
}

pub(crate) fn is_valid_information_report_dates(
    current_day: i64,
    created_day: i64,
    expires_day: i64,
) -> bool {
    created_day <= current_day
        && expires_day >= current_day
        && expires_day != i64::MAX
        && expires_day
            .checked_sub(created_day)
            .is_some_and(|lifetime| (0..=INFORMATION_REPORT_LIFETIME_DAYS).contains(&lifetime))
}

pub(crate) fn is_valid_institution_selection_day(
    term_started_day: i64,
    next_selection_day: i64,
) -> bool {
    next_selection_day != i64::MAX
        && next_selection_day
            .checked_sub(term_started_day)
            .is_some_and(|offset| (0..=OFFICE_TERM_DAYS).contains(&offset))
}

/// Quality a guild-trained master can sustain above the default policy
/// target: charter membership carries craft training that shows in the work.
/// Kept deliberately small because Rivergate's incumbent dynasty heads hold
/// several charters each, so a large bonus would uniformly enrich every
/// rival house and drain the private credit market of borrowers.
pub(crate) const GUILD_CRAFT_QUALITY_TARGET_BONUS: u16 = 100;

/// Market-access share a non-member keeps per unit of active
/// `GuildEntryRestriction` value: at the maximum 10,000 restriction an
/// outsider still places three quarters of what a chartered member does.
pub(crate) const GUILD_RESTRICTION_OUTSIDER_DIVISOR: i64 = 4;

/// Whether a business's manager holds membership in the guild that charters
/// the business's trade. Guild standing is trade-specific, so membership in
/// any other institution never substitutes for the chartered guild.
pub(crate) fn manager_holds_chartered_guild_membership(
    registry: &Registry,
    state: &crate::core::AppState,
    recipe_id: RecipeId,
    manager_id: CharacterId,
) -> bool {
    registry
        .guild_for_recipe(recipe_id)
        .and_then(|institution_id| state.institutions.get(&institution_id))
        .is_some_and(|institution| institution.members.contains(&manager_id))
}

/// Capacity-weighted disruption across active external routes, mirroring
/// household and import-trade availability. Centralized so crisis detection,
/// household income, and import-trade throttling share one weighting.
#[must_use]
pub(crate) fn capacity_weighted_route_disruption(state: &crate::core::AppState) -> u16 {
    if state.external_routes.is_empty() {
        return 0;
    }
    let active: Vec<_> = state
        .external_routes
        .values()
        .filter(|route| route.active)
        .collect();
    if active.is_empty() {
        return 10_000;
    }
    let mut total_weighted: u64 = 0;
    let mut total_capacity_milli: u64 = 0;
    for route in &active {
        let disruption = u64::from(route.disruption_basis_points);
        let capacity_milli = u64::try_from(route.daily_capacity.milliunits().max(0)).unwrap_or(0);
        if capacity_milli == 0 {
            continue;
        }
        total_weighted = total_weighted.saturating_add(disruption.saturating_mul(capacity_milli));
        total_capacity_milli = total_capacity_milli.saturating_add(capacity_milli);
    }
    if total_capacity_milli == 0 {
        let total: u32 = active
            .iter()
            .map(|route| u32::from(route.disruption_basis_points))
            .sum();
        let count = u32::try_from(active.len()).unwrap_or(u32::MAX);
        return u16::try_from(total / count.max(1)).unwrap_or(10_000);
    }
    u16::try_from(total_weighted / total_capacity_milli).unwrap_or(10_000)
}

pub(crate) fn institution_powers_for(
    kind: crate::registry::InstitutionKind,
) -> std::collections::BTreeSet<crate::core::OfficePower> {
    use crate::core::OfficePower;
    use crate::registry::InstitutionKind;

    let values: &[OfficePower] = match kind {
        InstitutionKind::CraftGuild => &[OfficePower::Licenses, OfficePower::Inspections],
        InstitutionKind::MerchantGuild => &[OfficePower::CityContracts, OfficePower::MarketTolls],
        InstitutionKind::Council => &[
            OfficePower::Taxation,
            OfficePower::PublicWorks,
            OfficePower::CityContracts,
        ],
        InstitutionKind::Court => &[OfficePower::DebtEnforcement],
        InstitutionKind::Watch => &[OfficePower::WatchPriorities],
        InstitutionKind::Treasury => &[OfficePower::Taxation, OfficePower::CityContracts],
        InstitutionKind::Charity => &[OfficePower::EmergencyImports],
        InstitutionKind::MarketOffice => &[
            OfficePower::Inspections,
            OfficePower::MarketTolls,
            OfficePower::EmergencyImports,
        ],
    };
    values.iter().copied().collect()
}

pub(crate) fn supported_worker_capacity(business: &crate::core::Business) -> u32 {
    u32::from(business.operations.capacity_batches_per_day)
        .saturating_mul(u32::from(WORKERS_PER_BATCH))
}

/// Output quantity a business holds back to keep its target-stock policy
/// covered before any surplus is planned for the market.
#[must_use]
pub(crate) fn business_policy_reserve(
    business: &crate::core::Business,
    output_quantity: crate::money::Quantity,
) -> crate::money::Quantity {
    let reserve_batches = i64::from(business.operations.capacity_batches_per_day)
        .saturating_mul(i64::from(business.policy.target_output_days));
    output_quantity.saturating_mul_ratio(reserve_batches, 1)
}

/// How much more of one good the market can absorb before stock exceeds the
/// 150%-of-target placement ceiling.
#[must_use]
pub(crate) fn market_absorption_capacity(
    state: &crate::core::AppState,
    good_id: crate::ids::GoodId,
) -> crate::money::Quantity {
    state
        .market
        .quotes
        .get(&good_id)
        .map_or(crate::money::Quantity::ZERO, |quote| {
            quote
                .target_stock
                .saturating_mul_ratio(3, 2)
                .saturating_sub(quote.stock)
                .max(crate::money::Quantity::ZERO)
        })
}

/// Derives public-work completion progress in basis points from spent funds
/// against budget. The single canonical derivation: mutation sites write it
/// and both validation layers read the identical expression.
#[must_use]
pub(crate) fn public_work_progress_basis_points(
    spent: crate::money::Money,
    budget: crate::money::Money,
) -> u16 {
    let ratio = if budget.copper() > 0 {
        spent.saturating_mul_ratio(10_000, budget.copper()).copper()
    } else {
        0
    };
    u16::try_from(ratio.clamp(0, 10_000)).expect("clamped public-work progress must fit u16")
}

/// Per-day capacity inputs resolved once for all businesses instead of
/// rescanning employment agreements, contracts, and institutions once per
/// business per planning phase.
///
/// Business statuses do not change between the purchase, production, and
/// sale planning phases (lifecycle evaluation runs after all three), so a
/// collection taken at the start of a phase answers identically to the
/// per-business scans it replaces. Stores are ordered, so collection and
/// lookups stay deterministic.
pub(crate) struct DailyCapacityScratch {
    /// Effective workforce per business slot: active crews at full strength,
    /// disputed crews at half strength, suspended and ended crews absent,
    /// summed before the workers-per-batch division.
    worker_capacity_workers: Vec<u32>,
    /// Weekly contracted output owed by each business per good. Distressed
    /// sellers hold no contract reserve: they cannibalize their commitments
    /// to survive, so scheduled deliveries can genuinely fail and buyers who
    /// rely on a struggling supplier face real shortage risk.
    contract_reserves:
        BTreeMap<(crate::ids::BusinessId, crate::ids::GoodId), crate::money::Quantity>,
    office_administrative_loads: BTreeMap<crate::ids::DynastyId, u16>,
}

impl DailyCapacityScratch {
    pub(crate) fn collect(state: &crate::core::AppState) -> Self {
        let business_slots = state
            .businesses
            .records()
            .keys()
            .next_back()
            .map_or(0, |id| id.value() as usize + 1);
        let mut worker_capacity_workers = vec![0_u32; business_slots];
        for agreement in state.employment.values() {
            let weighted = match agreement.status {
                crate::core::EmploymentStatus::Active => u32::from(agreement.workers),
                // A disputed crew works at half strength; an odd worker sits
                // out with the even half rather than rounding up to full
                // capacity, which would make disputes free for small crews.
                crate::core::EmploymentStatus::Disputed => u32::from(agreement.workers) / 2,
                crate::core::EmploymentStatus::Suspended | crate::core::EmploymentStatus::Ended => {
                    0
                }
            };
            let slot = agreement.business_id.value() as usize;
            if let Some(capacity) = worker_capacity_workers.get_mut(slot) {
                *capacity = capacity.saturating_add(weighted);
            }
        }
        let mut contract_reserves = BTreeMap::new();
        for contract in state.contracts.values() {
            if contract.status != crate::core::ContractStatus::Active {
                continue;
            }
            let Some(seller) = state.businesses.get(contract.seller_business_id) else {
                continue;
            };
            if seller.status() == crate::core::BusinessStatus::Distressed {
                continue;
            }
            let entry = contract_reserves
                .entry((contract.seller_business_id, contract.good_id))
                .or_insert(crate::money::Quantity::ZERO);
            *entry = entry.saturating_add(contract.quantity_per_week);
        }
        let office_administrative_loads = strategic::dynasty_office_administrative_loads(state);
        Self {
            worker_capacity_workers,
            contract_reserves,
            office_administrative_loads,
        }
    }

    /// Batch ceiling from the workforce recorded at collection time.
    pub(crate) fn worker_limited_batches(&self, business_id: crate::ids::BusinessId) -> u16 {
        let workers = self
            .worker_capacity_workers
            .get(business_id.value() as usize)
            .copied()
            .unwrap_or(0);
        u16::try_from(workers / u32::from(WORKERS_PER_BATCH)).unwrap_or(u16::MAX)
    }

    /// Weekly contracted output owed by one business for one good.
    pub(crate) fn business_contract_reserve(
        &self,
        business_id: crate::ids::BusinessId,
        good_id: crate::ids::GoodId,
    ) -> crate::money::Quantity {
        self.contract_reserves
            .get(&(business_id, good_id))
            .copied()
            .unwrap_or(crate::money::Quantity::ZERO)
    }

    /// Institutional office load recorded at collection time.
    pub(crate) fn office_administrative_load(&self, dynasty_id: crate::ids::DynastyId) -> u16 {
        self.office_administrative_loads
            .get(&dynasty_id)
            .copied()
            .unwrap_or(0)
    }
}

pub(crate) fn saturating_worker_count(workers: impl Iterator<Item = u32>) -> u32 {
    workers.fold(0_u32, u32::saturating_add)
}

pub(crate) fn available_household_workers(
    state: &crate::core::AppState,
    household_id: crate::ids::HouseholdId,
) -> u32 {
    let members = state
        .households
        .get(household_id)
        .map_or(0, |household| u32::from(household.members()));
    let assigned = state
        .employment
        .values()
        .filter(|agreement| {
            agreement.household_id == household_id
                && agreement.status != crate::core::EmploymentStatus::Ended
        })
        .fold(0_u32, |total, agreement| {
            total.saturating_add(u32::from(agreement.workers))
        });
    members.saturating_sub(assigned)
}

pub(crate) const fn is_employment_status_compatible(
    business_status: crate::core::BusinessStatus,
    employment_status: crate::core::EmploymentStatus,
) -> bool {
    match employment_status {
        crate::core::EmploymentStatus::Active | crate::core::EmploymentStatus::Disputed => {
            matches!(
                business_status,
                crate::core::BusinessStatus::Active | crate::core::BusinessStatus::Distressed
            )
        }
        // Suspension is the reversible state an insolvent employer's crews
        // hold while recovery remains possible. Closure is terminal, so a
        // closed firm's agreements end outright and release their workers
        // back to the household labor pool.
        crate::core::EmploymentStatus::Suspended => {
            matches!(business_status, crate::core::BusinessStatus::Insolvent)
        }
        crate::core::EmploymentStatus::Ended => true,
    }
}

pub(crate) fn synchronize_employment_for_business_status(
    state: &mut crate::core::AppState,
    business_id: crate::ids::BusinessId,
    business_status: crate::core::BusinessStatus,
) {
    match business_status {
        crate::core::BusinessStatus::Active | crate::core::BusinessStatus::Distressed => {
            // Resumed operation recalls every withheld crew as disputed:
            // suspended crews return while their insolvent employer
            // rehabilitates, and ended crews return when an explicit
            // acquisition reopens a closed firm under a new owner.
            for agreement in state.employment.values_mut().filter(|agreement| {
                agreement.business_id == business_id
                    && matches!(
                        agreement.status,
                        crate::core::EmploymentStatus::Suspended
                            | crate::core::EmploymentStatus::Ended
                    )
            }) {
                agreement.status = crate::core::EmploymentStatus::Disputed;
            }
        }
        crate::core::BusinessStatus::Insolvent => {
            for agreement in state.employment.values_mut().filter(|agreement| {
                agreement.business_id == business_id
                    && matches!(
                        agreement.status,
                        crate::core::EmploymentStatus::Active
                            | crate::core::EmploymentStatus::Disputed
                    )
            }) {
                agreement.status = crate::core::EmploymentStatus::Suspended;
            }
        }
        // Closure ends the current agreements so crews are immediately
        // released to other employers. A later explicit acquisition may
        // reopen the firm and rebuild staffing through that acquisition path.
        crate::core::BusinessStatus::Closed => {
            for agreement in state.employment.values_mut().filter(|agreement| {
                agreement.business_id == business_id
                    && agreement.status != crate::core::EmploymentStatus::Ended
            }) {
                agreement.status = crate::core::EmploymentStatus::Ended;
            }
        }
    }
}

pub use bootstrap::{NewGameError, build_new_game};
#[cfg(test)]
pub(crate) use commands::INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT;

/// Canonical `heir={id}` component of a `HeirDesignation` audit detail. The
/// writer and every reader share this one format, so reformatting the record
/// cannot silently break heir cooldowns or succession preparation checks.
#[must_use]
pub(crate) fn heir_designation_detail_component(character_id: crate::ids::CharacterId) -> String {
    format!("heir={character_id}")
}

/// Whether a `HeirDesignation` audit record names `character_id` as heir.
#[must_use]
pub(crate) fn heir_audit_detail_matches(
    record: &crate::core::AuditRecord,
    character_id: crate::ids::CharacterId,
) -> bool {
    record
        .detail()
        .split(';')
        .any(|part| part == heir_designation_detail_component(character_id))
}

/// Canonical detail marking an institution withdrawal that resigned a held
/// office. The writer and reader share this one constant.
pub(crate) const OFFICE_RESIGNATION_AUDIT_DETAIL: &str = "resigned_office=true";

pub(crate) use commands::apply_player_command_scratch;
pub(crate) use commands::{
    BUSINESS_POLICY_CHANGE_INTERVAL_DAYS, BUSINESS_WAGE_CHANGE_INTERVAL_DAYS,
    CIVIC_DEBT_CREDITOR_RESERVE, COMMISSIONED_INFORMATION_SOURCE, CRISIS_REFORM_COST,
    CRISIS_SUPPRESS_COST, CRISIS_SUPPRESS_LEGITIMACY_COST, FAMILY_COUNCIL_MEETING_COST,
    FAMILY_COUNCIL_MEETING_INTERVAL_DAYS, FAMILY_EDUCATION_COST, HEIR_DESIGNATION_INTERVAL_DAYS,
    HEIR_DESIGNATION_LEGITIMACY_COST, HEIR_DESIGNATION_UNITY_COST,
    HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS, HOUSE_GOVERNANCE_UNITY_COST,
    INFORMATION_COMMISSION_COST, INFORMATION_COMMISSION_INTERVAL_DAYS, INFORMATION_LEVERAGE_COST,
    INFORMATION_REPORT_LIFETIME_DAYS, INSTITUTION_ENDOWMENT_MAX, INSTITUTION_ENDOWMENT_MIN,
    INSTITUTION_SUPPORT_COST, INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS,
    INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT, LABOR_CONDITIONS_IMPROVEMENT_COST,
    LABOR_NEGOTIATION_COST, LABOR_REPLACEMENT_COST, LAW_LEGITIMACY_REQUIREMENT,
    LAW_SPONSORSHIP_COST, LAW_SPONSORSHIP_INTERVAL_DAYS, MAX_ACTIVE_SPONSORED_PUBLIC_WORKS,
    MAX_ACTIVE_WARDS, MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER, MAX_WEEKLY_WAGE_PER_WORKER,
    OFFICE_NOMINATION_CAMPAIGN_COST, OFFICE_NOMINATION_DELIVERY_REQUIREMENT,
    OFFICE_NOMINATION_REPUTATION_REQUIREMENT, OFFICE_NOMINATION_RESOLUTION_DAYS,
    OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS, OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST,
    PRIVATE_LOAN_COUNTERPARTY_RESERVE, PROPERTY_COUNTERPARTY_BUYER_RESERVE,
    PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS, WARD_ADOPTION_COST, WARD_ADOPTION_DELIVERY_REQUIREMENT,
    WARD_ADOPTION_INTERVAL_DAYS, WARD_ADOPTION_LEGITIMACY_REQUIREMENT,
    WARD_ADOPTION_REPUTATION_REQUIREMENT, WARD_ADOPTION_UNITY_COST, active_player_ward_count,
    business_operating_spendable_cash, contract_counterparty_price_bounds,
    contract_relationship_pressure_basis_points, crisis_relief_cost, family_education_next_day,
    has_established_player_institution_membership, has_established_player_office_power,
    has_player_office, institution_endowment_next_day, institution_membership_count,
    institution_support_day, institution_support_delivery_requirement,
    institution_support_next_day, office_nomination_delivery_requirement,
    office_nomination_next_day, player_contract_deliveries,
    private_loan_borrower_financing_pressure, public_work_initial_contribution,
    quote_information_leverage, quote_player_legal_claim, quote_player_legal_settlement,
    required_office_power_for_law,
};
pub use commands::{
    CommandError, CommandOutcome, CrisisResponse, EducationFocus, InformationFocus, LaborResponse,
    PlayerCommand, PublicWorkFundingError, apply_player_command,
};
#[cfg(test)]
pub(crate) use commands::{
    FAMILY_EDUCATION_INTERVAL_DAYS, INSTITUTION_SUPPORT_INTERVAL_DAYS,
    INSTITUTION_WITHDRAWAL_RECOVERY_DAYS,
};
pub use invariants::validate_invariants;
pub(crate) use legal::{
    LEGAL_CASE_FILING_COST, LEGAL_CASE_FILING_INTERVAL_DAYS, LEGAL_CASE_HEARING_DELAY_DAYS,
    LegalClaimQuote, collect_court_filing_fee, court_filing_fee_headroom,
    is_valid_legal_hearing_day, quote_grounded_legal_claim,
};
pub(crate) use progression::{
    campaign_phase_is_consistent, campaign_phases_are_consistent, contract_deliveries_for_dynasty,
    refresh_campaign_phases,
};
pub use simulation::advance_days;
pub(crate) use simulation::{advance_days_scratch, business_sustainable_unit_cost};
pub(crate) use strategic::MAX_RELATIONSHIP_MEMORIES;
#[cfg(test)]
pub(crate) use strategic::issue_loan;
pub use strategic::{
    BusinessAcquisitionQuote, LoanTerms, PropertyLiquidationQuote, StrategicError,
    SupplyContractTerms, quote_business_acquisition, quote_property_liquidation,
};
pub(crate) use strategic::{
    CRISIS_RESPONSE_WINDOW_DAYS, STANDARD_CONTRACT_BATCHES_PER_WEEK, acquire_business_scratch,
    available_supply_contract_capacity, business_owner_distribution_reserve,
    business_recapitalization_target, buy_unowned_property, capitalize_owned_business,
    credit_pair_blocks_new_loan, crisis_response_contains_crisis,
    defaulted_loan_restructuring_available, distribute_owned_business_cash,
    district_unrest_pressures, dynasty_office_administrative_load, effective_property_weekly_rent,
    expire_time_limited_state, institution_capability_score, latest_defaulted_loan_for_pair,
    market_reference_weekly_wage, projected_dynasty_monthly_office_duty,
    projected_dynasty_monthly_office_duty_with_additional_offices, sell_owned_property_scratch,
    unresolved_default_owed_elsewhere, validate_loan, validate_supply_contract,
};
pub(crate) use transactions::transfer_business_cash;
pub use transactions::{SimulationError, TimelineError};

#[cfg(test)]
pub(crate) use strategic::DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS;

#[cfg(test)]
mod tests {
    #[test]
    fn weekly_due_days_stay_inside_the_settleable_fortnight() {
        // Boundary-aligned schedules always sit exactly one week ahead.
        assert!(super::is_settleable_weekly_due_day(20, 21));
        assert!(super::is_settleable_weekly_due_day(21, 28));

        // A nominal one-week date signed mid-week is valid until its first
        // boundary settlement snaps it onto the global cadence.
        assert!(super::is_settleable_weekly_due_day(20, 22));
        assert!(super::is_settleable_weekly_due_day(20, 27));
        assert!(!super::is_settleable_weekly_due_day(20, 28 + 14));

        // Overdue and unschedulable dates stay invalid.
        assert!(!super::is_settleable_weekly_due_day(20, 21 - 7));
        assert!(!super::is_settleable_weekly_due_day(20, i64::MAX));
        assert!(!super::is_settleable_weekly_due_day(i64::MIN, 0));
    }

    #[test]
    fn institution_selection_days_cannot_extend_a_term() {
        assert!(super::is_valid_institution_selection_day(0, 0));
        assert!(super::is_valid_institution_selection_day(
            0,
            super::OFFICE_TERM_DAYS
        ));
        assert!(!super::is_valid_institution_selection_day(
            0,
            super::OFFICE_TERM_DAYS + 1
        ));
        assert!(!super::is_valid_institution_selection_day(1, 0));
        assert!(!super::is_valid_institution_selection_day(0, i64::MAX));
        assert!(!super::is_valid_institution_selection_day(i64::MIN, 0));
    }

    #[test]
    fn time_limited_strategic_state_rejects_extended_lifetimes() {
        assert!(super::is_valid_active_directive_expiry(
            10,
            10 + super::OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS
        ));
        assert!(!super::is_valid_active_directive_expiry(
            10,
            11 + super::OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS
        ));
        assert!(!super::is_valid_active_directive_expiry(10, 9));

        assert!(super::is_valid_information_report_dates(
            100,
            10,
            10 + super::INFORMATION_REPORT_LIFETIME_DAYS
        ));
        assert!(!super::is_valid_information_report_dates(
            100,
            10,
            11 + super::INFORMATION_REPORT_LIFETIME_DAYS
        ));
        assert!(!super::is_valid_information_report_dates(100, 101, 101));
        assert!(!super::is_valid_information_report_dates(100, 10, i64::MAX));
    }

    #[test]
    fn worker_count_saturates_instead_of_wrapping() {
        assert_eq!(
            super::saturating_worker_count([u32::MAX, 1].into_iter()),
            u32::MAX
        );
    }
}
