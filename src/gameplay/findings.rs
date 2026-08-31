//! Gameplay findings and limitations — the interpretation layer.
//!
//! Purpose: turn reconciled `GameplayAggregate` / `GameplayCampaignReport`
//! evidence into `Info`/`Warning`/`Critical` findings and stated limitations.
//! Owns: every finding rule (unreachable command families, quiet-cycle
//! dominance, weak interconnection, narrow mature play, etc.) and their
//! severity thresholds.
//! Reads: `GameplayHarnessReport` aggregates only (no direct state).
//! Mutates: nothing (pure predicates over the report).
//! Does not own: harness orchestration or scoring weights.
//! Invariants: every finding carries severity, domain, and reproducibility
//! context; no rule invents evidence absent from the report; limited tiers
//! stay combinable.
//! Focused tests: `src/gameplay_tests.rs` finding-rule unit tests.

#[allow(clippy::wildcard_imports)] // the module tree re-exports one flat namespace
use super::*;

pub(crate) fn derive_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
) -> Vec<GameplayFinding> {
    let mut findings = Vec::new();
    add_score_findings(aggregate, &mut findings);
    add_command_findings(aggregate, &mut findings);
    add_domain_findings(aggregate, &mut findings);
    add_action_concentration_finding(aggregate, &mut findings);
    add_operational_rebalancing_finding(aggregate, &mut findings);
    add_institutional_campaign_concentration_finding(aggregate, &mut findings);
    add_phase_institutional_campaign_concentration_finding(aggregate, &mut findings);
    add_repetitive_command_streak_finding(campaigns, &mut findings);
    add_information_routine_finding(campaigns, &mut findings);
    add_information_leverage_trajectory_finding(aggregate, &mut findings);
    add_crisis_trajectory_finding(aggregate, &mut findings);
    add_crisis_coverage_finding(aggregate, campaigns, &mut findings);
    add_crisis_determinism_finding(aggregate, campaigns, &mut findings);
    add_office_directive_trajectory_finding(aggregate, &mut findings);
    add_welfare_dynamism_finding(aggregate, campaigns, &mut findings);
    add_long_horizon_risk_findings(aggregate, campaigns, &mut findings);
    add_player_borrowing_distress_finding(campaigns, &mut findings);
    add_counterparty_risk_finding(campaigns, &mut findings);
    add_mature_capital_pressure_finding(campaigns, &mut findings);
    add_starting_trade_economic_balance_finding(campaigns, &mut findings);
    add_early_background_imbalance_finding(campaigns, &mut findings);
    add_wealth_rank_persistence_finding(campaigns, &mut findings);
    add_rival_commercial_pressure_finding(aggregate, campaigns, &mut findings);
    add_succession_cohesion_finding(campaigns, &mut findings);
    add_succession_political_recovery_finding(campaigns, &mut findings);
    add_long_substantive_gap_finding(campaigns, &mut findings);
    add_asset_liquidity_drought_finding(campaigns, &mut findings);
    add_economic_recovery_dead_end_finding(campaigns, &mut findings);
    add_campaign_blocking_finding(campaigns, &mut findings);
    add_business_survival_finding(campaigns, &mut findings);
    add_system_health_findings(aggregate, campaigns, &mut findings);
    add_choice_quality_finding(aggregate, &mut findings);
    add_institutional_reach_finding(campaigns, &mut findings);
    add_property_concentration_finding(aggregate, campaigns, &mut findings);
    add_property_affordability_finding(campaigns, &mut findings);
    add_strategic_cadence_finding(aggregate, campaigns, &mut findings);
    add_phase_quality_findings(aggregate, campaigns, &mut findings);
    add_phase_action_mix_findings(aggregate, &mut findings);
    add_individual_action_concentration_finding(campaigns, &mut findings);
    add_persona_variety_findings(campaigns, &mut findings);
    add_core_fantasy_findings(aggregate, campaigns, &mut findings);
    add_succession_before_office_finding(campaigns, &mut findings);
    add_short_horizon_background_imbalance_finding(campaigns, &mut findings);
    add_governance_phase_gap_finding(aggregate, campaigns, &mut findings);
    add_variance_finding(campaigns, &mut findings);
    if findings.is_empty() {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Info,
            title: "No material gameplay harness concerns".to_owned(),
            evidence: "All configured command and system thresholds were satisfied.".to_owned(),
        });
    }
    findings
}

pub(crate) fn add_persona_variety_findings(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    const MINIMUM_MATURE_PERSONA_VARIETY: u16 = 70;

    for (persona, aggregate) in aggregate_campaigns_by_persona(campaigns) {
        if average_campaign_days(&aggregate) < 3_600
            || aggregate.scores.variety >= MINIMUM_MATURE_PERSONA_VARIETY
        {
            continue;
        }
        let mut executed = aggregate
            .commands
            .iter()
            .filter(|(kind, stats)| is_substantive_command_kind(**kind) && stats.executed > 0)
            .map(|(kind, stats)| (*kind, stats.executed))
            .collect::<Vec<_>>();
        executed.sort_by_key(|(kind, count)| (std::cmp::Reverse(*count), *kind));
        let leading_actions = executed
            .into_iter()
            .take(3)
            .map(|(kind, count)| format!("{}={count}", kind.label()))
            .collect::<Vec<_>>()
            .join(", ");
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "A mature persona has weak strategic variety".to_owned(),
            evidence: format!(
                "The {} persona scored {} for variety across {} mature campaign(s), below the diagnostic floor of {MINIMUM_MATURE_PERSONA_VARIETY}. Its most-used substantive command families were {leading_actions}. Aggregate matrices can hide a persona that repeatedly reaches the same narrow slice of the game, so persona-level variety is evaluated independently.",
                persona.label(),
                aggregate.scores.variety,
                aggregate.campaigns,
            ),
        });
    }
}

pub(crate) fn add_player_borrowing_distress_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let Some(worst) = campaigns
        .iter()
        .filter(|campaign| campaign.simulated_days >= 720)
        .max_by_key(|campaign| {
            (
                campaign.end.player_defaulted_borrowing,
                campaign.maximum_player_defaulted_borrowing,
                campaign.end.player_delinquent_borrowing,
                campaign.maximum_player_delinquent_borrowing,
                std::cmp::Reverse(campaign.end.player_treasury),
            )
        })
    else {
        return;
    };
    if worst.maximum_player_defaulted_borrowing == 0
        && worst.maximum_player_delinquent_borrowing == 0
    {
        return;
    }
    let severity = if worst.end.player_defaulted_borrowing > 0
        || (worst.end.player_delinquent_borrowing > 0 && worst.end.player_treasury <= Money::ZERO)
    {
        GameplayFindingSeverity::Warning
    } else {
        GameplayFindingSeverity::Info
    };
    findings.push(GameplayFinding {
        severity,
        title: "Player borrowing enters material credit distress".to_owned(),
        evidence: format!(
            "Seed {}, {} {:?} reached {} delinquent and {} defaulted player-borrowed loan(s) at peak; it ended with {} delinquent, {} defaulted, treasury {}, and {} properties. Borrower distress is now tracked separately from unrelated private defaults and player-issued credit risk.",
            worst.seed,
            worst.persona.label(),
            worst.background,
            worst.maximum_player_delinquent_borrowing,
            worst.maximum_player_defaulted_borrowing,
            worst.end.player_delinquent_borrowing,
            worst.end.player_defaulted_borrowing,
            worst.end.player_treasury,
            worst.end.player_properties,
        ),
    });
}

pub(crate) fn add_mature_capital_pressure_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let mature: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.simulated_days >= 3_600)
        .collect();
    if mature.len() < 4 {
        return;
    }
    let financially_unpressured: Vec<_> = mature
        .iter()
        .copied()
        .filter(|campaign| {
            let growth_floor = campaign.start.player_treasury.saturating_mul(5);
            campaign.end.player_treasury >= growth_floor.max(Money::from_copper(200_000))
                && campaign.maximum_player_delinquent_borrowing == 0
                && campaign.maximum_player_defaulted_borrowing == 0
        })
        .collect();
    if scaled_ratio_usize(financially_unpressured.len(), mature.len(), 100) < 50 {
        return;
    }
    let liquidators = financially_unpressured
        .iter()
        .filter(|campaign| {
            campaign
                .commands
                .get(&GameplayCommandKind::SellProperty)
                .is_some_and(|stats| stats.executed > 0)
        })
        .count();
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Mature liquidity can outgrow meaningful financial pressure".to_owned(),
        evidence: format!(
            "{} of {} mature campaigns ended with at least five times their starting treasury and at least 2,000 cr in liquid dynasty cash without ever entering player-borrowing delinquency or default; only {liquidators} of those campaigns needed to liquidate property. This is an anti-snowball warning: successful houses may be accumulating cash faster than business investment, credit, civic commitments, family strategy, and political obligations can absorb it.",
            financially_unpressured.len(),
            mature.len(),
        ),
    });
}

pub(crate) fn add_early_background_imbalance_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let mut averages = Vec::new();
    let backgrounds: BTreeSet<StartingBackground> = campaigns
        .iter()
        .map(|campaign| campaign.background)
        .collect();
    for background in backgrounds {
        let sampled: Vec<_> = campaigns
            .iter()
            .filter(|campaign| {
                campaign.background == background && campaign.simulated_days >= 1_080
            })
            .collect();
        if sampled.len() < 4 {
            continue;
        }
        let total_margin: i128 = sampled.iter().fold(0_i128, |sum, campaign| {
            let margin = campaign
                .end
                .player_business_lifetime_revenue
                .copper()
                .saturating_sub(campaign.end.player_business_lifetime_costs.copper());
            sum.saturating_add(i128::from(margin))
        });
        let avg_margin = total_margin / i128::try_from(sampled.len()).expect("count fits i128");
        averages.push((background, avg_margin));
    }
    if averages.len() < 2 {
        return;
    }
    let Some((strongest_background, strongest_avg)) =
        averages.iter().max_by_key(|(_, avg)| *avg).copied()
    else {
        return;
    };
    let Some((weakest_background, weakest_avg)) =
        averages.iter().min_by_key(|(_, avg)| *avg).copied()
    else {
        return;
    };
    let strongest_treasury = campaigns
        .iter()
        .filter(|c| c.background == strongest_background)
        .map(|c| c.end.player_treasury.copper())
        .sum::<i64>()
        / i64::try_from(
            campaigns
                .iter()
                .filter(|c| c.background == strongest_background)
                .count(),
        )
        .unwrap_or(0);
    let weakest_treasury = campaigns
        .iter()
        .filter(|c| c.background == weakest_background)
        .map(|c| c.end.player_treasury.copper())
        .sum::<i64>()
        / i64::try_from(
            campaigns
                .iter()
                .filter(|c| c.background == weakest_background)
                .count(),
        )
        .unwrap_or(0);
    let margin_spread = strongest_avg.saturating_sub(weakest_avg);
    let treasury_spread = strongest_treasury.saturating_sub(weakest_treasury);
    if margin_spread < 30_000 && treasury_spread < 50_000 {
        return;
    }
    if weakest_avg < 0 && strongest_avg > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Early background economics diverge sharply by trade".to_owned(),
            evidence: format!(
                "At 1080-day horizon, {strongest_background:?} averaged margin {} vs {weakest_background:?} at {} (spread {}), with treasury {} vs {}. A trade that is structurally unprofitable in the first three years creates a hidden difficulty mode rather than distinct pressures.",
                Money::from_copper(i64::try_from(strongest_avg).expect("avg fits")),
                Money::from_copper(i64::try_from(weakest_avg).expect("avg fits")),
                Money::from_copper(i64::try_from(margin_spread).expect("spread fits")),
                Money::from_copper(strongest_treasury),
                Money::from_copper(weakest_treasury),
            ),
        });
    }
}

pub(crate) fn add_wealth_rank_persistence_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.len() < 8 {
        return;
    }
    let dynasty_count = campaigns
        .first()
        .map_or(0, |c| c.rival_context.dynasty_count);
    let poorest = campaigns
        .iter()
        .filter(|c| c.rival_context.player_treasury_rank == dynasty_count)
        .count();
    if scaled_ratio_usize(poorest, campaigns.len(), 100) < 75 {
        return;
    }
    let best_rank = campaigns
        .iter()
        .map(|c| c.rival_context.player_treasury_rank)
        .min()
        .unwrap_or(dynasty_count);
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Player wealth stays persistently bottom-ranked".to_owned(),
        evidence: format!(
            "{poorest} of {} campaigns ended ranked {dynasty_count}/{dynasty_count} by treasury (poorest), with best rank {best_rank}/{dynasty_count}. Persistent bottom wealth suggests starting capital, early margins, or AI wealth scaling leaves little room to outcompete rivals within the evaluated horizon.",
            campaigns.len()
        ),
    });
}

pub(crate) fn add_starting_trade_economic_balance_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let mut averages = Vec::new();
    let backgrounds: BTreeSet<StartingBackground> = campaigns
        .iter()
        .map(|campaign| campaign.background)
        .collect();
    for background in backgrounds {
        let mature: Vec<_> = campaigns
            .iter()
            .filter(|campaign| {
                campaign.background == background && campaign.simulated_days >= 3_600
            })
            .collect();
        if mature.len() < 4 {
            continue;
        }
        let total = mature.iter().fold(0_i128, |sum, campaign| {
            sum.saturating_add(i128::from(campaign.end.player_treasury.copper()))
        });
        averages.push((
            background,
            total / i128::try_from(mature.len()).expect("campaign count must fit i128"),
        ));
    }
    if averages.len() < 2 {
        return;
    }
    let Some((strongest_background, strongest_average)) =
        averages.iter().max_by_key(|(_, average)| *average).copied()
    else {
        return;
    };
    let Some((weakest_background, weakest_average)) =
        averages.iter().min_by_key(|(_, average)| *average).copied()
    else {
        return;
    };
    if strongest_average < weakest_average.saturating_mul(2)
        || strongest_average.saturating_sub(weakest_average) < 100_000
    {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Starting trade behaves like a hidden mature-economy advantage".to_owned(),
        evidence: format!(
            "Mature {strongest_background:?} campaigns ended with average dynasty treasury {} versus {} for {weakest_background:?}, more than a twofold gap. Starting trades are intended to create different pressures and opportunities, not a hidden difficulty mode, so persistent endpoint liquidity this far apart indicates background economics need review.",
            Money::from_copper(
                i64::try_from(strongest_average).expect("average treasury must fit money range")
            ),
            Money::from_copper(
                i64::try_from(weakest_average).expect("average treasury must fit money range")
            ),
        ),
    });
}

pub(crate) fn add_property_concentration_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.len() < 8 || average_campaign_days(aggregate) < 720 {
        return;
    }
    let repeated_acquirers = campaigns
        .iter()
        .filter(|campaign| {
            campaign
                .commands
                .get(&GameplayCommandKind::BuyProperty)
                .is_some_and(|stats| stats.executed >= 2)
        })
        .count();
    if scaled_ratio_usize(repeated_acquirers, campaigns.len(), 100) < 50 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Property acquisition becomes a universal progression path".to_owned(),
        evidence: format!(
            "{repeated_acquirers} of {} campaigns acquired at least two additional properties. Repeated land acquisition across distinct personas is a concentration signal because property is intended to compete with business investment, credit, family capacity, and political commitments rather than become an automatic wealth step.",
            campaigns.len()
        ),
    });
}

pub(crate) fn add_property_affordability_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    // Distinguishes an affordability ceiling from a deliberate choice: a
    // campaign whose peak treasury never reached the cheapest unowned
    // property never had the option to decline. That is a game-economy
    // signal (income too weak or asset prices too high), not agent restraint.
    let unreachable: Vec<&GameplayCampaignReport> = campaigns
        .iter()
        .filter(|campaign| {
            campaign
                .commands
                .get(&GameplayCommandKind::BuyProperty)
                .is_none_or(|stats| stats.executed == 0)
                && campaign
                    .minimum_unowned_property_value
                    .is_some_and(|cheapest| campaign.peak_player_treasury < cheapest)
        })
        .collect();
    if unreachable.is_empty() || unreachable.len() * 2 < campaigns.len() {
        return;
    }
    let worst = unreachable
        .iter()
        .min_by_key(|campaign| campaign.peak_player_treasury)
        .copied()
        .expect("unreachable campaigns are non-empty");
    let worst_cheapest = worst
        .minimum_unowned_property_value
        .map_or_else(|| "n/a".to_owned(), |value| value.to_string());
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Info,
        title: "Property market stayed priced out of reach".to_owned(),
        evidence: format!(
            "{} of {} campaigns never bought property and their peak treasury never reached the cheapest unowned property price, so the purchase route was unaffordable rather than declined; review dynasty income scaling or entry-level asset pricing. Worst: seed {} {} peaked at {} against a cheapest property of {}.",
            unreachable.len(),
            campaigns.len(),
            worst.seed,
            worst.persona.label(),
            worst.peak_player_treasury,
            worst_cheapest,
        ),
    });
}

pub(crate) fn add_rival_commercial_pressure_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 3_600 || campaigns.len() < 4 {
        return;
    }
    let pressured = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_contract_relationship_pressure_basis_points >= 1_000)
        .count();
    if scaled_ratio_usize(pressured, campaigns.len(), 100) >= 50 {
        return;
    }
    let maximum = campaigns
        .iter()
        .map(|campaign| campaign.maximum_contract_relationship_pressure_basis_points)
        .max()
        .unwrap_or(0);
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Rivalry rarely changes commercial leverage".to_owned(),
        evidence: format!(
            "Only {pressured} of {} mature campaigns ever reached 1,000 bp of relationship-driven contract pressure; the maximum observed pressure was {maximum} bp. Rival houses may dislike the player, but that hostility is not consistently changing the price of doing business with them.",
            campaigns.len()
        ),
    });
}

pub(crate) fn add_succession_cohesion_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let succession_campaigns: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.fantasy_arc.first_succession_day.is_some())
        .collect();
    if succession_campaigns.len() < 4 {
        return;
    }
    let highly_stable = succession_campaigns
        .iter()
        .filter(|campaign| {
            campaign
                .minimum_post_succession_family_unity
                .is_some_and(|unity| unity >= 7_000)
        })
        .count();
    if scaled_ratio_usize(highly_stable, succession_campaigns.len(), 100) < 75 {
        return;
    }
    let minimum = succession_campaigns
        .iter()
        .filter_map(|campaign| campaign.minimum_post_succession_family_unity)
        .min()
        .unwrap_or(10_000);
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Succession rarely destabilizes family cohesion".to_owned(),
        evidence: format!(
            "{highly_stable} of {} succession campaigns never fell below 7,000 bp of family unity after transition; the lowest observed post-succession unity was {minimum} bp. Inheritance changes the officeholder, but the family order is usually too stable to demand a new internal strategy.",
            succession_campaigns.len()
        ),
    });
}

pub(crate) fn add_succession_political_recovery_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let stranded = campaigns.iter().filter_map(|campaign| {
        let transition = campaign.succession_transition?;
        let post_succession_days = campaign.end.day.saturating_sub(transition.day);
        if post_succession_days < 720
            || (transition.offices_before < 2 && transition.represented_institutions_before < 2)
        {
            return None;
        }
        let lost_reach = transition.offices_after < transition.offices_before
            || transition.represented_institutions_after
                < transition.represented_institutions_before;
        if !lost_reach {
            return None;
        }
        let phase = campaign.phase_stats.get(&GameplayPhase::SuccessionLegacy)?;
        let rebuild_actions = phase
            .executed_commands
            .get(&GameplayCommandKind::CultivateInstitutionSupport)
            .copied()
            .unwrap_or(0)
            .saturating_add(
                phase
                    .executed_commands
                    .get(&GameplayCommandKind::NominateForOffice)
                    .copied()
                    .unwrap_or(0),
            );
        let still_weaker = campaign.end.offices_held < transition.offices_before
            || campaign.end.player_institutions_represented
                < transition.represented_institutions_before;
        (rebuild_actions == 0
            && still_weaker
            && campaign.end.legitimacy < OFFICE_POWER_DIRECTIVE_LEGITIMACY_COST)
            .then_some((campaign, transition, post_succession_days))
    });
    let Some((campaign, transition, post_succession_days)) =
        stranded.max_by_key(|(campaign, transition, _)| {
            (
                transition
                    .offices_before
                    .saturating_sub(campaign.end.offices_held),
                transition
                    .represented_institutions_before
                    .saturating_sub(campaign.end.player_institutions_represented),
            )
        })
    else {
        return;
    };
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Political succession can strand institutional recovery".to_owned(),
        evidence: format!(
            "Seed {}, {} {:?} entered succession with {} office(s), {} institutional membership(s), {} represented institution(s), and {} bp legitimacy. The first post-transition observation had {} office(s), {} membership(s), {} represented institution(s), and {} bp legitimacy. After another {post_succession_days} day(s), the dynasty ended with {} office(s), {} represented institution(s), and {} bp legitimacy without executing institutional patronage or a new office campaign. A dynasty built around political embedding needs an explicit recovery route after succession loss.",
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
            transition.offices_before,
            transition.institution_memberships_before,
            transition.represented_institutions_before,
            transition.legitimacy_before,
            transition.offices_after,
            transition.institution_memberships_after,
            transition.represented_institutions_after,
            transition.legitimacy_after,
            campaign.end.offices_held,
            campaign.end.player_institutions_represented,
            campaign.end.legitimacy,
        ),
    });
}

pub(crate) fn add_strategic_cadence_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if aggregate.decision_cycles == 0 {
        return;
    }
    let static_quiet_cycles = aggregate
        .quiet_cycles
        .saturating_sub(aggregate.quiet_cycles_with_ambient_change);
    let static_quiet_share = scaled_ratio_u64(static_quiet_cycles, aggregate.decision_cycles, 100);
    if static_quiet_share < 25 {
        return;
    }
    let worst = campaigns.iter().max_by_key(|campaign| {
        let static_quiet = campaign
            .quiet_cycles
            .saturating_sub(campaign.quiet_cycles_with_ambient_change);
        scaled_ratio_u64(
            u64::from(static_quiet),
            u64::from(campaign.decision_cycles),
            100,
        )
    });
    let worst_evidence = worst.map_or_else(String::new, |campaign| {
        let campaign_static_quiet = campaign
            .quiet_cycles
            .saturating_sub(campaign.quiet_cycles_with_ambient_change);
        let campaign_static_quiet_share = scaled_ratio_u64(
            u64::from(campaign_static_quiet),
            u64::from(campaign.decision_cycles),
            100,
        );
        format!(
            " The most static campaign was seed {}, {} {:?}, at {campaign_static_quiet_share}%.",
            campaign.seed,
            campaign.persona.label(),
            campaign.background
        )
    });
    findings.push(GameplayFinding {
        severity: if static_quiet_share >= 40 {
            GameplayFindingSeverity::Critical
        } else {
            GameplayFindingSeverity::Warning
        },
        title: "Strategic cadence leaves too many static decision cycles".to_owned(),
        evidence: format!(
            "{} of {} decision cycles were quiet, but {} still contained ambient world change. The remaining {static_quiet_cycles} static cycles were {static_quiet_share}% of all decisions.{worst_evidence}",
            aggregate.quiet_cycles,
            aggregate.decision_cycles,
            aggregate.quiet_cycles_with_ambient_change,
        ),
    });
}

#[derive(Clone, Copy)]
pub(crate) struct PhaseQualityThresholds {
    pub minimum_action_share: u64,
    pub maximum_static_quiet_share: u64,
    pub maximum_quiet_streak_cycles: u32,
    pub minimum_multi_family_share: u64,
    pub minimum_average_choices_tenths: u64,
    pub minimum_average_families_tenths: u64,
    pub require_family_breadth: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct PhaseQualityMeasures {
    pub action_share: u64,
    pub quiet_share: u64,
    pub static_quiet_share: u64,
    pub multi_family_share: u64,
    pub average_choices_tenths: u64,
    pub average_families_tenths: u64,
}

// At the default 30-day decision cadence, an annual 360-day civic commitment leaves eleven
// observation cycles between the action and its next legal opportunity. Mature governance should
// not call that intentional cadence a drought when every intervening cycle still contains world
// movement. A twelfth consecutive quiet cycle exceeds the annual commitment window.
pub(crate) const GOVERNANCE_MAX_QUIET_STREAK_CYCLES: u32 = 11;

impl PhaseQualityMeasures {
    pub fn from_stats(stats: &GameplayPhaseStats) -> Self {
        let decision_cycles = u64::from(stats.decision_cycles);
        let opportunity_cycles =
            u64::from(stats.decision_cycles.saturating_sub(stats.quiet_cycles));
        Self {
            action_share: scaled_ratio_u64(
                u64::from(stats.substantive_actions),
                decision_cycles,
                100,
            ),
            quiet_share: scaled_ratio_u64(u64::from(stats.quiet_cycles), decision_cycles, 100),
            static_quiet_share: scaled_ratio_u64(
                u64::from(
                    stats
                        .quiet_cycles
                        .saturating_sub(stats.quiet_cycles_with_ambient_change),
                ),
                decision_cycles,
                100,
            ),
            multi_family_share: scaled_ratio_u64(
                u64::from(stats.cycles_with_multiple_viable_command_kinds),
                opportunity_cycles,
                100,
            ),
            average_choices_tenths: scaled_ratio_u64(
                u64::from(stats.total_viable_choices),
                opportunity_cycles,
                10,
            ),
            average_families_tenths: scaled_ratio_u64(
                u64::from(stats.total_viable_command_kinds),
                opportunity_cycles,
                10,
            ),
        }
    }
}

pub(crate) fn add_phase_quality_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    add_phase_quality_finding(
        aggregate,
        campaigns,
        findings,
        GameplayPhase::Establishment,
        "establishment",
        "Establishment becomes a waiting phase",
        PhaseQualityThresholds {
            minimum_action_share: 60,
            maximum_static_quiet_share: 40,
            maximum_quiet_streak_cycles: 6,
            minimum_multi_family_share: 25,
            minimum_average_choices_tenths: 25,
            minimum_average_families_tenths: 16,
            require_family_breadth: false,
        },
    );
    add_phase_quality_finding(
        aggregate,
        campaigns,
        findings,
        GameplayPhase::InstitutionalAscent,
        "ascent",
        "Institutional ascent lacks parallel political work",
        PhaseQualityThresholds {
            minimum_action_share: 60,
            maximum_static_quiet_share: 35,
            maximum_quiet_streak_cycles: 9,
            minimum_multi_family_share: 25,
            minimum_average_choices_tenths: 25,
            minimum_average_families_tenths: 15,
            require_family_breadth: false,
        },
    );
    add_phase_quality_finding(
        aggregate,
        campaigns,
        findings,
        GameplayPhase::DynasticGovernance,
        "governance",
        "Dynastic governance remains intermittent and strategically narrow",
        PhaseQualityThresholds {
            minimum_action_share: 0,
            maximum_static_quiet_share: 30,
            maximum_quiet_streak_cycles: GOVERNANCE_MAX_QUIET_STREAK_CYCLES,
            minimum_multi_family_share: 30,
            minimum_average_choices_tenths: 30,
            minimum_average_families_tenths: 16,
            require_family_breadth: true,
        },
    );
    add_phase_quality_finding(
        aggregate,
        campaigns,
        findings,
        GameplayPhase::SuccessionLegacy,
        "succession and legacy",
        "Succession and legacy lack post-transition strategy",
        PhaseQualityThresholds {
            minimum_action_share: 50,
            maximum_static_quiet_share: 35,
            maximum_quiet_streak_cycles: GOVERNANCE_MAX_QUIET_STREAK_CYCLES,
            minimum_multi_family_share: 30,
            minimum_average_choices_tenths: 25,
            minimum_average_families_tenths: 16,
            require_family_breadth: true,
        },
    );
}

pub(crate) fn add_phase_action_mix_findings(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    for phase in [
        GameplayPhase::Foundation,
        GameplayPhase::Establishment,
        GameplayPhase::InstitutionalAscent,
        GameplayPhase::DynasticGovernance,
        GameplayPhase::SuccessionLegacy,
    ] {
        let Some(stats) = aggregate.phase_stats.get(&phase) else {
            continue;
        };
        if stats.substantive_actions < 25 {
            continue;
        }
        let Some((kind, executed)) = stats
            .executed_commands
            .iter()
            .max_by_key(|(kind, count)| (**count, std::cmp::Reverse(**kind)))
        else {
            continue;
        };
        let share = scaled_ratio_u64(
            u64::from(*executed),
            u64::from(stats.substantive_actions),
            100,
        );
        if share < 25 {
            continue;
        }
        let intentional_foundation_setup = phase == GameplayPhase::Foundation
            && *kind == GameplayCommandKind::SetBusinessPolicy
            && *executed <= aggregate.campaigns;
        let warning_threshold = if phase == GameplayPhase::InstitutionalAscent
            && matches!(
                kind,
                GameplayCommandKind::CultivateInstitutionSupport
                    | GameplayCommandKind::NominateForOffice
            ) {
            65
        } else {
            35
        };
        findings.push(GameplayFinding {
            severity: if !intentional_foundation_setup && share >= warning_threshold {
                GameplayFindingSeverity::Warning
            } else {
                GameplayFindingSeverity::Info
            },
            title: format!("{} action mix is concentrated", phase.label()),
            evidence: format!(
                "{} accounted for {executed} of {} substantive {} actions ({share}%). Phase-level command usage is retained in the report so repeated optimization work cannot hide behind otherwise healthy choice and feedback scores.",
                kind.label(),
                stats.substantive_actions,
                phase.label()
            ),
        });
    }
}

pub(crate) fn add_individual_action_concentration_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let worst = campaigns
        .iter()
        .filter(|campaign| campaign.simulated_days >= 3_600)
        .filter_map(|campaign| {
            let substantive_actions = campaign
                .commands
                .iter()
                .filter(|(kind, _)| is_substantive_command_kind(**kind))
                .fold(0_u32, |total, (_, stats)| {
                    total.saturating_add(stats.executed)
                });
            if substantive_actions < 50 {
                return None;
            }
            let (kind, stats) = campaign
                .commands
                .iter()
                .filter(|(kind, _)| is_substantive_command_kind(**kind))
                .max_by_key(|(kind, stats)| (stats.executed, std::cmp::Reverse(**kind)))?;
            let share = scaled_ratio_u64(
                u64::from(stats.executed),
                u64::from(substantive_actions),
                100,
            );
            Some((campaign, *kind, stats.executed, substantive_actions, share))
        })
        .max_by_key(|(campaign, kind, executed, _, share)| {
            (*share, *executed, campaign.seed, campaign.persona, *kind)
        });
    let Some((campaign, kind, executed, substantive_actions, share)) = worst else {
        return;
    };
    if share < 25 {
        return;
    }
    findings.push(GameplayFinding {
        severity: if share >= 35 {
            GameplayFindingSeverity::Warning
        } else {
            GameplayFindingSeverity::Info
        },
        title: "An individual mature campaign has a concentrated action mix".to_owned(),
        evidence: format!(
            "Seed {}, {} {:?} used {} for {executed} of {substantive_actions} substantive actions ({share}%). Aggregate phase mixes can hide a repetitive persona-specific experience, so mature campaigns are also checked individually.",
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
            kind.label(),
        ),
    });
}

pub(crate) fn add_phase_quality_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
    phase: GameplayPhase,
    phase_label: &str,
    title: &str,
    thresholds: PhaseQualityThresholds,
) {
    let stats = aggregate
        .phase_stats
        .get(&phase)
        .cloned()
        .unwrap_or_default();
    if stats.decision_cycles < 20 {
        return;
    }
    let measures = PhaseQualityMeasures::from_stats(&stats);
    let action_share = measures.action_share;
    let quiet_share = measures.quiet_share;
    let static_quiet_share = measures.static_quiet_share;
    let multi_family_share = measures.multi_family_share;
    let average_choices_tenths = measures.average_choices_tenths;
    let average_families_tenths = measures.average_families_tenths;
    let missed_thresholds = phase_quality_missed_thresholds(&stats, measures, thresholds);
    if missed_thresholds.is_empty() {
        return;
    }
    let threshold_evidence = missed_thresholds.join("; ");
    let worst_streak_evidence = phase_worst_streak_evidence(campaigns, phase);
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: title.to_owned(),
        evidence: format!(
            "Across {} {phase_label} cycles, substantive actions occurred in {action_share}%, {quiet_share}% were quiet, {}% were quiet while the world still changed, {static_quiet_share}% were static, the longest quiet streak lasted {} cycles, multiple command families were viable in {multi_family_share}% of actionable cycles, and actionable cycles averaged {} viable choices across {} families. Quiet causes: policy gates {}, dormant {}, generator gaps {}, restrained routes {}, validation gates {}. Thresholds missed: {threshold_evidence}.{worst_streak_evidence}",
            stats.decision_cycles,
            scaled_ratio_u64(
                u64::from(stats.quiet_cycles_with_ambient_change),
                u64::from(stats.decision_cycles),
                100,
            ),
            stats.longest_quiet_streak_cycles,
            format_tenths(average_choices_tenths),
            format_tenths(average_families_tenths),
            stats.policy_gate_cycles,
            stats.dormant_cycles,
            stats.generator_gap_cycles,
            stats.restrained_cycles,
            stats.validation_gate_cycles
        ),
    });
}

pub(crate) fn phase_quality_missed_thresholds(
    stats: &GameplayPhaseStats,
    measures: PhaseQualityMeasures,
    thresholds: PhaseQualityThresholds,
) -> Vec<String> {
    let choices_are_sufficient =
        measures.average_choices_tenths >= thresholds.minimum_average_choices_tenths;
    let families_are_sufficient =
        measures.average_families_tenths >= thresholds.minimum_average_families_tenths;
    let choice_depth_is_sufficient = if thresholds.require_family_breadth {
        choices_are_sufficient && families_are_sufficient
    } else {
        choices_are_sufficient || families_are_sufficient
    };
    let mut missed_thresholds = Vec::new();
    if measures.action_share < thresholds.minimum_action_share {
        missed_thresholds.push(format!(
            "action share {}% < {}%",
            measures.action_share, thresholds.minimum_action_share
        ));
    }
    // A maximum expressed as an inclusive ceiling: landing exactly on the
    // configured share satisfies it, matching the strict `>` used by every
    // other maximum in this function.
    if measures.static_quiet_share > thresholds.maximum_static_quiet_share {
        missed_thresholds.push(format!(
            "static quiet share {}% > {}%",
            measures.static_quiet_share, thresholds.maximum_static_quiet_share
        ));
    }
    if stats.longest_quiet_streak_cycles > thresholds.maximum_quiet_streak_cycles {
        missed_thresholds.push(format!(
            "longest quiet streak {} > {} cycles",
            stats.longest_quiet_streak_cycles, thresholds.maximum_quiet_streak_cycles
        ));
    }
    if measures.multi_family_share < thresholds.minimum_multi_family_share {
        missed_thresholds.push(format!(
            "multi-family share {}% < {}%",
            measures.multi_family_share, thresholds.minimum_multi_family_share
        ));
    }
    if thresholds.require_family_breadth {
        if !choices_are_sufficient {
            missed_thresholds.push(format!(
                "average choice depth {} < {} choices",
                format_tenths(measures.average_choices_tenths),
                format_tenths(thresholds.minimum_average_choices_tenths)
            ));
        }
        if !families_are_sufficient {
            missed_thresholds.push(format!(
                "average family breadth {} < {} families",
                format_tenths(measures.average_families_tenths),
                format_tenths(thresholds.minimum_average_families_tenths)
            ));
        }
    } else if !choice_depth_is_sufficient {
        missed_thresholds.push(format!(
            "choice depth {} choices / {} families < {} choices or {} families",
            format_tenths(measures.average_choices_tenths),
            format_tenths(measures.average_families_tenths),
            format_tenths(thresholds.minimum_average_choices_tenths),
            format_tenths(thresholds.minimum_average_families_tenths)
        ));
    }
    missed_thresholds
}

pub(crate) fn phase_worst_streak_evidence(
    campaigns: &[GameplayCampaignReport],
    phase: GameplayPhase,
) -> String {
    campaigns
        .iter()
        .filter_map(|campaign| {
            campaign
                .phase_stats
                .get(&phase)
                .map(|stats| (campaign, stats.longest_quiet_streak_cycles))
        })
        .max_by_key(|(campaign, streak)| {
            (
                *streak,
                campaign.seed,
                campaign.persona,
                campaign.background.recipe_key(),
            )
        })
        .map_or_else(String::new, |(campaign, streak)| {
            format!(
                " Worst uninterrupted quiet streak: {streak} cycles in seed {}, {} {:?}.",
                campaign.seed,
                campaign.persona.label(),
                campaign.background
            )
        })
}

pub(crate) fn add_repetitive_command_streak_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let Some(campaign) = campaigns
        .iter()
        .max_by_key(|campaign| campaign.longest_substantive_command_streak)
    else {
        return;
    };
    if campaign.longest_substantive_command_streak < 8 {
        return;
    }
    let command = campaign
        .longest_substantive_streak_command
        .map_or("unknown", GameplayCommandKind::label);
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Repeated command streak resembles routine micromanagement".to_owned(),
        evidence: format!(
            "The longest streak was {} consecutive {command} actions for seed {}, {} {:?}.",
            campaign.longest_substantive_command_streak,
            campaign.seed,
            campaign.persona.label(),
            campaign.background
        ),
    });
}

pub(crate) fn add_information_routine_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let routine_campaigns: Vec<_> = campaigns
        .iter()
        .filter(|campaign| {
            // Political personas also accelerate their commission cadence in
            // response to relationship strain, which the report cannot
            // reconstruct after the fact; only the personas whose accelerated
            // cadence is fully explained by contract pressure are eligible
            // for the routine-ritual diagnosis.
            matches!(
                campaign.persona,
                GameplayPersona::Steward | GameplayPersona::Entrepreneur
            ) && campaign.maximum_contract_relationship_pressure_basis_points
                < AGENT_INFORMATION_SEVERE_COUNTERPARTY_PRESSURE_BASIS_POINTS
        })
        .collect();
    if routine_campaigns.is_empty() {
        return;
    }
    let commissions: u32 = routine_campaigns
        .iter()
        .map(|campaign| {
            campaign
                .commands
                .get(&GameplayCommandKind::CommissionInformation)
                .map_or(0, |stats| stats.executed)
        })
        .sum();
    let pairs: u32 = routine_campaigns
        .iter()
        .map(|campaign| u32::from(campaign.commission_leverage_pairs))
        .sum();
    let simulated_days = routine_campaigns.iter().fold(0_u64, |total, campaign| {
        total.saturating_add(u64::from(campaign.simulated_days))
    });
    let commissions_per_hundred_campaign_years = scaled_ratio_u64(
        u64::from(commissions).saturating_mul(360),
        simulated_days.max(1),
        100,
    );
    if commissions < 20
        || pairs.saturating_mul(100) < commissions.saturating_mul(75)
        || commissions_per_hundred_campaign_years < 50
    {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Commissioned intelligence becomes a routine two-step ritual".to_owned(),
        evidence: format!(
            "{pairs} of {commissions} commissioned reports in non-severe-pressure campaigns were leveraged within {INFORMATION_ROUTINE_PAIR_WINDOW_DAYS} days, at a rate of {commissions_per_hundred_campaign_years} commissions per 100 campaign-years. Intelligence is functioning, but the repeated commission-then-spend sequence risks becoming scheduled maintenance rather than a response to uncertainty. Campaigns that reached at least {AGENT_INFORMATION_SEVERE_COUNTERPARTY_PRESSURE_BASIS_POINTS} bp of relationship-driven contract pressure are excluded because their faster political intelligence cadence is an explicit response to material exposure."
        ),
    });
}

pub(crate) fn add_crisis_trajectory_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let stats = aggregate
        .commands
        .get(&GameplayCommandKind::RespondToCrisis)
        .expect("crisis response statistics must exist");
    if stats.executed < 20 {
        return;
    }
    let future_consequences = stats
        .actions_with_persistent_consequences
        .max(stats.actions_with_delayed_consequences);
    let future_share = scaled_ratio_u64(
        u64::from(future_consequences),
        u64::from(stats.executed),
        100,
    );
    if future_share >= 50 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Crisis responses rarely change the future trajectory".to_owned(),
        evidence: format!(
            "At least {future_consequences} of {} crisis responses produced an action-attributable consequence that persisted or emerged after time advanced ({future_share}%). Crises are visible and actionable, but intervention seldom changes what happens after the immediate resolution step.",
            stats.executed,
        ),
    });
}

pub(crate) fn add_information_leverage_trajectory_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 1_800 {
        return;
    }
    let stats = aggregate
        .commands
        .get(&GameplayCommandKind::LeverageInformation)
        .expect("information leverage statistics must exist");
    if stats.executed < 12 {
        return;
    }
    let future_consequences = stats
        .actions_with_persistent_consequences
        .max(stats.actions_with_delayed_consequences);
    let future_share = scaled_ratio_u64(
        u64::from(future_consequences),
        u64::from(stats.executed),
        100,
    );
    if future_share >= 50 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Commissioned intelligence rarely changes the later trajectory".to_owned(),
        evidence: format!(
            "Only {future_consequences} of {} information-leverage actions produced an action-attributable consequence that persisted or emerged after time advanced ({future_share}%). Intelligence is being converted into visible immediate state changes, but too often stops at the report/action pair instead of affecting subsequent economic, political, or civic behavior.",
            stats.executed,
        ),
    });
}

pub(crate) fn add_office_directive_trajectory_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 1_800 {
        return;
    }
    let stats = aggregate
        .commands
        .get(&GameplayCommandKind::ExerciseOfficePower)
        .expect("office-power statistics must exist");
    if stats.executed < 20 {
        return;
    }
    let delayed_share = scaled_ratio_u64(
        u64::from(stats.actions_with_delayed_consequences),
        u64::from(stats.executed),
        100,
    );
    if delayed_share >= 15 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Office directives rarely alter the later trajectory".to_owned(),
        evidence: format!(
            "Only {} of {} office directives produced a newly attributable consequence after time advanced ({delayed_share}%). Directives create immediate visible effects, but mature political power is not consistently changing later system behavior.",
            stats.actions_with_delayed_consequences,
            stats.executed,
        ),
    });
}

/// Surfaces crisis kinds that no campaign in the matrix ever detected. Every
/// crisis kind owns detection, escalation, response options, and persistent
/// consequences; a kind no session reaches is unreachable content whose
/// design intent cannot be exercised or evaluated at all.
pub(crate) fn add_crisis_coverage_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.len() < 4 || average_campaign_days(aggregate) < 720 {
        return;
    }
    let observed: BTreeSet<&'static str> = campaigns
        .iter()
        .flat_map(|campaign| {
            campaign
                .observed_crisis_kinds
                .iter()
                .map(|kind| kind.label())
        })
        .collect();
    let all_kinds = [
        CrisisKind::GrainShortage,
        CrisisKind::BankingPanic,
        CrisisKind::UrbanFire,
        CrisisKind::GuildRevolt,
        CrisisKind::NobleDemand,
        CrisisKind::Epidemic,
        CrisisKind::TradeDisruption,
    ];
    let unobserved: Vec<&'static str> = all_kinds
        .iter()
        .map(|kind| kind.label())
        .filter(|label| !observed.contains(label))
        .collect();
    if unobserved.is_empty() {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Info,
        title: format!(
            "{} crisis kind(s) were never detected in this horizon",
            unobserved.len()
        ),
        evidence: format!(
            "Observed kinds across {} campaign(s): {}. Never detected: {}. A crisis kind no session reaches cannot expose structural weakness or exercise its response routes; treat it as dead detection content rather than rare drama.",
            campaigns.len(),
            if observed.is_empty() {
                "none".to_owned()
            } else {
                observed.into_iter().collect::<Vec<_>>().join(", ")
            },
            unobserved.join(", "),
        ),
    });
}

pub(crate) fn add_crisis_determinism_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.len() < 8 || average_campaign_days(aggregate) < 720 {
        return;
    }
    // Long horizons (10 years) should eventually see every crisis; determinism
    // is only a tuning signal on the standard 3-year matrix where emergence
    // must be proven per crisis kind within a single generation.
    if average_campaign_days(aggregate) >= 1_800 {
        return;
    }
    let total = campaigns.len();
    let mut kind_counts: std::collections::BTreeMap<CrisisKind, usize> =
        std::collections::BTreeMap::new();
    for campaign in campaigns {
        for kind in &campaign.observed_crisis_kinds {
            *kind_counts.entry(*kind).or_default() += 1;
        }
    }
    for (kind, count) in &kind_counts {
        let share = scaled_ratio_usize(*count, total, 100);
        if share >= 95 {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Info,
                title: format!("{kind:?} is near-deterministic, not emergent"),
                evidence: format!(
                    "{count} of {total} campaigns observed {kind:?} ({share}%). When a crisis kind appears in essentially every world seed it is a guaranteed schedule rather than an emergent response to structural weakness. Consider raising its disruption/threshold so route and credit stress must actually accumulate.",
                ),
            });
        }
    }
    // Banking panic specific diagnostic: when it never appears, show how close default pressure came.
    // Only warn on horizons long enough for distress to plausibly accumulate.
    if !kind_counts.contains_key(&CrisisKind::BankingPanic)
        && average_campaign_days(aggregate) >= 1_800
    {
        let max_defaults = campaigns
            .iter()
            .map(|c| c.end.defaulted_loans.max(c.maximum_defaulted_loans))
            .max()
            .unwrap_or(0);
        let max_delinquent = campaigns
            .iter()
            .map(|c| c.end.delinquent_loans.max(c.maximum_delinquent_loans))
            .max()
            .unwrap_or(0);
        if max_defaults == 0 && max_delinquent == 0 {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Warning,
                title: "Credit distress never reaches delinquency, so banking panic cannot trigger"
                    .to_owned(),
                evidence: format!(
                    "Across {total} campaigns the city never recorded a delinquent or defaulted private loan (peak delinquent {max_delinquent}, peak defaulted {max_defaults}). Banking panic detection requires ≥2 concurrent defaults (rising with each prior panic within 3 years), so the crisis is structurally unreachable until lending becomes riskier or businesses face tighter cash flow. The recent speculative-loan tuning aims to create that pressure; if this persists, consider lowering the effective default threshold or loosening speculative eligibility further."
                ),
            });
        }
    }
}

pub(crate) fn add_welfare_dynamism_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 1_800 || campaigns.is_empty() {
        return;
    }
    let crisis_exposed: Vec<_> = campaigns
        .iter()
        .filter(|campaign| {
            campaign
                .observed_crisis_kinds
                .iter()
                .any(|kind| matches!(kind, CrisisKind::GrainShortage | CrisisKind::Epidemic))
        })
        .collect();
    if crisis_exposed.len() < 4 {
        return;
    }
    let mechanically_stable = crisis_exposed
        .iter()
        .filter(|campaign| campaign.minimum_district_food_satisfaction >= 9_500)
        .count();
    if scaled_ratio_usize(mechanically_stable, crisis_exposed.len(), 100) < 75 {
        return;
    }
    let minimum = crisis_exposed
        .iter()
        .map(|campaign| campaign.minimum_district_food_satisfaction)
        .min()
        .unwrap_or(10_000);
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Crises leave household welfare almost mechanically flat".to_owned(),
        evidence: format!(
            "{mechanically_stable} of {} campaigns exposed to grain shortage or epidemic kept their worst district at or above 95% food satisfaction; the lowest observed district value was {:.2}%. Food-relevant crises are visible in state, but ordinary households experience little material disruption.",
            crisis_exposed.len(),
            f64::from(minimum) / 100.0,
        ),
    });
}

pub(crate) fn add_long_horizon_risk_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 3_600 || campaigns.is_empty() {
        return;
    }
    add_credit_productive_link_finding(aggregate, findings);
    add_risk_seeking_credit_coverage_finding(campaigns, findings);
    add_debt_enforcement_ecosystem_finding(aggregate, campaigns, findings);
    add_background_route_coverage_findings(campaigns, findings);
    let credit_actions = aggregate
        .commands
        .get(&GameplayCommandKind::ExtendCredit)
        .map_or(0, |stats| stats.executed);
    let player_lending_distress = campaigns.iter().any(|campaign| {
        campaign.maximum_player_delinquent_lending > 0
            || campaign.maximum_player_defaulted_lending > 0
    });
    let stress_sample_campaigns = campaigns
        .iter()
        .filter(|campaign| {
            campaign.persona == GameplayPersona::Opportunist && campaign.simulated_days >= 3_600
        })
        .count();
    if stress_sample_campaigns < 3 && credit_actions >= 20 && !player_lending_distress {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Long-horizon player lending never encounters credit distress".to_owned(),
            evidence: format!(
                "Agents extended player credit {credit_actions} times, but no campaign recorded a delinquent or defaulted loan issued by the player dynasty. Distress on unrelated private loans does not count as coverage of the player's lending risk."
            ),
        });
    }

    let civic_actions = aggregate
        .commands
        .get(&GameplayCommandKind::StartPublicWork)
        .map_or(0, |stats| stats.executed)
        .saturating_add(
            aggregate
                .commands
                .get(&GameplayCommandKind::FundPublicWork)
                .map_or(0, |stats| stats.executed),
        )
        .saturating_add(
            aggregate
                .commands
                .get(&GameplayCommandKind::EnactLaw)
                .map_or(0, |stats| stats.executed),
        );
    let civic_debt_activity = campaigns.iter().any(|campaign| {
        campaign.maximum_delinquent_civic_debts > 0
            || campaign.maximum_defaulted_civic_debts > 0
            || campaign.end.current_civic_debts > 0
            || campaign.end.repaid_civic_debts > 0
    });
    if civic_actions >= 20 && !civic_debt_activity {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Mature civic ambition never activates municipal finance".to_owned(),
            evidence: format!(
                "Agents enacted laws or sponsored public works {civic_actions} times without issuing, repaying, or distressing civic debt. City-shaping expenditure is not testing the municipal financing layer."
            ),
        });
    }
}

pub(crate) fn add_background_route_coverage_findings(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let backgrounds: BTreeSet<StartingBackground> = campaigns
        .iter()
        .map(|campaign| campaign.background)
        .collect();
    for background in backgrounds {
        let background_campaigns = campaigns
            .iter()
            .filter(|campaign| {
                campaign.background == background && campaign.simulated_days >= 3_600
            })
            .collect::<Vec<_>>();
        if background_campaigns.len() < 4 {
            continue;
        }
        for command in [
            GameplayCommandKind::AcquireBusiness,
            GameplayCommandKind::BuyProperty,
        ] {
            let generated = background_campaigns.iter().fold(0_u32, |total, campaign| {
                total.saturating_add(
                    campaign
                        .commands
                        .get(&command)
                        .map_or(0, |stats| stats.generated),
                )
            });
            if generated > 0 {
                continue;
            }
            let generated_elsewhere = campaigns
                .iter()
                .filter(|campaign| {
                    campaign.background != background && campaign.simulated_days >= 3_600
                })
                .any(|campaign| {
                    campaign
                        .commands
                        .get(&command)
                        .is_some_and(|stats| stats.generated > 0)
                });
            if !generated_elsewhere {
                continue;
            }
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Info,
                title: format!(
                    "{:?} background never exposes {}",
                    background,
                    command.label()
                ),
                evidence: format!(
                    "Across {} mature {:?} campaign(s), {} never produced a candidate even though the same route was generated for another starting background. Aggregate command coverage would hide this background-specific strategic ceiling.",
                    background_campaigns.len(),
                    background,
                    command.label()
                ),
            });
        }
    }
}

/// Detects a city whose commercial obligations never fail: no contract
/// delivery ever misses, no counterparty is ever recorded as a breach victim,
/// and no legal case is ever filed. Under those conditions the courts, breach
/// penalties, and enforcement claims are unreachable content rather than
/// risky routes, so the report names the missing grievance flow directly.
pub(crate) fn add_counterparty_risk_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.is_empty() {
        return;
    }
    let total_contract_failures: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.contract_failures))
        .sum();
    let total_attributed_breaches: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.attributed_breach_contracts))
        .sum();
    let total_legal_cases: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.legal_cases_filed_total))
        .sum();
    if total_contract_failures > 0 || total_attributed_breaches > 0 || total_legal_cases > 0 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Counterparty performance never fails, so enforcement stays unreachable".to_owned(),
        evidence: format!(
            "Across {} campaign(s), zero city-wide contract deliveries were missed, no contract recorded an attributed breach victim, and no legal case was ever filed. Breach penalties, grounded court claims, settlements, and debt-seizure drama cannot occur without an originating grievance; treat the absence as a fulfillment-risk tuning signal rather than agent restraint.",
            campaigns.len(),
        ),
    });
}

pub(crate) fn add_debt_enforcement_ecosystem_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let distressed_campaigns = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_defaulted_loans > 0)
        .count();
    if distressed_campaigns < 2 {
        return;
    }
    let legal_transitions = aggregate
        .causal_domain_changes
        .get(&GameplayDomain::Legal)
        .copied()
        .unwrap_or(0)
        .saturating_add(
            aggregate
                .ambient_domain_changes
                .get(&GameplayDomain::Legal)
                .copied()
                .unwrap_or(0),
        );
    if legal_transitions > 0 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Defaulted private debt never reaches institutional enforcement".to_owned(),
        evidence: format!(
            "{distressed_campaigns} mature campaign(s) recorded at least one defaulted private loan, but the legal domain had no causal or autonomous transition. Debt distress exists materially without ever becoming a court dispute, so the political economy is bypassing its enforcement institution."
        ),
    });
}

pub(crate) fn add_credit_productive_link_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let Some(credit_stats) = aggregate.commands.get(&GameplayCommandKind::ExtendCredit) else {
        return;
    };
    let credit_actions = credit_stats
        .productive_financing_actions
        .saturating_add(credit_stats.nonproductive_financing_actions);
    if credit_actions < 10 {
        return;
    }
    if credit_stats.productive_financing_actions.saturating_mul(2) >= credit_actions {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Player lending is detached from productive financing".to_owned(),
        evidence: format!(
            "Agents advanced new player credit {credit_actions} times, but only {} accepted loans immediately changed business state; {} remained treasury-only at command commit. Another {} actions were zero-principal workouts and are excluded from the new-financing sample. Credit should usually finance a real commercial pressure rather than behave like an idle treasury transfer whose principal can fund its own repayment.",
            credit_stats.productive_financing_actions,
            credit_stats.nonproductive_financing_actions,
            credit_stats.financing_workout_actions,
        ),
    });
}

pub(crate) fn add_risk_seeking_credit_coverage_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let opportunist_campaigns: Vec<_> = campaigns
        .iter()
        .filter(|campaign| {
            campaign.persona == GameplayPersona::Opportunist && campaign.simulated_days >= 7_200
        })
        .collect();
    if opportunist_campaigns.len() < 3 {
        return;
    }
    let credit_actions: u32 = opportunist_campaigns
        .iter()
        .map(|campaign| {
            campaign
                .commands
                .get(&GameplayCommandKind::ExtendCredit)
                .map_or(0, |stats| stats.executed)
        })
        .sum();
    let debt_enforcement_actions: u32 = opportunist_campaigns
        .iter()
        .map(|campaign| u32::from(campaign.player_debt_enforcement_cases))
        .sum();
    let distressed_campaigns = opportunist_campaigns
        .iter()
        .filter(|campaign| {
            campaign.maximum_player_delinquent_lending > 0
                || campaign.maximum_player_defaulted_lending > 0
        })
        .count();
    let minimum_credit_sample = usize_to_u32(opportunist_campaigns.len()).saturating_mul(2);
    if credit_actions < minimum_credit_sample {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Info,
            title: "Risk-seeking player-credit sample remains thin".to_owned(),
            evidence: format!(
                "Across {} long-horizon opportunist campaigns, agents extended external credit {credit_actions} time(s). At least {minimum_credit_sample} player loans are required before the harness treats an absence of delinquency or default as evidence that the credit system may be too safe.",
                opportunist_campaigns.len(),
            ),
        });
        return;
    }
    if distressed_campaigns == 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Risk-seeking player lending never becomes distressed".to_owned(),
            evidence: format!(
                "Across {} long-horizon opportunist campaigns, agents extended external credit {credit_actions} time(s), but no campaign recorded delinquency or default on a player-issued loan. The sample is large enough that persistent perfect repayment indicates the stress strategy may still be too safe.",
                opportunist_campaigns.len(),
            ),
        });
        return;
    }
    if debt_enforcement_actions == 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Player credit distress never reaches enforcement".to_owned(),
            evidence: format!(
                "Across {} long-horizon opportunist campaigns, agents extended external credit {credit_actions} time(s), {distressed_campaigns} campaign(s) recorded delinquency or default on player-issued loans, but agents filed no player debt-enforcement case. Contract-breach litigation and unrelated private-loan distress do not count as proof that the player can act on failed credit.",
                opportunist_campaigns.len(),
            ),
        });
    }
}

pub(crate) fn add_long_substantive_gap_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let eligible: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.simulated_days >= 720)
        .collect();
    if eligible.is_empty() {
        return;
    }
    let long_gaps = eligible
        .iter()
        .filter(|campaign| campaign.longest_substantive_action_gap_days >= 360)
        .count();
    let worst = eligible
        .iter()
        .max_by_key(|campaign| campaign.longest_substantive_action_gap_days)
        .expect("eligible campaigns must have a longest gap");
    if scaled_ratio_usize(long_gaps, eligible.len(), 100) >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Long stretches pass without a substantive player decision".to_owned(),
            evidence: format!(
                "{long_gaps} of {} campaigns had a decision gap of at least one year; the worst gap was {} days for seed {}, {} {:?}.",
                eligible.len(),
                worst.longest_substantive_action_gap_days,
                worst.seed,
                worst.persona.label(),
                worst.background
            ),
        });
    } else if worst.longest_substantive_action_gap_days >= 540 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "An individual campaign has a prolonged strategic drought".to_owned(),
            evidence: format!(
                "Seed {}, {} {:?} passed {} days without a substantive action even though the aggregate drought rate remained below 25%.",
                worst.seed,
                worst.persona.label(),
                worst.background,
                worst.longest_substantive_action_gap_days
            ),
        });
    }
}

pub(crate) fn add_asset_liquidity_drought_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let trapped = campaigns
        .iter()
        .find(|campaign| campaign.longest_asset_rich_quiet_gap_days >= 360);
    let Some(campaign) = trapped else {
        return;
    };
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Owned wealth can become decision-poor".to_owned(),
        evidence: format!(
            "Seed {}, {} {:?} spent {} consecutive days without a substantive action while treasury cash was below 40 cr and material wealth remained locked in property or operating businesses. The harness should surface costly liquidity routes instead of treating owned wealth as unusable.",
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
            campaign.longest_asset_rich_quiet_gap_days
        ),
    });
}

pub(crate) fn add_economic_recovery_dead_end_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let trapped: Vec<_> = campaigns
        .iter()
        .filter(|campaign| {
            campaign.longest_substantive_action_gap_days >= 360
                && campaign.end.player_treasury <= Money::ZERO
                && campaign.end.active_businesses == 0
                && campaign
                    .end
                    .distressed_businesses
                    .saturating_add(campaign.end.insolvent_businesses)
                    > 0
                && campaign.end.player_properties == 0
                && campaign
                    .end
                    .current_loans
                    .saturating_add(campaign.end.delinquent_loans)
                    .saturating_add(campaign.end.restructured_loans)
                    == 0
        })
        .collect();
    if let Some(worst) = trapped
        .iter()
        .max_by_key(|campaign| campaign.longest_substantive_action_gap_days)
    {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Critical,
            title: "Economic failure can become an unrecoverable campaign state".to_owned(),
            evidence: format!(
                "{} campaign(s) ended with no treasury, no healthy business, no property, no active or restructured credit, and a year-scale decision drought. The worst was seed {}, {} {:?}, with {} days without a substantive route.",
                trapped.len(),
                worst.seed,
                worst.persona.label(),
                worst.background,
                worst.longest_substantive_action_gap_days
            ),
        });
    }

    if let Some(campaign) = campaigns
        .iter()
        .find(|campaign| campaign.terminal_recovery_pressure_days >= 360)
    {
        let borrowing = campaign
            .commands
            .get(&GameplayCommandKind::BorrowFunds)
            .map_or(0, |stats| stats.executed);
        let investment = campaign
            .commands
            .get(&GameplayCommandKind::InvestInBusiness)
            .map_or(0, |stats| stats.executed);
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "An individual dynasty remains trapped in recovery churn".to_owned(),
            evidence: format!(
                "Seed {}, {} {:?} remained under recovery pressure for {} consecutive days through the campaign endpoint, with no treasury, property, or active business and {} defaulted loans despite {borrowing} borrowing or restructuring actions and {investment} recapitalizations. Activity continued, but it did not produce a credible recovery path.",
                campaign.seed,
                campaign.persona.label(),
                campaign.background,
                campaign.terminal_recovery_pressure_days,
                campaign.end.defaulted_loans
            ),
        });
    }
}

pub(crate) fn add_campaign_blocking_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let Some((campaign, opportunity_cycles, blocked_share)) = campaigns
        .iter()
        .filter_map(|campaign| {
            let opportunity_cycles = campaign
                .decision_cycles
                .saturating_sub(campaign.quiet_cycles);
            (opportunity_cycles > 0).then_some((
                campaign,
                opportunity_cycles,
                scaled_ratio_u64(
                    u64::from(campaign.blocked_cycles),
                    u64::from(opportunity_cycles),
                    100,
                ),
            ))
        })
        .max_by_key(|(_, _, blocked_share)| *blocked_share)
    else {
        return;
    };
    if campaign.blocked_cycles < 4 || blocked_share < 25 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "An individual campaign becomes strategically blocked".to_owned(),
        evidence: format!(
            "{} of {opportunity_cycles} actionable cycles in seed {}, {} {:?} ended with no viable command ({blocked_share}%). Aggregate averages can hide this start-specific failure mode.",
            campaign.blocked_cycles,
            campaign.seed,
            campaign.persona.label(),
            campaign.background,
        ),
    });
}

pub(crate) fn add_score_findings(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    for (label, score) in [
        ("actionability", aggregate.scores.actionability),
        ("variety", aggregate.scores.variety),
        ("interconnection", aggregate.scores.interconnection),
        ("feedback", aggregate.scores.feedback),
        ("resilience", aggregate.scores.resilience),
    ] {
        let severity = if score < 30 {
            GameplayFindingSeverity::Critical
        } else if score < 60 {
            GameplayFindingSeverity::Warning
        } else {
            continue;
        };
        findings.push(GameplayFinding {
            severity,
            title: format!("Low {label} score"),
            evidence: format!("The aggregate {label} score was {score}/100."),
        });
    }
}

/// Commands whose candidate generators deliberately narrow the canonical
/// game's offer to distress, repositioning, or strategic-need conditions.
///
/// For these routes the canonical game accepts the command broadly, but the
/// agent's portfolio or political policy intentionally avoids routine use. When
/// such a route shows `generated == 0` despite world activation, the signal is
/// agent restraint (a coverage gap the design review should weigh), not a broken
/// or unreachable game route, so the finding is a Warning rather than Critical.
///
/// The liquidity routes are included because the agent deliberately narrows
/// them through its rebalancing cadence, cash targets, and distribution
/// reserves: an activation opportunity without a candidate there means the
/// portfolio simply had no shortfall worth acting on.
///
/// The standing-expense routes (family education, ward adoption, institution
/// endowment) are included because their generators require the shared
/// discretionary floor on top of canonical affordability, so a below-floor
/// treasury declines them by design instead of by coverage hole.
pub(crate) const fn is_policy_gated_command_route(kind: GameplayCommandKind) -> bool {
    matches!(
        kind,
        GameplayCommandKind::SellProperty
            | GameplayCommandKind::ExerciseOfficePower
            | GameplayCommandKind::SecureSupply
            | GameplayCommandKind::SellOutput
            | GameplayCommandKind::InvestInBusiness
            | GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::WithdrawFromInstitution
            | GameplayCommandKind::FundPublicWork
            | GameplayCommandKind::TransferBusinessCash
            | GameplayCommandKind::WithdrawBusinessCash
            // Threshold-narrowed routes: each generator below only builds when
            // its own strategic-need condition is stricter than the canonical
            // validation the activation predicate mirrors.
            | GameplayCommandKind::SetBusinessWages
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::ConveneFamilyCouncil
            | GameplayCommandKind::AcknowledgeNotification
            | GameplayCommandKind::DesignateHeir
            | GameplayCommandKind::EducateFamilyMember
            | GameplayCommandKind::AdoptWard
            | GameplayCommandKind::EndowInstitution
            | GameplayCommandKind::LeverageInformation
            | GameplayCommandKind::CommissionInformation
            | GameplayCommandKind::BuyProperty
            // Institution support only targets memberships the persona ranks
            // as strategically useful, even though canonical validation may
            // accept other institutions for the same character.
            | GameplayCommandKind::CultivateInstitutionSupport
            // Borrowing and charter amendment generators only build when the
            // persona's strategic-need condition fires, which is stricter than
            // the canonical acceptance their activation predicate mirrors.
            | GameplayCommandKind::BorrowFunds
            | GameplayCommandKind::SetHouseGovernance
            // Lending carries persona reserves, portfolio limits, and
            // restructuring framing: a house whose policy declines to lend
            // leaves canonically valid credit routes unbuilt by design.
            | GameplayCommandKind::ExtendCredit
            // Sponsorship is gated on office-power establishment, a 360-day
            // cooldown, per-district duplicates, and an upfront contribution;
            // an activation without a candidate there means one of the
            // agent's own affordability or cadence gates declined it.
            | GameplayCommandKind::StartPublicWork
            // Law sponsorship generators only build kinds the persona ranks
            // as contextually relevant; a broadly valid enactment the agent's
            // persona policy declines is restraint, not a coverage hole.
            | GameplayCommandKind::EnactLaw
    )
}

pub(crate) fn add_command_findings(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    // Unexecuted routes with no candidates share one of three causes: the
    // world never offered an activation, the agent's standing policy
    // deliberately narrowed a valid route, or the generator missed a route
    // the world offered. Each cause gets one aggregated finding so the
    // finding list reads as design signals instead of thirty near-duplicates.
    let mut dormant_routes: Vec<&'static str> = Vec::new();
    let mut restrained_routes: Vec<&'static str> = Vec::new();
    let mut unreachable_routes: Vec<&'static str> = Vec::new();
    for kind in ALL_COMMAND_KINDS {
        let stats = aggregate
            .commands
            .get(&kind)
            .expect("every command kind must have aggregate statistics");
        if stats.executed == 0 && stats.generated == 0 {
            if command_route_expected(aggregate, kind) {
                if is_policy_gated_command_route(kind) {
                    restrained_routes.push(kind.label());
                } else {
                    unreachable_routes.push(kind.label());
                }
            } else {
                // No activation opportunity fired anywhere in the horizon:
                // the world simply offered nothing of this kind.
                dormant_routes.push(kind.label());
            }
        } else if stats.executed == 0 && stats.generated > 0 && stats.considered == 0 {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Warning,
                title: format!("{} candidates were never probed", kind.label()),
                evidence: format!(
                    "activation_opportunities={}, offered_cycles={}, generated={}; probe capacity never reached them",
                    stats.activation_opportunities, stats.offered_cycles, stats.generated
                ),
            });
        } else if stats.executed == 0 && stats.viable == 0 && stats.generated > 0 {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Critical,
                title: format!("{} was always rejected", kind.label()),
                evidence: format!(
                    "activation_opportunities={}, offered_cycles={}, generated={}, considered={}, rejected={}; canonical validation declined every candidate",
                    stats.activation_opportunities,
                    stats.offered_cycles,
                    stats.generated,
                    stats.considered,
                    stats.rejected
                ),
            });
        } else if stats.executed == 0 {
            let (severity, title) = if !is_substantive_command_kind(kind) {
                // Operational liquidity plumbing is deliberately excluded from
                // substantive-action metrics; an unselected rebalancing route
                // is routine portfolio discipline, not a design warning.
                (
                    GameplayFindingSeverity::Info,
                    format!("{} was viable but never selected", kind.label()),
                )
            } else if stats.offered_cycles < 3 {
                (
                    GameplayFindingSeverity::Info,
                    format!(
                        "{} appeared only as a rare unselected alternative",
                        kind.label()
                    ),
                )
            } else {
                (
                    GameplayFindingSeverity::Warning,
                    format!("{} was viable but never selected", kind.label()),
                )
            };
            findings.push(GameplayFinding {
                severity,
                title,
                evidence: format!(
                    "activation_opportunities={}, offered_cycles={}, generated={}, considered={}, viable={}, rejected={}; no configured agent executed it",
                    stats.activation_opportunities,
                    stats.offered_cycles,
                    stats.generated,
                    stats.considered,
                    stats.viable,
                    stats.rejected
                ),
            });
        } else if stats.changed_domains.is_empty() {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Warning,
                title: format!("{} produced no observed system change", kind.label()),
                evidence: format!("The command executed {} times.", stats.executed),
            });
        }
    }
    if !unreachable_routes.is_empty() || !restrained_routes.is_empty() || !dormant_routes.is_empty()
    {
        push_route_summary_findings(
            &unreachable_routes,
            &restrained_routes,
            &dormant_routes,
            findings,
        );
    }
}

/// One finding per unexecuted-route cause, so the finding list reads as design
/// signals instead of thirty near-duplicates.
fn push_route_summary_findings(
    unreachable_routes: &[&str],
    restrained_routes: &[&str],
    dormant_routes: &[&str],
    findings: &mut Vec<GameplayFinding>,
) {
    if !unreachable_routes.is_empty() {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Critical,
            title: "Command routes fired activations but no generator ever built a candidate"
                .to_owned(),
            evidence: format!(
                "{} route(s): {}; the canonical game accepted some concrete action of each kind in observed states, yet no configured agent could construct one",
                unreachable_routes.len(),
                unreachable_routes.join(", ")
            ),
        });
    }
    if !restrained_routes.is_empty() {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Deliberately-narrowed routes fired activations but built no candidate"
                .to_owned(),
            evidence: format!(
                "{} route(s): {}; these generators narrow the canonical offer to strategic-need conditions by standing policy, so an unfired opportunity is agent restraint rather than a coverage hole",
                restrained_routes.len(),
                restrained_routes.join(", ")
            ),
        });
    }
    if !dormant_routes.is_empty() {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Info,
            title: "Command kinds were not exercised in this horizon".to_owned(),
            evidence: format!(
                "{} kind(s): {}; no activation opportunity fired and no configured agent executed them, so this horizon offered nothing to do with these commands",
                dormant_routes.len(),
                dormant_routes.join(", ")
            ),
        });
    }
}

pub(crate) fn add_domain_findings(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let campaign_days = average_campaign_days(aggregate);
    for domain in ALL_DOMAINS {
        let causal = aggregate
            .causal_domain_changes
            .get(&domain)
            .copied()
            .unwrap_or(0);
        let ambient = aggregate
            .ambient_domain_changes
            .get(&domain)
            .copied()
            .unwrap_or(0);
        let player_route_expected = domain_player_commands(domain)
            .iter()
            .any(|kind| command_route_expected(aggregate, *kind));
        if causal == 0 && ambient == 0 {
            findings.push(GameplayFinding {
                severity: if player_route_expected {
                    GameplayFindingSeverity::Warning
                } else {
                    GameplayFindingSeverity::Info
                },
                title: if player_route_expected {
                    format!("{} domain remained static", domain.label())
                } else {
                    format!("{} domain was inactive in this horizon", domain.label())
                },
                evidence: format!(
                    "No snapshot transition touched this domain across {campaign_days} days per campaign."
                ),
            });
        } else if causal == 0 {
            let player_route_offered = domain_player_commands(domain).iter().any(|kind| {
                aggregate
                    .commands
                    .get(kind)
                    .is_some_and(|stats| stats.offered_cycles > 0)
            });
            if domain == GameplayDomain::Legal && !player_route_offered && !player_route_expected {
                findings.push(GameplayFinding {
                    severity: GameplayFindingSeverity::Info,
                    title: "Legal domain was active without player-facing standing".to_owned(),
                    evidence: format!(
                        "It changed in {ambient} baseline observations, but neither a grounded player claim nor a grounded unresolved case against the player activated during the configured horizon."
                    ),
                });
                continue;
            }
            findings.push(GameplayFinding {
                severity: if player_route_offered || player_route_expected {
                    GameplayFindingSeverity::Warning
                } else {
                    GameplayFindingSeverity::Info
                },
                title: if player_route_offered || player_route_expected {
                    format!("{} domain is autonomous but not player-responsive", domain.label())
                } else {
                    format!(
                        "{} domain changed before a player route became available",
                        domain.label()
                    )
                },
                evidence: if player_route_offered {
                    format!(
                        "It changed in {ambient} baseline observations but no offered command produced an attributable transition."
                    )
                } else {
                    format!(
                        "It changed in {ambient} baseline observations, but no command associated with this domain was offered during the configured horizon."
                    )
                },
            });
        }
    }
}

pub(crate) fn command_route_expected(
    aggregate: &GameplayAggregate,
    kind: GameplayCommandKind,
) -> bool {
    // Every route is gated on real world or cooldown conditions, so
    // expectation is decided by observed opportunity evidence uniformly: a
    // generator that kept finding real activation opportunities makes the
    // route expected regardless of any nominal horizon estimate.
    aggregate
        .commands
        .get(&kind)
        .is_some_and(|stats| stats.activation_opportunities > 0 || stats.offered_cycles > 0)
}

pub(crate) fn average_campaign_days(aggregate: &GameplayAggregate) -> u64 {
    if aggregate.campaigns == 0 {
        0
    } else {
        aggregate.simulated_days / u64::from(aggregate.campaigns)
    }
}

pub(crate) fn domain_player_commands(domain: GameplayDomain) -> &'static [GameplayCommandKind] {
    if matches!(
        domain,
        GameplayDomain::Economy
            | GameplayDomain::Business
            | GameplayDomain::Market
            | GameplayDomain::Contracts
            | GameplayDomain::Loans
            | GameplayDomain::Property
            | GameplayDomain::Labor
            | GameplayDomain::Relationships
    ) {
        commercial_domain_player_commands(domain)
    } else {
        civic_domain_player_commands(domain)
    }
}

pub(crate) const fn commercial_domain_player_commands(
    domain: GameplayDomain,
) -> &'static [GameplayCommandKind] {
    match domain {
        GameplayDomain::Economy => &[
            GameplayCommandKind::TransferBusinessCash,
            GameplayCommandKind::WithdrawBusinessCash,
            GameplayCommandKind::AcquireBusiness,
            GameplayCommandKind::InvestInBusiness,
            GameplayCommandKind::BorrowFunds,
            GameplayCommandKind::ExtendCredit,
            GameplayCommandKind::BuyProperty,
            GameplayCommandKind::SellProperty,
            GameplayCommandKind::CultivateInstitutionSupport,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Business => &[
            GameplayCommandKind::AcquireBusiness,
            GameplayCommandKind::InvestInBusiness,
            GameplayCommandKind::SetBusinessPolicy,
            GameplayCommandKind::SetBusinessWages,
        ],
        GameplayDomain::Market => &[
            GameplayCommandKind::SetBusinessPolicy,
            GameplayCommandKind::SecureSupply,
            GameplayCommandKind::SellOutput,
            GameplayCommandKind::EnactLaw,
        ],
        GameplayDomain::Contracts => &[
            GameplayCommandKind::SecureSupply,
            GameplayCommandKind::SellOutput,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Loans => &[
            GameplayCommandKind::BorrowFunds,
            GameplayCommandKind::ExtendCredit,
        ],
        GameplayDomain::Property => &[
            GameplayCommandKind::BuyProperty,
            GameplayCommandKind::SellProperty,
        ],
        GameplayDomain::Labor => &[
            GameplayCommandKind::InvestInBusiness,
            GameplayCommandKind::SetBusinessPolicy,
            GameplayCommandKind::SetBusinessWages,
            GameplayCommandKind::ResolveLaborDispute,
        ],
        GameplayDomain::Relationships => &[
            GameplayCommandKind::SecureSupply,
            GameplayCommandKind::SellOutput,
            GameplayCommandKind::BorrowFunds,
            GameplayCommandKind::ExtendCredit,
            GameplayCommandKind::SellProperty,
            GameplayCommandKind::FileLegalCase,
            GameplayCommandKind::SettleLegalCase,
            GameplayCommandKind::CultivateInstitutionSupport,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Dynasty
        | GameplayDomain::Family
        | GameplayDomain::Institutions
        | GameplayDomain::Law
        | GameplayDomain::Districts
        | GameplayDomain::Legal
        | GameplayDomain::Crises
        | GameplayDomain::Information
        | GameplayDomain::Feedback => &[],
    }
}

pub(crate) const fn civic_domain_player_commands(
    domain: GameplayDomain,
) -> &'static [GameplayCommandKind] {
    match domain {
        GameplayDomain::Dynasty => &[
            GameplayCommandKind::BorrowFunds,
            GameplayCommandKind::ExtendCredit,
            GameplayCommandKind::SellProperty,
            GameplayCommandKind::EnactLaw,
            GameplayCommandKind::DesignateHeir,
            GameplayCommandKind::AdoptWard,
            GameplayCommandKind::EducateFamilyMember,
            GameplayCommandKind::CultivateInstitutionSupport,
            GameplayCommandKind::NominateForOffice,
            GameplayCommandKind::ExerciseOfficePower,
            GameplayCommandKind::WithdrawFromInstitution,
            GameplayCommandKind::RespondToCrisis,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Family => &[
            GameplayCommandKind::SetHouseGovernance,
            GameplayCommandKind::DesignateHeir,
            GameplayCommandKind::AdoptWard,
            GameplayCommandKind::EducateFamilyMember,
            GameplayCommandKind::WithdrawFromInstitution,
        ],
        GameplayDomain::Institutions => &[
            GameplayCommandKind::StartPublicWork,
            GameplayCommandKind::FundPublicWork,
            GameplayCommandKind::CultivateInstitutionSupport,
            GameplayCommandKind::NominateForOffice,
            GameplayCommandKind::ExerciseOfficePower,
            GameplayCommandKind::WithdrawFromInstitution,
        ],
        GameplayDomain::Law => &[GameplayCommandKind::EnactLaw],
        GameplayDomain::Districts => &[
            GameplayCommandKind::StartPublicWork,
            GameplayCommandKind::FundPublicWork,
            GameplayCommandKind::ExerciseOfficePower,
            GameplayCommandKind::RespondToCrisis,
            GameplayCommandKind::ResolveLaborDispute,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Legal => &[
            GameplayCommandKind::FileLegalCase,
            GameplayCommandKind::SettleLegalCase,
        ],
        GameplayDomain::Crises => &[GameplayCommandKind::RespondToCrisis],
        GameplayDomain::Information => &[
            GameplayCommandKind::SecureSupply,
            GameplayCommandKind::SellOutput,
            GameplayCommandKind::BorrowFunds,
            GameplayCommandKind::ExtendCredit,
            GameplayCommandKind::CommissionInformation,
            GameplayCommandKind::LeverageInformation,
        ],
        GameplayDomain::Feedback => &ALL_COMMAND_KINDS,
        GameplayDomain::Economy
        | GameplayDomain::Business
        | GameplayDomain::Market
        | GameplayDomain::Contracts
        | GameplayDomain::Loans
        | GameplayDomain::Property
        | GameplayDomain::Labor
        | GameplayDomain::Relationships => &[],
    }
}

pub(crate) fn add_action_concentration_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let Some((kind, stats)) = aggregate
        .commands
        .iter()
        .filter(|(kind, _)| is_substantive_command_kind(**kind))
        .max_by_key(|(_, stats)| stats.executed)
    else {
        return;
    };
    if aggregate.substantive_actions == 0 {
        return;
    }
    let share = scaled_ratio_u64(
        u64::from(stats.executed),
        aggregate.substantive_actions,
        100,
    );
    if share < 35 {
        return;
    }
    findings.push(GameplayFinding {
        severity: if share >= 60 {
            GameplayFindingSeverity::Critical
        } else {
            GameplayFindingSeverity::Warning
        },
        title: format!("{} dominates player decisions", kind.label()),
        evidence: format!(
            "It accounted for {share}% of {} executed actions.",
            aggregate.substantive_actions
        ),
    });
}

pub(crate) fn add_operational_rebalancing_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let transfers = aggregate
        .commands
        .get(&GameplayCommandKind::TransferBusinessCash)
        .map_or(0, |stats| stats.executed);
    let withdrawals = aggregate
        .commands
        .get(&GameplayCommandKind::WithdrawBusinessCash)
        .map_or(0, |stats| stats.executed);
    let operational = transfers.saturating_add(withdrawals);
    let housekeeping = aggregate
        .commands
        .get(&GameplayCommandKind::AcknowledgeNotification)
        .map_or(0, |stats| stats.executed);
    let player_actions = aggregate
        .successful_actions
        .saturating_sub(u64::from(housekeeping));
    if operational < 12 || player_actions == 0 {
        return;
    }
    let share = scaled_ratio_u64(u64::from(operational), player_actions, 100);
    if share < 25 {
        return;
    }
    findings.push(GameplayFinding {
        severity: if share >= 40 {
            GameplayFindingSeverity::Warning
        } else {
            GameplayFindingSeverity::Info
        },
        title: "Operational liquidity management dominates player decisions".to_owned(),
        evidence: format!(
            "Portfolio cash rebalancing accounted for {operational} of {player_actions} non-notification actions ({share}%): {transfers} inter-business transfers and {withdrawals} business-cash withdrawals. These actions keep the portfolio liquid, but are operational support rather than a strategic commitment; the harness excludes them from substantive-action scores and reports them separately so treasury plumbing cannot masquerade as dynasty direction."
        ),
    });
}

pub(crate) fn add_institutional_campaign_concentration_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    if aggregate.substantive_actions == 0 {
        return;
    }
    let support_actions = aggregate
        .commands
        .get(&GameplayCommandKind::CultivateInstitutionSupport)
        .map_or(0, |stats| stats.executed);
    let nomination_actions = aggregate
        .commands
        .get(&GameplayCommandKind::NominateForOffice)
        .map_or(0, |stats| stats.executed);
    let campaign_actions = support_actions.saturating_add(nomination_actions);
    if campaign_actions < 20 {
        return;
    }
    let share = scaled_ratio_u64(
        u64::from(campaign_actions),
        aggregate.substantive_actions,
        100,
    );
    if share < 35 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Institutional campaigning dominates the decision loop".to_owned(),
        evidence: format!(
            "Patronage and nominations accounted for {campaign_actions} of {} substantive actions ({share}%). Family political capacity should create strategic reach without becoming recurring campaign administration.",
            aggregate.substantive_actions
        ),
    });
}

pub(crate) fn add_phase_institutional_campaign_concentration_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let stats = aggregate
        .phase_stats
        .get(&GameplayPhase::InstitutionalAscent)
        .cloned()
        .unwrap_or_default();
    if stats.substantive_actions < 20 || stats.institutional_campaign_actions < 20 {
        return;
    }
    let share = scaled_ratio_u64(
        u64::from(stats.institutional_campaign_actions),
        u64::from(stats.substantive_actions),
        100,
    );
    if share < 65 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Institutional ascent becomes campaign administration".to_owned(),
        evidence: format!(
            "Patronage and nominations accounted for {} of {} substantive institutional-ascent actions ({share}%). Political ascent should still leave room for commercial, family, information, and civic decisions while support and campaigns mature.",
            stats.institutional_campaign_actions, stats.substantive_actions
        ),
    });
}

pub(crate) fn add_business_survival_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.is_empty() {
        return;
    }
    let non_operational = campaigns
        .iter()
        .filter(|campaign| {
            campaign
                .end
                .active_businesses
                .saturating_add(campaign.end.distressed_businesses)
                == 0
        })
        .count();
    if non_operational > 0 {
        let share = scaled_ratio_usize(non_operational, campaigns.len(), 100);
        findings.push(GameplayFinding {
            severity: if share >= 50 {
                GameplayFindingSeverity::Critical
            } else {
                GameplayFindingSeverity::Warning
            },
            title: "Player businesses become non-operational".to_owned(),
            evidence: format!(
                "{non_operational} of {} campaigns ended with every player business insolvent or closed ({share}%).",
                campaigns.len()
            ),
        });
    }
    let fully_stressed = campaigns
        .iter()
        .filter(|campaign| {
            campaign.end.active_businesses == 0
                && campaign
                    .end
                    .distressed_businesses
                    .saturating_add(campaign.end.insolvent_businesses)
                    > 0
        })
        .count();
    let stressed_share = scaled_ratio_usize(fully_stressed, campaigns.len(), 100);
    if stressed_share >= 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Player portfolios frequently lack a healthy active business".to_owned(),
            evidence: format!(
                "{fully_stressed} of {} campaigns ended with every player business distressed or insolvent ({stressed_share}%).",
                campaigns.len()
            ),
        });
    }
}

pub(crate) fn add_choice_quality_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let opportunity_cycles = aggregate
        .decision_cycles
        .saturating_sub(aggregate.quiet_cycles);
    if opportunity_cycles == 0 {
        return;
    }
    let average_kinds_tenths =
        scaled_ratio_u64(aggregate.viable_command_kinds, opportunity_cycles, 10);
    let average_choices_tenths = scaled_ratio_u64(aggregate.viable_choices, opportunity_cycles, 10);
    let multiple_share = scaled_ratio_u64(
        aggregate.cycles_with_multiple_viable_command_kinds,
        opportunity_cycles,
        100,
    );
    add_choice_breadth_finding(
        average_choices_tenths,
        average_kinds_tenths,
        multiple_share,
        findings,
    );
    add_choice_tradeoff_findings(aggregate, findings);
    add_option_tradeoff_findings(aggregate, findings);
    add_blocked_choice_finding(aggregate, opportunity_cycles, findings);
}

pub(crate) fn add_choice_breadth_finding(
    average_choices_tenths: u64,
    average_kinds_tenths: u64,
    multiple_share: u64,
    findings: &mut Vec<GameplayFinding>,
) {
    if average_choices_tenths < 20 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Actionable cycles offer too few meaningful alternatives".to_owned(),
            evidence: format!(
                "The average actionable cycle exposed {} viable choices across {} command families; {multiple_share}% offered at least two substantive families.",
                format_tenths(average_choices_tenths),
                format_tenths(average_kinds_tenths)
            ),
        });
    } else if average_kinds_tenths < 15 || multiple_share < 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Actionable cycles are usually single-track".to_owned(),
            evidence: format!(
                "The average actionable cycle exposed {} viable choices but only {} command families; just {multiple_share}% offered at least two substantive families. Mature play risks becoming a sequence of predetermined task categories rather than competing plans.",
                format_tenths(average_choices_tenths),
                format_tenths(average_kinds_tenths)
            ),
        });
    } else if average_kinds_tenths < 20 || multiple_share < 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Info,
            title: "Strategic alternatives concentrate within command families".to_owned(),
            evidence: format!(
                "The average actionable cycle exposed {} viable choices but only {} command families; {multiple_share}% offered at least two families. Policy templates, targets, projects, and counterparties provide choice depth even when the strategic category is focused.",
                format_tenths(average_choices_tenths),
                format_tenths(average_kinds_tenths)
            ),
        });
    }
}

pub(crate) fn add_choice_tradeoff_findings(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    if aggregate.cycles_with_multiple_viable_command_kinds < 20 {
        return;
    }
    let denominator = aggregate.cycles_with_multiple_viable_command_kinds;
    let close_share = scaled_ratio_u64(
        aggregate.cycles_with_close_viable_command_kinds,
        denominator,
        100,
    );
    let immediate_share = scaled_ratio_u64(
        aggregate.cycles_with_distinct_immediate_consequences,
        denominator,
        100,
    );
    let projected_share = scaled_ratio_u64(
        aggregate.cycles_with_distinct_projected_consequences,
        denominator,
        100,
    );
    if close_share < 20 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Most multi-option cycles still have an obvious winner".to_owned(),
            evidence: format!(
                "Only {} of {} cycles with multiple viable command families placed the two highest-ranked viable families within {CLOSE_CHOICE_SCORE_GAP} score points ({close_share}%). The harness sees breadth, but the agent rarely faces a close strategic tradeoff.",
                aggregate.cycles_with_close_viable_command_kinds,
                denominator
            ),
        });
    }
    if immediate_share < 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Viable alternatives often share the same immediate consequence profile"
                .to_owned(),
            evidence: format!(
                "Only {} of {} multi-family cycles exposed at least two distinct immediate domain-change profiles ({immediate_share}%). Delayed effects may still diverge, but the first-order feedback risks making different commands feel interchangeable.",
                aggregate.cycles_with_distinct_immediate_consequences,
                denominator
            ),
        });
    }
    if projected_share < 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Strategic alternatives converge at the shared projected horizon".to_owned(),
            evidence: format!(
                "Only {} of {} multi-family cycles produced at least two distinct projected domain-change profiles at the shared projected horizon ({projected_share}%). Immediate feedback may differ while the simulated trajectories still converge.",
                aggregate.cycles_with_distinct_projected_consequences,
                denominator
            ),
        });
    }
}

pub(crate) fn add_option_tradeoff_findings(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let denominator = aggregate.cycles_with_multiple_viable_options;
    if denominator < 20 {
        return;
    }
    let close_share =
        scaled_ratio_u64(aggregate.cycles_with_close_viable_options, denominator, 100);
    let immediate_share = scaled_ratio_u64(
        aggregate.cycles_with_distinct_immediate_option_consequences,
        denominator,
        100,
    );
    let projected_share = scaled_ratio_u64(
        aggregate.cycles_with_distinct_projected_option_consequences,
        denominator,
        100,
    );
    if projected_share < 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Concrete alternatives converge despite different targets".to_owned(),
            evidence: format!(
                "Only {} of {denominator} cycles with at least two viable concrete options produced distinct projected consequence profiles at the shared projected horizon ({projected_share}%). The harness compares targets and templates inside the same command family as well as different families, including whether observed strategic measures rise or fall. A low share means apparent target choice often changes labels more than trajectory.",
                aggregate.cycles_with_distinct_projected_option_consequences,
            ),
        });
    } else if immediate_share < 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Info,
            title: "Concrete alternatives differentiate mainly through delayed effects".to_owned(),
            evidence: format!(
                "{immediate_share}% of multi-option cycles had distinct immediate consequence profiles, while {projected_share}% diverged at the shared projected horizon. Target-level choices are systemic, but much of their identity emerges through simulation rather than at commit time."
            ),
        });
    }
    if close_share < 15 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Info,
            title: "Concrete choices are usually strongly ranked by the diagnostic persona".to_owned(),
            evidence: format!(
                "Only {} of {denominator} cycles with multiple concrete viable options placed the top two within {CLOSE_CHOICE_SCORE_GAP} score points ({close_share}%). This is not inherently a balance defect because persona scores encode deliberate priorities, but it identifies where human playtesting should verify that lower-ranked targets remain credible tradeoffs rather than dominated options.",
                aggregate.cycles_with_close_viable_options,
            ),
        });
    }
}

pub(crate) fn add_blocked_choice_finding(
    aggregate: &GameplayAggregate,
    opportunity_cycles: u64,
    findings: &mut Vec<GameplayFinding>,
) {
    let blocked_share = scaled_ratio_u64(aggregate.blocked_cycles, opportunity_cycles, 100);
    if blocked_share >= 10 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Substantive choices are frequently blocked".to_owned(),
            evidence: format!(
                "{} of {opportunity_cycles} cycles with substantive candidates ended without a viable action ({blocked_share}%).",
                aggregate.blocked_cycles
            ),
        });
    }
}

pub(crate) fn add_institutional_reach_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let eligible: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.simulated_days >= 1_800 && campaign.end.available_offices >= 5)
        .collect();
    if eligible.is_empty() {
        return;
    }
    let near_universal = eligible
        .iter()
        .filter(|campaign| {
            u32::from(campaign.end.player_institutions_represented).saturating_mul(100)
                >= u32::from(campaign.end.available_offices).saturating_mul(80)
        })
        .count();
    let share = scaled_ratio_usize(near_universal, eligible.len(), 100);
    if share >= 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Dynasty networks become institutionally universal".to_owned(),
            evidence: format!(
                "{near_universal} of {} mature campaigns ended with player representation in at least 80% of institutions ({share}%). Family growth should create parallel strategies, but near-universal access weakens specialization, coalition choice, and the cost of succession.",
                eligible.len()
            ),
        });
    }
}

pub(crate) fn format_tenths(value: u64) -> String {
    format!("{}.{:01}", value / 10, value % 10)
}

pub(crate) fn add_system_health_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.is_empty() {
        return;
    }
    add_food_health_findings(campaigns, findings);
    add_economic_health_findings(campaigns, findings);
    add_business_condition_finding(campaigns, findings);
    add_civic_health_findings(campaigns, findings);
    add_public_work_health_finding(aggregate, campaigns, findings);
    add_public_work_portfolio_variety_finding(campaigns, findings);
    add_political_health_finding(aggregate, campaigns, findings);
    add_feed_health_findings(aggregate, campaigns, findings);
}

pub(crate) fn add_food_health_findings(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let collapsed_food = campaigns
        .iter()
        .filter(|campaign| {
            campaign.end.average_food_satisfaction < 1_000
                || campaign.minimum_food_satisfaction < 1_000
        })
        .count();
    if collapsed_food > 0 {
        let share = scaled_ratio_usize(collapsed_food, campaigns.len(), 100);
        findings.push(GameplayFinding {
            severity: if share >= 25 {
                GameplayFindingSeverity::Critical
            } else {
                GameplayFindingSeverity::Warning
            },
            title: "At least one campaign experiences complete food collapse".to_owned(),
            evidence: format!(
                "{collapsed_food} of {} campaigns fell below 10% food satisfaction at an endpoint or during the simulated trajectory ({share}%).",
                campaigns.len()
            ),
        });
    }
    let low_food = campaigns
        .iter()
        .filter(|campaign| campaign.end.average_food_satisfaction < 3_000)
        .count();
    if scaled_ratio_usize(low_food, campaigns.len(), 100) >= 25 && low_food > collapsed_food {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Critical,
            title: "Household food access collapses in many campaigns".to_owned(),
            evidence: format!(
                "{low_food} of {} campaigns ended below 30% food satisfaction.",
                campaigns.len()
            ),
        });
    }
}

pub(crate) fn add_economic_health_findings(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let player_fulfilled: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.player_fulfilled_contracts))
        .sum();
    let player_breached: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.player_breached_contracts))
        .sum();
    if player_breached > player_fulfilled && player_breached > 0 {
        let player_failures: u64 = campaigns
            .iter()
            .map(|campaign| u64::from(campaign.end.player_contract_failures))
            .sum();
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Player contracts breach more often than they complete".to_owned(),
            evidence: format!(
                "Player businesses ended with {player_breached} breached and {player_fulfilled} fulfilled contracts after {player_failures} missed deliveries."
            ),
        });
    }
    let fulfilled: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.fulfilled_contracts))
        .sum();
    let breached: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.breached_contracts))
        .sum();
    if breached > fulfilled.saturating_mul(2) && breached > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Contracts fail more often than they complete".to_owned(),
            evidence: format!("Observed {breached} breached and {fulfilled} fulfilled contracts."),
        });
    }
    let defaults: u64 = campaigns
        .iter()
        .map(|campaign| {
            u64::from(campaign.end.defaulted_loans)
                .saturating_add(u64::from(campaign.end.defaulted_civic_debts))
        })
        .sum();
    let repaid: u64 = campaigns
        .iter()
        .map(|campaign| {
            u64::from(campaign.end.repaid_loans)
                .saturating_add(u64::from(campaign.end.repaid_civic_debts))
        })
        .sum();
    if defaults > repaid && defaults > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Credit defaults outnumber successful repayments".to_owned(),
            evidence: format!(
                "Observed {defaults} defaulted and {repaid} repaid private or municipal obligations."
            ),
        });
    }
    let disputed: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.player_disputed_employment))
        .sum();
    let active: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.player_active_employment))
        .sum();
    if disputed > active && disputed > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Labor disputes dominate the player workforce".to_owned(),
            evidence: format!(
                "Player endpoints contained {disputed} disputed and {active} active agreements."
            ),
        });
    }
}

pub(crate) fn add_business_condition_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let collapsed = campaigns
        .iter()
        .filter(|campaign| campaign.end.average_business_condition < 1_000)
        .count();
    if collapsed == 0 {
        return;
    }
    let share = scaled_ratio_usize(collapsed, campaigns.len(), 100);
    findings.push(GameplayFinding {
        severity: if share >= 50 {
            GameplayFindingSeverity::Critical
        } else {
            GameplayFindingSeverity::Warning
        },
        title: "Business condition collapses over the campaign".to_owned(),
        evidence: format!(
            "{collapsed} of {} campaigns ended below 10% average business condition ({share}%).",
            campaigns.len()
        ),
    });
}

pub(crate) fn add_civic_health_findings(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let mature: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.simulated_days >= 1_080)
        .collect();
    if mature.is_empty() {
        return;
    }

    let employment_collapse = mature
        .iter()
        .filter(|campaign| {
            campaign.start.average_district_employment >= 6_000
                && campaign
                    .end
                    .average_district_employment
                    .saturating_add(1_500)
                    < campaign.start.average_district_employment
        })
        .count();
    if scaled_ratio_usize(employment_collapse, mature.len(), 100) >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "District employment collapses from the campaign baseline".to_owned(),
            evidence: format!(
                "{employment_collapse} of {} mature campaigns lost more than 1,500 bp of average district employment from start to finish. District employment is part of civic stability and must remain bounded rather than collapsing under background economy accounting.",
                mature.len()
            ),
        });
    }

    let structurally_weak = mature
        .iter()
        .filter(|campaign| campaign.end.average_district_employment < 4_500)
        .count();
    if scaled_ratio_usize(structurally_weak, mature.len(), 100) >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "District employment remains structurally weak".to_owned(),
            evidence: format!(
                "{structurally_weak} of {} mature campaigns ended below 4,500 bp average district employment. Low employment feeds unrest and should remain a material civic problem rather than an unscored background statistic.",
                mature.len()
            ),
        });
    }

    let broad_civic_distress = mature
        .iter()
        .filter(|campaign| {
            campaign.end.average_district_sanitation < 4_500
                || campaign.end.average_district_safety < 4_500
                || campaign.end.average_district_unrest > 5_000
        })
        .count();
    if scaled_ratio_usize(broad_civic_distress, mature.len(), 100) >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Material district conditions remain broadly distressed".to_owned(),
            evidence: format!(
                "{broad_civic_distress} of {} mature campaigns ended with citywide sanitation or safety below 4,500 bp, or unrest above 5,000 bp. Civic power should be judged against the material city it leaves behind.",
                mature.len()
            ),
        });
    }
}

pub(crate) fn add_public_work_health_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let starts = aggregate
        .commands
        .get(&GameplayCommandKind::StartPublicWork)
        .map_or(0_u64, |stats| u64::from(stats.executed));
    if starts == 0 {
        return;
    }
    let overloaded = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_unfinished_public_works > 4)
        .count();
    if scaled_ratio_usize(overloaded, campaigns.len(), 100) < 25 {
        return;
    }
    let completed: u64 = campaigns
        .iter()
        .map(|campaign| {
            u64::from(
                campaign
                    .end
                    .completed_public_works
                    .saturating_sub(campaign.start.completed_public_works),
            )
        })
        .sum();
    let suspended: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.end.suspended_public_works))
        .sum();
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Public works accumulate faster than the city can execute them".to_owned(),
        evidence: format!(
            "{overloaded} of {} campaigns exceeded four unfinished projects; agents started {starts}, completed {completed}, and ended with {suspended} suspended projects.",
            campaigns.len()
        ),
    });
}

pub(crate) fn add_public_work_portfolio_variety_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let active_builders: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.simulated_days >= 1_800)
        .filter(|campaign| {
            campaign
                .commands
                .get(&GameplayCommandKind::StartPublicWork)
                .is_some_and(|stats| stats.executed >= 3)
        })
        .collect();
    if active_builders.len() < 4 {
        return;
    }
    let single_kind_builders = active_builders
        .iter()
        .filter(|campaign| campaign.end.player_completed_public_work_kinds.len() <= 1)
        .count();
    let share = scaled_ratio_usize(single_kind_builders, active_builders.len(), 100);
    if share < 50 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Civic construction portfolios converge on one project type".to_owned(),
        evidence: format!(
            "{single_kind_builders} of {} mature campaigns that sponsored at least three public works completed no more than one player-sponsored project kind ({share}%). Repeated civic investment should react to changing district needs instead of becoming a persona-specific construction routine.",
            active_builders.len()
        ),
    });
}

pub(crate) fn add_political_health_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let nominations = aggregate
        .commands
        .get(&GameplayCommandKind::NominateForOffice)
        .map_or(0_u64, |stats| u64::from(stats.executed));
    let offices_ever_held: u64 = campaigns
        .iter()
        .map(|campaign| u64::from(campaign.maximum_offices_held))
        .sum();
    if nominations > 0 && offices_ever_held == 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Critical,
            title: "Office nominations never produce political power".to_owned(),
            evidence: format!(
                "The harness executed {nominations} nominations without any campaign ever producing a player officeholder."
            ),
        });
    }
    let complete_capture = campaigns
        .iter()
        .filter(|campaign| {
            campaign.end.available_offices > 1
                && campaign.maximum_offices_held >= campaign.end.available_offices
        })
        .count();
    if scaled_ratio_usize(complete_capture, campaigns.len(), 100) >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Player captures every political office".to_owned(),
            evidence: format!(
                "{complete_capture} of {} campaigns held every available office at some point.",
                campaigns.len()
            ),
        });
    }
    let officeholder_capacity_capture = campaigns
        .iter()
        .filter(|campaign| {
            let effective_capacity = campaign
                .end
                .available_offices
                .min(campaign.end.eligible_officeholders);
            let adopted_ward = campaign
                .commands
                .get(&GameplayCommandKind::AdoptWard)
                .is_some_and(|stats| stats.executed > 0);
            effective_capacity > 1
                && !adopted_ward
                && campaign.maximum_offices_held >= effective_capacity
        })
        .count();
    if scaled_ratio_usize(officeholder_capacity_capture, campaigns.len(), 100) >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Dynasty fills every available officeholder slot".to_owned(),
            evidence: format!(
                "{officeholder_capacity_capture} of {} campaigns filled every office slot their active family members could legally occupy without ever adopting a ward, so political growth stalled at the founding household.",
                campaigns.len()
            ),
        });
    }
}

pub(crate) fn add_feed_health_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let overloaded = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_unread_notifications > 100)
        .count();
    if scaled_ratio_usize(overloaded, campaigns.len(), 100) >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Notification volume exceeds a usable decision feed".to_owned(),
            evidence: format!(
                "{overloaded} of {} campaigns accumulated more than 100 unread notifications.",
                campaigns.len()
            ),
        });
    } else if overloaded > 0 {
        let worst = campaigns
            .iter()
            .max_by_key(|campaign| campaign.maximum_unread_notifications)
            .expect("non-empty campaigns must have a maximum notification backlog");
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Individual campaigns experience notification overload".to_owned(),
            evidence: format!(
                "{overloaded} of {} campaigns exceeded 100 unread notifications; the worst reached {} ({:?}, {:?}, seed {}).",
                campaigns.len(),
                worst.maximum_unread_notifications,
                worst.persona,
                worst.background,
                worst.seed
            ),
        });
    }
    let crisis_actions = aggregate
        .commands
        .get(&GameplayCommandKind::RespondToCrisis)
        .map_or(0_u64, |stats| u64::from(stats.executed));
    let crisis_share = scaled_ratio_u64(crisis_actions, aggregate.substantive_actions, 100);
    if aggregate.substantive_actions > 0 && crisis_share >= 35 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Crisis response crowds out strategic play".to_owned(),
            evidence: format!(
                "Crisis responses accounted for {crisis_share}% of executed actions."
            ),
        });
    }
}

pub(crate) fn add_core_fantasy_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    add_fantasy_arc_findings(aggregate, campaigns, findings);
    add_information_agency_finding(aggregate, findings);
    add_power_conversion_finding(aggregate, findings);
    add_player_labor_agency_finding(aggregate, campaigns, findings);
    add_persona_convergence_finding(campaigns, findings);
    add_civic_convergence_finding(aggregate, campaigns, findings);
    add_material_civic_outcome_convergence_finding(aggregate, campaigns, findings);
    add_house_governance_convergence_finding(aggregate, campaigns, findings);
    add_power_exposure_finding(aggregate, campaigns, findings);
    add_office_duty_failure_finding(aggregate, campaigns, findings);
    add_dynastic_continuity_finding(aggregate, campaigns, findings);
}

pub(crate) fn add_fantasy_arc_findings(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.is_empty() {
        return;
    }
    add_fantasy_arc_order_finding(campaigns, findings);
    add_fantasy_arc_compression_findings(campaigns, findings);
    add_absolute_fantasy_pacing_finding(campaigns, findings);
    add_synchronized_fantasy_timing_finding(campaigns, findings);
    add_fantasy_arc_completion_findings(average_campaign_days(aggregate), campaigns, findings);
}

pub(crate) fn add_fantasy_arc_order_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let political_before_commercial = campaigns
        .iter()
        .filter(|campaign| {
            matches!(
                (
                    campaign.fantasy_arc.first_commercial_standing_day,
                    campaign.fantasy_arc.first_office_campaign_day,
                ),
                (Some(commercial), Some(political)) if political < commercial
            )
        })
        .count();
    if political_before_commercial > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Political ascent precedes commercial standing".to_owned(),
            evidence: format!(
                "{political_before_commercial} of {} campaigns launched an office campaign before establishing both the required reputation and delivery record.",
                campaigns.len()
            ),
        });
    }
}

pub(crate) fn add_fantasy_arc_compression_findings(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let immediate_political_access = campaigns
        .iter()
        .filter(|campaign| {
            matches!(
                (
                    campaign.fantasy_arc.first_institution_support_day,
                    campaign.fantasy_arc.first_office_campaign_day,
                ),
                (Some(support), Some(campaign_day)) if campaign_day <= support.saturating_add(60)
            )
        })
        .count();
    if scaled_ratio_usize(immediate_political_access, campaigns.len(), 100) >= 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Institutional support immediately becomes candidacy".to_owned(),
            evidence: format!(
                "{immediate_political_access} of {} campaigns launched an office campaign within 60 days of first cultivating institutional support, leaving little distinct coalition-building phase.",
                campaigns.len()
            ),
        });
    }

    let immediate_city_power = campaigns
        .iter()
        .filter(|campaign| {
            matches!(
                (
                    campaign.fantasy_arc.first_office_day,
                    campaign.fantasy_arc.first_city_shaping_action_day,
                ),
                (Some(office), Some(city_action)) if city_action <= office.saturating_add(90)
            )
        })
        .count();
    if scaled_ratio_usize(immediate_city_power, campaigns.len(), 100) >= 50 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Officeholding immediately becomes city-shaping power".to_owned(),
            evidence: format!(
                "{immediate_city_power} of {} campaigns sponsored a law, started a public work, or issued an office directive within 90 days of first taking office, leaving little time for office-specific duties or coalition building.",
                campaigns.len()
            ),
        });
    }
}

pub(crate) fn add_absolute_fantasy_pacing_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let eligible: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.simulated_days >= 720)
        .collect();
    if eligible.is_empty() {
        return;
    }
    let compressed = eligible
        .iter()
        .filter(|campaign| {
            matches!(
                (
                    campaign.fantasy_arc.first_commercial_standing_day,
                    campaign.fantasy_arc.first_institution_support_day,
                    campaign.fantasy_arc.first_office_campaign_day,
                    campaign.fantasy_arc.first_city_shaping_action_day,
                ),
                (Some(standing), Some(support), Some(campaign_day), Some(city_day))
                    if standing <= 420 && support <= 480 && campaign_day <= 600 && city_day <= 900
            )
        })
        .count();
    if scaled_ratio_usize(compressed, eligible.len(), 100) < 50 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "The core fantasy arc is compressed into the opening establishment cycle".to_owned(),
        evidence: format!(
            "{compressed} of {} campaigns established a commercial record within 420 days, cultivated institutional support within 480 days, began an office campaign within 600 days, and exercised city-shaping power within 900 days. Foundation, social ascent, and institutional authority may not be receiving distinct enough phases for a multi-generation campaign.",
            eligible.len()
        ),
    });
}

pub(crate) fn add_synchronized_fantasy_timing_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    const SYNCHRONIZED_MILESTONE_WINDOW_DAYS: i64 = 60;
    const SYNCHRONIZED_MILESTONE_COUNT: usize = 4;
    let mut campaigns_by_start: BTreeMap<(u64, &'static str), Vec<&GameplayCampaignReport>> =
        BTreeMap::new();
    for campaign in campaigns {
        campaigns_by_start
            .entry((campaign.seed, campaign.background.recipe_key()))
            .or_default()
            .push(campaign);
    }
    let eligible_start_cohorts = campaigns_by_start
        .values()
        .filter(|cohort| {
            cohort
                .iter()
                .map(|campaign| campaign.persona)
                .collect::<BTreeSet<_>>()
                .len()
                >= GameplayPersona::all().len()
        })
        .count();
    let synchronized_cohorts: Vec<_> = campaigns_by_start
        .values()
        .filter(|cohort| {
            cohort
                .iter()
                .map(|campaign| campaign.persona)
                .collect::<BTreeSet<_>>()
                .len()
                >= GameplayPersona::all().len()
        })
        .filter(|cohort| {
            let milestones: [fn(&GameplayCampaignReport) -> Option<i64>; 5] = [
                |campaign: &GameplayCampaignReport| {
                    campaign.fantasy_arc.first_commercial_standing_day
                },
                |campaign: &GameplayCampaignReport| {
                    campaign.fantasy_arc.first_institution_support_day
                },
                |campaign: &GameplayCampaignReport| campaign.fantasy_arc.first_office_campaign_day,
                |campaign: &GameplayCampaignReport| campaign.fantasy_arc.first_office_day,
                |campaign: &GameplayCampaignReport| {
                    campaign.fantasy_arc.first_city_shaping_action_day
                },
            ];
            milestones
                .into_iter()
                .filter(|milestone| {
                    milestone_is_synchronized(
                        cohort,
                        *milestone,
                        SYNCHRONIZED_MILESTONE_WINDOW_DAYS,
                    )
                })
                .count()
                >= SYNCHRONIZED_MILESTONE_COUNT
        })
        .collect();
    let synchronized_start_cohorts = synchronized_cohorts.len();
    let synchronized_route_cohorts = synchronized_cohorts
        .iter()
        .filter(|cohort| {
            cohort
                .iter()
                .map(|campaign| {
                    (
                        campaign.fantasy_arc.first_institution_support_target,
                        campaign.fantasy_arc.first_office_campaign_target,
                        campaign.fantasy_arc.first_city_shaping_command,
                    )
                })
                .collect::<BTreeSet<_>>()
                .len()
                == 1
        })
        .count();
    if eligible_start_cohorts > 0
        && scaled_ratio_usize(synchronized_route_cohorts, eligible_start_cohorts, 100) >= 50
    {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Core fantasy timing is highly synchronized".to_owned(),
            evidence: format!(
                "{synchronized_route_cohorts} of {eligible_start_cohorts} same-seed, same-background persona cohorts reached at least {SYNCHRONIZED_MILESTONE_COUNT} of the five early fantasy milestones within {SYNCHRONIZED_MILESTONE_WINDOW_DAYS} days of each other while also choosing the same first institutional support target, office campaign target, and city-shaping command. Persona strategy should materially change the route and timing into commercial and institutional power."
            ),
        });
    } else if synchronized_start_cohorts > 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Info,
            title: "Fantasy timing converges across distinct political routes".to_owned(),
            evidence: format!(
                "{synchronized_start_cohorts} of {eligible_start_cohorts} same-seed, same-background persona cohorts reached at least {SYNCHRONIZED_MILESTONE_COUNT} early milestones within {SYNCHRONIZED_MILESTONE_WINDOW_DAYS} days, but their first institutional or city-shaping routes diverged. Shared eligibility gates compress timing, while persona strategy still changes how authority is pursued."
            ),
        });
    }
}

pub(crate) fn milestone_is_synchronized(
    cohort: &[&GameplayCampaignReport],
    milestone: fn(&GameplayCampaignReport) -> Option<i64>,
    maximum_span_days: i64,
) -> bool {
    let days: Vec<_> = cohort
        .iter()
        .filter_map(|campaign| milestone(campaign))
        .collect();
    days.len() == cohort.len()
        && milestone_span(days.into_iter()).is_some_and(|span| span <= maximum_span_days)
}

pub(crate) fn add_fantasy_arc_completion_findings(
    average_days: u64,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_days >= 1_080 {
        let incomplete = campaigns
            .iter()
            .filter(|campaign| {
                campaign.fantasy_arc.first_commercial_standing_day.is_none()
                    || campaign.fantasy_arc.first_institution_support_day.is_none()
                    || campaign.fantasy_arc.first_office_campaign_day.is_none()
                    || campaign.fantasy_arc.first_office_day.is_none()
            })
            .count();
        if scaled_ratio_usize(incomplete, campaigns.len(), 100) >= 25 {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Warning,
                title: "The early commercial-to-political arc is incomplete".to_owned(),
                evidence: format!(
                    "{incomplete} of {} campaigns did not reach commercial standing, cultivate institutional support, launch an office campaign, and obtain office within the measured horizon.",
                    campaigns.len()
                ),
            });
        }
    }
    if average_days >= 1_080 {
        let established_in_time: Vec<_> = campaigns
            .iter()
            .filter(|campaign| {
                campaign.fantasy_arc.first_office_day.is_some_and(|day| {
                    day <= i64::try_from(average_days)
                        .unwrap_or(i64::MAX)
                        .saturating_sub(OFFICE_POWER_ESTABLISHMENT_DAYS)
                })
            })
            .collect();
        let without_city_shaping = established_in_time
            .iter()
            .filter(|campaign| campaign.fantasy_arc.first_city_shaping_action_day.is_none())
            .count();
        if !established_in_time.is_empty()
            && scaled_ratio_usize(without_city_shaping, established_in_time.len(), 100) >= 25
        {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Warning,
                title: "Institutional power does not become city-shaping action".to_owned(),
                evidence: format!(
                    "{without_city_shaping} of {} campaigns whose office powers had time to establish never sponsored a law, started a public work, or issued an office directive.",
                    established_in_time.len()
                ),
            });
        }
    }
    if average_days >= 7_200 {
        let without_succession = campaigns
            .iter()
            .filter(|campaign| campaign.fantasy_arc.first_succession_day.is_none())
            .count();
        let missing_share = scaled_ratio_usize(without_succession, campaigns.len(), 100);
        if missing_share >= 25 {
            findings.push(GameplayFinding {
                severity: GameplayFindingSeverity::Warning,
                title: "The dynastic arc does not reach succession".to_owned(),
                evidence: format!(
                    "{without_succession} of {} generation-length campaigns did not transfer leadership to a successor ({missing_share}%).",
                    campaigns.len(),
                ),
            });
        }
    }
}

pub(crate) fn milestone_span(days: impl Iterator<Item = i64>) -> Option<i64> {
    let mut minimum = None;
    let mut maximum = None;
    for day in days {
        minimum = Some(minimum.map_or(day, |current: i64| current.min(day)));
        maximum = Some(maximum.map_or(day, |current: i64| current.max(day)));
    }
    minimum
        .zip(maximum)
        .map(|(minimum, maximum)| maximum - minimum)
}

pub(crate) fn add_player_labor_agency_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.is_empty() || average_campaign_days(aggregate) < 720 {
        return;
    }
    let player_dispute_campaigns = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_player_disputed_employment > 0)
        .count();
    let ambient_labor_changes = aggregate
        .ambient_domain_changes
        .get(&GameplayDomain::Labor)
        .copied()
        .unwrap_or(0);
    if player_dispute_campaigns == 0 && ambient_labor_changes > 0 {
        findings.push(GameplayFinding {
            severity: if campaigns.len() >= 3 {
                GameplayFindingSeverity::Warning
            } else {
                GameplayFindingSeverity::Info
            },
            title: "Labor conflict remains ambient to the player".to_owned(),
            evidence: format!(
                "Labor changed in {ambient_labor_changes} baseline observations, but none of {} campaigns produced a dispute in a player-owned business. A single campaign is insufficient to distinguish successful dispute avoidance from a systemic exposure gap.",
                campaigns.len()
            ),
        });
    }
}

pub(crate) fn add_information_agency_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    let information_changes = aggregate
        .ambient_domain_changes
        .get(&GameplayDomain::Information)
        .copied()
        .unwrap_or(0);
    let player_information_changes = aggregate
        .causal_domain_changes
        .get(&GameplayDomain::Information)
        .copied()
        .unwrap_or(0);
    let commissions = aggregate
        .commands
        .get(&GameplayCommandKind::CommissionInformation)
        .map_or(0, |stats| stats.executed);
    let commission_opportunities = aggregate
        .commands
        .get(&GameplayCommandKind::CommissionInformation)
        .map_or(0, |stats| stats.activation_opportunities);
    let leverage_actions = aggregate
        .commands
        .get(&GameplayCommandKind::LeverageInformation)
        .map_or(0, |stats| stats.executed);
    let leverage_opportunities = aggregate
        .commands
        .get(&GameplayCommandKind::LeverageInformation)
        .map_or(0, |stats| stats.activation_opportunities);
    if commission_opportunities > 0 && commissions == 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Commercial intelligence is not player-directed".to_owned(),
            evidence: format!(
                "The harness observed {commission_opportunities} material intelligence opportunities and {information_changes} baseline information changes, but agents commissioned {commissions} reports and produced {player_information_changes} causally attributed information changes."
            ),
        });
    }
    if leverage_opportunities > 0 && leverage_actions == 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Commissioned intelligence does not become action".to_owned(),
            evidence: format!(
                "Agents commissioned {commissions} reports and observed {leverage_opportunities} actionable leverage opportunities, but never converted one into a contract renegotiation, targeted outreach, or district initiative."
            ),
        });
    }
}

pub(crate) fn add_power_conversion_finding(
    aggregate: &GameplayAggregate,
    findings: &mut Vec<GameplayFinding>,
) {
    const ECONOMIC_COMMANDS: [GameplayCommandKind; 8] = [
        GameplayCommandKind::AcquireBusiness,
        GameplayCommandKind::InvestInBusiness,
        GameplayCommandKind::SetBusinessPolicy,
        GameplayCommandKind::SecureSupply,
        GameplayCommandKind::SellOutput,
        GameplayCommandKind::BorrowFunds,
        GameplayCommandKind::ExtendCredit,
        GameplayCommandKind::BuyProperty,
    ];
    const INSTITUTIONAL_COMMANDS: [GameplayCommandKind; 8] = [
        GameplayCommandKind::EnactLaw,
        GameplayCommandKind::StartPublicWork,
        GameplayCommandKind::FundPublicWork,
        GameplayCommandKind::FileLegalCase,
        GameplayCommandKind::SettleLegalCase,
        GameplayCommandKind::SetHouseGovernance,
        GameplayCommandKind::NominateForOffice,
        GameplayCommandKind::ExerciseOfficePower,
    ];
    let economic_to_social = aggregate.interactions.iter().any(|edge| {
        ECONOMIC_COMMANDS.contains(&edge.command)
            && matches!(
                edge.domain,
                GameplayDomain::Relationships | GameplayDomain::Institutions | GameplayDomain::Law
            )
    });
    let institutional_to_material = aggregate.interactions.iter().any(|edge| {
        INSTITUTIONAL_COMMANDS.contains(&edge.command)
            && matches!(
                edge.domain,
                GameplayDomain::Economy
                    | GameplayDomain::Business
                    | GameplayDomain::Market
                    | GameplayDomain::Districts
            )
    });
    if !economic_to_social || !institutional_to_material {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "The power-conversion loop is incomplete".to_owned(),
            evidence: format!(
                "economic_to_social={economic_to_social}; institutional_to_material={institutional_to_material}. The core fantasy requires commercial power to create social or institutional leverage and institutional power to reshape material conditions."
            ),
        });
    }
}

pub(crate) fn add_persona_convergence_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let personas: BTreeSet<_> = campaigns.iter().map(|campaign| campaign.persona).collect();
    if personas.len() < 3 {
        return;
    }
    let top_sets: Vec<BTreeSet<GameplayCommandKind>> = personas
        .iter()
        .map(|persona| {
            let mut totals = BTreeMap::<GameplayCommandKind, u32>::new();
            for campaign in campaigns
                .iter()
                .filter(|campaign| campaign.persona == *persona)
            {
                for (kind, stats) in &campaign.commands {
                    if is_persona_identity_command(*kind) && !is_cross_persona_enabler(*kind) {
                        let total = totals.entry(*kind).or_default();
                        *total = total.saturating_add(stats.executed);
                    }
                }
            }
            let mut ranked: Vec<_> = totals.into_iter().collect();
            ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
            ranked.into_iter().take(3).map(|(kind, _)| kind).collect()
        })
        .collect();
    let mut common = top_sets
        .iter()
        .skip(1)
        .fold(top_sets[0].clone(), |common, next| {
            common.intersection(next).copied().collect()
        });
    if common.contains(&GameplayCommandKind::NominateForOffice)
        && persona_outcomes_diverge(campaigns, |campaign| campaign.end.player_office_checksum)
    {
        common.remove(&GameplayCommandKind::NominateForOffice);
    }
    if persona_outcomes_diverge(campaigns, |campaign| {
        campaign.end.player_family_capability_checksum
    }) {
        common.remove(&GameplayCommandKind::AdoptWard);
        common.remove(&GameplayCommandKind::EducateFamilyMember);
    }
    if common.len() >= 2 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Distinct personas converge on the same action priorities".to_owned(),
            evidence: format!(
                "At least {} of the three most-used substantive command families were shared by every configured persona: {}.",
                common.len(),
                common
                    .iter()
                    .map(|kind| kind.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
}

pub(crate) fn persona_outcomes_diverge<T: Copy + Ord>(
    campaigns: &[GameplayCampaignReport],
    outcome: impl Fn(&GameplayCampaignReport) -> T,
) -> bool {
    let personas: BTreeSet<_> = campaigns.iter().map(|campaign| campaign.persona).collect();
    let outcome_sets: Vec<BTreeSet<T>> = personas
        .iter()
        .map(|persona| {
            campaigns
                .iter()
                .filter(|campaign| campaign.persona == *persona)
                .map(&outcome)
                .collect()
        })
        .collect();
    let mut comparisons = 0_u64;
    let mut overlap_total = 0_u64;
    for (index, left) in outcome_sets.iter().enumerate() {
        for right in outcome_sets.iter().skip(index + 1) {
            let union = left.union(right).count();
            if union == 0 {
                continue;
            }
            let intersection = left.intersection(right).count();
            overlap_total =
                overlap_total.saturating_add(scaled_ratio_usize(intersection, union, 100));
            comparisons = comparisons.saturating_add(1);
        }
    }
    comparisons > 0 && overlap_total / comparisons < 75
}

pub(crate) const fn is_cross_persona_enabler(kind: GameplayCommandKind) -> bool {
    matches!(
        kind,
        GameplayCommandKind::CommissionInformation
            | GameplayCommandKind::CultivateInstitutionSupport
    )
}

pub(crate) const fn is_persona_identity_command(kind: GameplayCommandKind) -> bool {
    matches!(
        kind,
        GameplayCommandKind::AcquireBusiness
            | GameplayCommandKind::SetBusinessPolicy
            | GameplayCommandKind::SellOutput
            | GameplayCommandKind::ExtendCredit
            | GameplayCommandKind::SellProperty
            | GameplayCommandKind::EnactLaw
            | GameplayCommandKind::StartPublicWork
            | GameplayCommandKind::FundPublicWork
            | GameplayCommandKind::FileLegalCase
            | GameplayCommandKind::SettleLegalCase
            | GameplayCommandKind::SetHouseGovernance
            | GameplayCommandKind::AdoptWard
            | GameplayCommandKind::EducateFamilyMember
            | GameplayCommandKind::CommissionInformation
            | GameplayCommandKind::CultivateInstitutionSupport
            | GameplayCommandKind::NominateForOffice
            | GameplayCommandKind::WithdrawFromInstitution
    )
}

pub(crate) fn add_civic_convergence_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.len() < 8 || average_campaign_days(aggregate) < 720 {
        return;
    }
    let fixed_dimensions = [
        campaigns
            .iter()
            .map(|campaign| campaign.end.active_law_checksum)
            .collect::<BTreeSet<_>>()
            .len(),
        campaigns
            .iter()
            .map(|campaign| campaign.end.player_completed_public_work_checksum)
            .collect::<BTreeSet<_>>()
            .len(),
        campaigns
            .iter()
            .map(|campaign| campaign.end.player_office_checksum)
            .collect::<BTreeSet<_>>()
            .len(),
        campaigns
            .iter()
            .map(|campaign| campaign.end.house_governance as u8)
            .collect::<BTreeSet<_>>()
            .len(),
    ]
    .into_iter()
    .filter(|unique_values| *unique_values == 1)
    .count();
    if fixed_dimensions >= 3 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Civic progression converges despite different strategies".to_owned(),
            evidence: format!(
                "{fixed_dimensions} of four identity-sensitive civic outcome dimensions had no variation across {} campaigns: active law mix, sponsored works, offices held, and house governance.",
                campaigns.len()
            ),
        });
    }
}

pub(crate) fn add_material_civic_outcome_convergence_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.len() < 8 || average_campaign_days(aggregate) < 3_600 {
        return;
    }
    let mut campaigns_by_start: BTreeMap<(u64, &'static str), Vec<&GameplayCampaignReport>> =
        BTreeMap::new();
    for campaign in campaigns {
        campaigns_by_start
            .entry((campaign.seed, campaign.background.recipe_key()))
            .or_default()
            .push(campaign);
    }
    let mut eligible = 0_usize;
    let mut materially_converged = 0_usize;
    let mut dimension_convergence = [0_usize; 5];
    for cohort in campaigns_by_start.values() {
        if cohort
            .iter()
            .map(|campaign| campaign.persona)
            .collect::<BTreeSet<_>>()
            .len()
            < GameplayPersona::all().len()
        {
            continue;
        }
        eligible = eligible.saturating_add(1);
        let civic_identity_variants = cohort
            .iter()
            .map(|campaign| {
                (
                    campaign.end.active_law_checksum,
                    campaign.end.player_completed_public_work_checksum,
                    campaign.end.player_office_checksum,
                    campaign.end.house_governance as u8,
                )
            })
            .collect::<BTreeSet<_>>()
            .len();
        if civic_identity_variants <= 1 {
            continue;
        }
        let converged = [
            endpoint_span(cohort, |campaign| campaign.end.average_food_satisfaction) <= 200,
            district_endpoint_span(cohort, |district| district.unrest_basis_points) <= 400,
            district_endpoint_span(cohort, |district| district.employment_basis_points) <= 500,
            district_endpoint_span(cohort, |district| district.sanitation_basis_points) <= 500,
            district_endpoint_span(cohort, |district| district.safety_basis_points) <= 500,
        ];
        for (count, converged) in dimension_convergence.iter_mut().zip(converged) {
            if converged {
                *count = count.saturating_add(1);
            }
        }
        let converged_dimensions = converged.into_iter().filter(|converged| *converged).count();
        if converged_dimensions >= 4 {
            materially_converged = materially_converged.saturating_add(1);
        }
    }
    if eligible == 0 || materially_converged == 0 {
        return;
    }
    let share = scaled_ratio_usize(materially_converged, eligible, 100);
    findings.push(GameplayFinding {
        severity: if share >= 50 {
            GameplayFindingSeverity::Warning
        } else {
            GameplayFindingSeverity::Info
        },
        title: "Different civic strategies converge on similar material city conditions".to_owned(),
        evidence: format!(
            "{materially_converged} of {eligible} same-start persona cohorts ended within the convergence band in at least four of five material measures despite different laws, public works, offices, or governance. Food uses the citywide endpoint; district measures compare the largest same-district persona span so localized projects are not averaged away. Converged by measure: food {}/{eligible}, unrest {}/{eligible}, employment {}/{eligible}, sanitation {}/{eligible}, safety {}/{eligible}.",
            dimension_convergence[0],
            dimension_convergence[1],
            dimension_convergence[2],
            dimension_convergence[3],
            dimension_convergence[4],
        ),
    });
}

pub(crate) fn endpoint_span(
    cohort: &[&GameplayCampaignReport],
    measure: impl Fn(&GameplayCampaignReport) -> u16,
) -> u16 {
    let minimum = cohort.iter().map(|campaign| measure(campaign)).min();
    let maximum = cohort.iter().map(|campaign| measure(campaign)).max();
    minimum
        .zip(maximum)
        .map_or(0, |(minimum, maximum)| maximum.saturating_sub(minimum))
}

pub(crate) fn district_endpoint_span(
    cohort: &[&GameplayCampaignReport],
    measure: impl Fn(&GameplayDistrictCondition) -> u16 + Copy,
) -> u16 {
    let mut ranges = BTreeMap::<DistrictId, (u16, u16)>::new();
    for campaign in cohort {
        for district in &campaign.end.district_conditions {
            let value = measure(district);
            ranges
                .entry(district.district_id)
                .and_modify(|range| {
                    range.0 = range.0.min(value);
                    range.1 = range.1.max(value);
                })
                .or_insert((value, value));
        }
    }
    ranges
        .values()
        .map(|(minimum, maximum)| maximum.saturating_sub(*minimum))
        .max()
        .unwrap_or(0)
}

pub(crate) fn add_house_governance_convergence_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if campaigns.len() < 8 || average_campaign_days(aggregate) < 1_800 {
        return;
    }
    let mut counts = BTreeMap::<u8, (HouseGovernance, usize)>::new();
    for campaign in campaigns {
        let governance = campaign.end.house_governance;
        counts
            .entry(governance as u8)
            .and_modify(|(_, count)| *count = count.saturating_add(1))
            .or_insert((governance, 1));
    }
    let Some((_, (governance, dominant_count))) =
        counts.into_iter().max_by_key(|(_, (_, count))| *count)
    else {
        return;
    };
    if scaled_ratio_usize(dominant_count, campaigns.len(), 100) < 75 {
        return;
    }
    let governance_changes = campaigns
        .iter()
        .filter(|campaign| {
            campaign
                .commands
                .get(&GameplayCommandKind::SetHouseGovernance)
                .is_some_and(|stats| stats.executed > 0)
        })
        .count();
    if governance_changes < campaigns.len() / 3 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "House governance converges on one succession model".to_owned(),
        evidence: format!(
            "{dominant_count} of {} mature campaigns ended under {governance:?}, even though {governance_changes} campaigns actively rewrote their family charter. Governance is intended to trade succession stability, unity, and administrative capacity rather than collapse to one universal late-game answer.",
            campaigns.len()
        ),
    });
}

pub(crate) fn add_power_exposure_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 3_600 {
        return;
    }
    let established: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.maximum_offices_held > 0)
        .collect();
    if established.len() < 4 {
        return;
    }
    let sheltered = established
        .iter()
        .filter(|campaign| {
            let unmet_duties = campaign
                .end
                .player_unmet_office_duties
                .saturating_sub(campaign.start.player_unmet_office_duties);
            campaign.maximum_player_disputed_employment == 0
                && campaign.end.player_contract_failures == 0
                && campaign.end.distressed_businesses == 0
                && campaign.end.insolvent_businesses == 0
                && campaign.end.player_treasury.copper()
                    >= campaign.start.player_treasury.copper().saturating_div(2)
                && campaign.maximum_contract_relationship_pressure_basis_points < 1_500
                && unmet_duties == 0
        })
        .count();
    if scaled_ratio_usize(sheltered, established.len(), 100) < 50 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Established dynasties often avoid measured power exposure".to_owned(),
        evidence: format!(
            "{sheltered} of {} officeholding campaigns reached the endpoint without a player labor dispute, contract failure, distressed business, insolvent business, major treasury drawdown, at least 1,500 basis points of relationship-driven contract pressure, or unmet office duty. Routine civic payments are not meaningful exposure by themselves; political backlash counts once it materially worsens commercial bargaining. The design calls for greater power to create consequential obligations and vulnerability, not only additional tools.",
            established.len()
        ),
    });
}

pub(crate) fn add_office_duty_failure_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 1_080 || campaigns.is_empty() {
        return;
    }
    let chronic_failures: Vec<_> = campaigns
        .iter()
        .filter_map(|campaign| {
            let failures = campaign
                .end
                .player_unmet_office_duties
                .saturating_sub(campaign.start.player_unmet_office_duties);
            (failures >= 12).then_some((campaign, failures))
        })
        .collect();
    if chronic_failures.is_empty() {
        return;
    }
    let (worst, failures) = chronic_failures
        .into_iter()
        .max_by_key(|(_, failures)| *failures)
        .expect("non-empty chronic office-duty failures must have a maximum");
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Office obligations repeatedly exceed dynasty liquidity".to_owned(),
        evidence: format!(
            "At least one campaign accumulated twelve or more unmet monthly office duties; the worst was {failures} for seed {}, {} {:?}. Political service is creating a recurring liquidity trap rather than a manageable strategic liability.",
            worst.seed,
            worst.persona.label(),
            worst.background,
        ),
    });
}

pub(crate) fn add_dynastic_continuity_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    if average_campaign_days(aggregate) < 7_200 || campaigns.is_empty() {
        return;
    }
    let successions = campaigns
        .iter()
        .filter(|campaign| campaign.end.generation > campaign.start.generation)
        .count();
    if successions == 0 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Long campaigns do not exercise dynastic continuity".to_owned(),
            evidence: format!(
                "None of {} campaigns advanced beyond their starting generation over {} days per campaign.",
                campaigns.len(),
                average_campaign_days(aggregate)
            ),
        });
        return;
    }
    if aggregate
        .commands
        .get(&GameplayCommandKind::DesignateHeir)
        .is_none_or(|stats| stats.executed == 0)
    {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Succession occurs without player preparation".to_owned(),
            evidence: format!(
                "{successions} of {} long campaigns reached a new generation, but none designated an heir. The continuity system is functioning as simulation, not yet as a player-authored dynasty strategy.",
                campaigns.len()
            ),
        });
    }
}

pub(crate) fn add_variance_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let Some(minimum) = campaigns
        .iter()
        .min_by_key(|campaign| campaign.scores.overall)
    else {
        return;
    };
    let Some(maximum) = campaigns
        .iter()
        .max_by_key(|campaign| campaign.scores.overall)
    else {
        return;
    };
    let spread = maximum
        .scores
        .overall
        .saturating_sub(minimum.scores.overall);
    if spread >= 25 {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Large experience variance between starts".to_owned(),
            evidence: format!(
                "Scores ranged from {}/100 ({:?}, {:?}, seed {}) to {}/100 ({:?}, {:?}, seed {}).",
                minimum.scores.overall,
                minimum.persona,
                minimum.background,
                minimum.seed,
                maximum.scores.overall,
                maximum.persona,
                maximum.background,
                maximum.seed
            ),
        });
    }
}

pub(crate) fn add_succession_before_office_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let with_both = campaigns
        .iter()
        .filter(|c| {
            c.fantasy_arc.first_succession_day.is_some() && c.fantasy_arc.first_office_day.is_some()
        })
        .collect::<Vec<_>>();
    if with_both.len() < 4 {
        return;
    }
    let inverted = with_both
        .iter()
        .filter(|c| {
            c.fantasy_arc
                .first_succession_day
                .expect("filtered to some succession")
                < c.fantasy_arc
                    .first_office_day
                    .expect("filtered to some office")
        })
        .count();
    if scaled_ratio_usize(inverted, with_both.len(), 100) < 50 {
        return;
    }
    let earliest_office = with_both
        .iter()
        .filter_map(|c| c.fantasy_arc.first_office_day)
        .min()
        .unwrap_or(0);
    let latest_succ = with_both
        .iter()
        .filter_map(|c| c.fantasy_arc.first_succession_day)
        .max()
        .unwrap_or(0);
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Warning,
        title: "Succession precedes office in most successions".to_owned(),
        evidence: format!(
            "{inverted} of {} campaigns that reached both succession and office saw succession first (succession day precedes office day). Earliest office {earliest_office}, latest succession {latest_succ}. The dynasty fantasy expects the founder to hold office and memberships worth testing at succession; when the Founder dies before first office, governance and succession phases invert and the legacy phase never exercises Dynastic Governance. Consider founder age or succession pressure tuning.",
            with_both.len()
        ),
    });
}

pub(crate) fn add_short_horizon_background_imbalance_finding(
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    // Early-game balance matters as much as mature balance: a 1-year horizon
    // reveals whether a starting trade is a hidden difficulty mode before
    // compound growth hides it. The existing mature checks trigger at 1080
    // and 3600 days; this short-horizon check surfaces the same signal at
    // 360 days when sample is sufficient.
    let eligible: Vec<_> = campaigns
        .iter()
        .filter(|c| c.simulated_days >= 360 && c.simulated_days < 1080)
        .collect();
    if eligible.len() < 12 {
        return;
    }
    let backgrounds: BTreeSet<StartingBackground> = eligible.iter().map(|c| c.background).collect();
    if backgrounds.len() < 2 {
        return;
    }
    let mut averages = Vec::new();
    for bg in backgrounds {
        let sampled: Vec<_> = eligible.iter().filter(|c| c.background == bg).collect();
        if sampled.len() < 4 {
            continue;
        }
        let total_margin: i128 = sampled.iter().fold(0, |s, c| {
            s + i128::from(
                c.end
                    .player_business_lifetime_revenue
                    .copper()
                    .saturating_sub(c.end.player_business_lifetime_costs.copper()),
            )
        });
        let avg = total_margin / i128::try_from(sampled.len()).unwrap_or(1);
        averages.push((bg, avg));
    }
    if averages.len() < 2 {
        return;
    }
    let (strongest, s_avg) = *averages
        .iter()
        .max_by_key(|(_, a)| *a)
        .expect("averages has at least two backgrounds");
    let (weakest, w_avg) = *averages
        .iter()
        .min_by_key(|(_, a)| *a)
        .expect("averages has at least two backgrounds");
    let spread = s_avg.saturating_sub(w_avg);
    if spread < 12_000 {
        return;
    }
    if weakest == StartingBackground::Blacksmith && s_avg > w_avg.saturating_mul(2) {
        findings.push(GameplayFinding {
            severity: GameplayFindingSeverity::Warning,
            title: "Early background economics already diverge sharply by trade".to_owned(),
            evidence: format!(
                "At 360-day horizon, {strongest:?} averaged margin {} vs {weakest:?} at {} (spread {}). A trade that is already 2x behind after one year creates a hidden difficulty mode before compound growth and should be balanced at the recipe/operating-cost level."
                , Money::from_copper(i64::try_from(s_avg).unwrap_or(0)), Money::from_copper(i64::try_from(w_avg).unwrap_or(0)), Money::from_copper(i64::try_from(spread).unwrap_or(0))
            ),
        });
    }
}

pub(crate) fn add_governance_phase_gap_finding(
    aggregate: &GameplayAggregate,
    campaigns: &[GameplayCampaignReport],
    findings: &mut Vec<GameplayFinding>,
) {
    let governance = aggregate
        .phase_stats
        .get(&GameplayPhase::DynasticGovernance)
        .cloned()
        .unwrap_or_default();
    if governance.decision_cycles > 0 {
        return;
    }
    let city_shapers = campaigns
        .iter()
        .filter(|c| c.fantasy_arc.first_city_shaping_action_day.is_some())
        .count();
    let successions = campaigns
        .iter()
        .filter(|c| c.fantasy_arc.first_succession_day.is_some())
        .count();
    if city_shapers == 0 || successions == 0 {
        return;
    }
    let avg_days = average_campaign_days(aggregate);
    if avg_days < 1_000 {
        return;
    }
    findings.push(GameplayFinding {
        severity: GameplayFindingSeverity::Info,
        title: "Dynastic governance phase remains unentered despite city-shaping".to_owned(),
        evidence: format!(
            "{city_shapers} campaigns reached city-shaping and {successions} reached succession, but DynasticGovernance recorded 0 decision cycles. Successions precede city-shaping in this world sample, so post-succession legacy absorbs governance. If average_days={avg_days}, check that succession median sits after typical city-shaping day (~600-800) rather than before.",
        ),
    });
}
