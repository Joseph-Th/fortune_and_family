//! Behavioral coverage for the deterministic gameplay harness.

use super::*;
use crate::core::{Crisis, CrisisKind, OutboxKind, OutboxMessage};
use crate::ids::OutboxMessageId;
use crate::registry::build_rivergate_registry;
use crate::test_support::make_test_campaign;

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

#[test]
fn rejects_empty_campaign_dimensions() {
    let registry = build_rivergate_registry();
    let mut config = focused_config(30);
    config.personas.clear();

    let error = run_gameplay_harness(&registry, config).expect_err("empty personas must fail");

    assert!(matches!(error, GameplayHarnessError::InvalidConfig { .. }));
}

#[test]
fn identical_configuration_produces_identical_report() {
    let registry = build_rivergate_registry();
    let config = focused_config(90);

    let first = run_gameplay_harness(&registry, config.clone()).expect("first run must succeed");
    let second = run_gameplay_harness(&registry, config).expect("second run must succeed");

    assert_eq!(first, second, "gameplay reports must be reproducible");
}

#[test]
fn plays_through_real_commands_and_reports_system_reactions() {
    let registry = build_rivergate_registry();
    let report = run_gameplay_harness(&registry, focused_config(180))
        .expect("gameplay harness must complete");
    let campaign = report.campaigns.first().expect("one campaign must run");

    assert_eq!(report.schema_version, GAMEPLAY_REPORT_SCHEMA_VERSION);
    assert_eq!(report.aggregate.campaigns, 1);
    assert_eq!(report.aggregate.simulated_days, 180);
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
    assert!(report.aggregate.command_coverage >= 6);
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
}

#[test]
fn candidate_builder_can_reach_every_command_family() {
    let registry = build_rivergate_registry();
    let mut state = build_new_game(
        &registry,
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
    add_active_crisis(&mut state);
    make_player_labor_disputed(&mut state);
    for _ in 0..LEGAL_CASE_FILING_INTERVAL_DAYS {
        state.clock.advance_one_day();
    }

    let candidates = ranked_candidates(
        &registry,
        &state,
        GameplayPersona::Opportunist,
        &CampaignAccumulator::new(),
    );
    let mut kinds: BTreeSet<_> = candidates
        .into_iter()
        .map(|candidate| candidate.kind)
        .collect();
    make_player_business_distressed(&mut state);
    kinds.extend(
        ranked_candidates(
            &registry,
            &state,
            GameplayPersona::Opportunist,
            &CampaignAccumulator::new(),
        )
        .into_iter()
        .map(|candidate| candidate.kind),
    );

    assert_eq!(
        kinds,
        ALL_COMMAND_KINDS.into_iter().collect(),
        "state-derived candidates must cover every player command family"
    );
}

#[test]
fn contract_candidates_require_buyer_working_cash() {
    let registry = build_rivergate_registry();
    let mut state = build_new_game(
        &registry,
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
    let seller = find_contract_seller(&registry, &state, input.good_id(), state.player_dynasty_id)
        .expect("a nonplayer seller must exist");
    state
        .businesses
        .get_mut(buyer)
        .expect("buyer must exist")
        .finance
        .cash = Money::ZERO;

    assert!(
        !contract_terms_are_operationally_supported(
            &registry,
            &state,
            buyer,
            seller,
            input.good_id(),
            input.quantity().saturating_mul_ratio(4, 1),
        ),
        "agents must not propose supply contracts the buyer cannot finance"
    );
}

#[test]
fn rendered_report_surfaces_scores_findings_and_traces() {
    let registry = build_rivergate_registry();
    let report = run_gameplay_harness(&registry, focused_config(60))
        .expect("gameplay harness must complete");

    let rendered = render_gameplay_report(&report);

    for heading in [
        "scores:",
        "Experience health",
        "Command coverage",
        "Strongest observed command consequences",
        "Findings",
        "Representative decisions",
    ] {
        assert!(rendered.contains(heading), "report must contain {heading}");
    }
    serde_json::to_string(&report).expect("report must serialize to JSON");
}

#[test]
fn notification_housekeeping_is_offered_only_for_a_meaningful_batch() {
    let mut state = make_test_campaign();
    state.outbox.clear();
    for index in 1..NOTIFICATION_BATCH_THRESHOLD {
        state.outbox.push(OutboxMessage {
            id: OutboxMessageId::new(u32::try_from(index).expect("test index fits u32")),
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
    state.outbox.push(OutboxMessage {
        id: OutboxMessageId::new(u32::try_from(index).expect("test index fits u32")),
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
fn findings_surface_a_single_complete_food_collapse() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");
    let campaign = report
        .campaigns
        .first_mut()
        .expect("focused configuration must produce one campaign");
    campaign.end.average_food_satisfaction = 0;
    campaign.minimum_food_satisfaction = 0;

    let findings = derive_findings(&report.aggregate, &report.campaigns);

    assert!(findings.iter().any(|finding| {
        finding.title == "At least one campaign experiences complete food collapse"
    }));
}

#[test]
fn findings_surface_single_campaign_notification_overload() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");
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
    report.campaigns[0].maximum_unread_notifications = 101;
    for campaign in &mut report.campaigns[1..] {
        campaign.maximum_unread_notifications = 0;
    }

    let findings = derive_findings(&report.aggregate, &report.campaigns);

    assert!(findings.iter().any(|finding| {
        finding.title == "Individual campaigns experience notification overload"
    }));
}

#[test]
fn short_horizon_absent_reactive_commands_are_informational() {
    let registry = build_rivergate_registry();
    let report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");

    let finding = report
        .findings
        .iter()
        .find(|finding| finding.title == "crisis-response was not exercised in this horizon")
        .expect("a short run must explain absent event-driven commands");
    let contract_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.title == "contracts domain changed before a player route became available"
        })
        .expect("autonomous short-horizon domains must explain missing player routes");
    let legal_finding = report
        .findings
        .iter()
        .find(|finding| finding.title == "legal-case was not exercised in this horizon")
        .expect("short runs must not require litigation before its prerequisites develop");
    let crisis_domain_finding = report
        .findings
        .iter()
        .find(|finding| finding.title == "crises domain was inactive in this horizon")
        .expect("short runs must classify inactive event domains informationally");

    assert_eq!(finding.severity, GameplayFindingSeverity::Info);
    assert_eq!(contract_finding.severity, GameplayFindingSeverity::Info);
    assert_eq!(legal_finding.severity, GameplayFindingSeverity::Info);
    assert_eq!(
        crisis_domain_finding.severity,
        GameplayFindingSeverity::Info
    );
}

#[test]
fn long_horizon_absent_command_routes_are_critical() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");
    report.aggregate.simulated_days = 720;

    let findings = derive_findings(&report.aggregate, &report.campaigns);
    let finding = findings
        .iter()
        .find(|finding| finding.title == "labor-response had no reachable candidate")
        .expect("a long run must require labor-response reachability");

    assert_eq!(finding.severity, GameplayFindingSeverity::Critical);
}

#[test]
fn findings_surface_public_work_overload() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");
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

    assert!(findings.iter().any(|finding| {
        finding.title == "Public works accumulate faster than the city can execute them"
    }));
}

#[test]
fn findings_use_player_contract_outcomes_not_only_citywide_totals() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");
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

    assert!(findings.iter().any(|finding| {
        finding.title == "Player contracts breach more often than they complete"
    }));
}

#[test]
fn political_reachability_uses_peak_office_attainment_not_endpoint_incumbency() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");
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

    assert!(
        findings
            .iter()
            .all(|finding| { finding.title != "Office nominations never produce political power" })
    );
}

#[test]
fn findings_surface_complete_player_capture_of_all_offices() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");
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

    assert!(
        findings
            .iter()
            .any(|finding| { finding.title == "Player captures every political office" })
    );
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
