//! Shared deterministic fixtures and state-difference diagnostics for unit tests.
//!
//! Purpose: provide one `rivergate_registry_for_test` / `make_test_campaign` per suite so
//! every behavioral test starts from the same deterministic baseline without rebuilding.
//! Owns: `rivergate_registry_for_test`, `make_test_campaign*`, `assert_state_unchanged` /
//! `assert_state_eq` diagnostics, and JSON fixture helpers.
//! Reads: `Registry`, `AppState` via lib entry points.
//! Mutates: nothing persistent (returns owned clones).
//! Does not own: domain rules or persistence IO.
//! Canonical operations: `make_test_campaign`, `rivergate_registry_for_test` fixtures.
//! Relevant invariants: fixtures are deterministic clones; no hidden mutation.
//! Focused tests: as consumers (`*_tests.rs`) — this is test infrastructure.

use crate::core::{AppState, NewGameConfig};
use crate::money::{Money, Quantity};
use crate::registry::{Registry, build_rivergate_registry};
use crate::systems::{
    CommandError, CommandOutcome, PlayerCommand, apply_player_command, build_new_game,
    validate_invariants,
};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use tempfile::TempDir;

static RIVERGATE_REGISTRY: OnceLock<Registry> = OnceLock::new();
static DEFAULT_CAMPAIGN: OnceLock<AppState> = OnceLock::new();

pub(crate) fn rivergate_registry_for_test() -> &'static Registry {
    RIVERGATE_REGISTRY.get_or_init(build_rivergate_registry)
}

pub(crate) fn make_test_campaign() -> AppState {
    DEFAULT_CAMPAIGN
        .get_or_init(|| make_test_campaign_with(NewGameConfig::default()))
        .clone()
}

pub(crate) fn make_test_campaign_with(config: NewGameConfig) -> AppState {
    build_new_game(rivergate_registry_for_test(), config).expect("test campaign fixture must build")
}

pub(crate) fn write_test_json_fixture(file_name: &str, value: &Value) -> (TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("temporary test directory must be created");
    let path = directory.path().join(file_name);
    let bytes = serde_json::to_vec_pretty(value).expect("JSON test fixture must serialize");
    fs::write(&path, bytes).expect("JSON test fixture must be written");
    (directory, path)
}

#[track_caller]
pub(crate) fn assert_state_eq(expected: &AppState, actual: &AppState, context: &str) {
    if expected == actual {
        return;
    }
    let expected = serde_json::to_value(expected).expect("expected state must serialize");
    let actual = serde_json::to_value(actual).expect("actual state must serialize");
    let (path, expected, actual) = first_json_difference(&expected, &actual, "$")
        .expect("unequal states must have a JSON difference");
    panic!("{context}; first state difference at {path}: expected {expected}, actual {actual}");
}

#[track_caller]
pub(crate) fn assert_state_unchanged(before: &AppState, after: &AppState, context: &str) {
    assert_state_eq(before, after, context);
}

#[track_caller]
pub(crate) fn assert_set_eq<T>(expected: &BTreeSet<T>, actual: &BTreeSet<T>, context: &str)
where
    T: Debug + Ord,
{
    let missing: Vec<_> = expected.difference(actual).collect();
    let unexpected: Vec<_> = actual.difference(expected).collect();
    if missing.is_empty() && unexpected.is_empty() {
        return;
    }
    panic!("{context}; missing members: {missing:#?}; unexpected members: {unexpected:#?}");
}

#[track_caller]
pub(crate) fn assert_money_eq(actual: Money, expected: Money, context: &str) {
    if actual == expected {
        return;
    }
    panic!(
        "{context}; money mismatch: expected {} ({} copper), got {} ({} copper)",
        expected,
        expected.copper(),
        actual,
        actual.copper(),
    );
}

#[track_caller]
pub(crate) fn assert_quantity_eq(actual: Quantity, expected: Quantity, context: &str) {
    if actual == expected {
        return;
    }
    panic!(
        "{context}; quantity mismatch: expected {} ({} milliunits), got {} ({} milliunits)",
        expected,
        expected.milliunits(),
        actual,
        actual.milliunits(),
    );
}

#[track_caller]
pub(crate) fn assert_in_range<T>(actual: T, low: T, high: T, context: &str)
where
    T: Debug + PartialOrd + Copy,
{
    if actual >= low && actual <= high {
        return;
    }
    panic!("{context}; value {actual:?} not in range [{low:?}, {high:?}]");
}

#[track_caller]
pub(crate) fn assert_money_in_range(actual: Money, low: Money, high: Money, context: &str) {
    if actual.copper() >= low.copper() && actual.copper() <= high.copper() {
        return;
    }
    panic!("{context}; money {actual} not in range [{low}, {high}]");
}

#[track_caller]
pub(crate) fn assert_command_rejected_with(
    registry: &Registry,
    state: &mut AppState,
    command: PlayerCommand,
    expected_error: &CommandError,
    context: &str,
) {
    let before = state.clone();
    let result = apply_player_command(registry, state, command);
    match result {
        Ok(outcome) => {
            panic!(
                "{context}; expected command to be rejected with error {expected_error:?}, but succeeded with {outcome:?}"
            );
        }
        Err(err) => {
            assert_eq!(
                &err, expected_error,
                "{context}; command rejected with wrong error"
            );
        }
    }
    assert_state_unchanged(
        &before,
        state,
        &format!("{context} (rejected mutation must leave campaign state unchanged)"),
    );
}

#[track_caller]
pub(crate) fn assert_command_success(
    registry: &Registry,
    state: &mut AppState,
    command: PlayerCommand,
    context: &str,
) -> CommandOutcome {
    match apply_player_command(registry, state, command) {
        Ok(outcome) => {
            validate_invariants(registry, state);
            outcome
        }
        Err(err) => {
            panic!("{context}; expected command to succeed, but failed with error: {err:?}");
        }
    }
}

fn display_json(value: &Value) -> String {
    const MAX_CHARACTERS: usize = 240;
    let rendered = value.to_string();
    let mut characters = rendered.chars();
    let prefix: String = characters.by_ref().take(MAX_CHARACTERS).collect();
    if characters.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn first_json_difference(
    expected: &Value,
    actual: &Value,
    path: &str,
) -> Option<(String, String, String)> {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            let keys: BTreeSet<_> = expected.keys().chain(actual.keys()).collect();
            for key in keys {
                let next_path = format!("{path}.{key}");
                match (expected.get(key), actual.get(key)) {
                    (Some(expected), Some(actual)) => {
                        if let Some(difference) =
                            first_json_difference(expected, actual, &next_path)
                        {
                            return Some(difference);
                        }
                    }
                    (Some(expected), None) => {
                        return Some((next_path, display_json(expected), "<missing>".to_owned()));
                    }
                    (None, Some(actual)) => {
                        return Some((next_path, "<missing>".to_owned(), display_json(actual)));
                    }
                    (None, None) => unreachable!("key must exist in at least one object"),
                }
            }
            None
        }
        (Value::Array(expected), Value::Array(actual)) => {
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                let next_path = format!("{path}[{index}]");
                if let Some(difference) = first_json_difference(expected, actual, &next_path) {
                    return Some(difference);
                }
            }
            (expected.len() != actual.len()).then(|| {
                (
                    format!("{path}.length"),
                    expected.len().to_string(),
                    actual.len().to_string(),
                )
            })
        }
        _ if expected == actual => None,
        _ => Some((
            path.to_owned(),
            display_json(expected),
            display_json(actual),
        )),
    }
}

mod tests {
    use super::*;

    #[test]
    fn default_campaign_fixture_returns_isolated_clones() {
        let mut first = make_test_campaign();
        let second = make_test_campaign();
        first
            .dynasties
            .get_mut(&first.player_dynasty_id)
            .expect("player dynasty must exist")
            .resources
            .treasury = crate::money::Money::ZERO;

        assert_ne!(
            first, second,
            "mutating one fixture must not affect another"
        );
        assert_state_eq(
            &second,
            &make_test_campaign(),
            "cached fixture clones must remain deterministic and isolated",
        );
    }

    #[test]
    fn state_diff_reports_the_first_nested_value() {
        let expected = serde_json::json!({"businesses": [{"cash": 100}, {"cash": 200}]});
        let actual = serde_json::json!({"businesses": [{"cash": 100}, {"cash": 175}]});

        assert_eq!(
            first_json_difference(&expected, &actual, "$"),
            Some((
                "$.businesses[1].cash".to_owned(),
                "200".to_owned(),
                "175".to_owned(),
            ))
        );
    }

    #[test]
    fn state_diff_truncates_large_values() {
        let expected = Value::String("x".repeat(300));
        let actual = Value::String("y".repeat(300));

        let (_, expected, actual) = first_json_difference(&expected, &actual, "$")
            .expect("different values must produce a difference");

        assert!(expected.ends_with('…'));
        assert!(actual.ends_with('…'));
        assert!(expected.chars().count() <= 241);
        assert!(actual.chars().count() <= 241);
    }

    #[test]
    fn assert_money_and_quantity_helpers_match_exact_values() {
        assert_money_eq(
            Money::from_copper(1250),
            Money::from_copper(1250),
            "identical money must pass",
        );
        assert_quantity_eq(
            Quantity::from_milliunits(500),
            Quantity::from_milliunits(500),
            "identical quantity must pass",
        );
    }

    #[test]
    fn range_helpers_accept_bounds_and_reject_outliers() {
        assert_in_range(5, 1, 10, "in-range integer must pass");
        assert_money_in_range(
            Money::from_copper(500),
            Money::from_copper(100),
            Money::from_copper(1000),
            "in-range money must pass",
        );
    }

    #[test]
    fn assert_command_helpers_validate_rejection_and_success() {
        let registry = rivergate_registry_for_test();
        let mut state = make_test_campaign();

        let outcome = assert_command_success(
            registry,
            &mut state,
            PlayerCommand::SetHouseGovernance {
                governance: crate::core::HouseGovernance::FamilyPartnership,
            },
            "setting house governance must succeed",
        );
        assert!(
            outcome.summary.contains("FamilyPartnership"),
            "feedback summary must reflect governance update"
        );

        // Immediate duplicate change must be rejected and preserve state
        assert_command_rejected_with(
            registry,
            &mut state,
            PlayerCommand::SetHouseGovernance {
                governance: crate::core::HouseGovernance::FamilyPartnership,
            },
            &CommandError::UnchangedHouseGovernance {
                governance: crate::core::HouseGovernance::FamilyPartnership,
            },
            "duplicate governance update must be rejected",
        );
    }
}
