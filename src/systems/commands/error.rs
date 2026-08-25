//! Typed player-command failures and their conversions.

#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CommandError {
    #[error(transparent)]
    IdentifierAllocation(#[from] IdentifierAllocationError),
    #[error(transparent)]
    Timeline(#[from] TimelineError),
    #[error(transparent)]
    Strategic(#[from] StrategicError),
    #[error(transparent)]
    Simulation(#[from] crate::systems::SimulationError),
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
