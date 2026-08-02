//! Canonical validation, decision, commit, and simulation pipelines.

mod bootstrap;
mod commands;
mod invariants;
mod simulation;
mod strategic;
mod transactions;

pub(crate) const WORKERS_PER_BATCH: u16 = 4;

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

pub use bootstrap::{NewGameError, build_new_game};
pub(crate) use commands::BUSINESS_POLICY_CHANGE_INTERVAL_DAYS;
pub use commands::{
    CommandError, CommandOutcome, CrisisResponse, LaborResponse, PlayerCommand,
    apply_player_command,
};
pub use invariants::validate_invariants;
pub use simulation::advance_days;
pub(crate) use strategic::initialize_strategic_state;
pub use strategic::{
    BusinessAcquisitionQuote, LoanTerms, StrategicError, SupplyContractTerms, ValidatedLoan,
    ValidatedSupplyContract, acquire_business, buy_unowned_property, issue_loan,
    quote_business_acquisition, sign_supply_contract, validate_loan, validate_supply_contract,
};
pub use transactions::{SimulationError, ValidatedCashTransfer, transfer_business_cash};
