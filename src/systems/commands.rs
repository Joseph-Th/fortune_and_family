//! Canonical player-command validation and dispatch across simulation subsystems.

use super::legal::{
    LEGAL_CASE_FILING_COST, LEGAL_CASE_FILING_INTERVAL_DAYS, LEGAL_CASE_HEARING_DELAY_DAYS,
};
use super::transactions::{
    TimelineError, checked_future_day, credit_market_clearing_account,
    debit_market_clearing_account, next_business_finance_version, next_family_charter_version,
};
use super::{
    LoanTerms, OFFICE_POWER_ESTABLISHMENT_DAYS, StrategicError, SupplyContractTerms,
    acquire_business, available_supply_contract_capacity, business_recapitalization_target,
    buy_unowned_property, capitalize_owned_business, distribute_owned_business_cash,
    quote_property_liquidation, sell_owned_property, transfer_business_cash, validate_loan,
    validate_supply_contract,
};
use crate::core::{
    AppState, AuditKind, AuditRecord, BusinessStatus, Character, CharacterCapabilities,
    CharacterIdentity, CharacterRole, CharacterRuntime, CharacterStatus, ChronicleEntry,
    ChronicleKind, CivicDebt, CivicDebtStatus, ContractStatus, CrisisKind, CrisisStatus,
    DynastyPair, EmploymentAgreement, EmploymentStatus, EnactedLaw, FamilyLink, FamilyLinkKind,
    HouseGovernance, InformationConfidence, InformationReport, InformationTarget,
    InstitutionRuntime, LawKind, LegalCase, LegalCaseKind, LegalCaseStatus, LoanStatus,
    OfficeDirectiveState, OfficePower, OutboxKind, PublicWork, PublicWorkKind, PublicWorkStatus,
};
use crate::ids::{
    BusinessId, CharacterId, ContractId, CrisisId, DistrictId, DynastyId, EmploymentId,
    ExternalRouteId, GoodId, IdentifierAllocationError, InformationReportId, InstitutionId,
    LegalCaseId, OutboxMessageId, PropertyId, PublicWorkId,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BusinessPolicyInput {
    target_input_days: u16,
    target_output_days: u16,
    minimum_cash_reserve: Money,
    maintenance_basis_points: u16,
    quality_target_basis_points: u16,
}

pub(crate) const BUSINESS_POLICY_CHANGE_INTERVAL_DAYS: i64 = 180;
/// Maximum idle cash a policy may hold: one year of operating cover.
const BUSINESS_RESERVE_MAX_OPERATING_DAYS: i64 = 360;
/// Weeks between wage renegotiations for one business. Wage posture is a
/// standing labor commitment, so it changes on a slower cadence than policy.
pub(crate) const BUSINESS_WAGE_CHANGE_INTERVAL_DAYS: i64 = 90;
/// Upper bound on the weekly wage per worker the command accepts. It keeps
/// checked arithmetic unreachable while still allowing generous wages.
pub(crate) const MAX_WEEKLY_WAGE_PER_WORKER: Money = Money::from_copper(400);
pub(crate) const LAW_SPONSORSHIP_INTERVAL_DAYS: i64 = 360;
pub(crate) const LAW_SPONSORSHIP_COST: Money = Money::from_copper(2_000);
pub(crate) const LAW_LEGITIMACY_REQUIREMENT: u16 = 3_000;
pub(crate) const LAW_LEGITIMACY_COST: u16 = 250;
pub(crate) const CIVIC_DEBT_INTEREST_BASIS_POINTS: u16 = 600;
pub(crate) const CIVIC_DEBT_TERM_WEEKS: i64 = 104;
pub(crate) const CIVIC_DEBT_CREDITOR_RESERVE: Money = Money::from_copper(10_000);
pub(crate) const PRIVATE_LOAN_COUNTERPARTY_RESERVE: Money = Money::from_copper(10_000);
pub(crate) const PRIVATE_LOAN_COUNTERPARTY_BORROWER_LIQUIDITY_TARGET: Money =
    Money::from_copper(25_000);
const PRIVATE_LOAN_COUNTERPARTY_MIN_INTEREST_BASIS_POINTS: u16 = 400;
const PRIVATE_LOAN_COUNTERPARTY_MAX_INTEREST_BASIS_POINTS: u16 = 2_500;
const PRIVATE_LOAN_COUNTERPARTY_MAX_AMORTIZATION_WEEKS: i64 = 260;
const PRIVATE_LOAN_COUNTERPARTY_MIN_AMORTIZATION_WEEKS: i64 = 13;
const PRIVATE_LOAN_DISTRESSED_BORROWER_MIN_AMORTIZATION_WEEKS: i64 = 8;
const PRIVATE_LOAN_COUNTERPARTY_MIN_COLLATERAL_LTV_BASIS_POINTS: i64 = 2_000;
pub(crate) const PROPERTY_COUNTERPARTY_BUYER_RESERVE: Money = Money::from_copper(10_000);
pub(crate) const PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS: i64 = 360;
pub(crate) const MAX_ACTIVE_SPONSORED_PUBLIC_WORKS: usize = 2;
pub(crate) const PUBLIC_WORK_MINIMUM_BUDGET: Money = Money::from_copper(1_000);
pub(crate) const LABOR_REPLACEMENT_COST: Money = Money::from_copper(750);
pub(crate) const LABOR_CONDITIONS_IMPROVEMENT_COST: Money = Money::from_copper(1_000);
pub(crate) const LABOR_NEGOTIATION_COST: Money = Money::from_copper(500);
pub(crate) const HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS: i64 = 1_080;
pub(crate) const HOUSE_GOVERNANCE_UNITY_COST: u16 = 250;
pub(crate) const FAMILY_COUNCIL_MEETING_INTERVAL_DAYS: i64 = 360;
pub(crate) const FAMILY_COUNCIL_MEETING_COST: Money = Money::from_copper(2_500);
const FAMILY_COUNCIL_MEETING_UNITY_GAIN: u16 = 1_500;
const FAMILY_COUNCIL_MEETING_LOYALTY_GAIN: u16 = 600;
pub(crate) const HEIR_DESIGNATION_INTERVAL_DAYS: i64 = 720;
pub(crate) const HEIR_DESIGNATION_LEGITIMACY_COST: u16 = 300;
pub(crate) const HEIR_DESIGNATION_UNITY_COST: u16 = 250;
pub(crate) const HEIR_MINIMUM_AGE_DAYS: i64 = 18 * 360;
pub(crate) const OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS: i64 = 180;
pub(crate) const OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST: u16 = 100;
pub(crate) const INSTITUTION_SUPPORT_INTERVAL_DAYS: i64 = 360;
pub(crate) const INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS: i64 = 90;
/// Recovery period after a character withdraws from an institution.
pub(crate) const INSTITUTION_WITHDRAWAL_RECOVERY_DAYS: i64 = 720;
pub(crate) const INSTITUTION_SUPPORT_COST: Money = Money::from_copper(1_200);
pub(crate) const INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT: u16 = 5_500;
pub(crate) const INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT: u32 = 52;
pub(crate) const INSTITUTION_ENDOWMENT_INTERVAL_DAYS: i64 = 360;
pub(crate) const INSTITUTION_ENDOWMENT_MIN: Money = Money::from_copper(5_000);
pub(crate) const INSTITUTION_ENDOWMENT_MAX: Money = Money::from_copper(50_000);
const INSTITUTION_SUPPORT_CAPABILITY_TARGET_SCORE: u32 = 10_000;
const INSTITUTION_SUPPORT_CAPABILITY_DELIVERY_STEP: u32 = 200;
const INSTITUTION_SUPPORT_MAX_PREPARATION_DELIVERIES: u32 = 13;
pub(crate) const MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER: usize = 2;
pub(crate) const OFFICE_NOMINATION_RECOVERY_DAYS: i64 = 720;
pub(crate) const OFFICE_NOMINATION_RESOLUTION_DAYS: i64 = 120;
pub(crate) const OFFICE_NOMINATION_CAMPAIGN_COST: Money = Money::from_copper(300);
pub(crate) const OFFICE_NOMINATION_REPUTATION_REQUIREMENT: u16 = 5_500;
pub(crate) const OFFICE_NOMINATION_DELIVERY_REQUIREMENT: u32 = 78;
const OFFICE_NOMINATION_CAPABILITY_TARGET_SCORE: u32 = 10_000;
const OFFICE_NOMINATION_CAPABILITY_DELIVERY_STEP: u32 = 100;
const OFFICE_NOMINATION_MAX_PREPARATION_DELIVERIES: u32 = 26;
pub(crate) const WARD_ADOPTION_INTERVAL_DAYS: i64 = 720;
pub(crate) const WARD_ADOPTION_COST: Money = Money::from_copper(6_000);
pub(crate) const WARD_ADOPTION_LEGITIMACY_REQUIREMENT: u16 = 3_500;
pub(crate) const WARD_ADOPTION_REPUTATION_REQUIREMENT: u16 = 5_200;
pub(crate) const WARD_ADOPTION_DELIVERY_REQUIREMENT: u32 = 52;
pub(crate) const WARD_ADOPTION_UNITY_COST: u16 = 100;
const WARD_ADOPTION_LEGITIMACY_COST: u16 = 250;
pub(crate) const MAX_ACTIVE_WARDS: usize = 4;
pub(crate) const FAMILY_EDUCATION_INTERVAL_DAYS: i64 = 360;
pub(crate) const FAMILY_EDUCATION_DYNASTY_INTERVAL_DAYS: i64 = 180;
pub(crate) const FAMILY_EDUCATION_COST: Money = Money::from_copper(2_000);
pub(crate) const INFORMATION_COMMISSION_INTERVAL_DAYS: i64 = 360;
pub(crate) const INFORMATION_COMMISSION_COST: Money = Money::from_copper(600);
pub(crate) const INFORMATION_LEVERAGE_COST: Money = Money::from_copper(600);
/// Relief mobilization base in copper; see [`crisis_relief_cost`].
pub(crate) const CRISIS_RELIEF_BASE_COST_COPPER: i64 = 1_200;
/// Severity points per copper of scaling relief grant; see [`crisis_relief_cost`].
pub(crate) const CRISIS_RELIEF_SEVERITY_DIVISOR: i64 = 3;
pub(crate) const CRISIS_REFORM_COST: Money = Money::from_copper(1_500);
pub(crate) const CRISIS_SUPPRESS_COST: Money = Money::from_copper(900);
const CRISIS_RELIEF_LEGITIMACY_GAIN: u16 = 500;
const CRISIS_REFORM_LEGITIMACY_GAIN: u16 = 300;
const CRISIS_SUPPRESS_LEGITIMACY_COST: u16 = 450;
const CRISIS_RELIEF_UNREST_REDUCTION: u16 = 800;
const CRISIS_REFORM_UNREST_REDUCTION: u16 = 500;
const CRISIS_SUPPRESS_UNREST_INCREASE: u16 = 700;
const CRISIS_EXPLOIT_LEGITIMACY_REQUIREMENT: u16 = 600;
const CRISIS_EXPLOIT_SEVERITY_INCREASE: u16 = 500;
const CRISIS_EXPLOIT_LEGITIMACY_COST: u16 = 600;
const CRISIS_EXPLOIT_UNREST_INCREASE: u16 = 600;
pub(crate) const INFORMATION_REPORT_LIFETIME_DAYS: i64 = 540;
pub(crate) const COMMISSIONED_INFORMATION_SOURCE: &str = "Commissioned intelligence";

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CommandError {
    #[error(transparent)]
    IdentifierAllocation(#[from] IdentifierAllocationError),
    #[error(transparent)]
    Timeline(#[from] TimelineError),
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
    #[error(
        "non-player lender {lender_dynasty_id} must retain at least {required_reserve} after advancing {principal}; available treasury is {available}"
    )]
    LoanCounterpartyLenderReserve {
        lender_dynasty_id: DynastyId,
        available: Money,
        principal: Money,
        required_reserve: Money,
    },
    #[error(
        "non-player lender requires at least {minimum_basis_points} basis points of interest; proposed rate is {interest_basis_points}"
    )]
    LoanCounterpartyInterestTooLow {
        interest_basis_points: u16,
        minimum_basis_points: u16,
    },
    #[error(
        "non-player borrower accepts at most {maximum_basis_points} basis points of interest; proposed rate is {interest_basis_points}"
    )]
    LoanCounterpartyInterestTooHigh {
        interest_basis_points: u16,
        maximum_basis_points: u16,
    },
    #[error(
        "non-player lender requires a weekly payment of at least {minimum_payment}; proposed payment is {weekly_payment}"
    )]
    LoanCounterpartyPaymentTooLow {
        weekly_payment: Money,
        minimum_payment: Money,
    },
    #[error(
        "non-player borrower accepts a weekly payment of at most {maximum_payment}; proposed payment is {weekly_payment}"
    )]
    LoanCounterpartyPaymentTooHigh {
        weekly_payment: Money,
        maximum_payment: Money,
    },
    #[error(
        "non-player borrower will not pledge property {property_id} valued at {property_value} for exposure below {minimum_exposure}; proposed exposure is {exposure}"
    )]
    LoanCounterpartyCollateralTooLarge {
        property_id: PropertyId,
        property_value: Money,
        exposure: Money,
        minimum_exposure: Money,
    },
    #[error(
        "non-player borrower {borrower_dynasty_id} has no material financing pressure and declines unsolicited credit"
    )]
    LoanCounterpartyNoFinancingNeed { borrower_dynasty_id: DynastyId },
    #[error(
        "non-player contract seller requires at least {minimum_price} per unit; proposed price is {unit_price}"
    )]
    ContractCounterpartyPriceTooLow {
        unit_price: Money,
        minimum_price: Money,
    },
    #[error(
        "non-player contract buyer accepts at most {maximum_price} per unit; proposed price is {unit_price}"
    )]
    ContractCounterpartyPriceTooHigh {
        unit_price: Money,
        maximum_price: Money,
    },
    #[error(
        "contract counterparty accepts a penalty from {minimum_penalty} through {maximum_penalty}; proposed penalty is {penalty}"
    )]
    ContractCounterpartyPenaltyOutOfRange {
        penalty: Money,
        minimum_penalty: Money,
        maximum_penalty: Money,
    },
    #[error(
        "non-player business {business_id} has weekly contract capacity {available}, below requested quantity {requested}"
    )]
    ContractCounterpartyCapacity {
        business_id: BusinessId,
        requested: crate::money::Quantity,
        available: crate::money::Quantity,
    },
    #[error(
        "property buyer {buyer_dynasty_id} must retain at least {required_reserve} after contributing {buyer_contribution}; available treasury is {available}"
    )]
    PropertyCounterpartyBuyerReserve {
        buyer_dynasty_id: DynastyId,
        available: Money,
        buyer_contribution: Money,
        required_reserve: Money,
    },
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
    #[error("weekly wage per worker is zero or above the supported maximum {maximum}")]
    InvalidBusinessWage { maximum: Money },
    #[error("business {business_id} already pays the requested weekly wage")]
    UnchangedBusinessWage { business_id: BusinessId },
    #[error("business {business_id} cannot change wages again before day {next_change_day}")]
    BusinessWageCooldown {
        business_id: BusinessId,
        next_change_day: i64,
    },
    #[error("business {business_id} has no workforce to recompense")]
    BusinessHasNoWorkforce { business_id: BusinessId },
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
    #[error("family unity is {available}, but command requires {required}")]
    InsufficientFamilyUnity { available: u16, required: u16 },
    #[error("the panicked market holds only {available}, so there is nothing left to extract")]
    MarketExtractionUnavailable { available: Money },
    #[error("business {business_id} has {available}, but command requires {required}")]
    InsufficientBusinessFunds {
        business_id: BusinessId,
        available: Money,
        required: Money,
    },
    #[error("public-work budget must be at least {minimum}")]
    InvalidPublicWorkBudget { minimum: Money },
    #[error(transparent)]
    PublicWorkFunding(#[from] PublicWorkFundingError),
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
    #[error("there is no grounded {kind:?} claim against dynasty {defendant_dynasty_id}")]
    LegalClaimNotGrounded {
        defendant_dynasty_id: DynastyId,
        kind: LegalCaseKind,
    },
    #[error(
        "legal evidence {evidence_basis_points} exceeds the supported claim evidence {maximum_basis_points}"
    )]
    LegalEvidenceExceedsClaim {
        evidence_basis_points: u16,
        maximum_basis_points: u16,
    },
    #[error("legal damages {damages} exceed the supported claim amount {maximum_damages}")]
    LegalDamagesExceedClaim {
        damages: Money,
        maximum_damages: Money,
    },
    #[error("an unresolved {kind:?} case against dynasty {defendant_dynasty_id} already exists")]
    DuplicateActiveLegalCase {
        defendant_dynasty_id: DynastyId,
        kind: LegalCaseKind,
    },
    #[error("the player dynasty cannot file another legal case before day {next_filing_day}")]
    LegalCaseCooldown { next_filing_day: i64 },
    #[error("legal case {case_id} does not exist")]
    MissingLegalCase { case_id: LegalCaseId },
    #[error("legal case {case_id} is not an unresolved claim against the player dynasty")]
    LegalSettlementUnavailable { case_id: LegalCaseId },
    #[error("character {character_id} does not exist")]
    MissingCharacter { character_id: CharacterId },
    #[error(
        "the grounded obligation behind legal case {case_id} no longer exists, so there is nothing to settle"
    )]
    LegalSettlementNothingToSettle { case_id: LegalCaseId },
    #[error(
        "legal settlement would overflow plaintiff dynasty {plaintiff_dynasty_id} treasury {current} by {incoming}"
    )]
    LegalSettlementTreasuryOverflow {
        plaintiff_dynasty_id: DynastyId,
        current: Money,
        incoming: Money,
    },
    #[error("family council for dynasty {dynasty_id} does not exist")]
    MissingFamilyCouncil { dynasty_id: DynastyId },
    #[error("house governance is already {governance:?}")]
    UnchangedHouseGovernance { governance: HouseGovernance },
    #[error("house governance cannot change again before day {next_change_day}")]
    HouseGovernanceCooldown { next_change_day: i64 },
    #[error("the family council cannot be convened again before day {next_meeting_day}")]
    FamilyCouncilMeetingCooldown { next_meeting_day: i64 },
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
    #[error(
        "institutional endowment must be between {minimum} and {maximum}; requested {requested}"
    )]
    InstitutionEndowmentOutOfRange {
        minimum: Money,
        maximum: Money,
        requested: Money,
    },
    #[error("the dynasty has no established membership in institution {institution_id}")]
    InstitutionEndowmentRequiresMembership { institution_id: InstitutionId },
    #[error(
        "the dynasty cannot make another institutional endowment before day {next_endowment_day}"
    )]
    InstitutionEndowmentCooldown { next_endowment_day: i64 },
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
    #[error("notifications through {message_id} are already acknowledged")]
    NotificationAlreadyAcknowledged { message_id: OutboxMessageId },
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum PublicWorkFundingError {
    #[error("public-work funding must be positive")]
    InvalidAmount,
    #[error("public work {public_work_id} does not exist")]
    Missing { public_work_id: PublicWorkId },
    #[error("public work {public_work_id} is already complete")]
    AlreadyComplete { public_work_id: PublicWorkId },
    #[error(
        "public work {public_work_id} has only {remaining} unfunded; requested contribution {requested}"
    )]
    ExceedsRemaining {
        public_work_id: PublicWorkId,
        remaining: Money,
        requested: Money,
    },
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
    match dispatch_player_command(registry, &mut candidate, command) {
        Ok(outcome) => {
            super::expire_time_limited_state(&mut candidate);
            super::refresh_campaign_phases(&mut candidate);
            super::validate_invariants(registry, &candidate);
            *state = candidate;
            Ok(outcome)
        }
        Err(error) => Err(error),
    }
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

fn apply_contract(
    registry: &Registry,
    state: &mut AppState,
    terms: &SupplyContractTerms,
) -> Result<CommandOutcome, CommandError> {
    ensure_player_contract_party(state, terms)?;
    let validated = validate_supply_contract(registry, state, terms.clone())?;
    ensure_non_player_contract_counterparty_accepts(registry, state, terms)?;
    let id = validated.commit(registry, state)?;
    Ok(CommandOutcome {
        summary: format!("Created supply contract {id}."),
    })
}

fn apply_loan(
    registry: &Registry,
    state: &mut AppState,
    terms: &LoanTerms,
) -> Result<CommandOutcome, CommandError> {
    ensure_player_loan_party(state, terms)?;
    let validated = validate_loan(state, terms.clone())?;
    ensure_non_player_loan_counterparty_accepts(state, terms)?;
    let restructured = validated.restructures_defaulted_loan();
    let id = validated.commit(state)?;
    deploy_non_player_financing_package(registry, state, terms)?;
    let summary = if restructured {
        format!("Restructured loan {id}.")
    } else {
        format!("Issued loan {id}.")
    };
    Ok(CommandOutcome { summary })
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
    let buyer_treasury = state
        .dynasties
        .get(&buyer_dynasty_id)
        .ok_or(CommandError::MissingDynasty {
            dynasty_id: buyer_dynasty_id,
        })?
        .treasury();
    // Resolve the complete sale result before committing anything: the quote is
    // pure, so the counterparty reserve is enforced pre-commit and a rejected
    // sale never mutates state.
    let quote = quote_property_liquidation(
        registry,
        state,
        state.player_dynasty_id,
        buyer_dynasty_id,
        property_id,
    )?;
    let buyer_after = buyer_treasury
        .checked_sub(quote.buyer_contribution)
        .expect("quoted property buyer contribution must fit treasury");
    // The counterparty reserve protects against selling to a buyer who
    // cannot genuinely afford the asset. In a civic-guaranteed auction the
    // buyer has committed their entire treasury by construction and the
    // civic treasury funds the shortfall, so the discretionary reserve does
    // not apply there.
    if quote.civic_guarantee == Money::ZERO && buyer_after < PROPERTY_COUNTERPARTY_BUYER_RESERVE {
        return Err(CommandError::PropertyCounterpartyBuyerReserve {
            buyer_dynasty_id,
            available: buyer_treasury,
            buyer_contribution: quote.buyer_contribution,
            required_reserve: PROPERTY_COUNTERPARTY_BUYER_RESERVE,
        });
    }
    sell_owned_property(
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
    let character = state
        .characters
        .get(character_id)
        .ok_or(CommandError::MissingCharacter { character_id })?;
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
    let replacement_selection_day = resigned_office
        .then(|| checked_future_day(day, 30))
        .transpose()?;
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    institution.members.remove(&character_id);
    if resigned_office {
        institution.office_holder_id = None;
        // The departed holder's policy dies with the resignation: a directive
        // nobody holds office for must not keep shaping the district.
        institution.active_directive = None;
        if let Some(replacement_selection_day) = replacement_selection_day {
            institution.next_selection_day = institution
                .next_selection_day
                .min(replacement_selection_day);
        }
    }
    state.audit_log.push(AuditRecord {
        day,
        kind: AuditKind::InstitutionWithdrawal,
        subject: institution_support_subject(institution_id, character_id).into(),
        detail: if resigned_office {
            super::OFFICE_RESIGNATION_AUDIT_DETAIL.to_owned()
        } else {
            "resigned_office=false".to_owned()
        },
    });
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("Character {character_id} withdrew from institution {institution_id}"),
        if resigned_office {
            "The dynasty surrendered the office and its institutional membership; a replacement selection will be scheduled.".to_owned()
        } else {
            "The dynasty surrendered this institutional membership.".to_owned()
        },
    )?;
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
    ensure_operable_owned_business(state, from_business_id)?;
    // Receiving a cash infusion is allowed for any owned business still
    // operating, including one under distress the transfer is meant to
    // relieve; rescuing an insolvent or closed firm is InvestInBusiness's
    // role, so `transfer_business_cash` rejects a terminated destination and
    // only the source must be operable here.
    ensure_owned_business(state, to_business_id)?;
    // Portfolio transfers honor the same operating-reserve floor as every
    // other player-driven route out of the business, so a firm cannot be
    // hollowed out below the reserve its daily decisions rely on.
    let spendable = state
        .businesses
        .get(from_business_id)
        .map(business_operating_spendable_cash)
        .expect("owned business must exist");
    if spendable < amount {
        return Err(CommandError::InsufficientBusinessFunds {
            business_id: from_business_id,
            available: spendable,
            required: amount,
        });
    }
    transfer_business_cash(state, from_business_id, to_business_id, amount)?;
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Finance,
        format!("Portfolio cash moved to business {to_business_id}"),
        format!(
            "The dynasty transferred {amount} from business {from_business_id} to business {to_business_id}."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!(
            "Transferred {amount} from business {from_business_id} to business {to_business_id}."
        ),
    })
}

fn apply_business_cash_withdrawal(
    registry: &Registry,
    state: &mut AppState,
    business_id: BusinessId,
    amount: Money,
) -> Result<CommandOutcome, CommandError> {
    ensure_owned_business(state, business_id)?;
    distribute_owned_business_cash(
        registry,
        state,
        state.player_dynasty_id,
        business_id,
        amount,
    )?;
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Finance,
        format!("Business {business_id} distributed cash to the dynasty"),
        format!(
            "The dynasty withdrew {amount} of surplus cash from business {business_id} while preserving its operating reserve."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!("Withdrew {amount} from business {business_id}."),
    })
}

fn apply_business_investment(
    state: &mut AppState,
    business_id: BusinessId,
    amount: Money,
) -> Result<CommandOutcome, CommandError> {
    ensure_owned_business(state, business_id)?;
    // The canonical capitalization path owns every rule (positive amount,
    // lifecycle, treasury, overflow); the command translates its typed
    // rejections instead of duplicating them under a second taxonomy.
    let rehabilitation = super::strategic::capitalize_owned_business(
        state,
        state.player_dynasty_id,
        business_id,
        amount,
    )
    .map_err(CommandError::Strategic)?;
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Finance,
        format!("Business {business_id} capitalized"),
        format!(
            "The dynasty invested {amount} into the enterprise, restoring {rehabilitation} basis points of operating condition."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!(
            "Invested {amount} in business {business_id} and restored {rehabilitation} basis points of condition."
        ),
    })
}

/// A policy reserve may hold at most one full year of the firm's operating
/// cover idle: defensive enough for any downturn, bounded against locking a
/// firm's economy behind an unreachable cash floor.
fn recipe_daily_operating_cover_ceiling(
    registry: &Registry,
    business: &crate::core::Business,
) -> Money {
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe reference must be valid");
    recipe
        .daily_operating_cost()
        .saturating_mul(BUSINESS_RESERVE_MAX_OPERATING_DAYS)
}

fn apply_business_policy(
    registry: &Registry,
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
    ensure_operable_owned_business(state, business_id)?;
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
    // The reserve gates every spendable-copper route for the business, so an
    // unbounded floor would let one policy change permanently lock the firm's
    // payroll, purchasing, and rescue financing. A full year of operating
    // cover held idle is already an extreme defensive posture; beyond that
    // the request serves no operating purpose. The bound is deliberately
    // independent of the firm's current policy, so it never rejects a raise
    // merely because today's reserve happens to be small.
    if minimum_cash_reserve > recipe_daily_operating_cover_ceiling(registry, business) {
        return Err(CommandError::InvalidBusinessPolicy);
    }
    if business.policy.target_input_days == target_input_days
        && business.policy.target_output_days == target_output_days
        && business.policy.minimum_cash_reserve == minimum_cash_reserve
        && business.policy.maintenance_basis_points == maintenance_basis_points
        && business.policy.quality_target_basis_points == quality_target_basis_points
    {
        return Err(CommandError::UnchangedBusinessPolicy { business_id });
    }
    let subject = format!("business:{business_id}");
    if let Some(last_change_day) =
        latest_audit_day_for_subject(state, AuditKind::BusinessPolicyChange, &subject)
    {
        let next_change_day =
            checked_future_day(last_change_day, BUSINESS_POLICY_CHANGE_INTERVAL_DAYS)?;
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
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Finance,
        format!("Business {business_id} operating policy updated"),
        format!(
            "The enterprise now targets {target_input_days} input days, {target_output_days} output days, a {minimum_cash_reserve} cash reserve, {maintenance_basis_points} maintenance basis points, and {quality_target_basis_points} quality basis points."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!("Updated operating policy for business {business_id}."),
    })
}

/// Sets the standing weekly wage for every workforce agreement of one owned
/// business. Wage posture is the player's proactive labor lever: wages below
/// the market reference erode workforce loyalty and eventually provoke
/// disputes, while generous wages build loyalty that absorbs operating strain.
fn apply_business_wages(
    state: &mut AppState,
    business_id: BusinessId,
    weekly_wage_per_worker: Money,
) -> Result<CommandOutcome, CommandError> {
    ensure_operable_owned_business(state, business_id)?;
    if weekly_wage_per_worker <= Money::ZERO || weekly_wage_per_worker > MAX_WEEKLY_WAGE_PER_WORKER
    {
        return Err(CommandError::InvalidBusinessWage {
            maximum: MAX_WEEKLY_WAGE_PER_WORKER,
        });
    }
    let subject = format!("business:{business_id}");
    if let Some(last_change_day) =
        latest_audit_day_for_subject(state, AuditKind::BusinessWageChange, &subject)
    {
        let next_change_day =
            checked_future_day(last_change_day, BUSINESS_WAGE_CHANGE_INTERVAL_DAYS)?;
        if state.clock.day() < next_change_day {
            return Err(CommandError::BusinessWageCooldown {
                business_id,
                next_change_day,
            });
        }
    }
    let agreements: Vec<EmploymentId> = state
        .employment
        .values()
        .filter(|agreement| {
            agreement.business_id() == business_id && agreement.status != EmploymentStatus::Ended
        })
        .map(EmploymentAgreement::id)
        .collect();
    if agreements.is_empty() {
        return Err(CommandError::BusinessHasNoWorkforce { business_id });
    }
    // Every agreement must already pay the requested per-worker wage for this
    // to be a genuine no-op: individual agreements drift apart when the
    // market wage system renegotiates them, so keying off one arbitrary
    // agreement would reject changes the workforce would still feel. The
    // comparison multiplies the requested per-worker wage back up to the
    // agreement total, so a renegotiated non-divisible payroll never reads as
    // an exact match and gets silently re-rounded by a "no-op" rewrite.
    let all_match_requested = agreements.iter().all(|agreement_id| {
        state.employment.get(agreement_id).is_some_and(|agreement| {
            weekly_wage_per_worker
                .checked_mul_ratio(i64::from(agreement.workers().max(1)), 1)
                .is_some_and(|total| total == agreement.weekly_wage())
        })
    });
    if all_match_requested {
        return Err(CommandError::UnchangedBusinessWage { business_id });
    }
    for agreement_id in &agreements {
        let agreement = state
            .employment
            .get_mut(agreement_id)
            .expect("collected employment agreement must exist");
        // The wage is already validated against MAX_WEEKLY_WAGE_PER_WORKER and
        // workers fit u16, so the product cannot overflow the fixed-point range.
        let total = weekly_wage_per_worker
            .checked_mul_ratio(i64::from(agreement.workers().max(1)), 1)
            .expect("validated wage times u16 workers cannot overflow");
        agreement.weekly_wage = total;
    }
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::BusinessWageChange,
        subject: subject.into(),
        detail: format!(
            "wage_per_worker={}; agreements={}",
            weekly_wage_per_worker.copper(),
            agreements.len()
        ),
    });
    super::strategic::try_push_outbox(
        state,
        OutboxKind::District,
        format!("Business {business_id} wage terms updated"),
        format!(
            "The enterprise now pays {weekly_wage_per_worker} per worker each week across {} workforce agreement(s).",
            agreements.len()
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!(
            "Set the weekly wage of business {business_id} to {weekly_wage_per_worker} per worker."
        ),
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

fn ensure_non_player_contract_counterparty_accepts(
    registry: &Registry,
    state: &AppState,
    terms: &SupplyContractTerms,
) -> Result<(), CommandError> {
    let buyer = state
        .businesses
        .get(terms.buyer_business_id)
        .expect("validated contract buyer must exist");
    let seller = state
        .businesses
        .get(terms.seller_business_id)
        .expect("validated contract seller must exist");
    let market_price = state
        .market
        .get_quote(terms.good_id)
        .ok_or(CommandError::MissingMarketQuote {
            good_id: terms.good_id,
        })?
        .price();
    let price_bounds = contract_counterparty_price_bounds(
        state,
        terms.buyer_business_id,
        terms.seller_business_id,
        market_price,
    );
    let minimum_price = price_bounds.minimum_seller_price;
    let maximum_price = price_bounds.maximum_buyer_price;
    if seller.owner_dynasty_id() != state.player_dynasty_id && terms.unit_price < minimum_price {
        return Err(CommandError::ContractCounterpartyPriceTooLow {
            unit_price: terms.unit_price,
            minimum_price,
        });
    }
    if buyer.owner_dynasty_id() != state.player_dynasty_id && terms.unit_price > maximum_price {
        return Err(CommandError::ContractCounterpartyPriceTooHigh {
            unit_price: terms.unit_price,
            maximum_price,
        });
    }

    let weekly_payment = crate::money::checked_cost_for(terms.quantity_per_week, terms.unit_price)
        .expect("validated contract payment must fit the supported money range");
    let minimum_penalty = weekly_payment.ceil_div_positive(4);
    let maximum_penalty = weekly_payment.saturating_mul(4);
    if terms.penalty < minimum_penalty || terms.penalty > maximum_penalty {
        return Err(CommandError::ContractCounterpartyPenaltyOutOfRange {
            penalty: terms.penalty,
            minimum_penalty,
            maximum_penalty,
        });
    }

    let capacity = available_supply_contract_capacity(
        registry,
        state,
        terms.buyer_business_id,
        terms.seller_business_id,
        terms.good_id,
    )
    .expect("validated contract parties must have compatible capacity");
    if seller.owner_dynasty_id() != state.player_dynasty_id
        && terms.quantity_per_week > capacity.seller
    {
        return Err(CommandError::ContractCounterpartyCapacity {
            business_id: terms.seller_business_id,
            requested: terms.quantity_per_week,
            available: capacity.seller,
        });
    }
    if buyer.owner_dynasty_id() != state.player_dynasty_id
        && terms.quantity_per_week > capacity.buyer
    {
        return Err(CommandError::ContractCounterpartyCapacity {
            business_id: terms.buyer_business_id,
            requested: terms.quantity_per_week,
            available: capacity.buyer,
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContractCounterpartyPriceBounds {
    pub(crate) minimum_seller_price: Money,
    pub(crate) maximum_buyer_price: Money,
    pub(crate) relationship_pressure_basis_points: u16,
}

/// Returns the price band an NPC counterparty will accept against the player.
///
/// Neutral houses tolerate a modest bargaining band around the market quote. Distrust and
/// resentment narrow that band and can eventually require a premium from an NPC seller or a
/// discount for an NPC buyer. The relationship surcharge is capped so hostility cannot move a
/// negotiated price more than 15% away from market solely through this rule.
pub(crate) fn contract_counterparty_price_bounds(
    state: &AppState,
    buyer_business_id: BusinessId,
    seller_business_id: BusinessId,
    market_price: Money,
) -> ContractCounterpartyPriceBounds {
    let player_id = state.player_dynasty_id;
    let buyer_owner = state
        .businesses
        .get(buyer_business_id)
        .map(crate::core::Business::owner_dynasty_id);
    let seller_owner = state
        .businesses
        .get(seller_business_id)
        .map(crate::core::Business::owner_dynasty_id);
    let counterparty_id = match (buyer_owner, seller_owner) {
        (Some(buyer_owner), Some(seller_owner)) if buyer_owner == player_id => {
            (seller_owner != player_id).then_some(seller_owner)
        }
        (Some(buyer_owner), Some(seller_owner)) if seller_owner == player_id => {
            (buyer_owner != player_id).then_some(buyer_owner)
        }
        (Some(_) | None, Some(_) | None) => None,
    };
    let relationship_pressure_basis_points = counterparty_id.map_or(0, |counterparty_id| {
        contract_relationship_pressure_basis_points(state, counterparty_id)
    });
    let seller_factor = 9_000_i64.saturating_add(i64::from(relationship_pressure_basis_points));
    let buyer_factor = 11_000_i64.saturating_sub(i64::from(relationship_pressure_basis_points));
    ContractCounterpartyPriceBounds {
        minimum_seller_price: market_price.saturating_mul_ratio(seller_factor, 10_000),
        maximum_buyer_price: market_price.saturating_mul_ratio(buyer_factor, 10_000),
        relationship_pressure_basis_points,
    }
}

pub(crate) fn contract_relationship_pressure_basis_points(
    state: &AppState,
    counterparty_id: DynastyId,
) -> u16 {
    let Some(relationship) = state
        .relationships
        .get(&DynastyPair::new(state.player_dynasty_id, counterparty_id))
    else {
        return 0;
    };
    let distrust = 4_000_u16.saturating_sub(relationship.trust_basis_points);
    let resentment = relationship.resentment_basis_points.saturating_sub(3_500);
    distrust
        .saturating_add(resentment)
        .saturating_div(2)
        .min(2_500)
}

fn ensure_player_loan_party(state: &AppState, terms: &LoanTerms) -> Result<(), CommandError> {
    if terms.lender_dynasty_id != state.player_dynasty_id
        && terms.borrower_dynasty_id != state.player_dynasty_id
    {
        return Err(CommandError::PlayerNotParty);
    }
    Ok(())
}

pub(crate) fn private_loan_borrower_financing_pressure(
    state: &AppState,
    dynasty_id: DynastyId,
) -> u8 {
    if state.loans.values().any(|loan| {
        loan.borrower_dynasty_id == dynasty_id
            && matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted)
    }) {
        return 3;
    }
    if state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == dynasty_id
            && matches!(
                business.status(),
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            )
    }) {
        return 2;
    }
    u8::from(state.dynasties.get(&dynasty_id).is_some_and(|dynasty| {
        dynasty.treasury() < PRIVATE_LOAN_COUNTERPARTY_BORROWER_LIQUIDITY_TARGET
    }))
}

fn ensure_non_player_loan_counterparty_accepts(
    state: &AppState,
    terms: &LoanTerms,
) -> Result<(), CommandError> {
    let player_id = state.player_dynasty_id;
    let exposure = negotiated_loan_exposure(state, terms);

    if terms.lender_dynasty_id != player_id {
        let lender = state
            .dynasties
            .get(&terms.lender_dynasty_id)
            .expect("validated loan lender must exist");
        let lender_after = lender
            .treasury()
            .checked_sub(terms.principal)
            .expect("validated loan lender must cover principal");
        if lender_after < PRIVATE_LOAN_COUNTERPARTY_RESERVE {
            return Err(CommandError::LoanCounterpartyLenderReserve {
                lender_dynasty_id: terms.lender_dynasty_id,
                available: lender.treasury(),
                principal: terms.principal,
                required_reserve: PRIVATE_LOAN_COUNTERPARTY_RESERVE,
            });
        }
        if terms.interest_basis_points < PRIVATE_LOAN_COUNTERPARTY_MIN_INTEREST_BASIS_POINTS {
            return Err(CommandError::LoanCounterpartyInterestTooLow {
                interest_basis_points: terms.interest_basis_points,
                minimum_basis_points: PRIVATE_LOAN_COUNTERPARTY_MIN_INTEREST_BASIS_POINTS,
            });
        }
        let minimum_payment =
            exposure.ceil_div_positive(PRIVATE_LOAN_COUNTERPARTY_MAX_AMORTIZATION_WEEKS);
        if terms.weekly_payment < minimum_payment {
            return Err(CommandError::LoanCounterpartyPaymentTooLow {
                weekly_payment: terms.weekly_payment,
                minimum_payment,
            });
        }
    }

    if terms.borrower_dynasty_id != player_id {
        if terms.interest_basis_points > PRIVATE_LOAN_COUNTERPARTY_MAX_INTEREST_BASIS_POINTS {
            return Err(CommandError::LoanCounterpartyInterestTooHigh {
                interest_basis_points: terms.interest_basis_points,
                maximum_basis_points: PRIVATE_LOAN_COUNTERPARTY_MAX_INTEREST_BASIS_POINTS,
            });
        }
        let minimum_amortization_weeks =
            if private_loan_borrower_financing_pressure(state, terms.borrower_dynasty_id) >= 2 {
                PRIVATE_LOAN_DISTRESSED_BORROWER_MIN_AMORTIZATION_WEEKS
            } else {
                PRIVATE_LOAN_COUNTERPARTY_MIN_AMORTIZATION_WEEKS
            };
        let maximum_payment = exposure.ceil_div_positive(minimum_amortization_weeks);
        if terms.weekly_payment > maximum_payment {
            return Err(CommandError::LoanCounterpartyPaymentTooHigh {
                weekly_payment: terms.weekly_payment,
                maximum_payment,
            });
        }
        if let Some(property_id) = terms.collateral_property_id {
            let property = state
                .properties
                .get(&property_id)
                .expect("validated loan collateral must exist");
            let minimum_exposure = ceil_basis_point_share(
                property.value,
                PRIVATE_LOAN_COUNTERPARTY_MIN_COLLATERAL_LTV_BASIS_POINTS,
            );
            if exposure < minimum_exposure {
                return Err(CommandError::LoanCounterpartyCollateralTooLarge {
                    property_id,
                    property_value: property.value,
                    exposure,
                    minimum_exposure,
                });
            }
        }
        if private_loan_borrower_financing_pressure(state, terms.borrower_dynasty_id) == 0 {
            return Err(CommandError::LoanCounterpartyNoFinancingNeed {
                borrower_dynasty_id: terms.borrower_dynasty_id,
            });
        }
    }
    Ok(())
}

fn deploy_non_player_financing_package(
    registry: &Registry,
    state: &mut AppState,
    terms: &LoanTerms,
) -> Result<(), CommandError> {
    if terms.borrower_dynasty_id == state.player_dynasty_id {
        return Ok(());
    }
    // One pass resolves each candidate's recapitalization shortfall; the
    // neediest operable firm wins by lifecycle severity, then cash, then ID.
    let selected = state
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id() == terms.borrower_dynasty_id)
        // Closed premises cannot be recapitalized; deploying funds there would
        // fail after the loan itself has committed.
        .filter(|business| business.status() != BusinessStatus::Closed)
        .filter_map(|business| {
            let target_cash = business_recapitalization_target(registry, state, business);
            let shortfall = target_cash.saturating_sub(business.cash());
            if shortfall <= Money::ZERO {
                return None;
            }
            // Closed premises are filtered out above; that arm exists for
            // exhaustiveness only.
            let severity = match business.status() {
                BusinessStatus::Insolvent => 0_u8,
                BusinessStatus::Distressed => 1,
                BusinessStatus::Active | BusinessStatus::Closed => 2,
            };
            Some((severity, business.cash(), business.id(), shortfall))
        })
        .min_by_key(|&(severity, cash, id, _)| (severity, cash, id));
    let Some((_, _, business_id, shortfall)) = selected else {
        return Ok(());
    };
    // The new principal is the financing package being deployed. The
    // borrower's existing treasury remains its household reserve; requiring
    // the post-loan treasury to clear that reserve would make small, valid
    // rescue loans inert before they can reach the business.
    let amount = shortfall.min(terms.principal);
    if amount > Money::ZERO {
        capitalize_owned_business(state, terms.borrower_dynasty_id, business_id, amount)?;
    }
    Ok(())
}

fn negotiated_loan_exposure(state: &AppState, terms: &LoanTerms) -> Money {
    let prior_default = state
        .loans
        .values()
        .filter(|loan| {
            loan.lender_dynasty_id == terms.lender_dynasty_id
                && loan.borrower_dynasty_id == terms.borrower_dynasty_id
                && loan.status == LoanStatus::Defaulted
        })
        .max_by_key(|loan| (loan.next_due_day, loan.id))
        .map_or(Money::ZERO, |loan| loan.balance);
    prior_default
        .checked_add(terms.principal)
        .expect("validated loan exposure must fit the supported money range")
}

fn ceil_basis_point_share(value: Money, basis_points: i64) -> Money {
    debug_assert!(value >= Money::ZERO);
    debug_assert!((0..=10_000).contains(&basis_points));
    let numerator = i128::from(value.copper()) * i128::from(basis_points);
    let copper = (numerator + 9_999) / 10_000;
    Money::from_copper(
        i64::try_from(copper).expect("basis-point share of supported money must fit money"),
    )
}

#[derive(Clone, Copy, Debug)]
struct ValidatedCivicDebtIssuance {
    treasury_id: InstitutionId,
    creditor_dynasty_id: DynastyId,
    principal: Money,
    creditor_treasury_after: Money,
    treasury_budget_after: Money,
    weekly_payment: Money,
    next_due_day: i64,
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
    Ok(ValidatedCivicDebtIssuance {
        treasury_id,
        creditor_dynasty_id: creditor.id(),
        principal,
        creditor_treasury_after: creditor
            .treasury()
            .checked_sub(principal)
            .expect("validated civic debt creditor must cover the principal"),
        treasury_budget_after,
        weekly_payment: principal.ceil_div_positive(CIVIC_DEBT_TERM_WEEKS),
        next_due_day: checked_future_day(state.clock.day(), 7)?,
    })
}

fn commit_civic_debt_issuance(
    state: &mut AppState,
    law_id: crate::ids::LawId,
    sponsor_dynasty_id: DynastyId,
    issuance: ValidatedCivicDebtIssuance,
) -> Result<crate::ids::CivicDebtId, CommandError> {
    let id = state.next_ids.try_civic_debt()?;
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
            next_due_day: issuance.next_due_day,
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
    super::strategic::try_record_counterparty_information(
        state,
        sponsor_dynasty_id,
        issuance.creditor_dynasty_id,
        "Municipal debt underwriting and treasury records",
    )?;
    Ok(id)
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
        let next_enactment_day =
            checked_future_day(last_enactment_day, LAW_SPONSORSHIP_INTERVAL_DAYS)?;
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
    let available_day = checked_player_office_power_available_day(state, required_power)?
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
    // Resolve every fallible step before the first mutation.
    let id = state.next_ids.try_law()?;
    spend_player_treasury_to_market(state, LAW_SPONSORSHIP_COST)?;
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
        .map(|issuance| commit_civic_debt_issuance(state, id, state.player_dynasty_id, issuance))
        .transpose()?;
    super::strategic::try_push_outbox(
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
    )?;
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
    if budget < PUBLIC_WORK_MINIMUM_BUDGET {
        return Err(CommandError::InvalidPublicWorkBudget {
            minimum: PUBLIC_WORK_MINIMUM_BUDGET,
        });
    }
    if state.public_works.values().any(|work| {
        work.district_id == district_id && work.kind == kind && work.status.is_unfinished()
    }) {
        return Err(CommandError::DuplicateActivePublicWork { district_id, kind });
    }
    let active_sponsored = state
        .public_works
        .values()
        .filter(|work| {
            work.sponsor_dynasty_id == Some(state.player_dynasty_id) && work.status.is_unfinished()
        })
        .count();
    if active_sponsored >= MAX_ACTIVE_SPONSORED_PUBLIC_WORKS {
        return Err(CommandError::PublicWorkCapacity {
            active: active_sponsored,
            maximum: MAX_ACTIVE_SPONSORED_PUBLIC_WORKS,
        });
    }
    let subject = format!("dynasty:{}", state.player_dynasty_id);
    validate_public_work_cooldown(state, &subject)?;
    if !has_player_office(state) {
        return Err(CommandError::PublicWorkSponsorshipRequiresOffice);
    }
    if !has_player_office_power(state, OfficePower::PublicWorks) {
        return Err(CommandError::PublicWorkSponsorshipRequiresPower);
    }
    let available_day = checked_player_office_power_available_day(state, OfficePower::PublicWorks)?
        .expect("validated public-works office must have an availability day");
    if state.clock.day() < available_day {
        return Err(CommandError::PublicWorkPowerNotEstablished { available_day });
    }
    let contribution = public_work_initial_contribution(budget);
    // Resolve every fallible step before the first mutation so a rejected
    // sponsorship never leaves a debited treasury behind.
    let id = state.next_ids.try_public_work()?;
    spend_player_treasury(state, contribution)?;
    // The sponsor pays construction labor and materials directly, so the
    // contribution keeps a credited counterparty in the market clearing pool
    // instead of vanishing from the economy.
    credit_market_clearing_account(state, contribution)?;
    let progress_basis_points = super::public_work_progress_basis_points(contribution, budget);
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
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("Public work {id} started"),
        format!("Construction began on a {kind:?} project in district {district_id}."),
    )?;
    Ok(CommandOutcome {
        summary: format!("Started public work {id}."),
    })
}

#[derive(Clone, Copy, Debug)]
struct PublicWorkFundingQuote {
    player_id: DynastyId,
    district_id: DistrictId,
    kind: PublicWorkKind,
    external_sponsor_dynasty_id: Option<DynastyId>,
    treasury_after: Money,
    contributions_after: Money,
    spent_after: Money,
    progress_basis_points: u16,
    legitimacy_gain: u16,
    completed: bool,
}

fn apply_public_work_funding(
    state: &mut AppState,
    public_work_id: PublicWorkId,
    amount: Money,
) -> Result<CommandOutcome, CommandError> {
    let quote = quote_public_work_funding(state, public_work_id, amount)?;
    let player = state
        .dynasties
        .get_mut(&quote.player_id)
        .expect("validated player dynasty must exist");
    player.resources.treasury = quote.treasury_after;
    player.resources.civic_contributions = quote.contributions_after;
    if quote.legitimacy_gain > 0 {
        player.resources.legitimacy_basis_points = player
            .resources
            .legitimacy_basis_points
            .saturating_add(quote.legitimacy_gain)
            .min(10_000);
    }
    // Direct funding pays construction costs immediately, so the amount keeps
    // a credited counterparty in the market clearing pool.
    credit_market_clearing_account(state, amount)?;
    let work = state
        .public_works
        .get_mut(&public_work_id)
        .expect("validated public work must exist");
    work.spent = quote.spent_after;
    work.progress_basis_points = quote.progress_basis_points;
    if quote.completed {
        work.status = PublicWorkStatus::Completed;
        super::strategic::apply_public_work_completion(state, quote.district_id, quote.kind);
    }
    // A rival house whose project the dynasty bankrolls remembers the favor:
    // trust and standing rise and the sponsor carries a durable obligation.
    if let Some(sponsor_dynasty_id) = quote.external_sponsor_dynasty_id {
        super::strategic::adjust_dynasty_relationship(
            state,
            quote.player_id,
            sponsor_dynasty_id,
            super::strategic::RelationshipDelta::new(60, 80, 0, 0, 40),
        );
        super::strategic::remember_dynasty_interaction(
            state,
            quote.player_id,
            sponsor_dynasty_id,
            &format!(
                "the player dynasty contributed {amount} to the sponsor's {:?} project in district {}",
                quote.kind, quote.district_id
            ),
        );
    }
    let (title, detail) = if quote.external_sponsor_dynasty_id.is_some() {
        (
            format!("Public work {public_work_id} received dynasty funding"),
            format!(
                "The dynasty contributed {amount} to another house's {:?} project in district {}; its progress is now {} basis points and the city has taken notice.",
                quote.kind, quote.district_id, quote.progress_basis_points
            ),
        )
    } else if quote.completed {
        (
            format!("Public work {public_work_id} completed with dynasty funding"),
            format!(
                "The dynasty contributed {amount} directly to finish the {:?} project in district {}.",
                quote.kind, quote.district_id
            ),
        )
    } else {
        (
            format!("Public work {public_work_id} received dynasty funding"),
            format!(
                "The dynasty contributed {amount} directly to public work {public_work_id}; project progress is now {} basis points.",
                quote.progress_basis_points
            ),
        )
    };
    super::strategic::try_push_outbox(state, OutboxKind::Politics, title, detail)?;
    Ok(CommandOutcome {
        summary: if quote.completed {
            format!("Funded and completed public work {public_work_id} with {amount}.")
        } else {
            format!("Funded public work {public_work_id} with {amount}.")
        },
    })
}

fn quote_public_work_funding(
    state: &AppState,
    public_work_id: PublicWorkId,
    amount: Money,
) -> Result<PublicWorkFundingQuote, CommandError> {
    if amount <= Money::ZERO {
        return Err(PublicWorkFundingError::InvalidAmount.into());
    }
    let (sponsor_dynasty_id, status, budget, spent, district_id, kind) = {
        let work = state
            .public_works
            .get(&public_work_id)
            .ok_or(PublicWorkFundingError::Missing { public_work_id })?;
        (
            work.sponsor_dynasty_id,
            work.status,
            work.budget,
            work.spent,
            work.district_id,
            work.kind,
        )
    };
    if status == PublicWorkStatus::Completed {
        return Err(PublicWorkFundingError::AlreadyComplete { public_work_id }.into());
    }
    let remaining = budget
        .checked_sub(spent)
        .expect("validated public work spending must not exceed its budget");
    if amount > remaining {
        return Err(PublicWorkFundingError::ExceedsRemaining {
            public_work_id,
            remaining,
            requested: amount,
        }
        .into());
    }
    let player_id = state.player_dynasty_id;
    let player = state
        .dynasties
        .get(&player_id)
        .expect("player dynasty must exist");
    if player.treasury() < amount {
        return Err(CommandError::InsufficientPlayerFunds {
            available: player.treasury(),
            required: amount,
        });
    }
    let treasury_after = player
        .treasury()
        .checked_sub(amount)
        .expect("validated public-work funding must fit player treasury");
    let contributions_after = player.civic_contributions().checked_add(amount).ok_or(
        super::SimulationError::DynastyCivicContributionsOverflow {
            dynasty_id: player_id,
            current: player.civic_contributions(),
            incoming: amount,
        },
    )?;
    let spent_after = spent
        .checked_add(amount)
        .expect("bounded public-work funding must fit project budget");
    let progress_basis_points = super::public_work_progress_basis_points(spent_after, budget);
    // Contributing to a project the dynasty did not sponsor is public spirit
    // the city can see: it earns a bounded legitimacy gain in the same
    // proportion an institution endowment pays, scaled down because the
    // district benefit itself is part of the return.
    let external_sponsor_dynasty_id = sponsor_dynasty_id.filter(|sponsor| *sponsor != player_id);
    let legitimacy_gain = if external_sponsor_dynasty_id.is_some() {
        u16::try_from((amount.copper() / 400).clamp(10, 120))
            .expect("bounded civic-funding legitimacy gain must fit u16")
    } else {
        0
    };
    Ok(PublicWorkFundingQuote {
        player_id,
        district_id,
        kind,
        external_sponsor_dynasty_id,
        treasury_after,
        contributions_after,
        spent_after,
        progress_basis_points,
        legitimacy_gain,
        completed: spent_after == budget,
    })
}

fn validate_public_work_cooldown(state: &AppState, subject: &str) -> Result<(), CommandError> {
    if let Some(last_start_day) =
        latest_audit_day_for_subject(state, AuditKind::PublicWorkStarted, subject)
    {
        let next_start_day =
            checked_future_day(last_start_day, PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS)?;
        if state.clock.day() < next_start_day {
            return Err(CommandError::PublicWorkCooldown { next_start_day });
        }
    }
    Ok(())
}

/// Whether the named office power is currently held by an active player-dynasty
/// character. This is the single canonical gate for exercising office powers:
/// incapacitation vacates offices, but this predicate does not depend on that
/// cross-system invariant holding.
fn office_power_is_player_held(
    state: &AppState,
    institution: &InstitutionRuntime,
    power: OfficePower,
) -> bool {
    institution.powers.contains(&power)
        && institution.office_holder_id.is_some_and(|character_id| {
            state.characters.get(character_id).is_some_and(|character| {
                character.status() == CharacterStatus::Active
                    && character.dynasty_id() == state.player_dynasty_id
            })
        })
}

pub(crate) fn has_player_office(state: &AppState) -> bool {
    state.institutions.values().any(|institution| {
        institution.office_holder_id.is_some_and(|character_id| {
            state.characters.get(character_id).is_some_and(|character| {
                character.status() == CharacterStatus::Active
                    && character.dynasty_id() == state.player_dynasty_id
            })
        })
    })
}

pub(crate) fn has_player_office_power(state: &AppState, power: OfficePower) -> bool {
    state
        .institutions
        .values()
        .any(|institution| office_power_is_player_held(state, institution, power))
}

pub(crate) fn player_office_power_available_day(
    state: &AppState,
    power: OfficePower,
) -> Option<i64> {
    checked_player_office_power_available_day(state, power).unwrap_or(Some(i64::MAX))
}

fn checked_player_office_power_available_day(
    state: &AppState,
    power: OfficePower,
) -> Result<Option<i64>, TimelineError> {
    let mut earliest = None;
    let mut range_error = None;
    for institution in state
        .institutions
        .values()
        .filter(|institution| office_power_is_player_held(state, institution, power))
    {
        match checked_future_day(
            institution.term_started_day,
            OFFICE_POWER_ESTABLISHMENT_DAYS,
        ) {
            Ok(available_day) => {
                earliest = Some(earliest.map_or(available_day, |day: i64| day.min(available_day)));
            }
            Err(error) if range_error.is_none() => range_error = Some(error),
            Err(_) => {}
        }
    }
    match earliest {
        Some(day) => Ok(Some(day)),
        None => range_error.map_or(Ok(None), Err),
    }
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

pub(crate) fn quote_player_legal_claim(
    state: &AppState,
    defendant_dynasty_id: DynastyId,
    kind: LegalCaseKind,
) -> Result<super::LegalClaimQuote, CommandError> {
    if defendant_dynasty_id == state.player_dynasty_id {
        return Err(CommandError::SameLegalParty);
    }
    if !state.dynasties.contains_key(&defendant_dynasty_id) {
        return Err(CommandError::MissingDynasty {
            dynasty_id: defendant_dynasty_id,
        });
    }
    super::quote_grounded_legal_claim(state, state.player_dynasty_id, defendant_dynasty_id, kind)
        .ok_or(CommandError::LegalClaimNotGrounded {
            defendant_dynasty_id,
            kind,
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LegalSettlementQuote {
    pub(crate) case_id: LegalCaseId,
    pub(crate) plaintiff_dynasty_id: DynastyId,
    pub(crate) kind: LegalCaseKind,
    pub(crate) amount: Money,
}

pub(crate) fn quote_player_legal_settlement(
    state: &AppState,
    case_id: LegalCaseId,
) -> Result<LegalSettlementQuote, CommandError> {
    let legal_case = state
        .legal_cases
        .get(&case_id)
        .ok_or(CommandError::MissingLegalCase { case_id })?;
    if legal_case.defendant_dynasty_id != state.player_dynasty_id
        || !matches!(
            legal_case.status,
            LegalCaseStatus::Filed | LegalCaseStatus::Hearing
        )
        || legal_case.claim_source.is_none()
    {
        return Err(CommandError::LegalSettlementUnavailable { case_id });
    }
    let exposure = super::strategic::recoverable_legal_damages(
        state,
        legal_case.claim_source,
        legal_case.damages,
    );
    if exposure <= Money::ZERO {
        return Err(CommandError::LegalSettlementNothingToSettle { case_id });
    }
    let settlement_basis_points = 5_000_i64
        .saturating_add(i64::from(legal_case.evidence_basis_points) / 2)
        .clamp(5_000, 10_000);
    let amount = exposure.saturating_mul_ratio_ceil_nonnegative(settlement_basis_points, 10_000);
    Ok(LegalSettlementQuote {
        case_id,
        plaintiff_dynasty_id: legal_case.plaintiff_dynasty_id,
        kind: legal_case.kind,
        amount,
    })
}

fn apply_legal_case(
    registry: &Registry,
    state: &mut AppState,
    defendant_dynasty_id: DynastyId,
    kind: LegalCaseKind,
    evidence_basis_points: u16,
    damages: Money,
) -> Result<CommandOutcome, CommandError> {
    if evidence_basis_points > 10_000 || damages <= Money::ZERO {
        // Zero damages would file an unresolvable case: nothing to settle, no
        // judgment award, and a claim source occupied for its whole life.
        return Err(CommandError::InvalidLegalTerms);
    }
    // Party validity and existence are the quote's own preconditions, so they
    // are checked once inside `quote_player_legal_claim`.
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
        let next_filing_day = checked_future_day(last_filing_day, LEGAL_CASE_FILING_INTERVAL_DAYS)?;
        if state.clock.day() < next_filing_day {
            return Err(CommandError::LegalCaseCooldown { next_filing_day });
        }
    }
    let claim = quote_player_legal_claim(state, defendant_dynasty_id, kind)?;
    if evidence_basis_points > claim.evidence_basis_points {
        return Err(CommandError::LegalEvidenceExceedsClaim {
            evidence_basis_points,
            maximum_basis_points: claim.evidence_basis_points,
        });
    }
    if damages > claim.maximum_damages {
        return Err(CommandError::LegalDamagesExceedClaim {
            damages,
            maximum_damages: claim.maximum_damages,
        });
    }
    let hearing_day = checked_future_day(state.clock.day(), LEGAL_CASE_HEARING_DELAY_DAYS)?;
    // Resolve every fallible step before the first mutation so a rejected
    // filing never leaves a debited plaintiff or a credited court behind.
    let id = state.next_ids.try_legal_case()?;
    super::court_filing_fee_headroom(registry, state)?;
    spend_player_treasury(state, LEGAL_CASE_FILING_COST)?;
    super::collect_court_filing_fee(registry, state);
    state.legal_cases.insert(
        id,
        LegalCase {
            id,
            plaintiff_dynasty_id: state.player_dynasty_id,
            defendant_dynasty_id,
            kind,
            claim_source: Some(claim.claim_source),
            evidence_basis_points,
            public_attention_basis_points: 1_500,
            filed_day: state.clock.day(),
            hearing_day,
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
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Legal,
        format!("Legal case {id} filed"),
        format!(
            "A {kind:?} case was filed against dynasty {defendant_dynasty_id}: {}.",
            claim.description
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!("Filed legal case {id}."),
    })
}

fn apply_legal_settlement(
    state: &mut AppState,
    case_id: LegalCaseId,
) -> Result<CommandOutcome, CommandError> {
    let quote = quote_player_legal_settlement(state, case_id)?;
    let player_id = state.player_dynasty_id;
    // Resolve both sides' resulting balances before committing anything so a
    // rejected settlement never leaves a debited payer behind.
    let plaintiff_treasury = state
        .dynasties
        .get(&quote.plaintiff_dynasty_id)
        .expect("legal plaintiff dynasty must exist")
        .treasury();
    let plaintiff_after = plaintiff_treasury.checked_add(quote.amount).ok_or(
        CommandError::LegalSettlementTreasuryOverflow {
            plaintiff_dynasty_id: quote.plaintiff_dynasty_id,
            current: plaintiff_treasury,
            incoming: quote.amount,
        },
    )?;
    let player_treasury = state
        .dynasties
        .get(&player_id)
        .expect("player dynasty must exist")
        .treasury();
    if player_treasury < quote.amount {
        return Err(CommandError::InsufficientPlayerFunds {
            available: player_treasury,
            required: quote.amount,
        });
    }
    spend_player_treasury(state, quote.amount)?;
    let claim_source = state
        .legal_cases
        .get(&case_id)
        .expect("quoted legal case must exist")
        .claim_source;

    state
        .dynasties
        .get_mut(&quote.plaintiff_dynasty_id)
        .expect("legal plaintiff dynasty must exist")
        .resources
        .treasury = plaintiff_after;
    // A negotiated settlement closes the grounded obligation in full only
    // when the payment actually covers it; filing small damages and settling
    // them cannot erase a larger underlying debt.
    let settles_in_full =
        quote.amount >= super::strategic::outstanding_legal_claim_obligation(state, claim_source);
    super::strategic::settle_legal_claim_source(
        state,
        claim_source,
        quote.plaintiff_dynasty_id,
        player_id,
        quote.amount,
        settles_in_full,
    );
    state
        .legal_cases
        .get_mut(&case_id)
        .expect("quoted legal case must exist")
        .status = LegalCaseStatus::Settled;
    super::strategic::adjust_dynasty_relationship(
        state,
        quote.plaintiff_dynasty_id,
        player_id,
        super::strategic::RelationshipDelta::new(80, 40, -20, -120, 0),
    );
    super::strategic::remember_dynasty_interaction(
        state,
        quote.plaintiff_dynasty_id,
        player_id,
        &format!(
            "Legal case {case_id} was settled by negotiated payment of {}.",
            quote.amount
        ),
    );
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Legal,
        format!("Legal case {case_id} settled"),
        if settles_in_full {
            format!(
                "The dynasty paid {} to settle the {:?} claim before judgment; the grounded obligation is closed.",
                quote.amount, quote.kind
            )
        } else {
            format!(
                "The dynasty paid {} toward the {:?} claim before judgment; the remaining grounded obligation stands.",
                quote.amount, quote.kind
            )
        },
    )?;
    Ok(CommandOutcome {
        summary: format!("Settled legal case {case_id} for {}.", quote.amount),
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
    if let Some(last_change_day) =
        latest_audit_day_for_subject(state, AuditKind::HouseGovernanceChange, &subject)
    {
        let next_change_day =
            checked_future_day(last_change_day, HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS)?;
        if state.clock.day() < next_change_day {
            return Err(CommandError::HouseGovernanceCooldown { next_change_day });
        }
    }
    // Family unity is a real cost of forcing a governance change: a house
    // too divided to pay it cannot amend its own charter.
    if council.unity_basis_points < HOUSE_GOVERNANCE_UNITY_COST {
        return Err(CommandError::InsufficientFamilyUnity {
            available: council.unity_basis_points,
            required: HOUSE_GOVERNANCE_UNITY_COST,
        });
    }
    let next_charter_version = next_family_charter_version(dynasty_id, council.charter_version)?;
    let council = state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("validated family council must exist");
    council.governance = governance;
    council.charter_version = next_charter_version;
    council.unity_basis_points = council
        .unity_basis_points
        .checked_sub(HOUSE_GOVERNANCE_UNITY_COST)
        .expect("validated family unity must cover the governance cost");
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::HouseGovernanceChange,
        subject: subject.into(),
        detail: format!("governance={governance:?}"),
    });
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Family,
        "House charter amended".to_owned(),
        format!(
            "The dynasty adopted {governance:?} governance, changing administrative coordination, family cohesion, and succession risk."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!("Changed house governance to {governance:?}."),
    })
}

fn apply_family_council_meeting(state: &mut AppState) -> Result<CommandOutcome, CommandError> {
    let dynasty_id = state.player_dynasty_id;
    let subject = format!("dynasty:{dynasty_id};council-meeting");
    let council = state
        .family_councils
        .get(&dynasty_id)
        .ok_or(CommandError::MissingFamilyCouncil { dynasty_id })?;
    if let Some(last_meeting_day) =
        latest_audit_day_for_subject(state, AuditKind::FamilyCouncilMeeting, &subject)
    {
        let next_meeting_day =
            checked_future_day(last_meeting_day, FAMILY_COUNCIL_MEETING_INTERVAL_DAYS)?;
        if state.clock.day() < next_meeting_day {
            return Err(CommandError::FamilyCouncilMeetingCooldown { next_meeting_day });
        }
    }
    let member_ids: Vec<_> = council.members.iter().copied().collect();
    let unity_before = council.unity_basis_points;
    spend_player_treasury_to_market(state, FAMILY_COUNCIL_MEETING_COST)?;
    for character_id in member_ids {
        if let Some(character) = state.characters.get_mut(character_id)
            && character.status() == CharacterStatus::Active
        {
            character.runtime.loyalty_basis_points = character
                .runtime
                .loyalty_basis_points
                .saturating_add(FAMILY_COUNCIL_MEETING_LOYALTY_GAIN)
                .min(10_000);
        }
    }
    let council = state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("validated family council must exist");
    // A council already at full unity cannot gain more; report what actually
    // happened instead of claiming a rise that saturation absorbed.
    let unity_gain = FAMILY_COUNCIL_MEETING_UNITY_GAIN.min(10_000_u16.saturating_sub(unity_before));
    council.unity_basis_points = council
        .unity_basis_points
        .saturating_add(unity_gain)
        .min(10_000);
    let unity_after = council.unity_basis_points;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::FamilyCouncilMeeting,
        subject: subject.into(),
        detail: format!(
            "cost={};unity_before={unity_before};unity_after={unity_after};loyalty_gain={FAMILY_COUNCIL_MEETING_LOYALTY_GAIN}",
            FAMILY_COUNCIL_MEETING_COST.copper()
        ),
    });
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Family,
        "Family council convened".to_owned(),
        if unity_gain > 0 {
            format!(
                "The dynasty spent {FAMILY_COUNCIL_MEETING_COST} on settlements, hospitality, and internal obligations. Family unity rose from {unity_before} to {unity_after} bp and active council members gained loyalty."
            )
        } else {
            format!(
                "The dynasty spent {FAMILY_COUNCIL_MEETING_COST} on settlements, hospitality, and internal obligations. Family unity held at {unity_before} bp and active council members gained loyalty."
            )
        },
    )?;
    Ok(CommandOutcome {
        summary: format!("Convened the family council; unity is now {unity_after} bp."),
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
    let latest_designation =
        state.audit_log.iter().rev().find(|record| {
            record.kind() == AuditKind::HeirDesignation && record.subject() == subject
        });
    let confirmation = prior_heir_id == Some(character_id);
    // Confirming the sitting heir is a no-op only when the most recent
    // designation already names them. A later designation of a different
    // heir must not lock the family out of re-preparing this one.
    if confirmation
        && latest_designation
            .is_some_and(|record| super::heir_audit_detail_matches(record, character_id))
    {
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
    // Redrawing the succession wounds family cohesion; a divided house cannot
    // pay the unity price of a new charter line.
    if council.unity_basis_points < HEIR_DESIGNATION_UNITY_COST {
        return Err(CommandError::InsufficientFamilyUnity {
            available: council.unity_basis_points,
            required: HEIR_DESIGNATION_UNITY_COST,
        });
    }
    if let Some(last_designation_day) = latest_designation.map(AuditRecord::day) {
        let next_designation_day =
            checked_future_day(last_designation_day, HEIR_DESIGNATION_INTERVAL_DAYS)?;
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
        .checked_sub(HEIR_DESIGNATION_UNITY_COST)
        .expect("validated family unity must cover the heir designation cost");
    council.charter_version = next_charter_version;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::HeirDesignation,
        subject: subject.into(),
        detail: format!(
            "prior_heir={};{};confirmation={confirmation};legitimacy_cost={HEIR_DESIGNATION_LEGITIMACY_COST};unity_cost={HEIR_DESIGNATION_UNITY_COST}",
            prior_heir_id.map_or_else(|| "none".to_owned(), |id| id.to_string()),
            super::heir_designation_detail_component(character_id)
        ),
    });
    let chronicle_id = state.next_ids.try_chronicle()?;
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
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Family,
        if confirmation {
            format!("Character {character_id} confirmed as heir")
        } else {
            format!("Character {character_id} designated as heir")
        },
        format!(
            "The family charter now names character {character_id} as successor. The change cost \
             {HEIR_DESIGNATION_LEGITIMACY_COST} legitimacy and {HEIR_DESIGNATION_UNITY_COST} \
             family unity, but a formally prepared heir will face a far gentler transition when \
             the head of house dies."
        ),
    )?;
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
    // Resolve every fallible allocation before the first mutation so a
    // rejected adoption never leaves a debited treasury or orphan record.
    let ward_id = state.next_ids.try_character()?;
    let family_link_id: crate::ids::FamilyLinkId = state.next_ids.try_family_link()?;
    let chronicle_id: crate::ids::ChronicleEntryId = state.next_ids.try_chronicle()?;
    spend_player_treasury_to_market(state, WARD_ADOPTION_COST)?;
    let ward_name = format!("{dynasty_name} Ward {ward_id}");
    insert_ward_character(state, dynasty_id, ward_id, ward_name.clone(), focus);
    insert_ward_family_link(state, family_link_id, head_id, ward_id);
    let council = state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("validated family council must exist");
    council.members.insert(ward_id);
    council.unity_basis_points = council
        .unity_basis_points
        .checked_sub(WARD_ADOPTION_UNITY_COST)
        .expect("validated family unity must cover the ward adoption cost");
    let dynasty = state
        .dynasties
        .get_mut(&dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.legitimacy_basis_points = dynasty
        .resources
        .legitimacy_basis_points
        .saturating_sub(WARD_ADOPTION_LEGITIMACY_COST);
    dynasty.resources.administrative_capacity =
        dynasty.resources.administrative_capacity.saturating_add(8);
    record_ward_adoption(state, dynasty_id, ward_id, &ward_name, focus, chronicle_id)?;
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
    ensure_standing(
        state,
        WARD_ADOPTION_REPUTATION_REQUIREMENT,
        WARD_ADOPTION_DELIVERY_REQUIREMENT,
        |quality, reliability, required| CommandError::InsufficientWardReputation {
            quality,
            reliability,
            required,
        },
        |delivered, required| CommandError::InsufficientWardCommercialRecord {
            delivered,
            required,
        },
    )?;
    if legitimacy < WARD_ADOPTION_LEGITIMACY_REQUIREMENT {
        return Err(CommandError::InsufficientPlayerLegitimacy {
            available: legitimacy,
            required: WARD_ADOPTION_LEGITIMACY_REQUIREMENT,
        });
    }
    // Bringing a ward into the house strains family cohesion; a divided
    // council cannot absorb the adoption.
    let council = state
        .family_councils
        .get(&dynasty_id)
        .expect("validated family council must exist");
    if council.unity_basis_points < WARD_ADOPTION_UNITY_COST {
        return Err(CommandError::InsufficientFamilyUnity {
            available: council.unity_basis_points,
            required: WARD_ADOPTION_UNITY_COST,
        });
    }
    if let Some(last_adoption_day) =
        latest_audit_day(state, AuditKind::WardAdoption, |record_subject| {
            record_subject.starts_with(&format!("dynasty:{dynasty_id}:"))
        })
    {
        let next_adoption_day = checked_future_day(last_adoption_day, WARD_ADOPTION_INTERVAL_DAYS)?;
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
            incapacitated_day: None,
        },
    });
}

fn insert_ward_family_link(
    state: &mut AppState,
    family_link_id: crate::ids::FamilyLinkId,
    head_id: CharacterId,
    ward_id: CharacterId,
) {
    state.family_links.insert(
        family_link_id,
        FamilyLink {
            id: family_link_id,
            first_character_id: head_id,
            second_character_id: ward_id,
            kind: FamilyLinkKind::Ward,
            active: true,
        },
    );
}

fn record_ward_adoption(
    state: &mut AppState,
    dynasty_id: DynastyId,
    ward_id: CharacterId,
    ward_name: &str,
    focus: EducationFocus,
    chronicle_id: crate::ids::ChronicleEntryId,
) -> Result<(), CommandError> {
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::WardAdoption,
        subject: format!("dynasty:{dynasty_id}:character:{ward_id}").into(),
        detail: format!("focus={focus:?};cost={}", WARD_ADOPTION_COST.copper()),
    });
    state.chronicle.push(ChronicleEntry {
        id: chronicle_id,
        day: state.clock.day(),
        kind: ChronicleKind::FamilyExpanded,
        summary: format!("{ward_name} entered the dynasty as a ward focused on {focus:?}."),
    });
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Family,
        format!("Ward adopted: {ward_name}"),
        format!(
            "The dynasty spent {WARD_ADOPTION_COST} to adopt and train a new {focus:?}-focused household member."
        ),
    )?;
    Ok(())
}
/// Canonical active ward count for the player dynasty: a ward occupies a slot
/// only while both its guardian and the ward are active.
pub(crate) fn active_player_ward_count(state: &AppState) -> usize {
    state
        .family_links
        .values()
        .filter(|link| link.active && link.kind == FamilyLinkKind::Ward)
        .filter(|link| {
            let guardian_active = state
                .characters
                .get(link.first_character_id)
                .is_some_and(|character| character.status() == CharacterStatus::Active);
            let ward_active =
                state
                    .characters
                    .get(link.second_character_id)
                    .is_some_and(|character| {
                        character.dynasty_id() == state.player_dynasty_id
                            && character.status() == CharacterStatus::Active
                    });
            guardian_active && ward_active
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
    if let Some(next_education_day) = family_education_next_day(state, character_id)
        && state.clock.day() < next_education_day
    {
        return Err(CommandError::FamilyEducationCooldown { next_education_day });
    }
    spend_player_treasury_to_market(state, FAMILY_EDUCATION_COST)?;
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
        subject: family_education_subject(state.player_dynasty_id, character_id).into(),
        detail: format!("focus={focus:?};cost={}", FAMILY_EDUCATION_COST.copper()),
    });
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Family,
        format!("Family education completed for character {character_id}"),
        format!("The dynasty spent {FAMILY_EDUCATION_COST} on advanced {focus:?} training."),
    )?;
    Ok(CommandOutcome {
        summary: format!("Educated character {character_id} in {focus:?}."),
    })
}

fn family_education_subject(dynasty_id: DynastyId, character_id: CharacterId) -> String {
    format!("dynasty:{dynasty_id}:character:{character_id}")
}

pub(crate) fn family_education_next_day(
    state: &AppState,
    character_id: CharacterId,
) -> Option<i64> {
    let dynasty_prefix = format!("dynasty:{}:", state.player_dynasty_id);
    let dynasty_next = state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::FamilyEducation
                && record.subject().starts_with(&dynasty_prefix)
        })
        .map(|record| future_day_or_terminal(record.day(), FAMILY_EDUCATION_DYNASTY_INTERVAL_DAYS));
    let subject = family_education_subject(state.player_dynasty_id, character_id);
    let character_next = state
        .audit_log
        .iter()
        .rev()
        .find(|record| record.kind() == AuditKind::FamilyEducation && record.subject() == subject)
        .map(|record| future_day_or_terminal(record.day(), FAMILY_EDUCATION_INTERVAL_DAYS));
    dynasty_next.into_iter().chain(character_next).max()
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
    registry: &Registry,
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
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    validate_institution_support_standing(registry, state, institution_id, character_id)?;
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
    if let Some(next_support_day) =
        institution_support_next_day(state, institution_id, character_id)
        && state.clock.day() < next_support_day
    {
        return Err(CommandError::InstitutionSupportCooldown { next_support_day });
    }
    // An active guild entry restriction prices outsiders out of the charter:
    // joining costs more as the restriction tightens, up to a fifty percent
    // surcharge on the standard patronage contribution.
    let entry_restriction =
        super::strategic::active_law_value(state, LawKind::GuildEntryRestriction)
            .unwrap_or(0)
            .clamp(0, 10_000);
    let support_cost =
        INSTITUTION_SUPPORT_COST.saturating_mul_ratio(10_000 + entry_restriction / 2, 10_000);
    let budget_after = institution.budget.checked_add(support_cost).ok_or(
        CommandError::InstitutionBudgetOverflow {
            institution_id,
            current: institution.budget,
            incoming: support_cost,
        },
    )?;
    let member_dynasties: BTreeSet<_> = institution
        .members
        .iter()
        .filter_map(|member_id| state.characters.get(*member_id))
        .map(Character::dynasty_id)
        .filter(|dynasty_id| *dynasty_id != state.player_dynasty_id)
        .collect();
    let established_day =
        checked_future_day(state.clock.day(), INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS)?;
    spend_player_treasury(state, support_cost)?;
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    institution.budget = budget_after;
    institution.members.insert(character_id);
    record_institution_patronage_relationships(
        state,
        institution_id,
        character_id,
        member_dynasties,
    );
    finish_institution_patronage(
        state,
        institution_id,
        character_id,
        subject,
        established_day,
        support_cost,
    )
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
    established_day: i64,
    contribution: Money,
) -> Result<CommandOutcome, CommandError> {
    let day = state.clock.day();
    state.audit_log.push(AuditRecord {
        day,
        kind: AuditKind::InstitutionPatronage,
        subject: subject.into(),
        detail: format!("contribution={}", contribution.copper()),
    });
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("Institutional support cultivated for character {character_id}"),
        format!(
            "The dynasty patronized institution {institution_id}; character {character_id}'s support, and the legitimacy it earns the house, will be established by day {established_day}."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!(
            "Cultivated support for character {character_id} in institution {institution_id}."
        ),
    })
}

#[derive(Clone, Debug)]
struct ValidatedInstitutionEndowment {
    player_id: DynastyId,
    institution_id: InstitutionId,
    amount: Money,
    treasury_after: Money,
    contributions_after: Money,
    budget_after: Money,
    legitimacy_gain: u16,
    relationship_scale: i16,
    member_dynasties: BTreeSet<DynastyId>,
}

fn apply_institution_endowment(
    state: &mut AppState,
    institution_id: InstitutionId,
    amount: Money,
) -> Result<CommandOutcome, CommandError> {
    let validated = validate_institution_endowment(state, institution_id, amount)?;
    commit_institution_endowment(state, &validated)?;
    Ok(CommandOutcome {
        summary: format!("Endowed institution {institution_id} with {amount}."),
    })
}

fn validate_institution_endowment(
    state: &AppState,
    institution_id: InstitutionId,
    amount: Money,
) -> Result<ValidatedInstitutionEndowment, CommandError> {
    if amount < INSTITUTION_ENDOWMENT_MIN || amount > INSTITUTION_ENDOWMENT_MAX {
        return Err(CommandError::InstitutionEndowmentOutOfRange {
            minimum: INSTITUTION_ENDOWMENT_MIN,
            maximum: INSTITUTION_ENDOWMENT_MAX,
            requested: amount,
        });
    }
    let institution = state
        .institutions
        .get(&institution_id)
        .ok_or(CommandError::MissingInstitution { institution_id })?;
    if !has_established_player_institution_membership(state, institution_id) {
        return Err(CommandError::InstitutionEndowmentRequiresMembership { institution_id });
    }
    if let Some(next_endowment_day) = institution_endowment_next_day(state)
        && state.clock.day() < next_endowment_day
    {
        return Err(CommandError::InstitutionEndowmentCooldown { next_endowment_day });
    }
    let player_id = state.player_dynasty_id;
    let player = state
        .dynasties
        .get(&player_id)
        .expect("player dynasty must exist");
    if player.treasury() < amount {
        return Err(CommandError::InsufficientPlayerFunds {
            available: player.treasury(),
            required: amount,
        });
    }
    let treasury_after = player
        .treasury()
        .checked_sub(amount)
        .expect("validated endowment must fit player treasury");
    let contributions_after = player.civic_contributions().checked_add(amount).ok_or(
        super::SimulationError::DynastyCivicContributionsOverflow {
            dynasty_id: player_id,
            current: player.civic_contributions(),
            incoming: amount,
        },
    )?;
    let budget_after =
        institution
            .budget
            .checked_add(amount)
            .ok_or(CommandError::InstitutionBudgetOverflow {
                institution_id,
                current: institution.budget,
                incoming: amount,
            })?;
    let member_dynasties: BTreeSet<_> = institution
        .members
        .iter()
        .filter_map(|character_id| state.characters.get(*character_id))
        .map(Character::dynasty_id)
        .filter(|dynasty_id| *dynasty_id != player_id)
        .collect();
    let legitimacy_gain = u16::try_from((amount.copper() / 200).clamp(25, 250))
        .expect("bounded endowment legitimacy gain must fit u16");
    let relationship_scale =
        i16::try_from((amount.copper() / INSTITUTION_ENDOWMENT_MIN.copper()).clamp(1, 10))
            .expect("bounded endowment relationship scale must fit i16");
    Ok(ValidatedInstitutionEndowment {
        player_id,
        institution_id,
        amount,
        treasury_after,
        contributions_after,
        budget_after,
        legitimacy_gain,
        relationship_scale,
        member_dynasties,
    })
}

fn commit_institution_endowment(
    state: &mut AppState,
    endowment: &ValidatedInstitutionEndowment,
) -> Result<(), CommandError> {
    let player = state
        .dynasties
        .get_mut(&endowment.player_id)
        .expect("validated player dynasty must exist");
    player.resources.treasury = endowment.treasury_after;
    player.resources.civic_contributions = endowment.contributions_after;
    let institution = state
        .institutions
        .get_mut(&endowment.institution_id)
        .expect("validated institution must exist");
    institution.budget = endowment.budget_after;
    // Report the legitimacy the endowment actually bought: a gain absorbed by
    // the cap must not be recorded as if it had been delivered.
    let legitimacy_before = institution.legitimacy_basis_points;
    institution.legitimacy_basis_points = institution
        .legitimacy_basis_points
        .saturating_add(endowment.legitimacy_gain)
        .min(10_000);
    let applied_legitimacy_gain = institution.legitimacy_basis_points - legitimacy_before;
    for member_dynasty_id in &endowment.member_dynasties {
        super::strategic::adjust_dynasty_relationship(
            state,
            endowment.player_id,
            *member_dynasty_id,
            super::strategic::RelationshipDelta::new(
                endowment.relationship_scale.saturating_mul(8),
                endowment.relationship_scale.saturating_mul(15),
                0,
                -endowment.relationship_scale.saturating_mul(5),
                i32::from((endowment.relationship_scale.saturating_add(1)) / 2),
            ),
        );
        super::strategic::remember_dynasty_interaction(
            state,
            endowment.player_id,
            *member_dynasty_id,
            &format!(
                "the player dynasty endowed institution {} with {}, strengthening its standing among the membership",
                endowment.institution_id, endowment.amount
            ),
        );
    }
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::InstitutionEndowment,
        subject: format!(
            "institution:{};dynasty:{}",
            endowment.institution_id, endowment.player_id
        )
        .into(),
        detail: format!(
            "amount={};institution_legitimacy_gain={}",
            endowment.amount.copper(),
            applied_legitimacy_gain
        ),
    });
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("Institution {} endowed", endowment.institution_id),
        format!(
            "The dynasty endowed institution {} with {}, strengthening its budget, civic legitimacy, and standing among member houses.",
            endowment.institution_id, endowment.amount
        ),
    )?;
    Ok(())
}

pub(crate) fn has_established_player_institution_membership(
    state: &AppState,
    institution_id: InstitutionId,
) -> bool {
    let Some(institution) = state.institutions.get(&institution_id) else {
        return false;
    };
    institution.members.iter().copied().any(|character_id| {
        let active_player_member = state.characters.get(character_id).is_some_and(|character| {
            character.dynasty_id() == state.player_dynasty_id
                && character.status() == CharacterStatus::Active
        });
        active_player_member
            && (institution.office_holder_id == Some(character_id)
                || institution_support_day(state, institution_id, character_id).is_some_and(
                    |support_day| {
                        state.clock.day()
                            >= future_day_or_terminal(
                                support_day,
                                INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS,
                            )
                    },
                ))
    })
}

pub(crate) fn institution_endowment_next_day(state: &AppState) -> Option<i64> {
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::InstitutionEndowment
                && record.audit_subject().dynasty_id() == Some(state.player_dynasty_id)
        })
        .map(|record| future_day_or_terminal(record.day(), INSTITUTION_ENDOWMENT_INTERVAL_DAYS))
}

/// Reputation standing gate shared by every command that requires an
/// established house: the better of quality and reliability must clear the
/// requirement. The error payload carries the observed values so each caller
/// can surface its own typed variant.
fn ensure_reputation_standing(state: &AppState, required: u16) -> Result<(), (u16, u16, u16)> {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let quality = player.resources.reputation_quality_basis_points;
    let reliability = player.resources.reputation_reliability_basis_points;
    if quality.max(reliability) < required {
        Err((quality, reliability, required))
    } else {
        Ok(())
    }
}

/// Shared standing gate for privileged commands: the house needs both the
/// required reputation standing and a durable record of fulfilled contract
/// deliveries before it may act.
fn ensure_standing(
    state: &AppState,
    reputation_required: u16,
    deliveries_required: u32,
    insufficient_reputation: impl FnOnce(u16, u16, u16) -> CommandError,
    insufficient_deliveries: impl FnOnce(u32, u32) -> CommandError,
) -> Result<(), CommandError> {
    ensure_reputation_standing(state, reputation_required).map_err(
        |(quality, reliability, required)| insufficient_reputation(quality, reliability, required),
    )?;
    let delivered = player_contract_deliveries(state);
    if delivered < deliveries_required {
        return Err(insufficient_deliveries(delivered, deliveries_required));
    }
    Ok(())
}

fn validate_institution_support_standing(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Result<(), CommandError> {
    let required =
        institution_support_delivery_requirement(registry, state, institution_id, character_id);
    ensure_standing(
        state,
        INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT,
        required,
        |quality, reliability, required| CommandError::InsufficientInstitutionSupportReputation {
            quality,
            reliability,
            required,
        },
        |delivered, required| CommandError::InsufficientInstitutionSupportCommercialRecord {
            delivered,
            required,
        },
    )
}

pub(crate) fn institution_support_delivery_requirement(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> u32 {
    capability_adjusted_delivery_requirement(
        registry,
        state,
        institution_id,
        character_id,
        &StandingGateTuning {
            context: "institution support",
            base_requirement: INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT,
            capability_target_score: INSTITUTION_SUPPORT_CAPABILITY_TARGET_SCORE,
            capability_delivery_step: INSTITUTION_SUPPORT_CAPABILITY_DELIVERY_STEP,
            max_preparation_deliveries: INSTITUTION_SUPPORT_MAX_PREPARATION_DELIVERIES,
        },
    )
}

/// Tuning for a standing-gated action's commercial-record requirement.
#[derive(Clone, Copy)]
struct StandingGateTuning {
    /// Names the gate in validation panics.
    context: &'static str,
    base_requirement: u32,
    capability_target_score: u32,
    capability_delivery_step: u32,
    max_preparation_deliveries: u32,
}

/// Deliveries a standing-gated action requires: its base commercial record plus
/// preparation deliveries that cover the nominee's capability deficit for the
/// institution's line of work.
fn capability_adjusted_delivery_requirement(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
    tuning: &StandingGateTuning,
) -> u32 {
    let StandingGateTuning {
        context,
        base_requirement,
        capability_target_score,
        capability_delivery_step,
        max_preparation_deliveries,
    } = *tuning;
    let character = state
        .characters
        .get(character_id)
        .unwrap_or_else(|| panic!("{context} character must exist"));
    let institution_kind = registry
        .get_institution(institution_id)
        .unwrap_or_else(|| panic!("{context} institution must exist in the registry"))
        .kind();
    let capability_score =
        super::strategic::institution_capability_score(character, institution_kind);
    let deficit = capability_target_score.saturating_sub(capability_score);
    let extra_deliveries =
        deficit.saturating_add(capability_delivery_step - 1) / capability_delivery_step;
    base_requirement.saturating_add(extra_deliveries.min(max_preparation_deliveries))
}

fn apply_office_nomination(
    registry: &Registry,
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
    validate_office_nomination_standing(registry, state, institution_id, character_id)?;
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
    let available_day = checked_future_day(support_day, INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS)?;
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
    let selection_day = checked_future_day(state.clock.day(), OFFICE_NOMINATION_RESOLUTION_DAYS)?;
    spend_player_treasury_to_market(state, OFFICE_NOMINATION_CAMPAIGN_COST)?;
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    institution.next_selection_day = institution.next_selection_day.min(selection_day);
    let subject = office_nomination_subject(institution_id, character_id);
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::OfficeNomination,
        subject: subject.into(),
        detail: format!("campaign_cost={}", OFFICE_NOMINATION_CAMPAIGN_COST.copper()),
    });
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("Office campaign launched for character {character_id}"),
        format!(
            "The dynasty nominated character {character_id} to institution {institution_id}; selection is scheduled by day {selection_day}."
        ),
    )?;
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
    if !office_power_is_player_held(state, institution, power) {
        return Err(CommandError::OfficePowerUnavailable {
            institution_id,
            power,
        });
    }
    let available_day = checked_future_day(
        institution.term_started_day,
        OFFICE_POWER_ESTABLISHMENT_DAYS,
    )?;
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
    // The directive cadence belongs to the issuing officeholder's dynasty, not
    // to the institution: an elected successor must not inherit the previous
    // holder's cooldown on their first directive.
    if let Some(last_directive_day) = state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::OfficeDirective
                && record.audit_subject().institution_id() == Some(institution_id)
                && record.audit_subject().dynasty_id() == Some(state.player_dynasty_id)
        })
        .map(AuditRecord::day)
    {
        let next_directive_day =
            checked_future_day(last_directive_day, OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS)?;
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
    let subject = format!(
        "institution:{institution_id};dynasty:{}",
        state.player_dynasty_id
    );
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

/// Returns the legitimacy actually applied after the 10,000 bp cap, so
/// callers can report what was delivered rather than what was requested.
fn raise_institution_legitimacy(
    state: &mut AppState,
    institution_id: InstitutionId,
    amount: u16,
) -> u16 {
    let institution = state
        .institutions
        .get_mut(&institution_id)
        .expect("validated institution must exist");
    let before = institution.legitimacy_basis_points;
    institution.legitimacy_basis_points = before.saturating_add(amount).min(10_000);
    institution.legitimacy_basis_points - before
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
    let directive_expires_day =
        checked_future_day(state.clock.day(), OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS)?;
    // Resolve every fallible step before the first mutation.
    let chronicle_id = state.next_ids.try_chronicle()?;
    state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .legitimacy_basis_points = legitimacy
        .checked_sub(OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST)
        .expect("validated office directive legitimacy cost must fit");
    apply_office_power_directive_effect(state, institution_id, district_id, power);
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
    state.chronicle.push(ChronicleEntry {
        id: chronicle_id,
        day: state.clock.day(),
        kind: ChronicleKind::OfficeDirective,
        summary: format!(
            "The player dynasty directed institution {institution_id} to exercise {power:?} in district {district_id}."
        ),
    });
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Politics,
        format!("{power:?} directive issued through institution {institution_id}"),
        format!(
            "The dynasty spent {OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST} legitimacy to intensify {power:?} policy in district {district_id}."
        ),
    )?;
    Ok(CommandOutcome {
        summary: format!("Exercised {power:?} through institution {institution_id}."),
    })
}

fn validate_office_nomination_standing(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Result<(), CommandError> {
    let required =
        office_nomination_delivery_requirement(registry, state, institution_id, character_id);
    ensure_standing(
        state,
        OFFICE_NOMINATION_REPUTATION_REQUIREMENT,
        required,
        |quality, reliability, required| CommandError::InsufficientOfficeReputation {
            quality,
            reliability,
            required,
        },
        |delivered, required| CommandError::InsufficientOfficeCommercialRecord {
            delivered,
            required,
        },
    )
}

pub(crate) fn office_nomination_delivery_requirement(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> u32 {
    capability_adjusted_delivery_requirement(
        registry,
        state,
        institution_id,
        character_id,
        &StandingGateTuning {
            context: "office nomination",
            base_requirement: OFFICE_NOMINATION_DELIVERY_REQUIREMENT,
            capability_target_score: OFFICE_NOMINATION_CAPABILITY_TARGET_SCORE,
            capability_delivery_step: OFFICE_NOMINATION_CAPABILITY_DELIVERY_STEP,
            max_preparation_deliveries: OFFICE_NOMINATION_MAX_PREPARATION_DELIVERIES,
        },
    )
}

pub(super) fn office_nomination_subject(
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> String {
    format!("institution:{institution_id}:character:{character_id}")
}

fn future_day_or_terminal(day: i64, offset_days: i64) -> i64 {
    checked_future_day(day, offset_days).unwrap_or(i64::MAX)
}

pub(crate) fn office_nomination_next_day(
    state: &AppState,
    character_id: CharacterId,
) -> Option<i64> {
    // Every office campaign imposes its full recovery period from the
    // campaign day. Because the resolution window is shorter than the
    // ordinary interval, any retry at that interval would already land after
    // resolution, so a single stable schedule keeps the quoted retry day
    // honest and the accept/reject decisions unchanged.
    let campaign = latest_character_campaign_day(state, AuditKind::OfficeNomination, character_id)
        .map(|day| future_day_or_terminal(day, OFFICE_NOMINATION_RECOVERY_DAYS));
    let dynasty_office_resignation = latest_player_office_resignation_day(state)
        .map(|day| future_day_or_terminal(day, INSTITUTION_WITHDRAWAL_RECOVERY_DAYS));
    campaign.into_iter().chain(dynasty_office_resignation).max()
}

pub(crate) fn institution_support_subject(
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> String {
    format!("institution:{institution_id}:character:{character_id}")
}

pub(crate) fn institution_support_next_day(
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Option<i64> {
    let patronage =
        latest_character_campaign_day(state, AuditKind::InstitutionPatronage, character_id)
            .map(|day| future_day_or_terminal(day, INSTITUTION_SUPPORT_INTERVAL_DAYS));
    // Withdrawing from one institution costs standing with that house, not
    // with the whole city: the recovery cooldown is scoped to the institution
    // the character walked out of.
    let withdrawal =
        latest_character_campaign_day(state, AuditKind::InstitutionWithdrawal, character_id)
            .filter(|_| latest_withdrawal_institution(state, character_id) == Some(institution_id))
            .map(|day| future_day_or_terminal(day, INSTITUTION_WITHDRAWAL_RECOVERY_DAYS));
    // Resigning a held office, by contrast, is a city-visible act: it pauses
    // support cultivation everywhere until the same recovery period passes.
    let dynasty_office_resignation = latest_player_office_resignation_day(state)
        .map(|day| future_day_or_terminal(day, INSTITUTION_WITHDRAWAL_RECOVERY_DAYS));
    patronage
        .into_iter()
        .chain(withdrawal)
        .chain(dynasty_office_resignation)
        .max()
}

fn latest_withdrawal_institution(
    state: &AppState,
    character_id: CharacterId,
) -> Option<InstitutionId> {
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::InstitutionWithdrawal
                && record
                    .audit_subject()
                    .institution_character_ids()
                    .is_some_and(|(_, subject_character)| subject_character == character_id)
        })
        .and_then(|record| record.audit_subject().institution_character_ids())
        .map(|(institution_id, _)| institution_id)
}

fn latest_player_office_resignation_day(state: &AppState) -> Option<i64> {
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == AuditKind::InstitutionWithdrawal
                && record.detail() == super::OFFICE_RESIGNATION_AUDIT_DETAIL
                && record
                    .audit_subject()
                    .institution_character_ids()
                    .and_then(|(_, character_id)| state.characters.get(character_id))
                    .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
        })
        .map(AuditRecord::day)
}

pub(crate) fn institution_membership_count(state: &AppState, character_id: CharacterId) -> usize {
    state
        .institutions
        .values()
        .filter(|institution| institution.members.contains(&character_id))
        .count()
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

fn latest_audit_day_for_subject(state: &AppState, kind: AuditKind, subject: &str) -> Option<i64> {
    latest_audit_day(state, kind, |record_subject| record_subject == subject)
}

fn latest_character_campaign_day(
    state: &AppState,
    kind: AuditKind,
    character_id: CharacterId,
) -> Option<i64> {
    state
        .audit_log
        .iter()
        .rev()
        .find(|record| {
            record.kind() == kind
                && record
                    .audit_subject()
                    .institution_character_ids()
                    .is_some_and(|(_, recorded_character_id)| recorded_character_id == character_id)
        })
        .map(AuditRecord::day)
}

pub(crate) fn institution_support_day(
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> Option<i64> {
    let subject = institution_support_subject(institution_id, character_id);
    latest_audit_day_for_subject(state, AuditKind::InstitutionPatronage, &subject)
}

pub(crate) fn player_contract_deliveries(state: &AppState) -> u32 {
    super::contract_deliveries_for_dynasty(state, state.player_dynasty_id)
}

fn apply_crisis_response(
    registry: &Registry,
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
    let crisis_kind = crisis.kind;
    // Organized responses to a trade disruption send aid down the routes
    // themselves, so they heal route disruption by the same amount as crisis
    // severity; otherwise the tracked cause would outlive every response.
    // Profiteering heals nothing.
    let organized_response_severity_reduction = match response {
        CrisisResponse::Relief => 2_500,
        CrisisResponse::Reform => 1_800,
        CrisisResponse::Suppress => 2_000,
        CrisisResponse::Exploit => 0,
    };
    // Suppression spends standing like every other priced cost: a dynasty
    // without the legitimacy to pay rejects up front instead of silently
    // paying what little it has.
    if response == CrisisResponse::Suppress {
        let available_legitimacy = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points;
        if available_legitimacy < CRISIS_SUPPRESS_LEGITIMACY_COST {
            return Err(CommandError::InsufficientPlayerLegitimacy {
                available: available_legitimacy,
                required: CRISIS_SUPPRESS_LEGITIMACY_COST,
            });
        }
    }
    // Organized responses to a trade disruption heal the tracked routes first,
    // so the severity reduction is clamped against the post-healing route
    // condition and cannot declare victory over disruption that remains.
    if organized_response_severity_reduction > 0 && crisis_kind == CrisisKind::TradeDisruption {
        heal_disrupted_routes(state, organized_response_severity_reduction);
    }
    match response {
        CrisisResponse::Relief => {
            spend_player_treasury_to_market(state, crisis_relief_cost(severity))?;
            reduce_crisis(state, crisis_id, organized_response_severity_reduction);
            adjust_player_legitimacy(state, CRISIS_RELIEF_LEGITIMACY_GAIN, true);
            adjust_district_unrest(state, district_id, CRISIS_RELIEF_UNREST_REDUCTION, false);
        }
        CrisisResponse::Reform => {
            spend_player_treasury_to_market(state, CRISIS_REFORM_COST)?;
            reduce_crisis(state, crisis_id, organized_response_severity_reduction);
            adjust_player_legitimacy(state, CRISIS_REFORM_LEGITIMACY_GAIN, true);
            adjust_district_unrest(state, district_id, CRISIS_REFORM_UNREST_REDUCTION, false);
        }
        CrisisResponse::Suppress => {
            spend_player_treasury_to_market(state, CRISIS_SUPPRESS_COST)?;
            reduce_crisis(state, crisis_id, organized_response_severity_reduction);
            adjust_player_legitimacy(state, CRISIS_SUPPRESS_LEGITIMACY_COST, false);
            adjust_district_unrest(state, district_id, CRISIS_SUPPRESS_UNREST_INCREASE, true);
        }
        CrisisResponse::Exploit => {
            apply_crisis_exploitation(state, crisis_id, severity, district_id)?;
        }
    }
    // A guild revolt is a crisis of guild standing: the answer changes how
    // every chartered guild is seen, and that standing feeds back into future
    // revolt pressure.
    let mut guild_standing_note = String::new();
    if crisis_kind == CrisisKind::GuildRevolt {
        let standing_delta =
            super::strategic::apply_guild_revolt_standing_response(registry, state, response);
        guild_standing_note = if standing_delta >= 0 {
            format!(
                " The chartered guilds' public standing improves by {standing_delta} basis points."
            )
        } else {
            format!(
                " The chartered guilds' public standing falls by {} basis points.",
                -standing_delta
            )
        };
    }
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Crisis,
        format!("Response applied to crisis {crisis_id}"),
        format!("The dynasty chose {response:?}.{guild_standing_note}"),
    )?;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::CrisisResponse,
        subject: subject.into(),
        detail: super::strategic::crisis_response_audit_detail(response),
    });
    Ok(CommandOutcome {
        summary: format!("Applied {response:?} response to crisis {crisis_id}."),
    })
}

/// Crisis profiteering: extracts wealth from the panicked market into the
/// player treasury. The extraction matches what relief would cost so
/// profiteering is a genuine liquidity-of-last-resort trade: real money now,
/// in exchange for a worsened crisis, legitimacy, and unrest. The whole
/// result resolves before any mutation: an empty or overdrawn clearing pool
/// offers nothing to take, so spending legitimacy on the attempt must reject
/// up front instead of paying full price for zero gain.
fn apply_crisis_exploitation(
    state: &mut AppState,
    crisis_id: CrisisId,
    severity: u16,
    district_id: Option<DistrictId>,
) -> Result<(), CommandError> {
    let required_legitimacy = CRISIS_EXPLOIT_LEGITIMACY_REQUIREMENT;
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
    let desired_gain = crisis_relief_cost(severity);
    let gain = desired_gain.min(state.market.clearing_account.max(Money::ZERO));
    if gain <= Money::ZERO {
        return Err(CommandError::MarketExtractionUnavailable {
            available: state.market.clearing_account,
        });
    }
    let current_treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let resulting_treasury = current_treasury
        .checked_add(gain)
        .ok_or(CommandError::Strategic(
            StrategicError::DynastyTreasuryOverflow {
                dynasty_id: state.player_dynasty_id,
                current: current_treasury,
                incoming: gain,
            },
        ))?;
    debit_market_clearing_account(state, gain)?;
    let dynasty = state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    dynasty.resources.treasury = resulting_treasury;
    let crisis = state
        .crises
        .get_mut(&crisis_id)
        .expect("validated crisis must exist");
    crisis.severity_basis_points = severity
        .saturating_add(CRISIS_EXPLOIT_SEVERITY_INCREASE)
        .min(10_000);
    crisis.status = CrisisStatus::from_severity(crisis.severity_basis_points);
    adjust_player_legitimacy(state, CRISIS_EXPLOIT_LEGITIMACY_COST, false);
    adjust_district_unrest(state, district_id, CRISIS_EXPLOIT_UNREST_INCREASE, true);
    Ok(())
}

fn validate_crisis_response_history(
    state: &AppState,
    crisis_id: CrisisId,
    response: CrisisResponse,
) -> Result<String, CommandError> {
    let crisis_kind = state
        .crises
        .get(&crisis_id)
        .map(|crisis| crisis.kind)
        .ok_or(CommandError::MissingCrisis { crisis_id })?;
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
    let has_exploitation_response = prior_responses.iter().any(|record| {
        super::strategic::audit_record_crisis_response(record) == Some(CrisisResponse::Exploit)
    });
    // A trade disruption is contained by healing its routes, and each response
    // heals a bounded amount of the tracked disruption. Repeated organized
    // responses are therefore a real strategy with real costs, not spam; other
    // crises remain one-organized-response problems.
    let containment_locked = has_containment_response && crisis_kind != CrisisKind::TradeDisruption;
    if containment_locked || (response == CrisisResponse::Exploit && has_exploitation_response) {
        return Err(CommandError::CrisisAlreadyAddressed { crisis_id });
    }
    Ok(subject)
}

/// A trade-disruption response sends aid down the disrupted routes. The aid
/// budget equals the response's severity reduction and is spent where
/// disruption runs deepest first, so one response heals a bounded amount of
/// total disruption instead of magically treating every route at full strength.
fn heal_disrupted_routes(state: &mut AppState, mut aid_basis_points: u16) {
    let mut disrupted: Vec<(std::cmp::Reverse<u16>, ExternalRouteId)> = state
        .external_routes
        .values()
        .filter(|route| route.disruption_basis_points > 0)
        .map(|route| (std::cmp::Reverse(route.disruption_basis_points), route.id))
        .collect();
    disrupted.sort_unstable();
    for (_, route_id) in disrupted {
        if aid_basis_points == 0 {
            break;
        }
        let Some(route) = state.external_routes.get_mut(&route_id) else {
            continue;
        };
        let healed = route.disruption_basis_points.min(aid_basis_points);
        route.disruption_basis_points = route.disruption_basis_points.saturating_sub(healed);
        aid_basis_points = aid_basis_points.saturating_sub(healed);
    }
}

fn reduce_crisis(state: &mut AppState, crisis_id: CrisisId, amount: u16) {
    let crisis = state
        .crises
        .get_mut(&crisis_id)
        .expect("validated crisis must exist");
    crisis.severity_basis_points = crisis.severity_basis_points.saturating_sub(amount);
    // A tracked trade disruption holds at the condition that spawned it: a
    // response cannot mark it resolved while any route remains disrupted at or
    // above the detection threshold, or the next monthly pass would immediately
    // re-detect an identical replacement crisis and orphan this audit trail.
    if crisis.kind == CrisisKind::TradeDisruption {
        let worst_route_disruption = state
            .external_routes
            .values()
            .map(|route| route.disruption_basis_points)
            .max()
            .unwrap_or(0);
        if worst_route_disruption >= super::strategic::TRADE_DISRUPTION_ROUTE_DISRUPTION_THRESHOLD {
            crisis.severity_basis_points = crisis.severity_basis_points.max(worst_route_disruption);
        }
    }
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
        LaborResponse::Negotiate => {
            // Negotiation raises the wage by a tenth, but never past the same
            // per-worker ceiling the direct wage lever enforces, so repeated
            // dispute cycles cannot ratchet payroll past the supported range.
            let workers = i64::from(agreement.workers().max(1));
            let raise = agreement.weekly_wage.checked_mul_ratio(11, 10).ok_or(
                CommandError::LaborWageOverflow {
                    employment_id,
                    current: agreement.weekly_wage,
                },
            )?;
            let ceiling = MAX_WEEKLY_WAGE_PER_WORKER
                .checked_mul_ratio(workers, 1)
                .ok_or(CommandError::LaborWageOverflow {
                    employment_id,
                    current: agreement.weekly_wage,
                })?;
            // Negotiation is a concession, never a cut: a wage already above
            // the per-worker ceiling (reachable through market wage pressure)
            // stays where it is instead of being lowered onto disputing
            // workers.
            Ok(Some(raise.min(ceiling).max(agreement.weekly_wage)))
        }
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
    ensure_operable_owned_business(state, business_id)?;
    if agreement.status != EmploymentStatus::Disputed {
        return Err(CommandError::InvalidLaborDispute { employment_id });
    }
    let negotiated_weekly_wage =
        validate_negotiated_weekly_wage(agreement, employment_id, response)?;
    match response {
        LaborResponse::ImproveConditions => {
            spend_business_cash(state, business_id, LABOR_CONDITIONS_IMPROVEMENT_COST)?;
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
            spend_business_cash(state, business_id, LABOR_NEGOTIATION_COST)?;
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
                            && super::available_household_workers(state, **id) >= u32::from(workers)
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
    super::strategic::try_push_outbox(
        state,
        OutboxKind::District,
        format!("Labor dispute {employment_id} resolved"),
        format!("The dynasty chose {response:?}."),
    )?;
    Ok(CommandOutcome {
        summary: format!("Resolved labor dispute {employment_id} with {response:?}."),
    })
}

/// Treasury cost of funding crisis relief for a crisis of the given severity.
///
/// Relief is the premium response: a fixed mobilization base plus a scaling
/// grant per severity point keeps it two to three times the cost of Reform
/// across the working severity range, instead of an unbounded per-point rate
/// that priced a single response above a small house's entire treasury.
#[must_use]
pub(crate) fn crisis_relief_cost(severity_basis_points: u16) -> Money {
    Money::from_copper(
        CRISIS_RELIEF_BASE_COST_COPPER
            + i64::from(severity_basis_points.max(1)) / CRISIS_RELIEF_SEVERITY_DIVISOR,
    )
}

/// Initial sponsor contribution demanded for a public work of the given budget.
#[must_use]
pub(crate) fn public_work_initial_contribution(budget: Money) -> Money {
    Money::from_copper((budget.copper() / 10).max(1)).min(budget)
}

/// Cash a player-driven business action may spend: everything above the
/// operating-reserve floor enforced by the business's own daily purchase and
/// production decisions.
#[must_use]
pub(crate) fn business_operating_spendable_cash(business: &crate::core::Business) -> Money {
    business
        .cash()
        .saturating_sub(business.policy.minimum_cash_reserve)
        .max(Money::ZERO)
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
    let day = state.clock.day();
    let expires_day = checked_future_day(day, INFORMATION_REPORT_LIFETIME_DAYS)?;
    // Resolve every fallible step before the first mutation.
    let id = state.next_ids.try_information_report()?;
    spend_player_treasury_to_market(state, INFORMATION_COMMISSION_COST)?;
    state.information_reports.retain(|_, report| {
        report.owner_dynasty_id != state.player_dynasty_id || report.target != Some(plan.target)
    });
    state.information_reports.insert(
        id,
        InformationReport {
            id,
            owner_dynasty_id: state.player_dynasty_id,
            target: Some(plan.target),
            subject: plan.subject.clone(),
            confidence: InformationConfidence::Confirmed,
            created_day: day,
            expires_day,
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
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Information,
        "Commissioned intelligence delivered".to_owned(),
        format!("{} is now available to the dynasty.", plan.subject),
    )?;
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
            checked_future_day(last_commission_day, INFORMATION_COMMISSION_INTERVAL_DAYS)?;
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
            "Treasury {}; reliability {}; trust {}; respect {}; fear {}; resentment {}; obligation {}; unsettled bilateral credit {}.",
            dynasty.treasury(),
            basis_points_label(dynasty.resources.reputation_reliability_basis_points),
            basis_points_label(relationship.trust_basis_points),
            basis_points_label(relationship.respect_basis_points),
            basis_points_label(relationship.fear_basis_points),
            basis_points_label(relationship.resentment_basis_points),
            relationship.obligation,
            unsettled_credit
        ),
    })
}

/// Formats basis points as a percentage with one correctly rounded decimal
/// digit, so intelligence briefs never truncate their own figures.
fn basis_points_label(basis_points: u16) -> String {
    let tenths_of_percent = (basis_points + 5) / 10;
    format!("{}.{}%", tenths_of_percent / 10, tenths_of_percent % 10)
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
            "Rent index {}; employment {}; sanitation {}; safety {}; unrest {}; population {}.",
            basis_points_label(runtime.rent_index_basis_points),
            basis_points_label(runtime.employment_basis_points),
            basis_points_label(runtime.sanitation_basis_points),
            basis_points_label(runtime.safety_basis_points),
            basis_points_label(runtime.unrest_basis_points),
            district.population()
        ),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InformationLeverageQuote {
    pub report_id: InformationReportId,
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
    CounterpartyContract {
        dynasty_id: DynastyId,
        contract_id: ContractId,
        previous_price: Money,
        new_price: Money,
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
    let (contract, counterparty_id, new_price) = state
        .contracts
        .values()
        .filter(|contract| contract.status == ContractStatus::Active && contract.good_id == good_id)
        .find_map(|contract| {
            market_contract_leverage_terms(state, player_id, contract)
                .map(|(counterparty_id, new_price)| (contract, counterparty_id, new_price))
        })
        .ok_or(CommandError::InformationReportHasNoLeverage { report_id })?;
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

fn market_contract_leverage_terms(
    state: &AppState,
    player_id: DynastyId,
    contract: &crate::core::SupplyContract,
) -> Option<(DynastyId, Money)> {
    let buyer_owner = state
        .businesses
        .get(contract.buyer_business_id)?
        .owner_dynasty_id();
    let seller_owner = state
        .businesses
        .get(contract.seller_business_id)?
        .owner_dynasty_id();
    let one_copper = Money::from_copper(1);
    let (counterparty_id, new_price) = if buyer_owner == player_id && seller_owner != player_id {
        let discounted = contract.unit_price.checked_mul_ratio(95, 100)?;
        let one_copper_less = contract.unit_price.checked_sub(one_copper)?;
        (
            seller_owner,
            discounted.min(one_copper_less).max(one_copper),
        )
    } else if seller_owner == player_id && buyer_owner != player_id {
        let increased = contract.unit_price.checked_mul_ratio(105, 100)?;
        let one_copper_more = contract.unit_price.checked_add(one_copper)?;
        (buyer_owner, increased.max(one_copper_more))
    } else {
        return None;
    };
    if new_price == contract.unit_price
        || crate::money::checked_cost_for(contract.quantity_per_week, new_price).is_none()
    {
        return None;
    }
    Some((counterparty_id, new_price))
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
    if let Some((contract, new_price)) = state
        .contracts
        .values()
        .filter(|contract| contract.status == ContractStatus::Active)
        .find_map(|contract| {
            market_contract_leverage_terms(state, state.player_dynasty_id, contract)
                .filter(|(counterparty_id, _)| *counterparty_id == dynasty_id)
                .map(|(_, new_price)| (contract, new_price))
        })
    {
        return Ok(InformationLeveragePlan {
            quote: InformationLeverageQuote {
                report_id,
                description: format!(
                    "use report {report_id} to negotiate contract {} with House {} from {} to {} per unit",
                    contract.id,
                    dynasty.name(),
                    contract.unit_price,
                    new_price
                ),
            },
            effect: InformationLeverageEffect::CounterpartyContract {
                dynasty_id,
                contract_id: contract.id,
                previous_price: contract.unit_price,
                new_price,
            },
        });
    }
    Ok(InformationLeveragePlan {
        quote: InformationLeverageQuote {
            report_id,
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
    spend_player_treasury_to_market(state, INFORMATION_LEVERAGE_COST)?;
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
    super::strategic::try_push_outbox(
        state,
        OutboxKind::Information,
        "Commissioned intelligence converted into action".to_owned(),
        format!(
            "{} at a cost of {INFORMATION_LEVERAGE_COST}.",
            plan.quote.description
        ),
    )?;
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
            adjust_information_relationship(state, counterparty_id, -75, 50, 125, 0, &memory);
        }
        InformationLeverageEffect::Counterparty { dynasty_id } => {
            adjust_information_relationship(
                state,
                dynasty_id,
                300,
                200,
                -200,
                2,
                "targeted outreach based on a commissioned house brief",
            );
        }
        InformationLeverageEffect::CounterpartyContract {
            dynasty_id,
            contract_id,
            previous_price,
            new_price,
        } => {
            state
                .contracts
                .get_mut(&contract_id)
                .expect("validated counterparty contract must exist")
                .unit_price = new_price;
            // Forcing a price concession with commissioned intelligence reads
            // to the target exactly like the market-brief squeeze: distrust
            // and resentment, not gratitude. Positive deltas are reserved for
            // outreach that extracts no price change.
            adjust_information_relationship(
                state,
                dynasty_id,
                -75,
                50,
                125,
                0,
                &format!(
                    "a commissioned house brief supported a negotiated contract adjustment from {previous_price} to {new_price}"
                ),
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
    obligation_change: i32,
    memory: &str,
) {
    let player_id = state.player_dynasty_id;
    super::strategic::adjust_dynasty_relationship(
        state,
        player_id,
        counterparty_id,
        super::strategic::RelationshipDelta::new(
            trust_change,
            respect_change,
            0,
            resentment_change,
            obligation_change,
        ),
    );
    super::strategic::remember_dynasty_interaction(state, player_id, counterparty_id, memory);
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
    if acknowledged == 0 {
        // Every notification through `message_id` was already acknowledged:
        // like other unchanged-state requests this is a typed no-op failure,
        // not a successful command that mutated nothing.
        return Err(CommandError::NotificationAlreadyAcknowledged { message_id });
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
