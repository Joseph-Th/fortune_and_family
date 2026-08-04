//! Strategic runtime records spanning contracts, finance, property, institutions, law, and civic life.

use crate::ids::{
    BusinessId, CharacterId, CivicDebtId, ContractId, CrisisId, DistrictId, DynastyId,
    EmploymentId, ExternalRouteId, FamilyLinkId, GoodId, InformationReportId, InstitutionId, LawId,
    LegalCaseId, LoanId, ObjectiveId, OutboxMessageId, PropertyId, PublicWorkId,
};
use crate::money::{Money, Quantity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractStatus {
    Active,
    Fulfilled,
    Breached,
    Renegotiated,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyContract {
    pub(crate) id: ContractId,
    pub(crate) buyer_business_id: BusinessId,
    pub(crate) seller_business_id: BusinessId,
    pub(crate) good_id: GoodId,
    pub(crate) quantity_per_week: Quantity,
    pub(crate) unit_price: Money,
    pub(crate) penalty: Money,
    pub(crate) next_due_day: i64,
    pub(crate) end_day: i64,
    pub(crate) fulfilled_deliveries: u16,
    pub(crate) missed_deliveries: u16,
    pub(crate) breaching_dynasty_id: Option<DynastyId>,
    pub(crate) status: ContractStatus,
}

impl SupplyContract {
    #[must_use]
    pub const fn id(&self) -> ContractId {
        self.id
    }

    #[must_use]
    pub const fn status(&self) -> ContractStatus {
        self.status
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoanStatus {
    Current,
    Delinquent,
    Defaulted,
    Repaid,
    Restructured,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loan {
    pub(crate) id: LoanId,
    pub(crate) lender_dynasty_id: DynastyId,
    pub(crate) borrower_dynasty_id: DynastyId,
    pub(crate) principal: Money,
    pub(crate) balance: Money,
    pub(crate) weekly_payment: Money,
    pub(crate) interest_basis_points: u16,
    pub(crate) next_due_day: i64,
    pub(crate) missed_payments: u16,
    pub(crate) collateral_property_id: Option<PropertyId>,
    pub(crate) status: LoanStatus,
}

impl Loan {
    #[must_use]
    pub const fn id(&self) -> LoanId {
        self.id
    }

    #[must_use]
    pub const fn status(&self) -> LoanStatus {
        self.status
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropertyKind {
    Residence,
    Workshop,
    Warehouse,
    Tenement,
    MarketRight,
    RuralEstate,
    CivicBuilding,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Property {
    pub(crate) id: PropertyId,
    pub(crate) name: String,
    pub(crate) kind: PropertyKind,
    pub(crate) district_id: DistrictId,
    pub(crate) owner_dynasty_id: Option<DynastyId>,
    pub(crate) occupant_business_id: Option<BusinessId>,
    pub(crate) tenant_dynasty_id: Option<DynastyId>,
    pub(crate) value: Money,
    pub(crate) weekly_rent: Money,
    pub(crate) condition_basis_points: u16,
    pub(crate) collateral_loan_id: Option<LoanId>,
}

impl Property {
    #[must_use]
    pub const fn id(&self) -> PropertyId {
        self.id
    }

    #[must_use]
    pub const fn district_id(&self) -> DistrictId {
        self.district_id
    }

    #[must_use]
    pub const fn owner_dynasty_id(&self) -> Option<DynastyId> {
        self.owner_dynasty_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmploymentStatus {
    Active,
    Disputed,
    Suspended,
    Ended,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmploymentAgreement {
    pub(crate) id: EmploymentId,
    pub(crate) business_id: BusinessId,
    pub(crate) household_id: crate::ids::HouseholdId,
    pub(crate) workers: u16,
    pub(crate) weekly_wage: Money,
    pub(crate) loyalty_basis_points: u16,
    pub(crate) conditions_basis_points: u16,
    pub(crate) status: EmploymentStatus,
}

impl EmploymentAgreement {
    #[must_use]
    pub const fn id(&self) -> EmploymentId {
        self.id
    }

    #[must_use]
    pub const fn business_id(&self) -> BusinessId {
        self.business_id
    }

    #[must_use]
    pub const fn household_id(&self) -> crate::ids::HouseholdId {
        self.household_id
    }

    #[must_use]
    pub const fn workers(&self) -> u16 {
        self.workers
    }

    #[must_use]
    pub const fn weekly_wage(&self) -> Money {
        self.weekly_wage
    }

    #[must_use]
    pub const fn loyalty_basis_points(&self) -> u16 {
        self.loyalty_basis_points
    }

    #[must_use]
    pub const fn conditions_basis_points(&self) -> u16 {
        self.conditions_basis_points
    }

    #[must_use]
    pub const fn status(&self) -> EmploymentStatus {
        self.status
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FamilyLinkKind {
    Marriage,
    ParentChild,
    Sibling,
    Ward,
    Adoptive,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyLink {
    pub(crate) id: FamilyLinkId,
    pub(crate) first_character_id: CharacterId,
    pub(crate) second_character_id: CharacterId,
    pub(crate) kind: FamilyLinkKind,
    pub(crate) active: bool,
    pub(crate) property_claim_basis_points: u16,
}

impl FamilyLink {
    #[must_use]
    pub const fn id(&self) -> FamilyLinkId {
        self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HouseGovernance {
    HeadCommand,
    Primogeniture,
    FamilyPartnership,
    BranchFederation,
    ElectedHead,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyCouncilState {
    pub(crate) dynasty_id: DynastyId,
    pub(crate) governance: HouseGovernance,
    pub(crate) members: BTreeSet<CharacterId>,
    pub(crate) unity_basis_points: u16,
    pub(crate) charter_version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum OfficePower {
    Licenses,
    Inspections,
    MarketTolls,
    DebtEnforcement,
    CityContracts,
    PublicWorks,
    WatchPriorities,
    Taxation,
    EmergencyImports,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstitutionRuntime {
    pub(crate) institution_id: InstitutionId,
    pub(crate) members: BTreeSet<CharacterId>,
    pub(crate) office_holder_id: Option<CharacterId>,
    pub(crate) powers: BTreeSet<OfficePower>,
    pub(crate) budget: Money,
    pub(crate) legitimacy_basis_points: u16,
    pub(crate) term_started_day: i64,
    pub(crate) next_selection_day: i64,
    pub(crate) term_number: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LawKind {
    BreadPriceCeiling,
    ForeignMerchantToll,
    InterestLimit,
    FireCode,
    RentRestriction,
    GuildEntryRestriction,
    EmergencyImports,
    PublicDebtAuthorization,
}

impl LawKind {
    #[must_use]
    pub const fn remains_active_after_enactment(self) -> bool {
        !matches!(self, Self::PublicDebtAuthorization)
    }

    #[must_use]
    pub const fn is_value_valid(self, value: i64) -> bool {
        match self {
            Self::BreadPriceCeiling => value > 0,
            Self::PublicDebtAuthorization => value > 0 && value <= 1_000_000,
            Self::ForeignMerchantToll
            | Self::InterestLimit
            | Self::FireCode
            | Self::RentRestriction
            | Self::GuildEntryRestriction
            | Self::EmergencyImports => value >= 0 && value <= 10_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CivicDebtStatus {
    Current,
    Delinquent,
    Defaulted,
    Repaid,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CivicDebt {
    pub(crate) id: CivicDebtId,
    pub(crate) creditor_dynasty_id: DynastyId,
    pub(crate) authorizing_law_id: LawId,
    pub(crate) sponsor_dynasty_id: Option<DynastyId>,
    pub(crate) principal: Money,
    pub(crate) balance: Money,
    pub(crate) weekly_payment: Money,
    pub(crate) interest_basis_points: u16,
    pub(crate) issued_day: i64,
    pub(crate) next_due_day: i64,
    pub(crate) missed_payments: u8,
    pub(crate) status: CivicDebtStatus,
}

impl CivicDebt {
    #[must_use]
    pub const fn id(&self) -> CivicDebtId {
        self.id
    }

    #[must_use]
    pub const fn creditor_dynasty_id(&self) -> DynastyId {
        self.creditor_dynasty_id
    }

    #[must_use]
    pub const fn balance(&self) -> Money {
        self.balance
    }

    #[must_use]
    pub const fn status(&self) -> CivicDebtStatus {
        self.status
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnactedLaw {
    pub(crate) id: LawId,
    pub(crate) kind: LawKind,
    pub(crate) enacted_day: i64,
    pub(crate) sponsor_dynasty_id: Option<DynastyId>,
    pub(crate) value: i64,
    pub(crate) active: bool,
}

impl EnactedLaw {
    #[must_use]
    pub const fn id(&self) -> LawId {
        self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DynastyPair {
    pub(crate) first: DynastyId,
    pub(crate) second: DynastyId,
}

impl DynastyPair {
    #[must_use]
    /// Creates a canonical pair of distinct dynasties.
    ///
    /// # Panics
    ///
    /// Panics when both identifiers refer to the same dynasty.
    pub fn new(left: DynastyId, right: DynastyId) -> Self {
        assert_ne!(left, right, "dynasty relationship pair must be distinct");
        if left <= right {
            Self {
                first: left,
                second: right,
            }
        } else {
            Self {
                first: right,
                second: left,
            }
        }
    }
}

impl Serialize for DynastyPair {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{}:{}", self.first.value(), self.second.value()))
    }
}

impl<'de> Deserialize<'de> for DynastyPair {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let (first, second) = value
            .split_once(':')
            .ok_or_else(|| serde::de::Error::custom("dynasty pair must use first:second format"))?;
        let first = first
            .parse::<u32>()
            .map_err(|_| serde::de::Error::custom("invalid first dynasty ID"))?;
        let second = second
            .parse::<u32>()
            .map_err(|_| serde::de::Error::custom("invalid second dynasty ID"))?;
        if first == second {
            return Err(serde::de::Error::custom(
                "dynasty relationship pair must be distinct",
            ));
        }
        Ok(Self::new(DynastyId::new(first), DynastyId::new(second)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipState {
    pub(crate) pair: DynastyPair,
    pub(crate) trust_basis_points: u16,
    pub(crate) fear_basis_points: u16,
    pub(crate) respect_basis_points: u16,
    pub(crate) obligation: i32,
    pub(crate) resentment_basis_points: u16,
    pub(crate) last_interaction_day: i64,
    pub(crate) memories: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InformationConfidence {
    Rumored,
    Probable,
    Confirmed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InformationReport {
    pub(crate) id: InformationReportId,
    pub(crate) owner_dynasty_id: DynastyId,
    pub(crate) subject: String,
    pub(crate) confidence: InformationConfidence,
    pub(crate) created_day: i64,
    pub(crate) expires_day: i64,
    pub(crate) source: String,
    pub(crate) summary: String,
}

impl InformationReport {
    #[must_use]
    pub const fn id(&self) -> InformationReportId {
        self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectiveKind {
    AccumulateCash,
    AcquireProperty,
    WinOffice,
    SecureSupply,
    ReduceDebt,
    ImproveLegitimacy,
    ContainRival,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectiveStatus {
    Planned,
    Pursuing,
    Achieved,
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiObjective {
    pub(crate) id: ObjectiveId,
    pub(crate) dynasty_id: DynastyId,
    pub(crate) kind: ObjectiveKind,
    pub(crate) target_dynasty_id: Option<DynastyId>,
    pub(crate) priority: u16,
    pub(crate) created_day: i64,
    pub(crate) status: ObjectiveStatus,
    pub(crate) rationale: String,
}

impl AiObjective {
    #[must_use]
    pub const fn id(&self) -> ObjectiveId {
        self.id
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistrictRuntime {
    pub(crate) district_id: DistrictId,
    pub(crate) rent_index_basis_points: u16,
    pub(crate) employment_basis_points: u16,
    pub(crate) sanitation_basis_points: u16,
    pub(crate) safety_basis_points: u16,
    pub(crate) unrest_basis_points: u16,
    pub(crate) dynasty_support: Vec<(DynastyId, u16)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublicWorkKind {
    Road,
    Bridge,
    Market,
    Granary,
    Drainage,
    WatchStation,
    Hospital,
    School,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublicWorkStatus {
    Planned,
    Building,
    Completed,
    Suspended,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicWork {
    pub(crate) id: PublicWorkId,
    pub(crate) district_id: DistrictId,
    pub(crate) kind: PublicWorkKind,
    pub(crate) sponsor_dynasty_id: Option<DynastyId>,
    pub(crate) budget: Money,
    pub(crate) spent: Money,
    pub(crate) progress_basis_points: u16,
    pub(crate) status: PublicWorkStatus,
}

impl PublicWork {
    #[must_use]
    pub const fn id(&self) -> PublicWorkId {
        self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LegalCaseKind {
    Debt,
    ContractBreach,
    Property,
    Corruption,
    Fraud,
    Inheritance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegalCaseStatus {
    Filed,
    Hearing,
    DecidedForPlaintiff,
    DecidedForDefendant,
    Settled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegalCase {
    pub(crate) id: LegalCaseId,
    pub(crate) plaintiff_dynasty_id: DynastyId,
    pub(crate) defendant_dynasty_id: DynastyId,
    pub(crate) kind: LegalCaseKind,
    pub(crate) evidence_basis_points: u16,
    pub(crate) public_attention_basis_points: u16,
    pub(crate) filed_day: i64,
    pub(crate) hearing_day: i64,
    pub(crate) damages: Money,
    pub(crate) status: LegalCaseStatus,
}

impl LegalCase {
    #[must_use]
    pub const fn id(&self) -> LegalCaseId {
        self.id
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRoute {
    pub(crate) id: ExternalRouteId,
    pub(crate) name: String,
    pub(crate) good_id: GoodId,
    pub(crate) daily_capacity: Quantity,
    pub(crate) risk_basis_points: u16,
    pub(crate) disruption_basis_points: u16,
    pub(crate) toll_basis_points: u16,
    pub(crate) active: bool,
}

impl ExternalRoute {
    #[must_use]
    pub const fn id(&self) -> ExternalRouteId {
        self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrisisKind {
    GrainShortage,
    BankingPanic,
    UrbanFire,
    GuildRevolt,
    NobleDemand,
    Epidemic,
    TradeDisruption,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrisisStatus {
    Emerging,
    Active,
    Resolved,
    Escalated,
}

impl CrisisStatus {
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Emerging | Self::Active | Self::Escalated)
    }

    #[must_use]
    pub const fn from_severity(severity_basis_points: u16) -> Self {
        if severity_basis_points < 500 {
            Self::Resolved
        } else if severity_basis_points >= 8_000 {
            Self::Escalated
        } else {
            Self::Active
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crisis {
    pub(crate) id: CrisisId,
    pub(crate) kind: CrisisKind,
    pub(crate) district_id: Option<DistrictId>,
    pub(crate) started_day: i64,
    pub(crate) severity_basis_points: u16,
    pub(crate) status: CrisisStatus,
    pub(crate) cause: String,
}

impl Crisis {
    #[must_use]
    pub const fn id(&self) -> CrisisId {
        self.id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutboxKind {
    Contract,
    Finance,
    Property,
    Family,
    Politics,
    Law,
    District,
    Legal,
    Crisis,
    Information,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxMessage {
    pub(crate) id: OutboxMessageId,
    pub(crate) day: i64,
    pub(crate) kind: OutboxKind,
    pub(crate) subject: String,
    pub(crate) body: String,
    pub(crate) acknowledged: bool,
}

impl OutboxMessage {
    #[must_use]
    pub const fn id(&self) -> OutboxMessageId {
        self.id
    }

    #[must_use]
    pub const fn day(&self) -> i64 {
        self.day
    }

    #[must_use]
    pub const fn kind(&self) -> OutboxKind {
        self.kind
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }
}
