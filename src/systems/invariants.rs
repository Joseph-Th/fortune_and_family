//! Runtime invariant battery: debug-only assertions over registry, references, indexes,
//! lifecycle, and numeric ranges.
//!
//! Purpose: give simulation (`advance_days`) and commands (`apply_player_command_scratch`)
//! a single `validate_invariants(registry, state)` that fails fast in debug builds and
//! collapses to zero cost in release (`prepare_invariant_ids` short-circuits).
//! Owns: every `validate_{market,characters,dynasties,businesses,…}` check; titles encode
//! the invariant family (`Lifecycle Validity`, `Record Reference Validity`, `Ownership
//! Exclusivity`, `Derived Data Consistency`, etc.).
//! Reads: `Registry` + `AppState` (immutable); `validate_invariants_with_ids` reuses
//! one `RegistryIds` across many consecutive checks.
//! Mutates: nothing.
//! Does not own: authoritative persistence validation (`src/persistence.rs`) or recovery.
//! Focused tests: exercised indirectly by every behavioral test that builds or advances
//! a campaign in debug mode; persistence tests cover release validation.

use super::is_schedulable_day;
use crate::core::{
    AppState, AuditKind, Business, CharacterStatus, CivicDebtStatus, ContractStatus, DynastyPair,
    EmploymentStatus, FamilyLinkKind, LegalCaseKind, LegalCaseStatus, LegalClaimSource, LoanStatus,
    PublicWorkStatus,
};
use crate::ids::{
    BusinessId, CharacterId, DistrictId, DynastyId, GoodId, HouseholdId, InstitutionId, RecipeId,
};
use crate::registry::{DistrictDef, GoodDef, InstitutionDef, RecipeDef, Registry};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub(crate) struct RegistryIds {
    districts: BTreeSet<DistrictId>,
    goods: BTreeSet<GoodId>,
    recipes: BTreeSet<RecipeId>,
    institutions: BTreeSet<InstitutionId>,
}

impl RegistryIds {
    fn new(registry: &Registry) -> Self {
        Self {
            districts: registry.districts().iter().map(DistrictDef::id).collect(),
            goods: registry.goods().iter().map(GoodDef::id).collect(),
            recipes: registry.recipes().iter().map(RecipeDef::id).collect(),
            institutions: registry
                .institutions()
                .iter()
                .map(InstitutionDef::id)
                .collect(),
        }
    }
}

/// Prepares the per-registry lookup sets the invariant battery needs, but only
/// when debug assertions are active (the battery's documented build profile).
/// Returns `None` in release builds so hot loops skip preparation entirely.
pub(crate) fn prepare_invariant_ids(registry: &Registry) -> Option<RegistryIds> {
    if cfg!(debug_assertions) {
        Some(RegistryIds::new(registry))
    } else {
        None
    }
}

/// Asserts all cheap runtime invariants in debug builds.
///
/// # Panics
///
/// Panics in debug builds when state contains invalid references, stale indexes, invalid lifecycle
/// combinations, negative constrained values, or inconsistent derived data.
pub fn validate_invariants(registry: &Registry, state: &AppState) {
    // Battery is debug-only: every check is `debug_assert!`. The
    // compile-time gate skips registry-ID and index preparation in release
    // so the call is free with no change to debug behavior.
    if !cfg!(debug_assertions) {
        return;
    }
    validate_invariants_with_ids(registry, state, &RegistryIds::new(registry));
}

/// Shared body of [`validate_invariants`] for callers that run it across many
/// consecutive states with one registry, such as the daily simulation loop.
pub(crate) fn validate_invariants_with_ids(
    registry: &Registry,
    state: &AppState,
    ids: &RegistryIds,
) {
    debug_assert!(
        state.clock.day() >= 0 && state.clock.day() < i64::MAX,
        "Lifecycle Validity: simulation clock must be nonnegative and retain advancement headroom"
    );
    debug_assert_eq!(
        state.scenario_key,
        registry.scenario().key(),
        "Registry Reference Validity: state scenario does not match loaded registry"
    );
    debug_assert_eq!(
        state.registry_fingerprint,
        registry.fingerprint(),
        "Registry Reference Validity: state registry fingerprint does not match loaded registry"
    );
    debug_assert!(
        state.dynasties.contains_key(&state.player_dynasty_id),
        "Record Reference Validity: player dynasty does not exist"
    );
    debug_assert!(
        state.validate_next_ids().is_ok(),
        "Identifier Allocation: next-ID state is stale or exhausted"
    );

    validate_market(registry, state, ids);
    validate_characters(state);
    validate_dynasties(state);
    validate_businesses(registry, state, ids);
    validate_households(state, ids);
    validate_institutions(registry, state, ids);
    validate_strategic_state(registry, state, ids);
    validate_history(state);
}

fn validate_market(registry: &Registry, state: &AppState, ids: &RegistryIds) {
    debug_assert_eq!(
        state.market.quotes.len(),
        registry.goods().len(),
        "Registry Reference Validity: every good must have exactly one market quote"
    );
    for (good_id, quote) in &state.market.quotes {
        debug_assert_eq!(
            *good_id, quote.good_id,
            "Derived Data Consistency: market quote key and record ID differ"
        );
        debug_assert!(
            ids.goods.contains(good_id),
            "Registry Reference Validity: market quote references missing good {good_id}"
        );
        debug_assert!(
            quote.price.copper() > 0,
            "Lifecycle Validity: market price must remain positive for good {good_id}"
        );
        debug_assert!(
            quote.previous_price.copper() > 0,
            "Lifecycle Validity: previous market price must remain positive for good {good_id}"
        );
        debug_assert!(
            !quote.stock.is_negative(),
            "Lifecycle Validity: market stock must remain nonnegative for good {good_id}"
        );
        debug_assert!(
            !quote.demand_today.is_negative() && !quote.supply_today.is_negative(),
            "Lifecycle Validity: market flows must remain nonnegative for good {good_id}"
        );
        debug_assert!(
            quote.target_stock > crate::money::Quantity::ZERO,
            "Definition/Runtime Separation: market target stock must remain positive"
        );
        debug_assert_eq!(
            quote.target_stock,
            registry
                .get_good(*good_id)
                .expect("validated market good must exist")
                .target_market_stock(),
            "Definition/Runtime Separation: market target stock differs from its definition"
        );
    }
}

fn validate_characters(state: &AppState) {
    let mut expected_index: BTreeMap<DynastyId, BTreeSet<CharacterId>> = BTreeMap::new();
    for character in state.characters.records().values() {
        debug_assert!(
            !character.name().trim().is_empty(),
            "No Lost Runtime State: character {} has a blank name",
            character.id()
        );
        debug_assert!(
            character.birth_day() <= state.clock.day(),
            "Lifecycle Validity: character {} is born after the current day",
            character.id()
        );
        debug_assert!(
            state.dynasties.contains_key(&character.dynasty_id()),
            "Record Reference Validity: character {} references missing dynasty {}",
            character.id(),
            character.dynasty_id()
        );
        debug_assert!(
            character.runtime.health_basis_points <= 10_000
                && (character.status() != CharacterStatus::Active
                    || character.runtime.health_basis_points > 0),
            "Lifecycle Validity: active character {} must have positive in-range health",
            character.id()
        );
        debug_assert_eq!(
            character.status() == CharacterStatus::Incapacitated,
            character.runtime.incapacitated_day.is_some(),
            "Lifecycle Validity: incapacitation state and collapse date must agree for character {}",
            character.id()
        );
        if let Some(collapsed_day) = character.runtime.incapacitated_day {
            debug_assert!(
                collapsed_day <= state.clock.day(),
                "Record Reference Validity: character {} collapsed in the future",
                character.id()
            );
        }
        debug_assert!(
            character.runtime.loyalty_basis_points <= 10_000,
            "Lifecycle Validity: character {} loyalty is outside basis-point range",
            character.id()
        );
        debug_assert!(
            character.capabilities.administration <= 100
                && character.capabilities.commerce <= 100
                && character.capabilities.social <= 100
                && character.capabilities.craft <= 100,
            "Lifecycle Validity: character {} capability is outside the 0..=100 range",
            character.id()
        );
        expected_index
            .entry(character.dynasty_id())
            .or_default()
            .insert(character.id());
    }
    debug_assert_eq!(
        &expected_index,
        state.characters.index(),
        "Index Completeness: character dynasty index does not match character records"
    );
}

fn validate_dynasties(state: &AppState) {
    // One audit-derived evidence fold for every dynasty in this battery run:
    // per-dynasty collection would rescan the unbounded audit log per house.
    let campaign_phase_evidence =
        crate::systems::progression::CampaignPhaseEvidence::collect(state);
    for dynasty in state.dynasties.values() {
        debug_assert!(
            !dynasty.name().trim().is_empty(),
            "No Lost Runtime State: dynasty {} has a blank name",
            dynasty.id()
        );
        debug_assert_ne!(
            Some(dynasty.head_id()),
            dynasty.heir_id(),
            "Ownership Exclusivity: dynasty head and heir must differ"
        );
        let head = state.characters.get(dynasty.head_id());
        debug_assert!(
            head.is_some(),
            "Record Reference Validity: dynasty {} head {} does not exist",
            dynasty.id(),
            dynasty.head_id()
        );
        if let Some(head) = head {
            debug_assert_eq!(
                head.dynasty_id(),
                dynasty.id(),
                "Ownership Exclusivity: dynasty head belongs to another dynasty"
            );
            debug_assert_eq!(
                head.status(),
                CharacterStatus::Active,
                "Lifecycle Validity: dynasty head must be active"
            );
            debug_assert_eq!(
                head.role(),
                crate::core::CharacterRole::HeadOfHouse,
                "Lifecycle Validity: dynasty head must have the head-of-house role"
            );
        }
        validate_heir(state, dynasty.id(), dynasty.heir_id());
        debug_assert!(
            !dynasty.treasury().is_negative(),
            "Lifecycle Validity: dynasty {} treasury is negative",
            dynasty.id()
        );
        debug_assert!(
            !dynasty.civic_contributions().is_negative(),
            "Lifecycle Validity: dynasty {} civic contributions are negative",
            dynasty.id()
        );
        debug_assert!(
            dynasty.resources.legitimacy_basis_points <= 10_000,
            "Lifecycle Validity: dynasty legitimacy is outside basis-point range"
        );
        debug_assert!(
            dynasty.resources.reputation_quality_basis_points <= 10_000
                && dynasty.resources.reputation_reliability_basis_points <= 10_000
                && dynasty.runtime.succession_risk_basis_points <= 10_000,
            "Lifecycle Validity: dynasty reputation or succession risk is outside basis-point range"
        );
        debug_assert!(
            dynasty.runtime.generation > 0 && dynasty.runtime.generation < u16::MAX,
            "Lifecycle Validity: dynasty generation must be positive and retain succession headroom"
        );
        debug_assert!(
            super::progression::campaign_phase_is_consistent_with(
                &campaign_phase_evidence,
                state,
                dynasty.id(),
            ),
            "Derived Data Consistency: dynasty {} campaign phase is stale or incompatible with progression",
            dynasty.id()
        );
    }
}

fn validate_heir(state: &AppState, dynasty_id: DynastyId, heir_id: Option<CharacterId>) {
    let Some(heir_id) = heir_id else {
        return;
    };
    let heir = state.characters.get(heir_id);
    debug_assert!(
        heir.is_some(),
        "Record Reference Validity: dynasty {dynasty_id} heir {heir_id} does not exist"
    );
    if let Some(heir) = heir {
        debug_assert_eq!(
            heir.dynasty_id(),
            dynasty_id,
            "Ownership Exclusivity: dynasty heir belongs to another dynasty"
        );
        debug_assert_eq!(
            heir.status(),
            CharacterStatus::Active,
            "Lifecycle Validity: dynasty heir must be active"
        );
        debug_assert_eq!(
            heir.role(),
            crate::core::CharacterRole::Heir,
            "Lifecycle Validity: dynasty heir must have the heir role"
        );
    }
}

fn validate_businesses(registry: &Registry, state: &AppState, ids: &RegistryIds) {
    let mut owner_index: BTreeMap<DynastyId, BTreeSet<BusinessId>> = BTreeMap::new();
    let mut district_index: BTreeMap<DistrictId, BTreeSet<BusinessId>> = BTreeMap::new();
    let mut administrative_load: BTreeMap<DynastyId, u64> = BTreeMap::new();

    for business in state.businesses.records().values() {
        validate_business_record(state, business, ids);
        owner_index
            .entry(business.owner_dynasty_id())
            .or_default()
            .insert(business.id());
        district_index
            .entry(business.district_id())
            .or_default()
            .insert(business.id());
        let recipe = registry
            .get_recipe(business.recipe_id())
            .expect("validated business recipe must exist");
        administrative_load
            .entry(business.owner_dynasty_id())
            .and_modify(|load| *load += u64::from(recipe.administrative_load()))
            .or_insert(u64::from(recipe.administrative_load()));
    }

    debug_assert_eq!(
        &owner_index,
        state.businesses.owner_index(),
        "Index Completeness: business owner index does not match business records"
    );
    debug_assert_eq!(
        &district_index,
        state.businesses.district_index(),
        "Index Completeness: business district index does not match business records"
    );
    validate_administrative_load(state, &administrative_load);
}

fn validate_business_record(state: &AppState, business: &Business, ids: &RegistryIds) {
    debug_assert!(
        !business.name().trim().is_empty(),
        "No Lost Runtime State: business {} has a blank name",
        business.id()
    );
    debug_assert!(
        state.dynasties.contains_key(&business.owner_dynasty_id()),
        "Record Reference Validity: business {} owner dynasty {} does not exist",
        business.id(),
        business.owner_dynasty_id()
    );
    debug_assert!(
        ids.districts.contains(&business.district_id()),
        "Registry Reference Validity: business {} district {} does not exist",
        business.id(),
        business.district_id()
    );
    debug_assert!(
        ids.recipes.contains(&business.recipe_id()),
        "Registry Reference Validity: business {} recipe {} does not exist",
        business.id(),
        business.recipe_id()
    );
    validate_manager(state, business);
    debug_assert!(
        !business.cash().is_negative(),
        "Lifecycle Validity: business {} cash {} is negative on day {}",
        business.id(),
        business.cash(),
        state.clock.day()
    );
    debug_assert!(
        business.condition_basis_points() <= 10_000,
        "Lifecycle Validity: business {} condition is outside basis-point range",
        business.id()
    );
    debug_assert!(
        business.operations.quality_basis_points <= 10_000,
        "Lifecycle Validity: business {} quality is outside basis-point range",
        business.id()
    );
    debug_assert!(
        business.operations.capacity_batches_per_day > 0,
        "Lifecycle Validity: business {} has zero production capacity",
        business.id()
    );
    debug_assert!(
        business.policy.target_input_days <= 30
            && business.policy.target_output_days <= 30
            && !business.policy.minimum_cash_reserve.is_negative()
            && business.policy.maintenance_basis_points <= 10_000
            && business.policy.quality_target_basis_points <= 10_000,
        "Lifecycle Validity: business {} policy is outside supported ranges",
        business.id()
    );
    debug_assert!(
        business.finance.version < u64::MAX,
        "Lifecycle Validity: business {} finance version is exhausted",
        business.id()
    );
    for (good_id, quantity) in business.inventory() {
        debug_assert!(
            ids.goods.contains(good_id),
            "Registry Reference Validity: business {} inventory references missing good {}",
            business.id(),
            good_id
        );
        debug_assert!(
            !quantity.is_negative(),
            "Lifecycle Validity: business {} inventory is negative for good {}",
            business.id(),
            good_id
        );
    }
}

fn validate_manager(state: &AppState, business: &Business) {
    let manager = state.characters.get(business.manager_id());
    debug_assert!(
        manager.is_some(),
        "Record Reference Validity: business {} manager {} does not exist",
        business.id(),
        business.manager_id()
    );
    if let Some(manager) = manager {
        debug_assert_eq!(
            manager.dynasty_id(),
            business.owner_dynasty_id(),
            "Ownership Exclusivity: business manager belongs to another dynasty"
        );
        debug_assert_eq!(
            manager.status(),
            CharacterStatus::Active,
            "Lifecycle Validity: operating business manager must be active"
        );
    }
}

fn validate_administrative_load(state: &AppState, expected: &BTreeMap<DynastyId, u64>) {
    for dynasty in state.dynasties.values() {
        let load = expected.get(&dynasty.id()).copied().unwrap_or(0);
        debug_assert_eq!(
            u64::from(dynasty.administrative_load()),
            load,
            "Derived Data Consistency: dynasty {} administrative load is stale",
            dynasty.id()
        );
    }
}

fn validate_households(state: &AppState, ids: &RegistryIds) {
    let mut expected_index: BTreeMap<DistrictId, BTreeSet<HouseholdId>> = BTreeMap::new();
    for household in state.households.records().values() {
        debug_assert!(
            ids.districts.contains(&household.district_id()),
            "Registry Reference Validity: household {} district {} does not exist",
            household.id(),
            household.district_id()
        );
        debug_assert!(
            !household.cash().is_negative(),
            "Lifecycle Validity: household {} cash is negative",
            household.id()
        );
        debug_assert!(
            household.food_satisfaction_basis_points() <= 10_000,
            "Lifecycle Validity: household satisfaction is outside basis-point range"
        );
        debug_assert!(
            household.members > 0
                && !household.weekly_income.is_negative()
                && !household.bread_need_daily.is_negative()
                && !household.ale_need_daily.is_negative(),
            "Lifecycle Validity: household {} has invalid membership, income, or needs",
            household.id()
        );
        expected_index
            .entry(household.district_id())
            .or_default()
            .insert(household.id());
    }
    debug_assert_eq!(
        &expected_index,
        state.households.index(),
        "Index Completeness: household district index does not match household records"
    );
}

fn validate_institutions(registry: &Registry, state: &AppState, ids: &RegistryIds) {
    debug_assert_eq!(
        state.institutions.len(),
        ids.institutions.len(),
        "Registry Reference Validity: runtime institutions must match registry definitions"
    );
    let mut officeholders = BTreeSet::new();
    for (institution_id, institution) in &state.institutions {
        debug_assert_eq!(
            *institution_id, institution.institution_id,
            "Derived Data Consistency: institution map key and record ID differ"
        );
        debug_assert!(
            ids.institutions.contains(institution_id),
            "Registry Reference Validity: runtime institution references missing definition"
        );
        debug_assert_eq!(
            institution.powers,
            super::institution_powers_for(
                registry
                    .get_institution(*institution_id)
                    .expect("validated institution definition must exist")
                    .kind()
            ),
            "Definition/Runtime Separation: institution powers differ from their definition"
        );
        debug_assert!(
            !institution.budget.is_negative()
                && institution.term_number > 0
                && institution.term_number < u32::MAX
                && institution.term_started_day <= state.clock.day()
                && super::is_valid_institution_selection_day(
                    institution.term_started_day,
                    institution.next_selection_day,
                ),
            "Lifecycle Validity: institution budget or term timing is invalid"
        );
        debug_assert!(
            institution.legitimacy_basis_points <= 10_000,
            "Lifecycle Validity: institution legitimacy is outside basis-point range"
        );
        debug_assert!(
            institution.active_directive.is_none_or(|directive| {
                super::is_valid_active_directive_expiry(state.clock.day(), directive.expires_day)
                    && institution.powers.contains(&directive.power)
            }),
            "Lifecycle Validity: institution directive is invalid"
        );
        for member_id in &institution.members {
            debug_assert!(
                state
                    .characters
                    .get(*member_id)
                    .is_some_and(|character| character.status() == CharacterStatus::Active),
                "Lifecycle Validity: institution member must exist and be active"
            );
            if state
                .characters
                .get(*member_id)
                .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
                && institution.office_holder_id != Some(*member_id)
            {
                debug_assert!(
                    state.audit_log.iter().any(|record| {
                        record.kind() == AuditKind::InstitutionPatronage
                            && record
                                .audit_subject()
                                .references_institution_character(*institution_id, *member_id)
                    }),
                    "Canonical Mutation: player institution membership requires cultivated support"
                );
            }
        }
        if let Some(holder_id) = institution.office_holder_id {
            debug_assert!(
                institution.members.contains(&holder_id),
                "Ownership Exclusivity: officeholder is not an institution member"
            );
            debug_assert!(
                state
                    .characters
                    .get(holder_id)
                    .is_some_and(|character| { character.status() == CharacterStatus::Active }),
                "Lifecycle Validity: institution office holder must exist and be active"
            );
            debug_assert!(
                officeholders.insert(holder_id),
                "Ownership Exclusivity: a character cannot hold multiple offices simultaneously"
            );
        }
    }
}

fn validate_strategic_state(registry: &Registry, state: &AppState, ids: &RegistryIds) {
    validate_contracts(registry, state, ids);
    validate_loans_and_properties(state, ids);
    validate_civic_debts(state);
    validate_employment(state);
    validate_family_state(state);
    validate_laws_and_relationships(state);
    validate_information_and_ai(state, ids);
    validate_districts_and_public_works(state, ids);
    validate_legal_cases(state);
    validate_routes_and_crises(state, ids);
    validate_outbox(state);
}

fn validate_contracts(registry: &Registry, state: &AppState, ids: &RegistryIds) {
    for (contract_id, contract) in &state.contracts {
        debug_assert_eq!(
            *contract_id, contract.id,
            "Derived Data Consistency: contract key and record ID differ"
        );
        let buyer = state.businesses.get(contract.buyer_business_id);
        let seller = state.businesses.get(contract.seller_business_id);
        debug_assert!(
            buyer.is_some(),
            "Record Reference Validity: contract buyer is missing"
        );
        debug_assert!(
            seller.is_some(),
            "Record Reference Validity: contract seller is missing"
        );
        debug_assert_ne!(
            contract.buyer_business_id, contract.seller_business_id,
            "Ownership Exclusivity: contract buyer and seller must differ"
        );
        if contract.status == ContractStatus::Active
            && let (Some(buyer), Some(seller)) = (buyer, seller)
        {
            debug_assert_ne!(
                buyer.owner_dynasty_id(),
                seller.owner_dynasty_id(),
                "Ownership Exclusivity: contract businesses must belong to different dynasties"
            );
            debug_assert!(
                !matches!(
                    buyer.status(),
                    crate::core::BusinessStatus::Insolvent | crate::core::BusinessStatus::Closed
                ) && !matches!(
                    seller.status(),
                    crate::core::BusinessStatus::Insolvent | crate::core::BusinessStatus::Closed
                ),
                "Lifecycle Validity: active contract has an insolvent or closed party"
            );
        }
        debug_assert!(
            ids.goods.contains(&contract.good_id),
            "Registry Reference Validity: contract good does not exist"
        );
        validate_contract_financial_values(contract);
        validate_contract_schedule(contract);
        debug_assert!(
            contract_breach_attribution_is_valid(state, contract),
            "Lifecycle Validity: contract breach attribution is inconsistent"
        );
        debug_assert!(
            contract_breach_penalty_is_valid(contract),
            "Lifecycle Validity: unpaid contract breach penalty is inconsistent"
        );
        debug_assert!(
            contract
                .fulfilled_deliveries_by_dynasty
                .iter()
                .all(|(dynasty_id, deliveries)| {
                    state.dynasties.contains_key(dynasty_id)
                        && *deliveries > 0
                        && *deliveries <= contract.fulfilled_deliveries
                }),
            "Record Reference Validity: contract delivery attribution is invalid"
        );
        debug_assert!(
            contract.has_consistent_delivery_attribution(),
            "Derived Data Consistency: contract delivery attribution does not match fulfillment"
        );
        if let Some(seller) = seller {
            let recipe = registry
                .get_recipe(seller.recipe_id())
                .expect("validated seller recipe must exist");
            debug_assert_eq!(
                recipe.output_good_id(),
                contract.good_id,
                "Definition/Runtime Separation: contract seller cannot produce contracted good"
            );
        }
        if let Some(buyer) = buyer {
            let recipe = registry
                .get_recipe(buyer.recipe_id())
                .expect("validated buyer recipe must exist");
            debug_assert!(
                recipe
                    .inputs()
                    .iter()
                    .any(|input| input.good_id() == contract.good_id),
                "Definition/Runtime Separation: contract buyer does not consume contracted good"
            );
        }
    }
}

fn validate_contract_schedule(contract: &crate::core::SupplyContract) {
    debug_assert!(
        is_schedulable_day(contract.next_due_day) && is_schedulable_day(contract.end_day),
        "Lifecycle Validity: contract schedule exceeds the supported timeline"
    );
    debug_assert!(
        contract.next_due_day <= contract.end_day || contract.status != ContractStatus::Active,
        "Lifecycle Validity: active contract due date exceeds its term"
    );
}

fn validate_contract_financial_values(contract: &crate::core::SupplyContract) {
    debug_assert!(
        contract.quantity_per_week > crate::money::Quantity::ZERO,
        "Lifecycle Validity: contract quantity must remain positive"
    );
    debug_assert!(
        contract.unit_price > crate::money::Money::ZERO,
        "Lifecycle Validity: contract price must remain positive"
    );
    debug_assert!(
        crate::money::checked_cost_for(contract.quantity_per_week, contract.unit_price).is_some(),
        "Lifecycle Validity: contract weekly invoice exceeds the supported money range"
    );
    debug_assert!(
        !contract.penalty.is_negative(),
        "Lifecycle Validity: contract penalty must not be negative"
    );
}

fn validate_loans_and_properties(state: &AppState, ids: &RegistryIds) {
    validate_properties(state, ids);
    validate_loans(state);
}

fn validate_properties(state: &AppState, ids: &RegistryIds) {
    let mut occupied_businesses = BTreeSet::new();
    for (property_id, property) in &state.properties {
        debug_assert_eq!(
            *property_id, property.id,
            "Derived Data Consistency: property key and record ID differ"
        );
        debug_assert!(
            !property.name.trim().is_empty(),
            "No Lost Runtime State: property has a blank name"
        );
        debug_assert!(
            ids.districts.contains(&property.district_id),
            "Registry Reference Validity: property district does not exist"
        );
        debug_assert!(
            property.value > crate::money::Money::ZERO && !property.weekly_rent.is_negative(),
            "Lifecycle Validity: property value must be positive and rent nonnegative"
        );
        debug_assert!(
            !property.anchor_value.is_negative(),
            "Lifecycle Validity: property revaluation anchor must be nonnegative"
        );
        debug_assert!(
            property.condition_basis_points <= 10_000,
            "Lifecycle Validity: property condition is outside basis-point range"
        );
        if let Some(owner_id) = property.owner_dynasty_id {
            debug_assert!(
                state.dynasties.contains_key(&owner_id),
                "Record Reference Validity: property owner dynasty does not exist"
            );
        }
        if let Some(tenant_id) = property.tenant_dynasty_id {
            debug_assert!(
                state.dynasties.contains_key(&tenant_id),
                "Record Reference Validity: property tenant dynasty does not exist"
            );
        }
        debug_assert!(
            property.tenant_dynasty_id.is_none() || property.owner_dynasty_id.is_some(),
            "Lifecycle Validity: property has a tenant without an owner"
        );
        if let Some(business_id) = property.occupant_business_id {
            debug_assert!(
                property.owner_dynasty_id.is_some(),
                "Ownership Exclusivity: occupied property has no owner"
            );
            debug_assert!(
                occupied_businesses.insert(business_id),
                "Ownership Exclusivity: business occupies more than one property"
            );
            let business = state.businesses.get(business_id);
            debug_assert!(
                business.is_some(),
                "Record Reference Validity: property occupant is missing"
            );
            if let Some(business) = business {
                debug_assert_eq!(
                    business.district_id(),
                    property.district_id,
                    "Ownership Exclusivity: business and occupied property districts differ"
                );
                debug_assert_eq!(
                    business.premises_property_id(),
                    Some(*property_id),
                    "Derived Data Consistency: occupied property and business premises pointers differ"
                );
                let expected_tenant = property
                    .owner_dynasty_id
                    .filter(|owner_id| *owner_id != business.owner_dynasty_id())
                    .map(|_| business.owner_dynasty_id());
                debug_assert_eq!(
                    property.tenant_dynasty_id, expected_tenant,
                    "Derived Data Consistency: occupied property tenancy differs from business ownership"
                );
            }
        }
        validate_property_collateral(state, *property_id, property);
    }
}

/// Collateral-pledge consistency for one property: the loan must reference it
/// back, sit on the owner's books, and still be live.
fn validate_property_collateral(
    state: &AppState,
    property_id: crate::ids::PropertyId,
    property: &crate::core::Property,
) {
    if let Some(loan_id) = property.collateral_loan_id {
        let loan = state.loans.get(&loan_id);
        debug_assert!(
            loan.is_some(),
            "Record Reference Validity: property collateral loan does not exist"
        );
        if let Some(loan) = loan {
            debug_assert_eq!(
                loan.collateral_property_id,
                Some(property_id),
                "Derived Data Consistency: collateral property and loan references differ"
            );
            debug_assert_eq!(
                property.owner_dynasty_id,
                Some(loan.borrower_dynasty_id),
                "Ownership Exclusivity: pledged property is not owned by its borrower"
            );
            debug_assert!(
                !matches!(
                    loan.status,
                    LoanStatus::Defaulted | LoanStatus::Repaid | LoanStatus::WrittenOff
                ),
                "Lifecycle Validity: settled loan retains an active collateral pledge"
            );
        }
    }
}

fn contract_breach_attribution_is_valid(
    state: &AppState,
    contract: &crate::core::SupplyContract,
) -> bool {
    match (
        contract.breaching_dynasty_id,
        contract.breach_victim_dynasty_id,
    ) {
        (None, None) => true,
        // Attribution records the defendant for recoverable breach debt from
        // the first attributable miss, so it may outlive any status.
        (Some(breacher), Some(victim)) => {
            breacher != victim
                && state.dynasties.contains_key(&breacher)
                && state.dynasties.contains_key(&victim)
        }
        (Some(_), None) | (None, Some(_)) => false,
    }
}

fn contract_breach_penalty_is_valid(contract: &crate::core::SupplyContract) -> bool {
    !contract.unpaid_breach_penalty.is_negative()
        && contract.unpaid_breach_penalty <= contract.penalty
        && !contract.collected_breach_penalty.is_negative()
        && contract
            .collected_breach_penalty
            .saturating_add(contract.unpaid_breach_penalty)
            <= contract.penalty
        && (contract.unpaid_breach_penalty == crate::money::Money::ZERO
            || (contract.breaching_dynasty_id.is_some()
                && contract.breach_victim_dynasty_id.is_some()))
}

fn validate_loans(state: &AppState) {
    let mut active_loan_pairs = BTreeSet::new();
    for (loan_id, loan) in &state.loans {
        debug_assert_eq!(
            *loan_id, loan.id,
            "Derived Data Consistency: loan key and record ID differ"
        );
        debug_assert_ne!(
            loan.lender_dynasty_id, loan.borrower_dynasty_id,
            "Ownership Exclusivity: loan lender and borrower must differ"
        );
        debug_assert!(
            state.dynasties.contains_key(&loan.lender_dynasty_id)
                && state.dynasties.contains_key(&loan.borrower_dynasty_id),
            "Record Reference Validity: loan party dynasty does not exist"
        );
        debug_assert!(
            loan.principal > crate::money::Money::ZERO
                && loan.weekly_payment > crate::money::Money::ZERO,
            "Lifecycle Validity: loan principal and payment must remain positive"
        );
        debug_assert!(
            !loan.balance.is_negative(),
            "Lifecycle Validity: loan balance must not be negative"
        );
        debug_assert!(
            loan.interest_basis_points <= 10_000,
            "Lifecycle Validity: loan interest is outside basis-point range"
        );
        debug_assert!(
            is_schedulable_day(loan.next_due_day),
            "Lifecycle Validity: loan due date exceeds the supported timeline"
        );
        if loan.status.is_repayment_active() {
            debug_assert!(
                active_loan_pairs.insert((loan.lender_dynasty_id, loan.borrower_dynasty_id)),
                "Ownership Exclusivity: a lender/borrower pair cannot have multiple repayment-active loans"
            );
        }
        debug_assert!(
            loan.status.has_consistent_arrears(loan.missed_payments),
            "Lifecycle Validity: loan status does not match its missed-payment count"
        );
        match loan.status {
            LoanStatus::Current
            | LoanStatus::Delinquent
            | LoanStatus::Restructured
            | LoanStatus::Defaulted => debug_assert!(
                loan.balance > crate::money::Money::ZERO,
                "Lifecycle Validity: unsettled loan has no remaining balance"
            ),
            LoanStatus::Repaid | LoanStatus::WrittenOff => debug_assert_eq!(
                loan.balance,
                crate::money::Money::ZERO,
                "Lifecycle Validity: settled loan retains a balance"
            ),
        }
        if let Some(property_id) = loan.collateral_property_id {
            let property = state.properties.get(&property_id);
            debug_assert!(
                property.is_some(),
                "Record Reference Validity: loan collateral is missing"
            );
            match loan.status {
                LoanStatus::Current | LoanStatus::Delinquent | LoanStatus::Restructured => {
                    debug_assert_eq!(
                        property.and_then(|property| property.collateral_loan_id),
                        Some(*loan_id),
                        "Derived Data Consistency: active collateral does not reference its loan"
                    );
                    debug_assert_eq!(
                        property.and_then(|property| property.owner_dynasty_id),
                        Some(loan.borrower_dynasty_id),
                        "Ownership Exclusivity: active collateral is not owned by the borrower"
                    );
                }
                LoanStatus::Defaulted | LoanStatus::Repaid | LoanStatus::WrittenOff => {
                    debug_assert!(
                        property.is_some_and(|property| {
                            property.collateral_loan_id != Some(*loan_id)
                        }),
                        "Derived Data Consistency: collateral remains pledged to its settled loan"
                    );
                }
            }
        }
    }
}

fn validate_civic_debts(state: &AppState) {
    let mut authorizing_law_ids = BTreeSet::new();
    for (debt_id, debt) in &state.civic_debts {
        debug_assert_eq!(
            *debt_id, debt.id,
            "Derived Data Consistency: civic debt key and record ID differ"
        );
        debug_assert!(
            state.dynasties.contains_key(&debt.creditor_dynasty_id),
            "Record Reference Validity: civic debt creditor dynasty does not exist"
        );
        debug_assert_ne!(
            debt.sponsor_dynasty_id,
            Some(debt.creditor_dynasty_id),
            "Ownership Exclusivity: civic debt sponsor cannot also be its creditor"
        );
        let authorizing_law = state.laws.get(&debt.authorizing_law_id);
        debug_assert!(
            authorizing_law.is_some_and(|law| {
                law.kind == crate::core::LawKind::PublicDebtAuthorization
                    && law.sponsor_dynasty_id == debt.sponsor_dynasty_id
                    && law.value == debt.principal.copper()
            }),
            "Record Reference Validity: civic debt authorization is missing or inconsistent"
        );
        debug_assert!(
            authorizing_law_ids.insert(debt.authorizing_law_id),
            "Ownership Exclusivity: a consumed public-debt authorization cannot back multiple civic debts"
        );
        debug_assert!(
            debt.sponsor_dynasty_id
                .is_none_or(|dynasty_id| state.dynasties.contains_key(&dynasty_id)),
            "Record Reference Validity: civic debt sponsor dynasty does not exist"
        );
        debug_assert!(
            debt.principal > crate::money::Money::ZERO
                && debt.weekly_payment > crate::money::Money::ZERO,
            "Lifecycle Validity: civic debt principal and payment must remain positive"
        );
        debug_assert!(
            !debt.balance.is_negative() && debt.interest_basis_points <= 10_000,
            "Lifecycle Validity: civic debt balance or interest is invalid"
        );
        debug_assert!(
            is_schedulable_day(debt.next_due_day)
                && debt.issued_day <= state.clock.day()
                && debt.next_due_day >= debt.issued_day,
            "Lifecycle Validity: civic debt dates are invalid"
        );
        match debt.status {
            CivicDebtStatus::Current => debug_assert!(
                debt.balance > crate::money::Money::ZERO && debt.missed_payments == 0,
                "Lifecycle Validity: current civic debt has invalid balance or arrears"
            ),
            CivicDebtStatus::Delinquent => debug_assert!(
                debt.balance > crate::money::Money::ZERO && (1..3).contains(&debt.missed_payments),
                "Lifecycle Validity: delinquent civic debt has invalid balance or arrears"
            ),
            CivicDebtStatus::Defaulted => debug_assert!(
                debt.balance > crate::money::Money::ZERO && debt.missed_payments >= 3,
                "Lifecycle Validity: defaulted civic debt has invalid balance or arrears"
            ),
            CivicDebtStatus::Repaid => {
                debug_assert_eq!(
                    debt.balance,
                    crate::money::Money::ZERO,
                    "Lifecycle Validity: repaid civic debt retains a balance"
                );
                debug_assert_eq!(
                    debt.missed_payments, 0,
                    "Lifecycle Validity: repaid civic debt retains arrears"
                );
            }
        }
    }
}

fn validate_employment(state: &AppState) {
    let mut workers_by_business: BTreeMap<BusinessId, u64> = BTreeMap::new();
    let mut workers_by_household: BTreeMap<HouseholdId, u64> = BTreeMap::new();
    for (employment_id, agreement) in &state.employment {
        debug_assert_eq!(
            *employment_id, agreement.id,
            "Derived Data Consistency: employment key and record ID differ"
        );
        debug_assert!(
            state.businesses.get(agreement.business_id).is_some(),
            "Record Reference Validity: employment business does not exist"
        );
        debug_assert!(
            state.households.get(agreement.household_id).is_some(),
            "Record Reference Validity: employment household does not exist"
        );
        debug_assert!(
            agreement.workers > 0 && agreement.weekly_wage > crate::money::Money::ZERO,
            "Lifecycle Validity: employment workers and wage must remain positive"
        );
        debug_assert!(
            agreement.loyalty_basis_points <= 10_000 && agreement.conditions_basis_points <= 10_000,
            "Lifecycle Validity: employment measures are outside basis-point range"
        );
        if agreement.status != EmploymentStatus::Ended {
            workers_by_business
                .entry(agreement.business_id)
                .and_modify(|workers| *workers += u64::from(agreement.workers))
                .or_insert(u64::from(agreement.workers));
            workers_by_household
                .entry(agreement.household_id)
                .and_modify(|workers| *workers += u64::from(agreement.workers))
                .or_insert(u64::from(agreement.workers));
        }
        if let Some(business) = state.businesses.get(agreement.business_id) {
            debug_assert!(
                super::is_employment_status_compatible(business.status(), agreement.status),
                "Lifecycle Validity: employment status is incompatible with business status"
            );
        }
    }
    for business in state.businesses.iter() {
        let workers = workers_by_business
            .get(&business.id())
            .copied()
            .unwrap_or(0);
        let supported_workers = super::supported_worker_capacity(business);
        debug_assert!(
            workers <= u64::from(supported_workers),
            "Lifecycle Validity: employment exceeds business operating capacity"
        );
    }
    for (household_id, workers) in workers_by_household {
        let members = state
            .households
            .get(household_id)
            .expect("validated employment household must exist")
            .members();
        debug_assert!(
            workers <= u64::from(members),
            "Lifecycle Validity: employment exceeds household labor capacity"
        );
    }
}

fn validate_family_state(state: &AppState) {
    validate_family_links(state);
    validate_family_councils(state);
}

fn validate_family_links(state: &AppState) {
    let mut actively_married_characters = BTreeSet::new();
    let mut active_wards = BTreeSet::new();
    let mut active_player_wards = 0_usize;
    for (link_id, link) in &state.family_links {
        debug_assert_eq!(
            *link_id, link.id,
            "Derived Data Consistency: family link key and record ID differ"
        );
        debug_assert_ne!(
            link.first_character_id, link.second_character_id,
            "Ownership Exclusivity: family link cannot connect a character to itself"
        );
        debug_assert!(
            state.characters.get(link.first_character_id).is_some()
                && state.characters.get(link.second_character_id).is_some(),
            "Record Reference Validity: family link character does not exist"
        );
        if link.active && link.kind == FamilyLinkKind::Marriage {
            debug_assert!(
                actively_married_characters.insert(link.first_character_id)
                    && actively_married_characters.insert(link.second_character_id),
                "Ownership Exclusivity: a character cannot have multiple active marriages"
            );
            debug_assert!(
                state
                    .characters
                    .get(link.first_character_id)
                    .is_some_and(|character| character.status() == CharacterStatus::Active)
                    && state
                        .characters
                        .get(link.second_character_id)
                        .is_some_and(|character| character.status() == CharacterStatus::Active),
                "Lifecycle Validity: active marriages require active participants"
            );
        }
        validate_parent_child_chronology(state, link);
        if matches!(link.kind, FamilyLinkKind::Ward) {
            let first = state
                .characters
                .get(link.first_character_id)
                .expect("validated family link character must exist");
            let second = state
                .characters
                .get(link.second_character_id)
                .expect("validated family link character must exist");
            debug_assert_eq!(
                first.dynasty_id(),
                second.dynasty_id(),
                "Ownership Exclusivity: adoptive and ward links must remain within one dynasty"
            );
            if link.active && link.kind == FamilyLinkKind::Ward {
                debug_assert!(
                    first.status() == CharacterStatus::Active
                        && second.status() == CharacterStatus::Active,
                    "Lifecycle Validity: active ward relationships require active participants"
                );
                debug_assert!(
                    state
                        .family_councils
                        .get(&second.dynasty_id())
                        .is_some_and(|council| council.members.contains(&second.id())),
                    "Index Completeness: an active ward must belong to its dynasty council"
                );
                debug_assert!(
                    active_wards.insert(link.second_character_id),
                    "Ownership Exclusivity: a character cannot have multiple active ward relationships"
                );
                if second.dynasty_id() == state.player_dynasty_id {
                    active_player_wards += 1;
                }
            }
        }
    }
    debug_assert!(
        active_player_wards <= super::MAX_ACTIVE_WARDS,
        "Lifecycle Validity: player dynasty exceeds the active ward limit"
    );
}

fn validate_family_councils(state: &AppState) {
    debug_assert_eq!(
        state.family_councils.len(),
        state.dynasties.len(),
        "Record Reference Validity: every dynasty must have one family council"
    );
    for (dynasty_id, council) in &state.family_councils {
        debug_assert_eq!(
            *dynasty_id, council.dynasty_id,
            "Derived Data Consistency: family council key and dynasty differ"
        );
        debug_assert!(
            state.dynasties.contains_key(dynasty_id),
            "Record Reference Validity: family council dynasty does not exist"
        );
        debug_assert!(
            council.unity_basis_points <= 10_000 && council.charter_version < u64::MAX,
            "Lifecycle Validity: family unity or charter version is invalid"
        );
        let dynasty = state
            .dynasties
            .get(dynasty_id)
            .expect("validated family council dynasty must exist");
        debug_assert!(
            council.members.contains(&dynasty.head_id()),
            "Index Completeness: family council omits the dynasty head"
        );
        if let Some(heir_id) = dynasty.heir_id() {
            debug_assert!(
                council.members.contains(&heir_id),
                "Index Completeness: family council omits the dynasty heir"
            );
        }
        for character_id in &council.members {
            debug_assert!(
                state
                    .characters
                    .get(*character_id)
                    .is_some_and(|character| {
                        character.dynasty_id() == *dynasty_id
                            && character.status() == CharacterStatus::Active
                    }),
                "Lifecycle Validity: family council member must be active and belong to its dynasty"
            );
        }
    }
}

fn validate_parent_child_chronology(state: &AppState, link: &crate::core::FamilyLink) {
    if link.kind != FamilyLinkKind::ParentChild {
        return;
    }
    let parent = state
        .characters
        .get(link.first_character_id)
        .expect("validated family link character must exist");
    let child = state
        .characters
        .get(link.second_character_id)
        .expect("validated family link character must exist");
    debug_assert!(
        child.birth_day().saturating_sub(parent.birth_day())
            >= crate::core::MIN_PARENT_CHILD_AGE_GAP_DAYS,
        "Lifecycle Validity: parent-child family link has impossible chronology"
    );
}

fn validate_laws_and_relationships(state: &AppState) {
    let mut active_law_kinds = Vec::new();
    for (law_id, law) in &state.laws {
        debug_assert_eq!(
            *law_id, law.id,
            "Derived Data Consistency: law key and record ID differ"
        );
        debug_assert!(
            law.enacted_day <= state.clock.day(),
            "No Lost Runtime State: law is enacted after current simulation time"
        );
        debug_assert!(
            law.kind.is_value_valid(law.value),
            "Lifecycle Validity: law value is invalid for its kind"
        );
        debug_assert!(
            !law.active || law.kind.remains_active_after_enactment(),
            "Lifecycle Validity: one-time law authorization remains active after enactment"
        );
        if law.active {
            debug_assert!(
                !active_law_kinds.contains(&law.kind),
                "Ownership Exclusivity: more than one active law exists for the same kind"
            );
            active_law_kinds.push(law.kind);
        }
        if let Some(sponsor_id) = law.sponsor_dynasty_id {
            debug_assert!(
                state.dynasties.contains_key(&sponsor_id),
                "Record Reference Validity: law sponsor dynasty does not exist"
            );
        }
    }
    for (pair, relationship) in &state.relationships {
        debug_assert_eq!(
            *pair, relationship.pair,
            "Derived Data Consistency: relationship key and pair differ"
        );
        debug_assert_ne!(
            pair.first, pair.second,
            "Ownership Exclusivity: relationship pair must contain distinct dynasties"
        );
        debug_assert!(
            state.dynasties.contains_key(&pair.first) && state.dynasties.contains_key(&pair.second),
            "Record Reference Validity: relationship dynasty does not exist"
        );
        debug_assert!(
            relationship.trust_basis_points <= 10_000
                && relationship.fear_basis_points <= 10_000
                && relationship.respect_basis_points <= 10_000
                && relationship.resentment_basis_points <= 10_000,
            "Lifecycle Validity: relationship measure is outside basis-point range"
        );
        debug_assert!(
            relationship.last_interaction_day <= state.clock.day(),
            "No Lost Runtime State: relationship interaction is dated in the future"
        );
        debug_assert!(
            relationship.memories.len() <= super::MAX_RELATIONSHIP_MEMORIES,
            "Lifecycle Validity: relationship history exceeds its retention bound"
        );
        debug_assert!(
            relationship
                .memories
                .iter()
                .all(|memory| !memory.trim().is_empty()),
            "No Lost Runtime State: relationship history contains a blank memory"
        );
    }
    let dynasty_ids: Vec<_> = state.dynasties.keys().copied().collect();
    for (index, left_dynasty_id) in dynasty_ids.iter().enumerate() {
        for right_dynasty_id in dynasty_ids.iter().skip(index + 1) {
            debug_assert!(
                state
                    .relationships
                    .contains_key(&DynastyPair::new(*left_dynasty_id, *right_dynasty_id)),
                "Index Completeness: every distinct dynasty pair requires a relationship record"
            );
        }
    }
}

fn validate_information_and_ai(state: &AppState, ids: &RegistryIds) {
    for (report_id, report) in &state.information_reports {
        debug_assert_eq!(
            *report_id, report.id,
            "Derived Data Consistency: information report key and ID differ"
        );
        debug_assert!(
            state.dynasties.contains_key(&report.owner_dynasty_id),
            "Record Reference Validity: information report owner does not exist"
        );
        debug_assert!(
            super::is_valid_information_report_dates(
                state.clock.day(),
                report.created_day,
                report.expires_day,
            ),
            "Lifecycle Validity: information report dates are invalid"
        );
        debug_assert!(
            !report.subject.trim().is_empty()
                && !report.source.trim().is_empty()
                && !report.summary.trim().is_empty(),
            "No Lost Runtime State: information report lacks user-facing content"
        );
        debug_assert!(
            report.target.is_none_or(|target| match target {
                crate::core::InformationTarget::Market { good_id } => ids.goods.contains(&good_id),
                crate::core::InformationTarget::Counterparty { dynasty_id } => {
                    state.dynasties.contains_key(&dynasty_id)
                }
                crate::core::InformationTarget::District { district_id } => {
                    ids.districts.contains(&district_id)
                }
            }),
            "Record Reference Validity: information report target does not exist"
        );
    }
    let mut pursuing_objectives = BTreeMap::<DynastyId, usize>::new();
    for (objective_id, objective) in &state.ai_objectives {
        debug_assert_eq!(
            *objective_id, objective.id,
            "Derived Data Consistency: AI objective key and ID differ"
        );
        debug_assert!(
            state.dynasties.contains_key(&objective.dynasty_id)
                && objective.dynasty_id != state.player_dynasty_id,
            "Record Reference Validity: AI objective dynasty does not exist or is the player"
        );
        debug_assert!(
            objective.created_day <= state.clock.day(),
            "No Lost Runtime State: AI objective is created in the future"
        );
        debug_assert!(
            !objective.rationale.trim().is_empty(),
            "No Lost Runtime State: AI objective has no rationale"
        );
        if objective.status == crate::core::ObjectiveStatus::Pursuing {
            *pursuing_objectives.entry(objective.dynasty_id).or_default() += 1;
        }
    }
    for dynasty_id in state
        .dynasties
        .keys()
        .copied()
        .filter(|dynasty_id| *dynasty_id != state.player_dynasty_id)
    {
        debug_assert_eq!(
            pursuing_objectives.get(&dynasty_id).copied(),
            Some(1),
            "Lifecycle Validity: every non-player dynasty must have exactly one pursuing AI objective"
        );
    }
}

fn validate_districts_and_public_works(state: &AppState, ids: &RegistryIds) {
    debug_assert_eq!(
        state.districts.len(),
        ids.districts.len(),
        "Registry Reference Validity: every district needs runtime civic state"
    );
    for (district_id, district) in &state.districts {
        debug_assert_eq!(
            *district_id, district.district_id,
            "Derived Data Consistency: district runtime key and ID differ"
        );
        debug_assert!(
            ids.districts.contains(district_id),
            "Registry Reference Validity: district runtime references missing definition"
        );
        debug_assert!(
            district.rent_index_basis_points >= super::MIN_DISTRICT_RENT_INDEX_BASIS_POINTS
                && district.rent_index_basis_points <= super::MAX_DISTRICT_RENT_INDEX_BASIS_POINTS
                && district.employment_basis_points <= 10_000
                && district.sanitation_basis_points <= 10_000
                && district.safety_basis_points <= 10_000
                && district.unrest_basis_points <= 10_000,
            "Lifecycle Validity: district measure is outside basis-point range"
        );
    }
    let mut active_public_works = BTreeSet::new();
    let mut active_player_sponsored_works = 0_usize;
    for (work_id, work) in &state.public_works {
        debug_assert_eq!(
            *work_id, work.id,
            "Derived Data Consistency: public work key and ID differ"
        );
        debug_assert!(
            ids.districts.contains(&work.district_id),
            "Registry Reference Validity: public work district does not exist"
        );
        debug_assert!(
            work.budget > crate::money::Money::ZERO
                && !work.spent.is_negative()
                && work.spent <= work.budget,
            "Lifecycle Validity: public work budget accounting is invalid"
        );
        debug_assert!(
            work.progress_basis_points <= 10_000,
            "Lifecycle Validity: public work progress is outside basis-point range"
        );
        let expected_progress = super::public_work_progress_basis_points(work.spent, work.budget);
        debug_assert_eq!(
            work.progress_basis_points, expected_progress,
            "Derived Data Consistency: public work progress does not match spending"
        );
        if let Some(sponsor_id) = work.sponsor_dynasty_id {
            debug_assert!(
                state.dynasties.contains_key(&sponsor_id),
                "Record Reference Validity: public work sponsor does not exist"
            );
        }
        if work.status.is_unfinished() {
            debug_assert!(
                active_public_works.insert((work.district_id, work.kind)),
                "Ownership Exclusivity: duplicate unfinished public work exists for one district and kind"
            );
            if work.sponsor_dynasty_id == Some(state.player_dynasty_id) {
                active_player_sponsored_works += 1;
            }
        }
        debug_assert_eq!(
            work.status == PublicWorkStatus::Completed,
            work.spent == work.budget,
            "Lifecycle Validity: public work completion status does not match full funding"
        );
    }
    debug_assert!(
        active_player_sponsored_works <= super::MAX_ACTIVE_SPONSORED_PUBLIC_WORKS,
        "Lifecycle Validity: player dynasty exceeds the unfinished public-work sponsorship limit"
    );
}

#[allow(clippy::too_many_lines)]
fn validate_legal_cases(state: &AppState) {
    let mut active_cases = BTreeSet::new();
    let mut litigated_loans = BTreeSet::new();
    let mut litigated_contracts = BTreeSet::new();
    for (case_id, case) in &state.legal_cases {
        debug_assert_eq!(
            *case_id, case.id,
            "Derived Data Consistency: legal case key and ID differ"
        );
        debug_assert_ne!(
            case.plaintiff_dynasty_id, case.defendant_dynasty_id,
            "Ownership Exclusivity: legal case parties must differ"
        );
        debug_assert!(
            state.dynasties.contains_key(&case.plaintiff_dynasty_id)
                && state.dynasties.contains_key(&case.defendant_dynasty_id),
            "Record Reference Validity: legal case party dynasty does not exist"
        );
        debug_assert!(
            case.evidence_basis_points <= 10_000 && case.public_attention_basis_points <= 10_000,
            "Lifecycle Validity: legal case measure is outside basis-point range"
        );
        debug_assert!(
            case.filed_day <= state.clock.day()
                && super::is_valid_legal_hearing_day(case.filed_day, case.hearing_day),
            "Lifecycle Validity: legal case dates are invalid"
        );
        debug_assert!(
            !case.damages.is_negative(),
            "Lifecycle Validity: legal case damages are negative"
        );
        if let Some(claim_source) = case.claim_source {
            debug_assert!(
                match claim_source {
                    LegalClaimSource::Loan { loan_id } => litigated_loans.insert(loan_id),
                    LegalClaimSource::Contract { contract_id } => {
                        litigated_contracts.insert(contract_id)
                    }
                },
                "Ownership Exclusivity: grounded legal claim source is reused by multiple cases"
            );
            match claim_source {
                LegalClaimSource::Loan { loan_id } => {
                    let loan = state.loans.get(&loan_id);
                    debug_assert!(
                        loan.is_some_and(|loan| {
                            case.kind == LegalCaseKind::Debt
                                && loan.lender_dynasty_id == case.plaintiff_dynasty_id
                                && loan.borrower_dynasty_id == case.defendant_dynasty_id
                        }),
                        "Record Reference Validity: debt-case claim source does not match its loan and parties"
                    );
                    if case.status == LegalCaseStatus::DecidedForDefendant {
                        debug_assert!(
                            loan.is_some_and(|loan| loan.status.is_settled()),
                            "Lifecycle Validity: defendant-won debt case retains an enforceable loan"
                        );
                    }
                }
                LegalClaimSource::Contract { contract_id } => {
                    let contract = state.contracts.get(&contract_id);
                    debug_assert!(
                        contract.is_some_and(|contract| {
                            // Recoverable breach debt grounds on the attributed
                            // debt from the first attributable miss, which can
                            // exist while the contract itself is still Active.
                            case.kind == LegalCaseKind::ContractBreach
                                && contract.breaching_dynasty_id == Some(case.defendant_dynasty_id)
                                && contract.breach_victim_dynasty_id
                                    == Some(case.plaintiff_dynasty_id)
                        }),
                        "Record Reference Validity: contract-breach claim source does not match its contract and parties"
                    );
                    if case.status == LegalCaseStatus::DecidedForDefendant {
                        debug_assert!(
                            contract.is_some_and(|contract| {
                                contract.unpaid_breach_penalty == crate::money::Money::ZERO
                            }),
                            "Lifecycle Validity: defendant-won contract case retains an enforceable breach penalty"
                        );
                    }
                }
            }
        }
        if matches!(
            case.status,
            LegalCaseStatus::DecidedForPlaintiff | LegalCaseStatus::DecidedForDefendant
        ) {
            debug_assert!(
                case.hearing_day <= state.clock.day(),
                "Lifecycle Validity: decided legal case hearing is in the future"
            );
        }
        if matches!(
            case.status,
            LegalCaseStatus::Filed | LegalCaseStatus::Hearing
        ) {
            debug_assert!(
                active_cases.insert((
                    case.plaintiff_dynasty_id,
                    case.defendant_dynasty_id,
                    case.kind,
                )),
                "Ownership Exclusivity: duplicate unresolved legal case exists between the same parties"
            );
        }
    }
}

fn validate_routes_and_crises(state: &AppState, ids: &RegistryIds) {
    for (route_id, route) in &state.external_routes {
        debug_assert_eq!(
            *route_id, route.id,
            "Derived Data Consistency: external route key and ID differ"
        );
        debug_assert!(
            !route.name.trim().is_empty(),
            "No Lost Runtime State: external route has a blank name"
        );
        debug_assert!(
            ids.goods.contains(&route.good_id),
            "Registry Reference Validity: external route good does not exist"
        );
        debug_assert!(
            !route.daily_capacity.is_negative()
                && route.risk_basis_points <= 10_000
                && route.disruption_basis_points <= 10_000
                && route.toll_basis_points <= 10_000,
            "Lifecycle Validity: external route values are invalid"
        );
    }
    for (crisis_id, crisis) in &state.crises {
        debug_assert_eq!(
            *crisis_id, crisis.id,
            "Derived Data Consistency: crisis key and ID differ"
        );
        debug_assert!(
            !crisis.cause.trim().is_empty(),
            "No Lost Runtime State: crisis has no recorded cause"
        );
        debug_assert!(
            crisis.started_day <= state.clock.day(),
            "No Lost Runtime State: crisis starts in the future"
        );
        debug_assert!(
            crisis.severity_basis_points <= 10_000,
            "Lifecycle Validity: crisis severity is outside basis-point range"
        );
        if let Some(district_id) = crisis.district_id {
            debug_assert!(
                ids.districts.contains(&district_id),
                "Registry Reference Validity: crisis district does not exist"
            );
        }
        debug_assert!(
            crisis
                .status
                .has_consistent_severity(crisis.severity_basis_points),
            "Lifecycle Validity: crisis status does not match severity"
        );
    }
}

fn validate_outbox(state: &AppState) {
    let mut ids = BTreeSet::new();
    let mut prior_id = None;
    let mut prior_day = i64::MIN;
    for message in &state.outbox {
        debug_assert!(
            ids.insert(message.id),
            "Index Uniqueness: duplicate outbox message ID"
        );
        debug_assert!(
            prior_id.is_none_or(|prior_id| message.id > prior_id),
            "Deterministic Decision Ordering: outbox message IDs are not strictly increasing"
        );
        debug_assert!(
            message.day >= prior_day && message.day <= state.clock.day(),
            "Deterministic Decision Ordering: outbox messages are not chronologically valid"
        );
        debug_assert!(
            !message.subject.trim().is_empty() && !message.body.trim().is_empty(),
            "No Lost Runtime State: outbox message lacks user-facing content"
        );
        prior_id = Some(message.id);
        prior_day = message.day;
    }
}

fn validate_history(state: &AppState) {
    let mut chronicle_ids = BTreeSet::new();
    let mut prior_id = None;
    let mut prior_day = i64::MIN;
    for entry in &state.chronicle {
        debug_assert!(
            chronicle_ids.insert(entry.id()),
            "Index Uniqueness: duplicate chronicle entry ID {}",
            entry.id()
        );
        debug_assert!(
            prior_id.is_none_or(|prior_id| entry.id() > prior_id),
            "Deterministic Decision Ordering: chronicle entry IDs are not strictly increasing"
        );
        debug_assert!(
            entry.day() >= prior_day,
            "Deterministic Decision Ordering: chronicle entries are not ordered by day"
        );
        debug_assert!(
            entry.day() <= state.clock.day(),
            "No Lost Runtime State: chronicle entry is dated after current simulation time"
        );
        debug_assert!(
            !entry.summary().trim().is_empty(),
            "No Lost Runtime State: chronicle entry lacks user-facing content"
        );
        prior_id = Some(entry.id());
        prior_day = entry.day();
    }
    let mut prior_audit_day = i64::MIN;
    for record in &state.audit_log {
        debug_assert!(
            record.day() >= prior_audit_day && record.day() <= state.clock.day(),
            "Deterministic Decision Ordering: audit records are not chronologically valid"
        );
        debug_assert!(
            !record.subject().trim().is_empty() && !record.detail().trim().is_empty(),
            "No Lost Runtime State: audit record lacks diagnostic content"
        );
        validate_audit_record_invariants(state, record);
        prior_audit_day = record.day();
    }
}

fn validate_audit_record_invariants(state: &AppState, record: &crate::core::AuditRecord) {
    if matches!(
        record.kind(),
        AuditKind::InstitutionPatronage
            | AuditKind::InstitutionWithdrawal
            | AuditKind::OfficeNomination
    ) {
        debug_assert!(
            record
                .audit_subject()
                .institution_character_ids()
                .is_some_and(|(institution_id, character_id)| {
                    state.institutions.contains_key(&institution_id)
                        && state.characters.get(character_id).is_some()
                }),
            "Record Reference Validity: institutional audit record has invalid institution/character subject"
        );
    }
    if record.kind() == AuditKind::InstitutionEndowment {
        let subject = record.audit_subject();
        debug_assert!(
            subject.institution_id().is_some_and(|institution_id| {
                state.institutions.contains_key(&institution_id)
                    && subject.dynasty_id().is_some_and(|dynasty_id| {
                        state.dynasties.contains_key(&dynasty_id)
                            && subject.as_str()
                                == format!("institution:{institution_id};dynasty:{dynasty_id}")
                    })
            }),
            "Record Reference Validity: institution endowment audit record has invalid institution/dynasty subject"
        );
    }
    if matches!(
        record.kind(),
        AuditKind::OfficeDirective
            | AuditKind::OfficeDutyShortfall
            | AuditKind::OfficeDutyForfeiture
    ) {
        debug_assert!(
            record
                .audit_subject()
                .institution_id()
                .is_some_and(|institution_id| state.institutions.contains_key(&institution_id)),
            "Record Reference Validity: office audit record has invalid institution subject"
        );
    }
    if matches!(
        record.kind(),
        AuditKind::OfficeDutyShortfall | AuditKind::OfficeDutyForfeiture
    ) {
        let subject = record.audit_subject();
        let valid_subject = subject
            .institution_id()
            .zip(subject.dynasty_id())
            .is_some_and(|(institution_id, dynasty_id)| {
                state.institutions.contains_key(&institution_id)
                    && state.dynasties.contains_key(&dynasty_id)
                    && subject.as_str()
                        == format!("institution:{institution_id};dynasty:{dynasty_id}")
            });
        debug_assert!(
            valid_subject,
            "Record Reference Validity: office duty audit record has invalid subject"
        );
    }
    if record.kind() == AuditKind::OfficeDirective {
        let subject = record.audit_subject();
        let valid_attribution = subject
            .institution_id()
            .zip(subject.dynasty_id())
            .is_some_and(|(institution_id, dynasty_id)| {
                state.institutions.contains_key(&institution_id)
                    && state.dynasties.contains_key(&dynasty_id)
                    && subject.as_str()
                        == format!("institution:{institution_id};dynasty:{dynasty_id}")
            });
        debug_assert!(
            valid_attribution,
            "Record Reference Validity: office directive audit record has invalid dynasty attribution"
        );
    }
}
