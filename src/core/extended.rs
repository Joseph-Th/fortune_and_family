//! Strategic, civic, family, and crisis records — the cross-domain state
//! that scheduled strategic systems mutate and invariants validate.
//!
//! Purpose: define durable record shapes for contracts, credit, property,
//! employment, family governance, institutions, law, routes, crises, and
//! outbox, so `src/systems/strategic/*` operates on typed state rather than
//! untyped maps.
//! Owns: every `*Status` / `*Kind` enum, `SupplyContract` / `Loan` /
//! `Property` / `EmploymentAgreement` / `FamilyLink` / `FamilyCouncilState` /
//! `InstitutionRuntime` / `CivicDebt` / `EnactedLaw` / `LegalCase` /
//! `Crisis` / `OutboxMessage` etc. as pure data, plus helpers like
//! `has_consistent_arrears`, `has_consistent_severity`, and `label()`.
//! Reads: nothing (pure definitions).
//! Mutates: nothing directly (systems in `src/systems/strategic/` own
//! mutation; records validate via invariants on load).
//! Does not own: `AppState` container, synchronized indexes, persistence, or
//! business rules.
//! Canonical operations: record construction, `status()`/`label()` queries,
//! `has_consistent_*` validation predicates used by invariants.
//! Relevant invariants: every numeric field is validated at persistence
//! boundaries and by `src/systems/invariants.rs`; `Option` is explicit for
//! optional relationships (no sentinel IDs); lifecycle/status transitions are
//! validated where they mutate.
//! Focused tests: `src/systems/strategic/strategic_tests.rs`, persistence
//! numeric-range checks, and invariant batteries.

use crate::ids::{
    BusinessId, CharacterId, CivicDebtId, ContractId, CrisisId, DistrictId, DynastyId,
    EmploymentId, ExternalRouteId, FamilyLinkId, GoodId, InformationReportId, InstitutionId, LawId,
    LegalCaseId, LoanId, ObjectiveId, OutboxMessageId, PropertyId, PublicWorkId,
};
use crate::money::{Money, Quantity};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractStatus {
    Active,
    Fulfilled,
    Breached,
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
    /// Per-dynasty delivery attribution. Both the buyer's and the seller's
    /// owner are credited for each fulfilled week, so a purchasing house can
    /// qualify for delivery-gated milestones through its commercial
    /// throughput; `has_consistent_delivery_attribution` tolerates that
    /// double-attribution deliberately.
    pub(crate) fulfilled_deliveries_by_dynasty: BTreeMap<DynastyId, u16>,
    pub(crate) missed_deliveries: u16,
    pub(crate) breaching_dynasty_id: Option<DynastyId>,
    pub(crate) breach_victim_dynasty_id: Option<DynastyId>,
    pub(crate) unpaid_breach_penalty: Money,
    /// Penalty cash already collected from attributed misses. Together with
    /// `unpaid_breach_penalty` this bounds total breach exposure to exactly
    /// `penalty`: repeated misses collect only what the contract still owes,
    /// never a fresh penalty per missed delivery.
    pub(crate) collected_breach_penalty: Money,
    pub(crate) status: ContractStatus,
}

impl SupplyContract {
    /// Whether per-dynasty delivery attribution agrees with total fulfillment:
    /// every attributed dynasty delivered at least once and no more than the
    /// fulfilled total, and attribution covers fulfillment within a factor of
    /// two (deliveries may be shared between partner dynasties).
    #[must_use]
    pub fn has_consistent_delivery_attribution(&self) -> bool {
        let attributed_deliveries = self
            .fulfilled_deliveries_by_dynasty
            .values()
            .map(|deliveries| u64::from(*deliveries))
            .sum::<u64>();
        let fulfilled_deliveries = u64::from(self.fulfilled_deliveries);
        self.fulfilled_deliveries_by_dynasty
            .values()
            .all(|deliveries| *deliveries > 0 && *deliveries <= self.fulfilled_deliveries)
            && if fulfilled_deliveries == 0 {
                self.fulfilled_deliveries_by_dynasty.is_empty()
            } else {
                attributed_deliveries >= fulfilled_deliveries
                    && attributed_deliveries <= fulfilled_deliveries * 2
            }
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
    /// Terminal lender loss after final legal enforcement finds no remaining
    /// collectible dynasty assets. Unlike `Repaid`, no money is fabricated:
    /// the unpaid balance is explicitly recognized as a credit loss.
    WrittenOff,
}

impl LoanStatus {
    /// Human-readable label shared by every read model.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Current => "Current",
            Self::Delinquent => "Delinquent",
            Self::Defaulted => "Defaulted",
            Self::Repaid => "Repaid",
            Self::Restructured => "Restructured",
            Self::WrittenOff => "Written off",
        }
    }

    /// Returns whether the loan still participates in scheduled repayment.
    #[must_use]
    pub const fn is_repayment_active(self) -> bool {
        matches!(self, Self::Current | Self::Delinquent | Self::Restructured)
    }

    /// Returns whether no enforceable balance remains on the loan.
    #[must_use]
    pub const fn is_settled(self) -> bool {
        matches!(self, Self::Repaid | Self::WrittenOff)
    }

    /// Returns whether the stored missed-payment counter agrees with this lifecycle state.
    #[must_use]
    pub const fn has_consistent_arrears(self, missed_payments: u16) -> bool {
        match self {
            Self::Current | Self::Restructured | Self::Repaid | Self::WrittenOff => {
                missed_payments == 0
            }
            Self::Delinquent => missed_payments == 1 || missed_payments == 2,
            Self::Defaulted => missed_payments >= 3,
        }
    }
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
    pub const fn status(&self) -> LoanStatus {
        self.status
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropertyKind {
    Residence,
    Workshop,
    Warehouse,
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
    /// The price a neutral district (rent index 10,000) would command for this
    /// property. Monthly revaluation targets `anchor_value * rent_index /
    /// 10_000`, so a one-time change in district desirability reprices the
    /// property once toward a stable level instead of compounding against its
    /// own drifting value every month.
    pub(crate) anchor_value: Money,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FamilyLinkKind {
    Marriage,
    ParentChild,
    Sibling,
    Ward,
}

pub(crate) const MIN_PARENT_CHILD_AGE_GAP_DAYS: i64 = 12 * 360;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyLink {
    pub(crate) id: FamilyLinkId,
    pub(crate) first_character_id: CharacterId,
    pub(crate) second_character_id: CharacterId,
    pub(crate) kind: FamilyLinkKind,
    pub(crate) active: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfficeDirectiveState {
    pub(crate) power: OfficePower,
    pub(crate) expires_day: i64,
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
    pub(crate) active_directive: Option<OfficeDirectiveState>,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnactedLaw {
    pub(crate) id: LawId,
    pub(crate) kind: LawKind,
    pub(crate) enacted_day: i64,
    pub(crate) sponsor_dynasty_id: Option<DynastyId>,
    pub(crate) value: i64,
    pub(crate) active: bool,
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
        if first > second {
            return Err(serde::de::Error::custom(
                "dynasty pair must use ascending first:second order",
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
    Probable,
    Confirmed,
}

impl InformationConfidence {
    /// Human-readable label shared by every read model.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Probable => "Probable",
            Self::Confirmed => "Confirmed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InformationTarget {
    Market { good_id: GoodId },
    Counterparty { dynasty_id: DynastyId },
    District { district_id: DistrictId },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InformationReport {
    pub(crate) id: InformationReportId,
    pub(crate) owner_dynasty_id: DynastyId,
    pub(crate) target: Option<InformationTarget>,
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
    Pursuing,
    Achieved,
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiObjective {
    pub(crate) id: ObjectiveId,
    pub(crate) dynasty_id: DynastyId,
    pub(crate) kind: ObjectiveKind,
    pub(crate) priority: u16,
    pub(crate) created_day: i64,
    pub(crate) status: ObjectiveStatus,
    pub(crate) rationale: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistrictRuntime {
    pub(crate) district_id: DistrictId,
    pub(crate) rent_index_basis_points: u16,
    pub(crate) employment_basis_points: u16,
    pub(crate) sanitation_basis_points: u16,
    pub(crate) safety_basis_points: u16,
    pub(crate) unrest_basis_points: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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

impl PublicWorkStatus {
    /// Returns whether the project still participates in the unfinished-work lifecycle.
    #[must_use]
    pub const fn is_unfinished(self) -> bool {
        matches!(self, Self::Planned | Self::Building | Self::Suspended)
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LegalCaseKind {
    Debt,
    ContractBreach,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegalClaimSource {
    Loan { loan_id: LoanId },
    Contract { contract_id: ContractId },
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
    pub(crate) claim_source: Option<LegalClaimSource>,
    pub(crate) evidence_basis_points: u16,
    pub(crate) public_attention_basis_points: u16,
    pub(crate) filed_day: i64,
    pub(crate) hearing_day: i64,
    pub(crate) damages: Money,
    pub(crate) status: LegalCaseStatus,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CrisisKind {
    GrainShortage,
    BankingPanic,
    UrbanFire,
    GuildRevolt,
    NobleDemand,
    Epidemic,
    TradeDisruption,
}

impl CrisisKind {
    /// Human-readable label shared by every read model.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::GrainShortage => "Grain shortage",
            Self::BankingPanic => "Banking panic",
            Self::UrbanFire => "Urban fire",
            Self::GuildRevolt => "Guild revolt",
            Self::NobleDemand => "Noble demand",
            Self::Epidemic => "Epidemic",
            Self::TradeDisruption => "Trade disruption",
        }
    }
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

    /// Returns whether severity agrees with the persisted crisis lifecycle.
    ///
    /// `Emerging` is the creation state and may carry any still-active severity until the next
    /// monthly crisis update normalizes it to `Active` or `Escalated`.
    #[must_use]
    pub const fn has_consistent_severity(self, severity_basis_points: u16) -> bool {
        match self {
            Self::Emerging => severity_basis_points >= 500,
            Self::Active => severity_basis_points >= 500 && severity_basis_points < 8_000,
            Self::Resolved => severity_basis_points < 500,
            Self::Escalated => severity_basis_points >= 8_000,
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
    /// The simulation day this crisis was first detected.
    ///
    /// Audit records naming this crisis cannot predate this day, so history
    /// scans for crisis responses can skip everything earlier.
    #[must_use]
    pub const fn started_day(&self) -> i64 {
        self.started_day
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
    pub const fn kind(&self) -> OutboxKind {
        self.kind
    }
}
