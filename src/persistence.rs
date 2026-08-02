//! JSON persistence adapter with explicit schema migration and contextual errors.

use crate::core::{AppState, CURRENT_SCHEMA_VERSION};
use crate::ids::{BusinessId, HouseholdId};
use crate::money::{Money, Quantity};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::Builder;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateValidationKind {
    Schema,
    Scenario,
    DefinitionReferences,
    PrimaryRecords,
    StrategicRecords,
    NumericRanges,
    IdentifierAllocation,
}

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("failed to create save directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create temporary save beside {path}: {source}")]
    CreateTemporary {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize application state: {source}")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write save file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read save file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse save file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("save file {path} has no numeric schema_version")]
    MissingSchemaVersion { path: PathBuf },
    #[error("save schema version {found} is newer than supported version {supported}")]
    FutureSchema { found: u32, supported: u32 },
    #[error("save schema version {version} has no migration path")]
    UnsupportedSchema { version: u32 },
    #[error("schema migration from version {version} failed: {reason}")]
    Migration { version: u32, reason: String },
    #[error("save file {path} contains invalid {kind:?} state: {reason}")]
    InvalidState {
        path: PathBuf,
        kind: StateValidationKind,
        reason: String,
    },
}

#[derive(Debug)]
struct StateValidationError {
    kind: StateValidationKind,
    reason: String,
}

impl StateValidationError {
    fn new(kind: StateValidationKind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            reason: reason.into(),
        }
    }
}

/// Serializes the complete application state to a JSON save file.
///
/// # Errors
///
/// Returns an error when the parent directory or temporary file cannot be created, serialization
/// fails, or the destination cannot be atomically replaced.
pub fn save_state(path: impl AsRef<Path>, state: &AppState) -> Result<(), PersistenceError> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if parent != Path::new(".") {
        fs::create_dir_all(parent).map_err(|source| PersistenceError::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|source| PersistenceError::Serialize { source })?;
    let prefix = path
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| ".campaign-save-".to_owned(), |name| format!(".{name}."));
    let mut temporary = Builder::new()
        .prefix(&prefix)
        .tempfile_in(parent)
        .map_err(|source| PersistenceError::CreateTemporary {
            path: path.to_path_buf(),
            source,
        })?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|source| PersistenceError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    temporary
        .persist(path)
        .map_err(|error| PersistenceError::Write {
            path: path.to_path_buf(),
            source: error.error,
        })?;
    Ok(())
}

/// Loads, migrates, and deserializes a JSON save file.
///
/// # Errors
///
/// Returns an error when the file cannot be read, parsed, migrated, or deserialized.
pub fn load_state(path: impl AsRef<Path>) -> Result<AppState, PersistenceError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| PersistenceError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|source| PersistenceError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    let source_schema_version = read_schema_version(&value, path)?;
    let legacy_officeholders = if source_schema_version < 2 {
        Some(read_legacy_officeholders(&value, source_schema_version)?)
    } else {
        None
    };
    let migrated = migrate_to_current(value, path)?;
    let mut state: AppState =
        serde_json::from_value(migrated).map_err(|source| PersistenceError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    hydrate_strategic_state(&mut state, source_schema_version < 2);
    if let Some(officeholders) = legacy_officeholders {
        restore_legacy_officeholders(&mut state, officeholders);
    }
    validate_loaded_state(&state).map_err(|error| PersistenceError::InvalidState {
        path: path.to_path_buf(),
        kind: error.kind,
        reason: error.reason,
    })?;
    Ok(state)
}

fn read_legacy_officeholders(
    value: &Value,
    version: u32,
) -> Result<BTreeMap<crate::ids::InstitutionId, Option<crate::ids::CharacterId>>, PersistenceError>
{
    let institutions = value
        .get("institutions")
        .and_then(Value::as_object)
        .ok_or_else(|| PersistenceError::Migration {
            version,
            reason: "legacy save institutions must be an object".to_owned(),
        })?;
    institutions
        .values()
        .map(|institution| {
            let institution_id = institution
                .get("institution_id")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .map(crate::ids::InstitutionId::new)
                .ok_or_else(|| PersistenceError::Migration {
                    version,
                    reason: "legacy institution has an invalid institution_id".to_owned(),
                })?;
            let office_holder_id = institution
                .get("office_holder_id")
                .filter(|value| !value.is_null())
                .map(|value| {
                    value
                        .as_u64()
                        .and_then(|value| u32::try_from(value).ok())
                        .map(crate::ids::CharacterId::new)
                        .ok_or_else(|| PersistenceError::Migration {
                            version,
                            reason: format!(
                                "legacy institution {institution_id} has an invalid officeholder"
                            ),
                        })
                })
                .transpose()?;
            Ok((institution_id, office_holder_id))
        })
        .collect()
}

fn restore_legacy_officeholders(
    state: &mut AppState,
    officeholders: BTreeMap<crate::ids::InstitutionId, Option<crate::ids::CharacterId>>,
) {
    for (institution_id, office_holder_id) in officeholders {
        if let Some(institution) = state.institutions.get_mut(&institution_id) {
            if let Some(character_id) = office_holder_id {
                institution.members.insert(character_id);
            }
            institution.office_holder_id = office_holder_id;
        }
    }
}

fn validate_loaded_state(state: &AppState) -> Result<(), StateValidationError> {
    if state.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(StateValidationError::new(
            StateValidationKind::Schema,
            format!(
                "state schema {} does not match current schema {CURRENT_SCHEMA_VERSION}",
                state.schema_version
            ),
        ));
    }
    if state.scenario_key != "rivergate" {
        return Err(StateValidationError::new(
            StateValidationKind::Scenario,
            format!("unsupported scenario key {:?}", state.scenario_key),
        ));
    }
    let registry = crate::registry::build_rivergate_registry();
    validate_definition_references(&registry, state).map_err(|reason| {
        StateValidationError::new(StateValidationKind::DefinitionReferences, reason)
    })?;
    validate_primary_records(&registry, state)
        .map_err(|reason| StateValidationError::new(StateValidationKind::PrimaryRecords, reason))?;
    validate_strategic_records(&registry, state).map_err(|reason| {
        StateValidationError::new(StateValidationKind::StrategicRecords, reason)
    })?;
    validate_numeric_ranges(state)
        .map_err(|reason| StateValidationError::new(StateValidationKind::NumericRanges, reason))?;
    state.validate_next_ids().map_err(|reason| {
        StateValidationError::new(StateValidationKind::IdentifierAllocation, reason)
    })
}

fn validate_numeric_ranges(state: &AppState) -> Result<(), String> {
    validate_core_numeric_ranges(state)?;
    validate_financial_numeric_ranges(state)?;
    validate_civic_numeric_ranges(state)
}

fn validate_core_numeric_ranges(state: &AppState) -> Result<(), String> {
    if state.clock.day() < 0 {
        return Err(format!(
            "simulation clock has negative elapsed day {}",
            state.clock.day()
        ));
    }
    for dynasty in state.dynasties.values() {
        if dynasty.treasury() < Money::ZERO
            || dynasty.resources.legitimacy_basis_points > 10_000
            || dynasty.resources.reputation_quality_basis_points > 10_000
            || dynasty.resources.reputation_reliability_basis_points > 10_000
            || dynasty.runtime.succession_risk_basis_points > 10_000
        {
            return Err(format!(
                "dynasty {} has an invalid resource value",
                dynasty.id()
            ));
        }
    }
    for character in state.characters.iter() {
        if character.birth_day() > state.clock.day()
            || character.runtime.health_basis_points > 10_000
            || character.runtime.loyalty_basis_points > 10_000
            || character.capabilities.administration > 100
            || character.capabilities.commerce > 100
            || character.capabilities.social > 100
            || character.capabilities.craft > 100
        {
            return Err(format!(
                "character {} has an invalid birth date, capability, or basis-point value",
                character.id()
            ));
        }
    }
    for household in state.households.iter() {
        if household.members == 0
            || household.cash() < Money::ZERO
            || household.weekly_income < Money::ZERO
            || household.bread_need_daily < Quantity::ZERO
            || household.ale_need_daily < Quantity::ZERO
            || household.food_satisfaction_basis_points() > 10_000
        {
            return Err(format!(
                "household {} has an invalid economic value",
                household.id()
            ));
        }
    }
    for business in state.businesses.iter() {
        if business.cash() < Money::ZERO
            || business.operations.capacity_batches_per_day == 0
            || business.operations.condition_basis_points > 10_000
            || business.operations.quality_basis_points > 10_000
            || business.policy.target_input_days > 30
            || business.policy.target_output_days > 30
            || business.policy.minimum_cash_reserve < Money::ZERO
            || business.policy.maintenance_basis_points > 10_000
            || business.policy.quality_target_basis_points > 10_000
            || business
                .inventory()
                .values()
                .any(|quantity| *quantity < Quantity::ZERO)
        {
            return Err(format!(
                "business {} has an invalid economic value",
                business.id()
            ));
        }
    }
    for quote in state.market.quotes.values() {
        if quote.price <= Money::ZERO
            || quote.previous_price <= Money::ZERO
            || quote.stock < Quantity::ZERO
            || quote.target_stock <= Quantity::ZERO
            || quote.demand_today < Quantity::ZERO
            || quote.supply_today < Quantity::ZERO
        {
            return Err(format!(
                "market quote {} has an invalid value",
                quote.good_id()
            ));
        }
    }
    Ok(())
}

fn validate_financial_numeric_ranges(state: &AppState) -> Result<(), String> {
    for loan in state.loans.values() {
        if loan.principal <= Money::ZERO
            || loan.balance < Money::ZERO
            || loan.weekly_payment <= Money::ZERO
            || loan.interest_basis_points > 10_000
        {
            return Err(format!("loan {} has an invalid financial value", loan.id));
        }
    }
    for property in state.properties.values() {
        if property.value < Money::ZERO
            || property.weekly_rent < Money::ZERO
            || property.condition_basis_points > 10_000
        {
            return Err(format!(
                "property {} has an invalid financial value",
                property.id
            ));
        }
    }
    for agreement in state.employment.values() {
        if agreement.weekly_wage <= Money::ZERO
            || agreement.workers == 0
            || agreement.conditions_basis_points > 10_000
            || agreement.loyalty_basis_points > 10_000
        {
            return Err(format!(
                "employment agreement {} has an invalid financial value",
                agreement.id
            ));
        }
    }
    for contract in state.contracts.values() {
        if contract.quantity_per_week <= Quantity::ZERO
            || contract.unit_price <= Money::ZERO
            || contract.penalty < Money::ZERO
        {
            return Err(format!(
                "supply contract {} has an invalid financial value",
                contract.id
            ));
        }
    }
    for institution in state.institutions.values() {
        if institution.budget < Money::ZERO || institution.legitimacy_basis_points > 10_000 {
            return Err(format!(
                "institution {} has an invalid financial value",
                institution.institution_id
            ));
        }
    }
    Ok(())
}

fn validate_civic_numeric_ranges(state: &AppState) -> Result<(), String> {
    for link in state.family_links.values() {
        if link.property_claim_basis_points > 10_000 {
            return Err(format!(
                "family link {} has an invalid property claim",
                link.id
            ));
        }
    }
    for council in state.family_councils.values() {
        if council.unity_basis_points > 10_000 {
            return Err(format!(
                "family council {} has invalid unity",
                council.dynasty_id
            ));
        }
    }
    for district in state.districts.values() {
        if district.employment_basis_points > 10_000
            || district.sanitation_basis_points > 10_000
            || district.safety_basis_points > 10_000
            || district.unrest_basis_points > 10_000
            || district
                .dynasty_support
                .iter()
                .any(|(_, support)| *support > 10_000)
        {
            return Err(format!(
                "district {} has an invalid basis-point value",
                district.district_id
            ));
        }
    }
    for work in state.public_works.values() {
        if work.budget <= Money::ZERO
            || work.spent < Money::ZERO
            || work.spent > work.budget
            || work.progress_basis_points > 10_000
            || (work.status == crate::core::PublicWorkStatus::Completed
                && work.progress_basis_points != 10_000)
        {
            return Err(format!(
                "public work {} has an invalid progress value",
                work.id
            ));
        }
    }
    for route in state.external_routes.values() {
        if route.daily_capacity < Quantity::ZERO
            || route.risk_basis_points > 10_000
            || route.disruption_basis_points > 10_000
            || route.toll_basis_points > 10_000
        {
            return Err(format!("external route {} has an invalid value", route.id));
        }
    }
    for crisis in state.crises.values() {
        if crisis.severity_basis_points > 10_000 {
            return Err(format!("crisis {} has an invalid severity", crisis.id));
        }
    }
    for legal_case in state.legal_cases.values() {
        if legal_case.evidence_basis_points > 10_000
            || legal_case.public_attention_basis_points > 10_000
            || legal_case.damages < Money::ZERO
        {
            return Err(format!(
                "legal case {} has an invalid measure or damages value",
                legal_case.id
            ));
        }
    }
    for relationship in state.relationships.values() {
        if relationship.trust_basis_points > 10_000
            || relationship.fear_basis_points > 10_000
            || relationship.respect_basis_points > 10_000
            || relationship.resentment_basis_points > 10_000
        {
            return Err("relationship contains an invalid basis-point value".to_owned());
        }
    }
    Ok(())
}

fn validate_definition_references(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), String> {
    let expected_goods: BTreeSet<_> = registry
        .goods()
        .iter()
        .map(crate::registry::GoodDef::id)
        .collect();
    let actual_goods: BTreeSet<_> = state.market.quotes.keys().copied().collect();
    if actual_goods != expected_goods {
        return Err("market quote IDs do not match the scenario registry".to_owned());
    }
    if state
        .market
        .quotes
        .iter()
        .any(|(good_id, quote)| quote.good_id() != *good_id)
    {
        return Err("market quote map key differs from its record ID".to_owned());
    }
    let expected_districts: BTreeSet<_> = registry
        .districts()
        .iter()
        .map(crate::registry::DistrictDef::id)
        .collect();
    let actual_districts: BTreeSet<_> = state.districts.keys().copied().collect();
    if actual_districts != expected_districts {
        return Err("district runtime IDs do not match the scenario registry".to_owned());
    }
    if state
        .districts
        .iter()
        .any(|(district_id, district)| district.district_id != *district_id)
    {
        return Err("district runtime map key differs from its record ID".to_owned());
    }
    for district in state.districts.values() {
        let mut supported_dynasties = BTreeSet::new();
        for (dynasty_id, _) in &district.dynasty_support {
            if !state.dynasties.contains_key(dynasty_id) {
                return Err(format!(
                    "district {} support references missing dynasty {dynasty_id}",
                    district.district_id
                ));
            }
            if !supported_dynasties.insert(*dynasty_id) {
                return Err(format!(
                    "district {} contains duplicate support for dynasty {dynasty_id}",
                    district.district_id
                ));
            }
        }
    }
    let expected_institutions: BTreeSet<_> = registry
        .institutions()
        .iter()
        .map(crate::registry::InstitutionDef::id)
        .collect();
    let actual_institutions: BTreeSet<_> = state.institutions.keys().copied().collect();
    if actual_institutions != expected_institutions {
        return Err("institution state IDs do not match the scenario registry".to_owned());
    }
    Ok(())
}

fn validate_primary_records(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), String> {
    if !state.dynasties.contains_key(&state.player_dynasty_id) {
        return Err("player dynasty does not exist".to_owned());
    }
    for (dynasty_id, dynasty) in &state.dynasties {
        if dynasty.id() != *dynasty_id {
            return Err(format!(
                "dynasty map key {dynasty_id} differs from record ID"
            ));
        }
        if dynasty.heir_id() == Some(dynasty.head_id()) {
            return Err(format!(
                "dynasty {dynasty_id} uses the same character as head and heir"
            ));
        }
        for character_id in [Some(dynasty.head_id()), dynasty.heir_id()]
            .into_iter()
            .flatten()
        {
            let character = state.characters.get(character_id).ok_or_else(|| {
                format!("dynasty {dynasty_id} references missing character {character_id}")
            })?;
            if character.dynasty_id() != *dynasty_id {
                return Err(format!(
                    "dynasty {dynasty_id} references character {character_id} from another dynasty"
                ));
            }
            if character.status() != crate::core::CharacterStatus::Active {
                return Err(format!(
                    "dynasty {dynasty_id} references inactive head or heir {character_id}"
                ));
            }
            let expected_role = if character_id == dynasty.head_id() {
                crate::core::CharacterRole::HeadOfHouse
            } else {
                crate::core::CharacterRole::Heir
            };
            if character.role() != expected_role {
                return Err(format!(
                    "dynasty {dynasty_id} head or heir {character_id} has the wrong role"
                ));
            }
        }
    }

    let mut character_index: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for (character_id, character) in state.characters.records() {
        if character.id() != *character_id || !state.dynasties.contains_key(&character.dynasty_id())
        {
            return Err(format!(
                "character {character_id} has an invalid identity reference"
            ));
        }
        character_index
            .entry(character.dynasty_id())
            .or_default()
            .insert(*character_id);
    }
    if &character_index != state.characters.index() {
        return Err("character dynasty index is stale or incomplete".to_owned());
    }

    let mut household_index: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for (household_id, household) in state.households.records() {
        if household.id() != *household_id
            || registry.get_district(household.district_id()).is_none()
        {
            return Err(format!(
                "household {household_id} has an invalid identity reference"
            ));
        }
        household_index
            .entry(household.district_id())
            .or_default()
            .insert(*household_id);
    }
    if &household_index != state.households.index() {
        return Err("household district index is stale or incomplete".to_owned());
    }

    validate_business_records(registry, state)
}

fn validate_business_records(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), String> {
    let mut owner_index: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    let mut district_index: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    let mut administrative_load = BTreeMap::<_, u16>::new();
    for (business_id, business) in state.businesses.records() {
        let owner_id = business.owner_dynasty_id();
        if business.id() != *business_id
            || !state.dynasties.contains_key(&owner_id)
            || registry.get_district(business.district_id()).is_none()
            || registry.get_recipe(business.recipe_id()).is_none()
        {
            return Err(format!(
                "business {business_id} has an invalid definition reference"
            ));
        }
        let manager = state
            .characters
            .get(business.manager_id())
            .ok_or_else(|| format!("business {business_id} references a missing manager"))?;
        if manager.dynasty_id() != owner_id {
            return Err(format!(
                "business {business_id} manager belongs to another dynasty"
            ));
        }
        if manager.status() != crate::core::CharacterStatus::Active {
            return Err(format!(
                "business {business_id} references an inactive manager"
            ));
        }
        if business
            .inventory()
            .keys()
            .any(|good_id| registry.get_good(*good_id).is_none())
        {
            return Err(format!(
                "business {business_id} contains an unknown inventory good"
            ));
        }
        owner_index
            .entry(owner_id)
            .or_default()
            .insert(*business_id);
        district_index
            .entry(business.district_id())
            .or_default()
            .insert(*business_id);
        let recipe = registry
            .get_recipe(business.recipe_id())
            .expect("validated business recipe must exist");
        administrative_load
            .entry(owner_id)
            .and_modify(|load| *load = load.saturating_add(recipe.administrative_load()))
            .or_insert(recipe.administrative_load());
    }
    if &owner_index != state.businesses.owner_index()
        || &district_index != state.businesses.district_index()
    {
        return Err("business ownership or district index is stale or incomplete".to_owned());
    }
    for dynasty in state.dynasties.values() {
        let expected = administrative_load.get(&dynasty.id()).copied().unwrap_or(0);
        if dynasty.administrative_load() != expected {
            return Err(format!(
                "dynasty {} administrative load {} does not match derived load {expected}",
                dynasty.id(),
                dynasty.administrative_load()
            ));
        }
    }
    Ok(())
}

fn validate_strategic_records(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), String> {
    for (contract_id, contract) in &state.contracts {
        let buyer = state.businesses.get(contract.buyer_business_id);
        let seller = state.businesses.get(contract.seller_business_id);
        if contract.id != *contract_id
            || contract.buyer_business_id == contract.seller_business_id
            || buyer.is_none()
            || seller.is_none()
            || !state.market.quotes.contains_key(&contract.good_id)
        {
            return Err(format!(
                "supply contract {contract_id} has an invalid reference"
            ));
        }
        let buyer_recipe = registry
            .get_recipe(
                buyer
                    .expect("validated contract buyer must exist")
                    .recipe_id(),
            )
            .expect("validated business recipe must exist");
        let seller_recipe = registry
            .get_recipe(
                seller
                    .expect("validated contract seller must exist")
                    .recipe_id(),
            )
            .expect("validated business recipe must exist");
        if seller_recipe.output_good_id() != contract.good_id
            || !buyer_recipe
                .inputs()
                .iter()
                .any(|input| input.good_id() == contract.good_id)
            || (contract.status == crate::core::ContractStatus::Active
                && contract.end_day < contract.next_due_day.saturating_sub(7))
        {
            return Err(format!(
                "supply contract {contract_id} is incompatible with its parties or term"
            ));
        }
    }
    let mut occupied_businesses = BTreeSet::new();
    for (property_id, property) in &state.properties {
        if property.id != *property_id || !state.districts.contains_key(&property.district_id) {
            return Err(format!(
                "property {property_id} has an invalid identity reference"
            ));
        }
        for dynasty_id in [property.owner_dynasty_id, property.tenant_dynasty_id]
            .into_iter()
            .flatten()
        {
            if !state.dynasties.contains_key(&dynasty_id) {
                return Err(format!(
                    "property {property_id} references a missing dynasty"
                ));
            }
        }
        if let Some(business_id) = property.occupant_business_id {
            let business = state.businesses.get(business_id).ok_or_else(|| {
                format!("property {property_id} references a missing occupant business")
            })?;
            if !occupied_businesses.insert(business_id)
                || business.district_id() != property.district_id
            {
                return Err(format!(
                    "property {property_id} has an invalid or duplicate occupant"
                ));
            }
        }
        if let Some(loan_id) = property.collateral_loan_id {
            let loan = state
                .loans
                .get(&loan_id)
                .ok_or_else(|| format!("property {property_id} references a missing loan"))?;
            if loan.collateral_property_id != Some(*property_id)
                || matches!(
                    loan.status,
                    crate::core::LoanStatus::Defaulted | crate::core::LoanStatus::Repaid
                )
                || property.owner_dynasty_id != Some(loan.borrower_dynasty_id)
            {
                return Err(format!(
                    "property {property_id} has an invalid collateral relationship"
                ));
            }
        }
    }
    validate_finance_and_organization_records(state)
}

fn validate_finance_and_organization_records(state: &AppState) -> Result<(), String> {
    validate_loan_records(state)?;
    validate_employment_records(state)?;
    validate_family_records(state)?;
    validate_institution_and_misc_records(state)
}

fn validate_loan_records(state: &AppState) -> Result<(), String> {
    for (loan_id, loan) in &state.loans {
        if loan.id != *loan_id
            || loan.lender_dynasty_id == loan.borrower_dynasty_id
            || !state.dynasties.contains_key(&loan.lender_dynasty_id)
            || !state.dynasties.contains_key(&loan.borrower_dynasty_id)
        {
            return Err(format!("loan {loan_id} has an invalid dynasty reference"));
        }
        if loan.status == crate::core::LoanStatus::Repaid && loan.balance != Money::ZERO {
            return Err(format!("repaid loan {loan_id} retains a balance"));
        }
        if let Some(property_id) = loan.collateral_property_id {
            let property = state.properties.get(&property_id).ok_or_else(|| {
                format!("loan {loan_id} references a missing collateral property")
            })?;
            if !matches!(
                loan.status,
                crate::core::LoanStatus::Defaulted | crate::core::LoanStatus::Repaid
            ) && (property.collateral_loan_id != Some(*loan_id)
                || property.owner_dynasty_id != Some(loan.borrower_dynasty_id))
            {
                return Err(format!(
                    "loan {loan_id} has an invalid collateral relationship"
                ));
            }
        }
    }
    Ok(())
}

fn validate_employment_records(state: &AppState) -> Result<(), String> {
    let mut workers_by_business = BTreeMap::<BusinessId, u32>::new();
    let mut workers_by_household = BTreeMap::<HouseholdId, u32>::new();
    for (employment_id, agreement) in &state.employment {
        let business = state.businesses.get(agreement.business_id);
        if agreement.id != *employment_id
            || agreement.workers == 0
            || agreement.weekly_wage <= Money::ZERO
            || business.is_none()
            || state.households.get(agreement.household_id).is_none()
            || (agreement.status == crate::core::EmploymentStatus::Active
                && business.is_some_and(|business| {
                    business.status() == crate::core::BusinessStatus::Closed
                }))
        {
            return Err(format!(
                "employment agreement {employment_id} has an invalid reference"
            ));
        }
        if agreement.status != crate::core::EmploymentStatus::Ended {
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
    }
    for (business_id, workers) in workers_by_business {
        let business = state
            .businesses
            .get(business_id)
            .expect("validated employment business must exist");
        let supported_workers = crate::systems::supported_worker_capacity(business);
        if workers > supported_workers {
            return Err(format!(
                "business {business_id} employment exceeds operating capacity"
            ));
        }
    }
    for (household_id, workers) in workers_by_household {
        let members = state
            .households
            .get(household_id)
            .expect("validated employment household must exist")
            .members();
        if workers > u32::from(members) {
            return Err(format!(
                "household {household_id} employment exceeds household labor capacity"
            ));
        }
    }
    Ok(())
}

fn validate_family_records(state: &AppState) -> Result<(), String> {
    for (link_id, link) in &state.family_links {
        if link.id != *link_id
            || link.first_character_id == link.second_character_id
            || state.characters.get(link.first_character_id).is_none()
            || state.characters.get(link.second_character_id).is_none()
        {
            return Err(format!(
                "family link {link_id} has an invalid character reference"
            ));
        }
    }
    if state.family_councils.len() != state.dynasties.len() {
        return Err("every dynasty must have exactly one family council".to_owned());
    }
    for (dynasty_id, council) in &state.family_councils {
        let dynasty = state.dynasties.get(dynasty_id);
        if council.dynasty_id != *dynasty_id
            || dynasty.is_none()
            || council.members.iter().any(|character_id| {
                !state
                    .characters
                    .get(*character_id)
                    .is_some_and(|character| {
                        character.dynasty_id() == *dynasty_id
                            && character.status() == crate::core::CharacterStatus::Active
                    })
            })
        {
            return Err(format!(
                "family council {dynasty_id} has an invalid reference"
            ));
        }
        let dynasty = dynasty.expect("validated council dynasty must exist");
        if !council.members.contains(&dynasty.head_id())
            || dynasty
                .heir_id()
                .is_some_and(|heir_id| !council.members.contains(&heir_id))
        {
            return Err(format!(
                "family council {dynasty_id} omits the dynasty head or heir"
            ));
        }
    }
    Ok(())
}

fn validate_institution_and_misc_records(state: &AppState) -> Result<(), String> {
    for (institution_id, institution) in &state.institutions {
        if institution.institution_id != *institution_id
            || institution.members.iter().any(|character_id| {
                !state
                    .characters
                    .get(*character_id)
                    .is_some_and(|character| {
                        character.status() == crate::core::CharacterStatus::Active
                    })
            })
            || institution
                .office_holder_id
                .is_some_and(|holder| !institution.members.contains(&holder))
        {
            return Err(format!(
                "institution {institution_id} has inconsistent runtime state"
            ));
        }
    }
    for (pair, relationship) in &state.relationships {
        if relationship.pair != *pair
            || pair.first == pair.second
            || !state.dynasties.contains_key(&pair.first)
            || !state.dynasties.contains_key(&pair.second)
            || relationship.last_interaction_day > state.clock.day()
        {
            return Err("relationship map contains an invalid dynasty pair".to_owned());
        }
    }
    validate_misc_record_ids_and_refs(state)
}

fn validate_misc_record_ids_and_refs(state: &AppState) -> Result<(), String> {
    validate_law_report_and_objective_records(state)?;
    validate_civic_event_records(state)?;
    validate_persisted_history(state)
}

fn validate_law_report_and_objective_records(state: &AppState) -> Result<(), String> {
    for (law_id, law) in &state.laws {
        if law.id != *law_id
            || !law.kind.is_value_valid(law.value)
            || law.enacted_day > state.clock.day()
            || law
                .sponsor_dynasty_id
                .is_some_and(|dynasty_id| !state.dynasties.contains_key(&dynasty_id))
        {
            return Err(format!("law {law_id} has an invalid identity reference"));
        }
        if law.active
            && state
                .laws
                .values()
                .any(|other| other.id != law.id && other.active && other.kind == law.kind)
        {
            return Err(format!(
                "law kind {:?} has multiple active records",
                law.kind
            ));
        }
        if law.active && !law.kind.is_implemented() {
            return Err(format!("active law kind {:?} is not implemented", law.kind));
        }
    }
    for (report_id, report) in &state.information_reports {
        if report.id != *report_id
            || !state.dynasties.contains_key(&report.owner_dynasty_id)
            || report.created_day > state.clock.day()
            || report.expires_day < report.created_day
        {
            return Err(format!(
                "information report {report_id} has an invalid reference"
            ));
        }
    }
    for (objective_id, objective) in &state.ai_objectives {
        if objective.id != *objective_id
            || !state.dynasties.contains_key(&objective.dynasty_id)
            || objective.created_day > state.clock.day()
            || objective
                .target_dynasty_id
                .is_some_and(|dynasty_id| !state.dynasties.contains_key(&dynasty_id))
        {
            return Err(format!(
                "AI objective {objective_id} has an invalid reference"
            ));
        }
        if objective.status == crate::core::ObjectiveStatus::Achieved
            && objective.rationale.is_empty()
        {
            return Err(format!(
                "achieved AI objective {objective_id} has no rationale"
            ));
        }
    }
    Ok(())
}

fn validate_civic_event_records(state: &AppState) -> Result<(), String> {
    for (work_id, work) in &state.public_works {
        if work.id != *work_id
            || !state.districts.contains_key(&work.district_id)
            || work
                .sponsor_dynasty_id
                .is_some_and(|dynasty_id| !state.dynasties.contains_key(&dynasty_id))
        {
            return Err(format!("public work {work_id} has an invalid reference"));
        }
    }
    let mut active_cases = BTreeSet::new();
    for (case_id, legal_case) in &state.legal_cases {
        if legal_case.id != *case_id
            || legal_case.plaintiff_dynasty_id == legal_case.defendant_dynasty_id
            || !state
                .dynasties
                .contains_key(&legal_case.plaintiff_dynasty_id)
            || !state
                .dynasties
                .contains_key(&legal_case.defendant_dynasty_id)
            || legal_case.filed_day > state.clock.day()
            || legal_case.hearing_day < legal_case.filed_day
            || (matches!(
                legal_case.status,
                crate::core::LegalCaseStatus::DecidedForPlaintiff
                    | crate::core::LegalCaseStatus::DecidedForDefendant
            ) && legal_case.hearing_day > state.clock.day())
        {
            return Err(format!("legal case {case_id} has an invalid reference"));
        }
        if matches!(
            legal_case.status,
            crate::core::LegalCaseStatus::Filed | crate::core::LegalCaseStatus::Hearing
        ) && !active_cases.insert((
            legal_case.plaintiff_dynasty_id,
            legal_case.defendant_dynasty_id,
            legal_case.kind,
        )) {
            return Err(format!(
                "legal case {case_id} duplicates an unresolved case between the same parties"
            ));
        }
    }
    for (route_id, route) in &state.external_routes {
        if route.id != *route_id || !state.market.quotes.contains_key(&route.good_id) {
            return Err(format!(
                "external route {route_id} has an invalid reference"
            ));
        }
    }
    for (crisis_id, crisis) in &state.crises {
        if crisis.id != *crisis_id
            || crisis.started_day > state.clock.day()
            || crisis
                .district_id
                .is_some_and(|district_id| !state.districts.contains_key(&district_id))
            || (crisis.status == crate::core::CrisisStatus::Resolved
                && crisis.severity_basis_points >= 500)
        {
            return Err(format!("crisis {crisis_id} has an invalid reference"));
        }
    }
    Ok(())
}

fn validate_persisted_history(state: &AppState) -> Result<(), String> {
    let mut outbox_ids = BTreeSet::new();
    let mut prior_outbox_day = i64::MIN;
    for message in &state.outbox {
        if !outbox_ids.insert(message.id) {
            return Err("outbox contains duplicate message IDs".to_owned());
        }
        if message.day < prior_outbox_day || message.day > state.clock.day() {
            return Err("outbox messages are not chronologically valid".to_owned());
        }
        if message.subject.is_empty() || message.body.is_empty() {
            return Err("outbox message lacks user-facing content".to_owned());
        }
        prior_outbox_day = message.day;
    }

    let mut chronicle_ids = BTreeSet::new();
    let mut prior_chronicle_day = i64::MIN;
    for entry in &state.chronicle {
        if !chronicle_ids.insert(entry.id()) {
            return Err("chronicle contains duplicate entry IDs".to_owned());
        }
        if entry.day() < prior_chronicle_day || entry.day() > state.clock.day() {
            return Err("chronicle entries are not chronologically valid".to_owned());
        }
        prior_chronicle_day = entry.day();
    }
    let mut prior_audit_day = i64::MIN;
    for record in &state.audit_log {
        if record.day() < prior_audit_day || record.day() > state.clock.day() {
            return Err("audit log is not chronologically valid".to_owned());
        }
        prior_audit_day = record.day();
    }
    Ok(())
}

fn hydrate_strategic_state(state: &mut AppState, migrated_from_legacy: bool) {
    if !migrated_from_legacy || state.scenario_key != "rivergate" {
        return;
    }
    let registry = crate::registry::build_rivergate_registry();
    crate::systems::initialize_strategic_state(&registry, state);
}

fn migrate_to_current(mut value: Value, path: &Path) -> Result<Value, PersistenceError> {
    let mut version = read_schema_version(&value, path)?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(PersistenceError::FutureSchema {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    while version < CURRENT_SCHEMA_VERSION {
        value = match version {
            0 => migrate_v0_to_v1(value)?,
            1 => migrate_v1_to_v2(value)?,
            2 => migrate_v2_to_v3(value)?,
            3 => migrate_v3_to_v4(value)?,
            _ => return Err(PersistenceError::UnsupportedSchema { version }),
        };
        version += 1;
    }
    Ok(value)
}

fn read_schema_version(value: &Value, path: &Path) -> Result<u32, PersistenceError> {
    let raw_version = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| PersistenceError::MissingSchemaVersion {
            path: path.to_path_buf(),
        })?;
    u32::try_from(raw_version).map_err(|_| PersistenceError::Migration {
        version: u32::MAX,
        reason: format!("schema version {raw_version} does not fit u32"),
    })
}

fn migrate_v0_to_v1(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 0,
            reason: "save root must be an object".to_owned(),
        })?;
    object.insert("schema_version".to_owned(), Value::from(1));
    object
        .entry("audit_log".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    Ok(value)
}

fn migrate_v1_to_v2(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 1,
            reason: "save root must be an object".to_owned(),
        })?;
    object.insert("schema_version".to_owned(), Value::from(2));
    for field in [
        "institution_runtime",
        "contracts",
        "loans",
        "properties",
        "employment",
        "family_links",
        "family_councils",
        "laws",
        "relationships",
        "information_reports",
        "ai_objectives",
        "districts",
        "public_works",
        "legal_cases",
        "external_routes",
        "crises",
    ] {
        object
            .entry(field.to_owned())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
    object
        .entry("outbox".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let next_ids = object
        .get_mut("next_ids")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 1,
            reason: "save next_ids must be an object".to_owned(),
        })?;
    for field in [
        "contract",
        "property",
        "loan",
        "employment",
        "family_link",
        "law",
        "information_report",
        "objective",
        "public_work",
        "legal_case",
        "external_route",
        "crisis",
        "outbox",
    ] {
        next_ids
            .entry(field.to_owned())
            .or_insert_with(|| Value::from(0));
    }
    Ok(value)
}

fn migrate_v2_to_v3(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 2,
            reason: "save root must be an object".to_owned(),
        })?;
    let institutions =
        object
            .remove("institution_runtime")
            .ok_or_else(|| PersistenceError::Migration {
                version: 2,
                reason: "save is missing institution_runtime".to_owned(),
            })?;
    object.insert("institutions".to_owned(), institutions);

    let business_records = object
        .get_mut("businesses")
        .and_then(Value::as_object_mut)
        .and_then(|businesses| businesses.get_mut("records"))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 2,
            reason: "save businesses.records must be an object".to_owned(),
        })?;
    for business in business_records.values_mut() {
        let operations = business
            .as_object_mut()
            .and_then(|business| business.get_mut("operations"))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| PersistenceError::Migration {
                version: 2,
                reason: "save business operations must be an object".to_owned(),
            })?;
        operations.remove("employees");
    }
    object.insert("schema_version".to_owned(), Value::from(3));
    Ok(value)
}

fn migrate_v3_to_v4(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 3,
            reason: "save root must be an object".to_owned(),
        })?;
    let business_records = object
        .get_mut("businesses")
        .and_then(Value::as_object_mut)
        .and_then(|businesses| businesses.get_mut("records"))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 3,
            reason: "save businesses.records must be an object".to_owned(),
        })?;
    for business in business_records.values_mut() {
        let finance = business
            .as_object_mut()
            .and_then(|business| business.get_mut("finance"))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| PersistenceError::Migration {
                version: 3,
                reason: "save business finance must be an object".to_owned(),
            })?;
        finance.remove("debt");
    }
    object.insert("schema_version".to_owned(), Value::from(4));
    Ok(value)
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
