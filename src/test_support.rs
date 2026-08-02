use crate::core::{AppState, NewGameConfig};
use crate::registry::{Registry, build_rivergate_registry};
use crate::systems::build_new_game;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use tempfile::TempDir;

static RIVERGATE_REGISTRY: OnceLock<Registry> = OnceLock::new();

pub(crate) fn rivergate_registry_for_test() -> &'static Registry {
    RIVERGATE_REGISTRY.get_or_init(build_rivergate_registry)
}

pub(crate) fn make_test_campaign() -> AppState {
    make_test_campaign_with(NewGameConfig::default())
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
}
