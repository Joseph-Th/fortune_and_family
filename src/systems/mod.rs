//! Canonical validation, decision, commit, and simulation pipelines.

mod bootstrap;
mod commands;
mod invariants;
mod simulation;
mod strategic;
mod transactions;

pub(crate) const WORKERS_PER_BATCH: u16 = 4;
pub(crate) const EMPLOYMENT_RECOVERY_BASIS_POINTS: u16 = 3_000;
pub(crate) const MIN_DISTRICT_RENT_INDEX_BASIS_POINTS: u16 = 7_000;
pub(crate) const MAX_DISTRICT_RENT_INDEX_BASIS_POINTS: u16 = 14_000;
pub(crate) const OFFICE_TERM_DAYS: i64 = 360;
pub(crate) const OFFICE_POWER_ESTABLISHMENT_DAYS: i64 = 120;

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
pub(crate) use commands::INSTITUTION_SUPPORT_INTERVAL_DAYS;
pub(crate) use commands::{
    BUSINESS_POLICY_CHANGE_INTERVAL_DAYS, CIVIC_DEBT_CREDITOR_RESERVE,
    COMMISSIONED_INFORMATION_SOURCE, FAMILY_COUNCIL_MEETING_COST,
    FAMILY_COUNCIL_MEETING_INTERVAL_DAYS, FAMILY_EDUCATION_COST, FAMILY_EDUCATION_INTERVAL_DAYS,
    HEIR_DESIGNATION_INTERVAL_DAYS, HEIR_DESIGNATION_LEGITIMACY_COST,
    HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS, INFORMATION_COMMISSION_COST,
    INFORMATION_COMMISSION_INTERVAL_DAYS, INFORMATION_LEVERAGE_COST, INSTITUTION_SUPPORT_COST,
    INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT, INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS,
    INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT, LABOR_REPLACEMENT_COST, LAW_LEGITIMACY_REQUIREMENT,
    LAW_SPONSORSHIP_INTERVAL_DAYS, LEGAL_CASE_FILING_COST, LEGAL_CASE_FILING_INTERVAL_DAYS,
    MAX_ACTIVE_SPONSORED_PUBLIC_WORKS, MAX_ACTIVE_WARDS, MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER,
    OFFICE_NOMINATION_DELIVERY_REQUIREMENT, OFFICE_NOMINATION_REPUTATION_REQUIREMENT,
    OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS, OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST,
    PRIVATE_LOAN_COUNTERPARTY_RESERVE, PROPERTY_COUNTERPARTY_BUYER_RESERVE,
    PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS, WARD_ADOPTION_COST, WARD_ADOPTION_DELIVERY_REQUIREMENT,
    WARD_ADOPTION_INTERVAL_DAYS, WARD_ADOPTION_LEGITIMACY_REQUIREMENT,
    WARD_ADOPTION_REPUTATION_REQUIREMENT, contract_counterparty_price_bounds,
    contract_relationship_pressure_basis_points, has_established_player_office_power,
    institution_membership_count, institution_support_day, institution_support_next_day,
    office_nomination_delivery_requirement, office_nomination_next_day, player_contract_deliveries,
    quote_information_leverage, required_office_power_for_law,
};
pub use commands::{
    CommandError, CommandOutcome, CrisisResponse, EducationFocus, InformationFocus, LaborResponse,
    PlayerCommand, apply_player_command,
};
pub use invariants::validate_invariants;
pub use simulation::advance_days;
pub(crate) use strategic::MAX_RELATIONSHIP_MEMORIES;
pub use strategic::{
    BusinessAcquisitionQuote, LoanTerms, PropertyLiquidationQuote, StrategicError,
    SupplyContractTerms, ValidatedLoan, ValidatedSupplyContract, acquire_business,
    buy_unowned_property, issue_loan, quote_business_acquisition, quote_property_liquidation,
    sell_owned_property, sign_supply_contract, validate_loan, validate_supply_contract,
};
pub(crate) use strategic::{
    DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS, STANDARD_CONTRACT_BATCHES_PER_WEEK,
    available_supply_contract_capacity, business_recapitalization_target,
    capitalize_owned_business, crisis_response_contains_crisis, dynasty_office_administrative_load,
    initialize_strategic_state, institution_capability_score,
    projected_dynasty_monthly_office_duty,
};
pub use transactions::{
    SimulationError, ValidatedCashTransfer, transfer_business_cash, validate_business_cash_transfer,
};

#[cfg(test)]
mod tests {
    #[test]
    fn worker_count_saturates_instead_of_wrapping() {
        assert_eq!(
            super::saturating_worker_count([u32::MAX, 1].into_iter()),
            u32::MAX
        );
    }
}
