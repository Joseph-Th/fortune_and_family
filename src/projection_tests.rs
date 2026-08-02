//! Projection completeness and HTML escaping tests for presentation adapters.

use super::*;
use crate::core::NewGameConfig;
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
            projection.information.len(),
            state
                .information_reports
                .values()
                .filter(|report| report.owner_dynasty_id == state.player_dynasty_id)
                .count(),
            "information projection must include only player-owned reports"
        );
        assert_eq!(
            projection.notifications.len(),
            state.outbox.len().min(50),
            "notification projection must honor its 50-message cap"
        );
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
            html.contains(player_name),
            "dashboard must identify the player dynasty"
        );
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
