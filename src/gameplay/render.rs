//! Part of the gameplay harness module tree.

#[allow(clippy::wildcard_imports)] // the module tree re-exports one flat namespace
use super::*;

/// Renders a compact, human-readable gameplay report suitable for CI logs and design review.
#[must_use]
pub fn render_gameplay_report(report: &GameplayHarnessReport) -> String {
    let mut output = String::new();
    render_report_header(report, &mut output);
    render_persona_summary(report, &mut output);
    render_phase_summary(report, &mut output);
    render_health_summary(report, &mut output);
    render_command_table(report, &mut output);
    render_domain_table(report, &mut output);
    render_interactions(report, &mut output);
    render_rejections(report, &mut output);
    render_quiet_diagnosis(report, &mut output);
    render_findings(report, &mut output);
    render_limitations(report, &mut output);
    render_fantasy_arcs(report, &mut output);
    render_campaign_summaries(report, &mut output);
    render_decision_log(report, &mut output);
    output
}

pub(crate) fn render_persona_summary(report: &GameplayHarnessReport, output: &mut String) {
    if report.persona_aggregates.is_empty() {
        return;
    }
    let _ = writeln!(output, "Persona comparison");
    for persona in GameplayPersona::all() {
        let Some(aggregate) = report.persona_aggregates.get(&persona) else {
            continue;
        };
        let opportunity_cycles = aggregate
            .decision_cycles
            .saturating_sub(aggregate.quiet_cycles);
        let average_families_tenths = aggregate
            .viable_command_kinds
            .saturating_mul(10)
            .checked_div(opportunity_cycles)
            .unwrap_or(0);
        let mut top_commands: Vec<_> = aggregate
            .commands
            .iter()
            .filter(|(kind, stats)| is_substantive_command_kind(**kind) && stats.executed > 0)
            .map(|(kind, stats)| (*kind, stats.executed))
            .collect();
        top_commands.sort_by_key(|(kind, executed)| (std::cmp::Reverse(*executed), *kind));
        let command_summary = top_commands
            .into_iter()
            .take(3)
            .map(|(kind, executed)| format!("{} {executed}", kind.label()))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            output,
            "  {:<12} campaigns {:>2} | score {:>3} | substantive {:>4} | quiet {:>4} | families {} / actionable cycle | top: {}",
            persona.label(),
            aggregate.campaigns,
            aggregate.scores.overall,
            aggregate.substantive_actions,
            aggregate.quiet_cycles,
            format_tenths(average_families_tenths),
            if command_summary.is_empty() {
                "none"
            } else {
                &command_summary
            }
        );
    }
    let _ = writeln!(output);
}

pub(crate) fn render_phase_summary(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Phase quality");
    for phase in [
        GameplayPhase::Foundation,
        GameplayPhase::Establishment,
        GameplayPhase::InstitutionalAscent,
        GameplayPhase::DynasticGovernance,
        GameplayPhase::SuccessionLegacy,
    ] {
        let stats = report
            .aggregate
            .phase_stats
            .get(&phase)
            .cloned()
            .unwrap_or_default();
        let action_share = scaled_ratio_u64(
            u64::from(stats.substantive_actions),
            u64::from(stats.decision_cycles),
            100,
        );
        let campaign_admin_share = scaled_ratio_u64(
            u64::from(stats.institutional_campaign_actions),
            u64::from(stats.substantive_actions),
            100,
        );
        // Same denominator as the findings-side measure of this name: the
        // multi-family share reads as a share of actionable cycles, so the
        // rendered phase table and the matching finding cannot disagree.
        let multi_family_share = scaled_ratio_u64(
            u64::from(stats.cycles_with_multiple_viable_command_kinds),
            u64::from(stats.decision_cycles.saturating_sub(stats.quiet_cycles)),
            100,
        );
        let close_choice_share = scaled_ratio_u64(
            u64::from(stats.cycles_with_close_viable_command_kinds),
            u64::from(stats.cycles_with_multiple_viable_command_kinds),
            100,
        );
        let distinct_choice_share = scaled_ratio_u64(
            u64::from(stats.cycles_with_distinct_immediate_consequences),
            u64::from(stats.cycles_with_multiple_viable_command_kinds),
            100,
        );
        let projected_choice_share = scaled_ratio_u64(
            u64::from(stats.cycles_with_distinct_projected_consequences),
            u64::from(stats.cycles_with_multiple_viable_command_kinds),
            100,
        );
        let opportunity_cycles = stats.decision_cycles.saturating_sub(stats.quiet_cycles);
        let average_choices_tenths = scaled_ratio_u64(
            u64::from(stats.total_viable_choices),
            u64::from(opportunity_cycles),
            10,
        );
        let average_families_tenths = scaled_ratio_u64(
            u64::from(stats.total_viable_command_kinds),
            u64::from(opportunity_cycles),
            10,
        );
        let dominant_action = stats
            .executed_commands
            .iter()
            .max_by_key(|(kind, count)| (**count, std::cmp::Reverse(**kind)))
            .map_or_else(
                || "none".to_owned(),
                |(kind, executed)| {
                    let share = scaled_ratio_u64(
                        u64::from(*executed),
                        u64::from(stats.substantive_actions),
                        100,
                    );
                    format!("{} {share}%", kind.label())
                },
            );
        let _ = writeln!(
            output,
            "  {:<22} cycles {:>5} | action {:>3}% | top {:<24} | campaign admin {:>3}% | multi {:>3}% | close {:>3}% | distinct now {:>3}% / next {:>3}% | choices {}.{} / families {}.{} | quiet {:>5} (ambient {:>5}, longest {:>2}) | blocked {:>5} | causes policy {} / dormant {} / gaps {} / restrained {} / validation {}",
            phase.label(),
            stats.decision_cycles,
            action_share,
            dominant_action,
            campaign_admin_share,
            multi_family_share,
            close_choice_share,
            distinct_choice_share,
            projected_choice_share,
            average_choices_tenths / 10,
            average_choices_tenths % 10,
            average_families_tenths / 10,
            average_families_tenths % 10,
            stats.quiet_cycles,
            stats.quiet_cycles_with_ambient_change,
            stats.longest_quiet_streak_cycles,
            stats.blocked_cycles,
            stats.policy_gate_cycles,
            stats.dormant_cycles,
            stats.generator_gap_cycles,
            stats.restrained_cycles,
            stats.validation_gate_cycles
        );
    }
    let _ = writeln!(output);
}

pub(crate) fn render_limitations(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Harness limits");
    for limitation in &report.limitations {
        let _ = writeln!(output, "  - {limitation}");
    }
    let _ = writeln!(output);
}

pub(crate) fn render_fantasy_arcs(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Core fantasy milestones");
    for campaign in &report.campaigns {
        let arc = campaign.fantasy_arc;
        let _ = writeln!(
            output,
            "  seed {:>3} {:<12} {:?}: reputation {} | commercial record {} | institutional support {} target {:?} | campaign {} target {:?} | office {} | city-shaping {} via {:?} | labor conflict {} | heir designated {} | succession {}",
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
            milestone_day(arc.first_reputation_standing_day),
            milestone_day(arc.first_commercial_standing_day),
            milestone_day(arc.first_institution_support_day),
            arc.first_institution_support_target,
            milestone_day(arc.first_office_campaign_day),
            arc.first_office_campaign_target,
            milestone_day(arc.first_office_day),
            milestone_day(arc.first_city_shaping_action_day),
            arc.first_city_shaping_command,
            milestone_day(arc.first_player_labor_dispute_day),
            milestone_day(arc.first_heir_designation_day),
            milestone_day(arc.first_succession_day),
        );
    }
    let _ = writeln!(output);
}

pub(crate) fn milestone_day(day: Option<i64>) -> String {
    day.map_or_else(|| "not reached".to_owned(), |day| format!("day {day}"))
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HealthSummary {
    pub minimum_food: (u16, u16),
    pub minimum_district_food: (u16, u16),
    pub end_district_employment: (u16, u16),
    pub end_district_sanitation: (u16, u16),
    pub end_district_safety: (u16, u16),
    pub end_district_unrest: (u16, u16),
    pub operating_businesses: (u16, u16),
    pub peak_offices: (u16, u16),
    pub peak_unread: (u16, u16),
    pub peak_private_credit_distress: (u16, u16),
    pub peak_player_lending_distress: (u16, u16),
    pub peak_player_borrowing_distress: (u16, u16),
    pub peak_civic_credit_distress: (u16, u16),
    pub available_offices: u16,
    pub represented_institutions: (u16, u16),
    pub fulfilled_contracts: u64,
    pub breached_contracts: u64,
    pub repaid_loans: u64,
    pub defaulted_loans: u64,
    pub player_debt_enforcement_cases: u64,
    pub repaid_civic_debts: u64,
    pub defaulted_civic_debts: u64,
    pub completed_works: u64,
    pub suspended_works: u64,
}

impl HealthSummary {
    pub fn new(first: &GameplayCampaignReport) -> Self {
        Self {
            minimum_food: (
                first.minimum_food_satisfaction,
                first.minimum_food_satisfaction,
            ),
            minimum_district_food: (
                first.minimum_district_food_satisfaction,
                first.minimum_district_food_satisfaction,
            ),
            end_district_employment: (
                first.end.average_district_employment,
                first.end.average_district_employment,
            ),
            end_district_sanitation: (
                first.end.average_district_sanitation,
                first.end.average_district_sanitation,
            ),
            end_district_safety: (
                first.end.average_district_safety,
                first.end.average_district_safety,
            ),
            end_district_unrest: (
                first.end.average_district_unrest,
                first.end.average_district_unrest,
            ),
            operating_businesses: (
                first.minimum_operating_businesses,
                first.minimum_operating_businesses,
            ),
            peak_offices: (first.maximum_offices_held, first.maximum_offices_held),
            peak_unread: (
                first.maximum_unread_notifications,
                first.maximum_unread_notifications,
            ),
            peak_private_credit_distress: (0, 0),
            peak_player_lending_distress: (0, 0),
            peak_player_borrowing_distress: (0, 0),
            peak_civic_credit_distress: (0, 0),
            available_offices: first.end.available_offices,
            represented_institutions: (
                first.end.player_institutions_represented,
                first.end.player_institutions_represented,
            ),
            fulfilled_contracts: 0,
            breached_contracts: 0,
            repaid_loans: 0,
            defaulted_loans: 0,
            player_debt_enforcement_cases: 0,
            repaid_civic_debts: 0,
            defaulted_civic_debts: 0,
            completed_works: 0,
            suspended_works: 0,
        }
    }

    pub fn observe(&mut self, campaign: &GameplayCampaignReport) {
        self.minimum_food.0 = self.minimum_food.0.min(campaign.minimum_food_satisfaction);
        self.minimum_food.1 = self.minimum_food.1.max(campaign.minimum_food_satisfaction);
        self.minimum_district_food.0 = self
            .minimum_district_food
            .0
            .min(campaign.minimum_district_food_satisfaction);
        self.minimum_district_food.1 = self
            .minimum_district_food
            .1
            .max(campaign.minimum_district_food_satisfaction);
        self.observe_civic_conditions(&campaign.end);
        self.operating_businesses.0 = self
            .operating_businesses
            .0
            .min(campaign.minimum_operating_businesses);
        self.operating_businesses.1 = self
            .operating_businesses
            .1
            .max(campaign.minimum_operating_businesses);
        self.peak_offices.0 = self.peak_offices.0.min(campaign.maximum_offices_held);
        self.peak_offices.1 = self.peak_offices.1.max(campaign.maximum_offices_held);
        self.peak_unread.0 = self
            .peak_unread
            .0
            .min(campaign.maximum_unread_notifications);
        self.peak_unread.1 = self
            .peak_unread
            .1
            .max(campaign.maximum_unread_notifications);
        self.peak_private_credit_distress.0 = self
            .peak_private_credit_distress
            .0
            .max(campaign.maximum_delinquent_loans);
        self.peak_private_credit_distress.1 = self
            .peak_private_credit_distress
            .1
            .max(campaign.maximum_defaulted_loans);
        self.peak_player_lending_distress.0 = self
            .peak_player_lending_distress
            .0
            .max(campaign.maximum_player_delinquent_lending);
        self.peak_player_lending_distress.1 = self
            .peak_player_lending_distress
            .1
            .max(campaign.maximum_player_defaulted_lending);
        self.peak_player_borrowing_distress.0 = self
            .peak_player_borrowing_distress
            .0
            .max(campaign.maximum_player_delinquent_borrowing);
        self.peak_player_borrowing_distress.1 = self
            .peak_player_borrowing_distress
            .1
            .max(campaign.maximum_player_defaulted_borrowing);
        self.peak_civic_credit_distress.0 = self
            .peak_civic_credit_distress
            .0
            .max(campaign.maximum_delinquent_civic_debts);
        self.peak_civic_credit_distress.1 = self
            .peak_civic_credit_distress
            .1
            .max(campaign.maximum_defaulted_civic_debts);
        self.available_offices = self.available_offices.max(campaign.end.available_offices);
        self.represented_institutions.0 = self
            .represented_institutions
            .0
            .min(campaign.end.player_institutions_represented);
        self.represented_institutions.1 = self
            .represented_institutions
            .1
            .max(campaign.end.player_institutions_represented);
        self.fulfilled_contracts = self
            .fulfilled_contracts
            .saturating_add(u64::from(campaign.end.player_fulfilled_contracts));
        self.breached_contracts = self
            .breached_contracts
            .saturating_add(u64::from(campaign.end.player_breached_contracts));
        self.repaid_loans = self
            .repaid_loans
            .saturating_add(u64::from(campaign.end.repaid_loans));
        self.defaulted_loans = self
            .defaulted_loans
            .saturating_add(u64::from(campaign.end.defaulted_loans));
        self.player_debt_enforcement_cases = self
            .player_debt_enforcement_cases
            .saturating_add(u64::from(campaign.player_debt_enforcement_cases));
        self.repaid_civic_debts = self
            .repaid_civic_debts
            .saturating_add(u64::from(campaign.end.repaid_civic_debts));
        self.defaulted_civic_debts = self
            .defaulted_civic_debts
            .saturating_add(u64::from(campaign.end.defaulted_civic_debts));
        self.completed_works = self
            .completed_works
            .saturating_add(u64::from(campaign.end.completed_public_works));
        self.suspended_works = self
            .suspended_works
            .saturating_add(u64::from(campaign.end.suspended_public_works));
    }

    pub fn observe_civic_conditions(&mut self, snapshot: &GameplaySnapshot) {
        update_range(
            &mut self.end_district_employment,
            snapshot.average_district_employment,
        );
        update_range(
            &mut self.end_district_sanitation,
            snapshot.average_district_sanitation,
        );
        update_range(
            &mut self.end_district_safety,
            snapshot.average_district_safety,
        );
        update_range(
            &mut self.end_district_unrest,
            snapshot.average_district_unrest,
        );
    }
}

pub(crate) fn update_range(range: &mut (u16, u16), value: u16) {
    range.0 = range.0.min(value);
    range.1 = range.1.max(value);
}

pub(crate) fn summarize_health(campaigns: &[GameplayCampaignReport]) -> Option<HealthSummary> {
    let mut summary = HealthSummary::new(campaigns.first()?);
    for campaign in campaigns {
        summary.observe(campaign);
    }
    Some(summary)
}

pub(crate) fn render_health_summary(report: &GameplayHarnessReport, output: &mut String) {
    let Some(summary) = summarize_health(&report.campaigns) else {
        return;
    };
    let _ = writeln!(output, "Experience health");
    let _ = writeln!(
        output,
        "  trajectory ranges: city food {:.2}-{:.2}% | worst district food {:.2}-{:.2}% | operating businesses {}-{} | peak offices {}-{}/{} | represented institutions {}-{}/{} | peak unread {}-{}",
        f64::from(summary.minimum_food.0) / 100.0,
        f64::from(summary.minimum_food.1) / 100.0,
        f64::from(summary.minimum_district_food.0) / 100.0,
        f64::from(summary.minimum_district_food.1) / 100.0,
        summary.operating_businesses.0,
        summary.operating_businesses.1,
        summary.peak_offices.0,
        summary.peak_offices.1,
        summary.available_offices,
        summary.represented_institutions.0,
        summary.represented_institutions.1,
        summary.available_offices,
        summary.peak_unread.0,
        summary.peak_unread.1
    );
    let _ = writeln!(
        output,
        "  ending civic conditions: employment {:.2}-{:.2}% | sanitation {:.2}-{:.2}% | safety {:.2}-{:.2}% | unrest {:.2}-{:.2}%",
        f64::from(summary.end_district_employment.0) / 100.0,
        f64::from(summary.end_district_employment.1) / 100.0,
        f64::from(summary.end_district_sanitation.0) / 100.0,
        f64::from(summary.end_district_sanitation.1) / 100.0,
        f64::from(summary.end_district_safety.0) / 100.0,
        f64::from(summary.end_district_safety.1) / 100.0,
        f64::from(summary.end_district_unrest.0) / 100.0,
        f64::from(summary.end_district_unrest.1) / 100.0,
    );
    let _ = writeln!(
        output,
        "  outcomes: player contracts {} fulfilled / {} breached | private loans {} repaid / {} defaulted | player debt enforcement {} case(s) | civic debts {} repaid / {} defaulted | public works {} completed / {} suspended\n",
        summary.fulfilled_contracts,
        summary.breached_contracts,
        summary.repaid_loans,
        summary.defaulted_loans,
        summary.player_debt_enforcement_cases,
        summary.repaid_civic_debts,
        summary.defaulted_civic_debts,
        summary.completed_works,
        summary.suspended_works,
    );
    let _ = writeln!(
        output,
        // Each slot is an independent cross-campaign peak, so the label must
        // not claim both numbers came from a single campaign.
        "  campaign peaks of credit distress: private {} delinquent / {} defaulted | player-issued {} delinquent / {} defaulted | player-borrowed {} delinquent / {} defaulted | civic {} delinquent / {} defaulted\n",
        summary.peak_private_credit_distress.0,
        summary.peak_private_credit_distress.1,
        summary.peak_player_lending_distress.0,
        summary.peak_player_lending_distress.1,
        summary.peak_player_borrowing_distress.0,
        summary.peak_player_borrowing_distress.1,
        summary.peak_civic_credit_distress.0,
        summary.peak_civic_credit_distress.1,
    );
}

pub(crate) fn count_player_borrowing_status(
    state: &AppState,
    player_id: DynastyId,
    status: LoanStatus,
) -> u16 {
    usize_to_u16(
        state
            .loans
            .values()
            .filter(|loan| loan.borrower_dynasty_id == player_id && loan.status == status)
            .count(),
    )
}

pub(crate) fn render_report_header(report: &GameplayHarnessReport, output: &mut String) {
    let aggregate = &report.aggregate;
    let _ = writeln!(output, "Civic Dynasty gameplay harness");
    let _ = writeln!(
        output,
        "{} campaigns | {} simulated days | {} substantive actions ({} total) | {} candidate probes",
        aggregate.campaigns,
        aggregate.simulated_days,
        aggregate.substantive_actions,
        aggregate.successful_actions,
        aggregate.candidate_probes
    );
    let _ = writeln!(
        output,
        "scores: overall {:>3} | actionability {:>3} | variety {:>3} | interconnection {:>3} | feedback {:>3} | resilience {:>3}",
        aggregate.scores.overall,
        aggregate.scores.actionability,
        aggregate.scores.variety,
        aggregate.scores.interconnection,
        aggregate.scores.feedback,
        aggregate.scores.resilience
    );
    // Coverage counts only substantive command kinds; the three operational
    // housekeeping kinds are excluded from both numerator and denominator.
    let _ = writeln!(
        output,
        "coverage: {}/{} command kinds | causal domains {}/{} | ambient domains {}/{} | {} command-domain edges | {} quiet ({} with ambient change) / {} blocked cycles",
        aggregate.command_coverage,
        ALL_COMMAND_KINDS
            .iter()
            .filter(|kind| is_substantive_command_kind(**kind))
            .count(),
        aggregate.causal_domain_coverage,
        ALL_DOMAINS.len(),
        aggregate.ambient_domain_coverage,
        ALL_DOMAINS.len(),
        aggregate.interactions.len(),
        aggregate.quiet_cycles,
        aggregate.quiet_cycles_with_ambient_change,
        aggregate.blocked_cycles
    );
    let opportunity_cycles = aggregate
        .decision_cycles
        .saturating_sub(aggregate.quiet_cycles);
    let average_families_tenths = aggregate
        .viable_command_kinds
        .saturating_mul(10)
        .checked_div(opportunity_cycles)
        .unwrap_or(0);
    let average_choices_tenths = aggregate
        .viable_choices
        .saturating_mul(10)
        .checked_div(opportunity_cycles)
        .unwrap_or(0);
    let average_families = format_tenths(average_families_tenths);
    let average_choices = format_tenths(average_choices_tenths);
    let _ = writeln!(
        output,
        "choice quality: {average_choices} viable choices / {average_families} command families per actionable cycle | family: {} multi / {} close / {} distinct immediate / {} distinct projected | concrete: {} multi / {} close / {} distinct immediate / {} distinct projected\n",
        aggregate.cycles_with_multiple_viable_command_kinds,
        aggregate.cycles_with_close_viable_command_kinds,
        aggregate.cycles_with_distinct_immediate_consequences,
        aggregate.cycles_with_distinct_projected_consequences,
        aggregate.cycles_with_multiple_viable_options,
        aggregate.cycles_with_close_viable_options,
        aggregate.cycles_with_distinct_immediate_option_consequences,
        aggregate.cycles_with_distinct_projected_option_consequences,
    );
}

pub(crate) fn render_command_table(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Command coverage");
    let _ = writeln!(
        output,
        "  {:<20} {:>8} {:>7} {:>9} {:>8} {:>8} {:>8} {:>8} {:>10} {:>8} {:>8}",
        "command",
        "triggers",
        "offered",
        "generated",
        "probed",
        "viable",
        "used",
        "feedback",
        "persistent",
        "delayed",
        "domains"
    );
    for kind in ALL_COMMAND_KINDS {
        let stats = report
            .aggregate
            .commands
            .get(&kind)
            .expect("every command kind must have aggregate statistics");
        let _ = writeln!(
            output,
            "  {:<20} {:>8} {:>7} {:>9} {:>8} {:>8} {:>8} {:>8} {:>10} {:>8} {:>8}",
            kind.label(),
            stats.activation_opportunities,
            stats.offered_cycles,
            stats.generated,
            stats.considered,
            stats.viable,
            stats.executed,
            stats.actions_with_feedback,
            stats.actions_with_persistent_consequences,
            stats.actions_with_delayed_consequences,
            stats.changed_domains.len()
        );
    }
    let _ = writeln!(output);
}

pub(crate) fn render_domain_table(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Observed domain transitions (causal / ambient)");
    for row in ALL_DOMAINS.chunks(3) {
        let mut line = String::new();
        for domain in row {
            let causal = report
                .aggregate
                .causal_domain_changes
                .get(domain)
                .copied()
                .unwrap_or(0);
            let ambient = report
                .aggregate
                .ambient_domain_changes
                .get(domain)
                .copied()
                .unwrap_or(0);
            let _ = write!(
                line,
                "  {:<14} {:>5}/{:<5}",
                domain.label(),
                causal,
                ambient
            );
        }
        let _ = writeln!(output, "{line}");
    }
    let _ = writeln!(output);
}

pub(crate) fn render_interactions(report: &GameplayHarnessReport, output: &mut String) {
    let mut edges = report.aggregate.interactions.clone();
    edges.sort_by(|left, right| {
        right
            .observations
            .cmp(&left.observations)
            .then_with(|| left.command.cmp(&right.command))
            .then_with(|| left.domain.cmp(&right.domain))
    });
    let _ = writeln!(output, "Strongest observed command consequences");
    for edge in edges.into_iter().take(10) {
        let _ = writeln!(
            output,
            "  {:<20} -> {:<14} {:>6} observations",
            edge.command.label(),
            edge.domain.label(),
            edge.observations
        );
    }
    let _ = writeln!(output);
}

pub(crate) fn render_rejections(report: &GameplayHarnessReport, output: &mut String) {
    if report.aggregate.rejection_reasons.is_empty() {
        return;
    }
    let mut reasons: Vec<_> = report.aggregate.rejection_reasons.iter().collect();
    reasons.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
    let _ = writeln!(output, "Most common blocked choices");
    for (reason, count) in reasons.into_iter().take(8) {
        let _ = writeln!(output, "  {count:>6}  {reason}");
    }
    let _ = writeln!(output);
}

pub(crate) fn render_quiet_diagnosis(report: &GameplayHarnessReport, output: &mut String) {
    let diagnostic = &report.aggregate.quiet_diagnostic;
    if diagnostic.generator_gaps.is_empty()
        && diagnostic.policy_gates.is_empty()
        && diagnostic.restrained_routes.is_empty()
        && diagnostic.validation_gates.is_empty()
        && diagnostic.budget_gates.is_empty()
        && diagnostic.dormant_cycles == 0
    {
        return;
    }
    let _ = writeln!(output, "Quiet cycle diagnosis");
    if !diagnostic.restrained_routes.is_empty() {
        let mut restrained: Vec<_> = diagnostic.restrained_routes.iter().collect();
        restrained.sort_by_key(|(kind, count)| (std::cmp::Reverse(**count), **kind));
        let restrained_text = restrained
            .into_iter()
            .take(8)
            .map(|(kind, count)| format!("{} {count}", kind.label()))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(output, "  reserved by agent policy: {restrained_text}");
        let _ = writeln!(
            output,
            "    (an activation opportunity fired but the persona's standing policy deliberately narrows the route to strategic-need conditions; not a game gap)"
        );
    }
    if !diagnostic.generator_gaps.is_empty() {
        let mut gaps: Vec<_> = diagnostic.generator_gaps.iter().collect();
        gaps.sort_by_key(|(kind, count)| (std::cmp::Reverse(**count), **kind));
        let gap_text = gaps
            .into_iter()
            .take(5)
            .map(|(kind, count)| format!("{} {count}", kind.label()))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(output, "  opportunities without candidates: {gap_text}");
        let _ = writeln!(
            output,
            "    (the world offered a route outside the agent's narrowed set and no candidate was built; investigate these as possible coverage holes)"
        );
    }
    if !diagnostic.policy_gates.is_empty() {
        let mut gates: Vec<_> = diagnostic.policy_gates.iter().collect();
        gates.sort_by_key(|(kind, count)| (std::cmp::Reverse(**count), **kind));
        let gate_text = gates
            .into_iter()
            .take(5)
            .map(|(kind, count)| format!("{} {count}", kind.label()))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(output, "  declined by agent spending policy: {gate_text}");
    }
    if !diagnostic.validation_gates.is_empty() {
        let mut gates: Vec<_> = diagnostic.validation_gates.iter().collect();
        gates.sort_by_key(|(kind, count)| (std::cmp::Reverse(**count), **kind));
        let gate_text = gates
            .into_iter()
            .take(5)
            .map(|(kind, count)| format!("{} {count}", kind.label()))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(output, "  generated but rejected: {gate_text}");
    }
    if !diagnostic.budget_gates.is_empty() {
        let mut gates: Vec<_> = diagnostic.budget_gates.iter().collect();
        gates.sort_by_key(|(kind, count)| (std::cmp::Reverse(**count), **kind));
        let gate_text = gates
            .into_iter()
            .take(5)
            .map(|(kind, count)| format!("{} {count}", kind.label()))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(output, "  unverified due to probe budget: {gate_text}");
    }
    if diagnostic.dormant_cycles > 0 {
        let _ = writeln!(
            output,
            "  dormant: {} quiet cycle(s) where no candidate was built and no activation opportunity fired; the world offered no detected action",
            diagnostic.dormant_cycles
        );
    }
    let _ = writeln!(output);
}

pub(crate) fn render_findings(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Findings");
    for finding in &report.findings {
        let _ = writeln!(
            output,
            "  [{:?}] {}: {}",
            finding.severity, finding.title, finding.evidence
        );
    }
    let _ = writeln!(output);
}

pub(crate) fn render_campaign_summaries(report: &GameplayHarnessReport, output: &mut String) {
    let _ = writeln!(output, "Campaign summaries");
    for campaign in &report.campaigns {
        let actions: u32 = campaign.commands.values().map(|stats| stats.executed).sum();
        let _ = writeln!(
            output,
            "  seed {:>3} | {:<12} | {:<11?} | score {:>3} | actions {:>3} | choices {:>4} | treasury {} (peak {}) | businesses A:{} D:{} I:{}",
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
            campaign.scores.overall,
            actions,
            campaign.total_viable_choices,
            campaign.end.player_treasury,
            campaign.peak_player_treasury,
            campaign.end.active_businesses,
            campaign.end.distressed_businesses,
            campaign.end.insolvent_businesses
        );
        let _ = writeln!(
            output,
            "      civic | laws {:?} | works {:?} | employment {:.2}% | sanitation {:.2}% | safety {:.2}% | unrest {:.2}%",
            campaign.end.active_law_kinds,
            campaign.end.player_completed_public_work_kinds,
            f64::from(campaign.end.average_district_employment) / 100.0,
            f64::from(campaign.end.average_district_sanitation) / 100.0,
            f64::from(campaign.end.average_district_safety) / 100.0,
            f64::from(campaign.end.average_district_unrest) / 100.0,
        );
        let ledger_margin = campaign
            .end
            .player_business_lifetime_revenue
            .copper()
            .saturating_sub(campaign.end.player_business_lifetime_costs.copper());
        let _ = writeln!(
            output,
            "      ledger | lifetime revenue {} | costs {} | margin {} | business cash {}",
            campaign.end.player_business_lifetime_revenue,
            campaign.end.player_business_lifetime_costs,
            Money::from_copper(ledger_margin),
            campaign.end.player_business_cash
        );
        if let Some(transition) = campaign.succession_transition {
            let _ = writeln!(
                output,
                "      succession day {} | unity {}->{} | legitimacy {}->{} | offices {}->{} | memberships {}->{} | represented institutions {}->{}",
                transition.day,
                transition.family_unity_before,
                transition.family_unity_after,
                transition.legitimacy_before,
                transition.legitimacy_after,
                transition.offices_before,
                transition.offices_after,
                transition.institution_memberships_before,
                transition.institution_memberships_after,
                transition.represented_institutions_before,
                transition.represented_institutions_after,
            );
        }
    }
    let _ = writeln!(output);
}

pub(crate) fn render_decision_log(report: &GameplayHarnessReport, output: &mut String) {
    let selected =
        decision_log_campaigns(report, usize::from(report.config.decision_log_campaigns));
    if selected.is_empty() {
        return;
    }
    let _ = writeln!(output, "Decision log");
    let _ = writeln!(
        output,
        "  campaigns shown: {} (of {}); ordered to favor city-shaping, succession, quiet diagnosis, and command variety",
        selected.len(),
        report.campaigns.len()
    );
    for campaign in selected {
        let no_action_cycles = campaign.quiet_cycles + campaign.blocked_cycles;
        let _ = writeln!(
            output,
            "  campaign seed {} | {:<12} | {:<10?} | {} days | {} cycles | {} actions | {} quiet/blocked | {} viable choices | score {}",
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
            campaign.simulated_days,
            campaign.decision_cycles,
            campaign
                .commands
                .values()
                .map(|stats| stats.executed)
                .sum::<u32>(),
            no_action_cycles,
            campaign.total_viable_choices,
            campaign.scores.overall,
        );
        for step in &campaign.trace {
            let context = &step.context;
            let options = format!(
                "{}/{} offered/viable",
                step.considered_candidates, step.viable_candidates
            );
            let action_text = match &step.selected_command {
                Some(kind) => {
                    let description = step
                        .command_description
                        .as_deref()
                        .unwrap_or("(no description)");
                    let outcome = step
                        .outcome
                        .as_deref()
                        .map(|text| truncate_label(text, 72))
                        .unwrap_or_default();
                    format!(
                        "{:<18} \"{}\"{}",
                        kind.label(),
                        truncate_label(description, 64),
                        if outcome.is_empty() {
                            String::new()
                        } else {
                            format!(" | result: {outcome}")
                        }
                    )
                }
                None => format!(
                    "NO ACTION ({options}) => {}",
                    compact_no_action_reason(step.no_action_reason.as_deref())
                ),
            };
            let _ = writeln!(
                output,
                "  day {:>4} {:<19} | treasury {:>9} | biz {:>9} | businesses {}{} | offices {} | legit {:.0}% | gen {}{} | crises {} | {}",
                step.day,
                phase_label_at_day(&campaign.fantasy_arc, step.day),
                context.player_treasury,
                context.player_business_cash,
                context.active_businesses,
                if context.distressed_businesses > 0 {
                    format!(" ({} distressed)", context.distressed_businesses)
                } else {
                    String::new()
                },
                context.offices_held,
                f64::from(context.legitimacy) / 100.0,
                context.generation,
                legal_pressure_suffix(context),
                context.active_crises,
                action_text,
            );
            let _ = writeln!(
                output,
                "             {} now [{}] later [{}] signals [{}]",
                options,
                domain_labels(&step.immediate_domains),
                domain_labels(&step.delayed_domains),
                trace_signal_labels(&step.signals),
            );
            render_trace_alternatives(step, output);
            render_trace_deltas(step, output);
        }
        let _ = writeln!(output);
    }
}

pub(crate) fn render_trace_alternatives(step: &GameplayTraceStep, output: &mut String) {
    if step.viable_options.len() < 2 {
        return;
    }
    let _ = writeln!(
        output,
        "             alternatives (shared projected horizon):"
    );
    // Distinct-option dedupe: several concrete targets often project the same
    // measurable outcome (for example identical ward adoptions), so repeated
    // rows would only re-render one tradeoff three times.
    let mut rendered_profiles: std::collections::BTreeSet<(GameplayCommandKind, String)> =
        std::collections::BTreeSet::new();
    let mut shown = 0;
    for option in &step.viable_options {
        if shown >= 3 {
            break;
        }
        let profile_key = (
            option.command,
            format!(
                "{}|{}",
                domain_labels(&option.projected_domains),
                format_measure_changes(&option.projected_profile)
            ),
        );
        if !rendered_profiles.insert(profile_key) {
            continue;
        }
        let _ = writeln!(
            output,
            "               {:<18} score {:>5} | {}d later [{}] | changes [{}]",
            option.command.label(),
            option.score,
            option.projected_horizon_days,
            domain_labels(&option.projected_domains),
            format_measure_changes(&option.projected_profile),
        );
        shown += 1;
    }
}

/// Caps the bracketed command-family list inside quiet-cycle reasons so a
/// twelve-family restraint list cannot dominate every no-action line. The full
/// list stays in the structured report; the human log keeps the leading
/// families plus a count of the remainder.
fn compact_no_action_reason(reason: Option<&str>) -> String {
    let Some(reason) = reason else {
        return "no reason recorded".to_owned();
    };
    const MAX_FAMILIES: usize = 6;
    let Some((prefix, list)) = reason.split_once('[') else {
        return reason.to_owned();
    };
    let Some((families, suffix)) = list.split_once(']') else {
        return reason.to_owned();
    };
    let mut parts: Vec<&str> = families.split(',').map(str::trim).collect();
    if parts.len() <= MAX_FAMILIES {
        return reason.to_owned();
    }
    let omitted = parts.len() - MAX_FAMILIES;
    parts.truncate(MAX_FAMILIES);
    format!("{prefix}[{}, +{omitted} more]{suffix}", parts.join(", "))
}

/// A short suffix surfacing player-facing legal exposure that the standard
/// columns do not show, so a decision log explains lawsuit-driven context.
fn legal_pressure_suffix(context: &GameplayDecisionContext) -> String {
    let mut parts = Vec::new();
    if context.player_open_legal_cases_as_defendant > 0 {
        parts.push(format!(
            "sued {}",
            context.player_open_legal_cases_as_defendant
        ));
    }
    if context.player_breach_victim_contracts > 0 {
        parts.push(format!(
            "breached {}x",
            context.player_breach_victim_contracts
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

pub(crate) fn render_trace_deltas(step: &GameplayTraceStep, output: &mut String) {
    let _ = writeln!(
        output,
        "             deltas | immediate [{}] attributable [{}] ambient [{}]",
        format_measure_changes(&step.immediate_consequences),
        format_measure_changes(&step.attributed_consequences),
        format_measure_changes(&step.ambient_consequences),
    );
    render_feedback_group("command feedback", &step.command_feedback, None, output);
    render_feedback_group(
        "simulation feedback",
        &step.simulation_feedback,
        Some(step.simulation_window_days),
        output,
    );
    render_feedback_group(
        "ambient feedback",
        &step.ambient_feedback,
        Some(step.ambient_window_days),
        output,
    );
}

pub(crate) fn render_feedback_group(
    label: &str,
    feedback: &[GameplayFeedbackEvent],
    window_days: Option<u32>,
    output: &mut String,
) {
    if feedback.is_empty() {
        return;
    }
    // The attribution windows differ per branch (a substantive cycle's ambient
    // horizon can span several decision intervals), so each group states how
    // much time its events cover instead of leaving readers to infer it.
    let label = match window_days {
        Some(days) => format!("{label} over {days}d"),
        None => label.to_owned(),
    };
    let summaries = feedback
        .iter()
        .take(3)
        .map(|event| {
            let subject = if event.subject.is_empty() {
                String::new()
            } else {
                format!(" {}:", event.subject)
            };
            format!(
                "{}{} {}",
                event.kind,
                subject,
                truncate_label(&event.text, 120)
            )
        })
        .collect::<Vec<_>>()
        .join(" | ");
    let omitted = feedback.len().saturating_sub(3);
    let suffix = if omitted > 0 {
        format!(" (+{omitted} more)")
    } else {
        String::new()
    };
    let _ = writeln!(output, "             {label}: {summaries}{suffix}");
}

pub(crate) fn decision_log_campaigns(
    report: &GameplayHarnessReport,
    limit: usize,
) -> Vec<&GameplayCampaignReport> {
    if limit == 0 {
        return Vec::new();
    }
    let mut campaigns: Vec<_> = report.campaigns.iter().collect();
    campaigns.sort_by(|left, right| {
        let left_no_action = left.quiet_cycles + left.blocked_cycles;
        let right_no_action = right.quiet_cycles + right.blocked_cycles;
        (
            std::cmp::Reverse(left.fantasy_arc.first_city_shaping_action_day.is_some()),
            std::cmp::Reverse(left.fantasy_arc.first_succession_day.is_some()),
            std::cmp::Reverse(left_no_action),
            std::cmp::Reverse(left.total_viable_command_kinds),
            left.seed,
            left.persona,
            format!("{:?}", left.background),
        )
            .cmp(&(
                std::cmp::Reverse(right.fantasy_arc.first_city_shaping_action_day.is_some()),
                std::cmp::Reverse(right.fantasy_arc.first_succession_day.is_some()),
                std::cmp::Reverse(right_no_action),
                std::cmp::Reverse(right.total_viable_command_kinds),
                right.seed,
                right.persona,
                format!("{:?}", right.background),
            ))
    });
    campaigns.truncate(limit);
    campaigns
}

pub(crate) fn phase_label_at_day(arc: &GameplayFantasyArc, day: i64) -> &'static str {
    if arc
        .first_succession_day
        .is_some_and(|succession| day >= succession)
    {
        GameplayPhase::SuccessionLegacy.label()
    } else if arc
        .first_city_shaping_action_day
        .is_some_and(|shaping| day >= shaping)
    {
        GameplayPhase::DynasticGovernance.label()
    } else if arc
        .first_commercial_standing_day
        .is_some_and(|standing| day >= standing)
    {
        GameplayPhase::InstitutionalAscent.label()
    } else if arc
        .first_reputation_standing_day
        .is_some_and(|standing| day >= standing)
    {
        GameplayPhase::Establishment.label()
    } else {
        GameplayPhase::Foundation.label()
    }
}

pub(crate) fn truncate_label(text: &str, max_chars: usize) -> String {
    let characters = text.chars();
    let total = characters.clone().count();
    if total <= max_chars {
        return text.to_owned();
    }
    let prefix: String = characters.take(max_chars.saturating_sub(1)).collect();
    format!("{prefix}…")
}

pub(crate) fn trace_signal_labels(signals: &BTreeSet<GameplayTraceSignal>) -> String {
    if signals.is_empty() {
        return "none".to_owned();
    }
    signals
        .iter()
        .map(|signal| match signal {
            GameplayTraceSignal::ImmediateWorldFeedback => "immediate-feedback",
            GameplayTraceSignal::DelayedWorldFeedback => "delayed-feedback",
            GameplayTraceSignal::AmbientWorldFeedback => "ambient-feedback",
            GameplayTraceSignal::PersistentHistoryChange => "persistent-history",
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn domain_labels(domains: &BTreeSet<GameplayDomain>) -> String {
    if domains.is_empty() {
        return "none".to_owned();
    }
    domains
        .iter()
        .map(|domain| domain.label())
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn measure_label(measure: GameplayMeasure) -> &'static str {
    match measure {
        GameplayMeasure::PlayerTreasury => "treasury",
        GameplayMeasure::PlayerBusinessCash => "business-cash",
        GameplayMeasure::PlayerBusinessLifetimeProfit => "business-lifetime-profit",
        GameplayMeasure::ActiveBusinesses => "active-businesses",
        GameplayMeasure::DistressedBusinesses => "distressed-businesses",
        GameplayMeasure::PlayerActiveContracts => "active-contracts",
        GameplayMeasure::PlayerFulfilledContracts => "fulfilled-contracts",
        GameplayMeasure::PlayerBreachedContracts => "breached-contracts",
        GameplayMeasure::CurrentLoans => "current-loans",
        GameplayMeasure::PlayerCurrentLending => "current-lending",
        GameplayMeasure::PlayerCurrentBorrowing => "current-borrowing",
        GameplayMeasure::PlayerProperties => "properties",
        GameplayMeasure::PlayerPledgedProperties => "pledged-properties",
        GameplayMeasure::Legitimacy => "legitimacy",
        GameplayMeasure::FamilyUnity => "family-unity",
        GameplayMeasure::OfficesHeld => "offices",
        GameplayMeasure::InstitutionMemberships => "institution-memberships",
        GameplayMeasure::InstitutionRepresentation => "represented-institutions",
        GameplayMeasure::Generation => "generation",
        GameplayMeasure::ActiveLaws => "active-laws",
        GameplayMeasure::BuildingPublicWorks => "building-works",
        GameplayMeasure::CompletedPublicWorks => "completed-works",
        GameplayMeasure::AverageFoodSatisfaction => "food",
        GameplayMeasure::AverageDistrictUnrest => "district-unrest",
        GameplayMeasure::AverageDistrictEmployment => "district-employment",
        GameplayMeasure::AverageDistrictSanitation => "district-sanitation",
        GameplayMeasure::AverageDistrictSafety => "district-safety",
        GameplayMeasure::ActiveCrises => "active-crises",
        GameplayMeasure::ContractRelationshipPressure => "contract-pressure",
        GameplayMeasure::PlayerDisputedEmployment => "player-disputed-labor",
        GameplayMeasure::DefaultedLoans => "defaulted-loans",
        GameplayMeasure::PlayerDelinquentBorrowing => "delinquent-borrowing",
        GameplayMeasure::PlayerDefaultedBorrowing => "defaulted-borrowing",
        GameplayMeasure::UnmetOfficeDuties => "unmet-office-duties",
        GameplayMeasure::PlayerOpenLegalCasesAsDefendant => "open-legal-cases",
        GameplayMeasure::InformationReports => "information-reports",
    }
}

pub(crate) fn format_measure_change(
    measure: GameplayMeasure,
    change: GameplayMeasureChange,
) -> String {
    let values = if matches!(
        measure,
        GameplayMeasure::PlayerTreasury
            | GameplayMeasure::PlayerBusinessCash
            | GameplayMeasure::PlayerBusinessLifetimeProfit
    ) {
        format!(
            "{}->{}",
            Money::from_copper(change.before),
            Money::from_copper(change.after)
        )
    } else {
        format!("{}->{}", change.before, change.after)
    };
    format!("{} {values}", measure_label(measure))
}

pub(crate) fn format_measure_changes(profile: &GameplayConsequenceProfile) -> String {
    if profile.changes.is_empty() {
        return "none".to_owned();
    }
    let mut changes = profile
        .changes
        .iter()
        .map(|(measure, change)| format_measure_change(*measure, *change))
        .collect::<Vec<_>>();
    let omitted = changes.len().saturating_sub(5);
    changes.truncate(5);
    let mut rendered = changes.join(", ");
    if omitted > 0 {
        let _ = write!(rendered, " (+{omitted} more)");
    }
    rendered
}

pub(crate) struct StableChecksumWriter(u64);

impl StableChecksumWriter {
    const OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
    const PRIME: u64 = 1_099_511_628_211;
}

impl std::io::Write for StableChecksumWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for byte in bytes {
            self.0 = self.0.wrapping_mul(Self::PRIME) ^ u64::from(*byte);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) fn stable_serialized_checksum<T: Serialize + ?Sized>(value: &T) -> u64 {
    let mut writer = StableChecksumWriter(StableChecksumWriter::OFFSET_BASIS);
    serde_json::to_writer(&mut writer, value)
        .expect("gameplay observation state must serialize into its checksum");
    writer.0
}

pub(crate) fn persistent_history_changed(
    before: &GameplaySnapshot,
    after_command: &GameplaySnapshot,
    after_time: &GameplaySnapshot,
    baseline_after_time: &GameplaySnapshot,
) -> bool {
    before.audit_state_checksum != after_command.audit_state_checksum
        && baseline_after_time.audit_state_checksum != after_time.audit_state_checksum
}

pub(crate) fn dynasty_state_checksum(state: &AppState) -> u64 {
    let observations: Vec<_> = state
        .dynasties
        .values()
        .map(|dynasty| {
            (
                dynasty.id(),
                dynasty.head_id(),
                dynasty.heir_id(),
                (
                    dynasty.runtime.phase,
                    dynasty.runtime.generation,
                    dynasty.runtime.succession_risk_basis_points,
                ),
                (
                    dynasty.resources.unmet_office_duties,
                    dynasty.resources.legitimacy_basis_points,
                    dynasty.resources.administrative_capacity,
                    dynasty.resources.administrative_load,
                    dynasty.resources.reputation_quality_basis_points,
                    dynasty.resources.reputation_reliability_basis_points,
                ),
            )
        })
        .collect();
    stable_serialized_checksum(&observations)
}

pub(crate) fn compare_economy_and_business(
    earlier: &GameplaySnapshot,
    later: &GameplaySnapshot,
    domains: &mut BTreeSet<GameplayDomain>,
) {
    if earlier.player_treasury != later.player_treasury
        || earlier.player_civic_contributions != later.player_civic_contributions
        || earlier.player_business_cash != later.player_business_cash
        || earlier.household_state_checksum != later.household_state_checksum
    {
        domains.insert(GameplayDomain::Economy);
    }
    if earlier.active_businesses != later.active_businesses
        || earlier.distressed_businesses != later.distressed_businesses
        || earlier.insolvent_businesses != later.insolvent_businesses
        || earlier.average_business_condition != later.average_business_condition
        || earlier.average_business_quality != later.average_business_quality
        || earlier.business_policy_checksum != later.business_policy_checksum
        || earlier.business_state_checksum != later.business_state_checksum
    {
        domains.insert(GameplayDomain::Business);
    }
    if earlier.market_price_total != later.market_price_total
        || earlier.market_stock_total != later.market_stock_total
        || earlier.market_state_checksum != later.market_state_checksum
        || earlier.external_route_state_checksum != later.external_route_state_checksum
    {
        domains.insert(GameplayDomain::Market);
    }
}

pub(crate) fn compare_contracts_and_finance(
    earlier: &GameplaySnapshot,
    later: &GameplaySnapshot,
    domains: &mut BTreeSet<GameplayDomain>,
) {
    if earlier.active_contracts != later.active_contracts
        || earlier.fulfilled_contracts != later.fulfilled_contracts
        || earlier.breached_contracts != later.breached_contracts
        || earlier.contract_failures != later.contract_failures
        || earlier.player_active_contracts != later.player_active_contracts
        || earlier.player_fulfilled_contracts != later.player_fulfilled_contracts
        || earlier.player_breached_contracts != later.player_breached_contracts
        || earlier.player_contract_failures != later.player_contract_failures
        || earlier.player_contract_deliveries != later.player_contract_deliveries
        || earlier.contract_state_checksum != later.contract_state_checksum
    {
        domains.insert(GameplayDomain::Contracts);
    }
    if earlier.current_loans != later.current_loans
        || earlier.delinquent_loans != later.delinquent_loans
        || earlier.restructured_loans != later.restructured_loans
        || earlier.defaulted_loans != later.defaulted_loans
        || earlier.repaid_loans != later.repaid_loans
        || earlier.total_loan_balance != later.total_loan_balance
        || earlier.current_civic_debts != later.current_civic_debts
        || earlier.delinquent_civic_debts != later.delinquent_civic_debts
        || earlier.defaulted_civic_debts != later.defaulted_civic_debts
        || earlier.repaid_civic_debts != later.repaid_civic_debts
        || earlier.total_civic_debt_balance != later.total_civic_debt_balance
        || earlier.loan_state_checksum != later.loan_state_checksum
        || earlier.civic_debt_state_checksum != later.civic_debt_state_checksum
    {
        domains.insert(GameplayDomain::Loans);
    }
    if earlier.player_properties != later.player_properties
        || earlier.player_pledged_properties != later.player_pledged_properties
        || earlier.player_collateral_balance != later.player_collateral_balance
        || earlier.occupied_properties != later.occupied_properties
        || earlier.property_state_checksum != later.property_state_checksum
    {
        domains.insert(GameplayDomain::Property);
    }
    if earlier.active_employment != later.active_employment
        || earlier.disputed_employment != later.disputed_employment
        || earlier.player_active_employment != later.player_active_employment
        || earlier.player_disputed_employment != later.player_disputed_employment
        || earlier.average_labor_loyalty != later.average_labor_loyalty
        || earlier.employment_state_checksum != later.employment_state_checksum
    {
        domains.insert(GameplayDomain::Labor);
    }
    if earlier.average_relationship_trust != later.average_relationship_trust
        || earlier.average_relationship_respect != later.average_relationship_respect
        || earlier.average_relationship_fear != later.average_relationship_fear
        || earlier.average_relationship_resentment != later.average_relationship_resentment
        || earlier.relationship_obligation_total != later.relationship_obligation_total
        || earlier.relationship_memory_count != later.relationship_memory_count
        || earlier.relationship_state_checksum != later.relationship_state_checksum
    {
        domains.insert(GameplayDomain::Relationships);
    }
}

pub(crate) fn compare_dynasty_and_civic(
    earlier: &GameplaySnapshot,
    later: &GameplaySnapshot,
    domains: &mut BTreeSet<GameplayDomain>,
) {
    if earlier.legitimacy != later.legitimacy
        || earlier.quality_reputation != later.quality_reputation
        || earlier.reliability_reputation != later.reliability_reputation
        || earlier.player_unmet_office_duties != later.player_unmet_office_duties
        || earlier.generation != later.generation
        || earlier.achieved_ai_objectives != later.achieved_ai_objectives
        || earlier.dynasty_state_checksum != later.dynasty_state_checksum
        || earlier.ai_objective_state_checksum != later.ai_objective_state_checksum
    {
        domains.insert(GameplayDomain::Dynasty);
    }
    if earlier.family_unity != later.family_unity
        || earlier.family_charter_version != later.family_charter_version
        || earlier.house_governance != later.house_governance
        || earlier.active_wards != later.active_wards
        || earlier.player_family_capability_checksum != later.player_family_capability_checksum
        || earlier.character_state_checksum != later.character_state_checksum
        || earlier.family_state_checksum != later.family_state_checksum
    {
        domains.insert(GameplayDomain::Family);
    }
    if earlier.offices_held != later.offices_held
        || earlier.eligible_officeholders != later.eligible_officeholders
        || earlier.player_office_checksum != later.player_office_checksum
        || earlier.institution_memberships != later.institution_memberships
        || earlier.player_institutions_represented != later.player_institutions_represented
        || earlier.institution_budget_total != later.institution_budget_total
        || earlier.player_civic_contributions != later.player_civic_contributions
        || earlier.player_unmet_office_duties != later.player_unmet_office_duties
        || earlier.institution_state_checksum != later.institution_state_checksum
    {
        domains.insert(GameplayDomain::Institutions);
    }
    if earlier.active_laws != later.active_laws
        || earlier.active_law_kinds != later.active_law_kinds
        || earlier.law_value_checksum != later.law_value_checksum
        || earlier.active_law_checksum != later.active_law_checksum
        || earlier.law_state_checksum != later.law_state_checksum
    {
        domains.insert(GameplayDomain::Law);
    }
    if earlier.average_food_satisfaction != later.average_food_satisfaction
        || earlier.minimum_district_food_satisfaction != later.minimum_district_food_satisfaction
        || earlier.average_district_unrest != later.average_district_unrest
        || earlier.average_district_employment != later.average_district_employment
        || earlier.average_district_sanitation != later.average_district_sanitation
        || earlier.average_district_safety != later.average_district_safety
        || earlier.district_conditions != later.district_conditions
        || earlier.public_work_progress_total != later.public_work_progress_total
        || earlier.building_public_works != later.building_public_works
        || earlier.completed_public_works != later.completed_public_works
        || earlier.suspended_public_works != later.suspended_public_works
        || earlier.player_completed_public_work_kinds != later.player_completed_public_work_kinds
        || earlier.player_completed_public_work_checksum
            != later.player_completed_public_work_checksum
        || earlier.public_work_state_checksum != later.public_work_state_checksum
        || earlier.district_state_checksum != later.district_state_checksum
    {
        domains.insert(GameplayDomain::Districts);
    }
}

pub(crate) fn compare_world_and_information(
    earlier: &GameplaySnapshot,
    later: &GameplaySnapshot,
    domains: &mut BTreeSet<GameplayDomain>,
) {
    if earlier.open_legal_cases != later.open_legal_cases
        || earlier.decided_legal_cases != later.decided_legal_cases
        || earlier.legal_case_state_checksum != later.legal_case_state_checksum
    {
        domains.insert(GameplayDomain::Legal);
    }
    if earlier.active_crises != later.active_crises
        || earlier.escalated_crises != later.escalated_crises
        || earlier.resolved_crises != later.resolved_crises
        || earlier.crisis_severity_total != later.crisis_severity_total
        || earlier.crisis_state_checksum != later.crisis_state_checksum
    {
        domains.insert(GameplayDomain::Crises);
    }
    if earlier.information_reports != later.information_reports
        || earlier.information_report_checksum != later.information_report_checksum
        || earlier.information_state_checksum != later.information_state_checksum
    {
        domains.insert(GameplayDomain::Information);
    }
    if earlier.unread_notifications != later.unread_notifications
        || earlier.outbox_messages != later.outbox_messages
        || earlier.chronicle_entries != later.chronicle_entries
        || earlier.outbox_state_checksum != later.outbox_state_checksum
        || earlier.chronicle_state_checksum != later.chronicle_state_checksum
    {
        domains.insert(GameplayDomain::Feedback);
    }
}

pub(crate) fn initialized_command_stats() -> BTreeMap<GameplayCommandKind, GameplayCommandStats> {
    ALL_COMMAND_KINDS
        .into_iter()
        .map(|kind| (kind, GameplayCommandStats::default()))
        .collect()
}

pub(crate) fn initialized_phase_stats() -> BTreeMap<GameplayPhase, GameplayPhaseStats> {
    [
        GameplayPhase::Foundation,
        GameplayPhase::Establishment,
        GameplayPhase::InstitutionalAscent,
        GameplayPhase::DynasticGovernance,
        GameplayPhase::SuccessionLegacy,
    ]
    .into_iter()
    .map(|phase| (phase, GameplayPhaseStats::default()))
    .collect()
}

pub(crate) fn initialized_phase_counts() -> BTreeMap<GameplayPhase, u32> {
    initialized_phase_stats()
        .into_keys()
        .map(|phase| (phase, 0))
        .collect()
}

pub(crate) fn initialized_domain_counts() -> BTreeMap<GameplayDomain, u32> {
    ALL_DOMAINS.into_iter().map(|domain| (domain, 0)).collect()
}

pub(crate) fn interaction_vec(
    interactions: &BTreeMap<(GameplayCommandKind, GameplayDomain), u32>,
) -> Vec<GameplayInteractionEdge> {
    interactions
        .iter()
        .map(
            |((command, domain), observations)| GameplayInteractionEdge {
                command: *command,
                domain: *domain,
                observations: *observations,
            },
        )
        .collect()
}

pub(crate) fn select_trace(
    mut trace: Vec<GameplayTraceStep>,
    limit: usize,
) -> Vec<GameplayTraceStep> {
    if limit == 0 {
        return Vec::new();
    }
    if trace.len() <= limit {
        return trace;
    }
    let edge_count = (limit / 4).max(1);
    let mut indices = BTreeSet::new();
    indices.extend(0..edge_count.min(trace.len()));
    indices.extend(trace.len().saturating_sub(edge_count)..trace.len());
    let mut ranked: Vec<_> = trace
        .iter()
        .enumerate()
        .map(|(index, step)| (step.consequence_breadth(), step.viable_candidates, index))
        .collect();
    ranked.sort_by(|left, right| right.cmp(left));
    for (_, _, index) in ranked {
        if indices.len() >= limit {
            break;
        }
        indices.insert(index);
    }
    trace
        .drain(..)
        .enumerate()
        .filter_map(|(index, step)| indices.contains(&index).then_some(step))
        .collect()
}

pub(crate) fn count_business_status(
    businesses: &[&crate::core::Business],
    status: BusinessStatus,
) -> u16 {
    usize_to_u16(
        businesses
            .iter()
            .filter(|business| business.status() == status)
            .count(),
    )
}

pub(crate) fn count_contract_status(state: &AppState, status: ContractStatus) -> u16 {
    usize_to_u16(
        state
            .contracts
            .values()
            .filter(|contract| contract.status == status)
            .count(),
    )
}

pub(crate) fn count_loan_status(state: &AppState, status: LoanStatus) -> u16 {
    usize_to_u16(
        state
            .loans
            .values()
            .filter(|loan| loan.status == status)
            .count(),
    )
}

pub(crate) fn count_player_lending_status(
    state: &AppState,
    player_id: DynastyId,
    status: LoanStatus,
) -> u16 {
    usize_to_u16(
        state
            .loans
            .values()
            .filter(|loan| loan.lender_dynasty_id == player_id && loan.status == status)
            .count(),
    )
}

pub(crate) fn count_civic_debt_status(state: &AppState, status: CivicDebtStatus) -> u16 {
    usize_to_u16(
        state
            .civic_debts
            .values()
            .filter(|debt| debt.status == status)
            .count(),
    )
}

pub(crate) fn count_employment_status(state: &AppState, status: EmploymentStatus) -> u16 {
    usize_to_u16(
        state
            .employment
            .values()
            .filter(|agreement| agreement.status == status)
            .count(),
    )
}

pub(crate) fn count_player_offices(state: &AppState, player_id: DynastyId) -> u16 {
    usize_to_u16(
        state
            .institutions
            .values()
            .filter(|institution| {
                institution.office_holder_id.is_some_and(|character_id| {
                    state
                        .characters
                        .get(character_id)
                        .is_some_and(|character| character.dynasty_id() == player_id)
                })
            })
            .count(),
    )
}

pub(crate) fn count_eligible_officeholders(state: &AppState, player_id: DynastyId) -> u16 {
    usize_to_u16(
        state
            .characters
            .iter()
            .filter(|character| {
                character.dynasty_id() == player_id && character.status() == CharacterStatus::Active
            })
            .count(),
    )
}

pub(crate) fn player_family_capability_checksum(state: &AppState, player_id: DynastyId) -> u32 {
    state
        .characters
        .iter()
        .filter(|character| {
            character.dynasty_id() == player_id && character.status() == CharacterStatus::Active
        })
        .fold(0_u32, |total, character| {
            total
                .saturating_add(u32::from(character.capabilities.administration) * 11)
                .saturating_add(u32::from(character.capabilities.commerce) * 13)
                .saturating_add(u32::from(character.capabilities.social) * 17)
                .saturating_add(u32::from(character.capabilities.craft) * 19)
        })
}

pub(crate) fn count_player_memberships(state: &AppState, player_id: DynastyId) -> u16 {
    usize_to_u16(
        state
            .institutions
            .values()
            .map(|institution| {
                institution
                    .members
                    .iter()
                    .filter(|character_id| {
                        state
                            .characters
                            .get(**character_id)
                            .is_some_and(|character| character.dynasty_id() == player_id)
                    })
                    .count()
            })
            .sum(),
    )
}

pub(crate) fn count_player_institutions_represented(state: &AppState, player_id: DynastyId) -> u16 {
    usize_to_u16(
        state
            .institutions
            .values()
            .filter(|institution| {
                institution.members.iter().any(|character_id| {
                    state
                        .characters
                        .get(*character_id)
                        .is_some_and(|character| character.dynasty_id() == player_id)
                })
            })
            .count(),
    )
}

pub(crate) fn average_u16(values: impl Iterator<Item = u16>) -> u16 {
    let (total, count) = values.fold((0_u64, 0_u64), |(total, count), value| {
        (
            total.saturating_add(u64::from(value)),
            count.saturating_add(1),
        )
    });
    u16::try_from(total.checked_div(count).unwrap_or(0)).unwrap_or(u16::MAX)
}

pub(crate) fn average_scores(values: &[u16]) -> u16 {
    average_u16(values.iter().copied())
}

pub(crate) fn scaled_ratio_u64(numerator: u64, denominator: u64, scale: u64) -> u64 {
    if denominator == 0 {
        return 0;
    }
    let result = u128::from(numerator).saturating_mul(u128::from(scale)) / u128::from(denominator);
    u64::try_from(result).unwrap_or(u64::MAX)
}

pub(crate) fn scaled_ratio_usize(numerator: usize, denominator: usize, scale: u64) -> u64 {
    scaled_ratio_u64(
        u64::try_from(numerator).unwrap_or(u64::MAX),
        u64::try_from(denominator).unwrap_or(u64::MAX),
        scale,
    )
}

pub(crate) fn ratio_score(numerator: u32, denominator: u32) -> u16 {
    u16::try_from(scaled_ratio_u64(
        u64::from(numerator),
        u64::from(denominator),
        100,
    ))
    .unwrap_or(100)
    .min(100)
}

pub(crate) fn usize_to_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

pub(crate) fn usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
