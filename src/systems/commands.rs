//! Canonical player-command validation and dispatch across simulation subsystems.

use super::{
    LoanTerms, StrategicError, SupplyContractTerms, acquire_business, buy_unowned_property,
    issue_loan, sign_supply_contract, transfer_business_cash,
};
use crate::core::{
    AppState, AuditKind, AuditRecord, BusinessStatus, CrisisStatus, EmploymentStatus, EnactedLaw,
    HouseGovernance, LawKind, LegalCase, LegalCaseKind, LegalCaseStatus, OutboxKind, PublicWork,
    PublicWorkKind, PublicWorkStatus,
};
use crate::ids::{
    BusinessId, CharacterId, CrisisId, DistrictId, DynastyId, EmploymentId, InstitutionId,
    OutboxMessageId, PropertyId,
};
use crate::money::Money;
use crate::registry::Registry;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerCommand {
    TransferBusinessCash {
        from_business_id: BusinessId,
        to_business_id: BusinessId,
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
    CreateSupplyContract {
        terms: SupplyContractTerms,
    },
    IssueLoan {
        terms: LoanTerms,
    },
    BuyProperty {
        property_id: PropertyId,
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
    FileLegalCase {
        defendant_dynasty_id: DynastyId,
        kind: LegalCaseKind,
        evidence_basis_points: u16,
        damages: Money,
    },
    SetHouseGovernance {
        governance: HouseGovernance,
    },
    NominateForOffice {
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
    AcknowledgeNotification {
        message_id: OutboxMessageId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutcome {
    pub summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BusinessPolicyInput {
    target_input_days: u16,
    target_output_days: u16,
    minimum_cash_reserve: Money,
    maintenance_basis_points: u16,
    quality_target_basis_points: u16,
}

pub(crate) const BUSINESS_POLICY_CHANGE_INTERVAL_DAYS: i64 = 180;
pub(crate) const LEGAL_CASE_FILING_INTERVAL_DAYS: i64 = 90;
pub(crate) const LAW_SPONSORSHIP_INTERVAL_DAYS: i64 = 360;
pub(crate) const LAW_LEGITIMACY_REQUIREMENT: u16 = 3_000;
pub(crate) const LAW_LEGITIMACY_COST: u16 = 250;
pub(crate) const PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS: i64 = 360;
pub(crate) const MAX_ACTIVE_SPONSORED_PUBLIC_WORKS: usize = 2;
pub(crate) const LABOR_REPLACEMENT_COST: Money = Money::from_copper(750);
pub(crate) const HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS: i64 = 1_800;
pub(crate) const OFFICE_NOMINATION_INTERVAL_DAYS: i64 = 180;
pub(crate) const OFFICE_NOMINATION_REPUTATION_REQUIREMENT: u16 = 5_500;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CommandError {
    #[error(transparent)]
    Strategic(#[from] StrategicError),
    #[error(transparent)]
    Simulation(#[from] super::SimulationError),
    #[error("business {business_id} does not exist")]
    MissingBusiness { business_id: BusinessId },
    #[error("business {business_id} is not owned by the player dynasty")]
    BusinessNotOwned { business_id: BusinessId },
    #[error("command does not involve the player dynasty")]
    PlayerNotParty,
    #[error("business policy values are outside supported ranges")]
    InvalidBusinessPolicy,
    #[error("business {business_id} already uses the requested operating policy")]
    UnchangedBusinessPolicy { business_id: BusinessId },
    #[error(
        "business {business_id} cannot change operating policy again before day {next_change_day}"
    )]
    BusinessPolicyCooldown {
        business_id: BusinessId,
        next_change_day: i64,
    },
    #[error("business investment must be positive")]
    InvalidBusinessInvestment,
    #[error("law {kind:?} does not support value {value}")]
    InvalidLawValue { kind: LawKind, value: i64 },
    #[error("law {kind:?} is already active with value {value}")]
    UnchangedLaw { kind: LawKind, value: i64 },
    #[error("law {kind:?} is not implemented by the current simulation")]
    UnsupportedLaw { kind: LawKind },
    #[error("the player dynasty must hold political office before sponsoring a law")]
    LawSponsorshipRequiresOffice,
    #[error("the player dynasty cannot sponsor another law before day {next_enactment_day}")]
    LawCooldown { next_enactment_day: i64 },
    #[error("district {district_id} does not exist")]
    MissingDistrict { district_id: DistrictId },
    #[error("dynasty {dynasty_id} does not exist")]
    MissingDynasty { dynasty_id: DynastyId },
    #[error("player treasury has {available}, but command requires {required}")]
    InsufficientPlayerFunds { available: Money, required: Money },
    #[error("player legitimacy is {available}, but command requires {required}")]
    InsufficientPlayerLegitimacy { available: u16, required: u16 },
    #[error("business {business_id} has {available}, but command requires {required}")]
    InsufficientBusinessFunds {
        business_id: BusinessId,
        available: Money,
        required: Money,
    },
    #[error("public-work budget must be positive")]
    InvalidPublicWorkBudget,
    #[error("the player dynasty must hold political office before sponsoring a public work")]
    PublicWorkSponsorshipRequiresOffice,
    #[error("an unfinished {kind:?} public work already exists in district {district_id}")]
    DuplicateActivePublicWork {
        district_id: DistrictId,
        kind: PublicWorkKind,
    },
    #[error("the player dynasty cannot sponsor another public work before day {next_start_day}")]
    PublicWorkCooldown { next_start_day: i64 },
    #[error(
        "the player dynasty already has {active} unfinished public works, the maximum is {maximum}"
    )]
    PublicWorkCapacity { active: usize, maximum: usize },
    #[error("legal case cannot target the player dynasty")]
    SameLegalParty,
    #[error("legal evidence or damages are invalid")]
    InvalidLegalTerms,
    #[error("an unresolved {kind:?} case against dynasty {defendant_dynasty_id} already exists")]
    DuplicateActiveLegalCase {
        defendant_dynasty_id: DynastyId,
        kind: LegalCaseKind,
    },
    #[error("the player dynasty cannot file another legal case before day {next_filing_day}")]
    LegalCaseCooldown { next_filing_day: i64 },
    #[error("family council for dynasty {dynasty_id} does not exist")]
    MissingFamilyCouncil { dynasty_id: DynastyId },
    #[error("house governance is already {governance:?}")]
    UnchangedHouseGovernance { governance: HouseGovernance },
    #[error("house governance cannot change again before day {next_change_day}")]
    HouseGovernanceCooldown { next_change_day: i64 },
    #[error(
        "player reputation is too weak for an office campaign: quality {quality}, reliability {reliability}, required {required}"
    )]
    InsufficientOfficeReputation {
        quality: u16,
        reliability: u16,
        required: u16,
    },
    #[error(
        "the player dynasty cannot launch another office campaign before day {next_nomination_day}"
    )]
    OfficeNominationCooldown { next_nomination_day: i64 },
    #[error("institution {institution_id} does not exist")]
    MissingInstitution { institution_id: InstitutionId },
    #[error("character {character_id} is already a member of institution {institution_id}")]
    AlreadyInstitutionMember {
        institution_id: InstitutionId,
        character_id: CharacterId,
    },
    #[error("character {character_id} is not an active member of the player dynasty")]
    InvalidNominee { character_id: CharacterId },
    #[error("character {character_id} already holds office in institution {institution_id}")]
    NomineeAlreadyHoldsOffice {
        character_id: CharacterId,
        institution_id: InstitutionId,
    },
    #[error("crisis {crisis_id} does not exist")]
    MissingCrisis { crisis_id: CrisisId },
    #[error("crisis {crisis_id} is no longer active")]
    InactiveCrisis { crisis_id: CrisisId },
    #[error("crisis {crisis_id} already has a committed player response")]
    CrisisAlreadyAddressed { crisis_id: CrisisId },
    #[error("employment agreement {employment_id} does not exist")]
    MissingEmployment { employment_id: EmploymentId },
    #[error("employment agreement {employment_id} is not a player labor dispute")]
    InvalidLaborDispute { employment_id: EmploymentId },
    #[error(
        "district {district_id} has no replacement household able to supply {workers} workers for employment {employment_id}"
    )]
    NoReplacementLaborAvailable {
        employment_id: EmploymentId,
        district_id: DistrictId,
        workers: u16,
    },
    #[error("notification {message_id} does not exist")]
    MissingNotification { message_id: OutboxMessageId },
}

/// Applies a validated player command through the owning subsystem's canonical mutation path.
///
/// # Errors
///
/// Returns a dedicated error when a command references missing records, violates ownership,
/// exceeds available funds, or supplies invalid terms. Failed commands leave state unchanged.
pub fn apply_player_command(
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
        PlayerCommand::CreateSupplyContract { terms } => {
            ensure_player_contract_party(state, &terms)?;
            let id = sign_supply_contract(registry, state, terms)?;
            Ok(CommandOutcome {
                summary: format!("Created supply contract {id}."),
            })
        }
        PlayerCommand::IssueLoan { terms } => {
            ensure_player_loan_party(state, &terms)?;
            let id = issue_loan(state, terms)?;
            Ok(CommandOutcome {
                summary: format!("Issued loan {id}."),
            })
        }
        PlayerCommand::BuyProperty { property_id } => {
            buy_unowned_property(state, state.player_dynasty_id, property_id)?;
            Ok(CommandOutcome {
                summary: format!("Acquired property {property_id}."),
            })
        }
        PlayerCommand::EnactLaw { kind, value } => apply_law(state, kind, value),
        PlayerCommand::StartPublicWork {
            district_id,
            kind,
            budget,
        } => apply_public_work(registry, state, district_id, kind, budget),
        PlayerCommand::FileLegalCase {
            defendant_dynasty_id,
            kind,
            evidence_basis_points,
            damages,
        } => apply_legal_case(
            state,
            defendant_dynasty_id,
            kind,
            evidence_basis_points,
            damages,
        ),
        PlayerCommand::SetHouseGovernance { governance } => {
            apply_house_governance(state, governance)
        }
        PlayerCommand::NominateForOffice {
            institution_id,
            character_id,
        } => apply_office_nomination(state, institution_id, character_id),
        PlayerCommand::RespondToCrisis {
            crisis_id,
            response,
        } => apply_crisis_response(state, crisis_id, response),
        PlayerCommand::ResolveLaborDispute {
            employment_id,
            response,
        } => apply_labor_response(state, employment_id, response),
        PlayerCommand::AcknowledgeNotification { message_id } => {
            apply_acknowledgement(state, message_id)
        }
    }
}

fn apply_business_acquisition(
    registry: &Registry,
    state: &mut AppState,
    business_id: BusinessId,
    manager_id: CharacterId,
    recapitalization: Money,
) -> Result<CommandOutcome, CommandError> {
    let quote = acquire_business(
        registry,
        state,
        state.player_dynasty_id,
        business_id,
        manager_id,
        recapitalization,
    )?;
    Ok(CommandOutcome {
        summary: format!(
            "Acquired business {business_id} for {} with {} working capital.",
            quote.purchase_price, recapitalization
        ),
    })
}

fn apply_cash_transfer(
    state: &mut AppState,
    from_business_id: BusinessId,
    to_business_id: BusinessId,
    amount: Money,
) -> Result<CommandOutcome, CommandError> {
    ensure_owned_business(state, from_business_id)?;
    ensure_owned_business(state, to_business_id)?;
    transfer_business_cash(state, from_business_id, to_business_id, amount)?;
    super::strategic::push_outbox(
        state,
        OutboxKind::Finance,
        format!("Portfolio cash moved to business {to_business_id}"),
        format!(
            "The dynasty transferred {amount} from business {from_business_id} to business {to_business_id}."
        ),
    );
    Ok(CommandOutcome {
        summary: format!(
            "Transferred {amount} from business {from_business_id} to business {to_business_id}."
        ),
    })
}

fn apply_business_investment(
    state: &mut AppState,
    business_id: BusinessId,
    amount: Money,
) -> Result<CommandOutcome, CommandError> {
    ensure_owned_business(state, business_id)?;
    if amount <= Money::ZERO {
        return Err(CommandError::InvalidBusinessInvestment);
    }
    let business = state
        .businesses
        .get(business_id)
        .expect("owned business must exist");
    if business.status() == BusinessStatus::Closed {
        return Err(CommandError::Strategic(StrategicError::BusinessInactive {
            business_id,
        }));
    }
    let resulting_cash = business.cash().checked_add(amount).ok_or_else(|| {
        CommandError::Simulation(super::SimulationError::BusinessCashOverflow {
            business_id,
            current: business.cash(),
            incoming: amount,
        })
    })?;
    spend_player_treasury(state, amount)?;
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("validated business must exist");
    business.finance.cash = resulting_cash;
    business.finance.version = business.finance.version.saturating_add(1);
    let rehabilitation = u16::try_from((amount.copper() / 2).clamp(0, 3_000))
        .expect("bounded rehabilitation must fit u16");
    business.operations.condition_basis_points = business
        .operations
        .condition_basis_points
        .saturating_add(rehabilitation)
        .min(10_000);
    business.operations.quality_basis_points = business
        .operations
        .quality_basis_points
        .saturating_add(rehabilitation / 2)
        .min(10_000);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::BusinessCapitalization,
        subject: format!("business:{business_id}"),
        detail: format!(
            "amount={};rehabilitation_basis_points={rehabilitation}",
            amount.copper()
        ),
    });
    super::strategic::push_outbox(
        state,
        OutboxKind::Finance,
        format!("Business {business_id} capitalized"),
        format!(
            "The dynasty invested {amount} into the enterprise, restoring {rehabilitation} basis points of operating condition."
        ),
    );
    Ok(CommandOutcome {
        summary: format!(
            "Invested {amount} in business {business_id} and restored {rehabilitation} basis points of condition."
        ),
    })
}

fn apply_business_policy(
    state: &mut AppState,
    business_id: BusinessId,
    input: BusinessPolicyInput,
) -> Result<CommandOutcome, CommandError> {
    let BusinessPolicyInput {
        target_input_days,
        target_output_days,
        minimum_cash_reserve,
        maintenance_basis_points,
        quality_target_basis_points,
    } = input;
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
    if target_input_days > 30
        || target_output_days > 30
        || minimum_cash_reserve.is_negative()
        || maintenance_basis_points > 10_000
        || quality_target_basis_points > 10_000
    {
        return Err(CommandError::InvalidBusinessPolicy);
    }
    let business = state
        .businesses
        .get(business_id)
        .expect("validated business must exist");
    if business.policy.target_input_days == target_input_days
        && business.policy.target_output_days == target_output_days
        && business.policy.minimum_cash_reserve == minimum_cash_reserve
        && business.policy.maintenance_basis_points == maintenance_basis_points
        && business.policy.quality_target_basis_points == quality_target_basis_points
    {
        return Err(CommandError::UnchangedBusinessPolicy { business_id });
    }
    let subject = format!("business:{business_id}");
    if let Some(last_change_day) = state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::BusinessPolicyChange && record.subject() == subject
        })
        .map(AuditRecord::day)
    {
        let next_change_day = last_change_day.saturating_add(BUSINESS_POLICY_CHANGE_INTERVAL_DAYS);
        if state.clock.day() < next_change_day {
            return Err(CommandError::BusinessPolicyCooldown {
                business_id,
                next_change_day,
            });
        }
    }
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("validated business must exist");
    business.policy.target_input_days = target_input_days;
    business.policy.target_output_days = target_output_days;
    business.policy.minimum_cash_reserve = minimum_cash_reserve;
    business.policy.maintenance_basis_points = maintenance_basis_points;
    business.policy.quality_target_basis_points = quality_target_basis_points;
    business.finance.version = business.finance.version.saturating_add(1);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::BusinessPolicyChange,
        subject,
        detail: format!(
            "input_days={target_input_days}; output_days={target_output_days}; reserve={}; maintenance={maintenance_basis_points}; quality={quality_target_basis_points}",
            minimum_cash_reserve.copper()
        ),
    });
    super::strategic::push_outbox(
        state,
        OutboxKind::Finance,
        format!("Business {business_id} operating policy updated"),
        format!(
            "The enterprise now targets {target_input_days} input days, {target_output_days} output days, a {minimum_cash_reserve} cash reserve, {maintenance_basis_points} maintenance basis points, and {quality_target_basis_points} quality basis points."
        ),
    );
    Ok(CommandOutcome {
        summary: format!("Updated operating policy for business {business_id}."),
    })
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

fn ensure_player_contract_party(
    state: &AppState,
    terms: &SupplyContractTerms,
) -> Result<(), CommandError> {
    let buyer =
        state
            .businesses
            .get(terms.buyer_business_id)
            .ok_or(CommandError::MissingBusiness {
                business_id: terms.buyer_business_id,
            })?;
    let seller =
        state
            .businesses
            .get(terms.seller_business_id)
            .ok_or(CommandError::MissingBusiness {
                business_id: terms.seller_business_id,
            })?;
    if buyer.owner_dynasty_id() != state.player_dynasty_id
        && seller.owner_dynasty_id() != state.player_dynasty_id
    {
        return Err(CommandError::PlayerNotParty);
    }
    Ok(())
}

fn ensure_player_loan_party(state: &AppState, terms: &LoanTerms) -> Result<(), CommandError> {
    if terms.lender_dynasty_id != state.player_dynasty_id
        && terms.borrower_dynasty_id != state.player_dynasty_id
    {
        return Err(CommandError::PlayerNotParty);
    }
    Ok(())
}

fn apply_law(
    state: &mut AppState,
    kind: LawKind,
    value: i64,
) -> Result<CommandOutcome, CommandError> {
    if !kind.is_implemented() {
        return Err(CommandError::UnsupportedLaw { kind });
    }
    if !kind.is_value_valid(value) {
        return Err(CommandError::InvalidLawValue { kind, value });
    }
    if state
        .laws
        .values()
        .any(|law| law.active && law.kind == kind && law.value == value)
    {
        return Err(CommandError::UnchangedLaw { kind, value });
    }
    if let Some(last_enactment_day) = state
        .laws
        .values()
        .filter(|law| law.sponsor_dynasty_id == Some(state.player_dynasty_id))
        .map(|law| law.enacted_day)
        .max()
    {
        let next_enactment_day = last_enactment_day.saturating_add(LAW_SPONSORSHIP_INTERVAL_DAYS);
        if state.clock.day() < next_enactment_day {
            return Err(CommandError::LawCooldown { next_enactment_day });
        }
    }
    let legitimacy = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .legitimacy_basis_points;
    if legitimacy < LAW_LEGITIMACY_REQUIREMENT {
        return Err(CommandError::InsufficientPlayerLegitimacy {
            available: legitimacy,
            required: LAW_LEGITIMACY_REQUIREMENT,
        });
    }
    if !has_player_office(state) {
        return Err(CommandError::LawSponsorshipRequiresOffice);
    }
    let cost = Money::from_copper(2_000);
    spend_player_treasury(state, cost)?;
    state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .legitimacy_basis_points = legitimacy.saturating_sub(LAW_LEGITIMACY_COST);
    for law in state
        .laws
        .values_mut()
        .filter(|law| law.kind == kind && law.active)
    {
        law.active = false;
    }
    let id = state.next_ids.law();
    state.laws.insert(
        id,
        EnactedLaw {
            id,
            kind,
            enacted_day: state.clock.day(),
            sponsor_dynasty_id: Some(state.player_dynasty_id),
            value,
            active: true,
        },
    );
    super::strategic::push_outbox(
        state,
        OutboxKind::Law,
        format!("Law {id} enacted"),
        format!("The player dynasty sponsored {kind:?} with value {value}."),
    );
    Ok(CommandOutcome {
        summary: format!("Enacted law {id}: {kind:?}."),
    })
}

fn apply_public_work(
    registry: &Registry,
    state: &mut AppState,
    district_id: DistrictId,
    kind: PublicWorkKind,
    budget: Money,
) -> Result<CommandOutcome, CommandError> {
    if registry.get_district(district_id).is_none() {
        return Err(CommandError::MissingDistrict { district_id });
    }
    if budget <= Money::ZERO {
        return Err(CommandError::InvalidPublicWorkBudget);
    }
    if state.public_works.values().any(|work| {
        work.district_id == district_id
            && work.kind == kind
            && matches!(
                work.status,
                PublicWorkStatus::Building | PublicWorkStatus::Suspended
            )
    }) {
        return Err(CommandError::DuplicateActivePublicWork { district_id, kind });
    }
    let active_sponsored = state
        .public_works
        .values()
        .filter(|work| {
            work.sponsor_dynasty_id == Some(state.player_dynasty_id)
                && matches!(
                    work.status,
                    PublicWorkStatus::Building | PublicWorkStatus::Suspended
                )
        })
        .count();
    if active_sponsored >= MAX_ACTIVE_SPONSORED_PUBLIC_WORKS {
        return Err(CommandError::PublicWorkCapacity {
            active: active_sponsored,
            maximum: MAX_ACTIVE_SPONSORED_PUBLIC_WORKS,
        });
    }
    let subject = format!("dynasty:{}", state.player_dynasty_id);
    if let Some(last_start_day) = state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::PublicWorkStarted && record.subject() == subject)
        .map(AuditRecord::day)
    {
        let next_start_day = last_start_day.saturating_add(PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS);
        if state.clock.day() < next_start_day {
            return Err(CommandError::PublicWorkCooldown { next_start_day });
        }
    }
    if !has_player_office(state) {
        return Err(CommandError::PublicWorkSponsorshipRequiresOffice);
    }
    let contribution = Money::from_copper((budget.copper() / 10).max(1)).min(budget);
    spend_player_treasury(state, contribution)?;
    let progress_basis_points = u16::try_from(
        contribution
            .saturating_mul_ratio(10_000, budget.copper())
            .copper(),
    )
    .unwrap_or(10_000)
    .min(10_000);
    let id = state.next_ids.public_work();
    state.public_works.insert(
        id,
        PublicWork {
            id,
            district_id,
            kind,
            sponsor_dynasty_id: Some(state.player_dynasty_id),
            budget,
            spent: contribution,
            progress_basis_points,
            status: PublicWorkStatus::Building,
        },
    );
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::PublicWorkStarted,
        subject,
        detail: format!(
            "district={};kind={kind:?};budget={};contribution={}",
            district_id.value(),
            budget.copper(),
            contribution.copper()
        ),
    });
    super::strategic::push_outbox(
        state,
        OutboxKind::Politics,
        format!("Public work {id} started"),
        format!("Construction began on a {kind:?} project in district {district_id}."),
    );
    Ok(CommandOutcome {
        summary: format!("Started public work {id}."),
    })
}

fn has_player_office(state: &AppState) -> bool {
    state.institutions.values().any(|institution| {
        institution.office_holder_id.is_some_and(|character_id| {
            state
                .characters
                .get(character_id)
                .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
        })
    })
}

fn apply_legal_case(
    state: &mut AppState,
    defendant_dynasty_id: DynastyId,
    kind: LegalCaseKind,
    evidence_basis_points: u16,
    damages: Money,
) -> Result<CommandOutcome, CommandError> {
    if defendant_dynasty_id == state.player_dynasty_id {
        return Err(CommandError::SameLegalParty);
    }
    if !state.dynasties.contains_key(&defendant_dynasty_id) {
        return Err(CommandError::MissingDynasty {
            dynasty_id: defendant_dynasty_id,
        });
    }
    if evidence_basis_points > 10_000 || damages.is_negative() {
        return Err(CommandError::InvalidLegalTerms);
    }
    if state.legal_cases.values().any(|legal_case| {
        legal_case.plaintiff_dynasty_id == state.player_dynasty_id
            && legal_case.defendant_dynasty_id == defendant_dynasty_id
            && legal_case.kind == kind
            && matches!(
                legal_case.status,
                LegalCaseStatus::Filed | LegalCaseStatus::Hearing
            )
    }) {
        return Err(CommandError::DuplicateActiveLegalCase {
            defendant_dynasty_id,
            kind,
        });
    }
    if let Some(last_filing_day) = state
        .legal_cases
        .values()
        .filter(|legal_case| legal_case.plaintiff_dynasty_id == state.player_dynasty_id)
        .map(|legal_case| legal_case.filed_day)
        .max()
    {
        let next_filing_day = last_filing_day.saturating_add(LEGAL_CASE_FILING_INTERVAL_DAYS);
        if state.clock.day() < next_filing_day {
            return Err(CommandError::LegalCaseCooldown { next_filing_day });
        }
    }
    spend_player_treasury(state, Money::from_copper(300))?;
    let id = state.next_ids.legal_case();
    state.legal_cases.insert(
        id,
        LegalCase {
            id,
            plaintiff_dynasty_id: state.player_dynasty_id,
            defendant_dynasty_id,
            kind,
            evidence_basis_points,
            public_attention_basis_points: 1_500,
            filed_day: state.clock.day(),
            hearing_day: state.clock.day().saturating_add(60),
            damages,
            status: LegalCaseStatus::Filed,
        },
    );
    super::strategic::adjust_dynasty_relationship(
        state,
        state.player_dynasty_id,
        defendant_dynasty_id,
        super::strategic::RelationshipDelta::new(-100, -30, 0, 150, 0),
    );
    super::strategic::push_outbox(
        state,
        OutboxKind::Legal,
        format!("Legal case {id} filed"),
        format!("A {kind:?} case was filed against dynasty {defendant_dynasty_id}."),
    );
    Ok(CommandOutcome {
        summary: format!("Filed legal case {id}."),
    })
}

fn apply_house_governance(
    state: &mut AppState,
    governance: HouseGovernance,
) -> Result<CommandOutcome, CommandError> {
    let dynasty_id = state.player_dynasty_id;
    let council = state
        .family_councils
        .get(&dynasty_id)
        .ok_or(CommandError::MissingFamilyCouncil { dynasty_id })?;
    if council.governance == governance {
        return Err(CommandError::UnchangedHouseGovernance { governance });
    }
    let subject = format!("dynasty:{dynasty_id}");
    if let Some(last_change_day) = state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::HouseGovernanceChange && record.subject() == subject
        })
        .map(AuditRecord::day)
    {
        let next_change_day = last_change_day.saturating_add(HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS);
        if state.clock.day() < next_change_day {
            return Err(CommandError::HouseGovernanceCooldown { next_change_day });
        }
    }
    let council = state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("validated family council must exist");
    council.governance = governance;
    council.charter_version = council.charter_version.saturating_add(1);
    council.unity_basis_points = council.unity_basis_points.saturating_sub(250);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::HouseGovernanceChange,
        subject,
        detail: format!("governance={governance:?}"),
    });
    super::strategic::push_outbox(
        state,
        OutboxKind::Family,
        "House charter amended".to_owned(),
        format!(
            "The dynasty adopted {governance:?} governance, changing administrative coordination, family cohesion, and succession risk."
        ),
    );
    Ok(CommandOutcome {
        summary: format!("Changed house governance to {governance:?}."),
    })
}

fn apply_office_nomination(
    state: &mut AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Result<CommandOutcome, CommandError> {
    let character = state
        .characters
        .get(character_id)
        .ok_or(CommandError::InvalidNominee { character_id })?;
    if character.dynasty_id() != state.player_dynasty_id
        || character.status() != crate::core::CharacterStatus::Active
    {
        return Err(CommandError::InvalidNominee { character_id });
    }
    if let Some(existing_institution_id) = state
        .institutions
        .values()
        .find(|institution| institution.office_holder_id == Some(character_id))
        .map(|institution| institution.institution_id)
    {
        return Err(CommandError::NomineeAlreadyHoldsOffice {
            character_id,
            institution_id: existing_institution_id,
        });
    }
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    if institution.members.contains(&character_id) {
        return Err(CommandError::AlreadyInstitutionMember {
            institution_id,
            character_id,
        });
    }
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let quality = player.resources.reputation_quality_basis_points;
    let reliability = player.resources.reputation_reliability_basis_points;
    if quality.max(reliability) < OFFICE_NOMINATION_REPUTATION_REQUIREMENT {
        return Err(CommandError::InsufficientOfficeReputation {
            quality,
            reliability,
            required: OFFICE_NOMINATION_REPUTATION_REQUIREMENT,
        });
    }
    if let Some(last_nomination_day) = state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::OfficeNomination)
        .map(AuditRecord::day)
    {
        let next_nomination_day =
            last_nomination_day.saturating_add(OFFICE_NOMINATION_INTERVAL_DAYS);
        if state.clock.day() < next_nomination_day {
            return Err(CommandError::OfficeNominationCooldown {
                next_nomination_day,
            });
        }
    }
    let campaign_cost = Money::from_copper(300);
    spend_player_treasury(state, campaign_cost)?;
    let selection_day = state.clock.day().saturating_add(60);
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    institution.members.insert(character_id);
    institution.next_selection_day = institution.next_selection_day.min(selection_day);
    let dynasty = state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.legitimacy_basis_points = dynasty
        .resources
        .legitimacy_basis_points
        .saturating_add(150)
        .min(10_000);
    let subject = office_nomination_subject(institution_id, character_id);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::OfficeNomination,
        subject,
        detail: format!("campaign_cost={}", campaign_cost.copper()),
    });
    super::strategic::push_outbox(
        state,
        OutboxKind::Politics,
        format!("Office campaign launched for character {character_id}"),
        format!(
            "The dynasty nominated character {character_id} to institution {institution_id}; selection is scheduled by day {selection_day}."
        ),
    );
    Ok(CommandOutcome {
        summary: format!("Nominated character {character_id} for institution {institution_id}."),
    })
}

pub(super) fn office_nomination_subject(
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> String {
    format!("institution:{institution_id}:character:{character_id}")
}

fn apply_crisis_response(
    state: &mut AppState,
    crisis_id: CrisisId,
    response: CrisisResponse,
) -> Result<CommandOutcome, CommandError> {
    let crisis = state
        .crises
        .get(&crisis_id)
        .ok_or(CommandError::MissingCrisis { crisis_id })?;
    if !crisis.status.is_active() {
        return Err(CommandError::InactiveCrisis { crisis_id });
    }
    let subject = format!("crisis:{crisis_id}");
    if state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::CrisisResponse && record.subject() == subject)
        .is_some()
    {
        return Err(CommandError::CrisisAlreadyAddressed { crisis_id });
    }
    let severity = crisis.severity_basis_points;
    let district_id = crisis.district_id;
    match response {
        CrisisResponse::Relief => {
            let cost = Money::from_copper(i64::from(severity).saturating_mul(2));
            spend_player_treasury(state, cost)?;
            reduce_crisis(state, crisis_id, 2_500);
            adjust_player_legitimacy(state, 500, true);
            adjust_district_unrest(state, district_id, 800, false);
        }
        CrisisResponse::Reform => {
            spend_player_treasury(state, Money::from_copper(1_500))?;
            reduce_crisis(state, crisis_id, 1_800);
            adjust_player_legitimacy(state, 300, true);
            adjust_district_unrest(state, district_id, 500, false);
        }
        CrisisResponse::Suppress => {
            spend_player_treasury(state, Money::from_copper(900))?;
            reduce_crisis(state, crisis_id, 2_000);
            adjust_player_legitimacy(state, 450, false);
            adjust_district_unrest(state, district_id, 700, true);
        }
        CrisisResponse::Exploit => {
            let required_legitimacy = 600;
            let available_legitimacy = state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .resources
                .legitimacy_basis_points;
            if available_legitimacy < required_legitimacy {
                return Err(CommandError::InsufficientPlayerLegitimacy {
                    available: available_legitimacy,
                    required: required_legitimacy,
                });
            }
            let gain = Money::from_copper(i64::from(severity));
            let current_treasury = state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .treasury();
            let resulting_treasury =
                current_treasury
                    .checked_add(gain)
                    .ok_or(CommandError::Strategic(
                        StrategicError::DynastyTreasuryOverflow {
                            dynasty_id: state.player_dynasty_id,
                            current: current_treasury,
                            incoming: gain,
                        },
                    ))?;
            let dynasty = state
                .dynasties
                .get_mut(&state.player_dynasty_id)
                .expect("player dynasty must exist");
            dynasty.resources.treasury = resulting_treasury;
            let crisis = state
                .crises
                .get_mut(&crisis_id)
                .expect("validated crisis must exist");
            crisis.severity_basis_points = severity.saturating_add(500).min(10_000);
            crisis.status = CrisisStatus::from_severity(crisis.severity_basis_points);
            adjust_player_legitimacy(state, 600, false);
            adjust_district_unrest(state, district_id, 600, true);
        }
    }
    super::strategic::push_outbox(
        state,
        OutboxKind::Crisis,
        format!("Response applied to crisis {crisis_id}"),
        format!("The dynasty chose {response:?}."),
    );
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::CrisisResponse,
        subject,
        detail: format!("response={response:?}"),
    });
    Ok(CommandOutcome {
        summary: format!("Applied {response:?} response to crisis {crisis_id}."),
    })
}

fn reduce_crisis(state: &mut AppState, crisis_id: CrisisId, amount: u16) {
    let crisis = state
        .crises
        .get_mut(&crisis_id)
        .expect("validated crisis must exist");
    crisis.severity_basis_points = crisis.severity_basis_points.saturating_sub(amount);
    crisis.status = CrisisStatus::from_severity(crisis.severity_basis_points);
}

fn adjust_player_legitimacy(state: &mut AppState, amount: u16, increase: bool) {
    let dynasty = state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.legitimacy_basis_points = if increase {
        dynasty
            .resources
            .legitimacy_basis_points
            .saturating_add(amount)
            .min(10_000)
    } else {
        dynasty
            .resources
            .legitimacy_basis_points
            .saturating_sub(amount)
    };
}

fn adjust_district_unrest(
    state: &mut AppState,
    district_id: Option<DistrictId>,
    amount: u16,
    increase: bool,
) {
    let Some(district_id) = district_id else {
        return;
    };
    let Some(district) = state.districts.get_mut(&district_id) else {
        return;
    };
    district.unrest_basis_points = if increase {
        district
            .unrest_basis_points
            .saturating_add(amount)
            .min(10_000)
    } else {
        district.unrest_basis_points.saturating_sub(amount)
    };
}

fn apply_labor_response(
    state: &mut AppState,
    employment_id: EmploymentId,
    response: LaborResponse,
) -> Result<CommandOutcome, CommandError> {
    let agreement = state
        .employment
        .get(&employment_id)
        .ok_or(CommandError::MissingEmployment { employment_id })?;
    let business_id = agreement.business_id;
    let workers = agreement.workers;
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
    if agreement.status != EmploymentStatus::Disputed {
        return Err(CommandError::InvalidLaborDispute { employment_id });
    }
    match response {
        LaborResponse::ImproveConditions => {
            spend_business_cash(state, business_id, Money::from_copper(1_000))?;
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("validated employment must exist");
            agreement.conditions_basis_points = agreement
                .conditions_basis_points
                .saturating_add(2_000)
                .clamp(super::EMPLOYMENT_RECOVERY_BASIS_POINTS, 10_000);
            agreement.loyalty_basis_points = agreement
                .loyalty_basis_points
                .saturating_add(1_000)
                .clamp(super::EMPLOYMENT_RECOVERY_BASIS_POINTS, 10_000);
            agreement.status = EmploymentStatus::Active;
        }
        LaborResponse::Negotiate => {
            spend_business_cash(state, business_id, Money::from_copper(500))?;
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("validated employment must exist");
            agreement.weekly_wage = agreement.weekly_wage.saturating_mul_ratio(11, 10);
            agreement.loyalty_basis_points = agreement.loyalty_basis_points.max(4_500);
            agreement.conditions_basis_points = agreement
                .conditions_basis_points
                .max(super::EMPLOYMENT_RECOVERY_BASIS_POINTS);
            agreement.status = EmploymentStatus::Active;
        }
        LaborResponse::ReplaceWorkers => {
            let district_id = state
                .businesses
                .get(business_id)
                .expect("validated business must exist")
                .district_id();
            let replacement = state
                .households
                .ids_for_district(district_id)
                .and_then(|ids| {
                    ids.iter().find(|id| {
                        **id != agreement.household_id
                            && super::available_household_workers(state, **id, None)
                                >= u32::from(workers)
                    })
                })
                .copied()
                .ok_or(CommandError::NoReplacementLaborAvailable {
                    employment_id,
                    district_id,
                    workers,
                })?;
            spend_business_cash(state, business_id, LABOR_REPLACEMENT_COST)?;
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("validated employment must exist");
            agreement.household_id = replacement;
            agreement.loyalty_basis_points = 6_000;
            agreement.conditions_basis_points = 6_000;
            agreement.status = EmploymentStatus::Active;
            adjust_district_unrest(state, Some(district_id), 400, true);
        }
    }
    super::strategic::push_outbox(
        state,
        OutboxKind::District,
        format!("Labor dispute {employment_id} resolved"),
        format!("The dynasty chose {response:?}."),
    );
    Ok(CommandOutcome {
        summary: format!("Resolved labor dispute {employment_id} with {response:?}."),
    })
}

fn spend_business_cash(
    state: &mut AppState,
    business_id: BusinessId,
    amount: Money,
) -> Result<(), CommandError> {
    let cash = state
        .businesses
        .get(business_id)
        .ok_or(CommandError::MissingBusiness { business_id })?
        .cash();
    if cash < amount {
        return Err(CommandError::InsufficientBusinessFunds {
            business_id,
            available: cash,
            required: amount,
        });
    }
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("validated business must exist");
    business.finance.cash = cash.saturating_sub(amount);
    business.finance.lifetime_costs = business.finance.lifetime_costs.saturating_add(amount);
    business.finance.version = business.finance.version.saturating_add(1);
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
        .treasury = treasury.saturating_sub(amount);
    Ok(())
}

fn apply_acknowledgement(
    state: &mut AppState,
    message_id: OutboxMessageId,
) -> Result<CommandOutcome, CommandError> {
    if !state.outbox.iter().any(|message| message.id == message_id) {
        return Err(CommandError::MissingNotification { message_id });
    }
    let mut acknowledged = 0_u32;
    for message in state
        .outbox
        .iter_mut()
        .filter(|message| message.id <= message_id && !message.acknowledged)
    {
        message.acknowledged = true;
        acknowledged = acknowledged.saturating_add(1);
    }
    Ok(CommandOutcome {
        summary: format!(
            "Acknowledged {acknowledged} notifications through notification {message_id}."
        ),
    })
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
