//! Application-state root, ID allocation, synchronized stores, and read-only access.

use super::{
    AiObjective, AuditRecord, Business, Character, ChronicleEntry, CivicDebt, Crisis,
    DistrictRuntime, Dynasty, DynastyPair, EmploymentAgreement, EnactedLaw, ExternalRoute,
    FamilyCouncilState, FamilyLink, Household, InformationReport, InstitutionRuntime, LegalCase,
    Loan, MarketState, OutboxMessage, Property, PublicWork, RelationshipState, StartingBackground,
    SupplyContract,
};
use crate::ids::{
    BusinessId, CharacterId, ChronicleEntryId, CivicDebtId, ContractId, CrisisId, DistrictId,
    DynastyId, EmploymentId, ExternalRouteId, FamilyLinkId, HouseholdId, IdentifierAllocationError,
    InformationReportId, LawId, LegalCaseId, LoanId, ObjectiveId, OutboxMessageId, PropertyId,
    PublicWorkId,
};
use crate::rng::DeterministicRng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CURRENT_SCHEMA_VERSION: u32 = 19;

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
            !self.records.contains_key(&character_id),
            "duplicate character ID {character_id}"
        );
        assert!(
            !self
                .by_dynasty
                .values()
                .any(|characters| characters.contains(&character_id)),
            "duplicate character index entry {character_id}"
        );
        self.records.insert(character_id, character);
        let inserted = self
            .by_dynasty
            .entry(dynasty_id)
            .or_default()
            .insert(character_id);
        debug_assert!(inserted);
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
            !self.records.contains_key(&business_id),
            "duplicate business ID {business_id}"
        );
        assert!(
            !self
                .by_owner
                .values()
                .any(|businesses| businesses.contains(&business_id)),
            "duplicate owner index entry {business_id}"
        );
        assert!(
            !self
                .by_district
                .values()
                .any(|businesses| businesses.contains(&business_id)),
            "duplicate district index entry {business_id}"
        );
        self.records.insert(business_id, business);
        let owner_inserted = self
            .by_owner
            .entry(owner_id)
            .or_default()
            .insert(business_id);
        let district_inserted = self
            .by_district
            .entry(district_id)
            .or_default()
            .insert(business_id);
        debug_assert!(owner_inserted && district_inserted);
    }

    #[must_use]
    pub fn get(&self, id: BusinessId) -> Option<&Business> {
        self.records.get(&id)
    }

    pub(crate) fn get_mut(&mut self, id: BusinessId) -> Option<&mut Business> {
        self.records.get_mut(&id)
    }

    pub(crate) fn transfer_ownership(
        &mut self,
        business_id: BusinessId,
        new_owner_id: DynastyId,
        new_manager_id: CharacterId,
    ) -> Option<DynastyId> {
        let business = self.records.get(&business_id)?;
        let prior_owner_id = business.identity.owner_dynasty_id;
        let district_id = business.identity.district_id;
        assert!(
            self.by_owner
                .get(&prior_owner_id)
                .is_some_and(|businesses| businesses.contains(&business_id)),
            "missing owner index entry {business_id}"
        );
        assert_eq!(
            self.by_owner
                .values()
                .filter(|businesses| businesses.contains(&business_id))
                .count(),
            1,
            "business {business_id} has duplicate owner index entries"
        );
        assert!(
            self.by_district
                .get(&district_id)
                .is_some_and(|businesses| businesses.contains(&business_id)),
            "missing district index entry {business_id}"
        );
        assert_eq!(
            self.by_district
                .values()
                .filter(|businesses| businesses.contains(&business_id))
                .count(),
            1,
            "business {business_id} has duplicate district index entries"
        );
        if prior_owner_id == new_owner_id {
            self.records
                .get_mut(&business_id)
                .expect("validated business must exist")
                .operations
                .manager_id = new_manager_id;
            return Some(prior_owner_id);
        }
        let remove_prior_owner = self
            .by_owner
            .get_mut(&prior_owner_id)
            .expect("validated owner index must exist");
        let removed = remove_prior_owner.remove(&business_id);
        debug_assert!(removed);
        let remove_prior_owner = remove_prior_owner.is_empty();
        if remove_prior_owner {
            self.by_owner.remove(&prior_owner_id);
        }
        let inserted = self
            .by_owner
            .entry(new_owner_id)
            .or_default()
            .insert(business_id);
        debug_assert!(inserted);
        let business = self
            .records
            .get_mut(&business_id)
            .expect("validated business must exist");
        business.identity.owner_dynasty_id = new_owner_id;
        business.operations.manager_id = new_manager_id;
        Some(prior_owner_id)
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
            !self.records.contains_key(&household_id),
            "duplicate household ID {household_id}"
        );
        assert!(
            !self
                .by_district
                .values()
                .any(|households| households.contains(&household_id)),
            "duplicate household district index entry {household_id}"
        );
        self.records.insert(household_id, household);
        let inserted = self
            .by_district
            .entry(district_id)
            .or_default()
            .insert(household_id);
        debug_assert!(inserted);
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

pub(crate) fn population_weighted_food_satisfaction_basis_points<'a>(
    households: impl IntoIterator<Item = &'a Household>,
) -> Option<u16> {
    let mut weighted_total = 0_u128;
    let mut total_members = 0_u128;
    for household in households {
        let members = u128::from(household.members());
        let weighted_satisfaction = members
            .checked_mul(u128::from(household.food_satisfaction_basis_points()))
            .expect("weighted household satisfaction must fit u128");
        weighted_total = weighted_total
            .checked_add(weighted_satisfaction)
            .expect("total weighted household satisfaction must fit u128");
        total_members = total_members
            .checked_add(members)
            .expect("total household population must fit u128");
    }
    let average = weighted_total.checked_div(total_members)?;
    Some(u16::try_from(average).expect("population-weighted satisfaction must fit u16"))
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
    civic_debt: u32,
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

macro_rules! try_next_id_method {
    ($try_method:ident, $field:ident, $id_type:ident, $error:ident) => {
        pub(crate) const fn $try_method(&mut self) -> Result<$id_type, IdentifierAllocationError> {
            // `u32::MAX` is an invalid exhausted allocator state. Keep
            // `u32::MAX - 1` as the terminal valid counter value and reject an
            // allocation that would advance the campaign into that invalid state.
            if self.$field >= u32::MAX - 1 {
                return Err(IdentifierAllocationError::$error);
            }
            let id = $id_type::new(self.$field);
            self.$field += 1;
            Ok(id)
        }
    };
}

macro_rules! next_id_method {
    ($method:ident, $try_method:ident, $field:ident, $id_type:ident, $error:ident) => {
        try_next_id_method!($try_method, $field, $id_type, $error);
        pub(crate) const fn $method(&mut self) -> $id_type {
            match self.$try_method() {
                Ok(id) => id,
                Err(_) => panic!(concat!(stringify!($id_type), " identifier space exhausted")),
            }
        }
    };
}

macro_rules! test_next_id_method {
    ($method:ident, $try_method:ident, $field:ident, $id_type:ident, $error:ident) => {
        try_next_id_method!($try_method, $field, $id_type, $error);
        #[cfg(test)]
        pub(crate) const fn $method(&mut self) -> $id_type {
            match self.$try_method() {
                Ok(id) => id,
                Err(_) => panic!(concat!(stringify!($id_type), " identifier space exhausted")),
            }
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
            civic_debt: 0,
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

    next_id_method!(dynasty, try_dynasty, dynasty, DynastyId, Dynasty);
    next_id_method!(character, try_character, character, CharacterId, Character);
    next_id_method!(household, try_household, household, HouseholdId, Household);
    next_id_method!(business, try_business, business, BusinessId, Business);
    test_next_id_method!(contract, try_contract, contract, ContractId, Contract);
    next_id_method!(property, try_property, property, PropertyId, Property);
    test_next_id_method!(loan, try_loan, loan, LoanId, Loan);
    test_next_id_method!(
        civic_debt,
        try_civic_debt,
        civic_debt,
        CivicDebtId,
        CivicDebt
    );
    next_id_method!(
        employment,
        try_employment,
        employment,
        EmploymentId,
        Employment
    );
    next_id_method!(
        family_link,
        try_family_link,
        family_link,
        FamilyLinkId,
        FamilyLink
    );
    next_id_method!(law, try_law, law, LawId, Law);
    next_id_method!(
        information_report,
        try_information_report,
        information_report,
        InformationReportId,
        InformationReport
    );
    next_id_method!(objective, try_objective, objective, ObjectiveId, Objective);
    next_id_method!(
        public_work,
        try_public_work,
        public_work,
        PublicWorkId,
        PublicWork
    );
    test_next_id_method!(
        legal_case,
        try_legal_case,
        legal_case,
        LegalCaseId,
        LegalCase
    );
    next_id_method!(
        external_route,
        try_external_route,
        external_route,
        ExternalRouteId,
        ExternalRoute
    );
    test_next_id_method!(crisis, try_crisis, crisis, CrisisId, Crisis);
    test_next_id_method!(outbox, try_outbox, outbox, OutboxMessageId, OutboxMessage);
    next_id_method!(
        chronicle,
        try_chronicle,
        chronicle,
        ChronicleEntryId,
        ChronicleEntry
    );
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
    pub(crate) institutions: BTreeMap<crate::ids::InstitutionId, InstitutionRuntime>,
    pub(crate) market: MarketState,
    pub(crate) contracts: BTreeMap<ContractId, SupplyContract>,
    pub(crate) loans: BTreeMap<LoanId, Loan>,
    pub(crate) civic_debts: BTreeMap<CivicDebtId, CivicDebt>,
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

    pub fn institutions(&self) -> impl Iterator<Item = &InstitutionRuntime> {
        self.institutions.values()
    }

    pub fn contracts(&self) -> impl Iterator<Item = &SupplyContract> {
        self.contracts.values()
    }

    pub fn loans(&self) -> impl Iterator<Item = &Loan> {
        self.loans.values()
    }

    pub fn civic_debts(&self) -> impl Iterator<Item = &CivicDebt> {
        self.civic_debts.values()
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
                if self.next_ids.$field == u32::MAX {
                    return Err(format!(
                        "next {} ID has exhausted the supported identifier space",
                        $label
                    ));
                }
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
        require_next_id!(civic_debt, "civic debt", self.civic_debts.keys().copied());
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
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
