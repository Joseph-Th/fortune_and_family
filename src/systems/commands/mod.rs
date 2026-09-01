//! Canonical `PlayerCommand` schema, dispatch, and shared command plumbing.
//!
//! Purpose: own the single semantic mutation path every caller (CLI,
//! gameplay harness, tests, AI) must use, with typed `CommandError` rejection
//! and atomic commit via `apply_player_command*`.
//! Owns: `PlayerCommand` enum, `CommandOutcome`/`CommandError`, dispatch
//! table, treasury/market spending helpers, audit-cooldown lookups, and
//! re-exports of constants needed by `simulation`/`strategic`.
//! Reads: `Registry` + `AppState` for validation.
//! Mutates: `AppState` only after complete validation; `apply_player_command`
//! clones-then-replaces for failure atomicity, `apply_player_command_scratch`
//! mutates an exclusively owned branch.
//! Does not own: domain policy (each `commands/*.rs` submodule + the
//! `strategic/*.rs` helpers it calls own their own policy).
//! Canonical operations: `PlayerCommand` enum, `apply_player_command` dispatch.
//! Relevant invariants: single dispatch; validation before mutation; atomic clone-then-replace.
//! Focused tests: `src/systems/commands/commands_tests.rs`, gameplay candidate
//! coverage, CLI `execute` smoke.

pub(crate) use super::legal::{
    LEGAL_CASE_FILING_COST, LEGAL_CASE_FILING_INTERVAL_DAYS, LEGAL_CASE_HEARING_DELAY_DAYS,
};
pub(crate) use super::transactions::{
    TimelineError, checked_future_day, credit_market_clearing_account,
    debit_market_clearing_account, next_business_finance_version, next_family_charter_version,
};
pub(crate) use super::{
    LoanTerms, OFFICE_POWER_ESTABLISHMENT_DAYS, StrategicError, SupplyContractTerms,
    acquire_business_scratch, available_supply_contract_capacity, business_recapitalization_target,
    buy_unowned_property, capitalize_owned_business, distribute_owned_business_cash,
    latest_defaulted_loan_for_pair, quote_property_liquidation, sell_owned_property_scratch,
    transfer_business_cash, unresolved_default_owed_elsewhere, validate_loan,
    validate_supply_contract,
};
pub(crate) use crate::core::{
    AppState, AuditKind, AuditRecord, BusinessStatus, Character, CharacterCapabilities,
    CharacterIdentity, CharacterRole, CharacterRuntime, CharacterStatus, ChronicleEntry,
    ChronicleKind, CivicDebt, CivicDebtStatus, ContractStatus, CrisisKind, CrisisStatus,
    DynastyPair, EmploymentAgreement, EmploymentStatus, EnactedLaw, FamilyLink, FamilyLinkKind,
    HouseGovernance, InformationConfidence, InformationReport, InformationTarget,
    InstitutionRuntime, LawKind, LegalCase, LegalCaseKind, LegalCaseStatus, LoanStatus,
    OfficeDirectiveState, OfficePower, OutboxKind, PublicWork, PublicWorkKind, PublicWorkStatus,
};
pub(crate) use crate::ids::{
    BusinessId, CharacterId, ContractId, CrisisId, DistrictId, DynastyId, EmploymentId,
    ExternalRouteId, GoodId, IdentifierAllocationError, InformationReportId, InstitutionId,
    LegalCaseId, OutboxMessageId, PropertyId, PublicWorkId,
};
pub(crate) use crate::money::Money;
pub(crate) use crate::registry::Registry;
pub(crate) use serde::{Deserialize, Serialize};
pub(crate) use std::collections::BTreeSet;
pub(crate) use thiserror::Error;

mod civic;
mod consts;
mod error;
mod family;
mod holdings;
mod information;
mod law;
mod legal_cmd;
mod politics;
mod property_cmd;
mod response;
mod trade;

pub(crate) use civic::*;
pub(crate) use consts::*;
#[allow(clippy::wildcard_imports)]
pub use error::*;
pub(crate) use family::*;
pub(crate) use holdings::*;
pub(crate) use information::*;
pub(crate) use law::*;
pub(crate) use legal_cmd::*;
pub(crate) use politics::*;
pub(crate) use property_cmd::*;
pub(crate) use response::*;
pub(crate) use trade::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrisisResponse {
    Relief,
    Reform,
    Suppress,
    Exploit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaborResponse {
    ImproveConditions,
    Negotiate,
    ReplaceWorkers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EducationFocus {
    Administration,
    Commerce,
    Social,
    Craft,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InformationFocus {
    Market { good_id: GoodId },
    Counterparty { dynasty_id: DynastyId },
    District { district_id: DistrictId },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PlayerCommand {
    TransferBusinessCash {
        from_business_id: BusinessId,
        to_business_id: BusinessId,
        amount: Money,
    },
    WithdrawBusinessCash {
        business_id: BusinessId,
        amount: Money,
    },
    AcquireBusiness {
        business_id: BusinessId,
        manager_id: CharacterId,
        recapitalization: Money,
    },
    InvestInBusiness {
        business_id: BusinessId,
        amount: Money,
    },
    SetBusinessPolicy {
        business_id: BusinessId,
        target_input_days: u16,
        target_output_days: u16,
        minimum_cash_reserve: Money,
        maintenance_basis_points: u16,
        quality_target_basis_points: u16,
    },
    SetBusinessWages {
        business_id: BusinessId,
        weekly_wage_per_worker: Money,
    },
    CreateSupplyContract {
        terms: SupplyContractTerms,
    },
    IssueLoan {
        terms: LoanTerms,
    },
    BuyProperty {
        property_id: PropertyId,
    },
    SellProperty {
        property_id: PropertyId,
        buyer_dynasty_id: DynastyId,
    },
    EnactLaw {
        kind: LawKind,
        value: i64,
    },
    StartPublicWork {
        district_id: DistrictId,
        kind: PublicWorkKind,
        budget: Money,
    },
    FundPublicWork {
        public_work_id: PublicWorkId,
        amount: Money,
    },
    FileLegalCase {
        defendant_dynasty_id: DynastyId,
        kind: LegalCaseKind,
        evidence_basis_points: u16,
        damages: Money,
    },
    SettleLegalCase {
        case_id: LegalCaseId,
    },
    SetHouseGovernance {
        governance: HouseGovernance,
    },
    ConveneFamilyCouncil,
    DesignateHeir {
        character_id: CharacterId,
    },
    AdoptWard {
        focus: EducationFocus,
    },
    EducateFamilyMember {
        character_id: CharacterId,
        focus: EducationFocus,
    },
    CultivateInstitutionSupport {
        institution_id: InstitutionId,
        character_id: CharacterId,
    },
    EndowInstitution {
        institution_id: InstitutionId,
        amount: Money,
    },
    NominateForOffice {
        institution_id: InstitutionId,
        character_id: CharacterId,
    },
    ExerciseOfficePower {
        institution_id: InstitutionId,
        power: OfficePower,
    },
    WithdrawFromInstitution {
        institution_id: InstitutionId,
        character_id: CharacterId,
    },
    RespondToCrisis {
        crisis_id: CrisisId,
        response: CrisisResponse,
    },
    ResolveLaborDispute {
        employment_id: EmploymentId,
        response: LaborResponse,
    },
    CommissionInformation {
        focus: InformationFocus,
    },
    LeverageInformation {
        report_id: InformationReportId,
    },
    AcknowledgeNotification {
        message_id: OutboxMessageId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutcome {
    pub summary: String,
}

impl From<super::strategic::DurableFeedbackError> for CommandError {
    fn from(error: super::strategic::DurableFeedbackError) -> Self {
        match error {
            super::strategic::DurableFeedbackError::IdentifierAllocation(error) => {
                Self::IdentifierAllocation(error)
            }
            super::strategic::DurableFeedbackError::Timeline(error) => Self::Timeline(error),
        }
    }
}

/// Applies a validated player command through the owning subsystem's canonical mutation path.
///
/// # Errors
///
/// Returns a dedicated error when a command references missing records, violates ownership,
/// exceeds available funds, supplies invalid terms, exhausts an identifier space needed for
/// committed records or durable feedback, or cannot represent a required future schedule. Failed
/// commands leave state unchanged.
pub fn apply_player_command(
    registry: &Registry,
    state: &mut AppState,
    command: PlayerCommand,
) -> Result<CommandOutcome, CommandError> {
    if state.scenario_key() != registry.scenario().key() {
        return Err(super::SimulationError::RegistryMismatch {
            state_scenario: state.scenario_key().to_owned(),
            registry_scenario: registry.scenario().key().to_owned(),
        }
        .into());
    }
    let mut candidate = state.clone();
    let outcome = super::apply_player_command_scratch(registry, &mut candidate, command)?;
    *state = candidate;
    Ok(outcome)
}

/// Applies a player command to an exclusively owned scratch campaign.
///
/// Identical to [`apply_player_command`] on success, including post-command
/// expiry, phase refresh, and invariant validation. It skips only the
/// defensive whole-campaign copy: the caller must hold `state` as a
/// disposable working branch whose rejection would simply be discarded.
///
/// The gameplay harness probes candidates on private clones of the live
/// campaign; cloning once per probe instead of twice halves the probing
/// cost without weakening any externally observable guarantee.
pub(crate) fn apply_player_command_scratch(
    registry: &Registry,
    state: &mut AppState,
    command: PlayerCommand,
) -> Result<CommandOutcome, CommandError> {
    if state.scenario_key() != registry.scenario().key() {
        return Err(super::SimulationError::RegistryMismatch {
            state_scenario: state.scenario_key().to_owned(),
            registry_scenario: registry.scenario().key().to_owned(),
        }
        .into());
    }
    dispatch_player_command(registry, state, command).inspect(|_| {
        super::expire_time_limited_state(state);
        super::refresh_campaign_phases(state);
        super::validate_invariants(registry, state);
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "the exhaustive player-command dispatcher keeps canonical command routing visible"
)]
fn dispatch_player_command(
    registry: &Registry,
    state: &mut AppState,
    command: PlayerCommand,
) -> Result<CommandOutcome, CommandError> {
    match command {
        PlayerCommand::TransferBusinessCash {
            from_business_id,
            to_business_id,
            amount,
        } => apply_cash_transfer(state, from_business_id, to_business_id, amount),
        PlayerCommand::WithdrawBusinessCash {
            business_id,
            amount,
        } => apply_business_cash_withdrawal(registry, state, business_id, amount),
        PlayerCommand::AcquireBusiness {
            business_id,
            manager_id,
            recapitalization,
        } => apply_business_acquisition(registry, state, business_id, manager_id, recapitalization),
        PlayerCommand::InvestInBusiness {
            business_id,
            amount,
        } => apply_business_investment(state, business_id, amount),
        PlayerCommand::SetBusinessPolicy {
            business_id,
            target_input_days,
            target_output_days,
            minimum_cash_reserve,
            maintenance_basis_points,
            quality_target_basis_points,
        } => apply_business_policy(
            registry,
            state,
            business_id,
            BusinessPolicyInput {
                target_input_days,
                target_output_days,
                minimum_cash_reserve,
                maintenance_basis_points,
                quality_target_basis_points,
            },
        ),
        PlayerCommand::SetBusinessWages {
            business_id,
            weekly_wage_per_worker,
        } => apply_business_wages(state, business_id, weekly_wage_per_worker),
        PlayerCommand::CreateSupplyContract { terms } => apply_contract(registry, state, &terms),
        PlayerCommand::IssueLoan { terms } => apply_loan(registry, state, &terms),
        PlayerCommand::BuyProperty { property_id } => apply_property_purchase(state, property_id),
        PlayerCommand::SellProperty {
            property_id,
            buyer_dynasty_id,
        } => apply_property_sale(registry, state, property_id, buyer_dynasty_id),
        PlayerCommand::EnactLaw { kind, value } => apply_law(registry, state, kind, value),
        PlayerCommand::StartPublicWork {
            district_id,
            kind,
            budget,
        } => apply_public_work(registry, state, district_id, kind, budget),
        PlayerCommand::FundPublicWork {
            public_work_id,
            amount,
        } => apply_public_work_funding(state, public_work_id, amount),
        PlayerCommand::FileLegalCase {
            defendant_dynasty_id,
            kind,
            evidence_basis_points,
            damages,
        } => apply_legal_case(
            registry,
            state,
            defendant_dynasty_id,
            kind,
            evidence_basis_points,
            damages,
        ),
        PlayerCommand::SettleLegalCase { case_id } => apply_legal_settlement(state, case_id),
        PlayerCommand::SetHouseGovernance { governance } => apply_governance(state, governance),
        PlayerCommand::ConveneFamilyCouncil => apply_family_council_meeting(state),
        PlayerCommand::DesignateHeir { character_id } => apply_heir(state, character_id),
        PlayerCommand::AdoptWard { focus } => apply_adopt_ward(state, focus),
        PlayerCommand::EducateFamilyMember {
            character_id,
            focus,
        } => apply_family_education(state, character_id, focus),
        PlayerCommand::CultivateInstitutionSupport {
            institution_id,
            character_id,
        } => apply_institution_support(registry, state, institution_id, character_id),
        PlayerCommand::EndowInstitution {
            institution_id,
            amount,
        } => apply_institution_endowment(state, institution_id, amount),
        PlayerCommand::NominateForOffice {
            institution_id,
            character_id,
        } => apply_office_nomination(registry, state, institution_id, character_id),
        PlayerCommand::ExerciseOfficePower {
            institution_id,
            power,
        } => apply_office_power_directive(registry, state, institution_id, power),
        PlayerCommand::WithdrawFromInstitution {
            institution_id,
            character_id,
        } => apply_institution_withdrawal(state, institution_id, character_id),
        PlayerCommand::RespondToCrisis {
            crisis_id,
            response,
        } => apply_crisis_response(registry, state, crisis_id, response),
        PlayerCommand::ResolveLaborDispute {
            employment_id,
            response,
        } => apply_labor_response(state, employment_id, response),
        PlayerCommand::CommissionInformation { focus } => {
            commission_information(registry, state, focus)
        }
        PlayerCommand::LeverageInformation { report_id } => {
            leverage_information(registry, state, report_id)
        }
        PlayerCommand::AcknowledgeNotification { message_id } => acknowledge(state, message_id),
    }
}

fn ensure_owned_business(state: &AppState, business_id: BusinessId) -> Result<(), CommandError> {
    let business = state
        .businesses
        .get(business_id)
        .ok_or(CommandError::MissingBusiness { business_id })?;
    if business.owner_dynasty_id() != state.player_dynasty_id {
        return Err(CommandError::BusinessNotOwned { business_id });
    }
    Ok(())
}

/// Owned-business gate for operating controls. Rescue capital
/// (`InvestInBusiness`) is the only player lever an insolvent firm still
/// accepts; policy, wages, and labor responses stay locked while insolvency or
/// closure runs and unlock again on recovery.
fn ensure_operable_owned_business(
    state: &AppState,
    business_id: BusinessId,
) -> Result<(), CommandError> {
    ensure_owned_business(state, business_id)?;
    if state.businesses.get(business_id).is_some_and(|business| {
        matches!(
            business.status(),
            BusinessStatus::Insolvent | BusinessStatus::Closed
        )
    }) {
        return Err(CommandError::Strategic(StrategicError::BusinessInactive {
            business_id,
        }));
    }
    Ok(())
}

fn future_day_or_terminal(day: i64, offset_days: i64) -> i64 {
    checked_future_day(day, offset_days).unwrap_or(i64::MAX)
}

/// Most recent day an audit record of `kind` whose subject satisfies
/// `subject_matches` was appended, scanning from newest to oldest.
fn latest_audit_day(
    state: &AppState,
    kind: AuditKind,
    subject_matches: impl Fn(&str) -> bool,
) -> Option<i64> {
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == kind && subject_matches(record.subject()))
        .map(AuditRecord::day)
}

/// Most recent day of `kind` matching `subject_matches` inside an
/// `interval_days` cooldown, scanning from newest to oldest.
///
/// A record older than the cooldown can never reject an action (`day +
/// interval <= today`), and audit days never decrease (an enforced
/// invariant), so the scan stops exactly at the window boundary instead of
/// walking the whole campaign history. Only the presence of an in-window
/// record decides the cooldown, so callers must use this for rejection tests
/// rather than for reporting the underlying record's day.
fn latest_cooldown_audit_day(
    state: &AppState,
    kind: AuditKind,
    interval_days: i64,
    subject_matches: impl Fn(&str) -> bool,
) -> Option<i64> {
    let earliest_day = state.clock.day().saturating_sub(interval_days - 1);
    state
        .audit_log
        .iter()
        .rev()
        .take_while(|record| record.day() >= earliest_day)
        .find(|record| record.kind() == kind && subject_matches(record.subject()))
        .map(AuditRecord::day)
}

fn latest_audit_day_for_subject(state: &AppState, kind: AuditKind, subject: &str) -> Option<i64> {
    latest_audit_day(state, kind, |record_subject| record_subject == subject)
}

/// Cooldown-window counterpart to the plain latest-record lookup; see
/// [`latest_cooldown_audit_day`] for the scan-boundary argument.
fn latest_character_campaign_day_in_cooldown(
    state: &AppState,
    kind: AuditKind,
    interval_days: i64,
    character_id: CharacterId,
) -> Option<i64> {
    let earliest_day = state.clock.day().saturating_sub(interval_days - 1);
    state
        .audit_log
        .iter()
        .rev()
        .take_while(|record| record.day() >= earliest_day)
        .find(|record| {
            record.kind() == kind
                && record
                    .audit_subject()
                    .institution_character_ids()
                    .is_some_and(|(_, recorded_character_id)| recorded_character_id == character_id)
        })
        .map(AuditRecord::day)
}

pub(crate) fn player_contract_deliveries(state: &AppState) -> u32 {
    super::contract_deliveries_for_dynasty(state, state.player_dynasty_id)
}

fn spend_business_cash(
    state: &mut AppState,
    business_id: BusinessId,
    amount: Money,
) -> Result<(), CommandError> {
    let business = state
        .businesses
        .get(business_id)
        .ok_or(CommandError::MissingBusiness { business_id })?;
    let cash = business.cash();
    // Player-driven business spending honors the same operating-reserve floor the
    // business's own daily purchase and production decisions honor.
    let spendable = business_operating_spendable_cash(business);
    if spendable < amount {
        return Err(CommandError::InsufficientBusinessFunds {
            business_id,
            available: spendable,
            required: amount,
        });
    }
    let resulting_lifetime_costs =
        business
            .finance
            .lifetime_costs
            .checked_add(amount)
            .ok_or(CommandError::Simulation(
                super::SimulationError::BusinessLifetimeCostsOverflow {
                    business_id,
                    current: business.finance.lifetime_costs,
                    incoming: amount,
                },
            ))?;
    let next_finance_version = next_business_finance_version(business)?;
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("validated business must exist");
    business.finance.cash = cash
        .checked_sub(amount)
        .expect("validated business spend must fit available cash");
    business.finance.lifetime_costs = resulting_lifetime_costs;
    business.finance.version = next_finance_version;
    Ok(())
}

fn spend_player_treasury(state: &mut AppState, amount: Money) -> Result<(), CommandError> {
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    if treasury < amount {
        return Err(CommandError::InsufficientPlayerFunds {
            available: treasury,
            required: amount,
        });
    }
    state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .treasury = treasury
        .checked_sub(amount)
        .expect("validated dynasty spend must fit available treasury");
    Ok(())
}

/// Debits the player treasury for an unmodeled service cost — legal,
/// educational, informational, or crisis-mobilization work outside the modeled
/// business economy — and credits the same amount to the market clearing
/// account, so the payment keeps a credited counterparty instead of vanishing
/// from the economy. Flows whose counterparty is a named record (court filing
/// fees, institutional endowments, public-work construction) debit through
/// [`spend_player_treasury`] and credit that record directly.
fn spend_player_treasury_to_market(
    state: &mut AppState,
    amount: Money,
) -> Result<(), CommandError> {
    spend_player_treasury(state, amount)?;
    credit_market_clearing_account(state, amount)?;
    Ok(())
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
