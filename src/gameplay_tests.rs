//! Behavioral coverage for the deterministic gameplay harness.

use super::*;
use crate::core::{
    AuditKind, AuditRecord, Crisis, CrisisKind, FamilyLinkKind, OutboxKind, OutboxMessage,
};
use crate::ids::{DynastyId, OutboxMessageId};
use crate::systems::{INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT, INSTITUTION_SUPPORT_INTERVAL_DAYS};
use crate::systems::{OFFICE_POWER_ESTABLISHMENT_DAYS, OFFICE_TERM_DAYS, issue_loan};
use crate::test_support::{assert_set_eq, make_test_campaign, rivergate_registry_for_test};
use std::sync::OnceLock;

static FOCUSED_REPORT_30_DAYS: OnceLock<GameplayHarnessReport> = OnceLock::new();
static FOCUSED_REPORT_60_DAYS: OnceLock<GameplayHarnessReport> = OnceLock::new();
static FOCUSED_REPORT_180_DAYS: OnceLock<GameplayHarnessReport> = OnceLock::new();

fn focused_config(days: u32) -> GameplayHarnessConfig {
    GameplayHarnessConfig {
        days_per_campaign: days,
        max_candidate_probes: 16,
        trace_limit_per_campaign: 16,
        personas: vec![GameplayPersona::Steward],
        backgrounds: vec![StartingBackground::Baker],
        ..GameplayHarnessConfig::default()
    }
}

fn cached_focused_report(days: u32) -> GameplayHarnessReport {
    let report = match days {
        30 => FOCUSED_REPORT_30_DAYS.get_or_init(|| build_focused_report(30)),
        60 => FOCUSED_REPORT_60_DAYS.get_or_init(|| build_focused_report(60)),
        180 => FOCUSED_REPORT_180_DAYS.get_or_init(|| build_focused_report(180)),
        _ => panic!("no cached focused report is configured for {days} days"),
    };
    report.clone()
}

fn build_focused_report(days: u32) -> GameplayHarnessReport {
    run_gameplay_harness(rivergate_registry_for_test(), focused_config(days))
        .expect("cached focused gameplay report must build")
}

fn background_order(background: StartingBackground) -> u8 {
    match background {
        StartingBackground::Baker => 0,
        StartingBackground::ClothTrader => 1,
        StartingBackground::Blacksmith => 2,
    }
}

#[track_caller]
fn single_candidate(candidates: &[Candidate], context: &str) -> Candidate {
    match candidates {
        [candidate] => candidate.clone(),
        _ => panic!(
            "{context}: expected exactly one candidate, got {}: {candidates:#?}",
            candidates.len()
        ),
    }
}

#[track_caller]
fn finding_with_title<'a>(
    findings: &'a [GameplayFinding],
    expected_title: &str,
) -> &'a GameplayFinding {
    findings
        .iter()
        .find(|finding| finding.title == expected_title)
        .unwrap_or_else(|| {
            let available: Vec<_> = findings
                .iter()
                .map(|finding| finding.title.as_str())
                .collect();
            panic!("expected finding {expected_title:?}; available findings: {available:#?}");
        })
}

#[track_caller]
fn assert_finding_absent(findings: &[GameplayFinding], unexpected_title: &str) {
    if let Some(finding) = findings
        .iter()
        .find(|finding| finding.title == unexpected_title)
    {
        panic!("unexpected finding {unexpected_title:?}: {finding:#?}");
    }
}

mod harness {
    use super::*;

    fn information_candidate_kinds(registry: &Registry) -> BTreeSet<GameplayCommandKind> {
        let mut state = make_test_candidate_coverage_state(registry);
        for _ in 0..180 {
            state.clock.advance_one_day();
        }
        let mut kinds = candidate_kinds_for_test(registry, &state);
        let counterparty_id = state
            .relationships
            .values()
            .find_map(|relationship| {
                if relationship.pair.first == state.player_dynasty_id {
                    Some(relationship.pair.second)
                } else if relationship.pair.second == state.player_dynasty_id {
                    Some(relationship.pair.first)
                } else {
                    None
                }
            })
            .expect("coverage fixture must contain a known counterparty");
        let counterparty_name = state
            .dynasties
            .get(&counterparty_id)
            .expect("counterparty dynasty must exist")
            .name()
            .to_owned();
        // Strain the counterparty relationship so the commissioned brief is still
        // material at leverage time; otherwise the agent would hold the report
        // rather than spend to act on resolved intelligence.
        let player_id = state.player_dynasty_id;
        if let Some(relationship) = state.relationships.values_mut().find(|relationship| {
            relationship.pair.first == player_id || relationship.pair.second == player_id
        }) {
            relationship.trust_basis_points = 3_500;
            relationship.resentment_basis_points = 3_000;
        }
        let report_id = state.next_ids.information_report();
        state.information_reports.insert(
            report_id,
            crate::core::InformationReport {
                id: report_id,
                owner_dynasty_id: state.player_dynasty_id,
                target: Some(crate::core::InformationTarget::Counterparty {
                    dynasty_id: counterparty_id,
                }),
                subject: format!("Commissioned house brief: House {counterparty_name}"),
                confidence: crate::core::InformationConfidence::Confirmed,
                created_day: state.clock.day(),
                expires_day: state.clock.day().saturating_add(540),
                source: COMMISSIONED_INFORMATION_SOURCE.to_owned(),
                summary: "Coverage report".to_owned(),
            },
        );
        for _ in 0..AGENT_INFORMATION_LEVERAGE_DELAY_DAYS {
            state.clock.advance_one_day();
        }
        kinds.extend(candidate_kinds_for_test(registry, &state));
        kinds
    }

    fn support_candidate_kinds(registry: &Registry) -> BTreeSet<GameplayCommandKind> {
        let mut state = make_test_candidate_coverage_state(registry);
        for _ in 0..INSTITUTION_SUPPORT_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        let officeholders: BTreeSet<_> = state
            .institutions
            .values()
            .filter_map(|institution| institution.office_holder_id)
            .collect();
        let character_id = state
            .characters
            .iter()
            .find(|character| {
                character.dynasty_id() == state.player_dynasty_id
                    && !officeholders.contains(&character.id())
            })
            .expect("fixture must contain a player character without an office")
            .id();
        for institution in state.institutions.values_mut() {
            institution.members.remove(&character_id);
        }
        state.audit_log.retain(|record| {
            record.kind() != AuditKind::InstitutionPatronage
                || record
                    .audit_subject()
                    .institution_character_ids()
                    .is_none_or(|(_, recorded_character_id)| recorded_character_id != character_id)
        });
        candidate_kinds_for_test(registry, &state)
    }

    fn succession_candidate_kinds(registry: &Registry) -> BTreeSet<GameplayCommandKind> {
        let mut state = make_test_candidate_coverage_state(registry);
        for _ in 0..(20 * 360) {
            state.clock.advance_one_day();
        }
        let player_id = state.player_dynasty_id;
        let current_heir_id = state
            .dynasties
            .get(&player_id)
            .and_then(crate::core::Dynasty::heir_id)
            .expect("coverage fixture must contain a current heir");
        let replacement_id = state.next_ids.character();
        let mut replacement = state
            .characters
            .get(current_heir_id)
            .expect("current heir must exist")
            .clone();
        replacement.identity.id = replacement_id;
        replacement.identity.name = "Harness Successor".to_owned();
        replacement.identity.birth_day = state.clock.day().saturating_sub(30 * 360);
        replacement.capabilities.administration = 100;
        replacement.capabilities.commerce = 100;
        replacement.capabilities.social = 100;
        replacement.capabilities.craft = 100;
        replacement.runtime.role = crate::core::CharacterRole::Clerk;
        state.characters.insert(replacement);
        state
            .family_councils
            .get_mut(&player_id)
            .expect("player family council must exist")
            .members
            .insert(replacement_id);
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 5_000;
        candidate_kinds_for_test(registry, &state)
    }

    #[test]
    fn default_harness_uses_the_monthly_strategic_cadence() {
        assert_eq!(GameplayHarnessConfig::default().decision_interval_days, 30);
    }

    #[test]
    fn every_serialized_app_state_component_is_reviewed_by_the_harness() {
        let state = make_test_campaign();
        let serialized = serde_json::to_value(&state).expect("campaign state must serialize");
        let actual: BTreeSet<String> = serialized
            .as_object()
            .expect("app state must serialize as an object")
            .keys()
            .cloned()
            .collect();
        let observed: BTreeSet<String> = HARNESS_OBSERVED_STATE_COMPONENTS
            .iter()
            .map(|component| (*component).to_owned())
            .collect();
        let intentionally_unobserved: BTreeSet<String> =
            HARNESS_INTENTIONALLY_UNOBSERVED_STATE_COMPONENTS
                .iter()
                .map(|component| (*component).to_owned())
                .collect();

        assert!(
            observed.is_disjoint(&intentionally_unobserved),
            "state components cannot be both observed and intentionally unobserved"
        );
        assert_set_eq(
            &actual,
            &observed.union(&intentionally_unobserved).cloned().collect(),
            "adding or removing AppState fields requires an explicit gameplay-harness review",
        );
    }

    #[test]
    fn mislabeled_player_command_candidates_fail_before_probing() {
        let state = make_test_campaign();
        let candidates = vec![Candidate {
            kind: GameplayCommandKind::BuyProperty,
            command: PlayerCommand::SetHouseGovernance {
                governance: HouseGovernance::Primogeniture,
            },
            description: "deliberately stale route".to_owned(),
            score: 0,
        }];

        let error = validate_candidate_classifications(&state, &candidates)
            .expect_err("mislabeled candidates must fail");

        assert!(matches!(
            error,
            GameplayHarnessError::CandidateKindMismatch {
                declared: GameplayCommandKind::BuyProperty,
                actual: GameplayCommandKind::SetHouseGovernance,
                ..
            }
        ));
    }

    #[test]
    fn scaled_ratios_use_wide_intermediates() {
        assert_eq!(
            scaled_ratio_u64(u64::MAX, u64::MAX, 100),
            100,
            "equal maximum counters must still report a complete share"
        );
        assert_eq!(
            ratio_score(u32::MAX, u32::MAX),
            100,
            "score ratios must not saturate before division"
        );
    }

    #[test]
    fn rejects_empty_campaign_dimensions() {
        let registry = rivergate_registry_for_test();
        let mut config = focused_config(30);
        config.personas.clear();

        let error = run_gameplay_harness(registry, config).expect_err("empty personas must fail");

        assert!(matches!(error, GameplayHarnessError::InvalidConfig { .. }));
    }

    #[test]
    fn rejects_duplicate_campaign_dimensions() {
        let registry = rivergate_registry_for_test();
        let mut config = focused_config(30);
        config.personas.push(GameplayPersona::Steward);

        let error = run_gameplay_harness(registry, config)
            .expect_err("duplicate personas must not duplicate matrix samples");

        assert!(matches!(
            error,
            GameplayHarnessError::InvalidConfig { reason }
                if reason == "persona Steward was configured more than once"
        ));

        let mut config = focused_config(30);
        config.backgrounds.push(StartingBackground::Baker);

        let error = run_gameplay_harness(registry, config)
            .expect_err("duplicate backgrounds must not duplicate matrix samples");

        assert!(matches!(
            error,
            GameplayHarnessError::InvalidConfig { reason }
                if reason == "background Baker was configured more than once"
        ));
    }

    #[test]
    fn rejects_seed_ranges_that_would_repeat_through_saturation() {
        let registry = rivergate_registry_for_test();
        let mut config = focused_config(30);
        config.start_seed = u64::MAX;
        config.seed_count = 2;

        let error = run_gameplay_harness(registry, config)
            .expect_err("overflowing seed ranges must be rejected");

        assert!(matches!(
            error,
            GameplayHarnessError::InvalidConfig { reason }
                if reason == "configured seed range exceeds u64::MAX"
        ));
    }

    #[test]
    fn identical_configuration_produces_identical_report() {
        let registry = rivergate_registry_for_test();
        let config = focused_config(90);

        let first = run_gameplay_harness(registry, config.clone()).expect("first run must succeed");
        let second = run_gameplay_harness(registry, config).expect("second run must succeed");

        assert_eq!(first, second, "gameplay reports must be reproducible");
    }

    #[test]
    fn parallel_counterfactual_probes_match_serial_results() {
        let registry = rivergate_registry_for_test();
        let config = focused_config(90);

        let parallel = run_campaign(
            registry,
            &config,
            config.start_seed,
            StartingBackground::Baker,
            GameplayPersona::Steward,
            true,
        )
        .expect("parallel campaign must succeed");
        let serial = run_campaign(
            registry,
            &config,
            config.start_seed,
            StartingBackground::Baker,
            GameplayPersona::Steward,
            false,
        )
        .expect("serial campaign must succeed");

        assert_eq!(
            parallel, serial,
            "parallel probes must preserve report semantics"
        );
    }

    #[test]
    fn organic_candidate_variation_is_bounded_reproducible_and_non_mutating() {
        let mut state = make_test_campaign();
        let original = state.clone();
        let candidate = Candidate {
            kind: GameplayCommandKind::SetHouseGovernance,
            command: PlayerCommand::ConveneFamilyCouncil,
            description: "sample nearby choice".to_owned(),
            score: 0,
        };
        let accumulator = CampaignAccumulator::new();
        let first =
            organic_candidate_variation(&state, GameplayPersona::Steward, &accumulator, &candidate);
        let second =
            organic_candidate_variation(&state, GameplayPersona::Steward, &accumulator, &candidate);

        assert_eq!(first, second, "variation must be reproducible");
        assert!(
            (-ORGANIC_CANDIDATE_VARIATION_RANGE..=ORGANIC_CANDIDATE_VARIATION_RANGE)
                .contains(&first),
            "variation must remain bounded: {first}"
        );
        assert_eq!(state, original, "variation must not consume the game RNG");

        let mut samples = BTreeSet::new();
        for _ in 0..12 {
            state.clock.advance_one_day();
            samples.insert(organic_candidate_variation(
                &state,
                GameplayPersona::Steward,
                &accumulator,
                &candidate,
            ));
        }
        assert!(
            samples.len() >= 3,
            "variation should sample nearby choices across campaign time: {samples:?}"
        );
    }

    #[test]
    fn multi_campaign_reports_are_deterministic_across_parallel_runs() {
        let registry = rivergate_registry_for_test();
        let mut config = focused_config(90);
        config.personas = GameplayPersona::all().to_vec();
        config.backgrounds.push(StartingBackground::ClothTrader);
        config.backgrounds.push(StartingBackground::Blacksmith);

        let first = run_gameplay_harness(registry, config.clone()).expect("first run must succeed");
        let second = run_gameplay_harness(registry, config).expect("second run must succeed");

        assert_eq!(
            first, second,
            "parallel gameplay reports must be reproducible"
        );
        assert_eq!(
            first.aggregate.campaigns, 12,
            "every persona-background pair must run"
        );
        assert!(
            first.aggregate.simulated_days > 0,
            "parallel campaigns must simulate time"
        );
        assert_eq!(
            first.persona_aggregates.len(),
            GameplayPersona::all().len(),
            "each persona must have its own aggregate"
        );
    }

    #[test]
    fn parallel_matrix_preserves_seed_background_persona_ordering() {
        let registry = rivergate_registry_for_test();
        let mut config = focused_config(30);
        config.start_seed = 5;
        config.seed_count = 2;
        config.personas = GameplayPersona::all().to_vec();
        config.backgrounds.push(StartingBackground::ClothTrader);
        config.backgrounds.push(StartingBackground::Blacksmith);

        let report =
            run_gameplay_harness(registry, config).expect("gameplay harness must complete");

        assert_eq!(
            report.campaigns.len(),
            24,
            "matrix must run every combination"
        );
        let expected: Vec<_> = report
            .campaigns
            .iter()
            .map(|campaign| {
                (
                    campaign.seed,
                    background_order(campaign.background),
                    campaign.persona,
                )
            })
            .collect();
        let mut sorted = expected.clone();
        sorted.sort();
        assert_eq!(
            expected, sorted,
            "report campaigns must be ordered by seed, then background, then persona"
        );
        assert_eq!(
            report.campaigns.iter().map(|campaign| campaign.seed).min(),
            Some(5),
            "the matrix must include the configured starting seed"
        );
    }

    #[test]
    fn plays_through_real_commands_and_reports_system_reactions() {
        let registry = rivergate_registry_for_test();
        let mut config = focused_config(180);
        config.decision_interval_days = 7;
        config.personas = vec![GameplayPersona::Entrepreneur];
        let report =
            run_gameplay_harness(registry, config).expect("gameplay harness must complete");
        let campaign = report.campaigns.first().expect("one campaign must run");

        assert_eq!(report.schema_version, GAMEPLAY_REPORT_SCHEMA_VERSION);
        assert_eq!(report.aggregate.campaigns, 1);
        assert_eq!(report.aggregate.simulated_days, 180);
        assert_eq!(
            report
                .persona_aggregates
                .get(&GameplayPersona::Entrepreneur),
            Some(&report.aggregate),
            "a single-persona run should expose the same metrics through its persona aggregate"
        );
        assert!(report.aggregate.successful_actions > 0);
        assert!(report.aggregate.candidate_probes > report.aggregate.successful_actions);
        assert!(
            report
                .aggregate
                .commands
                .values()
                .map(|stats| stats.generated)
                .sum::<u32>()
                >= u32::try_from(report.aggregate.candidate_probes).unwrap_or(u32::MAX),
            "generated choices must include every probed choice"
        );
        assert!(report.aggregate.command_coverage >= 4);
        assert!(report.aggregate.domain_coverage >= 10);
        assert!(!report.aggregate.interactions.is_empty());
        assert_eq!(
            report.aggregate.no_action_cycles,
            report
                .aggregate
                .quiet_cycles
                .saturating_add(report.aggregate.blocked_cycles),
            "every no-action cycle must be classified as quiet or blocked"
        );
        assert_eq!(
            u64::from(campaign.no_action_cycles),
            u64::from(campaign.quiet_cycles).saturating_add(u64::from(campaign.blocked_cycles)),
            "campaign-level no-action classification must remain complete"
        );
        assert!(
            report
                .aggregate
                .commands
                .values()
                .any(|stats| stats.offered_cycles > 0),
            "reports must distinguish cycles that offered a command family"
        );
        assert!(
            report
                .aggregate
                .commands
                .values()
                .any(|stats| stats.actions_with_persistent_consequences > 0),
            "reports must retain persistent consequences separately from delayed ones"
        );
        assert!(
            campaign
                .trace
                .iter()
                .any(|step| step.selected_command.is_some() && step.outcome.is_some()),
            "trace must preserve actual command outcomes"
        );
        assert!(
            campaign
                .trace
                .iter()
                .any(|step| !step.ambient_domains.is_empty()),
            "trace must distinguish autonomous simulation activity"
        );
        assert!(
            campaign.trace.iter().all(|step| {
                step.viable_options
                    .iter()
                    .all(|option| step.viable_command_kinds.contains(&option.command))
            }),
            "reported alternatives must be successfully probed command families"
        );
        assert!(
            report.aggregate.cycles_with_close_viable_command_kinds
                <= report.aggregate.cycles_with_multiple_viable_command_kinds
        );
        assert!(
            report.aggregate.cycles_with_distinct_immediate_consequences
                <= report.aggregate.cycles_with_multiple_viable_command_kinds
        );
        assert!(
            report.aggregate.cycles_with_distinct_projected_consequences
                <= report.aggregate.cycles_with_multiple_viable_command_kinds
        );
        assert!(report.aggregate.quiet_cycles_with_ambient_change <= report.aggregate.quiet_cycles);
    }

    #[test]
    fn urgent_world_state_shortens_the_next_observation_window() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_candidate_coverage_state(registry);
        add_active_crisis(&mut state);

        assert_eq!(
            next_campaign_step_days(&state, 30),
            7,
            "an uncontained crisis must be observed before the normal monthly cadence elapses"
        );
    }

    #[test]
    fn trace_retains_feedback_and_explicit_phase_context() {
        let report = cached_focused_report(180);
        let campaign = report.campaigns.first().expect("one campaign must run");

        assert!(
            campaign.trace.iter().any(|step| {
                !step.command_feedback.is_empty()
                    || !step.simulation_feedback.is_empty()
                    || !step.ambient_feedback.is_empty()
            }),
            "trace must retain durable feedback that explains at least one state transition"
        );
        assert!(
            campaign.trace.iter().all(|step| {
                step.phase.label() == phase_label_at_day(&campaign.fantasy_arc, step.day)
            }),
            "trace phase must agree with the campaign fantasy arc"
        );
    }

    #[test]
    fn trace_feedback_windows_state_what_each_branch_covered() {
        let report = cached_focused_report(180);
        let campaign = report.campaigns.first().expect("one campaign must run");

        for step in &campaign.trace {
            if step.selected_command.is_some() {
                assert!(
                    step.ambient_window_days >= step.simulation_window_days,
                    "the attribution horizon must cover at least the simulation advance"
                );
            } else {
                assert_eq!(
                    step.ambient_window_days, step.simulation_window_days,
                    "quiet cycles never branch, so their feedback windows must agree"
                );
            }
        }
        let rendered = render_gameplay_report(&report);
        assert!(
            rendered.contains("simulation feedback over"),
            "the decision log must state how much time each feedback window covers"
        );
    }

    #[test]
    fn trace_steps_record_no_action_causes_and_render_a_decision_log() {
        let report = cached_focused_report(180);
        let campaign = report.campaigns.first().expect("one campaign must run");

        for step in &campaign.trace {
            if step.selected_command.is_none() {
                assert!(
                    step.no_action_reason.is_some(),
                    "no-action trace steps must explain why the agent could not act"
                );
            } else {
                assert_eq!(
                    step.no_action_reason, None,
                    "action trace steps must not carry a no-action reason"
                );
            }
        }
        let rendered = render_gameplay_report(&report);
        assert!(
            rendered.contains("Decision log") && rendered.contains("campaign seed"),
            "the rendered report must include the chronological decision log"
        );
        assert!(
            rendered.contains("deltas | immediate ["),
            "the decision log must expose measured immediate, attributed, and ambient consequences"
        );
        if campaign
            .trace
            .iter()
            .any(|step| step.selected_command.is_none())
        {
            assert!(
                rendered.contains("NO ACTION"),
                "the decision log must expose no-action cycles with their cause"
            );
            assert!(
                rendered.contains("dormant")
                    || rendered.contains("opportunities without candidates"),
                "the rendered report must expose the quiet diagnosis when no action occurs"
            );
        }
    }

    #[test]
    fn candidate_scenarios_cover_every_command_family() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_candidate_coverage_state(registry);
        let mut kinds = candidate_kinds_for_test(registry, &state);

        let mut directional_state = state.clone();
        make_supply_security_and_borrowing_available(&mut directional_state);
        kinds.extend(candidate_kinds_for_test(registry, &directional_state));

        let mut lending_state = state.clone();
        make_external_credit_need_available_for_test(&mut lending_state);
        kinds.extend(candidate_kinds_for_test(registry, &lending_state));

        make_player_business_distressed(&mut state);
        kinds.extend(candidate_kinds_for_test(registry, &state));
        restore_distressed_business_cash_for_test(&mut state);
        add_active_crisis(&mut state);
        make_player_labor_disputed(&mut state);
        make_player_contract_breached(&mut state);
        for _ in 0..LEGAL_CASE_FILING_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        kinds.extend(candidate_kinds_for_test(registry, &state));

        remove_internal_transfer_surplus_for_test(&mut state);
        make_player_business_distressed(&mut state);
        kinds.extend(candidate_kinds_for_test(registry, &state));

        let mut liquidation_state = make_test_candidate_coverage_state(registry);
        make_property_liquidation_available_for_test(&mut liquidation_state);
        kinds.extend(candidate_kinds_for_test(registry, &liquidation_state));

        let mut withdrawal_state = make_test_candidate_coverage_state(registry);
        make_institution_withdrawal_available_for_test(&mut withdrawal_state);
        kinds.extend(candidate_kinds_for_test(registry, &withdrawal_state));

        let mut funding_state = make_test_candidate_coverage_state(registry);
        make_public_work_funding_available_for_test(&mut funding_state);
        kinds.extend(candidate_kinds_for_test(registry, &funding_state));

        let mut settlement_state = make_test_candidate_coverage_state(registry);
        make_legal_settlement_available_for_test(&mut settlement_state);
        kinds.extend(candidate_kinds_for_test(registry, &settlement_state));

        let mut family_state = make_test_candidate_coverage_state(registry);
        let player_id = family_state.player_dynasty_id;
        family_state
            .family_councils
            .get_mut(&player_id)
            .expect("player family council must exist")
            .unity_basis_points = 5_000;
        family_state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(100_000);
        kinds.extend(candidate_kinds_for_test(registry, &family_state));

        kinds.extend(information_candidate_kinds(registry));
        kinds.extend(support_candidate_kinds(registry));
        kinds.extend(succession_candidate_kinds(registry));

        assert_set_eq(
            &ALL_COMMAND_KINDS.into_iter().collect(),
            &kinds,
            "candidate scenarios must cover every player command family",
        );
    }

    #[test]
    fn contract_candidates_require_buyer_working_cash() {
        let registry = rivergate_registry_for_test();
        let mut state = build_new_game(
            registry,
            NewGameConfig {
                seed: 17,
                dynasty_name: "Harness".to_owned(),
                founder_name: "Harness Founder".to_owned(),
                background: StartingBackground::Baker,
            },
        )
        .expect("campaign must build");
        let buyer = state
            .businesses
            .iter()
            .find(|business| business.owner_dynasty_id() == state.player_dynasty_id)
            .expect("player business must exist")
            .id();
        let buyer_recipe = registry
            .get_recipe(
                state
                    .businesses
                    .get(buyer)
                    .expect("buyer must exist")
                    .recipe_id(),
            )
            .expect("buyer recipe must exist");
        let input = buyer_recipe
            .inputs()
            .first()
            .expect("baker recipe must consume an input");
        let seller = contract_sellers(registry, &state, input.good_id(), state.player_dynasty_id)
            .next()
            .expect("a nonplayer seller must exist");
        state
            .businesses
            .get_mut(buyer)
            .expect("buyer must exist")
            .finance
            .cash = Money::ZERO;

        assert!(
            !can_support_contract_terms(
                registry,
                &state,
                buyer,
                seller,
                input.good_id(),
                input.quantity().saturating_mul_ratio(4, 1),
                state
                    .market
                    .get_quote(input.good_id())
                    .expect("input good must have a market quote")
                    .price(),
            ),
            "agents must not propose supply contracts the buyer cannot finance"
        );
    }

    #[test]
    fn contract_candidates_reject_unrepresentable_working_cash_requirements() {
        let registry = rivergate_registry_for_test();
        let mut state = build_new_game(
            registry,
            NewGameConfig {
                seed: 17,
                dynasty_name: "Harness".to_owned(),
                founder_name: "Harness Founder".to_owned(),
                background: StartingBackground::Baker,
            },
        )
        .expect("campaign must build");
        let buyer = state
            .businesses
            .iter()
            .find(|business| business.owner_dynasty_id() == state.player_dynasty_id)
            .expect("player business must exist")
            .id();
        let buyer_recipe = registry
            .get_recipe(
                state
                    .businesses
                    .get(buyer)
                    .expect("buyer must exist")
                    .recipe_id(),
            )
            .expect("buyer recipe must exist");
        let input = buyer_recipe
            .inputs()
            .first()
            .expect("baker recipe must consume an input");
        let seller = contract_sellers(registry, &state, input.good_id(), state.player_dynasty_id)
            .next()
            .expect("a nonplayer seller must exist");
        let buyer_business = state.businesses.get_mut(buyer).expect("buyer must exist");
        buyer_business.finance.cash = Money::from_copper(i64::MAX);
        buyer_business.policy.minimum_cash_reserve = Money::ZERO;

        assert!(
            !can_support_contract_terms(
                registry,
                &state,
                buyer,
                seller,
                input.good_id(),
                input.quantity().saturating_mul_ratio(4, 1),
                Money::from_copper(i64::MAX),
            ),
            "agents must reject contracts whose payment or four-week liquidity requirement cannot be represented exactly"
        );
    }

    #[test]
    fn contract_counterparty_discovery_keeps_multiple_viable_houses_visible() {
        let registry = rivergate_registry_for_test();
        let state = make_test_campaign();
        let grain_id = registry
            .get_good_id("grain")
            .expect("registry must define grain");

        let sellers = contract_sellers(registry, &state, grain_id, state.player_dynasty_id)
            .collect::<Vec<_>>();

        assert!(
            sellers.len() >= 2,
            "Rivergate contains multiple external grain suppliers; the harness must not collapse them to the first match"
        );
        assert!(
            sellers.windows(2).all(|pair| pair[0] < pair[1]),
            "counterparty discovery must preserve deterministic business ordering"
        );
    }

    #[test]
    fn rendered_report_surfaces_scores_findings_and_traces() {
        let report = cached_focused_report(60);

        let rendered = render_gameplay_report(&report);

        for heading in [
            "scores:",
            "Persona comparison",
            "Experience health",
            "ending civic conditions:",
            "player-issued",
            "Command coverage",
            "Strongest observed command consequences",
            "Findings",
            "Harness limits",
            "Decision log",
        ] {
            assert!(rendered.contains(heading), "report must contain {heading}");
        }
        assert!(
            rendered.contains("civic | laws") && rendered.contains("| works"),
            "campaign summaries must expose readable civic identity instead of only checksums"
        );
        assert!(
            rendered.contains("NO ACTION") || rendered.contains("day"),
            "the decision log must expose chronological per-cycle play"
        );
        serde_json::to_string(&report).expect("report must serialize to JSON");
        assert!(
            !report.limitations.is_empty()
                && report
                    .limitations
                    .iter()
                    .all(|limitation| !limitation.trim().is_empty()),
            "reports must expose at least one nonempty interpretation limit"
        );
    }
}

mod candidates {
    use super::*;
    use crate::core::{CharacterCapabilities, PropertyKind};

    fn establish_player_contract_market(state: &mut AppState) -> crate::ids::GoodId {
        for _ in 0..180 {
            state.clock.advance_one_day();
        }
        let player_business_ids: BTreeSet<_> = state
            .businesses
            .iter()
            .filter(|business| business.owner_dynasty_id() == state.player_dynasty_id)
            .map(crate::core::Business::id)
            .collect();
        for contract in state.contracts.values_mut().filter(|contract| {
            player_business_ids.contains(&contract.buyer_business_id)
                || player_business_ids.contains(&contract.seller_business_id)
        }) {
            contract.end_day = state.clock.day().saturating_add(360);
        }
        state
            .contracts
            .values()
            .find(|contract| player_external_contract(state, contract))
            .expect("player must have an external contract")
            .good_id
    }

    fn make_aged_player_lender_default() -> (AppState, DynastyId, crate::ids::LoanId, Money, Money)
    {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let borrower_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| {
                *dynasty_id != player_id
                    && !state.loans.values().any(|loan| {
                        loan.lender_dynasty_id == player_id
                            && loan.borrower_dynasty_id == *dynasty_id
                            && matches!(
                                loan.status,
                                LoanStatus::Current
                                    | LoanStatus::Delinquent
                                    | LoanStatus::Restructured
                            )
                    })
            })
            .expect("campaign must contain a borrowing dynasty");
        let loan_id = issue_loan(
            &mut state,
            LoanTerms {
                lender_dynasty_id: player_id,
                borrower_dynasty_id: borrower_id,
                principal: Money::from_copper(1_000),
                weekly_payment: Money::from_copper(10),
                interest_basis_points: 900,
                collateral_property_id: None,
            },
        )
        .expect("player credit must be issued");
        let (principal_before, balance_before) = {
            let loan = state.loans.get_mut(&loan_id).expect("loan must exist");
            loan.status = LoanStatus::Defaulted;
            loan.missed_payments = 3;
            loan.next_due_day = state
                .clock
                .day()
                .saturating_sub(DEFAULTED_LOAN_RESTRUCTURING_COOLDOWN_DAYS);
            (loan.principal, loan.balance)
        };
        for dynasty in state.dynasties.values_mut() {
            dynasty.resources.treasury = Money::from_copper(100_000);
        }
        state
            .dynasties
            .get_mut(&borrower_id)
            .expect("borrower must exist")
            .resources
            .treasury = Money::ZERO;
        (
            state,
            borrower_id,
            loan_id,
            principal_before,
            balance_before,
        )
    }

    #[test]
    fn office_candidates_include_existing_institution_members_after_cooldown() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_office_nomination_record_for_test(registry, &mut state);
        let player_character_ids: Vec<_> = state
            .characters
            .iter()
            .filter(|character| {
                character.dynasty_id() == state.player_dynasty_id
                    && character.status() == CharacterStatus::Active
            })
            .map(crate::core::Character::id)
            .collect();
        for institution in state.institutions.values_mut() {
            for character_id in &player_character_ids {
                institution.members.insert(*character_id);
            }
            if institution
                .office_holder_id
                .is_some_and(|holder_id| player_character_ids.contains(&holder_id))
            {
                institution.office_holder_id = None;
            }
        }
        let player = state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        player.resources.reputation_quality_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        player.resources.treasury = Money::from_copper(10_000);
        let mut candidates = Vec::new();

        generate_family_candidates(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut candidates,
        );

        let nominations: Vec<_> = candidates
            .iter()
            .filter_map(|candidate| match &candidate.command {
                PlayerCommand::NominateForOffice {
                    institution_id,
                    character_id,
                } => Some((*institution_id, *character_id)),
                _ => None,
            })
            .collect();
        assert!(
            !nominations.is_empty(),
            "institution membership must not permanently remove office campaigns from the choice set"
        );
        assert!(nominations.iter().all(|(institution_id, character_id)| {
            state
                .institutions
                .get(institution_id)
                .is_some_and(|institution| institution.members.contains(character_id))
        }));
    }

    #[test]
    fn political_agent_does_not_stack_redundant_family_patronage_in_one_institution() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_office_nomination_record_for_test(registry, &mut state);
        let character_id = eligible_office_characters(&state)
            .first()
            .expect("player dynasty must have an office-eligible character")
            .id();
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        state
            .institutions
            .get_mut(&institution_id)
            .expect("selected institution must exist")
            .members
            .insert(character_id);
        let mut candidates = Vec::new();

        generate_institution_ascent_candidates(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut candidates,
        );

        assert!(candidates.iter().all(|candidate| {
            !matches!(
                candidate.command,
                PlayerCommand::CultivateInstitutionSupport {
                    institution_id: candidate_institution_id,
                    ..
                } if candidate_institution_id == institution_id
            )
        }));
    }

    #[test]
    fn institution_capability_fit_bonus_is_bounded_to_strategic_scale() {
        assert_eq!(institution_capability_fit_bonus(0), 0);
        assert_eq!(institution_capability_fit_bonus(5_000), 250);
        assert_eq!(institution_capability_fit_bonus(10_000), 500);
        assert_eq!(institution_capability_fit_bonus(13_000), 500);
    }

    #[test]
    fn family_education_opens_before_full_office_candidacy() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        {
            let player = state
                .dynasties
                .get_mut(&player_id)
                .expect("player dynasty must exist");
            player.resources.treasury = Money::from_copper(20_000);
            player.resources.reputation_reliability_basis_points =
                INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT;
        }
        grant_player_contract_deliveries_for_test(&mut state, 4);
        assert!(
            player_contract_deliveries(&state) < INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT,
            "fixture must remain below the institution-support commercial threshold"
        );
        let mut candidates = Vec::new();

        generate_family_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &mut candidates,
        );

        let candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::EducateFamilyMember)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "expected establishment-stage education candidate; observed: {candidates:#?}"
                )
            });
        apply_player_command(registry, &mut state, candidate.command)
            .expect("generated establishment-stage education must be executable");
    }

    #[test]
    fn wealthy_established_dynasty_can_allocate_surplus_to_an_institution() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(200_000);
        let protected_floor = AGENT_ENDOWMENT_LIQUIDITY_FLOOR.max(
            player_office_duty_reserve(&state, 0).saturating_add(AGENT_ENDOWMENT_OFFICE_BUFFER),
        );
        let mut candidates = Vec::new();

        generate_family_candidates(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut candidates,
        );

        let candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::EndowInstitution)
            .expect("wealthy established dynasty must receive an endowment choice");
        let PlayerCommand::EndowInstitution { amount, .. } = candidate.command else {
            panic!("endowment command kind must carry an endowment command");
        };
        assert!(amount >= INSTITUTION_ENDOWMENT_MIN);
        assert!(amount <= INSTITUTION_ENDOWMENT_MAX);
        assert!(Money::from_copper(200_000).saturating_sub(amount) >= protected_floor);
    }

    #[test]
    fn active_office_campaign_does_not_bypass_mature_endowment_liquidity_floor() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let character_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        let institution_id = *state
            .institutions
            .keys()
            .next()
            .expect("campaign must contain an institution");
        let required_deliveries = institution_support_delivery_requirement(
            registry,
            &state,
            institution_id,
            character_id,
        );
        {
            let player = state
                .dynasties
                .get_mut(&player_id)
                .expect("player dynasty must exist");
            player.resources.treasury = Money::from_copper(100_000);
            player.resources.reputation_reliability_basis_points =
                INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT;
        }
        grant_player_contract_deliveries_for_test(&mut state, required_deliveries);
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CultivateInstitutionSupport {
                institution_id,
                character_id,
            },
        )
        .expect("qualified patronage must succeed");
        advance_days(
            registry,
            &mut state,
            u32::try_from(INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS)
                .expect("support establishment period must fit u32"),
        )
        .expect("campaign must reach established support");
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeNomination,
            subject: format!("institution:{institution_id}:character:{character_id}").into(),
            detail: "campaign_cost=300".to_owned(),
        });
        let protected_floor = AGENT_ENDOWMENT_LIQUIDITY_FLOOR.max(
            player_office_duty_reserve(&state, 0).saturating_add(AGENT_ENDOWMENT_OFFICE_BUFFER),
        );
        let treasury = protected_floor
            .saturating_add(INSTITUTION_ENDOWMENT_MIN)
            .saturating_sub(Money::from_copper(1));
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = treasury;
        let mut candidates = Vec::new();

        generate_family_candidates(registry, &state, GameplayPersona::Steward, &mut candidates);

        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.kind != GameplayCommandKind::EndowInstitution),
            "a live office campaign must not bypass the mature endowment liquidity floor: {candidates:#?}"
        );
    }

    #[test]
    fn unresolved_office_campaign_preserves_parallel_family_nomination_route() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let characters: Vec<_> = state
            .characters
            .ids_for_dynasty(player_id)
            .into_iter()
            .flatten()
            .copied()
            .take(2)
            .collect();
        assert_eq!(
            characters.len(),
            2,
            "campaign must contain two family members"
        );
        let institutions: Vec<_> = state.institutions.keys().copied().take(2).collect();
        assert_eq!(
            institutions.len(),
            2,
            "campaign must contain two institutions"
        );
        for (character_id, institution_id) in characters.iter().zip(institutions.iter()) {
            state
                .institutions
                .get_mut(institution_id)
                .expect("institution must exist")
                .members
                .insert(*character_id);
            state.audit_log.push(AuditRecord {
                day: state
                    .clock
                    .day()
                    .saturating_sub(INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS),
                kind: AuditKind::InstitutionPatronage,
                subject: format!("institution:{institution_id}:character:{character_id}").into(),
                detail: "test support".to_owned(),
            });
        }
        {
            let player = state
                .dynasties
                .get_mut(&player_id)
                .expect("player dynasty must exist");
            player.resources.treasury = Money::from_copper(100_000);
            player.resources.reputation_quality_basis_points =
                OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
            player.resources.reputation_reliability_basis_points =
                OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
        }
        let required_deliveries = characters
            .iter()
            .zip(institutions.iter())
            .map(|(character_id, institution_id)| {
                office_nomination_delivery_requirement(
                    registry,
                    &state,
                    *institution_id,
                    *character_id,
                )
            })
            .max()
            .expect("fixture must have nomination requirements");
        grant_player_contract_deliveries_for_test(&mut state, required_deliveries);
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeNomination,
            subject: format!(
                "institution:{}:character:{}",
                institutions[0], characters[0]
            )
            .into(),
            detail: "campaign_cost=300".to_owned(),
        });
        let mut candidates = Vec::new();

        generate_institution_ascent_candidates(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut candidates,
        );

        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::NominateForOffice),
            "a larger trained family must retain the option to run a parallel office campaign: {candidates:#?}"
        );
    }

    #[test]
    fn established_dynasty_does_not_offer_generic_family_skill_grinding() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        {
            let player = state
                .dynasties
                .get_mut(&player_id)
                .expect("player dynasty must exist");
            player.resources.treasury = Money::from_copper(20_000);
            player.resources.reputation_reliability_basis_points =
                INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT;
            player.runtime.succession_risk_basis_points = 0;
        }
        grant_player_contract_deliveries_for_test(
            &mut state,
            INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT,
        );
        let head_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        {
            let head = state
                .characters
                .get_mut(head_id)
                .expect("player head must exist");
            head.capabilities = CharacterCapabilities {
                administration: 100,
                commerce: 100,
                social: 100,
                craft: 100,
            };
            head.identity.birth_day = state.clock.day().saturating_sub(30 * 360);
        }
        let institution_id = state
            .institutions
            .keys()
            .copied()
            .next()
            .expect("campaign must contain an institution");
        state
            .institutions
            .get_mut(&institution_id)
            .expect("institution must exist")
            .members
            .insert(head_id);
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::InstitutionPatronage,
            subject: format!("institution:{institution_id}:character:{head_id}").into(),
            detail: "test support".to_owned(),
        });
        let mut candidates = Vec::new();

        generate_family_education_candidates(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut candidates,
        );

        assert!(
            candidates.is_empty(),
            "once the dynasty has an institutional foothold, education must prepare a concrete office or succession role rather than fill cooldowns: {candidates:#?}"
        );
    }

    #[test]
    fn succession_pressure_targets_heir_primary_capability() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let (head_id, heir_id) = {
            let player = state
                .dynasties
                .get_mut(&player_id)
                .expect("player dynasty must exist");
            player.resources.treasury = Money::from_copper(20_000);
            player.resources.reputation_reliability_basis_points =
                INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT;
            player.runtime.succession_risk_basis_points = 3_000;
            (
                player.head_id(),
                player.heir_id().expect("player dynasty must have an heir"),
            )
        };
        grant_player_contract_deliveries_for_test(
            &mut state,
            INSTITUTION_SUPPORT_DELIVERY_REQUIREMENT,
        );
        {
            let head = state
                .characters
                .get_mut(head_id)
                .expect("player head must exist");
            head.capabilities = CharacterCapabilities {
                administration: 100,
                commerce: 100,
                social: 100,
                craft: 100,
            };
        }
        {
            let heir = state
                .characters
                .get_mut(heir_id)
                .expect("player heir must exist");
            heir.identity.birth_day = state.clock.day().saturating_sub(25 * 360);
            heir.capabilities.administration = heir.capabilities.administration.min(80);
        }
        let institution_id = state
            .institutions
            .keys()
            .copied()
            .next()
            .expect("campaign must contain an institution");
        state
            .institutions
            .get_mut(&institution_id)
            .expect("institution must exist")
            .members
            .insert(head_id);
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::InstitutionPatronage,
            subject: format!("institution:{institution_id}:character:{head_id}").into(),
            detail: "test support".to_owned(),
        });
        let mut candidates = Vec::new();

        generate_family_education_candidates(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut candidates,
        );

        let succession = candidates
            .iter()
            .find(|candidate| {
                matches!(
                    candidate.command,
                    PlayerCommand::EducateFamilyMember {
                        character_id,
                        focus: EducationFocus::Administration,
                    } if character_id == heir_id
                ) && candidate.description.contains("succession preparation")
            })
            .unwrap_or_else(|| {
                panic!(
                    "succession pressure must produce a concrete heir-preparation education candidate: {candidates:#?}"
                )
            });
        apply_player_command(registry, &mut state, succession.command.clone())
            .expect("succession-preparation education must be executable");
    }

    #[test]
    fn established_dynasty_targets_new_offices_that_fit_its_power_strategy() {
        let mut state = make_test_campaign();
        let controlled_powers = BTreeSet::new();
        let aligned = state
            .institutions
            .values()
            .find(|institution| {
                institution
                    .powers
                    .iter()
                    .any(|power| office_power_persona_bonus(GameplayPersona::Steward, *power) > 0)
            })
            .expect("campaign must contain a steward-aligned institution");
        let unaligned = state
            .institutions
            .values()
            .find(|institution| {
                institution
                    .powers
                    .iter()
                    .all(|power| office_power_persona_bonus(GameplayPersona::Steward, *power) == 0)
            })
            .expect("campaign must contain an institution outside the steward power strategy");

        assert!(institution_is_strategic_target(
            &state,
            aligned,
            &controlled_powers,
            true,
            GameplayPersona::Steward,
        ));
        assert!(!institution_is_strategic_target(
            &state,
            unaligned,
            &controlled_powers,
            true,
            GameplayPersona::Steward,
        ));
        assert!(
            institution_is_strategic_target(
                &state,
                unaligned,
                &controlled_powers,
                false,
                GameplayPersona::Steward,
            ),
            "before winning an office the dynasty may use any viable institution as its first foothold"
        );

        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 0;
        assert_eq!(
            institution_support_recovery_bonus(&state, true, GameplayPersona::Steward),
            AGENT_POLITICAL_RECOVERY_SUPPORT_BONUS
        );
        assert!(
            institution_is_strategic_target(
                &state,
                unaligned,
                &controlled_powers,
                true,
                GameplayPersona::Steward,
            ),
            "a politically stranded dynasty must consider new patronage as a legitimacy recovery route even when the institution does not add a preferred office power"
        );
    }

    #[test]
    fn illiquid_officeholder_is_offered_executable_institutional_withdrawal() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        make_institution_withdrawal_available_for_test(&mut state);
        assert!(has_institution_withdrawal_opportunity(&state));
        let mut accumulator = CampaignAccumulator::new();
        let generated_kinds =
            ranked_candidates(registry, &state, GameplayPersona::Steward, &accumulator).1;
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut accumulator,
            &generated_kinds,
        );
        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::WithdrawFromInstitution)
                .expect("withdrawal statistics must exist")
                .activation_opportunities,
            1
        );
        let mut candidates = Vec::new();

        generate_family_candidates(registry, &state, GameplayPersona::Steward, &mut candidates);

        let candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::WithdrawFromInstitution)
            .cloned()
            .unwrap_or_else(|| {
                panic!("expected institution-withdrawal candidate; observed: {candidates:#?}")
            });
        let (institution_id, character_id) = match &candidate.command {
            PlayerCommand::WithdrawFromInstitution {
                institution_id,
                character_id,
            } => (*institution_id, *character_id),
            _ => unreachable!("withdrawal kind must contain an institution withdrawal"),
        };

        apply_player_command(registry, &mut state, candidate.command)
            .expect("generated institution withdrawal must be executable");

        let institution = state
            .institutions
            .get(&institution_id)
            .expect("institution must remain present");
        assert_eq!(institution.office_holder_id, None);
        assert!(!institution.members.contains(&character_id));
    }

    #[test]
    fn debt_service_makes_office_retreat_available_before_office_cost_alone_would() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        let lender_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != player_id)
            .expect("campaign must contain a rival lender");
        issue_loan(
            &mut state,
            LoanTerms {
                lender_dynasty_id: lender_id,
                borrower_dynasty_id: player_id,
                principal: Money::from_copper(8_000),
                weekly_payment: Money::from_copper(500),
                interest_basis_points: 500,
                collateral_property_id: None,
            },
        )
        .expect("fixture loan must be issuable");
        let office_cost = player_current_office_duty_cost(&state);
        let treasury = office_cost
            .saturating_mul(6)
            .saturating_add(Money::from_copper(1_000));
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = treasury;
        assert!(
            treasury >= office_cost.saturating_mul(6),
            "fixture must stay above the former office-only withdrawal threshold"
        );
        assert_eq!(count_player_offices(&state, player_id), 1);
        assert!(!player_is_politically_overextended(&state));

        assert!(
            has_institution_withdrawal_opportunity(&state),
            "loan service should make surrendering an office a viable retreat before office costs alone exhaust liquidity"
        );
        let mut candidates = Vec::new();
        generate_family_candidates(registry, &state, GameplayPersona::Steward, &mut candidates);
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::WithdrawFromInstitution),
            "financial overextension must surface an executable resignation candidate"
        );
    }

    #[test]
    fn defaulted_debt_does_not_count_as_committed_office_service() {
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        let lender_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != player_id)
            .expect("campaign must contain a rival lender");
        let loan_id = issue_loan(
            &mut state,
            LoanTerms {
                lender_dynasty_id: lender_id,
                borrower_dynasty_id: player_id,
                principal: Money::from_copper(8_000),
                weekly_payment: Money::from_copper(500),
                interest_basis_points: 500,
                collateral_property_id: None,
            },
        )
        .expect("fixture loan must be issuable");
        let loan = state
            .loans
            .get_mut(&loan_id)
            .expect("fixture loan must exist");
        loan.status = LoanStatus::Defaulted;
        loan.missed_payments = 3;

        assert_eq!(
            player_monthly_committed_duty_cost(&state),
            player_current_office_duty_cost(&state),
            "defaulted loans no longer participate in scheduled repayment and must not inflate the office-retreat reserve"
        );
    }

    #[test]
    fn office_retreat_uses_the_same_forward_reserve_as_other_office_decisions() {
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        let reserve = player_committed_duty_reserve(&state);
        assert_eq!(reserve, player_office_duty_reserve(&state, 0));
        assert!(reserve > AGENT_OFFICE_LIQUIDITY_BUFFER);

        // The activation predicate mirrors the canonical withdrawal route, which
        // accepts any player character who is an institution member; the reserve
        // is the agent's retreat policy in the candidate generator. Meeting the
        // reserve must keep the *candidate* list retreat-free while the world
        // still accepts a withdrawal.
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = reserve;
        let mut candidates = Vec::new();
        generate_family_candidates(
            rivergate_registry_for_test(),
            &state,
            GameplayPersona::Steward,
            &mut candidates,
        );
        assert!(
            !candidates
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::WithdrawFromInstitution),
            "meeting the full forward reserve should not offer an office retreat"
        );

        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = reserve.saturating_sub(Money::from_copper(1));
        let mut candidates = Vec::new();
        generate_family_candidates(
            rivergate_registry_for_test(),
            &state,
            GameplayPersona::Steward,
            &mut candidates,
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::WithdrawFromInstitution),
            "falling below the same reserve used to block discretionary spending should surface a retreat option"
        );
    }

    #[test]
    fn pending_office_campaigns_are_reserved_as_a_portfolio() {
        let mut state = make_test_campaign();
        let player = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist");
        let character_id = player.head_id();
        let institution_ids: Vec<_> = state.institutions.keys().copied().take(2).collect();
        assert_eq!(
            institution_ids.len(),
            2,
            "campaign must contain two institutions"
        );
        let base_reserve = player_office_duty_reserve(&state, 0);

        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeNomination,
            subject: format!(
                "institution:{}:character:{}",
                institution_ids[0], character_id
            )
            .into(),
            detail: "campaign_cost=300".to_owned(),
        });
        let one_campaign_reserve = player_office_duty_reserve(&state, 0);

        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::OfficeNomination,
            subject: format!(
                "institution:{}:character:{}",
                institution_ids[1], character_id
            )
            .into(),
            detail: "campaign_cost=300".to_owned(),
        });
        let two_campaign_reserve = player_office_duty_reserve(&state, 0);

        assert!(one_campaign_reserve > base_reserve);
        assert!(
            two_campaign_reserve > one_campaign_reserve,
            "a second unresolved campaign must reserve for the larger possible office portfolio"
        );
    }

    #[test]
    fn voluntary_withdrawal_creates_a_real_reentry_recovery_period() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        let player = state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist");
        player.resources.reputation_quality_basis_points =
            INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT;
        player.resources.reputation_reliability_basis_points =
            INSTITUTION_SUPPORT_REPUTATION_REQUIREMENT;
        player.resources.treasury = Money::from_copper(100_000);
        let (institution_id, character_id) = state
            .institutions
            .values()
            .find_map(|institution| {
                institution.office_holder_id.and_then(|character_id| {
                    state.characters.get(character_id).and_then(|character| {
                        (character.dynasty_id() == state.player_dynasty_id)
                            .then_some((institution.institution_id, character_id))
                    })
                })
            })
            .expect("coverage fixture must contain a player officeholder");
        let required_deliveries = institution_support_delivery_requirement(
            registry,
            &state,
            institution_id,
            character_id,
        );
        grant_player_contract_deliveries_for_test(&mut state, required_deliveries);
        let withdrawal_day = state.clock.day();

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::WithdrawFromInstitution {
                institution_id,
                character_id,
            },
        )
        .expect("voluntary withdrawal must succeed");

        let recovery_day =
            withdrawal_day.saturating_add(crate::systems::INSTITUTION_WITHDRAWAL_RECOVERY_DAYS);
        assert_eq!(
            institution_support_next_day(&state, institution_id, character_id),
            Some(recovery_day)
        );
        let alternate_character_id = state
            .characters
            .iter()
            .find(|character| {
                character.id() != character_id
                    && character.dynasty_id() == state.player_dynasty_id
                    && character.status() == CharacterStatus::Active
            })
            .map(crate::core::Character::id)
            .expect("campaign must contain another active family member");
        assert_eq!(
            institution_support_next_day(&state, institution_id, alternate_character_id),
            Some(recovery_day),
            "an office resignation must create dynasty-wide political recovery rather than inviting an immediate family-member swap"
        );
        assert_eq!(
            office_nomination_next_day(&state, alternate_character_id),
            Some(recovery_day),
            "existing support in another institution must not bypass the dynasty-wide recovery period"
        );
        let error = apply_player_command(
            registry,
            &mut state,
            PlayerCommand::CultivateInstitutionSupport {
                institution_id,
                character_id,
            },
        )
        .expect_err("a resigned character must not immediately buy back institutional support");
        assert_eq!(
            error,
            CommandError::InstitutionSupportCooldown {
                next_support_day: recovery_day
            }
        );
    }

    #[test]
    fn legitimacy_exhausted_multi_office_dynasty_can_reduce_political_overextension() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let holder_id = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        let second_institution = state
            .institutions
            .values_mut()
            .find(|institution| institution.office_holder_id.is_none())
            .expect("campaign must contain a second available institution");
        second_institution.members.insert(holder_id);
        second_institution.office_holder_id = Some(holder_id);
        {
            let player = state
                .dynasties
                .get_mut(&state.player_dynasty_id)
                .expect("player dynasty must exist");
            player.resources.treasury = Money::from_copper(100_000);
            player.resources.legitimacy_basis_points = 0;
        }

        assert_eq!(count_player_offices(&state, state.player_dynasty_id), 2);
        assert!(player_is_politically_overextended(&state));
        assert!(
            has_institution_withdrawal_opportunity(&state),
            "a multi-office dynasty with no legitimacy should be able to shed an unusable office before liquidity collapses"
        );

        let mut candidates = Vec::new();
        generate_family_candidates(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut candidates,
        );
        let candidate = candidates
            .into_iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::WithdrawFromInstitution)
            .expect("political overextension should produce an institutional-withdrawal candidate");

        apply_player_command(registry, &mut state, candidate.command)
            .expect("political-overextension withdrawal must be executable");
        assert_eq!(count_player_offices(&state, state.player_dynasty_id), 1);
        assert!(!player_is_politically_overextended(&state));
    }

    #[test]
    fn distressed_asset_owner_is_offered_executable_property_liquidation() {
        let registry = rivergate_registry_for_test();
        let healthy_state = make_test_campaign();
        // The activation predicate mirrors the canonical sale route, which
        // accepts any owned property with a solvent buyer even for a healthy
        // dynasty. The agent's *generator* reserves liquidation candidates for
        // distress and repositioning windows; the finding layer treats that
        // restraint as a policy-gated route (Warning) rather than a game gap
        // (Critical).
        let healthy_candidates = {
            let accumulator = CampaignAccumulator::new();
            ranked_candidates(
                registry,
                &healthy_state,
                GameplayPersona::Steward,
                &accumulator,
            )
            .0
        };
        assert!(
            !healthy_candidates
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::SellProperty),
            "a healthy campaign must not offer a liquidation candidate to the agent"
        );
        let mut state = make_test_campaign();
        make_property_liquidation_available_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        assert!(has_property_liquidation_opportunity(registry, &state));
        let mut accumulator = CampaignAccumulator::new();
        let generated_kinds =
            ranked_candidates(registry, &state, GameplayPersona::Steward, &accumulator).1;
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut accumulator,
            &generated_kinds,
        );
        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::SellProperty)
                .expect("sell-property statistics must exist")
                .activation_opportunities,
            1
        );
        let mut candidates = Vec::new();

        generate_finance_candidates(registry, &state, GameplayPersona::Steward, &mut candidates);

        let candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::SellProperty)
            .cloned()
            .unwrap_or_else(|| {
                panic!("expected sell-property candidate; observed candidates: {candidates:#?}")
            });
        let sold_property_id = match &candidate.command {
            PlayerCommand::SellProperty { property_id, .. } => *property_id,
            _ => unreachable!("sell-property kind must contain a property sale command"),
        };
        let treasury_before = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .treasury();

        apply_player_command(registry, &mut state, candidate.command)
            .expect("generated liquidation candidate must be executable");

        assert!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .treasury()
                > treasury_before,
            "liquidation must create recovery liquidity"
        );
        assert_ne!(
            state
                .properties
                .get(&sold_property_id)
                .expect("property must remain present")
                .owner_dynasty_id,
            Some(player_id),
            "liquidation must transfer ownership"
        );
    }

    #[test]
    fn property_liquidation_opportunity_requires_an_executable_counterparty_reserve() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        make_property_liquidation_available_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        for dynasty in state
            .dynasties
            .values_mut()
            .filter(|dynasty| dynasty.id() != player_id)
        {
            dynasty.resources.treasury = PROPERTY_COUNTERPARTY_BUYER_RESERVE;
        }
        let property_id = state
            .properties
            .values()
            .find(|property| {
                property.owner_dynasty_id == Some(player_id)
                    && property.collateral_loan_id.is_none()
            })
            .expect("player must own a liquidatable property")
            .id;
        assert!(
            state
                .dynasties
                .keys()
                .copied()
                .filter(|dynasty_id| *dynasty_id != player_id)
                .any(|buyer_id| quote_property_liquidation(
                    registry,
                    &state,
                    player_id,
                    buyer_id,
                    property_id,
                )
                .is_ok()),
            "the fixture must retain a raw liquidation quote so the reserve check is material"
        );
        // With buyers holding exactly the reserve, every discretionary purchase
        // would end below it, and an unfunded civic treasury rules out the
        // guaranteed-auction exemption.
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("registry must define a treasury");
        state
            .institutions
            .get_mut(&treasury_id)
            .expect("treasury runtime must exist")
            .budget = Money::ZERO;

        assert!(
            !has_property_liquidation_opportunity(registry, &state),
            "an opportunity must not be reported when every buyer would violate its reserve and no auction guarantee is available"
        );
        let mut candidates = Vec::new();
        generate_finance_candidates(registry, &state, GameplayPersona::Steward, &mut candidates);
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.kind != GameplayCommandKind::SellProperty),
            "candidate generation and opportunity accounting must agree"
        );

        // A funded civic treasury enables the guaranteed auction, where the
        // buyer commits its entire treasury by construction and the reserve
        // does not apply; the agent must surface exactly what commits.
        state
            .institutions
            .get_mut(&treasury_id)
            .expect("treasury runtime must exist")
            .budget = Money::from_copper(10_000_000);
        assert!(
            has_property_liquidation_opportunity(registry, &state),
            "a civic-guaranteed auction must count as an executable opportunity"
        );
        candidates.clear();
        generate_finance_candidates(registry, &state, GameplayPersona::Steward, &mut candidates);
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::SellProperty),
            "civic-guaranteed sales must be proposed as candidates"
        );
    }

    #[test]
    fn committed_office_and_debt_costs_surface_property_liquidation_before_emergency_cash() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        let lender_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != player_id)
            .expect("campaign must contain a rival lender");
        issue_loan(
            &mut state,
            LoanTerms {
                lender_dynasty_id: lender_id,
                borrower_dynasty_id: player_id,
                principal: Money::from_copper(8_000),
                weekly_payment: Money::from_copper(500),
                interest_basis_points: 500,
                collateral_property_id: None,
            },
        )
        .expect("fixture loan must be issuable");
        let two_month_obligations = projected_dynasty_monthly_office_duty(&state, player_id, 0)
            .saturating_mul(2)
            .saturating_add(Money::from_copper(500).saturating_mul(8));
        let treasury = Money::from_copper(2_000)
            .saturating_add(two_month_obligations)
            .saturating_sub(Money::from_copper(1));
        assert!(
            treasury > Money::from_copper(2_000),
            "fixture must remain above the previous emergency-only liquidation threshold"
        );
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = treasury;

        assert!(
            has_property_liquidation_opportunity(registry, &state),
            "near-term committed obligations should make an owned asset liquidatable before emergency cash levels"
        );
        let mut candidates = Vec::new();
        generate_finance_candidates(registry, &state, GameplayPersona::Steward, &mut candidates);
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::SellProperty),
            "the early recovery opportunity must produce an executable property-sale candidate"
        );
    }

    #[test]
    fn defaulted_debt_does_not_trigger_property_liquidation_for_future_service() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let lender_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != player_id)
            .expect("campaign must contain a rival lender");
        let loan_id = issue_loan(
            &mut state,
            LoanTerms {
                lender_dynasty_id: lender_id,
                borrower_dynasty_id: player_id,
                principal: Money::from_copper(8_000),
                weekly_payment: Money::from_copper(500),
                interest_basis_points: 500,
                collateral_property_id: None,
            },
        )
        .expect("fixture loan must be issuable");
        let loan = state
            .loans
            .get_mut(&loan_id)
            .expect("fixture loan must exist");
        loan.status = LoanStatus::Defaulted;
        loan.missed_payments = 3;
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(2_500);

        assert!(
            !player_needs_property_liquidation(&state),
            "defaulted debt must not cause asset sales solely to reserve cash for payments that are no longer scheduled"
        );
    }

    #[test]
    fn succession_pressure_preserves_persona_specific_governance_tradeoffs() {
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .runtime
            .succession_risk_basis_points = 3_000;

        assert_eq!(
            preferred_house_governance(&state, GameplayPersona::Steward),
            Some(HouseGovernance::Primogeniture)
        );
        assert_eq!(
            preferred_house_governance(&state, GameplayPersona::PowerBroker),
            Some(HouseGovernance::Primogeniture)
        );
        assert_eq!(
            preferred_house_governance(&state, GameplayPersona::Entrepreneur),
            Some(HouseGovernance::FamilyPartnership)
        );
        assert_eq!(
            preferred_house_governance(&state, GameplayPersona::Opportunist),
            Some(HouseGovernance::HeadCommand)
        );
    }

    #[test]
    fn asset_rich_cash_poor_dynasty_is_offered_property_liquidation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(100);
        let additional_property = state
            .properties
            .values_mut()
            .find(|property| {
                property.owner_dynasty_id.is_none() && property.collateral_loan_id.is_none()
            })
            .expect("fixture must contain an unowned property");
        additional_property.owner_dynasty_id = Some(player_id);
        for business in state
            .businesses
            .iter_mut()
            .filter(|business| business.owner_dynasty_id() == player_id)
        {
            business.operations.status = BusinessStatus::Active;
            business.operations.condition_basis_points = 9_000;
            business.finance.cash = Money::from_copper(20_000);
        }
        for dynasty in state
            .dynasties
            .values_mut()
            .filter(|dynasty| dynasty.id() != player_id)
        {
            dynasty.resources.treasury = Money::from_copper(1_000_000);
        }

        assert!(
            has_property_liquidation_opportunity(registry, &state),
            "healthy assets must still provide emergency liquidity when treasury cash is exhausted"
        );
        let mut candidates = Vec::new();
        generate_finance_candidates(registry, &state, GameplayPersona::Steward, &mut candidates);
        let candidate = candidates
            .into_iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::SellProperty)
            .expect("cash-poor asset owners must receive an executable liquidation route");
        let treasury_before = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .treasury();

        apply_player_command(registry, &mut state, candidate.command)
            .expect("generated emergency liquidation must be executable");

        assert!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .treasury()
                > treasury_before
        );
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "this test verifies the complete restructuring command contract end to end"
    )]
    fn player_lender_can_offer_an_aged_default_restructuring() {
        let registry = rivergate_registry_for_test();
        let (mut state, borrower_id, loan_id, principal_before, balance_before) =
            make_aged_player_lender_default();
        let player_id = state.player_dynasty_id;
        let loan_count = state.loans.len();
        let player_before = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .treasury();
        assert!(has_extend_credit_opportunity(
            registry,
            &state,
            GameplayPersona::PowerBroker
        ));
        let mut accumulator = CampaignAccumulator::new();
        let generated_kinds =
            ranked_candidates(registry, &state, GameplayPersona::PowerBroker, &accumulator).1;
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut accumulator,
            &generated_kinds,
        );
        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::ExtendCredit)
                .expect("extend-credit statistics must exist")
                .activation_opportunities,
            1
        );
        let mut candidates = Vec::new();

        generate_finance_candidates(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut candidates,
        );

        let candidate = candidates
            .iter()
            .find(|candidate| {
                candidate.kind == GameplayCommandKind::ExtendCredit
                    && matches!(
                        candidate.command,
                        PlayerCommand::IssueLoan {
                            terms: LoanTerms {
                                lender_dynasty_id,
                                borrower_dynasty_id,
                                ..
                            }
                        } if lender_dynasty_id == player_id && borrower_dynasty_id == borrower_id
                    )
            })
            .cloned()
            .unwrap_or_else(|| {
                panic!("expected lender restructuring candidate; observed: {candidates:#?}")
            });
        assert!(candidate.description.contains("restructure defaulted loan"));
        let advance = match &candidate.command {
            PlayerCommand::IssueLoan { terms } => terms.principal,
            _ => unreachable!("extend-credit candidate must issue a loan"),
        };

        apply_player_command(registry, &mut state, candidate.command)
            .expect("generated lender restructuring must be executable");

        assert_eq!(
            state.loans.len(),
            loan_count,
            "restructuring must reuse the loan record"
        );
        let loan = state.loans.get(&loan_id).expect("loan must remain present");
        assert_eq!(loan.status, LoanStatus::Restructured);
        assert_eq!(
            loan.principal,
            principal_before
                .checked_add(advance)
                .expect("test principal must fit")
        );
        assert_eq!(
            loan.balance,
            balance_before
                .checked_add(advance)
                .expect("test balance must fit")
        );
        assert_eq!(
            state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist")
                .treasury(),
            player_before.saturating_sub(advance)
        );
        assert_eq!(
            state
                .dynasties
                .get(&borrower_id)
                .expect("borrower must exist")
                .treasury(),
            advance
        );
    }

    #[test]
    fn policy_candidates_offer_one_contextual_strategy_instead_of_policy_cycling() {
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("player business must exist");
        business.operations.condition_basis_points = 4_000;
        business.finance.cash = Money::from_copper(20_000);
        let mut candidates = Vec::new();

        generate_business_policy_candidates(
            &state,
            GameplayPersona::Entrepreneur,
            state
                .businesses
                .get(business_id)
                .expect("player business must exist"),
            &mut candidates,
        );

        let candidate = single_candidate(&candidates, "policy generation");
        assert!(matches!(
            candidate.command,
            PlayerCommand::SetBusinessPolicy {
                maintenance_basis_points: 1_300,
                minimum_cash_reserve,
                ..
            } if minimum_cash_reserve == Money::from_copper(8_000)
        ));
    }

    #[test]
    fn opportunist_growth_policy_accepts_maintenance_exposure() {
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("player business must exist");
        business.operations.condition_basis_points = 9_000;
        business.finance.cash = Money::from_copper(20_000);
        let mut candidates = Vec::new();

        generate_business_policy_candidates(
            &state,
            GameplayPersona::Opportunist,
            state
                .businesses
                .get(business_id)
                .expect("player business must exist"),
            &mut candidates,
        );

        let candidate = single_candidate(&candidates, "opportunist growth policy");
        assert!(matches!(
            candidate.command,
            PlayerCommand::SetBusinessPolicy {
                maintenance_basis_points: 400,
                minimum_cash_reserve,
                ..
            } if minimum_cash_reserve == Money::from_copper(1_000)
        ));
    }

    #[test]
    fn healthy_businesses_offer_bounded_annual_modernization() {
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(100_000);
        let mut candidates = Vec::new();

        generate_planned_business_investment(
            &state,
            GameplayPersona::Entrepreneur,
            state
                .businesses
                .get(business_id)
                .expect("player business must exist"),
            &mut candidates,
        );

        assert!(
            candidates.is_empty(),
            "a new enterprise should establish a strategy and produce trade evidence before generic modernization becomes a diagnostic priority"
        );
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("player business must exist");
            business.finance.lifetime_revenue = Money::from_copper(8_000);
            business.finance.lifetime_costs = Money::from_copper(6_000);
        }
        generate_planned_business_investment(
            &state,
            GameplayPersona::Entrepreneur,
            state
                .businesses
                .get(business_id)
                .expect("player business must exist"),
            &mut candidates,
        );

        let candidate = single_candidate(&candidates, "planned modernization");
        assert!(matches!(
            candidate.command,
            PlayerCommand::InvestInBusiness { amount, .. }
                if amount >= Money::from_copper(1_000)
                    && amount <= AGENT_PLANNED_CAPITALIZATION_MAX
        ));

        for persona in [
            GameplayPersona::Steward,
            GameplayPersona::PowerBroker,
            GameplayPersona::Opportunist,
        ] {
            let mut other_candidates = Vec::new();
            generate_planned_business_investment(
                &state,
                persona,
                state
                    .businesses
                    .get(business_id)
                    .expect("player business must exist"),
                &mut other_candidates,
            );
            // Every persona considers healthy modernization; persona weights,
            // not a hard gate, decide how often each one pursues it.
            assert!(
                !other_candidates.is_empty(),
                "healthy generic modernization must stay reachable for every persona so the commercial loop is measurable"
            );
        }

        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::BusinessCapitalization,
            subject: format!("business:{business_id}").into(),
            detail: "amount=6000;rehabilitation_basis_points=3000".to_owned(),
        });
        candidates.clear();
        generate_planned_business_investment(
            &state,
            GameplayPersona::Entrepreneur,
            state
                .businesses
                .get(business_id)
                .expect("player business must exist"),
            &mut candidates,
        );

        assert!(candidates.is_empty());
    }

    #[test]
    fn supply_renewal_scales_after_the_business_proves_itself() {
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("player business must exist");
        business.finance.lifetime_revenue = Money::from_copper(8_000);
        business.finance.lifetime_costs = Money::from_copper(10_000);

        assert_eq!(secure_supply_batches(business), 1);

        business.finance.lifetime_revenue = Money::from_copper(12_000);

        assert_eq!(
            secure_supply_batches(business),
            STANDARD_CONTRACT_BATCHES_PER_WEEK
        );
    }

    #[test]
    fn cash_rebalancing_requires_a_real_liquidity_shortfall() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        add_second_player_business(&mut state);
        let business_ids: Vec<_> = state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .expect("player businesses must exist")
            .iter()
            .copied()
            .collect();
        for business_id in &business_ids {
            let business = state
                .businesses
                .get_mut(*business_id)
                .expect("player business must exist");
            business.finance.cash = Money::from_copper(10_000);
            business.policy.minimum_cash_reserve = Money::from_copper(500);
            business.operations.status = BusinessStatus::Active;
        }
        let businesses: Vec<_> = business_ids
            .iter()
            .filter_map(|business_id| state.businesses.get(*business_id))
            .collect();
        let mut candidates = Vec::new();
        generate_cash_rebalance_candidate(registry, &state, &businesses, &mut candidates);
        assert!(candidates.is_empty());

        let [_, target_id] = business_ids.as_slice() else {
            panic!("fixture must contain exactly two player businesses: {business_ids:?}");
        };
        let target = state
            .businesses
            .get_mut(*target_id)
            .expect("target business must exist");
        target.finance.cash = Money::ZERO;
        target.operations.status = BusinessStatus::Distressed;
        let businesses: Vec<_> = business_ids
            .iter()
            .filter_map(|business_id| state.businesses.get(*business_id))
            .collect();
        generate_cash_rebalance_candidate(registry, &state, &businesses, &mut candidates);

        let candidate = single_candidate(&candidates, "cash rebalancing");
        let PlayerCommand::TransferBusinessCash {
            from_business_id,
            to_business_id,
            amount,
        } = candidate.command
        else {
            panic!("liquidity shortfall must produce a cash-transfer candidate");
        };
        let source = state
            .businesses
            .get(from_business_id)
            .expect("candidate source must exist");
        let target = state
            .businesses
            .get(to_business_id)
            .expect("candidate target must exist");
        let source_surplus = source
            .cash()
            .saturating_sub(business_cash_target(registry, &state, source));
        let buffered_deficit = business_cash_target(registry, &state, target)
            .saturating_add(AGENT_CASH_REBALANCE_BUFFER)
            .saturating_sub(target.cash());
        assert_eq!(amount, source_surplus.min(buffered_deficit));
        assert!(amount >= AGENT_CASH_REBALANCE_TRIGGER);
    }

    #[test]
    fn cash_rebalancing_waits_between_portfolio_interventions() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        add_second_player_business(&mut state);
        let business_ids: Vec<_> = state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .expect("player businesses must exist")
            .iter()
            .copied()
            .collect();
        for business_id in &business_ids {
            let business = state
                .businesses
                .get_mut(*business_id)
                .expect("player business must exist");
            business.finance.cash = Money::from_copper(20_000);
            business.policy.minimum_cash_reserve = Money::from_copper(500);
            business.operations.status = BusinessStatus::Active;
        }
        let [source_id, target_id] = business_ids.as_slice() else {
            panic!("fixture must contain exactly two player businesses: {business_ids:?}");
        };
        let target = state
            .businesses
            .get_mut(*target_id)
            .expect("target business must exist");
        target.finance.cash = Money::ZERO;
        target.operations.status = BusinessStatus::Distressed;
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::CashTransfer,
            subject: format!("business:{source_id}->business:{target_id}").into(),
            detail: "amount=1000".to_owned(),
        });
        let businesses: Vec<_> = business_ids
            .iter()
            .filter_map(|business_id| state.businesses.get(*business_id))
            .collect();
        let mut candidates = Vec::new();

        generate_cash_rebalance_candidate(registry, &state, &businesses, &mut candidates);

        assert!(candidates.is_empty());

        for _ in 0..AGENT_CASH_REBALANCE_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        let businesses: Vec<_> = business_ids
            .iter()
            .filter_map(|business_id| state.businesses.get(*business_id))
            .collect();
        generate_cash_rebalance_candidate(registry, &state, &businesses, &mut candidates);

        single_candidate(&candidates, "cash rebalancing after cooldown");
    }

    #[test]
    fn cash_poor_dynasty_can_distribute_only_safe_business_surplus() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::ZERO;
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let reserve = business_owner_distribution_reserve(
            registry,
            state
                .businesses
                .get(business_id)
                .expect("owned business must exist"),
        );
        let safe_surplus = Money::from_copper(1_200);
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("owned business must exist");
            business.operations.status = BusinessStatus::Active;
            business.finance.cash = reserve.saturating_add(safe_surplus);
        }
        let businesses = vec![
            state
                .businesses
                .get(business_id)
                .expect("owned business must exist"),
        ];
        let mut candidates = Vec::new();

        generate_owner_distribution_candidate(
            registry,
            &state,
            GameplayPersona::Opportunist,
            &businesses,
            &mut candidates,
        );

        let candidate = single_candidate(&candidates, "owner distribution");
        assert_eq!(candidate.kind, GameplayCommandKind::WithdrawBusinessCash);
        let PlayerCommand::WithdrawBusinessCash {
            business_id: candidate_business_id,
            amount,
        } = candidate.command
        else {
            panic!("cash-poor dynasty must receive an owner-distribution candidate");
        };
        assert_eq!(candidate_business_id, business_id);
        assert_eq!(amount, safe_surplus);
        assert!(amount >= AGENT_OWNER_DISTRIBUTION_TRIGGER);

        let mut accumulator = CampaignAccumulator::new();
        let generated_kinds =
            ranked_candidates(registry, &state, GameplayPersona::Opportunist, &accumulator).1;
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Opportunist,
            &mut accumulator,
            &generated_kinds,
        );
        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::WithdrawBusinessCash)
                .expect("cash-withdrawal stats must exist")
                .activation_opportunities,
            1,
            "activation metrics must include safe owner distributions"
        );
    }

    #[test]
    fn pending_legal_settlement_unlocks_emergency_business_liquidity() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        make_legal_settlement_available_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        let requirement = active_legal_settlement_requirement(&state)
            .expect("grounded defendant case must expose a settlement requirement");
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::ZERO;
        let business_id = *state
            .businesses
            .ids_for_owner(player_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let reserve = business_owner_distribution_reserve(
            registry,
            state
                .businesses
                .get(business_id)
                .expect("owned business must exist"),
        );
        state
            .businesses
            .get_mut(business_id)
            .expect("owned business must exist")
            .finance
            .cash = reserve.saturating_add(requirement);
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::BusinessDividend,
            subject: format!("business:{business_id}").into(),
            detail: "owner_distribution=500".to_owned(),
        });
        let businesses = vec![
            state
                .businesses
                .get(business_id)
                .expect("owned business must exist"),
        ];
        let mut candidates = Vec::new();

        generate_owner_distribution_candidate(
            registry,
            &state,
            GameplayPersona::Steward,
            &businesses,
            &mut candidates,
        );

        let candidate = single_candidate(&candidates, "settlement liquidity distribution");
        assert!(matches!(
            candidate.command,
            PlayerCommand::WithdrawBusinessCash { amount, .. } if amount == requirement
        ));
        assert!(candidate.score >= 3_900);
    }

    #[test]
    fn legal_funding_with_existing_treasury_withdraws_only_the_remaining_shortfall() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        make_legal_settlement_available_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        let treasury = Money::from_copper(1_000);
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = treasury;
        let funding_target = legal_settlement_funding_target(&state)
            .expect("grounded case must create a legal funding target");
        let expected_shortfall = funding_target.saturating_sub(treasury);
        assert!(
            expected_shortfall >= AGENT_STRATEGIC_WITHDRAWAL_TRIGGER,
            "fixture must require a strategic withdrawal"
        );
        let business_id = *state
            .businesses
            .ids_for_owner(player_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let reserve = business_owner_distribution_reserve(
            registry,
            state
                .businesses
                .get(business_id)
                .expect("owned business must exist"),
        );
        state
            .businesses
            .get_mut(business_id)
            .expect("owned business must exist")
            .finance
            .cash = reserve.saturating_add(expected_shortfall);
        let businesses = vec![
            state
                .businesses
                .get(business_id)
                .expect("owned business must exist"),
        ];
        let mut candidates = Vec::new();

        generate_owner_distribution_candidate(
            registry,
            &state,
            GameplayPersona::Steward,
            &businesses,
            &mut candidates,
        );

        let candidate = single_candidate(&candidates, "remaining legal funding shortfall");
        assert!(matches!(
            candidate.command,
            PlayerCommand::WithdrawBusinessCash { amount, .. } if amount == expected_shortfall
        ));
    }

    #[test]
    fn pending_legal_settlement_reserve_blocks_discretionary_spending() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        make_legal_settlement_available_for_test(&mut state);
        let requirement = active_legal_settlement_requirement(&state)
            .expect("grounded defendant case must expose a settlement requirement");
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = requirement;
        let discretionary = Candidate {
            kind: GameplayCommandKind::ConveneFamilyCouncil,
            command: PlayerCommand::ConveneFamilyCouncil,
            description: "discretionary family meeting".to_owned(),
            score: 0,
        };
        let case_id = state
            .legal_cases
            .values()
            .find_map(|legal_case| {
                quote_player_legal_settlement(&state, legal_case.id)
                    .ok()
                    .map(|quote| quote.case_id)
            })
            .expect("grounded case must remain settleable");
        let settlement = Candidate {
            kind: GameplayCommandKind::SettleLegalCase,
            command: PlayerCommand::SettleLegalCase { case_id },
            description: "settle grounded case".to_owned(),
            score: 0,
        };

        assert!(!candidate_preserves_office_duty_reserve(
            registry,
            &state,
            &discretionary
        ));
        assert!(candidate_preserves_office_duty_reserve(
            registry,
            &state,
            &settlement
        ));
    }

    #[test]
    fn world_state_predicates_reveal_activation_for_non_reactive_families() {
        // A campaign that can afford family education and buy a qualifying
        // property must register activation opportunities for those families
        // even though they have no dedicated reactive predicate -- so a quiet
        // cycle is diagnosed as a generator gap, not as a dormant world.
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(200_000);
        let mut accumulator = CampaignAccumulator::new();
        let generated = BTreeSet::new();

        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut accumulator,
            &generated,
        );

        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::EducateFamilyMember)
                .expect("education statistics must exist")
                .activation_opportunities,
            1,
            "the world offers family education for an affordable dynasty"
        );
        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::BuyProperty)
                .expect("buy-property statistics must exist")
                .activation_opportunities,
            1,
            "the world offers an affordable property purchase"
        );
    }

    #[test]
    fn insolvent_businesses_remain_investment_activation_opportunities() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let business_id = *state
            .businesses
            .ids_for_owner(player_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        state
            .businesses
            .get_mut(business_id)
            .expect("owned business must exist")
            .operations
            .status = BusinessStatus::Insolvent;
        state
            .businesses
            .get_mut(business_id)
            .expect("owned business must exist")
            .finance
            .cash = Money::ZERO;
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(20_000);
        let mut accumulator = CampaignAccumulator::new();

        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut accumulator,
            &BTreeSet::new(),
        );

        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::InvestInBusiness)
                .expect("investment statistics must exist")
                .activation_opportunities,
            1,
            "an insolvent player business can still be rehabilitated"
        );
    }

    #[test]
    fn property_repositioning_excludes_the_dynasty_residence() {
        let state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let residence = state
            .properties
            .values()
            .find(|property| {
                property.owner_dynasty_id == Some(player_id)
                    && property.kind == PropertyKind::Residence
            })
            .expect("campaign must start with a dynasty residence");
        assert!(
            !property_underperforms_investment_hurdle(&state, residence),
            "the dynasty residence must never be flagged for repositioning"
        );
    }

    #[test]
    fn strategic_withdrawal_capitalizes_an_endowment_commitment() {
        // A dynasty that is represented in an institution and holds business
        // surplus above the operating reserve must be offered a withdrawal to
        // capitalize an endowment, even before its family treasury is poor.
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let character_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        // Establish membership by holding office (the immediate membership
        // route, which needs no support-establishment wait).
        let institution_id = state
            .institutions
            .iter()
            .find(|(_, institution)| {
                institution.members.iter().any(|member_id| {
                    state
                        .characters
                        .get(*member_id)
                        .is_some_and(|character| character.dynasty_id() == player_id)
                })
            })
            .map(|(id, _)| *id)
            .or_else(|| state.institutions.keys().next().copied())
            .expect("campaign must contain an institution");
        // Establish membership via a matured patronage record (no office duty,
        // so the endowment is the dominant strategic treasury need).
        let supply_subject = format!("institution:{institution_id}:character:{character_id}");
        state.audit_log.push(AuditRecord {
            day: state
                .clock
                .day()
                .saturating_sub(INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS),
            kind: AuditKind::InstitutionPatronage,
            subject: supply_subject.into(),
            detail: "support_established".to_owned(),
        });
        {
            let institution = state
                .institutions
                .get_mut(&institution_id)
                .expect("institution must exist");
            if !institution.members.contains(&character_id) {
                institution.members.insert(character_id);
            }
        }
        // A modest family treasury below the endowment commitment: the dynasty
        // must pull business surplus to fund the endowment.
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(1_000);
        let business_id = *state
            .businesses
            .ids_for_owner(player_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let reserve = business_owner_distribution_reserve(
            registry,
            state
                .businesses
                .get(business_id)
                .expect("owned business must exist"),
        );
        state
            .businesses
            .get_mut(business_id)
            .expect("owned business must exist")
            .finance
            .cash = reserve.saturating_add(Money::from_copper(30_000));
        let businesses = vec![
            state
                .businesses
                .get(business_id)
                .expect("owned business must exist"),
        ];
        let mut candidates = Vec::new();

        generate_owner_distribution_candidate(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &businesses,
            &mut candidates,
        );

        let candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::WithdrawBusinessCash)
            .unwrap_or_else(|| {
                panic!(
                    "represented wealthy dynasty must be offered an endowment withdrawal: {candidates:#?}"
                )
            });
        assert!(
            candidate.description.contains("endowment"),
            "the strategic withdrawal should name its endowment purpose: {}",
            candidate.description
        );
    }

    #[test]
    fn urgent_legal_funding_shortens_the_harness_decision_window() {
        let mut state = make_test_campaign();
        make_legal_settlement_available_for_test(&mut state);
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::ZERO;
        let legal_case = state
            .legal_cases
            .values_mut()
            .find(|legal_case| legal_case.defendant_dynasty_id == state.player_dynasty_id)
            .expect("fixture must contain a player-defendant case");
        legal_case.hearing_day = state.clock.day().saturating_add(20);

        assert_eq!(
            next_campaign_step_days(&state, 30),
            10,
            "an unaffordable settlement with twenty days remaining must preserve another decision before judgment"
        );

        let requirement = active_legal_settlement_requirement(&state)
            .expect("grounded case must remain settleable");
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = requirement;
        assert_eq!(
            next_campaign_step_days(&state, 30),
            30,
            "once settlement is affordable the harness should return to its configured cadence"
        );
    }

    #[test]
    fn urgent_legal_funding_prioritizes_liquidity_over_internal_rebalancing() {
        let mut state = make_test_campaign();
        make_legal_settlement_available_for_test(&mut state);
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::ZERO;
        let player_id = state.player_dynasty_id;
        let lender_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != player_id)
            .expect("campaign must contain a lender");
        let business_id = *state
            .businesses
            .ids_for_owner(player_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        let borrow = Candidate {
            kind: GameplayCommandKind::BorrowFunds,
            command: PlayerCommand::IssueLoan {
                terms: LoanTerms {
                    lender_dynasty_id: lender_id,
                    borrower_dynasty_id: player_id,
                    principal: Money::from_copper(5_000),
                    weekly_payment: Money::from_copper(100),
                    interest_basis_points: 700,
                    collateral_property_id: None,
                },
            },
            description: "raise settlement cash".to_owned(),
            score: 0,
        };
        let rebalance = Candidate {
            kind: GameplayCommandKind::TransferBusinessCash,
            command: PlayerCommand::TransferBusinessCash {
                from_business_id: business_id,
                to_business_id: business_id,
                amount: Money::from_copper(1),
            },
            description: "internal rebalance".to_owned(),
            score: 0,
        };

        assert!(legal_funding_candidate_adjustment(&state, &borrow) > 0);
        assert!(legal_funding_candidate_adjustment(&state, &rebalance) < 0);
    }

    #[test]
    fn labor_agents_defer_disputes_until_the_business_can_fix_poor_conditions() {
        let mut state = make_test_campaign();
        let employment_id = state
            .employment
            .values()
            .find(|agreement| {
                state
                    .businesses
                    .get(agreement.business_id)
                    .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id)
            })
            .expect("player business must have employment")
            .id;
        let business_id = {
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("employment must exist");
            agreement.status = EmploymentStatus::Disputed;
            agreement.conditions_basis_points = 4_000;
            agreement.loyalty_basis_points = 1_500;
            agreement.business_id
        };
        state
            .businesses
            .get_mut(business_id)
            .expect("business must exist")
            .finance
            .cash = LABOR_REPLACEMENT_COST;
        let mut candidates = Vec::new();

        generate_reactive_candidates(&state, GameplayPersona::Entrepreneur, &mut candidates);
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.kind != GameplayCommandKind::ResolveLaborDispute),
            "no labor response may be proposed when nothing is affordable above the operating reserve"
        );

        // With the default 500-copper reserve, 1_000 cash leaves only 500
        // spendable: negotiation commits, condition improvement does not.
        state
            .businesses
            .get_mut(business_id)
            .expect("business must exist")
            .finance
            .cash = Money::from_copper(1_000);
        candidates.clear();
        generate_reactive_candidates(&state, GameplayPersona::Entrepreneur, &mut candidates);

        let labor_candidates: Vec<_> = candidates
            .iter()
            .filter(|candidate| candidate.kind == GameplayCommandKind::ResolveLaborDispute)
            .cloned()
            .collect();
        let candidate = single_candidate(&labor_candidates, "labor dispute response");
        assert!(matches!(
            candidate.command,
            PlayerCommand::ResolveLaborDispute {
                response: LaborResponse::Negotiate,
                ..
            }
        ));

        // Above the reserve floor, improving unsafe conditions takes priority.
        state
            .businesses
            .get_mut(business_id)
            .expect("business must exist")
            .finance
            .cash = Money::from_copper(1_000 + 1_000);
        candidates.clear();
        generate_reactive_candidates(&state, GameplayPersona::Entrepreneur, &mut candidates);

        let labor_candidates: Vec<_> = candidates
            .iter()
            .filter(|candidate| candidate.kind == GameplayCommandKind::ResolveLaborDispute)
            .cloned()
            .collect();
        let candidate = single_candidate(&labor_candidates, "labor dispute response");
        assert!(matches!(
            candidate.command,
            PlayerCommand::ResolveLaborDispute {
                response: LaborResponse::ImproveConditions,
                ..
            }
        ));
    }

    #[test]
    fn reactive_agents_can_contain_a_crisis_after_exploiting_it() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        add_active_crisis(&mut state);
        let crisis_id = *state
            .crises
            .keys()
            .next_back()
            .expect("test crisis must exist");
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::CrisisResponse,
            subject: format!("crisis:{crisis_id}").into(),
            detail: "response=Exploit".to_owned(),
        });
        let mut candidates = Vec::new();

        generate_reactive_candidates(&state, GameplayPersona::Opportunist, &mut candidates);

        let crisis_candidates: Vec<_> = candidates
            .iter()
            .filter(|candidate| candidate.kind == GameplayCommandKind::RespondToCrisis)
            .collect();
        assert!(!crisis_candidates.is_empty());
        assert!(crisis_candidates.iter().all(|candidate| {
            !matches!(
                candidate.command,
                PlayerCommand::RespondToCrisis {
                    response: CrisisResponse::Exploit,
                    ..
                }
            )
        }));

        let mut accumulator = CampaignAccumulator::new();
        let generated_kinds =
            ranked_candidates(registry, &state, GameplayPersona::Opportunist, &accumulator).1;
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Opportunist,
            &mut accumulator,
            &generated_kinds,
        );
        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::RespondToCrisis)
                .expect("crisis-response statistics must exist")
                .activation_opportunities,
            1,
            "an exploited but not contained crisis remains an active recovery opportunity"
        );

        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::CrisisResponse,
            subject: format!("crisis:{crisis_id}").into(),
            detail: "response=Reform".to_owned(),
        });
        candidates.clear();
        generate_reactive_candidates(&state, GameplayPersona::Opportunist, &mut candidates);
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.kind != GameplayCommandKind::RespondToCrisis)
        );

        let generated_kinds =
            ranked_candidates(registry, &state, GameplayPersona::Opportunist, &accumulator).1;
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Opportunist,
            &mut accumulator,
            &generated_kinds,
        );
        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::RespondToCrisis)
                .expect("crisis-response statistics must exist")
                .activation_opportunities,
            1,
            "a contained crisis must not create another crisis-response opportunity"
        );
    }

    #[test]
    fn acquisition_waits_until_the_existing_portfolio_is_healthy_and_funded() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        make_nonplayer_business_acquirable(&mut state);
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(100_000);
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player business must exist");
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("player business must exist");
            business.operations.condition_basis_points = 6_999;
            business.finance.cash = Money::from_copper(20_000);
        }
        let player_businesses: Vec<_> = state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .into_iter()
            .flatten()
            .filter_map(|id| state.businesses.get(*id))
            .collect();
        let mut candidates = Vec::new();

        generate_business_acquisition_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &player_businesses,
            &mut candidates,
        );
        assert!(
            candidates.is_empty(),
            "expansion must wait while an existing business still needs rehabilitation"
        );

        state
            .businesses
            .get_mut(business_id)
            .expect("player business must exist")
            .operations
            .condition_basis_points = 8_000;
        let player_businesses: Vec<_> = state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .into_iter()
            .flatten()
            .filter_map(|id| state.businesses.get(*id))
            .collect();
        generate_business_acquisition_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &player_businesses,
            &mut candidates,
        );

        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::AcquireBusiness),
            "a healthy funded portfolio must retain an expansion route"
        );
    }

    #[test]
    fn borrowing_is_generated_for_need_not_as_a_recurring_default_action() {
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(50_000);
        let mut candidates = Vec::new();

        add_borrow_candidate(&state, GameplayPersona::Opportunist, &mut candidates);
        assert!(candidates.is_empty());

        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::ZERO;
        add_borrow_candidate(&state, GameplayPersona::Opportunist, &mut candidates);
        let candidate = single_candidate(&candidates, "borrowing under liquidity pressure");
        assert!(matches!(
            candidate.command,
            PlayerCommand::IssueLoan { ref terms }
                if terms.weekly_payment
                    == terms.principal.ceil_div_positive(AGENT_LOAN_AMORTIZATION_WEEKS)
        ));
    }

    #[test]
    fn defaulted_credit_redirects_borrowing_to_another_lender() {
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::ZERO;
        let mut candidates = Vec::new();
        add_borrow_candidate(&state, GameplayPersona::Opportunist, &mut candidates);
        let first_terms = match &candidates
            .first()
            .expect("liquidity need must generate credit")
            .command
        {
            PlayerCommand::IssueLoan { terms } => terms.clone(),
            command => panic!("expected loan candidate, found {command:?}"),
        };
        let first_lender_id = first_terms.lender_dynasty_id;
        let loan_id = issue_loan(&mut state, first_terms).expect("first loan must issue");
        let collateral_id = state
            .loans
            .get(&loan_id)
            .expect("issued loan must exist")
            .collateral_property_id;
        state
            .loans
            .get_mut(&loan_id)
            .expect("issued loan must exist")
            .status = LoanStatus::Defaulted;
        if let Some(property_id) = collateral_id {
            let property = state
                .properties
                .get_mut(&property_id)
                .expect("loan collateral must exist");
            property.owner_dynasty_id = Some(first_lender_id);
            property.collateral_loan_id = None;
        }
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::ZERO;
        candidates.clear();

        add_borrow_candidate(&state, GameplayPersona::Opportunist, &mut candidates);

        let candidate = single_candidate(&candidates, "redirected borrowing after default");
        assert!(matches!(
            candidate.command,
            PlayerCommand::IssueLoan { ref terms }
                if terms.lender_dynasty_id != first_lender_id
        ));
    }

    #[test]
    fn underfunded_civic_treasury_offers_public_debt_when_credit_is_available() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let treasury_id = registry
            .get_institution_id("treasury")
            .expect("registry must define a treasury");
        state
            .institutions
            .get_mut(&treasury_id)
            .expect("treasury runtime must exist")
            .budget = Money::ZERO;
        state
            .dynasties
            .values_mut()
            .find(|dynasty| dynasty.id() != state.player_dynasty_id)
            .expect("campaign must contain a creditor dynasty")
            .resources
            .treasury = Money::from_copper(200_000);

        let candidates = law_candidates(registry, &state);

        assert!(candidates.iter().any(|(kind, value)| {
            *kind == LawKind::PublicDebtAuthorization && (10_000..=100_000).contains(value)
        }));
    }

    #[test]
    fn law_relevance_distinguishes_persona_preference_from_world_pressure() {
        let mut state = make_test_campaign();

        assert!(law_persona_bonus(GameplayPersona::Entrepreneur, LawKind::ForeignMerchantToll) > 0);
        assert!(
            law_persona_bonus(
                GameplayPersona::Entrepreneur,
                LawKind::GuildEntryRestriction
            ) < 0,
            "entrepreneur policy should not drift toward restrictive guild law merely because the office permits it"
        );
        assert_eq!(
            law_context_relevance_bonus(&state, LawKind::InterestLimit),
            0,
            "credit regulation should not become generic legislative maintenance without debt distress"
        );
        state
            .loans
            .values_mut()
            .next()
            .expect("campaign must contain a loan")
            .status = LoanStatus::Delinquent;
        assert!(
            law_context_relevance_bonus(&state, LawKind::InterestLimit) > 0,
            "actual credit distress should make an interest limit strategically relevant"
        );
    }

    #[test]
    fn opportunist_defers_debt_office_until_credit_exists() {
        let mut state = make_test_campaign();
        for loan in state.loans.values_mut() {
            loan.status = LoanStatus::Repaid;
        }
        for debt in state.civic_debts.values_mut() {
            debt.status = CivicDebtStatus::Repaid;
        }

        assert_eq!(
            office_power_ascent_bonus(
                &state,
                GameplayPersona::Opportunist,
                OfficePower::DebtEnforcement,
            ),
            0,
            "a debt-enforcement office should not be the opportunist's default first political route in a city with no active credit"
        );

        let player_id = state.player_dynasty_id;
        let lender_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != player_id)
            .expect("campaign must contain a rival lender");
        issue_loan(
            &mut state,
            LoanTerms {
                lender_dynasty_id: lender_id,
                borrower_dynasty_id: player_id,
                principal: Money::from_copper(1_000),
                weekly_payment: Money::from_copper(100),
                interest_basis_points: 500,
                collateral_property_id: None,
            },
        )
        .expect("fixture credit must be issuable");

        assert_eq!(
            office_power_ascent_bonus(
                &state,
                GameplayPersona::Opportunist,
                OfficePower::DebtEnforcement,
            ),
            office_power_persona_bonus(GameplayPersona::Opportunist, OfficePower::DebtEnforcement,),
            "once credit exists, debt enforcement becomes a coherent opportunist political target"
        );
    }

    #[test]
    fn officeholders_defer_discretionary_spending_that_consumes_term_reserves() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let reserve = player_office_duty_reserve(&state, 0);
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = reserve;
        let property_id = state
            .properties
            .values()
            .find(|property| property.owner_dynasty_id.is_none())
            .expect("campaign must contain an unowned property")
            .id;
        let candidate = Candidate {
            kind: GameplayCommandKind::BuyProperty,
            command: PlayerCommand::BuyProperty { property_id },
            description: "buy property without preserving office duties".to_owned(),
            score: 0,
        };

        assert!(!candidate_preserves_office_duty_reserve(
            registry, &state, &candidate
        ));
    }

    #[test]
    fn family_council_can_draw_into_long_term_office_reserve_without_risking_near_term_duties() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        state
            .family_councils
            .get_mut(&state.player_dynasty_id)
            .expect("player family council must exist")
            .unity_basis_points = FAMILY_COUNCIL_INTERVENTION_UNITY_THRESHOLD.saturating_sub(1);
        let full_reserve = player_office_duty_reserve(&state, 0);
        let recovery_reserve = player_family_recovery_office_duty_reserve(&state);
        assert!(recovery_reserve < full_reserve);
        let candidate = Candidate {
            kind: GameplayCommandKind::ConveneFamilyCouncil,
            command: PlayerCommand::ConveneFamilyCouncil,
            description: "reconcile a divided officeholding family".to_owned(),
            score: 0,
        };
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = FAMILY_COUNCIL_MEETING_COST.saturating_add(recovery_reserve);

        assert!(candidate_preserves_office_duty_reserve(
            registry, &state, &candidate
        ));
        assert!(
            state
                .dynasties
                .get(&state.player_dynasty_id)
                .expect("player dynasty must exist")
                .treasury()
                .saturating_sub(FAMILY_COUNCIL_MEETING_COST)
                < full_reserve,
            "the family intervention should be allowed to use part of the long-term reserve"
        );

        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = FAMILY_COUNCIL_MEETING_COST
            .saturating_add(recovery_reserve)
            .saturating_sub(Money::from_copper(1));
        assert!(!candidate_preserves_office_duty_reserve(
            registry, &state, &candidate
        ));
    }

    #[test]
    fn office_power_agent_waits_for_material_need_instead_of_using_directives_on_cooldown() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 10_000;
        let institution = state
            .institutions
            .values_mut()
            .find(|institution| institution.office_holder_id.is_some())
            .expect("fixture must grant the player an office");
        institution.powers = BTreeSet::from([OfficePower::PublicWorks]);
        let district_id = registry
            .get_institution(institution.institution_id)
            .expect("runtime institution must have a registry definition")
            .district_id();
        let district = state
            .districts
            .get_mut(&district_id)
            .expect("institution district must exist");
        district.employment_basis_points = 10_000;
        district.sanitation_basis_points = 10_000;
        let mut candidates = Vec::new();

        generate_office_power_directive_candidates(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut candidates,
        );

        assert!(candidates.is_empty());
    }

    #[test]
    fn office_power_agent_ignores_minor_deficits_and_bounds_material_need_scoring() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 10_000;
        let institution = state
            .institutions
            .values_mut()
            .find(|institution| institution.office_holder_id.is_some())
            .expect("fixture must grant the player an office");
        institution.powers = BTreeSet::from([OfficePower::PublicWorks]);
        let district_id = registry
            .get_institution(institution.institution_id)
            .expect("runtime institution must have a registry definition")
            .district_id();
        let district = state
            .districts
            .get_mut(&district_id)
            .expect("institution district must exist");
        district.employment_basis_points = 6_400;
        district.sanitation_basis_points = 6_400;
        let mut candidates = Vec::new();

        generate_office_power_directive_candidates(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut candidates,
        );

        assert!(
            candidates.is_empty(),
            "negligible district deficits should not turn office power into routine maintenance"
        );

        let district = state
            .districts
            .get_mut(&district_id)
            .expect("institution district must exist");
        // A moderate deficit is now within the agent's material-need bar:
        // directives are rationed by legitimacy cost and cooldown, so an
        // ordinary term should offer one instead of waiting for a crisis.
        district.employment_basis_points = 6_200;
        district.sanitation_basis_points = 6_200;
        generate_office_power_directive_candidates(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut candidates,
        );

        let candidate_score = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::ExerciseOfficePower)
            .expect("a visible district gap should create an office-power candidate")
            .score;
        assert!(
            candidate_score <= 1_700,
            "need scoring should stay comparable to other strategic families before global rank adjustment"
        );

        let district = state
            .districts
            .get_mut(&district_id)
            .expect("institution district must exist");
        district.employment_basis_points = 4_000;
        district.sanitation_basis_points = 4_000;
        candidates.clear();
        generate_office_power_directive_candidates(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut candidates,
        );

        let severe_candidate_score = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::ExerciseOfficePower)
            .expect("material district need should create an office-power candidate")
            .score;
        assert!(
            severe_candidate_score > candidate_score,
            "severe need should outrank moderate need: {severe_candidate_score} vs {candidate_score}"
        );
    }

    #[test]
    fn office_duty_audits_require_an_exact_dynasty_subject_segment() {
        let mut state = make_test_campaign();
        state.player_dynasty_id = DynastyId::new(1);
        for kind in [
            AuditKind::OfficeDutyForfeiture,
            AuditKind::OfficeDutyShortfall,
        ] {
            state.audit_log.push(AuditRecord {
                day: state.clock.day(),
                kind,
                subject: "institution:3;dynasty:10".into(),
                detail: "different dynasty".to_owned(),
            });
        }

        assert!(!player_has_office_duty_forfeiture(&state));
        assert!(!has_recent_player_office_duty_shortfall(&state));

        for kind in [
            AuditKind::OfficeDutyForfeiture,
            AuditKind::OfficeDutyShortfall,
        ] {
            state.audit_log.push(AuditRecord {
                day: state.clock.day(),
                kind,
                subject: "institution:3;dynasty:1".into(),
                detail: "player dynasty".to_owned(),
            });
        }

        assert!(player_has_office_duty_forfeiture(&state));
        assert!(has_recent_player_office_duty_shortfall(&state));
    }

    #[test]
    fn office_reserves_do_not_block_emergency_business_rehabilitation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        state
            .businesses
            .get_mut(business_id)
            .expect("player business must exist")
            .operations
            .status = BusinessStatus::Distressed;
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(1_000);
        let candidate = Candidate {
            kind: GameplayCommandKind::InvestInBusiness,
            command: PlayerCommand::InvestInBusiness {
                business_id,
                amount: Money::from_copper(1_000),
            },
            description: "emergency business rehabilitation".to_owned(),
            score: 0,
        };

        assert!(candidate_preserves_office_duty_reserve(
            registry, &state, &candidate
        ));
    }

    #[test]
    fn ordinary_crisis_spending_preserves_office_reserves_but_escalation_can_override_them() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        add_active_crisis(&mut state);
        let crisis_id = *state
            .crises
            .keys()
            .next_back()
            .expect("test crisis must exist");
        let reserve = player_office_duty_reserve(&state, 0);
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = reserve;
        let candidate = Candidate {
            kind: GameplayCommandKind::RespondToCrisis,
            command: PlayerCommand::RespondToCrisis {
                crisis_id,
                response: CrisisResponse::Suppress,
            },
            description: "suppress a crisis".to_owned(),
            score: 0,
        };

        assert!(!candidate_preserves_office_duty_reserve(
            registry, &state, &candidate
        ));

        let crisis = state
            .crises
            .get_mut(&crisis_id)
            .expect("test crisis must exist");
        crisis.severity_basis_points = 8_000;
        crisis.status = CrisisStatus::Escalated;
        assert!(candidate_preserves_office_duty_reserve(
            registry, &state, &candidate
        ));
    }

    #[test]
    fn collapsed_portfolio_can_commit_all_available_treasury_to_rehabilitation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = *state
            .businesses
            .ids_for_owner(state.player_dynasty_id)
            .and_then(|ids| ids.iter().next())
            .expect("player dynasty must own a business");
        {
            let business = state
                .businesses
                .get_mut(business_id)
                .expect("player business must exist");
            business.operations.status = BusinessStatus::Distressed;
            business.operations.condition_basis_points = 500;
            business.finance.cash = Money::ZERO;
        }
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(5_000);
        let business = state
            .businesses
            .get(business_id)
            .expect("player business must exist");
        let mut candidates = Vec::new();

        generate_business_investment_candidate(
            registry,
            &state,
            GameplayPersona::Steward,
            business,
            &mut candidates,
        );

        let candidate = single_candidate(&candidates, "severe business rehabilitation");
        assert!(matches!(
            candidate.command,
            PlayerCommand::InvestInBusiness { amount, .. }
                if amount > Money::from_copper(3_000) && amount <= Money::from_copper(5_000)
        ));
    }

    #[test]
    fn succession_pressure_offers_formal_confirmation_when_no_better_heir_exists() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let (head_id, heir_id) = {
            let dynasty = state
                .dynasties
                .get(&player_id)
                .expect("player dynasty must exist");
            (
                dynasty.head_id(),
                dynasty.heir_id().expect("player dynasty must have an heir"),
            )
        };
        state
            .characters
            .get_mut(head_id)
            .expect("player head must exist")
            .identity
            .birth_day = state.clock.day().saturating_sub(55 * 360);
        state
            .family_councils
            .get_mut(&player_id)
            .expect("player family council must exist")
            .members
            .retain(|character_id| *character_id == head_id || *character_id == heir_id);
        let mut candidates = Vec::new();

        generate_heir_designation_candidates(&state, GameplayPersona::Steward, &mut candidates);

        let candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::DesignateHeir)
            .expect("succession pressure must expose a formal preparation decision");
        assert!(candidate.description.contains("formally confirm"));
        assert!(matches!(
            candidate.command,
            PlayerCommand::DesignateHeir { character_id } if character_id == heir_id
        ));
        apply_player_command(registry, &mut state, candidate.command.clone())
            .expect("formal confirmation candidate must be executable");
    }

    #[test]
    fn legal_candidates_require_a_simulated_grievance_and_do_not_repeat_it() {
        let mut state = make_test_campaign();
        state.legal_cases.clear();
        for _ in 0..LEGAL_CASE_FILING_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        let mut candidates = Vec::new();

        generate_legal_candidates(&state, GameplayPersona::PowerBroker, &mut candidates);
        assert!(
            candidates.is_empty(),
            "agents must not manufacture quarterly lawsuits without a default or attributable breach"
        );

        let unattributed_contract_id = state
            .contracts
            .values()
            .find(|contract| {
                state
                    .businesses
                    .get(contract.buyer_business_id)
                    .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id)
                    || state
                        .businesses
                        .get(contract.seller_business_id)
                        .is_some_and(|business| {
                            business.owner_dynasty_id() == state.player_dynasty_id
                        })
            })
            .expect("campaign must contain a player contract")
            .id;
        state
            .contracts
            .get_mut(&unattributed_contract_id)
            .expect("player contract must exist")
            .status = ContractStatus::Breached;
        generate_legal_candidates(&state, GameplayPersona::PowerBroker, &mut candidates);
        assert!(
            candidates.is_empty(),
            "an unattributed breach must not cause the agent to accuse an arbitrary counterparty"
        );

        let defendant_id = make_player_contract_breached(&mut state);
        generate_legal_candidates(&state, GameplayPersona::PowerBroker, &mut candidates);
        let legal_candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::FileLegalCase)
            .expect("a breached player contract must create a litigation route")
            .clone();
        assert!(matches!(
            legal_candidate.command,
            PlayerCommand::FileLegalCase {
                defendant_dynasty_id,
                kind: LegalCaseKind::ContractBreach,
                ..
            } if defendant_dynasty_id == defendant_id
        ));

        apply_player_command(
            rivergate_registry_for_test(),
            &mut state,
            legal_candidate.command,
        )
        .expect("grievance-backed legal case must apply");
        let legal_case = state
            .legal_cases
            .values_mut()
            .find(|legal_case| {
                legal_case.plaintiff_dynasty_id == state.player_dynasty_id
                    && legal_case.defendant_dynasty_id == defendant_id
                    && legal_case.kind == LegalCaseKind::ContractBreach
            })
            .expect("filed case must exist");
        legal_case.status = LegalCaseStatus::DecidedForPlaintiff;
        for _ in 0..LEGAL_CASE_FILING_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        candidates.clear();

        generate_legal_candidates(&state, GameplayPersona::PowerBroker, &mut candidates);
        assert!(
            candidates.iter().all(|candidate| {
                !matches!(
                    candidate.command,
                    PlayerCommand::FileLegalCase {
                        defendant_dynasty_id,
                        kind: LegalCaseKind::ContractBreach,
                        ..
                    } if defendant_dynasty_id == defendant_id
                )
            }),
            "a decided case must not be refiled against the same historical breach"
        );
    }

    #[test]
    fn legal_candidates_require_filing_funds() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        state.legal_cases.clear();
        for _ in 0..LEGAL_CASE_FILING_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        make_player_contract_breached(&mut state);
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = LEGAL_CASE_FILING_COST.saturating_sub(Money::from_copper(1));
        let mut candidates = Vec::new();

        generate_legal_candidates(&state, GameplayPersona::PowerBroker, &mut candidates);
        assert!(
            candidates.is_empty(),
            "the agent must not repeatedly offer a lawsuit the dynasty cannot fund"
        );
        let mut accumulator = CampaignAccumulator::new();
        let generated_kinds =
            ranked_candidates(registry, &state, GameplayPersona::PowerBroker, &accumulator).1;
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut accumulator,
            &generated_kinds,
        );
        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::FileLegalCase)
                .expect("legal-case statistics must exist")
                .activation_opportunities,
            0,
            "an unaffordable grievance is not an executable filing opportunity"
        );

        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = LEGAL_CASE_FILING_COST;
        generate_legal_candidates(&state, GameplayPersona::PowerBroker, &mut candidates);
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::FileLegalCase),
            "the filing route should become available at the exact cost boundary"
        );
        let generated_kinds =
            ranked_candidates(registry, &state, GameplayPersona::PowerBroker, &accumulator).1;
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut accumulator,
            &generated_kinds,
        );
        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::FileLegalCase)
                .expect("legal-case statistics must exist")
                .activation_opportunities,
            1,
            "the exact filing-fee boundary must activate the legal route"
        );
    }

    #[test]
    fn settlement_activation_requires_an_affordable_grounded_quote() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        make_legal_settlement_available_for_test(&mut state);
        let case_id = *state
            .legal_cases
            .keys()
            .next_back()
            .expect("settlement fixture must create a legal case");
        let quote = quote_player_legal_settlement(&state, case_id)
            .expect("settlement fixture must produce a grounded quote");
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = quote.amount.saturating_sub(Money::from_copper(1));

        let mut accumulator = CampaignAccumulator::new();
        let generated_kinds =
            ranked_candidates(registry, &state, GameplayPersona::Steward, &accumulator).1;
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut accumulator,
            &generated_kinds,
        );
        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::SettleLegalCase)
                .expect("legal-settlement statistics must exist")
                .activation_opportunities,
            0,
            "an unaffordable settlement must not be reported as an executable opportunity"
        );

        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = quote.amount;
        let generated_kinds =
            ranked_candidates(registry, &state, GameplayPersona::Steward, &accumulator).1;
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut accumulator,
            &generated_kinds,
        );
        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::SettleLegalCase)
                .expect("legal-settlement statistics must exist")
                .activation_opportunities,
            1,
            "the exact settlement quote must activate the legal route"
        );
    }

    #[test]
    fn rival_lawsuit_creates_an_executable_player_settlement_choice() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let lender_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| {
                *dynasty_id != player_id
                    && !state.loans.values().any(|loan| {
                        loan.lender_dynasty_id == *dynasty_id
                            && loan.borrower_dynasty_id == player_id
                            && loan.status.is_repayment_active()
                    })
            })
            .expect("campaign must contain a rival available to lend to the player");
        state
            .dynasties
            .get_mut(&lender_id)
            .expect("rival dynasty must exist")
            .resources
            .treasury = Money::from_copper(100_000);
        let loan_id = issue_loan(
            &mut state,
            LoanTerms {
                lender_dynasty_id: lender_id,
                borrower_dynasty_id: player_id,
                principal: Money::from_copper(5_000),
                weekly_payment: Money::from_copper(300),
                interest_basis_points: 1_000,
                collateral_property_id: None,
            },
        )
        .expect("fixture loan must be issuable");
        let loan = state
            .loans
            .get_mut(&loan_id)
            .expect("fixture loan must exist");
        loan.status = LoanStatus::Delinquent;
        loan.missed_payments = 1;
        let case_id = state.next_ids.legal_case();
        state.legal_cases.insert(
            case_id,
            crate::core::LegalCase {
                id: case_id,
                plaintiff_dynasty_id: lender_id,
                defendant_dynasty_id: player_id,
                kind: LegalCaseKind::Debt,
                claim_source: Some(crate::core::LegalClaimSource::Loan { loan_id }),
                evidence_basis_points: 7_500,
                public_attention_basis_points: 1_500,
                filed_day: state.clock.day(),
                hearing_day: state.clock.day().saturating_add(60),
                damages: Money::from_copper(5_000),
                status: LegalCaseStatus::Filed,
            },
        );
        assert_eq!(
            GameplaySnapshot::capture(&state).player_open_legal_cases_as_defendant,
            1,
            "the harness snapshot must expose direct legal pressure on the player"
        );
        let mut candidates = Vec::new();

        generate_reactive_candidates(&state, GameplayPersona::PowerBroker, &mut candidates);

        let candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::SettleLegalCase)
            .expect("a grounded rival lawsuit must expose a settlement response")
            .clone();
        assert!(matches!(
            candidate.command,
            PlayerCommand::SettleLegalCase {
                case_id: candidate_case_id
            } if candidate_case_id == case_id
        ));
        apply_player_command(registry, &mut state, candidate.command)
            .expect("generated legal settlement must be executable");
        assert_eq!(
            GameplaySnapshot::capture(&state).player_open_legal_cases_as_defendant,
            0,
            "settlement must remove the player-facing unresolved legal pressure"
        );
    }

    #[test]
    fn establishment_agents_can_commission_executable_persona_specific_intelligence() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let mut candidates = Vec::new();

        generate_information_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &mut candidates,
        );
        assert!(
            candidates.is_empty(),
            "commissioned intelligence should enter the agent loop after initial commercial observation"
        );
        let pressured_good_id = establish_player_contract_market(&mut state);

        generate_information_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &mut candidates,
        );
        assert!(
            candidates.is_empty(),
            "calm market conditions should not produce scheduled intelligence work"
        );
        state
            .market
            .quotes
            .get_mut(&pressured_good_id)
            .expect("contract good must have a quote")
            .stock = Quantity::ZERO;
        generate_information_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &mut candidates,
        );

        let candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::CommissionInformation)
            .expect("an established dynasty must be offered commissioned intelligence")
            .clone();
        assert!(matches!(
            candidate.command,
            PlayerCommand::CommissionInformation {
                focus: InformationFocus::Market { .. }
            }
        ));
        apply_player_command(registry, &mut state, candidate.command)
            .expect("generated intelligence commission must be executable");
        assert!(state.information_reports.values().any(|report| {
            report.owner_dynasty_id == state.player_dynasty_id
                && report.source == COMMISSIONED_INFORMATION_SOURCE
        }));

        candidates.clear();
        generate_information_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &mut candidates,
        );
        assert!(
            candidates.is_empty(),
            "automated personas should hold a report long enough for world conditions to change before leveraging it"
        );
        for _ in 0..AGENT_INFORMATION_LEVERAGE_DELAY_DAYS {
            state.clock.advance_one_day();
        }
        generate_information_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &mut candidates,
        );
        let leverage = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::LeverageInformation)
            .expect("commissioned intelligence must unlock a concrete follow-up action")
            .clone();
        apply_player_command(registry, &mut state, leverage.command)
            .expect("generated intelligence leverage must be executable");
        assert!(
            state.information_reports.values().all(|report| {
                report.owner_dynasty_id != state.player_dynasty_id
                    || report.source != COMMISSIONED_INFORMATION_SOURCE
            }),
            "leveraging the report must consume the commissioned intelligence"
        );

        candidates.clear();
        generate_information_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &mut candidates,
        );
        assert!(
            candidates.is_empty(),
            "the annual commission interval must prevent repetitive intelligence housekeeping after leverage"
        );
    }

    #[test]
    fn market_information_materiality_uses_wide_ratio_intermediates() {
        let mut state = make_test_campaign();
        let good_id = establish_player_contract_market(&mut state);
        let contract_id = state
            .contracts
            .values()
            .find(|contract| {
                contract.good_id == good_id && player_external_contract(&state, contract)
            })
            .expect("player must have an external contract for the selected good")
            .id;
        state
            .contracts
            .get_mut(&contract_id)
            .expect("selected contract must exist")
            .unit_price = Money::from_copper(i64::MAX);
        let quote = state
            .market
            .quotes
            .get_mut(&good_id)
            .expect("contract good must have a market quote");
        quote.previous_price = Money::from_copper(i64::MAX / 2);
        quote.price = Money::from_copper(i64::MAX);
        quote.stock = quote.target_stock;

        assert!(
            market_information_is_material(
                &state,
                state
                    .contracts
                    .get(&contract_id)
                    .expect("selected contract must exist"),
            ),
            "a roughly 100% price move must remain material even when a basis-point numerator would overflow u64"
        );
    }

    #[test]
    fn severe_counterparty_pressure_accelerates_political_intelligence_without_becoming_routine() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let relationship = state
            .relationships
            .values_mut()
            .find(|relationship| {
                relationship.pair.first == player_id || relationship.pair.second == player_id
            })
            .expect("campaign must contain a player counterparty relationship");
        relationship.trust_basis_points = 1_002;
        relationship.resentment_basis_points = 3_500;
        state.audit_log.push(AuditRecord {
            day: 0,
            kind: AuditKind::InformationCommission,
            subject: format!("dynasty:{player_id}").into(),
            detail: "prior commissioned intelligence".to_owned(),
        });
        for _ in 0..INFORMATION_COMMISSION_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        let mut candidates = Vec::new();

        // Routine commissions are paced at the report lifetime (540 days) unless
        // severe contract-relationship pressure exists; the agent's acceleration
        // under exposure is what uses the canonical 360-day game floor. A calm
        // counterparty with material trust strain must wait for the routine window.
        generate_information_candidates(
            registry,
            &state,
            GameplayPersona::Opportunist,
            &mut candidates,
        );
        assert!(
            candidates.iter().any(|candidate| {
                candidate.kind == GameplayCommandKind::CommissionInformation
                    && matches!(
                        candidate.command,
                        PlayerCommand::CommissionInformation {
                            focus: InformationFocus::Counterparty { .. }
                        }
                    )
            }),
            "material relationship strain must be committable once the routine commission window elapses"
        );

        // The cooldown still prevents a second commission before the interval
        // elapses, so pressure never becomes a routine every-cycle ritual.
        candidates.clear();
        for _ in 0..INFORMATION_COMMISSION_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::InformationCommission,
            subject: format!("dynasty:{player_id}").into(),
            detail: "fresh commission".to_owned(),
        });
        generate_information_candidates(
            registry,
            &state,
            GameplayPersona::Opportunist,
            &mut candidates,
        );
        assert!(
            !candidates
                .iter()
                .any(|candidate| { candidate.kind == GameplayCommandKind::CommissionInformation }),
            "a fresh commission must restart the accelerated commission window"
        );
    }

    #[test]
    fn weakened_power_broker_can_study_an_equally_embedded_rival() {
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        let counterparty_id = state
            .relationships
            .values()
            .find_map(|relationship| relationship_counterparty_id(relationship, player_id))
            .expect("campaign must contain a player counterparty relationship");
        let counterparty_head = state
            .dynasties
            .get(&counterparty_id)
            .expect("counterparty dynasty must exist")
            .head_id();
        for institution in state.institutions.values_mut() {
            if institution.office_holder_id.is_some_and(|character_id| {
                state
                    .characters
                    .get(character_id)
                    .is_some_and(|character| character.dynasty_id() != player_id)
            }) {
                institution.office_holder_id = None;
            }
        }
        let rival_institution = state
            .institutions
            .values_mut()
            .find(|institution| institution.office_holder_id.is_none())
            .expect("campaign must contain an office available for the rival fixture");
        rival_institution.members.insert(counterparty_head);
        rival_institution.office_holder_id = Some(counterparty_head);
        for relationship in state.relationships.values_mut().filter(|relationship| {
            relationship.pair.first == player_id || relationship.pair.second == player_id
        }) {
            relationship.trust_basis_points = 5_000;
            relationship.resentment_basis_points = 0;
        }
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points =
            AGENT_INFORMATION_POLITICAL_VULNERABILITY_LEGITIMACY.saturating_sub(1);

        let (focus, _) =
            preferred_counterparty_information_focus(&state, GameplayPersona::PowerBroker).expect(
                "a low-legitimacy officeholder should be able to study an equally embedded rival",
            );
        let InformationFocus::Counterparty { dynasty_id } = focus else {
            panic!("power-broker counterparty intelligence must target another dynasty");
        };
        assert!(
            count_player_offices(&state, dynasty_id)
                >= count_player_offices(&state, state.player_dynasty_id)
        );

        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = 8_000;
        assert!(
            preferred_counterparty_information_focus(&state, GameplayPersona::PowerBroker)
                .is_none(),
            "political intelligence should remain conditional rather than become scheduled housekeeping"
        );
    }

    #[test]
    fn entrepreneur_market_intelligence_reacts_to_an_adverse_contract_gap() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let good_id = establish_player_contract_market(&mut state);
        let player_id = state.player_dynasty_id;
        let contract_id = state
            .contracts
            .values()
            .find(|contract| {
                player_external_contract(&state, contract) && contract.good_id == good_id
            })
            .expect("player must have an external contract for the selected good")
            .id;
        let market_price = state
            .market
            .quotes
            .get(&good_id)
            .expect("contract good must have a market quote")
            .price;
        {
            let quote = state
                .market
                .quotes
                .get_mut(&good_id)
                .expect("contract good must have a market quote");
            quote.previous_price = quote.price;
            quote.stock = quote.target_stock;
        }
        let contract = state
            .contracts
            .get_mut(&contract_id)
            .expect("selected contract must remain present");
        let buyer_is_player = state
            .businesses
            .get(contract.buyer_business_id)
            .is_some_and(|business| business.owner_dynasty_id() == player_id);
        contract.unit_price = if buyer_is_player {
            market_price
                .checked_mul_ratio(120, 100)
                .expect("test contract premium must fit")
        } else {
            market_price
                .checked_mul_ratio(80, 100)
                .expect("test contract discount must fit")
                .max(Money::from_copper(1))
        };
        let mut candidates = Vec::new();

        generate_information_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &mut candidates,
        );

        assert!(candidates.iter().any(|candidate| {
            candidate.kind == GameplayCommandKind::CommissionInformation
                && matches!(
                    candidate.command,
                    PlayerCommand::CommissionInformation {
                        focus: InformationFocus::Market { good_id: candidate_good_id }
                    } if candidate_good_id == good_id
                )
        }));
    }

    #[test]
    fn power_broker_preserves_political_reserves_before_extending_credit() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(49_999);
        let mut candidates = Vec::new();

        add_lend_candidate(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut candidates,
        );

        assert!(
            candidates.is_empty(),
            "a power broker must not sacrifice campaign and office reserves to habitual lending"
        );
    }

    #[test]
    fn opportunist_credit_uses_short_term_high_yield_terms() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(100_000);
        make_external_credit_need_available_for_test(&mut state);
        let mut candidates = Vec::new();

        add_lend_candidate(
            registry,
            &state,
            GameplayPersona::Opportunist,
            &mut candidates,
        );

        let candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::ExtendCredit)
            .expect("opportunist must receive a lending candidate");
        let PlayerCommand::IssueLoan { terms } = &candidate.command else {
            panic!("extend-credit candidate must issue a loan");
        };
        assert_eq!(
            terms.interest_basis_points,
            AGENT_OPPORTUNIST_LOAN_INTEREST_BASIS_POINTS
        );
        assert_eq!(
            terms.weekly_payment,
            terms
                .principal
                .ceil_div_positive(AGENT_OPPORTUNIST_LOAN_AMORTIZATION_WEEKS)
        );
        assert!(candidate.description.contains("high-yield short-term loan"));
    }

    #[test]
    fn opportunist_does_not_offer_credit_without_a_real_financing_pressure() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(100_000);
        for dynasty in state
            .dynasties
            .values_mut()
            .filter(|dynasty| dynasty.id() != player_id)
        {
            dynasty.resources.treasury = Money::from_copper(100_000);
        }
        for loan in state.loans.values_mut() {
            if matches!(loan.status, LoanStatus::Delinquent | LoanStatus::Defaulted) {
                loan.status = LoanStatus::Repaid;
                loan.balance = Money::ZERO;
            }
        }
        assert!(
            state
                .dynasties
                .values()
                .filter(|dynasty| dynasty.id() != player_id)
                .all(|dynasty| lending_pressure(&state, dynasty.id()) == 0)
        );

        let mut opportunist_candidates = Vec::new();
        add_lend_candidate(
            registry,
            &state,
            GameplayPersona::Opportunist,
            &mut opportunist_candidates,
        );
        assert!(
            opportunist_candidates
                .iter()
                .all(|candidate| candidate.kind != GameplayCommandKind::ExtendCredit),
            "surplus player cash must not manufacture a safe high-yield loan when the counterparty has no financing need"
        );

        let mut steward_candidates = Vec::new();
        add_lend_candidate(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut steward_candidates,
        );
        assert!(
            steward_candidates
                .iter()
                .all(|candidate| candidate.kind != GameplayCommandKind::ExtendCredit),
            "conservative personas must also require an actual counterparty financing pressure"
        );
    }

    #[test]
    fn opportunist_uses_more_aggressive_terms_for_a_distressed_borrower() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        make_external_credit_need_available_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        let borrower_id = eligible_lending_borrower(registry, &state)
            .expect("fixture must expose a financing-pressure borrower")
            .id();
        state
            .businesses
            .iter_mut()
            .find(|business| business.owner_dynasty_id() == borrower_id)
            .expect("borrower must own a business")
            .operations
            .status = BusinessStatus::Distressed;
        assert!(lending_pressure(&state, borrower_id) >= 2);
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(120_000);
        let mut candidates = Vec::new();

        add_lend_candidate(
            registry,
            &state,
            GameplayPersona::Opportunist,
            &mut candidates,
        );

        let candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::ExtendCredit)
            .expect("distressed borrower must expose opportunistic credit");
        let PlayerCommand::IssueLoan { terms } = &candidate.command else {
            panic!("extend-credit candidate must issue a loan");
        };
        assert!(terms.principal >= Money::from_copper(5_000));
        assert_eq!(
            terms.weekly_payment,
            terms
                .principal
                .ceil_div_positive(AGENT_OPPORTUNIST_STRESSED_LOAN_AMORTIZATION_WEEKS)
        );
    }

    #[test]
    fn delinquent_player_credit_creates_a_legal_grievance() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let borrower_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| {
                *dynasty_id != player_id
                    && !same_pair_credit_blocks_new_loan(&state, player_id, *dynasty_id)
            })
            .expect("campaign must contain a rival available for new player credit");
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(100_000);
        state
            .dynasties
            .get_mut(&borrower_id)
            .expect("borrower dynasty must exist")
            .resources
            .treasury = Money::from_copper(20_000);
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::IssueLoan {
                terms: LoanTerms {
                    lender_dynasty_id: player_id,
                    borrower_dynasty_id: borrower_id,
                    principal: Money::from_copper(5_000),
                    weekly_payment: Money::from_copper(300),
                    interest_basis_points: 1_800,
                    collateral_property_id: None,
                },
            },
        )
        .expect("test loan must be executable");
        state
            .loans
            .values_mut()
            .find(|loan| {
                loan.lender_dynasty_id == player_id && loan.borrower_dynasty_id == borrower_id
            })
            .expect("test loan must exist")
            .status = LoanStatus::Delinquent;
        state
            .legal_cases
            .retain(|_, legal_case| legal_case.plaintiff_dynasty_id != player_id);

        assert_eq!(
            legal_grievance_kind(&state, borrower_id),
            Some(LegalCaseKind::Debt)
        );
        assert_eq!(
            legal_case_urgency(&state),
            800,
            "delinquent player credit should make enforcement strategically relevant"
        );
        let mut candidates = Vec::new();
        generate_legal_candidates(&state, GameplayPersona::Opportunist, &mut candidates);
        assert!(candidates.iter().any(|candidate| {
            candidate.kind == GameplayCommandKind::FileLegalCase
                && matches!(
                    candidate.command,
                    PlayerCommand::FileLegalCase {
                        defendant_dynasty_id,
                        kind: LegalCaseKind::Debt,
                        ..
                    } if defendant_dynasty_id == borrower_id
                )
        }));

        state
            .loans
            .values_mut()
            .find(|loan| {
                loan.lender_dynasty_id == player_id && loan.borrower_dynasty_id == borrower_id
            })
            .expect("test loan must still exist")
            .status = LoanStatus::Defaulted;
        assert_eq!(
            legal_case_urgency(&state),
            1_200,
            "defaulted player credit should outrank routine governance work"
        );
    }

    #[test]
    fn established_office_power_is_recorded_as_an_activation_opportunity() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        grant_player_office_for_test(&mut state);
        let institution_id = state
            .institutions
            .values()
            .find(|institution| institution.office_holder_id.is_some())
            .expect("fixture must grant the player an office")
            .institution_id;
        let district_id = registry
            .get_institution(institution_id)
            .expect("runtime institution must have a registry definition")
            .district_id();
        state
            .districts
            .get_mut(&district_id)
            .expect("institution district must exist")
            .employment_basis_points = 0;
        let mut accumulator = CampaignAccumulator::new();
        let generated_kinds =
            ranked_candidates(registry, &state, GameplayPersona::Steward, &accumulator).1;

        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut accumulator,
            &generated_kinds,
        );

        assert_eq!(
            accumulator
                .commands
                .get(&GameplayCommandKind::ExerciseOfficePower)
                .expect("office-power statistics must exist")
                .activation_opportunities,
            1
        );
    }

    #[test]
    fn wage_posture_candidates_repair_strained_workforces() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = *player_business_ids_for_test(&state)
            .first()
            .expect("player business must exist");
        let employment_id = state
            .employment
            .values()
            .find(|agreement| agreement.business_id == business_id)
            .expect("player business must have an employment agreement")
            .id;
        {
            let agreement = state
                .employment
                .get_mut(&employment_id)
                .expect("employment must exist");
            let workers = i64::from(agreement.workers.max(1));
            agreement.weekly_wage = Money::from_copper(20 * workers);
            agreement.loyalty_basis_points = 3_200;
            agreement.conditions_basis_points = 4_000;
        }

        let mut candidates = Vec::new();
        generate_business_wage_candidates(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut candidates,
        );

        let candidate = single_candidate(&candidates, "wage repair posture");
        assert!(matches!(
            candidate.command,
            PlayerCommand::SetBusinessWages {
                weekly_wage_per_worker,
                ..
            } if weekly_wage_per_worker.copper() >= 35
        ));
    }

    #[test]
    fn wage_activation_predicate_mirrors_the_canonical_cooldown_route() {
        let mut state = make_test_campaign();
        assert!(
            has_business_wage_opportunity(&state),
            "a workforceable player business starts off cooldown"
        );

        let business_id = *player_business_ids_for_test(&state)
            .first()
            .expect("player business must exist");
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::BusinessWageChange,
            subject: format!("business:{business_id}").into(),
            detail: "wage_per_worker=42; agreements=1".to_owned(),
        });
        assert!(
            !has_business_wage_opportunity(&state),
            "a fresh renegotiation blocks the route inside the cooldown window"
        );
        for _ in 0..BUSINESS_WAGE_CHANGE_INTERVAL_DAYS {
            state.clock.advance_one_day();
        }
        assert!(
            has_business_wage_opportunity(&state),
            "the route reopens once the cooldown elapses"
        );
    }
}

mod metrics {
    use super::*;

    #[test]
    fn succession_starts_a_distinct_legacy_phase() {
        let mut arc = GameplayFantasyArc {
            first_city_shaping_action_day: Some(1_000),
            ..GameplayFantasyArc::default()
        };
        assert_eq!(gameplay_phase(&arc), GameplayPhase::DynasticGovernance);

        arc.first_succession_day = Some(4_500);

        assert_eq!(gameplay_phase(&arc), GameplayPhase::SuccessionLegacy);
    }

    #[test]
    fn early_patronage_remains_part_of_establishment_until_commercial_standing() {
        let mut arc = GameplayFantasyArc {
            first_reputation_standing_day: Some(90),
            first_institution_support_day: Some(420),
            ..GameplayFantasyArc::default()
        };

        assert_eq!(gameplay_phase(&arc), GameplayPhase::Establishment);

        arc.first_commercial_standing_day = Some(600);
        assert_eq!(gameplay_phase(&arc), GameplayPhase::InstitutionalAscent);
    }

    #[test]
    fn phase_stats_record_consecutive_quiet_streaks() {
        let mut accumulator = CampaignAccumulator::new();
        let quiet = PhaseCycleObservation {
            action: None,
            choices: ChoiceCycleMetrics {
                substantive_candidate_count: 0,
                substantive_viable_count: 0,
                viable_command_kind_count: 0,
                family_quality: AlternativeQuality::default(),
                option_quality: AlternativeQuality::default(),
            },
            ambient_change: true,
            quiet_cause: None,
        };

        accumulator.record_phase_cycle(GameplayPhase::InstitutionalAscent, quiet);
        accumulator.record_phase_cycle(GameplayPhase::InstitutionalAscent, quiet);
        accumulator.record_phase_cycle(
            GameplayPhase::InstitutionalAscent,
            PhaseCycleObservation {
                action: Some(GameplayCommandKind::EducateFamilyMember),
                choices: ChoiceCycleMetrics {
                    substantive_candidate_count: 1,
                    substantive_viable_count: 1,
                    viable_command_kind_count: 1,
                    family_quality: AlternativeQuality::default(),
                    option_quality: AlternativeQuality::default(),
                },
                ambient_change: true,
                quiet_cause: None,
            },
        );
        accumulator.record_phase_cycle(GameplayPhase::InstitutionalAscent, quiet);

        let stats = accumulator
            .phase_stats
            .get(&GameplayPhase::InstitutionalAscent)
            .expect("institutional-ascent phase statistics must exist");
        assert_eq!(stats.quiet_cycles, 3);
        assert_eq!(stats.longest_quiet_streak_cycles, 2);
    }

    #[test]
    fn quiet_diagnostic_separates_policy_gates_from_generator_gaps() {
        let mut accumulator = CampaignAccumulator::new();
        let activation_delta = BTreeMap::from([(GameplayCommandKind::EnactLaw, 1_u32)]);
        let raw_generated_kinds = BTreeSet::from([
            GameplayCommandKind::StartPublicWork,
            GameplayCommandKind::FileLegalCase,
        ]);
        let retained_kinds = BTreeSet::from([GameplayCommandKind::FileLegalCase]);
        let retained_counts_by_kind = BTreeMap::from([(GameplayCommandKind::FileLegalCase, 1)]);
        let probed_counts_by_kind = BTreeMap::from([(GameplayCommandKind::FileLegalCase, 1)]);
        let probe = ProbeResult {
            selected: None,
            viable_count: 0,
            substantive_viable_count: 0,
            viable_command_kinds: BTreeSet::new(),
            viable_options: Vec::new(),
            close_choice_score_gap: None,
            distinct_immediate_choice_profiles: 0,
            distinct_projected_choice_profiles: 0,
            family_close_choice_score_gap: None,
            distinct_immediate_family_profiles: 0,
            distinct_projected_family_profiles: 0,
            rejections: Vec::new(),
        };

        record_quiet_diagnostic(
            &mut accumulator,
            &probe,
            &raw_generated_kinds,
            &retained_kinds,
            &retained_counts_by_kind,
            &probed_counts_by_kind,
            &activation_delta,
        );

        assert_eq!(
            accumulator
                .quiet_diagnostic
                .generator_gaps
                .get(&GameplayCommandKind::EnactLaw),
            Some(&1),
            "an activation opportunity without any built candidate must be a generator gap"
        );
        assert_eq!(
            accumulator
                .quiet_diagnostic
                .policy_gates
                .get(&GameplayCommandKind::StartPublicWork),
            Some(&1),
            "built candidates removed by agent spending policy must be policy gates"
        );
        assert_eq!(
            accumulator
                .quiet_diagnostic
                .validation_gates
                .get(&GameplayCommandKind::FileLegalCase),
            Some(&1),
            "probed candidates the game rejected must be validation gates"
        );
    }

    #[test]
    fn quiet_diagnostic_skips_cycles_with_an_action() {
        let mut accumulator = CampaignAccumulator::new();
        let probe = ProbeResult {
            selected: None,
            viable_count: 1,
            substantive_viable_count: 1,
            viable_command_kinds: BTreeSet::from([GameplayCommandKind::EducateFamilyMember]),
            viable_options: Vec::new(),
            close_choice_score_gap: None,
            distinct_immediate_choice_profiles: 0,
            distinct_projected_choice_profiles: 0,
            family_close_choice_score_gap: None,
            distinct_immediate_family_profiles: 0,
            distinct_projected_family_profiles: 0,
            rejections: Vec::new(),
        };

        record_quiet_diagnostic(
            &mut accumulator,
            &probe,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        );

        assert_eq!(accumulator.quiet_diagnostic.generator_gaps.len(), 0);
        assert_eq!(accumulator.quiet_diagnostic.policy_gates.len(), 0);
        assert_eq!(accumulator.quiet_diagnostic.validation_gates.len(), 0);
        assert_eq!(accumulator.quiet_diagnostic.budget_gates.len(), 0);
    }

    #[test]
    fn quiet_diagnostic_aggregate_sums_campaign_gates() {
        let report = cached_focused_report(30);
        let mut expected = GameplayQuietDiagnostic::default();
        for campaign in &report.campaigns {
            for (kind, count) in &campaign.quiet_diagnostic.generator_gaps {
                *expected.generator_gaps.entry(*kind).or_default() += *count;
            }
            for (kind, count) in &campaign.quiet_diagnostic.policy_gates {
                *expected.policy_gates.entry(*kind).or_default() += *count;
            }
            for (kind, count) in &campaign.quiet_diagnostic.validation_gates {
                *expected.validation_gates.entry(*kind).or_default() += *count;
            }
            for (kind, count) in &campaign.quiet_diagnostic.budget_gates {
                *expected.budget_gates.entry(*kind).or_default() += *count;
            }
            expected.dormant_cycles += campaign.quiet_diagnostic.dormant_cycles;
        }
        assert_eq!(report.aggregate.quiet_diagnostic, expected);
    }

    #[test]
    fn quiet_diagnostic_counts_dormant_cycles_and_describes_them() {
        let mut accumulator = CampaignAccumulator::new();
        let probe = ProbeResult {
            selected: None,
            viable_count: 0,
            substantive_viable_count: 0,
            viable_command_kinds: BTreeSet::new(),
            viable_options: Vec::new(),
            close_choice_score_gap: None,
            distinct_immediate_choice_profiles: 0,
            distinct_projected_choice_profiles: 0,
            family_close_choice_score_gap: None,
            distinct_immediate_family_profiles: 0,
            distinct_projected_family_profiles: 0,
            rejections: Vec::new(),
        };

        let reason = record_quiet_diagnostic(
            &mut accumulator,
            &probe,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        );

        assert_eq!(accumulator.quiet_diagnostic.dormant_cycles, 1);
        let reason = reason.expect("dormant cycles must report a reason");
        assert!(
            reason.contains("dormant"),
            "dormant reason must identify the waiting state, got {reason:?}"
        );
        assert_eq!(accumulator.quiet_diagnostic.generator_gaps.len(), 0);
        assert_eq!(accumulator.quiet_diagnostic.policy_gates.len(), 0);
        assert_eq!(accumulator.quiet_diagnostic.validation_gates.len(), 0);
        assert_eq!(accumulator.quiet_diagnostic.budget_gates.len(), 0);

        let actionable_probe = ProbeResult {
            selected: None,
            viable_count: 1,
            substantive_viable_count: 1,
            ..probe
        };
        let actionable_reason = record_quiet_diagnostic(
            &mut accumulator,
            &actionable_probe,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        assert_eq!(
            actionable_reason, None,
            "actionable cycles must not record a no-action reason"
        );
        assert_eq!(
            accumulator.quiet_diagnostic.dormant_cycles, 1,
            "actionable cycles must not add dormant cycles"
        );
    }

    #[test]
    fn quiet_diagnostic_reason_joins_gap_and_gate_causes() {
        let mut accumulator = CampaignAccumulator::new();
        // NominateForOffice is outside the deliberately narrowed route set, so
        // an activation without a candidate is a true generator gap here;
        // SellProperty would now classify as agent restraint instead.
        let activation_delta = BTreeMap::from([(GameplayCommandKind::NominateForOffice, 1_u32)]);
        let raw_generated_kinds = BTreeSet::from([GameplayCommandKind::BuyProperty]);
        let retained_kinds = BTreeSet::from([
            GameplayCommandKind::EnactLaw,
            GameplayCommandKind::FundPublicWork,
        ]);
        let retained_counts_by_kind = BTreeMap::from([
            (GameplayCommandKind::EnactLaw, 2),
            (GameplayCommandKind::FundPublicWork, 3),
        ]);
        let probed_counts_by_kind = BTreeMap::from([
            (GameplayCommandKind::EnactLaw, 2),
            (GameplayCommandKind::FundPublicWork, 1),
        ]);
        let probe = ProbeResult {
            selected: None,
            viable_count: 0,
            substantive_viable_count: 0,
            viable_command_kinds: BTreeSet::new(),
            viable_options: Vec::new(),
            close_choice_score_gap: None,
            distinct_immediate_choice_profiles: 0,
            distinct_projected_choice_profiles: 0,
            family_close_choice_score_gap: None,
            distinct_immediate_family_profiles: 0,
            distinct_projected_family_profiles: 0,
            rejections: Vec::new(),
        };

        let reason = record_quiet_diagnostic(
            &mut accumulator,
            &probe,
            &raw_generated_kinds,
            &retained_kinds,
            &retained_counts_by_kind,
            &probed_counts_by_kind,
            &activation_delta,
        )
        .expect("a caused quiet cycle must report a reason");

        assert!(reason.contains("activation without candidate [office-nomination]"));
        assert!(reason.contains("declined by agent policy [buy-property]"));
        assert!(reason.contains("rejected by validation [enact-law]"));
        assert!(reason.contains("unverified due to probe budget [public-work-funding]"));
        assert_eq!(accumulator.quiet_diagnostic.dormant_cycles, 0);
    }

    #[test]
    fn quiet_diagnostic_classifies_narrowed_routes_as_agent_restraint() {
        let mut accumulator = CampaignAccumulator::new();
        // SellProperty is a deliberately narrowed route: the canonical game
        // accepts a liquidation, but the persona only sells under distress or
        // committed need. An activation without a candidate there is agent
        // restraint, not a generator hole.
        let activation_delta = BTreeMap::from([(GameplayCommandKind::SellProperty, 1_u32)]);
        let raw_generated_kinds = BTreeSet::new();
        let retained_kinds = BTreeSet::new();
        let retained_counts_by_kind = BTreeMap::new();
        let probed_counts_by_kind = BTreeMap::new();
        let probe = ProbeResult {
            selected: None,
            viable_count: 0,
            substantive_viable_count: 0,
            viable_command_kinds: BTreeSet::new(),
            viable_options: Vec::new(),
            close_choice_score_gap: None,
            distinct_immediate_choice_profiles: 0,
            distinct_projected_choice_profiles: 0,
            family_close_choice_score_gap: None,
            distinct_immediate_family_profiles: 0,
            distinct_projected_family_profiles: 0,
            rejections: Vec::new(),
        };

        let reason = record_quiet_diagnostic(
            &mut accumulator,
            &probe,
            &raw_generated_kinds,
            &retained_kinds,
            &retained_counts_by_kind,
            &probed_counts_by_kind,
            &activation_delta,
        )
        .expect("a restrained quiet cycle must report a reason");

        assert!(
            reason.contains("reserved by agent policy [sell-property]"),
            "sell-property deliberately narrows to strategic-need conditions, so its unfired activation is agent restraint, not a coverage hole"
        );
        assert!(!reason.contains("activation without candidate"));
        assert_eq!(
            accumulator
                .quiet_diagnostic
                .restrained_routes
                .get(&GameplayCommandKind::SellProperty),
            Some(&1),
            "restrained routes must be counted separately from generator gaps"
        );
        assert_eq!(
            accumulator
                .quiet_diagnostic
                .generator_gaps
                .get(&GameplayCommandKind::SellProperty),
            None
        );
    }

    #[test]
    fn quiet_diagnostic_classifies_ward_adoption_floor_declines_as_agent_restraint() {
        let mut accumulator = CampaignAccumulator::new();
        // Ward adoption is a standing expense whose generator requires the
        // shared discretionary floor on top of the canonical cost, so a
        // below-floor treasury declines it by design. An activation without a
        // candidate there is agent restraint, not a generator hole drowning
        // real coverage gaps in the succession phase.
        let activation_delta = BTreeMap::from([(GameplayCommandKind::AdoptWard, 1_u32)]);
        let raw_generated_kinds = BTreeSet::new();
        let retained_kinds = BTreeSet::new();
        let retained_counts_by_kind = BTreeMap::new();
        let probed_counts_by_kind = BTreeMap::new();
        let probe = ProbeResult {
            selected: None,
            viable_count: 0,
            substantive_viable_count: 0,
            viable_command_kinds: BTreeSet::new(),
            viable_options: Vec::new(),
            close_choice_score_gap: None,
            distinct_immediate_choice_profiles: 0,
            distinct_projected_choice_profiles: 0,
            family_close_choice_score_gap: None,
            distinct_immediate_family_profiles: 0,
            distinct_projected_family_profiles: 0,
            rejections: Vec::new(),
        };

        let reason = record_quiet_diagnostic(
            &mut accumulator,
            &probe,
            &raw_generated_kinds,
            &retained_kinds,
            &retained_counts_by_kind,
            &probed_counts_by_kind,
            &activation_delta,
        )
        .expect("a restrained quiet cycle must report a reason");

        assert!(
            reason.contains("reserved by agent policy [adopt-ward]"),
            "ward adoption deliberately narrows to above-floor treasuries, so its unfired activation is agent restraint, not a coverage hole"
        );
        assert!(!reason.contains("activation without candidate"));
        assert_eq!(
            accumulator
                .quiet_diagnostic
                .restrained_routes
                .get(&GameplayCommandKind::AdoptWard),
            Some(&1),
            "restrained routes must be counted separately from generator gaps"
        );
        assert_eq!(
            accumulator
                .quiet_diagnostic
                .generator_gaps
                .get(&GameplayCommandKind::AdoptWard),
            None
        );
    }

    #[test]
    fn probe_keeps_target_identity_separate_from_consequence_divergence() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(1_000_000);
        for property in state
            .properties
            .values_mut()
            .filter(|property| property.owner_dynasty_id.is_none())
            .take(2)
        {
            property.weekly_rent = Money::from_copper(600);
        }
        let mut candidates = Vec::new();
        generate_finance_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &mut candidates,
        );
        let property_candidates: Vec<_> = candidates
            .into_iter()
            .filter(|candidate| candidate.kind == GameplayCommandKind::BuyProperty)
            .take(2)
            .collect();
        assert_eq!(
            property_candidates.len(),
            2,
            "test campaign must expose two affordable property targets"
        );
        let mut accumulator = CampaignAccumulator::new();

        let probe = probe_candidates(
            registry,
            &state,
            property_candidates.into_iter(),
            30,
            360,
            &mut accumulator,
        )
        .expect("property alternatives must be probeable");

        assert_eq!(probe.viable_command_kinds.len(), 1);
        assert_eq!(probe.viable_options.len(), 2);
        assert!(
            probe
                .viable_options
                .iter()
                .all(|option| option.projected_horizon_days == 90)
        );
        assert_eq!(
            probe.distinct_immediate_choice_profiles, 1,
            "equivalent warehouse purchases must not be counted as different consequence profiles solely because the property IDs differ"
        );
        assert!(probe.viable_options.iter().all(|option| {
            option
                .immediate_profile
                .increases
                .contains(&GameplayMeasure::PlayerProperties)
                && option
                    .immediate_profile
                    .decreases
                    .contains(&GameplayMeasure::PlayerTreasury)
        }));
        assert_ne!(
            probe.viable_options[0]
                .immediate_profile
                .strategic_fingerprint,
            probe.viable_options[1]
                .immediate_profile
                .strategic_fingerprint,
            "different property targets must retain distinct strategic identities"
        );
    }

    #[test]
    fn property_candidates_filter_yield_below_every_persona_hurdle() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(1_000_000);

        let mut steward_candidates = Vec::new();
        generate_finance_candidates(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut steward_candidates,
        );
        assert!(
            steward_candidates
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::BuyProperty),
            "a dynasty with idle capital must be able to engage the property market"
        );

        let mut entrepreneur_candidates = Vec::new();
        generate_finance_candidates(
            registry,
            &state,
            GameplayPersona::Entrepreneur,
            &mut entrepreneur_candidates,
        );
        assert!(
            entrepreneur_candidates
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::BuyProperty),
            "a commercially oriented persona should also see sufficiently productive property"
        );

        for property_id in state
            .properties
            .values()
            .filter(|property| property.owner_dynasty_id.is_none())
            .map(|property| property.id)
            .collect::<Vec<_>>()
        {
            state
                .properties
                .get_mut(&property_id)
                .expect("unowned property must exist")
                .weekly_rent = Money::from_copper(100);
        }

        for persona in [GameplayPersona::Steward, GameplayPersona::Entrepreneur] {
            let mut candidates = Vec::new();
            generate_finance_candidates(registry, &state, persona, &mut candidates);
            assert!(
                candidates
                    .iter()
                    .all(|candidate| candidate.kind != GameplayCommandKind::BuyProperty),
                "property yield below every persona hurdle must not generate a purchase for {persona:?}"
            );
        }
    }

    #[test]
    fn public_work_preferences_expose_distinct_need_and_persona_routes() {
        let mut state = make_test_campaign();
        let district_id = state
            .districts
            .keys()
            .copied()
            .next()
            .expect("campaign must contain a district");
        {
            let district = state
                .districts
                .get_mut(&district_id)
                .expect("campaign district must exist");
            district.employment_basis_points = 2_000;
            district.sanitation_basis_points = 9_000;
            district.safety_basis_points = 9_000;
            district.unrest_basis_points = 1_000;
        }
        let district = state
            .districts
            .get(&district_id)
            .expect("campaign district must exist");

        let entrepreneur =
            preferred_public_work_kinds(&state, district, GameplayPersona::Entrepreneur, 180);
        let steward = preferred_public_work_kinds(&state, district, GameplayPersona::Steward, 440);

        assert_eq!(entrepreneur[0], PublicWorkKind::Market);
        assert_ne!(entrepreneur[0], entrepreneur[1]);
        assert_ne!(
            entrepreneur, steward,
            "different governing priorities should expose different project shortlists under the same district conditions"
        );
    }

    #[test]
    fn public_work_candidate_ranking_preserves_material_need() {
        let mut state = make_test_campaign();
        let district_id = state
            .districts
            .keys()
            .copied()
            .next()
            .expect("campaign must contain a district");
        {
            let district = state
                .districts
                .get_mut(&district_id)
                .expect("campaign district must exist");
            district.employment_basis_points = 8_000;
            district.sanitation_basis_points = 9_000;
            district.safety_basis_points = 9_000;
            district.unrest_basis_points = 4_500;
        }
        let district = state
            .districts
            .get(&district_id)
            .expect("campaign district must exist");

        let shortlist =
            preferred_public_work_kinds(&state, district, GameplayPersona::Steward, 440);
        assert_eq!(
            shortlist[0],
            PublicWorkKind::School,
            "high unrest should make a school the steward's strongest need-driven project"
        );
        assert!(
            public_work_candidate_priority(
                440,
                district,
                GameplayPersona::Steward,
                shortlist[0],
                0,
            ) > public_work_candidate_priority(
                440,
                district,
                GameplayPersona::Steward,
                shortlist[1],
                0,
            ),
            "final candidate ranking must preserve the need ordering used to build the shortlist"
        );
        assert!(
            public_work_candidate_priority(
                440,
                district,
                GameplayPersona::Steward,
                shortlist[0],
                1,
            ) < public_work_candidate_priority(
                440,
                district,
                GameplayPersona::Steward,
                shortlist[1],
                0,
            ),
            "after completing the strongest project kind once, a close unmet need should be able to become the better civic investment"
        );
    }

    #[test]
    fn public_work_shortlist_rotates_after_repeated_portfolio_investment() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let district_id = state
            .districts
            .keys()
            .copied()
            .next()
            .expect("campaign must contain a district");
        for _ in 0..4 {
            let public_work_id = state.next_ids.public_work();
            state.public_works.insert(
                public_work_id,
                crate::core::PublicWork {
                    id: public_work_id,
                    district_id,
                    kind: PublicWorkKind::Market,
                    sponsor_dynasty_id: Some(player_id),
                    budget: Money::from_copper(12_000),
                    spent: Money::from_copper(12_000),
                    progress_basis_points: 10_000,
                    status: PublicWorkStatus::Completed,
                },
            );
        }
        let district = state
            .districts
            .get(&district_id)
            .expect("campaign district must exist");

        let shortlist =
            preferred_public_work_kinds(&state, district, GameplayPersona::PowerBroker, 520);

        assert!(
            !shortlist.contains(&PublicWorkKind::Market),
            "a power broker that has already built several markets should surface other civic investments"
        );
    }

    #[test]
    fn stalled_sponsored_public_work_creates_a_private_funding_candidate() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let district_id = state
            .districts
            .keys()
            .copied()
            .next()
            .expect("campaign must contain a district");
        let public_work_id = state.next_ids.public_work();
        state.public_works.insert(
            public_work_id,
            crate::core::PublicWork {
                id: public_work_id,
                district_id,
                kind: PublicWorkKind::Drainage,
                sponsor_dynasty_id: Some(player_id),
                budget: Money::from_copper(10_000),
                spent: Money::from_copper(7_500),
                progress_basis_points: 7_500,
                status: PublicWorkStatus::Suspended,
            },
        );
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(20_000);
        let mut candidates = Vec::new();

        generate_public_work_funding_candidates(
            rivergate_registry_for_test(),
            &state,
            GameplayPersona::Steward,
            &mut candidates,
        );

        let candidate = candidates
            .iter()
            .find(|candidate| {
                matches!(
                    candidate.command,
                    PlayerCommand::FundPublicWork {
                        public_work_id: candidate_id,
                        amount,
                    } if candidate_id == public_work_id && amount == Money::from_copper(2_500)
                )
            })
            .expect("a wealthy sponsor must be able to finish its stalled civic commitment");
        assert_eq!(candidate.kind, GameplayCommandKind::FundPublicWork);
        assert!(candidate.description.contains("finish stalled"));
    }

    #[test]
    fn wealthy_sponsor_can_accelerate_an_active_public_work() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let district_id = state
            .districts
            .keys()
            .copied()
            .next()
            .expect("campaign must contain a district");
        let public_work_id = state.next_ids.public_work();
        state.public_works.insert(
            public_work_id,
            crate::core::PublicWork {
                id: public_work_id,
                district_id,
                kind: PublicWorkKind::Market,
                sponsor_dynasty_id: Some(player_id),
                budget: Money::from_copper(12_000),
                spent: Money::from_copper(1_200),
                progress_basis_points: 1_000,
                status: PublicWorkStatus::Building,
            },
        );
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(100_000);
        let mut candidates = Vec::new();

        generate_public_work_funding_candidates(
            rivergate_registry_for_test(),
            &state,
            GameplayPersona::Entrepreneur,
            &mut candidates,
        );

        let candidate = candidates
            .iter()
            .find(|candidate| {
                matches!(
                    candidate.command,
                    PlayerCommand::FundPublicWork {
                        public_work_id: candidate_id,
                        amount,
                    } if candidate_id == public_work_id && amount == Money::from_copper(10_800)
                )
            })
            .expect("a wealthy sponsor must be able to accelerate an active civic commitment");
        assert_eq!(candidate.kind, GameplayCommandKind::FundPublicWork);
        assert!(
            candidate
                .description
                .contains("finish the Market project in")
        );
    }

    #[test]
    fn terminal_phase_transition_requests_one_decision_cycle() {
        let mut accumulator = CampaignAccumulator::new();
        accumulator.fantasy_arc.first_succession_day = Some(7_200);

        assert!(terminal_phase_needs_decision(&accumulator));

        accumulator
            .phase_stats
            .get_mut(&GameplayPhase::SuccessionLegacy)
            .expect("succession phase statistics must exist")
            .decision_cycles = 1;
        assert!(!terminal_phase_needs_decision(&accumulator));
    }

    #[test]
    fn snapshots_detect_information_refreshes_and_civic_identity_changes() {
        let mut state = make_test_campaign();
        let earlier = GameplaySnapshot::capture(&state);
        let player_id = state.player_dynasty_id;
        state
            .information_reports
            .values_mut()
            .find(|report| report.owner_dynasty_id == player_id)
            .expect("player information report must exist")
            .created_day += 1;
        let later = GameplaySnapshot::capture(&state);
        assert!(
            earlier
                .changed_domains(&later)
                .contains(&GameplayDomain::Information)
        );

        let mut identity_change = earlier.clone();
        identity_change.player_office_checksum += 1;
        identity_change.player_civic_contributions = Money::from_copper(150);
        identity_change.player_unmet_office_duties += 1;
        identity_change.active_law_checksum += 1;
        identity_change.player_completed_public_work_checksum += 1;
        let domains = earlier.changed_domains(&identity_change);
        assert!(domains.contains(&GameplayDomain::Institutions));
        assert!(domains.contains(&GameplayDomain::Economy));
        assert!(domains.contains(&GameplayDomain::Dynasty));
        assert!(domains.contains(&GameplayDomain::Law));
        assert!(domains.contains(&GameplayDomain::Districts));

        let mut debt_change = earlier.clone();
        debt_change.current_civic_debts += 1;
        debt_change.total_civic_debt_balance = Money::from_copper(10_000);
        assert!(
            earlier
                .changed_domains(&debt_change)
                .contains(&GameplayDomain::Loans),
            "municipal debt changes must be attributed to the finance domain"
        );
    }

    #[test]
    fn snapshots_attribute_player_lending_distress_separately() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let borrower_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| {
                *dynasty_id != player_id
                    && !same_pair_credit_blocks_new_loan(&state, player_id, *dynasty_id)
            })
            .expect("campaign must contain an unused player-lending counterparty");
        let loan_id = issue_loan(
            &mut state,
            LoanTerms {
                lender_dynasty_id: player_id,
                borrower_dynasty_id: borrower_id,
                principal: Money::from_copper(1_000),
                weekly_payment: Money::from_copper(50),
                interest_basis_points: 700,
                collateral_property_id: None,
            },
        )
        .expect("test player loan must be issuable");
        state
            .loans
            .get_mut(&loan_id)
            .expect("new player loan must exist")
            .status = LoanStatus::Delinquent;
        let unrelated_loan_id = state
            .loans
            .values()
            .find(|loan| loan.id != loan_id && loan.lender_dynasty_id != player_id)
            .expect("campaign must contain an unrelated private loan")
            .id;
        state
            .loans
            .get_mut(&unrelated_loan_id)
            .expect("unrelated loan must exist")
            .status = LoanStatus::Defaulted;

        let snapshot = GameplaySnapshot::capture(&state);

        assert_eq!(snapshot.player_delinquent_lending, 1);
        assert_eq!(snapshot.player_defaulted_lending, 0);
        assert!(snapshot.delinquent_loans >= 1);
        assert!(snapshot.defaulted_loans >= 1);
    }

    #[test]
    fn snapshots_attribute_player_borrowing_distress_separately() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let lender_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| {
                *dynasty_id != player_id
                    && !same_pair_credit_blocks_new_loan(&state, *dynasty_id, player_id)
            })
            .expect("campaign must contain an unused player-borrowing counterparty");
        let loan_id = issue_loan(
            &mut state,
            LoanTerms {
                lender_dynasty_id: lender_id,
                borrower_dynasty_id: player_id,
                principal: Money::from_copper(1_000),
                weekly_payment: Money::from_copper(50),
                interest_basis_points: 700,
                collateral_property_id: None,
            },
        )
        .expect("test player borrowing must be issuable");
        state
            .loans
            .get_mut(&loan_id)
            .expect("new player borrowing must exist")
            .status = LoanStatus::Delinquent;
        let unrelated_loan_id = state
            .loans
            .values()
            .find(|loan| loan.id != loan_id && loan.borrower_dynasty_id != player_id)
            .expect("campaign must contain an unrelated private loan")
            .id;
        state
            .loans
            .get_mut(&unrelated_loan_id)
            .expect("unrelated loan must exist")
            .status = LoanStatus::Defaulted;

        let snapshot = GameplaySnapshot::capture(&state);

        assert_eq!(snapshot.player_delinquent_borrowing, 1);
        assert_eq!(snapshot.player_defaulted_borrowing, 0);
        assert!(snapshot.delinquent_loans >= 1);
        assert!(snapshot.defaulted_loans >= 1);
    }

    #[test]
    fn consequence_profiles_preserve_exact_material_before_and_after_values() {
        let state = make_test_campaign();
        let earlier = GameplaySnapshot::capture(&state);
        let mut later = earlier.clone();
        later.player_treasury = earlier
            .player_treasury
            .checked_add(Money::from_copper(123))
            .expect("fixture treasury change must fit");
        later.player_defaulted_borrowing = 1;
        later.player_active_contracts = earlier.player_active_contracts.saturating_add(1);
        later.player_current_lending = earlier.player_current_lending.saturating_add(1);
        later.building_public_works = earlier.building_public_works.saturating_add(1);

        let profile = GameplayConsequenceProfile::between(&earlier, &later);

        assert_eq!(
            profile.changes.get(&GameplayMeasure::PlayerTreasury),
            Some(&GameplayMeasureChange {
                before: earlier.player_treasury.copper(),
                after: later.player_treasury.copper(),
            })
        );
        assert_eq!(
            profile
                .changes
                .get(&GameplayMeasure::PlayerDefaultedBorrowing),
            Some(&GameplayMeasureChange {
                before: 0,
                after: 1,
            })
        );
        assert_eq!(
            profile.changes.get(&GameplayMeasure::PlayerActiveContracts),
            Some(&GameplayMeasureChange {
                before: i64::from(earlier.player_active_contracts),
                after: i64::from(later.player_active_contracts),
            })
        );
        assert!(
            profile
                .changes
                .contains_key(&GameplayMeasure::PlayerCurrentLending)
        );
        assert!(
            profile
                .changes
                .contains_key(&GameplayMeasure::BuildingPublicWorks)
        );
    }

    #[test]
    fn snapshots_measure_material_district_conditions() {
        let mut state = make_test_campaign();
        let earlier = GameplaySnapshot::capture(&state);
        for district in state.districts.values_mut() {
            district.employment_basis_points = district.employment_basis_points.saturating_add(100);
            district.sanitation_basis_points = district.sanitation_basis_points.saturating_add(100);
            district.safety_basis_points = district.safety_basis_points.saturating_add(100);
        }
        let later = GameplaySnapshot::capture(&state);

        assert!(
            earlier
                .changed_domains(&later)
                .contains(&GameplayDomain::Districts)
        );
        assert_eq!(
            later.average_district_employment,
            earlier.average_district_employment.saturating_add(100)
        );
        assert_eq!(
            later.average_district_sanitation,
            earlier.average_district_sanitation.saturating_add(100)
        );
        assert_eq!(
            later.average_district_safety,
            earlier.average_district_safety.saturating_add(100)
        );
        assert_eq!(
            earlier.district_conditions.len(),
            later.district_conditions.len()
        );
        for (before, after) in earlier
            .district_conditions
            .iter()
            .zip(&later.district_conditions)
        {
            assert_eq!(before.district_id, after.district_id);
            assert_eq!(
                after.employment_basis_points,
                before.employment_basis_points.saturating_add(100)
            );
            assert_eq!(
                after.sanitation_basis_points,
                before.sanitation_basis_points.saturating_add(100)
            );
            assert_eq!(
                after.safety_basis_points,
                before.safety_basis_points.saturating_add(100)
            );
        }
    }

    #[test]
    fn player_borrowing_default_is_reported_as_a_material_experience_problem() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.simulated_days = 720;
        campaign.maximum_player_delinquent_borrowing = 1;
        campaign.maximum_player_defaulted_borrowing = 1;
        campaign.end.player_delinquent_borrowing = 0;
        campaign.end.player_defaulted_borrowing = 1;
        campaign.end.player_treasury = Money::ZERO;
        campaign.end.player_properties = 0;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(
            &findings,
            "Player borrowing enters material credit distress",
        );
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
        assert!(
            finding
                .evidence
                .contains("ended with 0 delinquent, 1 defaulted")
        );
    }

    #[test]
    fn snapshots_detect_external_route_changes_before_market_totals_move() {
        let mut state = make_test_campaign();
        let earlier = GameplaySnapshot::capture(&state);
        state
            .external_routes
            .values_mut()
            .next()
            .expect("campaign must contain an external route")
            .disruption_basis_points += 1;
        let later = GameplaySnapshot::capture(&state);

        assert!(
            earlier
                .changed_domains(&later)
                .contains(&GameplayDomain::Market)
        );
    }

    #[test]
    fn snapshots_attribute_new_player_collateral_to_property() {
        let mut state = make_test_campaign();
        let property_id = state
            .properties
            .values()
            .find(|property| {
                property.owner_dynasty_id == Some(state.player_dynasty_id)
                    && property.collateral_loan_id.is_none()
            })
            .expect("campaign must contain an unpledged player property")
            .id;
        let loan_id = *state
            .loans
            .keys()
            .next()
            .expect("campaign must contain a loan");
        let earlier = GameplaySnapshot::capture(&state);
        state
            .properties
            .get_mut(&property_id)
            .expect("player property must exist")
            .collateral_loan_id = Some(loan_id);
        let later = GameplaySnapshot::capture(&state);

        assert_eq!(
            later.player_pledged_properties,
            earlier.player_pledged_properties.saturating_add(1)
        );
        assert!(
            earlier
                .changed_domains(&later)
                .contains(&GameplayDomain::Property)
        );
    }

    #[test]
    fn snapshots_detect_persistent_audit_history_that_changes_future_routes() {
        let mut state = make_test_campaign();
        let earlier = GameplaySnapshot::capture(&state);
        let business_id = state
            .businesses
            .iter()
            .find(|business| business.owner_dynasty_id() == state.player_dynasty_id)
            .expect("player must own a business")
            .id();
        state.audit_log.push(AuditRecord {
            day: state.clock.day(),
            kind: AuditKind::BusinessPolicyChange,
            subject: format!("business:{business_id}").into(),
            detail: "synthetic cooldown marker".to_owned(),
        });
        let later = GameplaySnapshot::capture(&state);

        assert_ne!(earlier.audit_state_checksum, later.audit_state_checksum);
        assert!(persistent_history_changed(
            &earlier, &later, &later, &earlier
        ));
        assert!(
            !earlier
                .changed_domains(&later)
                .contains(&GameplayDomain::Feedback),
            "internal audit history must not be mislabeled as user-facing feedback"
        );
    }

    #[test]
    fn snapshots_detect_legal_hearing_progress_with_unchanged_case_counts() {
        let mut state = make_test_campaign();
        let mut dynasty_ids = state.dynasties.keys().copied();
        let plaintiff_dynasty_id = dynasty_ids.next().expect("campaign must contain a dynasty");
        let defendant_dynasty_id = dynasty_ids
            .next()
            .expect("campaign must contain a second dynasty");
        let legal_case_id = state.next_ids.legal_case();
        state.legal_cases.insert(
            legal_case_id,
            crate::core::LegalCase {
                id: legal_case_id,
                plaintiff_dynasty_id,
                defendant_dynasty_id,
                kind: LegalCaseKind::ContractBreach,
                claim_source: None,
                evidence_basis_points: 6_500,
                public_attention_basis_points: 2_000,
                filed_day: state.clock.day(),
                hearing_day: state.clock.day().saturating_add(60),
                damages: Money::from_copper(2_500),
                status: LegalCaseStatus::Filed,
            },
        );
        let earlier = GameplaySnapshot::capture(&state);
        state
            .legal_cases
            .get_mut(&legal_case_id)
            .expect("legal case must exist")
            .status = LegalCaseStatus::Hearing;
        let later = GameplaySnapshot::capture(&state);

        assert_eq!(earlier.open_legal_cases, later.open_legal_cases);
        assert!(
            earlier
                .changed_domains(&later)
                .contains(&GameplayDomain::Legal)
        );
    }

    #[test]
    fn probe_budget_preserves_command_family_breadth_before_extra_variants() {
        let candidates = vec![
            Candidate {
                kind: GameplayCommandKind::NominateForOffice,
                command: PlayerCommand::AcknowledgeNotification {
                    message_id: OutboxMessageId::new(1),
                },
                description: "first nomination variant".to_owned(),
                score: 900,
            },
            Candidate {
                kind: GameplayCommandKind::NominateForOffice,
                command: PlayerCommand::AcknowledgeNotification {
                    message_id: OutboxMessageId::new(2),
                },
                description: "second nomination variant".to_owned(),
                score: 850,
            },
            Candidate {
                kind: GameplayCommandKind::EnactLaw,
                command: PlayerCommand::AcknowledgeNotification {
                    message_id: OutboxMessageId::new(3),
                },
                description: "law variant".to_owned(),
                score: 800,
            },
            Candidate {
                kind: GameplayCommandKind::BuyProperty,
                command: PlayerCommand::AcknowledgeNotification {
                    message_id: OutboxMessageId::new(4),
                },
                description: "property variant".to_owned(),
                score: 750,
            },
        ];

        let selected = select_probe_candidates(candidates, 3);
        let kinds: BTreeSet<_> = selected.iter().map(|candidate| candidate.kind).collect();

        let [highest_ranked, _, _] = selected.as_slice() else {
            panic!("probe budget must select exactly three candidates: {selected:#?}");
        };
        assert_eq!(
            kinds,
            [
                GameplayCommandKind::NominateForOffice,
                GameplayCommandKind::EnactLaw,
                GameplayCommandKind::BuyProperty,
            ]
            .into_iter()
            .collect(),
            "probe selection must preserve command-family breadth"
        );
        assert_eq!(
            highest_ranked.kind,
            GameplayCommandKind::NominateForOffice,
            "the highest-ranked candidate must remain first"
        );
    }

    #[test]
    fn notification_housekeeping_is_offered_only_for_a_meaningful_batch() {
        let mut state = make_test_campaign();
        state.outbox.clear();
        for index in 1..NOTIFICATION_BATCH_THRESHOLD {
            let message_id = state.next_ids.outbox();
            state.outbox.push(OutboxMessage {
                id: message_id,
                day: 0,
                kind: OutboxKind::Information,
                subject: format!("message {index}"),
                body: "test".to_owned(),
                acknowledged: false,
            });
        }
        let mut candidates = Vec::new();

        generate_reactive_candidates(&state, GameplayPersona::Steward, &mut candidates);

        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.kind != GameplayCommandKind::AcknowledgeNotification),
            "small notification counts should remain passive information"
        );

        let index = NOTIFICATION_BATCH_THRESHOLD;
        let message_id = state.next_ids.outbox();
        state.outbox.push(OutboxMessage {
            id: message_id,
            day: 0,
            kind: OutboxKind::Information,
            subject: format!("message {index}"),
            body: "test".to_owned(),
            acknowledged: false,
        });
        candidates.clear();

        generate_reactive_candidates(&state, GameplayPersona::Steward, &mut candidates);

        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| candidate.kind == GameplayCommandKind::AcknowledgeNotification)
                .count(),
            1,
            "a meaningful backlog should produce one batched acknowledgement route"
        );
    }

    #[test]
    fn notification_housekeeping_does_not_displace_a_substantive_choice() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_head = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        state
            .characters
            .get_mut(player_head)
            .expect("player head must exist")
            .identity
            .birth_day = state.clock.day().saturating_sub(40 * 360);
        state.outbox.clear();
        for index in 1..=NOTIFICATION_BATCH_THRESHOLD {
            let message_id = state.next_ids.outbox();
            state.outbox.push(OutboxMessage {
                id: message_id,
                day: 0,
                kind: OutboxKind::Information,
                subject: format!("message {index}"),
                body: "test".to_owned(),
                acknowledged: false,
            });
        }
        let mut reactive = Vec::new();
        generate_reactive_candidates(&state, GameplayPersona::Steward, &mut reactive);
        let mut acknowledgement = reactive
            .into_iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::AcknowledgeNotification)
            .expect("notification backlog must create a housekeeping candidate");
        acknowledgement.score = 10_000;
        let mut family = Vec::new();
        generate_family_candidates(registry, &state, GameplayPersona::Steward, &mut family);
        let mut governance = family
            .into_iter()
            .find(|candidate| candidate.kind == GameplayCommandKind::SetHouseGovernance)
            .expect("initial campaign must offer a governance choice");
        governance.score = -10_000;
        let mut accumulator = CampaignAccumulator::new();

        let probe = probe_candidates(
            registry,
            &state,
            [acknowledgement, governance].into_iter(),
            30,
            360,
            &mut accumulator,
        )
        .expect("candidate projection must remain representable");

        assert_eq!(probe.viable_count, 2);
        assert_eq!(probe.substantive_viable_count, 1);
        assert!(probe.rejections.is_empty());
        assert_eq!(
            probe.viable_command_kinds,
            [GameplayCommandKind::SetHouseGovernance]
                .into_iter()
                .collect()
        );
        assert_eq!(
            probe.selected.expect("a candidate must be selected").kind,
            GameplayCommandKind::SetHouseGovernance,
            "housekeeping may be a fallback but must not consume a strategic decision cycle"
        );
    }

    #[test]
    fn automatic_notification_housekeeping_clears_backlog_without_advancing_the_campaign() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_head = state
            .dynasties
            .get(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .head_id();
        state
            .characters
            .get_mut(player_head)
            .expect("player head must exist")
            .identity
            .birth_day = state.clock.day().saturating_sub(40 * 360);
        state.outbox.clear();
        for index in 1..=NOTIFICATION_BATCH_THRESHOLD {
            let message_id = state.next_ids.outbox();
            state.outbox.push(OutboxMessage {
                id: message_id,
                day: 0,
                kind: OutboxKind::Information,
                subject: format!("message {index}"),
                body: "test".to_owned(),
                acknowledged: false,
            });
        }
        let day_before = state.clock.day();
        let mut accumulator = CampaignAccumulator::new();

        apply_notification_housekeeping(registry, &mut state, &mut accumulator)
            .expect("automatic housekeeping must succeed");

        assert_eq!(state.clock.day(), day_before);
        assert!(state.outbox.iter().all(|message| message.acknowledged));
        let command_stats = accumulator
            .commands
            .get(&GameplayCommandKind::AcknowledgeNotification)
            .expect("acknowledgement statistics must exist");
        assert_eq!(command_stats.executed, 1);
        // Housekeeping executes mechanically outside a decision cycle, so it
        // is not credited with measured feedback or persistent consequences;
        // those stay reserved for baseline-compared command families.
        assert_eq!(command_stats.immediate_world_feedback, 0);
        assert_eq!(command_stats.actions_with_feedback, 0);
        assert_eq!(command_stats.actions_with_persistent_consequences, 0);
        assert!(
            command_stats
                .changed_domains
                .contains(&GameplayDomain::Feedback)
        );
        assert!(
            ranked_candidates(registry, &state, GameplayPersona::Steward, &accumulator,)
                .0
                .iter()
                .any(|candidate| candidate.kind == GameplayCommandKind::SetHouseGovernance)
        );
    }

    #[test]
    fn trajectory_food_minimum_excludes_the_bootstrap_baseline() {
        let state = make_test_campaign();
        let initial = GameplaySnapshot::capture(&state);
        assert_eq!(initial.average_food_satisfaction, 8_000);
        let mut accumulator = CampaignAccumulator::new();

        accumulator.observe_initial_snapshot(&initial);

        assert_eq!(
            accumulator.minimum_food_satisfaction,
            u16::MAX,
            "the authored starting value must not mask later trajectory movement"
        );

        let mut later = initial;
        later.average_food_satisfaction = 9_250;
        accumulator.observe_snapshot(&later);

        assert_eq!(accumulator.minimum_food_satisfaction, 9_250);
    }

    #[test]
    fn substantive_command_streaks_ignore_notification_housekeeping() {
        let mut accumulator = CampaignAccumulator::new();
        for day in (0..56).step_by(7) {
            accumulator.record_executed_command(GameplayCommandKind::TransferBusinessCash, day);
        }
        accumulator.record_executed_command(GameplayCommandKind::AcknowledgeNotification, 56);
        accumulator.record_executed_command(GameplayCommandKind::TransferBusinessCash, 63);

        assert_eq!(accumulator.longest_substantive_command_streak, 0);
        assert_eq!(accumulator.longest_substantive_streak_command, None);

        accumulator.record_executed_command(GameplayCommandKind::ExtendCredit, 70);
        assert_eq!(accumulator.current_substantive_command_streak, 1);
    }

    #[test]
    fn substantive_command_streaks_reset_after_a_strategic_pause() {
        let mut accumulator = CampaignAccumulator::new();
        accumulator.record_executed_command(GameplayCommandKind::TransferBusinessCash, 0);
        accumulator.record_executed_command(GameplayCommandKind::TransferBusinessCash, 7);
        accumulator.record_executed_command(
            GameplayCommandKind::TransferBusinessCash,
            7 + SUBSTANTIVE_STREAK_MAX_GAP_DAYS + 1,
        );

        assert_eq!(accumulator.longest_substantive_command_streak, 0);
        assert_eq!(accumulator.current_substantive_command_streak, 0);
    }

    #[test]
    fn substantive_action_gaps_include_quiet_and_housekeeping_cycles() {
        let mut accumulator = CampaignAccumulator::new();
        let snapshot = GameplaySnapshot::capture(&make_test_campaign());

        accumulator.record_action_gap(None, 180, &snapshot);
        accumulator.record_action_gap(
            Some(GameplayCommandKind::AcknowledgeNotification),
            180,
            &snapshot,
        );

        assert_eq!(accumulator.longest_substantive_action_gap_days, 360);
        accumulator.record_action_gap(Some(GameplayCommandKind::EnactLaw), 30, &snapshot);
        assert_eq!(accumulator.current_substantive_action_gap_days, 0);
        assert_eq!(accumulator.longest_substantive_action_gap_days, 360);
    }

    #[test]
    fn asset_rich_liquidity_gaps_require_temporal_overlap() {
        let mut accumulator = CampaignAccumulator::new();
        let mut snapshot = GameplaySnapshot::capture(&make_test_campaign());
        snapshot.player_treasury = Money::from_copper(100);
        snapshot.player_properties = 2;
        snapshot.player_business_cash = Money::from_copper(20_000);
        snapshot.active_businesses = 1;

        accumulator.record_action_gap(None, 180, &snapshot);
        accumulator.record_action_gap(None, 180, &snapshot);

        assert_eq!(accumulator.longest_asset_rich_quiet_gap_days, 360);
        snapshot.player_treasury = Money::from_copper(10_000);
        accumulator.record_action_gap(None, 30, &snapshot);
        assert_eq!(accumulator.current_asset_rich_quiet_gap_days, 0);
        assert_eq!(accumulator.longest_asset_rich_quiet_gap_days, 360);
    }

    #[test]
    fn personas_value_distinct_institutional_powers() {
        assert!(
            office_power_persona_bonus(GameplayPersona::Steward, OfficePower::PublicWorks)
                > office_power_persona_bonus(
                    GameplayPersona::Entrepreneur,
                    OfficePower::PublicWorks,
                )
        );
        assert!(
            office_power_persona_bonus(GameplayPersona::Entrepreneur, OfficePower::MarketTolls)
                > office_power_persona_bonus(GameplayPersona::Steward, OfficePower::MarketTolls)
        );
        assert!(
            office_power_persona_bonus(GameplayPersona::Opportunist, OfficePower::DebtEnforcement)
                > office_power_persona_bonus(
                    GameplayPersona::Steward,
                    OfficePower::DebtEnforcement
                )
        );
        assert!(
            office_power_persona_bonus(GameplayPersona::PowerBroker, OfficePower::Taxation)
                > office_power_persona_bonus(GameplayPersona::Entrepreneur, OfficePower::Taxation)
        );
    }

    #[test]
    fn power_broker_rebuilds_legitimacy_before_lawmaking_becomes_impossible() {
        let mut state = make_test_campaign();
        state
            .dynasties
            .get_mut(&state.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .legitimacy_basis_points = WARD_ADOPTION_LEGITIMACY_REQUIREMENT.saturating_sub(1);

        assert_eq!(
            institution_support_recovery_bonus(&state, true, GameplayPersona::PowerBroker),
            AGENT_POLITICAL_RECOVERY_SUPPORT_BONUS
        );
        assert_eq!(
            institution_support_recovery_bonus(&state, true, GameplayPersona::Entrepreneur),
            0,
            "non-political personas should keep the emergency-only recovery threshold"
        );
    }

    #[test]
    fn personas_prioritize_distinct_family_education() {
        assert!(
            education_focus_persona_bonus(GameplayPersona::Steward, EducationFocus::Administration)
                > education_focus_persona_bonus(GameplayPersona::Steward, EducationFocus::Craft)
        );
        assert!(
            education_focus_persona_bonus(GameplayPersona::Entrepreneur, EducationFocus::Commerce)
                > education_focus_persona_bonus(
                    GameplayPersona::Entrepreneur,
                    EducationFocus::Social
                )
        );
        assert!(
            education_focus_persona_bonus(GameplayPersona::PowerBroker, EducationFocus::Social)
                > education_focus_persona_bonus(
                    GameplayPersona::PowerBroker,
                    EducationFocus::Craft
                )
        );
    }

    #[test]
    fn snapshots_attribute_ward_adoption_and_education_to_family_state() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        {
            let player = state
                .dynasties
                .get_mut(&player_id)
                .expect("player dynasty must exist");
            player.resources.treasury = Money::from_copper(30_000);
            player.resources.legitimacy_basis_points = WARD_ADOPTION_LEGITIMACY_REQUIREMENT;
            player.resources.reputation_reliability_basis_points =
                WARD_ADOPTION_REPUTATION_REQUIREMENT;
        }
        grant_office_nomination_record_for_test(registry, &mut state);
        let before = GameplaySnapshot::capture(&state);

        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::AdoptWard {
                focus: EducationFocus::Administration,
            },
        )
        .expect("ward adoption must succeed in the prepared fixture");
        let after_adoption = GameplaySnapshot::capture(&state);

        assert_eq!(after_adoption.active_wards, before.active_wards + 1);
        assert_eq!(
            after_adoption.eligible_officeholders,
            before.eligible_officeholders + 1
        );
        assert!(
            before
                .changed_domains(&after_adoption)
                .contains(&GameplayDomain::Family)
        );

        advance_days(
            registry,
            &mut state,
            u32::try_from(crate::systems::FAMILY_EDUCATION_INTERVAL_DAYS)
                .expect("family education interval must fit simulation API"),
        )
        .expect("campaign must advance through the education interval");
        let ward_id = state
            .family_links
            .values()
            .find(|link| link.kind == FamilyLinkKind::Ward)
            .expect("ward link must exist")
            .second_character_id;
        apply_player_command(
            registry,
            &mut state,
            PlayerCommand::EducateFamilyMember {
                character_id: ward_id,
                focus: EducationFocus::Social,
            },
        )
        .expect("ward education must succeed after the interval");
        let after_education = GameplaySnapshot::capture(&state);

        assert_ne!(
            after_adoption.player_family_capability_checksum,
            after_education.player_family_capability_checksum
        );
        assert!(
            after_adoption
                .changed_domains(&after_education)
                .contains(&GameplayDomain::Family)
        );
    }

    #[test]
    fn persona_outcome_divergence_distinguishes_shared_command_families() {
        let base = cached_focused_report(30)
            .campaigns
            .first()
            .expect("focused configuration must produce a campaign")
            .clone();
        let mut campaigns = Vec::new();
        for (index, persona) in [
            GameplayPersona::Steward,
            GameplayPersona::Entrepreneur,
            GameplayPersona::PowerBroker,
            GameplayPersona::Opportunist,
        ]
        .into_iter()
        .enumerate()
        {
            let mut campaign = base.clone();
            campaign.persona = persona;
            campaign.end.player_office_checksum = i64::try_from(index + 1).unwrap_or(i64::MAX);
            campaign.end.player_family_capability_checksum =
                u32::try_from((index + 1) * 1_000).unwrap_or(u32::MAX);
            campaigns.push(campaign);
        }

        assert!(persona_outcomes_diverge(&campaigns, |campaign| {
            campaign.end.player_office_checksum
        }));
        assert!(persona_outcomes_diverge(&campaigns, |campaign| {
            campaign.end.player_family_capability_checksum
        }));

        for campaign in &mut campaigns {
            campaign.end.player_office_checksum = 7;
        }
        assert!(!persona_outcomes_diverge(&campaigns, |campaign| {
            campaign.end.player_office_checksum
        }));
    }

    #[test]
    fn foundational_and_reactive_commands_do_not_define_persona_identity() {
        assert!(!is_persona_identity_command(
            GameplayCommandKind::SecureSupply
        ));
        assert!(!is_persona_identity_command(
            GameplayCommandKind::BuyProperty
        ));
        assert!(!is_persona_identity_command(
            GameplayCommandKind::BorrowFunds
        ));
        assert!(!is_persona_identity_command(
            GameplayCommandKind::RespondToCrisis
        ));
        assert!(!is_persona_identity_command(
            GameplayCommandKind::ResolveLaborDispute
        ));
        assert!(!is_persona_identity_command(
            GameplayCommandKind::AcknowledgeNotification
        ));
        assert!(is_persona_identity_command(
            GameplayCommandKind::SetBusinessPolicy
        ));
        assert!(is_persona_identity_command(
            GameplayCommandKind::NominateForOffice
        ));
    }

    #[test]
    fn fantasy_arc_records_commercial_political_and_dynastic_milestones() {
        let state = make_test_campaign();
        let mut snapshot = GameplaySnapshot::capture(&state);
        let mut accumulator = CampaignAccumulator::new();
        accumulator.observe_initial_snapshot(&snapshot);

        snapshot.day = 70;
        snapshot.quality_reputation = COMMERCIAL_STANDING_REPUTATION_REQUIREMENT;
        accumulator.observe_snapshot(&snapshot);

        accumulator.record_executed_candidate(
            GameplayCommandKind::CultivateInstitutionSupport,
            &PlayerCommand::CultivateInstitutionSupport {
                institution_id: InstitutionId::new(3),
                character_id: CharacterId::new(1),
            },
            180,
        );

        snapshot.day = 360;
        snapshot.player_contract_deliveries = OFFICE_NOMINATION_DELIVERY_REQUIREMENT;
        accumulator.observe_snapshot(&snapshot);
        accumulator.record_executed_candidate(
            GameplayCommandKind::NominateForOffice,
            &PlayerCommand::NominateForOffice {
                institution_id: InstitutionId::new(5),
                character_id: CharacterId::new(1),
            },
            377,
        );

        snapshot.day = 440;
        snapshot.offices_held = 1;
        accumulator.observe_snapshot(&snapshot);
        accumulator.record_executed_command(GameplayCommandKind::StartPublicWork, 454);

        snapshot.day = 900;
        snapshot.player_disputed_employment = 1;
        snapshot.family_unity = 8_000;
        snapshot.legitimacy = 4_000;
        snapshot.offices_held = 3;
        snapshot.institution_memberships = 3;
        snapshot.player_institutions_represented = 3;
        accumulator.observe_snapshot(&snapshot);
        accumulator.record_executed_command(GameplayCommandKind::DesignateHeir, 1_200);

        snapshot.day = 5_200;
        snapshot.generation = snapshot.generation.saturating_add(1);
        snapshot.family_unity = 6_500;
        snapshot.legitimacy = 3_000;
        snapshot.offices_held = 1;
        snapshot.institution_memberships = 1;
        snapshot.player_institutions_represented = 1;
        accumulator.observe_snapshot(&snapshot);

        assert_eq!(
            accumulator.fantasy_arc,
            GameplayFantasyArc {
                first_reputation_standing_day: Some(70),
                first_commercial_standing_day: Some(360),
                first_institution_support_day: Some(180),
                first_institution_support_target: Some(InstitutionId::new(3)),
                first_office_campaign_day: Some(377),
                first_office_campaign_target: Some(InstitutionId::new(5)),
                first_office_day: Some(440),
                first_city_shaping_action_day: Some(454),
                first_city_shaping_command: Some(GameplayCommandKind::StartPublicWork),
                first_player_labor_dispute_day: Some(900),
                first_heir_designation_day: Some(1_200),
                first_succession_day: Some(5_200),
            }
        );
        assert_eq!(
            accumulator.succession_transition,
            Some(GameplaySuccessionTransition {
                day: 5_200,
                family_unity_before: 8_000,
                family_unity_after: 6_500,
                legitimacy_before: 4_000,
                legitimacy_after: 3_000,
                offices_before: 3,
                offices_after: 1,
                institution_memberships_before: 3,
                institution_memberships_after: 1,
                represented_institutions_before: 3,
                represented_institutions_after: 1,
            })
        );
    }
}

mod findings {
    use super::*;

    #[test]
    fn politically_stranded_succession_is_reported_until_rebuild_begins() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.simulated_days = 7_200;
        campaign.end.day = 7_200;
        campaign.end.legitimacy = 0;
        campaign.end.offices_held = 1;
        campaign.end.player_institutions_represented = 1;
        campaign.succession_transition = Some(GameplaySuccessionTransition {
            day: 5_400,
            family_unity_before: 8_000,
            family_unity_after: 6_500,
            legitimacy_before: 2_000,
            legitimacy_after: 600,
            offices_before: 3,
            offices_after: 1,
            institution_memberships_before: 3,
            institution_memberships_after: 1,
            represented_institutions_before: 3,
            represented_institutions_after: 1,
        });
        let phase = campaign
            .phase_stats
            .entry(GameplayPhase::SuccessionLegacy)
            .or_default();
        phase.executed_commands.clear();

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "Political succession can strand institutional recovery",
        );
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);

        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.end.legitimacy = 250;
        campaign
            .phase_stats
            .entry(GameplayPhase::SuccessionLegacy)
            .or_default()
            .executed_commands
            .insert(GameplayCommandKind::CultivateInstitutionSupport, 1);
        let findings = derive_findings(&report.aggregate, &report.campaigns);
        assert_finding_absent(
            &findings,
            "Political succession can strand institutional recovery",
        );
    }

    #[test]
    fn background_specific_expansion_ceiling_is_reported() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        let mut campaigns = Vec::new();
        for (background, generated) in [
            (StartingBackground::Blacksmith, 0),
            (StartingBackground::Baker, 1),
        ] {
            for offset in 0..4_u64 {
                let mut campaign = baseline.clone();
                campaign.seed = offset + 1;
                campaign.background = background;
                campaign.simulated_days = 3_600;
                campaign
                    .commands
                    .get_mut(&GameplayCommandKind::BuyProperty)
                    .expect("property command stats must exist")
                    .generated = generated;
                campaigns.push(campaign);
            }
        }
        report.campaigns = campaigns;
        report.aggregate.campaigns = 8;
        report.aggregate.simulated_days = 28_800;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Blacksmith background never exposes buy-property",
        );
    }

    #[test]
    fn mature_private_defaults_without_courts_are_reported() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = [1_u64, 2]
            .into_iter()
            .map(|seed| {
                let mut campaign = baseline.clone();
                campaign.seed = seed;
                campaign.simulated_days = 3_600;
                campaign.maximum_defaulted_loans = 1;
                campaign
            })
            .collect();
        report.aggregate.campaigns = 2;
        report.aggregate.simulated_days = 7_200;
        report
            .aggregate
            .causal_domain_changes
            .insert(GameplayDomain::Legal, 0);
        report
            .aggregate
            .ambient_domain_changes
            .insert(GameplayDomain::Legal, 0);

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Defaulted private debt never reaches institutional enforcement",
        );
    }

    #[test]
    fn legal_activity_satisfies_private_debt_enforcement_ecosystem_check() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = [1_u64, 2]
            .into_iter()
            .map(|seed| {
                let mut campaign = baseline.clone();
                campaign.seed = seed;
                campaign.simulated_days = 3_600;
                campaign.maximum_defaulted_loans = 1;
                campaign
            })
            .collect();
        report.aggregate.campaigns = 2;
        report.aggregate.simulated_days = 7_200;
        report
            .aggregate
            .ambient_domain_changes
            .insert(GameplayDomain::Legal, 1);

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert!(findings.iter().all(|finding| {
            finding.title != "Defaulted private debt never reaches institutional enforcement"
        }));
    }

    #[test]
    fn office_directives_without_later_effects_are_reported() {
        let mut report = cached_focused_report(30);
        report.aggregate.campaigns = 1;
        report.aggregate.simulated_days = 1_800;
        let stats = report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::ExerciseOfficePower)
            .expect("office-power statistics must exist");
        stats.executed = 20;
        stats.actions_with_delayed_consequences = 0;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Office directives rarely alter the later trajectory",
        );
    }

    #[test]
    fn flat_local_welfare_under_crisis_is_reported() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = (0..4)
            .map(|offset| {
                let mut campaign = baseline.clone();
                campaign.seed = campaign.seed.saturating_add(offset);
                campaign.simulated_days = 1_800;
                campaign.maximum_active_crises = 2;
                campaign.observed_crisis_kinds = BTreeSet::from([CrisisKind::Epidemic]);
                campaign.minimum_district_food_satisfaction = 9_800;
                campaign
            })
            .collect();
        report.aggregate.campaigns = 4;
        report.aggregate.simulated_days = 7_200;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Crises leave household welfare almost mechanically flat",
        );
    }

    #[test]
    fn non_food_crises_do_not_trigger_food_welfare_warning() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = (0..4)
            .map(|offset| {
                let mut campaign = baseline.clone();
                campaign.seed = campaign.seed.saturating_add(offset);
                campaign.simulated_days = 1_800;
                campaign.maximum_active_crises = 2;
                campaign.observed_crisis_kinds =
                    BTreeSet::from([CrisisKind::UrbanFire, CrisisKind::NobleDemand]);
                campaign.minimum_district_food_satisfaction = 9_900;
                campaign
            })
            .collect();
        report.aggregate.campaigns = 4;
        report.aggregate.simulated_days = 7_200;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert!(findings.iter().all(|finding| {
            finding.title != "Crises leave household welfare almost mechanically flat"
        }));
    }

    #[test]
    fn materially_convergent_cities_are_reported_despite_civic_identity_variation() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = [1_u64, 2]
            .into_iter()
            .flat_map(|seed| {
                [
                    GameplayPersona::Steward,
                    GameplayPersona::Entrepreneur,
                    GameplayPersona::PowerBroker,
                    GameplayPersona::Opportunist,
                ]
                .into_iter()
                .enumerate()
                .map({
                    let baseline = baseline.clone();
                    move |(index, persona)| {
                        let offset = u16::try_from(index).expect("persona index must fit") * 20;
                        let mut campaign = baseline.clone();
                        campaign.seed = seed;
                        campaign.persona = persona;
                        campaign.background = StartingBackground::Baker;
                        campaign.simulated_days = 3_600;
                        campaign.end.active_law_checksum = i64::try_from(index).unwrap_or(0) + 1;
                        campaign.end.average_food_satisfaction = 9_800 + offset;
                        campaign.end.average_district_unrest = 700 + offset;
                        campaign.end.average_district_employment = 6_000 + offset;
                        campaign.end.average_district_sanitation = 7_000 + offset;
                        campaign.end.average_district_safety = 7_500 + offset;
                        campaign
                    }
                })
            })
            .collect();
        report.aggregate.campaigns = 8;
        report.aggregate.simulated_days = 28_800;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(
            &findings,
            "Different civic strategies converge on similar material city conditions",
        );
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn mature_reports_surface_structural_district_employment_collapse() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = (1_u64..=4)
            .map(|seed| {
                let mut campaign = baseline.clone();
                campaign.seed = seed;
                campaign.simulated_days = 1_080;
                campaign.start.average_district_employment = 7_200;
                campaign.end.average_district_employment = 3_000;
                campaign
            })
            .collect();
        report.aggregate.campaigns = 4;
        report.aggregate.simulated_days = 4_320;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(
            &findings,
            "District employment collapses from the campaign baseline",
        );
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
        finding_with_title(&findings, "District employment remains structurally weak");
    }

    #[test]
    fn localized_civic_divergence_is_not_erased_by_citywide_averages() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = [1_u64, 2]
            .into_iter()
            .flat_map(|seed| {
                [
                    GameplayPersona::Steward,
                    GameplayPersona::Entrepreneur,
                    GameplayPersona::PowerBroker,
                    GameplayPersona::Opportunist,
                ]
                .into_iter()
                .enumerate()
                .map({
                    let baseline = baseline.clone();
                    move |(index, persona)| {
                        let mut campaign = baseline.clone();
                        campaign.seed = seed;
                        campaign.persona = persona;
                        campaign.background = StartingBackground::Baker;
                        campaign.simulated_days = 3_600;
                        campaign.end.active_law_checksum = i64::try_from(index).unwrap_or(0) + 1;
                        campaign.end.average_food_satisfaction = 9_900;
                        campaign.end.average_district_unrest = 700;
                        campaign.end.average_district_employment = 6_000;
                        campaign.end.average_district_sanitation = 7_000;
                        campaign.end.average_district_safety = 7_500;
                        let local_offset =
                            u16::try_from(index).expect("persona index must fit") * 250;
                        let district = campaign
                            .end
                            .district_conditions
                            .first_mut()
                            .expect("campaign snapshot must contain district conditions");
                        district.employment_basis_points = district
                            .employment_basis_points
                            .saturating_add(local_offset);
                        district.sanitation_basis_points = district
                            .sanitation_basis_points
                            .saturating_add(local_offset);
                        campaign
                    }
                })
            })
            .collect();
        report.aggregate.campaigns = 8;
        report.aggregate.simulated_days = 28_800;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(
            &findings,
            "Different civic strategies converge on similar material city conditions",
        );
    }

    #[test]
    fn generation_length_credit_without_distress_is_reported() {
        let mut report = cached_focused_report(30);
        report.aggregate.campaigns = 1;
        report.aggregate.simulated_days = 3_600;
        report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::ExtendCredit)
            .expect("credit statistics must exist")
            .executed = 50;
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.simulated_days = 3_600;
        campaign.maximum_delinquent_loans = 1;
        campaign.maximum_defaulted_loans = 0;
        campaign.maximum_player_delinquent_lending = 0;
        campaign.maximum_player_defaulted_lending = 0;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Long-horizon player lending never encounters credit distress",
        );
    }

    #[test]
    fn deep_audit_reports_missing_credit_enforcement_coverage() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = (0_u64..3)
            .map(|seed_offset| {
                let mut campaign = baseline.clone();
                campaign.seed = campaign.seed.saturating_add(seed_offset);
                campaign.persona = GameplayPersona::Opportunist;
                campaign.simulated_days = 7_200;
                campaign.maximum_player_delinquent_lending = u16::from(seed_offset == 0);
                campaign
                    .commands
                    .get_mut(&GameplayCommandKind::ExtendCredit)
                    .expect("credit statistics must exist")
                    .executed = 2;
                campaign
                    .commands
                    .get_mut(&GameplayCommandKind::FileLegalCase)
                    .expect("legal statistics must exist")
                    .executed = 0;
                campaign
            })
            .collect();
        report.aggregate.campaigns = 3;
        report.aggregate.simulated_days = 21_600;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(
            &findings,
            "Player credit distress never reaches enforcement",
        );
        assert!(
            finding
                .evidence
                .contains("3 long-horizon opportunist campaigns"),
            "coverage finding should state how much stress-policy evidence was available: {}",
            finding.evidence
        );
        assert!(
            finding
                .evidence
                .contains("1 campaign(s) recorded delinquency"),
            "coverage finding should distinguish player-issued distress from unrelated private loans: {}",
            finding.evidence
        );

        report
            .campaigns
            .first_mut()
            .expect("coverage report must contain a campaign")
            .player_debt_enforcement_cases = 1;
        let findings = derive_findings(&report.aggregate, &report.campaigns);
        assert_finding_absent(
            &findings,
            "Player credit distress never reaches enforcement",
        );
    }

    #[test]
    fn credit_risk_horizon_does_not_overinterpret_ten_year_perfect_repayment() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = (0_u64..3)
            .map(|seed_offset| {
                let mut campaign = baseline.clone();
                campaign.seed = campaign.seed.saturating_add(seed_offset);
                campaign.persona = GameplayPersona::Opportunist;
                campaign.simulated_days = 3_600;
                campaign
                    .commands
                    .get_mut(&GameplayCommandKind::ExtendCredit)
                    .expect("credit statistics must exist")
                    .executed = 6;
                campaign
            })
            .collect();
        report.aggregate.campaigns = 3;
        report.aggregate.simulated_days = 10_800;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(
            &findings,
            "Risk-seeking player lending never becomes distressed",
        );

        for campaign in &mut report.campaigns {
            campaign.simulated_days = 7_200;
        }
        report.aggregate.simulated_days = 21_600;
        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "Risk-seeking player lending never becomes distressed",
        );
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn mature_lending_without_borrower_business_effects_is_reported() {
        let mut report = cached_focused_report(30);
        report.aggregate.campaigns = 1;
        report.aggregate.simulated_days = 3_600;
        report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::ExtendCredit)
            .expect("credit statistics must exist")
            .executed = 20;
        report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign")
            .simulated_days = 3_600;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Player lending is detached from productive financing",
        );

        report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::ExtendCredit)
            .expect("credit statistics must exist")
            .productive_financing_actions = 20;
        let findings = derive_findings(&report.aggregate, &report.campaigns);
        assert_finding_absent(
            &findings,
            "Player lending is detached from productive financing",
        );
    }

    #[test]
    fn findings_surface_political_power_that_precedes_commercial_standing() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.fantasy_arc.first_commercial_standing_day = Some(90);
        campaign.fantasy_arc.first_office_campaign_day = Some(30);

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(&findings, "Political ascent precedes commercial standing");
    }

    #[test]
    fn findings_surface_synchronized_core_fantasy_timing() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = [
            GameplayPersona::Steward,
            GameplayPersona::Entrepreneur,
            GameplayPersona::PowerBroker,
            GameplayPersona::Opportunist,
        ]
        .into_iter()
        .map(|persona| {
            let mut campaign = baseline.clone();
            campaign.persona = persona;
            campaign.fantasy_arc.first_commercial_standing_day = Some(70);
            campaign.fantasy_arc.first_institution_support_day = Some(70);
            campaign.fantasy_arc.first_office_campaign_day = Some(70);
            campaign.fantasy_arc.first_office_day = Some(154);
            campaign.fantasy_arc.first_city_shaping_action_day = Some(154);
            campaign
        })
        .collect();

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(&findings, "Core fantasy timing is highly synchronized");
    }

    #[test]
    fn synchronization_finding_detects_same_background_persona_railroading() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = [
            GameplayPersona::Steward,
            GameplayPersona::Entrepreneur,
            GameplayPersona::PowerBroker,
            GameplayPersona::Opportunist,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, persona)| {
            let offset = i64::try_from(index).expect("persona index must fit i64") * 20;
            let mut campaign = baseline.clone();
            campaign.persona = persona;
            campaign.background = StartingBackground::Baker;
            campaign.fantasy_arc.first_commercial_standing_day = Some(600 + offset);
            campaign.fantasy_arc.first_institution_support_day = Some(420 + offset);
            campaign.fantasy_arc.first_office_campaign_day = Some(630 + offset);
            campaign.fantasy_arc.first_office_day = Some(720 + offset);
            campaign.fantasy_arc.first_city_shaping_action_day = Some(840 + offset * 3);
            campaign
        })
        .collect();

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(&findings, "Core fantasy timing is highly synchronized");
        assert!(
            finding.evidence.contains("same-seed, same-background"),
            "diagnostic should explain that starts are compared like-for-like: {}",
            finding.evidence
        );
    }

    #[test]
    fn synchronized_timing_with_distinct_routes_is_informational() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns = [
            GameplayPersona::Steward,
            GameplayPersona::Entrepreneur,
            GameplayPersona::PowerBroker,
            GameplayPersona::Opportunist,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, persona)| {
            let mut campaign = baseline.clone();
            campaign.persona = persona;
            campaign.background = StartingBackground::Baker;
            campaign.fantasy_arc.first_commercial_standing_day = Some(600);
            campaign.fantasy_arc.first_institution_support_day = Some(420);
            campaign.fantasy_arc.first_office_campaign_day = Some(600);
            campaign.fantasy_arc.first_office_day = Some(720);
            campaign.fantasy_arc.first_city_shaping_action_day = Some(870);
            let institution_id = InstitutionId::new(
                u32::try_from(index).expect("persona index must fit institution ID") + 1,
            );
            campaign.fantasy_arc.first_institution_support_target = Some(institution_id);
            campaign.fantasy_arc.first_office_campaign_target = Some(institution_id);
            campaign.fantasy_arc.first_city_shaping_command = Some(match persona {
                GameplayPersona::Steward => GameplayCommandKind::StartPublicWork,
                GameplayPersona::Entrepreneur | GameplayPersona::Opportunist => {
                    GameplayCommandKind::ExerciseOfficePower
                }
                GameplayPersona::PowerBroker => GameplayCommandKind::EnactLaw,
            });
            campaign
        })
        .collect();

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Fantasy timing converges across distinct political routes",
        );
        assert!(
            findings
                .iter()
                .all(|finding| finding.title != "Core fantasy timing is highly synchronized")
        );
    }

    #[test]
    fn findings_surface_long_repetitive_command_streaks() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.longest_substantive_command_streak = 8;
        campaign.longest_substantive_streak_command =
            Some(GameplayCommandKind::TransferBusinessCash);

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Repeated command streak resembles routine micromanagement",
        );
    }

    #[test]
    fn findings_surface_year_long_action_droughts() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 720;
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.simulated_days = 720;
        campaign.longest_substantive_action_gap_days = 360;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Long stretches pass without a substantive player decision",
        );
    }

    #[test]
    fn findings_surface_owned_wealth_liquidity_droughts() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.longest_asset_rich_quiet_gap_days = 360;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(&findings, "Owned wealth can become decision-poor");
    }

    #[test]
    fn findings_surface_operational_liquidity_dominance_separately_from_strategy() {
        let mut report = cached_focused_report(30);
        report.aggregate.successful_actions = 40;
        report
            .aggregate
            .commands
            .entry(GameplayCommandKind::TransferBusinessCash)
            .or_default()
            .executed = 12;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(
            &findings,
            "Operational liquidity management dominates player decisions",
        );
        assert_eq!(finding.severity, GameplayFindingSeverity::Info);
        assert!(
            finding
                .evidence
                .contains("Portfolio cash rebalancing accounted for 12 of")
        );
    }

    #[test]
    fn findings_surface_mature_liquidity_without_financial_pressure() {
        let report = cached_focused_report(30);
        let template = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        let campaigns: Vec<_> = (1_u64..=4)
            .map(|seed| {
                let mut campaign = template.clone();
                campaign.seed = seed;
                campaign.simulated_days = 3_600;
                campaign.start.player_treasury = Money::from_copper(37_000);
                campaign.end.player_treasury = Money::from_copper(250_000);
                campaign.maximum_player_delinquent_borrowing = 0;
                campaign.maximum_player_defaulted_borrowing = 0;
                campaign
                    .commands
                    .get_mut(&GameplayCommandKind::SellProperty)
                    .expect("every campaign tracks property liquidation")
                    .executed = 0;
                campaign
            })
            .collect();
        let mut findings = Vec::new();

        add_mature_capital_pressure_finding(&campaigns, &mut findings);

        let finding = finding_with_title(
            &findings,
            "Mature liquidity can outgrow meaningful financial pressure",
        );
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn findings_surface_starting_trade_hidden_difficulty() {
        let report = cached_focused_report(30);
        let template = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        let mut campaigns = Vec::new();
        for (background, treasury) in [
            (StartingBackground::Baker, Money::from_copper(320_000)),
            (StartingBackground::Blacksmith, Money::from_copper(120_000)),
        ] {
            for seed in 1_u64..=4 {
                let mut campaign = template.clone();
                campaign.seed = seed;
                campaign.background = background;
                campaign.simulated_days = 3_600;
                campaign.end.player_treasury = treasury;
                campaigns.push(campaign);
            }
        }
        let mut findings = Vec::new();

        add_starting_trade_economic_balance_finding(&campaigns, &mut findings);

        let finding = finding_with_title(
            &findings,
            "Starting trade behaves like a hidden mature-economy advantage",
        );
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn phase_findings_surface_quiet_establishment_and_governance() {
        let mut report = cached_focused_report(30);
        report.aggregate.phase_stats.insert(
            GameplayPhase::Establishment,
            GameplayPhaseStats {
                decision_cycles: 100,
                substantive_actions: 55,
                institutional_campaign_actions: 0,
                quiet_cycles: 45,
                quiet_cycles_with_ambient_change: 0,
                longest_quiet_streak_cycles: 4,
                blocked_cycles: 0,
                cycles_with_multiple_viable_command_kinds: 20,
                cycles_with_close_viable_command_kinds: 0,
                cycles_with_distinct_immediate_consequences: 0,
                cycles_with_distinct_projected_consequences: 0,
                cycles_with_multiple_viable_options: 0,
                cycles_with_close_viable_options: 0,
                cycles_with_distinct_immediate_option_consequences: 0,
                cycles_with_distinct_projected_option_consequences: 0,
                total_viable_choices: 80,
                total_viable_command_kinds: 80,
                executed_commands: BTreeMap::new(),
                ..GameplayPhaseStats::default()
            },
        );
        report.aggregate.phase_stats.insert(
            GameplayPhase::DynasticGovernance,
            GameplayPhaseStats {
                decision_cycles: 100,
                substantive_actions: 65,
                institutional_campaign_actions: 0,
                quiet_cycles: 35,
                quiet_cycles_with_ambient_change: 0,
                longest_quiet_streak_cycles: 3,
                blocked_cycles: 0,
                cycles_with_multiple_viable_command_kinds: 29,
                cycles_with_close_viable_command_kinds: 0,
                cycles_with_distinct_immediate_consequences: 0,
                cycles_with_distinct_projected_consequences: 0,
                cycles_with_multiple_viable_options: 0,
                cycles_with_close_viable_options: 0,
                cycles_with_distinct_immediate_option_consequences: 0,
                cycles_with_distinct_projected_option_consequences: 0,
                total_viable_choices: 110,
                total_viable_command_kinds: 110,
                executed_commands: BTreeMap::new(),
                ..GameplayPhaseStats::default()
            },
        );

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(&findings, "Establishment becomes a waiting phase");
        finding_with_title(
            &findings,
            "Dynastic governance remains intermittent and strategically narrow",
        );
    }

    #[test]
    fn governance_multi_family_share_is_measured_on_actionable_cycles() {
        let mut report = cached_focused_report(30);
        report.aggregate.phase_stats.insert(
            GameplayPhase::DynasticGovernance,
            GameplayPhaseStats {
                decision_cycles: 100,
                substantive_actions: 72,
                institutional_campaign_actions: 0,
                quiet_cycles: 28,
                quiet_cycles_with_ambient_change: 28,
                longest_quiet_streak_cycles: 2,
                blocked_cycles: 0,
                cycles_with_multiple_viable_command_kinds: 22,
                cycles_with_close_viable_command_kinds: 20,
                cycles_with_distinct_immediate_consequences: 22,
                cycles_with_distinct_projected_consequences: 22,
                cycles_with_multiple_viable_options: 72,
                cycles_with_close_viable_options: 20,
                cycles_with_distinct_immediate_option_consequences: 72,
                cycles_with_distinct_projected_option_consequences: 72,
                total_viable_choices: 324,
                total_viable_command_kinds: 150,
                executed_commands: BTreeMap::new(),
                ..GameplayPhaseStats::default()
            },
        );

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(
            &findings,
            "Dynastic governance remains intermittent and strategically narrow",
        );
    }

    #[test]
    fn governance_target_depth_cannot_substitute_for_cross_system_breadth() {
        let mut report = cached_focused_report(30);
        report.aggregate.phase_stats.insert(
            GameplayPhase::DynasticGovernance,
            GameplayPhaseStats {
                decision_cycles: 100,
                substantive_actions: 72,
                institutional_campaign_actions: 0,
                quiet_cycles: 28,
                quiet_cycles_with_ambient_change: 28,
                longest_quiet_streak_cycles: 2,
                blocked_cycles: 0,
                cycles_with_multiple_viable_command_kinds: 30,
                cycles_with_close_viable_command_kinds: 20,
                cycles_with_distinct_immediate_consequences: 30,
                cycles_with_distinct_projected_consequences: 30,
                cycles_with_multiple_viable_options: 72,
                cycles_with_close_viable_options: 20,
                cycles_with_distinct_immediate_option_consequences: 72,
                cycles_with_distinct_projected_option_consequences: 72,
                total_viable_choices: 360,
                total_viable_command_kinds: 110,
                executed_commands: BTreeMap::new(),
                ..GameplayPhaseStats::default()
            },
        );

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(
            &findings,
            "Dynastic governance remains intermittent and strategically narrow",
        );
        assert!(
            finding
                .evidence
                .contains("average family breadth 1.5 < 1.6 families"),
            "mature phase diagnostics must report cross-system family breadth even when many concrete targets are viable: {}",
            finding.evidence
        );
    }

    #[test]
    fn governance_quiet_streak_warns_only_beyond_annual_commitment_cadence() {
        let mut report = cached_focused_report(30);
        report.aggregate.phase_stats.insert(
            GameplayPhase::DynasticGovernance,
            GameplayPhaseStats {
                decision_cycles: 100,
                substantive_actions: 70,
                institutional_campaign_actions: 0,
                quiet_cycles: 30,
                quiet_cycles_with_ambient_change: 30,
                longest_quiet_streak_cycles: 12,
                blocked_cycles: 0,
                cycles_with_multiple_viable_command_kinds: 40,
                cycles_with_close_viable_command_kinds: 20,
                cycles_with_distinct_immediate_consequences: 40,
                cycles_with_distinct_projected_consequences: 40,
                cycles_with_multiple_viable_options: 70,
                cycles_with_close_viable_options: 20,
                cycles_with_distinct_immediate_option_consequences: 70,
                cycles_with_distinct_projected_option_consequences: 70,
                total_viable_choices: 300,
                total_viable_command_kinds: 160,
                executed_commands: BTreeMap::new(),
                ..GameplayPhaseStats::default()
            },
        );
        report
            .campaigns
            .first_mut()
            .expect("focused report must contain one campaign")
            .phase_stats
            .entry(GameplayPhase::DynasticGovernance)
            .or_default()
            .longest_quiet_streak_cycles = 12;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(
            &findings,
            "Dynastic governance remains intermittent and strategically narrow",
        );
        assert!(
            finding
                .evidence
                .contains("Thresholds missed: longest quiet streak 12 > 11 cycles"),
            "phase warning must identify the exact failed quality gate: {}",
            finding.evidence
        );
        assert!(
            finding
                .evidence
                .contains("Worst uninterrupted quiet streak: 12 cycles in seed"),
            "phase warning must identify the campaign that produced the longest drought: {}",
            finding.evidence
        );

        report
            .aggregate
            .phase_stats
            .get_mut(&GameplayPhase::DynasticGovernance)
            .expect("governance phase statistics must exist")
            .longest_quiet_streak_cycles = 11;
        report
            .campaigns
            .first_mut()
            .expect("focused report must contain one campaign")
            .phase_stats
            .entry(GameplayPhase::DynasticGovernance)
            .or_default()
            .longest_quiet_streak_cycles = 11;
        let findings = derive_findings(&report.aggregate, &report.campaigns);
        assert_finding_absent(
            &findings,
            "Dynastic governance remains intermittent and strategically narrow",
        );
    }

    #[test]
    fn findings_surface_campaign_administration_dominance() {
        let mut report = cached_focused_report(30);
        report.aggregate.substantive_actions = 100;
        report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::CultivateInstitutionSupport)
            .expect("support statistics must exist")
            .executed = 20;
        report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::NominateForOffice)
            .expect("nomination statistics must exist")
            .executed = 20;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Institutional campaigning dominates the decision loop",
        );
    }

    #[test]
    fn findings_surface_repetitive_public_work_portfolios() {
        let mut report = cached_focused_report(30);
        let template = report
            .campaigns
            .first()
            .expect("focused report must contain one campaign")
            .clone();
        report.campaigns = (0_u64..4)
            .map(|seed_offset| {
                let mut campaign = template.clone();
                campaign.seed = campaign.seed.saturating_add(seed_offset);
                campaign.simulated_days = 1_800;
                campaign
                    .commands
                    .get_mut(&GameplayCommandKind::StartPublicWork)
                    .expect("public-work statistics must exist")
                    .executed = 3;
                campaign.end.player_completed_public_work_kinds.clear();
                campaign
                    .end
                    .player_completed_public_work_kinds
                    .insert(PublicWorkKind::School);
                campaign
            })
            .collect();
        report.aggregate.campaigns = 4;
        report.aggregate.simulated_days = 7_200;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Civic construction portfolios converge on one project type",
        );
    }

    #[test]
    fn findings_surface_phase_specific_campaign_administration_dominance() {
        let mut report = cached_focused_report(30);
        report.aggregate.phase_stats.insert(
            GameplayPhase::InstitutionalAscent,
            GameplayPhaseStats {
                decision_cycles: 60,
                substantive_actions: 40,
                institutional_campaign_actions: 28,
                quiet_cycles: 20,
                quiet_cycles_with_ambient_change: 20,
                longest_quiet_streak_cycles: 2,
                blocked_cycles: 0,
                cycles_with_multiple_viable_command_kinds: 20,
                cycles_with_close_viable_command_kinds: 10,
                cycles_with_distinct_immediate_consequences: 20,
                cycles_with_distinct_projected_consequences: 20,
                cycles_with_multiple_viable_options: 40,
                cycles_with_close_viable_options: 10,
                cycles_with_distinct_immediate_option_consequences: 40,
                cycles_with_distinct_projected_option_consequences: 40,
                total_viable_choices: 120,
                total_viable_command_kinds: 70,
                executed_commands: BTreeMap::new(),
                ..GameplayPhaseStats::default()
            },
        );

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Institutional ascent becomes campaign administration",
        );
    }

    #[test]
    fn ordinary_nomination_share_in_ascent_is_informational_not_a_duplicate_warning() {
        let mut report = cached_focused_report(30);
        let stats = report
            .aggregate
            .phase_stats
            .entry(GameplayPhase::InstitutionalAscent)
            .or_default();
        *stats = GameplayPhaseStats::default();
        stats.decision_cycles = 50;
        stats.substantive_actions = 40;
        stats.total_viable_choices = 120;
        stats.total_viable_command_kinds = 70;
        stats
            .executed_commands
            .insert(GameplayCommandKind::NominateForOffice, 16);
        stats
            .executed_commands
            .insert(GameplayCommandKind::EducateFamilyMember, 10);

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding =
            finding_with_title(&findings, "institutional-ascent action mix is concentrated");
        assert_eq!(finding.severity, GameplayFindingSeverity::Info);
        assert_finding_absent(
            &findings,
            "Institutional ascent becomes campaign administration",
        );
    }

    #[test]
    fn findings_surface_phase_command_dominance() {
        let mut report = cached_focused_report(30);
        let stats = report
            .aggregate
            .phase_stats
            .get_mut(&GameplayPhase::DynasticGovernance)
            .expect("governance phase statistics must exist");
        stats.decision_cycles = 60;
        stats.substantive_actions = 40;
        stats.quiet_cycles = 20;
        stats.quiet_cycles_with_ambient_change = 20;
        stats.total_viable_choices = 120;
        stats.total_viable_command_kinds = 70;
        stats.executed_commands.clear();
        stats
            .executed_commands
            .insert(GameplayCommandKind::EducateFamilyMember, 16);
        stats
            .executed_commands
            .insert(GameplayCommandKind::StartPublicWork, 8);

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding =
            finding_with_title(&findings, "dynastic-governance action mix is concentrated");
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
        assert!(
            finding
                .evidence
                .contains("family-education accounted for 16 of 40")
        );
    }

    #[test]
    fn findings_surface_foundation_command_dominance() {
        let mut report = cached_focused_report(30);
        let stats = report
            .aggregate
            .phase_stats
            .entry(GameplayPhase::Foundation)
            .or_default();
        *stats = GameplayPhaseStats::default();
        stats.decision_cycles = 30;
        stats.substantive_actions = 30;
        stats.total_viable_choices = 70;
        stats.total_viable_command_kinds = 50;
        stats
            .executed_commands
            .insert(GameplayCommandKind::InvestInBusiness, 15);
        stats
            .executed_commands
            .insert(GameplayCommandKind::SetBusinessPolicy, 8);

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(&findings, "foundation action mix is concentrated");
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
        assert!(
            finding
                .evidence
                .contains("invest-business accounted for 15 of 30")
        );
    }

    #[test]
    fn one_initial_policy_choice_per_campaign_is_informational() {
        let mut report = cached_focused_report(30);
        report.aggregate.campaigns = 24;
        let stats = report
            .aggregate
            .phase_stats
            .entry(GameplayPhase::Foundation)
            .or_default();
        *stats = GameplayPhaseStats::default();
        stats.decision_cycles = 72;
        stats.substantive_actions = 48;
        stats.total_viable_choices = 120;
        stats.total_viable_command_kinds = 80;
        stats
            .executed_commands
            .insert(GameplayCommandKind::SetBusinessPolicy, 24);
        stats
            .executed_commands
            .insert(GameplayCommandKind::SetHouseGovernance, 12);

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(&findings, "foundation action mix is concentrated");
        assert_eq!(finding.severity, GameplayFindingSeverity::Info);

        report
            .aggregate
            .phase_stats
            .get_mut(&GameplayPhase::Foundation)
            .expect("foundation stats must exist")
            .executed_commands
            .insert(GameplayCommandKind::SetBusinessPolicy, 25);
        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(&findings, "foundation action mix is concentrated");
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn findings_surface_persona_specific_mature_action_concentration() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused report must contain one campaign");
        campaign.simulated_days = 3_600;
        for stats in campaign.commands.values_mut() {
            stats.executed = 0;
        }
        campaign
            .commands
            .get_mut(&GameplayCommandKind::ResolveLaborDispute)
            .expect("labor command statistics must exist")
            .executed = 31;
        campaign
            .commands
            .get_mut(&GameplayCommandKind::SecureSupply)
            .expect("contract command statistics must exist")
            .executed = 29;
        campaign
            .commands
            .get_mut(&GameplayCommandKind::EducateFamilyMember)
            .expect("education command statistics must exist")
            .executed = 20;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(
            &findings,
            "An individual mature campaign has a concentrated action mix",
        );
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
        assert!(finding.evidence.contains("labor-response for 31 of 80"));
    }

    #[test]
    fn findings_surface_weak_persona_level_variety() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused report must contain one campaign");
        campaign.simulated_days = 3_600;
        campaign.scores.variety = 65;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(&findings, "A mature persona has weak strategic variety");
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
        assert!(
            finding
                .evidence
                .contains("steward persona scored 65 for variety")
        );
    }

    #[test]
    fn findings_identify_an_economic_recovery_dead_end() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.longest_substantive_action_gap_days = 720;
        campaign.end.player_treasury = Money::ZERO;
        campaign.end.active_businesses = 0;
        campaign.end.distressed_businesses = 1;
        campaign.end.insolvent_businesses = 0;
        campaign.end.player_properties = 0;
        campaign.end.current_loans = 0;
        campaign.end.delinquent_loans = 0;
        campaign.end.restructured_loans = 0;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Economic failure can become an unrecoverable campaign state",
        );
    }

    #[test]
    fn findings_identify_active_but_ineffective_recovery_churn() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.longest_substantive_action_gap_days = 30;
        campaign.end.player_treasury = Money::ZERO;
        campaign.end.active_businesses = 0;
        campaign.end.distressed_businesses = 1;
        campaign.end.insolvent_businesses = 0;
        campaign.end.player_properties = 0;
        campaign.end.defaulted_loans = 2;
        campaign.longest_recovery_pressure_days = 360;
        campaign.terminal_recovery_pressure_days = 360;
        campaign
            .commands
            .get_mut(&GameplayCommandKind::BorrowFunds)
            .expect("borrowing statistics must exist")
            .executed = 4;
        campaign
            .commands
            .get_mut(&GameplayCommandKind::InvestInBusiness)
            .expect("investment statistics must exist")
            .executed = 3;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "An individual dynasty remains trapped in recovery churn",
        );
    }

    #[test]
    fn endpoint_only_recovery_pressure_is_not_persistent_churn() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.end.player_treasury = Money::ZERO;
        campaign.end.active_businesses = 0;
        campaign.end.distressed_businesses = 1;
        campaign.end.player_properties = 0;
        campaign.end.defaulted_loans = 2;
        campaign.longest_recovery_pressure_days = 30;
        campaign.terminal_recovery_pressure_days = 30;
        campaign
            .commands
            .get_mut(&GameplayCommandKind::BorrowFunds)
            .expect("borrowing statistics must exist")
            .executed = 4;
        campaign
            .commands
            .get_mut(&GameplayCommandKind::InvestInBusiness)
            .expect("investment statistics must exist")
            .executed = 3;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert!(
            findings.iter().all(|finding| finding.title
                != "An individual dynasty remains trapped in recovery churn"),
            "a single endpoint interval must not be described as persistent recovery churn"
        );
    }

    #[test]
    fn findings_surface_compressed_commercial_and_political_phases() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 720;
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.simulated_days = 720;
        campaign.fantasy_arc.first_commercial_standing_day = Some(70);
        campaign.fantasy_arc.first_institution_support_day = Some(70);
        campaign.fantasy_arc.first_office_campaign_day = Some(70);
        campaign.fantasy_arc.first_office_day = Some(140);
        campaign.fantasy_arc.first_city_shaping_action_day = Some(147);

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Institutional support immediately becomes candidacy",
        );
        finding_with_title(
            &findings,
            "Officeholding immediately becomes city-shaping power",
        );
    }

    #[test]
    fn findings_surface_effective_officeholder_capacity_capture() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 720;
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.simulated_days = 720;
        campaign.end.available_offices = 11;
        campaign.end.eligible_officeholders = 2;
        campaign.maximum_offices_held = 2;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(&findings, "Dynasty fills every available officeholder slot");
    }

    #[test]
    fn officeholder_capacity_finding_respects_prior_ward_adoption() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 720;
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.simulated_days = 720;
        campaign.end.available_offices = 11;
        campaign.end.eligible_officeholders = 2;
        campaign.maximum_offices_held = 2;
        campaign
            .commands
            .get_mut(&GameplayCommandKind::AdoptWard)
            .expect("ward command statistics must exist")
            .executed = 1;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(&findings, "Dynasty fills every available officeholder slot");
    }

    #[test]
    fn single_track_actionable_cycles_are_a_warning() {
        let mut report = cached_focused_report(30);
        report.aggregate.decision_cycles = 10;
        report.aggregate.quiet_cycles = 0;
        report.aggregate.viable_choices = 30;
        report.aggregate.viable_command_kinds = 10;
        report.aggregate.cycles_with_multiple_viable_command_kinds = 0;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(&findings, "Actionable cycles are usually single-track");

        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
        assert_finding_absent(
            &findings,
            "Actionable cycles offer too few meaningful alternatives",
        );
    }

    #[test]
    fn focused_but_competing_action_families_remain_informational() {
        let mut report = cached_focused_report(30);
        report.aggregate.decision_cycles = 10;
        report.aggregate.quiet_cycles = 0;
        report.aggregate.viable_choices = 30;
        report.aggregate.viable_command_kinds = 18;
        report.aggregate.cycles_with_multiple_viable_command_kinds = 4;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "Strategic alternatives concentrate within command families",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Info);
        assert_finding_absent(&findings, "Actionable cycles are usually single-track");
    }

    #[test]
    fn findings_surface_multi_option_cycles_with_an_obvious_winner() {
        let mut report = cached_focused_report(30);
        report.aggregate.decision_cycles = 100;
        report.aggregate.quiet_cycles = 0;
        report.aggregate.viable_choices = 300;
        report.aggregate.viable_command_kinds = 250;
        report.aggregate.cycles_with_multiple_viable_command_kinds = 80;
        report.aggregate.cycles_with_close_viable_command_kinds = 10;
        report.aggregate.cycles_with_distinct_immediate_consequences = 80;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Most multi-option cycles still have an obvious winner",
        );
    }

    #[test]
    fn findings_surface_alternatives_with_identical_immediate_profiles() {
        let mut report = cached_focused_report(30);
        report.aggregate.decision_cycles = 100;
        report.aggregate.quiet_cycles = 0;
        report.aggregate.viable_choices = 300;
        report.aggregate.viable_command_kinds = 250;
        report.aggregate.cycles_with_multiple_viable_command_kinds = 80;
        report.aggregate.cycles_with_close_viable_command_kinds = 80;
        report.aggregate.cycles_with_distinct_immediate_consequences = 20;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Viable alternatives often share the same immediate consequence profile",
        );
    }

    #[test]
    fn findings_surface_alternatives_that_converge_after_projection() {
        let mut report = cached_focused_report(30);
        report.aggregate.decision_cycles = 100;
        report.aggregate.quiet_cycles = 0;
        report.aggregate.viable_choices = 300;
        report.aggregate.viable_command_kinds = 250;
        report.aggregate.cycles_with_multiple_viable_command_kinds = 80;
        report.aggregate.cycles_with_close_viable_command_kinds = 80;
        report.aggregate.cycles_with_distinct_immediate_consequences = 80;
        report.aggregate.cycles_with_distinct_projected_consequences = 20;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Strategic alternatives converge at the shared projected horizon",
        );
    }

    #[test]
    fn findings_surface_concrete_targets_that_converge_after_projection() {
        let mut report = cached_focused_report(30);
        report.aggregate.cycles_with_multiple_viable_options = 100;
        report.aggregate.cycles_with_close_viable_options = 70;
        report
            .aggregate
            .cycles_with_distinct_immediate_option_consequences = 70;
        report
            .aggregate
            .cycles_with_distinct_projected_option_consequences = 40;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "Concrete alternatives converge despite different targets",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn concrete_target_divergence_avoids_false_convergence_warning() {
        let mut report = cached_focused_report(30);
        report.aggregate.cycles_with_multiple_viable_options = 100;
        report.aggregate.cycles_with_close_viable_options = 70;
        report
            .aggregate
            .cycles_with_distinct_immediate_option_consequences = 85;
        report
            .aggregate
            .cycles_with_distinct_projected_option_consequences = 90;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(
            &findings,
            "Concrete alternatives converge despite different targets",
        );
    }

    #[test]
    fn findings_surface_property_acquisition_as_universal_progression() {
        let mut report = cached_focused_report(30);
        let template = report
            .campaigns
            .first()
            .expect("focused report must contain a campaign")
            .clone();
        report.campaigns = (0..8).map(|_| template.clone()).collect();
        report.aggregate.campaigns = 8;
        report.aggregate.simulated_days = 7_200;
        for campaign in &mut report.campaigns {
            campaign.simulated_days = 900;
            campaign
                .commands
                .get_mut(&GameplayCommandKind::BuyProperty)
                .expect("property command statistics must exist")
                .executed = 2;
        }

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "Property acquisition becomes a universal progression path",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn diversified_property_growth_avoids_universal_progression_warning() {
        let mut report = cached_focused_report(30);
        let template = report
            .campaigns
            .first()
            .expect("focused report must contain a campaign")
            .clone();
        report.campaigns = (0..8).map(|_| template.clone()).collect();
        report.aggregate.campaigns = 8;
        report.aggregate.simulated_days = 7_200;
        for (index, campaign) in report.campaigns.iter_mut().enumerate() {
            campaign.simulated_days = 900;
            campaign
                .commands
                .get_mut(&GameplayCommandKind::BuyProperty)
                .expect("property command statistics must exist")
                .executed = if index < 3 { 2 } else { 1 };
        }

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(
            &findings,
            "Property acquisition becomes a universal progression path",
        );
    }

    #[test]
    fn findings_surface_mature_house_governance_convergence() {
        let mut report = cached_focused_report(30);
        let template = report
            .campaigns
            .first()
            .expect("focused report must contain a campaign")
            .clone();
        report.campaigns = (0..8).map(|_| template.clone()).collect();
        report.aggregate.campaigns = 8;
        report.aggregate.simulated_days = 14_400;
        for campaign in &mut report.campaigns {
            campaign.simulated_days = 1_800;
            campaign.end.house_governance = HouseGovernance::Primogeniture;
            campaign
                .commands
                .get_mut(&GameplayCommandKind::SetHouseGovernance)
                .expect("house-governance command statistics must exist")
                .executed = 1;
        }

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "House governance converges on one succession model",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn findings_surface_routine_commission_and_leverage_pairs() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign
            .commands
            .get_mut(&GameplayCommandKind::CommissionInformation)
            .expect("commission statistics must exist")
            .executed = 20;
        campaign.commission_leverage_pairs = 15;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Commissioned intelligence becomes a routine two-step ritual",
        );
    }

    #[test]
    fn findings_do_not_treat_occasional_intelligence_as_scheduled_maintenance() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.simulated_days = 18_000;
        campaign
            .commands
            .get_mut(&GameplayCommandKind::CommissionInformation)
            .expect("commission statistics must exist")
            .executed = 20;
        campaign.commission_leverage_pairs = 20;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(
            &findings,
            "Commissioned intelligence becomes a routine two-step ritual",
        );
    }

    #[test]
    fn findings_do_not_treat_severe_pressure_intelligence_as_scheduled_maintenance() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign
            .commands
            .get_mut(&GameplayCommandKind::CommissionInformation)
            .expect("commission statistics must exist")
            .executed = 20;
        campaign.commission_leverage_pairs = 20;
        campaign.maximum_contract_relationship_pressure_basis_points =
            AGENT_INFORMATION_SEVERE_COUNTERPARTY_PRESSURE_BASIS_POINTS;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(
            &findings,
            "Commissioned intelligence becomes a routine two-step ritual",
        );
    }

    #[test]
    fn passive_information_changes_without_a_material_trigger_do_not_imply_missing_player_agency() {
        let mut report = cached_focused_report(30);
        *report
            .aggregate
            .ambient_domain_changes
            .get_mut(&GameplayDomain::Information)
            .expect("information domain statistics must exist") = 20;
        // Commissioning is canonically available whenever the dynasty can pay
        // and is off cooldown, so the focused report records real commission
        // opportunities. Isolate the passive-only scenario by clearing the
        // activation count: ambient information change alone must not imply a
        // missing player route.
        report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::CommissionInformation)
            .expect("commission statistics must exist")
            .activation_opportunities = 0;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(&findings, "Commercial intelligence is not player-directed");
    }

    #[test]
    fn material_information_opportunities_without_commissioning_are_reported() {
        let mut report = cached_focused_report(30);
        *report
            .aggregate
            .ambient_domain_changes
            .get_mut(&GameplayDomain::Information)
            .expect("information domain statistics must exist") = 20;
        report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::CommissionInformation)
            .expect("commission statistics must exist")
            .activation_opportunities = 4;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding =
            finding_with_title(&findings, "Commercial intelligence is not player-directed");

        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
        assert!(
            finding
                .evidence
                .contains("4 material intelligence opportunities")
        );
    }

    #[test]
    fn information_pair_tracking_survives_intervening_decisions() {
        let mut accumulator = CampaignAccumulator::new();

        accumulator.record_executed_command(GameplayCommandKind::CommissionInformation, 100);
        accumulator.record_executed_command(GameplayCommandKind::SetBusinessPolicy, 130);
        accumulator.record_executed_command(GameplayCommandKind::LeverageInformation, 190);

        assert_eq!(accumulator.commission_leverage_pairs, 1);
        assert_eq!(accumulator.last_information_commission_day, None);
    }

    #[test]
    fn findings_surface_information_leverage_without_delayed_trajectory_change() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 3_600;
        let stats = report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::LeverageInformation)
            .expect("information leverage statistics must exist");
        stats.executed = 20;
        stats.actions_with_persistent_consequences = 2;
        stats.actions_with_delayed_consequences = 2;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        let finding = finding_with_title(
            &findings,
            "Commissioned intelligence rarely changes the later trajectory",
        );
        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn delayed_information_consequences_avoid_false_trajectory_warning() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 3_600;
        let stats = report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::LeverageInformation)
            .expect("information leverage statistics must exist");
        stats.executed = 20;
        stats.actions_with_persistent_consequences = 20;
        stats.actions_with_delayed_consequences = 0;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(
            &findings,
            "Commissioned intelligence rarely changes the later trajectory",
        );
    }

    #[test]
    fn succession_legacy_quiet_streak_uses_mature_annual_cadence() {
        let mut report = cached_focused_report(30);
        report.aggregate.phase_stats.insert(
            GameplayPhase::SuccessionLegacy,
            GameplayPhaseStats {
                decision_cycles: 100,
                substantive_actions: 70,
                institutional_campaign_actions: 0,
                quiet_cycles: 30,
                quiet_cycles_with_ambient_change: 30,
                longest_quiet_streak_cycles: GOVERNANCE_MAX_QUIET_STREAK_CYCLES,
                blocked_cycles: 0,
                cycles_with_multiple_viable_command_kinds: 40,
                cycles_with_close_viable_command_kinds: 20,
                cycles_with_distinct_immediate_consequences: 40,
                cycles_with_distinct_projected_consequences: 40,
                cycles_with_multiple_viable_options: 70,
                cycles_with_close_viable_options: 20,
                cycles_with_distinct_immediate_option_consequences: 70,
                cycles_with_distinct_projected_option_consequences: 70,
                total_viable_choices: 300,
                total_viable_command_kinds: 160,
                executed_commands: BTreeMap::new(),
                ..GameplayPhaseStats::default()
            },
        );

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        assert_finding_absent(
            &findings,
            "Succession and legacy lack post-transition strategy",
        );

        report
            .aggregate
            .phase_stats
            .get_mut(&GameplayPhase::SuccessionLegacy)
            .expect("succession phase statistics must exist")
            .longest_quiet_streak_cycles = GOVERNANCE_MAX_QUIET_STREAK_CYCLES + 1;
        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "Succession and legacy lack post-transition strategy",
        );
        assert!(
            finding
                .evidence
                .contains("longest quiet streak 12 > 11 cycles")
        );
    }

    #[test]
    fn findings_surface_crisis_actions_without_delayed_trajectory_change() {
        let mut report = cached_focused_report(30);
        let stats = report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::RespondToCrisis)
            .expect("crisis response statistics must exist");
        stats.executed = 20;
        stats.actions_with_delayed_consequences = 4;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Crisis responses rarely change the future trajectory",
        );
    }

    #[test]
    fn persistent_crisis_consequences_count_as_future_trajectory_change() {
        let mut report = cached_focused_report(30);
        let stats = report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::RespondToCrisis)
            .expect("crisis response statistics must exist");
        stats.executed = 20;
        stats.actions_with_persistent_consequences = 20;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(
            &findings,
            "Crisis responses rarely change the future trajectory",
        );
    }

    #[test]
    fn findings_surface_near_universal_institutional_reach() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.simulated_days = 3_600;
        campaign.end.available_offices = 10;
        campaign.end.player_institutions_represented = 9;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Dynasty networks become institutionally universal",
        );
    }

    #[test]
    fn findings_surface_start_specific_blocking() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.decision_cycles = 10;
        campaign.quiet_cycles = 0;
        campaign.blocked_cycles = 4;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "An individual campaign becomes strategically blocked",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn findings_surface_absolute_core_fantasy_compression() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 720;
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.simulated_days = 720;
        campaign.fantasy_arc.first_commercial_standing_day = Some(70);
        campaign.fantasy_arc.first_institution_support_day = Some(100);
        campaign.fantasy_arc.first_office_campaign_day = Some(140);
        campaign.fantasy_arc.first_city_shaping_action_day = Some(420);

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "The core fantasy arc is compressed into the opening establishment cycle",
        );
    }

    #[test]
    fn foundation_year_does_not_require_completed_political_ascent() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 360;
        report.campaigns[0].simulated_days = 360;
        report.campaigns[0].fantasy_arc.first_office_campaign_day = None;
        report.campaigns[0].fantasy_arc.first_office_day = None;
        report.campaigns[0]
            .fantasy_arc
            .first_city_shaping_action_day = None;

        let foundation_findings = derive_findings(&report.aggregate, &report.campaigns);
        assert_finding_absent(
            &foundation_findings,
            "The early commercial-to-political arc is incomplete",
        );
        assert_finding_absent(
            &foundation_findings,
            "Institutional power does not become city-shaping action",
        );

        report.aggregate.simulated_days = 1_080;
        report.campaigns[0].simulated_days = 1_080;
        let mature_findings = derive_findings(&report.aggregate, &report.campaigns);
        finding_with_title(
            &mature_findings,
            "The early commercial-to-political arc is incomplete",
        );
        assert_finding_absent(
            &mature_findings,
            "Institutional power does not become city-shaping action",
        );
    }

    #[test]
    fn city_shaping_warning_waits_for_office_powers_to_establish() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 1_080;
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.simulated_days = 1_080;
        campaign.fantasy_arc.first_city_shaping_action_day = None;
        campaign.fantasy_arc.first_office_day = Some(
            1_080_i64
                .saturating_sub(OFFICE_POWER_ESTABLISHMENT_DAYS)
                .saturating_add(1),
        );

        let premature_findings = derive_findings(&report.aggregate, &report.campaigns);
        assert_finding_absent(
            &premature_findings,
            "Institutional power does not become city-shaping action",
        );

        report.campaigns[0].fantasy_arc.first_office_day =
            Some(1_080_i64.saturating_sub(OFFICE_POWER_ESTABLISHMENT_DAYS));
        let mature_findings = derive_findings(&report.aggregate, &report.campaigns);
        finding_with_title(
            &mature_findings,
            "Institutional power does not become city-shaping action",
        );
    }

    #[test]
    fn dynastic_continuity_is_only_required_at_generation_length() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 3_600;
        report.campaigns[0].simulated_days = 3_600;

        let midgame_findings = derive_findings(&report.aggregate, &report.campaigns);
        assert_finding_absent(
            &midgame_findings,
            "Long campaigns do not exercise dynastic continuity",
        );

        report.aggregate.simulated_days = 7_200;
        report.campaigns[0].simulated_days = 7_200;
        let generation_findings = derive_findings(&report.aggregate, &report.campaigns);
        finding_with_title(
            &generation_findings,
            "Long campaigns do not exercise dynastic continuity",
        );
    }

    #[test]
    fn isolated_late_succession_does_not_condemn_a_generation_length_matrix() {
        let mut report = cached_focused_report(30);
        let baseline = report.campaigns[0].clone();
        report.aggregate.campaigns = 6;
        report.aggregate.simulated_days = 43_200;
        report.campaigns = (0..6)
            .map(|index| {
                let mut campaign = baseline.clone();
                campaign.simulated_days = 7_200;
                campaign.fantasy_arc.first_succession_day =
                    (index != 0).then_some(5_400_i64.saturating_add(i64::from(index) * 30));
                campaign
            })
            .collect();

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(&findings, "The dynastic arc does not reach succession");
    }

    #[test]
    fn findings_surface_low_exposure_after_officeholding() {
        let mut report = cached_focused_report(30);
        let baseline = report.campaigns[0].clone();
        report.aggregate.campaigns = 4;
        report.aggregate.simulated_days = 14_400;
        report.campaigns = (0..4)
            .map(|_| {
                let mut campaign = baseline.clone();
                campaign.simulated_days = 3_600;
                campaign.maximum_offices_held = 1;
                campaign.maximum_player_disputed_employment = 0;
                campaign.end.player_contract_failures = 0;
                campaign.end.distressed_businesses = 0;
                campaign.end.insolvent_businesses = 0;
                campaign.end.player_treasury = campaign.start.player_treasury;
                campaign.end.player_civic_contributions = campaign.start.player_civic_contributions;
                campaign.end.player_unmet_office_duties = campaign.start.player_unmet_office_duties;
                campaign
            })
            .collect();

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Established dynasties often avoid measured power exposure",
        );
    }

    #[test]
    fn routine_civic_duties_do_not_mask_sheltered_power() {
        let mut report = cached_focused_report(30);
        let baseline = report.campaigns[0].clone();
        report.aggregate.campaigns = 4;
        report.aggregate.simulated_days = 14_400;
        report.campaigns = (0..4)
            .map(|_| {
                let mut campaign = baseline.clone();
                campaign.simulated_days = 3_600;
                campaign.maximum_offices_held = 1;
                campaign.maximum_player_disputed_employment = 0;
                campaign.end.player_contract_failures = 0;
                campaign.end.distressed_businesses = 0;
                campaign.end.insolvent_businesses = 0;
                campaign.end.player_treasury = campaign.start.player_treasury;
                campaign.end.player_civic_contributions = campaign
                    .start
                    .player_civic_contributions
                    .saturating_add(Money::from_copper(5_000));
                campaign
            })
            .collect();

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Established dynasties often avoid measured power exposure",
        );
    }

    #[test]
    fn material_relationship_pressure_counts_as_power_exposure() {
        let mut report = cached_focused_report(30);
        let baseline = report.campaigns[0].clone();
        report.aggregate.campaigns = 4;
        report.aggregate.simulated_days = 14_400;
        report.campaigns = (0..4)
            .map(|_| {
                let mut campaign = baseline.clone();
                campaign.simulated_days = 3_600;
                campaign.maximum_offices_held = 1;
                campaign.maximum_player_disputed_employment = 0;
                campaign.end.player_contract_failures = 0;
                campaign.end.distressed_businesses = 0;
                campaign.end.insolvent_businesses = 0;
                campaign.end.player_treasury = campaign.start.player_treasury;
                campaign.maximum_contract_relationship_pressure_basis_points = 1_500;
                campaign
            })
            .collect();

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(
            &findings,
            "Established dynasties often avoid measured power exposure",
        );
    }

    #[test]
    fn findings_surface_chronic_office_duty_failures() {
        let mut report = cached_focused_report(30);
        let baseline = report.campaigns[0].clone();
        report.aggregate.campaigns = 4;
        report.aggregate.simulated_days = 4_320;
        report.campaigns = (0..4)
            .map(|index| {
                let mut campaign = baseline.clone();
                campaign.simulated_days = 1_080;
                if index == 0 {
                    campaign.end.player_unmet_office_duties =
                        campaign.start.player_unmet_office_duties.saturating_add(12);
                }
                campaign
            })
            .collect();

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "Office obligations repeatedly exceed dynasty liquidity",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn genuinely_sparse_actionable_cycles_remain_a_warning() {
        let mut report = cached_focused_report(30);
        report.aggregate.decision_cycles = 10;
        report.aggregate.quiet_cycles = 0;
        report.aggregate.viable_choices = 10;
        report.aggregate.viable_command_kinds = 10;
        report.aggregate.cycles_with_multiple_viable_command_kinds = 0;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "Actionable cycles offer too few meaningful alternatives",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn cadence_findings_distinguish_static_quiet_from_world_movement() {
        let mut report = cached_focused_report(30);
        report.aggregate.decision_cycles = 100;
        report.aggregate.quiet_cycles = 30;
        report.aggregate.quiet_cycles_with_ambient_change = 0;

        let static_findings = derive_findings(&report.aggregate, &report.campaigns);
        finding_with_title(
            &static_findings,
            "Strategic cadence leaves too many static decision cycles",
        );

        report.aggregate.quiet_cycles_with_ambient_change = 30;
        let dynamic_findings = derive_findings(&report.aggregate, &report.campaigns);
        assert_finding_absent(
            &dynamic_findings,
            "Strategic cadence leaves too many static decision cycles",
        );
    }

    #[test]
    fn findings_surface_a_single_complete_food_collapse() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.end.average_food_satisfaction = 0;
        campaign.minimum_food_satisfaction = 0;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "At least one campaign experiences complete food collapse",
        );
    }

    #[test]
    fn findings_surface_single_campaign_notification_overload() {
        let mut report = cached_focused_report(30);
        let baseline = report
            .campaigns
            .first()
            .expect("focused configuration must produce one campaign")
            .clone();
        report.campaigns.extend([
            baseline.clone(),
            baseline.clone(),
            baseline.clone(),
            baseline,
        ]);
        let (overloaded, other_campaigns) = report
            .campaigns
            .split_first_mut()
            .expect("report must retain at least one campaign");
        overloaded.maximum_unread_notifications = 101;
        for campaign in other_campaigns {
            campaign.maximum_unread_notifications = 0;
        }

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Individual campaigns experience notification overload",
        );
    }

    #[test]
    fn short_horizon_absent_reactive_commands_are_informational() {
        let report = cached_focused_report(30);

        let finding = finding_with_title(
            &report.findings,
            "crisis-response was not exercised in this horizon",
        );
        // Contract supply routes are only player-responsive when an executable
        // route exists: an uncontracted counterparty pair with capacity. The
        // focused fixture's player business already trades through the ambient
        // market, so a short horizon reports the domain as not yet
        // player-connected rather than as a used-but-ambient route.
        let contract_finding = finding_with_title(
            &report.findings,
            "contracts domain changed before a player route became available",
        );
        assert!(
            !report.findings.iter().any(|finding| {
                finding.title == "contracts domain is autonomous but not player-responsive"
            }),
            "an activation predicate must not claim player responsiveness without an executable route"
        );
        let legal_finding = finding_with_title(
            &report.findings,
            "legal-case was not exercised in this horizon",
        );
        let crisis_domain_finding = finding_with_title(
            &report.findings,
            "crises domain was inactive in this horizon",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Info);
        assert_eq!(
            contract_finding.severity,
            GameplayFindingSeverity::Info,
            "a domain that changed only before any player route existed is informational, not a broken player route"
        );
        assert_eq!(legal_finding.severity, GameplayFindingSeverity::Info);
        assert_eq!(
            crisis_domain_finding.severity,
            GameplayFindingSeverity::Info
        );
    }

    #[test]
    fn long_horizon_absent_command_routes_are_critical() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 720;
        report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::ResolveLaborDispute)
            .expect("all command statistics must exist")
            .activation_opportunities = 1;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(&findings, "labor-response had no reachable candidate");

        assert_eq!(finding.severity, GameplayFindingSeverity::Critical);
    }

    #[test]
    fn policy_gated_liquidity_routes_without_candidates_are_warnings() {
        for kind in [
            GameplayCommandKind::TransferBusinessCash,
            GameplayCommandKind::WithdrawBusinessCash,
        ] {
            let mut report = cached_focused_report(30);
            report.aggregate.simulated_days = 720;
            let stats = report
                .aggregate
                .commands
                .get_mut(&kind)
                .expect("all command statistics must exist");
            stats.activation_opportunities = 1;

            let findings = derive_findings(&report.aggregate, &report.campaigns);
            let finding = finding_with_title(
                &findings,
                &format!("{} had no reachable candidate", kind.label()),
            );

            assert_eq!(
                finding.severity,
                GameplayFindingSeverity::Warning,
                "an idle liquidity route reflects the agent's rebalancing policy, not a broken game route"
            );
        }
    }

    #[test]
    fn event_driven_routes_without_a_trigger_remain_informational() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 7_200;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let labor_finding = finding_with_title(
            &findings,
            "labor-response was not exercised in this horizon",
        );
        let office_finding =
            finding_with_title(&findings, "office-power was not exercised in this horizon");
        // Settlement is only reachable when a case exists against the dynasty
        // that quotes a settlement; no such event fires in this horizon, so
        // the unexercised route stays informational.
        let settlement_finding = finding_with_title(
            &findings,
            "legal-settlement was not exercised in this horizon",
        );

        assert_eq!(labor_finding.severity, GameplayFindingSeverity::Info);
        assert_eq!(office_finding.severity, GameplayFindingSeverity::Info);
        assert_eq!(settlement_finding.severity, GameplayFindingSeverity::Info);
    }

    #[test]
    fn condition_driven_information_routes_without_a_trigger_remain_informational() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 7_200;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        // Commissioning is canonically available whenever the dynasty can pay
        // and is off cooldown, so a focused horizon now records real commission
        // opportunities. When the agent builds but never executes them, the
        // report surfaces the unselected route as informational rather than
        // implying a systemic generator gap.
        let commission = finding_with_title(
            &findings,
            "commission-intelligence appeared only as a rare unselected alternative",
        );
        let leverage = finding_with_title(
            &findings,
            "leverage-intelligence was not exercised in this horizon",
        );

        assert_eq!(commission.severity, GameplayFindingSeverity::Info);
        assert_eq!(leverage.severity, GameplayFindingSeverity::Info);
    }

    #[test]
    fn triggered_information_route_without_a_candidate_is_warning_for_a_policy_gated_route() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 7_200;
        let stats = report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::CommissionInformation)
            .expect("all command statistics must exist");
        stats.activation_opportunities = 1;
        // Commissioning is canonically available in the focused horizon, so
        // isolate the generator-gap scenario explicitly: the world offered the
        // route but no candidate was ever built. Commissioning deliberately
        // narrows to strategic-need conditions, so the finding warns instead
        // of declaring the route unreachable.
        stats.generated = 0;
        stats.offered_cycles = 0;
        stats.considered = 0;
        stats.viable = 0;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "commission-intelligence had no reachable candidate",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn triggered_ungated_route_without_a_candidate_is_critical() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 7_200;
        let stats = report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::RespondToCrisis)
            .expect("all command statistics must exist");
        stats.activation_opportunities = 1;
        // Crisis response has no strategic-need narrowing: when the world
        // offers it and no candidate is ever constructed, that is a true
        // coverage hole in the harness's command surface.
        stats.generated = 0;
        stats.offered_cycles = 0;
        stats.considered = 0;
        stats.viable = 0;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(&findings, "crisis-response had no reachable candidate");

        assert_eq!(finding.severity, GameplayFindingSeverity::Critical);
    }

    #[test]
    fn one_campaign_without_player_labor_conflict_is_only_informational() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 7_200;
        *report
            .aggregate
            .ambient_domain_changes
            .get_mut(&GameplayDomain::Labor)
            .expect("labor domain statistics must exist") = 50;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(&findings, "Labor conflict remains ambient to the player");

        assert_eq!(finding.severity, GameplayFindingSeverity::Info);
    }

    #[test]
    fn repeated_labor_avoidance_across_campaigns_remains_a_warning() {
        let mut report = cached_focused_report(30);
        let baseline = report.campaigns[0].clone();
        report.aggregate.campaigns = 3;
        report.aggregate.simulated_days = 2_160;
        *report
            .aggregate
            .ambient_domain_changes
            .get_mut(&GameplayDomain::Labor)
            .expect("labor domain statistics must exist") = 50;
        report.campaigns = vec![baseline; 3];

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(&findings, "Labor conflict remains ambient to the player");

        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn one_off_viable_alternatives_remain_informational() {
        let mut report = cached_focused_report(30);
        let stats = report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::WithdrawFromInstitution)
            .expect("all command statistics must exist");
        stats.offered_cycles = 1;
        stats.generated = 3;
        stats.considered = 3;
        stats.viable = 3;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "institution-withdrawal appeared only as a rare unselected alternative",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Info);
    }

    #[test]
    fn repeatedly_viable_unselected_commands_remain_warnings() {
        let mut report = cached_focused_report(30);
        let stats = report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::WithdrawFromInstitution)
            .expect("all command statistics must exist");
        stats.offered_cycles = 3;
        stats.generated = 3;
        stats.considered = 3;
        stats.viable = 3;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "institution-withdrawal was viable but never selected",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
    }

    #[test]
    fn findings_surface_public_work_overload() {
        let mut report = cached_focused_report(30);
        report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::StartPublicWork)
            .expect("all command statistics must exist")
            .executed = 5;
        report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign")
            .maximum_unfinished_public_works = 5;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Public works accumulate faster than the city can execute them",
        );
    }

    #[test]
    fn findings_use_player_contract_outcomes_not_only_citywide_totals() {
        let mut report = cached_focused_report(30);
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.end.fulfilled_contracts = 100;
        campaign.end.breached_contracts = 1;
        campaign.end.player_fulfilled_contracts = 1;
        campaign.end.player_breached_contracts = 2;
        campaign.end.player_contract_failures = 4;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(
            &findings,
            "Player contracts breach more often than they complete",
        );
    }

    #[test]
    fn political_reachability_uses_peak_office_attainment_not_endpoint_incumbency() {
        let mut report = cached_focused_report(30);
        report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::NominateForOffice)
            .expect("all command statistics must exist")
            .executed = 1;
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.end.offices_held = 0;
        campaign.maximum_offices_held = 1;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        assert_finding_absent(
            &findings,
            "Office nominations never produce political power",
        );
    }

    #[test]
    fn findings_surface_complete_player_capture_of_all_offices() {
        let mut report = cached_focused_report(30);
        report
            .aggregate
            .commands
            .get_mut(&GameplayCommandKind::NominateForOffice)
            .expect("all command statistics must exist")
            .executed = 1;
        let campaign = report
            .campaigns
            .first_mut()
            .expect("focused configuration must produce one campaign");
        campaign.end.available_offices = 4;
        campaign.maximum_offices_held = 4;

        let findings = derive_findings(&report.aggregate, &report.campaigns);

        finding_with_title(&findings, "Player captures every political office");
    }
}

fn make_test_candidate_coverage_state(registry: &Registry) -> AppState {
    let mut state = build_new_game(
        registry,
        NewGameConfig {
            seed: 7,
            dynasty_name: "Harness".to_owned(),
            founder_name: "Harness Founder".to_owned(),
            background: StartingBackground::Baker,
        },
    )
    .expect("campaign must build");
    add_second_player_business(&mut state);
    make_nonplayer_business_acquirable(&mut state);
    let player = state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist");
    player.resources.treasury = Money::from_copper(100_000);
    player.resources.reputation_quality_basis_points = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
    grant_office_nomination_record_for_test(registry, &mut state);
    grant_player_office_for_test(&mut state);
    for business_id in player_business_ids_for_test(&state) {
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("player business must exist");
        business.operations.status = BusinessStatus::Active;
        business.operations.condition_basis_points = 8_000;
        business.finance.cash = Money::from_copper(20_000);
    }
    state
}

fn candidate_kinds_for_test(
    registry: &Registry,
    state: &AppState,
) -> BTreeSet<GameplayCommandKind> {
    GameplayPersona::all()
        .into_iter()
        .flat_map(|persona| {
            ranked_candidates(registry, state, persona, &CampaignAccumulator::new()).0
        })
        .map(|candidate| candidate.kind)
        .collect()
}

fn player_business_ids_for_test(state: &AppState) -> Vec<BusinessId> {
    state
        .businesses
        .ids_for_owner(state.player_dynasty_id)
        .into_iter()
        .flatten()
        .copied()
        .collect()
}

fn restore_distressed_business_cash_for_test(state: &mut AppState) {
    let business_id = *player_business_ids_for_test(state)
        .first()
        .expect("player business must exist");
    state
        .businesses
        .get_mut(business_id)
        .expect("player business must exist")
        .finance
        .cash = Money::from_copper(1_000);
}

fn remove_internal_transfer_surplus_for_test(state: &mut AppState) {
    let distressed_id = *player_business_ids_for_test(state)
        .first()
        .expect("player business must exist");
    for business_id in player_business_ids_for_test(state)
        .into_iter()
        .filter(|business_id| *business_id != distressed_id)
    {
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("player business must exist");
        business.finance.cash = business.policy.minimum_cash_reserve;
    }
}

fn grant_player_office_for_test(state: &mut AppState) {
    let mature_term_started_day = state
        .clock
        .day()
        .saturating_sub(OFFICE_POWER_ESTABLISHMENT_DAYS);
    let mature_next_selection_day = state
        .clock
        .day()
        .saturating_add(OFFICE_TERM_DAYS)
        .saturating_sub(OFFICE_POWER_ESTABLISHMENT_DAYS);
    let holder_id = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .head_id();
    let institution = state
        .institutions
        .values_mut()
        .find(|institution| institution.powers.contains(&OfficePower::PublicWorks))
        .expect("campaign must contain an office with public-works power");
    institution.members.insert(holder_id);
    institution.office_holder_id = Some(holder_id);
    institution.term_started_day = mature_term_started_day;
    institution.next_selection_day = mature_next_selection_day;
}

fn make_institution_withdrawal_available_for_test(state: &mut AppState) {
    grant_player_office_for_test(state);
    state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .treasury = Money::ZERO;
}

fn make_public_work_funding_available_for_test(state: &mut AppState) {
    let player_id = state.player_dynasty_id;
    let district_id = state
        .districts
        .keys()
        .copied()
        .next()
        .expect("campaign must contain a district");
    let public_work_id = state.next_ids.public_work();
    state.public_works.insert(
        public_work_id,
        crate::core::PublicWork {
            id: public_work_id,
            district_id,
            kind: PublicWorkKind::Market,
            sponsor_dynasty_id: Some(player_id),
            budget: Money::from_copper(12_000),
            spent: Money::from_copper(2_000),
            progress_basis_points: 1_500,
            status: PublicWorkStatus::Suspended,
        },
    );
}

fn make_legal_settlement_available_for_test(state: &mut AppState) {
    let player_id = state.player_dynasty_id;
    let lender_id = state
        .dynasties
        .keys()
        .copied()
        .find(|dynasty_id| {
            *dynasty_id != player_id
                && !state.loans.values().any(|loan| {
                    loan.lender_dynasty_id == *dynasty_id
                        && loan.borrower_dynasty_id == player_id
                        && loan.status.is_repayment_active()
                })
        })
        .expect("campaign must contain a rival available to lend to the player");
    state
        .dynasties
        .get_mut(&lender_id)
        .expect("rival dynasty must exist")
        .resources
        .treasury = Money::from_copper(100_000);
    let loan_id = issue_loan(
        state,
        LoanTerms {
            lender_dynasty_id: lender_id,
            borrower_dynasty_id: player_id,
            principal: Money::from_copper(5_000),
            weekly_payment: Money::from_copper(300),
            interest_basis_points: 1_000,
            collateral_property_id: None,
        },
    )
    .expect("coverage loan must be issuable");
    let loan = state
        .loans
        .get_mut(&loan_id)
        .expect("coverage loan must exist");
    loan.status = LoanStatus::Delinquent;
    loan.missed_payments = 1;
    let case_id = state.next_ids.legal_case();
    state.legal_cases.insert(
        case_id,
        crate::core::LegalCase {
            id: case_id,
            plaintiff_dynasty_id: lender_id,
            defendant_dynasty_id: player_id,
            kind: LegalCaseKind::Debt,
            claim_source: Some(crate::core::LegalClaimSource::Loan { loan_id }),
            evidence_basis_points: 7_500,
            public_attention_basis_points: 1_500,
            filed_day: state.clock.day(),
            hearing_day: state.clock.day().saturating_add(60),
            damages: Money::from_copper(5_000),
            status: LegalCaseStatus::Filed,
        },
    );
}

fn grant_player_contract_deliveries_for_test(state: &mut AppState, deliveries: u32) {
    let player_businesses: BTreeSet<_> = player_business_ids_for_test(state).into_iter().collect();
    let contract = state
        .contracts
        .values_mut()
        .find(|contract| {
            player_businesses.contains(&contract.buyer_business_id)
                || player_businesses.contains(&contract.seller_business_id)
        })
        .expect("campaign must contain a player contract");
    let deliveries =
        u16::try_from(deliveries).expect("test delivery count must fit contract counters");
    contract.fulfilled_deliveries = deliveries;
    contract
        .fulfilled_deliveries_by_dynasty
        .insert(state.player_dynasty_id, deliveries);
}

fn grant_office_nomination_record_for_test(registry: &Registry, state: &mut AppState) {
    let support_day = state
        .clock
        .day()
        .saturating_sub(INSTITUTION_SUPPORT_ESTABLISHMENT_DAYS);
    let player_character_ids: Vec<_> = state
        .characters
        .iter()
        .filter(|character| character.dynasty_id() == state.player_dynasty_id)
        .map(crate::core::Character::id)
        .collect();
    let institution_ids: Vec<_> = state.institutions.keys().copied().collect();
    for institution_id in institution_ids {
        for character_id in &player_character_ids {
            state
                .institutions
                .get_mut(&institution_id)
                .expect("institution must exist")
                .members
                .insert(*character_id);
            state.audit_log.push(AuditRecord {
                day: support_day,
                kind: AuditKind::InstitutionPatronage,
                subject: format!("institution:{institution_id}:character:{character_id}").into(),
                detail: "test support".to_owned(),
            });
        }
    }
    state.audit_log.sort_by_key(AuditRecord::day);
    let institution_ids: Vec<_> = state.institutions.keys().copied().collect();
    let mut required_deliveries = OFFICE_NOMINATION_DELIVERY_REQUIREMENT;
    for institution_id in institution_ids {
        for character_id in &player_character_ids {
            required_deliveries = required_deliveries.max(office_nomination_delivery_requirement(
                registry,
                state,
                institution_id,
                *character_id,
            ));
        }
    }
    grant_player_contract_deliveries_for_test(state, required_deliveries);
}

fn make_supply_security_and_borrowing_available(state: &mut AppState) {
    let player_businesses: BTreeSet<_> = player_business_ids_for_test(state).into_iter().collect();
    state
        .contracts
        .retain(|_, contract| !player_businesses.contains(&contract.buyer_business_id));
    state
        .dynasties
        .get_mut(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .resources
        .treasury = Money::ZERO;
}

fn make_external_credit_need_available_for_test(state: &mut AppState) {
    let player_id = state.player_dynasty_id;
    state
        .dynasties
        .get_mut(&player_id)
        .expect("player dynasty must exist")
        .resources
        .treasury = Money::from_copper(100_000);
    let borrower_id = state
        .dynasties
        .keys()
        .copied()
        .find(|dynasty_id| {
            *dynasty_id != player_id
                && !same_pair_credit_blocks_new_loan(state, player_id, *dynasty_id)
        })
        .expect("campaign must contain an unused external-credit counterparty");
    state
        .dynasties
        .get_mut(&borrower_id)
        .expect("selected borrower must exist")
        .resources
        .treasury = Money::from_copper(20_000);
    state
        .businesses
        .iter_mut()
        .find(|business| business.owner_dynasty_id() == borrower_id)
        .expect("selected borrower must own a business")
        .finance
        .cash = Money::ZERO;
}

fn add_second_player_business(state: &mut AppState) {
    let player_id = state.player_dynasty_id;
    let manager_id = state
        .dynasties
        .get(&player_id)
        .expect("player dynasty must exist")
        .head_id();
    let mut business = state
        .businesses
        .iter()
        .find(|business| business.owner_dynasty_id() != player_id)
        .expect("campaign must contain a nonplayer business")
        .clone();
    business.identity.id = state.next_ids.business();
    business.identity.owner_dynasty_id = player_id;
    business.identity.name = "Harness Second Business".to_owned();
    business.operations.manager_id = manager_id;
    business.finance.cash = Money::from_copper(1_000);
    state.businesses.insert(business);
}

fn make_nonplayer_business_acquirable(state: &mut AppState) {
    let player_id = state.player_dynasty_id;
    let business_id = state
        .businesses
        .iter()
        .find(|business| business.owner_dynasty_id() != player_id)
        .expect("campaign must contain a nonplayer business")
        .id();
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("selected business must exist");
    business.operations.status = BusinessStatus::Distressed;
    business.finance.cash = Money::ZERO;
}

fn make_player_business_distressed(state: &mut AppState) {
    let business_id = *state
        .businesses
        .ids_for_owner(state.player_dynasty_id)
        .and_then(|ids| ids.iter().next())
        .expect("player dynasty must own a business");
    let business = state
        .businesses
        .get_mut(business_id)
        .expect("player business must exist");
    business.operations.status = BusinessStatus::Distressed;
    business.finance.cash = Money::ZERO;
}

fn make_property_liquidation_available_for_test(state: &mut AppState) {
    let player_id = state.player_dynasty_id;
    state
        .properties
        .values()
        .find(|property| {
            property.owner_dynasty_id == Some(player_id) && property.collateral_loan_id.is_none()
        })
        .expect("player must own an unpledged property for liquidation coverage");
    state
        .dynasties
        .get_mut(&player_id)
        .expect("player dynasty must exist")
        .resources
        .treasury = Money::from_copper(58);
    make_player_business_distressed(state);
    let business_id = *player_business_ids_for_test(state)
        .first()
        .expect("player business must exist");
    state
        .businesses
        .get_mut(business_id)
        .expect("player business must exist")
        .operations
        .condition_basis_points = 1_000;
    for dynasty in state
        .dynasties
        .values_mut()
        .filter(|dynasty| dynasty.id() != player_id)
    {
        dynasty.resources.treasury = Money::from_copper(1_000_000);
    }
}

fn add_active_crisis(state: &mut AppState) {
    let id = state.next_ids.crisis();
    state.crises.insert(
        id,
        Crisis {
            id,
            kind: CrisisKind::NobleDemand,
            district_id: state.districts.keys().copied().next(),
            started_day: state.clock.day(),
            severity_basis_points: 4_000,
            status: CrisisStatus::Active,
            cause: "gameplay harness candidate coverage".to_owned(),
        },
    );
}

fn make_player_labor_disputed(state: &mut AppState) {
    let employment_id = state
        .employment
        .values()
        .find(|agreement| {
            state
                .businesses
                .get(agreement.business_id)
                .is_some_and(|business| business.owner_dynasty_id() == state.player_dynasty_id)
        })
        .expect("player business must have employment")
        .id;
    state
        .employment
        .get_mut(&employment_id)
        .expect("employment must exist")
        .status = EmploymentStatus::Disputed;
}

fn make_player_contract_breached(state: &mut AppState) -> DynastyId {
    let player_id = state.player_dynasty_id;
    let (contract_id, defendant_id) = state
        .contracts
        .values()
        .find_map(|contract| {
            let buyer_owner = state
                .businesses
                .get(contract.buyer_business_id)
                .expect("contract buyer must exist")
                .owner_dynasty_id();
            let seller_owner = state
                .businesses
                .get(contract.seller_business_id)
                .expect("contract seller must exist")
                .owner_dynasty_id();
            if buyer_owner == player_id && seller_owner != player_id {
                Some((contract.id, seller_owner))
            } else if seller_owner == player_id && buyer_owner != player_id {
                Some((contract.id, buyer_owner))
            } else {
                None
            }
        })
        .expect("campaign must contain a player contract with another dynasty");
    let contract = state
        .contracts
        .get_mut(&contract_id)
        .expect("selected contract must exist");
    contract.status = ContractStatus::Breached;
    contract.breaching_dynasty_id = Some(defendant_id);
    contract.breach_victim_dynasty_id = Some(player_id);
    contract.unpaid_breach_penalty = contract.penalty;
    defendant_id
}
