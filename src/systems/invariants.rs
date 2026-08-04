//! Debug-only assertions for registry, reference, index, lifecycle, and value invariants.

use crate::core::{
    AppState, Business, CharacterStatus, CivicDebtStatus, ContractStatus, CrisisStatus,
    EmploymentStatus, FamilyLinkKind, LegalCaseStatus, LoanStatus, ObjectiveStatus,
    PublicWorkStatus,
};
use crate::ids::{
    BusinessId, CharacterId, DistrictId, DynastyId, GoodId, HouseholdId, InstitutionId, RecipeId,
};
use crate::registry::{DistrictDef, GoodDef, InstitutionDef, RecipeDef, Registry};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
struct RegistryIds {
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

/// Asserts all cheap runtime invariants in debug builds.
///
/// # Panics
///
/// Panics in debug builds when state contains invalid references, stale indexes, invalid lifecycle
/// combinations, negative constrained values, or inconsistent derived data.
pub fn validate_invariants(registry: &Registry, state: &AppState) {
    let ids = RegistryIds::new(registry);
    debug_assert!(
        state.clock.day() >= 0,
        "Lifecycle Validity: simulation clock cannot be negative"
    );
    debug_assert_eq!(
        state.scenario_key,
        registry.scenario().key(),
        "Registry Reference Validity: state scenario does not match loaded registry"
    );
    debug_assert!(
        state.dynasties.contains_key(&state.player_dynasty_id),
        "Record Reference Validity: player dynasty does not exist"
    );
    debug_assert!(
        state.validate_next_ids().is_ok(),
        "Identifier Allocation: next-ID state is stale or exhausted"
    );

    validate_market(registry, state, &ids);
    validate_characters(state);
    validate_dynasties(state);
    validate_businesses(registry, state, &ids);
    validate_households(state, &ids);
    validate_institutions(state, &ids);
    validate_strategic_state(registry, state, &ids);
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
    }
}

fn validate_characters(state: &AppState) {
    let mut expected_index: BTreeMap<DynastyId, BTreeSet<CharacterId>> = BTreeMap::new();
    for character in state.characters.records().values() {
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
            character.runtime.health_basis_points <= 10_000,
            "Lifecycle Validity: character {} health is outside basis-point range",
            character.id()
        );
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
    for dynasty in state.dynasties.values() {
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
            dynasty.runtime.generation > 0,
            "Lifecycle Validity: dynasty generation must be positive"
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
    let mut administrative_load: BTreeMap<DynastyId, u16> = BTreeMap::new();

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
            .and_modify(|load| *load = load.saturating_add(recipe.administrative_load()))
            .or_insert(recipe.administrative_load());
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

fn validate_administrative_load(state: &AppState, expected: &BTreeMap<DynastyId, u16>) {
    for dynasty in state.dynasties.values() {
        let load = expected.get(&dynasty.id()).copied().unwrap_or(0);
        debug_assert_eq!(
            dynasty.administrative_load(),
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

fn validate_institutions(state: &AppState, ids: &RegistryIds) {
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
        debug_assert!(
            !institution.budget.is_negative()
                && institution.term_number > 0
                && institution.term_started_day <= state.clock.day()
                && institution.next_selection_day >= institution.term_started_day,
            "Lifecycle Validity: institution budget or term timing is invalid"
        );
        debug_assert!(
            institution.legitimacy_basis_points <= 10_000,
            "Lifecycle Validity: institution legitimacy is outside basis-point range"
        );
        for member_id in &institution.members {
            debug_assert!(
                state
                    .characters
                    .get(*member_id)
                    .is_some_and(|character| character.status() == CharacterStatus::Active),
                "Lifecycle Validity: institution member must exist and be active"
            );
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
    validate_information_and_ai(state);
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
        debug_assert!(
            ids.goods.contains(&contract.good_id),
            "Registry Reference Validity: contract good does not exist"
        );
        debug_assert!(
            contract.quantity_per_week > crate::money::Quantity::ZERO,
            "Lifecycle Validity: contract quantity must remain positive"
        );
        debug_assert!(
            contract.unit_price > crate::money::Money::ZERO,
            "Lifecycle Validity: contract price must remain positive"
        );
        debug_assert!(
            !contract.penalty.is_negative(),
            "Lifecycle Validity: contract penalty must not be negative"
        );
        debug_assert!(
            contract.next_due_day <= contract.end_day || contract.status != ContractStatus::Active,
            "Lifecycle Validity: active contract due date exceeds its term"
        );
        debug_assert!(
            contract.breaching_dynasty_id.is_none_or(|dynasty_id| {
                contract.status == ContractStatus::Breached
                    && state.dynasties.contains_key(&dynasty_id)
            }),
            "Lifecycle Validity: contract breach attribution is inconsistent with its parties"
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
            ids.districts.contains(&property.district_id),
            "Registry Reference Validity: property district does not exist"
        );
        debug_assert!(
            !property.value.is_negative() && !property.weekly_rent.is_negative(),
            "Lifecycle Validity: property value and rent must be nonnegative"
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
        if let Some(loan_id) = property.collateral_loan_id {
            let loan = state.loans.get(&loan_id);
            debug_assert!(
                loan.is_some(),
                "Record Reference Validity: property collateral loan does not exist"
            );
            if let Some(loan) = loan {
                debug_assert_eq!(
                    loan.collateral_property_id,
                    Some(*property_id),
                    "Derived Data Consistency: collateral property and loan references differ"
                );
                debug_assert_eq!(
                    property.owner_dynasty_id,
                    Some(loan.borrower_dynasty_id),
                    "Ownership Exclusivity: pledged property is not owned by its borrower"
                );
                debug_assert!(
                    !matches!(loan.status, LoanStatus::Defaulted | LoanStatus::Repaid),
                    "Lifecycle Validity: settled loan retains an active collateral pledge"
                );
            }
        }
    }
}

fn validate_loans(state: &AppState) {
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
        match loan.status {
            LoanStatus::Current
            | LoanStatus::Delinquent
            | LoanStatus::Restructured
            | LoanStatus::Defaulted => debug_assert!(
                loan.balance > crate::money::Money::ZERO,
                "Lifecycle Validity: unsettled loan has no remaining balance"
            ),
            LoanStatus::Repaid => debug_assert_eq!(
                loan.balance,
                crate::money::Money::ZERO,
                "Lifecycle Validity: repaid loan retains a balance"
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
                LoanStatus::Defaulted => {
                    debug_assert!(
                        property.is_some_and(|property| {
                            property.collateral_loan_id != Some(*loan_id)
                        }),
                        "Derived Data Consistency: defaulted collateral remains pledged to its settled loan"
                    );
                }
                LoanStatus::Repaid => {
                    debug_assert!(
                        property.is_some_and(|property| {
                            property.collateral_loan_id != Some(*loan_id)
                        }),
                        "Derived Data Consistency: repaid collateral remains pledged to its settled loan"
                    );
                }
            }
        }
    }
}

fn validate_civic_debts(state: &AppState) {
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
            debt.issued_day <= state.clock.day() && debt.next_due_day >= debt.issued_day,
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
    let mut workers_by_business: BTreeMap<BusinessId, u32> = BTreeMap::new();
    let mut workers_by_household: BTreeMap<HouseholdId, u32> = BTreeMap::new();
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
                .and_modify(|workers| {
                    *workers = workers.saturating_add(u32::from(agreement.workers));
                })
                .or_insert(u32::from(agreement.workers));
            workers_by_household
                .entry(agreement.household_id)
                .and_modify(|workers| {
                    *workers = workers.saturating_add(u32::from(agreement.workers));
                })
                .or_insert(u32::from(agreement.workers));
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
            workers <= supported_workers,
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
            workers <= u32::from(members),
            "Lifecycle Validity: employment exceeds household labor capacity"
        );
    }
}

fn validate_family_state(state: &AppState) {
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
        debug_assert!(
            link.property_claim_basis_points <= 10_000,
            "Lifecycle Validity: family property claim is outside basis-point range"
        );
        if matches!(link.kind, FamilyLinkKind::Adoptive | FamilyLinkKind::Ward) {
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
                    state
                        .family_councils
                        .get(&second.dynasty_id())
                        .is_some_and(|council| council.members.contains(&second.id())),
                    "Index Completeness: an active ward must belong to its dynasty council"
                );
            }
        }
    }
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
            council.unity_basis_points <= 10_000,
            "Lifecycle Validity: family unity is outside basis-point range"
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
    }
}

fn validate_information_and_ai(state: &AppState) {
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
            report.created_day <= state.clock.day() && report.expires_day >= report.created_day,
            "Lifecycle Validity: information report dates are invalid"
        );
    }
    for (objective_id, objective) in &state.ai_objectives {
        debug_assert_eq!(
            *objective_id, objective.id,
            "Derived Data Consistency: AI objective key and ID differ"
        );
        debug_assert!(
            state.dynasties.contains_key(&objective.dynasty_id),
            "Record Reference Validity: AI objective dynasty does not exist"
        );
        if let Some(target_id) = objective.target_dynasty_id {
            debug_assert!(
                state.dynasties.contains_key(&target_id),
                "Record Reference Validity: AI objective target dynasty does not exist"
            );
        }
        debug_assert!(
            objective.created_day <= state.clock.day(),
            "No Lost Runtime State: AI objective is created in the future"
        );
        if objective.status == ObjectiveStatus::Achieved {
            debug_assert!(
                !objective.rationale.is_empty(),
                "No Lost Runtime State: achieved AI objective has no rationale"
            );
        }
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
        let mut support_ids = BTreeSet::new();
        for (dynasty_id, support) in &district.dynasty_support {
            debug_assert!(
                support_ids.insert(*dynasty_id),
                "Index Uniqueness: duplicate district dynasty support entry"
            );
            debug_assert!(
                state.dynasties.contains_key(dynasty_id) && *support <= 10_000,
                "Record Reference Validity: invalid district dynasty support entry"
            );
        }
    }
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
        let expected_progress = u16::try_from(
            work.spent
                .saturating_mul_ratio(10_000, work.budget.copper())
                .copper()
                .clamp(0, 10_000),
        )
        .expect("clamped public-work progress must fit u16");
        debug_assert_eq!(
            work.progress_basis_points, expected_progress,
            "Derived Data Consistency: public work progress does not match spending"
        );
        if work.status == PublicWorkStatus::Completed {
            debug_assert_eq!(
                work.spent, work.budget,
                "Lifecycle Validity: completed public work is not fully funded"
            );
        }
        if let Some(sponsor_id) = work.sponsor_dynasty_id {
            debug_assert!(
                state.dynasties.contains_key(&sponsor_id),
                "Record Reference Validity: public work sponsor does not exist"
            );
        }
    }
}

fn validate_legal_cases(state: &AppState) {
    let mut active_cases = BTreeSet::new();
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
            case.filed_day <= state.clock.day() && case.hearing_day >= case.filed_day,
            "Lifecycle Validity: legal case dates are invalid"
        );
        debug_assert!(
            !case.damages.is_negative(),
            "Lifecycle Validity: legal case damages are negative"
        );
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
        if crisis.status == CrisisStatus::Resolved {
            debug_assert!(
                crisis.severity_basis_points < 500,
                "Lifecycle Validity: resolved crisis remains severe"
            );
        }
    }
}

fn validate_outbox(state: &AppState) {
    let mut ids = BTreeSet::new();
    let mut prior_day = i64::MIN;
    for message in &state.outbox {
        debug_assert!(
            ids.insert(message.id),
            "Index Uniqueness: duplicate outbox message ID"
        );
        debug_assert!(
            message.day >= prior_day && message.day <= state.clock.day(),
            "Deterministic Decision Ordering: outbox messages are not chronologically valid"
        );
        debug_assert!(
            !message.subject.is_empty() && !message.body.is_empty(),
            "No Lost Runtime State: outbox message lacks user-facing content"
        );
        prior_day = message.day;
    }
}

fn validate_history(state: &AppState) {
    let mut chronicle_ids = BTreeSet::new();
    let mut prior_day = i64::MIN;
    for entry in &state.chronicle {
        debug_assert!(
            chronicle_ids.insert(entry.id()),
            "Index Uniqueness: duplicate chronicle entry ID {}",
            entry.id()
        );
        debug_assert!(
            entry.day() >= prior_day,
            "Deterministic Decision Ordering: chronicle entries are not ordered by day"
        );
        debug_assert!(
            entry.day() <= state.clock.day(),
            "No Lost Runtime State: chronicle entry is dated after current simulation time"
        );
        prior_day = entry.day();
    }
    let mut prior_audit_day = i64::MIN;
    for record in &state.audit_log {
        debug_assert!(
            record.day() >= prior_audit_day && record.day() <= state.clock.day(),
            "Deterministic Decision Ordering: audit records are not chronologically valid"
        );
        prior_audit_day = record.day();
    }
}
