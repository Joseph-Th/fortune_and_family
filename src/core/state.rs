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
use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub const CURRENT_SCHEMA_VERSION: u32 = 30;

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

    /// The one-based day within the 360-day calendar year.
    ///
    /// # Panics
    ///
    /// Never panics: `rem_euclid(360)` is bounded to 0..=359 for any `i64`
    /// day, so the one-based result always fits a `u16`.
    #[must_use]
    pub fn day_of_year(self) -> u16 {
        u16::try_from(self.day.rem_euclid(360) + 1).expect("day-of-year fits a u16")
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

/// Satisfaction assumed wherever no population exists to average over: a
/// district without households is neither starving nor sated. The simulation's
/// unrest model and every read model share this one neutral value.
pub const NEUTRAL_FOOD_SATISFACTION_BASIS_POINTS: u16 = 5_000;

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
            // `u32::MAX` is an invalid exhausted allocator state, and so is
            // `u32::MAX - 1`: both are rejected at allocation and load time.
            // The terminal valid counter value is `u32::MAX - 2`, whose
            // increment lands on the last valid ID below the rejected band.
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
        /// Panics on identifier exhaustion. Production callers are the
        /// fresh-campaign constructors, where a brand-new allocator cannot
        /// be exhausted; every incremental system path uses the `try_*`
        /// variant so exhaustion surfaces as a typed error.
        pub(crate) const fn $method(&mut self) -> $id_type {
            match self.$try_method() {
                Ok(id) => id,
                Err(_) => panic!(concat!(stringify!($id_type), " identifier space exhausted")),
            }
        }
    };
}

/// Generates both the fallible `try_*` allocator (a production path used by
/// every consequential system) and a panicking twin compiled only for tests,
/// where fixtures treat identifier exhaustion as unreachable.
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

/// Append-only history container whose clones stay cheap as campaigns age.
///
/// The audit log, chronicle, and outbox grow without bound, and every
/// transactional commit clones the whole working state and then drops the
/// replaced original. A plain `Vec` makes both sides of that idiom
/// proportional to total campaign history even though histories only ever
/// gain entries. [`HistoryLog`] appends into a small exclusive tail while the
/// immutable bulk is shared through an arc, so a clone is one refcount plus a
/// short copy and dropping a superseded working copy releases its share of
/// the bulk untouched. Iteration order, serialized shape, and observable
/// values are identical to the plain vector it replaces.
///
/// The log also maintains an incremental structural checksum over its entry
/// stream (see [`HistoryLog::structural_checksum`]). Appends extend the fold
/// in constant time, so observation paths that re-read the checksum after
/// every simulated day stay flat-cost across campaign length instead of
/// reserializing the whole history. The memo never affects stored content,
/// equality (which compares element-wise), or serialization.
#[derive(Debug)]
pub struct HistoryLog<T> {
    base: Arc<Vec<T>>,
    tail: Vec<T>,
    /// Number of entries folded into `checksum_state`, or
    /// [`HISTORY_CHECKSUM_UNSYNCED`](self::HISTORY_CHECKSUM_UNSYNCED) when
    /// non-append mutations made the memo stale and the next read must
    /// rebuild it.
    checksum_len: AtomicU64,
    /// Running FNV-1a mid-state covering entries `0..checksum_len`.
    checksum_state: AtomicU64,
}

/// Memo sentinel meaning "the running checksum no longer matches the log".
const HISTORY_CHECKSUM_UNSYNCED: u64 = u64::MAX;

impl<T: Clone> Clone for HistoryLog<T> {
    fn clone(&self) -> Self {
        Self {
            base: self.base.clone(),
            tail: self.tail.clone(),
            // A cloned log shares the same entry stream, so the memo state
            // transfers verbatim and stays valid on both copies.
            checksum_len: AtomicU64::new(self.checksum_len.load(Ordering::Relaxed)),
            checksum_state: AtomicU64::new(self.checksum_state.load(Ordering::Relaxed)),
        }
    }
}

/// Entries appended since the last fold; past this many, an exclusively
/// owned log folds them into the shared bulk so the tail stays a short copy.
const HISTORY_TAIL_FOLD_THRESHOLD: usize = 1024;

impl<T> Default for HistoryLog<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Double-ended, exact-size iterator over a [`HistoryLog`]'s entries in
/// insertion order.
#[derive(Clone, Debug)]
pub struct HistoryLogIter<'a, T> {
    base: std::slice::Iter<'a, T>,
    tail: std::slice::Iter<'a, T>,
}

impl<'a, T> Iterator for HistoryLogIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.base.next().or_else(|| self.tail.next())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.base.len() + self.tail.len();
        (remaining, Some(remaining))
    }

    fn count(self) -> usize {
        self.base.count() + self.tail.count()
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        // Specialized so adapters like `Skip` position in constant time
        // instead of walking the folded bulk one entry at a time.
        let base_len = self.base.len();
        if n < base_len {
            return self.base.nth(n);
        }
        // Skipping past every remaining base entry exhausts it outright.
        let _ = self.base.nth(base_len);
        self.tail.nth(n - base_len)
    }

    fn last(mut self) -> Option<Self::Item> {
        self.next_back()
    }
}

impl<T> DoubleEndedIterator for HistoryLogIter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.tail.next_back().or_else(|| self.base.next_back())
    }

    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        // Mirror of `nth` from the back: constant-time positioning.
        let tail_len = self.tail.len();
        if n < tail_len {
            return self.tail.nth_back(n);
        }
        let _ = self.tail.nth_back(tail_len);
        self.base.nth_back(n - tail_len)
    }
}

impl<T> ExactSizeIterator for HistoryLogIter<'_, T> {}

impl<T> HistoryLog<T> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            base: Arc::new(Vec::new()),
            tail: Vec::new(),
            checksum_len: AtomicU64::new(0),
            checksum_state: AtomicU64::new(super::checksum::ChecksumFolder::new().raw()),
        }
    }

    pub fn push(&mut self, entry: T)
    where
        T: Serialize,
    {
        // Folding requires exclusive access: under a shared base (a live
        // transactional clone elsewhere) folding would deep-copy the entire
        // bulk, so the tail simply keeps growing until ownership is sole.
        if self.tail.len() >= HISTORY_TAIL_FOLD_THRESHOLD
            && let Some(base) = Arc::get_mut(&mut self.base)
        {
            base.append(&mut self.tail);
        }
        let total_before = self.len();
        self.tail.push(entry);
        // Extending the running checksum is only valid when it currently
        // covers exactly the pre-push entries; after an invalidating
        // mutation the next read rebuilds from scratch instead.
        if self.checksum_len.load(Ordering::Relaxed) == total_before as u64
            && let Some(entry) = self.tail.last()
        {
            let mut folder = super::checksum::ChecksumFolder::from_raw(
                self.checksum_state.load(Ordering::Relaxed),
            );
            let _ = entry.serialize(&mut folder);
            self.checksum_state.store(folder.raw(), Ordering::Relaxed);
            self.checksum_len
                .store(total_before as u64 + 1, Ordering::Relaxed);
        }
    }

    /// Marks the incremental checksum stale so the next read rebuilds it.
    ///
    /// Call this from every operation that can alter already-folded entries.
    /// Appends via [`Self::push`] extend the memo instead of invalidating it.
    fn invalidate_checksum(&self) {
        self.checksum_len
            .store(HISTORY_CHECKSUM_UNSYNCED, Ordering::Relaxed);
    }

    /// The structural checksum over the log's entry stream in insertion
    /// order: an FNV-1a fold of each entry's serialized shape, terminated by
    /// the entry count. Equal contents always produce equal values, any
    /// appended or mutated entry changes the value, and repeated reads are
    /// stable.
    ///
    /// Appends are folded incrementally, so reading stays flat-cost across
    /// campaign length. A rebuild after non-append mutations is proportional
    /// to the history once, then incremental again.
    #[must_use]
    pub fn structural_checksum(&self) -> u64
    where
        T: Serialize,
    {
        let total = self.len();
        let len = total as u64;
        let hashed_len = self.checksum_len.load(Ordering::Relaxed);
        if hashed_len != len {
            let mut folder = super::checksum::ChecksumFolder::new();
            for entry in self {
                let _ = entry.serialize(&mut folder);
            }
            self.checksum_state.store(folder.raw(), Ordering::Relaxed);
            // Another reader may have rebuilt concurrently with the same
            // deterministic result; only refuse to store a *stale* length
            // (impossible without a concurrent writer, but cheap to guard).
            let _ = self.checksum_len.compare_exchange(
                hashed_len,
                len,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
        let finisher =
            super::checksum::ChecksumFolder::from_raw(self.checksum_state.load(Ordering::Relaxed));
        finisher.finish_with_entry_count(total)
    }

    /// Folds the tail into the bulk, taking exclusive ownership of it. A
    /// shared bulk is first cloned, mirroring `Arc::make_mut` semantics.
    fn fold_tail(&mut self)
    where
        T: Clone,
    {
        if !self.tail.is_empty() {
            Arc::make_mut(&mut self.base).append(&mut self.tail);
        }
    }

    /// The number of entries in the log.
    ///
    /// # Panics
    ///
    /// Panics only if the total entry count exceeds `usize::MAX`, which is
    /// unreachable for any representable campaign history.
    #[must_use]
    pub fn len(&self) -> usize {
        self.base
            .len()
            .checked_add(self.tail.len())
            .expect("history length must fit usize")
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.base.is_empty() && self.tail.is_empty()
    }

    #[must_use]
    pub fn last(&self) -> Option<&T> {
        self.tail.last().or_else(|| self.base.last())
    }

    #[must_use]
    pub fn iter(&self) -> HistoryLogIter<'_, T> {
        HistoryLogIter {
            base: self.base.iter(),
            tail: self.tail.iter(),
        }
    }

    /// Mutable iteration over every entry in one folded buffer. A shared
    /// bulk is first cloned, mirroring `Arc::make_mut` copy-on-write
    /// semantics.
    pub fn iter_mut(&mut self) -> HistoryLogIterMut<'_, T>
    where
        T: Clone,
    {
        self.fold_tail();
        // Entries can change through this iterator, so the incremental
        // checksum must be rebuilt on its next read.
        self.invalidate_checksum();
        HistoryLogIterMut {
            entries: Arc::make_mut(&mut self.base).iter_mut(),
        }
    }

    /// Partitions the day-ordered history at the first entry satisfying
    /// `predicate`, mirroring `<[T]>::partition_point` on the combined log.
    #[must_use]
    pub fn partition_point<F>(&self, mut predicate: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        let base_position = self.base.partition_point(|entry| predicate(entry));
        if base_position < self.base.len() {
            base_position
        } else {
            base_position + self.tail.partition_point(predicate)
        }
    }

    /// Retains only the entries the predicate accepts, preserving order.
    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&T) -> bool,
        T: Clone,
    {
        self.fold_tail();
        self.invalidate_checksum();
        Arc::make_mut(&mut self.base).retain(|entry| keep(entry));
    }

    /// Removes every entry.
    pub fn clear(&mut self) {
        // A fresh empty log simply releases this handle's share of the bulk.
        *self = Self::new();
    }

    /// Stable-sorts entries by `compare`; production histories are appended
    /// in day order and are never reordered, so this exists for test
    /// fixtures that assemble records out of order.
    #[cfg(test)]
    pub fn sort_by_key<K, F>(&mut self, mut compare: F)
    where
        F: FnMut(&T) -> K,
        K: Ord,
        T: Clone,
    {
        self.fold_tail();
        self.invalidate_checksum();
        Arc::make_mut(&mut self.base).sort_by_key(|entry| compare(entry));
    }
}

/// Mutable counterpart to [`HistoryLogIter`]; iteration always operates on
/// one folded buffer.
#[derive(Debug)]
pub struct HistoryLogIterMut<'a, T> {
    entries: std::slice::IterMut<'a, T>,
}

impl<'a, T> Iterator for HistoryLogIterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl<T> DoubleEndedIterator for HistoryLogIterMut<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.entries.next_back()
    }
}

impl<T> ExactSizeIterator for HistoryLogIterMut<'_, T> {}

impl<T> PartialEq for HistoryLog<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<T> Eq for HistoryLog<T> where T: Eq {}

impl<'a, T> IntoIterator for &'a HistoryLog<T> {
    type Item = &'a T;
    type IntoIter = HistoryLogIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut HistoryLog<T>
where
    T: Clone,
{
    type Item = &'a mut T;
    type IntoIter = HistoryLogIterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<T> Serialize for HistoryLog<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut sequence = serializer.serialize_seq(Some(self.len()))?;
        for entry in self {
            sequence.serialize_element(entry)?;
        }
        sequence.end()
    }
}

impl<'de, T> Deserialize<'de> for HistoryLog<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self {
            base: Arc::new(Vec::<T>::deserialize(deserializer)?),
            tail: Vec::new(),
            // A fresh memo: the first checksum read rebuilds it once.
            checksum_len: AtomicU64::new(HISTORY_CHECKSUM_UNSYNCED),
            checksum_state: AtomicU64::new(super::checksum::ChecksumFolder::new().raw()),
        })
    }
}

/// Incremental fold of campaign-phase evidence over the append-only audit
/// log.
///
/// Phase derivation reads three audit record kinds (`OfficeDirective`,
/// `OfficeNomination`, `InstitutionPatronage`) whose records accumulate for
/// the lifetime of the campaign. Rescanning the whole log every simulated day
/// made each day's cost grow with campaign length; this memo folds only the
/// entries appended since the last synchronization and keeps the folded
/// answers in small ID sets.
///
/// The memo is a pure derivation of persisted state and never affects
/// behavior, so it is excluded from serialization (the save schema is
/// unchanged) and from [`AppState`] equality. It is rebuilt lazily after a
/// load from its guards:
///
/// - Audit entries are append-only with chronologically nondecreasing days
///   (an enforced invariant), so a shrinking history or a last-day regression
///   invalidates the fold and forces a full rebuild.
/// - Typed IDs are never reused, so a character resolved into a nomination or
///   patronage bucket can be dropped once its record is gone without that ID
///   ever contributing again through a re-created record holder.
///
/// Records whose referenced character does not resolve yet are retried on
/// every synchronization, mirroring a full rescan's behavior when such a
/// character appears later.
#[derive(Clone, Debug, Default)]
pub(crate) struct CampaignEvidenceMemo {
    /// Number of leading audit entries already folded into the sets below.
    pub(crate) folded_len: usize,
    /// Day of the last folded entry; unused while `folded_len` is zero.
    pub(crate) folded_last_day: i64,
    /// Dynasty/institution pairs named by folded `OfficeDirective` records.
    /// Institution existence is deliberately checked at materialization time
    /// so the memo never encodes an institution-lifecycle answer.
    pub(crate) office_directive_houses: BTreeSet<(DynastyId, crate::ids::InstitutionId)>,
    /// Characters named by folded `OfficeNomination` records, mapped to their
    /// dynasty at first resolution.
    pub(crate) nomination_characters: BTreeMap<CharacterId, DynastyId>,
    /// Characters named by folded `InstitutionPatronage` records.
    pub(crate) patronage_characters: BTreeMap<CharacterId, DynastyId>,
    /// Nomination characters not yet resolvable against current state.
    pub(crate) unresolved_nomination_characters: BTreeSet<CharacterId>,
    /// Patronage characters not yet resolvable against current state.
    pub(crate) unresolved_patronage_characters: BTreeSet<CharacterId>,
}

impl CampaignEvidenceMemo {
    /// Whether the memo can be extended by folding entries past
    /// `folded_len`: the history must still contain every folded entry and
    /// must not regress below the last folded day. Anything else replaces the
    /// memo with a fresh full-history rebuild.
    pub(crate) fn is_consistent_with(&self, audit_log: &HistoryLog<AuditRecord>) -> bool {
        if self.folded_len > audit_log.len() {
            return false;
        }
        if self.folded_len == 0 {
            return true;
        }
        audit_log
            .last()
            .is_some_and(|record| record.day() >= self.folded_last_day)
    }
}

/// The serializable campaign state.
///
/// Equality deliberately excludes `campaign_evidence_memo`: it is a pure
/// derivation of the other fields. When adding a field, extend the hand-written
/// `PartialEq` implementation below — the derive was removed so a new field
/// cannot silently bypass the comparison contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppState {
    pub(crate) schema_version: u32,
    pub(crate) scenario_key: String,
    pub(crate) registry_fingerprint: u64,
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
    pub(crate) outbox: HistoryLog<OutboxMessage>,
    pub(crate) chronicle: HistoryLog<ChronicleEntry>,
    pub(crate) audit_log: HistoryLog<AuditRecord>,
    #[serde(skip)]
    pub(crate) campaign_evidence_memo: CampaignEvidenceMemo,
}

impl PartialEq for AppState {
    fn eq(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.scenario_key == other.scenario_key
            && self.registry_fingerprint == other.registry_fingerprint
            && self.clock == other.clock
            && self.rng == other.rng
            && self.next_ids == other.next_ids
            && self.player_dynasty_id == other.player_dynasty_id
            && self.dynasties == other.dynasties
            && self.characters == other.characters
            && self.households == other.households
            && self.businesses == other.businesses
            && self.institutions == other.institutions
            && self.market == other.market
            && self.contracts == other.contracts
            && self.loans == other.loans
            && self.civic_debts == other.civic_debts
            && self.properties == other.properties
            && self.employment == other.employment
            && self.family_links == other.family_links
            && self.family_councils == other.family_councils
            && self.laws == other.laws
            && self.relationships == other.relationships
            && self.information_reports == other.information_reports
            && self.ai_objectives == other.ai_objectives
            && self.districts == other.districts
            && self.public_works == other.public_works
            && self.legal_cases == other.legal_cases
            && self.external_routes == other.external_routes
            && self.crises == other.crises
            && self.outbox == other.outbox
            && self.chronicle == other.chronicle
            && self.audit_log == other.audit_log
        // `campaign_evidence_memo` is intentionally absent: it is a pure
        // function of the fields above and never affects behavior.
    }
}

impl Eq for AppState {}

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
    pub const fn registry_fingerprint(&self) -> u64 {
        self.registry_fingerprint
    }

    #[must_use]
    pub const fn clock(&self) -> SimulationClock {
        self.clock
    }

    #[must_use]
    pub const fn player_dynasty_id(&self) -> DynastyId {
        self.player_dynasty_id
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
    pub const fn businesses(&self) -> &BusinessStore {
        &self.businesses
    }

    #[must_use]
    pub fn chronicle(&self) -> &HistoryLog<ChronicleEntry> {
        &self.chronicle
    }

    pub(crate) fn validate_next_ids(&self) -> Result<(), String> {
        macro_rules! require_next_id {
            ($field:ident, $label:literal, $ids:expr) => {
                if self.next_ids.$field >= u32::MAX - 1 {
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
