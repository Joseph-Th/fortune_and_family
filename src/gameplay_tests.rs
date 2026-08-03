//! Behavioral coverage for the deterministic gameplay harness.

use super::*;
use crate::core::{AuditKind, AuditRecord, Crisis, CrisisKind, OutboxKind, OutboxMessage};
use crate::ids::OutboxMessageId;
use crate::registry::build_rivergate_registry;
use crate::systems::issue_loan;
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
    let mut state = make_test_candidate_coverage_state(&registry);
    let mut kinds = candidate_kinds_for_test(&registry, &state);

    let mut directional_state = state.clone();
    make_supply_security_and_borrowing_available(&mut directional_state);
    kinds.extend(candidate_kinds_for_test(&registry, &directional_state));

    make_player_business_distressed(&mut state);
    kinds.extend(candidate_kinds_for_test(&registry, &state));
    restore_distressed_business_cash_for_test(&mut state);
    add_active_crisis(&mut state);
    make_player_labor_disputed(&mut state);
    make_player_contract_breached(&mut state);
    for _ in 0..LEGAL_CASE_FILING_INTERVAL_DAYS {
        state.clock.advance_one_day();
    }
    kinds.extend(candidate_kinds_for_test(&registry, &state));

    remove_internal_transfer_surplus_for_test(&mut state);
    make_player_business_distressed(&mut state);
    kinds.extend(candidate_kinds_for_test(&registry, &state));

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
        !can_support_contract_terms(
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
        "Harness limits",
        "Representative decisions",
    ] {
        assert!(rendered.contains(heading), "report must contain {heading}");
    }
    serde_json::to_string(&report).expect("report must serialize to JSON");
    assert_eq!(report.limitations.len(), 3);
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

    assert_eq!(candidates.len(), 1);
    assert!(matches!(
        candidates[0].command,
        PlayerCommand::SetBusinessPolicy {
            maintenance_basis_points: 1_300,
            minimum_cash_reserve,
            ..
        } if minimum_cash_reserve == Money::from_copper(8_000)
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

    assert_eq!(candidates.len(), 1);
    assert!(matches!(
        candidates[0].command,
        PlayerCommand::InvestInBusiness { amount, .. }
            if amount >= Money::from_copper(1_000)
                && amount <= AGENT_PLANNED_CAPITALIZATION_MAX
    ));

    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::BusinessCapitalization,
        subject: format!("business:{business_id}"),
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
    let registry = build_rivergate_registry();
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
    generate_cash_rebalance_candidate(&registry, &state, &businesses, &mut candidates);
    assert!(candidates.is_empty());

    let target_id = business_ids[1];
    let target = state
        .businesses
        .get_mut(target_id)
        .expect("target business must exist");
    target.finance.cash = Money::ZERO;
    target.operations.status = BusinessStatus::Distressed;
    let businesses: Vec<_> = business_ids
        .iter()
        .filter_map(|business_id| state.businesses.get(*business_id))
        .collect();
    generate_cash_rebalance_candidate(&registry, &state, &businesses, &mut candidates);

    assert_eq!(candidates.len(), 1);
    let PlayerCommand::TransferBusinessCash {
        from_business_id,
        to_business_id,
        amount,
    } = candidates[0].command
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
        .saturating_sub(business_cash_target(&registry, &state, source));
    let buffered_deficit = business_cash_target(&registry, &state, target)
        .saturating_add(AGENT_CASH_REBALANCE_BUFFER)
        .saturating_sub(target.cash());
    assert_eq!(amount, source_surplus.min(buffered_deficit));
    assert!(amount >= AGENT_CASH_REBALANCE_TRIGGER);
}

#[test]
fn cash_rebalancing_waits_between_portfolio_interventions() {
    let registry = build_rivergate_registry();
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
    let target = state
        .businesses
        .get_mut(business_ids[1])
        .expect("target business must exist");
    target.finance.cash = Money::ZERO;
    target.operations.status = BusinessStatus::Distressed;
    state.audit_log.push(AuditRecord {
        day: state.clock.day(),
        kind: AuditKind::CashTransfer,
        subject: format!("business:{}->business:{}", business_ids[0], business_ids[1]),
        detail: "amount=1000".to_owned(),
    });
    let businesses: Vec<_> = business_ids
        .iter()
        .filter_map(|business_id| state.businesses.get(*business_id))
        .collect();
    let mut candidates = Vec::new();

    generate_cash_rebalance_candidate(&registry, &state, &businesses, &mut candidates);

    assert!(candidates.is_empty());

    for _ in 0..AGENT_CASH_REBALANCE_INTERVAL_DAYS {
        state.clock.advance_one_day();
    }
    let businesses: Vec<_> = business_ids
        .iter()
        .filter_map(|business_id| state.businesses.get(*business_id))
        .collect();
    generate_cash_rebalance_candidate(&registry, &state, &businesses, &mut candidates);

    assert_eq!(candidates.len(), 1);
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
        "worker replacement must not mask unaffordable unsafe conditions"
    );

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
        .collect();
    assert_eq!(labor_candidates.len(), 1);
    assert!(matches!(
        labor_candidates[0].command,
        PlayerCommand::ResolveLaborDispute {
            response: LaborResponse::ImproveConditions,
            ..
        }
    ));
}

#[test]
fn acquisition_waits_until_the_existing_portfolio_is_healthy_and_funded() {
    let registry = build_rivergate_registry();
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
        &registry,
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
        &registry,
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
    assert_eq!(candidates.len(), 1);
    assert!(matches!(
        candidates[0].command,
        PlayerCommand::IssueLoan { ref terms }
            if terms.weekly_payment
                == Money::from_copper(
                    (terms.principal.copper() / AGENT_LOAN_AMORTIZATION_WEEKS).max(1)
                )
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

    assert_eq!(candidates.len(), 1);
    assert!(matches!(
        candidates[0].command,
        PlayerCommand::IssueLoan { ref terms }
            if terms.lender_dynasty_id != first_lender_id
    ));
}

#[test]
fn severe_business_rehabilitation_can_use_treasury_above_household_emergency_reserve() {
    let registry = build_rivergate_registry();
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
        &registry,
        &state,
        GameplayPersona::Steward,
        business,
        &mut candidates,
    );

    assert_eq!(candidates.len(), 1);
    assert!(matches!(
        candidates[0].command,
        PlayerCommand::InvestInBusiness { amount, .. }
            if amount > Money::ZERO && amount <= Money::from_copper(3_000)
    ));
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
        "agents must not manufacture quarterly lawsuits without a debt, breach, or hostile relationship"
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
        &build_rivergate_registry(),
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
    identity_change.active_law_checksum += 1;
    identity_change.player_completed_public_work_checksum += 1;
    let domains = earlier.changed_domains(&identity_change);
    assert!(domains.contains(&GameplayDomain::Institutions));
    assert!(domains.contains(&GameplayDomain::Law));
    assert!(domains.contains(&GameplayDomain::Districts));
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

    assert_eq!(selected.len(), 3);
    assert_eq!(kinds.len(), 3);
    assert_eq!(selected[0].kind, GameplayCommandKind::NominateForOffice);
    assert!(kinds.contains(&GameplayCommandKind::EnactLaw));
    assert!(kinds.contains(&GameplayCommandKind::BuyProperty));
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
fn substantive_command_streaks_ignore_notification_housekeeping() {
    let mut accumulator = CampaignAccumulator::new();
    for day in (0..56).step_by(7) {
        accumulator.record_executed_command(GameplayCommandKind::TransferBusinessCash, day);
    }
    accumulator.record_executed_command(GameplayCommandKind::AcknowledgeNotification, 56);
    accumulator.record_executed_command(GameplayCommandKind::TransferBusinessCash, 63);

    assert_eq!(accumulator.longest_substantive_command_streak, 9);
    assert_eq!(
        accumulator.longest_substantive_streak_command,
        Some(GameplayCommandKind::TransferBusinessCash)
    );

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

    assert_eq!(accumulator.longest_substantive_command_streak, 2);
    assert_eq!(accumulator.current_substantive_command_streak, 1);
}

#[test]
fn fantasy_arc_records_commercial_political_and_dynastic_milestones() {
    let state = make_test_campaign();
    let mut snapshot = GameplaySnapshot::capture(&state);
    let mut accumulator = CampaignAccumulator::new();
    accumulator.observe_initial_snapshot(&snapshot);

    snapshot.day = 70;
    snapshot.quality_reputation = OFFICE_NOMINATION_REPUTATION_REQUIREMENT;
    accumulator.observe_snapshot(&snapshot);
    accumulator.record_executed_command(GameplayCommandKind::NominateForOffice, 77);

    snapshot.day = 140;
    snapshot.offices_held = 1;
    accumulator.observe_snapshot(&snapshot);
    accumulator.record_executed_command(GameplayCommandKind::StartPublicWork, 154);

    snapshot.day = 900;
    snapshot.player_disputed_employment = 1;
    accumulator.observe_snapshot(&snapshot);

    snapshot.day = 5_200;
    snapshot.generation = snapshot.generation.saturating_add(1);
    accumulator.observe_snapshot(&snapshot);

    assert_eq!(
        accumulator.fantasy_arc,
        GameplayFantasyArc {
            first_commercial_standing_day: Some(70),
            first_office_campaign_day: Some(77),
            first_office_day: Some(140),
            first_city_shaping_action_day: Some(154),
            first_player_labor_dispute_day: Some(900),
            first_succession_day: Some(5_200),
        }
    );
}

#[test]
fn findings_surface_political_power_that_precedes_commercial_standing() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(360))
        .expect("gameplay harness must complete");
    let campaign = report
        .campaigns
        .first_mut()
        .expect("focused configuration must produce one campaign");
    campaign.fantasy_arc.first_commercial_standing_day = Some(90);
    campaign.fantasy_arc.first_office_campaign_day = Some(30);

    let findings = derive_findings(&report.aggregate, &report.campaigns);

    assert!(
        findings
            .iter()
            .any(|finding| finding.title == "Political ascent precedes commercial standing")
    );
}

#[test]
fn findings_surface_synchronized_core_fantasy_timing() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(360))
        .expect("gameplay harness must complete");
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
        campaign.fantasy_arc.first_office_campaign_day = Some(70);
        campaign.fantasy_arc.first_office_day = Some(154);
        campaign.fantasy_arc.first_city_shaping_action_day = Some(154);
        campaign
    })
    .collect();

    let findings = derive_findings(&report.aggregate, &report.campaigns);

    assert!(
        findings
            .iter()
            .any(|finding| finding.title == "Core fantasy timing is highly synchronized")
    );
}

#[test]
fn findings_surface_long_repetitive_command_streaks() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");
    let campaign = report
        .campaigns
        .first_mut()
        .expect("focused configuration must produce one campaign");
    campaign.longest_substantive_command_streak = 8;
    campaign.longest_substantive_streak_command = Some(GameplayCommandKind::TransferBusinessCash);

    let findings = derive_findings(&report.aggregate, &report.campaigns);

    assert!(findings.iter().any(|finding| {
        finding.title == "Repeated command streak resembles routine micromanagement"
    }));
}

#[test]
fn choice_depth_within_a_focused_family_is_informational() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");
    report.aggregate.decision_cycles = 10;
    report.aggregate.quiet_cycles = 0;
    report.aggregate.viable_choices = 30;
    report.aggregate.viable_command_kinds = 10;
    report.aggregate.cycles_with_multiple_viable_command_kinds = 0;

    let findings = derive_findings(&report.aggregate, &report.campaigns);
    let finding = findings
        .iter()
        .find(|finding| {
            finding.title == "Strategic alternatives concentrate within command families"
        })
        .expect("focused but deep choices must remain visible");

    assert_eq!(finding.severity, GameplayFindingSeverity::Info);
    assert!(!findings.iter().any(|finding| {
        finding.title == "Actionable cycles offer too few meaningful alternatives"
    }));
}

#[test]
fn genuinely_sparse_actionable_cycles_remain_a_warning() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");
    report.aggregate.decision_cycles = 10;
    report.aggregate.quiet_cycles = 0;
    report.aggregate.viable_choices = 10;
    report.aggregate.viable_command_kinds = 10;
    report.aggregate.cycles_with_multiple_viable_command_kinds = 0;

    let findings = derive_findings(&report.aggregate, &report.campaigns);
    let finding = findings
        .iter()
        .find(|finding| finding.title == "Actionable cycles offer too few meaningful alternatives")
        .expect("one-option actionable cycles must be warned about");

    assert_eq!(finding.severity, GameplayFindingSeverity::Warning);
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
    report
        .aggregate
        .commands
        .get_mut(&GameplayCommandKind::ResolveLaborDispute)
        .expect("all command statistics must exist")
        .activation_opportunities = 1;

    let findings = derive_findings(&report.aggregate, &report.campaigns);
    let finding = findings
        .iter()
        .find(|finding| finding.title == "labor-response had no reachable candidate")
        .expect("a long run must require labor-response reachability");

    assert_eq!(finding.severity, GameplayFindingSeverity::Critical);
}

#[test]
fn event_driven_routes_without_a_trigger_remain_informational() {
    let registry = build_rivergate_registry();
    let mut report = run_gameplay_harness(&registry, focused_config(30))
        .expect("gameplay harness must complete");
    report.aggregate.simulated_days = 7_200;

    let findings = derive_findings(&report.aggregate, &report.campaigns);
    let finding = findings
        .iter()
        .find(|finding| finding.title == "labor-response was not exercised in this horizon")
        .expect("an untriggered reactive route must be explained informationally");

    assert_eq!(finding.severity, GameplayFindingSeverity::Info);
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
    ranked_candidates(
        registry,
        state,
        GameplayPersona::Opportunist,
        &CampaignAccumulator::new(),
    )
    .into_iter()
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
    let holder_id = state
        .dynasties
        .get(&state.player_dynasty_id)
        .expect("player dynasty must exist")
        .head_id();
    state
        .institutions
        .values_mut()
        .find(|institution| institution.office_holder_id.is_none())
        .expect("campaign must contain a vacant office")
        .office_holder_id = Some(holder_id);
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
    state
        .contracts
        .get_mut(&contract_id)
        .expect("selected contract must exist")
        .status = ContractStatus::Breached;
    defendant_id
}
