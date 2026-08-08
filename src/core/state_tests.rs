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

mod id_allocation {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn allocator_stops_at_the_terminal_valid_counter_without_corrupting_it() {
        let mut next_ids = NextIds::new();
        next_ids.business = u32::MAX - 2;

        let final_usable_id = next_ids.business();

        assert_eq!(final_usable_id.value(), u32::MAX - 2);
        assert_eq!(next_ids.business, u32::MAX - 1);

        let before = next_ids.clone();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = next_ids.business();
        }));

        assert!(
            result.is_err(),
            "allocation must stop before advancing into the exhausted sentinel"
        );
        assert_eq!(
            next_ids, before,
            "allocation exhaustion must not advance the counter into an invalid save state"
        );
    }
}

mod synchronized_stores {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn business_ownership_transfer_updates_record_and_index_together() {
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let prior_owner_id = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .owner_dynasty_id();
        let new_owner_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != prior_owner_id)
            .expect("campaign must contain another dynasty");
        let new_manager_id = state
            .characters
            .ids_for_dynasty(new_owner_id)
            .into_iter()
            .flatten()
            .copied()
            .find(|character_id| {
                state
                    .characters
                    .get(*character_id)
                    .is_some_and(|character| {
                        character.status() == crate::core::CharacterStatus::Active
                    })
            })
            .expect("new owner must have an active manager");

        let returned_owner =
            state
                .businesses
                .transfer_ownership(business_id, new_owner_id, new_manager_id);

        assert_eq!(returned_owner, Some(prior_owner_id));
        let business = state
            .businesses
            .get(business_id)
            .expect("transferred business must exist");
        assert_eq!(business.owner_dynasty_id(), new_owner_id);
        assert_eq!(business.manager_id(), new_manager_id);
        assert!(
            state
                .businesses
                .ids_for_owner(prior_owner_id)
                .is_none_or(|businesses| !businesses.contains(&business_id))
        );
        assert!(
            state
                .businesses
                .ids_for_owner(new_owner_id)
                .is_some_and(|businesses| businesses.contains(&business_id))
        );
    }

    #[test]
    fn ownership_transfer_detects_a_stale_source_index_before_mutation() {
        let mut state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let prior_owner_id = state
            .businesses
            .get(business_id)
            .expect("business must exist")
            .owner_dynasty_id();
        let new_owner_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != prior_owner_id)
            .expect("campaign must contain another dynasty");
        let new_manager_id = state
            .characters
            .ids_for_dynasty(new_owner_id)
            .into_iter()
            .flatten()
            .next()
            .copied()
            .expect("new owner must have a character");
        state
            .businesses
            .by_owner
            .get_mut(&prior_owner_id)
            .expect("owner index must exist")
            .remove(&business_id);
        let before = state.businesses.clone();

        let result = catch_unwind(AssertUnwindSafe(|| {
            state
                .businesses
                .transfer_ownership(business_id, new_owner_id, new_manager_id);
        }));

        assert!(
            result.is_err(),
            "stale synchronized indexes must be detected"
        );
        assert_eq!(
            state.businesses, before,
            "failed ownership transfer validation must not mutate either store representation"
        );
    }

    #[test]
    fn store_insertions_preflight_derived_indexes_before_records() {
        let mut state = make_test_campaign();

        let mut character = state
            .characters
            .iter()
            .next()
            .expect("campaign must contain a character")
            .clone();
        let character_id = state.next_ids.character();
        character.identity.id = character_id;
        let stale_dynasty_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != character.dynasty_id())
            .expect("campaign must contain another dynasty");
        state
            .characters
            .by_dynasty
            .entry(stale_dynasty_id)
            .or_default()
            .insert(character_id);
        let characters_before = state.characters.clone();
        let character_result = catch_unwind(AssertUnwindSafe(|| {
            state.characters.insert(character);
        }));
        assert!(character_result.is_err());
        assert_eq!(state.characters, characters_before);

        let mut business = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .clone();
        let business_id = state.next_ids.business();
        business.identity.id = business_id;
        let stale_owner_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != business.owner_dynasty_id())
            .expect("campaign must contain another dynasty");
        state
            .businesses
            .by_owner
            .entry(stale_owner_id)
            .or_default()
            .insert(business_id);
        let businesses_before = state.businesses.clone();
        let business_result = catch_unwind(AssertUnwindSafe(|| {
            state.businesses.insert(business);
        }));
        assert!(business_result.is_err());
        assert_eq!(state.businesses, businesses_before);

        let mut household = state
            .households
            .iter()
            .next()
            .expect("campaign must contain a household")
            .clone();
        let household_id = state.next_ids.household();
        household.id = household_id;
        let stale_district_id = state
            .districts
            .keys()
            .copied()
            .find(|district_id| *district_id != household.district_id())
            .expect("campaign must contain another district");
        state
            .households
            .by_district
            .entry(stale_district_id)
            .or_default()
            .insert(household_id);
        let households_before = state.households.clone();
        let household_result = catch_unwind(AssertUnwindSafe(|| {
            state.households.insert(household);
        }));
        assert!(household_result.is_err());
        assert_eq!(state.households, households_before);
    }

    #[test]
    fn ownership_transfer_preflights_duplicate_owner_and_district_indexes() {
        let state = make_test_campaign();
        let business_id = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        let business = state
            .businesses
            .get(business_id)
            .expect("business must exist");
        let prior_owner_id = business.owner_dynasty_id();
        let district_id = business.district_id();
        let new_owner_id = state
            .dynasties
            .keys()
            .copied()
            .find(|dynasty_id| *dynasty_id != prior_owner_id)
            .expect("campaign must contain another dynasty");
        let new_manager_id = state
            .characters
            .ids_for_dynasty(new_owner_id)
            .into_iter()
            .flatten()
            .next()
            .copied()
            .expect("new owner must have a manager");
        let other_district_id = state
            .districts
            .keys()
            .copied()
            .find(|candidate| *candidate != district_id)
            .expect("campaign must contain another district");

        let mut duplicate_owner = state.clone();
        duplicate_owner
            .businesses
            .by_owner
            .entry(new_owner_id)
            .or_default()
            .insert(business_id);
        let owner_before = duplicate_owner.businesses.clone();
        let owner_result = catch_unwind(AssertUnwindSafe(|| {
            duplicate_owner.businesses.transfer_ownership(
                business_id,
                new_owner_id,
                new_manager_id,
            );
        }));
        assert!(owner_result.is_err());
        assert_eq!(duplicate_owner.businesses, owner_before);

        let mut duplicate_district = state;
        duplicate_district
            .businesses
            .by_district
            .entry(other_district_id)
            .or_default()
            .insert(business_id);
        let district_before = duplicate_district.businesses.clone();
        let district_result = catch_unwind(AssertUnwindSafe(|| {
            duplicate_district.businesses.transfer_ownership(
                business_id,
                new_owner_id,
                new_manager_id,
            );
        }));
        assert!(district_result.is_err());
        assert_eq!(duplicate_district.businesses, district_before);
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
