//! Gameplay candidate generation, probing, and selection — the acting layer.
//!
//! Purpose: build state-derived `PlayerCommand` candidates, probe them
//! through `apply_player_command_scratch` on clones, rank them by persona
//! priorities + urgency + reserves, and diagnose quiet cycles.
//! Owns: `probe_candidates`, per-command-kind generators, persona ranking,
//! `GameplayCommandKind` coverage, and quiet-cause taxonomy.
//! Reads: `Registry` + `AppState` via authoritative projections and
//! production validators (never a second legality oracle).
//! Mutates: nothing durable (operates on disposable clones); probes are
//! bounded by `max_candidate_probes`.
//! Does not own: report schema/finding rules (findings/scoring own).
//! Invariants: every candidate that reaches selection has passed the
//! canonical validator; `ActivationPredicateDrift` fails the cycle if a
//! probe proves the predicate wrong; determinism via stable ordering.
//! Focused tests: `src/gameplay_tests.rs` candidate reachability.

#[allow(clippy::wildcard_imports)] // the module tree re-exports one flat namespace
use super::*;

#[cfg(test)]
pub(crate) fn probe_candidates(
    registry: &Registry,
    state: &AppState,
    candidates: impl Iterator<Item = Candidate>,
    projection_days: u32,
    max_consequence_horizon_days: u16,
    accumulator: &mut CampaignAccumulator,
) -> Result<ProbeResult, GameplayHarnessError> {
    probe_candidates_with_parallelism(
        registry,
        state,
        candidates,
        projection_days,
        max_consequence_horizon_days,
        false,
        accumulator,
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "candidate probing keeps validation, counterfactual projection, and selection accounting in one auditable path"
)]
pub(crate) fn probe_candidates_with_parallelism(
    registry: &Registry,
    state: &AppState,
    candidates: impl Iterator<Item = Candidate>,
    projection_days: u32,
    max_consequence_horizon_days: u16,
    parallel_counterfactuals: bool,
    accumulator: &mut CampaignAccumulator,
) -> Result<ProbeResult, GameplayHarnessError> {
    let baseline = GameplaySnapshot::capture(state);
    let candidates: Vec<_> = candidates.collect();
    // Project three decision intervals for counterfactual comparison; keep one shared horizon
    // so profiles remain comparable across command families.
    let shared_projection_days = projection_days
        .saturating_mul(3)
        .min(u32::from(max_consequence_horizon_days))
        .max(projection_days);
    let mut projected_baseline_state = state.clone();
    advance_days_scratch(
        registry,
        &mut projected_baseline_state,
        shared_projection_days,
    )?;
    let projected_baseline = GameplaySnapshot::capture(&projected_baseline_state);
    let outcomes = probe_candidate_outcomes(
        registry,
        state,
        &baseline,
        &projected_baseline,
        candidates,
        shared_projection_days,
        parallel_counterfactuals,
    )?;
    let mut selected_substantive = None;
    let mut selected_operational = None;
    let mut housekeeping_fallback = None;
    let mut viable_count = 0_usize;
    let mut substantive_viable_count = 0_usize;
    let mut viable_command_kinds = BTreeSet::new();
    let mut viable_options = Vec::new();
    let mut immediate_profiles = BTreeSet::new();
    let mut projected_profiles = BTreeSet::new();
    let mut immediate_family_profiles = BTreeSet::new();
    let mut projected_family_profiles = BTreeSet::new();
    let mut option_scores = Vec::new();
    let mut family_scores = Vec::new();
    let mut rejections = Vec::new();
    for outcome in outcomes {
        let candidate = outcome.candidate();
        let command_stats = accumulator
            .commands
            .get_mut(&candidate.kind)
            .expect("every command kind must have statistics");
        command_stats.considered = command_stats.considered.saturating_add(1);
        match outcome {
            CandidateProbeOutcome::Viable {
                candidate,
                evaluated,
            } => {
                command_stats.viable = command_stats.viable.saturating_add(1);
                viable_count = viable_count.saturating_add(1);
                if is_substantive_command_kind(candidate.kind) {
                    substantive_viable_count = substantive_viable_count.saturating_add(1);
                    let immediate_choice_profile = evaluated.immediate_profile_key.clone();
                    let projected_choice_profile = evaluated.projected_profile_key.clone();
                    immediate_profiles.insert(immediate_choice_profile.clone());
                    projected_profiles.insert(projected_choice_profile.clone());
                    option_scores.push(candidate.score);
                    if viable_command_kinds.insert(candidate.kind) {
                        immediate_family_profiles.insert(immediate_choice_profile);
                        projected_family_profiles.insert(projected_choice_profile);
                        family_scores.push(candidate.score);
                    }
                    viable_options.push(evaluated.option);
                    // Select the highest-ranked viable action; probe order is kind-diversity-first.
                    // Ties keep the earlier candidate for determinism.
                    if selected_substantive
                        .as_ref()
                        .is_none_or(|current: &Candidate| candidate.score > current.score)
                    {
                        selected_substantive = Some(candidate);
                    }
                } else if matches!(
                    candidate.kind,
                    GameplayCommandKind::TransferBusinessCash
                        | GameplayCommandKind::WithdrawBusinessCash
                ) {
                    if selected_operational
                        .as_ref()
                        .is_none_or(|current: &Candidate| candidate.score > current.score)
                    {
                        selected_operational = Some(candidate);
                    }
                } else if housekeeping_fallback.is_none() {
                    housekeeping_fallback = Some(candidate);
                }
            }
            CandidateProbeOutcome::Rejected { category, .. } => {
                command_stats.rejected = command_stats.rejected.saturating_add(1);
                *accumulator
                    .rejection_reasons
                    .entry(category.clone())
                    .or_default() += 1;
                if rejections.len() < 4 {
                    rejections.push(category);
                }
            }
        }
    }
    option_scores.sort_unstable_by(|left, right| right.cmp(left));
    family_scores.sort_unstable_by(|left, right| right.cmp(left));
    let close_choice_score_gap = score_gap(&option_scores);
    let family_close_choice_score_gap = score_gap(&family_scores);
    Ok(ProbeResult {
        selected: selected_substantive
            .or(selected_operational)
            .or(housekeeping_fallback),
        viable_count,
        substantive_viable_count,
        viable_command_kinds,
        viable_options,
        close_choice_score_gap,
        distinct_immediate_choice_profiles: immediate_profiles.len(),
        distinct_projected_choice_profiles: projected_profiles.len(),
        family_close_choice_score_gap,
        distinct_immediate_family_profiles: immediate_family_profiles.len(),
        distinct_projected_family_profiles: projected_family_profiles.len(),
        rejections,
    })
}

pub(crate) enum CandidateProbeOutcome {
    Viable {
        candidate: Candidate,
        evaluated: Box<EvaluatedViableOption>,
    },
    Rejected {
        candidate: Candidate,
        category: String,
    },
}

impl CandidateProbeOutcome {
    pub fn candidate(&self) -> &Candidate {
        match self {
            Self::Viable { candidate, .. } | Self::Rejected { candidate, .. } => candidate,
        }
    }
}

pub(crate) fn probe_candidate(
    registry: &Registry,
    state: &AppState,
    baseline: &GameplaySnapshot,
    projected_baseline: &GameplaySnapshot,
    candidate: Candidate,
    projection_days: u32,
) -> Result<CandidateProbeOutcome, GameplayHarnessError> {
    let mut probe = state.clone();
    // Probe via the scratch entry to avoid a second deep copy of the campaign.
    match apply_player_command_scratch(registry, &mut probe, candidate.command.clone()) {
        Ok(_) => Ok(CandidateProbeOutcome::Viable {
            evaluated: Box::new(evaluate_viable_option(
                registry,
                baseline,
                projected_baseline,
                &probe,
                &candidate,
                projection_days,
            )?),
            candidate,
        }),
        Err(error) => Ok(CandidateProbeOutcome::Rejected {
            candidate,
            category: command_error_category(&error).to_owned(),
        }),
    }
}

pub(crate) fn probe_candidate_outcomes(
    registry: &Registry,
    state: &AppState,
    baseline: &GameplaySnapshot,
    projected_baseline: &GameplaySnapshot,
    candidates: Vec<Candidate>,
    projection_days: u32,
    parallel_counterfactuals: bool,
) -> Result<Vec<CandidateProbeOutcome>, GameplayHarnessError> {
    if !parallel_counterfactuals || candidates.len() <= 1 {
        return candidates
            .into_iter()
            .map(|candidate| {
                probe_candidate(
                    registry,
                    state,
                    baseline,
                    projected_baseline,
                    candidate,
                    projection_days,
                )
            })
            .collect();
    }

    // Cap nested workers to bound clone-memory residency; each campaign clone shares history
    // text but remains comparatively large.
    let worker_count = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(8)
        .min(candidates.len());
    let chunk_size = candidates.len().div_ceil(worker_count);
    let chunks: Vec<&[Candidate]> = candidates.chunks(chunk_size).collect();
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .cloned()
                        .map(|candidate| {
                            probe_candidate(
                                registry,
                                state,
                                baseline,
                                projected_baseline,
                                candidate,
                                projection_days,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| GameplayHarnessError::CounterfactualWorkerPanicked)?
            })
            .collect::<Result<Vec<_>, GameplayHarnessError>>()
    })?;

    Ok(results.into_iter().flatten().collect())
}

pub(crate) type ConsequenceProfileKey = (
    BTreeSet<GameplayDomain>,
    bool,
    BTreeSet<GameplayMeasure>,
    BTreeSet<GameplayMeasure>,
    u64,
);

pub(crate) struct EvaluatedViableOption {
    pub option: GameplayViableOption,
    pub immediate_profile_key: ConsequenceProfileKey,
    pub projected_profile_key: ConsequenceProfileKey,
}

pub(crate) fn evaluate_viable_option(
    registry: &Registry,
    baseline: &GameplaySnapshot,
    projected_baseline: &GameplaySnapshot,
    immediate_state: &AppState,
    candidate: &Candidate,
    projection_days: u32,
) -> Result<EvaluatedViableOption, GameplayHarnessError> {
    let immediate_snapshot = GameplaySnapshot::capture(immediate_state);
    let immediate_domains = baseline.changed_domains(&immediate_snapshot);
    let immediate_history_change =
        baseline.audit_state_checksum != immediate_snapshot.audit_state_checksum;
    let immediate_profile = GameplayConsequenceProfile::between(baseline, &immediate_snapshot);
    let mut projected_state = immediate_state.clone();
    advance_days_scratch(registry, &mut projected_state, projection_days)?;
    let projected_snapshot = GameplaySnapshot::capture(&projected_state);
    let projected_domains = projected_baseline.changed_domains(&projected_snapshot);
    let projected_history_change =
        projected_baseline.audit_state_checksum != projected_snapshot.audit_state_checksum;
    let projected_profile =
        GameplayConsequenceProfile::between(projected_baseline, &projected_snapshot);
    Ok(EvaluatedViableOption {
        immediate_profile_key: consequence_profile_key(
            &immediate_domains,
            immediate_history_change,
            &immediate_profile,
        ),
        projected_profile_key: consequence_profile_key(
            &projected_domains,
            projected_history_change,
            &projected_profile,
        ),
        option: GameplayViableOption {
            command: candidate.kind,
            score: candidate.score,
            description: candidate.description.clone(),
            projected_horizon_days: u16::try_from(projection_days).unwrap_or(u16::MAX),
            immediate_domains,
            projected_domains,
            immediate_history_change,
            projected_history_change,
            immediate_profile,
            projected_profile,
        },
    })
}

pub(crate) fn consequence_profile_key(
    domains: &BTreeSet<GameplayDomain>,
    history_change: bool,
    profile: &GameplayConsequenceProfile,
) -> ConsequenceProfileKey {
    (
        domains.clone(),
        history_change,
        profile.increases.clone(),
        profile.decreases.clone(),
        profile.impact_fingerprint,
    )
}

pub(crate) fn score_gap(scores_descending: &[i64]) -> Option<i64> {
    scores_descending
        .first()
        .zip(scores_descending.get(1))
        .map(|(first, second)| first.saturating_sub(*second))
}

pub(crate) struct CycleObservation<'a> {
    pub before: &'a GameplaySnapshot,
    pub after_command: &'a GameplaySnapshot,
    pub after_time: &'a GameplaySnapshot,
    pub baseline_after_time: &'a GameplaySnapshot,
    pub considered: usize,
    pub viable: usize,
    pub substantive_viable: usize,
    pub viable_command_kinds: BTreeSet<GameplayCommandKind>,
    pub ranked_candidates: Vec<GameplayCandidateRanking>,
    pub phase: GameplayPhase,
    pub viable_options: Vec<GameplayViableOption>,
    pub close_choice_score_gap: Option<i64>,
    pub distinct_immediate_choice_profiles: usize,
    pub distinct_projected_choice_profiles: usize,
    pub rejections: Vec<String>,
    pub action: Option<ExecutedAction>,
    pub no_action_reason: Option<String>,
    pub command_feedback: Vec<GameplayFeedbackEvent>,
    pub simulation_window_days: u32,
    pub simulation_feedback: Vec<GameplayFeedbackEvent>,
    pub ambient_window_days: u32,
    pub ambient_feedback: Vec<GameplayFeedbackEvent>,
}

// Consequence attribution and trace assembly are kept together so every
// reported domain and feedback event comes from the same pair of branches.
#[expect(
    clippy::too_many_lines,
    reason = "the dispatch keeps the full decision path in one auditable function"
)]
pub(crate) fn record_cycle(
    observation: CycleObservation<'_>,
    accumulator: &mut CampaignAccumulator,
) {
    let CycleObservation {
        before,
        after_command,
        after_time,
        baseline_after_time,
        considered,
        viable,
        substantive_viable,
        viable_command_kinds,
        ranked_candidates,
        phase,
        viable_options,
        close_choice_score_gap,
        distinct_immediate_choice_profiles,
        distinct_projected_choice_profiles,
        rejections,
        action,
        no_action_reason,
        command_feedback,
        simulation_window_days,
        simulation_feedback,
        ambient_window_days,
        ambient_feedback,
    } = observation;
    let immediate_domains = before.changed_domains(after_command);
    let total_causal_domains = baseline_after_time.changed_domains(after_time);
    let persistent_domains: BTreeSet<_> = immediate_domains
        .intersection(&total_causal_domains)
        .copied()
        .collect();
    let persistent_history_change =
        persistent_history_changed(before, after_command, after_time, baseline_after_time);
    let delayed_domains: BTreeSet<_> = total_causal_domains
        .difference(&immediate_domains)
        .copied()
        .collect();
    let ambient_domains = before.changed_domains(baseline_after_time);
    let signals = cycle_trace_signals(
        before,
        after_command,
        after_time,
        baseline_after_time,
        persistent_history_change,
    );
    let immediate_consequences = GameplayConsequenceProfile::between(before, after_command);
    let attributed_consequences =
        GameplayConsequenceProfile::between(baseline_after_time, after_time);
    let ambient_consequences = GameplayConsequenceProfile::between(before, baseline_after_time);
    let observed_domains: BTreeSet<_> = immediate_domains
        .union(&delayed_domains)
        .copied()
        .chain(ambient_domains.iter().copied())
        .collect();
    record_cycle_domain_changes(
        &observed_domains,
        &immediate_domains,
        &delayed_domains,
        &ambient_domains,
        accumulator,
    );
    if let Some(action) = &action {
        record_action_consequences(
            action.kind,
            ActionConsequenceObservation {
                immediate: &immediate_domains,
                persistent: &persistent_domains,
                delayed: &delayed_domains,
                signals: &signals,
                productive_business_change: before.business_state_checksum
                    != after_command.business_state_checksum,
                financing_workout: after_command.player_restructured_lending
                    > before.player_restructured_lending
                    && after_command.player_defaulted_lending < before.player_defaulted_lending
                    && after_command.total_loan_balance == before.total_loan_balance,
            },
            accumulator,
        );
    }
    accumulator.trace.push(GameplayTraceStep {
        day: before.day,
        phase,
        context: GameplayDecisionContext::from(before),
        considered_candidates: usize_to_u16(considered),
        viable_candidates: usize_to_u16(viable),
        substantive_viable_candidates: usize_to_u16(substantive_viable),
        viable_command_kinds,
        ranked_candidates,
        viable_options,
        close_choice_score_gap,
        distinct_immediate_choice_profiles: usize_to_u16(distinct_immediate_choice_profiles),
        distinct_projected_choice_profiles: usize_to_u16(distinct_projected_choice_profiles),
        selected_command: action.as_ref().map(|action| action.kind),
        command_description: action.as_ref().map(|action| action.description.clone()),
        outcome: action.map(|action| action.outcome),
        rejection_summary: rejections,
        no_action_reason,
        immediate_consequences,
        attributed_consequences,
        ambient_consequences,
        command_feedback,
        simulation_window_days,
        simulation_feedback,
        ambient_window_days,
        ambient_feedback,
        immediate_domains,
        delayed_domains,
        persistent_domains,
        ambient_domains,
        signals,
    });
}

pub(crate) fn cycle_trace_signals(
    before: &GameplaySnapshot,
    after_command: &GameplaySnapshot,
    after_time: &GameplaySnapshot,
    baseline_after_time: &GameplaySnapshot,
    persistent_history_change: bool,
) -> BTreeSet<GameplayTraceSignal> {
    let immediate_feedback = !before.changed_domains(after_command).is_empty()
        || after_command.outbox_messages > before.outbox_messages
        || after_command.chronicle_entries > before.chronicle_entries;
    let delayed_feedback = after_time
        .outbox_messages
        .saturating_sub(after_command.outbox_messages)
        != baseline_after_time
            .outbox_messages
            .saturating_sub(before.outbox_messages)
        || after_time
            .chronicle_entries
            .saturating_sub(after_command.chronicle_entries)
            != baseline_after_time
                .chronicle_entries
                .saturating_sub(before.chronicle_entries);
    let ambient_feedback = baseline_after_time.outbox_messages > before.outbox_messages
        || baseline_after_time.chronicle_entries > before.chronicle_entries;
    [
        immediate_feedback.then_some(GameplayTraceSignal::ImmediateWorldFeedback),
        delayed_feedback.then_some(GameplayTraceSignal::DelayedWorldFeedback),
        ambient_feedback.then_some(GameplayTraceSignal::AmbientWorldFeedback),
        persistent_history_change.then_some(GameplayTraceSignal::PersistentHistoryChange),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub(crate) fn record_cycle_domain_changes(
    observed: &BTreeSet<GameplayDomain>,
    immediate: &BTreeSet<GameplayDomain>,
    delayed: &BTreeSet<GameplayDomain>,
    ambient: &BTreeSet<GameplayDomain>,
    accumulator: &mut CampaignAccumulator,
) {
    for domain in observed {
        *accumulator.domain_changes.entry(*domain).or_default() += 1;
    }
    for domain in immediate.union(delayed) {
        *accumulator
            .causal_domain_changes
            .entry(*domain)
            .or_default() += 1;
    }
    for domain in ambient {
        *accumulator
            .ambient_domain_changes
            .entry(*domain)
            .or_default() += 1;
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ActionConsequenceObservation<'a> {
    pub immediate: &'a BTreeSet<GameplayDomain>,
    pub persistent: &'a BTreeSet<GameplayDomain>,
    pub delayed: &'a BTreeSet<GameplayDomain>,
    pub signals: &'a BTreeSet<GameplayTraceSignal>,
    pub productive_business_change: bool,
    pub financing_workout: bool,
}

pub(crate) fn record_action_consequences(
    kind: GameplayCommandKind,
    observation: ActionConsequenceObservation<'_>,
    accumulator: &mut CampaignAccumulator,
) {
    let ActionConsequenceObservation {
        immediate,
        persistent,
        delayed,
        signals,
        productive_business_change,
        financing_workout,
    } = observation;
    let immediate_feedback = signals.contains(&GameplayTraceSignal::ImmediateWorldFeedback);
    let delayed_feedback = signals.contains(&GameplayTraceSignal::DelayedWorldFeedback);
    let command_stats = accumulator
        .commands
        .get_mut(&kind)
        .expect("every command kind must have statistics");
    if immediate_feedback {
        command_stats.immediate_world_feedback =
            command_stats.immediate_world_feedback.saturating_add(1);
    }
    if delayed_feedback {
        command_stats.delayed_world_feedback =
            command_stats.delayed_world_feedback.saturating_add(1);
    }
    if immediate_feedback || delayed_feedback {
        command_stats.actions_with_feedback = command_stats.actions_with_feedback.saturating_add(1);
    }
    // Persistence means an attributable world-state change survived to the
    // horizon. Every command appends audit records, so an append-only
    // history checksum cannot distinguish durable consequences from mere
    // bookkeeping and must not feed this metric.
    if !persistent.is_empty() {
        command_stats.actions_with_persistent_consequences = command_stats
            .actions_with_persistent_consequences
            .saturating_add(1);
    }
    if !delayed.is_empty() {
        command_stats.actions_with_delayed_consequences = command_stats
            .actions_with_delayed_consequences
            .saturating_add(1);
    }
    if kind == GameplayCommandKind::ExtendCredit {
        if financing_workout {
            command_stats.financing_workout_actions =
                command_stats.financing_workout_actions.saturating_add(1);
        } else if productive_business_change {
            command_stats.productive_financing_actions =
                command_stats.productive_financing_actions.saturating_add(1);
        } else {
            command_stats.nonproductive_financing_actions = command_stats
                .nonproductive_financing_actions
                .saturating_add(1);
        }
    }
    for domain in immediate.union(delayed) {
        command_stats.changed_domains.insert(*domain);
        *accumulator.interactions.entry((kind, *domain)).or_default() += 1;
    }
}

pub(crate) fn ranked_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    accumulator: &CampaignAccumulator,
) -> (Vec<Candidate>, BTreeSet<GameplayCommandKind>) {
    let mut candidates = Vec::new();
    generate_reactive_candidates(state, persona, &mut candidates);
    generate_business_candidates(registry, state, persona, &mut candidates);
    generate_contract_candidates(registry, state, persona, &mut candidates);
    generate_finance_candidates(registry, state, persona, &mut candidates);
    generate_information_candidates(registry, state, persona, &mut candidates);
    generate_civic_candidates(registry, state, persona, &mut candidates);
    generate_family_candidates(registry, state, persona, &mut candidates);
    // Organic exploration: policy-gated generators deliberately narrow
    // the canonical offer to strategic-need conditions. Without a small
    // deterministic exploration chance, every persona replays the same
    // narrow path per world state and the harness becomes rigid. When an
    // activation exists but no candidate was built, inject an exploratory
    // candidate ~12% of the time so organic gameplay variance is actually
    // measured without overriding reserves or urgency.
    inject_exploratory_candidates(registry, state, persona, accumulator, &mut candidates);
    let generated_kinds: BTreeSet<_> = candidates.iter().map(|candidate| candidate.kind).collect();
    candidates
        .retain(|candidate| candidate_preserves_office_duty_reserve(registry, state, candidate));
    for candidate in &mut candidates {
        candidate.score = candidate
            .score
            .saturating_add(rank_adjustment(candidate.kind, state, persona, accumulator))
            .saturating_add(legal_funding_candidate_adjustment(state, candidate))
            .saturating_add(organic_candidate_variation(
                state,
                persona,
                accumulator,
                candidate,
            ));
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.description.cmp(&right.description))
    });
    (candidates, generated_kinds)
}

/// Adds bounded, state-derived exploration noise to otherwise deterministic policy scores.
///
/// The harness must sample nearby legal choices instead of replaying one rigid policy path in
/// every campaign. The variation is derived from a copy of the campaign RNG, so it is fully
/// reproducible and cannot consume or perturb the simulation's random stream. Its small range
/// only changes close calls; urgency, safety reserves, and persona priorities remain dominant.
pub(crate) fn organic_candidate_variation(
    state: &AppState,
    persona: GameplayPersona,
    accumulator: &CampaignAccumulator,
    candidate: &Candidate,
) -> i64 {
    let mut variation_rng = state.rng;
    let mut value = variation_rng.next_u64();
    value = value.wrapping_add(state.clock.day().cast_unsigned());
    value = value.wrapping_add(u64::from(accumulator.decision_cycles));
    value = value.wrapping_add(u64::from(accumulator.total_viable_choices));
    value = value.wrapping_add(u64::from(accumulator.quiet_cycles));
    // Mix campaign-distinct state so nearby worlds/personas do not replay
    // identical close-call rankings. Uses only live AppState/accumulator
    // signals already sampled during the decision cycle to keep the variation
    // reproducible without consuming the game RNG.
    value ^= accumulator
        .peak_player_treasury
        .copper()
        .cast_unsigned()
        .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    value = value.wrapping_add(
        u64::from(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .map_or(0, |d| d.runtime.generation),
        )
        .wrapping_mul(0x94D0_49BB_1331_11EB),
    );
    value = value.wrapping_add(
        u64::try_from(state.businesses.iter().count())
            .unwrap_or(u64::MAX)
            .wrapping_mul(0xDA94_2042_E4DD_58B5),
    );
    value ^= u64::try_from(
        state
            .crises
            .values()
            .filter(|c| c.status.is_active())
            .count(),
    )
    .unwrap_or(u64::MAX)
    .wrapping_mul(0xA409_3822_299F_31D0);
    value ^= u64::from(accumulator.total_viable_command_kinds).wrapping_mul(0xBE54_66CF_34E9_0C6C);
    value ^= u64::try_from(
        state
            .properties
            .values()
            .filter(|p| p.owner_dynasty_id == Some(state.player_dynasty_id))
            .count(),
    )
    .unwrap_or(u64::MAX)
    .wrapping_mul(0x94D0_49BB_1331_11EB ^ 0x1234_5678);
    value ^= u64::try_from(
        state
            .legal_cases
            .values()
            .filter(|c| {
                matches!(
                    c.status,
                    crate::core::LegalCaseStatus::Filed | crate::core::LegalCaseStatus::Hearing
                )
            })
            .count(),
    )
    .unwrap_or(u64::MAX)
    .wrapping_mul(0xBF58_476D_1CE4_E5B9 ^ 0x9ABC_DEF0);
    for byte in persona
        .label()
        .bytes()
        .chain(candidate.kind.label().bytes())
        .chain(candidate.description.bytes())
    {
        value ^= u64::from(byte);
        value = value.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17);
    }
    let span = ORGANIC_CANDIDATE_VARIATION_RANGE
        .saturating_mul(2)
        .saturating_add(1);
    i64::try_from(value % u64::try_from(span).expect("variation span must fit u64"))
        .expect("variation sample must fit i64")
        - ORGANIC_CANDIDATE_VARIATION_RANGE
}

pub(crate) fn legal_funding_candidate_adjustment(state: &AppState, candidate: &Candidate) -> i64 {
    let Some(requirement) = active_legal_settlement_requirement(state) else {
        return 0;
    };
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    if treasury >= requirement {
        return 0;
    }
    match &candidate.command {
        PlayerCommand::WithdrawBusinessCash { .. } => 5_000,
        PlayerCommand::IssueLoan { terms }
            if terms.borrower_dynasty_id == state.player_dynasty_id =>
        {
            5_000
        }
        PlayerCommand::SellProperty { .. } => 4_500,
        PlayerCommand::TransferBusinessCash { .. } => -4_000,
        PlayerCommand::AcquireBusiness { .. }
        | PlayerCommand::InvestInBusiness { .. }
        | PlayerCommand::SetBusinessPolicy { .. }
        | PlayerCommand::SetBusinessWages { .. }
        | PlayerCommand::CreateSupplyContract { .. }
        | PlayerCommand::IssueLoan { .. }
        | PlayerCommand::BuyProperty { .. }
        | PlayerCommand::EnactLaw { .. }
        | PlayerCommand::StartPublicWork { .. }
        | PlayerCommand::FundPublicWork { .. }
        | PlayerCommand::FileLegalCase { .. }
        | PlayerCommand::SettleLegalCase { .. }
        | PlayerCommand::SetHouseGovernance { .. }
        | PlayerCommand::ConveneFamilyCouncil
        | PlayerCommand::DesignateHeir { .. }
        | PlayerCommand::AdoptWard { .. }
        | PlayerCommand::EducateFamilyMember { .. }
        | PlayerCommand::CultivateInstitutionSupport { .. }
        | PlayerCommand::EndowInstitution { .. }
        | PlayerCommand::NominateForOffice { .. }
        | PlayerCommand::ExerciseOfficePower { .. }
        | PlayerCommand::WithdrawFromInstitution { .. }
        | PlayerCommand::RespondToCrisis { .. }
        | PlayerCommand::ResolveLaborDispute { .. }
        | PlayerCommand::CommissionInformation { .. }
        | PlayerCommand::LeverageInformation { .. }
        | PlayerCommand::AcknowledgeNotification { .. } => 0,
    }
}

pub(crate) fn candidate_preserves_office_duty_reserve(
    registry: &Registry,
    state: &AppState,
    candidate: &Candidate,
) -> bool {
    if candidate_is_emergency_spending(state, candidate)
        || matches!(candidate.command, PlayerCommand::SettleLegalCase { .. })
    {
        return true;
    }
    let nomination_institution_id = match &candidate.command {
        PlayerCommand::NominateForOffice { institution_id, .. } => Some(*institution_id),
        PlayerCommand::TransferBusinessCash { .. }
        | PlayerCommand::WithdrawBusinessCash { .. }
        | PlayerCommand::AcquireBusiness { .. }
        | PlayerCommand::InvestInBusiness { .. }
        | PlayerCommand::SetBusinessPolicy { .. }
        | PlayerCommand::SetBusinessWages { .. }
        | PlayerCommand::CreateSupplyContract { .. }
        | PlayerCommand::IssueLoan { .. }
        | PlayerCommand::BuyProperty { .. }
        | PlayerCommand::SellProperty { .. }
        | PlayerCommand::EnactLaw { .. }
        | PlayerCommand::StartPublicWork { .. }
        | PlayerCommand::FundPublicWork { .. }
        | PlayerCommand::FileLegalCase { .. }
        | PlayerCommand::SettleLegalCase { .. }
        | PlayerCommand::SetHouseGovernance { .. }
        | PlayerCommand::ConveneFamilyCouncil
        | PlayerCommand::DesignateHeir { .. }
        | PlayerCommand::AdoptWard { .. }
        | PlayerCommand::EducateFamilyMember { .. }
        | PlayerCommand::CultivateInstitutionSupport { .. }
        | PlayerCommand::EndowInstitution { .. }
        | PlayerCommand::ExerciseOfficePower { .. }
        | PlayerCommand::WithdrawFromInstitution { .. }
        | PlayerCommand::RespondToCrisis { .. }
        | PlayerCommand::ResolveLaborDispute { .. }
        | PlayerCommand::CommissionInformation { .. }
        | PlayerCommand::LeverageInformation { .. }
        | PlayerCommand::AcknowledgeNotification { .. } => None,
    };
    let reserve = if matches!(candidate.command, PlayerCommand::ConveneFamilyCouncil)
        && state
            .family_councils
            .get(&state.player_dynasty_id)
            .is_some_and(|council| {
                council.unity_basis_points < FAMILY_COUNCIL_INTERVENTION_UNITY_THRESHOLD
            }) {
        player_family_recovery_office_duty_reserve(state)
    } else {
        nomination_institution_id.map_or_else(
            || player_office_duty_reserve(state, 0),
            |institution_id| player_office_duty_reserve_for_nomination(state, institution_id),
        )
    };
    let reserve = if nomination_institution_id.is_some() && player_has_office_duty_forfeiture(state)
    {
        reserve.saturating_mul(3)
    } else {
        reserve
    };
    let reserve = reserve.max(active_legal_settlement_requirement(state).unwrap_or(Money::ZERO));
    if reserve == Money::ZERO {
        return true;
    }
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let cost = candidate_player_treasury_cost(registry, state, candidate);
    cost == Money::ZERO || treasury.saturating_sub(cost) >= reserve
}

pub(crate) fn active_legal_settlement_requirement(state: &AppState) -> Option<Money> {
    state
        .legal_cases
        .values()
        .filter_map(|legal_case| {
            quote_player_legal_settlement(state, legal_case.id)
                .ok()
                .map(|quote| (legal_case.hearing_day, quote.case_id, quote.amount))
        })
        .min_by_key(|(hearing_day, case_id, _)| (*hearing_day, *case_id))
        .map(|(_, _, amount)| amount)
}

pub(crate) fn legal_settlement_funding_target(state: &AppState) -> Option<Money> {
    let settlement = active_legal_settlement_requirement(state)?;
    let next_month_office_duty =
        projected_dynasty_monthly_office_duty(state, state.player_dynasty_id, 0);
    let existing_monthly_loan_service = state
        .loans
        .values()
        .filter(|loan| {
            loan.borrower_dynasty_id == state.player_dynasty_id && loan.status.is_repayment_active()
        })
        .fold(Money::ZERO, |total, loan| {
            total.saturating_add(loan.weekly_payment.saturating_mul(4))
        });
    Some(
        settlement
            .saturating_add(next_month_office_duty)
            .saturating_add(existing_monthly_loan_service)
            .saturating_add(Money::from_copper(500)),
    )
}

pub(crate) fn player_has_office_duty_forfeiture(state: &AppState) -> bool {
    state.audit_log.iter().any(|record| {
        record.kind() == AuditKind::OfficeDutyForfeiture
            && audit_subject_has_dynasty(record.audit_subject(), state.player_dynasty_id)
    })
}

pub(crate) fn audit_subject_has_dynasty(subject: &AuditSubject, dynasty_id: DynastyId) -> bool {
    subject.references_dynasty(dynasty_id)
}

pub(crate) fn candidate_is_emergency_spending(state: &AppState, candidate: &Candidate) -> bool {
    match &candidate.command {
        PlayerCommand::RespondToCrisis { crisis_id, .. } => {
            state.crises.get(crisis_id).is_some_and(|crisis| {
                crisis.status == CrisisStatus::Escalated || crisis.severity_basis_points >= 8_000
            })
        }
        PlayerCommand::InvestInBusiness { business_id, .. } => {
            state.businesses.get(*business_id).is_some_and(|business| {
                matches!(
                    business.status(),
                    BusinessStatus::Distressed | BusinessStatus::Insolvent
                ) || business.operations.condition_basis_points < 2_000
            })
        }
        PlayerCommand::TransferBusinessCash { .. }
        | PlayerCommand::WithdrawBusinessCash { .. }
        | PlayerCommand::AcquireBusiness { .. }
        | PlayerCommand::SetBusinessPolicy { .. }
        | PlayerCommand::SetBusinessWages { .. }
        | PlayerCommand::CreateSupplyContract { .. }
        | PlayerCommand::IssueLoan { .. }
        | PlayerCommand::BuyProperty { .. }
        | PlayerCommand::SellProperty { .. }
        | PlayerCommand::EnactLaw { .. }
        | PlayerCommand::StartPublicWork { .. }
        | PlayerCommand::FundPublicWork { .. }
        | PlayerCommand::FileLegalCase { .. }
        | PlayerCommand::SettleLegalCase { .. }
        | PlayerCommand::SetHouseGovernance { .. }
        | PlayerCommand::ConveneFamilyCouncil
        | PlayerCommand::DesignateHeir { .. }
        | PlayerCommand::AdoptWard { .. }
        | PlayerCommand::EducateFamilyMember { .. }
        | PlayerCommand::CultivateInstitutionSupport { .. }
        | PlayerCommand::EndowInstitution { .. }
        | PlayerCommand::NominateForOffice { .. }
        | PlayerCommand::ExerciseOfficePower { .. }
        | PlayerCommand::WithdrawFromInstitution { .. }
        | PlayerCommand::ResolveLaborDispute { .. }
        | PlayerCommand::CommissionInformation { .. }
        | PlayerCommand::LeverageInformation { .. }
        | PlayerCommand::AcknowledgeNotification { .. } => false,
    }
}

/// Treasury a solvent house keeps before spending on optional standing —
/// education, wards, endowments, patronage. The floor is an emergency
/// reserve plus two months of committed loan service; a house that spends
/// past it converts every surprise into new borrowing, which is how the
/// credit treadmill starts.
pub(crate) fn dynasty_discretionary_floor(state: &AppState) -> Money {
    let two_month_loan_service = state
        .loans
        .values()
        .filter(|loan| {
            loan.borrower_dynasty_id == state.player_dynasty_id && loan.status.is_repayment_active()
        })
        .fold(Money::ZERO, |total, loan| {
            total.saturating_add(loan.weekly_payment.saturating_mul(8))
        });
    Money::from_copper(2_000).saturating_add(two_month_loan_service)
}

pub(crate) fn player_office_duty_reserve(state: &AppState, additional_powers: usize) -> Money {
    let mut additional_offices: Vec<_> = pending_player_nomination_power_counts(state)
        .into_values()
        .collect();
    if additional_powers > 0 {
        additional_offices.push(additional_powers);
    }
    let monthly_duty = projected_dynasty_monthly_office_duty_with_additional_offices(
        state,
        state.player_dynasty_id,
        &additional_offices,
    );
    if monthly_duty == Money::ZERO {
        return Money::ZERO;
    }
    monthly_duty
        .saturating_mul(AGENT_OFFICE_DUTY_RESERVE_MONTHS)
        .saturating_add(AGENT_OFFICE_LIQUIDITY_BUFFER)
}

pub(crate) fn player_office_duty_reserve_for_nomination(
    state: &AppState,
    institution_id: InstitutionId,
) -> Money {
    let mut pending = pending_player_nomination_power_counts(state);
    if let Some(institution) = state.institutions.get(&institution_id) {
        pending
            .entry(institution_id)
            .or_insert(institution.powers.len());
    }
    let additional_offices: Vec<_> = pending.into_values().collect();
    let monthly_duty = projected_dynasty_monthly_office_duty_with_additional_offices(
        state,
        state.player_dynasty_id,
        &additional_offices,
    );
    if monthly_duty == Money::ZERO {
        return Money::ZERO;
    }
    monthly_duty
        .saturating_mul(AGENT_OFFICE_DUTY_RESERVE_MONTHS)
        .saturating_add(AGENT_OFFICE_LIQUIDITY_BUFFER)
}

pub(crate) fn pending_player_nomination_power_counts(
    state: &AppState,
) -> BTreeMap<InstitutionId, usize> {
    let day = state.clock.day();
    // Nominations stop counting once `day >= record.day() + resolution`, and
    // audit days never decrease, so everything before that boundary can be
    // skipped instead of filtered.
    let window_start = day.saturating_sub(OFFICE_NOMINATION_RESOLUTION_DAYS - 1);
    let window_start_index = state
        .audit_log
        .partition_point(|record| record.day() < window_start);
    state
        .audit_log
        .iter()
        .skip(window_start_index)
        .filter(|record| record.kind() == AuditKind::OfficeNomination)
        .filter_map(|record| {
            let (institution_id, character_id) =
                record.audit_subject().institution_character_ids()?;
            let character = state.characters.get(character_id)?;
            if character.dynasty_id() != state.player_dynasty_id {
                return None;
            }
            state
                .institutions
                .get(&institution_id)
                .map(|institution| (institution_id, institution.powers.len()))
        })
        .collect()
}

pub(crate) fn player_family_recovery_office_duty_reserve(state: &AppState) -> Money {
    let monthly_duty = projected_dynasty_monthly_office_duty(state, state.player_dynasty_id, 0);
    if monthly_duty == Money::ZERO {
        return Money::ZERO;
    }
    monthly_duty
        .saturating_mul(AGENT_FAMILY_COUNCIL_DUTY_RESERVE_MONTHS)
        .saturating_add(AGENT_FAMILY_COUNCIL_LIQUIDITY_BUFFER)
}

pub(crate) fn candidate_player_treasury_cost(
    registry: &Registry,
    state: &AppState,
    candidate: &Candidate,
) -> Money {
    match &candidate.command {
        PlayerCommand::AcquireBusiness {
            business_id,
            recapitalization,
            ..
        } => quote_business_acquisition(registry, state, state.player_dynasty_id, *business_id)
            .map_or(Money::ZERO, |quote| {
                quote.purchase_price.saturating_add(*recapitalization)
            }),
        PlayerCommand::InvestInBusiness { amount, .. }
        | PlayerCommand::FundPublicWork { amount, .. }
        | PlayerCommand::EndowInstitution { amount, .. } => *amount,
        PlayerCommand::IssueLoan { terms }
            if terms.lender_dynasty_id == state.player_dynasty_id =>
        {
            terms.principal
        }
        PlayerCommand::BuyProperty { property_id } => state
            .properties
            .get(property_id)
            .map_or(Money::ZERO, |property| property.value),
        PlayerCommand::EnactLaw { .. } => LAW_SPONSORSHIP_COST,
        PlayerCommand::StartPublicWork { budget, .. } => public_work_initial_contribution(*budget),
        PlayerCommand::FileLegalCase { .. } => LEGAL_CASE_FILING_COST,
        PlayerCommand::SettleLegalCase { case_id } => {
            quote_player_legal_settlement(state, *case_id).map_or(Money::ZERO, |quote| quote.amount)
        }
        PlayerCommand::ConveneFamilyCouncil => FAMILY_COUNCIL_MEETING_COST,
        PlayerCommand::AdoptWard { .. } => WARD_ADOPTION_COST,
        PlayerCommand::EducateFamilyMember { .. } => FAMILY_EDUCATION_COST,
        PlayerCommand::CultivateInstitutionSupport { .. } => {
            // Mirror the canonical entry surcharge so ranking and reserve
            // math see the same price validation will charge.
            let restriction =
                crate::systems::active_law_value(state, LawKind::GuildEntryRestriction)
                    .unwrap_or(0)
                    .clamp(0, 10_000);
            INSTITUTION_SUPPORT_COST.saturating_mul_ratio(10_000 + restriction / 2, 10_000)
        }
        PlayerCommand::NominateForOffice { .. } => OFFICE_NOMINATION_CAMPAIGN_COST,
        PlayerCommand::CommissionInformation { .. } => INFORMATION_COMMISSION_COST,
        PlayerCommand::LeverageInformation { .. } => INFORMATION_LEVERAGE_COST,
        PlayerCommand::RespondToCrisis {
            crisis_id,
            response,
        } => match response {
            CrisisResponse::Relief => state.crises.get(crisis_id).map_or(Money::ZERO, |crisis| {
                crisis_relief_cost(crisis.severity_basis_points)
            }),
            CrisisResponse::Reform => CRISIS_REFORM_COST,
            CrisisResponse::Suppress => CRISIS_SUPPRESS_COST,
            CrisisResponse::Exploit => Money::ZERO,
        },
        PlayerCommand::TransferBusinessCash { .. }
        | PlayerCommand::WithdrawBusinessCash { .. }
        | PlayerCommand::SetBusinessPolicy { .. }
        | PlayerCommand::SetBusinessWages { .. }
        | PlayerCommand::CreateSupplyContract { .. }
        | PlayerCommand::IssueLoan { .. }
        | PlayerCommand::SellProperty { .. }
        | PlayerCommand::WithdrawFromInstitution { .. }
        | PlayerCommand::ExerciseOfficePower { .. }
        | PlayerCommand::SetHouseGovernance { .. }
        | PlayerCommand::DesignateHeir { .. }
        | PlayerCommand::ResolveLaborDispute { .. }
        | PlayerCommand::AcknowledgeNotification { .. } => Money::ZERO,
    }
}

pub(crate) fn generate_reactive_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    generate_legal_settlement_candidates(state, persona, candidates);
    for crisis in state.crises.values().filter(|crisis| {
        crisis.status.is_active() && !crisis_has_containment_response(state, crisis.id)
    }) {
        let was_exploited = crisis_was_exploited(state, crisis.id);
        for response in crisis_responses(persona) {
            if response == CrisisResponse::Exploit && was_exploited {
                continue;
            }
            if !can_afford_crisis_response(state, crisis, response) {
                continue;
            }
            push_candidate(
                candidates,
                GameplayCommandKind::RespondToCrisis,
                PlayerCommand::RespondToCrisis {
                    crisis_id: crisis.id,
                    response,
                },
                format!(
                    "respond {response:?} to the {:?} crisis (crisis {})",
                    crisis.kind, crisis.id
                ),
                crisis_response_bonus_for_state(state, persona, response),
            );
        }
    }
    for agreement in state.employment.values().filter(|agreement| {
        agreement.status == EmploymentStatus::Disputed
            && state
                .businesses
                .get(agreement.business_id)
                .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id)
    }) {
        if let Some(response) = preferred_labor_response(state, agreement, persona) {
            push_candidate(
                candidates,
                GameplayCommandKind::ResolveLaborDispute,
                PlayerCommand::ResolveLaborDispute {
                    employment_id: agreement.id,
                    response,
                },
                format!("resolve labor dispute {} with {response:?}", agreement.id),
                labor_response_bonus(persona, response),
            );
        }
    }
    let unread_notifications = state
        .outbox
        .iter()
        .filter(|message| !message.acknowledged)
        .count();
    if unread_notifications >= NOTIFICATION_BATCH_THRESHOLD
        && let Some(message) = state
            .outbox
            .iter()
            .rev()
            .find(|message| !message.acknowledged)
    {
        push_candidate(
            candidates,
            GameplayCommandKind::AcknowledgeNotification,
            PlayerCommand::AcknowledgeNotification {
                message_id: message.id,
            },
            format!(
                "acknowledge {unread_notifications} notifications through notification {}",
                message.id
            ),
            0,
        );
    }
}

pub(crate) fn has_legal_settlement_opportunity(state: &AppState) -> bool {
    let Some(player_treasury) = state
        .dynasties
        .get(&state.player_dynasty_id)
        .map(crate::core::Dynasty::treasury)
    else {
        return false;
    };
    state.legal_cases.values().any(|legal_case| {
        quote_player_legal_settlement(state, legal_case.id)
            .is_ok_and(|quote| player_treasury >= quote.amount)
    })
}

pub(crate) fn generate_legal_settlement_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player_treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    for legal_case in state.legal_cases.values() {
        let Ok(quote) = quote_player_legal_settlement(state, legal_case.id) else {
            continue;
        };
        if player_treasury < quote.amount {
            continue;
        }
        let days_to_hearing = legal_case
            .hearing_day
            .saturating_sub(state.clock.day())
            .max(0);
        let urgency = 60_i64.saturating_sub(days_to_hearing).saturating_mul(20);
        let persona_bonus: i64 = match persona {
            GameplayPersona::Steward => 700,
            GameplayPersona::Entrepreneur => 480,
            GameplayPersona::PowerBroker => 760,
            GameplayPersona::Opportunist => 260,
        };
        push_candidate(
            candidates,
            GameplayCommandKind::SettleLegalCase,
            PlayerCommand::SettleLegalCase {
                case_id: quote.case_id,
            },
            format!(
                "settle {:?} case {} for {} before judgment",
                quote.kind, quote.case_id, quote.amount
            ),
            persona_bonus.saturating_add(urgency),
        );
    }
}

pub(crate) fn crisis_has_containment_response(
    state: &AppState,
    crisis_id: crate::ids::CrisisId,
) -> bool {
    let subject = format!("crisis:{crisis_id}");
    // Mirrors the canonical validator: a response counts as ongoing
    // containment only for the bounded response window, so a cheap response
    // years ago neither grants permanent immunity nor blocks a fresh one.
    audit_records_within_cooldown(state, crate::systems::CRISIS_RESPONSE_WINDOW_DAYS)
        .any(|record| record.subject() == subject && crisis_response_contains_crisis(record))
}

pub(crate) fn crisis_was_exploited(state: &AppState, crisis_id: crate::ids::CrisisId) -> bool {
    let subject = format!("crisis:{crisis_id}");
    // Responses cannot predate the crisis, so the scan starts there.
    let earliest_day = state
        .crises
        .get(&crisis_id)
        .map_or(0, crate::core::Crisis::started_day);
    audit_records_from(state, earliest_day).any(|record| {
        record.kind() == AuditKind::CrisisResponse
            && record.subject() == subject
            && record.detail() == "response=Exploit"
    })
}

pub(crate) fn can_afford_crisis_response(
    state: &AppState,
    crisis: &crate::core::Crisis,
    response: CrisisResponse,
) -> bool {
    // Standing-reserve policy: legitimacy is the house's scarce political
    // resource — offices, laws, and heir designations all spend it. Below
    // this floor the agent declines standing-burning responses entirely,
    // exactly as its spending policy reserves treasury against known
    // obligations instead of converting every surprise into new borrowing.
    const STANDING_RESERVE_BASIS_POINTS: u16 = 2_500;
    let standing_reserve =
        CRISIS_SUPPRESS_LEGITIMACY_COST.saturating_add(STANDING_RESERVE_BASIS_POINTS);
    let dynasty = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    match response {
        CrisisResponse::Relief => {
            dynasty.treasury() >= crisis_relief_cost(crisis.severity_basis_points)
        }
        CrisisResponse::Reform => dynasty.treasury() >= CRISIS_REFORM_COST,
        // Suppression pays in treasury and standing, mirroring the canonical
        // gate so the agent never proposes a guaranteed rejection.
        CrisisResponse::Suppress => {
            dynasty.treasury() >= CRISIS_SUPPRESS_COST
                && dynasty.resources.legitimacy_basis_points >= standing_reserve
        }
        // Profiteering spends standing equal to its requirement and extracts
        // from the panicked market's clearing pool, so an empty pool or an
        // empty legitimacy reserve makes the attempt a guaranteed rejection.
        CrisisResponse::Exploit => {
            dynasty.resources.legitimacy_basis_points >= standing_reserve
                && state.market.clearing_account > Money::ZERO
        }
    }
}

pub(crate) fn preferred_labor_response(
    state: &AppState,
    agreement: &crate::core::EmploymentAgreement,
    persona: GameplayPersona,
) -> Option<LaborResponse> {
    // Poor working conditions prioritize improvement, but a business that
    // cannot fund it falls through to the persona's remaining options instead
    // of re-proposing a guaranteed rejection every cycle.
    if agreement.conditions_basis_points < 5_000
        && can_execute_labor_response(state, agreement, LaborResponse::ImproveConditions)
    {
        return Some(LaborResponse::ImproveConditions);
    }
    labor_responses(persona)
        .into_iter()
        .find(|response| can_execute_labor_response(state, agreement, *response))
}

pub(crate) fn can_execute_labor_response(
    state: &AppState,
    agreement: &crate::core::EmploymentAgreement,
    response: LaborResponse,
) -> bool {
    let Some(business) = state.businesses.get(agreement.business_id) else {
        return false;
    };
    // Player-driven labor spending draws on the business's cash above its
    // operating-reserve floor, exactly like the canonical command validation.
    let spendable = business_operating_spendable_cash(business);
    match response {
        LaborResponse::ImproveConditions => spendable >= LABOR_CONDITIONS_IMPROVEMENT_COST,
        LaborResponse::Negotiate => spendable >= LABOR_NEGOTIATION_COST,
        LaborResponse::ReplaceWorkers => {
            spendable >= LABOR_REPLACEMENT_COST
                && state
                    .households
                    .ids_for_district(business.district_id())
                    .is_some_and(|ids| {
                        ids.iter().any(|household_id| {
                            *household_id != agreement.household_id
                                && available_household_workers(state, *household_id)
                                    >= u32::from(agreement.workers)
                        })
                    })
        }
    }
}

pub(crate) fn generate_business_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player_businesses: Vec<_> = state
        .businesses
        .ids_for_owner(state.player_dynasty_id)
        .into_iter()
        .flatten()
        .filter_map(|id| state.businesses.get(*id))
        .collect();
    generate_business_acquisition_candidates(
        registry,
        state,
        persona,
        &player_businesses,
        candidates,
    );
    for business in &player_businesses {
        generate_business_investment_candidate(registry, state, persona, business, candidates);
        generate_business_policy_candidates(state, persona, business, candidates);
    }
    generate_business_wage_candidates(registry, state, persona, candidates);
    generate_cash_rebalance_candidate(registry, state, &player_businesses, candidates);
    generate_owner_distribution_candidate(registry, state, persona, &player_businesses, candidates);
}

/// Wage-posture policy per persona. Wages are a standing labor commitment:
/// strained or disputed workforces get restored to at least fair pay,
/// stewards buy loyalty buffer with generosity, and opportunists squeeze a
/// healthy workforce for margin while it lasts.
pub(crate) fn generate_business_wage_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player_id = state.player_dynasty_id;
    let reference_copper =
        market_reference_weekly_wage(registry, state).map_or(35_i64, Money::copper);
    for business in state
        .businesses
        .ids_for_owner(player_id)
        .into_iter()
        .flatten()
        .filter_map(|id| state.businesses.get(*id))
        .filter(|business| is_open_business(business))
    {
        let subject = format!("business:{}", business.id());
        if audit_records_within_cooldown(state, BUSINESS_WAGE_CHANGE_INTERVAL_DAYS)
            .find(|record| {
                record.kind() == crate::core::AuditKind::BusinessWageChange
                    && record.subject() == subject
            })
            .is_some()
        {
            continue;
        }
        // Canonical wage changes apply to every non-Ended agreement of the
        // business at once, so the posture reads the whole workforce instead
        // of one arbitrary agreement whose pay may have drifted.
        let agreements: Vec<_> = state
            .employment
            .values()
            .filter(|agreement| {
                agreement.business_id() == business.id()
                    && agreement.status != EmploymentStatus::Ended
            })
            .collect();
        if agreements.is_empty() {
            continue;
        }
        let total_workers: i64 = agreements
            .iter()
            .map(|agreement| i64::from(agreement.workers().max(1)))
            .sum();
        let current_per_worker = Money::from_copper(
            agreements
                .iter()
                .map(|agreement| agreement.weekly_wage().copper().max(0))
                .sum::<i64>()
                / total_workers.max(1),
        );
        let Some(target_per_worker) = wage_posture_target_copper(
            persona,
            &agreements,
            reference_copper,
            business.finance.lifetime_revenue,
            business.finance.lifetime_costs,
        ) else {
            continue;
        };
        let target = Money::from_copper(target_per_worker.min(MAX_WEEKLY_WAGE_PER_WORKER.copper()));
        let change = (target.copper() - current_per_worker.copper()).abs();
        if target == current_per_worker || change.saturating_mul(20) < current_per_worker.copper() {
            continue;
        }
        let direction = if target > current_per_worker {
            "raise"
        } else {
            "reduce"
        };
        push_candidate(
            candidates,
            GameplayCommandKind::SetBusinessWages,
            PlayerCommand::SetBusinessWages {
                business_id: business.id(),
                weekly_wage_per_worker: target,
            },
            format!(
                "{direction} the wage of {} from {current_per_worker} to {target} per worker",
                business_label(state, business.id())
            ),
            700 + workforce_strain_urgency(state),
        );
    }
}

/// Returns the persona's desired per-worker wage in copper, or `None` when the
/// current posture already fits.
/// Returns the persona's desired per-worker wage in copper, or `None` when the
/// current posture already fits. Reads the business's whole non-Ended
/// workforce: canonical wage changes apply to every agreement at once.
pub(crate) fn wage_posture_target_copper(
    persona: GameplayPersona,
    agreements: &[&crate::core::EmploymentAgreement],
    reference_copper: i64,
    lifetime_revenue: Money,
    lifetime_costs: Money,
) -> Option<i64> {
    debug_assert!(!agreements.is_empty());
    let total_workers = agreements
        .iter()
        .map(|agreement| i64::from(agreement.workers().max(1)))
        .sum::<i64>()
        .max(1);
    let current = agreements
        .iter()
        .map(|agreement| agreement.weekly_wage().copper().max(0))
        .sum::<i64>()
        / total_workers;
    let weakest = agreements
        .iter()
        .map(|agreement| {
            agreement
                .loyalty_basis_points()
                .min(agreement.conditions_basis_points())
        })
        .min()
        .unwrap_or(10_000);
    let disputed = agreements
        .iter()
        .any(|agreement| agreement.status == EmploymentStatus::Disputed);
    if disputed || weakest < 3_500 {
        // Repair posture: restore at least fair pay before resistance hardens.
        return Some(current.max(reference_copper));
    }
    match persona {
        GameplayPersona::Steward => {
            // Generosity builds the loyal buffer that absorbs operating strain.
            let generous = reference_copper * 5 / 4;
            (current < generous).then_some(generous)
        }
        GameplayPersona::Opportunist => {
            // Squeeze a healthy, profitable workforce while it stays calm.
            let profitable = lifetime_revenue >= lifetime_costs && lifetime_revenue > Money::ZERO;
            let calm = weakest > 7_500;
            if profitable && calm && current >= reference_copper * 6 / 5 {
                Some(current * 4 / 5)
            } else {
                None
            }
        }
        GameplayPersona::Entrepreneur | GameplayPersona::PowerBroker => None,
    }
}

pub(crate) fn has_transfer_cash_opportunity(state: &AppState) -> bool {
    // The canonical route (`apply_cash_transfer`) accepts a positive transfer
    // between two distinct player-owned, non-insolvent, non-closed businesses
    // when the source covers the amount. The agent's rebalancing cadence and
    // minimum amounts are policy; the activation predicate mirrors the game so
    // an idle portfolio with transferable cash is never misread as dormant.
    let player_businesses: Vec<_> = state
        .businesses
        .ids_for_owner(state.player_dynasty_id)
        .into_iter()
        .flatten()
        .filter_map(|id| state.businesses.get(*id))
        .filter(|business| is_open_business(business))
        .collect();
    if player_businesses.len() < 2 {
        return false;
    }
    let Some(source) = player_businesses
        .iter()
        .max_by_key(|business| business.cash())
    else {
        return false;
    };
    source.cash() > Money::ZERO
        && player_businesses.iter().any(|business| {
            business.id() != source.id() && business.cash().checked_add(source.cash()).is_some()
        })
}

pub(crate) fn has_withdrawal_cash_opportunity(registry: &Registry, state: &AppState) -> bool {
    // The canonical route (`apply_business_cash_withdrawal`) accepts a positive
    // withdrawal from any Active player-owned business up to its surplus over
    // the owner-distribution reserve. The agent's distribution cadence and
    // thresholds are policy; the activation predicate mirrors the game so an
    // Active business with distributable surplus is never misread as dormant.
    state
        .businesses
        .ids_for_owner(state.player_dynasty_id)
        .into_iter()
        .flatten()
        .filter_map(|id| state.businesses.get(*id))
        .any(|business| {
            business.status() == BusinessStatus::Active
                && business.cash() > business_owner_distribution_reserve(registry, business)
        })
}

pub(crate) fn generate_business_policy_candidates(
    state: &AppState,
    persona: GameplayPersona,
    business: &crate::core::Business,
    candidates: &mut Vec<Candidate>,
) {
    let policy_subject = format!("business:{}", business.id());
    let policy_change_available =
        audit_records_within_cooldown(state, BUSINESS_POLICY_CHANGE_INTERVAL_DAYS)
            .find(|record| {
                record.kind() == AuditKind::BusinessPolicyChange
                    && record.subject() == policy_subject
            })
            .is_none();
    if !policy_change_available {
        return;
    }
    let desired_label = preferred_policy_label(persona, business);
    for template in policy_templates(persona)
        .into_iter()
        .filter(|template| template.label == desired_label)
    {
        if business.policy.target_input_days == template.target_input_days
            && business.policy.target_output_days == template.target_output_days
            && business.policy.minimum_cash_reserve == template.minimum_cash_reserve
            && business.policy.maintenance_basis_points == template.maintenance_basis_points
            && business.policy.quality_target_basis_points == template.quality_target_basis_points
        {
            continue;
        }
        push_candidate(
            candidates,
            GameplayCommandKind::SetBusinessPolicy,
            PlayerCommand::SetBusinessPolicy {
                business_id: business.id(),
                target_input_days: template.target_input_days,
                target_output_days: template.target_output_days,
                minimum_cash_reserve: template.minimum_cash_reserve,
                maintenance_basis_points: template.maintenance_basis_points,
                quality_target_basis_points: template.quality_target_basis_points,
            },
            format!(
                "set {} policy on {}",
                template.label,
                business_label(state, business.id())
            ),
            template.bonus,
        );
    }
}

pub(crate) fn preferred_policy_label(
    persona: GameplayPersona,
    business: &crate::core::Business,
) -> &'static str {
    let stressed = matches!(
        business.status(),
        BusinessStatus::Distressed | BusinessStatus::Insolvent
    ) || business.operations.condition_basis_points < 6_000
        || business.cash() < business.policy.minimum_cash_reserve;
    if stressed {
        return "defensive";
    }
    match persona {
        GameplayPersona::Steward => {
            if business.operations.quality_basis_points < 8_500
                && business.cash() >= Money::from_copper(6_000)
            {
                "premium"
            } else {
                "defensive"
            }
        }
        GameplayPersona::Entrepreneur => {
            if business.operations.quality_basis_points < 8_000 {
                "premium"
            } else {
                "growth"
            }
        }
        GameplayPersona::PowerBroker => "defensive",
        GameplayPersona::Opportunist => "growth",
    }
}

pub(crate) fn generate_cash_rebalance_candidate(
    registry: &Registry,
    state: &AppState,
    player_businesses: &[&crate::core::Business],
    candidates: &mut Vec<Candidate>,
) {
    if player_businesses.len() < 2 {
        return;
    }
    if audit_records_within_cooldown(state, AGENT_CASH_REBALANCE_INTERVAL_DAYS)
        .any(|record| record.kind() == AuditKind::CashTransfer)
    {
        return;
    }
    let source = player_businesses.iter().max_by_key(|business| {
        business
            .cash()
            .copper()
            .saturating_sub(business_cash_target(registry, state, business).copper())
    });
    let target = player_businesses.iter().max_by_key(|business| {
        business_cash_target(registry, state, business)
            .copper()
            .saturating_sub(business.cash().copper())
    });
    let (Some(source), Some(target)) = (source, target) else {
        return;
    };
    if source.id() == target.id() {
        return;
    }
    let source_surplus = source
        .cash()
        .copper()
        .saturating_sub(business_cash_target(registry, state, source).copper())
        .max(0);
    let target_deficit = business_cash_target(registry, state, target)
        .copper()
        .saturating_sub(target.cash().copper())
        .max(0);
    if target_deficit < AGENT_CASH_REBALANCE_TRIGGER.copper() {
        return;
    }
    let buffered_deficit = business_cash_target(registry, state, target)
        .saturating_add(AGENT_CASH_REBALANCE_BUFFER)
        .copper()
        .saturating_sub(target.cash().copper())
        .max(0);
    let amount = Money::from_copper(source_surplus.min(buffered_deficit));
    if amount < AGENT_CASH_REBALANCE_TRIGGER {
        return;
    }
    let urgency = if matches!(
        target.status(),
        BusinessStatus::Distressed | BusinessStatus::Insolvent
    ) {
        900
    } else {
        250
    };
    push_candidate(
        candidates,
        GameplayCommandKind::TransferBusinessCash,
        PlayerCommand::TransferBusinessCash {
            from_business_id: source.id(),
            to_business_id: target.id(),
            amount,
        },
        format!(
            "cover a {amount} liquidity shortfall from {} to {}",
            business_label(state, source.id()),
            business_label(state, target.id())
        ),
        urgency,
    );
}

pub(crate) fn generate_owner_distribution_candidate(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    player_businesses: &[&crate::core::Business],
    candidates: &mut Vec<Candidate>,
) {
    if generate_strategic_withdrawal_candidate(
        registry,
        state,
        persona,
        player_businesses,
        candidates,
    ) {
        return;
    }

    generate_ordinary_distribution_candidate(
        registry,
        state,
        persona,
        player_businesses,
        candidates,
    );
}

/// Offer an owner withdrawal when business surplus can fund a known dynasty
/// commitment without violating the business operating reserve.
pub(crate) fn generate_strategic_withdrawal_candidate(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    player_businesses: &[&crate::core::Business],
    candidates: &mut Vec<Candidate>,
) -> bool {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let office_reserve = player_office_duty_reserve(state, 0);
    let legal_commitment = legal_settlement_funding_target(state).unwrap_or(Money::ZERO);
    let endowment_commitment = if state.institutions.values().any(|institution| {
        has_established_player_institution_membership(state, institution.institution_id)
    }) {
        INSTITUTION_ENDOWMENT_MIN
    } else {
        Money::ZERO
    };
    let strategic_need = office_reserve
        .saturating_add(legal_commitment)
        .saturating_add(endowment_commitment);
    let strategic_shortfall = strategic_need.saturating_sub(player.treasury());
    let Some((source, surplus)) = player_businesses
        .iter()
        .filter(|business| business.status() == BusinessStatus::Active)
        .filter_map(|business| {
            let reserve = business_owner_distribution_reserve(registry, business);
            let surplus = business.cash().saturating_sub(reserve);
            (surplus >= AGENT_STRATEGIC_WITHDRAWAL_TRIGGER).then_some((*business, surplus))
        })
        .max_by_key(|(business, surplus)| (*surplus, business.id()))
    else {
        return false;
    };
    let amount = surplus
        .min(strategic_shortfall)
        .min(AGENT_STRATEGIC_WITHDRAWAL_MAX);
    if amount < AGENT_STRATEGIC_WITHDRAWAL_TRIGGER {
        return false;
    }
    let intent = if office_reserve >= legal_commitment.max(endowment_commitment) {
        "cover projected office duties"
    } else if legal_commitment >= endowment_commitment && legal_commitment > Money::ZERO {
        "fund the pending legal settlement"
    } else if endowment_commitment > Money::ZERO {
        "capitalize an institution endowment"
    } else {
        "restore dynasty liquidity"
    };
    push_candidate(
        candidates,
        GameplayCommandKind::WithdrawBusinessCash,
        PlayerCommand::WithdrawBusinessCash {
            business_id: source.id(),
            amount,
        },
        format!(
            "withdraw {amount} of surplus from {} to {intent}",
            business_label(state, source.id())
        ),
        2_400_i64
            .saturating_add(amount.copper() / 20)
            .saturating_add(persona_distribution_bonus(persona)),
    );
    true
}

pub(crate) fn generate_ordinary_distribution_candidate(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    player_businesses: &[&crate::core::Business],
    candidates: &mut Vec<Candidate>,
) {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let legal_requirement = active_legal_settlement_requirement(state);

    let ordinary_liquidity_target = match persona {
        GameplayPersona::Entrepreneur => Money::from_copper(3_500),
        GameplayPersona::Steward | GameplayPersona::PowerBroker => Money::from_copper(3_000),
        GameplayPersona::Opportunist => Money::from_copper(2_500),
    };
    let liquidity_target = ordinary_liquidity_target.max(legal_requirement.unwrap_or(Money::ZERO));
    if player.treasury() >= liquidity_target {
        return;
    }
    let recent_owner_distribution =
        audit_records_within_cooldown(state, AGENT_OWNER_DISTRIBUTION_INTERVAL_DAYS).any(
            |record| {
                record.kind() == AuditKind::BusinessDividend
                    && record.subject().starts_with("business:")
                    && record.detail().starts_with("owner_distribution=")
            },
        );
    if recent_owner_distribution && legal_requirement.is_none() {
        return;
    }
    let source = player_businesses
        .iter()
        .filter(|business| business.status() == BusinessStatus::Active)
        .filter_map(|business| {
            let reserve = business_owner_distribution_reserve(registry, business);
            let surplus = business.cash().saturating_sub(reserve);
            (surplus >= AGENT_OWNER_DISTRIBUTION_TRIGGER).then_some((*business, surplus))
        })
        .max_by_key(|(business, surplus)| (*surplus, business.id()));
    let Some((source, surplus)) = source else {
        return;
    };
    let liquidity_gap = liquidity_target.saturating_sub(player.treasury());
    if legal_requirement.is_some() && surplus < liquidity_gap {
        return;
    }
    let amount = surplus.min(liquidity_gap);
    if amount < AGENT_OWNER_DISTRIBUTION_TRIGGER {
        return;
    }
    let bonus = if legal_requirement.is_some() {
        persona_distribution_bonus(persona).saturating_add(2_500)
    } else {
        persona_distribution_bonus(persona)
    };
    push_candidate(
        candidates,
        GameplayCommandKind::WithdrawBusinessCash,
        PlayerCommand::WithdrawBusinessCash {
            business_id: source.id(),
            amount,
        },
        format!(
            "withdraw {amount} of surplus from {} to restore dynasty liquidity",
            business_label(state, source.id())
        ),
        bonus,
    );
}

pub(crate) const fn persona_distribution_bonus(persona: GameplayPersona) -> i64 {
    match persona {
        GameplayPersona::Steward => 1_450,
        GameplayPersona::Entrepreneur => 1_550,
        GameplayPersona::PowerBroker => 1_500,
        GameplayPersona::Opportunist => 1_650,
    }
}

pub(crate) fn business_cash_target(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
) -> Money {
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe must exist");
    let payroll_buffer = state
        .employment
        .values()
        .filter(|agreement| {
            agreement.business_id == business.id()
                && matches!(
                    agreement.status,
                    EmploymentStatus::Active | EmploymentStatus::Disputed
                )
        })
        .fold(Money::ZERO, |total, agreement| {
            total.saturating_add(agreement.weekly_wage)
        });
    let recovery_buffer = if matches!(
        business.status(),
        BusinessStatus::Distressed | BusinessStatus::Insolvent
    ) {
        Money::from_copper(2_000)
    } else {
        Money::ZERO
    };
    business
        .policy
        .minimum_cash_reserve
        .saturating_add(recipe.daily_operating_cost().saturating_mul(7))
        .saturating_add(payroll_buffer)
        .saturating_add(recovery_buffer)
}

pub(crate) fn generate_business_investment_candidate(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    business: &crate::core::Business,
    candidates: &mut Vec<Candidate>,
) {
    if business.status() == BusinessStatus::Active {
        generate_planned_business_investment(state, persona, business, candidates);
        return;
    }
    if !matches!(
        business.status(),
        BusinessStatus::Distressed | BusinessStatus::Insolvent
    ) {
        return;
    }
    if has_internal_cash_recovery(registry, state, business) {
        return;
    }
    let player_treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe must exist");
    let average_food_satisfaction = average_household_food_satisfaction(state);
    let staple_emergency = registry
        .get_good(recipe.output_good_id())
        .is_some_and(|good| good.category() == GoodCategory::Staple)
        && average_food_satisfaction < 5_000;
    let severe_rehabilitation = business.operations.condition_basis_points < 2_000;
    let portfolio_emergency = player_has_no_active_business(state);
    let dynasty_reserve = if portfolio_emergency {
        Money::ZERO
    } else if severe_rehabilitation {
        Money::from_copper(2_000)
    } else {
        recapitalization_dynasty_reserve(persona, staple_emergency)
    };
    let spendable = Money::from_copper(
        player_treasury
            .copper()
            .saturating_sub(dynasty_reserve.copper())
            .max(0),
    );
    if spendable <= Money::ZERO {
        return;
    }
    let target_cash = business_recapitalization_target(registry, state, business);
    let shortfall = Money::from_copper(
        target_cash
            .copper()
            .saturating_sub(business.cash().copper())
            .max(0),
    );
    let amount = shortfall.min(spendable);
    let minimum_meaningful = recipe.daily_operating_cost().saturating_mul(7);
    if amount <= Money::ZERO
        || (!staple_emergency && amount < minimum_meaningful && amount < shortfall)
    {
        return;
    }
    let persona_bonus: i64 = match persona {
        GameplayPersona::Steward => 760,
        GameplayPersona::Entrepreneur => 700,
        GameplayPersona::PowerBroker => 260,
        GameplayPersona::Opportunist => 180,
    };
    let emergency_bonus = if portfolio_emergency {
        4_500
    } else if staple_emergency {
        3_000
    } else if severe_rehabilitation {
        2_600
    } else {
        0
    };
    push_candidate(
        candidates,
        GameplayCommandKind::InvestInBusiness,
        PlayerCommand::InvestInBusiness {
            business_id: business.id(),
            amount,
        },
        format!(
            "invest {amount} in {}",
            business_label(state, business.id())
        ),
        persona_bonus
            .saturating_add(1_700)
            .saturating_add(emergency_bonus),
    );
}

pub(crate) fn generate_planned_business_investment(
    state: &AppState,
    persona: GameplayPersona,
    business: &crate::core::Business,
    candidates: &mut Vec<Candidate>,
) {
    let has_trade_evidence = business.finance.lifetime_revenue > Money::ZERO
        || business.finance.lifetime_costs > Money::ZERO;
    if !has_trade_evidence {
        return;
    }
    let subject = format!("business:{}", business.id());
    if audit_records_within_cooldown(state, AGENT_PLANNED_CAPITALIZATION_INTERVAL_DAYS).any(
        |record| record.kind() == AuditKind::BusinessCapitalization && record.subject() == subject,
    ) {
        return;
    }
    let target_condition = 9_000_u16;
    let target_quality = business.policy.quality_target_basis_points.max(7_500);
    let condition_investment =
        i64::from(target_condition.saturating_sub(business.operations.condition_basis_points))
            .saturating_mul(2);
    let quality_investment =
        i64::from(target_quality.saturating_sub(business.operations.quality_basis_points))
            .saturating_mul(4);
    let desired = Money::from_copper(condition_investment.max(quality_investment));
    if desired < Money::from_copper(600) {
        return;
    }
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let mut reserve = recapitalization_dynasty_reserve(persona, false);
    if state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_some_and(|d| d.resources.legitimacy_basis_points > 6_000)
        && state.institutions.values().any(|i| {
            i.office_holder_id.is_some_and(|h| {
                state
                    .characters
                    .get(h)
                    .is_some_and(|c| c.dynasty_id() == state.player_dynasty_id)
            })
        })
    {
        reserve = Money::from_copper(reserve.copper().saturating_sub(2_000).max(0));
    }
    let spendable = Money::from_copper(treasury.copper().saturating_sub(reserve.copper()).max(0));
    let amount = desired.min(AGENT_PLANNED_CAPITALIZATION_MAX).min(spendable);
    if amount < Money::from_copper(600) {
        return;
    }
    let bonus = match persona {
        GameplayPersona::Entrepreneur => 900,
        GameplayPersona::Steward => 750,
        GameplayPersona::Opportunist => 500,
        GameplayPersona::PowerBroker => 550,
    };
    push_candidate(
        candidates,
        GameplayCommandKind::InvestInBusiness,
        PlayerCommand::InvestInBusiness {
            business_id: business.id(),
            amount,
        },
        format!(
            "modernize {} with {amount} of condition and quality investment",
            business_label(state, business.id())
        ),
        bonus,
    );
}

pub(crate) fn has_internal_cash_recovery(
    registry: &Registry,
    state: &AppState,
    target: &crate::core::Business,
) -> bool {
    let target_deficit = business_cash_target(registry, state, target)
        .copper()
        .saturating_sub(target.cash().copper())
        .max(0);
    if target_deficit < AGENT_CASH_REBALANCE_TRIGGER.copper() {
        return false;
    }
    state
        .businesses
        .ids_for_owner(state.player_dynasty_id)
        .into_iter()
        .flatten()
        .filter_map(|business_id| state.businesses.get(*business_id))
        .filter(|business| business.id() != target.id())
        .any(|business| {
            business
                .cash()
                .copper()
                .saturating_sub(business_cash_target(registry, state, business).copper())
                >= AGENT_CASH_REBALANCE_TRIGGER.copper()
        })
}

pub(crate) fn average_household_food_satisfaction(state: &AppState) -> u16 {
    crate::core::population_weighted_food_satisfaction_basis_points(state.households.iter())
        .unwrap_or(10_000)
}

pub(crate) const fn recapitalization_dynasty_reserve(
    persona: GameplayPersona,
    staple_emergency: bool,
) -> Money {
    if staple_emergency {
        return Money::ZERO;
    }
    match persona {
        GameplayPersona::Steward => Money::from_copper(8_000),
        GameplayPersona::Entrepreneur => Money::from_copper(5_000),
        GameplayPersona::PowerBroker => Money::from_copper(10_000),
        GameplayPersona::Opportunist => Money::from_copper(4_000),
    }
}

pub(crate) fn generate_business_acquisition_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    player_businesses: &[&crate::core::Business],
    candidates: &mut Vec<Candidate>,
) {
    let portfolio_limit = match persona {
        GameplayPersona::Entrepreneur | GameplayPersona::Opportunist => 3,
        GameplayPersona::Steward | GameplayPersona::PowerBroker => 2,
    };
    if player_businesses.len() >= portfolio_limit {
        return;
    }
    if !portfolio_ready_for_acquisition(state, player_businesses) {
        return;
    }
    let has_financially_stressed_business = player_businesses.iter().any(|business| {
        matches!(
            business.status(),
            BusinessStatus::Distressed | BusinessStatus::Insolvent
        )
    });
    let Some(manager_id) = acquisition_manager_id(state, player_businesses) else {
        return;
    };
    let operating_businesses = player_businesses
        .iter()
        .filter(|business| {
            matches!(
                business.status(),
                BusinessStatus::Active | BusinessStatus::Distressed
            )
        })
        .count();
    if operating_businesses > 0 && has_financially_stressed_business {
        return;
    }
    let persona_bonus: i64 = match persona {
        GameplayPersona::Entrepreneur => 720,
        GameplayPersona::Opportunist => 520,
        GameplayPersona::Steward => 320,
        GameplayPersona::PowerBroker => 280,
    };
    let recovery_bonus = if operating_businesses == 0 { 1_000 } else { 0 };
    for business in state
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id() != state.player_dynasty_id)
    {
        push_acquisition_candidate(
            registry,
            state,
            persona,
            business,
            manager_id,
            i64::try_from(player_businesses.len()).unwrap_or(i64::MAX),
            persona_bonus + recovery_bonus,
            recovery_bonus,
            candidates,
        );
    }
}

/// Evaluates one acquisition target under the agent's affordability and thesis
/// policies. A rescue of a failing trade outranks a premium purchase of a
/// healthy one: distress discounts are scarce, while going concerns are always
/// theoretically for sale at the right price.
#[allow(clippy::too_many_arguments)]
fn push_acquisition_candidate(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    business: &crate::core::Business,
    manager_id: crate::ids::CharacterId,
    owned_business_count: i64,
    rescue_bonus: i64,
    recovery_bonus: i64,
    candidates: &mut Vec<Candidate>,
) {
    let going_concern = business.status() == BusinessStatus::Active;
    // A premium purchase must buy a real going concern: an active firm whose
    // equipment has already run down is a distress sale waiting to happen, and
    // paying a controlling premium for it is not growth.
    if going_concern && business.operations.condition_basis_points < 5_000 {
        return;
    }
    let Ok(quote) =
        quote_business_acquisition(registry, state, state.player_dynasty_id, business.id())
    else {
        return;
    };
    let Some(recapitalization) = acquisition_recapitalization(registry, state, business, quote)
    else {
        return;
    };
    let required = quote.purchase_price.saturating_add(recapitalization);
    let player_treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let mut expansion_reserve = recapitalization_dynasty_reserve(persona, false).saturating_add(
        Money::from_copper(owned_business_count.saturating_mul(2_000)),
    );
    if going_concern {
        // Paying a controlling premium drains the treasury materially: hold
        // back half the price again so the exchange cannot strip every reserve
        // the house keeps for obligations and shocks.
        expansion_reserve =
            expansion_reserve.saturating_add(Money::from_copper(quote.purchase_price.copper() / 2));
    }
    if player_treasury < required.saturating_add(expansion_reserve) {
        return;
    }
    if !acquisition_has_turnaround_thesis(registry, state, business) {
        return;
    }
    let bonus = if going_concern {
        rescue_bonus / 2 + recovery_bonus / 2
    } else {
        rescue_bonus
    };
    push_candidate(
        candidates,
        GameplayCommandKind::AcquireBusiness,
        PlayerCommand::AcquireBusiness {
            business_id: business.id(),
            manager_id,
            recapitalization,
        },
        format!(
            "acquire {}{} for {} with {} working capital",
            if going_concern {
                "the going concern "
            } else {
                ""
            },
            business_label(state, business.id()),
            quote.purchase_price,
            recapitalization
        ),
        bonus,
    );
}

/// A turnaround thesis: recapitalization fixes condition and working capital,
/// so only acquisitions whose output already sells above the firm's own
/// break-even are sound. A business whose market price sits below its
/// sustainable unit cost keeps bleeding no matter how much capital it is
/// handed, and buying one converts a profitable estate into a consolidated
/// loss centre.
pub(crate) fn acquisition_has_turnaround_thesis(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
) -> bool {
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("acquisition target recipe must exist");
    let Some(quote_price) = state.market.quotes.get(&recipe.output_good_id()) else {
        return false;
    };
    // The same sustainable unit cost the market's production price floors are
    // built from, so the agent's thesis can never drift from the simulation's
    // own definition of "sells above break-even".
    let unit_cost = business_sustainable_unit_cost(
        registry,
        state,
        business,
        crate::systems::dynasty_office_administrative_load(state, business.owner_dynasty_id()),
    );
    quote_price.price >= unit_cost
}

pub(crate) fn portfolio_ready_for_acquisition(
    state: &AppState,
    player_businesses: &[&crate::core::Business],
) -> bool {
    player_businesses.iter().all(|business| {
        business.status() == BusinessStatus::Active
            && business.operations.condition_basis_points >= 7_000
            && business.cash() >= business.policy.minimum_cash_reserve
            && business.finance.lifetime_revenue >= business.finance.lifetime_costs
            && !state.employment.values().any(|agreement| {
                agreement.business_id == business.id()
                    && agreement.status == EmploymentStatus::Disputed
            })
    })
}

pub(crate) fn acquisition_manager_id(
    state: &AppState,
    player_businesses: &[&crate::core::Business],
) -> Option<crate::ids::CharacterId> {
    let assigned_managers: BTreeSet<_> = player_businesses
        .iter()
        .map(|business| business.manager_id())
        .collect();
    let active_characters = || {
        state
            .characters
            .ids_for_dynasty(state.player_dynasty_id)
            .into_iter()
            .flatten()
            .filter_map(|character_id| state.characters.get(*character_id))
            .filter(|character| character.status() == CharacterStatus::Active)
    };
    active_characters()
        .filter(|character| !assigned_managers.contains(&character.id()))
        .max_by_key(|character| {
            u32::from(character.capabilities.craft)
                .saturating_add(u32::from(character.capabilities.commerce))
        })
        .or_else(|| {
            active_characters().max_by_key(|character| {
                u32::from(character.capabilities.craft)
                    .saturating_add(u32::from(character.capabilities.commerce))
            })
        })
        .map(crate::core::Character::id)
}

/// Sizes the recapitalization for an acquisition: a comfortable cushion above
/// the quoted minimum when the treasury allows it. Returns `None` when the
/// treasury cannot fund even the canonical minimum after the purchase price,
/// so the candidate is skipped instead of being proposed as a guaranteed
/// rejection.
pub(crate) fn acquisition_recapitalization(
    registry: &Registry,
    state: &AppState,
    business: &crate::core::Business,
    quote: crate::systems::BusinessAcquisitionQuote,
) -> Option<Money> {
    let recipe = registry
        .get_recipe(business.recipe_id())
        .expect("business recipe must exist");
    let desired = quote
        .minimum_recapitalization
        .saturating_add(recipe.daily_operating_cost().saturating_mul(14));
    let player_treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let available = player_treasury
        .copper()
        .saturating_sub(quote.purchase_price.copper())
        .max(0);
    if available < quote.minimum_recapitalization.copper() {
        return None;
    }
    Some(Money::from_copper(desired.copper().min(available)))
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PolicyTemplate {
    pub label: &'static str,
    pub target_input_days: u16,
    pub target_output_days: u16,
    pub minimum_cash_reserve: Money,
    pub maintenance_basis_points: u16,
    pub quality_target_basis_points: u16,
    pub bonus: i64,
}

pub(crate) fn policy_templates(persona: GameplayPersona) -> [PolicyTemplate; 3] {
    let premium_bonus = match persona {
        GameplayPersona::Entrepreneur => 260,
        GameplayPersona::Steward => 160,
        GameplayPersona::PowerBroker | GameplayPersona::Opportunist => 40,
    };
    let growth_bonus = match persona {
        GameplayPersona::Entrepreneur | GameplayPersona::Opportunist => 240,
        GameplayPersona::Steward => 80,
        GameplayPersona::PowerBroker => 20,
    };
    let defensive_bonus = match persona {
        GameplayPersona::Steward => 300,
        GameplayPersona::PowerBroker => 100,
        GameplayPersona::Entrepreneur => 60,
        GameplayPersona::Opportunist => 10,
    };
    let growth_maintenance_basis_points = if persona == GameplayPersona::Opportunist {
        400
    } else {
        800
    };
    [
        PolicyTemplate {
            label: "premium",
            target_input_days: 7,
            target_output_days: 3,
            minimum_cash_reserve: Money::from_copper(4_000),
            maintenance_basis_points: 1_800,
            quality_target_basis_points: 9_000,
            bonus: premium_bonus,
        },
        PolicyTemplate {
            label: "growth",
            target_input_days: 12,
            target_output_days: 1,
            minimum_cash_reserve: Money::from_copper(1_000),
            maintenance_basis_points: growth_maintenance_basis_points,
            quality_target_basis_points: 7_000,
            bonus: growth_bonus,
        },
        PolicyTemplate {
            label: "defensive",
            target_input_days: 5,
            target_output_days: 4,
            minimum_cash_reserve: Money::from_copper(8_000),
            maintenance_basis_points: 1_300,
            quality_target_basis_points: 7_800,
            bonus: defensive_bonus,
        },
    ]
}

pub(crate) fn generate_contract_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player_id = state.player_dynasty_id;
    for business in state.businesses.iter().filter(|business| {
        business.owner_dynasty_id() == player_id
            && !matches!(
                business.status(),
                BusinessStatus::Closed | BusinessStatus::Insolvent
            )
    }) {
        let recipe = registry
            .get_recipe(business.recipe_id())
            .expect("business recipe must exist");
        for input in recipe.inputs() {
            for seller in contract_sellers(registry, state, input.good_id(), player_id) {
                // Commit near the buyer's real consumption, sized down to what
                // the working-cash cushion supports. A token contract cannot
                // secure supply, and a supplier that never owes enough to hurt
                // makes counterparty reliability invisible. The five-day basis
                // matches the canonical `CONTRACT_CAPACITY_COMMITMENT_DAYS`,
                // keeping open-market trade possible alongside the contract.
                let weekly_need = input.quantity().saturating_mul_ratio(
                    i64::from(business.operations.capacity_batches_per_day)
                        .saturating_mul(AGENT_CONTRACT_COMMITMENT_DAYS),
                    1,
                );
                for quantity in contract_size_ladder(weekly_need) {
                    let candidate = ContractCandidateInput {
                        kind: GameplayCommandKind::SecureSupply,
                        buyer_business_id: business.id(),
                        seller_business_id: seller,
                        good_id: input.good_id(),
                        quantity_per_week: quantity,
                        bonus: secure_supply_bonus(persona),
                    };
                    if contract_candidate_fits(registry, state, &candidate) {
                        add_contract_candidate(registry, state, candidates, candidate);
                        break;
                    }
                }
            }
        }
        for buyer in contract_buyers(registry, state, recipe.output_good_id(), player_id) {
            // Outgoing commitments stay deliberately lighter than incoming
            // ones: a three-day basis leaves the seller margin to absorb a bad
            // production week, so signing away most of the firm's capacity is
            // an aggressive bet the player can decline rather than a trap.
            let weekly_output = recipe.output_quantity().saturating_mul_ratio(
                i64::from(business.operations.capacity_batches_per_day)
                    .saturating_mul(AGENT_SELL_COMMITMENT_DAYS),
                1,
            );
            for quantity in contract_size_ladder(weekly_output) {
                let candidate = ContractCandidateInput {
                    kind: GameplayCommandKind::SellOutput,
                    buyer_business_id: buyer,
                    seller_business_id: business.id(),
                    good_id: recipe.output_good_id(),
                    quantity_per_week: quantity,
                    bonus: sell_output_bonus(persona),
                };
                if contract_candidate_fits(registry, state, &candidate) {
                    add_contract_candidate(registry, state, candidates, candidate);
                    break;
                }
            }
        }
    }
}

/// Commitment bases mirror the canonical contract capacity window:
/// incoming supply commits a fuller week than outgoing sales — buyers need
/// dependable input while sellers reserve slack. The window sizes contracts
/// so fulfillment pressure and breach grievances surface in ordinary play.
const AGENT_CONTRACT_COMMITMENT_DAYS: i64 = 7;
const AGENT_SELL_COMMITMENT_DAYS: i64 = 5;

/// Candidate sizes to try, largest first: the full commitment, then halves,
/// so a house with thin working cash still secures some supply instead of
/// dropping the route entirely.
fn contract_size_ladder(full: Quantity) -> Vec<Quantity> {
    let half = full.saturating_mul_ratio(1, 2);
    let quarter = full.saturating_mul_ratio(1, 4);
    vec![full, half, quarter]
}

/// Whether this exact contract would pass the shared support checks used by
/// [`add_contract_candidate`], so size ladders stop at the largest viable rung.
fn contract_candidate_fits(
    registry: &Registry,
    state: &AppState,
    candidate: &ContractCandidateInput,
) -> bool {
    if candidate.quantity_per_week <= Quantity::ZERO {
        return false;
    }
    let Some(quote) = state.market.quotes.get(&candidate.good_id) else {
        return false;
    };
    let unit_price = contract_candidate_unit_price(
        state,
        candidate.buyer_business_id,
        candidate.seller_business_id,
        quote.price,
    );
    can_support_contract_terms(
        registry,
        state,
        candidate.buyer_business_id,
        candidate.seller_business_id,
        candidate.good_id,
        candidate.quantity_per_week,
        unit_price,
    )
}

pub(crate) fn secure_supply_batches(business: &crate::core::Business) -> i64 {
    let has_trade_history = business.finance.lifetime_revenue > Money::ZERO
        || business.finance.lifetime_costs > Money::ZERO;
    if has_trade_history && business.finance.lifetime_revenue >= business.finance.lifetime_costs {
        STANDARD_CONTRACT_BATCHES_PER_WEEK
    } else {
        1
    }
}

pub(crate) const fn secure_supply_bonus(persona: GameplayPersona) -> i64 {
    match persona {
        GameplayPersona::Steward => 420,
        GameplayPersona::Entrepreneur => 520,
        GameplayPersona::PowerBroker => 80,
        GameplayPersona::Opportunist => 120,
    }
}

pub(crate) const fn sell_output_bonus(persona: GameplayPersona) -> i64 {
    match persona {
        GameplayPersona::Steward | GameplayPersona::PowerBroker => 100,
        GameplayPersona::Entrepreneur => 620,
        GameplayPersona::Opportunist => 560,
    }
}

pub(crate) fn contract_sellers<'a>(
    registry: &'a Registry,
    state: &'a AppState,
    good_id: crate::ids::GoodId,
    excluded_owner: DynastyId,
) -> impl Iterator<Item = BusinessId> + 'a {
    state.businesses.iter().filter_map(move |business| {
        let recipe = registry.get_recipe(business.recipe_id())?;
        (business.owner_dynasty_id() != excluded_owner
            && !matches!(
                business.status(),
                BusinessStatus::Closed | BusinessStatus::Insolvent
            )
            && recipe.output_good_id() == good_id)
            .then_some(business.id())
    })
}

pub(crate) fn contract_buyers<'a>(
    registry: &'a Registry,
    state: &'a AppState,
    good_id: crate::ids::GoodId,
    excluded_owner: DynastyId,
) -> impl Iterator<Item = BusinessId> + 'a {
    state.businesses.iter().filter_map(move |business| {
        let recipe = registry.get_recipe(business.recipe_id())?;
        (business.owner_dynasty_id() != excluded_owner
            && !matches!(
                business.status(),
                BusinessStatus::Closed | BusinessStatus::Insolvent
            )
            && recipe
                .inputs()
                .iter()
                .any(|input| input.good_id() == good_id))
        .then_some(business.id())
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ContractCandidateInput {
    pub kind: GameplayCommandKind,
    pub buyer_business_id: BusinessId,
    pub seller_business_id: BusinessId,
    pub good_id: crate::ids::GoodId,
    pub quantity_per_week: Quantity,
    pub bonus: i64,
}

pub(crate) fn add_contract_candidate(
    registry: &Registry,
    state: &AppState,
    candidates: &mut Vec<Candidate>,
    input: ContractCandidateInput,
) {
    let ContractCandidateInput {
        kind,
        buyer_business_id,
        seller_business_id,
        good_id,
        quantity_per_week,
        bonus,
    } = input;
    if state.contracts.values().any(|contract| {
        contract.status == ContractStatus::Active
            && contract.buyer_business_id == buyer_business_id
            && contract.seller_business_id == seller_business_id
            && contract.good_id == good_id
    }) {
        return;
    }
    let Some(quote) = state.market.quotes.get(&good_id) else {
        return;
    };
    let price_bounds = contract_counterparty_price_bounds(
        state,
        buyer_business_id,
        seller_business_id,
        quote.price,
    );
    let unit_price =
        contract_candidate_unit_price(state, buyer_business_id, seller_business_id, quote.price);
    if !can_support_contract_terms(
        registry,
        state,
        buyer_business_id,
        seller_business_id,
        good_id,
        quantity_per_week,
        unit_price,
    ) {
        return;
    }
    let Some(weekly_payment) = checked_cost_for(quantity_per_week, unit_price) else {
        return;
    };
    let Some(penalty) = weekly_payment.checked_mul_ratio(4, 1) else {
        return;
    };
    let total_scheduled_value = weekly_payment
        .checked_mul_ratio(i64::from(AGENT_CONTRACT_DURATION_WEEKS), 1)
        .unwrap_or(Money::from_copper(i64::MAX));
    let relationship_note = if price_bounds.relationship_pressure_basis_points > 0 {
        format!(
            " under {} bp of counterparty pressure",
            price_bounds.relationship_pressure_basis_points
        )
    } else {
        String::new()
    };
    push_candidate(
        candidates,
        kind,
        PlayerCommand::CreateSupplyContract {
            terms: SupplyContractTerms {
                buyer_business_id,
                seller_business_id,
                good_id,
                quantity_per_week,
                unit_price,
                penalty,
                duration_weeks: AGENT_CONTRACT_DURATION_WEEKS,
            },
        },
        format!(
            "contract {good} from {seller} to {buyer} at {unit_price}; {AGENT_CONTRACT_DURATION_WEEKS}-week term, {weekly_payment}/week ({total_scheduled_value} scheduled value){relationship_note}",
            good = good_label(registry, good_id),
            seller = business_label(state, seller_business_id),
            buyer = business_label(state, buyer_business_id),
        ),
        bonus,
    );
}

pub(crate) fn contract_candidate_unit_price(
    state: &AppState,
    buyer_business_id: BusinessId,
    seller_business_id: BusinessId,
    market_price: Money,
) -> Money {
    let price_bounds = contract_counterparty_price_bounds(
        state,
        buyer_business_id,
        seller_business_id,
        market_price,
    );
    let buyer_is_player = state
        .businesses
        .get(buyer_business_id)
        .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id);
    if buyer_is_player {
        market_price.max(price_bounds.minimum_seller_price)
    } else {
        market_price.min(price_bounds.maximum_buyer_price)
    }
}

pub(crate) fn can_support_contract_terms(
    registry: &Registry,
    state: &AppState,
    buyer_business_id: BusinessId,
    seller_business_id: BusinessId,
    good_id: crate::ids::GoodId,
    quantity_per_week: Quantity,
    unit_price: Money,
) -> bool {
    let Some(buyer) = state.businesses.get(buyer_business_id) else {
        return false;
    };
    let Some(seller) = state.businesses.get(seller_business_id) else {
        return false;
    };
    let Some(seller_recipe) = registry.get_recipe(seller.recipe_id()) else {
        return false;
    };
    let Some(capacity) = available_supply_contract_capacity(
        registry,
        state,
        buyer_business_id,
        seller_business_id,
        good_id,
    ) else {
        return false;
    };
    if quantity_per_week > capacity.seller || quantity_per_week > capacity.buyer {
        return false;
    }
    let Some(weekly_payment) = checked_cost_for(quantity_per_week, unit_price) else {
        return false;
    };
    let Some(required_working_cash) = weekly_payment.checked_mul_ratio(4, 1) else {
        return false;
    };
    let buyer_working_cash = buyer
        .cash()
        .saturating_sub(buyer.policy.minimum_cash_reserve);
    if buyer_working_cash < required_working_cash {
        return false;
    }
    seller.inventory_quantity(good_id) >= quantity_per_week
        || seller.cash() >= seller_recipe.daily_operating_cost().saturating_mul(7)
}

/// Whether the canonical game would accept a contract between these parties,
/// mirroring `ensure_non_player_contract_counterparty_accepts`: a resolvable
/// market quote, an in-band unit price for the non-player side, and capacity
/// for the non-player side. The buyer working-cash cushion and seller liquidity
/// cushion are agent policy and stay in `can_support_contract_terms`.
pub(crate) fn game_accepts_contract_terms(
    registry: &Registry,
    state: &AppState,
    buyer_business_id: BusinessId,
    seller_business_id: BusinessId,
    good_id: crate::ids::GoodId,
    quantity_per_week: Quantity,
    unit_price: Money,
) -> bool {
    let Some(market_price) = state
        .market
        .get_quote(good_id)
        .map(crate::core::MarketQuote::price)
    else {
        return false;
    };
    let price_bounds = contract_counterparty_price_bounds(
        state,
        buyer_business_id,
        seller_business_id,
        market_price,
    );
    let player_id = state.player_dynasty_id;
    let seller_is_player = state
        .businesses
        .get(seller_business_id)
        .is_some_and(|business| business.owner_dynasty_id() == player_id);
    let buyer_is_player = state
        .businesses
        .get(buyer_business_id)
        .is_some_and(|business| business.owner_dynasty_id() == player_id);
    if !seller_is_player && unit_price < price_bounds.minimum_seller_price {
        return false;
    }
    if !buyer_is_player && unit_price > price_bounds.maximum_buyer_price {
        return false;
    }
    let Some(capacity) = available_supply_contract_capacity(
        registry,
        state,
        buyer_business_id,
        seller_business_id,
        good_id,
    ) else {
        return false;
    };
    if !seller_is_player && quantity_per_week > capacity.seller {
        return false;
    }
    if !buyer_is_player && quantity_per_week > capacity.buyer {
        return false;
    }
    true
}

pub(crate) fn generate_finance_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    add_borrow_candidate(registry, state, persona, candidates);
    add_lend_candidate(registry, state, persona, candidates);
    add_property_liquidation_candidates(registry, state, persona, candidates);
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    // Property is one asset-building route among several: strong enough to be
    // chosen when capital idles, never so dominant that every house becomes a
    // landlord. Each investment property already held thins the appeal of the
    // next purchase so portfolios grow deliberately, not automatically.
    let owned_properties = state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id == Some(state.player_dynasty_id))
        .count();
    let portfolio_steps = owned_properties.saturating_sub(2).min(4);
    let portfolio_satiation =
        Money::from_copper(i64::try_from(portfolio_steps).unwrap_or(0) * 45_000);
    let property_bonus: i64 = match persona {
        GameplayPersona::Entrepreneur => 1_200,
        GameplayPersona::Opportunist => 1_050,
        GameplayPersona::PowerBroker => 850,
        GameplayPersona::Steward => 700,
    };
    let minimum_property_yield_basis_points = match persona {
        GameplayPersona::Entrepreneur => 900,
        GameplayPersona::Opportunist => 1_000,
        GameplayPersona::PowerBroker | GameplayPersona::Steward => 1_050,
    };
    let affordability_cap = treasury.saturating_sub(portfolio_satiation).saturating_sub(
        property_purchase_liquidity_floor(state, state.player_dynasty_id),
    );
    let mut properties: Vec<_> = state
        .properties
        .values()
        .filter(|property| {
            property.owner_dynasty_id.is_none()
                && property.value <= affordability_cap
                && property_meets_investment_hurdle(
                    state,
                    property,
                    minimum_property_yield_basis_points,
                )
        })
        .collect();
    properties.sort_by_key(|property| {
        (
            std::cmp::Reverse(effective_property_annual_rent(state, property)),
            property.value,
            property.id,
        )
    });
    for property in properties.into_iter().take(4) {
        let district = registry
            .get_district(property.district_id)
            .expect("property district must exist");
        let rent_index = state
            .districts
            .get(&property.district_id)
            .expect("property district runtime must exist")
            .rent_index_basis_points;
        let effective_rent = crate::systems::effective_property_weekly_rent(state, property);
        let annual_rent = effective_property_annual_rent(state, property);
        let minimum_annual_return = property
            .value
            .saturating_mul_ratio(i64::from(minimum_property_yield_basis_points), 10_000);
        let yield_bonus = annual_rent
            .saturating_sub(minimum_annual_return)
            .copper()
            .saturating_div(10);
        push_candidate(
            candidates,
            GameplayCommandKind::BuyProperty,
            PlayerCommand::BuyProperty {
                property_id: property.id,
            },
            format!(
                "buy {:?} property {} in {} for {}; effective rent {effective_rent}/week at rent index {rent_index}",
                property.kind,
                property.id,
                district.name(),
                property.value,
            ),
            property_bonus
                .saturating_sub(
                    i64::try_from(portfolio_steps)
                        .unwrap_or(0)
                        .saturating_mul(350),
                )
                .saturating_add(yield_bonus),
        );
    }
}

pub(crate) fn effective_property_annual_rent(
    state: &AppState,
    property: &crate::core::Property,
) -> Money {
    crate::systems::effective_property_weekly_rent(state, property).saturating_mul(52)
}

/// Near-term liquidity the dynasty must keep accessible after acquiring
/// property: an emergency reserve, two months of office duties, two months of
/// loan service, and two months of family upkeep. Property purchases should not
/// decapitalize the household.
pub(crate) fn property_purchase_liquidity_floor(state: &AppState, dynasty_id: DynastyId) -> Money {
    let emergency_reserve = Money::from_copper(2_000);
    let two_month_office_duty =
        projected_dynasty_monthly_office_duty(state, dynasty_id, 0).saturating_mul(2);
    let two_month_loan_service = state
        .loans
        .values()
        .filter(|loan| loan.borrower_dynasty_id == dynasty_id && loan.status.is_repayment_active())
        .fold(Money::ZERO, |total, loan| {
            total.saturating_add(loan.weekly_payment.saturating_mul(8))
        });
    let family_members = state
        .family_councils
        .get(&dynasty_id)
        .map_or(0, |council| council.members.len());
    let two_month_family_upkeep = FAMILY_MAINTENANCE_MONTHLY_COPPER
        .saturating_mul(i64::try_from(family_members).unwrap_or(0))
        .saturating_mul(2);
    emergency_reserve
        .saturating_add(two_month_office_duty)
        .saturating_add(two_month_loan_service)
        .saturating_add(Money::from_copper(two_month_family_upkeep))
        // A purchase must leave enough working capital to keep acting on the
        // political economy rather than forcing a long liquidity recovery.
        .saturating_add(AGENT_OFFICE_LIQUIDITY_BUFFER)
}

pub(crate) fn property_meets_investment_hurdle(
    state: &AppState,
    property: &crate::core::Property,
    minimum_yield_basis_points: u16,
) -> bool {
    let minimum_annual_return = property
        .value
        .saturating_mul_ratio(i64::from(minimum_yield_basis_points), 10_000);
    effective_property_annual_rent(state, property) >= minimum_annual_return
}

pub(crate) fn add_property_liquidation_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player_id = state.player_dynasty_id;
    let distress_liquidation = player_needs_property_liquidation(state);
    if !distress_liquidation && !player_holds_underperforming_property(state) {
        return;
    }
    let force_repositioning = !distress_liquidation;
    let mut properties: Vec<_> = state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id == Some(player_id))
        .filter(|property| {
            distress_liquidation || property_underperforms_investment_hurdle(state, property)
        })
        .collect();
    properties.sort_by_key(|property| {
        (
            property.occupant_business_id.is_some(),
            std::cmp::Reverse(effective_property_annual_rent(state, property)),
            property.value,
            property.id,
        )
    });
    let buyers: Vec<_> = state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != player_id)
        .collect();
    let persona_bonus = if force_repositioning {
        1_800
    } else {
        match persona {
            GameplayPersona::Steward => 6_000,
            GameplayPersona::Entrepreneur => 5_600,
            GameplayPersona::PowerBroker => 5_200,
            GameplayPersona::Opportunist => 6_400,
        }
    };
    for property in properties.into_iter().take(2) {
        let buyer = buyers
            .iter()
            .filter_map(|buyer| {
                accepted_property_liquidation_quote(registry, state, buyer.id(), property.id)
                    .map(|quote| (*buyer, quote))
            })
            .max_by_key(|(buyer, quote)| (quote.buyer_contribution, buyer.treasury(), buyer.id()));
        let Some((buyer, quote)) = buyer else {
            continue;
        };
        let reposition =
            force_repositioning && property_underperforms_investment_hurdle(state, property);
        push_candidate(
            candidates,
            GameplayCommandKind::SellProperty,
            PlayerCommand::SellProperty {
                property_id: property.id,
                buyer_dynasty_id: buyer.id(),
            },
            if reposition {
                format!(
                    "reposition underperforming {:?} property {} in {} to {} for {} net {}",
                    property.kind,
                    property.id,
                    district_label(registry, property.district_id()),
                    dynasty_label(state, buyer.id()),
                    quote.price,
                    quote.seller_proceeds
                )
            } else {
                format!(
                    "liquidate {:?} property {} in {} to {} for {} net {}; lien payoff {}; civic guarantee {}",
                    property.kind,
                    property.id,
                    district_label(registry, property.district_id()),
                    dynasty_label(state, buyer.id()),
                    quote.price,
                    quote.seller_proceeds,
                    quote.lien_payoff,
                    quote.civic_guarantee
                )
            },
            persona_bonus,
        );
    }
}

/// Whether the dynasty holds a property whose effective annual yield is below
/// the minimum hurdle the agent requires before buying property, meaning the
/// capital would earn a better return elsewhere (a repositioning opportunity).
pub(crate) fn player_holds_underperforming_property(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    state.properties.values().any(|property| {
        property.owner_dynasty_id == Some(player_id)
            && property_underperforms_investment_hurdle(state, property)
    })
}

pub(crate) fn property_underperforms_investment_hurdle(
    state: &AppState,
    property: &crate::core::Property,
) -> bool {
    // The dynasty residence serves the family rather than rental income; it is
    // not treated as an investment to reposition. (Distress liquidation uses a
    // separate path and may still shed it as a last resort.)
    if property.kind == crate::core::PropertyKind::Residence {
        return false;
    }
    let minimum_annual_return = property.value.saturating_mul_ratio(
        i64::from(PROPERTY_PORTFOLIO_REPOSITIONING_YIELD_BASIS_POINTS),
        10_000,
    );
    effective_property_annual_rent(state, property) < minimum_annual_return
}

pub(crate) fn player_needs_property_liquidation(state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    let player = state
        .dynasties
        .get(&player_id)
        .expect("player dynasty must exist");
    let emergency_reserve = Money::from_copper(2_000);
    let two_month_office_duty =
        projected_dynasty_monthly_office_duty(state, player_id, 0).saturating_mul(2);
    let two_month_loan_service = state
        .loans
        .values()
        .filter(|loan| loan.borrower_dynasty_id == player_id && loan.status.is_repayment_active())
        .fold(Money::ZERO, |total, loan| {
            total.saturating_add(loan.weekly_payment.saturating_mul(8))
        });
    let liquidity_floor = emergency_reserve
        .saturating_add(two_month_office_duty)
        .saturating_add(two_month_loan_service);
    if player.treasury() >= liquidity_floor {
        return false;
    }
    let business_rescue_needed = state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == player_id
            && (matches!(
                business.status(),
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            ) || business.cash() == Money::ZERO
                || business.operations.condition_basis_points < 2_000)
    });
    let owned_properties = state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id == Some(player_id))
        .count();
    let committed_financial_pressure =
        two_month_office_duty > Money::ZERO || two_month_loan_service > Money::ZERO;
    business_rescue_needed
        || owned_properties >= 2
        || (owned_properties > 0 && committed_financial_pressure)
}

pub(crate) fn accepted_property_liquidation_quote(
    registry: &Registry,
    state: &AppState,
    buyer_dynasty_id: DynastyId,
    property_id: crate::ids::PropertyId,
) -> Option<crate::systems::PropertyLiquidationQuote> {
    let quote = quote_property_liquidation(
        registry,
        state,
        state.player_dynasty_id,
        buyer_dynasty_id,
        property_id,
    )
    .ok()?;
    let buyer = state.dynasties.get(&buyer_dynasty_id)?;
    let buyer_after = buyer.treasury().checked_sub(quote.buyer_contribution)?;
    // Mirror the canonical sale validation: the discretionary counterparty
    // reserve does not apply to a civic-guaranteed auction, where the buyer
    // commits their entire treasury by construction.
    if quote.civic_guarantee == Money::ZERO && buyer_after < PROPERTY_COUNTERPARTY_BUYER_RESERVE {
        return None;
    }
    Some(quote)
}

pub(crate) fn has_property_liquidation_opportunity(registry: &Registry, state: &AppState) -> bool {
    let player_id = state.player_dynasty_id;
    // The canonical route (`apply_property_sale`) accepts the sale of any
    // player-owned property to a distinct dynasty whose post-purchase treasury
    // keeps the buyer reserve. The agent's distress/repositioning policy lives
    // in the generator; the activation predicate mirrors the game so an owned
    // property with a solvent buyer is never misread as dormant.
    state
        .properties
        .values()
        .filter(|property| property.owner_dynasty_id == Some(player_id))
        .any(|property| {
            state
                .dynasties
                .keys()
                .copied()
                .filter(|dynasty_id| *dynasty_id != player_id)
                .any(|buyer_dynasty_id| {
                    accepted_property_liquidation_quote(
                        registry,
                        state,
                        buyer_dynasty_id,
                        property.id,
                    )
                    .is_some()
                })
        })
}

#[allow(clippy::too_many_lines)]
pub(crate) fn add_borrow_candidate(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player_id = state.player_dynasty_id;
    let player = state
        .dynasties
        .get(&player_id)
        .expect("player dynasty must exist");
    let base_borrowing_trigger = match persona {
        GameplayPersona::Steward => Money::from_copper(4_000),
        GameplayPersona::Entrepreneur => Money::from_copper(12_000),
        GameplayPersona::PowerBroker | GameplayPersona::Opportunist => Money::from_copper(8_000),
    };
    let office_reserve = player_office_duty_reserve(state, 0);
    let legal_requirement = active_legal_settlement_requirement(state).unwrap_or(Money::ZERO);
    let legal_funding_target = legal_settlement_funding_target(state).unwrap_or(Money::ZERO);
    let borrowing_trigger = office_reserve
        .max(base_borrowing_trigger)
        .max(legal_requirement);
    // No global "any active loan blocks borrowing" — the canonical
    // `ExistingUnsettledLoan` is per lender/borrower pair. A borrower with
    // an active loan from lender A can still approach lender B. The
    // per-lender availability is checked via `credit_pair_blocks_new_loan`
    // in the fresh-lender scan below.

    // Defaults are recovery obligations, not a signal to shop the debt around
    // the city. While any default remains unresolved, borrowing is restricted
    // to an aged workout with the creditor that already owns the claim.
    // Pair-scoped blocking mirrors `has_borrow_opportunity`: a fresh loan is
    // still possible with lenders not owed the default, so we only block when
    // every viable lender is counterparty-blocked.
    let restructuring_default = state
        .loans
        .values()
        .filter(|loan| {
            loan.borrower_dynasty_id == player_id
                && defaulted_loan_restructuring_available(state, loan)
        })
        .min_by_key(|loan| (loan.next_due_day, loan.id));
    if restructuring_default.is_none() {
        let fresh_lender_exists = state.dynasties.values().any(|dynasty| {
            dynasty.id() != player_id
                && !credit_pair_blocks_new_loan(state, dynasty.id(), player_id)
                && unresolved_default_owed_elsewhere(state, player_id, dynasty.id()).is_none()
                && dynasty
                    .treasury()
                    .checked_sub(PRIVATE_LOAN_COUNTERPARTY_RESERVE)
                    .is_some_and(|available| available >= Money::from_copper(1_000))
        });
        let acquisition_borrow_need = has_acquisition_borrow_need(registry, state, persona);
        if !fresh_lender_exists
            || (player.treasury() >= borrowing_trigger && !acquisition_borrow_need)
        {
            return;
        }
    }
    let (lender, defaulted_loan) = if let Some(defaulted_loan) = restructuring_default {
        (
            state
                .dynasties
                .get(&defaulted_loan.lender_dynasty_id)
                .expect("defaulted loan lender must exist"),
            Some(defaulted_loan),
        )
    } else {
        let Some(lender) = state
            .dynasties
            .values()
            .filter(|dynasty| dynasty.id() != player_id)
            .filter(|dynasty| !credit_pair_blocks_new_loan(state, dynasty.id(), player_id))
            .filter(|dynasty| {
                unresolved_default_owed_elsewhere(state, player_id, dynasty.id()).is_none()
            })
            .filter(|dynasty| {
                dynasty
                    .treasury()
                    .checked_sub(PRIVATE_LOAN_COUNTERPARTY_RESERVE)
                    .is_some_and(|available| available >= Money::from_copper(1_000))
            })
            .max_by_key(|dynasty| dynasty.treasury())
        else {
            return;
        };
        (lender, None)
    };
    let legal_shortfall = legal_funding_target.saturating_sub(player.treasury());
    let lender_available = lender
        .treasury()
        .checked_sub(PRIVATE_LOAN_COUNTERPARTY_RESERVE)
        .unwrap_or(Money::ZERO);
    // A workout changes terms without forcing the distressed house to take
    // another advance. Fresh borrowing still preserves the lender's reserve.
    let principal = if defaulted_loan.is_some() {
        Money::ZERO
    } else {
        borrow_principal(lender.treasury())
            .max(legal_shortfall.min(lender_available))
            .min(lender_available)
    };
    let bonus = base_bonus(
        persona,
        defaulted_loan.is_some(),
        legal_shortfall > Money::ZERO,
    );
    push_candidate(
        candidates,
        GameplayCommandKind::BorrowFunds,
        PlayerCommand::IssueLoan {
            terms: LoanTerms {
                lender_dynasty_id: lender.id(),
                borrower_dynasty_id: player_id,
                principal,
                weekly_payment: restructure_payment_balance(defaulted_loan, principal)
                    .ceil_div_positive(borrow_amortization_weeks(defaulted_loan.is_some())),
                interest_basis_points: if defaulted_loan.is_some() { 1_000 } else { 700 },
                collateral_property_id: if defaulted_loan.is_some() {
                    None
                } else {
                    unpledged_player_property(state).map(|property| property.id)
                },
            },
        },
        defaulted_loan.map_or_else(
            || {
                format!(
                    "borrow {principal} from {}",
                    dynasty_label(state, lender.id())
                )
            },
            |loan| {
                format!(
                    "restructure defaulted loan {} on revised terms with {}",
                    loan.id,
                    dynasty_label(state, lender.id())
                )
            },
        ),
        bonus,
    );
}

/// Payment base for either a fresh advance or an existing default plus any
/// optional recovery advance negotiated as part of its restructuring.
pub(crate) fn restructure_payment_balance(
    defaulted_loan: Option<&crate::core::Loan>,
    principal: Money,
) -> Money {
    defaulted_loan.map_or(principal, |loan| loan.balance.saturating_add(principal))
}

pub(crate) const fn borrow_amortization_weeks(restructuring_default: bool) -> i64 {
    if restructuring_default {
        AGENT_LOAN_AMORTIZATION_WEEKS.saturating_mul(2)
    } else {
        AGENT_LOAN_AMORTIZATION_WEEKS
    }
}

pub(crate) fn base_bonus(persona: GameplayPersona, restructuring: bool, legal_need: bool) -> i64 {
    let base: i64 = match persona {
        GameplayPersona::Opportunist => 520,
        GameplayPersona::Entrepreneur => 380,
        GameplayPersona::Steward => 80,
        GameplayPersona::PowerBroker => 120,
    };
    base.saturating_add(if restructuring { 1_800 } else { 0 })
        .saturating_add(if legal_need { 2_000 } else { 0 })
}

pub(crate) fn unpledged_player_property(state: &AppState) -> Option<&crate::core::Property> {
    state.properties.values().find(|property| {
        property.owner_dynasty_id == Some(state.player_dynasty_id)
            && property.collateral_loan_id.is_none()
    })
}

pub(crate) fn has_acquisition_borrow_need(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
) -> bool {
    if !matches!(
        persona,
        GameplayPersona::Entrepreneur | GameplayPersona::Opportunist
    ) {
        return false;
    }
    let player_id = state.player_dynasty_id;
    let treasury = state
        .dynasties
        .get(&player_id)
        .expect("player dynasty must exist")
        .treasury();
    let owned = state
        .businesses
        .ids_for_owner(player_id)
        .into_iter()
        .flatten()
        .filter_map(|id| state.businesses.get(*id))
        .collect::<Vec<_>>();
    // If portfolio already at limit, no expansion borrowing needed.
    let portfolio_limit = match persona {
        GameplayPersona::Entrepreneur | GameplayPersona::Opportunist => 3,
        _ => 2,
    };
    if owned.len() >= portfolio_limit {
        return false;
    }
    if !portfolio_ready_for_acquisition(state, &owned) {
        return false;
    }
    // Check if there's any acquirable business whose cost exceeds treasury
    // but would be reachable with a modest loan (up to 12k principal).
    state.businesses.iter().any(|business| {
        if business.owner_dynasty_id() == player_id {
            return false;
        }
        let Ok(quote) = quote_business_acquisition(registry, state, player_id, business.id())
        else {
            return false;
        };
        let Some(recapitalization) = acquisition_recapitalization(registry, state, business, quote)
        else {
            return false;
        };
        let required = quote.purchase_price.saturating_add(recapitalization);
        required > treasury && required <= treasury.saturating_add(Money::from_copper(12_000))
    })
}

/// Fresh ordinary advances scale to the lender's available treasury.
pub(crate) fn borrow_principal(lender_treasury: Money) -> Money {
    Money::from_copper((lender_treasury.copper() / 8).clamp(1_000, 12_000))
}

pub(crate) fn lending_limits(persona: GameplayPersona) -> (Money, usize) {
    match persona {
        GameplayPersona::Steward => (Money::from_copper(20_000), 2),
        GameplayPersona::Entrepreneur => (Money::from_copper(18_000), 2),
        GameplayPersona::PowerBroker => (Money::from_copper(25_000), 2),
        // Opportunist lending is the persona's signature route: a smaller
        // reserve keeps high-yield short-term credit reachable at ordinary
        // dynasty treasuries instead of reserving it for rare surpluses, even
        // in a world where crises and speculative defaults keep rival ledgers
        // lean.
        GameplayPersona::Opportunist => (Money::from_copper(10_000), 2),
    }
}

pub(crate) fn active_player_lending(state: &AppState) -> usize {
    state
        .loans
        .values()
        .filter(|loan| {
            loan.lender_dynasty_id == state.player_dynasty_id && loan.status.is_repayment_active()
        })
        .count()
}

pub(crate) fn eligible_lending_borrower<'a>(
    registry: &Registry,
    state: &'a AppState,
) -> Option<&'a crate::core::Dynasty> {
    let eligible: Vec<_> = state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .filter(|dynasty| borrower_has_productive_financing_need(registry, state, dynasty.id()))
        .filter(|dynasty| {
            !credit_pair_blocks_new_loan(state, state.player_dynasty_id, dynasty.id())
        })
        .collect();
    eligible
        .iter()
        .copied()
        .filter(|dynasty| lending_pressure(state, dynasty.id()) > 0)
        .min_by_key(|dynasty| {
            (
                std::cmp::Reverse(lending_pressure(state, dynasty.id())),
                dynasty.treasury(),
                dynasty.id(),
            )
        })
}

pub(crate) fn lending_pressure(state: &AppState, dynasty_id: DynastyId) -> u8 {
    private_loan_borrower_financing_pressure(state, dynasty_id)
}

/// Player-issued credit is a strategic investment, not a generic way to move
/// money between dynasty treasuries. Keep the agent's lending route tied to a
/// business with an identifiable working-capital problem; the command system
/// then deploys the accepted financing package into that business.
pub(crate) fn borrower_has_productive_financing_need(
    registry: &Registry,
    state: &AppState,
    dynasty_id: DynastyId,
) -> bool {
    state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == dynasty_id
            && business.cash() < business_recapitalization_target(registry, state, business)
    })
}

pub(crate) fn eligible_lending_restructuring_borrower(
    state: &AppState,
) -> Option<&crate::core::Dynasty> {
    state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .filter(|dynasty| {
            latest_defaulted_loan_for_pair(state, state.player_dynasty_id, dynasty.id())
                .is_some_and(|loan| defaulted_loan_restructuring_available(state, loan))
        })
        .min_by_key(|dynasty| dynasty.treasury())
}

pub(crate) fn has_extend_credit_opportunity(registry: &Registry, state: &AppState) -> bool {
    // Mirror the canonical routes (`IssueLoan` with the player as lender): a
    // fresh loan to an eligible counterparty, or a restructuring offer to the
    // dynasty's own defaulted borrower. Lending reserves, portfolio limits,
    // and persona risk appetite are agent policy and stay in the generator.
    if eligible_lending_restructuring_borrower(state).is_some() {
        return true;
    }
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    player.treasury() >= Money::from_copper(1_000)
        && eligible_lending_borrower(registry, state).is_some()
}

pub(crate) fn add_lend_candidate(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let (lending_reserve, lending_limit) = lending_limits(persona);
    let restructuring_borrower = eligible_lending_restructuring_borrower(state);
    let borrower = if let Some(borrower) = restructuring_borrower {
        borrower
    } else {
        if player.treasury() < lending_reserve || active_player_lending(state) >= lending_limit {
            return;
        }
        let Some(borrower) = eligible_lending_borrower(registry, state) else {
            return;
        };
        borrower
    };
    let defaulted_loan =
        latest_defaulted_loan_for_pair(state, state.player_dynasty_id, borrower.id());
    let opportunistic_new_credit =
        defaulted_loan.is_none() && persona == GameplayPersona::Opportunist;
    let borrower_pressure = lending_pressure(state, borrower.id());
    let principal = if defaulted_loan.is_some() {
        Money::ZERO
    } else if opportunistic_new_credit && borrower_pressure >= 2 {
        Money::from_copper((player.treasury().copper() / 6).clamp(5_000, 20_000))
    } else if opportunistic_new_credit {
        Money::from_copper((player.treasury().copper() / 8).clamp(1_500, 10_000))
    } else {
        Money::from_copper((player.treasury().copper() / 10).clamp(1_000, 8_000))
    };
    let repayment_balance =
        defaulted_loan.map_or(principal, |loan| loan.balance.saturating_add(principal));
    let amortization_weeks = if defaulted_loan.is_some() {
        AGENT_LOAN_AMORTIZATION_WEEKS.saturating_mul(2)
    } else if opportunistic_new_credit && borrower_pressure >= 2 {
        AGENT_OPPORTUNIST_STRESSED_LOAN_AMORTIZATION_WEEKS
    } else if opportunistic_new_credit {
        AGENT_OPPORTUNIST_LOAN_AMORTIZATION_WEEKS
    } else {
        AGENT_LOAN_AMORTIZATION_WEEKS
    };
    let interest_basis_points = if defaulted_loan.is_some() {
        1_100
    } else if opportunistic_new_credit {
        AGENT_OPPORTUNIST_LOAN_INTEREST_BASIS_POINTS
    } else {
        900
    };
    let collateral = state.properties.values().find(|property| {
        property.owner_dynasty_id == Some(borrower.id())
            && property.collateral_loan_id.is_none()
            && repayment_balance >= property.value.saturating_mul_ratio(1, 5)
    });
    let base_bonus: i64 = match persona {
        GameplayPersona::PowerBroker => 430,
        GameplayPersona::Entrepreneur => 300,
        GameplayPersona::Opportunist => 520,
        GameplayPersona::Steward => 100,
    };
    let bonus = base_bonus.saturating_add(if defaulted_loan.is_some() { 1_400 } else { 0 });
    push_candidate(
        candidates,
        GameplayCommandKind::ExtendCredit,
        PlayerCommand::IssueLoan {
            terms: LoanTerms {
                lender_dynasty_id: state.player_dynasty_id,
                borrower_dynasty_id: borrower.id(),
                principal,
                weekly_payment: repayment_balance.ceil_div_positive(amortization_weeks),
                interest_basis_points,
                collateral_property_id: collateral.map(|property| property.id),
            },
        },
        defaulted_loan.map_or_else(
            || {
                if opportunistic_new_credit {
                    format!(
                        "offer a high-yield short-term loan of {principal} to {}",
                        dynasty_label(state, borrower.id())
                    )
                } else {
                    format!(
                        "lend {principal} to {}",
                        dynasty_label(state, borrower.id())
                    )
                }
            },
            |loan| {
                format!(
                    "restructure defaulted loan {} on revised terms for {}",
                    loan.id,
                    dynasty_label(state, borrower.id())
                )
            },
        ),
        bonus,
    );
}

/// Generates leverage candidates for commissioned reports whose subject is still
/// material. Returns whether at least one leverage option was offered; when it was,
/// the agent holds the report instead of commissioning a fresh one.
pub(crate) fn generate_information_leverage_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) -> bool {
    let mut leverage_available = false;
    for report in state.information_reports.values().filter(|report| {
        report.owner_dynasty_id == state.player_dynasty_id
            && report.source == COMMISSIONED_INFORMATION_SOURCE
            && state.clock.day()
                >= report
                    .created_day
                    .saturating_add(AGENT_INFORMATION_LEVERAGE_DELAY_DAYS)
    }) {
        // Leverage is a response to a still-material situation, not an automatic
        // follow-up for every report. If the commissioned subject has resolved --
        // the market moved back or the counterparty relationship repaired -- the
        // agent holds the report rather than spending 600 cr to act on stale
        // intelligence.
        if !commissioned_report_still_material(state, report) {
            continue;
        }
        let Ok(quote) = quote_information_leverage(registry, state, report.id()) else {
            continue;
        };
        leverage_available = true;
        // Leverage must compete with crisis response (steward crisis 900) so the
        // intelligence loop is actually exercised rather than permanently
        // crowded out by urgent crises. A small recency bonus favours fresh
        // reports without overriding standing reserves.
        let recency_bonus = 180_i64
            .saturating_sub(
                state
                    .clock
                    .day()
                    .saturating_sub(report.created_day)
                    .saturating_div(4),
            )
            .max(0);
        let base_bonus: i64 = match persona {
            GameplayPersona::Steward => 980,
            GameplayPersona::Entrepreneur => 1_060,
            GameplayPersona::PowerBroker => 1_100,
            GameplayPersona::Opportunist => 1_120,
        };
        let bonus = base_bonus.saturating_add(recency_bonus);
        push_candidate(
            candidates,
            GameplayCommandKind::LeverageInformation,
            PlayerCommand::LeverageInformation {
                report_id: quote.report_id,
            },
            quote.description,
            bonus,
        );
    }
    leverage_available
}

pub(crate) fn generate_information_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    if generate_information_leverage_candidates(registry, state, persona, candidates) {
        return;
    }
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    if player.treasury() < INFORMATION_COMMISSION_COST {
        return;
    }
    let report_commission_day = state
        .information_reports
        .values()
        .filter(|report| {
            report.owner_dynasty_id == state.player_dynasty_id
                && report.source == COMMISSIONED_INFORMATION_SOURCE
        })
        .map(|report| report.created_day)
        .max();
    let audit_subject = format!("dynasty:{}", state.player_dynasty_id);
    let audit_commission_day = state
        .audit_log
        .iter()
        .filter(|record| {
            record.kind() == AuditKind::InformationCommission && record.subject() == audit_subject
        })
        .map(AuditRecord::day)
        .max();
    // The canonical game floor is `INFORMATION_COMMISSION_INTERVAL_DAYS`. Routine
    // commissions respond to sustained material uncertainty, so the agent paces
    // them at two years unless severe counterparty pressure or material political
    // strain exists -- that acceleration is the agent's explicit response to
    // exposure, mirroring the design thesis that intelligence is strategic, not
    // scheduled maintenance. The activation predicate still mirrors the game
    // floor, so a calm campaign is never misread as dormant because the agent
    // chose not to commission.
    let severe_counterparty_pressure = information_commission_has_severe_pressure(state, persona);
    let agent_commission_interval = if severe_counterparty_pressure {
        INFORMATION_COMMISSION_INTERVAL_DAYS
    } else {
        AGENT_ROUTINE_COMMISSION_INTERVAL_DAYS
    };
    let available = report_commission_day
        .max(audit_commission_day)
        .is_none_or(|day| state.clock.day() >= day.saturating_add(agent_commission_interval));
    if !available {
        return;
    }
    let Some((focus, description)) = preferred_information_focus(registry, state, persona) else {
        return;
    };
    let bonus: i64 = match persona {
        GameplayPersona::Steward => 420,
        GameplayPersona::Entrepreneur => 520,
        GameplayPersona::PowerBroker => 560,
        GameplayPersona::Opportunist => 480,
    };
    push_candidate(
        candidates,
        GameplayCommandKind::CommissionInformation,
        PlayerCommand::CommissionInformation { focus },
        description,
        bonus,
    );
}

/// Whether the agent should accelerate commission cadence because the counterparty
/// situation is materially exposed: contract-relationship pressure above the severe
/// threshold, or for the political personas a strained relationship with a rival house.
pub(crate) fn information_commission_has_severe_pressure(
    state: &AppState,
    persona: GameplayPersona,
) -> bool {
    let player_id = state.player_dynasty_id;
    let contract_pressure =
        maximum_player_contract_relationship_pressure_basis_points(state, player_id)
            >= AGENT_INFORMATION_SEVERE_COUNTERPARTY_PRESSURE_BASIS_POINTS;
    if contract_pressure {
        return true;
    }
    matches!(
        persona,
        GameplayPersona::PowerBroker | GameplayPersona::Opportunist
    ) && state.relationships.values().any(|relationship| {
        (relationship.pair.first == player_id || relationship.pair.second == player_id)
            && (relationship.trust_basis_points <= AGENT_INFORMATION_COUNTERPARTY_TRUST_THRESHOLD
                || relationship.resentment_basis_points
                    >= AGENT_INFORMATION_COUNTERPARTY_RESENTMENT_THRESHOLD)
    })
}

/// Whether a commissioned report's subject is still material enough to act on at
/// leverage time. The agent holds or lets a report expire when the underlying
/// uncertainty has resolved, so intelligence is a response to a live situation
/// rather than an automatic two-step ritual.
pub(crate) fn commissioned_report_still_material(
    state: &AppState,
    report: &crate::core::InformationReport,
) -> bool {
    let Some(target) = report.target else {
        return false;
    };
    match target {
        crate::core::InformationTarget::Market { good_id } => {
            // A market brief is worth leveraging while the player still has a live
            // external contract on that good; the canonical leverage quote then
            // renegotiates that contract at a better price. If the contract ended,
            // the report has no remaining leverage and the canonical quote fails.
            state.contracts.values().any(|contract| {
                contract.status == ContractStatus::Active
                    && contract.good_id == good_id
                    && player_external_contract(state, contract)
            })
        }
        crate::core::InformationTarget::Counterparty { dynasty_id } => state
            .relationships
            .get(&DynastyPair::new(state.player_dynasty_id, dynasty_id))
            .is_some_and(|relationship| {
                relationship.trust_basis_points <= AGENT_INFORMATION_COUNTERPARTY_TRUST_THRESHOLD
                    || relationship.resentment_basis_points
                        >= AGENT_INFORMATION_COUNTERPARTY_RESENTMENT_THRESHOLD
            }),
        crate::core::InformationTarget::District { district_id } => {
            state.districts.get(&district_id).is_some_and(|district| {
                district_information_is_material(district)
                    && (district.employment_basis_points
                        < AGENT_INFORMATION_DISTRICT_CONDITION_THRESHOLD
                        || district.sanitation_basis_points
                            < AGENT_INFORMATION_DISTRICT_CONDITION_THRESHOLD
                        || district.safety_basis_points
                            < AGENT_INFORMATION_DISTRICT_CONDITION_THRESHOLD
                        || district.unrest_basis_points
                            >= AGENT_INFORMATION_DISTRICT_UNREST_THRESHOLD)
            })
        }
    }
}

pub(crate) fn preferred_information_focus(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
) -> Option<(InformationFocus, String)> {
    match persona {
        GameplayPersona::Entrepreneur => preferred_market_information_focus(registry, state),
        GameplayPersona::Steward => preferred_district_information_focus(registry, state),
        GameplayPersona::PowerBroker | GameplayPersona::Opportunist => {
            preferred_counterparty_information_focus(state, persona)
        }
    }
}

pub(crate) fn preferred_market_information_focus(
    registry: &Registry,
    state: &AppState,
) -> Option<(InformationFocus, String)> {
    let contract = state
        .contracts
        .values()
        .filter(|contract| player_external_contract(state, contract))
        .filter(|contract| market_information_is_material(state, contract))
        .max_by_key(|contract| market_information_priority(state, contract))?;
    let good = registry.get_good(contract.good_id)?;
    Some((
        InformationFocus::Market {
            good_id: contract.good_id,
        },
        format!("commission a market brief on {}", good.name()),
    ))
}

pub(crate) fn player_external_contract(
    state: &AppState,
    contract: &crate::core::SupplyContract,
) -> bool {
    if contract.status != ContractStatus::Active
        || contract.end_day < state.clock.day().saturating_add(60)
    {
        return false;
    }
    let buyer_is_player = state
        .businesses
        .get(contract.buyer_business_id)
        .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id);
    let seller_is_player = state
        .businesses
        .get(contract.seller_business_id)
        .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id);
    buyer_is_player != seller_is_player
}

pub(crate) fn market_information_priority(
    state: &AppState,
    contract: &crate::core::SupplyContract,
) -> (u64, i64, std::cmp::Reverse<crate::ids::GoodId>) {
    let (price_change, shortage) =
        state
            .market
            .quotes
            .get(&contract.good_id)
            .map_or((0, 0), |quote| {
                (
                    quote
                        .price
                        .copper()
                        .saturating_sub(quote.previous_price.copper())
                        .unsigned_abs(),
                    quote
                        .target_stock
                        .milliunits()
                        .saturating_sub(quote.stock.milliunits())
                        .max(0),
                )
            });
    (price_change, shortage, std::cmp::Reverse(contract.good_id))
}

pub(crate) fn market_information_is_material(
    state: &AppState,
    contract: &crate::core::SupplyContract,
) -> bool {
    state
        .market
        .quotes
        .get(&contract.good_id)
        .is_some_and(|quote| {
            let previous_price = quote.previous_price.copper().max(1).unsigned_abs();
            let price_change = quote
                .price
                .copper()
                .saturating_sub(quote.previous_price.copper())
                .unsigned_abs();
            let price_change_basis_points = scaled_ratio_u64(price_change, previous_price, 10_000);
            let target_stock = quote.target_stock.milliunits().max(1).unsigned_abs();
            let shortage = quote
                .target_stock
                .milliunits()
                .saturating_sub(quote.stock.milliunits())
                .max(0)
                .unsigned_abs();
            let shortage_basis_points = scaled_ratio_u64(shortage, target_stock, 10_000);
            let buyer_is_player = state
                .businesses
                .get(contract.buyer_business_id)
                .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id);
            let seller_is_player = state
                .businesses
                .get(contract.seller_business_id)
                .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id);
            let current_market_price = quote.price.copper().max(1);
            let adverse_contract_gap = if buyer_is_player && !seller_is_player {
                contract
                    .unit_price
                    .copper()
                    .saturating_sub(current_market_price)
            } else if seller_is_player && !buyer_is_player {
                current_market_price.saturating_sub(contract.unit_price.copper())
            } else {
                0
            }
            .max(0)
            .unsigned_abs();
            let adverse_contract_gap_basis_points = scaled_ratio_u64(
                adverse_contract_gap,
                current_market_price.unsigned_abs(),
                10_000,
            );
            price_change_basis_points >= AGENT_INFORMATION_MARKET_PRICE_CHANGE_BASIS_POINTS
                || shortage_basis_points >= AGENT_INFORMATION_MARKET_SHORTAGE_BASIS_POINTS
                || adverse_contract_gap_basis_points
                    >= AGENT_INFORMATION_MARKET_CONTRACT_GAP_BASIS_POINTS
        })
}

pub(crate) fn preferred_district_information_focus(
    registry: &Registry,
    state: &AppState,
) -> Option<(InformationFocus, String)> {
    let (district_id, _) = state
        .districts
        .iter()
        .filter(|(_, district)| district_information_is_material(district))
        .max_by_key(|(district_id, district)| {
            (
                district_hardship(district),
                std::cmp::Reverse(**district_id),
            )
        })?;
    let district = registry.get_district(*district_id)?;
    Some((
        InformationFocus::District {
            district_id: *district_id,
        },
        format!("commission a district brief on {}", district.name()),
    ))
}

pub(crate) fn district_hardship(district: &crate::core::DistrictRuntime) -> u32 {
    u32::from(district.unrest_basis_points)
        .saturating_add(u32::from(
            10_000_u16.saturating_sub(district.employment_basis_points),
        ))
        .saturating_add(u32::from(
            10_000_u16.saturating_sub(district.sanitation_basis_points),
        ))
        .saturating_add(u32::from(
            10_000_u16.saturating_sub(district.safety_basis_points),
        ))
}

pub(crate) fn district_information_is_material(district: &crate::core::DistrictRuntime) -> bool {
    district.employment_basis_points < AGENT_INFORMATION_DISTRICT_CONDITION_THRESHOLD
        || district.sanitation_basis_points < AGENT_INFORMATION_DISTRICT_CONDITION_THRESHOLD
        || district.safety_basis_points < AGENT_INFORMATION_DISTRICT_CONDITION_THRESHOLD
        || district.unrest_basis_points >= AGENT_INFORMATION_DISTRICT_UNREST_THRESHOLD
}

pub(crate) fn preferred_counterparty_information_focus(
    state: &AppState,
    persona: GameplayPersona,
) -> Option<(InformationFocus, String)> {
    let relationship = state
        .relationships
        .values()
        .filter(|relationship| {
            relationship.pair.first == state.player_dynasty_id
                || relationship.pair.second == state.player_dynasty_id
        })
        .filter(|relationship| counterparty_information_is_material(state, relationship, persona))
        .max_by_key(|relationship| {
            counterparty_information_priority(state, relationship, persona)
        })?;
    let dynasty_id = relationship_counterparty_id(relationship, state.player_dynasty_id)?;
    let dynasty = state.dynasties.get(&dynasty_id)?;
    Some((
        InformationFocus::Counterparty { dynasty_id },
        format!("commission a house brief on House {}", dynasty.name()),
    ))
}

pub(crate) fn counterparty_information_is_material(
    state: &AppState,
    relationship: &crate::core::RelationshipState,
    persona: GameplayPersona,
) -> bool {
    let strained = relationship.trust_basis_points
        <= AGENT_INFORMATION_COUNTERPARTY_TRUST_THRESHOLD
        || relationship.resentment_basis_points
            >= AGENT_INFORMATION_COUNTERPARTY_RESENTMENT_THRESHOLD;
    if strained {
        return true;
    }
    let Some(counterparty_id) = relationship_counterparty_id(relationship, state.player_dynasty_id)
    else {
        return false;
    };
    match persona {
        GameplayPersona::Opportunist => {
            let player_treasury = state
                .dynasties
                .get(&state.player_dynasty_id)
                .map_or(Money::ZERO, crate::core::Dynasty::treasury);
            state
                .dynasties
                .get(&counterparty_id)
                .is_some_and(|dynasty| dynasty.treasury() >= player_treasury.saturating_mul(2))
        }
        GameplayPersona::PowerBroker => {
            power_broker_political_intelligence_is_material(state, counterparty_id)
        }
        GameplayPersona::Steward | GameplayPersona::Entrepreneur => false,
    }
}

pub(crate) fn power_broker_political_intelligence_is_material(
    state: &AppState,
    counterparty_id: DynastyId,
) -> bool {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let player_offices = count_player_offices(state, state.player_dynasty_id);
    let counterparty_offices = count_player_offices(state, counterparty_id);
    player_offices > 0
        && counterparty_offices >= player_offices
        && player.resources.legitimacy_basis_points
            < AGENT_INFORMATION_POLITICAL_VULNERABILITY_LEGITIMACY
}

pub(crate) fn counterparty_information_priority(
    state: &AppState,
    relationship: &crate::core::RelationshipState,
    persona: GameplayPersona,
) -> (u32, std::cmp::Reverse<DynastyId>) {
    let counterparty_id = relationship_counterparty_id(relationship, state.player_dynasty_id)
        .expect("filtered relationship must contain the player dynasty");
    let score = match persona {
        GameplayPersona::PowerBroker => u32::from(count_player_offices(state, counterparty_id))
            .saturating_mul(20_000)
            .saturating_add(u32::from(
                relationship
                    .resentment_basis_points
                    .saturating_add(10_000_u16.saturating_sub(relationship.trust_basis_points)),
            )),
        GameplayPersona::Opportunist => u32::try_from(
            state
                .dynasties
                .get(&counterparty_id)
                .map_or(0_i64, |dynasty| dynasty.treasury().copper())
                .max(0),
        )
        .unwrap_or(u32::MAX),
        GameplayPersona::Steward | GameplayPersona::Entrepreneur => 0,
    };
    (score, std::cmp::Reverse(counterparty_id))
}

pub(crate) fn relationship_counterparty_id(
    relationship: &crate::core::RelationshipState,
    player_id: DynastyId,
) -> Option<DynastyId> {
    if relationship.pair.first == player_id {
        Some(relationship.pair.second)
    } else if relationship.pair.second == player_id {
        Some(relationship.pair.first)
    } else {
        None
    }
}

pub(crate) fn maximum_player_contract_relationship_pressure_basis_points(
    state: &AppState,
    player_id: DynastyId,
) -> u16 {
    state
        .relationships
        .values()
        .filter_map(|relationship| {
            relationship_counterparty_id(relationship, player_id)
                .map(|dynasty_id| contract_relationship_pressure_basis_points(state, dynasty_id))
        })
        .max()
        .unwrap_or(0)
}

pub(crate) fn generate_civic_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    generate_law_candidates(registry, state, persona, candidates);
    generate_public_work_funding_candidates(registry, state, persona, candidates);
    generate_public_work_candidates(registry, state, persona, candidates);
    generate_legal_candidates(state, persona, candidates);
}

pub(crate) fn generate_public_work_funding_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let base_bonus: i64 = match persona {
        GameplayPersona::Steward => 3_200,
        GameplayPersona::PowerBroker => 2_400,
        GameplayPersona::Entrepreneur => 1_700,
        GameplayPersona::Opportunist => 1_200,
    };
    let office_reserve = player_office_duty_reserve(state, 0);
    let discretionary_surplus = treasury
        .saturating_sub(office_reserve)
        .saturating_sub(AGENT_OFFICE_LIQUIDITY_BUFFER);
    let wealthy_acceleration = treasury >= AGENT_CIVIC_ACCELERATION_TREASURY_TRIGGER
        && discretionary_surplus > Money::ZERO;
    let mut works = state
        .public_works
        .values()
        .filter(|work| {
            work.status.is_unfinished()
                && work.budget.saturating_sub(work.spent) > Money::ZERO
                // The agent funds any unfinished project only as a deliberate
                // civic act: rescuing a stalled project or accelerating one
                // from clear surplus, never routine dribble spending.
                && (work.status == PublicWorkStatus::Suspended || wealthy_acceleration)
        })
        .collect::<Vec<_>>();
    works.sort_by_key(|work| (std::cmp::Reverse(work.progress_basis_points), work.id));
    for work in works.into_iter().take(2) {
        push_public_work_funding_candidate(
            registry,
            state,
            persona,
            candidates,
            work,
            treasury,
            discretionary_surplus,
            base_bonus,
        );
    }
}

/// Builds one funding candidate for an unfinished project, applying the
/// agent's contribution policy: a stalled own rescue may spend the whole
/// treasury, while acceleration and external patronage stay bounded by the
/// discretionary surplus so civic generosity never strips the house.
#[expect(clippy::too_many_arguments)]
fn push_public_work_funding_candidate(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
    work: &crate::core::PublicWork,
    treasury: Money,
    discretionary_surplus: Money,
    base_bonus: i64,
) {
    let remaining = work.budget.saturating_sub(work.spent);
    if remaining <= Money::ZERO {
        return;
    }
    let external = work.sponsor_dynasty_id != Some(state.player_dynasty_id);
    let district_need = state
        .districts
        .get(&work.district_id)
        .map_or(0, |runtime| public_work_need_score(runtime, work.kind));
    // Patronage answers visible need: a dynasty bankrolls someone else's
    // project when its district genuinely lacks what the project delivers,
    // not as an unconditional default action. A stalled project needs less
    // provocation than accelerating a healthy rival's construction.
    let patronage_need_floor = if work.status == PublicWorkStatus::Suspended {
        STALLED_PATRONAGE_MIN_NEED_SCORE
    } else {
        EXTERNAL_PATRONAGE_MIN_NEED_SCORE
    };
    if external && district_need < patronage_need_floor {
        return;
    }
    let base_bonus = if external {
        // Patronage is civic generosity, not the house's main business: a
        // rising dynasty keeps it well below commercial investment so early
        // play is not consumed by bankrolling other houses' monuments.
        match persona {
            GameplayPersona::Steward => 800,
            GameplayPersona::PowerBroker => 650,
            GameplayPersona::Entrepreneur => 350,
            GameplayPersona::Opportunist => 250,
        }
    } else {
        base_bonus
    };
    let amount = if !external && work.status == PublicWorkStatus::Suspended {
        remaining.min(treasury)
    } else {
        // Never sink the house treasury into routine acceleration or someone
        // else's project; both are bounded by discretionary surplus.
        remaining
            .min(discretionary_surplus)
            .min(AGENT_CIVIC_ACCELERATION_MAX_CONTRIBUTION)
    };
    if amount <= Money::ZERO {
        return;
    }
    let completes = amount >= remaining;
    let stalled = work.status == PublicWorkStatus::Suspended;
    let intent = match (external, stalled, completes) {
        (true, true, true) => "finish the city's stalled",
        (true, true, false) => "rescue the city's stalled",
        (true, false, true) => "finish a rival's",
        (true, false, false) => "accelerate a rival's",
        (false, true, true) => "finish stalled",
        (false, true, false) => "rescue stalled",
        (false, false, true) => "finish",
        (false, false, false) => "accelerate",
    };
    let article = if external { "" } else { "the" };
    let external_civic_bonus: i64 = match persona {
        GameplayPersona::Steward => 260,
        GameplayPersona::PowerBroker => 180,
        GameplayPersona::Entrepreneur | GameplayPersona::Opportunist => 0,
    };
    push_candidate(
        candidates,
        GameplayCommandKind::FundPublicWork,
        PlayerCommand::FundPublicWork {
            public_work_id: work.id,
            amount,
        },
        format!(
            "fund {amount} to {intent} {article} {:?} project in {} ({})",
            work.kind,
            district_label(registry, work.district_id),
            work.id,
        ),
        base_bonus
            .saturating_add(if external { external_civic_bonus } else { 0 })
            .saturating_add(district_need / 10)
            // Progress already made makes completion tangible, but it must not
            // dominate the score on its own: near-finished projects are common,
            // and a large constant bonus once drowned out every foundation-phase
            // alternative.
            .saturating_add(i64::from(work.progress_basis_points).min(1_200) / 2)
            .saturating_add(if stalled {
                if external { 400 } else { 1_000 }
            } else {
                350
            }),
    );
}

pub(crate) fn generate_law_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    if !has_player_office(state) {
        return;
    }
    let sponsorship_available = state
        .laws
        .values()
        .filter(|law| law.sponsor_dynasty_id == Some(state.player_dynasty_id))
        .map(|law| law.enacted_day)
        .max()
        .is_none_or(|day| state.clock.day() >= day.saturating_add(LAW_SPONSORSHIP_INTERVAL_DAYS));
    let has_legitimacy = state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_some_and(|dynasty| {
            dynasty.resources.legitimacy_basis_points >= LAW_LEGITIMACY_REQUIREMENT
        });
    if !sponsorship_available || !has_legitimacy {
        return;
    }
    if state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_none_or(|dynasty| dynasty.treasury() < Money::from_copper(2_000))
    {
        return;
    }
    let law_bonus: i64 = match persona {
        GameplayPersona::PowerBroker => 560,
        GameplayPersona::Steward => 260,
        GameplayPersona::Entrepreneur => 180,
        GameplayPersona::Opportunist => 140,
    };
    for (kind, value) in law_candidates(registry, state) {
        if !has_established_player_office_power(state, required_office_power_for_law(kind)) {
            continue;
        }
        if state
            .laws
            .values()
            .any(|law| law.active && law.kind == kind && law.value == value)
        {
            continue;
        }
        let persona_bonus = law_persona_bonus(persona, kind);
        let context_bonus = law_context_relevance_bonus(state, kind);
        if persona_bonus <= 0 && context_bonus <= 0 {
            continue;
        }
        push_candidate(
            candidates,
            GameplayCommandKind::EnactLaw,
            PlayerCommand::EnactLaw { kind, value },
            format!("enact {kind:?} with value {value}"),
            law_bonus
                .saturating_add(persona_bonus)
                .saturating_add(context_bonus),
        );
    }
}

pub(crate) fn law_candidates(registry: &Registry, state: &AppState) -> Vec<(LawKind, i64)> {
    let bread_price = registry
        .get_good_id("bread")
        .and_then(|good_id| state.market.quotes.get(&good_id))
        .map_or(1, |quote| quote.price.copper())
        .max(1);
    let mut candidates = vec![
        (LawKind::BreadPriceCeiling, bread_price),
        (LawKind::ForeignMerchantToll, 600),
        (LawKind::InterestLimit, 800),
        (LawKind::FireCode, 7_000),
        (LawKind::RentRestriction, 900),
        (LawKind::GuildEntryRestriction, 1_200),
        (LawKind::EmergencyImports, 250),
    ];
    if let Some(principal) = civic_debt_candidate_principal(registry, state) {
        candidates.push((LawKind::PublicDebtAuthorization, principal.copper()));
    }
    candidates
}

pub(crate) fn civic_debt_candidate_principal(
    registry: &Registry,
    state: &AppState,
) -> Option<Money> {
    let treasury_id = registry.get_institution_id("treasury")?;
    let treasury = state.institutions.get(&treasury_id)?;
    let unsettled = state
        .civic_debts
        .values()
        .filter(|debt| debt.status != CivicDebtStatus::Repaid)
        .count();
    if treasury.budget >= Money::from_copper(50_000) || unsettled >= 2 {
        return None;
    }
    let principal = Money::from_copper(
        Money::from_copper(50_000)
            .saturating_sub(treasury.budget)
            .copper()
            .clamp(10_000, 100_000),
    );
    state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .any(|dynasty| {
            dynasty
                .treasury()
                .saturating_sub(CIVIC_DEBT_CREDITOR_RESERVE)
                >= principal
        })
        .then_some(principal)
}

pub(crate) fn law_persona_bonus(persona: GameplayPersona, kind: LawKind) -> i64 {
    match persona {
        GameplayPersona::Steward => match kind {
            LawKind::BreadPriceCeiling | LawKind::EmergencyImports => 220,
            LawKind::FireCode | LawKind::RentRestriction | LawKind::PublicDebtAuthorization => 180,
            LawKind::ForeignMerchantToll
            | LawKind::InterestLimit
            | LawKind::GuildEntryRestriction => 0,
        },
        GameplayPersona::Entrepreneur => match kind {
            LawKind::ForeignMerchantToll => 180,
            LawKind::GuildEntryRestriction => -80,
            LawKind::BreadPriceCeiling
            | LawKind::InterestLimit
            | LawKind::FireCode
            | LawKind::RentRestriction
            | LawKind::EmergencyImports
            | LawKind::PublicDebtAuthorization => 0,
        },
        GameplayPersona::PowerBroker => match kind {
            LawKind::PublicDebtAuthorization => 360,
            LawKind::BreadPriceCeiling
            | LawKind::ForeignMerchantToll
            | LawKind::InterestLimit
            | LawKind::FireCode
            | LawKind::RentRestriction
            | LawKind::GuildEntryRestriction
            | LawKind::EmergencyImports => 120,
        },
        GameplayPersona::Opportunist => match kind {
            LawKind::InterestLimit => -100,
            LawKind::ForeignMerchantToll => 160,
            LawKind::BreadPriceCeiling
            | LawKind::FireCode
            | LawKind::RentRestriction
            | LawKind::GuildEntryRestriction
            | LawKind::EmergencyImports
            | LawKind::PublicDebtAuthorization => 0,
        },
    }
}

pub(crate) fn law_context_relevance_bonus(state: &AppState, kind: LawKind) -> i64 {
    let food_satisfaction =
        crate::core::population_weighted_food_satisfaction_basis_points(state.households.iter())
            .unwrap_or(10_000);
    match kind {
        LawKind::BreadPriceCeiling => {
            if food_satisfaction < 9_700 {
                420
            } else {
                0
            }
        }
        LawKind::ForeignMerchantToll | LawKind::GuildEntryRestriction => 0,
        LawKind::InterestLimit => {
            if state
                .loans
                .values()
                .any(|loan| matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted))
            {
                420
            } else {
                0
            }
        }
        LawKind::FireCode => {
            if state
                .districts
                .values()
                .map(|district| district.safety_basis_points)
                .min()
                .is_some_and(|safety| safety < 6_000)
            {
                360
            } else {
                0
            }
        }
        LawKind::RentRestriction => {
            if average_u16(
                state
                    .districts
                    .values()
                    .map(|district| district.rent_index_basis_points),
            ) > 11_000
            {
                320
            } else {
                0
            }
        }
        LawKind::EmergencyImports => {
            let grain_crisis = state.crises.values().any(|crisis| {
                crisis.kind == CrisisKind::GrainShortage && crisis.status.is_active()
            });
            if food_satisfaction < 9_800 || grain_crisis {
                520
            } else {
                0
            }
        }
        LawKind::PublicDebtAuthorization => 520,
    }
}

pub(crate) fn generate_public_work_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    if !has_established_player_office_power(state, OfficePower::PublicWorks) {
        return;
    }
    let active_sponsored = state
        .public_works
        .values()
        .filter(|work| {
            work.sponsor_dynasty_id == Some(state.player_dynasty_id) && work.status.is_unfinished()
        })
        .count();
    if active_sponsored >= MAX_ACTIVE_SPONSORED_PUBLIC_WORKS {
        return;
    }
    let subject = format!("dynasty:{}", state.player_dynasty_id);
    // Windowed scan: a sponsorship outside the cooldown interval cannot block.
    let sponsorship_available =
        audit_records_within_cooldown(state, PUBLIC_WORK_SPONSORSHIP_INTERVAL_DAYS).all(|record| {
            !(record.kind() == AuditKind::PublicWorkStarted && record.subject() == subject)
        });
    if !sponsorship_available {
        return;
    }
    // Candidate public works use a fixed budget; the agent must afford at
    // least the canonical initial sponsor contribution for that budget.
    if state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_none_or(|dynasty| {
            dynasty.treasury() < public_work_initial_contribution(CANDIDATE_PUBLIC_WORK_BUDGET)
        })
    {
        return;
    }
    let bonus: i64 = match persona {
        GameplayPersona::PowerBroker => 520,
        GameplayPersona::Steward => 440,
        GameplayPersona::Entrepreneur => 180,
        GameplayPersona::Opportunist => 60,
    };
    for district in registry.districts() {
        let runtime = state
            .districts
            .get(&district.id())
            .expect("district runtime must exist");
        for kind in preferred_public_work_kinds(state, runtime, persona, bonus) {
            if state.public_works.values().any(|work| {
                work.district_id == district.id()
                    && work.kind == kind
                    && work.status.is_unfinished()
            }) {
                continue;
            }
            push_candidate(
                candidates,
                GameplayCommandKind::StartPublicWork,
                PlayerCommand::StartPublicWork {
                    district_id: district.id(),
                    kind,
                    budget: CANDIDATE_PUBLIC_WORK_BUDGET,
                },
                format!(
                    "start {kind:?} in {} to {}",
                    district.name(),
                    public_work_intent(kind)
                ),
                public_work_candidate_priority(
                    bonus,
                    runtime,
                    persona,
                    kind,
                    completed_player_public_works_of_kind(state, kind),
                ),
            );
        }
    }
}

pub(crate) const PUBLIC_WORK_PORTFOLIO_REPEAT_PENALTY: i64 = 200;

pub(crate) fn completed_player_public_works_of_kind(
    state: &AppState,
    kind: PublicWorkKind,
) -> usize {
    state
        .public_works
        .values()
        .filter(|work| {
            work.sponsor_dynasty_id == Some(state.player_dynasty_id)
                && work.status == PublicWorkStatus::Completed
                && work.kind == kind
        })
        .count()
}

pub(crate) fn public_work_candidate_priority(
    base_bonus: i64,
    district: &crate::core::DistrictRuntime,
    persona: GameplayPersona,
    kind: PublicWorkKind,
    completed_same_kind: usize,
) -> i64 {
    let repeat_penalty = i64::try_from(completed_same_kind.min(4))
        .unwrap_or(4)
        .saturating_mul(PUBLIC_WORK_PORTFOLIO_REPEAT_PENALTY);
    base_bonus
        .saturating_add(public_work_persona_bonus(persona, kind))
        .saturating_add(public_work_need_score(district, kind) / 10)
        .saturating_sub(repeat_penalty)
}

pub(crate) fn preferred_public_work_kinds(
    state: &AppState,
    district: &crate::core::DistrictRuntime,
    persona: GameplayPersona,
    base_bonus: i64,
) -> [PublicWorkKind; 2] {
    let mut scored = [
        PublicWorkKind::Road,
        PublicWorkKind::Bridge,
        PublicWorkKind::Market,
        PublicWorkKind::Granary,
        PublicWorkKind::Drainage,
        PublicWorkKind::WatchStation,
        PublicWorkKind::Hospital,
        PublicWorkKind::School,
    ]
    .map(|kind| {
        (
            public_work_candidate_priority(
                base_bonus,
                district,
                persona,
                kind,
                completed_player_public_works_of_kind(state, kind),
            ),
            kind,
        )
    });
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    [scored[0].1, scored[1].1]
}

pub(crate) fn public_work_need_score(
    district: &crate::core::DistrictRuntime,
    kind: PublicWorkKind,
) -> i64 {
    let employment_need = i64::from(10_000_u16.saturating_sub(district.employment_basis_points));
    let sanitation_need = i64::from(10_000_u16.saturating_sub(district.sanitation_basis_points));
    let safety_need = i64::from(10_000_u16.saturating_sub(district.safety_basis_points));
    let unrest = i64::from(district.unrest_basis_points);
    match kind {
        PublicWorkKind::Drainage => sanitation_need,
        PublicWorkKind::Hospital => sanitation_need.saturating_mul(4) / 5 + unrest / 3,
        PublicWorkKind::WatchStation => safety_need,
        PublicWorkKind::Road => employment_need.saturating_mul(3) / 5 + safety_need / 3,
        PublicWorkKind::Bridge => employment_need.saturating_mul(3) / 5 + safety_need / 4,
        PublicWorkKind::Market => employment_need,
        PublicWorkKind::Granary => {
            employment_need / 3 + sanitation_need / 3 + unrest.saturating_mul(2) / 3
        }
        PublicWorkKind::School => employment_need / 2 + unrest,
    }
}

pub(crate) const fn public_work_persona_bonus(
    persona: GameplayPersona,
    kind: PublicWorkKind,
) -> i64 {
    match persona {
        GameplayPersona::Steward => match kind {
            PublicWorkKind::Drainage
            | PublicWorkKind::Granary
            | PublicWorkKind::Hospital
            | PublicWorkKind::School => 260,
            PublicWorkKind::Road
            | PublicWorkKind::Bridge
            | PublicWorkKind::Market
            | PublicWorkKind::WatchStation => 40,
        },
        GameplayPersona::Entrepreneur => match kind {
            PublicWorkKind::Road | PublicWorkKind::Bridge | PublicWorkKind::Market => 260,
            PublicWorkKind::Granary => 120,
            PublicWorkKind::Drainage
            | PublicWorkKind::WatchStation
            | PublicWorkKind::Hospital
            | PublicWorkKind::School => 20,
        },
        GameplayPersona::PowerBroker => match kind {
            PublicWorkKind::Road | PublicWorkKind::Market | PublicWorkKind::WatchStation => 260,
            PublicWorkKind::Bridge => 160,
            PublicWorkKind::Granary
            | PublicWorkKind::Drainage
            | PublicWorkKind::Hospital
            | PublicWorkKind::School => 20,
        },
        GameplayPersona::Opportunist => match kind {
            PublicWorkKind::Bridge | PublicWorkKind::Market | PublicWorkKind::WatchStation => 260,
            PublicWorkKind::Road => 140,
            PublicWorkKind::Granary
            | PublicWorkKind::Drainage
            | PublicWorkKind::Hospital
            | PublicWorkKind::School => 20,
        },
    }
}

pub(crate) const fn public_work_intent(kind: PublicWorkKind) -> &'static str {
    match kind {
        PublicWorkKind::Road => "expand employment and improve street safety",
        PublicWorkKind::Bridge => "expand employment and improve route safety",
        PublicWorkKind::Market => "create durable commercial employment",
        PublicWorkKind::Granary => "stabilize provisioning and create local employment",
        PublicWorkKind::Drainage => "improve sanitation",
        PublicWorkKind::WatchStation => "improve safety",
        PublicWorkKind::Hospital => "improve sanitation and social stability",
        PublicWorkKind::School => "create local employment and reduce unrest",
    }
}

pub(crate) fn generate_legal_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    if !legal_filing_is_available(state) {
        return;
    }
    let bonus = match persona {
        GameplayPersona::PowerBroker => 480,
        GameplayPersona::Opportunist => 420,
        GameplayPersona::Entrepreneur => 180,
        GameplayPersona::Steward => 80,
    };
    for claim in state
        .dynasties
        .values()
        .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
        .filter_map(|defendant| next_player_legal_claim(state, defendant.id()))
        .take(3)
    {
        if state.legal_cases.values().any(|case| {
            case.plaintiff_dynasty_id == state.player_dynasty_id
                && case.defendant_dynasty_id == claim.defendant_dynasty_id
                && case.kind == claim.kind
                && matches!(
                    case.status,
                    LegalCaseStatus::Filed | LegalCaseStatus::Hearing
                )
        }) {
            continue;
        }
        push_candidate(
            candidates,
            GameplayCommandKind::FileLegalCase,
            PlayerCommand::FileLegalCase {
                defendant_dynasty_id: claim.defendant_dynasty_id,
                kind: claim.kind,
                evidence_basis_points: claim.evidence_basis_points,
                damages: claim.maximum_damages,
            },
            format!(
                "file {:?} case against {}: {}",
                claim.kind,
                dynasty_label(state, claim.defendant_dynasty_id),
                claim.description
            ),
            bonus,
        );
    }
}

pub(crate) fn legal_filing_is_available(state: &AppState) -> bool {
    state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_some_and(|dynasty| dynasty.treasury() >= LEGAL_CASE_FILING_COST)
        && state
            .legal_cases
            .values()
            .filter(|legal_case| legal_case.plaintiff_dynasty_id == state.player_dynasty_id)
            .map(|legal_case| legal_case.filed_day)
            .max()
            .is_none_or(|last_filing_day| {
                state.clock.day() >= last_filing_day.saturating_add(LEGAL_CASE_FILING_INTERVAL_DAYS)
            })
}

pub(crate) fn has_legal_filing_opportunity(state: &AppState) -> bool {
    legal_filing_is_available(state)
        && state
            .dynasties
            .values()
            .filter(|dynasty| dynasty.id() != state.player_dynasty_id)
            .filter_map(|defendant| next_player_legal_claim(state, defendant.id()))
            .any(|claim| {
                !state.legal_cases.values().any(|case| {
                    case.plaintiff_dynasty_id == state.player_dynasty_id
                        && case.defendant_dynasty_id == claim.defendant_dynasty_id
                        && case.kind == claim.kind
                        && matches!(
                            case.status,
                            LegalCaseStatus::Filed | LegalCaseStatus::Hearing
                        )
                })
            })
}

pub(crate) fn legal_grievance_kind(
    state: &AppState,
    defendant_id: DynastyId,
) -> Option<LegalCaseKind> {
    next_player_legal_claim(state, defendant_id).map(|claim| claim.kind)
}

pub(crate) fn next_player_legal_claim(
    state: &AppState,
    defendant_id: DynastyId,
) -> Option<crate::systems::LegalClaimQuote> {
    [LegalCaseKind::Debt, LegalCaseKind::ContractBreach]
        .into_iter()
        .find_map(|kind| quote_player_legal_claim(state, defendant_id, kind).ok())
}

pub(crate) fn generate_family_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let council = state
        .family_councils
        .get(&state.player_dynasty_id)
        .expect("player family council must exist");
    let governance_subject = format!("dynasty:{}", state.player_dynasty_id);
    // Windowed scan: only a change inside the cooldown interval can block.
    let governance_available =
        audit_records_within_cooldown(state, HOUSE_GOVERNANCE_CHANGE_INTERVAL_DAYS).all(|record| {
            !(record.kind() == AuditKind::HouseGovernanceChange
                && record.subject() == governance_subject)
        });
    if governance_available
        && council.unity_basis_points >= HOUSE_GOVERNANCE_UNITY_COST
        && let Some(governance) = preferred_house_governance(state, persona)
        && governance != council.governance
    {
        push_candidate(
            candidates,
            GameplayCommandKind::SetHouseGovernance,
            PlayerCommand::SetHouseGovernance { governance },
            format!("adopt {governance:?} governance to address current family pressure"),
            governance_bonus(persona, governance),
        );
    }
    generate_family_council_candidate(state, persona, candidates);
    generate_heir_designation_candidates(state, persona, candidates);
    generate_ward_adoption_candidates(state, persona, candidates);
    generate_family_education_candidates(registry, state, persona, candidates);
    generate_institution_withdrawal_candidates(registry, state, persona, candidates);
    generate_office_power_directive_candidates(registry, state, persona, candidates);
    generate_institution_endowment_candidates(registry, state, persona, candidates);
    generate_institution_ascent_candidates(registry, state, persona, candidates);
}

pub(crate) fn generate_institution_endowment_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    if institution_endowment_next_day(state).is_some_and(|day| state.clock.day() < day) {
        return;
    }
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let office_reserve = player_office_duty_reserve(state, 0);
    let protected_floor = AGENT_ENDOWMENT_LIQUIDITY_FLOOR
        .max(office_reserve.saturating_add(AGENT_ENDOWMENT_OFFICE_BUFFER));
    let surplus = treasury.saturating_sub(protected_floor);
    if surplus < INSTITUTION_ENDOWMENT_MIN {
        return;
    }
    let amount = surplus.min(INSTITUTION_ENDOWMENT_MAX);
    let base_bonus: i64 = match persona {
        GameplayPersona::PowerBroker => 750,
        GameplayPersona::Steward => 450,
        GameplayPersona::Opportunist => 350,
        GameplayPersona::Entrepreneur => 250,
    };
    for institution in state.institutions.values().filter(|institution| {
        has_established_player_institution_membership(state, institution.institution_id)
    }) {
        let legitimacy_need =
            i64::from(10_000_u16.saturating_sub(institution.legitimacy_basis_points) / 10);
        let office_bonus = institution.office_holder_id.map_or(0, |holder_id| {
            state.characters.get(holder_id).map_or(0, |holder| {
                if holder.dynasty_id() == state.player_dynasty_id {
                    350
                } else {
                    0
                }
            })
        });
        let strategic_fit =
            institution_ascent_power_bonus(registry, state, institution, persona) / 3;
        push_candidate(
            candidates,
            GameplayCommandKind::EndowInstitution,
            PlayerCommand::EndowInstitution {
                institution_id: institution.institution_id,
                amount,
            },
            format!(
                "endow {} with {amount} to strengthen its capacity and member-house coalition",
                institution_label(registry, institution.institution_id)
            ),
            base_bonus
                .saturating_add(legitimacy_need)
                .saturating_add(office_bonus)
                .saturating_add(strategic_fit),
        );
    }
}

pub(crate) fn generate_family_council_candidate(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let dynasty_id = state.player_dynasty_id;
    let Some(council) = state.family_councils.get(&dynasty_id) else {
        return;
    };
    if council.unity_basis_points >= FAMILY_COUNCIL_INTERVENTION_UNITY_THRESHOLD
        || state
            .dynasties
            .get(&dynasty_id)
            .is_none_or(|dynasty| dynasty.treasury() < FAMILY_COUNCIL_MEETING_COST)
    {
        return;
    }
    let subject = format!("dynasty:{dynasty_id};council-meeting");
    // Windowed scan: only a meeting inside the cooldown interval can block.
    let available =
        audit_records_within_cooldown(state, FAMILY_COUNCIL_MEETING_INTERVAL_DAYS).all(|record| {
            !(record.kind() == AuditKind::FamilyCouncilMeeting && record.subject() == subject)
        });
    if !available {
        return;
    }
    let pressure_bonus = i64::from(
        FAMILY_COUNCIL_INTERVENTION_UNITY_THRESHOLD.saturating_sub(council.unity_basis_points) / 50,
    );
    let persona_bonus = match persona {
        GameplayPersona::Steward => 30,
        GameplayPersona::PowerBroker => 20,
        GameplayPersona::Entrepreneur => 15,
        GameplayPersona::Opportunist => 10,
    };
    push_candidate(
        candidates,
        GameplayCommandKind::ConveneFamilyCouncil,
        PlayerCommand::ConveneFamilyCouncil,
        format!(
            "convene the family council at {} bp unity to reconcile claims and obligations",
            council.unity_basis_points
        ),
        55_i64
            .saturating_add(pressure_bonus)
            .saturating_add(persona_bonus),
    );
}

pub(crate) fn preferred_house_governance(
    state: &AppState,
    persona: GameplayPersona,
) -> Option<HouseGovernance> {
    let dynasty = state.dynasties.get(&state.player_dynasty_id)?;
    let council = state.family_councils.get(&state.player_dynasty_id)?;
    let active_members = council
        .members
        .iter()
        .filter(|character_id| {
            state
                .characters
                .get(**character_id)
                .is_some_and(|character| character.status() == CharacterStatus::Active)
        })
        .count();
    let administrative_load = dynasty.administrative_load().saturating_add(
        crate::systems::dynasty_office_administrative_load(state, dynasty.id()),
    );
    let overextended = administrative_load > dynasty.administrative_capacity();
    let head_age = state.characters.get(dynasty.head_id()).map_or(0, |head| {
        state.clock.day().saturating_sub(head.birth_day()) / 360
    });
    if council.unity_basis_points < 5_500 {
        return Some(HouseGovernance::FamilyPartnership);
    }
    if overextended && active_members >= 4 {
        return Some(HouseGovernance::BranchFederation);
    }
    if head_age >= 50 || dynasty.runtime.succession_risk_basis_points >= 2_500 {
        return Some(match persona {
            GameplayPersona::Steward | GameplayPersona::PowerBroker => {
                HouseGovernance::Primogeniture
            }
            GameplayPersona::Entrepreneur if active_members >= 4 => {
                HouseGovernance::BranchFederation
            }
            GameplayPersona::Entrepreneur => HouseGovernance::FamilyPartnership,
            GameplayPersona::Opportunist => HouseGovernance::HeadCommand,
        });
    }
    Some(match persona {
        GameplayPersona::Entrepreneur if active_members >= 4 => HouseGovernance::BranchFederation,
        GameplayPersona::Steward | GameplayPersona::Entrepreneur => {
            HouseGovernance::FamilyPartnership
        }
        GameplayPersona::PowerBroker => HouseGovernance::Primogeniture,
        GameplayPersona::Opportunist if overextended => HouseGovernance::BranchFederation,
        GameplayPersona::Opportunist => HouseGovernance::HeadCommand,
    })
}

/// Whether any canonical heir-designation audit record exists for the
/// dynasty. Formal preparation is an "ever happened" predicate, so unlike
/// cooldown questions it may legitimately scan the full append-only history.
fn dynasty_has_heir_designation(state: &AppState) -> bool {
    let designation_subject = format!("dynasty:{}", state.player_dynasty_id);
    state.audit_log.iter().any(|record| {
        record.kind() == AuditKind::HeirDesignation && record.subject() == designation_subject
    })
}

pub(crate) fn generate_heir_designation_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let dynasty = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    if dynasty.resources.legitimacy_basis_points < HEIR_DESIGNATION_LEGITIMACY_COST {
        return;
    }
    // A designation is available once the canonical cadence since the last
    // recorded designation has elapsed.
    if !audit_records_within_cooldown(state, HEIR_DESIGNATION_INTERVAL_DAYS).all(|record| {
        !(record.kind() == AuditKind::HeirDesignation
            && record.subject() == format!("dynasty:{}", state.player_dynasty_id))
    }) {
        return;
    }
    let current_heir_id = dynasty.heir_id();
    let current_heir = current_heir_id.and_then(|heir_id| state.characters.get(heir_id));
    // The canonical route charges family unity for a designation, and a
    // divided council cannot pay: generating a candidate it would certainly
    // reject only burns a probe slot every cycle.
    if state
        .family_councils
        .get(&state.player_dynasty_id)
        .is_none_or(|council| council.unity_basis_points < HEIR_DESIGNATION_UNITY_COST)
    {
        return;
    }
    let (head_age, head_health) = character_age_and_health(state, dynasty.head_id());
    // A house without any designated heir wants one as soon as the succession
    // horizon matters at all; with an heir in place, only an aging or at-risk
    // head justifies reconsidering the line.
    if current_heir.is_some()
        && head_age < 48
        && dynasty.runtime.succession_risk_basis_points < 2_000
    {
        return;
    }
    let council = state
        .family_councils
        .get(&state.player_dynasty_id)
        .expect("player family council must exist");
    let replacement = council
        .members
        .iter()
        .filter_map(|character_id| state.characters.get(*character_id))
        .filter(|character| {
            character.id() != dynasty.head_id()
                && Some(character.id()) != current_heir_id
                && character.status() == CharacterStatus::Active
                && state.clock.day().saturating_sub(character.birth_day()) >= 18 * 360
        })
        .max_by_key(|character| {
            (
                successor_primary_capability(character, persona),
                successor_score(character, persona),
                character.id(),
            )
        });
    if let Some(replacement) = replacement {
        let should_designate = match current_heir {
            // Without an heir the strongest eligible member is always worth
            // designating; the canonical route supports first designation.
            None => true,
            Some(current_heir) => {
                let broadly_superior = successor_score(replacement, persona)
                    >= successor_score(current_heir, persona).saturating_add(20);
                let strategically_specialized = successor_primary_capability(replacement, persona)
                    >= successor_primary_capability(current_heir, persona).saturating_add(5);
                broadly_superior || strategically_specialized
            }
        };
        if should_designate {
            push_candidate(
                candidates,
                GameplayCommandKind::DesignateHeir,
                PlayerCommand::DesignateHeir {
                    character_id: replacement.id(),
                },
                format!(
                    "designate {} as {} for the {persona:?} succession strategy",
                    character_label(state, replacement.id()),
                    if current_heir.is_some() {
                        "heir"
                    } else {
                        "the first heir"
                    },
                ),
                1_000_i64.saturating_add(head_age.saturating_sub(47).saturating_mul(20)),
            );
            return;
        }
    }
    let (Some(current_heir_id), Some(current_heir)) = (current_heir_id, current_heir) else {
        return;
    };
    propose_formal_confirmation(
        state,
        persona,
        candidates,
        council,
        current_heir_id,
        current_heir,
        head_age,
        head_health,
        dynasty_has_heir_designation(state),
    );
}

/// Formally confirms an existing heir when the succession horizon demands it:
/// the heir must be eligible and the head old or frail enough that leaving the
/// line informal is a real risk.
#[expect(clippy::too_many_arguments)]
pub(crate) fn propose_formal_confirmation(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
    council: &crate::core::FamilyCouncilState,
    current_heir_id: crate::ids::CharacterId,
    current_heir: &crate::core::Character,
    head_age: i64,
    head_health: u16,
    designation_already_recorded: bool,
) {
    let current_heir_is_eligible = current_heir.status() == CharacterStatus::Active
        && state.clock.day().saturating_sub(current_heir.birth_day()) >= 18 * 360
        && council.members.contains(&current_heir_id);
    let confirmation_pressure = head_age >= HEIR_CONFIRMATION_HEAD_AGE_YEARS
        || head_health <= HEIR_CONFIRMATION_HEALTH_THRESHOLD;
    if designation_already_recorded || !current_heir_is_eligible || !confirmation_pressure {
        return;
    }
    push_candidate(
        candidates,
        GameplayCommandKind::DesignateHeir,
        PlayerCommand::DesignateHeir {
            character_id: current_heir_id,
        },
        format!(
            "formally confirm {} as heir for the {persona:?} succession strategy",
            character_label(state, current_heir_id)
        ),
        900_i64.saturating_add(head_age.saturating_sub(47).saturating_mul(20)),
    );
}

pub(crate) fn character_age_and_health(state: &AppState, character_id: CharacterId) -> (i64, u16) {
    state
        .characters
        .get(character_id)
        .map_or((0, 10_000), |character| {
            (
                state.clock.day().saturating_sub(character.birth_day()) / 360,
                character.runtime.health_basis_points,
            )
        })
}

pub(crate) fn successor_primary_capability(
    character: &crate::core::Character,
    persona: GameplayPersona,
) -> u16 {
    match persona {
        GameplayPersona::Steward => character.capabilities.administration,
        GameplayPersona::Entrepreneur => character.capabilities.commerce,
        GameplayPersona::PowerBroker => character.capabilities.social,
        GameplayPersona::Opportunist => character
            .capabilities
            .administration
            .max(character.capabilities.commerce)
            .max(character.capabilities.social)
            .max(character.capabilities.craft),
    }
}

pub(crate) fn successor_score(character: &crate::core::Character, persona: GameplayPersona) -> i64 {
    let capabilities = &character.capabilities;
    let loyalty = i64::from(character.runtime.loyalty_basis_points / 50);
    match persona {
        GameplayPersona::Steward => {
            i64::from(capabilities.administration) * 4
                + i64::from(capabilities.social) * 2
                + i64::from(capabilities.commerce)
                + loyalty
        }
        GameplayPersona::Entrepreneur => {
            i64::from(capabilities.commerce) * 4
                + i64::from(capabilities.administration) * 2
                + i64::from(capabilities.craft)
                + loyalty
        }
        GameplayPersona::PowerBroker => {
            i64::from(capabilities.social) * 4
                + i64::from(capabilities.administration) * 2
                + i64::from(capabilities.commerce)
                + loyalty
        }
        GameplayPersona::Opportunist => {
            i64::from(capabilities.administration) * 2
                + i64::from(capabilities.commerce) * 2
                + i64::from(capabilities.social) * 2
                + i64::from(capabilities.craft) * 2
                + loyalty
        }
    }
}

pub(crate) fn eligible_office_characters(state: &AppState) -> Vec<&crate::core::Character> {
    state
        .characters
        .iter()
        .filter(|character| {
            character.dynasty_id() == state.player_dynasty_id
                && character.status() == CharacterStatus::Active
                && !state
                    .institutions
                    .values()
                    .any(|institution| institution.office_holder_id == Some(character.id()))
        })
        .collect()
}

pub(crate) fn player_controlled_office_powers(state: &AppState) -> BTreeSet<OfficePower> {
    state
        .institutions
        .values()
        .filter(|institution| {
            institution.office_holder_id.is_some_and(|character_id| {
                state
                    .characters
                    .get(character_id)
                    .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
            })
        })
        .flat_map(|institution| institution.powers.iter().copied())
        .collect()
}

pub(crate) fn institution_is_strategic_target(
    state: &AppState,
    institution: &crate::core::InstitutionRuntime,
    controlled_powers: &BTreeSet<OfficePower>,
    player_has_institutional_foothold: bool,
    persona: GameplayPersona,
) -> bool {
    let political_recovery_target =
        institution_support_recovery_bonus(state, player_has_institutional_foothold, persona) > 0;
    let held_by_player = institution.office_holder_id.is_some_and(|character_id| {
        state
            .characters
            .get(character_id)
            .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
    });
    let represented_by_player = institution.members.iter().any(|character_id| {
        state
            .characters
            .get(*character_id)
            .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
    });
    !held_by_player
        && (represented_by_player
            || !player_has_institutional_foothold
            || political_recovery_target
            || institution.powers.iter().any(|power| {
                !controlled_powers.contains(power)
                    && office_power_persona_bonus(persona, *power) > 0
            }))
}

pub(crate) fn institution_support_recovery_bonus(
    state: &AppState,
    player_has_institutional_foothold: bool,
    persona: GameplayPersona,
) -> i64 {
    let recovery_threshold = if persona == GameplayPersona::PowerBroker {
        WARD_ADOPTION_LEGITIMACY_REQUIREMENT
    } else {
        OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST
    };
    if player_has_institutional_foothold
        && state
            .dynasties
            .get(&state.player_dynasty_id)
            .is_some_and(|dynasty| dynasty.resources.legitimacy_basis_points < recovery_threshold)
    {
        AGENT_POLITICAL_RECOVERY_SUPPORT_BONUS
    } else {
        0
    }
}

pub(crate) fn office_power_directive_available(
    state: &AppState,
    institution_id: InstitutionId,
) -> bool {
    // Windowed scan: a directive outside the cooldown interval cannot block.
    audit_records_within_cooldown(state, OFFICE_POWER_DIRECTIVE_INTERVAL_DAYS).all(|record| {
        !(record.kind() == AuditKind::OfficeDirective
            && record.audit_subject().institution_id() == Some(institution_id))
    })
}

pub(crate) fn district_food_satisfaction(state: &AppState, district_id: DistrictId) -> u16 {
    let (total, count) = state
        .households
        .ids_for_district(district_id)
        .into_iter()
        .flatten()
        .filter_map(|household_id| state.households.get(*household_id))
        .fold((0_u64, 0_u64), |(total, count), household| {
            (
                total.saturating_add(u64::from(household.food_satisfaction_basis_points())),
                count.saturating_add(1),
            )
        });
    total
        .checked_div(count)
        .and_then(|average| u16::try_from(average).ok())
        .unwrap_or(10_000)
}

pub(crate) fn office_power_need_bonus(
    state: &AppState,
    institution_id: InstitutionId,
    district_id: DistrictId,
    power: OfficePower,
) -> i64 {
    let district = state
        .districts
        .get(&district_id)
        .expect("institution district must exist");
    match power {
        OfficePower::Licenses => {
            i64::from(6_500_u16.saturating_sub(district.employment_basis_points))
        }
        OfficePower::Inspections => {
            i64::from(6_500_u16.saturating_sub(district.sanitation_basis_points))
        }
        OfficePower::MarketTolls | OfficePower::Taxation => state
            .institutions
            .get(&institution_id)
            .map_or(0, |institution| {
                i64::from(6_500_u16.saturating_sub(institution.legitimacy_basis_points))
            }),
        OfficePower::DebtEnforcement => {
            if state.loans.values().any(|loan| {
                (loan.lender_dynasty_id == state.player_dynasty_id
                    || loan.borrower_dynasty_id == state.player_dynasty_id)
                    && matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted)
            }) {
                1_800
            } else {
                0
            }
        }
        OfficePower::CityContracts => {
            if state.businesses.iter().any(|business| {
                business.owner_dynasty_id() == state.player_dynasty_id
                    && (business.status() == BusinessStatus::Distressed
                        || business.cash() < Money::from_copper(5_000))
            }) {
                1_500
            } else {
                0
            }
        }
        OfficePower::PublicWorks => {
            i64::from(6_500_u16.saturating_sub(district.employment_basis_points)).saturating_add(
                i64::from(6_500_u16.saturating_sub(district.sanitation_basis_points)),
            )
        }
        OfficePower::WatchPriorities => {
            i64::from(6_500_u16.saturating_sub(district.safety_basis_points))
                .saturating_add(i64::from(district.unrest_basis_points / 2))
        }
        OfficePower::EmergencyImports => {
            let crisis_pressure = state.crises.values().any(|crisis| {
                crisis.status.is_active()
                    && matches!(
                        crisis.kind,
                        CrisisKind::GrainShortage | CrisisKind::Epidemic
                    )
            });
            let food_pressure =
                7_000_u16.saturating_sub(district_food_satisfaction(state, district_id));
            i64::from(food_pressure).saturating_add(if crisis_pressure { 2_000 } else { 0 })
        }
    }
}

pub(crate) fn office_power_candidate_need_score(raw_need: i64) -> i64 {
    // District conditions in ordinary play sit near 6,300-6,600 bp, so a
    // material-need bar above a few hundred basis points would silence office
    // directives outside disasters. Directives are scarce through their own
    // legitimacy cost and cooldown; the need gate should only filter powers
    // whose district has no visible gap at all.
    const MATERIAL_NEED_THRESHOLD: i64 = 300;
    const NEED_SCORE_FLOOR: i64 = 300;
    const NEED_SCORE_CAP: i64 = 1_200;

    if raw_need < MATERIAL_NEED_THRESHOLD {
        return 0;
    }
    NEED_SCORE_FLOOR
        .saturating_add(raw_need.saturating_sub(MATERIAL_NEED_THRESHOLD) / 2)
        .min(NEED_SCORE_CAP)
}

pub(crate) fn generate_office_power_directive_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    if player.resources.legitimacy_basis_points < OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST {
        return;
    }
    for institution in state.institutions.values() {
        let held_by_player = institution.office_holder_id.is_some_and(|character_id| {
            state.characters.get(character_id).is_some_and(|character| {
                character.status() == CharacterStatus::Active
                    && character.dynasty_id() == state.player_dynasty_id
            })
        });
        if !held_by_player || !office_power_directive_available(state, institution.institution_id) {
            continue;
        }
        if state.clock.day()
            < institution
                .term_started_day
                .saturating_add(OFFICE_POWER_ESTABLISHMENT_DAYS)
        {
            continue;
        }
        let district_id = registry
            .get_institution(institution.institution_id)
            .expect("runtime institution must have a definition")
            .district_id();
        let selected = institution
            .powers
            .iter()
            .copied()
            .map(|power| {
                let raw_need =
                    office_power_need_bonus(state, institution.institution_id, district_id, power);
                let need = office_power_candidate_need_score(raw_need);
                let priority = office_power_persona_bonus(persona, power).saturating_add(need);
                (need, priority, power)
            })
            .filter(|(need, priority, _)| *need > 0 && *priority > 0)
            .max_by_key(|(_, priority, power)| (*priority, *power));
        let Some((_, priority, power)) = selected else {
            continue;
        };
        push_candidate(
            candidates,
            GameplayCommandKind::ExerciseOfficePower,
            PlayerCommand::ExerciseOfficePower {
                institution_id: institution.institution_id,
                power,
            },
            format!(
                "exercise {power:?} through {} to shape {}",
                institution_label(registry, institution.institution_id),
                district_label(registry, district_id)
            ),
            priority,
        );
    }
}

pub(crate) fn generate_institution_ascent_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let (support_bonus, nomination_bonus) = institution_ascent_bonuses(persona);
    let characters = eligible_office_characters(state);
    let controlled_powers = player_controlled_office_powers(state);
    let player_has_institutional_foothold = has_player_institutional_foothold(state);
    let recovery_bonus =
        institution_support_recovery_bonus(state, player_has_institutional_foothold, persona);
    for institution in state.institutions.values() {
        if !institution_is_strategic_target(
            state,
            institution,
            &controlled_powers,
            player_has_institutional_foothold,
            persona,
        ) {
            continue;
        }
        let institution_kind = registry
            .get_institution(institution.institution_id)
            .expect("runtime institution must have a registry definition")
            .kind();
        let strongest_character = strongest_institution_support_candidate(
            registry,
            state,
            institution,
            &characters,
            institution_kind,
        );
        let power_bonus = institution_ascent_power_bonus(registry, state, institution, persona);
        if let Some(character) = strongest_character {
            push_candidate(
                candidates,
                GameplayCommandKind::CultivateInstitutionSupport,
                PlayerCommand::CultivateInstitutionSupport {
                    institution_id: institution.institution_id,
                    character_id: character.id(),
                },
                format!(
                    "cultivate support for {} in {}",
                    character_label(state, character.id()),
                    institution_label(registry, institution.institution_id)
                ),
                support_bonus
                    .saturating_add(power_bonus)
                    .saturating_add(recovery_bonus),
            );
        }
        let nominee =
            strongest_office_nominee(registry, state, institution, &characters, institution_kind);
        if let Some(character) = nominee {
            push_candidate(
                candidates,
                GameplayCommandKind::NominateForOffice,
                PlayerCommand::NominateForOffice {
                    institution_id: institution.institution_id,
                    character_id: character.id(),
                },
                format!(
                    "nominate {} to {}",
                    character_label(state, character.id()),
                    institution_label(registry, institution.institution_id)
                ),
                nomination_bonus.saturating_add(power_bonus),
            );
        }
    }
}

pub(crate) fn strongest_institution_support_candidate<'a>(
    registry: &Registry,
    state: &AppState,
    institution: &crate::core::InstitutionRuntime,
    characters: &[&'a crate::core::Character],
    institution_kind: InstitutionKind,
) -> Option<&'a crate::core::Character> {
    if characters
        .iter()
        .any(|character| institution.members.contains(&character.id()))
    {
        return None;
    }
    characters
        .iter()
        .copied()
        .filter(|character| {
            is_institution_support_available(
                registry,
                state,
                institution.institution_id,
                character.id(),
            ) && institution_membership_count(state, character.id())
                < MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER
                && institution_support_day(state, institution.institution_id, character.id())
                    .is_none()
        })
        .max_by_key(|character| {
            (
                institution_capability_score(character, institution_kind),
                std::cmp::Reverse(character.id()),
            )
        })
}

pub(crate) fn strongest_office_nominee<'a>(
    registry: &Registry,
    state: &AppState,
    institution: &crate::core::InstitutionRuntime,
    characters: &[&'a crate::core::Character],
    institution_kind: InstitutionKind,
) -> Option<&'a crate::core::Character> {
    characters
        .iter()
        .copied()
        .filter(|character| institution.members.contains(&character.id()))
        .filter(|character| {
            is_office_nomination_available(
                registry,
                state,
                institution.institution_id,
                character.id(),
            )
        })
        .filter(|character| {
            institution_support_day(state, institution.institution_id, character.id()).is_some_and(
                |day| {
                    state.clock.day() >= day.saturating_add(INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS)
                },
            )
        })
        .max_by_key(|character| {
            (
                institution_capability_score(character, institution_kind),
                std::cmp::Reverse(character.id()),
            )
        })
}

pub(crate) const fn institution_ascent_bonuses(persona: GameplayPersona) -> (i64, i64) {
    match persona {
        GameplayPersona::PowerBroker => (850, 620),
        GameplayPersona::Steward => (420, 170),
        GameplayPersona::Entrepreneur => (260, 130),
        GameplayPersona::Opportunist => (540, 260),
    }
}

pub(crate) fn has_player_institutional_foothold(state: &AppState) -> bool {
    state.institutions.values().any(|institution| {
        institution.members.iter().any(|character_id| {
            state
                .characters
                .get(*character_id)
                .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
        })
    })
}

pub(crate) fn generate_institution_withdrawal_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    if !has_institution_withdrawal_pressure(state) {
        return;
    }
    let recent_shortfall = has_recent_player_office_duty_shortfall(state);
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    let monthly_cost = player_monthly_committed_duty_cost(state);
    let severe_liquidity = treasury < monthly_cost.saturating_mul(3);
    let reserve_pressure = treasury < player_committed_duty_reserve(state);
    let business_distress = player_has_severe_business_distress(state);
    let political_paralysis = player_is_politically_overextended(state);
    let persona_bonus: i64 = match persona {
        GameplayPersona::Steward => -100,
        GameplayPersona::Entrepreneur => 200,
        GameplayPersona::PowerBroker => -200,
        GameplayPersona::Opportunist => 350,
    };
    let urgency: i64 = if recent_shortfall {
        2_400
    } else if severe_liquidity {
        1_800
    } else if political_paralysis {
        1_600
    } else if business_distress {
        1_200
    } else if reserve_pressure {
        1_000
    } else {
        700
    };
    for institution in state.institutions.values() {
        let Some(character_id) = institution.office_holder_id else {
            continue;
        };
        if state
            .characters
            .get(character_id)
            .is_none_or(|character| character.dynasty_id() != state.player_dynasty_id)
        {
            continue;
        }
        push_candidate(
            candidates,
            GameplayCommandKind::WithdrawFromInstitution,
            PlayerCommand::WithdrawFromInstitution {
                institution_id: institution.institution_id,
                character_id,
            },
            format!(
                "withdraw {} from {} and surrender its office",
                character_label(state, character_id),
                institution_label(registry, institution.institution_id)
            ),
            urgency.saturating_add(persona_bonus),
        );
    }
}

pub(crate) fn player_current_office_duty_cost(state: &AppState) -> Money {
    projected_dynasty_monthly_office_duty(state, state.player_dynasty_id, 0)
}

pub(crate) fn player_monthly_committed_duty_cost(state: &AppState) -> Money {
    state
        .loans
        .values()
        .filter(|loan| {
            loan.borrower_dynasty_id == state.player_dynasty_id && loan.status.is_repayment_active()
        })
        .fold(player_current_office_duty_cost(state), |total, loan| {
            total.saturating_add(loan.weekly_payment.saturating_mul(4))
        })
}

pub(crate) fn player_committed_duty_reserve(state: &AppState) -> Money {
    let monthly_cost = player_monthly_committed_duty_cost(state);
    if monthly_cost == Money::ZERO {
        return Money::ZERO;
    }
    monthly_cost
        .saturating_mul(AGENT_OFFICE_DUTY_RESERVE_MONTHS)
        .saturating_add(AGENT_OFFICE_LIQUIDITY_BUFFER)
}

pub(crate) fn player_has_severe_business_distress(state: &AppState) -> bool {
    state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == state.player_dynasty_id
            && (matches!(
                business.status(),
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            ) || business.operations.condition_basis_points < 2_000)
    })
}

pub(crate) fn has_recent_player_office_duty_shortfall(state: &AppState) -> bool {
    // The 180-day lookback window bounds the scan: audit days never decrease,
    // so records older than the cutoff cannot be followed by relevant ones.
    let earliest_day = state.clock.day().saturating_sub(180);
    audit_records_from(state, earliest_day).any(|record| {
        record.kind() == AuditKind::OfficeDutyShortfall
            && audit_subject_has_dynasty(record.audit_subject(), state.player_dynasty_id)
    })
}

pub(crate) fn has_institution_withdrawal_opportunity(state: &AppState) -> bool {
    // The canonical route (`apply_institution_withdrawal`) accepts withdrawal of
    // any player character who is an institution member; there is no game-side
    // treasury, distress, or duty gate for the first withdrawal. The agent's
    // distress-only generator is policy; the activation predicate must not hide
    // a world in which the game accepts a withdrawal.
    state.institutions.values().any(|institution| {
        institution.members.iter().any(|character_id| {
            state
                .characters
                .get(*character_id)
                .is_some_and(|character| {
                    character.dynasty_id() == state.player_dynasty_id
                        && character.status() == CharacterStatus::Active
                })
        })
    })
}

/// The agent's office-retreat policy: a withdrawal candidate is surfaced only
/// when the dynasty faces a recent duty shortfall, political overextension,
/// committed-reserve pressure, or severe business distress. The canonical game
/// accepts a withdrawal without any of these; this gate keeps the agent from
/// treating a routine membership as a strategic retreat.
pub(crate) fn has_institution_withdrawal_pressure(state: &AppState) -> bool {
    let office_cost = player_current_office_duty_cost(state);
    if office_cost == Money::ZERO {
        return false;
    }
    let monthly_cost = player_monthly_committed_duty_cost(state);
    let treasury = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .treasury();
    has_recent_player_office_duty_shortfall(state)
        || player_is_politically_overextended(state)
        || treasury < player_committed_duty_reserve(state)
        || (player_has_severe_business_distress(state)
            && treasury
                < monthly_cost
                    .saturating_mul(AGENT_OFFICE_DUTY_RESERVE_MONTHS)
                    .saturating_add(AGENT_OFFICE_LIQUIDITY_BUFFER))
}

pub(crate) fn player_is_politically_overextended(state: &AppState) -> bool {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    count_player_offices(state, state.player_dynasty_id) >= 2
        && player.resources.legitimacy_basis_points < OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST
}

pub(crate) fn generate_ward_adoption_candidates(
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    let adoption_available = player.treasury()
        >= WARD_ADOPTION_COST.saturating_add(dynasty_discretionary_floor(state))
        && player.resources.legitimacy_basis_points >= WARD_ADOPTION_LEGITIMACY_REQUIREMENT
        && player
            .resources
            .reputation_quality_basis_points
            .max(player.resources.reputation_reliability_basis_points)
            >= WARD_ADOPTION_REPUTATION_REQUIREMENT
        && player_contract_deliveries(state) >= WARD_ADOPTION_DELIVERY_REQUIREMENT
        && active_player_ward_count(state) < MAX_ACTIVE_WARDS
        // The canonical route charges family unity for an adoption, and a
        // divided council cannot pay: skip generation when the house could
        // not commit.
        && state.family_councils.get(&state.player_dynasty_id).is_some_and(
            |council| council.unity_basis_points >= WARD_ADOPTION_UNITY_COST,
        )
        // Windowed scan: an adoption outside the cooldown interval cannot
        // block the next one.
        && audit_records_within_cooldown(state, WARD_ADOPTION_INTERVAL_DAYS)
            .all(|record| {
                !(record.kind() == AuditKind::WardAdoption
                    && record
                        .subject()
                        .starts_with(&format!("dynasty:{}:", state.player_dynasty_id)))
            });
    if !adoption_available {
        return;
    }
    let base_bonus: i64 = match persona {
        GameplayPersona::PowerBroker => 620,
        GameplayPersona::Steward => 500,
        GameplayPersona::Opportunist => 420,
        GameplayPersona::Entrepreneur => 360,
    };
    for focus in education_focus_order(persona) {
        push_candidate(
            candidates,
            GameplayCommandKind::AdoptWard,
            PlayerCommand::AdoptWard { focus },
            format!("adopt a {focus:?}-focused ward into the dynasty"),
            base_bonus.saturating_add(education_focus_persona_bonus(persona, focus)),
        );
    }
}

pub(crate) fn generate_family_education_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    candidates: &mut Vec<Candidate>,
) {
    const TARGETED_PREPARATION_BONUS: i64 = 700;
    const MIN_TARGETED_PREPARATION_DELIVERIES: u32 = 4;
    let player = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    // Education has no reputation or commercial-record requirement in the
    // canonical validation: a dynasty that can afford the tutor may train any
    // active family member below mastery. Requiring a reputation or a contract-
    // delivery record here produced dead foundation periods where the world
    // offered the command and the game accepted it but the agent declined it.
    // The targeted-institution preparation bonus below still rewards an
    // established commercial record; eligibility itself is governed by cost,
    // focus headroom, and cooldowns.
    let education_available = player.treasury()
        >= FAMILY_EDUCATION_COST.saturating_add(dynasty_discretionary_floor(state));
    if !education_available {
        return;
    }
    let base_bonus: i64 = match persona {
        GameplayPersona::PowerBroker => 480,
        GameplayPersona::Entrepreneur => 430,
        GameplayPersona::Steward => 400,
        GameplayPersona::Opportunist => 320,
    };
    let active_characters: Vec<_> = state
        .characters
        .iter()
        .filter(|character| {
            character.dynasty_id() == state.player_dynasty_id
                && character.status() == CharacterStatus::Active
        })
        .collect();
    let controlled_powers = player_controlled_office_powers(state);
    let player_has_institutional_foothold = has_player_institutional_foothold(state);
    for focus in education_focus_order(persona) {
        let targeted_student = targeted_family_education_student(
            registry,
            state,
            persona,
            focus,
            &controlled_powers,
            player_has_institutional_foothold,
            MIN_TARGETED_PREPARATION_DELIVERIES,
        )
        .filter(|(character, _, _)| {
            family_education_next_day(state, character.id())
                .is_none_or(|day| state.clock.day() >= day)
        });
        let succession_student = targeted_student
            .is_none()
            .then(|| succession_family_education_student(state, persona, focus))
            .flatten()
            .filter(|character| {
                family_education_next_day(state, character.id())
                    .is_none_or(|day| state.clock.day() >= day)
            });
        let student = targeted_student
            .map(|(character, _, _)| character)
            .or(succession_student)
            .or_else(|| {
                if player_has_institutional_foothold {
                    return None;
                }
                active_characters
                    .iter()
                    .copied()
                    .filter(|character| {
                        character_focus_value(character, focus) < 100
                            && family_education_next_day(state, character.id())
                                .is_none_or(|day| state.clock.day() >= day)
                    })
                    .min_by_key(|character| {
                        (character_focus_value(character, focus), character.id())
                    })
            });
        let Some(student) = student else {
            continue;
        };
        let preparation_bonus = targeted_student.map_or(0, |_| TARGETED_PREPARATION_BONUS);
        let succession_bonus = succession_student.map_or(0, |_| 500);
        push_candidate(
            candidates,
            GameplayCommandKind::EducateFamilyMember,
            PlayerCommand::EducateFamilyMember {
                character_id: student.id(),
                focus,
            },
            family_education_candidate_description(
                registry,
                state,
                student,
                focus,
                targeted_student,
                succession_student.is_some(),
            ),
            base_bonus
                .saturating_add(education_focus_persona_bonus(persona, focus))
                .saturating_add(preparation_bonus)
                .saturating_add(succession_bonus),
        );
    }
}

pub(crate) fn family_education_candidate_description(
    registry: &Registry,
    state: &AppState,
    student: &crate::core::Character,
    focus: EducationFocus,
    targeted_student: Option<(&crate::core::Character, u32, InstitutionId)>,
    succession_preparation: bool,
) -> String {
    if let Some((_, extra, institution_id)) = targeted_student {
        return format!(
            "educate {} in {focus:?} to qualify for {} (saves {extra} delivery requirements)",
            character_label(state, student.id()),
            institution_label(registry, institution_id)
        );
    }
    if succession_preparation {
        return format!(
            "educate heir {} in {focus:?} for succession preparation",
            character_label(state, student.id())
        );
    }
    format!(
        "educate {} in {focus:?}",
        character_label(state, student.id())
    )
}

pub(crate) fn succession_family_education_student(
    state: &AppState,
    persona: GameplayPersona,
    focus: EducationFocus,
) -> Option<&crate::core::Character> {
    if focus != succession_education_focus(persona) {
        return None;
    }
    let dynasty = state.dynasties.get(&state.player_dynasty_id)?;
    let heir_id = dynasty.heir_id()?;
    let heir = state.characters.get(heir_id)?;
    let council = state.family_councils.get(&state.player_dynasty_id)?;
    let (head_age, head_health) = character_age_and_health(state, dynasty.head_id());
    let succession_pressure = head_age >= 48
        || head_health <= HEIR_CONFIRMATION_HEALTH_THRESHOLD
        || dynasty.runtime.succession_risk_basis_points >= 2_000;
    (succession_pressure
        && heir.status() == CharacterStatus::Active
        && state.clock.day().saturating_sub(heir.birth_day()) >= 18 * 360
        && council.members.contains(&heir_id)
        && character_focus_value(heir, focus) < 100)
        .then_some(heir)
}

pub(crate) const fn succession_education_focus(persona: GameplayPersona) -> EducationFocus {
    match persona {
        GameplayPersona::Steward => EducationFocus::Administration,
        GameplayPersona::Entrepreneur => EducationFocus::Commerce,
        GameplayPersona::PowerBroker => EducationFocus::Social,
        GameplayPersona::Opportunist => EducationFocus::Craft,
    }
}

pub(crate) fn targeted_family_education_student<'a>(
    registry: &Registry,
    state: &'a AppState,
    persona: GameplayPersona,
    focus: EducationFocus,
    controlled_powers: &BTreeSet<OfficePower>,
    player_has_institutional_foothold: bool,
    minimum_extra_deliveries: u32,
) -> Option<(&'a crate::core::Character, u32, InstitutionId)> {
    state
        .institutions
        .values()
        .filter(|institution| {
            institution_is_strategic_target(
                state,
                institution,
                controlled_powers,
                player_has_institutional_foothold,
                persona,
            )
        })
        .filter_map(|institution| {
            let institution_kind = registry
                .get_institution(institution.institution_id)
                .expect("runtime institution must have a registry definition")
                .kind();
            if !institution_education_focus_is_relevant(institution_kind, focus) {
                return None;
            }
            institution
                .members
                .iter()
                .filter_map(|character_id| state.characters.get(*character_id))
                .filter(|character| {
                    character.dynasty_id() == state.player_dynasty_id
                        && character.status() == CharacterStatus::Active
                        && character_focus_value(character, focus) < 100
                })
                .filter_map(|character| {
                    let required = office_nomination_delivery_requirement(
                        registry,
                        state,
                        institution.institution_id,
                        character.id(),
                    );
                    let extra = required.saturating_sub(OFFICE_NOMINATION_DELIVERY_REQUIREMENT);
                    (extra >= minimum_extra_deliveries).then_some((
                        character,
                        extra,
                        institution.institution_id,
                    ))
                })
                .max_by_key(targeted_education_priority)
        })
        .max_by_key(targeted_education_priority)
}

pub(crate) fn targeted_education_priority(
    (character, extra, institution_id): &(&crate::core::Character, u32, InstitutionId),
) -> (
    u32,
    std::cmp::Reverse<InstitutionId>,
    std::cmp::Reverse<CharacterId>,
) {
    (
        *extra,
        std::cmp::Reverse(*institution_id),
        std::cmp::Reverse(character.id()),
    )
}

pub(crate) const fn institution_education_focus_is_relevant(
    institution_kind: InstitutionKind,
    focus: EducationFocus,
) -> bool {
    match institution_kind {
        InstitutionKind::CraftGuild => {
            matches!(focus, EducationFocus::Craft | EducationFocus::Commerce)
        }
        InstitutionKind::MerchantGuild | InstitutionKind::MarketOffice => {
            matches!(
                focus,
                EducationFocus::Commerce | EducationFocus::Administration
            )
        }
        InstitutionKind::Council | InstitutionKind::Charity => {
            matches!(
                focus,
                EducationFocus::Social | EducationFocus::Administration
            )
        }
        InstitutionKind::Court | InstitutionKind::Watch => {
            matches!(
                focus,
                EducationFocus::Administration | EducationFocus::Social
            )
        }
        InstitutionKind::Treasury => {
            matches!(
                focus,
                EducationFocus::Administration | EducationFocus::Commerce
            )
        }
    }
}

pub(crate) const ALL_EDUCATION_FOCUSSES: [EducationFocus; 4] = [
    EducationFocus::Administration,
    EducationFocus::Commerce,
    EducationFocus::Social,
    EducationFocus::Craft,
];

pub(crate) const fn education_focus_order(persona: GameplayPersona) -> [EducationFocus; 4] {
    match persona {
        GameplayPersona::Steward => [
            EducationFocus::Administration,
            EducationFocus::Social,
            EducationFocus::Commerce,
            EducationFocus::Craft,
        ],
        GameplayPersona::Entrepreneur => [
            EducationFocus::Commerce,
            EducationFocus::Administration,
            EducationFocus::Craft,
            EducationFocus::Social,
        ],
        GameplayPersona::PowerBroker => [
            EducationFocus::Social,
            EducationFocus::Administration,
            EducationFocus::Commerce,
            EducationFocus::Craft,
        ],
        GameplayPersona::Opportunist => [
            EducationFocus::Commerce,
            EducationFocus::Social,
            EducationFocus::Administration,
            EducationFocus::Craft,
        ],
    }
}

pub(crate) const fn education_focus_persona_bonus(
    persona: GameplayPersona,
    focus: EducationFocus,
) -> i64 {
    match persona {
        GameplayPersona::Steward => match focus {
            EducationFocus::Administration => 260,
            EducationFocus::Social => 140,
            EducationFocus::Commerce | EducationFocus::Craft => 0,
        },
        GameplayPersona::Entrepreneur => match focus {
            EducationFocus::Commerce => 260,
            EducationFocus::Administration => 140,
            EducationFocus::Social | EducationFocus::Craft => 0,
        },
        GameplayPersona::PowerBroker => match focus {
            EducationFocus::Social => 260,
            EducationFocus::Administration => 140,
            EducationFocus::Commerce | EducationFocus::Craft => 0,
        },
        GameplayPersona::Opportunist => match focus {
            EducationFocus::Commerce => 260,
            EducationFocus::Social => 140,
            EducationFocus::Administration | EducationFocus::Craft => 0,
        },
    }
}

pub(crate) const fn character_focus_value(
    character: &crate::core::Character,
    focus: EducationFocus,
) -> u16 {
    match focus {
        EducationFocus::Administration => character.capabilities.administration,
        EducationFocus::Commerce => character.capabilities.commerce,
        EducationFocus::Social => character.capabilities.social,
        EducationFocus::Craft => character.capabilities.craft,
    }
}

pub(crate) const fn office_power_persona_bonus(
    persona: GameplayPersona,
    power: OfficePower,
) -> i64 {
    match persona {
        GameplayPersona::Steward => match power {
            OfficePower::PublicWorks => 500,
            OfficePower::EmergencyImports => 420,
            OfficePower::Inspections => 300,
            OfficePower::Licenses
            | OfficePower::MarketTolls
            | OfficePower::DebtEnforcement
            | OfficePower::CityContracts
            | OfficePower::WatchPriorities
            | OfficePower::Taxation => 0,
        },
        GameplayPersona::Entrepreneur => match power {
            OfficePower::MarketTolls => 500,
            OfficePower::Licenses => 420,
            OfficePower::CityContracts => 360,
            OfficePower::Inspections
            | OfficePower::DebtEnforcement
            | OfficePower::PublicWorks
            | OfficePower::WatchPriorities
            | OfficePower::Taxation
            | OfficePower::EmergencyImports => 0,
        },
        GameplayPersona::PowerBroker => match power {
            OfficePower::Taxation => 500,
            OfficePower::PublicWorks => 440,
            OfficePower::DebtEnforcement => 400,
            OfficePower::Licenses
            | OfficePower::Inspections
            | OfficePower::MarketTolls
            | OfficePower::CityContracts
            | OfficePower::WatchPriorities
            | OfficePower::EmergencyImports => 0,
        },
        GameplayPersona::Opportunist => match power {
            OfficePower::DebtEnforcement => 500,
            OfficePower::MarketTolls => 420,
            OfficePower::WatchPriorities => 360,
            OfficePower::Licenses
            | OfficePower::Inspections
            | OfficePower::CityContracts
            | OfficePower::PublicWorks
            | OfficePower::Taxation
            | OfficePower::EmergencyImports => 0,
        },
    }
}

pub(crate) fn institution_power_bonus(
    state: &AppState,
    persona: GameplayPersona,
    powers: &BTreeSet<OfficePower>,
) -> i64 {
    powers
        .iter()
        .map(|power| office_power_ascent_bonus(state, persona, *power))
        .max()
        .unwrap_or(0)
}

pub(crate) fn office_power_ascent_bonus(
    state: &AppState,
    persona: GameplayPersona,
    power: OfficePower,
) -> i64 {
    if persona == GameplayPersona::Opportunist
        && power == OfficePower::DebtEnforcement
        && !city_credit_power_is_relevant(state)
    {
        return 0;
    }
    office_power_persona_bonus(persona, power)
}

pub(crate) fn city_credit_power_is_relevant(state: &AppState) -> bool {
    state.loans.values().any(|loan| !loan.status.is_settled())
        || state
            .civic_debts
            .values()
            .any(|debt| debt.status != CivicDebtStatus::Repaid)
}

pub(crate) fn institution_ascent_power_bonus(
    registry: &Registry,
    state: &AppState,
    institution: &crate::core::InstitutionRuntime,
    persona: GameplayPersona,
) -> i64 {
    let base = institution_power_bonus(state, persona, &institution.powers);
    let kind = registry
        .get_institution(institution.institution_id)
        .expect("runtime institution must have a registry definition")
        .kind();
    let capability_fit = eligible_office_characters(state)
        .into_iter()
        .map(|character| institution_capability_score(character, kind))
        .max()
        .unwrap_or(0);
    base.saturating_add(institution_capability_fit_bonus(capability_fit))
}

pub(crate) fn institution_capability_fit_bonus(capability_score: u32) -> i64 {
    const FULL_FIT_SCORE: u32 = 10_000;
    const FULL_FIT_BONUS: i64 = 500;

    i64::from(capability_score.min(FULL_FIT_SCORE)).saturating_mul(FULL_FIT_BONUS)
        / i64::from(FULL_FIT_SCORE)
}

pub(crate) fn is_office_nomination_available(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> bool {
    let Some(player) = state.dynasties.get(&state.player_dynasty_id) else {
        return false;
    };
    let required_deliveries =
        office_nomination_delivery_requirement(registry, state, institution_id, character_id);
    if player.treasury() < OFFICE_NOMINATION_CAMPAIGN_COST
        || player
            .resources
            .reputation_quality_basis_points
            .max(player.resources.reputation_reliability_basis_points)
            < OFFICE_NOMINATION_REPUTATION_REQUIREMENT
        || player_contract_deliveries(state) < required_deliveries
    {
        return false;
    }
    office_nomination_next_day(state, character_id)
        .is_none_or(|next_day| state.clock.day() >= next_day)
}

pub(crate) fn is_institution_support_available(
    registry: &Registry,
    state: &AppState,
    institution_id: InstitutionId,
    character_id: CharacterId,
) -> bool {
    let Some(player) = state.dynasties.get(&state.player_dynasty_id) else {
        return false;
    };
    let required_deliveries =
        institution_support_delivery_requirement(registry, state, institution_id, character_id);
    if player.treasury()
        < INSTITUTION_SUPPORT_COST.saturating_add(dynasty_discretionary_floor(state))
        || player
            .resources
            .reputation_quality_basis_points
            .max(player.resources.reputation_reliability_basis_points)
            < INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT
        || player_contract_deliveries(state) < required_deliveries
    {
        return false;
    }
    institution_support_next_day(state, institution_id, character_id)
        .is_none_or(|next_day| state.clock.day() >= next_day)
}

pub(crate) fn governance_bonus(persona: GameplayPersona, governance: HouseGovernance) -> i64 {
    match persona {
        GameplayPersona::Steward => match governance {
            HouseGovernance::FamilyPartnership => 420,
            HouseGovernance::HeadCommand
            | HouseGovernance::Primogeniture
            | HouseGovernance::BranchFederation
            | HouseGovernance::ElectedHead => 80,
        },
        GameplayPersona::Entrepreneur => match governance {
            HouseGovernance::FamilyPartnership => 240,
            HouseGovernance::HeadCommand
            | HouseGovernance::Primogeniture
            | HouseGovernance::BranchFederation
            | HouseGovernance::ElectedHead => 80,
        },
        GameplayPersona::PowerBroker => match governance {
            HouseGovernance::Primogeniture => 360,
            HouseGovernance::HeadCommand
            | HouseGovernance::FamilyPartnership
            | HouseGovernance::BranchFederation
            | HouseGovernance::ElectedHead => 80,
        },
        GameplayPersona::Opportunist => match governance {
            HouseGovernance::BranchFederation => 340,
            HouseGovernance::HeadCommand
            | HouseGovernance::Primogeniture
            | HouseGovernance::FamilyPartnership
            | HouseGovernance::ElectedHead => 80,
        },
    }
}

pub(crate) fn rank_adjustment(
    kind: GameplayCommandKind,
    state: &AppState,
    persona: GameplayPersona,
    accumulator: &CampaignAccumulator,
) -> i64 {
    let command_stats = accumulator
        .commands
        .get(&kind)
        .expect("every command kind must have statistics");
    let coverage = if command_stats.executed == 0 { 250 } else { 0 };
    let repetition = i64::from(command_stats.executed).saturating_mul(35);
    let repeat_last = if accumulator.last_command == Some(kind) {
        260
    } else {
        0
    };
    persona_weight(persona, kind)
        .saturating_add(coverage)
        .saturating_add(urgency_weight(state, kind))
        .saturating_add(institutional_conversion_priority(state, persona, kind))
        .saturating_add(recovery_priority_adjustment(state, kind))
        .saturating_add(legacy_rebuild_priority(&accumulator.fantasy_arc, kind))
        .saturating_sub(repetition)
        .saturating_sub(repeat_last)
}

/// After a succession the incoming head's job is rebuilding the estate and
/// restoring the house's standing, not idling between crises. Without this
/// nudge the legacy phase degenerates into crisis whack-a-mole while the
/// commercial recovery routes sit unexercised. Acquisitions are deliberately
/// included at a lower bonus than direct investment so a profitable recovery
/// still favors operational repair before expansion.
pub(crate) fn legacy_rebuild_priority(arc: &GameplayFantasyArc, kind: GameplayCommandKind) -> i64 {
    if arc.first_succession_day.is_none() {
        return 0;
    }
    match kind {
        GameplayCommandKind::InvestInBusiness
        | GameplayCommandKind::SecureSupply
        | GameplayCommandKind::BuyProperty
        | GameplayCommandKind::RespondToCrisis => 820,
        GameplayCommandKind::CultivateInstitutionSupport
        | GameplayCommandKind::EndowInstitution
        | GameplayCommandKind::NominateForOffice
        | GameplayCommandKind::SetBusinessPolicy
        | GameplayCommandKind::SetBusinessWages
        | GameplayCommandKind::FundPublicWork
        | GameplayCommandKind::EnactLaw
        | GameplayCommandKind::ExerciseOfficePower => 520,
        GameplayCommandKind::AcquireBusiness => 320,
        _ => 0,
    }
}

/// Once the dynasty has earned office, the diagnostic agent should actually
/// convert that access into a civic commitment. Otherwise a profitable credit
/// or property candidate can indefinitely crowd out the game's political
/// endpoint even though law and public-work candidates are valid.
pub(crate) fn institutional_conversion_priority(
    state: &AppState,
    persona: GameplayPersona,
    kind: GameplayCommandKind,
) -> i64 {
    if !has_player_office(state) {
        return 0;
    }
    let bonus = match persona {
        GameplayPersona::Steward => 1_600,
        GameplayPersona::Entrepreneur => 1_200,
        GameplayPersona::PowerBroker => 1_800,
        GameplayPersona::Opportunist => 1_000,
    };
    match kind {
        GameplayCommandKind::EnactLaw
        | GameplayCommandKind::StartPublicWork
        | GameplayCommandKind::FundPublicWork
        | GameplayCommandKind::ExerciseOfficePower => bonus,
        GameplayCommandKind::TransferBusinessCash
        | GameplayCommandKind::WithdrawBusinessCash
        | GameplayCommandKind::AcquireBusiness
        | GameplayCommandKind::InvestInBusiness
        | GameplayCommandKind::SetBusinessPolicy
        | GameplayCommandKind::SetBusinessWages
        | GameplayCommandKind::SecureSupply
        | GameplayCommandKind::SellOutput
        | GameplayCommandKind::BorrowFunds
        | GameplayCommandKind::ExtendCredit
        | GameplayCommandKind::BuyProperty
        | GameplayCommandKind::SellProperty
        | GameplayCommandKind::FileLegalCase
        | GameplayCommandKind::SettleLegalCase
        | GameplayCommandKind::SetHouseGovernance
        | GameplayCommandKind::ConveneFamilyCouncil
        | GameplayCommandKind::DesignateHeir
        | GameplayCommandKind::AdoptWard
        | GameplayCommandKind::EducateFamilyMember
        | GameplayCommandKind::CultivateInstitutionSupport
        | GameplayCommandKind::EndowInstitution
        | GameplayCommandKind::NominateForOffice
        | GameplayCommandKind::WithdrawFromInstitution
        | GameplayCommandKind::RespondToCrisis
        | GameplayCommandKind::ResolveLaborDispute
        | GameplayCommandKind::CommissionInformation
        | GameplayCommandKind::LeverageInformation
        | GameplayCommandKind::AcknowledgeNotification => 0,
    }
}

pub(crate) fn player_has_no_active_business(state: &AppState) -> bool {
    let mut owns_business = false;
    let mut has_active = false;
    let mut has_recoverable = false;
    for business in state
        .businesses
        .iter()
        .filter(|business| business.owner_dynasty_id() == state.player_dynasty_id)
    {
        owns_business = true;
        has_active |= business.status() == BusinessStatus::Active;
        has_recoverable |= matches!(
            business.status(),
            BusinessStatus::Distressed | BusinessStatus::Insolvent
        );
    }
    owns_business && !has_active && has_recoverable
}

pub(crate) fn recovery_priority_adjustment(state: &AppState, kind: GameplayCommandKind) -> i64 {
    if !player_has_no_active_business(state) {
        return 0;
    }
    match kind {
        GameplayCommandKind::TransferBusinessCash
        | GameplayCommandKind::WithdrawBusinessCash
        | GameplayCommandKind::InvestInBusiness
        | GameplayCommandKind::BorrowFunds
        | GameplayCommandKind::AcquireBusiness
        | GameplayCommandKind::SellProperty => 3_500,
        GameplayCommandKind::SecureSupply
        | GameplayCommandKind::SellOutput
        | GameplayCommandKind::SetBusinessWages
        | GameplayCommandKind::ResolveLaborDispute
        | GameplayCommandKind::RespondToCrisis => 500,
        GameplayCommandKind::BuyProperty
        | GameplayCommandKind::ExtendCredit
        | GameplayCommandKind::CommissionInformation
        | GameplayCommandKind::LeverageInformation
        | GameplayCommandKind::CultivateInstitutionSupport
        | GameplayCommandKind::EndowInstitution
        | GameplayCommandKind::NominateForOffice
        | GameplayCommandKind::EnactLaw
        | GameplayCommandKind::StartPublicWork
        | GameplayCommandKind::FundPublicWork
        | GameplayCommandKind::FileLegalCase
        | GameplayCommandKind::SettleLegalCase
        | GameplayCommandKind::SetHouseGovernance
        | GameplayCommandKind::ConveneFamilyCouncil
        | GameplayCommandKind::DesignateHeir
        | GameplayCommandKind::AdoptWard
        | GameplayCommandKind::EducateFamilyMember
        | GameplayCommandKind::ExerciseOfficePower => -2_500,
        GameplayCommandKind::SetBusinessPolicy
        | GameplayCommandKind::WithdrawFromInstitution
        | GameplayCommandKind::AcknowledgeNotification => 0,
    }
}

pub(crate) fn steward_weight(kind: GameplayCommandKind) -> i64 {
    match kind {
        GameplayCommandKind::RespondToCrisis | GameplayCommandKind::ResolveLaborDispute => 900,
        GameplayCommandKind::InvestInBusiness | GameplayCommandKind::ExerciseOfficePower => 800,
        GameplayCommandKind::ConveneFamilyCouncil => 850,
        GameplayCommandKind::DesignateHeir | GameplayCommandKind::EducateFamilyMember => 650,
        GameplayCommandKind::SetBusinessPolicy | GameplayCommandKind::StartPublicWork => 600,
        GameplayCommandKind::SetBusinessWages | GameplayCommandKind::FundPublicWork => 620,
        GameplayCommandKind::AdoptWard | GameplayCommandKind::EndowInstitution => 520,
        GameplayCommandKind::CommissionInformation => 480,
        GameplayCommandKind::LeverageInformation => 700,
        GameplayCommandKind::CultivateInstitutionSupport | GameplayCommandKind::SecureSupply => 420,
        GameplayCommandKind::AcknowledgeNotification
        | GameplayCommandKind::WithdrawFromInstitution => 300,
        GameplayCommandKind::TransferBusinessCash
        | GameplayCommandKind::WithdrawBusinessCash
        | GameplayCommandKind::AcquireBusiness
        | GameplayCommandKind::SellOutput
        | GameplayCommandKind::BorrowFunds
        | GameplayCommandKind::ExtendCredit
        | GameplayCommandKind::BuyProperty
        | GameplayCommandKind::SellProperty
        | GameplayCommandKind::EnactLaw
        | GameplayCommandKind::FileLegalCase
        | GameplayCommandKind::SettleLegalCase
        | GameplayCommandKind::SetHouseGovernance
        | GameplayCommandKind::NominateForOffice => 180,
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the exhaustive persona weight matrix keeps every command-family priority visible"
)]
pub(crate) fn persona_weight(persona: GameplayPersona, kind: GameplayCommandKind) -> i64 {
    match persona {
        GameplayPersona::Steward => steward_weight(kind),
        GameplayPersona::Entrepreneur => match kind {
            GameplayCommandKind::SellOutput | GameplayCommandKind::LeverageInformation => 950,
            GameplayCommandKind::SecureSupply
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::SetBusinessWages
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::SellProperty
            | GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::WithdrawBusinessCash => 850,
            GameplayCommandKind::BorrowFunds
            | GameplayCommandKind::CommissionInformation
            | GameplayCommandKind::DesignateHeir => 700,
            GameplayCommandKind::EducateFamilyMember => 600,
            GameplayCommandKind::ConveneFamilyCouncil | GameplayCommandKind::EndowInstitution => {
                320
            }
            GameplayCommandKind::ExtendCredit => 420,
            GameplayCommandKind::AdoptWard => 360,
            GameplayCommandKind::CultivateInstitutionSupport => 300,
            GameplayCommandKind::EnactLaw
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::FundPublicWork
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::SettleLegalCase
            | GameplayCommandKind::SetHouseGovernance
            | GameplayCommandKind::NominateForOffice
            | GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::ResolveLaborDispute
            | GameplayCommandKind::AcknowledgeNotification => 140,
            GameplayCommandKind::ExerciseOfficePower
            | GameplayCommandKind::WithdrawFromInstitution => 500,
        },
        GameplayPersona::PowerBroker => match kind {
            GameplayCommandKind::EnactLaw
            | GameplayCommandKind::CultivateInstitutionSupport
            | GameplayCommandKind::EndowInstitution
            | GameplayCommandKind::NominateForOffice
            | GameplayCommandKind::ExerciseOfficePower
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::FundPublicWork
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::SettleLegalCase
            | GameplayCommandKind::LeverageInformation => 900,
            GameplayCommandKind::ExtendCredit => 820,
            GameplayCommandKind::CommissionInformation => 760,
            GameplayCommandKind::DesignateHeir | GameplayCommandKind::AdoptWard => 780,
            GameplayCommandKind::EducateFamilyMember => 720,
            GameplayCommandKind::ConveneFamilyCouncil => 800,
            GameplayCommandKind::SetHouseGovernance => 700,
            GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::WithdrawBusinessCash
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::SetBusinessWages
            | GameplayCommandKind::SecureSupply
            | GameplayCommandKind::SellOutput
            | GameplayCommandKind::BorrowFunds
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::SellProperty
            | GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::ResolveLaborDispute
            | GameplayCommandKind::AcknowledgeNotification => 120,
            GameplayCommandKind::WithdrawFromInstitution => 50,
        },
        GameplayPersona::Opportunist => match kind {
            GameplayCommandKind::RespondToCrisis
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::BorrowFunds
            | GameplayCommandKind::BuyProperty
            | GameplayCommandKind::SellProperty
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::SettleLegalCase
            | GameplayCommandKind::LeverageInformation => 850,
            GameplayCommandKind::SellOutput | GameplayCommandKind::ExerciseOfficePower => 700,
            GameplayCommandKind::CommissionInformation
            | GameplayCommandKind::ExtendCredit
            | GameplayCommandKind::DesignateHeir => 620,
            GameplayCommandKind::CultivateInstitutionSupport
            | GameplayCommandKind::WithdrawFromInstitution => 650,
            GameplayCommandKind::EndowInstitution | GameplayCommandKind::AdoptWard => 500,
            GameplayCommandKind::EducateFamilyMember => 420,
            GameplayCommandKind::ConveneFamilyCouncil => 350,
            GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::WithdrawBusinessCash
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::SetBusinessWages
            | GameplayCommandKind::SecureSupply
            | GameplayCommandKind::EnactLaw
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::FundPublicWork
            | GameplayCommandKind::SetHouseGovernance
            | GameplayCommandKind::NominateForOffice
            | GameplayCommandKind::ResolveLaborDispute
            | GameplayCommandKind::AcknowledgeNotification => 100,
        },
    }
}

/// Urgency for proactive wage posture: rises as a player workforce approaches
/// dispute thresholds, so the agent treats falling loyalty as a live problem
/// instead of waiting for organized resistance.
pub(crate) fn workforce_strain_urgency(state: &AppState) -> i64 {
    let player_id = state.player_dynasty_id;
    state
        .employment
        .values()
        .filter(|agreement| {
            agreement.status == EmploymentStatus::Active
                && state
                    .businesses
                    .get(agreement.business_id())
                    .is_some_and(|business| business.owner_dynasty_id() == player_id)
        })
        .map(|agreement| {
            let weakest = agreement
                .loyalty_basis_points()
                .min(agreement.conditions_basis_points());
            (6_000_i64.saturating_sub(i64::from(weakest))).max(0)
        })
        .max()
        .unwrap_or(0)
}

pub(crate) fn active_crisis_urgency(state: &AppState) -> i64 {
    if state
        .crises
        .values()
        .any(|crisis| crisis.status.is_active())
    {
        2_500
    } else {
        0
    }
}

pub(crate) fn urgency_weight(state: &AppState, kind: GameplayCommandKind) -> i64 {
    match kind {
        GameplayCommandKind::RespondToCrisis => active_crisis_urgency(state),
        GameplayCommandKind::ResolveLaborDispute => labor_dispute_urgency(state),
        GameplayCommandKind::SetBusinessPolicy => business_policy_urgency(state),
        GameplayCommandKind::SetBusinessWages => workforce_strain_urgency(state),
        GameplayCommandKind::InvestInBusiness => impaired_business_urgency(state, 2_400),
        GameplayCommandKind::AcquireBusiness => acquisition_urgency(state),
        GameplayCommandKind::AcknowledgeNotification => notification_urgency(state),
        GameplayCommandKind::BorrowFunds => borrowing_urgency(state),
        GameplayCommandKind::SellProperty => 3_500,
        GameplayCommandKind::TransferBusinessCash | GameplayCommandKind::WithdrawBusinessCash => {
            impaired_business_urgency(state, 2_800)
        }
        GameplayCommandKind::LeverageInformation => 600,
        GameplayCommandKind::WithdrawFromInstitution => institution_withdrawal_urgency(state),
        GameplayCommandKind::FileLegalCase => legal_case_urgency(state),
        GameplayCommandKind::SettleLegalCase => legal_settlement_urgency(state),
        GameplayCommandKind::ConveneFamilyCouncil => family_council_urgency(state),
        GameplayCommandKind::SecureSupply
        | GameplayCommandKind::SellOutput
        | GameplayCommandKind::ExtendCredit
        | GameplayCommandKind::BuyProperty
        | GameplayCommandKind::EnactLaw
        | GameplayCommandKind::StartPublicWork
        | GameplayCommandKind::FundPublicWork
        | GameplayCommandKind::SetHouseGovernance
        | GameplayCommandKind::DesignateHeir
        | GameplayCommandKind::AdoptWard
        | GameplayCommandKind::EducateFamilyMember
        | GameplayCommandKind::CultivateInstitutionSupport
        | GameplayCommandKind::EndowInstitution
        | GameplayCommandKind::CommissionInformation
        | GameplayCommandKind::ExerciseOfficePower
        | GameplayCommandKind::NominateForOffice => 0,
    }
}

pub(crate) fn legal_settlement_urgency(state: &AppState) -> i64 {
    state
        .legal_cases
        .values()
        .filter(|legal_case| {
            legal_case.defendant_dynasty_id == state.player_dynasty_id
                && matches!(
                    legal_case.status,
                    LegalCaseStatus::Filed | LegalCaseStatus::Hearing
                )
        })
        .map(|legal_case| {
            let days = legal_case
                .hearing_day
                .saturating_sub(state.clock.day())
                .max(0);
            60_i64.saturating_sub(days).saturating_mul(25)
        })
        .max()
        .unwrap_or(0)
}

pub(crate) fn family_council_urgency(state: &AppState) -> i64 {
    let unity = state
        .family_councils
        .get(&state.player_dynasty_id)
        .map_or(10_000, |council| council.unity_basis_points);
    match unity {
        0..=3_499 => 2_400,
        3_500..=5_499 => 1_600,
        5_500..=6_999 => 800,
        _ => 0,
    }
}

pub(crate) fn legal_case_urgency(state: &AppState) -> i64 {
    let player_id = state.player_dynasty_id;
    let has_defaulted_debt = state.loans.values().any(|loan| {
        loan.lender_dynasty_id == player_id
            && loan.status == LoanStatus::Defaulted
            && legal_grievance_kind(state, loan.borrower_dynasty_id) == Some(LegalCaseKind::Debt)
    });
    if has_defaulted_debt {
        return 1_200;
    }
    let has_unresolved_grievance = state
        .dynasties
        .keys()
        .copied()
        .filter(|dynasty_id| *dynasty_id != player_id)
        .any(|dynasty_id| legal_grievance_kind(state, dynasty_id).is_some());
    if has_unresolved_grievance { 800 } else { 0 }
}

pub(crate) fn labor_dispute_urgency(state: &AppState) -> i64 {
    if state
        .employment
        .values()
        .any(|agreement| agreement.status == EmploymentStatus::Disputed)
    {
        2_100
    } else {
        0
    }
}

pub(crate) fn business_policy_urgency(state: &AppState) -> i64 {
    if state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == state.player_dynasty_id
            && business.status() == BusinessStatus::Distressed
    }) {
        1_000
    } else {
        0
    }
}

pub(crate) fn impaired_business_urgency(state: &AppState, urgency: i64) -> i64 {
    if state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == state.player_dynasty_id
            && matches!(
                business.status(),
                BusinessStatus::Distressed | BusinessStatus::Insolvent
            )
    }) {
        urgency
    } else {
        0
    }
}

pub(crate) fn acquisition_urgency(state: &AppState) -> i64 {
    if state.businesses.iter().any(|business| {
        business.owner_dynasty_id() == state.player_dynasty_id
            && matches!(
                business.status(),
                BusinessStatus::Active | BusinessStatus::Distressed
            )
    }) {
        0
    } else {
        2_300
    }
}

pub(crate) fn notification_urgency(state: &AppState) -> i64 {
    if state
        .outbox
        .iter()
        .filter(|message| !message.acknowledged)
        .count()
        > 8
    {
        650
    } else {
        0
    }
}

pub(crate) fn borrowing_urgency(state: &AppState) -> i64 {
    if player_has_no_active_business(state) {
        3_000
    } else if state
        .dynasties
        .get(&state.player_dynasty_id)
        .is_some_and(|dynasty| dynasty.treasury() < Money::from_copper(8_000))
    {
        700
    } else {
        0
    }
}

pub(crate) fn institution_withdrawal_urgency(state: &AppState) -> i64 {
    if has_institution_withdrawal_pressure(state) {
        1_500
    } else {
        0
    }
}

/// Human-readable labels for the entities named in candidate descriptions and
/// decision traces. Each label pairs the world's proper name with its stable
/// identifier so a trace stays readable without becoming ambiguous.
pub(crate) fn character_label(state: &AppState, character_id: CharacterId) -> String {
    state.characters.get(character_id).map_or_else(
        || format!("character {character_id}"),
        |character| format!("{} ({character_id})", character.name()),
    )
}

pub(crate) fn dynasty_label(state: &AppState, dynasty_id: DynastyId) -> String {
    state.dynasties.get(&dynasty_id).map_or_else(
        || format!("dynasty {dynasty_id}"),
        |dynasty| {
            if dynasty_id == state.player_dynasty_id {
                format!("the player house ({dynasty_id})")
            } else {
                format!("House {} ({dynasty_id})", dynasty.name())
            }
        },
    )
}

pub(crate) fn business_label(state: &AppState, business_id: BusinessId) -> String {
    state.businesses.get(business_id).map_or_else(
        || format!("business {business_id}"),
        |business| format!("{} ({business_id})", business.name()),
    )
}

pub(crate) fn institution_label(registry: &Registry, institution_id: InstitutionId) -> String {
    registry.get_institution(institution_id).map_or_else(
        || format!("institution {institution_id}"),
        |institution| format!("{} ({institution_id})", institution.name()),
    )
}

pub(crate) fn district_label(registry: &Registry, district_id: DistrictId) -> String {
    registry.get_district(district_id).map_or_else(
        || format!("district {district_id}"),
        |district| format!("{} ({district_id})", district.name()),
    )
}

pub(crate) fn good_label(registry: &Registry, good_id: GoodId) -> String {
    registry
        .get_good(good_id)
        .map_or_else(|| format!("good {good_id}"), |good| good.name().to_owned())
}

pub(crate) fn push_candidate(
    candidates: &mut Vec<Candidate>,
    kind: GameplayCommandKind,
    command: PlayerCommand,
    description: String,
    score: i64,
) {
    candidates.push(Candidate {
        kind,
        command,
        description,
        score,
    });
}

pub(crate) fn crisis_responses(persona: GameplayPersona) -> [CrisisResponse; 4] {
    match persona {
        GameplayPersona::Steward => [
            CrisisResponse::Relief,
            CrisisResponse::Reform,
            CrisisResponse::Suppress,
            CrisisResponse::Exploit,
        ],
        GameplayPersona::Entrepreneur => [
            CrisisResponse::Reform,
            CrisisResponse::Relief,
            CrisisResponse::Exploit,
            CrisisResponse::Suppress,
        ],
        GameplayPersona::PowerBroker => [
            CrisisResponse::Suppress,
            CrisisResponse::Reform,
            CrisisResponse::Relief,
            CrisisResponse::Exploit,
        ],
        GameplayPersona::Opportunist => [
            CrisisResponse::Exploit,
            CrisisResponse::Suppress,
            CrisisResponse::Reform,
            CrisisResponse::Relief,
        ],
    }
}

pub(crate) fn crisis_response_bonus(persona: GameplayPersona, response: CrisisResponse) -> i64 {
    match persona {
        GameplayPersona::Steward => match response {
            CrisisResponse::Relief => 600,
            CrisisResponse::Exploit => -200,
            CrisisResponse::Reform | CrisisResponse::Suppress => 100,
        },
        GameplayPersona::Entrepreneur => match response {
            CrisisResponse::Reform => 500,
            CrisisResponse::Exploit => -200,
            CrisisResponse::Relief | CrisisResponse::Suppress => 100,
        },
        GameplayPersona::PowerBroker => match response {
            CrisisResponse::Suppress => 520,
            CrisisResponse::Exploit => -200,
            CrisisResponse::Relief | CrisisResponse::Reform => 100,
        },
        GameplayPersona::Opportunist => match response {
            CrisisResponse::Exploit => 700,
            CrisisResponse::Relief | CrisisResponse::Reform | CrisisResponse::Suppress => 100,
        },
    }
}

pub(crate) fn crisis_response_bonus_for_state(
    state: &AppState,
    persona: GameplayPersona,
    response: CrisisResponse,
) -> i64 {
    let base = crisis_response_bonus(persona, response);
    if response == CrisisResponse::Exploit {
        let food = crate::core::population_weighted_food_satisfaction_basis_points(
            state.households.iter(),
        )
        .unwrap_or(10_000);
        if food < 5_000 {
            return base.saturating_sub(900);
        }
        if food < 7_000 {
            return base.saturating_sub(400);
        }
    }
    base
}

pub(crate) fn labor_responses(persona: GameplayPersona) -> [LaborResponse; 3] {
    match persona {
        GameplayPersona::Steward => [
            LaborResponse::ImproveConditions,
            LaborResponse::Negotiate,
            LaborResponse::ReplaceWorkers,
        ],
        GameplayPersona::Entrepreneur | GameplayPersona::PowerBroker => [
            LaborResponse::Negotiate,
            LaborResponse::ImproveConditions,
            LaborResponse::ReplaceWorkers,
        ],
        GameplayPersona::Opportunist => [
            LaborResponse::ReplaceWorkers,
            LaborResponse::Negotiate,
            LaborResponse::ImproveConditions,
        ],
    }
}

pub(crate) fn labor_response_bonus(persona: GameplayPersona, response: LaborResponse) -> i64 {
    match persona {
        GameplayPersona::Steward => match response {
            LaborResponse::ImproveConditions => 500,
            LaborResponse::Negotiate | LaborResponse::ReplaceWorkers => 80,
        },
        GameplayPersona::Entrepreneur => match response {
            LaborResponse::Negotiate => 450,
            LaborResponse::ImproveConditions | LaborResponse::ReplaceWorkers => 80,
        },
        GameplayPersona::PowerBroker => match response {
            LaborResponse::Negotiate => 400,
            LaborResponse::ImproveConditions | LaborResponse::ReplaceWorkers => 80,
        },
        GameplayPersona::Opportunist => match response {
            LaborResponse::ReplaceWorkers => 550,
            LaborResponse::ImproveConditions | LaborResponse::Negotiate => 80,
        },
    }
}

pub(crate) const LAW_POWER_PENDING: &str = "law sponsorship office power not established";
pub(crate) const WORK_REQUIRES_OFFICE: &str = "public-work sponsorship requires office";
pub(crate) const WORK_REQUIRES_POWER: &str = "public-work sponsorship requires office power";
pub(crate) const WORK_POWER_PENDING: &str = "public-work office power not established";
pub(crate) const OFFICE_RECORD_SHORT: &str = "insufficient office commercial record";
pub(crate) const WARD_RECORD_SHORT: &str = "insufficient ward commercial record";
pub(crate) const OFFICE_DIRECTIVE_PENDING: &str = "office power directive not established";
pub(crate) const SUPPORT_REPUTATION_SHORT: &str = "insufficient institution-support reputation";
pub(crate) const SUPPORT_RECORD_SHORT: &str = "insufficient institution-support commercial record";
pub(crate) const SUPPORT_EXISTS: &str = "institution support already established";
pub(crate) const SUPPORT_MISSING: &str = "institution support not established";
pub(crate) const REPORT_UNCOMMISSIONED: &str = "intelligence report not commissioned";
pub(crate) const REPORT_NO_LEVERAGE: &str = "intelligence report has no leverage";
pub(crate) const LOAN_COLLATERAL_LARGE: &str = "loan counterparty collateral too large";
pub(crate) const LOAN_NO_FINANCING_NEED: &str = "loan counterparty has no financing need";
pub(crate) const CONTRACT_PENALTY: &str = "contract counterparty penalty";
pub(crate) const NO_BIZ: &str = "business unavailable";
pub(crate) const BAD_BIZ: &str = "invalid business command";
pub(crate) const NO_CIVIC_DEBT: &str = "civic debt unavailable";
pub(crate) const NO_TARGET: &str = "missing command target";
pub(crate) const BAD_WORK: &str = "invalid public work";

#[expect(
    clippy::too_many_lines,
    reason = "exhaustive command-error classification is safer than wildcard fallback helpers"
)]
pub(crate) const fn command_error_category(error: &CommandError) -> &'static str {
    match error {
        CommandError::IdentifierAllocation(_) => "identifier allocation exhausted",
        CommandError::Timeline(_) => "timeline range exhausted",
        CommandError::Strategic(source) => strategic_error_category(source),
        CommandError::Simulation(source) => simulation_error_category(source),
        CommandError::MissingBusiness { .. } | CommandError::BusinessNotOwned { .. } => NO_BIZ,
        CommandError::PlayerNotParty => "player not party",
        CommandError::LoanCounterpartyLenderReserve { .. } => "loan counterparty lender reserve",
        CommandError::LoanCounterpartyBorrowerInDefault { .. } => {
            "loan counterparty borrower in default"
        }
        CommandError::LoanCounterpartyInterestTooLow { .. } => "loan counterparty interest low",
        CommandError::LoanCounterpartyInterestTooHigh { .. } => "loan counterparty interest high",
        CommandError::LoanCounterpartyPaymentTooLow { .. } => "loan counterparty payment low",
        CommandError::LoanCounterpartyPaymentTooHigh { .. } => "loan counterparty payment high",
        CommandError::LoanCounterpartyCollateralTooLarge { .. } => LOAN_COLLATERAL_LARGE,
        CommandError::LoanCounterpartyNoFinancingNeed { .. } => LOAN_NO_FINANCING_NEED,
        CommandError::ContractCounterpartyPriceTooLow { .. } => "contract counterparty price low",
        CommandError::ContractCounterpartyPriceTooHigh { .. } => "contract counterparty price high",
        CommandError::ContractCounterpartyPenaltyOutOfRange { .. } => CONTRACT_PENALTY,
        CommandError::ContractCounterpartyCapacity { .. } => "contract counterparty capacity",
        CommandError::PropertyCounterpartyBuyerReserve { .. } => "property buyer reserve",
        CommandError::InvalidBusinessPolicy => BAD_BIZ,
        CommandError::UnchangedBusinessPolicy { .. } => "unchanged business policy",
        CommandError::BusinessPolicyCooldown { .. } => "business policy cooldown",
        CommandError::InvalidBusinessWage { .. } | CommandError::BusinessHasNoWorkforce { .. } => {
            BAD_BIZ
        }
        CommandError::UnchangedBusinessWage { .. } => "unchanged business wage",
        CommandError::BusinessWageCooldown { .. } => "business wage cooldown",
        CommandError::InvalidLawValue { .. } => "invalid law value",
        CommandError::UnchangedLaw { .. } => "unchanged law",
        CommandError::MissingCivicTreasury | CommandError::NoCivicDebtCreditor { .. } => {
            NO_CIVIC_DEBT
        }
        CommandError::CivicTreasuryOverflow { .. } => "civic treasury overflow",
        CommandError::LawSponsorshipRequiresOffice => "law sponsorship requires office",
        CommandError::LawSponsorshipRequiresPower { .. } => "law sponsorship requires office power",
        CommandError::LawSponsorshipPowerNotEstablished { .. } => LAW_POWER_PENDING,
        CommandError::LawCooldown { .. } => "law cooldown",
        CommandError::MissingDistrict { .. } | CommandError::MissingDynasty { .. } => NO_TARGET,
        CommandError::InsufficientPlayerFunds { .. } => "insufficient player funds",
        CommandError::InsufficientPlayerLegitimacy { .. } => "insufficient player legitimacy",
        CommandError::InsufficientFamilyUnity { .. } => "insufficient family unity",
        CommandError::InsufficientBusinessFunds { .. } => "insufficient business funds",
        CommandError::InvalidPublicWorkBudget { .. }
        | CommandError::PublicWorkFunding(_)
        | CommandError::PublicWorkCapacity { .. } => BAD_WORK,
        CommandError::PublicWorkSponsorshipRequiresOffice => WORK_REQUIRES_OFFICE,
        CommandError::PublicWorkSponsorshipRequiresPower => WORK_REQUIRES_POWER,
        CommandError::PublicWorkPowerNotEstablished { .. } => WORK_POWER_PENDING,
        CommandError::DuplicateActivePublicWork { .. } => "duplicate active public work",
        CommandError::PublicWorkCooldown { .. } => "public-work cooldown",
        CommandError::SameLegalParty | CommandError::InvalidLegalTerms => "invalid legal terms",
        CommandError::LegalClaimNotGrounded { .. }
        | CommandError::LegalEvidenceExceedsClaim { .. }
        | CommandError::LegalDamagesExceedClaim { .. } => "invalid legal claim",
        CommandError::DuplicateActiveLegalCase { .. } => "duplicate active legal case",
        CommandError::LegalCaseCooldown { .. } => "legal-case cooldown",
        CommandError::MissingLegalCase { .. } => "missing legal case",
        CommandError::MissingCharacter { .. } => "missing character",
        CommandError::LegalSettlementUnavailable { .. } => "legal settlement unavailable",
        CommandError::LegalSettlementNothingToSettle { .. } => {
            "legal settlement has nothing left to settle"
        }
        CommandError::LegalSettlementTreasuryOverflow { .. } => {
            "legal settlement treasury overflow"
        }
        CommandError::MissingFamilyCouncil { .. } => "missing family council",
        CommandError::UnchangedHouseGovernance { .. } => "unchanged governance",
        CommandError::HouseGovernanceCooldown { .. } => "governance cooldown",
        CommandError::FamilyCouncilMeetingCooldown { .. } => "family council cooldown",
        CommandError::InvalidHeirCandidate { .. } => "invalid heir candidate",
        CommandError::UnchangedHeir { .. } => "unchanged heir",
        CommandError::HeirDesignationCooldown { .. } => "heir designation cooldown",
        CommandError::InsufficientOfficeReputation { .. } => "insufficient office reputation",
        CommandError::InsufficientOfficeCommercialRecord { .. } => OFFICE_RECORD_SHORT,
        CommandError::OfficeNominationCooldown { .. } => "office nomination cooldown",
        CommandError::WardAdoptionCooldown { .. } => "ward adoption cooldown",
        CommandError::WardCapacity { .. } => "ward capacity",
        CommandError::InsufficientWardReputation { .. } => "insufficient ward reputation",
        CommandError::InsufficientWardCommercialRecord { .. } => WARD_RECORD_SHORT,
        CommandError::InvalidFamilyStudent { .. } => "invalid family student",
        CommandError::FamilyEducationAtMaximum { .. } => "family education at maximum",
        CommandError::FamilyEducationCooldown { .. } => "family education cooldown",
        CommandError::MissingInstitution { .. } => "missing institution",
        CommandError::OfficePowerUnavailable { .. } => "office power unavailable",
        CommandError::OfficePowerDirectiveNotEstablished { .. } => OFFICE_DIRECTIVE_PENDING,
        CommandError::OfficePowerDirectiveCooldown { .. } => "office power directive cooldown",
        CommandError::InsufficientInstitutionSupportReputation { .. } => SUPPORT_REPUTATION_SHORT,
        CommandError::InsufficientInstitutionSupportCommercialRecord { .. } => SUPPORT_RECORD_SHORT,
        CommandError::InstitutionSupportAlreadyEstablished { .. } => SUPPORT_EXISTS,
        CommandError::InstitutionMembershipCapacity { .. } => "institution membership capacity",
        CommandError::InstitutionSupportCooldown { .. } => "institution support cooldown",
        CommandError::InstitutionEndowmentOutOfRange { .. } => "institution endowment out of range",
        CommandError::InstitutionEndowmentRequiresMembership { .. } => {
            "institution endowment requires membership"
        }
        CommandError::InstitutionEndowmentCooldown { .. } => "institution endowment cooldown",
        CommandError::InstitutionBudgetOverflow { .. } => "institution budget overflow",
        CommandError::MissingInstitutionSupport { .. } => "missing institution support",
        CommandError::InstitutionSupportNotEstablished { .. } => SUPPORT_MISSING,
        CommandError::InvalidNominee { .. } => "invalid nominee",
        CommandError::NomineeAlreadyHoldsOffice { .. } => "nominee already holds office",
        CommandError::InvalidInstitutionWithdrawal { .. } => "invalid institution withdrawal",
        CommandError::MissingCrisis { .. } => "missing crisis",
        CommandError::InactiveCrisis { .. } => "inactive crisis",
        CommandError::CrisisAlreadyAddressed { .. } => "crisis already addressed",
        CommandError::MissingEmployment { .. } => "missing employment",
        CommandError::InvalidLaborDispute { .. } => "invalid labor dispute",
        CommandError::LaborWageOverflow { .. } => "labor wage overflow",
        CommandError::NoReplacementLaborAvailable { .. } => "no replacement labor available",
        CommandError::MissingGood { .. } => "missing good",
        CommandError::MissingMarketQuote { .. } => "missing market quote",
        CommandError::InformationCannotTargetPlayer => "invalid intelligence target",
        CommandError::InformationCommissionCooldown { .. } => "intelligence commission cooldown",
        CommandError::MissingInformationReport { .. } => "missing intelligence report",
        CommandError::InformationReportNotOwned { .. } => "intelligence report not owned",
        CommandError::InformationReportNotCommissioned { .. } => REPORT_UNCOMMISSIONED,
        CommandError::InformationReportExpired { .. } => "intelligence report expired",
        CommandError::InformationReportHasNoLeverage { .. } => REPORT_NO_LEVERAGE,
        CommandError::MissingNotification { .. } => "missing notification",
        CommandError::NotificationAlreadyAcknowledged { .. } => "notification already acknowledged",
        CommandError::MarketExtractionUnavailable { .. } => "market extraction unavailable",
    }
}

pub(crate) const fn strategic_error_category(error: &StrategicError) -> &'static str {
    match error {
        StrategicError::IdentifierAllocation(_) => "strategic: identifier allocation exhausted",
        StrategicError::Timeline(_) => "strategic: timeline range exhausted",
        StrategicError::Simulation(error) => simulation_error_category(error),
        StrategicError::RegistryMismatch { .. } => "strategic: registry mismatch",
        StrategicError::MissingBusiness { .. } => "strategic: missing business",
        StrategicError::BusinessInactive { .. } => "strategic: inactive business",
        StrategicError::BusinessNotOwnedByDynasty { .. } => {
            "strategic: business ownership mismatch"
        }
        StrategicError::MissingDynasty { .. } => "strategic: missing dynasty",
        StrategicError::MissingProperty { .. } => "strategic: missing property",
        StrategicError::SameContractParty => "strategic: same contract party",
        StrategicError::SameContractOwner { .. } => "strategic: same contract owner",
        StrategicError::SameLoanParty => "strategic: same loan party",
        StrategicError::ExistingUnsettledLoan { .. } => "strategic: existing unsettled loan",
        StrategicError::DefaultedLoanRestructuringCooldown { .. } => {
            "strategic: restructuring cooldown"
        }
        StrategicError::LoanBalanceOverflow { .. } => "strategic: loan balance overflow",
        StrategicError::NonPositiveAmount => "strategic: nonpositive amount",
        StrategicError::NonPositiveQuantity => "strategic: nonpositive quantity",
        StrategicError::EmptyContractDuration => "strategic: empty contract duration",
        StrategicError::ContractPaymentOverflow { .. } => "strategic: contract payment overflow",
        StrategicError::SellerCannotProduce { .. } => "strategic: seller cannot produce",
        StrategicError::BuyerDoesNotConsume { .. } => "strategic: buyer does not consume",
        StrategicError::InsufficientDynastyFunds { .. } => "strategic: insufficient dynasty funds",
        StrategicError::DynastyTreasuryOverflow { .. } => "strategic: dynasty treasury overflow",
        StrategicError::BusinessCashOverflow { .. } => "strategic: business cash overflow",
        StrategicError::BusinessFinanceVersionExhausted { .. } => {
            "strategic: business finance version exhausted"
        }
        StrategicError::BusinessDistributionExceedsSurplus { .. } => {
            "strategic: business distribution exceeds surplus"
        }
        StrategicError::DynastyAdministrativeLoadUnderflow { .. } => {
            "strategic: administrative load underflow"
        }
        StrategicError::DynastyAdministrativeLoadOverflow { .. } => {
            "strategic: administrative load overflow"
        }
        StrategicError::AcquisitionCostOverflow { .. } => "strategic: acquisition cost overflow",
        StrategicError::BusinessValuationOverflow { .. } => {
            "strategic: business valuation overflow"
        }
        StrategicError::InterestOutOfRange { .. } => "strategic: interest out of range",
        StrategicError::CollateralNotOwned { .. } => "strategic: collateral not owned",
        StrategicError::PropertyAlreadyPledged { .. } => "strategic: property already pledged",
        StrategicError::PropertyAlreadyOwned { .. } => "strategic: property already owned",
        StrategicError::PropertyNotOwnedBySeller { .. } => {
            "strategic: property not owned by seller"
        }
        StrategicError::SamePropertyParty => "strategic: same property party",
        StrategicError::MissingCivicTreasury => "strategic: missing civic treasury",
        StrategicError::InsufficientPropertyAuctionLiquidity { .. } => {
            "strategic: insufficient property auction liquidity"
        }
        StrategicError::MissingCollateralLoan { .. } => "strategic: missing collateral loan",
        StrategicError::PropertyLienBorrowerMismatch { .. } => {
            "strategic: property lien borrower mismatch"
        }
        StrategicError::PropertySaleCannotSettleLien { .. } => {
            "strategic: property sale cannot settle lien"
        }
        StrategicError::BusinessAlreadyOwned { .. } => "strategic: business already owned",
        StrategicError::InvalidAcquisitionManager { .. } => {
            "strategic: invalid acquisition manager"
        }
        StrategicError::InsufficientBusinessRecapitalization { .. } => {
            "strategic: insufficient business recapitalization"
        }
    }
}

pub(crate) fn inject_exploratory_candidates(
    registry: &Registry,
    state: &AppState,
    persona: GameplayPersona,
    accumulator: &CampaignAccumulator,
    candidates: &mut Vec<Candidate>,
) {
    if candidates
        .iter()
        .any(|c| c.kind == GameplayCommandKind::BorrowFunds)
    {
        return;
    }
    if !has_borrow_opportunity(state) {
        return;
    }
    // Deterministic exploration roll derived from the campaign RNG
    // and cycle state so the same world/persona pair does not replay
    // identical restraint every cycle. ~12% chance.
    let mut rng = state.rng;
    let mut roll = rng.next_u64();
    roll = roll.wrapping_add(state.clock.day().cast_unsigned());
    roll = roll
        .wrapping_add(u64::from(accumulator.decision_cycles).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    roll ^= u64::from(accumulator.quiet_cycles).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    for b in persona.label().bytes() {
        roll = roll.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(11) ^ u64::from(b);
    }
    if roll % 8 != 0 {
        return;
    }
    let player_id = state.player_dynasty_id;
    let Some(lender) = state
        .dynasties
        .values()
        .filter(|d| d.id() != player_id)
        .filter(|d| !credit_pair_blocks_new_loan(state, d.id(), player_id))
        .filter(|d| unresolved_default_owed_elsewhere(state, player_id, d.id()).is_none())
        .filter(|d| {
            d.treasury()
                .checked_sub(PRIVATE_LOAN_COUNTERPARTY_RESERVE)
                .is_some_and(|a| a >= Money::from_copper(1_000))
        })
        .max_by_key(|d| d.treasury())
    else {
        return;
    };
    let available = lender
        .treasury()
        .checked_sub(PRIVATE_LOAN_COUNTERPARTY_RESERVE)
        .unwrap_or(Money::ZERO);
    let principal = Money::from_copper((available.copper() / 10).clamp(1_000, 8_000));
    if principal <= Money::ZERO {
        return;
    }
    push_candidate(
        candidates,
        GameplayCommandKind::BorrowFunds,
        PlayerCommand::IssueLoan {
            terms: LoanTerms {
                lender_dynasty_id: lender.id(),
                borrower_dynasty_id: player_id,
                principal,
                weekly_payment: principal.ceil_div_positive(AGENT_LOAN_AMORTIZATION_WEEKS),
                interest_basis_points: 700,
                collateral_property_id: unpledged_player_property(state).map(|p| p.id),
            },
        },
        format!(
            "exploratory borrow {principal} from {} (organic variation)",
            dynasty_label(state, lender.id())
        ),
        50,
    );
    // Keep harness bounded: more than one exploratory injection per cycle
    // would defeat the probe budget and persona priorities. The single
    // low-score borrow is enough to prove the route was sampled without
    // dominating urgency-driven selection.
    let _ = registry;
}

pub(crate) const fn simulation_error_category(error: &SimulationError) -> &'static str {
    match error {
        SimulationError::IdentifierAllocation(_) => "simulation: identifier allocation exhausted",
        SimulationError::Timeline(_) => "simulation: timeline range exhausted",
        SimulationError::InvalidDayCount { .. } => "simulation: invalid day count",
        SimulationError::DayRangeExhausted { .. } => "simulation: day range exhausted",
        SimulationError::RegistryMismatch { .. } => "simulation: registry mismatch",
        SimulationError::BusinessNotFound { .. } => "simulation: business not found",
        SimulationError::BusinessInactive { .. } => "simulation: inactive business",
        SimulationError::SameBusiness { .. } => "simulation: same business",
        SimulationError::NonPositiveAmount { .. } => "simulation: nonpositive amount",
        SimulationError::InsufficientBusinessCash { .. } => {
            "simulation: insufficient business cash"
        }
        SimulationError::BusinessCashOverflow { .. } => "simulation: business cash overflow",
        SimulationError::BusinessInventoryOverflow { .. } => {
            "simulation: business inventory overflow"
        }
        SimulationError::BusinessLifetimeCostsOverflow { .. } => {
            "simulation: business lifetime costs overflow"
        }
        SimulationError::BusinessLifetimeRevenueOverflow { .. } => {
            "simulation: business lifetime revenue overflow"
        }
        SimulationError::StaleBusinessFinance { .. } => "simulation: stale business finance",
        SimulationError::BusinessFinanceVersionExhausted { .. } => {
            "simulation: business finance version exhausted"
        }
        SimulationError::FamilyCharterVersionExhausted { .. } => {
            "simulation: family charter version exhausted"
        }
        SimulationError::DynastyGenerationExhausted { .. } => {
            "simulation: dynasty generation exhausted"
        }
        SimulationError::DynastyCivicContributionsOverflow { .. } => {
            "simulation: civic contributions overflow"
        }
        SimulationError::DynastyTreasuryOverflow { .. } => "simulation: dynasty treasury overflow",
        SimulationError::HouseholdCashOverflow { .. } => "simulation: household cash overflow",
        SimulationError::InstitutionBudgetOverflow { .. } => {
            "simulation: institution budget overflow"
        }
        SimulationError::InstitutionTermNumberExhausted { .. } => {
            "simulation: institution term number exhausted"
        }
        SimulationError::MarketQuoteMissing { .. } => "simulation: missing market quote",
        SimulationError::NegativeMarketDebit { .. } => "simulation: negative market debit",
        SimulationError::NegativeMarketCredit { .. } => "simulation: negative market credit",
        SimulationError::NegativeMarketSupply { .. } => "simulation: negative market supply",
        SimulationError::MarketDemandOverflow { .. } => "simulation: market demand overflow",
        SimulationError::MarketStockOverflow { .. } => "simulation: market stock overflow",
        SimulationError::MarketSupplyOverflow { .. } => "simulation: market supply overflow",
        SimulationError::MarketTradeValueOverflow { .. } => {
            "simulation: market trade value overflow"
        }
        SimulationError::WeeklyExternalIncomeOverflow { .. } => {
            "simulation: weekly external income overflow"
        }
        SimulationError::HouseholdLivingCostOverflow { .. } => {
            "simulation: household living cost overflow"
        }
        SimulationError::LoanBalanceOverflow { .. } => "simulation: loan balance overflow",
        SimulationError::CivicDebtBalanceOverflow { .. } => {
            "simulation: civic debt balance overflow"
        }
        SimulationError::MarketClearingAccountOverflow { .. } => {
            "simulation: market clearing account overflow"
        }
    }
}
