//! Projection completeness and HTML escaping tests for presentation adapters.

use super::*;
use crate::core::{CivicDebt, EnactedLaw, NewGameConfig};
use crate::test_support::{
    make_test_campaign, make_test_campaign_with, rivergate_registry_for_test,
};
use std::collections::BTreeSet;

mod coverage {
    use super::*;

    fn assert_registry_views(
        registry: &crate::registry::Registry,
        projection: &CampaignProjection,
    ) {
        assert_eq!(
            projection
                .districts
                .iter()
                .map(|district| district.id)
                .collect::<BTreeSet<_>>(),
            registry
                .districts()
                .iter()
                .map(crate::registry::DistrictDef::id)
                .collect(),
            "district projection IDs must match the registry"
        );
        assert_eq!(
            projection
                .market
                .iter()
                .map(|market| market.good.as_str())
                .collect::<BTreeSet<_>>(),
            registry
                .goods()
                .iter()
                .map(crate::registry::GoodDef::name)
                .collect(),
            "market projection names must match the registry"
        );
        assert_eq!(
            projection
                .institutions
                .iter()
                .map(|institution| institution.id)
                .collect::<BTreeSet<_>>(),
            registry
                .institutions()
                .iter()
                .map(crate::registry::InstitutionDef::id)
                .collect(),
            "every authored institution must appear exactly once"
        );
    }

    fn assert_runtime_views(state: &AppState, projection: &CampaignProjection) {
        assert_eq!(
            projection
                .dynasties
                .iter()
                .map(|dynasty| dynasty.id)
                .collect::<BTreeSet<_>>(),
            state.dynasties.keys().copied().collect(),
            "dynasty projection IDs must match runtime state"
        );
        assert_eq!(
            projection
                .contracts
                .iter()
                .map(|contract| contract.id)
                .collect::<BTreeSet<_>>(),
            state.contracts.keys().copied().collect()
        );
        assert_eq!(
            projection
                .businesses
                .iter()
                .map(|business| business.id)
                .collect::<BTreeSet<_>>(),
            state
                .businesses
                .iter()
                .map(crate::core::Business::id)
                .collect(),
            "business projection IDs must match runtime state"
        );
        assert_eq!(
            projection
                .loans
                .iter()
                .map(|loan| loan.id)
                .collect::<BTreeSet<_>>(),
            state.loans.keys().copied().collect()
        );
        assert_eq!(
            projection
                .civic_debts
                .iter()
                .map(|debt| debt.id)
                .collect::<BTreeSet<_>>(),
            state.civic_debts.keys().copied().collect()
        );
        assert_eq!(
            projection
                .properties
                .iter()
                .map(|property| property.id)
                .collect::<BTreeSet<_>>(),
            state.properties.keys().copied().collect()
        );
        assert_eq!(
            projection
                .laws
                .iter()
                .map(|law| law.id)
                .collect::<BTreeSet<_>>(),
            state.laws.keys().copied().collect()
        );
        assert_eq!(
            projection
                .public_works
                .iter()
                .map(|work| work.id)
                .collect::<BTreeSet<_>>(),
            state.public_works.keys().copied().collect()
        );
        assert_eq!(
            projection
                .legal_cases
                .iter()
                .map(|case| case.id)
                .collect::<BTreeSet<_>>(),
            state.legal_cases.keys().copied().collect()
        );
        assert_eq!(
            projection
                .crises
                .iter()
                .map(|crisis| crisis.id)
                .collect::<BTreeSet<_>>(),
            state.crises.keys().copied().collect()
        );
    }

    fn assert_filtered_views(state: &AppState, projection: &CampaignProjection) {
        assert_eq!(
            projection.relationships.len(),
            state.dynasties.len().saturating_sub(1),
            "relationship projection must expose every other dynasty's relationship to the player"
        );
        assert!(projection.relationships.iter().all(|relationship| {
            relationship.dynasty_id != state.player_dynasty_id
                && state.relationships.values().any(|runtime| {
                    runtime.pair.first == state.player_dynasty_id
                        && runtime.pair.second == relationship.dynasty_id
                        || runtime.pair.second == state.player_dynasty_id
                            && runtime.pair.first == relationship.dynasty_id
                })
        }));
        assert_eq!(
            projection.information.len(),
            state
                .information_reports
                .values()
                .filter(|report| report.owner_dynasty_id == state.player_dynasty_id)
                .count(),
            "information projection must include only player-owned reports"
        );
        assert!(
            projection
                .information
                .iter()
                .zip(
                    state
                        .information_reports
                        .values()
                        .filter(|report| report.owner_dynasty_id == state.player_dynasty_id)
                )
                .all(|(projected, report)| projected.target == report.target)
        );
        assert_eq!(
            projection.notifications.len(),
            state.outbox.len().min(50),
            "notification projection must honor its 50-message cap"
        );
    }

    #[test]
    fn summary_and_campaign_projection_share_the_same_read_model() {
        let registry = rivergate_registry_for_test();
        let state = make_test_campaign();

        let summary = build_state_summary(registry, &state);
        let projection = build_campaign_projection(registry, &state);

        assert_eq!(summary.scenario_name, projection.scenario.name);
        assert_eq!(summary.year, projection.scenario.year);
        assert_eq!(summary.day_of_year, projection.scenario.day_of_year);
        assert_eq!(summary.elapsed_days, projection.scenario.elapsed_days);
        assert_eq!(summary.phase, projection.scenario.phase);
        assert_eq!(
            summary.average_food_satisfaction_basis_points,
            projection.scenario.average_food_satisfaction_basis_points
        );
        assert_eq!(summary.active_crises, projection.scenario.active_crises);
        assert_eq!(summary.dynasty_name, projection.player.name);
        assert_eq!(summary.dynasty_treasury, projection.player.treasury);
        assert_eq!(summary.businesses, projection.player.businesses);
    }

    #[test]
    #[should_panic(expected = "state and registry scenarios must match before projection")]
    fn summary_rejects_registry_mismatch() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        state.scenario_key = "another-scenario".to_owned();

        let _ = build_state_summary(registry, &state);
    }

    #[test]
    fn summary_counts_defaulted_debt_as_outstanding_but_excludes_repaid_debt() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let creditor_dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != state.player_dynasty_id)
            .expect("campaign must contain a creditor dynasty");
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::PublicDebtAuthorization,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                value: 10_000,
                active: false,
            },
        );
        let debt_id = state.next_ids.civic_debt();
        state.civic_debts.insert(
            debt_id,
            CivicDebt {
                id: debt_id,
                creditor_dynasty_id,
                authorizing_law_id: law_id,
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                principal: Money::from_copper(10_000),
                balance: Money::from_copper(10_000),
                weekly_payment: Money::from_copper(100),
                interest_basis_points: 600,
                issued_day: state.clock.day(),
                next_due_day: state.clock.day().saturating_add(7),
                missed_payments: 3,
                status: CivicDebtStatus::Defaulted,
            },
        );

        let summary = build_state_summary(registry, &state);
        assert_eq!(summary.outstanding_civic_debts, 1);
        assert_eq!(summary.civic_debt_balance, Money::from_copper(10_000));

        let debt = state
            .civic_debts
            .get_mut(&debt_id)
            .expect("civic debt must exist");
        debt.balance = Money::ZERO;
        debt.missed_payments = 0;
        debt.status = CivicDebtStatus::Repaid;

        let summary = build_state_summary(registry, &state);
        assert_eq!(summary.outstanding_civic_debts, 0);
        assert_eq!(summary.civic_debt_balance, Money::ZERO);
    }

    #[test]
    fn includes_each_primary_record_exactly_once() {
        let registry = rivergate_registry_for_test();
        let state = make_test_campaign();
        let projection = build_campaign_projection(registry, &state);

        assert_registry_views(registry, &projection);
        assert_runtime_views(&state, &projection);
        assert_filtered_views(&state, &projection);
    }

    #[test]
    fn exposes_private_costs_and_administrative_burden_of_officeholding() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_id = state.player_dynasty_id;
        let holder_id = state
            .dynasties
            .get(&player_id)
            .expect("player dynasty must exist")
            .head_id();
        for institution in state.institutions.values_mut() {
            institution.office_holder_id = None;
        }
        let council_id = registry
            .get_institution_id("city_council")
            .expect("registry must define the city council");
        state
            .institutions
            .get_mut(&council_id)
            .expect("city council must exist")
            .office_holder_id = Some(holder_id);
        let player = state
            .dynasties
            .get_mut(&player_id)
            .expect("player dynasty must exist");
        player.resources.civic_contributions = Money::from_copper(3_600);
        player.resources.unmet_office_duties = 2;

        let projection = build_campaign_projection(registry, &state);

        assert_eq!(
            projection.player.civic_contributions,
            Money::from_copper(3_600)
        );
        assert_eq!(projection.player.unmet_office_duties, 2);
        assert!(projection.player.office_administrative_load > 0);
        assert_eq!(
            projection.player.effective_administrative_load,
            projection
                .player
                .administrative_load
                .saturating_add(projection.player.office_administrative_load)
        );
    }

    #[test]
    fn exposes_acquisition_terms_for_troubled_nonplayer_businesses() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .find(|business| business.owner_dynasty_id() != state.player_dynasty_id)
            .expect("campaign must contain a nonplayer business")
            .id();
        let business = state
            .businesses
            .get_mut(business_id)
            .expect("selected business must exist");
        business.operations.status = crate::core::BusinessStatus::Distressed;
        business.finance.cash = Money::ZERO;

        let projection = build_campaign_projection(registry, &state);
        let projected = projection
            .businesses
            .iter()
            .find(|business| business.id == business_id)
            .expect("troubled business must be projected");

        let acquisition = projected
            .acquisition
            .expect("troubled nonplayer business must expose acquisition terms");
        assert!(acquisition.purchase_price > Money::ZERO);
        assert!(acquisition.minimum_recapitalization > Money::ZERO);
    }

    #[test]
    fn exposes_business_policy_values_used_by_player_commands() {
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
            business.policy.target_input_days = 7;
            business.policy.target_output_days = 5;
            business.policy.minimum_cash_reserve = Money::from_copper(1_234);
            business.policy.maintenance_basis_points = 2_345;
            business.policy.quality_target_basis_points = 8_765;
        }

        let projection = build_campaign_projection(registry, &state);
        let business = projection
            .businesses
            .iter()
            .find(|business| business.id == business_id)
            .expect("player business must be projected");

        assert_eq!(business.target_input_days, 7);
        assert_eq!(business.target_output_days, 5);
        assert_eq!(business.minimum_cash_reserve, Money::from_copper(1_234));
        assert_eq!(business.maintenance_basis_points, 2_345);
        assert_eq!(business.quality_target_basis_points, 8_765);
    }

    #[test]
    fn exposes_attributed_contract_breaches() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let contract_id = *state
            .contracts
            .keys()
            .next()
            .expect("campaign must contain a contract");
        let seller_business_id = state
            .contracts
            .get(&contract_id)
            .expect("contract must exist")
            .seller_business_id;
        let seller_dynasty_id = state
            .businesses
            .get(seller_business_id)
            .expect("seller business must exist")
            .owner_dynasty_id();
        let seller_name = state
            .dynasties
            .get(&seller_dynasty_id)
            .expect("seller dynasty must exist")
            .name()
            .to_owned();
        let contract = state
            .contracts
            .get_mut(&contract_id)
            .expect("contract must exist");
        contract.status = ContractStatus::Breached;
        contract.breaching_dynasty_id = Some(seller_dynasty_id);

        let projection = build_campaign_projection(registry, &state);
        let contract = projection
            .contracts
            .iter()
            .find(|contract| contract.id == contract_id)
            .expect("contract must be projected");

        assert_eq!(contract.breaching_dynasty_id, Some(seller_dynasty_id));
        assert_eq!(
            contract.breaching_dynasty.as_deref(),
            Some(seller_name.as_str())
        );
    }

    #[test]
    fn exposes_dynasties_that_earned_contract_delivery_credit() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let contract_id = *state
            .contracts
            .keys()
            .next()
            .expect("campaign must contain a contract");
        let dynasty_id = state.player_dynasty_id;
        let dynasty_name = state
            .dynasties
            .get(&dynasty_id)
            .expect("player dynasty must exist")
            .name()
            .to_owned();
        let contract = state
            .contracts
            .get_mut(&contract_id)
            .expect("contract must exist");
        contract.fulfilled_deliveries = 6;
        contract
            .fulfilled_deliveries_by_dynasty
            .insert(dynasty_id, 6);

        let projection = build_campaign_projection(registry, &state);
        let contract = projection
            .contracts
            .iter()
            .find(|contract| contract.id == contract_id)
            .expect("contract must be projected");

        assert_eq!(
            contract.delivery_credits,
            vec![ContractDeliveryCreditProjection {
                dynasty_id,
                dynasty: dynasty_name,
                deliveries: 6,
            }]
        );
    }
}

mod html {
    use super::*;

    #[test]
    fn embeds_projection_data() {
        let registry = rivergate_registry_for_test();
        let state = make_test_campaign();
        let player_name = state
            .get_dynasty(state.player_dynasty_id())
            .expect("player dynasty must exist")
            .name();

        let html = render_campaign_html(registry, &state).expect("dashboard must render");

        assert!(
            html.contains("<!doctype html>"),
            "dashboard must be standalone HTML"
        );
        assert!(
            html.contains("campaign-data"),
            "dashboard must embed its complete projection payload"
        );
        assert!(
            html.contains(registry.scenario().name()),
            "dashboard must identify the current scenario"
        );
        assert!(
            html.contains("Dynasty relationships"),
            "dashboard must expose the social network that influences institutional outcomes"
        );
        assert!(
            html.contains("<caption class=\"sr-only\">Business operations</caption>")
                && html.contains("<th scope=\"col\">Business</th>")
                && html.contains(
                    "<caption class=\"sr-only\">Supply contract obligations and performance</caption>",
                ),
            "dashboard tables must expose captions and column-header semantics"
        );
        assert!(
            html.contains(player_name),
            "dashboard must identify the player dynasty"
        );
    }

    #[test]
    fn renders_human_readable_market_causes_and_empty_states() {
        let rows = render_market_rows(&[MarketProjection {
            good: "Bread".to_owned(),
            price: Money::from_copper(25),
            previous_price: Money::from_copper(24),
            stock: Quantity::from_milliunits(1_000),
            target_stock: Quantity::from_milliunits(2_000),
            demand_today: Quantity::from_milliunits(3_000),
            supply_today: Quantity::from_milliunits(1_000),
            causes: vec![
                MarketCause::StockBelowTarget,
                MarketCause::DemandExceededSupply,
            ],
        }]);

        assert!(rows.contains("Stock below target, Demand exceeded supply"));
        assert!(!rows.contains("StockBelowTarget"));
        assert_eq!(
            render_notifications(&[]),
            "<article><p>No recent notices.</p></article>"
        );
        assert!(render_business_rows(&[]).contains("No businesses are operating"));
        assert!(render_contract_rows(&[]).contains("No supply contracts are recorded"));
    }

    #[test]
    fn renders_contract_obligations_and_attributed_performance() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let player_dynasty_id = state.player_dynasty_id;
        let player_name = state
            .dynasties
            .get(&player_dynasty_id)
            .expect("player dynasty must exist")
            .name()
            .to_owned();
        let contract = state
            .contracts
            .values_mut()
            .next()
            .expect("campaign must contain a contract");
        contract.fulfilled_deliveries = 3;
        contract
            .fulfilled_deliveries_by_dynasty
            .insert(player_dynasty_id, 3);
        let expected_terms = format!(
            "through day {} · penalty {}",
            contract.end_day, contract.penalty
        );
        let projection = build_campaign_projection(registry, &state);

        let rows = render_contract_rows(&projection.contracts);

        assert!(rows.contains("3 fulfilled"));
        assert!(rows.contains(&format!("{}: 3", escape_html(&player_name))));
        assert!(rows.contains(&expected_terms));
        assert!(rows.contains("Active"));
        assert!(!rows.contains("ContractStatus"));
    }

    #[test]
    fn escapes_user_authored_names_in_markup_and_json() {
        let registry = rivergate_registry_for_test();
        let state = make_test_campaign_with(NewGameConfig {
            dynasty_name: "</script><script>alert('dynasty')</script>".to_owned(),
            founder_name: "Founder & Steward".to_owned(),
            ..NewGameConfig::default()
        });

        let html = render_campaign_html(registry, &state).expect("dashboard must render");

        assert!(
            !html.contains("</script><script>alert('dynasty')</script>"),
            "user-authored names must not create executable markup"
        );
        assert!(
            html.contains("House &lt;/script&gt;&lt;script&gt;alert"),
            "visible user-authored text must be HTML escaped"
        );
        assert!(
            html.contains("\\u003c/script\\u003e\\u003cscript\\u003e"),
            "embedded JSON must escape script-delimiter characters"
        );
    }
}
