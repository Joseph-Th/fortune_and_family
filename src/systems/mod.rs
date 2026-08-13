//! Canonical validation, decision, commit, and simulation pipelines.

mod bootstrap;
mod commands;
mod invariants;
mod legal;
mod progression;
mod simulation;
mod strategic;
mod transactions;

pub(crate) const WORKERS_PER_BATCH: u16 = 4;
pub(crate) const EMPLOYMENT_RECOVERY_BASIS_POINTS: u16 = 3_000;
pub(crate) const MIN_DISTRICT_RENT_INDEX_BASIS_POINTS: u16 = 7_000;
pub(crate) const MAX_DISTRICT_RENT_INDEX_BASIS_POINTS: u16 = 14_000;
pub(crate) const OFFICE_TERM_DAYS: i64 = 360;
pub(crate) const OFFICE_POWER_ESTABLISHMENT_DAYS: i64 = 120;

pub(crate) fn is_current_weekly_due_day(current_day: i64, due_day: i64) -> bool {
    let Some(latest_weekly_boundary) = current_day.checked_sub(current_day.rem_euclid(7)) else {
        return false;
    };
    due_day != i64::MAX
        && due_day
            .checked_sub(latest_weekly_boundary)
            .is_some_and(|offset| (1..=7).contains(&offset))
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

pub(crate) fn saturating_worker_count(workers: impl Iterator<Item = u32>) -> u32 {
    workers.fold(0_u32, u32::saturating_add)
}

pub(crate) fn available_household_workers(
    state: &crate::core::AppState,
    household_id: crate::ids::HouseholdId,
    excluding_employment_id: Option<crate::ids::EmploymentId>,
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
                && Some(agreement.id) != excluding_employment_id
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
        crate::core::EmploymentStatus::Suspended => matches!(
            business_status,
            crate::core::BusinessStatus::Insolvent | crate::core::BusinessStatus::Closed
        ),
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
            for agreement in state.employment.values_mut().filter(|agreement| {
                agreement.business_id == business_id
                    && agreement.status == crate::core::EmploymentStatus::Suspended
            }) {
                agreement.status = crate::core::EmploymentStatus::Disputed;
            }
        }
        crate::core::BusinessStatus::Insolvent | crate::core::BusinessStatus::Closed => {
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
    }
}

pub use bootstrap::{NewGameError, build_new_game};
#[cfg(test)]
pub(crate) use commands::INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT;
pub(crate) use commands::{
    BUSINESS_POLICY_CHANGE_INTERVAL_DAYS, CIVIC_DEBT_CREDITOR_RESERVE,
    COMMISSIONED_INFORMATION_SOURCE, FAMILY_COUNCIL_MEETING_COST,
    FAMILY_COUNCIL_MEETING_INTERVAL_DAYS, FAMILY_EDUCATION_COST, HEIR_DESIGNATION_INTERVAL_DAYS,
    HEIR_DESIGNATION_LEGITIMACY_COST, HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS,
    INFORMATION_COMMISSION_COST, INFORMATION_COMMISSION_INTERVAL_DAYS, INFORMATION_LEVERAGE_COST,
    INFORMATION_REPORT_LIFETIME_DAYS, INSTITUTION_ENDOWMENT_MAX, INSTITUTION_ENDOWMENT_MIN,
    INSTITUTION_SUPPORT_COST, INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS,
    INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT, LABOR_REPLACEMENT_COST, LAW_LEGITIMACY_REQUIREMENT,
    LAW_SPONSORSHIP_INTERVAL_DAYS, MAX_ACTIVE_SPONSORED_PUBLIC_WORKS, MAX_ACTIVE_WARDS,
    MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER, OFFICE_NOMINATION_DELIVERY_REQUIREMENT,
    OFFICE_NOMINATION_REPUTATION_REQUIREMENT, OFFICE_NOMINATION_RESOLUTION_DAYS,
    OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS, OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST,
    PRIVATE_LOAN_COUNTERPARTY_RESERVE, PROPERTY_COUNTERPARTY_BUYER_RESERVE,
    PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS, WARD_ADOPTION_COST, WARD_ADOPTION_DELIVERY_REQUIREMENT,
    WARD_ADOPTION_INTERVAL_DAYS, WARD_ADOPTION_LEGITIMACY_REQUIREMENT,
    WARD_ADOPTION_REPUTATION_REQUIREMENT, contract_counterparty_price_bounds,
    contract_relationship_pressure_basis_points, family_education_next_day,
    has_established_player_institution_membership, has_established_player_office_power,
    institution_endowment_next_day, institution_membership_count, institution_support_day,
    institution_support_delivery_requirement, institution_support_next_day,
    office_nomination_delivery_requirement, office_nomination_next_day, player_contract_deliveries,
    private_loan_borrower_financing_pressure, quote_information_leverage, quote_player_legal_claim,
    quote_player_legal_settlement, required_office_power_for_law,
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
    LegalClaimQuote, is_valid_legal_hearing_day, quote_grounded_legal_claim,
};
pub(crate) use progression::{
    campaign_phase_is_consistent, campaign_phase_is_persistently_consistent,
    contract_deliveries_for_dynasty, refresh_campaign_phases,
};
pub use simulation::advance_days;
pub(crate) use strategic::MAX_RELATIONSHIP_MEMORIES;
#[cfg(test)]
pub(crate) use strategic::issue_loan;
pub use strategic::{
    BusinessAcquisitionQuote, LoanTerms, PropertyLiquidationQuote, StrategicError,
    SupplyContractTerms, quote_business_acquisition, quote_property_liquidation,
};
pub(crate) use strategic::{
    DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS, STANDARD_CONTRACT_BATCHES_PER_WEEK,
    acquire_business, available_supply_contract_capacity, business_owner_distribution_reserve,
    business_recapitalization_target, buy_unowned_property, capitalize_owned_business,
    crisis_response_contains_crisis, distribute_owned_business_cash,
    dynasty_office_administrative_load, effective_property_weekly_rent, expire_time_limited_state,
    institution_capability_score, projected_dynasty_monthly_office_duty,
    projected_dynasty_monthly_office_duty_with_additional_offices, sell_owned_property,
    validate_loan, validate_supply_contract,
};
pub(crate) use transactions::transfer_business_cash;
pub use transactions::{SimulationError, TimelineError};

#[cfg(test)]
mod tests {
    #[test]
    fn weekly_due_days_stay_inside_the_current_settlement_window() {
        assert!(super::is_current_weekly_due_day(20, 15));
        assert!(super::is_current_weekly_due_day(20, 21));
        assert!(!super::is_current_weekly_due_day(20, 14));
        assert!(!super::is_current_weekly_due_day(20, 22));
        assert!(!super::is_current_weekly_due_day(20, i64::MAX));

        assert!(super::is_current_weekly_due_day(21, 22));
        assert!(super::is_current_weekly_due_day(21, 28));
        assert!(!super::is_current_weekly_due_day(21, 21));
        assert!(!super::is_current_weekly_due_day(i64::MIN, 0));
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
