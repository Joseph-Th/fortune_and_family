//! Canonical player-command validation and dispatch across simulation subsystems.

use super::transactions::{next_business_finance_version, next_family_charter_version};
use super::{
    LoanTerms, OFFICE_POWER_ESTABLISHMENT_DAYS, StrategicError, SupplyContractTerms,
    acquire_business, buy_unowned_property, issue_loan, sell_owned_property, sign_supply_contract,
    transfer_business_cash,
};
use crate::core::{
    AppState, AuditKind, AuditRecord, BusinessStatus, Character, CharacterCapabilities,
    CharacterIdentity, CharacterRole, CharacterRuntime, CharacterStatus, ChronicleEntry,
    ChronicleKind, CivicDebt, CivicDebtStatus, ContractStatus, CrisisStatus, DynastyPair,
    EmploymentStatus, EnactedLaw, FamilyLink, FamilyLinkKind, HouseGovernance,
    InformationConfidence, InformationReport, InformationTarget, LawKind, LegalCase, LegalCaseKind,
    LegalCaseStatus, OfficeDirectiveState, OfficePower, OutboxKind, PublicWork, PublicWorkKind,
    PublicWorkStatus,
};
use crate::ids::{
    BusinessId, CharacterId, ContractId, CrisisId, DistrictId, DynastyId, EmploymentId, GoodId,
    InformationReportId, InstitutionId, OutboxMessageId, PropertyId,
};
use crate::money::Money;
use crate::registry::Registry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
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
    FileLegalCase {
        defendant_dynasty_id: DynastyId,
        kind: LegalCaseKind,
        evidence_basis_points: u16,
        damages: Money,
    },
    SetHouseGovernance {
        governance: HouseGovernance,
    },
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
pub(crate) const LEGAL_CASE_FILING_COST: Money = Money::from_copper(300);
pub(crate) const LAW_SPONSORSHIP_INTERVAL_DAYS: i64 = 360;
pub(crate) const LAW_LEGITIMACY_REQUIREMENT: u16 = 3_000;
pub(crate) const LAW_LEGITIMACY_COST: u16 = 250;
pub(crate) const CIVIC_DEBT_INTEREST_BASIS_POINTS: u16 = 600;
pub(crate) const CIVIC_DEBT_TERM_WEEKS: i64 = 104;
pub(crate) const CIVIC_DEBT_CREDITOR_RESERVE: Money = Money::from_copper(10_000);
pub(crate) const PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS: i64 = 360;
pub(crate) const MAX_ACTIVE_SPONSORED_PUBLIC_WORKS: usize = 2;
pub(crate) const LABOR_REPLACEMENT_COST: Money = Money::from_copper(750);
pub(crate) const HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS: i64 = 1_800;
pub(crate) const HEIR_DESIGNATION_INTERVAL_DAYS: i64 = 720;
pub(crate) const HEIR_DESIGNATION_LEGITIMACY_COST: u16 = 300;
const HEIR_DESIGNATION_UNITY_COST: u16 = 250;
const HEIR_MINIMUM_AGE_DAYS: i64 = 18 * 360;
pub(crate) const OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS: i64 = 180;
pub(crate) const OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST: u16 = 100;
pub(crate) const INSTITUTION_SUPPORT_INTERVAL_DAYS: i64 = 360;
pub(crate) const INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS: i64 = 180;
pub(crate) const INSTITUTION_SUPPORT_COST: Money = Money::from_copper(1_200);
pub(crate) const INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT: u16 = 5_500;
pub(crate) const INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT: u32 = 78;
pub(crate) const MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER: usize = 2;
pub(crate) const OFFICE_NOMINATION_INTERVAL_DAYS: i64 = 360;
pub(crate) const OFFICE_NOMINATION_RECOVERY_DAYS: i64 = 720;
const OFFICE_NOMINATION_RESOLUTION_DAYS: i64 = 120;
pub(crate) const OFFICE_NOMINATION_REPUTATION_REQUIREMENT: u16 = 5_500;
pub(crate) const OFFICE_NOMINATION_DELIVERY_REQUIREMENT: u32 = 78;
pub(crate) const WARD_ADOPTION_INTERVAL_DAYS: i64 = 720;
pub(crate) const WARD_ADOPTION_COST: Money = Money::from_copper(6_000);
pub(crate) const WARD_ADOPTION_LEGITIMACY_REQUIREMENT: u16 = 3_500;
pub(crate) const WARD_ADOPTION_REPUTATION_REQUIREMENT: u16 = 5_200;
pub(crate) const WARD_ADOPTION_DELIVERY_REQUIREMENT: u32 = 52;
pub(crate) const MAX_ACTIVE_WARDS: usize = 4;
pub(crate) const FAMILY_EDUCATION_INTERVAL_DAYS: i64 = 360;
pub(crate) const FAMILY_EDUCATION_COST: Money = Money::from_copper(2_000);
pub(crate) const INFORMATION_COMMISSION_INTERVAL_DAYS: i64 = 360;
pub(crate) const INFORMATION_COMMISSION_COST: Money = Money::from_copper(600);
pub(crate) const INFORMATION_LEVERAGE_COST: Money = Money::from_copper(600);
const INFORMATION_REPORT_LIFETIME_DAYS: i64 = 540;
pub(crate) const COMMISSIONED_INFORMATION_SOURCE: &str = "Commissioned intelligence";

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
    #[error("the scenario does not define a civic treasury institution")]
    MissingCivicTreasury,
    #[error("no non-player dynasty can fund civic debt principal {required}")]
    NoCivicDebtCreditor { required: Money },
    #[error("civic treasury budget {current} cannot receive debt proceeds {incoming}")]
    CivicTreasuryOverflow { current: Money, incoming: Money },
    #[error("the player dynasty must hold political office before sponsoring a law")]
    LawSponsorshipRequiresOffice,
    #[error("law {kind:?} requires an office with {required:?} power")]
    LawSponsorshipRequiresPower {
        kind: LawKind,
        required: OfficePower,
    },
    #[error(
        "law {kind:?} cannot use {required:?} power until the office is established on day {available_day}"
    )]
    LawSponsorshipPowerNotEstablished {
        kind: LawKind,
        required: OfficePower,
        available_day: i64,
    },
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
    #[error("public-work sponsorship requires an office with PublicWorks power")]
    PublicWorkSponsorshipRequiresPower,
    #[error(
        "public-work sponsorship cannot use PublicWorks power until the office is established on day {available_day}"
    )]
    PublicWorkPowerNotEstablished { available_day: i64 },
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
    #[error("character {character_id} is not an eligible heir candidate")]
    InvalidHeirCandidate { character_id: CharacterId },
    #[error("character {character_id} is already the designated heir")]
    UnchangedHeir { character_id: CharacterId },
    #[error("the dynasty cannot designate another heir before day {next_designation_day}")]
    HeirDesignationCooldown { next_designation_day: i64 },
    #[error(
        "player reputation is too weak for an office campaign: quality {quality}, reliability {reliability}, required {required}"
    )]
    InsufficientOfficeReputation {
        quality: u16,
        reliability: u16,
        required: u16,
    },
    #[error(
        "office nomination requires {required} completed contract deliveries, but dynasty has {delivered}"
    )]
    InsufficientOfficeCommercialRecord { delivered: u32, required: u32 },
    #[error(
        "the player dynasty cannot launch another office campaign before day {next_nomination_day}"
    )]
    OfficeNominationCooldown { next_nomination_day: i64 },
    #[error("the dynasty cannot adopt another ward before day {next_adoption_day}")]
    WardAdoptionCooldown { next_adoption_day: i64 },
    #[error("the dynasty already has {active} active wards; maximum is {maximum}")]
    WardCapacity { active: usize, maximum: usize },
    #[error(
        "ward adoption requires commercial reputation {required}, but quality is {quality} and reliability is {reliability}"
    )]
    InsufficientWardReputation {
        quality: u16,
        reliability: u16,
        required: u16,
    },
    #[error(
        "ward adoption requires {required} completed contract deliveries, but dynasty has {delivered}"
    )]
    InsufficientWardCommercialRecord { delivered: u32, required: u32 },
    #[error("character {character_id} is not an active member of the player dynasty")]
    InvalidFamilyStudent { character_id: CharacterId },
    #[error("character {character_id} has already mastered {focus:?}")]
    FamilyEducationAtMaximum {
        character_id: CharacterId,
        focus: EducationFocus,
    },
    #[error("the dynasty cannot fund another family education before day {next_education_day}")]
    FamilyEducationCooldown { next_education_day: i64 },
    #[error("institution {institution_id} does not exist")]
    MissingInstitution { institution_id: InstitutionId },
    #[error("institution {institution_id} does not grant the player office power {power:?}")]
    OfficePowerUnavailable {
        institution_id: InstitutionId,
        power: OfficePower,
    },
    #[error(
        "institution {institution_id} cannot exercise office power {power:?} before day {available_day}"
    )]
    OfficePowerDirectiveNotEstablished {
        institution_id: InstitutionId,
        power: OfficePower,
        available_day: i64,
    },
    #[error(
        "institution {institution_id} cannot exercise office power {power:?} again before day {next_directive_day}"
    )]
    OfficePowerDirectiveCooldown {
        institution_id: InstitutionId,
        power: OfficePower,
        next_directive_day: i64,
    },
    #[error(
        "institutional support requires reputation {required}, but quality is {quality} and reliability is {reliability}"
    )]
    InsufficientInstitutionSupportReputation {
        quality: u16,
        reliability: u16,
        required: u16,
    },
    #[error(
        "institutional support requires {required} completed contract deliveries, but dynasty has {delivered}"
    )]
    InsufficientInstitutionSupportCommercialRecord { delivered: u32, required: u32 },
    #[error(
        "character {character_id} already has cultivated support in institution {institution_id}"
    )]
    InstitutionSupportAlreadyEstablished {
        institution_id: InstitutionId,
        character_id: CharacterId,
    },
    #[error(
        "character {character_id} already belongs to {current} institutions; maximum supported portfolio is {maximum}"
    )]
    InstitutionMembershipCapacity {
        character_id: CharacterId,
        current: usize,
        maximum: usize,
    },
    #[error(
        "the dynasty cannot cultivate support in another institution before day {next_support_day}"
    )]
    InstitutionSupportCooldown { next_support_day: i64 },
    #[error("institution {institution_id} budget {current} cannot receive patronage {incoming}")]
    InstitutionBudgetOverflow {
        institution_id: InstitutionId,
        current: Money,
        incoming: Money,
    },
    #[error("character {character_id} is not an active member of the player dynasty")]
    InvalidNominee { character_id: CharacterId },
    #[error("character {character_id} has not cultivated support in institution {institution_id}")]
    MissingInstitutionSupport {
        institution_id: InstitutionId,
        character_id: CharacterId,
    },
    #[error(
        "character {character_id}'s support in institution {institution_id} is not established until day {available_day}"
    )]
    InstitutionSupportNotEstablished {
        institution_id: InstitutionId,
        character_id: CharacterId,
        available_day: i64,
    },
    #[error("character {character_id} already holds office in institution {institution_id}")]
    NomineeAlreadyHoldsOffice {
        character_id: CharacterId,
        institution_id: InstitutionId,
    },
    #[error("character {character_id} is not a player member of institution {institution_id}")]
    InvalidInstitutionWithdrawal {
        institution_id: InstitutionId,
        character_id: CharacterId,
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
    #[error("negotiating employment {employment_id} would overflow its weekly wage from {current}")]
    LaborWageOverflow {
        employment_id: EmploymentId,
        current: Money,
    },
    #[error(
        "district {district_id} has no replacement household able to supply {workers} workers for employment {employment_id}"
    )]
    NoReplacementLaborAvailable {
        employment_id: EmploymentId,
        district_id: DistrictId,
        workers: u16,
    },
    #[error("good {good_id} does not exist")]
    MissingGood { good_id: GoodId },
    #[error("market quote for good {good_id} does not exist")]
    MissingMarketQuote { good_id: GoodId },
    #[error("commissioned counterparty intelligence cannot target the player dynasty")]
    InformationCannotTargetPlayer,
    #[error(
        "the dynasty cannot commission another intelligence report before day {next_commission_day}"
    )]
    InformationCommissionCooldown { next_commission_day: i64 },
    #[error("information report {report_id} does not exist")]
    MissingInformationReport { report_id: InformationReportId },
    #[error("information report {report_id} is not owned by the player dynasty")]
    InformationReportNotOwned { report_id: InformationReportId },
    #[error("information report {report_id} is not confirmed commissioned intelligence")]
    InformationReportNotCommissioned { report_id: InformationReportId },
    #[error("information report {report_id} expired on day {expired_day}")]
    InformationReportExpired {
        report_id: InformationReportId,
        expired_day: i64,
    },
    #[error("information report {report_id} has no actionable leverage in the current state")]
    InformationReportHasNoLeverage { report_id: InformationReportId },
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
    if state.scenario_key() != registry.scenario().key() {
        return Err(super::SimulationError::RegistryMismatch {
            state_scenario: state.scenario_key().to_owned(),
            registry_scenario: registry.scenario().key().to_owned(),
        }
        .into());
    }
    let mut candidate = state.clone();
    match dispatch_player_command(registry, &mut candidate, command) {
        Ok(outcome) => {
            super::validate_invariants(registry, &candidate);
            *state = candidate;
            Ok(outcome)
        }
        Err(error) => Err(error),
    }
}

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
        PlayerCommand::CreateSupplyContract { terms } => apply_contract(registry, state, terms),
        PlayerCommand::IssueLoan { terms } => apply_loan(state, terms),
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
        PlayerCommand::SetHouseGovernance { governance } => apply_governance(state, governance),
        PlayerCommand::DesignateHeir { character_id } => apply_heir(state, character_id),
        PlayerCommand::AdoptWard { focus } => apply_adopt_ward(state, focus),
        PlayerCommand::EducateFamilyMember {
            character_id,
            focus,
        } => apply_family_education(state, character_id, focus),
        PlayerCommand::CultivateInstitutionSupport {
            institution_id,
            character_id,
        } => apply_institution_support(state, institution_id, character_id),
        PlayerCommand::NominateForOffice {
            institution_id,
            character_id,
        } => apply_office_nomination(state, institution_id, character_id),
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
        } => apply_crisis_response(state, crisis_id, response),
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

fn apply_contract(
    registry: &Registry,
    state: &mut AppState,
    terms: SupplyContractTerms,
) -> Result<CommandOutcome, CommandError> {
    ensure_player_contract_party(state, &terms)?;
    let id = sign_supply_contract(registry, state, terms)?;
    Ok(CommandOutcome {
        summary: format!("Created supply contract {id}."),
    })
}

fn apply_loan(state: &mut AppState, terms: LoanTerms) -> Result<CommandOutcome, CommandError> {
    ensure_player_loan_party(state, &terms)?;
    let id = issue_loan(state, terms)?;
    Ok(CommandOutcome {
        summary: format!("Issued loan {id}."),
    })
}

fn apply_property_purchase(
    state: &mut AppState,
    property_id: PropertyId,
) -> Result<CommandOutcome, CommandError> {
    buy_unowned_property(state, state.player_dynasty_id, property_id)?;
    Ok(CommandOutcome {
        summary: format!("Acquired property {property_id}."),
    })
}

fn apply_property_sale(
    registry: &Registry,
    state: &mut AppState,
    property_id: PropertyId,
    buyer_dynasty_id: DynastyId,
) -> Result<CommandOutcome, CommandError> {
    let quote = sell_owned_property(
        registry,
        state,
        state.player_dynasty_id,
        buyer_dynasty_id,
        property_id,
    )?;
    Ok(CommandOutcome {
        summary: format!("Sold property {property_id} for {}.", quote.price),
    })
}

fn apply_institution_withdrawal(
    state: &mut AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Result<CommandOutcome, CommandError> {
    let character =
        state
            .characters
            .get(character_id)
            .ok_or(CommandError::InvalidInstitutionWithdrawal {
                institution_id,
                character_id,
            })?;
    if character.dynasty_id() != state.player_dynasty_id {
        return Err(CommandError::InvalidInstitutionWithdrawal {
            institution_id,
            character_id,
        });
    }
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    if !institution.members.contains(&character_id) {
        return Err(CommandError::InvalidInstitutionWithdrawal {
            institution_id,
            character_id,
        });
    }
    let resigned_office = institution.office_holder_id == Some(character_id);
    let day = state.clock.day();
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    institution.members.remove(&character_id);
    if resigned_office {
        institution.office_holder_id = None;
        institution.next_selection_day = institution.next_selection_day.min(day.saturating_add(30));
    }
    super::strategic::push_outbox(
        state,
        OutboxKind::Politics,
        format!("Character {character_id} withdrew from institution {institution_id}"),
        if resigned_office {
            "The dynasty surrendered the office and its institutional membership; a replacement selection will be scheduled.".to_owned()
        } else {
            "The dynasty surrendered this institutional membership.".to_owned()
        },
    );
    Ok(CommandOutcome {
        summary: if resigned_office {
            format!(
                "Withdrew character {character_id} from institution {institution_id} and resigned the office."
            )
        } else {
            format!("Withdrew character {character_id} from institution {institution_id}.")
        },
    })
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
    let next_finance_version = next_business_finance_version(business)?;
    spend_player_treasury(state, amount)?;
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("validated business must exist");
    business.finance.cash = resulting_cash;
    business.finance.version = next_finance_version;
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
        subject: format!("business:{business_id}").into(),
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
    let next_finance_version = next_business_finance_version(business)?;
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("validated business must exist");
    business.policy.target_input_days = target_input_days;
    business.policy.target_output_days = target_output_days;
    business.policy.minimum_cash_reserve = minimum_cash_reserve;
    business.policy.maintenance_basis_points = maintenance_basis_points;
    business.policy.quality_target_basis_points = quality_target_basis_points;
    business.finance.version = next_finance_version;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::BusinessPolicyChange,
        subject: subject.into(),
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

#[derive(Clone, Copy, Debug)]
struct ValidatedCivicDebtIssuance {
    treasury_id: InstitutionId,
    creditor_dynasty_id: DynastyId,
    principal: Money,
    creditor_treasury_after: Money,
    treasury_budget_after: Money,
    weekly_payment: Money,
}

fn validate_civic_debt_issuance(
    registry: &Registry,
    state: &AppState,
    principal: Money,
) -> Result<ValidatedCivicDebtIssuance, CommandError> {
    let treasury_id = registry
        .get_institution_id("treasury")
        .ok_or(CommandError::MissingCivicTreasury)?;
    let treasury = state
        .institutions
        .get(&treasury_id)
        .ok_or(CommandError::MissingCivicTreasury)?;
    let treasury_budget_after =
        treasury
            .budget
            .checked_add(principal)
            .ok_or(CommandError::CivicTreasuryOverflow {
                current: treasury.budget,
                incoming: principal,
            })?;
    let creditor = state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .filter(|dynasty| {
            dynasty
                .treasury()
                .saturating_sub(CIVIC_DEBT_CREDITOR_RESERVE)
                >= principal
        })
        .max_by_key(|dynasty| (dynasty.treasury(), std::cmp::Reverse(dynasty.id())))
        .ok_or(CommandError::NoCivicDebtCreditor {
            required: principal,
        })?;
    let weekly_payment_copper = principal
        .copper()
        .saturating_add(CIVIC_DEBT_TERM_WEEKS.saturating_sub(1))
        / CIVIC_DEBT_TERM_WEEKS;
    Ok(ValidatedCivicDebtIssuance {
        treasury_id,
        creditor_dynasty_id: creditor.id(),
        principal,
        creditor_treasury_after: creditor
            .treasury()
            .checked_sub(principal)
            .expect("validated civic debt creditor must cover the principal"),
        treasury_budget_after,
        weekly_payment: Money::from_copper(weekly_payment_copper.max(1)),
    })
}

fn commit_civic_debt_issuance(
    state: &mut AppState,
    law_id: crate::ids::LawId,
    sponsor_dynasty_id: DynastyId,
    issuance: ValidatedCivicDebtIssuance,
) -> crate::ids::CivicDebtId {
    state
        .dynasties
        .get_mut(&issuance.creditor_dynasty_id)
        .expect("validated civic debt creditor must exist")
        .resources
        .treasury = issuance.creditor_treasury_after;
    state
        .institutions
        .get_mut(&issuance.treasury_id)
        .expect("validated civic treasury must exist")
        .budget = issuance.treasury_budget_after;
    let id = state.next_ids.civic_debt();
    state.civic_debts.insert(
        id,
        CivicDebt {
            id,
            creditor_dynasty_id: issuance.creditor_dynasty_id,
            authorizing_law_id: law_id,
            sponsor_dynasty_id: Some(sponsor_dynasty_id),
            principal: issuance.principal,
            balance: issuance.principal,
            weekly_payment: issuance.weekly_payment,
            interest_basis_points: CIVIC_DEBT_INTEREST_BASIS_POINTS,
            issued_day: state.clock.day(),
            next_due_day: state.clock.day().saturating_add(7),
            missed_payments: 0,
            status: CivicDebtStatus::Current,
        },
    );
    super::strategic::adjust_dynasty_relationship(
        state,
        sponsor_dynasty_id,
        issuance.creditor_dynasty_id,
        super::strategic::RelationshipDelta::new(40, 30, 0, -20, 1),
    );
    super::strategic::remember_dynasty_interaction(
        state,
        sponsor_dynasty_id,
        issuance.creditor_dynasty_id,
        &format!("Civic debt {id} financed the city treasury."),
    );
    super::strategic::record_counterparty_information(
        state,
        sponsor_dynasty_id,
        issuance.creditor_dynasty_id,
        "Municipal debt underwriting and treasury records",
    );
    id
}

#[derive(Clone, Copy, Debug)]
struct ValidatedLawSponsorship {
    legitimacy: u16,
    civic_debt_issuance: Option<ValidatedCivicDebtIssuance>,
}

fn validate_law_sponsorship(
    registry: &Registry,
    state: &AppState,
    kind: LawKind,
    value: i64,
) -> Result<ValidatedLawSponsorship, CommandError> {
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
    let required_power = required_office_power_for_law(kind);
    if !has_player_office_power(state, required_power) {
        return Err(CommandError::LawSponsorshipRequiresPower {
            kind,
            required: required_power,
        });
    }
    let available_day = player_office_power_available_day(state, required_power)
        .expect("validated office power must have an availability day");
    if state.clock.day() < available_day {
        return Err(CommandError::LawSponsorshipPowerNotEstablished {
            kind,
            required: required_power,
            available_day,
        });
    }
    let civic_debt_issuance = (kind == LawKind::PublicDebtAuthorization)
        .then(|| validate_civic_debt_issuance(registry, state, Money::from_copper(value)))
        .transpose()?;
    Ok(ValidatedLawSponsorship {
        legitimacy,
        civic_debt_issuance,
    })
}

fn apply_law(
    registry: &Registry,
    state: &mut AppState,
    kind: LawKind,
    value: i64,
) -> Result<CommandOutcome, CommandError> {
    let validation = validate_law_sponsorship(registry, state, kind, value)?;
    let cost = Money::from_copper(2_000);
    spend_player_treasury(state, cost)?;
    state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .legitimacy_basis_points = validation.legitimacy.saturating_sub(LAW_LEGITIMACY_COST);
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
            active: kind.remains_active_after_enactment(),
        },
    );
    let civic_debt_id = validation
        .civic_debt_issuance
        .map(|issuance| commit_civic_debt_issuance(state, id, state.player_dynasty_id, issuance));
    super::strategic::push_outbox(
        state,
        OutboxKind::Law,
        format!("Law {id} enacted"),
        civic_debt_id.map_or_else(
            || format!("The player dynasty sponsored {kind:?} with value {value}."),
            |debt_id| {
                format!(
                    "The player dynasty sponsored {kind:?}; civic debt {debt_id} issued {value} copper to the treasury."
                )
            },
        ),
    );
    Ok(CommandOutcome {
        summary: civic_debt_id.map_or_else(
            || format!("Enacted law {id}: {kind:?}."),
            |debt_id| format!("Enacted law {id}: {kind:?}, issuing civic debt {debt_id}."),
        ),
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
    if !has_player_office_power(state, OfficePower::PublicWorks) {
        return Err(CommandError::PublicWorkSponsorshipRequiresPower);
    }
    let available_day = player_office_power_available_day(state, OfficePower::PublicWorks)
        .expect("validated public-works office must have an availability day");
    if state.clock.day() < available_day {
        return Err(CommandError::PublicWorkPowerNotEstablished { available_day });
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
        subject: subject.into(),
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

pub(crate) fn has_player_office_power(state: &AppState, power: OfficePower) -> bool {
    state.institutions.values().any(|institution| {
        institution.powers.contains(&power)
            && institution.office_holder_id.is_some_and(|character_id| {
                state
                    .characters
                    .get(character_id)
                    .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
            })
    })
}

pub(crate) fn player_office_power_available_day(
    state: &AppState,
    power: OfficePower,
) -> Option<i64> {
    state
        .institutions
        .values()
        .filter(|institution| institution.powers.contains(&power))
        .filter(|institution| {
            institution.office_holder_id.is_some_and(|character_id| {
                state
                    .characters
                    .get(character_id)
                    .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
            })
        })
        .map(|institution| {
            institution
                .term_started_day
                .saturating_add(OFFICE_POWER_ESTABLISHMENT_DAYS)
        })
        .min()
}

pub(crate) fn has_established_player_office_power(state: &AppState, power: OfficePower) -> bool {
    player_office_power_available_day(state, power)
        .is_some_and(|available_day| state.clock.day() >= available_day)
}

pub(crate) const fn required_office_power_for_law(kind: LawKind) -> OfficePower {
    match kind {
        LawKind::BreadPriceCeiling | LawKind::ForeignMerchantToll => OfficePower::MarketTolls,
        LawKind::InterestLimit => OfficePower::DebtEnforcement,
        LawKind::FireCode => OfficePower::Inspections,
        LawKind::RentRestriction | LawKind::PublicDebtAuthorization => OfficePower::Taxation,
        LawKind::GuildEntryRestriction => OfficePower::Licenses,
        LawKind::EmergencyImports => OfficePower::EmergencyImports,
    }
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
    spend_player_treasury(state, LEGAL_CASE_FILING_COST)?;
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

fn apply_governance(
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
    let next_charter_version = next_family_charter_version(dynasty_id, council.charter_version)?;
    let council = state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("validated family council must exist");
    council.governance = governance;
    council.charter_version = next_charter_version;
    council.unity_basis_points = council.unity_basis_points.saturating_sub(250);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::HouseGovernanceChange,
        subject: subject.into(),
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

#[derive(Debug)]
struct HeirDesignationPlan {
    dynasty_id: DynastyId,
    prior_heir_id: Option<CharacterId>,
    legitimacy: u16,
    next_charter_version: u64,
    confirmation: bool,
    subject: String,
}

fn validate_heir_designation(
    state: &AppState,
    character_id: CharacterId,
) -> Result<HeirDesignationPlan, CommandError> {
    let dynasty_id = state.player_dynasty_id;
    let (head_id, prior_heir_id, legitimacy) = {
        let dynasty = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist");
        (
            dynasty.head_id(),
            dynasty.heir_id(),
            dynasty.resources.legitimacy_basis_points,
        )
    };
    let subject = format!("dynasty:{dynasty_id}");
    let last_designation_day = state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::HeirDesignation && record.subject() == subject)
        .map(AuditRecord::day);
    let confirmation = prior_heir_id == Some(character_id);
    if confirmation && last_designation_day.is_some() {
        return Err(CommandError::UnchangedHeir { character_id });
    }
    let candidate = state
        .characters
        .get(character_id)
        .ok_or(CommandError::InvalidHeirCandidate { character_id })?;
    let candidate_age = state.clock.day().saturating_sub(candidate.birth_day());
    let council = state
        .family_councils
        .get(&dynasty_id)
        .ok_or(CommandError::MissingFamilyCouncil { dynasty_id })?;
    if candidate.dynasty_id() != dynasty_id
        || candidate.status() != CharacterStatus::Active
        || character_id == head_id
        || candidate_age < HEIR_MINIMUM_AGE_DAYS
        || !council.members.contains(&character_id)
    {
        return Err(CommandError::InvalidHeirCandidate { character_id });
    }
    if legitimacy < HEIR_DESIGNATION_LEGITIMACY_COST {
        return Err(CommandError::InsufficientPlayerLegitimacy {
            available: legitimacy,
            required: HEIR_DESIGNATION_LEGITIMACY_COST,
        });
    }
    if let Some(last_designation_day) = last_designation_day {
        let next_designation_day =
            last_designation_day.saturating_add(HEIR_DESIGNATION_INTERVAL_DAYS);
        if state.clock.day() < next_designation_day {
            return Err(CommandError::HeirDesignationCooldown {
                next_designation_day,
            });
        }
    }
    let next_charter_version = next_family_charter_version(dynasty_id, council.charter_version)?;

    Ok(HeirDesignationPlan {
        dynasty_id,
        prior_heir_id,
        legitimacy,
        next_charter_version,
        confirmation,
        subject,
    })
}

fn apply_heir(
    state: &mut AppState,
    character_id: CharacterId,
) -> Result<CommandOutcome, CommandError> {
    let HeirDesignationPlan {
        dynasty_id,
        prior_heir_id,
        legitimacy,
        next_charter_version,
        confirmation,
        subject,
    } = validate_heir_designation(state, character_id)?;

    if !confirmation {
        if let Some(prior_heir_id) = prior_heir_id {
            let prior_heir = state
                .characters
                .get_mut(prior_heir_id)
                .expect("designated heir must exist");
            if prior_heir.status() == CharacterStatus::Active
                && prior_heir.role() == CharacterRole::Heir
            {
                prior_heir.runtime.role = CharacterRole::Clerk;
            }
        }
        state
            .characters
            .get_mut(character_id)
            .expect("validated heir candidate must exist")
            .runtime
            .role = CharacterRole::Heir;
    }
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("player dynasty must exist");
    dynasty.relationships.heir_id = Some(character_id);
    dynasty.resources.legitimacy_basis_points = legitimacy
        .checked_sub(HEIR_DESIGNATION_LEGITIMACY_COST)
        .expect("validated heir designation legitimacy cost must fit");
    let council = state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("validated family council must exist");
    council.unity_basis_points = council
        .unity_basis_points
        .saturating_sub(HEIR_DESIGNATION_UNITY_COST);
    council.charter_version = next_charter_version;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::HeirDesignation,
        subject: subject.into(),
        detail: format!(
            "prior_heir={};heir={character_id};confirmation={confirmation};legitimacy_cost={HEIR_DESIGNATION_LEGITIMACY_COST};unity_cost={HEIR_DESIGNATION_UNITY_COST}",
            prior_heir_id.map_or_else(|| "none".to_owned(), |id| id.to_string())
        ),
    });
    let chronicle_id = state.next_ids.chronicle();
    let chronicle_summary = if confirmation {
        format!("Dynasty {dynasty_id} formally confirmed character {character_id} as heir.")
    } else {
        format!(
            "Dynasty {dynasty_id} designated character {character_id} as heir, replacing {}.",
            prior_heir_id.map_or_else(
                || "no prior heir".to_owned(),
                |id| format!("character {id}")
            )
        )
    };
    state.chronicle.push(ChronicleEntry {
        id: chronicle_id,
        day: state.clock.day(),
        kind: ChronicleKind::SuccessionPrepared,
        summary: chronicle_summary,
    });
    let outcome_summary = if confirmation {
        format!("Formally confirmed character {character_id} as heir.")
    } else {
        format!("Designated character {character_id} as heir.")
    };
    super::strategic::push_outbox(
        state,
        OutboxKind::Family,
        if confirmation {
            format!("Character {character_id} confirmed as heir")
        } else {
            format!("Character {character_id} designated as heir")
        },
        format!(
            "The family charter now names character {character_id} as successor. The change cost {HEIR_DESIGNATION_LEGITIMACY_COST} legitimacy and {HEIR_DESIGNATION_UNITY_COST} family unity."
        ),
    );
    Ok(CommandOutcome {
        summary: outcome_summary,
    })
}

fn apply_adopt_ward(
    state: &mut AppState,
    focus: EducationFocus,
) -> Result<CommandOutcome, CommandError> {
    let context = validate_ward_adoption(state)?;
    let WardAdoptionContext {
        dynasty_id,
        head_id,
        dynasty_name,
    } = context;
    spend_player_treasury(state, WARD_ADOPTION_COST)?;
    let ward_id = state.next_ids.character();
    let ward_name = format!("{dynasty_name} Ward {ward_id}");
    insert_ward_character(state, dynasty_id, ward_id, ward_name.clone(), focus);
    insert_ward_family_link(state, head_id, ward_id);
    let council = state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("validated family council must exist");
    council.members.insert(ward_id);
    council.unity_basis_points = council.unity_basis_points.saturating_sub(100);
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.legitimacy_basis_points = dynasty
        .resources
        .legitimacy_basis_points
        .saturating_sub(250);
    dynasty.resources.administrative_capacity =
        dynasty.resources.administrative_capacity.saturating_add(8);
    record_ward_adoption(state, dynasty_id, ward_id, &ward_name, focus);
    Ok(CommandOutcome {
        summary: format!("Adopted ward {ward_id} with {focus:?} training."),
    })
}

#[derive(Debug)]
struct WardAdoptionContext {
    dynasty_id: DynastyId,
    head_id: CharacterId,
    dynasty_name: String,
}

fn validate_ward_adoption(state: &AppState) -> Result<WardAdoptionContext, CommandError> {
    let dynasty_id = state.player_dynasty_id;
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .expect("player dynasty must exist");
    let quality = dynasty.resources.reputation_quality_basis_points;
    let reliability = dynasty.resources.reputation_reliability_basis_points;
    let legitimacy = dynasty.resources.legitimacy_basis_points;
    if !state.family_councils.contains_key(&dynasty_id) {
        return Err(CommandError::MissingFamilyCouncil { dynasty_id });
    }
    let active = active_player_ward_count(state);
    if active >= MAX_ACTIVE_WARDS {
        return Err(CommandError::WardCapacity {
            active,
            maximum: MAX_ACTIVE_WARDS,
        });
    }
    if quality.max(reliability) < WARD_ADOPTION_REPUTATION_REQUIREMENT {
        return Err(CommandError::InsufficientWardReputation {
            quality,
            reliability,
            required: WARD_ADOPTION_REPUTATION_REQUIREMENT,
        });
    }
    let delivered = player_contract_deliveries(state);
    if delivered < WARD_ADOPTION_DELIVERY_REQUIREMENT {
        return Err(CommandError::InsufficientWardCommercialRecord {
            delivered,
            required: WARD_ADOPTION_DELIVERY_REQUIREMENT,
        });
    }
    if legitimacy < WARD_ADOPTION_LEGITIMACY_REQUIREMENT {
        return Err(CommandError::InsufficientPlayerLegitimacy {
            available: legitimacy,
            required: WARD_ADOPTION_LEGITIMACY_REQUIREMENT,
        });
    }
    if let Some(last_adoption_day) = state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::WardAdoption)
        .map(AuditRecord::day)
    {
        let next_adoption_day = last_adoption_day.saturating_add(WARD_ADOPTION_INTERVAL_DAYS);
        if state.clock.day() < next_adoption_day {
            return Err(CommandError::WardAdoptionCooldown { next_adoption_day });
        }
    }
    Ok(WardAdoptionContext {
        dynasty_id,
        head_id: dynasty.head_id(),
        dynasty_name: dynasty.name().to_owned(),
    })
}

fn insert_ward_character(
    state: &mut AppState,
    dynasty_id: DynastyId,
    ward_id: CharacterId,
    ward_name: String,
    focus: EducationFocus,
) {
    state.characters.insert(Character {
        identity: CharacterIdentity {
            id: ward_id,
            dynasty_id,
            name: ward_name,
            birth_day: state.clock.day().saturating_sub(18 * 360),
        },
        capabilities: ward_capabilities(focus),
        runtime: CharacterRuntime {
            status: CharacterStatus::Active,
            health_basis_points: 9_500,
            loyalty_basis_points: 8_500,
            role: CharacterRole::Clerk,
        },
    });
}

fn insert_ward_family_link(state: &mut AppState, head_id: CharacterId, ward_id: CharacterId) {
    let family_link_id = state.next_ids.family_link();
    state.family_links.insert(
        family_link_id,
        FamilyLink {
            id: family_link_id,
            first_character_id: head_id,
            second_character_id: ward_id,
            kind: FamilyLinkKind::Ward,
            active: true,
            property_claim_basis_points: 1_500,
        },
    );
}

fn record_ward_adoption(
    state: &mut AppState,
    dynasty_id: DynastyId,
    ward_id: CharacterId,
    ward_name: &str,
    focus: EducationFocus,
) {
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::WardAdoption,
        subject: format!("dynasty:{dynasty_id}:character:{ward_id}").into(),
        detail: format!("focus={focus:?};cost={}", WARD_ADOPTION_COST.copper()),
    });
    let chronicle_id = state.next_ids.chronicle();
    state.chronicle.push(ChronicleEntry {
        id: chronicle_id,
        day: state.clock.day(),
        kind: ChronicleKind::FamilyExpanded,
        summary: format!("{ward_name} entered the dynasty as a ward focused on {focus:?}."),
    });
    super::strategic::push_outbox(
        state,
        OutboxKind::Family,
        format!("Ward adopted: {ward_name}"),
        format!(
            "The dynasty spent {WARD_ADOPTION_COST} to adopt and train a new {focus:?}-focused household member."
        ),
    );
}

fn active_player_ward_count(state: &AppState) -> usize {
    state
        .family_links
        .values()
        .filter(|link| link.active && link.kind == FamilyLinkKind::Ward)
        .filter(|link| {
            state
                .characters
                .get(link.second_character_id)
                .is_some_and(|character| {
                    character.dynasty_id() == state.player_dynasty_id
                        && character.status() == CharacterStatus::Active
                })
        })
        .count()
}

const fn ward_capabilities(focus: EducationFocus) -> CharacterCapabilities {
    match focus {
        EducationFocus::Administration => CharacterCapabilities {
            administration: 62,
            commerce: 42,
            social: 45,
            craft: 35,
        },
        EducationFocus::Commerce => CharacterCapabilities {
            administration: 45,
            commerce: 62,
            social: 42,
            craft: 35,
        },
        EducationFocus::Social => CharacterCapabilities {
            administration: 45,
            commerce: 42,
            social: 62,
            craft: 35,
        },
        EducationFocus::Craft => CharacterCapabilities {
            administration: 40,
            commerce: 42,
            social: 40,
            craft: 62,
        },
    }
}

fn apply_family_education(
    state: &mut AppState,
    character_id: CharacterId,
    focus: EducationFocus,
) -> Result<CommandOutcome, CommandError> {
    let character = state
        .characters
        .get(character_id)
        .ok_or(CommandError::InvalidFamilyStudent { character_id })?;
    if character.dynasty_id() != state.player_dynasty_id
        || character.status() != CharacterStatus::Active
    {
        return Err(CommandError::InvalidFamilyStudent { character_id });
    }
    if education_focus_value(&character.capabilities, focus) >= 100 {
        return Err(CommandError::FamilyEducationAtMaximum {
            character_id,
            focus,
        });
    }
    if let Some(last_education_day) = state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::FamilyEducation)
        .map(AuditRecord::day)
    {
        let next_education_day = last_education_day.saturating_add(FAMILY_EDUCATION_INTERVAL_DAYS);
        if state.clock.day() < next_education_day {
            return Err(CommandError::FamilyEducationCooldown { next_education_day });
        }
    }
    spend_player_treasury(state, FAMILY_EDUCATION_COST)?;
    let character = state
        .characters
        .get_mut(character_id)
        .expect("validated family student must exist");
    apply_education_focus(&mut character.capabilities, focus);
    if focus == EducationFocus::Administration {
        let dynasty = state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        dynasty.resources.administrative_capacity =
            dynasty.resources.administrative_capacity.saturating_add(2);
    }
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::FamilyEducation,
        subject: format!(
            "dynasty:{}:character:{character_id}",
            state.player_dynasty_id
        )
        .into(),
        detail: format!("focus={focus:?};cost={}", FAMILY_EDUCATION_COST.copper()),
    });
    super::strategic::push_outbox(
        state,
        OutboxKind::Family,
        format!("Family education completed for character {character_id}"),
        format!("The dynasty spent {FAMILY_EDUCATION_COST} on advanced {focus:?} training."),
    );
    Ok(CommandOutcome {
        summary: format!("Educated character {character_id} in {focus:?}."),
    })
}

const fn education_focus_value(capabilities: &CharacterCapabilities, focus: EducationFocus) -> u16 {
    match focus {
        EducationFocus::Administration => capabilities.administration,
        EducationFocus::Commerce => capabilities.commerce,
        EducationFocus::Social => capabilities.social,
        EducationFocus::Craft => capabilities.craft,
    }
}

fn apply_education_focus(capabilities: &mut CharacterCapabilities, focus: EducationFocus) {
    match focus {
        EducationFocus::Administration => {
            capabilities.administration = capabilities.administration.saturating_add(8).min(100);
            capabilities.social = capabilities.social.saturating_add(2).min(100);
        }
        EducationFocus::Commerce => {
            capabilities.commerce = capabilities.commerce.saturating_add(8).min(100);
            capabilities.administration = capabilities.administration.saturating_add(2).min(100);
        }
        EducationFocus::Social => {
            capabilities.social = capabilities.social.saturating_add(8).min(100);
            capabilities.commerce = capabilities.commerce.saturating_add(2).min(100);
        }
        EducationFocus::Craft => {
            capabilities.craft = capabilities.craft.saturating_add(8).min(100);
            capabilities.commerce = capabilities.commerce.saturating_add(2).min(100);
        }
    }
}

fn apply_institution_support(
    state: &mut AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Result<CommandOutcome, CommandError> {
    let character = state
        .characters
        .get(character_id)
        .ok_or(CommandError::InvalidNominee { character_id })?;
    if character.dynasty_id() != state.player_dynasty_id
        || character.status() != CharacterStatus::Active
    {
        return Err(CommandError::InvalidNominee { character_id });
    }
    validate_institution_support_standing(state)?;
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    let subject = institution_support_subject(institution_id, character_id);
    if institution.members.contains(&character_id) {
        return Err(CommandError::InstitutionSupportAlreadyEstablished {
            institution_id,
            character_id,
        });
    }
    let membership_count = institution_membership_count(state, character_id);
    if membership_count >= MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER {
        return Err(CommandError::InstitutionMembershipCapacity {
            character_id,
            current: membership_count,
            maximum: MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER,
        });
    }
    if let Some(next_support_day) = institution_support_next_day(state, character_id)
        && state.clock.day() < next_support_day
    {
        return Err(CommandError::InstitutionSupportCooldown { next_support_day });
    }
    let budget_after = institution
        .budget
        .checked_add(INSTITUTION_SUPPORT_COST)
        .ok_or(CommandError::InstitutionBudgetOverflow {
            institution_id,
            current: institution.budget,
            incoming: INSTITUTION_SUPPORT_COST,
        })?;
    let member_dynasties: BTreeSet<_> = institution
        .members
        .iter()
        .filter_map(|member_id| state.characters.get(*member_id))
        .map(Character::dynasty_id)
        .filter(|dynasty_id| *dynasty_id != state.player_dynasty_id)
        .collect();
    spend_player_treasury(state, INSTITUTION_SUPPORT_COST)?;
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    institution.budget = budget_after;
    institution.members.insert(character_id);
    let dynasty = state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.legitimacy_basis_points = dynasty
        .resources
        .legitimacy_basis_points
        .saturating_add(250)
        .min(10_000);
    record_institution_patronage_relationships(
        state,
        institution_id,
        character_id,
        member_dynasties,
    );
    Ok(finish_institution_patronage(
        state,
        institution_id,
        character_id,
        subject,
    ))
}

fn record_institution_patronage_relationships(
    state: &mut AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
    member_dynasties: BTreeSet<DynastyId>,
) {
    let player_dynasty_id = state.player_dynasty_id;
    for member_dynasty_id in member_dynasties {
        super::strategic::adjust_dynasty_relationship(
            state,
            player_dynasty_id,
            member_dynasty_id,
            super::strategic::RelationshipDelta::new(180, 260, 0, -60, 75),
        );
        super::strategic::remember_dynasty_interaction(
            state,
            player_dynasty_id,
            member_dynasty_id,
            &format!(
                "the player dynasty patronized institution {institution_id} for character {character_id}"
            ),
        );
    }
}

fn finish_institution_patronage(
    state: &mut AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
    subject: String,
) -> CommandOutcome {
    let day = state.clock.day();
    state.audit_log.push(AuditRecord {
        day,
        kind: AuditKind::InstitutionPatronage,
        subject: subject.into(),
        detail: format!("contribution={}", INSTITUTION_SUPPORT_COST.copper()),
    });
    let established_day = day.saturating_add(INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS);
    super::strategic::push_outbox(
        state,
        OutboxKind::Politics,
        format!("Institutional support cultivated for character {character_id}"),
        format!(
            "The dynasty patronized institution {institution_id}; character {character_id}'s support will be established by day {established_day}."
        ),
    );
    CommandOutcome {
        summary: format!(
            "Cultivated support for character {character_id} in institution {institution_id}."
        ),
    }
}

fn validate_institution_support_standing(state: &AppState) -> Result<(), CommandError> {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let quality = player.resources.reputation_quality_basis_points;
    let reliability = player.resources.reputation_reliability_basis_points;
    if quality.max(reliability) < INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT {
        return Err(CommandError::InsufficientInstitutionSupportReputation {
            quality,
            reliability,
            required: INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT,
        });
    }
    let delivered = player_contract_deliveries(state);
    if delivered < INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT {
        return Err(
            CommandError::InsufficientInstitutionSupportCommercialRecord {
                delivered,
                required: INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT,
            },
        );
    }
    Ok(())
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
    validate_office_nomination_standing(state)?;
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    if !institution.members.contains(&character_id) {
        return Err(CommandError::MissingInstitutionSupport {
            institution_id,
            character_id,
        });
    }
    let support_day = institution_support_day(state, institution_id, character_id).ok_or(
        CommandError::MissingInstitutionSupport {
            institution_id,
            character_id,
        },
    )?;
    let available_day = support_day.saturating_add(INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS);
    if state.clock.day() < available_day {
        return Err(CommandError::InstitutionSupportNotEstablished {
            institution_id,
            character_id,
            available_day,
        });
    }
    if let Some(next_nomination_day) = office_nomination_next_day(state, character_id)
        && state.clock.day() < next_nomination_day
    {
        return Err(CommandError::OfficeNominationCooldown {
            next_nomination_day,
        });
    }
    let campaign_cost = Money::from_copper(300);
    spend_player_treasury(state, campaign_cost)?;
    let selection_day = state
        .clock
        .day()
        .saturating_add(OFFICE_NOMINATION_RESOLUTION_DAYS);
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
        subject: subject.into(),
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

#[derive(Debug)]
struct OfficePowerDirectivePlan {
    institution_id: InstitutionId,
    district_id: DistrictId,
    power: OfficePower,
    legitimacy: u16,
    subject: String,
}

fn validate_office_power_directive(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    power: OfficePower,
) -> Result<OfficePowerDirectivePlan, CommandError> {
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    let holder_is_player = institution.office_holder_id.is_some_and(|character_id| {
        state.characters.get(character_id).is_some_and(|character| {
            character.status() == CharacterStatus::Active
                && character.dynasty_id() == state.player_dynasty_id
        })
    });
    if !holder_is_player || !institution.powers.contains(&power) {
        return Err(CommandError::OfficePowerUnavailable {
            institution_id,
            power,
        });
    }
    let available_day = institution
        .term_started_day
        .saturating_add(OFFICE_POWER_ESTABLISHMENT_DAYS);
    if state.clock.day() < available_day {
        return Err(CommandError::OfficePowerDirectiveNotEstablished {
            institution_id,
            power,
            available_day,
        });
    }
    let legitimacy = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .legitimacy_basis_points;
    if legitimacy < OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST {
        return Err(CommandError::InsufficientPlayerLegitimacy {
            available: legitimacy,
            required: OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST,
        });
    }
    let subject = format!("institution:{institution_id}");
    if let Some(last_directive_day) = state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::OfficeDirective && record.subject() == subject)
        .map(AuditRecord::day)
    {
        let next_directive_day =
            last_directive_day.saturating_add(OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS);
        if state.clock.day() < next_directive_day {
            return Err(CommandError::OfficePowerDirectiveCooldown {
                institution_id,
                power,
                next_directive_day,
            });
        }
    }
    let district_id = registry
        .get_institution(institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?
        .district_id();
    Ok(OfficePowerDirectivePlan {
        institution_id,
        district_id,
        power,
        legitimacy,
        subject,
    })
}

fn improve_player_reputation(state: &mut AppState, quality: u16, reliability: u16) {
    let dynasty = state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.reputation_quality_basis_points = dynasty
        .resources
        .reputation_quality_basis_points
        .saturating_add(quality)
        .min(10_000);
    dynasty.resources.reputation_reliability_basis_points = dynasty
        .resources
        .reputation_reliability_basis_points
        .saturating_add(reliability)
        .min(10_000);
}

fn adjust_directive_district(
    state: &mut AppState,
    district_id: DistrictId,
    employment: u16,
    sanitation: u16,
    safety: u16,
    unrest: i16,
) {
    let district = state
        .districts
        .get_mut(&district_id)
        .expect("validated institution district must exist");
    district.employment_basis_points = district
        .employment_basis_points
        .saturating_add(employment)
        .min(10_000);
    district.sanitation_basis_points = district
        .sanitation_basis_points
        .saturating_add(sanitation)
        .min(10_000);
    district.safety_basis_points = district
        .safety_basis_points
        .saturating_add(safety)
        .min(10_000);
    district.unrest_basis_points = if unrest >= 0 {
        district
            .unrest_basis_points
            .saturating_add(unrest.unsigned_abs())
            .min(10_000)
    } else {
        district
            .unrest_basis_points
            .saturating_sub(unrest.unsigned_abs())
    };
}

fn apply_office_power_directive_effect(
    state: &mut AppState,
    institution_id: InstitutionId,
    district_id: DistrictId,
    power: OfficePower,
) {
    match power {
        OfficePower::Licenses => {
            adjust_directive_district(state, district_id, 250, 0, 0, 0);
            improve_player_reputation(state, 50, 0);
        }
        OfficePower::Inspections => {
            adjust_directive_district(state, district_id, 0, 300, 0, 50);
            improve_player_reputation(state, 100, 0);
        }
        OfficePower::MarketTolls => {
            adjust_directive_district(state, district_id, 0, 0, 0, 150);
            raise_institution_legitimacy(state, institution_id, 100);
        }
        OfficePower::DebtEnforcement => {
            adjust_directive_district(state, district_id, 0, 0, 0, 100);
            improve_player_reputation(state, 0, 100);
        }
        OfficePower::CityContracts => {
            adjust_directive_district(state, district_id, 250, 0, 0, 0);
            improve_player_reputation(state, 75, 75);
        }
        OfficePower::PublicWorks => adjust_directive_district(state, district_id, 200, 200, 0, 0),
        OfficePower::WatchPriorities => {
            adjust_directive_district(state, district_id, 0, 0, 350, -150);
        }
        OfficePower::Taxation => {
            adjust_directive_district(state, district_id, 0, 0, 0, 250);
            raise_institution_legitimacy(state, institution_id, 150);
        }
        OfficePower::EmergencyImports => {
            adjust_directive_district(state, district_id, 0, 0, 0, -200);
            for household in state
                .households
                .iter_mut()
                .filter(|household| household.district_id() == district_id)
            {
                household.food_satisfaction_basis_points = household
                    .food_satisfaction_basis_points
                    .saturating_add(300)
                    .min(10_000);
            }
        }
    }
}

fn raise_institution_legitimacy(state: &mut AppState, institution_id: InstitutionId, amount: u16) {
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    institution.legitimacy_basis_points = institution
        .legitimacy_basis_points
        .saturating_add(amount)
        .min(10_000);
}

fn apply_office_power_directive(
    registry: &Registry,
    state: &mut AppState,
    institution_id: InstitutionId,
    power: OfficePower,
) -> Result<CommandOutcome, CommandError> {
    let OfficePowerDirectivePlan {
        institution_id,
        district_id,
        power,
        legitimacy,
        subject,
    } = validate_office_power_directive(registry, state, institution_id, power)?;
    state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .legitimacy_basis_points = legitimacy
        .checked_sub(OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST)
        .expect("validated office directive legitimacy cost must fit");
    apply_office_power_directive_effect(state, institution_id, district_id, power);
    let directive_expires_day = state
        .clock
        .day()
        .saturating_add(OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS);
    state
        .institutions
        .get_mut(&institution_id)
        .expect("validated directive institution must exist")
        .active_directive = Some(OfficeDirectiveState {
        power,
        expires_day: directive_expires_day,
    });
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::OfficeDirective,
        subject: subject.into(),
        detail: format!(
            "district={district_id};power={power:?};legitimacy_cost={OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST}"
        ),
    });
    let chronicle_id = state.next_ids.chronicle();
    state.chronicle.push(ChronicleEntry {
        id: chronicle_id,
        day: state.clock.day(),
        kind: ChronicleKind::OfficeDirective,
        summary: format!(
            "The player dynasty directed institution {institution_id} to exercise {power:?} in district {district_id}."
        ),
    });
    super::strategic::push_outbox(
        state,
        OutboxKind::Politics,
        format!("{power:?} directive issued through institution {institution_id}"),
        format!(
            "The dynasty spent {OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST} legitimacy to intensify {power:?} policy in district {district_id}."
        ),
    );
    Ok(CommandOutcome {
        summary: format!("Exercised {power:?} through institution {institution_id}."),
    })
}

fn validate_office_nomination_standing(state: &AppState) -> Result<(), CommandError> {
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
    let delivered = player_contract_deliveries(state);
    if delivered < OFFICE_NOMINATION_DELIVERY_REQUIREMENT {
        return Err(CommandError::InsufficientOfficeCommercialRecord {
            delivered,
            required: OFFICE_NOMINATION_DELIVERY_REQUIREMENT,
        });
    }
    Ok(())
}

pub(super) fn office_nomination_subject(
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> String {
    format!("institution:{institution_id}:character:{character_id}")
}

pub(crate) fn office_nomination_next_day(
    state: &AppState,
    character_id: CharacterId,
) -> Option<i64> {
    latest_character_campaign_day(state, AuditKind::OfficeNomination, character_id).map(|day| {
        let resolution_day = day.saturating_add(OFFICE_NOMINATION_RESOLUTION_DAYS);
        let interval = if state.clock.day() < resolution_day {
            OFFICE_NOMINATION_INTERVAL_DAYS
        } else {
            OFFICE_NOMINATION_RECOVERY_DAYS
        };
        day.saturating_add(interval)
    })
}

pub(crate) fn institution_support_subject(
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> String {
    format!("institution:{institution_id}:character:{character_id}")
}

pub(crate) fn institution_support_next_day(
    state: &AppState,
    character_id: CharacterId,
) -> Option<i64> {
    latest_character_campaign_day(state, AuditKind::InstitutionPatronage, character_id)
        .map(|day| day.saturating_add(INSTITUTION_SUPPORT_INTERVAL_DAYS))
}

pub(crate) fn institution_membership_count(state: &AppState, character_id: CharacterId) -> usize {
    state
        .institutions
        .values()
        .filter(|institution| institution.members.contains(&character_id))
        .count()
}

fn latest_character_campaign_day(
    state: &AppState,
    kind: AuditKind,
    character_id: CharacterId,
) -> Option<i64> {
    let suffix = format!(":character:{character_id}");
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == kind && record.subject().ends_with(&suffix))
        .map(AuditRecord::day)
}

pub(crate) fn institution_support_day(
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Option<i64> {
    let subject = institution_support_subject(institution_id, character_id);
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::InstitutionPatronage && record.subject() == subject
        })
        .map(AuditRecord::day)
}

pub(crate) fn player_contract_deliveries(state: &AppState) -> u32 {
    state.contracts.values().fold(0_u32, |total, contract| {
        total.saturating_add(u32::from(
            contract
                .fulfilled_deliveries_by_dynasty
                .get(&state.player_dynasty_id)
                .copied()
                .unwrap_or(0),
        ))
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
    let subject = validate_crisis_response_history(state, crisis_id, response)?;
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
        subject: subject.into(),
        detail: format!("response={response:?}"),
    });
    Ok(CommandOutcome {
        summary: format!("Applied {response:?} response to crisis {crisis_id}."),
    })
}

fn validate_crisis_response_history(
    state: &AppState,
    crisis_id: CrisisId,
    response: CrisisResponse,
) -> Result<String, CommandError> {
    let subject = format!("crisis:{crisis_id}");
    let prior_responses: Vec<_> = state
        .audit_log
        .iter()
        .rev()
        .filter(|record| record.kind() == AuditKind::CrisisResponse && record.subject() == subject)
        .collect();
    let has_containment_response = prior_responses
        .iter()
        .any(|record| super::strategic::crisis_response_contains_crisis(record));
    let has_exploitation_response = prior_responses
        .iter()
        .any(|record| record.detail() == "response=Exploit");
    if has_containment_response
        || (response == CrisisResponse::Exploit && has_exploitation_response)
    {
        return Err(CommandError::CrisisAlreadyAddressed { crisis_id });
    }
    Ok(subject)
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

fn validate_negotiated_weekly_wage(
    agreement: &crate::core::EmploymentAgreement,
    employment_id: EmploymentId,
    response: LaborResponse,
) -> Result<Option<Money>, CommandError> {
    match response {
        LaborResponse::Negotiate => agreement
            .weekly_wage
            .checked_mul_ratio(11, 10)
            .map(Some)
            .ok_or(CommandError::LaborWageOverflow {
                employment_id,
                current: agreement.weekly_wage,
            }),
        LaborResponse::ImproveConditions | LaborResponse::ReplaceWorkers => Ok(None),
    }
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
    let negotiated_weekly_wage =
        validate_negotiated_weekly_wage(agreement, employment_id, response)?;
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
            agreement.weekly_wage =
                negotiated_weekly_wage.expect("negotiated wage must be prevalidated");
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
    let business = state
        .businesses
        .get(business_id)
        .ok_or(CommandError::MissingBusiness { business_id })?;
    let cash = business.cash();
    if cash < amount {
        return Err(CommandError::InsufficientBusinessFunds {
            business_id,
            available: cash,
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

#[derive(Debug)]
struct InformationCommissionPlan {
    target: InformationTarget,
    subject: String,
    summary: String,
}

fn commission_information(
    registry: &Registry,
    state: &mut AppState,
    focus: InformationFocus,
) -> Result<CommandOutcome, CommandError> {
    let plan = resolve_information_commission(registry, state, focus)?;
    spend_player_treasury(state, INFORMATION_COMMISSION_COST)?;
    state.information_reports.retain(|_, report| {
        report.owner_dynasty_id != state.player_dynasty_id || report.target != Some(plan.target)
    });
    let id = state.next_ids.information_report();
    let day = state.clock.day();
    state.information_reports.insert(
        id,
        InformationReport {
            id,
            owner_dynasty_id: state.player_dynasty_id,
            target: Some(plan.target),
            subject: plan.subject.clone(),
            confidence: InformationConfidence::Confirmed,
            created_day: day,
            expires_day: day.saturating_add(INFORMATION_REPORT_LIFETIME_DAYS),
            source: COMMISSIONED_INFORMATION_SOURCE.to_owned(),
            summary: plan.summary,
        },
    );
    state.audit_log.push(AuditRecord {
        day,
        kind: AuditKind::InformationCommission,
        subject: format!("dynasty:{}", state.player_dynasty_id).into(),
        detail: format!("report={id};subject={}", plan.subject),
    });
    super::strategic::push_outbox(
        state,
        OutboxKind::Information,
        "Commissioned intelligence delivered".to_owned(),
        format!("{} is now available to the dynasty.", plan.subject),
    );
    Ok(CommandOutcome {
        summary: format!("Commissioned intelligence report {id}: {}.", plan.subject),
    })
}

fn resolve_information_commission(
    registry: &Registry,
    state: &AppState,
    focus: InformationFocus,
) -> Result<InformationCommissionPlan, CommandError> {
    let report_commission_day = state
        .information_reports
        .values()
        .filter(|report| {
            report.owner_dynasty_id == state.player_dynasty_id
                && report.source == COMMISSIONED_INFORMATION_SOURCE
        })
        .map(|report| report.created_day)
        .max();
    let audit_subject = format!("dynasty:{}", state.player_dynasty_id);
    let audit_commission_day = state
        .audit_log
        .iter()
        .filter(|record| {
            record.kind() == AuditKind::InformationCommission && record.subject() == audit_subject
        })
        .map(AuditRecord::day)
        .max();
    if let Some(last_commission_day) = report_commission_day.max(audit_commission_day) {
        let next_commission_day =
            last_commission_day.saturating_add(INFORMATION_COMMISSION_INTERVAL_DAYS);
        if state.clock.day() < next_commission_day {
            return Err(CommandError::InformationCommissionCooldown {
                next_commission_day,
            });
        }
    }
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    if treasury < INFORMATION_COMMISSION_COST {
        return Err(CommandError::InsufficientPlayerFunds {
            available: treasury,
            required: INFORMATION_COMMISSION_COST,
        });
    }
    match focus {
        InformationFocus::Market { good_id } => {
            resolve_market_information(registry, state, good_id)
        }
        InformationFocus::Counterparty { dynasty_id } => {
            resolve_counterparty_information(state, dynasty_id)
        }
        InformationFocus::District { district_id } => {
            resolve_district_information(registry, state, district_id)
        }
    }
}

fn resolve_market_information(
    registry: &Registry,
    state: &AppState,
    good_id: GoodId,
) -> Result<InformationCommissionPlan, CommandError> {
    let good = registry
        .get_good(good_id)
        .ok_or(CommandError::MissingGood { good_id })?;
    let quote = state
        .market
        .quotes
        .get(&good_id)
        .ok_or(CommandError::MissingMarketQuote { good_id })?;
    Ok(InformationCommissionPlan {
        target: InformationTarget::Market { good_id },
        subject: format!("Commissioned market brief: {}", good.name()),
        summary: format!(
            "Price {}; previous price {}; stock {}; target stock {}; today's demand {}; today's supply {}; recorded causes {:?}.",
            quote.price,
            quote.previous_price,
            quote.stock,
            quote.target_stock,
            quote.demand_today,
            quote.supply_today,
            quote.causes
        ),
    })
}

fn resolve_counterparty_information(
    state: &AppState,
    dynasty_id: DynastyId,
) -> Result<InformationCommissionPlan, CommandError> {
    if dynasty_id == state.player_dynasty_id {
        return Err(CommandError::InformationCannotTargetPlayer);
    }
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .ok_or(CommandError::MissingDynasty { dynasty_id })?;
    let relationship = state
        .relationships
        .get(&DynastyPair::new(state.player_dynasty_id, dynasty_id))
        .expect("every dynasty pair must have a relationship record");
    let unsettled_credit = state
        .loans
        .values()
        .filter(|loan| {
            ((loan.lender_dynasty_id == state.player_dynasty_id
                && loan.borrower_dynasty_id == dynasty_id)
                || (loan.lender_dynasty_id == dynasty_id
                    && loan.borrower_dynasty_id == state.player_dynasty_id))
                && !matches!(loan.status, crate::core::LoanStatus::Repaid)
        })
        .count();
    Ok(InformationCommissionPlan {
        target: InformationTarget::Counterparty { dynasty_id },
        subject: format!("Commissioned house brief: House {}", dynasty.name()),
        summary: format!(
            "Treasury {}; reliability {} bp; trust {} bp; respect {} bp; fear {} bp; resentment {} bp; obligation {}; unsettled bilateral credit {}.",
            dynasty.treasury(),
            dynasty.resources.reputation_reliability_basis_points,
            relationship.trust_basis_points,
            relationship.respect_basis_points,
            relationship.fear_basis_points,
            relationship.resentment_basis_points,
            relationship.obligation,
            unsettled_credit
        ),
    })
}

fn resolve_district_information(
    registry: &Registry,
    state: &AppState,
    district_id: DistrictId,
) -> Result<InformationCommissionPlan, CommandError> {
    let district = registry
        .get_district(district_id)
        .ok_or(CommandError::MissingDistrict { district_id })?;
    let runtime = state
        .districts
        .get(&district_id)
        .ok_or(CommandError::MissingDistrict { district_id })?;
    Ok(InformationCommissionPlan {
        target: InformationTarget::District { district_id },
        subject: format!("Commissioned district brief: {}", district.name()),
        summary: format!(
            "Rent index {} bp; employment {} bp; sanitation {} bp; safety {} bp; unrest {} bp; population {}.",
            runtime.rent_index_basis_points,
            runtime.employment_basis_points,
            runtime.sanitation_basis_points,
            runtime.safety_basis_points,
            runtime.unrest_basis_points,
            district.population()
        ),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InformationLeverageQuote {
    pub report_id: InformationReportId,
    pub cost: Money,
    pub description: String,
}

#[derive(Clone, Copy, Debug)]
enum DistrictInformationInitiative {
    Employment,
    Sanitation,
    Safety,
}

impl DistrictInformationInitiative {
    const fn label(self) -> &'static str {
        match self {
            Self::Employment => "employment",
            Self::Sanitation => "sanitation",
            Self::Safety => "safety",
        }
    }
}

#[derive(Debug)]
enum InformationLeverageEffect {
    Contract {
        contract_id: ContractId,
        counterparty_id: DynastyId,
        previous_price: Money,
        new_price: Money,
    },
    Counterparty {
        dynasty_id: DynastyId,
    },
    District {
        district_id: DistrictId,
        initiative: DistrictInformationInitiative,
    },
}

#[derive(Debug)]
struct InformationLeveragePlan {
    quote: InformationLeverageQuote,
    effect: InformationLeverageEffect,
}

pub(crate) fn quote_information_leverage(
    registry: &Registry,
    state: &AppState,
    report_id: InformationReportId,
) -> Result<InformationLeverageQuote, CommandError> {
    resolve_information_leverage(registry, state, report_id).map(|plan| plan.quote)
}

fn resolve_information_leverage(
    registry: &Registry,
    state: &AppState,
    report_id: InformationReportId,
) -> Result<InformationLeveragePlan, CommandError> {
    let report = state
        .information_reports
        .get(&report_id)
        .ok_or(CommandError::MissingInformationReport { report_id })?;
    if report.owner_dynasty_id != state.player_dynasty_id {
        return Err(CommandError::InformationReportNotOwned { report_id });
    }
    if report.source != COMMISSIONED_INFORMATION_SOURCE
        || report.confidence != InformationConfidence::Confirmed
    {
        return Err(CommandError::InformationReportNotCommissioned { report_id });
    }
    if state.clock.day() > report.expires_day {
        return Err(CommandError::InformationReportExpired {
            report_id,
            expired_day: report.expires_day,
        });
    }
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    if treasury < INFORMATION_LEVERAGE_COST {
        return Err(CommandError::InsufficientPlayerFunds {
            available: treasury,
            required: INFORMATION_LEVERAGE_COST,
        });
    }

    match report.target {
        Some(InformationTarget::Market { good_id }) => {
            resolve_market_information_leverage(registry, state, report_id, good_id)
        }
        Some(InformationTarget::Counterparty { dynasty_id }) => {
            resolve_counterparty_information_leverage(state, report_id, dynasty_id)
        }
        Some(InformationTarget::District { district_id }) => {
            resolve_district_information_leverage(registry, state, report_id, district_id)
        }
        None => Err(CommandError::InformationReportHasNoLeverage { report_id }),
    }
}

fn resolve_market_information_leverage(
    registry: &Registry,
    state: &AppState,
    report_id: InformationReportId,
    good_id: GoodId,
) -> Result<InformationLeveragePlan, CommandError> {
    let good = registry
        .get_good(good_id)
        .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
    let player_id = state.player_dynasty_id;
    let contract = state.contracts.values().find(|contract| {
        if contract.status != ContractStatus::Active || contract.good_id != good_id {
            return false;
        }
        let buyer_owner = state
            .businesses
            .get(contract.buyer_business_id)
            .map(crate::core::Business::owner_dynasty_id);
        let seller_owner = state
            .businesses
            .get(contract.seller_business_id)
            .map(crate::core::Business::owner_dynasty_id);
        matches!(
            (buyer_owner, seller_owner),
            (Some(buyer), Some(seller))
                if (buyer == player_id && seller != player_id)
                    || (seller == player_id && buyer != player_id)
        )
    });
    let contract = contract.ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
    let buyer_owner = state
        .businesses
        .get(contract.buyer_business_id)
        .expect("validated contract buyer must exist")
        .owner_dynasty_id();
    let seller_owner = state
        .businesses
        .get(contract.seller_business_id)
        .expect("validated contract seller must exist")
        .owner_dynasty_id();
    let one_copper = Money::from_copper(1);
    let (counterparty_id, new_price) = if buyer_owner == player_id {
        let discounted = contract
            .unit_price
            .checked_mul_ratio(95, 100)
            .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
        let one_copper_less = contract
            .unit_price
            .checked_sub(one_copper)
            .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
        (
            seller_owner,
            discounted.min(one_copper_less).max(one_copper),
        )
    } else {
        let increased = contract
            .unit_price
            .checked_mul_ratio(105, 100)
            .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
        let one_copper_more = contract
            .unit_price
            .checked_add(one_copper)
            .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
        (buyer_owner, increased.max(one_copper_more))
    };
    if new_price == contract.unit_price {
        return Err(CommandError::InformationReportHasNoLeverage { report_id });
    }
    let description = format!(
        "use report {report_id} to renegotiate {} contract {} from {} to {} per unit",
        good.name(),
        contract.id,
        contract.unit_price,
        new_price
    );
    Ok(InformationLeveragePlan {
        quote: InformationLeverageQuote {
            report_id,
            cost: INFORMATION_LEVERAGE_COST,
            description,
        },
        effect: InformationLeverageEffect::Contract {
            contract_id: contract.id,
            counterparty_id,
            previous_price: contract.unit_price,
            new_price,
        },
    })
}

fn resolve_counterparty_information_leverage(
    state: &AppState,
    report_id: InformationReportId,
    dynasty_id: DynastyId,
) -> Result<InformationLeveragePlan, CommandError> {
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
    let pair = DynastyPair::new(state.player_dynasty_id, dynasty_id);
    if !state.relationships.contains_key(&pair) {
        return Err(CommandError::InformationReportHasNoLeverage { report_id });
    }
    Ok(InformationLeveragePlan {
        quote: InformationLeverageQuote {
            report_id,
            cost: INFORMATION_LEVERAGE_COST,
            description: format!(
                "use report {report_id} for targeted outreach to House {}",
                dynasty.name()
            ),
        },
        effect: InformationLeverageEffect::Counterparty { dynasty_id },
    })
}

fn resolve_district_information_leverage(
    registry: &Registry,
    state: &AppState,
    report_id: InformationReportId,
    district_id: DistrictId,
) -> Result<InformationLeveragePlan, CommandError> {
    let district_definition = registry
        .get_district(district_id)
        .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
    let district = state
        .districts
        .get(&district_id)
        .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
    let initiative = [
        (
            district.employment_basis_points,
            DistrictInformationInitiative::Employment,
        ),
        (
            district.sanitation_basis_points,
            DistrictInformationInitiative::Sanitation,
        ),
        (
            district.safety_basis_points,
            DistrictInformationInitiative::Safety,
        ),
    ]
    .into_iter()
    .min_by_key(|(value, _)| *value)
    .map(|(_, initiative)| initiative)
    .expect("district initiative list must be nonempty");
    Ok(InformationLeveragePlan {
        quote: InformationLeverageQuote {
            report_id,
            cost: INFORMATION_LEVERAGE_COST,
            description: format!(
                "use report {report_id} to fund a targeted {} initiative in {}",
                initiative.label(),
                district_definition.name()
            ),
        },
        effect: InformationLeverageEffect::District {
            district_id,
            initiative,
        },
    })
}

fn leverage_information(
    registry: &Registry,
    state: &mut AppState,
    report_id: InformationReportId,
) -> Result<CommandOutcome, CommandError> {
    let plan = resolve_information_leverage(registry, state, report_id)?;
    spend_player_treasury(state, plan.quote.cost)?;
    apply_information_leverage_effect(state, &plan.effect);
    state
        .information_reports
        .remove(&report_id)
        .expect("validated information report must exist");
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::InformationLeverage,
        subject: format!("information-report:{report_id}").into(),
        detail: plan.quote.description.clone(),
    });
    super::strategic::push_outbox(
        state,
        OutboxKind::Information,
        "Commissioned intelligence converted into action".to_owned(),
        format!(
            "{} at a cost of {}.",
            plan.quote.description, plan.quote.cost
        ),
    );
    Ok(CommandOutcome {
        summary: format!(
            "Leveraged intelligence report {report_id}: {}.",
            plan.quote.description
        ),
    })
}

fn apply_information_leverage_effect(state: &mut AppState, effect: &InformationLeverageEffect) {
    match *effect {
        InformationLeverageEffect::Contract {
            contract_id,
            counterparty_id,
            previous_price,
            new_price,
        } => {
            state
                .contracts
                .get_mut(&contract_id)
                .expect("validated contract must exist")
                .unit_price = new_price;
            let memory = format!(
                "intelligence-backed contract renegotiation changed unit price from {previous_price} to {new_price}"
            );
            adjust_information_relationship(state, counterparty_id, -75, 50, 125, &memory);
        }
        InformationLeverageEffect::Counterparty { dynasty_id } => {
            adjust_information_relationship(
                state,
                dynasty_id,
                300,
                200,
                -200,
                "targeted outreach based on a commissioned house brief",
            );
        }
        InformationLeverageEffect::District {
            district_id,
            initiative,
        } => {
            let district = state
                .districts
                .get_mut(&district_id)
                .expect("validated district must exist");
            match initiative {
                DistrictInformationInitiative::Employment => {
                    district.employment_basis_points = district
                        .employment_basis_points
                        .saturating_add(250)
                        .min(10_000);
                }
                DistrictInformationInitiative::Sanitation => {
                    district.sanitation_basis_points = district
                        .sanitation_basis_points
                        .saturating_add(250)
                        .min(10_000);
                }
                DistrictInformationInitiative::Safety => {
                    district.safety_basis_points =
                        district.safety_basis_points.saturating_add(250).min(10_000);
                }
            }
            district.unrest_basis_points = district.unrest_basis_points.saturating_sub(100);
            improve_player_reputation(state, 75, 75);
        }
    }
}

fn adjust_information_relationship(
    state: &mut AppState,
    counterparty_id: DynastyId,
    trust_change: i16,
    respect_change: i16,
    resentment_change: i16,
    memory: &str,
) {
    let day = state.clock.day();
    let relationship = state
        .relationships
        .get_mut(&DynastyPair::new(state.player_dynasty_id, counterparty_id))
        .expect("validated relationship must exist");
    relationship.trust_basis_points =
        adjust_basis_points(relationship.trust_basis_points, trust_change);
    relationship.respect_basis_points =
        adjust_basis_points(relationship.respect_basis_points, respect_change);
    relationship.resentment_basis_points =
        adjust_basis_points(relationship.resentment_basis_points, resentment_change);
    relationship.last_interaction_day = day;
    if relationship.memories.len() >= 12 {
        relationship.memories.remove(0);
    }
    relationship.memories.push(format!("Day {day}: {memory}"));
}

fn adjust_basis_points(value: u16, change: i16) -> u16 {
    if change >= 0 {
        value.saturating_add(change.unsigned_abs()).min(10_000)
    } else {
        value.saturating_sub(change.unsigned_abs())
    }
}

fn acknowledge(
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
