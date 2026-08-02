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
    #[error("business investment must be positive")]
    InvalidBusinessInvestment,
    #[error("law {kind:?} does not support value {value}")]
    InvalidLawValue { kind: LawKind, value: i64 },
    #[error("law {kind:?} is not implemented by the current simulation")]
    UnsupportedLaw { kind: LawKind },
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
    #[error("legal case cannot target the player dynasty")]
    SameLegalParty,
    #[error("legal evidence or damages are invalid")]
    InvalidLegalTerms,
    #[error("family council for dynasty {dynasty_id} does not exist")]
    MissingFamilyCouncil { dynasty_id: DynastyId },
    #[error("house governance is already {governance:?}")]
    UnchangedHouseGovernance { governance: HouseGovernance },
    #[error("institution {institution_id} does not exist")]
    MissingInstitution { institution_id: InstitutionId },
    #[error("character {character_id} is already a member of institution {institution_id}")]
    AlreadyInstitutionMember {
        institution_id: InstitutionId,
        character_id: CharacterId,
    },
    #[error("character {character_id} is not an active member of the player dynasty")]
    InvalidNominee { character_id: CharacterId },
    #[error("crisis {crisis_id} does not exist")]
    MissingCrisis { crisis_id: CrisisId },
    #[error("crisis {crisis_id} is no longer active")]
    InactiveCrisis { crisis_id: CrisisId },
    #[error("employment agreement {employment_id} does not exist")]
    MissingEmployment { employment_id: EmploymentId },
    #[error("employment agreement {employment_id} is not a player labor dispute")]
    InvalidLaborDispute { employment_id: EmploymentId },
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
    if state
        .businesses
        .get(business_id)
        .is_some_and(|business| business.status() == BusinessStatus::Closed)
    {
        return Err(CommandError::Strategic(StrategicError::BusinessInactive {
            business_id,
        }));
    }
    spend_player_treasury(state, amount)?;
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("validated business must exist");
    business.finance.cash = business.finance.cash.saturating_add(amount);
    business.finance.version = business.finance.version.saturating_add(1);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::BusinessCapitalization,
        subject: format!("business:{business_id}"),
        detail: format!("amount={}", amount.copper()),
    });
    super::strategic::push_outbox(
        state,
        OutboxKind::Finance,
        format!("Business {business_id} capitalized"),
        format!("The dynasty invested {amount} into the enterprise."),
    );
    Ok(CommandOutcome {
        summary: format!("Invested {amount} in business {business_id}."),
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
    let cost = Money::from_copper(2_000);
    spend_player_treasury(state, cost)?;
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
    let contribution = Money::from_copper((budget.copper() / 10).max(1)).min(budget);
    spend_player_treasury(state, contribution)?;
    let progress_basis_points =
        u16::try_from(contribution.copper().saturating_mul(10_000) / budget.copper())
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
        .get_mut(&dynasty_id)
        .ok_or(CommandError::MissingFamilyCouncil { dynasty_id })?;
    if council.governance == governance {
        return Err(CommandError::UnchangedHouseGovernance { governance });
    }
    council.governance = governance;
    council.charter_version = council.charter_version.saturating_add(1);
    council.unity_basis_points = council.unity_basis_points.saturating_sub(250);
    super::strategic::push_outbox(
        state,
        OutboxKind::Family,
        "House charter amended".to_owned(),
        format!("The dynasty adopted {governance:?} governance."),
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
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    if !institution.members.insert(character_id) {
        return Err(CommandError::AlreadyInstitutionMember {
            institution_id,
            character_id,
        });
    }
    let dynasty = state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.legitimacy_basis_points = dynasty
        .resources
        .legitimacy_basis_points
        .saturating_add(75)
        .min(10_000);
    Ok(CommandOutcome {
        summary: format!("Nominated character {character_id} for institution {institution_id}."),
    })
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
            let dynasty = state
                .dynasties
                .get_mut(&state.player_dynasty_id)
                .expect("player dynasty must exist");
            dynasty.resources.treasury = dynasty.resources.treasury.saturating_add(gain);
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
    ensure_owned_business(state, business_id)?;
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
                .min(10_000);
            agreement.loyalty_basis_points = agreement
                .loyalty_basis_points
                .saturating_add(1_000)
                .min(10_000);
            agreement.status = EmploymentStatus::Active;
        }
        LaborResponse::Negotiate => {
            spend_business_cash(state, business_id, Money::from_copper(500))?;
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("validated employment must exist");
            agreement.weekly_wage =
                Money::from_copper(agreement.weekly_wage.copper().saturating_mul(11) / 10);
            agreement.loyalty_basis_points = agreement.loyalty_basis_points.max(4_500);
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
                .and_then(|ids| ids.iter().find(|id| **id != agreement.household_id))
                .copied()
                .ok_or(CommandError::InvalidLaborDispute { employment_id })?;
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("validated employment must exist");
            agreement.household_id = replacement;
            agreement.loyalty_basis_points = 5_000;
            agreement.conditions_basis_points = 5_500;
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
    let message = state
        .outbox
        .iter_mut()
        .find(|message| message.id == message_id)
        .ok_or(CommandError::MissingNotification { message_id })?;
    message.acknowledged = true;
    Ok(CommandOutcome {
        summary: format!("Acknowledged notification {message_id}."),
    })
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
