//! Canonical campaign progression derived from durable dynasty milestones.

use super::{OFFICE_NOMINATION_DELIVERY_REQUIREMENT, OFFICE_NOMINATION_REPUTATION_REQUIREMENT};
use crate::core::{AppState, AuditKind, CampaignPhase};
use crate::ids::DynastyId;

const fn campaign_phase_rank(phase: CampaignPhase) -> u8 {
    match phase {
        CampaignPhase::Foundation => 0,
        CampaignPhase::Establishment => 1,
        CampaignPhase::Ascendancy => 2,
        CampaignPhase::Dominion => 3,
        CampaignPhase::Legacy => 4,
    }
}

pub(crate) fn contract_deliveries_for_dynasty(state: &AppState, dynasty_id: DynastyId) -> u32 {
    state.contracts.values().fold(0_u32, |total, contract| {
        total.saturating_add(u32::from(
            contract
                .fulfilled_deliveries_by_dynasty
                .get(&dynasty_id)
                .copied()
                .unwrap_or(0),
        ))
    })
}

fn dynasty_reputation_standing(state: &AppState, dynasty_id: DynastyId) -> bool {
    state.dynasties.get(&dynasty_id).is_some_and(|dynasty| {
        dynasty
            .resources
            .reputation_quality_basis_points
            .max(dynasty.resources.reputation_reliability_basis_points)
            >= OFFICE_NOMINATION_REPUTATION_REQUIREMENT
    })
}

fn audit_kind_references_dynasty_character(
    state: &AppState,
    dynasty_id: DynastyId,
    kind: AuditKind,
) -> bool {
    state.audit_log.iter().any(|record| {
        record.kind() == kind
            && record
                .audit_subject()
                .institution_character_ids()
                .and_then(|(_, character_id)| state.characters.get(character_id))
                .is_some_and(|character| character.dynasty_id() == dynasty_id)
    })
}

fn dynasty_city_shaping_history(state: &AppState, dynasty_id: DynastyId) -> bool {
    state
        .laws
        .values()
        .any(|law| law.sponsor_dynasty_id == Some(dynasty_id))
        || state
            .public_works
            .values()
            .any(|work| work.sponsor_dynasty_id == Some(dynasty_id))
        || state.audit_log.iter().any(|record| {
            if record.kind() != AuditKind::OfficeDirective {
                return false;
            }
            let subject = record.audit_subject();
            subject
                .institution_id()
                .is_some_and(|institution_id| state.institutions.contains_key(&institution_id))
                && subject.dynasty_id() == Some(dynasty_id)
        })
}

fn reconstructed_campaign_phase(state: &AppState, dynasty_id: DynastyId) -> CampaignPhase {
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .expect("campaign phase dynasty must exist");
    if dynasty.runtime.generation > 1 {
        return CampaignPhase::Legacy;
    }
    if dynasty_city_shaping_history(state, dynasty_id) {
        return CampaignPhase::Dominion;
    }
    let nomination_history =
        audit_kind_references_dynasty_character(state, dynasty_id, AuditKind::OfficeNomination);
    let patronage_history = nomination_history
        || audit_kind_references_dynasty_character(
            state,
            dynasty_id,
            AuditKind::InstitutionPatronage,
        );
    let deliveries = contract_deliveries_for_dynasty(state, dynasty_id);
    if nomination_history
        || (deliveries >= OFFICE_NOMINATION_DELIVERY_REQUIREMENT
            && (patronage_history || dynasty_reputation_standing(state, dynasty_id)))
    {
        CampaignPhase::Ascendancy
    } else if patronage_history || dynasty_reputation_standing(state, dynasty_id) {
        CampaignPhase::Establishment
    } else {
        CampaignPhase::Foundation
    }
}

fn campaign_phase_has_required_durable_evidence(state: &AppState, dynasty_id: DynastyId) -> bool {
    let Some(dynasty) = state.dynasties.get(&dynasty_id) else {
        return false;
    };
    if dynasty.runtime.generation > 1 {
        return dynasty.runtime.phase == CampaignPhase::Legacy;
    }
    match dynasty.runtime.phase {
        CampaignPhase::Foundation | CampaignPhase::Establishment => true,
        CampaignPhase::Ascendancy => {
            contract_deliveries_for_dynasty(state, dynasty_id)
                >= OFFICE_NOMINATION_DELIVERY_REQUIREMENT
                || audit_kind_references_dynasty_character(
                    state,
                    dynasty_id,
                    AuditKind::OfficeNomination,
                )
        }
        CampaignPhase::Dominion => dynasty_city_shaping_history(state, dynasty_id),
        CampaignPhase::Legacy => false,
    }
}

pub(crate) fn campaign_phase_is_consistent(state: &AppState, dynasty_id: DynastyId) -> bool {
    let Some(dynasty) = state.dynasties.get(&dynasty_id) else {
        return false;
    };
    if !campaign_phase_has_required_durable_evidence(state, dynasty_id) {
        return false;
    }
    if dynasty.runtime.generation > 1 {
        return true;
    }
    dynasty.runtime.phase == runtime_campaign_phase(state, dynasty_id)
}

pub(crate) fn campaign_phase_is_persistently_consistent(
    state: &AppState,
    dynasty_id: DynastyId,
) -> bool {
    let Some(dynasty) = state.dynasties.get(&dynasty_id) else {
        return false;
    };
    if !campaign_phase_has_required_durable_evidence(state, dynasty_id) {
        return false;
    }
    if dynasty.runtime.generation > 1 {
        return true;
    }
    campaign_phase_rank(dynasty.runtime.phase)
        >= campaign_phase_rank(reconstructed_campaign_phase(state, dynasty_id))
}

fn runtime_campaign_phase(state: &AppState, dynasty_id: DynastyId) -> CampaignPhase {
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .expect("campaign phase dynasty must exist");
    if dynasty.runtime.generation > 1 {
        return CampaignPhase::Legacy;
    }
    let current = dynasty.runtime.phase;
    if current == CampaignPhase::Dominion {
        return current;
    }
    if dynasty_city_shaping_history(state, dynasty_id) {
        return CampaignPhase::Dominion;
    }
    if current == CampaignPhase::Ascendancy {
        return current;
    }
    let deliveries = contract_deliveries_for_dynasty(state, dynasty_id);
    if current == CampaignPhase::Establishment {
        return if deliveries >= OFFICE_NOMINATION_DELIVERY_REQUIREMENT {
            CampaignPhase::Ascendancy
        } else {
            current
        };
    }
    if dynasty_reputation_standing(state, dynasty_id) {
        if deliveries >= OFFICE_NOMINATION_DELIVERY_REQUIREMENT {
            CampaignPhase::Ascendancy
        } else {
            CampaignPhase::Establishment
        }
    } else {
        CampaignPhase::Foundation
    }
}

pub(crate) fn refresh_campaign_phases(state: &mut AppState) {
    let updates = state
        .dynasties
        .keys()
        .copied()
        .map(|dynasty_id| (dynasty_id, runtime_campaign_phase(state, dynasty_id)))
        .collect::<Vec<_>>();
    for (dynasty_id, phase) in updates {
        state
            .dynasties
            .get_mut(&dynasty_id)
            .expect("campaign phase dynasty must exist")
            .runtime
            .phase = phase;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::make_test_campaign;

    #[test]
    fn campaign_phase_follows_progression_milestones_instead_of_elapsed_years() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .phase(),
            CampaignPhase::Foundation
        );

        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .reputation_reliability_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        refresh_campaign_phases(&mut state);
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .phase(),
            CampaignPhase::Establishment
        );

        let contract = state
            .contracts
            .values_mut()
            .next()
            .expect("campaign must contain a supply contract");
        let deliveries = u16::try_from(OFFICE_NOMINATION_DELIVERY_REQUIREMENT)
            .expect("office nomination delivery requirement must fit u16");
        contract.fulfilled_deliveries = deliveries;
        contract
            .fulfilled_deliveries_by_dynasty
            .insert(player_id, deliveries);
        refresh_campaign_phases(&mut state);
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .phase(),
            CampaignPhase::Ascendancy
        );

        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            crate::core::EnactedLaw {
                id: law_id,
                kind: crate::core::LawKind::BreadPriceCeiling,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: Some(player_id),
                value: 1,
                active: true,
            },
        );
        refresh_campaign_phases(&mut state);
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .phase(),
            CampaignPhase::Dominion
        );

        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .runtime
            .generation = 2;
        refresh_campaign_phases(&mut state);
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .phase(),
            CampaignPhase::Legacy
        );
    }

    #[test]
    fn advanced_phases_require_their_durable_milestone_evidence() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .runtime
            .phase = CampaignPhase::Ascendancy;

        assert!(!campaign_phase_is_consistent(&state, player_id));
        assert!(!campaign_phase_is_persistently_consistent(
            &state, player_id
        ));

        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .runtime
            .phase = CampaignPhase::Dominion;

        assert!(!campaign_phase_is_consistent(&state, player_id));
        assert!(!campaign_phase_is_persistently_consistent(
            &state, player_id
        ));

        state.audit_log.push(crate::core::AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeDirective,
            subject: "invalid-office-directive".into(),
            detail: "fabricated directive history".to_owned(),
        });

        assert!(!campaign_phase_is_consistent(&state, player_id));
        assert!(!campaign_phase_is_persistently_consistent(
            &state, player_id
        ));
    }

    #[test]
    fn establishment_remains_valid_after_reputation_standing_later_falls() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .reputation_reliability_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        refresh_campaign_phases(&mut state);
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .phase(),
            CampaignPhase::Establishment
        );

        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .reputation_reliability_basis_points = 0;

        assert!(campaign_phase_is_consistent(&state, player_id));
        assert!(campaign_phase_is_persistently_consistent(&state, player_id));
    }

    #[test]
    fn untagged_office_directive_does_not_create_current_schema_progression() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .runtime
            .phase = CampaignPhase::Ascendancy;
        let institution = state
            .institutions
            .values_mut()
            .find(|institution| !institution.powers.is_empty())
            .expect("campaign must contain an institution with office powers");
        let power = *institution
            .powers
            .iter()
            .next()
            .expect("institution must expose an office power");
        let institution_id = institution.institution_id;
        institution.active_directive = Some(crate::core::OfficeDirectiveState {
            power,
            expires_day: state.clock.day(),
        });
        state.audit_log.push(crate::core::AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeDirective,
            subject: format!("institution:{institution_id}").into(),
            detail: format!("power={power:?}"),
        });

        refresh_campaign_phases(&mut state);

        assert_ne!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .phase(),
            CampaignPhase::Dominion
        );
    }

    #[test]
    fn tagged_office_directive_progression_is_attributed_to_its_actor_dynasty() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let rival_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != player_id)
            .expect("campaign must contain a rival dynasty");
        let institution_id = state
            .institutions
            .keys()
            .copied()
            .next()
            .expect("campaign must contain an institution");
        state.audit_log.push(crate::core::AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeDirective,
            subject: format!("institution:{institution_id};dynasty:{rival_id}").into(),
            detail: "actor-attributed directive".to_owned(),
        });

        refresh_campaign_phases(&mut state);

        assert_ne!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .phase(),
            CampaignPhase::Dominion
        );
        assert_eq!(
            state
                .dynasties
                .get(&rival_id)
                .expect("rival dynasty must exist")
                .phase(),
            CampaignPhase::Dominion
        );
    }
}
