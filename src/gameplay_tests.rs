//! Behavioral coverage for the deterministic gameplay harness.

use super::*;
use crate::core::{Crisis, CrisisKind};
use crate::registry::build_rivergate_registry;

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
fn rendered_report_surfaces_scores_findings_and_traces() {
    let registry = build_rivergate_registry();
    let report = run_gameplay_harness(&registry, focused_config(60))
        .expect("gameplay harness must complete");

    let rendered = render_gameplay_report(&report);

    for heading in [
        "scores:",
        "Command coverage",
        "Strongest observed command consequences",
        "Findings",
        "Representative decisions",
    ] {
        assert!(rendered.contains(heading), "report must contain {heading}");
    }
    serde_json::to_string(&report).expect("report must serialize to JSON");
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
