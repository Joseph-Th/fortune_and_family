//! Concrete runtime records; sibling systems own validation and business logic.

use crate::ids::{
    BusinessId, CharacterId, ChronicleEntryId, DistrictId, DynastyId, GoodId, RecipeId,
};
use crate::money::{Money, Quantity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    BusinessManager,
    GuildRepresentative,
    Clerk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusinessStatus {
    Active,
    Distressed,
    Insolvent,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SocialClass {
    Laboring,
    Artisan,
    Merchant,
    Elite,
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

    #[must_use]
    pub const fn administration(&self) -> u16 {
        self.capabilities.administration
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
        self.inventory
            .insert(good_id, current.saturating_add(quantity));
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
        let remaining = current.saturating_sub(quantity);
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
}

impl MarketState {
    pub fn quotes(&self) -> impl Iterator<Item = &MarketQuote> {
        self.quotes.values()
    }

    #[must_use]
    pub fn get_quote(&self, good_id: GoodId) -> Option<&MarketQuote> {
        self.quotes.get(&good_id)
    }

    #[must_use]
    pub const fn clearing_account(&self) -> Money {
        self.clearing_account
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChronicleKind {
    CampaignFounded,
    PriceShock,
    BusinessDistress,
    BusinessRecovered,
    BusinessAcquired,
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
    OfficeNomination,
    HouseGovernanceChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    pub(crate) day: i64,
    pub(crate) kind: AuditKind,
    pub(crate) subject: String,
    pub(crate) detail: String,
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
        &self.subject
    }

    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}
