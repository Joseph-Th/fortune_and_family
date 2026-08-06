//! JSON persistence adapter with explicit schema migration and contextual errors.

use crate::core::{AppState, AuditKind, CURRENT_SCHEMA_VERSION, FamilyLinkKind, InformationTarget};
use crate::ids::{BusinessId, HouseholdId};
use crate::money::{Money, Quantity, checked_cost_for};
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
    #[error("failed to synchronize save directory {path}: {source}")]
    SyncDirectory {
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
/// Returns an error when state validation fails, the parent directory or temporary file cannot be
/// created, serialization fails, or the destination cannot be atomically replaced.
pub fn save_state(path: impl AsRef<Path>, state: &AppState) -> Result<(), PersistenceError> {
    let path = path.as_ref();
    validate_state(state).map_err(|error| PersistenceError::InvalidState {
        path: path.to_path_buf(),
        kind: error.kind,
        reason: error.reason,
    })?;
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
    #[cfg(unix)]
    sync_save_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_save_directory(parent: &Path) -> Result<(), PersistenceError> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| PersistenceError::SyncDirectory {
            path: parent.to_path_buf(),
            source,
        })
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
    validate_state(&state).map_err(|error| PersistenceError::InvalidState {
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
    let mut retained_officeholders = BTreeSet::new();
    for (institution_id, office_holder_id) in officeholders {
        if let Some(institution) = state.institutions.get_mut(&institution_id) {
            let office_holder_id = office_holder_id
                .filter(|character_id| retained_officeholders.insert(*character_id));
            if let Some(character_id) = office_holder_id {
                institution.members.insert(character_id);
            }
            institution.office_holder_id = office_holder_id;
        }
    }
}

fn validate_state(state: &AppState) -> Result<(), StateValidationError> {
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
    if state.clock.day() < 0 || state.clock.day() == i64::MAX {
        return Err(format!(
            "simulation clock has invalid or exhausted elapsed day {}",
            state.clock.day()
        ));
    }
    for dynasty in state.dynasties.values() {
        if dynasty.treasury() < Money::ZERO
            || dynasty.civic_contributions() < Money::ZERO
            || dynasty.resources.legitimacy_basis_points > 10_000
            || dynasty.resources.reputation_quality_basis_points > 10_000
            || dynasty.resources.reputation_reliability_basis_points > 10_000
            || dynasty.runtime.generation == 0
            || dynasty.runtime.generation == u16::MAX
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
            || (character.status() == crate::core::CharacterStatus::Active
                && character.runtime.health_basis_points == 0)
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
            || business.finance.lifetime_revenue < Money::ZERO
            || business.finance.lifetime_costs < Money::ZERO
            || business.operations.capacity_batches_per_day == 0
            || business.operations.condition_basis_points > 10_000
            || business.operations.quality_basis_points > 10_000
            || business.policy.target_input_days > 30
            || business.policy.target_output_days > 30
            || business.policy.minimum_cash_reserve < Money::ZERO
            || business.policy.maintenance_basis_points > 10_000
            || business.policy.quality_target_basis_points > 10_000
            || business.finance.version == u64::MAX
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
    for debt in state.civic_debts.values() {
        if debt.principal <= Money::ZERO
            || debt.balance < Money::ZERO
            || debt.weekly_payment <= Money::ZERO
            || debt.interest_basis_points > 10_000
        {
            return Err(format!(
                "civic debt {} has an invalid financial value",
                debt.id
            ));
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
            || checked_cost_for(contract.quantity_per_week, contract.unit_price).is_none()
        {
            return Err(format!(
                "supply contract {} has an invalid financial value",
                contract.id
            ));
        }
    }
    for institution in state.institutions.values() {
        if institution.budget < Money::ZERO
            || institution.legitimacy_basis_points > 10_000
            || institution.term_number == 0
            || institution.term_number == u32::MAX
            || institution.term_started_day > state.clock.day()
            || institution.next_selection_day < institution.term_started_day
            || institution.active_directive.is_some_and(|directive| {
                directive.expires_day < 0
                    || directive.expires_day == i64::MAX
                    || !institution.powers.contains(&directive.power)
            })
        {
            return Err(format!(
                "institution {} has an invalid budget, term timing, or directive",
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
        if council.unity_basis_points > 10_000 || council.charter_version == u64::MAX {
            return Err(format!(
                "family council {} has invalid unity or an exhausted charter version",
                council.dynasty_id
            ));
        }
    }
    for district in state.districts.values() {
        if district.rent_index_basis_points < crate::systems::MIN_DISTRICT_RENT_INDEX_BASIS_POINTS
            || district.rent_index_basis_points
                > crate::systems::MAX_DISTRICT_RENT_INDEX_BASIS_POINTS
            || district.employment_basis_points > 10_000
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
        {
            return Err(format!(
                "public work {} has an invalid progress value",
                work.id
            ));
        }
        let expected_progress = u16::try_from(
            work.spent
                .saturating_mul_ratio(10_000, work.budget.copper())
                .copper()
                .clamp(0, 10_000),
        )
        .expect("clamped public-work progress must fit u16");
        if work.progress_basis_points != expected_progress
            || (work.status == crate::core::PublicWorkStatus::Completed
                && work.spent != work.budget)
        {
            return Err(format!(
                "public work {} progress does not match its spending or lifecycle",
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
    for good in registry.goods() {
        let quote = state
            .market
            .quotes
            .get(&good.id())
            .expect("validated market quote ID set must contain every good");
        if quote.target_stock != good.target_market_stock() {
            return Err(format!(
                "market quote {} target stock does not match the scenario registry",
                good.id()
            ));
        }
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
    for definition in registry.institutions() {
        let institution = state
            .institutions
            .get(&definition.id())
            .expect("validated institution ID set must contain every definition");
        if institution.powers != crate::systems::institution_powers_for(definition.kind()) {
            return Err(format!(
                "institution {} powers do not match the scenario registry",
                definition.id()
            ));
        }
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
        if dynasty.name().trim().is_empty() {
            return Err(format!("dynasty {dynasty_id} has a blank name"));
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
        if character.name().trim().is_empty() {
            return Err(format!("character {character_id} has a blank name"));
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
    let mut administrative_load = BTreeMap::<_, u64>::new();
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
        if business.name().trim().is_empty() {
            return Err(format!("business {business_id} has a blank name"));
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
            .and_modify(|load| *load += u64::from(recipe.administrative_load()))
            .or_insert(u64::from(recipe.administrative_load()));
    }
    if &owner_index != state.businesses.owner_index()
        || &district_index != state.businesses.district_index()
    {
        return Err("business ownership or district index is stale or incomplete".to_owned());
    }
    for dynasty in state.dynasties.values() {
        let expected = administrative_load.get(&dynasty.id()).copied().unwrap_or(0);
        if u64::from(dynasty.administrative_load()) != expected {
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
    validate_contract_records(registry, state)?;
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
            if property.owner_dynasty_id.is_none()
                || !occupied_businesses.insert(business_id)
                || business.district_id() != property.district_id
            {
                return Err(format!(
                    "property {property_id} has an invalid or duplicate occupant"
                ));
            }
            let expected_tenant = property
                .owner_dynasty_id
                .filter(|owner_id| *owner_id != business.owner_dynasty_id())
                .map(|_| business.owner_dynasty_id());
            if property.tenant_dynasty_id != expected_tenant {
                return Err(format!(
                    "property {property_id} tenancy does not match its occupant business owner"
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

fn validate_contract_records(
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
        let buyer = buyer.expect("validated contract buyer must exist");
        let seller = seller.expect("validated contract seller must exist");
        let buyer_recipe = registry
            .get_recipe(buyer.recipe_id())
            .expect("validated business recipe must exist");
        let seller_recipe = registry
            .get_recipe(seller.recipe_id())
            .expect("validated business recipe must exist");
        let valid_breach_attribution = contract.breaching_dynasty_id.is_none_or(|dynasty_id| {
            contract.status == crate::core::ContractStatus::Breached
                && state.dynasties.contains_key(&dynasty_id)
        });
        let valid_delivery_attribution = contract_delivery_attribution_is_valid(state, contract);
        if seller_recipe.output_good_id() != contract.good_id
            || !buyer_recipe
                .inputs()
                .iter()
                .any(|input| input.good_id() == contract.good_id)
            || (contract.status == crate::core::ContractStatus::Active
                && buyer.owner_dynasty_id() == seller.owner_dynasty_id())
            || (contract.status == crate::core::ContractStatus::Active
                && contract.next_due_day > contract.end_day)
            || !valid_breach_attribution
            || !valid_delivery_attribution
        {
            return Err(format!(
                "supply contract {contract_id} is incompatible with its parties or term"
            ));
        }
    }
    Ok(())
}

fn contract_delivery_attribution_is_valid(
    state: &AppState,
    contract: &crate::core::SupplyContract,
) -> bool {
    let attributed_deliveries = contract
        .fulfilled_deliveries_by_dynasty
        .values()
        .map(|deliveries| u64::from(*deliveries))
        .sum::<u64>();
    let fulfilled_deliveries = u64::from(contract.fulfilled_deliveries);
    contract
        .fulfilled_deliveries_by_dynasty
        .iter()
        .all(|(dynasty_id, deliveries)| {
            state.dynasties.contains_key(dynasty_id)
                && *deliveries > 0
                && *deliveries <= contract.fulfilled_deliveries
        })
        && if fulfilled_deliveries == 0 {
            contract.fulfilled_deliveries_by_dynasty.is_empty()
        } else {
            attributed_deliveries >= fulfilled_deliveries
                && attributed_deliveries <= fulfilled_deliveries * 2
        }
}

fn validate_finance_and_organization_records(state: &AppState) -> Result<(), String> {
    validate_loan_records(state)?;
    validate_civic_debt_records(state)?;
    validate_employment_records(state)?;
    validate_family_records(state)?;
    validate_institution_and_misc_records(state)
}

fn validate_civic_debt_records(state: &AppState) -> Result<(), String> {
    for (debt_id, debt) in &state.civic_debts {
        let authorizing_law = state.laws.get(&debt.authorizing_law_id);
        if debt.id != *debt_id
            || !state.dynasties.contains_key(&debt.creditor_dynasty_id)
            || debt.sponsor_dynasty_id == Some(debt.creditor_dynasty_id)
            || debt
                .sponsor_dynasty_id
                .is_some_and(|dynasty_id| !state.dynasties.contains_key(&dynasty_id))
            || !authorizing_law.is_some_and(|law| {
                law.kind == crate::core::LawKind::PublicDebtAuthorization
                    && law.sponsor_dynasty_id == debt.sponsor_dynasty_id
                    && law.value == debt.principal.copper()
            })
            || debt.issued_day > state.clock.day()
            || debt.next_due_day < debt.issued_day
        {
            return Err(format!(
                "civic debt {debt_id} has an invalid identity or authorization reference"
            ));
        }
        match debt.status {
            crate::core::CivicDebtStatus::Current => {
                if debt.balance <= Money::ZERO || debt.missed_payments != 0 {
                    return Err(format!(
                        "current civic debt {debt_id} has invalid balance or arrears"
                    ));
                }
            }
            crate::core::CivicDebtStatus::Delinquent => {
                if debt.balance <= Money::ZERO || !(1..3).contains(&debt.missed_payments) {
                    return Err(format!(
                        "delinquent civic debt {debt_id} has invalid balance or arrears"
                    ));
                }
            }
            crate::core::CivicDebtStatus::Defaulted => {
                if debt.balance <= Money::ZERO || debt.missed_payments < 3 {
                    return Err(format!(
                        "defaulted civic debt {debt_id} has invalid balance or arrears"
                    ));
                }
            }
            crate::core::CivicDebtStatus::Repaid => {
                if debt.balance != Money::ZERO || debt.missed_payments != 0 {
                    return Err(format!(
                        "repaid civic debt {debt_id} retains balance or arrears"
                    ));
                }
            }
        }
    }
    Ok(())
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
        match loan.status {
            crate::core::LoanStatus::Current
            | crate::core::LoanStatus::Delinquent
            | crate::core::LoanStatus::Restructured
            | crate::core::LoanStatus::Defaulted => {
                if loan.balance <= Money::ZERO {
                    return Err(format!("unsettled loan {loan_id} has no remaining balance"));
                }
            }
            crate::core::LoanStatus::Repaid => {
                if loan.balance != Money::ZERO {
                    return Err(format!("repaid loan {loan_id} retains a balance"));
                }
            }
        }
        if let Some(property_id) = loan.collateral_property_id {
            let property = state.properties.get(&property_id).ok_or_else(|| {
                format!("loan {loan_id} references a missing collateral property")
            })?;
            match loan.status {
                crate::core::LoanStatus::Current
                | crate::core::LoanStatus::Delinquent
                | crate::core::LoanStatus::Restructured => {
                    if property.collateral_loan_id != Some(*loan_id)
                        || property.owner_dynasty_id != Some(loan.borrower_dynasty_id)
                    {
                        return Err(format!(
                            "loan {loan_id} has an invalid active collateral relationship"
                        ));
                    }
                }
                crate::core::LoanStatus::Defaulted => {
                    if property.collateral_loan_id == Some(*loan_id) {
                        return Err(format!(
                            "defaulted loan {loan_id} has an invalid collateral settlement"
                        ));
                    }
                }
                crate::core::LoanStatus::Repaid => {
                    if property.collateral_loan_id == Some(*loan_id) {
                        return Err(format!(
                            "repaid loan {loan_id} has an invalid collateral release"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_employment_records(state: &AppState) -> Result<(), String> {
    let mut workers_by_business = BTreeMap::<BusinessId, u64>::new();
    let mut workers_by_household = BTreeMap::<HouseholdId, u64>::new();
    for (employment_id, agreement) in &state.employment {
        let business = state.businesses.get(agreement.business_id);
        if agreement.id != *employment_id
            || agreement.workers == 0
            || agreement.weekly_wage <= Money::ZERO
            || business.is_none()
            || state.households.get(agreement.household_id).is_none()
        {
            return Err(format!(
                "employment agreement {employment_id} has an invalid reference"
            ));
        }
        if business.is_some_and(|business| {
            !crate::systems::is_employment_status_compatible(business.status(), agreement.status)
        }) {
            return Err(format!(
                "employment agreement {employment_id} status is incompatible with its business lifecycle"
            ));
        }
        if agreement.status != crate::core::EmploymentStatus::Ended {
            workers_by_business
                .entry(agreement.business_id)
                .and_modify(|workers| *workers += u64::from(agreement.workers))
                .or_insert(u64::from(agreement.workers));
            workers_by_household
                .entry(agreement.household_id)
                .and_modify(|workers| *workers += u64::from(agreement.workers))
                .or_insert(u64::from(agreement.workers));
        }
    }
    for (business_id, workers) in workers_by_business {
        let business = state
            .businesses
            .get(business_id)
            .expect("validated employment business must exist");
        let supported_workers = crate::systems::supported_worker_capacity(business);
        if workers > u64::from(supported_workers) {
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
        if workers > u64::from(members) {
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
        if matches!(link.kind, FamilyLinkKind::Adoptive | FamilyLinkKind::Ward) {
            let first = state
                .characters
                .get(link.first_character_id)
                .expect("validated family link character must exist");
            let second = state
                .characters
                .get(link.second_character_id)
                .expect("validated family link character must exist");
            if first.dynasty_id() != second.dynasty_id() {
                return Err(format!(
                    "family link {link_id} crosses dynasties for an adoptive or ward relationship"
                ));
            }
            if link.active
                && link.kind == FamilyLinkKind::Ward
                && !state
                    .family_councils
                    .get(&second.dynasty_id())
                    .is_some_and(|council| council.members.contains(&second.id()))
            {
                return Err(format!(
                    "family link {link_id} has an active ward outside its dynasty council"
                ));
            }
        }
        if link.kind == FamilyLinkKind::ParentChild {
            let parent = state
                .characters
                .get(link.first_character_id)
                .expect("validated family link character must exist");
            let child = state
                .characters
                .get(link.second_character_id)
                .expect("validated family link character must exist");
            if child.birth_day().saturating_sub(parent.birth_day())
                < crate::core::MIN_PARENT_CHILD_AGE_GAP_DAYS
            {
                return Err(format!(
                    "family link {link_id} has impossible parent-child chronology"
                ));
            }
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
    let mut officeholders = BTreeSet::new();
    for (institution_id, institution) in &state.institutions {
        let unsupported_player_member = institution.members.iter().any(|character_id| {
            state
                .characters
                .get(*character_id)
                .is_some_and(|character| {
                    character.dynasty_id() == state.player_dynasty_id
                        && institution.office_holder_id != Some(*character_id)
                })
                && !state.audit_log.iter().any(|record| {
                    record.kind() == AuditKind::InstitutionPatronage
                        && record
                            .audit_subject()
                            .references_institution_character(*institution_id, *character_id)
                })
        });
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
            || unsupported_player_member
        {
            return Err(format!(
                "institution {institution_id} has inconsistent runtime state"
            ));
        }
        if institution
            .office_holder_id
            .is_some_and(|holder_id| !officeholders.insert(holder_id))
        {
            return Err(format!(
                "institution {institution_id} duplicates an existing officeholder"
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
        if law.active && !law.kind.remains_active_after_enactment() {
            return Err(format!(
                "active law kind {:?} is a consumed one-time authorization",
                law.kind
            ));
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
        let target_exists = report.target.is_none_or(|target| match target {
            InformationTarget::Market { good_id } => state.market.quotes.contains_key(&good_id),
            InformationTarget::Counterparty { dynasty_id } => {
                state.dynasties.contains_key(&dynasty_id)
            }
            InformationTarget::District { district_id } => {
                state.districts.contains_key(&district_id)
            }
        });
        if !target_exists {
            return Err(format!(
                "information report {report_id} targets a missing record"
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
        if message.subject.trim().is_empty() || message.body.trim().is_empty() {
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
        if entry.summary().trim().is_empty() {
            return Err("chronicle entry lacks user-facing content".to_owned());
        }
        prior_chronicle_day = entry.day();
    }
    let mut prior_audit_day = i64::MIN;
    for record in &state.audit_log {
        if record.day() < prior_audit_day || record.day() > state.clock.day() {
            return Err("audit log is not chronologically valid".to_owned());
        }
        if record.subject().trim().is_empty() || record.detail().trim().is_empty() {
            return Err("audit record lacks diagnostic content".to_owned());
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
            4 => migrate_v4_to_v5(value)?,
            5 => migrate_v5_to_v6(value)?,
            6 => migrate_v6_to_v7(value)?,
            7 => migrate_v7_to_v8(value)?,
            8 => migrate_v8_to_v9(value)?,
            9 => migrate_v9_to_v10(value)?,
            10 => migrate_v10_to_v11(value)?,
            11 => migrate_v11_to_v12(value)?,
            12 => migrate_v12_to_v13(value)?,
            13 => migrate_v13_to_v14(value)?,
            14 => migrate_v14_to_v15(value)?,
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

fn migrate_v4_to_v5(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 4,
            reason: "save root must be an object".to_owned(),
        })?;
    {
        let institutions = object
            .get_mut("institutions")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| PersistenceError::Migration {
                version: 4,
                reason: "save institutions must be an object".to_owned(),
            })?;
        let mut ordered_institutions = institutions
            .iter()
            .map(|(key, institution)| {
                let institution_id = institution
                    .get("institution_id")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| PersistenceError::Migration {
                        version: 4,
                        reason: format!("institution {key} has an invalid institution_id"),
                    })?;
                Ok((institution_id, key.clone()))
            })
            .collect::<Result<Vec<_>, PersistenceError>>()?;
        ordered_institutions.sort_unstable();
        let mut retained_officeholders = BTreeSet::new();
        for (_, key) in ordered_institutions {
            let institution = institutions
                .get_mut(&key)
                .and_then(Value::as_object_mut)
                .expect("collected institution key must remain present");
            let duplicate = institution
                .get("office_holder_id")
                .filter(|holder| !holder.is_null())
                .and_then(Value::as_u64)
                .is_some_and(|holder_id| !retained_officeholders.insert(holder_id));
            if duplicate {
                institution.insert("office_holder_id".to_owned(), Value::Null);
            }
        }
    }
    object.insert("schema_version".to_owned(), Value::from(5));
    Ok(value)
}

fn migrate_v5_to_v6(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 5,
            reason: "save root must be an object".to_owned(),
        })?;
    let business_owners = object
        .get("businesses")
        .and_then(Value::as_object)
        .and_then(|businesses| businesses.get("records"))
        .and_then(Value::as_object)
        .ok_or_else(|| PersistenceError::Migration {
            version: 5,
            reason: "save businesses.records must be an object".to_owned(),
        })?
        .values()
        .map(|business| {
            let identity = business
                .get("identity")
                .and_then(Value::as_object)
                .ok_or_else(|| PersistenceError::Migration {
                    version: 5,
                    reason: "save business identity must be an object".to_owned(),
                })?;
            let business_id = identity.get("id").and_then(Value::as_u64).ok_or_else(|| {
                PersistenceError::Migration {
                    version: 5,
                    reason: "save business has an invalid id".to_owned(),
                }
            })?;
            let owner_dynasty_id = identity
                .get("owner_dynasty_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| PersistenceError::Migration {
                    version: 5,
                    reason: format!("save business {business_id} has an invalid owner"),
                })?;
            Ok((business_id, owner_dynasty_id))
        })
        .collect::<Result<BTreeMap<_, _>, PersistenceError>>()?;
    let properties = object
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 5,
            reason: "save properties must be an object".to_owned(),
        })?;
    for property in properties.values_mut() {
        let property = property
            .as_object_mut()
            .ok_or_else(|| PersistenceError::Migration {
                version: 5,
                reason: "save property must be an object".to_owned(),
            })?;
        let Some(occupant_business_id) = property
            .get("occupant_business_id")
            .filter(|value| !value.is_null())
            .and_then(Value::as_u64)
        else {
            continue;
        };
        let business_owner_id = business_owners
            .get(&occupant_business_id)
            .copied()
            .ok_or_else(|| PersistenceError::Migration {
                version: 5,
                reason: format!(
                    "save property references missing occupant business {occupant_business_id}"
                ),
            })?;
        let property_owner_id = property
            .get("owner_dynasty_id")
            .filter(|value| !value.is_null())
            .and_then(Value::as_u64);
        let tenant = if property_owner_id.is_some_and(|owner_id| owner_id != business_owner_id) {
            Value::from(business_owner_id)
        } else {
            Value::Null
        };
        property.insert("tenant_dynasty_id".to_owned(), tenant);
    }
    object.insert("schema_version".to_owned(), Value::from(6));
    Ok(value)
}

fn migrate_v6_to_v7(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 6,
            reason: "save root must be an object".to_owned(),
        })?;
    object.insert("schema_version".to_owned(), Value::from(7));
    Ok(value)
}

fn migrate_v7_to_v8(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 7,
            reason: "save root must be an object".to_owned(),
        })?;
    let institutions = object
        .get_mut("institutions")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 7,
            reason: "save institutions must be an object".to_owned(),
        })?;
    for institution in institutions.values_mut() {
        let institution =
            institution
                .as_object_mut()
                .ok_or_else(|| PersistenceError::Migration {
                    version: 7,
                    reason: "save institution must be an object".to_owned(),
                })?;
        let next_selection_day = institution
            .get("next_selection_day")
            .and_then(Value::as_i64)
            .ok_or_else(|| PersistenceError::Migration {
                version: 7,
                reason: "save institution has an invalid next_selection_day".to_owned(),
            })?;
        institution.insert(
            "term_started_day".to_owned(),
            Value::from(next_selection_day.saturating_sub(crate::systems::OFFICE_TERM_DAYS)),
        );
    }
    object.insert("schema_version".to_owned(), Value::from(8));
    Ok(value)
}

fn migrate_v8_to_v9(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 8,
            reason: "save root must be an object".to_owned(),
        })?;
    let dynasties = object
        .get_mut("dynasties")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 8,
            reason: "save dynasties must be an object".to_owned(),
        })?;
    for dynasty in dynasties.values_mut() {
        let resources = dynasty
            .as_object_mut()
            .and_then(|dynasty| dynasty.get_mut("resources"))
            .and_then(Value::as_object_mut)
            .ok_or_else(|| PersistenceError::Migration {
                version: 8,
                reason: "save dynasty resources must be an object".to_owned(),
            })?;
        resources
            .entry("civic_contributions".to_owned())
            .or_insert_with(|| Value::from(0));
        resources
            .entry("unmet_office_duties".to_owned())
            .or_insert_with(|| Value::from(0));
    }
    object.insert("schema_version".to_owned(), Value::from(9));
    Ok(value)
}

fn migrate_v9_to_v10(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 9,
            reason: "save root must be an object".to_owned(),
        })?;
    object
        .entry("civic_debts".to_owned())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let next_ids = object
        .get_mut("next_ids")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 9,
            reason: "save next_ids must be an object".to_owned(),
        })?;
    next_ids
        .entry("civic_debt".to_owned())
        .or_insert_with(|| Value::from(0));
    object.insert("schema_version".to_owned(), Value::from(10));
    Ok(value)
}

fn migrate_v10_to_v11(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 10,
            reason: "save root must be an object".to_owned(),
        })?;
    let contracts = object
        .get_mut("contracts")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 10,
            reason: "save contracts must be an object".to_owned(),
        })?;
    for contract in contracts.values_mut() {
        contract
            .as_object_mut()
            .ok_or_else(|| PersistenceError::Migration {
                version: 10,
                reason: "save contract must be an object".to_owned(),
            })?
            .entry("breaching_dynasty_id".to_owned())
            .or_insert(Value::Null);
    }
    let laws = object
        .get_mut("laws")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 10,
            reason: "save laws must be an object".to_owned(),
        })?;
    for law in laws.values_mut() {
        let law = law
            .as_object_mut()
            .ok_or_else(|| PersistenceError::Migration {
                version: 10,
                reason: "save law must be an object".to_owned(),
            })?;
        if law.get("kind").and_then(Value::as_str) == Some("PublicDebtAuthorization") {
            law.insert("active".to_owned(), Value::Bool(false));
        }
    }
    object.insert("schema_version".to_owned(), Value::from(11));
    Ok(value)
}

fn migrate_v11_to_v12(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 11,
            reason: "save root must be an object".to_owned(),
        })?;
    let business_owners = v11_business_owners(object)?;
    let contracts = object
        .get_mut("contracts")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 11,
            reason: "save contracts must be an object".to_owned(),
        })?;
    for contract in contracts.values_mut() {
        let contract = contract
            .as_object_mut()
            .ok_or_else(|| PersistenceError::Migration {
                version: 11,
                reason: "save contract must be an object".to_owned(),
            })?;
        let fulfilled_deliveries = contract
            .get("fulfilled_deliveries")
            .and_then(Value::as_u64)
            .ok_or_else(|| PersistenceError::Migration {
                version: 11,
                reason: "save contract has invalid fulfilled_deliveries".to_owned(),
            })?;
        let buyer_business_id = contract
            .get("buyer_business_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| PersistenceError::Migration {
                version: 11,
                reason: "save contract has an invalid buyer".to_owned(),
            })?;
        let seller_business_id = contract
            .get("seller_business_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| PersistenceError::Migration {
                version: 11,
                reason: "save contract has an invalid seller".to_owned(),
            })?;
        let buyer_owner_id = business_owners
            .get(&buyer_business_id)
            .copied()
            .ok_or_else(|| PersistenceError::Migration {
                version: 11,
                reason: format!(
                    "save contract references missing buyer business {buyer_business_id}"
                ),
            })?;
        let seller_owner_id = business_owners
            .get(&seller_business_id)
            .copied()
            .ok_or_else(|| PersistenceError::Migration {
                version: 11,
                reason: format!(
                    "save contract references missing seller business {seller_business_id}"
                ),
            })?;
        // Version 11 did not retain the performing dynasty for each delivery. Attribute the
        // historical total to the current parties deterministically; version 12 records exact
        // ownership at settlement time for all future deliveries.
        let mut attribution = serde_json::Map::new();
        if fulfilled_deliveries > 0 {
            attribution.insert(
                buyer_owner_id.to_string(),
                Value::from(fulfilled_deliveries),
            );
            if seller_owner_id != buyer_owner_id {
                attribution.insert(
                    seller_owner_id.to_string(),
                    Value::from(fulfilled_deliveries),
                );
            }
        }
        contract.insert(
            "fulfilled_deliveries_by_dynasty".to_owned(),
            Value::Object(attribution),
        );
    }
    object.insert("schema_version".to_owned(), Value::from(12));
    Ok(value)
}

fn migrate_v12_to_v13(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 12,
            reason: "save root must be an object".to_owned(),
        })?;
    let player_dynasty_id = object
        .get("player_dynasty_id")
        .and_then(Value::as_u64)
        .ok_or_else(|| PersistenceError::Migration {
            version: 12,
            reason: "save has an invalid player dynasty".to_owned(),
        })?;
    let player_character_ids = v12_player_character_ids(object, player_dynasty_id)?;
    let mut supported_subjects = v12_nominated_support(object, &player_character_ids)?;
    migrate_v12_institution_memberships(object, &player_character_ids, &mut supported_subjects)?;
    append_migrated_patronage_records(object, supported_subjects);
    object.insert("schema_version".to_owned(), Value::from(13));
    Ok(value)
}

fn migrate_v13_to_v14(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 13,
            reason: "save root must be an object".to_owned(),
        })?;
    migrate_v13_information_targets(object)?;
    migrate_v13_parent_child_chronology(object)?;
    object.insert("schema_version".to_owned(), Value::from(14));
    Ok(value)
}

fn migrate_v14_to_v15(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 14,
            reason: "save root must be an object".to_owned(),
        })?;
    let active_directives = v14_active_office_directives(object)?;
    let institutions = object
        .get_mut("institutions")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 14,
            reason: "save institutions must be an object".to_owned(),
        })?;
    for institution in institutions.values_mut() {
        let institution =
            institution
                .as_object_mut()
                .ok_or_else(|| PersistenceError::Migration {
                    version: 14,
                    reason: "save institution must be an object".to_owned(),
                })?;
        let institution_id = institution
            .get("institution_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| PersistenceError::Migration {
                version: 14,
                reason: "save institution has an invalid institution_id".to_owned(),
            })?;
        institution.insert(
            "active_directive".to_owned(),
            active_directives
                .get(&institution_id)
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    object.insert("schema_version".to_owned(), Value::from(15));
    Ok(value)
}

fn v14_active_office_directives(
    object: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<u64, Value>, PersistenceError> {
    let current_day = object
        .get("clock")
        .and_then(Value::as_object)
        .and_then(|clock| clock.get("day"))
        .and_then(Value::as_i64)
        .ok_or_else(|| PersistenceError::Migration {
            version: 14,
            reason: "save clock has an invalid day".to_owned(),
        })?;
    let audit_log = object
        .get("audit_log")
        .and_then(Value::as_array)
        .ok_or_else(|| PersistenceError::Migration {
            version: 14,
            reason: "save audit_log must be an array".to_owned(),
        })?;
    let mut active = BTreeMap::<u64, (i64, Value)>::new();
    for record in audit_log {
        let Some(record) = record.as_object() else {
            continue;
        };
        if record.get("kind").and_then(Value::as_str) != Some("OfficeDirective") {
            continue;
        }
        let Some(day) = record.get("day").and_then(Value::as_i64) else {
            continue;
        };
        let expires_day = day.saturating_add(crate::systems::OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS);
        if current_day > expires_day {
            continue;
        }
        let Some(institution_id) = record
            .get("subject")
            .and_then(Value::as_str)
            .and_then(|subject| subject.strip_prefix("institution:"))
            .and_then(|value| value.parse::<u64>().ok())
        else {
            continue;
        };
        let Some(power) = record
            .get("detail")
            .and_then(Value::as_str)
            .and_then(v14_directive_power)
        else {
            continue;
        };
        let directive = serde_json::json!({
            "power": power,
            "expires_day": expires_day,
        });
        let replace = active
            .get(&institution_id)
            .is_none_or(|(prior_day, _)| day >= *prior_day);
        if replace {
            active.insert(institution_id, (day, directive));
        }
    }
    Ok(active
        .into_iter()
        .map(|(institution_id, (_, directive))| (institution_id, directive))
        .collect())
}

fn v14_directive_power(detail: &str) -> Option<&str> {
    let power = detail
        .split(';')
        .find_map(|field| field.strip_prefix("power="))?;
    matches!(
        power,
        "Licenses"
            | "Inspections"
            | "MarketTolls"
            | "DebtEnforcement"
            | "CityContracts"
            | "PublicWorks"
            | "WatchPriorities"
            | "Taxation"
            | "EmergencyImports"
    )
    .then_some(power)
}

fn migrate_v13_information_targets(
    object: &mut serde_json::Map<String, Value>,
) -> Result<(), PersistenceError> {
    let player_dynasty_id = object
        .get("player_dynasty_id")
        .and_then(Value::as_u64)
        .ok_or_else(|| PersistenceError::Migration {
            version: 13,
            reason: "save has an invalid player dynasty".to_owned(),
        })?;
    let dynasty_ids_by_name = v13_dynasty_ids_by_name(object)?;
    let registry = crate::registry::build_rivergate_registry();
    let good_ids_by_name: BTreeMap<_, _> = registry
        .goods()
        .iter()
        .map(|good| (good.name().to_owned(), u64::from(good.id().value())))
        .collect();
    let district_ids_by_name: BTreeMap<_, _> = registry
        .districts()
        .iter()
        .map(|district| (district.name().to_owned(), u64::from(district.id().value())))
        .collect();
    let reports = object
        .get_mut("information_reports")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 13,
            reason: "save information_reports must be an object".to_owned(),
        })?;
    for report in reports.values_mut() {
        let report = report
            .as_object_mut()
            .ok_or_else(|| PersistenceError::Migration {
                version: 13,
                reason: "save information report must be an object".to_owned(),
            })?;
        let subject = report
            .get("subject")
            .and_then(Value::as_str)
            .ok_or_else(|| PersistenceError::Migration {
                version: 13,
                reason: "save information report has an invalid subject".to_owned(),
            })?;
        let target = infer_v13_information_target(
            subject,
            player_dynasty_id,
            &good_ids_by_name,
            &dynasty_ids_by_name,
            &district_ids_by_name,
        );
        report.insert("target".to_owned(), target.unwrap_or(Value::Null));
    }
    Ok(())
}

fn infer_v13_information_target(
    subject: &str,
    player_dynasty_id: u64,
    good_ids_by_name: &BTreeMap<String, u64>,
    dynasty_ids_by_name: &BTreeMap<String, Vec<u64>>,
    district_ids_by_name: &BTreeMap<String, u64>,
) -> Option<Value> {
    let market_name = subject
        .strip_prefix("Commissioned market brief: ")
        .or_else(|| subject.strip_prefix("Monthly market report: "));
    if let Some(name) = market_name {
        let good_id = good_ids_by_name.get(name)?;
        return Some(serde_json::json!({ "Market": { "good_id": good_id } }));
    }
    let dynasty_name = subject
        .strip_prefix("Commissioned house brief: House ")
        .or_else(|| subject.strip_prefix("Counterparty report: House "));
    if let Some(name) = dynasty_name {
        let dynasty_id = dynasty_ids_by_name
            .get(name)?
            .iter()
            .copied()
            .find(|dynasty_id| *dynasty_id != player_dynasty_id)?;
        return Some(serde_json::json!({
            "Counterparty": { "dynasty_id": dynasty_id }
        }));
    }
    if let Some(name) = subject.strip_prefix("Commissioned district brief: ") {
        let district_id = district_ids_by_name.get(name)?;
        return Some(serde_json::json!({
            "District": { "district_id": district_id }
        }));
    }
    None
}

fn v13_dynasty_ids_by_name(
    object: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<String, Vec<u64>>, PersistenceError> {
    let dynasties = object
        .get("dynasties")
        .and_then(Value::as_object)
        .ok_or_else(|| PersistenceError::Migration {
            version: 13,
            reason: "save dynasties must be an object".to_owned(),
        })?;
    let mut ids_by_name: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for dynasty in dynasties.values() {
        let identity = dynasty
            .get("identity")
            .and_then(Value::as_object)
            .ok_or_else(|| PersistenceError::Migration {
                version: 13,
                reason: "save dynasty has an invalid identity".to_owned(),
            })?;
        let dynasty_id = identity.get("id").and_then(Value::as_u64).ok_or_else(|| {
            PersistenceError::Migration {
                version: 13,
                reason: "save dynasty has an invalid ID".to_owned(),
            }
        })?;
        let name = identity
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| PersistenceError::Migration {
                version: 13,
                reason: format!("save dynasty {dynasty_id} has an invalid name"),
            })?;
        ids_by_name
            .entry(name.to_owned())
            .or_default()
            .push(dynasty_id);
    }
    Ok(ids_by_name)
}

fn migrate_v13_parent_child_chronology(
    object: &mut serde_json::Map<String, Value>,
) -> Result<(), PersistenceError> {
    let births = v13_character_birth_days(object)?;
    let links = object
        .get_mut("family_links")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 13,
            reason: "save family_links must be an object".to_owned(),
        })?;
    for link in links.values_mut() {
        let link = link
            .as_object_mut()
            .ok_or_else(|| PersistenceError::Migration {
                version: 13,
                reason: "save family link must be an object".to_owned(),
            })?;
        if link.get("kind").and_then(Value::as_str) != Some("ParentChild") {
            continue;
        }
        let first_id = link
            .get("first_character_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| PersistenceError::Migration {
                version: 13,
                reason: "save parent-child link has an invalid first character".to_owned(),
            })?;
        let second_id = link
            .get("second_character_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| PersistenceError::Migration {
                version: 13,
                reason: "save parent-child link has an invalid second character".to_owned(),
            })?;
        let first_birth =
            births
                .get(&first_id)
                .copied()
                .ok_or_else(|| PersistenceError::Migration {
                    version: 13,
                    reason: format!(
                        "save parent-child link references missing character {first_id}"
                    ),
                })?;
        let second_birth =
            births
                .get(&second_id)
                .copied()
                .ok_or_else(|| PersistenceError::Migration {
                    version: 13,
                    reason: format!(
                        "save parent-child link references missing character {second_id}"
                    ),
                })?;
        if second_birth.saturating_sub(first_birth) < crate::core::MIN_PARENT_CHILD_AGE_GAP_DAYS {
            link.insert("kind".to_owned(), Value::String("Sibling".to_owned()));
        }
    }
    Ok(())
}

fn v13_character_birth_days(
    object: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<u64, i64>, PersistenceError> {
    object
        .get("characters")
        .and_then(Value::as_object)
        .and_then(|characters| characters.get("records"))
        .and_then(Value::as_object)
        .ok_or_else(|| PersistenceError::Migration {
            version: 13,
            reason: "save characters.records must be an object".to_owned(),
        })?
        .values()
        .map(|character| {
            let identity = character
                .get("identity")
                .and_then(Value::as_object)
                .ok_or_else(|| PersistenceError::Migration {
                    version: 13,
                    reason: "save character has an invalid identity".to_owned(),
                })?;
            let id = identity.get("id").and_then(Value::as_u64).ok_or_else(|| {
                PersistenceError::Migration {
                    version: 13,
                    reason: "save character has an invalid ID".to_owned(),
                }
            })?;
            let birth_day = identity
                .get("birth_day")
                .and_then(Value::as_i64)
                .ok_or_else(|| PersistenceError::Migration {
                    version: 13,
                    reason: format!("save character {id} has an invalid birth day"),
                })?;
            Ok((id, birth_day))
        })
        .collect()
}

fn v12_nominated_support(
    object: &serde_json::Map<String, Value>,
    player_character_ids: &BTreeSet<u64>,
) -> Result<BTreeMap<String, i64>, PersistenceError> {
    let audit_log = object
        .get("audit_log")
        .and_then(Value::as_array)
        .ok_or_else(|| PersistenceError::Migration {
            version: 12,
            reason: "save audit_log must be an array".to_owned(),
        })?;
    let mut supported_subjects = BTreeMap::new();
    for record in audit_log {
        let record = record
            .as_object()
            .ok_or_else(|| PersistenceError::Migration {
                version: 12,
                reason: "save audit record must be an object".to_owned(),
            })?;
        if record.get("kind").and_then(Value::as_str) != Some("OfficeNomination") {
            continue;
        }
        let subject = record
            .get("subject")
            .and_then(Value::as_str)
            .ok_or_else(|| PersistenceError::Migration {
                version: 12,
                reason: "office nomination audit has an invalid subject".to_owned(),
            })?;
        let Some((_, character_id)) = parse_institution_character_subject(subject) else {
            return Err(PersistenceError::Migration {
                version: 12,
                reason: format!("office nomination has malformed subject {subject}"),
            });
        };
        if player_character_ids.contains(&character_id) {
            let day = record.get("day").and_then(Value::as_i64).ok_or_else(|| {
                PersistenceError::Migration {
                    version: 12,
                    reason: "office nomination audit has an invalid day".to_owned(),
                }
            })?;
            supported_subjects
                .entry(subject.to_owned())
                .or_insert(day.saturating_sub(90).max(0));
        }
    }
    Ok(supported_subjects)
}

fn migrate_v12_institution_memberships(
    object: &mut serde_json::Map<String, Value>,
    player_character_ids: &BTreeSet<u64>,
    supported_subjects: &mut BTreeMap<String, i64>,
) -> Result<(), PersistenceError> {
    let institutions = object
        .get_mut("institutions")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 12,
            reason: "save institutions must be an object".to_owned(),
        })?;
    for (institution_key, institution) in institutions {
        migrate_v12_institution(
            institution_key,
            institution,
            player_character_ids,
            supported_subjects,
        )?;
    }
    Ok(())
}

fn migrate_v12_institution(
    institution_key: &str,
    institution: &mut Value,
    player_character_ids: &BTreeSet<u64>,
    supported_subjects: &mut BTreeMap<String, i64>,
) -> Result<(), PersistenceError> {
    let institution = institution
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 12,
            reason: "save institution must be an object".to_owned(),
        })?;
    let institution_id = institution
        .get("institution_id")
        .and_then(Value::as_u64)
        .or_else(|| institution_key.parse().ok())
        .ok_or_else(|| PersistenceError::Migration {
            version: 12,
            reason: format!("institution {institution_key} has an invalid ID"),
        })?;
    let office_holder_id = institution.get("office_holder_id").and_then(Value::as_u64);
    if let Some(character_id) = office_holder_id
        && player_character_ids.contains(&character_id)
    {
        let subject = format!("institution:{institution_id}:character:{character_id}");
        let term_day = institution
            .get("term_started_day")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        supported_subjects
            .entry(subject)
            .or_insert(term_day.saturating_sub(90).max(0));
    }
    let members = institution
        .get_mut("members")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 12,
            reason: format!("institution {institution_id} members must be an array"),
        })?;
    members.retain(|member| {
        member.as_u64().is_some_and(|character_id| {
            !player_character_ids.contains(&character_id)
                || supported_subjects.contains_key(&format!(
                    "institution:{institution_id}:character:{character_id}"
                ))
        })
    });
    if let Some(character_id) = office_holder_id
        && player_character_ids.contains(&character_id)
        && !members
            .iter()
            .any(|member| member.as_u64() == Some(character_id))
    {
        members.push(Value::from(character_id));
    }
    Ok(())
}

fn append_migrated_patronage_records(
    object: &mut serde_json::Map<String, Value>,
    supported_subjects: BTreeMap<String, i64>,
) {
    let audit_log = object
        .get_mut("audit_log")
        .and_then(Value::as_array_mut)
        .expect("validated audit log must remain an array");
    for (subject, day) in supported_subjects {
        audit_log.push(serde_json::json!({
            "day": day,
            "kind": "InstitutionPatronage",
            "subject": subject,
            "detail": "migrated_from_version_12=true"
        }));
    }
    audit_log.sort_by(compare_migrated_audit_records);
}

fn compare_migrated_audit_records(left: &Value, right: &Value) -> std::cmp::Ordering {
    let day = |value: &Value| value.get("day").and_then(Value::as_i64).unwrap_or(i64::MAX);
    let left_kind = left.get("kind").and_then(Value::as_str).unwrap_or_default();
    let right_kind = right
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    day(left)
        .cmp(&day(right))
        .then_with(|| {
            audit_migration_priority(left_kind).cmp(&audit_migration_priority(right_kind))
        })
        .then_with(|| {
            left.get("subject")
                .and_then(Value::as_str)
                .cmp(&right.get("subject").and_then(Value::as_str))
        })
}

fn v12_player_character_ids(
    object: &serde_json::Map<String, Value>,
    player_dynasty_id: u64,
) -> Result<BTreeSet<u64>, PersistenceError> {
    let records = object
        .get("characters")
        .and_then(Value::as_object)
        .and_then(|characters| characters.get("records"))
        .and_then(Value::as_object)
        .ok_or_else(|| PersistenceError::Migration {
            version: 12,
            reason: "save characters.records must be an object".to_owned(),
        })?;
    let mut ids = BTreeSet::new();
    for character in records.values() {
        let identity = character
            .get("identity")
            .and_then(Value::as_object)
            .ok_or_else(|| PersistenceError::Migration {
                version: 12,
                reason: "save character has an invalid identity".to_owned(),
            })?;
        if identity.get("dynasty_id").and_then(Value::as_u64) == Some(player_dynasty_id) {
            let character_id = identity.get("id").and_then(Value::as_u64).ok_or_else(|| {
                PersistenceError::Migration {
                    version: 12,
                    reason: "player character has an invalid ID".to_owned(),
                }
            })?;
            ids.insert(character_id);
        }
    }
    Ok(ids)
}

fn parse_institution_character_subject(subject: &str) -> Option<(u64, u64)> {
    let mut parts = subject.split(':');
    if parts.next()? != "institution" {
        return None;
    }
    let institution_id = parts.next()?.parse().ok()?;
    if parts.next()? != "character" {
        return None;
    }
    let character_id = parts.next()?.parse().ok()?;
    parts
        .next()
        .is_none()
        .then_some((institution_id, character_id))
}

fn audit_migration_priority(kind: &str) -> u8 {
    u8::from(kind != "InstitutionPatronage")
}

fn v11_business_owners(
    object: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<u64, u64>, PersistenceError> {
    object
        .get("businesses")
        .and_then(Value::as_object)
        .and_then(|businesses| businesses.get("records"))
        .and_then(Value::as_object)
        .ok_or_else(|| PersistenceError::Migration {
            version: 11,
            reason: "save businesses.records must be an object".to_owned(),
        })?
        .values()
        .map(|business| {
            let identity = business
                .get("identity")
                .and_then(Value::as_object)
                .ok_or_else(|| PersistenceError::Migration {
                    version: 11,
                    reason: "save business identity must be an object".to_owned(),
                })?;
            let business_id = identity.get("id").and_then(Value::as_u64).ok_or_else(|| {
                PersistenceError::Migration {
                    version: 11,
                    reason: "save business has an invalid id".to_owned(),
                }
            })?;
            let owner_dynasty_id = identity
                .get("owner_dynasty_id")
                .and_then(Value::as_u64)
                .ok_or_else(|| PersistenceError::Migration {
                    version: 11,
                    reason: format!("save business {business_id} has an invalid owner"),
                })?;
            Ok((business_id, owner_dynasty_id))
        })
        .collect()
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
