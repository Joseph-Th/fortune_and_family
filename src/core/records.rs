//! Core population, economic, and chronicle records owned by `AppState`.
//!
//! Purpose: define the durable shape of characters, dynasties, households,
//! businesses, market quotes, chronicle, and audit events that every other
//! system reads and mutates through `AppState`.
//! Owns: record structs (`Character`, `Dynasty`, `Household`, `Business`,
//! `MarketState`, `ChronicleEntry`, `AuditRecord` …), lifecycle/status
//! enums, `CampaignPhase`, `StartingBackground`, and `AuditSubject` typed
//! subject text.
//! Reads: nothing (pure data definitions).
//! Mutates: only through store methods on `AppState`; systems validate every
//! reference, lifecycle, and numeric bound before mutation.
//! Does not own: state container, synchronized indexes, persistence I/O, or
//! business rules — those live in `state.rs`, `persistence.rs`, and
//! `src/systems/*`.
//! Canonical operations: record construction during bootstrap, field access
//! via typed getters (`character.id()`, `business.cash()`, etc.), inventory
//! `add`/`remove` with checked arithmetic.
//! Relevant invariants: private fields enforce construction through owners;
//! explicit `Option` for optional relations (no sentinel IDs); lifecycle-
//! membership coherence validated by `src/systems/invariants.rs`;
//! `AuditSubject` carries typed segment parsing, not raw string matching.
//! Focused tests: `src/core/state_tests.rs`, persistence round-trip, and
//! invariant batteries; `audit_subject_tests` for typed parsing.

use crate::ids::{
    BusinessId, CharacterId, ChronicleEntryId, DistrictId, DynastyId, GoodId, InstitutionId,
    PropertyId, RecipeId,
};
use crate::money::{Money, Quantity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StartingBackground {
    Baker,
    ClothTrader,
    Blacksmith,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("expected baker, cloth-trader, or blacksmith")]
pub struct ParseStartingBackgroundError;

impl StartingBackground {
    #[must_use]
    pub const fn recipe_key(self) -> &'static str {
        match self {
            Self::Baker => "baking",
            Self::ClothTrader => "weaving",
            Self::Blacksmith => "toolmaking",
        }
    }

    #[must_use]
    pub const fn business_name(self) -> &'static str {
        match self {
            Self::Baker => "Founder's Oven",
            Self::ClothTrader => "Founder's Loomhouse",
            Self::Blacksmith => "Founder's Smithy",
        }
    }
}

impl FromStr for StartingBackground {
    type Err = ParseStartingBackgroundError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "baker" => Ok(Self::Baker),
            "cloth-trader" | "cloth_trader" | "weaver" => Ok(Self::ClothTrader),
            "blacksmith" | "smith" => Ok(Self::Blacksmith),
            _ => Err(ParseStartingBackgroundError),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CampaignPhase {
    Foundation,
    Establishment,
    Ascendancy,
    Dominion,
    Legacy,
}

impl CampaignPhase {
    /// Returns the product-facing name for this stage of the dynasty campaign.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Foundation => "Foundation",
            Self::Establishment => "Establishment",
            Self::Ascendancy => "Institutional ascent",
            Self::Dominion => "Dynastic governance",
            Self::Legacy => "Succession and legacy",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CharacterStatus {
    Active,
    Incapacitated,
    Deceased,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CharacterRole {
    HeadOfHouse,
    Heir,
    Clerk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusinessStatus {
    Active,
    Distressed,
    Insolvent,
    Closed,
}

impl BusinessStatus {
    /// Human-readable label shared by every read model.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Distressed => "Distressed",
            Self::Insolvent => "Insolvent",
            Self::Closed => "Closed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SocialClass {
    Laboring,
    Artisan,
    Merchant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterIdentity {
    pub(crate) id: CharacterId,
    pub(crate) dynasty_id: DynastyId,
    pub(crate) name: String,
    pub(crate) birth_day: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterCapabilities {
    pub(crate) administration: u16,
    pub(crate) commerce: u16,
    pub(crate) social: u16,
    pub(crate) craft: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterRuntime {
    pub(crate) status: CharacterStatus,
    pub(crate) health_basis_points: u16,
    pub(crate) loyalty_basis_points: u16,
    pub(crate) role: CharacterRole,
    /// The day a character's health collapsed into incapacitation. A
    /// deterministic death window runs from here; `None` while active.
    pub(crate) incapacitated_day: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Character {
    pub(crate) identity: CharacterIdentity,
    pub(crate) capabilities: CharacterCapabilities,
    pub(crate) runtime: CharacterRuntime,
}

impl Character {
    #[must_use]
    pub const fn id(&self) -> CharacterId {
        self.identity.id
    }

    #[must_use]
    pub const fn dynasty_id(&self) -> DynastyId {
        self.identity.dynasty_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.identity.name
    }

    #[must_use]
    pub const fn birth_day(&self) -> i64 {
        self.identity.birth_day
    }

    #[must_use]
    pub const fn status(&self) -> CharacterStatus {
        self.runtime.status
    }

    #[must_use]
    pub const fn role(&self) -> CharacterRole {
        self.runtime.role
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynastyIdentity {
    pub(crate) id: DynastyId,
    pub(crate) name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynastyResources {
    pub(crate) treasury: Money,
    pub(crate) civic_contributions: Money,
    pub(crate) unmet_office_duties: u32,
    pub(crate) legitimacy_basis_points: u16,
    pub(crate) administrative_capacity: u16,
    pub(crate) administrative_load: u16,
    pub(crate) reputation_quality_basis_points: u16,
    pub(crate) reputation_reliability_basis_points: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynastyRelationships {
    pub(crate) head_id: CharacterId,
    pub(crate) heir_id: Option<CharacterId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynastyRuntime {
    pub(crate) phase: CampaignPhase,
    pub(crate) generation: u16,
    pub(crate) succession_risk_basis_points: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dynasty {
    pub(crate) identity: DynastyIdentity,
    pub(crate) resources: DynastyResources,
    pub(crate) relationships: DynastyRelationships,
    pub(crate) runtime: DynastyRuntime,
}

impl Dynasty {
    #[must_use]
    pub const fn id(&self) -> DynastyId {
        self.identity.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.identity.name
    }

    #[must_use]
    pub const fn head_id(&self) -> CharacterId {
        self.relationships.head_id
    }

    #[must_use]
    pub const fn heir_id(&self) -> Option<CharacterId> {
        self.relationships.heir_id
    }

    #[must_use]
    pub const fn treasury(&self) -> Money {
        self.resources.treasury
    }

    #[must_use]
    pub const fn civic_contributions(&self) -> Money {
        self.resources.civic_contributions
    }

    #[must_use]
    pub const fn unmet_office_duties(&self) -> u32 {
        self.resources.unmet_office_duties
    }

    #[must_use]
    pub const fn phase(&self) -> CampaignPhase {
        self.runtime.phase
    }

    #[must_use]
    pub const fn administrative_capacity(&self) -> u16 {
        self.resources.administrative_capacity
    }

    #[must_use]
    pub const fn administrative_load(&self) -> u16 {
        self.resources.administrative_load
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Household {
    pub(crate) id: crate::ids::HouseholdId,
    pub(crate) district_id: DistrictId,
    pub(crate) members: u16,
    pub(crate) social_class: SocialClass,
    pub(crate) cash: Money,
    pub(crate) weekly_income: Money,
    pub(crate) bread_need_daily: Quantity,
    pub(crate) ale_need_daily: Quantity,
    pub(crate) food_satisfaction_basis_points: u16,
}

impl Household {
    #[must_use]
    pub const fn id(&self) -> crate::ids::HouseholdId {
        self.id
    }

    #[must_use]
    pub const fn district_id(&self) -> DistrictId {
        self.district_id
    }

    #[must_use]
    pub const fn members(&self) -> u16 {
        self.members
    }

    #[must_use]
    pub const fn social_class(&self) -> SocialClass {
        self.social_class
    }

    #[must_use]
    pub const fn cash(&self) -> Money {
        self.cash
    }

    #[must_use]
    pub const fn food_satisfaction_basis_points(&self) -> u16 {
        self.food_satisfaction_basis_points
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessPolicy {
    pub(crate) target_input_days: u16,
    pub(crate) target_output_days: u16,
    pub(crate) minimum_cash_reserve: Money,
    pub(crate) maintenance_basis_points: u16,
    pub(crate) quality_target_basis_points: u16,
}

impl Default for BusinessPolicy {
    fn default() -> Self {
        Self {
            target_input_days: 3,
            target_output_days: 2,
            minimum_cash_reserve: Money::from_copper(500),
            maintenance_basis_points: 1_200,
            quality_target_basis_points: 7_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessIdentity {
    pub(crate) id: BusinessId,
    pub(crate) name: String,
    pub(crate) owner_dynasty_id: DynastyId,
    pub(crate) district_id: DistrictId,
    pub(crate) recipe_id: RecipeId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessOperations {
    pub(crate) manager_id: CharacterId,
    pub(crate) capacity_batches_per_day: u16,
    pub(crate) condition_basis_points: u16,
    pub(crate) quality_basis_points: u16,
    pub(crate) status: BusinessStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessFinance {
    pub(crate) cash: Money,
    pub(crate) version: u64,
    /// Cumulative value of goods placed on the market for sale. Sales settle
    /// against the market's clearing pool when goods are listed rather than
    /// when consumers buy them, so this measures placement volume, not
    /// consumer purchases (unsold stock spoils later without reversing it).
    pub(crate) lifetime_revenue: Money,
    pub(crate) lifetime_costs: Money,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Business {
    pub(crate) identity: BusinessIdentity,
    pub(crate) operations: BusinessOperations,
    pub(crate) finance: BusinessFinance,
    pub(crate) inventory: BTreeMap<GoodId, Quantity>,
    pub(crate) policy: BusinessPolicy,
    /// The workshop premises this business occupies when it operates. Kept on
    /// the business so an eviction during insolvency can be undone by
    /// re-occupancy once the firm trades again, instead of stranding a
    /// purpose-built premises as a vacancy-income windfall for its owner.
    pub(crate) premises_property_id: Option<PropertyId>,
}

impl Business {
    #[must_use]
    pub const fn id(&self) -> BusinessId {
        self.identity.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.identity.name
    }

    #[must_use]
    pub const fn premises_property_id(&self) -> Option<PropertyId> {
        self.premises_property_id
    }

    #[must_use]
    pub const fn owner_dynasty_id(&self) -> DynastyId {
        self.identity.owner_dynasty_id
    }

    #[must_use]
    pub const fn district_id(&self) -> DistrictId {
        self.identity.district_id
    }

    #[must_use]
    pub const fn recipe_id(&self) -> RecipeId {
        self.identity.recipe_id
    }

    #[must_use]
    pub const fn manager_id(&self) -> CharacterId {
        self.operations.manager_id
    }

    #[must_use]
    pub const fn status(&self) -> BusinessStatus {
        self.operations.status
    }

    #[must_use]
    pub const fn cash(&self) -> Money {
        self.finance.cash
    }

    #[must_use]
    pub const fn condition_basis_points(&self) -> u16 {
        self.operations.condition_basis_points
    }

    #[must_use]
    pub fn inventory(&self) -> &BTreeMap<GoodId, Quantity> {
        &self.inventory
    }

    #[must_use]
    pub fn inventory_quantity(&self, good_id: GoodId) -> Quantity {
        self.inventory
            .get(&good_id)
            .copied()
            .unwrap_or(Quantity::ZERO)
    }

    pub(crate) fn add_inventory(&mut self, good_id: GoodId, quantity: Quantity) {
        assert!(
            !quantity.is_negative(),
            "inventory additions must not be negative"
        );
        if quantity.is_zero() {
            return;
        }
        let current = self.inventory_quantity(good_id);
        let updated = current
            .checked_add(quantity)
            .expect("inventory additions must fit the supported quantity range");
        self.inventory.insert(good_id, updated);
    }

    pub(crate) fn remove_inventory(&mut self, good_id: GoodId, quantity: Quantity) {
        assert!(
            !quantity.is_negative(),
            "inventory removals must not be negative"
        );
        if quantity.is_zero() {
            return;
        }
        let current = self.inventory_quantity(good_id);
        assert!(
            current >= quantity,
            "business inventory underflow for good {good_id}"
        );
        let remaining = current
            .checked_sub(quantity)
            .expect("validated inventory removal must not underflow");
        if remaining.is_zero() {
            self.inventory.remove(&good_id);
        } else {
            self.inventory.insert(good_id, remaining);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketCause {
    StockBelowTarget,
    StockAboveTarget,
    DemandExceededSupply,
    SupplyExceededDemand,
    SeasonalPressure,
    StableConditions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketQuote {
    pub(crate) good_id: GoodId,
    pub(crate) price: Money,
    pub(crate) previous_price: Money,
    pub(crate) stock: Quantity,
    pub(crate) target_stock: Quantity,
    pub(crate) demand_today: Quantity,
    pub(crate) supply_today: Quantity,
    pub(crate) causes: Vec<MarketCause>,
}

impl MarketQuote {
    #[must_use]
    pub const fn good_id(&self) -> GoodId {
        self.good_id
    }

    #[must_use]
    pub const fn price(&self) -> Money {
        self.price
    }

    #[must_use]
    pub const fn previous_price(&self) -> Money {
        self.previous_price
    }

    #[must_use]
    pub const fn stock(&self) -> Quantity {
        self.stock
    }

    #[must_use]
    pub fn causes(&self) -> &[MarketCause] {
        &self.causes
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketState {
    pub(crate) quotes: BTreeMap<GoodId, MarketQuote>,
    pub(crate) clearing_account: Money,
    /// Price of each good at the last monthly boundary. The monthly market
    /// report measures movement against this reference, so it describes the
    /// whole month instead of the most recent day-over-day tick.
    pub(crate) month_start_prices: BTreeMap<GoodId, Money>,
}

impl MarketState {
    #[must_use]
    pub fn get_quote(&self, good_id: GoodId) -> Option<&MarketQuote> {
        self.quotes.get(&good_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChronicleKind {
    CampaignFounded,
    PriceShock,
    BusinessDistress,
    BusinessRecovered,
    BusinessAcquired,
    OfficeDirective,
    FamilyExpanded,
    SuccessionPrepared,
    NewYear,
    Succession,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChronicleEntry {
    pub(crate) id: ChronicleEntryId,
    pub(crate) day: i64,
    pub(crate) kind: ChronicleKind,
    pub(crate) summary: String,
}

impl ChronicleEntry {
    #[must_use]
    pub const fn id(&self) -> ChronicleEntryId {
        self.id
    }

    #[must_use]
    pub const fn day(&self) -> i64 {
        self.day
    }

    #[must_use]
    pub const fn kind(&self) -> ChronicleKind {
        self.kind
    }

    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditKind {
    CampaignCreated,
    MarketPurchase,
    MarketSale,
    Production,
    LaborSettlement,
    HouseholdConsumption,
    Maintenance,
    DayAdvanced,
    CashTransfer,
    BusinessCapitalization,
    BusinessPolicyChange,
    BusinessAcquisition,
    BusinessDividend,
    PublicWorkStarted,
    CrisisResponse,
    InstitutionPatronage,
    InstitutionEndowment,
    InstitutionWithdrawal,
    OfficeNomination,
    OfficeDutyShortfall,
    OfficeDutyForfeiture,
    OfficeDirective,
    HouseGovernanceChange,
    FamilyCouncilMeeting,
    HeirDesignation,
    WardAdoption,
    FamilyEducation,
    InformationCommission,
    InformationLeverage,
    HouseholdUpkeep,
    BusinessWageChange,
}

/// Serialized audit subject text wrapped as a domain type so runtime identity is not represented
/// by an unclassified `String` field. The transparent representation keeps the serialized shape
/// compact while preserving a typed runtime boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuditSubject(Arc<str>);

impl AuditSubject {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn references_dynasty(&self, dynasty_id: DynastyId) -> bool {
        self.0.split(';').any(|segment| {
            segment
                .strip_prefix("dynasty:")
                .and_then(|value| value.parse::<u32>().ok())
                == Some(dynasty_id.value())
        })
    }

    #[must_use]
    pub fn dynasty_id(&self) -> Option<DynastyId> {
        let mut dynasty_id = None;
        for segment in self.0.split(';') {
            let Some(value) = segment.strip_prefix("dynasty:") else {
                continue;
            };
            let parsed = value.parse::<u32>().ok().map(DynastyId::new)?;
            if dynasty_id.replace(parsed).is_some() {
                return None;
            }
        }
        dynasty_id
    }

    #[must_use]
    pub fn references_institution_character(
        &self,
        institution_id: InstitutionId,
        character_id: CharacterId,
    ) -> bool {
        self.institution_character_ids() == Some((institution_id, character_id))
    }

    #[must_use]
    pub fn institution_id(&self) -> Option<InstitutionId> {
        let mut institution_id = None;
        for segment in self.0.split(';') {
            let Some(value) = segment.strip_prefix("institution:") else {
                continue;
            };
            let parsed = value.parse::<u32>().ok().map(InstitutionId::new)?;
            if institution_id.replace(parsed).is_some() {
                return None;
            }
        }
        institution_id
    }

    #[must_use]
    pub fn institution_character_ids(&self) -> Option<(InstitutionId, CharacterId)> {
        let mut segments = self.0.split(':');
        if segments.next() != Some("institution") {
            return None;
        }
        let institution_id = segments
            .next()?
            .parse::<u32>()
            .ok()
            .map(InstitutionId::new)?;
        if segments.next() != Some("character") {
            return None;
        }
        let character_id = segments.next()?.parse::<u32>().ok().map(CharacterId::new)?;
        segments
            .next()
            .is_none()
            .then_some((institution_id, character_id))
    }
}

impl From<String> for AuditSubject {
    fn from(value: String) -> Self {
        Self(Arc::from(value))
    }
}

impl From<&str> for AuditSubject {
    fn from(value: &str) -> Self {
        Self(Arc::from(value))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    pub(crate) day: i64,
    pub(crate) kind: AuditKind,
    pub(crate) subject: AuditSubject,
    /// Shared, immutable after construction: the audit log is append-only and
    /// every simulation working copy clones it, so cheap reference-counted
    /// text keeps campaign-length history from dominating clone cost. The
    /// serialized shape is a plain string either way.
    pub(crate) detail: Arc<str>,
}

impl AuditRecord {
    #[must_use]
    pub const fn day(&self) -> i64 {
        self.day
    }

    #[must_use]
    pub const fn kind(&self) -> AuditKind {
        self.kind
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        self.subject.as_str()
    }

    #[must_use]
    pub const fn audit_subject(&self) -> &AuditSubject {
        &self.subject
    }

    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

#[cfg(test)]
mod audit_subject_tests {
    use super::*;

    #[test]
    fn transparent_serialization_uses_string_shape() {
        let subject = AuditSubject::from("institution:3;dynasty:10");

        let value = serde_json::to_value(&subject).expect("audit subject must serialize");
        let restored: AuditSubject =
            serde_json::from_value(value.clone()).expect("audit subject must deserialize");

        assert_eq!(
            value,
            serde_json::Value::String(subject.as_str().to_owned())
        );
        assert_eq!(restored, subject);
    }

    #[test]
    fn dynasty_matching_uses_exact_typed_segments() {
        let subject = AuditSubject::from("institution:3;dynasty:10");

        assert!(subject.references_dynasty(DynastyId::new(10)));
        assert!(!subject.references_dynasty(DynastyId::new(1)));
        assert_eq!(subject.dynasty_id(), Some(DynastyId::new(10)));
        assert_eq!(AuditSubject::from("dynasty:10:extra").dynasty_id(), None);
        assert_eq!(
            AuditSubject::from("dynasty:10;dynasty:11").dynasty_id(),
            None
        );
    }

    #[test]
    fn institution_character_matching_requires_the_complete_typed_shape() {
        let subject = AuditSubject::from("institution:3:character:10");

        assert!(
            subject.references_institution_character(InstitutionId::new(3), CharacterId::new(10))
        );
        assert!(
            !subject.references_institution_character(InstitutionId::new(3), CharacterId::new(1))
        );
        assert!(
            !AuditSubject::from("institution:3:character:10:extra")
                .references_institution_character(InstitutionId::new(3), CharacterId::new(10))
        );
        assert_eq!(
            subject.institution_character_ids(),
            Some((InstitutionId::new(3), CharacterId::new(10)))
        );
        assert_eq!(
            AuditSubject::from("other:3:character:10").institution_character_ids(),
            None
        );
    }

    #[test]
    fn institution_matching_requires_the_complete_typed_shape() {
        assert_eq!(
            AuditSubject::from("institution:3").institution_id(),
            Some(InstitutionId::new(3))
        );
        assert_eq!(
            AuditSubject::from("institution:3;dynasty:10").institution_id(),
            Some(InstitutionId::new(3))
        );
        assert_eq!(
            AuditSubject::from("institution:3:extra").institution_id(),
            None
        );
        assert_eq!(
            AuditSubject::from("institution:3;institution:4").institution_id(),
            None
        );
        assert_eq!(AuditSubject::from("other:3").institution_id(), None);
    }
}
