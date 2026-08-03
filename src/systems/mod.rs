//! Canonical validation, decision, commit, and simulation pipelines.

mod bootstrap;
mod commands;
mod invariants;
mod simulation;
mod strategic;
mod transactions;

pub(crate) const WORKERS_PER_BATCH: u16 = 4;
pub(crate) const EMPLOYMENT_RECOVERY_BASIS_POINTS: u16 = 3_000;

pub(crate) fn supported_worker_capacity(business: &crate::core::Business) -> u32 {
    u32::from(business.operations.capacity_batches_per_day)
        .saturating_mul(u32::from(WORKERS_PER_BATCH))
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
pub(crate) use commands::{
    BUSINESS_POLICY_CHANGE_INTERVAL_DAYS, HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS,
    LABOR_REPLACEMENT_COST, LAW_LEGITIMACY_REQUIREMENT, LAW_SPONSORSHIP_INTERVAL_DAYS,
    LEGAL_CASE_FILING_INTERVAL_DAYS, MAX_ACTIVE_SPONSORED_PUBLIC_WORKS,
    OFFICE_NOMINATION_INTERVAL_DAYS, OFFICE_NOMINATION_REPUTATION_REQUIREMENT,
    PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS,
};
pub use commands::{
    CommandError, CommandOutcome, CrisisResponse, LaborResponse, PlayerCommand,
    apply_player_command,
};
pub use invariants::validate_invariants;
pub use simulation::advance_days;
pub use strategic::{
    BusinessAcquisitionQuote, LoanTerms, StrategicError, SupplyContractTerms, ValidatedLoan,
    ValidatedSupplyContract, acquire_business, buy_unowned_property, issue_loan,
    quote_business_acquisition, sign_supply_contract, validate_loan, validate_supply_contract,
};
pub(crate) use strategic::{STANDARD_CONTRACT_BATCHES_PER_WEEK, initialize_strategic_state};
pub use transactions::{
    SimulationError, ValidatedCashTransfer, transfer_business_cash, validate_business_cash_transfer,
};
