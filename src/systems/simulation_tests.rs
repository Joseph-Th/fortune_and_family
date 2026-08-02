use super::*;
use crate::core::{EnactedLaw, LawKind};
use crate::test_support::{
    assert_state_unchanged, make_test_campaign, rivergate_registry_for_test,
};

mod preflight {
    use super::*;

    #[test]
    fn missing_market_quote_fails_before_day_mutation() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let missing_good_id = registry.goods()[0].id();
        state.market.quotes.remove(&missing_good_id);
        let before = state.clone();

        let result = advance_days(registry, &mut state, 1);

        assert_eq!(
            result,
            Err(SimulationError::MarketQuoteMissing {
                good_id: missing_good_id,
            })
        );
        assert_state_unchanged(
            &before,
            &state,
            "preflight failure must leave the entire campaign unchanged",
        );
    }
}

mod labor {
    use super::*;

    #[test]
    fn disputed_employment_prevents_production() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let initial_plan = decide_production(registry, &state);
        let business_id = initial_plan
            .lines
            .first()
            .expect("bootstrap must include a business able to produce")
            .business_id;
        for agreement in state
            .employment
            .values_mut()
            .filter(|agreement| agreement.business_id == business_id)
        {
            agreement.status = EmploymentStatus::Disputed;
        }

        let plan = decide_production(registry, &state);

        assert!(
            plan.lines
                .iter()
                .all(|line| line.business_id != business_id),
            "a business without active workers must not produce"
        );
    }
}

mod laws {
    use super::*;

    #[test]
    fn bread_price_ceiling_is_final_price_constraint() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let law_id = state.next_ids.law();
        state.laws.insert(
            law_id,
            EnactedLaw {
                id: law_id,
                kind: LawKind::BreadPriceCeiling,
                enacted_day: state.clock.day(),
                sponsor_dynasty_id: Some(state.player_dynasty_id),
                value: 1,
                active: true,
            },
        );

        advance_days(registry, &mut state, 1).expect("simulation must advance");

        let bread_id = registry
            .get_good_id("bread")
            .expect("registry must define bread");
        assert_eq!(
            state
                .market
                .get_quote(bread_id)
                .expect("bread quote must exist")
                .price(),
            Money::from_copper(1),
            "the statutory ceiling must be the final daily price constraint"
        );
    }
}
