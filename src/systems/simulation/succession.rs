//! Annual succession, health, and dynasty lifecycle.
//!
//! Purpose: own the yearly boundary that advances the campaign clock, rolls
//! character health, reconciles incapacitation/death, and executes dynastic
//! successions so the daily economic pipeline stays focused on markets.
//! Owns: `process_year_boundary` (chronicle + succession orchestration),
//! `update_character_health`, `designate_emergency_heirs`, and every
//! succession helper (`SuccessionLine`, `succession_shock`, etc.).
//! Reads: `Registry` (scenario start year), `AppState` characters/dynasties.
//! Mutates: character health/status, dynasty heads/heirs, family councils,
//! business managers, institutional memberships, chronicle/audit/outbox.
//! Does not own: daily production/market logic (parent `mod.rs`) or
//! strategic weekly/monthly hooks.
//! Canonical operations: `process_year_boundary` — the single yearly entry
//! called by `run_one_day` on `is_year_boundary`.
//! Relevant invariants: heir age floor (18y), parent-child gap (12y), no
//! business keeps an inactive manager, collapsed health window (3y), succession
//! shock preserves legitimacy floors.
//! Focused tests: `src/systems/simulation/simulation_tests.rs` succession.

use crate::core::{
    AppState, AuditKind, Character, CharacterCapabilities, CharacterIdentity, CharacterRole,
    CharacterRuntime, CharacterStatus, ChronicleEntry, ChronicleKind, CrisisKind, FamilyLink,
    FamilyLinkKind, HouseGovernance, OutboxKind,
};
use crate::ids::{BusinessId, CharacterId, DynastyId};
use crate::systems::SimulationError;
use crate::systems::transactions::{checked_future_day, next_family_charter_version};
use std::collections::{BTreeMap, BTreeSet};

use super::{
    AGE_PRESSURE_PER_YEAR_OVER_ELIGIBILITY, COLLAPSED_HEALTH_SURVIVABLE_FLOOR,
    SUCCESSION_ACCESSION_HEALTH_FLOOR, SUCCESSION_ELIGIBILITY_AGE_YEARS,
};

#[derive(Clone, Debug)]
pub(crate) struct SuccessionLine {
    pub(crate) dynasty_id: DynastyId,
    pub(crate) outgoing_head_id: CharacterId,
    pub(crate) incoming_head_id: CharacterId,
    pub(crate) formally_prepared: bool,
    pub(crate) family_unity_loss: u16,
    pub(crate) family_loyalty_loss: u16,
    pub(crate) legitimacy_loss: u16,
    pub(crate) new_heir_name: String,
    pub(crate) new_heir_birth_day: i64,
    pub(crate) new_heir_link_kind: FamilyLinkKind,
    pub(crate) next_generation: u16,
    pub(crate) next_charter_version: u64,
    pub(crate) new_heir_capabilities: CharacterCapabilities,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SuccessionShock {
    formally_prepared: bool,
    family_unity_loss: u16,
    family_loyalty_loss: u16,
    legitimacy_loss: u16,
}

pub(crate) fn process_year_boundary(
    registry: &crate::registry::Registry,
    state: &mut AppState,
) -> Result<(), SimulationError> {
    let year = state.clock.year(registry.scenario().start_year());
    let id = state.next_ids.try_chronicle()?;
    state.chronicle.push(ChronicleEntry {
        id,
        day: state.clock.day(),
        kind: ChronicleKind::NewYear,
        summary: format!("Rivergate entered the year {year}."),
    });

    update_succession_risks(state);
    update_character_health(state)?;
    let succession_plan = decide_successions(state)?;
    apply_successions(state, succession_plan)?;
    Ok(())
}

pub(crate) fn update_character_health(state: &mut AppState) -> Result<(), SimulationError> {
    // An incapacitated member has already left every active duty; a bounded
    // window of collapsed health eventually claims them instead of leaving
    // an inert record that can neither recover nor die.
    const INCAPACITATED_DEATH_WINDOW_DAYS: i64 = 3 * 360;
    let epidemic_severity = state
        .crises
        .values()
        .filter(|crisis| crisis.kind == CrisisKind::Epidemic && crisis.status.is_active())
        .map(|crisis| crisis.severity_basis_points)
        .max()
        .unwrap_or(0);
    let day = state.clock.day();
    let head_ids: BTreeSet<_> = state
        .dynasties
        .values()
        .map(crate::core::Dynasty::head_id)
        .collect();
    let heir_ids: BTreeSet<_> = state
        .dynasties
        .values()
        .filter_map(crate::core::Dynasty::heir_id)
        .collect();
    let mut newly_incapacitated = Vec::new();
    for character in state.characters.iter_mut() {
        if character.status() != CharacterStatus::Active {
            continue;
        }
        let age_years = day.saturating_sub(character.birth_day()) / 360;
        let resolved_health = resolve_annual_health(
            character.runtime.health_basis_points,
            age_years,
            epidemic_severity,
        );
        // A designated heir whose resolved health collapses is pinned at a
        // survivable floor for this year instead of becoming incapacitated:
        // succession needs a live designated heir, and the floor is lifted on
        // accession (SUCCESSION_ACCESSION_HEALTH_FLOOR). Heads are not
        // pinned: a sole head with collapsed health proceeds to forced
        // succession via the emergency-heir path, so no dynasty becomes
        // immortal at 1 hp while its house has no other members.
        let pinned_at_survivable_floor = resolved_health == 0 && heir_ids.contains(&character.id());
        character.runtime.health_basis_points = if pinned_at_survivable_floor {
            COLLAPSED_HEALTH_SURVIVABLE_FLOOR
        } else {
            resolved_health
        };
        if character.runtime.health_basis_points == 0 && !head_ids.contains(&character.id()) {
            if character.runtime.incapacitated_day.is_none() {
                character.runtime.incapacitated_day = Some(state.clock.day());
            }
            character.runtime.status = CharacterStatus::Incapacitated;
            newly_incapacitated.push((
                character.id(),
                character.dynasty_id(),
                character.name().to_owned(),
            ));
        }
    }
    for (character_id, dynasty_id, character_name) in newly_incapacitated {
        synchronize_character_incapacitation(state, character_id, dynasty_id, &character_name)?;
    }
    // An incapacitated member has already left every active duty; a bounded
    // window of collapsed health eventually claims them instead of leaving
    // an inert record that can neither recover nor die.
    let day = state.clock.day();
    let dying_ids: Vec<(CharacterId, DynastyId)> = state
        .characters
        .iter()
        .filter(|character| character.status() == CharacterStatus::Incapacitated)
        .filter(|character| {
            character
                .runtime
                .incapacitated_day
                .is_some_and(|collapsed_day| {
                    day.saturating_sub(collapsed_day) >= INCAPACITATED_DEATH_WINDOW_DAYS
                })
        })
        .map(|character| (character.id(), character.dynasty_id()))
        .collect();
    for (character_id, dynasty_id) in dying_ids {
        retire_incapacitated_member(state, character_id);
        if dynasty_id == state.player_dynasty_id {
            super::super::strategic::try_push_outbox(
                state,
                OutboxKind::Family,
                format!("Character {character_id} passed away"),
                "A family member who had been incapacitated by collapsed health has died."
                    .to_owned(),
            )?;
        }
    }
    reconcile_inactive_business_managers(state);
    designate_emergency_heirs(state);
    Ok(())
}

/// A head whose health has collapsed with no designated heir must not keep
/// running the house indefinitely: designate the most capable adult member
/// as emergency heir so this year's succession pass can execute normally.
/// When a house has no other active members at all, an emergency heir is
/// generated instead of pinning the head forever at 1 hp — the dynasty
/// remains mortal and succession tests the rebuilt house rather than an
/// immortal founder.
pub(crate) fn designate_emergency_heirs(state: &mut AppState) {
    let needing: Vec<DynastyId> = state
        .dynasties
        .values()
        .filter(|dynasty| {
            dynasty.heir_id().is_none()
                && state
                    .characters
                    .get(dynasty.head_id())
                    .is_some_and(|head| head.runtime.health_basis_points == 0)
        })
        .map(crate::core::Dynasty::id)
        .collect();
    for dynasty_id in needing {
        if let Some(successor_id) = emergency_successor(
            state,
            state
                .dynasties
                .get(&dynasty_id)
                .expect("dynasty must exist")
                .head_id(),
        ) {
            state
                .dynasties
                .get_mut(&dynasty_id)
                .expect("emergency succession dynasty must exist")
                .relationships
                .heir_id = Some(successor_id);
            continue;
        }
        // No other active member exists: generate a successor so the
        // collapsed sole head can retire without leaving an immortal
        // Active 0-health head or a headless dynasty. The new heir is a
        // young adult ward-like successor tied to the dynasty by history.
        let head_id = state
            .dynasties
            .get(&dynasty_id)
            .expect("dynasty must exist")
            .head_id();
        let (birth_day, link_kind, capabilities) = generate_next_heir(state, head_id);
        let dynasty_name = state
            .dynasties
            .get(&dynasty_id)
            .expect("dynasty must exist")
            .name()
            .to_owned();
        let generation = state
            .dynasties
            .get(&dynasty_id)
            .expect("dynasty must exist")
            .runtime
            .generation;
        let new_heir_name = format!("{dynasty_name} Heir {generation}");
        let new_heir_id = {
            let mut next_ids = state.next_ids.clone();
            let id = next_ids
                .try_character()
                .expect("emergency heir character space must be available");
            let link_id = next_ids
                .try_family_link()
                .expect("emergency heir link space must be available");
            state.next_ids = next_ids;
            state.characters.insert(Character {
                identity: CharacterIdentity {
                    id,
                    dynasty_id,
                    name: new_heir_name,
                    birth_day,
                },
                capabilities,
                runtime: CharacterRuntime {
                    status: CharacterStatus::Active,
                    health_basis_points: 9_500,
                    loyalty_basis_points: 8_000,
                    role: CharacterRole::Heir,
                    incapacitated_day: None,
                },
            });
            state.family_links.insert(
                link_id,
                FamilyLink {
                    id: link_id,
                    first_character_id: head_id,
                    second_character_id: id,
                    kind: link_kind,
                    active: true,
                },
            );
            state
                .family_councils
                .get_mut(&dynasty_id)
                .expect("council must exist")
                .members
                .insert(id);
            id
        };
        state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("dynasty must exist")
            .relationships
            .heir_id = Some(new_heir_id);
    }
}

/// Selects the most capable adult active dynasty member other than the head,
/// breaking ties by stable ID order. When the house has no adult besides the
/// head, the most capable active member of any age succeeds instead, so a
/// household of minors still designates an heir and the annual succession
/// pass can execute rather than leaving a headless house operating.
pub(crate) fn emergency_successor(state: &AppState, head_id: CharacterId) -> Option<CharacterId> {
    let head = state.characters.get(head_id)?;
    let candidate = |character: &&crate::core::Character| {
        character.dynasty_id() == head.dynasty_id()
            && character.id() != head_id
            && character.status() == CharacterStatus::Active
    };
    let mut candidates: Vec<_> = state.characters.iter().filter(candidate).collect();
    if candidates.iter().all(|character| {
        state.clock.day().saturating_sub(character.birth_day())
            < crate::systems::commands::HEIR_MINIMUM_AGE_DAYS
    }) {
        return candidates
            .into_iter()
            .max_by_key(|character| emergency_successor_rank(character))
            .map(crate::core::Character::id);
    }
    candidates.retain(|character| {
        state.clock.day().saturating_sub(character.birth_day())
            >= crate::systems::commands::HEIR_MINIMUM_AGE_DAYS
    });
    candidates
        .into_iter()
        .max_by_key(|character| emergency_successor_rank(character))
        .map(crate::core::Character::id)
}

/// Capability sum with stable typed-ID tie-breaking.
pub(crate) fn emergency_successor_rank(character: &crate::core::Character) -> (u32, u32) {
    (
        u32::from(character.capabilities.administration)
            + u32::from(character.capabilities.commerce)
            + u32::from(character.capabilities.social)
            + u32::from(character.capabilities.craft),
        character.id().value(),
    )
}

/// Vacates any office held by `character_id` (clamping the replacement
/// selection day) and removes them from every institutional membership.
pub(crate) fn vacate_character_institutional_roles(
    state: &mut AppState,
    character_id: CharacterId,
    replacement_selection_day: Option<i64>,
) {
    for institution in state.institutions.values_mut() {
        institution.members.remove(&character_id);
        if institution.office_holder_id == Some(character_id) {
            institution.office_holder_id = None;
            institution.next_selection_day = institution
                .next_selection_day
                .min(replacement_selection_day.expect("office replacement day was preflighted"));
        }
    }
}

/// Hands management of a character's businesses to `replacement_manager_id`.
pub(crate) fn reassign_managed_businesses(
    state: &mut AppState,
    dynasty_id: DynastyId,
    character_id: CharacterId,
    replacement_manager_id: CharacterId,
) {
    let managed_business_ids: Vec<_> = state
        .businesses
        .ids_for_owner(dynasty_id)
        .into_iter()
        .flatten()
        .copied()
        .filter(|business_id| {
            state
                .businesses
                .get(*business_id)
                .is_some_and(|business| business.manager_id() == character_id)
        })
        .collect();
    for business_id in managed_business_ids {
        state
            .businesses
            .get_mut(business_id)
            .expect("owner business index must resolve")
            .operations
            .manager_id = replacement_manager_id;
    }
}

/// Picks the active dynasty member who should take over management duties from
/// an inactive character. The head is preferred while active; otherwise the
/// most capable active member takes over, so an operating business is never
/// left in the hands of an incapacitated or deceased manager.
pub(crate) fn resolve_active_management_successor(
    state: &AppState,
    dynasty_id: DynastyId,
    departing_character_id: CharacterId,
) -> Option<CharacterId> {
    let dynasty = state.dynasties.get(&dynasty_id)?;
    let head_id = dynasty.head_id();
    if head_id != departing_character_id
        && state
            .characters
            .get(head_id)
            .is_some_and(|head| head.status() == CharacterStatus::Active)
    {
        return Some(head_id);
    }
    state
        .characters
        .iter()
        .filter(|character| {
            character.dynasty_id() == dynasty_id
                && character.id() != departing_character_id
                && character.status() == CharacterStatus::Active
        })
        .max_by_key(|character| emergency_successor_rank(character))
        .map(crate::core::Character::id)
}

/// Annual reconciliation: no business may keep a manager whose health or
/// succession has taken them out of active standing. The per-character handoff
/// in `synchronize_character_incapacitation` can target a head that a later
/// succession retires in the same pass, so this sweep guarantees the lifecycle
/// invariant instead of trusting once-per-character ordering.
pub(crate) fn reconcile_inactive_business_managers(state: &mut AppState) {
    let stale_managers: Vec<(BusinessId, DynastyId)> = state
        .businesses
        .iter()
        .filter(|business| {
            state
                .characters
                .get(business.manager_id())
                .is_none_or(|manager| manager.status() != CharacterStatus::Active)
        })
        .map(|business| (business.id(), business.owner_dynasty_id()))
        .collect();
    for (business_id, dynasty_id) in stale_managers {
        let departing_manager_id = state
            .businesses
            .get(business_id)
            .map(crate::core::Business::manager_id)
            .expect("stale-manager business must exist");
        if let Some(successor_id) =
            resolve_active_management_successor(state, dynasty_id, departing_manager_id)
        {
            reassign_managed_businesses(state, dynasty_id, departing_manager_id, successor_id);
        }
    }
}

pub(crate) fn synchronize_character_incapacitation(
    state: &mut AppState,
    character_id: CharacterId,
    dynasty_id: DynastyId,
    character_name: &str,
) -> Result<(), SimulationError> {
    let replacement_selection_day = state
        .institutions
        .values()
        .any(|institution| institution.office_holder_id == Some(character_id))
        .then(|| checked_future_day(state.clock.day(), 30))
        .transpose()?;
    for link in state.family_links.values_mut().filter(|link| {
        link.active
            && matches!(link.kind, FamilyLinkKind::Ward | FamilyLinkKind::Marriage)
            && (link.first_character_id == character_id || link.second_character_id == character_id)
    }) {
        link.active = false;
    }
    state
        .family_councils
        .get_mut(&dynasty_id)
        .expect("character dynasty must have a family council")
        .members
        .remove(&character_id);
    vacate_character_institutional_roles(state, character_id, replacement_selection_day);
    if let Some(replacement_manager_id) =
        resolve_active_management_successor(state, dynasty_id, character_id)
    {
        reassign_managed_businesses(state, dynasty_id, character_id, replacement_manager_id);
    }
    if dynasty_id == state.player_dynasty_id {
        super::super::strategic::try_push_outbox(
            state,
            OutboxKind::Family,
            format!("{character_name} became incapacitated"),
            format!(
                "Character {character_id} left active family, institutional, and business duties because their health reached zero."
            ),
        )?;
    }
    Ok(())
}

/// Marks a long-incapacitated member as deceased. Incapacitation already
/// vacated every council, institutional, and management duty, so only the
/// status and any surviving active family links need closing.
pub(crate) fn retire_incapacitated_member(state: &mut AppState, character_id: CharacterId) {
    state
        .characters
        .get_mut(character_id)
        .expect("incapacitated character must exist")
        .runtime
        .status = CharacterStatus::Deceased;
    for link in state.family_links.values_mut().filter(|link| {
        link.active
            && (link.first_character_id == character_id || link.second_character_id == character_id)
    }) {
        link.active = false;
    }
}

pub(crate) fn resolve_annual_health(current: u16, age_years: i64, epidemic_severity: u16) -> u16 {
    if current == 0 {
        return 0;
    }
    let age_delta = match age_years {
        ..=39 => 100,
        40..=54 => -100,
        55..=69 => -300,
        _ => -700,
    };
    let epidemic_penalty = i32::from(epidemic_severity / 10);
    i32::from(current)
        .saturating_add(age_delta)
        .saturating_sub(epidemic_penalty)
        .clamp(0, 10_000)
        .try_into()
        .expect("clamped health must fit u16")
}

pub(crate) fn update_succession_risks(state: &mut AppState) {
    let governance: BTreeMap<_, _> = state
        .family_councils
        .iter()
        .map(|(dynasty_id, council)| (*dynasty_id, council.governance))
        .collect();
    let office_loads: BTreeMap<_, _> = state
        .dynasties
        .keys()
        .copied()
        .map(|dynasty_id| {
            (
                dynasty_id,
                super::super::strategic::dynasty_office_administrative_load(state, dynasty_id),
            )
        })
        .collect();
    for dynasty in state.dynasties.values_mut() {
        let office_load = office_loads.get(&dynasty.id()).copied().unwrap_or(0);
        let overextension = dynasty
            .administrative_load()
            .saturating_add(office_load)
            .saturating_sub(dynasty.administrative_capacity());
        let base_risk = i32::from(
            1_000_u16
                .saturating_add(overextension.saturating_mul(25))
                .min(9_500),
        );
        let governance_adjustment = match governance
            .get(&dynasty.id())
            .copied()
            .unwrap_or(HouseGovernance::Primogeniture)
        {
            HouseGovernance::HeadCommand => 500,
            HouseGovernance::Primogeniture => -400,
            HouseGovernance::FamilyPartnership => -250,
            HouseGovernance::BranchFederation => 200,
            HouseGovernance::ElectedHead => 700,
        };
        dynasty.runtime.succession_risk_basis_points = u16::try_from(
            base_risk
                .saturating_add(governance_adjustment)
                .clamp(0, 9_500),
        )
        .expect("clamped succession risk must fit u16");
    }
}

pub(crate) fn decide_successions(
    state: &mut AppState,
) -> Result<Vec<SuccessionLine>, SimulationError> {
    let snapshots: Vec<_> = state
        .dynasties
        .values()
        .filter_map(|dynasty| {
            dynasty.heir_id().map(|heir_id| {
                (
                    dynasty.id(),
                    dynasty.name().to_owned(),
                    dynasty.head_id(),
                    heir_id,
                    dynasty.runtime.generation,
                    dynasty.runtime.succession_risk_basis_points,
                )
            })
        })
        .collect();
    let mut lines = Vec::new();

    for (dynasty_id, dynasty_name, head_id, heir_id, generation, succession_risk_basis_points) in
        snapshots
    {
        let head = state
            .characters
            .get(head_id)
            .expect("dynasty head reference must be valid");
        let age_days = state.clock.day().saturating_sub(head.birth_day());
        let age_years = age_days / 360;
        let health_forces_succession = head.runtime.health_basis_points == 0;
        if age_years < SUCCESSION_ELIGIBILITY_AGE_YEARS && !health_forces_succession {
            continue;
        }
        let annual_chance = succession_chance_basis_points(
            age_years,
            succession_risk_basis_points,
            head.runtime.health_basis_points,
        );
        if !health_forces_succession && !state.rng.is_chance_success(annual_chance) {
            continue;
        }
        let next_generation = generation
            .checked_add(1)
            .filter(|next| *next < u16::MAX)
            .ok_or(SimulationError::DynastyGenerationExhausted { dynasty_id })?;
        let current_charter_version = state
            .family_councils
            .get(&dynasty_id)
            .expect("succession dynasty must have a family council")
            .charter_version;
        let next_charter_version =
            next_family_charter_version(dynasty_id, current_charter_version)?;
        let SuccessionShock {
            formally_prepared,
            family_unity_loss,
            family_loyalty_loss,
            legitimacy_loss,
        } = succession_shock(state, dynasty_id, heir_id, succession_risk_basis_points);
        let (new_heir_birth_day, new_heir_link_kind, new_heir_capabilities) =
            generate_next_heir(state, heir_id);
        lines.push(SuccessionLine {
            dynasty_id,
            outgoing_head_id: head_id,
            incoming_head_id: heir_id,
            formally_prepared,
            family_unity_loss,
            family_loyalty_loss,
            legitimacy_loss,
            new_heir_name: format!("{dynasty_name} Heir {next_generation}"),
            new_heir_birth_day,
            new_heir_link_kind,
            next_generation,
            next_charter_version,
            new_heir_capabilities,
        });
    }

    Ok(lines)
}

pub(crate) fn heir_was_formally_prepared(
    state: &AppState,
    dynasty_id: DynastyId,
    incoming_head_id: CharacterId,
) -> bool {
    let subject = format!("dynasty:{dynasty_id}");
    // Any designation naming the incoming head counts, not just the most
    // recent one: a later re-designation of a different heir must not erase
    // an earlier formal preparation of the character who actually succeeds.
    state
        .audit_log
        .iter()
        .filter(|record| record.kind() == AuditKind::HeirDesignation && record.subject() == subject)
        .any(|record| super::super::heir_audit_detail_matches(record, incoming_head_id))
}

pub(crate) fn succession_shock(
    state: &AppState,
    dynasty_id: DynastyId,
    incoming_head_id: CharacterId,
    succession_risk_basis_points: u16,
) -> SuccessionShock {
    let formally_prepared = heir_was_formally_prepared(state, dynasty_id, incoming_head_id);
    if formally_prepared {
        // Harness showed stranded recoveries after succession: legitimacy at
        // 15 bp and no patronage for 720 days despite building political
        // embedding. Trim losses so a prepared heir can actually rebuild.
        SuccessionShock {
            formally_prepared,
            family_unity_loss: 1_000_u16
                .saturating_add(succession_risk_basis_points / 8)
                .min(2_200),
            family_loyalty_loss: 350_u16
                .saturating_add(succession_risk_basis_points / 12)
                .min(900),
            legitimacy_loss: succession_risk_basis_points / 10,
        }
    } else {
        SuccessionShock {
            formally_prepared,
            family_unity_loss: 2_200_u16
                .saturating_add(succession_risk_basis_points / 3)
                .min(4_500),
            family_loyalty_loss: 900_u16
                .saturating_add(succession_risk_basis_points / 6)
                .min(2_000),
            legitimacy_loss: succession_risk_basis_points / 3,
        }
    }
}

pub(crate) fn generate_next_heir(
    state: &mut AppState,
    incoming_head_id: CharacterId,
) -> (i64, FamilyLinkKind, CharacterCapabilities) {
    let incoming_age_days = state.clock.day().saturating_sub(
        state
            .characters
            .get(incoming_head_id)
            .expect("dynasty heir reference must be valid")
            .birth_day(),
    );
    let parent_child_age_requirement =
        (20 * 360_i64).saturating_add(crate::core::MIN_PARENT_CHILD_AGE_GAP_DAYS);
    let incoming_birth_day = state
        .characters
        .get(incoming_head_id)
        .expect("dynasty heir reference must be valid")
        .birth_day();
    let (birth_day, link_kind) = if incoming_age_days >= parent_child_age_requirement {
        (
            state.clock.day().saturating_sub(20 * 360),
            FamilyLinkKind::ParentChild,
        )
    } else {
        // A generated sibling must always be younger than the incoming head,
        // even when forced succession elevates a child or adolescent heir.
        // Enforce at least a one-year gap so siblings are not 1 day apart,
        // which would be biologically incoherent and could create a minor
        // heir whose sibling is practically the same age.
        (
            state
                .clock
                .day()
                .saturating_sub(18 * 360)
                .max(incoming_birth_day.saturating_add(360)),
            FamilyLinkKind::Sibling,
        )
    };
    let capabilities = CharacterCapabilities {
        administration: 40_u16
            .saturating_add(u16::try_from(state.rng.range_u32(50)).expect("random value fits u16")),
        commerce: 40_u16
            .saturating_add(u16::try_from(state.rng.range_u32(50)).expect("random value fits u16")),
        social: 40_u16
            .saturating_add(u16::try_from(state.rng.range_u32(50)).expect("random value fits u16")),
        craft: 30_u16
            .saturating_add(u16::try_from(state.rng.range_u32(55)).expect("random value fits u16")),
    };
    (birth_day, link_kind, capabilities)
}

pub(crate) fn succession_chance_basis_points(
    age_years: i64,
    succession_risk_basis_points: u16,
    health_basis_points: u16,
) -> u16 {
    if age_years < SUCCESSION_ELIGIBILITY_AGE_YEARS {
        return 0;
    }
    // The ramp must mature succession pressure inside the session that builds
    // the dynasty: founders begin at 56-58 years old, so this rate puts the
    // median first transition in the second or third campaign year while
    // still leaving most of an establishment phase untouched.
    let age_pressure = (age_years - SUCCESSION_ELIGIBILITY_AGE_YEARS)
        .saturating_mul(AGE_PRESSURE_PER_YEAR_OVER_ELIGIBILITY);
    let governance_pressure = i64::from(succession_risk_basis_points / 2);
    let health_pressure = i64::from(10_000_u16.saturating_sub(health_basis_points) / 2);
    u16::try_from(
        age_pressure
            .saturating_add(governance_pressure)
            .saturating_add(health_pressure)
            .clamp(0, 9_500),
    )
    .expect("clamped succession chance must fit u16")
}

pub(crate) fn retire_outgoing_head(state: &mut AppState, outgoing_head_id: CharacterId) {
    state
        .characters
        .get_mut(outgoing_head_id)
        .expect("succession outgoing head must exist")
        .runtime
        .status = CharacterStatus::Deceased;
    for link in state.family_links.values_mut().filter(|link| {
        link.active
            && matches!(link.kind, FamilyLinkKind::Marriage | FamilyLinkKind::Ward)
            && (link.first_character_id == outgoing_head_id
                || link.second_character_id == outgoing_head_id)
    }) {
        link.active = false;
    }
}

pub(crate) fn update_institutions_for_succession(
    state: &mut AppState,
    dynasty_id: DynastyId,
    outgoing_head_id: CharacterId,
    incoming_head_id: CharacterId,
    replacement_selection_day: Option<i64>,
) {
    // Non-player heads hold institutional seats by dynasty standing, so the
    // incoming head inherits them. Player dynasties earn membership through
    // patronage instead, so their seats are not transferred.
    let transfer_membership = dynasty_id != state.player_dynasty_id;
    for institution in state.institutions.values_mut() {
        institution.members.remove(&outgoing_head_id);
        if transfer_membership {
            institution.members.insert(incoming_head_id);
        }
        if institution.office_holder_id == Some(outgoing_head_id) {
            institution.office_holder_id = None;
            institution.next_selection_day = institution
                .next_selection_day
                .min(replacement_selection_day.expect("office replacement day was preflighted"));
        }
    }
}

pub(crate) fn insert_succession_heir(
    state: &mut AppState,
    dynasty_id: DynastyId,
    incoming_head_id: CharacterId,
    new_heir_name: String,
    new_heir_birth_day: i64,
    new_heir_link_kind: FamilyLinkKind,
    new_heir_capabilities: CharacterCapabilities,
) -> Result<CharacterId, SimulationError> {
    let mut next_ids = state.next_ids.clone();
    let new_heir_id = next_ids.try_character()?;
    let family_link_id = next_ids.try_family_link()?;
    state.next_ids = next_ids;
    state.characters.insert(Character {
        identity: CharacterIdentity {
            id: new_heir_id,
            dynasty_id,
            name: new_heir_name,
            birth_day: new_heir_birth_day,
        },
        capabilities: new_heir_capabilities,
        runtime: CharacterRuntime {
            status: CharacterStatus::Active,
            health_basis_points: 9_500,
            loyalty_basis_points: 8_000,
            role: CharacterRole::Heir,
            incapacitated_day: None,
        },
    });
    state.family_links.insert(
        family_link_id,
        FamilyLink {
            id: family_link_id,
            first_character_id: incoming_head_id,
            second_character_id: new_heir_id,
            kind: new_heir_link_kind,
            active: true,
        },
    );
    Ok(new_heir_id)
}
pub(crate) fn apply_family_succession_transition(
    state: &mut AppState,
    dynasty_id: DynastyId,
    outgoing_head_id: CharacterId,
    incoming_head_id: CharacterId,
    new_heir_id: CharacterId,
    shock: SuccessionShock,
    next_charter_version: u64,
) {
    let affected_family_members = {
        let council = state
            .family_councils
            .get_mut(&dynasty_id)
            .expect("succession dynasty must have a family council");
        council.members.remove(&outgoing_head_id);
        council.members.insert(incoming_head_id);
        council.members.insert(new_heir_id);
        council.unity_basis_points = council
            .unity_basis_points
            .saturating_sub(shock.family_unity_loss);
        council.charter_version = next_charter_version;
        council
            .members
            .iter()
            .copied()
            .filter(|character_id| {
                *character_id != incoming_head_id && *character_id != new_heir_id
            })
            .collect::<Vec<_>>()
    };
    for character_id in affected_family_members {
        if let Some(character) = state.characters.get_mut(character_id)
            && character.status() == CharacterStatus::Active
        {
            character.runtime.loyalty_basis_points = character
                .runtime
                .loyalty_basis_points
                .saturating_sub(shock.family_loyalty_loss);
        }
    }
}

pub(crate) fn record_succession_transition(
    state: &mut AppState,
    dynasty_id: DynastyId,
    outgoing_head_id: CharacterId,
    incoming_head_id: CharacterId,
    formally_prepared: bool,
    family_unity_loss: u16,
    legitimacy_loss: u16,
) -> Result<(), SimulationError> {
    let id = state.next_ids.try_chronicle()?;
    state.chronicle.push(ChronicleEntry {
        id,
        day: state.clock.day(),
        kind: ChronicleKind::Succession,
        summary: format!(
            "Dynasty {dynasty_id} passed from character {outgoing_head_id} to {incoming_head_id}; formal preparation was {formally_prepared}."
        ),
    });
    if dynasty_id == state.player_dynasty_id {
        super::super::strategic::try_push_outbox(
            state,
            OutboxKind::Family,
            "A new generation inherited the house".to_owned(),
            format!(
                "Character {incoming_head_id} succeeded character {outgoing_head_id}. Family unity fell by {family_unity_loss} bp and legitimacy by {legitimacy_loss} bp. Formal heir preparation was {formally_prepared}, so the severity of the transition reflects the dynasty's succession planning."
            ),
        )?;
    }
    Ok(())
}

pub(crate) fn apply_successions(
    state: &mut AppState,
    lines: Vec<SuccessionLine>,
) -> Result<(), SimulationError> {
    if lines.is_empty() {
        return Ok(());
    }
    let mut candidate = state.clone();
    apply_successions_in_place(&mut candidate, lines)?;
    *state = candidate;
    Ok(())
}

pub(crate) fn apply_successions_in_place(
    state: &mut AppState,
    lines: Vec<SuccessionLine>,
) -> Result<(), SimulationError> {
    for line in lines {
        let SuccessionLine {
            dynasty_id,
            outgoing_head_id,
            incoming_head_id,
            formally_prepared,
            family_unity_loss,
            family_loyalty_loss,
            legitimacy_loss,
            new_heir_name,
            new_heir_birth_day,
            new_heir_link_kind,
            next_generation,
            next_charter_version,
            new_heir_capabilities,
        } = line;
        let replacement_selection_day = state
            .institutions
            .values()
            .any(|institution| institution.office_holder_id == Some(outgoing_head_id))
            .then(|| checked_future_day(state.clock.day(), 30))
            .transpose()?;
        retire_outgoing_head(state, outgoing_head_id);
        {
            let incoming = state
                .characters
                .get_mut(incoming_head_id)
                .expect("succession incoming head must exist");
            incoming.runtime.role = CharacterRole::HeadOfHouse;
            incoming.runtime.loyalty_basis_points = 10_000;
            // Lift the heir health pin: see SUCCESSION_ACCESSION_HEALTH_FLOOR.
            incoming.runtime.health_basis_points = incoming
                .runtime
                .health_basis_points
                .max(SUCCESSION_ACCESSION_HEALTH_FLOOR);
        }

        update_institutions_for_succession(
            state,
            dynasty_id,
            outgoing_head_id,
            incoming_head_id,
            replacement_selection_day,
        );
        reassign_managed_businesses(state, dynasty_id, outgoing_head_id, incoming_head_id);
        let new_heir_id = insert_succession_heir(
            state,
            dynasty_id,
            incoming_head_id,
            new_heir_name,
            new_heir_birth_day,
            new_heir_link_kind,
            new_heir_capabilities,
        )?;
        apply_family_succession_transition(
            state,
            dynasty_id,
            outgoing_head_id,
            incoming_head_id,
            new_heir_id,
            SuccessionShock {
                formally_prepared,
                family_unity_loss,
                family_loyalty_loss,
                legitimacy_loss,
            },
            next_charter_version,
        );

        let dynasty = state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("succession dynasty must exist");
        dynasty.relationships.head_id = incoming_head_id;
        dynasty.relationships.heir_id = Some(new_heir_id);
        dynasty.runtime.generation = next_generation;
        dynasty.runtime.phase = crate::core::CampaignPhase::Legacy;
        let remaining = dynasty
            .resources
            .legitimacy_basis_points
            .saturating_sub(legitimacy_loss);
        // A dynasty that built political embedding should not emerge from
        // succession with single-digit legitimacy and no credible recovery
        // route — the harness stranded such houses for 720+ days at 15 bp.
        // Preserve a floor so the new head can afford the first patronage
        // step that rebuilds institutional standing.
        dynasty.resources.legitimacy_basis_points = if formally_prepared {
            remaining.max(2_000)
        } else {
            remaining.max(1_500)
        };
        record_succession_transition(
            state,
            dynasty_id,
            outgoing_head_id,
            incoming_head_id,
            formally_prepared,
            family_unity_loss,
            legitimacy_loss,
        )?;
    }
    Ok(())
}
