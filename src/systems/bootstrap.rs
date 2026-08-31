//! New-campaign assembly: the single deterministic validated path from `NewGameConfig` to `AppState`.
//!
//! Purpose: own the only way to build a Rivergate campaign so every
//! simulation, command, and persistence test shares one bootstrap contract.
//! Owns: `build_new_game`, name normalization, player/NPC foundation
//! insertion, household groups, strategic-state initialization, and final
//! `validate_invariants`.
//! Reads: `Registry` (immutable definitions).
//! Mutates: the freshly constructed `AppState` (no prior state).
//! Does not own: persistence IO, projection, or gameplay policy.
//! Invariants: validated treasury / capacity / inventory; founder ages
//! place first succession inside the playable horizon; RNG draws stay
//! state-owned for replay.
//! Focused tests: `src/systems/bootstrap_tests.rs`, persistence round-trip.

use crate::core::{
    AppState, AuditKind, AuditRecord, Business, BusinessFinance, BusinessIdentity,
    BusinessOperations, BusinessPolicy, BusinessStatus, BusinessStore, CURRENT_SCHEMA_VERSION,
    CampaignEvidenceMemo, CampaignPhase, Character, CharacterCapabilities, CharacterIdentity,
    CharacterRole, CharacterRuntime, CharacterStatus, CharacterStore, ChronicleEntry,
    ChronicleKind, Dynasty, DynastyIdentity, DynastyRelationships, DynastyResources,
    DynastyRuntime, HistoryLog, Household, HouseholdStore, MarketCause, MarketQuote, MarketState,
    NewGameConfig, NextIds, SimulationClock, SocialClass, StartingBackground,
};
use crate::ids::{CharacterId, DistrictId, DynastyId, RecipeId};
use crate::money::{Money, Quantity};
use crate::registry::Registry;
use crate::rng::DeterministicRng;
use std::collections::BTreeMap;
use thiserror::Error;

const MAX_DYNASTY_NAME_CHARACTERS: usize = 80;
const MAX_FOUNDER_NAME_CHARACTERS: usize = 120;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NewGameError {
    #[error("dynasty name must not be empty")]
    EmptyDynastyName,
    #[error("founder name must not be empty")]
    EmptyFounderName,
    #[error(
        "dynasty name contains {actual} characters, exceeding the supported maximum of {maximum}"
    )]
    DynastyNameTooLong { actual: usize, maximum: usize },
    #[error(
        "founder name contains {actual} characters, exceeding the supported maximum of {maximum}"
    )]
    FounderNameTooLong { actual: usize, maximum: usize },
    #[error("dynasty name contains unsupported control character {character:?}")]
    InvalidDynastyNameCharacter { character: char },
    #[error("founder name contains unsupported control character {character:?}")]
    InvalidFounderNameCharacter { character: char },
}

/// Builds a complete deterministic Rivergate campaign from authored definitions.
///
/// # Errors
///
/// Returns a dedicated error when a user-authored dynasty or founder name is empty, exceeds the
/// supported input limit, or contains a non-whitespace control character.
///
/// # Panics
///
/// Panics when the supplied registry is missing required Rivergate content.
pub fn build_new_game(
    registry: &Registry,
    config: NewGameConfig,
) -> Result<AppState, NewGameError> {
    let NewGameConfig {
        seed,
        dynasty_name,
        founder_name,
        background,
    } = config;
    let dynasty_name = normalize_player_name(&dynasty_name)
        .map_err(|character| NewGameError::InvalidDynastyNameCharacter { character })?;
    let founder_name = normalize_player_name(&founder_name)
        .map_err(|character| NewGameError::InvalidFounderNameCharacter { character })?;
    if dynasty_name.is_empty() {
        return Err(NewGameError::EmptyDynastyName);
    }
    if founder_name.is_empty() {
        return Err(NewGameError::EmptyFounderName);
    }
    let dynasty_name_characters = dynasty_name.chars().count();
    if dynasty_name_characters > MAX_DYNASTY_NAME_CHARACTERS {
        return Err(NewGameError::DynastyNameTooLong {
            actual: dynasty_name_characters,
            maximum: MAX_DYNASTY_NAME_CHARACTERS,
        });
    }
    let founder_name_characters = founder_name.chars().count();
    if founder_name_characters > MAX_FOUNDER_NAME_CHARACTERS {
        return Err(NewGameError::FounderNameTooLong {
            actual: founder_name_characters,
            maximum: MAX_FOUNDER_NAME_CHARACTERS,
        });
    }

    let mut state = empty_state(registry, seed);
    let player_dynasty_id = insert_player_foundation(
        &mut state,
        registry,
        &dynasty_name,
        &founder_name,
        background,
    );
    state.player_dynasty_id = player_dynasty_id;
    insert_npc_foundations(&mut state, registry);
    insert_household_groups(&mut state, registry);
    super::strategic::initialize_strategic_state(registry, &mut state);
    record_campaign_foundation(
        &mut state,
        player_dynasty_id,
        &dynasty_name,
        background,
        seed,
    );
    super::validate_invariants(registry, &state);
    Ok(state)
}

fn normalize_player_name(value: &str) -> Result<String, char> {
    let mut normalized = String::with_capacity(value.len());
    let mut needs_separator = false;
    for character in value.chars() {
        if character.is_whitespace() {
            needs_separator = !normalized.is_empty();
            continue;
        }
        if character.is_control() {
            return Err(character);
        }
        if needs_separator {
            normalized.push(' ');
            needs_separator = false;
        }
        normalized.push(character);
    }
    Ok(normalized)
}

fn empty_state(registry: &Registry, seed: u64) -> AppState {
    // Starting float in the market's internal cash pool. It is part of the
    // canonical scenario bootstrap, so changing it changes every campaign.
    const INITIAL_MARKET_CLEARING_COPPER: i64 = 2_000_000;
    let market = MarketState {
        quotes: registry
            .goods()
            .iter()
            .map(|good| {
                (
                    good.id(),
                    MarketQuote {
                        good_id: good.id(),
                        price: good.base_price(),
                        previous_price: good.base_price(),
                        stock: good.target_market_stock(),
                        target_stock: good.target_market_stock(),
                        demand_today: Quantity::ZERO,
                        supply_today: Quantity::ZERO,
                        causes: vec![MarketCause::StableConditions],
                    },
                )
            })
            .collect(),
        clearing_account: Money::from_copper(INITIAL_MARKET_CLEARING_COPPER),
        month_start_prices: registry
            .goods()
            .iter()
            .map(|good| (good.id(), good.base_price()))
            .collect(),
    };
    AppState {
        schema_version: CURRENT_SCHEMA_VERSION,
        scenario_key: registry.scenario().key().to_owned(),
        registry_fingerprint: registry.fingerprint(),
        clock: SimulationClock::new(),
        rng: DeterministicRng::seeded(seed),
        next_ids: NextIds::new(),
        player_dynasty_id: DynastyId::new(0),
        dynasties: BTreeMap::new(),
        characters: CharacterStore::new(),
        households: HouseholdStore::new(),
        businesses: BusinessStore::new(),
        institutions: BTreeMap::new(),
        market,
        contracts: BTreeMap::new(),
        loans: BTreeMap::new(),
        civic_debts: BTreeMap::new(),
        properties: BTreeMap::new(),
        employment: BTreeMap::new(),
        family_links: BTreeMap::new(),
        family_councils: BTreeMap::new(),
        laws: BTreeMap::new(),
        relationships: BTreeMap::new(),
        information_reports: BTreeMap::new(),
        ai_objectives: BTreeMap::new(),
        districts: BTreeMap::new(),
        public_works: BTreeMap::new(),
        legal_cases: BTreeMap::new(),
        external_routes: BTreeMap::new(),
        crises: BTreeMap::new(),
        outbox: HistoryLog::new(),
        chronicle: HistoryLog::new(),
        audit_log: HistoryLog::new(),
        campaign_evidence_memo: CampaignEvidenceMemo::default(),
    }
}

fn insert_player_foundation(
    state: &mut AppState,
    registry: &Registry,
    dynasty_name: &str,
    founder_name: &str,
    background: StartingBackground,
) -> DynastyId {
    let dynasty_id = insert_dynasty(
        state,
        dynasty_name,
        founder_name,
        Money::from_copper(58_000),
        82,
    );
    let head_id = state
        .dynasties
        .get(&dynasty_id)
        .expect("newly inserted player dynasty must exist")
        .head_id();
    let recipe_id = registry
        .get_recipe_id(background.recipe_key())
        .unwrap_or_else(|| panic!("missing starting recipe {}", background.recipe_key()));
    let district_id = match background {
        StartingBackground::Baker => required_district(registry, "southern_reach"),
        StartingBackground::ClothTrader => required_district(registry, "riverside"),
        StartingBackground::Blacksmith => required_district(registry, "northgate"),
    };
    insert_business(
        state,
        registry,
        BusinessSeed {
            owner_dynasty_id: dynasty_id,
            manager_id: head_id,
            district_id,
            recipe_id,
            name: background.business_name().to_owned(),
            cash: Money::from_copper(48_000),
            capacity_batches_per_day: 5,
        },
    );
    dynasty_id
}

fn insert_npc_foundations(state: &mut AppState, registry: &Registry) {
    for (index, seed) in npc_family_seeds().iter().enumerate() {
        let dynasty_id = insert_dynasty(
            state,
            seed.dynasty_name,
            seed.head_name,
            Money::from_copper(45_000 + i64::try_from(index).expect("index fits i64") * 5_000),
            64 + u16::try_from(index).expect("index fits u16") * 3,
        );
        let head_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("newly inserted NPC dynasty must exist")
            .head_id();
        let district_id = required_district(registry, seed.district_key);
        for (business_index, recipe_key) in seed.recipe_keys.iter().enumerate() {
            let recipe_id = registry
                .get_recipe_id(recipe_key)
                .unwrap_or_else(|| panic!("missing NPC recipe {recipe_key}"));
            let recipe = registry
                .get_recipe(recipe_id)
                .expect("resolved NPC recipe must exist");
            insert_business(
                state,
                registry,
                BusinessSeed {
                    owner_dynasty_id: dynasty_id,
                    manager_id: head_id,
                    district_id,
                    recipe_id,
                    name: format!("{} {}", seed.dynasty_name, recipe.name()),
                    cash: Money::from_copper(
                        38_000
                            + i64::try_from(business_index).expect("business index fits i64")
                                * 9_000,
                    ),
                    capacity_batches_per_day: 2 + u16::try_from(business_index)
                        .expect("business index fits u16"),
                },
            );
        }
    }
}

fn record_campaign_foundation(
    state: &mut AppState,
    player_id: DynastyId,
    dynasty_name: &str,
    background: StartingBackground,
    seed: u64,
) {
    let chronicle_id = state.next_ids.chronicle();
    state.chronicle.push(ChronicleEntry {
        id: chronicle_id,
        day: 0,
        kind: ChronicleKind::CampaignFounded,
        summary: format!(
            "House {dynasty_name} began its rise in Rivergate through {}.",
            background.business_name()
        ),
    });
    state.audit_log.push(AuditRecord {
        day: 0,
        kind: AuditKind::CampaignCreated,
        subject: format!("dynasty:{player_id}").into(),
        detail: format!("seed={seed}; background={background:?}").into(),
    });
}

#[derive(Clone, Copy, Debug)]
struct NpcFamilySeed {
    dynasty_name: &'static str,
    head_name: &'static str,
    district_key: &'static str,
    recipe_keys: &'static [&'static str],
}

fn npc_family_seeds() -> [NpcFamilySeed; 7] {
    [
        NpcFamilySeed {
            dynasty_name: "Sarno",
            head_name: "Mara Sarno",
            district_key: "market_ward",
            recipe_keys: &["grain_import", "milling"],
        },
        NpcFamilySeed {
            dynasty_name: "Bellafont",
            head_name: "Tomas Bellafont",
            district_key: "riverside",
            recipe_keys: &["brewing"],
        },
        NpcFamilySeed {
            dynasty_name: "Veyra",
            head_name: "Celene Veyra",
            district_key: "southern_reach",
            recipe_keys: &["wool_import", "weaving"],
        },
        NpcFamilySeed {
            dynasty_name: "Harrow",
            head_name: "Garran Harrow",
            district_key: "northgate",
            recipe_keys: &["timber_import", "charcoal_burning"],
        },
        NpcFamilySeed {
            dynasty_name: "Orsen",
            head_name: "Ilya Orsen",
            district_key: "northgate",
            recipe_keys: &["iron_import", "toolmaking"],
        },
        NpcFamilySeed {
            dynasty_name: "Dalmere",
            head_name: "Sabine Dalmere",
            district_key: "old_quarter",
            recipe_keys: &["baking"],
        },
        NpcFamilySeed {
            dynasty_name: "Quill",
            head_name: "Perrin Quill",
            district_key: "temple_hill",
            recipe_keys: &["grain_import"],
        },
    ]
}

fn required_district(registry: &Registry, key: &str) -> DistrictId {
    registry
        .get_district_id(key)
        .unwrap_or_else(|| panic!("missing required district {key}"))
}

fn insert_dynasty(
    state: &mut AppState,
    dynasty_name: &str,
    head_name: &str,
    treasury: Money,
    base_administration: u16,
) -> DynastyId {
    let dynasty_id = state.next_ids.dynasty();
    let head_id = state.next_ids.character();
    let heir_id = state.next_ids.character();

    let head_administration = base_administration
        .saturating_add(u16::try_from(state.rng.range_u32(18)).expect("random value fits u16"));
    let heir_administration = 45_u16
        .saturating_add(u16::try_from(state.rng.range_u32(35)).expect("random value fits u16"));

    state.characters.insert(Character {
        identity: CharacterIdentity {
            id: head_id,
            dynasty_id,
            name: head_name.to_owned(),
            // Founders begin near the age at which succession pressure becomes
            // material, so the first generational transition lands inside the
            // campaign session that builds the dynasty rather than beyond it.
            // At 52-54 years the median first succession sits near 600-800
            // days: late enough that a founder who pursues institutional
            // standing has offices and memberships worth testing when
            // succession arrives, early enough that continuity remains
            // ordinary play. Higher per-year age pressure (400 bp) plus
            // accelerated office eligibility ensures most dynasties can reach
            // institutional ascent before succession tests continuity.
            birth_day: -18_720 - i64::from(state.rng.range_u32(720)),
        },
        capabilities: CharacterCapabilities {
            administration: head_administration,
            commerce: 55_u16.saturating_add(
                u16::try_from(state.rng.range_u32(40)).expect("random value fits u16"),
            ),
            social: 40_u16.saturating_add(
                u16::try_from(state.rng.range_u32(50)).expect("random value fits u16"),
            ),
            craft: 50_u16.saturating_add(
                u16::try_from(state.rng.range_u32(45)).expect("random value fits u16"),
            ),
        },
        runtime: CharacterRuntime {
            status: CharacterStatus::Active,
            health_basis_points: 9_000,
            loyalty_basis_points: 10_000,
            role: CharacterRole::HeadOfHouse,
            incapacitated_day: None,
        },
    });

    state.characters.insert(Character {
        identity: CharacterIdentity {
            id: heir_id,
            dynasty_id,
            name: format!("{head_name} the Younger"),
            birth_day: -7_000 - i64::from(state.rng.range_u32(2_000)),
        },
        capabilities: CharacterCapabilities {
            administration: heir_administration,
            commerce: 45_u16.saturating_add(
                u16::try_from(state.rng.range_u32(35)).expect("random value fits u16"),
            ),
            social: 45_u16.saturating_add(
                u16::try_from(state.rng.range_u32(40)).expect("random value fits u16"),
            ),
            craft: 35_u16.saturating_add(
                u16::try_from(state.rng.range_u32(45)).expect("random value fits u16"),
            ),
        },
        runtime: CharacterRuntime {
            status: CharacterStatus::Active,
            health_basis_points: 9_500,
            loyalty_basis_points: 8_500,
            role: CharacterRole::Heir,
            incapacitated_day: None,
        },
    });

    let administrative_capacity = base_administration.saturating_add(head_administration / 2);
    let previous = state.dynasties.insert(
        dynasty_id,
        Dynasty {
            identity: DynastyIdentity {
                id: dynasty_id,
                name: dynasty_name.to_owned(),
            },
            resources: DynastyResources {
                treasury,
                civic_contributions: Money::ZERO,
                unmet_office_duties: 0,
                legitimacy_basis_points: 4_500,
                administrative_capacity,
                administrative_load: 0,
                reputation_quality_basis_points: 5_000,
                reputation_reliability_basis_points: 5_000,
            },
            relationships: DynastyRelationships {
                head_id,
                heir_id: Some(heir_id),
            },
            runtime: DynastyRuntime {
                phase: CampaignPhase::Foundation,
                generation: 1,
                succession_risk_basis_points: 1_200,
            },
        },
    );
    assert!(previous.is_none(), "duplicate dynasty ID {dynasty_id}");
    dynasty_id
}

#[derive(Clone, Debug)]
struct BusinessSeed {
    owner_dynasty_id: DynastyId,
    manager_id: CharacterId,
    district_id: DistrictId,
    recipe_id: RecipeId,
    name: String,
    cash: Money,
    capacity_batches_per_day: u16,
}

fn insert_business(state: &mut AppState, registry: &Registry, seed: BusinessSeed) {
    let BusinessSeed {
        owner_dynasty_id,
        manager_id,
        district_id,
        recipe_id,
        name,
        cash,
        capacity_batches_per_day,
    } = seed;
    let recipe = registry
        .get_recipe(recipe_id)
        .unwrap_or_else(|| panic!("missing recipe {recipe_id}"));
    let business_id = state.next_ids.business();
    let mut inventory = BTreeMap::new();
    for input in recipe.inputs() {
        inventory.insert(input.good_id(), input.quantity().saturating_mul_ratio(4, 1));
    }

    state.businesses.insert(Business {
        identity: BusinessIdentity {
            id: business_id,
            name,
            owner_dynasty_id,
            district_id,
            recipe_id,
        },
        operations: BusinessOperations {
            manager_id,
            capacity_batches_per_day,
            condition_basis_points: 8_500,
            quality_basis_points: 6_500,
            status: BusinessStatus::Active,
        },
        finance: BusinessFinance {
            cash,
            version: 0,
            lifetime_revenue: Money::ZERO,
            lifetime_costs: Money::ZERO,
        },
        inventory,
        policy: BusinessPolicy::default(),
        premises_property_id: None,
    });

    let dynasty = state
        .dynasties
        .get_mut(&owner_dynasty_id)
        .expect("business owner dynasty must exist");
    dynasty.resources.administrative_load = dynasty
        .resources
        .administrative_load
        .saturating_add(recipe.administrative_load());
}

fn insert_household_groups(state: &mut AppState, registry: &Registry) {
    let classes = [
        SocialClass::Laboring,
        SocialClass::Laboring,
        SocialClass::Laboring,
        SocialClass::Artisan,
        SocialClass::Artisan,
        SocialClass::Merchant,
    ];

    for district in registry.districts() {
        for (group_index, social_class) in classes.iter().enumerate() {
            let members = u16::try_from(
                district.population() / 6
                    + state.rng.range_u32(80)
                    + u32::try_from(group_index).expect("group index fits u32"),
            )
            .unwrap_or(u16::MAX);
            let income_multiplier = match social_class {
                SocialClass::Laboring => 1,
                SocialClass::Artisan => 2,
                SocialClass::Merchant => 3,
            };
            // Weekly income scales with household size so living costs (28/52/78 per
            // member monthly) remain coverable: a 1_200-member neighborhood group
            // earning a flat 1_800/week could never pay 33_600/month rent-equivalent
            // pressure. Per-member weekly income mirrors the per-member monthly
            // cost (7/13/19 copper weekly) plus a small margin for food, so
            // large groups stay solvent while still facing pressure when districts
            // degrade or food spikes.
            let per_member_weekly_copper: i64 = match social_class {
                SocialClass::Laboring => 9,
                SocialClass::Artisan => 14,
                SocialClass::Merchant => 19,
            };
            let weekly_income =
                Money::from_copper(i64::from(members).saturating_mul(per_member_weekly_copper));
            let household_id = state.next_ids.household();
            state.households.insert(Household {
                id: household_id,
                district_id: district.id(),
                members,
                social_class: *social_class,
                // Three weeks buffer so a brief income shock does not instantly
                // zero the household; district unrest still captures sustained pressure.
                cash: weekly_income
                    .saturating_mul(3)
                    .max(Money::from_copper(3_000)),
                weekly_income,
                bread_need_daily: Quantity::from_milliunits(500 * income_multiplier.min(2)),
                // Ale need saturates with income like bread: the city's single
                // brewery covers roughly the capped demand, mirroring how
                // cloth demand is calibrated just under weaving capacity. An
                // uncapped ×4 merchant thirst would structurally exceed
                // supply and permanently bid ale past every household's food
                // budget.
                ale_need_daily: Quantity::from_milliunits(500 * income_multiplier.min(2)),
                food_satisfaction_basis_points: 8_000,
            });
        }
    }
}

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
