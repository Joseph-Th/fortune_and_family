//! Campaign-phase progression derived from durable milestones — never backward.
//!
//! Purpose: own the single monotonic `CampaignPhase` ladder (Foundation →
//! Establishment → Ascendancy → Dominion → Legacy) so `ARCHITECTURE.md`'s
//! promised phase growth and `GAMEPLAY_HARNESS.md`'s phase grouping both
//! derive from the same durable evidence. Phases are earned by commercial,
//! institutional, civic, and succession milestones and never regress even
//! when underlying standing later softens.
//! Owns: `refresh_campaign_phases` (with `CampaignEvidenceMemo` incremental
//! fold), `campaign_phase_is_consistent` / `are_consistent`, and
//! `CampaignPhaseEvidence::collect` (one sweep for all dynasties).
//! Reads: `AppState` (audit log, dynasties, institutions, laws, etc.).
//! Mutates: `dynasty.runtime.phase` and the caller-owned memo.
//! Does not own: mission definitions or narrative prose (labels live on
//! `CampaignPhase::label`).
//! Invariants: phases never regress; evidence is memo-consistency checked
//! (audit shrinking or day regression forces rebuild); phase derived only
//! from durable milestones, not volatile cache.
//! Focused tests: `src/systems/strategic/*` succession, harness phase diagnostics.

use super::{OFFICE_NOMINATION_DELIVERY_REQUIREMENT, OFFICE_NOMINATION_REPUTATION_REQUIREMENT};
use crate::core::{AppState, AuditKind, CampaignEvidenceMemo, CampaignPhase};
use crate::ids::DynastyId;
use std::collections::{BTreeMap, BTreeSet};

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

/// Durable milestone evidence gathered from the append-only history ledgers
/// and live delivery records.
///
/// Audited milestones fold incrementally into [`CampaignEvidenceMemo`] and
/// live maps (laws, public works, contracts) are rescanned per refresh.
/// The result is identical to a full rescan and keeps daily cost bounded as
/// the audit log grows.
#[derive(Default, Clone, PartialEq, Debug)]
pub(crate) struct CampaignPhaseEvidence {
    city_shaping: BTreeSet<DynastyId>,
    nominations: BTreeSet<DynastyId>,
    patronage: BTreeSet<DynastyId>,
    contract_deliveries: BTreeMap<DynastyId, u32>,
}

impl CampaignPhaseEvidence {
    /// Uncached collection: builds the evidence from scratch through a
    /// throwaway memo. Used by immutable validation paths that run too rarely
    /// to keep a memo warm; callers validating many dynasties at once should
    /// collect once and pass the evidence to
    /// [`campaign_phase_is_consistent_with`].
    pub(crate) fn collect(state: &AppState) -> Self {
        Self::synchronize(&mut CampaignEvidenceMemo::default(), state)
    }

    /// Brings `memo` up to date with the append-only audit history and
    /// materializes current evidence from it.
    ///
    /// Audit-derived answers are resolved against *current* state exactly as
    /// a full rescan would: directive institutions are checked for existence
    /// at materialization time, and nomination/patronage characters are kept
    /// only while their records still exist. The live law, public-work, and
    /// contract scans are unchanged.
    fn synchronize(memo: &mut CampaignEvidenceMemo, state: &AppState) -> Self {
        if !memo.is_consistent_with(&state.audit_log) {
            *memo = CampaignEvidenceMemo::default();
        }
        let mut new_directives = BTreeSet::new();
        for record in state.audit_log.iter().skip(memo.folded_len) {
            match record.kind() {
                AuditKind::OfficeDirective => {
                    let subject = record.audit_subject();
                    if let Some(institution_id) = subject.institution_id()
                        && let Some(dynasty_id) = subject.dynasty_id()
                    {
                        new_directives.insert((dynasty_id, institution_id));
                    }
                }
                AuditKind::OfficeNomination | AuditKind::InstitutionPatronage => {
                    if let Some((_, character_id)) =
                        record.audit_subject().institution_character_ids()
                    {
                        if record.kind() == AuditKind::OfficeNomination {
                            resolve_or_stage_character(
                                character_id,
                                state,
                                &mut memo.nomination_characters,
                                &mut memo.unresolved_nomination_characters,
                            );
                        } else {
                            resolve_or_stage_character(
                                character_id,
                                state,
                                &mut memo.patronage_characters,
                                &mut memo.unresolved_patronage_characters,
                            );
                        }
                    }
                }
                _ => {}
            }
            memo.folded_len += 1;
            memo.folded_last_day = record.day();
        }
        memo.office_directive_houses.append(&mut new_directives);
        // A staged character may have appeared since it was first referenced.
        retry_staged_characters(
            &mut memo.unresolved_nomination_characters,
            &mut memo.nomination_characters,
            state,
        );
        retry_staged_characters(
            &mut memo.unresolved_patronage_characters,
            &mut memo.patronage_characters,
            state,
        );
        // Evidence requires the named character to still exist, mirroring a
        // full rescan; dropped IDs are never reused, so removal is final.
        let characters = &state.characters;
        memo.nomination_characters
            .retain(|character_id, _| characters.get(*character_id).is_some());
        memo.patronage_characters
            .retain(|character_id, _| characters.get(*character_id).is_some());

        let mut evidence = Self::default();
        for law in state.laws.values() {
            if let Some(sponsor_dynasty_id) = law.sponsor_dynasty_id {
                evidence.city_shaping.insert(sponsor_dynasty_id);
            }
        }
        for work in state.public_works.values() {
            if let Some(sponsor_dynasty_id) = work.sponsor_dynasty_id {
                evidence.city_shaping.insert(sponsor_dynasty_id);
            }
        }
        for contract in state.contracts.values() {
            for (dynasty_id, fulfilled) in &contract.fulfilled_deliveries_by_dynasty {
                let total = evidence
                    .contract_deliveries
                    .entry(*dynasty_id)
                    .or_insert_with(|| 0_u32);
                *total = total.saturating_add(u32::from(*fulfilled));
            }
        }
        for (dynasty_id, institution_id) in &memo.office_directive_houses {
            if state.institutions.contains_key(institution_id) {
                evidence.city_shaping.insert(*dynasty_id);
            }
        }
        evidence.nominations = memo.nomination_characters.values().copied().collect();
        evidence.patronage = memo.patronage_characters.values().copied().collect();
        evidence
    }

    fn city_shaping_history(&self, dynasty_id: DynastyId) -> bool {
        self.city_shaping.contains(&dynasty_id)
    }

    fn nomination_history(&self, dynasty_id: DynastyId) -> bool {
        self.nominations.contains(&dynasty_id)
    }

    fn patronage_history(&self, dynasty_id: DynastyId) -> bool {
        self.nominations.contains(&dynasty_id) || self.patronage.contains(&dynasty_id)
    }

    fn contract_deliveries(&self, dynasty_id: DynastyId) -> u32 {
        self.contract_deliveries
            .get(&dynasty_id)
            .copied()
            .unwrap_or(0)
    }
}

/// Records a nomination/patronage character into `resolved` when it exists,
/// or stages it in `staged` for later retries when it does not, matching a
/// full rescan that would pick the record up once the character appears.
fn resolve_or_stage_character(
    character_id: crate::ids::CharacterId,
    state: &AppState,
    resolved: &mut BTreeMap<crate::ids::CharacterId, DynastyId>,
    staged: &mut BTreeSet<crate::ids::CharacterId>,
) {
    match state.characters.get(character_id) {
        Some(character) => {
            resolved.insert(character_id, character.dynasty_id());
        }
        None => {
            staged.insert(character_id);
        }
    }
}

/// Moves every staged character that now exists into the resolved bucket.
fn retry_staged_characters(
    staged: &mut BTreeSet<crate::ids::CharacterId>,
    resolved: &mut BTreeMap<crate::ids::CharacterId, DynastyId>,
    state: &AppState,
) {
    let mut newly_resolvable = Vec::new();
    for character_id in staged.iter() {
        if let Some(character) = state.characters.get(*character_id) {
            newly_resolvable.push((*character_id, character.dynasty_id()));
        }
    }
    for (character_id, dynasty_id) in newly_resolvable {
        staged.remove(&character_id);
        resolved.insert(character_id, dynasty_id);
    }
}

/// Reputation standing keys the `Establishment` phase on the dynasty's
/// *current* commercial reputation rather than an append-only record. This is
/// the one deliberate live-value trigger: the monotonic phase clamp turns any
/// crossing into a permanent milestone, so the phase can be gained here but
/// never lost when standing later softens.
fn dynasty_reputation_standing(state: &AppState, dynasty_id: DynastyId) -> bool {
    state.dynasties.get(&dynasty_id).is_some_and(|dynasty| {
        dynasty
            .resources
            .reputation_quality_basis_points
            .max(dynasty.resources.reputation_reliability_basis_points)
            >= OFFICE_NOMINATION_REPUTATION_REQUIREMENT
    })
}

/// Derives the phase from already-collected durable evidence. The dynasty's
/// live reputation standing is the one deliberate live-value input and is
/// still read from current state here.
fn reconstructed_campaign_phase_with(
    evidence: &CampaignPhaseEvidence,
    state: &AppState,
    dynasty_id: DynastyId,
) -> CampaignPhase {
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .expect("campaign phase dynasty must exist");
    if dynasty.runtime.generation > 1 {
        return CampaignPhase::Legacy;
    }
    if evidence.city_shaping_history(dynasty_id) {
        return CampaignPhase::Dominion;
    }
    let nomination_history = evidence.nomination_history(dynasty_id);
    let patronage_history = evidence.patronage_history(dynasty_id);
    let deliveries = evidence.contract_deliveries(dynasty_id);
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

fn campaign_phase_has_required_durable_evidence(
    evidence: &CampaignPhaseEvidence,
    state: &AppState,
    dynasty_id: DynastyId,
) -> bool {
    let Some(dynasty) = state.dynasties.get(&dynasty_id) else {
        return false;
    };
    if dynasty.runtime.generation > 1 {
        return dynasty.runtime.phase == CampaignPhase::Legacy;
    }
    match dynasty.runtime.phase {
        CampaignPhase::Foundation | CampaignPhase::Establishment => true,
        CampaignPhase::Ascendancy => {
            evidence.contract_deliveries(dynasty_id) >= OFFICE_NOMINATION_DELIVERY_REQUIREMENT
                || evidence.nomination_history(dynasty_id)
        }
        CampaignPhase::Dominion => evidence.city_shaping_history(dynasty_id),
        CampaignPhase::Legacy => false,
    }
}

/// Whether the persisted phase agrees with a fresh derivation from durable
/// evidence: the phase must carry its required milestone evidence and never
/// sit below the reconstructed rank (promotion is monotonic, so evidence that
/// later softens does not invalidate an already-earned phase).
pub(crate) fn campaign_phase_is_consistent(state: &AppState, dynasty_id: DynastyId) -> bool {
    let evidence = CampaignPhaseEvidence::collect(state);
    campaign_phase_is_consistent_with(&evidence, state, dynasty_id)
}

/// Whole-campaign consistency check that collects the audit-derived evidence
/// exactly once for every dynasty. Validation callers (the daily invariant
/// battery and the persistence boundary) run this per dynasty; a naive loop
/// over the single-dynasty form would rescan the unbounded audit log once per
/// house.
pub(crate) fn campaign_phases_are_consistent(state: &AppState) -> bool {
    let evidence = CampaignPhaseEvidence::collect(state);
    state
        .dynasties
        .keys()
        .all(|dynasty_id| campaign_phase_is_consistent_with(&evidence, state, *dynasty_id))
}

/// Single-dynasty consistency check against pre-collected evidence. Callers
/// validating many dynasties collect the evidence once via
/// [`CampaignPhaseEvidence::collect`] and share it here.
pub(crate) fn campaign_phase_is_consistent_with(
    evidence: &CampaignPhaseEvidence,
    state: &AppState,
    dynasty_id: DynastyId,
) -> bool {
    let Some(dynasty) = state.dynasties.get(&dynasty_id) else {
        return false;
    };
    if !campaign_phase_has_required_durable_evidence(evidence, state, dynasty_id) {
        return false;
    }
    if dynasty.runtime.generation > 1 {
        return true;
    }
    campaign_phase_rank(dynasty.runtime.phase)
        >= campaign_phase_rank(reconstructed_campaign_phase_with(
            evidence, state, dynasty_id,
        ))
}

fn runtime_campaign_phase(
    evidence: &CampaignPhaseEvidence,
    state: &AppState,
    dynasty_id: DynastyId,
) -> CampaignPhase {
    let dynasty = state
        .dynasties
        .get(&dynasty_id)
        .expect("campaign phase dynasty must exist");
    if dynasty.runtime.generation > 1 {
        return CampaignPhase::Legacy;
    }
    // The live phase is the rank-maximum of the persisted phase and a fresh
    // reconstruction of the durable evidence. Because the derivation IS the
    // reconstruction (never regressed), the two can never disagree: a house
    // promoted on evidence keeps its phase when that evidence later softens,
    // and new evidence promotes it exactly as a save reload would.
    let current = dynasty.runtime.phase;
    if current == CampaignPhase::Legacy {
        // Legacy is the maximum rank, so no evidence comparison can promote
        // it further.
        return current;
    }
    let reconstructed = reconstructed_campaign_phase_with(evidence, state, dynasty_id);
    if campaign_phase_rank(reconstructed) > campaign_phase_rank(current) {
        reconstructed
    } else {
        current
    }
}

pub(crate) fn refresh_campaign_phases(state: &mut AppState) {
    // Evidence collection folds the append-only audit history incrementally
    // (see `CampaignEvidenceMemo`), so it is only worth paying for when some
    // dynasty could actually change phase. `runtime_campaign_phase` is an
    // identity whenever the persisted phase already is the terminal `Legacy`
    // rank — either because the house reached succession (`generation > 1`,
    // reconstructed as `Legacy`) or because evidence promoted it there
    // directly — so a campaign whose houses all rest at `Legacy` can skip
    // collection outright without changing a single derived value.
    let all_terminal = state
        .dynasties
        .values()
        .all(|dynasty| dynasty.runtime.phase == CampaignPhase::Legacy);
    if all_terminal {
        return;
    }
    // The memo rides on the state so transactional working copies each carry
    // their own warm fold; taking it out avoids overlapping borrows while
    // collection reads the rest of the state.
    let mut memo = std::mem::take(&mut state.campaign_evidence_memo);
    let evidence = CampaignPhaseEvidence::synchronize(&mut memo, state);
    state.campaign_evidence_memo = memo;
    let updates = state
        .dynasties
        .keys()
        .copied()
        .map(|dynasty_id| {
            (
                dynasty_id,
                runtime_campaign_phase(&evidence, state, dynasty_id),
            )
        })
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
    fn patronage_history_promotes_the_runtime_phase_like_reconstruction() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        state.audit_log.push(crate::core::AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::InstitutionPatronage,
            subject: format!("institution:{institution_id}:character:{}", {
                state
                    .dynasties
                    .get(&player_id)
                    .expect("player dynasty must exist")
                    .head_id()
            })
            .into(),
            detail: "durable patronage history".into(),
        });

        refresh_campaign_phases(&mut state);

        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .phase(),
            CampaignPhase::Establishment,
            "durable patronage must promote the live phase exactly like save reconstruction does"
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

        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .runtime
            .phase = CampaignPhase::Dominion;

        assert!(!campaign_phase_is_consistent(&state, player_id));

        state.audit_log.push(crate::core::AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeDirective,
            subject: "invalid-office-directive".into(),
            detail: "fabricated directive history".into(),
        });

        assert!(!campaign_phase_is_consistent(&state, player_id));
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
            detail: format!("power={power:?}").into(),
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
            detail: "actor-attributed directive".into(),
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

    /// Synchronizes a throwaway clone of the state's memo without mutating
    /// `state`.
    fn synchronized_evidence(state: &AppState) -> CampaignPhaseEvidence {
        let mut memo = state.campaign_evidence_memo.clone();
        let evidence = CampaignPhaseEvidence::synchronize(&mut memo, state);
        assert_eq!(memo.folded_len, state.audit_log.len());
        evidence
    }

    /// The core memo contract: incremental collection must always agree with
    /// a from-scratch collection over the same state.
    fn assert_memoized_evidence_matches_full_rescan(state: &AppState) {
        assert_eq!(
            synchronized_evidence(state),
            CampaignPhaseEvidence::collect(state),
            "incremental evidence must match a full rescan"
        );
    }

    #[test]
    fn repeated_appends_and_refreshes_keep_incremental_evidence_exact() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        let head_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();

        for round in 0..4_u32 {
            // Interleave every audited kind between refreshes so each fold
            // increment covers a mix of record kinds on top of an existing
            // warm memo.
            state.audit_log.push(crate::core::AuditRecord {
                day: state.clock.day(),
                kind: AuditKind::OfficeDirective,
                subject: format!("institution:{institution_id};dynasty:{player_id}").into(),
                detail: format!("directive round {round}").into(),
            });
            state.audit_log.push(crate::core::AuditRecord {
                day: state.clock.day(),
                kind: AuditKind::OfficeNomination,
                subject: format!("institution:{institution_id}:character:{head_id}").into(),
                detail: format!("nomination round {round}").into(),
            });
            state.audit_log.push(crate::core::AuditRecord {
                day: state.clock.day(),
                kind: AuditKind::InstitutionPatronage,
                subject: format!("institution:{institution_id}:character:{head_id}").into(),
                detail: format!("patronage round {round}").into(),
            });
            // Unrelated kinds must be skipped without disturbing the fold.
            state.audit_log.push(crate::core::AuditRecord {
                day: state.clock.day(),
                kind: AuditKind::DayAdvanced,
                subject: "simulation".into(),
                detail: format!("round {round}").into(),
            });

            assert_memoized_evidence_matches_full_rescan(&state);
            refresh_campaign_phases(&mut state);
            assert_memoized_evidence_matches_full_rescan(&state);
        }
    }

    #[test]
    fn branch_clones_fold_their_own_appends_independently() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        state.audit_log.push(crate::core::AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::InstitutionPatronage,
            subject: format!(
                "institution:{institution_id}:character:{}",
                state
                    .dynasties
                    .get(&player_id)
                    .expect("player dynasty must exist")
                    .head_id()
            )
            .into(),
            detail: "shared history".into(),
        });
        refresh_campaign_phases(&mut state);

        // A working-copy clone carries its own warm memo and appends its own
        // records, exactly like a transactional simulation branch.
        let mut branch = state.clone();
        let rival_id = branch
            .dynasties
            .keys()
            .copied()
            .find(|id| *id != player_id)
            .expect("campaign must contain a rival dynasty");
        branch.audit_log.push(crate::core::AuditRecord {
            day: branch.clock.day(),
            kind: AuditKind::OfficeDirective,
            subject: format!("institution:{institution_id};dynasty:{rival_id}").into(),
            detail: "branch-only directive".into(),
        });

        assert_memoized_evidence_matches_full_rescan(&branch);
        assert_memoized_evidence_matches_full_rescan(&state);
        refresh_campaign_phases(&mut branch);
        refresh_campaign_phases(&mut state);
        assert_memoized_evidence_matches_full_rescan(&branch);
        assert_memoized_evidence_matches_full_rescan(&state);
        assert_eq!(
            branch
                .dynasties
                .get(&rival_id)
                .expect("rival dynasty must exist")
                .phase(),
            CampaignPhase::Dominion,
            "the branch's own directive must promote inside the branch only"
        );
        assert_eq!(
            state
                .dynasties
                .get(&rival_id)
                .expect("rival dynasty must exist")
                .phase(),
            CampaignPhase::Foundation,
            "branch appends must not leak into the parent state"
        );
    }

    #[test]
    fn truncated_history_forces_a_full_memo_rebuild() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        state.audit_log.push(crate::core::AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::InstitutionPatronage,
            subject: format!(
                "institution:{institution_id}:character:{}",
                state
                    .dynasties
                    .get(&player_id)
                    .expect("player dynasty must exist")
                    .head_id()
            )
            .into(),
            detail: "warmup history".into(),
        });
        refresh_campaign_phases(&mut state);
        assert_memoized_evidence_matches_full_rescan(&state);

        // A cleared history invalidates the fold watermark; the rebuilt memo
        // must answer from what actually remains.
        state.audit_log.clear();
        assert_memoized_evidence_matches_full_rescan(&state);
        refresh_campaign_phases(&mut state);
        assert_memoized_evidence_matches_full_rescan(&state);
        // The rebuilt evidence does not contain the patronage record, but
        // the runtime phase is deliberately monotonic: an already-earned rank
        // survives evidence that is absent after clearing.
        let mut memo = std::mem::take(&mut state.campaign_evidence_memo);
        let rebuilt = CampaignPhaseEvidence::synchronize(&mut memo, &state);
        state.campaign_evidence_memo = memo;
        assert!(
            !rebuilt.patronage_history(player_id),
            "cleared patronage history must stop backing the promoted phase"
        );
    }

    #[test]
    fn serialized_state_omits_the_evidence_memo_entirely() {
        let mut state = make_test_campaign();
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        state.audit_log.push(crate::core::AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeDirective,
            subject: format!(
                "institution:{institution_id};dynasty:{}",
                state.player_dynasty_id
            )
            .into(),
            detail: "memo warming directive".into(),
        });
        refresh_campaign_phases(&mut state);
        assert!(
            !state
                .campaign_evidence_memo
                .office_directive_houses
                .is_empty(),
            "precondition: the refresh must have folded records into the memo"
        );

        // The memo is a non-persisted derivation: its field names never
        // appear in the save shape, and the serialized document round-trips
        // into an equal state because equality excludes the memo too.
        let serialized = serde_json::to_string(&state).expect("state must serialize");
        assert!(!serialized.contains("folded_len"));
        assert!(!serialized.contains("office_directive_houses"));
        let reloaded: AppState =
            serde_json::from_str(&serialized).expect("the memo-free shape must deserialize");
        assert_eq!(reloaded, state);
    }
}
