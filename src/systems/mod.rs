//! Canonical validation, decision, commit, and simulation pipelines.

mod bootstrap;
mod commands;
mod invariants;
mod simulation;
mod strategic;
mod transactions;

pub use bootstrap::build_new_game;
pub use commands::{
    CommandError, CommandOutcome, CrisisResponse, LaborResponse, PlayerCommand,
    apply_player_command,
};
pub use invariants::validate_invariants;
pub use simulation::advance_days;
pub(crate) use strategic::initialize_strategic_state;
pub use strategic::{
    LoanTerms, StrategicError, SupplyContractTerms, ValidatedLoan, ValidatedSupplyContract,
    buy_unowned_property, create_supply_contract, issue_loan, validate_loan,
    validate_supply_contract,
};
pub use transactions::{SimulationError, ValidatedCashTransfer, transfer_business_cash};
