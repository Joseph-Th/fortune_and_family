//! Determinism and long-running invariant tests for application state.

use super::*;
use crate::systems::{advance_days, validate_invariants};
use crate::test_support::{
    assert_state_eq, make_test_campaign, make_test_campaign_with, rivergate_registry_for_test,
};

mod aggregates {
    use super::*;

    #[test]
    fn food_satisfaction_is_weighted_by_household_population() {
        let mut state = make_test_campaign();
        let household_ids = state
            .households
            .iter()
            .take(2)
            .map(Household::id)
            .collect::<Vec<_>>();
        assert_eq!(
            household_ids.len(),
            2,
            "campaign must contain two households"
        );
        {
            let first = state
                .households
                .get_mut(household_ids[0])
                .expect("first household must exist");
            first.members = 1;
            first.food_satisfaction_basis_points = 0;
        }
        {
            let second = state
                .households
                .get_mut(household_ids[1])
                .expect("second household must exist");
            second.members = 9;
            second.food_satisfaction_basis_points = 10_000;
        }

        let satisfaction = population_weighted_food_satisfaction_basis_points([
            state
                .households
                .get(household_ids[0])
                .expect("first household must exist"),
            state
                .households
                .get(household_ids[1])
                .expect("second household must exist"),
        ]);

        assert_eq!(satisfaction, Some(9_000));
    }
}

mod determinism {
    use super::*;

    #[test]
    fn same_seed_and_inputs_produce_identical_state() {
        let registry = rivergate_registry_for_test();
        let config = NewGameConfig {
            seed: 9_814,
            dynasty_name: "Aster".to_owned(),
            founder_name: "Mira Aster".to_owned(),
            background: StartingBackground::ClothTrader,
        };
        let mut first = make_test_campaign_with(config.clone());
        let mut second = make_test_campaign_with(config);

        advance_days(registry, &mut first, 360).expect("first simulation must advance");
        advance_days(registry, &mut second, 360).expect("second simulation must advance");

        assert_state_eq(
            &first,
            &second,
            "identical seeds and inputs must remain deterministic across an annual boundary",
        );
    }
}

mod soak {
    use super::*;

    #[test]
    #[ignore = "long-running soak; run `bash scripts/test.sh soak`"]
    fn core_preserves_invariants() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign_with(NewGameConfig {
            seed: 77,
            dynasty_name: "Corren".to_owned(),
            founder_name: "Lysa Corren".to_owned(),
            background: StartingBackground::Blacksmith,
        });
        let initial_businesses = state.businesses().iter().count();

        advance_days(registry, &mut state, 3_000).expect("soak simulation must advance");
        validate_invariants(registry, &state);

        assert_eq!(state.clock().day(), 3_000, "all requested days must run");
        assert!(
            !state.chronicle().is_empty(),
            "a multi-year campaign must produce chronicle history"
        );
        assert!(
            state.businesses().iter().count() >= initial_businesses,
            "the soak must preserve the authored business population"
        );
    }

    #[test]
    #[ignore = "long-running soak; run `bash scripts/test.sh soak`"]
    fn strategic_preserves_two_generations() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        let initial_generation = state
            .dynasties
            .values()
            .map(|dynasty| dynasty.runtime.generation)
            .max()
            .expect("campaign must contain dynasties");

        advance_days(registry, &mut state, 7_200).expect("strategic simulation must advance");
        validate_invariants(registry, &state);

        let final_generation = state
            .dynasties
            .values()
            .map(|dynasty| dynasty.runtime.generation)
            .max()
            .expect("campaign must contain dynasties");
        assert_eq!(state.clock().day(), 7_200, "all requested days must run");
        assert!(
            final_generation > initial_generation,
            "the long soak must exercise at least one succession"
        );
        assert!(
            !state.information_reports.is_empty(),
            "long campaigns must retain current strategic reporting"
        );
        assert!(
            !state.ai_objectives.is_empty(),
            "AI dynasties must retain actionable objectives"
        );
    }
}
