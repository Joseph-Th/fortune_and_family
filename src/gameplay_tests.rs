//! Behavioral coverage for the deterministic gameplay harness.

use super::*;
use crate::core::{AuditKind, AuditRecord, Crisis, CrisisKind, OutboxKind, OutboxMessage};
use crate::ids::OutboxMessageId;
use crate::systems::{OFFICE_POWER_ESTABLISHMENT_DAYS, OFFICE_TERM_DAYS, issue_loan};
use crate::test_support::{assert_set_eq, make_test_campaign, rivergate_registry_for_test};
use std::sync::OnceLock;

static FOCUSED_REPORT_30_DAYS: OnceLock<GameplayHarnessReport> = OnceLock::new();
static FOCUSED_REPORT_360_DAYS: OnceLock<GameplayHarnessReport> = OnceLock::new();

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
        360 => FOCUSED_REPORT_360_DAYS.get_or_init(|| build_focused_report(360)),
        _ => panic!("no cached focused report is configured for {days} days"),
    };
    report.clone()
}

fn build_focused_report(days: u32) -> GameplayHarnessReport {
    run_gameplay_harness(rivergate_registry_for_test(), focused_config(days))
        .expect("cached focused gameplay report must build")
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

    #[test]
    fn default_harness_uses_the_monthly_strategic_cadence() {
        assert_eq!(GameplayHarnessConfig::default().decision_interval_days, 30);
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
    fn identical_configuration_produces_identical_report() {
        let registry = rivergate_registry_for_test();
        let config = focused_config(90);

        let first = run_gameplay_harness(registry, config.clone()).expect("first run must succeed");
        let second = run_gameplay_harness(registry, config).expect("second run must succeed");

        assert_eq!(first, second, "gameplay reports must be reproducible");
    }

    #[test]
    fn plays_through_real_commands_and_reports_system_reactions() {
        let registry = rivergate_registry_for_test();
        let mut config = focused_config(180);
        config.decision_interval_days = 7;
        let report =
            run_gameplay_harness(registry, config).expect("gameplay harness must complete");
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
    fn candidate_scenarios_cover_every_command_family() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_candidate_coverage_state(registry);
        let mut kinds = candidate_kinds_for_test(registry, &state);

        let mut directional_state = state.clone();
        make_supply_security_and_borrowing_available(&mut directional_state);
        kinds.extend(candidate_kinds_for_test(registry, &directional_state));

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

        let mut information_state = make_test_candidate_coverage_state(registry);
        for _ in 0..360 {
            information_state.clock.advance_one_day();
        }
        kinds.extend(candidate_kinds_for_test(registry, &information_state));

        let mut support_state = make_test_candidate_coverage_state(registry);
        for _ in 0..INSTITUTION_SUPPORT_INTERVAL_DAYS {
            support_state.clock.advance_one_day();
        }
        let player_character_ids: BTreeSet<_> = support_state
            .characters
            .iter()
            .filter(|character| character.dynasty_id() == support_state.player_dynasty_id)
            .map(crate::core::Character::id)
            .collect();
        let support_institution_id = support_state
            .institutions
            .values()
            .find(|institution| {
                institution
                    .office_holder_id
                    .is_none_or(|holder_id| !player_character_ids.contains(&holder_id))
            })
            .expect("fixture must contain an institution without a player officeholder")
            .institution_id;
        support_state
            .institutions
            .get_mut(&support_institution_id)
            .expect("support institution must exist")
            .members
            .retain(|character_id| !player_character_ids.contains(character_id));
        let subject_prefix = format!("institution:{support_institution_id}:");
        support_state.audit_log.retain(|record| {
            record.kind() != AuditKind::InstitutionPatronage
                || !record.subject().starts_with(&subject_prefix)
        });
        kinds.extend(candidate_kinds_for_test(registry, &support_state));

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
        let seller =
            find_contract_seller(registry, &state, input.good_id(), state.player_dynasty_id)
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
            ),
            "agents must not propose supply contracts the buyer cannot finance"
        );
    }

    #[test]
    fn rendered_report_surfaces_scores_findings_and_traces() {
        let registry = rivergate_registry_for_test();
        let report = run_gameplay_harness(registry, focused_config(60))
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
        grant_office_nomination_record_for_test(&mut state);
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
    fn illiquid_officeholder_is_offered_executable_institutional_withdrawal() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        make_institution_withdrawal_available_for_test(&mut state);
        assert!(has_institution_withdrawal_opportunity(&state));
        let mut accumulator = CampaignAccumulator::new();
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut accumulator,
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
    fn distressed_asset_owner_is_offered_executable_property_liquidation() {
        let registry = rivergate_registry_for_test();
        let healthy_state = make_test_campaign();
        assert!(
            !has_property_liquidation_opportunity(registry, &healthy_state),
            "liquidation must not be considered active for a healthy campaign"
        );
        let mut state = make_test_campaign();
        make_property_liquidation_available_for_test(&mut state);
        let player_id = state.player_dynasty_id;
        assert!(has_property_liquidation_opportunity(registry, &state));
        let mut accumulator = CampaignAccumulator::new();
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::Steward,
            &mut accumulator,
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
            &state,
            GameplayPersona::PowerBroker
        ));
        let mut accumulator = CampaignAccumulator::new();
        record_activation_opportunities(
            registry,
            &state,
            GameplayPersona::PowerBroker,
            &mut accumulator,
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

        let candidate = single_candidate(&candidates, "planned modernization");
        assert!(matches!(
            candidate.command,
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
            subject: format!("business:{source_id}->business:{target_id}"),
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
    fn severe_business_rehabilitation_can_use_treasury_above_household_emergency_reserve() {
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
            "commissioned intelligence should enter the agent loop after the foundation year"
        );
        for _ in 0..360 {
            state.clock.advance_one_day();
        }

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
            "the annual commission interval must prevent repetitive intelligence housekeeping"
        );
    }

    #[test]
    fn power_broker_preserves_political_reserves_before_extending_credit() {
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = Money::from_copper(49_999);
        let mut candidates = Vec::new();

        add_lend_candidate(&state, GameplayPersona::PowerBroker, &mut candidates);

        assert!(
            candidates.is_empty(),
            "a power broker must not sacrifice campaign and office reserves to habitual lending"
        );
    }
}

mod metrics {
    use super::*;

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

        let (selected, viable, substantive, kinds, rejections) = probe_candidates(
            registry,
            &state,
            [acknowledgement, governance].into_iter(),
            &mut accumulator,
        );

        assert_eq!(viable, 2);
        assert_eq!(substantive, 1);
        assert!(rejections.is_empty());
        assert_eq!(
            kinds,
            [GameplayCommandKind::SetHouseGovernance]
                .into_iter()
                .collect()
        );
        assert_eq!(
            selected.expect("a candidate must be selected").kind,
            GameplayCommandKind::SetHouseGovernance,
            "housekeeping may be a fallback but must not consume a strategic decision cycle"
        );
    }

    #[test]
    fn automatic_notification_housekeeping_clears_backlog_without_advancing_the_campaign() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
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
        assert_eq!(command_stats.actions_with_persistent_consequences, 1);
        assert!(
            command_stats
                .changed_domains
                .contains(&GameplayDomain::Feedback)
        );
        assert!(
            ranked_candidates(registry, &state, GameplayPersona::Steward, &accumulator,)
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
    fn substantive_action_gaps_include_quiet_and_housekeeping_cycles() {
        let mut accumulator = CampaignAccumulator::new();

        accumulator.record_action_gap(None, 180);
        accumulator.record_action_gap(Some(GameplayCommandKind::AcknowledgeNotification), 180);

        assert_eq!(accumulator.longest_substantive_action_gap_days, 360);
        accumulator.record_action_gap(Some(GameplayCommandKind::EnactLaw), 30);
        assert_eq!(accumulator.current_substantive_action_gap_days, 0);
        assert_eq!(accumulator.longest_substantive_action_gap_days, 360);
    }

    #[test]
    fn personas_value_distinct_institutional_powers() {
        let public_works = BTreeSet::from([OfficePower::PublicWorks]);
        let market_tolls = BTreeSet::from([OfficePower::MarketTolls]);
        let debt = BTreeSet::from([OfficePower::DebtEnforcement]);
        let taxation = BTreeSet::from([OfficePower::Taxation]);

        assert!(
            institution_power_bonus(GameplayPersona::Steward, &public_works)
                > institution_power_bonus(GameplayPersona::Entrepreneur, &public_works)
        );
        assert!(
            institution_power_bonus(GameplayPersona::Entrepreneur, &market_tolls)
                > institution_power_bonus(GameplayPersona::Steward, &market_tolls)
        );
        assert!(
            institution_power_bonus(GameplayPersona::Opportunist, &debt)
                > institution_power_bonus(GameplayPersona::Steward, &debt)
        );
        assert!(
            institution_power_bonus(GameplayPersona::PowerBroker, &taxation)
                > institution_power_bonus(GameplayPersona::Entrepreneur, &taxation)
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
        grant_office_nomination_record_for_test(&mut state);
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
            u32::try_from(FAMILY_EDUCATION_INTERVAL_DAYS)
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

        accumulator.record_executed_command(GameplayCommandKind::CultivateInstitutionSupport, 180);

        snapshot.day = 360;
        snapshot.player_contract_deliveries = OFFICE_NOMINATION_DELIVERY_REQUIREMENT;
        accumulator.observe_snapshot(&snapshot);
        accumulator.record_executed_command(GameplayCommandKind::NominateForOffice, 377);

        snapshot.day = 440;
        snapshot.offices_held = 1;
        accumulator.observe_snapshot(&snapshot);
        accumulator.record_executed_command(GameplayCommandKind::StartPublicWork, 454);

        snapshot.day = 900;
        snapshot.player_disputed_employment = 1;
        accumulator.observe_snapshot(&snapshot);

        snapshot.day = 5_200;
        snapshot.generation = snapshot.generation.saturating_add(1);
        accumulator.observe_snapshot(&snapshot);

        assert_eq!(
            accumulator.fantasy_arc,
            GameplayFantasyArc {
                first_reputation_standing_day: Some(70),
                first_commercial_standing_day: Some(360),
                first_institution_support_day: Some(180),
                first_office_campaign_day: Some(377),
                first_office_day: Some(440),
                first_city_shaping_action_day: Some(454),
                first_player_labor_dispute_day: Some(900),
                first_succession_day: Some(5_200),
            }
        );
    }
}

mod findings {
    use super::*;

    #[test]
    fn findings_surface_political_power_that_precedes_commercial_standing() {
        let mut report = cached_focused_report(360);
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
        let mut report = cached_focused_report(360);
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
            "Established dynasties often avoid measured internal exposure",
        );
    }

    #[test]
    fn material_civic_duties_count_as_power_exposure() {
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

        assert_finding_absent(
            &findings,
            "Established dynasties often avoid measured internal exposure",
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
        let contract_finding = finding_with_title(
            &report.findings,
            "contracts domain changed before a player route became available",
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
        assert_eq!(contract_finding.severity, GameplayFindingSeverity::Info);
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
    fn event_driven_routes_without_a_trigger_remain_informational() {
        let mut report = cached_focused_report(30);
        report.aggregate.simulated_days = 7_200;

        let findings = derive_findings(&report.aggregate, &report.campaigns);
        let finding = finding_with_title(
            &findings,
            "labor-response was not exercised in this horizon",
        );

        assert_eq!(finding.severity, GameplayFindingSeverity::Info);
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
    grant_office_nomination_record_for_test(&mut state);
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

fn grant_office_nomination_record_for_test(state: &mut AppState) {
    let player_businesses: BTreeSet<_> = player_business_ids_for_test(state).into_iter().collect();
    let contract = state
        .contracts
        .values_mut()
        .find(|contract| {
            player_businesses.contains(&contract.buyer_business_id)
                || player_businesses.contains(&contract.seller_business_id)
        })
        .expect("campaign must contain a player contract");
    let deliveries = u16::try_from(OFFICE_NOMINATION_DELIVERY_REQUIREMENT)
        .expect("office delivery requirement must fit contract counters");
    contract.fulfilled_deliveries = deliveries;
    contract
        .fulfilled_deliveries_by_dynasty
        .insert(state.player_dynasty_id, deliveries);
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
                subject: format!("institution:{institution_id}:character:{character_id}"),
                detail: "test support".to_owned(),
            });
        }
    }
    state.audit_log.sort_by_key(AuditRecord::day);
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
    state
        .contracts
        .get_mut(&contract_id)
        .expect("selected contract must exist")
        .status = ContractStatus::Breached;
    state
        .contracts
        .get_mut(&contract_id)
        .expect("selected contract must exist")
        .breaching_dynasty_id = Some(defendant_id);
    defendant_id
}
