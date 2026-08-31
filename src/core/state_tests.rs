//! Determinism and long-running invariant tests for application state.
//!
//! Purpose: prove ID allocation exhaustion/recovery, `HistoryLog`
//! copy-on-write + checksum memo, clock bounds, `NextIds` monotonicity,
//! and persistence-invariant parity via deterministic long horizons.
//! Owns: `state_tests` suite behind `src/core/state.rs`.
//! Reads: `AppState`, `CharacterStore`/`BusinessStore` via fixtures.
//! Mutates: local clones only.
//! Focused lane: `bash scripts/test.sh soak` (release) and `fast state`.

use super::*;
use crate::money::Money;
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
    use crate::ids::IdentifierAllocationError;
    use crate::systems::{SimulationError, StrategicError, buy_unowned_property};

    #[test]
    fn allocator_reports_exhaustion_without_corrupting_the_terminal_counter() {
        let mut next_ids = NextIds::new();
        next_ids.business = u32::MAX - 2;

        let final_usable_id = next_ids
            .try_business()
            .expect("penultimate counter must still allocate");

        assert_eq!(final_usable_id.value(), u32::MAX - 2);
        assert_eq!(next_ids.business, u32::MAX - 1);

        let before = next_ids.clone();
        let result = next_ids.try_business();

        assert_eq!(
            result,
            Err(IdentifierAllocationError::Business),
            "allocation must report exhaustion before advancing into the invalid sentinel"
        );
        assert_eq!(
            next_ids, before,
            "allocation exhaustion must not advance the counter into an invalid save state"
        );
    }

    #[test]
    fn public_transaction_rolls_back_when_feedback_identifier_space_is_exhausted() {
        let mut state = make_test_campaign();
        let player_dynasty_id = state.player_dynasty_id;
        let (property_id, price) = state
            .properties
            .values()
            .find(|property| property.owner_dynasty_id.is_none())
            .map(|property| (property.id, property.value))
            .expect("campaign must contain an unowned property");
        state
            .dynasties
            .get_mut(&player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = price;
        state.next_ids.outbox = u32::MAX - 1;
        let before = state.clone();

        let result = buy_unowned_property(&mut state, player_dynasty_id, property_id);

        assert_eq!(
            result,
            Err(StrategicError::IdentifierAllocation(
                IdentifierAllocationError::OutboxMessage
            ))
        );
        assert_state_eq(
            &before,
            &state,
            "feedback identifier exhaustion must roll back the entire property purchase",
        );
    }

    #[test]
    fn simulation_rolls_back_when_chronicle_identifier_space_is_exhausted() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();
        state.next_ids.chronicle = u32::MAX - 1;
        // Force a chronicle allocation on the first simulated day (business
        // distress entry) so the typed identifier error surfaces from the
        // daily pipeline before invariant validation examines the exhausted
        // allocator directly.
        let distressed_business = state
            .businesses
            .iter()
            .next()
            .expect("campaign must contain a business")
            .id();
        {
            let business = state
                .businesses
                .get_mut(distressed_business)
                .expect("business must exist");
            business.finance.cash = Money::ZERO;
            business.inventory.clear();
        }
        let before = state.clone();

        let result = advance_days(registry, &mut state, 360);

        assert_eq!(
            result,
            Err(SimulationError::IdentifierAllocation(
                IdentifierAllocationError::ChronicleEntry
            ))
        );
        assert_state_eq(
            &before,
            &state,
            "identifier exhaustion during simulation must discard the candidate state",
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

mod history_log {
    use super::*;
    use crate::core::checksum::ChecksumFolder;
    use crate::core::history::{HISTORY_CHECKSUM_UNSYNCED, HISTORY_TAIL_FOLD_THRESHOLD};
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    fn log_with(values: &[u32]) -> HistoryLog<u32> {
        let mut log = HistoryLog::new();
        for value in values {
            log.push(*value);
        }
        log
    }

    #[test]
    fn iteration_order_survives_the_tail_fold() {
        // Push well past the fold threshold so entries live in both the
        // folded bulk and the tail, then verify one ordered sequence.
        let count = u32::try_from(HISTORY_TAIL_FOLD_THRESHOLD * 3 + 17)
            .expect("fold threshold arithmetic must fit u32");
        let mut log = HistoryLog::new();
        for value in 0..u64::from(count) {
            log.push(value);
        }

        assert_eq!(log.len() as u64, u64::from(count));
        let seen: Vec<u64> = log.iter().copied().collect();
        let expected: Vec<u64> = (0..u64::from(count)).collect();
        assert_eq!(seen, expected, "iteration must follow insertion order");
        assert_eq!(
            log.last(),
            Some(&(u64::from(count) - 1)),
            "last must observe the most recent append"
        );

        // Reverse iteration must mirror it exactly.
        let reversed: Vec<u64> = log.iter().rev().copied().collect();
        let mut expected_reversed = expected;
        expected_reversed.reverse();
        assert_eq!(reversed, expected_reversed);
    }

    #[test]
    fn clones_append_independently_of_the_shared_bulk() {
        let original = log_with(&[1, 2, 3]);
        let mut branch = original.clone();

        branch.push(4);
        assert_eq!(original.len(), 3, "clone appends must not leak back");
        assert_eq!(branch.last(), Some(&4));
        assert_eq!(
            original.iter().copied().collect::<Vec<u32>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn partition_point_spans_bulk_and_tail() {
        let count = u32::try_from(HISTORY_TAIL_FOLD_THRESHOLD + 32)
            .expect("fold threshold arithmetic must fit u32");
        let mut log = HistoryLog::new();
        for value in 0..count {
            log.push(value);
        }

        assert_eq!(log.partition_point(|value| *value < 10), 10);
        assert_eq!(
            log.partition_point(|value| *value < count),
            count as usize,
            "a predicate every entry satisfies must land past the tail"
        );
        assert_eq!(
            log.partition_point(|_| false),
            0,
            "a predicate no entry satisfies must land at the front"
        );
    }

    #[test]
    fn retain_preserves_order_across_the_fold_boundary() {
        let count = u32::try_from(HISTORY_TAIL_FOLD_THRESHOLD + 32)
            .expect("fold threshold arithmetic must fit u32");
        let mut log = HistoryLog::new();
        for value in 0..count {
            log.push(value);
        }

        log.retain(|value| value % 2 == 0);
        let kept: Vec<u32> = log.iter().copied().collect();
        let expected: Vec<u32> = (0..count).filter(|value| value % 2 == 0).collect();
        assert_eq!(kept, expected, "retain must keep relative order");
    }

    #[test]
    fn iter_mut_reaches_every_entry_after_a_fold() {
        let count = u64::from(
            u32::try_from(HISTORY_TAIL_FOLD_THRESHOLD + 8)
                .expect("fold threshold arithmetic must fit u32"),
        );
        let mut log = HistoryLog::new();
        for value in 0..count {
            log.push(value);
        }

        for entry in &mut log {
            *entry += 1;
        }
        let seen: Vec<u64> = log.iter().copied().collect();
        let expected: Vec<u64> = (1..=count).collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn serialization_matches_the_plain_sequence_shape() {
        let log = log_with(&[7, 8, 9]);
        let serialized = serde_json::to_string(&log).expect("history must serialize");
        assert_eq!(serialized, "[7,8,9]", "the save shape stays a plain array");

        let round_tripped: HistoryLog<u32> =
            serde_json::from_str(&serialized).expect("history must deserialize");
        assert_eq!(round_tripped, log);

        // A freshly deserialized log keeps accepting appends in order.
        let mut reopened = round_tripped;
        reopened.push(10);
        assert_eq!(
            reopened.iter().copied().collect::<Vec<u32>>(),
            vec![7, 8, 9, 10]
        );
    }

    #[test]
    fn clear_releases_every_entry() {
        let mut log = log_with(&[1, 2, 3]);
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert_eq!(log.last(), None);

        log.push(5);
        assert_eq!(log.iter().copied().collect::<Vec<u32>>(), vec![5]);
    }

    #[test]
    fn skip_and_nth_position_in_order_across_the_fold_boundary() {
        // Push past the fold threshold so entries live in both segments;
        // `Skip` relies on `nth` positioning, which must stay exact.
        let count = u32::try_from(HISTORY_TAIL_FOLD_THRESHOLD + 40)
            .expect("fold threshold arithmetic must fit u32");
        let mut log = HistoryLog::new();
        for value in 0..count {
            log.push(value);
        }

        let skipped: Vec<u32> = log.iter().copied().skip(10).collect();
        let expected: Vec<u32> = (10..count).collect();
        assert_eq!(skipped, expected);

        let skipped_tail_only: Vec<u32> = log
            .iter()
            .copied()
            .skip(usize::try_from(count).expect("fits usize") - 3)
            .collect();
        assert_eq!(
            skipped_tail_only,
            vec![count - 3, count - 2, count - 1],
            "a skip landing inside the tail must not disturb order"
        );

        let reverse_skipped: Vec<u32> = log.iter().rev().copied().skip(5).collect();
        let mut expected_reverse: Vec<u32> = (0..count - 5).collect();
        expected_reverse.reverse();
        assert_eq!(reverse_skipped, expected_reverse);

        let mut iter = log.iter();
        assert_eq!(iter.nth(2), Some(&2));
        assert_eq!(iter.next(), Some(&3));
        // Backward positioning: next_back yields the newest entry, then
        // nth_back(1) skips `count - 2` and yields `count - 3`.
        let mut rev_iter = log.iter();
        assert_eq!(rev_iter.next_back(), Some(&(count - 1)));
        assert_eq!(rev_iter.nth_back(1), Some(&(count - 3)));
    }

    #[test]
    fn equality_compares_entries_not_sharing() {
        let shared_bulk = Arc::new(vec![1_u32, 2, 3]);
        let left = HistoryLog {
            base: Arc::clone(&shared_bulk),
            tail: Vec::new(),
            checksum_len: AtomicU64::new(HISTORY_CHECKSUM_UNSYNCED),
            checksum_state: AtomicU64::new(ChecksumFolder::new().raw()),
        };
        let right = HistoryLog {
            base: Arc::clone(&shared_bulk),
            tail: vec![],
            checksum_len: AtomicU64::new(HISTORY_CHECKSUM_UNSYNCED),
            checksum_state: AtomicU64::new(ChecksumFolder::new().raw()),
        };
        assert_eq!(left, right);

        let with_tail = HistoryLog {
            base: Arc::clone(&shared_bulk),
            tail: vec![4],
            checksum_len: AtomicU64::new(HISTORY_CHECKSUM_UNSYNCED),
            checksum_state: AtomicU64::new(ChecksumFolder::new().raw()),
        };
        let rebuilt = log_with(&[1, 2, 3, 4]);
        assert_eq!(
            with_tail, rebuilt,
            "identical sequences are equal regardless of internal split"
        );
    }
}

mod history_checksum {
    use super::*;

    fn log_with(entries: &[u32]) -> HistoryLog<u32> {
        let mut log = HistoryLog::new();
        for entry in entries {
            log.push(*entry);
        }
        log
    }

    #[test]
    fn checksum_is_stable_across_reads_and_clones() {
        let log = log_with(&[1, 2, 3]);
        let first = log.structural_checksum();
        assert_eq!(log.structural_checksum(), first);

        let clone = log.clone();
        assert_eq!(
            clone.structural_checksum(),
            first,
            "a cloned log shares the same entry stream and memo"
        );

        let rebuilt = log_with(&[1, 2, 3]);
        // The rebuild path starts from an unsynced memo (fresh pushes kept
        // this one synced), so force staleness to exercise the rebuild.
        let mut stale = log.clone();
        stale.retain(|entry| *entry % 2 == 0);
        let _ = stale.structural_checksum();
        assert_eq!(rebuilt.structural_checksum(), first);
    }

    #[test]
    fn appended_entries_change_the_checksum_incrementally() {
        let mut log = log_with(&[1, 2, 3]);
        let before = log.structural_checksum();

        log.push(4);
        let after_append = log.structural_checksum();
        assert_ne!(after_append, before);

        // A fresh log built with the same contents must agree exactly:
        // incremental extension and full rebuild are interchangeable.
        let rebuilt = log_with(&[1, 2, 3, 4]);
        assert_eq!(rebuilt.structural_checksum(), after_append);

        log.push(5);
        let extended = log.structural_checksum();
        assert_eq!(log_with(&[1, 2, 3, 4, 5]).structural_checksum(), extended);
    }

    #[test]
    fn mutated_entries_invalidate_the_memo_without_corrupting_it() {
        let mut log = log_with(&[10, 20, 30]);
        let baseline = log.structural_checksum();

        log.retain(|entry| *entry != 20);
        let after_retain = log.structural_checksum();
        assert_ne!(after_retain, baseline);

        // Appends after a non-append mutation must still produce a value
        // identical to a from-scratch build of the same sequence.
        log.push(40);
        let extended = log.structural_checksum();
        assert_eq!(log_with(&[10, 30, 40]).structural_checksum(), extended);
    }

    #[test]
    fn mutable_iteration_rebuilds_the_checksum_after_entry_edits() {
        let mut log = log_with(&[7, 8, 9]);
        let before = log.structural_checksum();

        for entry in &mut log {
            if *entry == 8 {
                *entry = 800;
            }
        }
        let after_edit = log.structural_checksum();
        assert_ne!(after_edit, before);
        assert_eq!(log_with(&[7, 800, 9]).structural_checksum(), after_edit);
    }

    #[test]
    fn cleared_logs_restart_from_the_empty_checksum() {
        let mut log = log_with(&[1, 2, 3]);
        let _ = log.structural_checksum();
        log.clear();
        let empty = HistoryLog::<u32>::new().structural_checksum();
        assert_eq!(log.structural_checksum(), empty);
    }
}
