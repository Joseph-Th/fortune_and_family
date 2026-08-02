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

pub use bootstrap::{NewGameError, build_new_game};
pub use commands::{
    CommandError, CommandOutcome, CrisisResponse, LaborResponse, PlayerCommand,
    apply_player_command,
};
pub use invariants::validate_invariants;
pub use simulation::advance_days;
pub(crate) use strategic::initialize_strategic_state;
pub use strategic::{
    LoanTerms, StrategicError, SupplyContractTerms, ValidatedLoan, ValidatedSupplyContract,
    buy_unowned_property, issue_loan, sign_supply_contract, validate_loan,
    validate_supply_contract,
};
pub use transactions::{SimulationError, ValidatedCashTransfer, transfer_business_cash};
