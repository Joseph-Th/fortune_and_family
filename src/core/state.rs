//! Application-state root, ID allocation, synchronized stores, and read-only summaries.

use super::{
    AiObjective, AuditRecord, Business, BusinessStatus, CampaignPhase, Character, ChronicleEntry,
    Crisis, DistrictRuntime, Dynasty, DynastyPair, EmploymentAgreement, EnactedLaw, ExternalRoute,
    FamilyCouncilState, FamilyLink, Household, InformationReport, InstitutionRuntime,
    InstitutionState, LegalCase, Loan, MarketState, OutboxMessage, Property, PublicWork,
    RelationshipState, StartingBackground, SupplyContract,
};
use crate::ids::{
    BusinessId, CharacterId, ChronicleEntryId, ContractId, CrisisId, DistrictId, DynastyId,
    EmploymentId, ExternalRouteId, FamilyLinkId, HouseholdId, InformationReportId, LawId,
    LegalCaseId, LoanId, ObjectiveId, OutboxMessageId, PropertyId, PublicWorkId,
};
use crate::money::Money;
use crate::registry::Registry;
use crate::rng::DeterministicRng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CURRENT_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewGameConfig {
    pub seed: u64,
    pub dynasty_name: String,
    pub founder_name: String,
    pub background: StartingBackground,
}

impl Default for NewGameConfig {
    fn default() -> Self {
        Self {
            seed: 1,
            dynasty_name: "Valeri".to_owned(),
            founder_name: "Elian Valeri".to_owned(),
            background: StartingBackground::Baker,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationClock {
    day: i64,
}

impl SimulationClock {
    #[must_use]
    pub const fn new() -> Self {
        Self { day: 0 }
    }

    #[must_use]
    pub const fn day(self) -> i64 {
        self.day
    }

    #[must_use]
    pub fn year(self, start_year: i32) -> i32 {
        let elapsed_years = self.day / 360;
        let converted = i32::try_from(elapsed_years).unwrap_or(if elapsed_years.is_negative() {
            i32::MIN
        } else {
            i32::MAX
        });
        start_year.saturating_add(converted)
    }

    #[must_use]
    pub fn day_of_year(self) -> u16 {
        u16::try_from(self.day.rem_euclid(360) + 1).unwrap_or(1)
    }

    #[must_use]
    pub const fn is_week_boundary(self) -> bool {
        self.day > 0 && self.day % 7 == 0
    }

    #[must_use]
    pub const fn is_year_boundary(self) -> bool {
        self.day > 0 && self.day % 360 == 0
    }

    pub(crate) const fn advance_one_day(&mut self) {
        self.day = self
            .day
            .checked_add(1)
            .expect("simulation clock exhausted its supported day range");
    }
}

impl Default for SimulationClock {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterStore {
    records: BTreeMap<CharacterId, Character>,
    by_dynasty: BTreeMap<DynastyId, BTreeSet<CharacterId>>,
}

impl CharacterStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: BTreeMap::new(),
            by_dynasty: BTreeMap::new(),
        }
    }

    pub(crate) fn insert(&mut self, character: Character) {
        let character_id = character.id();
        let dynasty_id = character.dynasty_id();
        assert!(
            self.records.insert(character_id, character).is_none(),
            "duplicate character ID {character_id}"
        );
        let inserted = self
            .by_dynasty
            .entry(dynasty_id)
            .or_default()
            .insert(character_id);
        assert!(inserted, "duplicate character index entry {character_id}");
    }

    #[must_use]
    pub fn get(&self, id: CharacterId) -> Option<&Character> {
        self.records.get(&id)
    }

    pub(crate) fn get_mut(&mut self, id: CharacterId) -> Option<&mut Character> {
        self.records.get_mut(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Character> {
        self.records.values()
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut Character> {
        self.records.values_mut()
    }

    #[must_use]
    pub fn ids_for_dynasty(&self, dynasty_id: DynastyId) -> Option<&BTreeSet<CharacterId>> {
        self.by_dynasty.get(&dynasty_id)
    }

    #[must_use]
    pub(crate) const fn records(&self) -> &BTreeMap<CharacterId, Character> {
        &self.records
    }

    #[must_use]
    pub(crate) const fn index(&self) -> &BTreeMap<DynastyId, BTreeSet<CharacterId>> {
        &self.by_dynasty
    }
}

impl Default for CharacterStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessStore {
    records: BTreeMap<BusinessId, Business>,
    by_owner: BTreeMap<DynastyId, BTreeSet<BusinessId>>,
    by_district: BTreeMap<DistrictId, BTreeSet<BusinessId>>,
}

impl BusinessStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: BTreeMap::new(),
            by_owner: BTreeMap::new(),
            by_district: BTreeMap::new(),
        }
    }

    pub(crate) fn insert(&mut self, business: Business) {
        let business_id = business.id();
        let owner_id = business.owner_dynasty_id();
        let district_id = business.district_id();
        assert!(
            self.records.insert(business_id, business).is_none(),
            "duplicate business ID {business_id}"
        );
        assert!(
            self.by_owner
                .entry(owner_id)
                .or_default()
                .insert(business_id),
            "duplicate owner index entry {business_id}"
        );
        assert!(
            self.by_district
                .entry(district_id)
                .or_default()
                .insert(business_id),
            "duplicate district index entry {business_id}"
        );
    }

    #[must_use]
    pub fn get(&self, id: BusinessId) -> Option<&Business> {
        self.records.get(&id)
    }

    pub(crate) fn get_mut(&mut self, id: BusinessId) -> Option<&mut Business> {
        self.records.get_mut(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Business> {
        self.records.values()
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut Business> {
        self.records.values_mut()
    }

    #[must_use]
    pub fn ids_for_owner(&self, owner_id: DynastyId) -> Option<&BTreeSet<BusinessId>> {
        self.by_owner.get(&owner_id)
    }

    #[must_use]
    pub fn ids_for_district(&self, district_id: DistrictId) -> Option<&BTreeSet<BusinessId>> {
        self.by_district.get(&district_id)
    }

    #[must_use]
    pub(crate) const fn records(&self) -> &BTreeMap<BusinessId, Business> {
        &self.records
    }

    #[must_use]
    pub(crate) const fn owner_index(&self) -> &BTreeMap<DynastyId, BTreeSet<BusinessId>> {
        &self.by_owner
    }

    #[must_use]
    pub(crate) const fn district_index(&self) -> &BTreeMap<DistrictId, BTreeSet<BusinessId>> {
        &self.by_district
    }
}

impl Default for BusinessStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HouseholdStore {
    records: BTreeMap<HouseholdId, Household>,
    by_district: BTreeMap<DistrictId, BTreeSet<HouseholdId>>,
}

impl HouseholdStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: BTreeMap::new(),
            by_district: BTreeMap::new(),
        }
    }

    pub(crate) fn insert(&mut self, household: Household) {
        let household_id = household.id();
        let district_id = household.district_id();
        assert!(
            self.records.insert(household_id, household).is_none(),
            "duplicate household ID {household_id}"
        );
        assert!(
            self.by_district
                .entry(district_id)
                .or_default()
                .insert(household_id),
            "duplicate household district index entry {household_id}"
        );
    }

    #[must_use]
    pub fn get(&self, id: HouseholdId) -> Option<&Household> {
        self.records.get(&id)
    }

    pub(crate) fn get_mut(&mut self, id: HouseholdId) -> Option<&mut Household> {
        self.records.get_mut(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Household> {
        self.records.values()
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut Household> {
        self.records.values_mut()
    }

    #[must_use]
    pub fn ids_for_district(&self, district_id: DistrictId) -> Option<&BTreeSet<HouseholdId>> {
        self.by_district.get(&district_id)
    }

    #[must_use]
    pub(crate) const fn records(&self) -> &BTreeMap<HouseholdId, Household> {
        &self.records
    }

    #[must_use]
    pub(crate) const fn index(&self) -> &BTreeMap<DistrictId, BTreeSet<HouseholdId>> {
        &self.by_district
    }
}

impl Default for HouseholdStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NextIds {
    dynasty: u32,
    character: u32,
    household: u32,
    business: u32,
    contract: u32,
    property: u32,
    loan: u32,
    employment: u32,
    family_link: u32,
    law: u32,
    information_report: u32,
    objective: u32,
    public_work: u32,
    legal_case: u32,
    external_route: u32,
    crisis: u32,
    outbox: u32,
    chronicle: u32,
}

macro_rules! next_id_method {
    ($method:ident, $field:ident, $id_type:ident) => {
        pub(crate) const fn $method(&mut self) -> $id_type {
            let id = $id_type::new(self.$field);
            self.$field = self
                .$field
                .checked_add(1)
                .expect(concat!(stringify!($id_type), " identifier space exhausted"));
            id
        }
    };
}

impl NextIds {
    pub(crate) const fn new() -> Self {
        Self {
            dynasty: 0,
            character: 0,
            household: 0,
            business: 0,
            contract: 0,
            property: 0,
            loan: 0,
            employment: 0,
            family_link: 0,
            law: 0,
            information_report: 0,
            objective: 0,
            public_work: 0,
            legal_case: 0,
            external_route: 0,
            crisis: 0,
            outbox: 0,
            chronicle: 0,
        }
    }

    next_id_method!(dynasty, dynasty, DynastyId);
    next_id_method!(character, character, CharacterId);
    next_id_method!(household, household, HouseholdId);
    next_id_method!(business, business, BusinessId);
    next_id_method!(contract, contract, ContractId);
    next_id_method!(property, property, PropertyId);
    next_id_method!(loan, loan, LoanId);
    next_id_method!(employment, employment, EmploymentId);
    next_id_method!(family_link, family_link, FamilyLinkId);
    next_id_method!(law, law, LawId);
    next_id_method!(information_report, information_report, InformationReportId);
    next_id_method!(objective, objective, ObjectiveId);
    next_id_method!(public_work, public_work, PublicWorkId);
    next_id_method!(legal_case, legal_case, LegalCaseId);
    next_id_method!(external_route, external_route, ExternalRouteId);
    next_id_method!(crisis, crisis, CrisisId);
    next_id_method!(outbox, outbox, OutboxMessageId);
    next_id_method!(chronicle, chronicle, ChronicleEntryId);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppState {
    pub(crate) schema_version: u32,
    pub(crate) scenario_key: String,
    pub(crate) clock: SimulationClock,
    pub(crate) rng: DeterministicRng,
    pub(crate) next_ids: NextIds,
    pub(crate) player_dynasty_id: DynastyId,
    pub(crate) dynasties: BTreeMap<DynastyId, Dynasty>,
    pub(crate) characters: CharacterStore,
    pub(crate) households: HouseholdStore,
    pub(crate) businesses: BusinessStore,
    pub(crate) institutions: BTreeMap<crate::ids::InstitutionId, InstitutionState>,
    pub(crate) institution_runtime: BTreeMap<crate::ids::InstitutionId, InstitutionRuntime>,
    pub(crate) market: MarketState,
    pub(crate) contracts: BTreeMap<ContractId, SupplyContract>,
    pub(crate) loans: BTreeMap<LoanId, Loan>,
    pub(crate) properties: BTreeMap<PropertyId, Property>,
    pub(crate) employment: BTreeMap<EmploymentId, EmploymentAgreement>,
    pub(crate) family_links: BTreeMap<FamilyLinkId, FamilyLink>,
    pub(crate) family_councils: BTreeMap<DynastyId, FamilyCouncilState>,
    pub(crate) laws: BTreeMap<LawId, EnactedLaw>,
    pub(crate) relationships: BTreeMap<DynastyPair, RelationshipState>,
    pub(crate) information_reports: BTreeMap<InformationReportId, InformationReport>,
    pub(crate) ai_objectives: BTreeMap<ObjectiveId, AiObjective>,
    pub(crate) districts: BTreeMap<DistrictId, DistrictRuntime>,
    pub(crate) public_works: BTreeMap<PublicWorkId, PublicWork>,
    pub(crate) legal_cases: BTreeMap<LegalCaseId, LegalCase>,
    pub(crate) external_routes: BTreeMap<ExternalRouteId, ExternalRoute>,
    pub(crate) crises: BTreeMap<CrisisId, Crisis>,
    pub(crate) outbox: Vec<OutboxMessage>,
    pub(crate) chronicle: Vec<ChronicleEntry>,
    pub(crate) audit_log: Vec<AuditRecord>,
}

impl AppState {
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub fn scenario_key(&self) -> &str {
        &self.scenario_key
    }

    #[must_use]
    pub const fn clock(&self) -> SimulationClock {
        self.clock
    }

    #[must_use]
    pub const fn player_dynasty_id(&self) -> DynastyId {
        self.player_dynasty_id
    }

    pub fn dynasties(&self) -> impl Iterator<Item = &Dynasty> {
        self.dynasties.values()
    }

    #[must_use]
    pub fn get_dynasty(&self, id: DynastyId) -> Option<&Dynasty> {
        self.dynasties.get(&id)
    }

    #[must_use]
    pub const fn characters(&self) -> &CharacterStore {
        &self.characters
    }

    #[must_use]
    pub const fn households(&self) -> &HouseholdStore {
        &self.households
    }

    #[must_use]
    pub const fn businesses(&self) -> &BusinessStore {
        &self.businesses
    }

    #[must_use]
    pub const fn market(&self) -> &MarketState {
        &self.market
    }

    pub fn institutions(&self) -> impl Iterator<Item = &InstitutionState> {
        self.institutions.values()
    }

    pub fn contracts(&self) -> impl Iterator<Item = &SupplyContract> {
        self.contracts.values()
    }

    pub fn loans(&self) -> impl Iterator<Item = &Loan> {
        self.loans.values()
    }

    pub fn properties(&self) -> impl Iterator<Item = &Property> {
        self.properties.values()
    }

    pub fn employment(&self) -> impl Iterator<Item = &EmploymentAgreement> {
        self.employment.values()
    }

    pub fn laws(&self) -> impl Iterator<Item = &EnactedLaw> {
        self.laws.values()
    }

    pub fn crises(&self) -> impl Iterator<Item = &Crisis> {
        self.crises.values()
    }

    #[must_use]
    pub fn outbox(&self) -> &[OutboxMessage] {
        &self.outbox
    }

    #[must_use]
    pub fn chronicle(&self) -> &[ChronicleEntry] {
        &self.chronicle
    }

    #[must_use]
    pub fn audit_log(&self) -> &[AuditRecord] {
        &self.audit_log
    }

    pub(crate) fn validate_next_ids(&self) -> Result<(), String> {
        macro_rules! require_next_id {
            ($field:ident, $label:literal, $ids:expr) => {
                if let Some(maximum) = ($ids).map(|id| id.value()).max()
                    && self.next_ids.$field <= maximum
                {
                    return Err(format!(
                        "next {} ID {} does not exceed existing maximum {}",
                        $label, self.next_ids.$field, maximum
                    ));
                }
            };
        }

        require_next_id!(dynasty, "dynasty", self.dynasties.keys().copied());
        require_next_id!(
            character,
            "character",
            self.characters.records().keys().copied()
        );
        require_next_id!(
            household,
            "household",
            self.households.records().keys().copied()
        );
        require_next_id!(
            business,
            "business",
            self.businesses.records().keys().copied()
        );
        require_next_id!(contract, "contract", self.contracts.keys().copied());
        require_next_id!(property, "property", self.properties.keys().copied());
        require_next_id!(loan, "loan", self.loans.keys().copied());
        require_next_id!(employment, "employment", self.employment.keys().copied());
        require_next_id!(
            family_link,
            "family link",
            self.family_links.keys().copied()
        );
        require_next_id!(law, "law", self.laws.keys().copied());
        require_next_id!(
            information_report,
            "information report",
            self.information_reports.keys().copied()
        );
        require_next_id!(objective, "objective", self.ai_objectives.keys().copied());
        require_next_id!(
            public_work,
            "public work",
            self.public_works.keys().copied()
        );
        require_next_id!(legal_case, "legal case", self.legal_cases.keys().copied());
        require_next_id!(
            external_route,
            "external route",
            self.external_routes.keys().copied()
        );
        require_next_id!(crisis, "crisis", self.crises.keys().copied());
        require_next_id!(
            outbox,
            "outbox message",
            self.outbox.iter().map(|item| item.id)
        );
        require_next_id!(
            chronicle,
            "chronicle entry",
            self.chronicle.iter().map(ChronicleEntry::id)
        );
        Ok(())
    }

    /// Builds a compact read-only projection for user-interface adapters.
    ///
    /// # Panics
    ///
    /// Panics when the player dynasty reference or a derived numeric invariant is corrupt.
    #[must_use]
    pub fn summary(&self, registry: &Registry) -> StateSummary {
        let dynasty = self
            .dynasties
            .get(&self.player_dynasty_id)
            .expect("player dynasty must exist");
        let business_ids = self
            .businesses
            .ids_for_owner(self.player_dynasty_id)
            .map_or(0, BTreeSet::len);
        let active_businesses = self
            .businesses
            .ids_for_owner(self.player_dynasty_id)
            .into_iter()
            .flatten()
            .filter_map(|id| self.businesses.get(*id))
            .filter(|business| business.status() == BusinessStatus::Active)
            .count();
        let business_cash = self
            .businesses
            .ids_for_owner(self.player_dynasty_id)
            .into_iter()
            .flatten()
            .filter_map(|id| self.businesses.get(*id))
            .fold(Money::ZERO, |total, business| {
                total.saturating_add(business.cash())
            });
        let average_food_satisfaction_basis_points = if self.households.records().is_empty() {
            0
        } else {
            let total: u64 = self
                .households
                .iter()
                .map(|household| u64::from(household.food_satisfaction_basis_points()))
                .sum();
            u16::try_from(total / self.households.records().len() as u64)
                .expect("average satisfaction must fit u16")
        };

        StateSummary {
            scenario_name: registry.scenario().name().to_owned(),
            year: self.clock.year(registry.scenario().start_year()),
            day_of_year: self.clock.day_of_year(),
            elapsed_days: self.clock.day(),
            dynasty_name: dynasty.name().to_owned(),
            phase: dynasty.phase(),
            dynasty_treasury: dynasty.treasury(),
            business_cash,
            businesses: business_ids,
            active_businesses,
            population_groups: self.households.records().len(),
            average_food_satisfaction_basis_points,
            chronicle_entries: self.chronicle.len(),
            active_contracts: self
                .contracts
                .values()
                .filter(|contract| contract.status() == super::ContractStatus::Active)
                .count(),
            current_loans: self
                .loans
                .values()
                .filter(|loan| {
                    matches!(
                        loan.status(),
                        super::LoanStatus::Current
                            | super::LoanStatus::Delinquent
                            | super::LoanStatus::Restructured
                    )
                })
                .count(),
            properties: self.properties.len(),
            active_crises: self
                .crises
                .values()
                .filter(|crisis| {
                    matches!(
                        crisis.status,
                        super::CrisisStatus::Emerging | super::CrisisStatus::Active
                    )
                })
                .count(),
            unread_notifications: self
                .outbox
                .iter()
                .filter(|message| !message.acknowledged)
                .count(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSummary {
    pub scenario_name: String,
    pub year: i32,
    pub day_of_year: u16,
    pub elapsed_days: i64,
    pub dynasty_name: String,
    pub phase: CampaignPhase,
    pub dynasty_treasury: Money,
    pub business_cash: Money,
    pub businesses: usize,
    pub active_businesses: usize,
    pub population_groups: usize,
    pub average_food_satisfaction_basis_points: u16,
    pub chronicle_entries: usize,
    pub active_contracts: usize,
    pub current_loans: usize,
    pub properties: usize,
    pub active_crises: usize,
    pub unread_notifications: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::systems::{advance_days, validate_invariants};
    use crate::test_support::{
        assert_state_eq, make_test_campaign, make_test_campaign_with, rivergate_registry_for_test,
    };

    #[test]
    fn same_seed_and_inputs_produce_identical_state() {
        let registry = rivergate_registry_for_test();
        let config = NewGameConfig {
            seed: 9_814,
            dynasty_name: "Aster".to_owned(),
            founder_name: "Mira Aster".to_owned(),
            background: StartingBackground::ClothTrader,
        };
        let mut first = make_test_campaign_with(config.clone());
        let mut second = make_test_campaign_with(config);

        advance_days(registry, &mut first, 360).expect("first simulation must advance");
        advance_days(registry, &mut second, 360).expect("second simulation must advance");

        assert_state_eq(
            &first,
            &second,
            "identical seeds and inputs must remain deterministic across an annual boundary",
        );
    }

    #[test]
    #[ignore = "long-running soak; run `bash scripts/test.sh soak`"]
    fn test_deterministic_core_soak_preserves_invariants() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign_with(NewGameConfig {
            seed: 77,
            dynasty_name: "Corren".to_owned(),
            founder_name: "Lysa Corren".to_owned(),
            background: StartingBackground::Blacksmith,
        });
        let initial_businesses = state.businesses().iter().count();

        advance_days(registry, &mut state, 3_000).expect("soak simulation must advance");
        validate_invariants(registry, &state);

        assert_eq!(state.clock().day(), 3_000, "all requested days must run");
        assert!(
            !state.chronicle().is_empty(),
            "a multi-year campaign must produce chronicle history"
        );
        assert!(
            state.businesses().iter().count() >= initial_businesses,
            "the soak must preserve the authored business population"
        );
    }

    #[test]
    #[ignore = "long-running soak; run `bash scripts/test.sh soak`"]
    fn test_deterministic_strategic_soak_preserves_two_generations() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let initial_generation = state
            .dynasties
            .values()
            .map(|dynasty| dynasty.runtime.generation)
            .max()
            .expect("campaign must contain dynasties");

        advance_days(registry, &mut state, 7_200).expect("strategic simulation must advance");
        validate_invariants(registry, &state);

        let final_generation = state
            .dynasties
            .values()
            .map(|dynasty| dynasty.runtime.generation)
            .max()
            .expect("campaign must contain dynasties");
        assert_eq!(state.clock().day(), 7_200, "all requested days must run");
        assert!(
            final_generation > initial_generation,
            "the long soak must exercise at least one succession"
        );
        assert!(
            !state.information_reports.is_empty(),
            "long campaigns must retain current strategic reporting"
        );
        assert!(
            !state.ai_objectives.is_empty(),
            "AI dynasties must retain actionable objectives"
        );
    }
}
