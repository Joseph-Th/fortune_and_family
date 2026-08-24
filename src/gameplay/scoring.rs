//! Part of the gameplay harness module tree.

#[allow(clippy::wildcard_imports)] // the module tree re-exports one flat namespace
use super::*;

pub(crate) fn score_campaign(
    accumulator: &CampaignAccumulator,
    start: &GameplaySnapshot,
    end: &GameplaySnapshot,
) -> GameplayScores {
    let executed: u32 = accumulator
        .commands
        .iter()
        .filter(|(kind, _)| is_substantive_command_kind(**kind))
        .map(|(_, stats)| stats.executed)
        .sum();
    let opportunity_cycles = accumulator
        .decision_cycles
        .saturating_sub(accumulator.quiet_cycles);
    let opportunity_conversion = if opportunity_cycles == 0 {
        100
    } else {
        ratio_score(accumulator.cycles_with_viable_choices, opportunity_cycles)
    };
    let strategic_cadence = ratio_score(
        accumulator.cycles_with_viable_choices,
        accumulator.decision_cycles,
    );
    let actionability = average_scores(&[opportunity_conversion, strategic_cadence]);
    let command_coverage = usize_to_u16(
        accumulator
            .commands
            .iter()
            .filter(|(kind, stats)| is_substantive_command_kind(**kind) && stats.executed > 0)
            .count(),
    );
    let substantive_kind_count = usize_to_u32(
        ALL_COMMAND_KINDS
            .iter()
            .filter(|kind| is_substantive_command_kind(**kind))
            .count(),
    );
    let coverage_score = ratio_score(u32::from(command_coverage), substantive_kind_count);
    let dominant_actions = accumulator
        .commands
        .iter()
        .filter(|(kind, _)| is_substantive_command_kind(**kind))
        .map(|(_, stats)| stats.executed)
        .max()
        .unwrap_or(0);
    // With fewer than two substantive executions there is no distribution to
    // be dominated, so the term stays neutral instead of punishing a campaign
    // that simply had one decision.
    let distribution_score = if executed < 2 {
        100
    } else {
        100_u16.saturating_sub(ratio_score(dominant_actions, executed))
    };
    let choice_richness = ratio_score(
        accumulator.total_viable_command_kinds,
        opportunity_cycles.saturating_mul(3),
    );
    let concrete_consequence_diversity = if accumulator.cycles_with_multiple_viable_options == 0 {
        choice_richness
    } else {
        ratio_score(
            accumulator.cycles_with_distinct_projected_option_consequences,
            accumulator.cycles_with_multiple_viable_options,
        )
    };
    let variety = average_scores(&[
        coverage_score,
        distribution_score,
        choice_richness,
        concrete_consequence_diversity,
    ]);
    let interconnection = campaign_interconnection_score(accumulator, executed, command_coverage);
    let feedback = campaign_feedback_score(accumulator, executed);
    let resilience = resilience_score(accumulator, start, end);
    let overall = weighted_overall(
        actionability,
        variety,
        interconnection,
        feedback,
        resilience,
    );
    GameplayScores {
        actionability,
        variety,
        interconnection,
        feedback,
        resilience,
        overall,
    }
}

pub(crate) fn campaign_interconnection_score(
    accumulator: &CampaignAccumulator,
    executed: u32,
    command_coverage: u16,
) -> u16 {
    let systemic_interactions = accumulator
        .interactions
        .iter()
        .filter(|((_, domain), _)| *domain != GameplayDomain::Feedback);
    interconnection_score(
        usize_to_u32(systemic_interactions.clone().count()),
        systemic_interactions.map(|(_, count)| *count).sum(),
        executed,
        u32::from(command_coverage),
    )
}

pub(crate) fn campaign_feedback_score(accumulator: &CampaignAccumulator, executed: u32) -> u16 {
    let feedback_actions: u32 = accumulator
        .commands
        .iter()
        .filter(|(kind, _)| is_substantive_command_kind(**kind))
        .map(|(_, stats)| stats.actions_with_feedback)
        .sum();
    let delayed_actions: u32 = accumulator
        .commands
        .iter()
        .filter(|(kind, _)| is_substantive_command_kind(**kind))
        .map(|(_, stats)| stats.actions_with_delayed_consequences)
        .sum();
    let visible_feedback = ratio_score(feedback_actions, executed);
    let delayed_feedback = ratio_score(delayed_actions, executed);
    u16::try_from((u32::from(visible_feedback) * 3 + u32::from(delayed_feedback)) / 4)
        .unwrap_or(100)
        .min(100)
}

pub(crate) fn interconnection_score(
    edge_count: u32,
    observations: u32,
    executed: u32,
    executed_kinds: u32,
) -> u16 {
    if executed == 0 || executed_kinds == 0 {
        return 0;
    }
    let target_edges = executed_kinds.saturating_mul(7);
    let edge_coverage = ratio_score(edge_count, target_edges);
    let target_observations = executed.saturating_mul(5);
    let breadth = ratio_score(observations, target_observations);
    average_scores(&[edge_coverage, breadth])
}

pub(crate) fn resilience_score(
    accumulator: &CampaignAccumulator,
    start: &GameplaySnapshot,
    end: &GameplaySnapshot,
) -> u16 {
    let business = if end.active_businesses > 0 && end.distressed_businesses == 0 {
        100
    } else if end.active_businesses > 0 {
        80
    } else if end.distressed_businesses > 0 {
        25
    } else {
        0
    };
    let debt = if end.player_defaulted_borrowing > 0 {
        0
    } else if end.player_delinquent_borrowing > 0 {
        35
    } else if end.player_restructured_borrowing > 0 {
        70
    } else if end.player_current_borrowing > 0 {
        85
    } else {
        100
    };
    let debt_trajectory = if accumulator.maximum_player_defaulted_borrowing > 0 {
        40
    } else if accumulator.maximum_player_delinquent_borrowing > 0 {
        70
    } else {
        100
    };
    let condition = (end.average_business_condition / 100).min(100);
    let food = end.average_food_satisfaction / 100;
    let treasury = if end.player_treasury >= start.player_treasury {
        100
    } else if end.player_treasury > Money::ZERO {
        60
    } else {
        0
    };
    let crisis = if end.escalated_crises == 0 { 100 } else { 35 };
    let civic = average_scores(&[
        (end.average_district_employment / 100).min(100),
        (end.average_district_sanitation / 100).min(100),
        (end.average_district_safety / 100).min(100),
        100_u16.saturating_sub(end.average_district_unrest / 100),
    ]);
    let trajectory = average_scores(&[
        accumulator.minimum_food_satisfaction / 100,
        accumulator.minimum_district_food_satisfaction / 100,
        if accumulator.minimum_operating_businesses > 0 {
            100
        } else {
            0
        },
        // Player-facing resilience measures the player's own labor disputes;
        // city-wide employment waves belong to civic-health findings, where
        // they do not blame the player for conditions it cannot fix.
        100_u16.saturating_sub(
            accumulator
                .maximum_player_disputed_employment
                .saturating_mul(8),
        ),
        100_u16.saturating_sub(accumulator.maximum_active_crises.saturating_mul(15)),
        debt_trajectory,
    ]);
    average_scores(&[
        business,
        condition,
        food.min(100),
        treasury,
        debt,
        crisis,
        civic,
        trajectory,
    ])
}

pub(crate) fn weighted_overall(
    actionability: u16,
    variety: u16,
    interconnection: u16,
    feedback: u16,
    resilience: u16,
) -> u16 {
    let total = u32::from(actionability) * 20
        + u32::from(variety) * 20
        + u32::from(interconnection) * 20
        + u32::from(feedback) * 15
        + u32::from(resilience) * 25;
    u16::try_from(total / 100).unwrap_or(100).min(100)
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AggregateCycleTotals {
    pub simulated_days: u64,
    pub decision_cycles: u64,
    pub viable_choices: u64,
    pub viable_command_kinds: u64,
    pub no_action_cycles: u64,
    pub quiet_cycles: u64,
    pub quiet_cycles_with_ambient_change: u64,
    pub blocked_cycles: u64,
    pub multiple_viable_command_kinds: u64,
    pub close_viable_command_kinds: u64,
    pub distinct_immediate_consequences: u64,
    pub distinct_projected_consequences: u64,
    pub multiple_viable_options: u64,
    pub close_viable_options: u64,
    pub distinct_immediate_option_consequences: u64,
    pub distinct_projected_option_consequences: u64,
}

impl AggregateCycleTotals {
    pub fn add_campaign(&mut self, campaign: &GameplayCampaignReport) {
        self.simulated_days = self
            .simulated_days
            .saturating_add(u64::from(campaign.simulated_days));
        self.decision_cycles = self
            .decision_cycles
            .saturating_add(u64::from(campaign.decision_cycles));
        self.viable_choices = self
            .viable_choices
            .saturating_add(u64::from(campaign.total_viable_choices));
        self.viable_command_kinds = self
            .viable_command_kinds
            .saturating_add(u64::from(campaign.total_viable_command_kinds));
        self.no_action_cycles = self
            .no_action_cycles
            .saturating_add(u64::from(campaign.no_action_cycles));
        self.quiet_cycles = self
            .quiet_cycles
            .saturating_add(u64::from(campaign.quiet_cycles));
        self.quiet_cycles_with_ambient_change = self
            .quiet_cycles_with_ambient_change
            .saturating_add(u64::from(campaign.quiet_cycles_with_ambient_change));
        self.blocked_cycles = self
            .blocked_cycles
            .saturating_add(u64::from(campaign.blocked_cycles));
        self.multiple_viable_command_kinds = self.multiple_viable_command_kinds.saturating_add(
            u64::from(campaign.cycles_with_multiple_viable_command_kinds),
        );
        self.close_viable_command_kinds = self
            .close_viable_command_kinds
            .saturating_add(u64::from(campaign.cycles_with_close_viable_command_kinds));
        self.distinct_immediate_consequences =
            self.distinct_immediate_consequences
                .saturating_add(u64::from(
                    campaign.cycles_with_distinct_immediate_consequences,
                ));
        self.distinct_projected_consequences =
            self.distinct_projected_consequences
                .saturating_add(u64::from(
                    campaign.cycles_with_distinct_projected_consequences,
                ));
        self.multiple_viable_options = self
            .multiple_viable_options
            .saturating_add(u64::from(campaign.cycles_with_multiple_viable_options));
        self.close_viable_options = self
            .close_viable_options
            .saturating_add(u64::from(campaign.cycles_with_close_viable_options));
        self.distinct_immediate_option_consequences = self
            .distinct_immediate_option_consequences
            .saturating_add(u64::from(
                campaign.cycles_with_distinct_immediate_option_consequences,
            ));
        self.distinct_projected_option_consequences = self
            .distinct_projected_option_consequences
            .saturating_add(u64::from(
                campaign.cycles_with_distinct_projected_option_consequences,
            ));
    }
}

pub(crate) fn aggregate_campaigns(campaigns: &[GameplayCampaignReport]) -> GameplayAggregate {
    let mut commands = initialized_command_stats();
    let mut phase_stats = initialized_phase_stats();
    let mut rejection_reasons = BTreeMap::new();
    let mut domain_changes = initialized_domain_counts();
    let mut causal_domain_changes = initialized_domain_counts();
    let mut ambient_domain_changes = initialized_domain_counts();
    let mut interactions = BTreeMap::new();
    let mut totals = AggregateCycleTotals::default();
    for campaign in campaigns {
        merge_phase_stats(campaign, &mut phase_stats);
        merge_campaign(
            campaign,
            &mut commands,
            &mut rejection_reasons,
            &mut domain_changes,
            &mut causal_domain_changes,
            &mut ambient_domain_changes,
            &mut interactions,
        );
        totals.add_campaign(campaign);
    }
    let successful_actions = commands
        .values()
        .map(|stats| u64::from(stats.executed))
        .sum();
    let substantive_actions = commands
        .iter()
        .filter(|(kind, _)| is_substantive_command_kind(**kind))
        .map(|(_, stats)| u64::from(stats.executed))
        .sum();
    let candidate_probes = commands
        .values()
        .map(|stats| u64::from(stats.considered))
        .sum();
    let command_coverage = usize_to_u16(
        commands
            .iter()
            .filter(|(kind, stats)| is_substantive_command_kind(**kind) && stats.executed > 0)
            .count(),
    );
    let domain_coverage = usize_to_u16(domain_changes.values().filter(|count| **count > 0).count());
    let causal_domain_coverage = usize_to_u16(
        causal_domain_changes
            .values()
            .filter(|count| **count > 0)
            .count(),
    );
    let ambient_domain_coverage = usize_to_u16(
        ambient_domain_changes
            .values()
            .filter(|count| **count > 0)
            .count(),
    );
    let scores = aggregate_scores(campaigns);
    let quiet_diagnostic = aggregate_quiet_diagnostics(campaigns);
    GameplayAggregate {
        campaigns: usize_to_u32(campaigns.len()),
        simulated_days: totals.simulated_days,
        decision_cycles: totals.decision_cycles,
        successful_actions,
        substantive_actions,
        candidate_probes,
        viable_choices: totals.viable_choices,
        viable_command_kinds: totals.viable_command_kinds,
        phase_stats,
        no_action_cycles: totals.no_action_cycles,
        quiet_cycles: totals.quiet_cycles,
        quiet_cycles_with_ambient_change: totals.quiet_cycles_with_ambient_change,
        blocked_cycles: totals.blocked_cycles,
        cycles_with_multiple_viable_command_kinds: totals.multiple_viable_command_kinds,
        cycles_with_close_viable_command_kinds: totals.close_viable_command_kinds,
        cycles_with_distinct_immediate_consequences: totals.distinct_immediate_consequences,
        cycles_with_distinct_projected_consequences: totals.distinct_projected_consequences,
        cycles_with_multiple_viable_options: totals.multiple_viable_options,
        cycles_with_close_viable_options: totals.close_viable_options,
        cycles_with_distinct_immediate_option_consequences: totals
            .distinct_immediate_option_consequences,
        cycles_with_distinct_projected_option_consequences: totals
            .distinct_projected_option_consequences,
        command_coverage,
        domain_coverage,
        commands,
        rejection_reasons,
        domain_changes,
        causal_domain_changes,
        ambient_domain_changes,
        causal_domain_coverage,
        ambient_domain_coverage,
        interactions: interaction_vec(&interactions),
        quiet_diagnostic,
        scores,
    }
}

pub(crate) fn aggregate_quiet_diagnostics(
    campaigns: &[GameplayCampaignReport],
) -> GameplayQuietDiagnostic {
    let mut diagnostic = GameplayQuietDiagnostic::default();
    for campaign in campaigns {
        for (kind, count) in &campaign.quiet_diagnostic.generator_gaps {
            *diagnostic.generator_gaps.entry(*kind).or_default() = diagnostic
                .generator_gaps
                .get(kind)
                .copied()
                .unwrap_or(0)
                .saturating_add(*count);
        }
        for (kind, count) in &campaign.quiet_diagnostic.policy_gates {
            *diagnostic.policy_gates.entry(*kind).or_default() = diagnostic
                .policy_gates
                .get(kind)
                .copied()
                .unwrap_or(0)
                .saturating_add(*count);
        }
        for (kind, count) in &campaign.quiet_diagnostic.restrained_routes {
            *diagnostic.restrained_routes.entry(*kind).or_default() = diagnostic
                .restrained_routes
                .get(kind)
                .copied()
                .unwrap_or(0)
                .saturating_add(*count);
        }
        for (kind, count) in &campaign.quiet_diagnostic.validation_gates {
            *diagnostic.validation_gates.entry(*kind).or_default() = diagnostic
                .validation_gates
                .get(kind)
                .copied()
                .unwrap_or(0)
                .saturating_add(*count);
        }
        for (kind, count) in &campaign.quiet_diagnostic.budget_gates {
            *diagnostic.budget_gates.entry(*kind).or_default() = diagnostic
                .budget_gates
                .get(kind)
                .copied()
                .unwrap_or(0)
                .saturating_add(*count);
        }
        diagnostic.dormant_cycles = diagnostic
            .dormant_cycles
            .saturating_add(campaign.quiet_diagnostic.dormant_cycles);
    }
    diagnostic
}

pub(crate) fn aggregate_campaigns_by_persona(
    campaigns: &[GameplayCampaignReport],
) -> BTreeMap<GameplayPersona, GameplayAggregate> {
    GameplayPersona::all()
        .into_iter()
        .filter_map(|persona| {
            let persona_campaigns: Vec<_> = campaigns
                .iter()
                .filter(|campaign| campaign.persona == persona)
                .cloned()
                .collect();
            (!persona_campaigns.is_empty())
                .then(|| (persona, aggregate_campaigns(&persona_campaigns)))
        })
        .collect()
}

pub(crate) fn merge_phase_stats(
    campaign: &GameplayCampaignReport,
    phase_stats: &mut BTreeMap<GameplayPhase, GameplayPhaseStats>,
) {
    for (phase, source) in &campaign.phase_stats {
        let target = phase_stats
            .get_mut(phase)
            .expect("every gameplay phase must have aggregate statistics");
        target.decision_cycles = target
            .decision_cycles
            .saturating_add(source.decision_cycles);
        target.substantive_actions = target
            .substantive_actions
            .saturating_add(source.substantive_actions);
        target.institutional_campaign_actions = target
            .institutional_campaign_actions
            .saturating_add(source.institutional_campaign_actions);
        target.quiet_cycles = target.quiet_cycles.saturating_add(source.quiet_cycles);
        target.quiet_cycles_with_ambient_change = target
            .quiet_cycles_with_ambient_change
            .saturating_add(source.quiet_cycles_with_ambient_change);
        target.longest_quiet_streak_cycles = target
            .longest_quiet_streak_cycles
            .max(source.longest_quiet_streak_cycles);
        target.blocked_cycles = target.blocked_cycles.saturating_add(source.blocked_cycles);
        target.generator_gap_cycles = target
            .generator_gap_cycles
            .saturating_add(source.generator_gap_cycles);
        target.policy_gate_cycles = target
            .policy_gate_cycles
            .saturating_add(source.policy_gate_cycles);
        target.restrained_cycles = target
            .restrained_cycles
            .saturating_add(source.restrained_cycles);
        target.validation_gate_cycles = target
            .validation_gate_cycles
            .saturating_add(source.validation_gate_cycles);
        target.dormant_cycles = target.dormant_cycles.saturating_add(source.dormant_cycles);
        target.cycles_with_multiple_viable_command_kinds = target
            .cycles_with_multiple_viable_command_kinds
            .saturating_add(source.cycles_with_multiple_viable_command_kinds);
        target.cycles_with_close_viable_command_kinds = target
            .cycles_with_close_viable_command_kinds
            .saturating_add(source.cycles_with_close_viable_command_kinds);
        target.cycles_with_distinct_immediate_consequences = target
            .cycles_with_distinct_immediate_consequences
            .saturating_add(source.cycles_with_distinct_immediate_consequences);
        target.cycles_with_distinct_projected_consequences = target
            .cycles_with_distinct_projected_consequences
            .saturating_add(source.cycles_with_distinct_projected_consequences);
        target.cycles_with_multiple_viable_options = target
            .cycles_with_multiple_viable_options
            .saturating_add(source.cycles_with_multiple_viable_options);
        target.cycles_with_close_viable_options = target
            .cycles_with_close_viable_options
            .saturating_add(source.cycles_with_close_viable_options);
        target.cycles_with_distinct_immediate_option_consequences = target
            .cycles_with_distinct_immediate_option_consequences
            .saturating_add(source.cycles_with_distinct_immediate_option_consequences);
        target.cycles_with_distinct_projected_option_consequences = target
            .cycles_with_distinct_projected_option_consequences
            .saturating_add(source.cycles_with_distinct_projected_option_consequences);
        target.total_viable_choices = target
            .total_viable_choices
            .saturating_add(source.total_viable_choices);
        target.total_viable_command_kinds = target
            .total_viable_command_kinds
            .saturating_add(source.total_viable_command_kinds);
        for (kind, count) in &source.executed_commands {
            let total = target.executed_commands.entry(*kind).or_default();
            *total = total.saturating_add(*count);
        }
    }
}

pub(crate) fn merge_campaign(
    campaign: &GameplayCampaignReport,
    commands: &mut BTreeMap<GameplayCommandKind, GameplayCommandStats>,
    rejections: &mut BTreeMap<String, u32>,
    domains: &mut BTreeMap<GameplayDomain, u32>,
    causal_domains: &mut BTreeMap<GameplayDomain, u32>,
    ambient_domains: &mut BTreeMap<GameplayDomain, u32>,
    interactions: &mut BTreeMap<(GameplayCommandKind, GameplayDomain), u32>,
) {
    for (kind, source) in &campaign.commands {
        let target = commands
            .get_mut(kind)
            .expect("every command kind must have aggregate statistics");
        target.activation_opportunities = target
            .activation_opportunities
            .saturating_add(source.activation_opportunities);
        target.offered_cycles = target.offered_cycles.saturating_add(source.offered_cycles);
        target.generated = target.generated.saturating_add(source.generated);
        target.considered = target.considered.saturating_add(source.considered);
        target.viable = target.viable.saturating_add(source.viable);
        target.executed = target.executed.saturating_add(source.executed);
        target.rejected = target.rejected.saturating_add(source.rejected);
        target.immediate_world_feedback = target
            .immediate_world_feedback
            .saturating_add(source.immediate_world_feedback);
        target.delayed_world_feedback = target
            .delayed_world_feedback
            .saturating_add(source.delayed_world_feedback);
        target.actions_with_feedback = target
            .actions_with_feedback
            .saturating_add(source.actions_with_feedback);
        target.actions_with_persistent_consequences = target
            .actions_with_persistent_consequences
            .saturating_add(source.actions_with_persistent_consequences);
        target.actions_with_delayed_consequences = target
            .actions_with_delayed_consequences
            .saturating_add(source.actions_with_delayed_consequences);
        target.productive_financing_actions = target
            .productive_financing_actions
            .saturating_add(source.productive_financing_actions);
        target.nonproductive_financing_actions = target
            .nonproductive_financing_actions
            .saturating_add(source.nonproductive_financing_actions);
        target.changed_domains.extend(&source.changed_domains);
    }
    for (reason, count) in &campaign.rejection_reasons {
        *rejections.entry(reason.clone()).or_default() += count;
    }
    for (domain, count) in &campaign.domain_changes {
        *domains.entry(*domain).or_default() += count;
    }
    for (domain, count) in &campaign.causal_domain_changes {
        *causal_domains.entry(*domain).or_default() += count;
    }
    for (domain, count) in &campaign.ambient_domain_changes {
        *ambient_domains.entry(*domain).or_default() += count;
    }
    for edge in &campaign.interactions {
        *interactions.entry((edge.command, edge.domain)).or_default() += edge.observations;
    }
}

pub(crate) fn aggregate_scores(campaigns: &[GameplayCampaignReport]) -> GameplayScores {
    if campaigns.is_empty() {
        return GameplayScores {
            actionability: 0,
            variety: 0,
            interconnection: 0,
            feedback: 0,
            resilience: 0,
            overall: 0,
        };
    }
    GameplayScores {
        actionability: average_u16(
            campaigns
                .iter()
                .map(|campaign| campaign.scores.actionability),
        ),
        variety: average_u16(campaigns.iter().map(|campaign| campaign.scores.variety)),
        interconnection: average_u16(
            campaigns
                .iter()
                .map(|campaign| campaign.scores.interconnection),
        ),
        feedback: average_u16(campaigns.iter().map(|campaign| campaign.scores.feedback)),
        resilience: average_u16(campaigns.iter().map(|campaign| campaign.scores.resilience)),
        overall: average_u16(campaigns.iter().map(|campaign| campaign.scores.overall)),
    }
}
