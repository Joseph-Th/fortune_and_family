//! JSON persistence adapter with explicit schema migration and contextual errors.

use crate::core::{AppState, CURRENT_SCHEMA_VERSION};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("failed to create save directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize application state: {source}")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write save file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read save file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse save file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("save file {path} has no numeric schema_version")]
    MissingSchemaVersion { path: PathBuf },
    #[error("save schema version {found} is newer than supported version {supported}")]
    FutureSchema { found: u32, supported: u32 },
    #[error("save schema version {version} has no migration path")]
    UnsupportedSchema { version: u32 },
    #[error("schema migration from version {version} failed: {reason}")]
    Migration { version: u32, reason: String },
}

/// Serializes the complete application state to a JSON save file.
///
/// # Errors
///
/// Returns an error when the parent directory cannot be created, serialization fails, or the
/// destination cannot be written.
pub fn save_state(path: impl AsRef<Path>, state: &AppState) -> Result<(), PersistenceError> {
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| PersistenceError::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|source| PersistenceError::Serialize { source })?;
    fs::write(path, bytes).map_err(|source| PersistenceError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// Loads, migrates, and deserializes a JSON save file.
///
/// # Errors
///
/// Returns an error when the file cannot be read, parsed, migrated, or deserialized.
pub fn load_state(path: impl AsRef<Path>) -> Result<AppState, PersistenceError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| PersistenceError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|source| PersistenceError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    let migrated = migrate_to_current(value, path)?;
    let mut state: AppState =
        serde_json::from_value(migrated).map_err(|source| PersistenceError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    hydrate_strategic_state(&mut state);
    Ok(state)
}

fn hydrate_strategic_state(state: &mut AppState) {
    if !state.properties.is_empty() || state.scenario_key != "rivergate" {
        return;
    }
    let registry = crate::registry::build_rivergate_registry();
    crate::systems::initialize_strategic_state(&registry, state);
}

fn migrate_to_current(mut value: Value, path: &Path) -> Result<Value, PersistenceError> {
    let raw_version = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| PersistenceError::MissingSchemaVersion {
            path: path.to_path_buf(),
        })?;
    let mut version = u32::try_from(raw_version).map_err(|_| PersistenceError::Migration {
        version: u32::MAX,
        reason: format!("schema version {raw_version} does not fit u32"),
    })?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(PersistenceError::FutureSchema {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    while version < CURRENT_SCHEMA_VERSION {
        value = match version {
            0 => migrate_v0_to_v1(value)?,
            1 => migrate_v1_to_v2(value)?,
            _ => return Err(PersistenceError::UnsupportedSchema { version }),
        };
        version += 1;
    }
    Ok(value)
}

fn migrate_v0_to_v1(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 0,
            reason: "save root must be an object".to_owned(),
        })?;
    object.insert("schema_version".to_owned(), Value::from(1));
    object
        .entry("audit_log".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    Ok(value)
}

fn migrate_v1_to_v2(mut value: Value) -> Result<Value, PersistenceError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| PersistenceError::Migration {
            version: 1,
            reason: "save root must be an object".to_owned(),
        })?;
    object.insert("schema_version".to_owned(), Value::from(2));
    for field in [
        "institution_runtime",
        "contracts",
        "loans",
        "properties",
        "employment",
        "family_links",
        "family_councils",
        "laws",
        "relationships",
        "information_reports",
        "ai_objectives",
        "districts",
        "public_works",
        "legal_cases",
        "external_routes",
        "crises",
    ] {
        object
            .entry(field.to_owned())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
    object
        .entry("outbox".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let next_ids = object
        .get_mut("next_ids")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| PersistenceError::Migration {
            version: 1,
            reason: "save next_ids must be an object".to_owned(),
        })?;
    for field in [
        "contract",
        "property",
        "loan",
        "employment",
        "family_link",
        "law",
        "information_report",
        "objective",
        "public_work",
        "legal_case",
        "external_route",
        "crisis",
        "outbox",
    ] {
        next_ids
            .entry(field.to_owned())
            .or_insert_with(|| Value::from(0));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::NewGameConfig;
    use crate::registry::build_rivergate_registry;
    use crate::systems::{advance_days, build_new_game};
    use pretty_assertions::assert_eq;

    #[test]
    fn save_load_round_trip_preserves_deterministic_state() {
        let registry = build_rivergate_registry();
        let mut state = build_new_game(&registry, NewGameConfig::default());
        advance_days(&registry, &mut state, 40).expect("simulation must advance");
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let path = directory.path().join("campaign.json");

        save_state(&path, &state).expect("state must save");
        let loaded = load_state(&path).expect("state must load");

        assert_eq!(loaded, state);
    }

    #[test]
    fn version_zero_migration_adds_audit_log() {
        let registry = build_rivergate_registry();
        let state = build_new_game(&registry, NewGameConfig::default());
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let object = value.as_object_mut().expect("state JSON must be an object");
        object.insert("schema_version".to_owned(), Value::from(0));
        object.remove("audit_log");

        let migrated =
            migrate_to_current(value, Path::new("memory.json")).expect("version zero must migrate");

        assert_eq!(migrated["schema_version"], Value::from(2));
        assert!(migrated["audit_log"].is_array());
    }

    #[test]
    fn version_one_load_hydrates_strategic_state() {
        let registry = build_rivergate_registry();
        let state = build_new_game(&registry, NewGameConfig::default());
        let mut value = serde_json::to_value(state).expect("state must serialize");
        let object = value.as_object_mut().expect("state JSON must be an object");
        object.insert("schema_version".to_owned(), Value::from(1));
        for field in [
            "institution_runtime",
            "contracts",
            "loans",
            "properties",
            "employment",
            "family_links",
            "family_councils",
            "laws",
            "relationships",
            "information_reports",
            "ai_objectives",
            "districts",
            "public_works",
            "legal_cases",
            "external_routes",
            "crises",
            "outbox",
        ] {
            object.remove(field);
        }
        let next_ids = object
            .get_mut("next_ids")
            .and_then(Value::as_object_mut)
            .expect("next IDs must be an object");
        for field in [
            "contract",
            "property",
            "loan",
            "employment",
            "family_link",
            "law",
            "information_report",
            "objective",
            "public_work",
            "legal_case",
            "external_route",
            "crisis",
            "outbox",
        ] {
            next_ids.remove(field);
        }
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let path = directory.path().join("version-one.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&value).expect("legacy state must serialize"),
        )
        .expect("legacy save must be written");

        let loaded = load_state(&path).expect("version one save must load");

        assert_eq!(loaded.schema_version(), CURRENT_SCHEMA_VERSION);
        assert!(!loaded.properties.is_empty());
        assert!(!loaded.contracts.is_empty());
        assert_eq!(loaded.districts.len(), registry.districts().len());
        crate::systems::validate_invariants(&registry, &loaded);
    }
}
