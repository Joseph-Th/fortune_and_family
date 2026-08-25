//! Current-schema JSON persistence with release validation and atomic writes.

use crate::core::{
    AppState, AuditKind, CURRENT_SCHEMA_VERSION, FamilyLinkKind, InformationTarget, LegalCaseKind,
    LegalClaimSource,
};
use crate::ids::{BusinessId, HouseholdId};
use crate::money::{Money, Quantity, checked_cost_for};
use crate::systems::is_schedulable_day;
use serde::Deserialize;
#[cfg(test)]
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tempfile::Builder;
use thiserror::Error;

pub const MAX_SAVE_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateValidationKind {
    Schema,
    Scenario,
    DefinitionReferences,
    PrimaryRecords,
    StrategicRecords,
    NumericRanges,
    IdentifierAllocation,
}

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("failed to create save directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create temporary save beside {path}: {source}")]
    CreateTemporary {
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
    #[error("failed to synchronize save directory {path}: {source}")]
    SyncDirectory {
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
    #[error("save path does not resolve to a regular file: {path}")]
    NotRegularFile { path: PathBuf },
    #[error("save file {path} is too large: {actual} bytes exceeds the {maximum}-byte limit")]
    SaveTooLarge {
        path: PathBuf,
        actual: u64,
        maximum: u64,
    },
    #[error("failed to parse save file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("save file {path} contains duplicate member {member:?} at {json_path}")]
    DuplicateMember {
        path: PathBuf,
        json_path: String,
        member: String,
    },
    #[error("save file {path} exists and explicit overwrite was not requested")]
    DestinationExists { path: PathBuf },
    #[error(
        "save file {path} was modified by another writer (expected revision {expected_revision}, found {current_revision})"
    )]
    StaleWriterConflict {
        path: PathBuf,
        expected_revision: String,
        current_revision: String,
    },
    #[error("save file {path} has no numeric schema_version")]
    MissingSchemaVersion { path: PathBuf },
    #[error(
        "save file {path} uses schema version {found}; only current schema version {supported} is supported"
    )]
    UnsupportedSchemaVersion {
        path: PathBuf,
        found: u64,
        supported: u32,
    },
    #[error("save file {path} contains invalid {kind:?} state: {reason}")]
    InvalidState {
        path: PathBuf,
        kind: StateValidationKind,
        reason: String,
    },
}

/// The outcome of a save operation after visibility commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Save was atomically replaced; parent directory durability was
    /// synchronized on platforms that support directory synchronization.
    Committed,
    /// Save was atomically replaced and is visible to subsequent reads, but the
    /// attempted parent directory synchronization failed or was degraded.
    CommittedWithDegradedDurability,
}

/// A deterministic fingerprint of a save file's committed contents for CAS / optimistic concurrency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveRevision {
    hash: u64,
    size: u64,
}

impl SaveRevision {
    #[must_use]
    pub fn of_bytes(bytes: &[u8]) -> Self {
        let mut hasher = crate::registry::DeterministicRegistryHasher::new();
        hasher.write(bytes);
        Self {
            hash: hasher.finish(),
            size: bytes.len() as u64,
        }
    }

    #[must_use]
    pub fn display_token(&self) -> String {
        format!("{:016x}:{}", self.hash, self.size)
    }
}

#[derive(Debug)]
struct StateValidationError {
    kind: StateValidationKind,
    reason: String,
}

impl StateValidationError {
    fn new(kind: StateValidationKind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
std::thread_local! {
    static INJECT_DIRECTORY_SYNC_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn set_inject_directory_sync_failure_for_test(inject: bool) {
    INJECT_DIRECTORY_SYNC_FAILURE.with(|cell| cell.set(inject));
}

/// Performs the platform's parent-directory synchronization where the platform
/// supports it. Unix opens and syncs the directory; platforms without
/// directory-fd semantics perform no synchronization and always report a full
/// [`SaveOutcome::Committed`], whose contract is scoped to platform support.
#[allow(clippy::unnecessary_wraps)]
fn sync_save_directory_with_injection(
    #[allow(unused_variables)] parent: &Path,
) -> Result<(), PersistenceError> {
    #[cfg(test)]
    if INJECT_DIRECTORY_SYNC_FAILURE.with(std::cell::Cell::get) {
        return Err(PersistenceError::SyncDirectory {
            path: parent.to_path_buf(),
            source: std::io::Error::other("injected directory sync failure"),
        });
    }
    #[cfg(unix)]
    sync_save_directory(parent)?;
    Ok(())
}

fn save_state_impl(
    path: &Path,
    state: &AppState,
    expected_revision: Option<&SaveRevision>,
    overwrite: Option<bool>,
) -> Result<SaveOutcome, PersistenceError> {
    if let Some(overwrite_allowed) = overwrite
        && !overwrite_allowed
        && (path.exists() || fs::symlink_metadata(path).is_ok())
    {
        return Err(PersistenceError::DestinationExists {
            path: path.to_path_buf(),
        });
    }

    // Compare-and-swap validation runs before anything else so a stale writer
    // is reported as a conflict rather than being masked by an unrelated
    // validation failure. The check-then-write window below is deliberately
    // narrow but not lock-free: two writers that read the same baseline may
    // still race, and single-writer callers (the supported usage) are exact.
    if let Some(expected) = expected_revision {
        if path.exists() {
            let current_bytes = read_bounded_save(path)?;
            let current_revision = SaveRevision::of_bytes(&current_bytes);
            if &current_revision != expected {
                return Err(PersistenceError::StaleWriterConflict {
                    path: path.to_path_buf(),
                    expected_revision: expected.display_token(),
                    current_revision: current_revision.display_token(),
                });
            }
        } else {
            return Err(PersistenceError::StaleWriterConflict {
                path: path.to_path_buf(),
                expected_revision: expected.display_token(),
                current_revision: "missing".to_owned(),
            });
        }
    }

    validate_state(state).map_err(|error| PersistenceError::InvalidState {
        path: path.to_path_buf(),
        kind: error.kind,
        reason: error.reason,
    })?;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if parent != Path::new(".") {
        fs::create_dir_all(parent).map_err(|source| PersistenceError::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|source| PersistenceError::Serialize { source })?;
    let prefix = path
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| ".campaign-save-".to_owned(), |name| format!(".{name}."));
    let mut temporary = Builder::new()
        .prefix(&prefix)
        .tempfile_in(parent)
        .map_err(|source| PersistenceError::CreateTemporary {
            path: path.to_path_buf(),
            source,
        })?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|source| PersistenceError::Write {
            path: path.to_path_buf(),
            source,
        })?;
    temporary
        .persist(path)
        .map_err(|error| PersistenceError::Write {
            path: path.to_path_buf(),
            source: error.error,
        })?;

    let outcome = match sync_save_directory_with_injection(parent) {
        Ok(()) => SaveOutcome::Committed,
        Err(_) => SaveOutcome::CommittedWithDegradedDurability,
    };
    Ok(outcome)
}

/// Serializes the complete application state to a JSON save file.
///
/// # Errors
///
/// Returns an error when state validation fails, the parent directory or temporary file cannot be
/// created, serialization fails, or the destination cannot be atomically replaced.
pub fn save_state(
    path: impl AsRef<Path>,
    state: &AppState,
) -> Result<SaveOutcome, PersistenceError> {
    save_state_impl(path.as_ref(), state, None, Some(true))
}

/// Serializes the application state to an existing campaign path using compare-and-swap validation.
///
/// # Errors
///
/// Returns `PersistenceError::StaleWriterConflict` when the destination was modified by another
/// writer since `expected_revision` was read.
pub fn save_state_cas(
    path: impl AsRef<Path>,
    state: &AppState,
    expected_revision: &SaveRevision,
) -> Result<SaveOutcome, PersistenceError> {
    save_state_impl(path.as_ref(), state, Some(expected_revision), Some(true))
}

/// Serializes a new campaign state, optionally requiring that the destination does not already exist.
///
/// # Errors
///
/// Returns `PersistenceError::DestinationExists` if `overwrite` is false and the file exists.
pub fn save_state_new(
    path: impl AsRef<Path>,
    state: &AppState,
    overwrite: bool,
) -> Result<SaveOutcome, PersistenceError> {
    save_state_impl(path.as_ref(), state, None, Some(overwrite))
}

#[cfg(unix)]
fn sync_save_directory(parent: &Path) -> Result<(), PersistenceError> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| PersistenceError::SyncDirectory {
            path: parent.to_path_buf(),
            source,
        })
}

/// Atomically writes adapter-generated output (dashboard HTML, gameplay and
/// art review reports) with the same staging, visibility, and durability
/// contract as campaign saves: synchronized same-directory temporary file,
/// atomic replacement, then best-effort parent-directory synchronization.
///
/// An existing destination must be a regular file; symlinks, directories, and
/// other non-regular targets are rejected.
///
/// # Errors
///
/// Returns an IO error when the destination is not a regular file, the parent
/// directory or temporary file cannot be created or written, or the
/// destination cannot be atomically replaced.
pub fn write_generated_file(
    path: impl AsRef<Path>,
    contents: &[u8],
) -> std::io::Result<SaveOutcome> {
    let path = path.as_ref();
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "generated output path is not a regular file: {}",
                    path.display()
                ),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if parent != Path::new(".") {
        fs::create_dir_all(parent)?;
    }
    let prefix = path.file_name().and_then(|name| name.to_str()).map_or_else(
        || ".generated-output-".to_owned(),
        |name| format!(".{name}."),
    );
    let mut temporary = Builder::new().prefix(&prefix).tempfile_in(parent)?;
    temporary.write_all(contents)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;

    Ok(match sync_save_directory_with_injection(parent) {
        Ok(()) => SaveOutcome::Committed,
        Err(_) => SaveOutcome::CommittedWithDegradedDurability,
    })
}

/// Loads and validates a current-schema JSON save file, returning its committed file revision.
///
/// # Errors
///
/// Returns an error when the path is not a bounded regular file, contains duplicate JSON members,
/// cannot be parsed, uses an unsupported schema, or fails release validation.
pub fn load_state_with_revision(
    path: impl AsRef<Path>,
) -> Result<(AppState, SaveRevision), PersistenceError> {
    let path = path.as_ref();
    let bytes = read_bounded_save(path)?;
    let revision = SaveRevision::of_bytes(&bytes);
    validate_no_duplicate_json_members(&bytes, path)?;
    require_current_schema(&bytes, path)?;
    let state: AppState =
        serde_json::from_slice(&bytes).map_err(|source| PersistenceError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    validate_state(&state).map_err(|error| PersistenceError::InvalidState {
        path: path.to_path_buf(),
        kind: error.kind,
        reason: error.reason,
    })?;
    Ok((state, revision))
}

/// Loads and validates a current-schema JSON save file.
///
/// # Errors
///
/// Returns an error when the path is not a bounded regular file, the file cannot be read or parsed,
/// contains duplicate JSON members, the schema version is not current, or the deserialized state
/// fails release validation.
pub fn load_state(path: impl AsRef<Path>) -> Result<AppState, PersistenceError> {
    load_state_with_revision(path).map(|(state, _)| state)
}

fn read_bounded_save(path: &Path) -> Result<Vec<u8>, PersistenceError> {
    // The pre-open stat rejects regular-file violations and gross oversizing
    // before allocation; the post-open re-stat closes the TOCTOU window
    // between stat and open. The post-read length check is the authoritative
    // bound: `take(MAX + 1)` guarantees the buffer can never exceed the limit.
    let metadata = fs::metadata(path).map_err(|source| PersistenceError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(PersistenceError::NotRegularFile {
            path: path.to_path_buf(),
        });
    }
    reject_oversized_save(path, metadata.len())?;

    let file = fs::File::open(path).map_err(|source| PersistenceError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let opened_metadata = file.metadata().map_err(|source| PersistenceError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if !opened_metadata.is_file() {
        return Err(PersistenceError::NotRegularFile {
            path: path.to_path_buf(),
        });
    }
    reject_oversized_save(path, opened_metadata.len())?;

    let mut bytes = Vec::with_capacity(usize::try_from(opened_metadata.len()).unwrap_or(0));
    file.take(MAX_SAVE_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| PersistenceError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    reject_oversized_save(path, u64::try_from(bytes.len()).unwrap_or(u64::MAX))?;
    Ok(bytes)
}

fn reject_oversized_save(path: &Path, actual: u64) -> Result<(), PersistenceError> {
    if actual > MAX_SAVE_FILE_BYTES {
        return Err(PersistenceError::SaveTooLarge {
            path: path.to_path_buf(),
            actual,
            maximum: MAX_SAVE_FILE_BYTES,
        });
    }
    Ok(())
}

fn validate_state(state: &AppState) -> Result<(), StateValidationError> {
    if state.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(StateValidationError::new(
            StateValidationKind::Schema,
            format!(
                "state schema {} does not match current schema {CURRENT_SCHEMA_VERSION}",
                state.schema_version
            ),
        ));
    }
    if state.scenario_key != "rivergate" {
        return Err(StateValidationError::new(
            StateValidationKind::Scenario,
            format!("unsupported scenario key {:?}", state.scenario_key),
        ));
    }
    let registry = crate::registry::build_rivergate_registry();
    if state.registry_fingerprint != registry.fingerprint() {
        return Err(StateValidationError::new(
            StateValidationKind::Scenario,
            format!(
                "registry fingerprint mismatch: save has {:016x}, current registry has {:016x}",
                state.registry_fingerprint,
                registry.fingerprint()
            ),
        ));
    }
    validate_simulation_clock(state)
        .map_err(|reason| StateValidationError::new(StateValidationKind::NumericRanges, reason))?;
    validate_definition_references(&registry, state).map_err(|reason| {
        StateValidationError::new(StateValidationKind::DefinitionReferences, reason)
    })?;
    validate_primary_records(&registry, state)
        .map_err(|reason| StateValidationError::new(StateValidationKind::PrimaryRecords, reason))?;
    validate_strategic_records(&registry, state).map_err(|reason| {
        StateValidationError::new(StateValidationKind::StrategicRecords, reason)
    })?;
    validate_numeric_ranges(state)
        .map_err(|reason| StateValidationError::new(StateValidationKind::NumericRanges, reason))?;
    validate_campaign_phase_consistency(state)
        .map_err(|reason| StateValidationError::new(StateValidationKind::PrimaryRecords, reason))?;
    state.validate_next_ids().map_err(|reason| {
        StateValidationError::new(StateValidationKind::IdentifierAllocation, reason)
    })
}

struct JsonDuplicateScanner<'a> {
    bytes: &'a [u8],
    pos: usize,
    path: &'a Path,
}

impl<'a> JsonDuplicateScanner<'a> {
    const fn new(bytes: &'a [u8], path: &'a Path) -> Self {
        Self {
            bytes,
            pos: 0,
            path,
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_whitespace();
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self, current_path: &str) -> Result<(), PersistenceError> {
        self.skip_whitespace();
        let b = self
            .bytes
            .get(self.pos)
            .copied()
            .ok_or_else(|| PersistenceError::Parse {
                path: self.path.to_path_buf(),
                source: serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "unexpected end of JSON input",
                )),
            })?;
        match b {
            b'{' => self.parse_object(current_path),
            b'[' => self.parse_array(current_path),
            b'"' => {
                self.parse_string()?;
                Ok(())
            }
            b't' | b'f' | b'n' => {
                self.parse_literal();
                Ok(())
            }
            b'-' | b'0'..=b'9' => {
                self.parse_number();
                Ok(())
            }
            _ => Err(PersistenceError::Parse {
                path: self.path.to_path_buf(),
                source: serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid character in JSON: {}", b as char),
                )),
            }),
        }
    }

    fn parse_object(&mut self, current_path: &str) -> Result<(), PersistenceError> {
        self.pos += 1; // skip '{'
        let mut seen_keys = BTreeSet::new();
        loop {
            self.skip_whitespace();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                break;
            }
            if self.peek() != Some(b'"') {
                return Err(PersistenceError::Parse {
                    path: self.path.to_path_buf(),
                    source: serde_json::Error::io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "expected string key in object",
                    )),
                });
            }
            let key = self.parse_string()?;
            if !seen_keys.insert(key.clone()) {
                return Err(PersistenceError::DuplicateMember {
                    path: self.path.to_path_buf(),
                    json_path: current_path.to_owned(),
                    member: key,
                });
            }
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(PersistenceError::Parse {
                    path: self.path.to_path_buf(),
                    source: serde_json::Error::io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "expected ':' after key",
                    )),
                });
            }
            self.pos += 1; // skip ':'
            let child_path = if current_path == "$" {
                format!("$.{key}")
            } else {
                format!("{current_path}.{key}")
            };
            self.parse_value(&child_path)?;
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    return Err(PersistenceError::Parse {
                        path: self.path.to_path_buf(),
                        source: serde_json::Error::io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "expected ',' or '}' in object",
                        )),
                    });
                }
            }
        }
        Ok(())
    }

    fn parse_array(&mut self, current_path: &str) -> Result<(), PersistenceError> {
        self.pos += 1; // skip '['
        let mut index = 0;
        loop {
            self.skip_whitespace();
            if self.peek() == Some(b']') {
                self.pos += 1;
                break;
            }
            let child_path = format!("{current_path}[{index}]");
            self.parse_value(&child_path)?;
            index += 1;
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => {
                    return Err(PersistenceError::Parse {
                        path: self.path.to_path_buf(),
                        source: serde_json::Error::io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "expected ',' or ']' in array",
                        )),
                    });
                }
            }
        }
        Ok(())
    }

    fn parse_string(&mut self) -> Result<String, PersistenceError> {
        self.pos += 1; // skip '"'
        let mut result = String::new();
        let mut escaped = false;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if escaped {
                match b {
                    b'"' => result.push('"'),
                    b'\\' => result.push('\\'),
                    b'/' => result.push('/'),
                    b'b' => result.push('\x08'),
                    b'f' => result.push('\x0c'),
                    b'n' => result.push('\n'),
                    b'r' => result.push('\r'),
                    b't' => result.push('\t'),
                    b'u' => {
                        if self.pos + 4 >= self.bytes.len() {
                            return Err(PersistenceError::Parse {
                                path: self.path.to_path_buf(),
                                source: serde_json::Error::io(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "incomplete unicode escape",
                                )),
                            });
                        }
                        let hex_slice = std::str::from_utf8(
                            &self.bytes[self.pos + 1..=self.pos + 4],
                        )
                        .map_err(|_| PersistenceError::Parse {
                            path: self.path.to_path_buf(),
                            source: serde_json::Error::io(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "invalid utf8 in unicode escape",
                            )),
                        })?;
                        let code = u32::from_str_radix(hex_slice, 16).map_err(|_| {
                            PersistenceError::Parse {
                                path: self.path.to_path_buf(),
                                source: serde_json::Error::io(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "invalid hex in unicode escape",
                                )),
                            }
                        })?;
                        if let Some(ch) = char::from_u32(code) {
                            result.push(ch);
                        }
                        self.pos += 4;
                    }
                    _ => result.push(b as char),
                }
                escaped = false;
                self.pos += 1;
            } else if b == b'\\' {
                escaped = true;
                self.pos += 1;
            } else if b == b'"' {
                self.pos += 1; // skip closing quote
                return Ok(result);
            } else {
                let start = self.pos;
                while self.pos < self.bytes.len()
                    && self.bytes[self.pos] != b'\\'
                    && self.bytes[self.pos] != b'"'
                {
                    self.pos += 1;
                }
                let chunk = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| {
                    PersistenceError::Parse {
                        path: self.path.to_path_buf(),
                        source: serde_json::Error::io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "invalid UTF-8 sequence in string",
                        )),
                    }
                })?;
                result.push_str(chunk);
            }
        }
        Err(PersistenceError::Parse {
            path: self.path.to_path_buf(),
            source: serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "unterminated string in JSON",
            )),
        })
    }

    fn parse_literal(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'a'..=b'z' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn parse_number(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E' => self.pos += 1,
                _ => break,
            }
        }
    }
}

fn validate_no_duplicate_json_members(bytes: &[u8], path: &Path) -> Result<(), PersistenceError> {
    let mut scanner = JsonDuplicateScanner::new(bytes, path);
    scanner.parse_value("$")?;
    scanner.skip_whitespace();
    Ok(())
}

fn validate_campaign_phase_consistency(state: &AppState) -> Result<(), String> {
    for dynasty_id in state.dynasties.keys().copied() {
        if !crate::systems::campaign_phase_is_consistent(state, dynasty_id) {
            return Err(format!(
                "dynasty {dynasty_id} has a stale or incompatible campaign phase"
            ));
        }
    }
    Ok(())
}

fn validate_numeric_ranges(state: &AppState) -> Result<(), String> {
    validate_core_numeric_ranges(state)?;
    validate_financial_numeric_ranges(state)?;
    validate_civic_numeric_ranges(state)
}

fn validate_simulation_clock(state: &AppState) -> Result<(), String> {
    if state.clock.day() < 0 || state.clock.day() == i64::MAX {
        return Err(format!(
            "simulation clock has invalid or exhausted elapsed day {}",
            state.clock.day()
        ));
    }
    Ok(())
}

fn validate_core_numeric_ranges(state: &AppState) -> Result<(), String> {
    for dynasty in state.dynasties.values() {
        if dynasty.treasury() < Money::ZERO
            || dynasty.civic_contributions() < Money::ZERO
            || dynasty.resources.legitimacy_basis_points > 10_000
            || dynasty.resources.reputation_quality_basis_points > 10_000
            || dynasty.resources.reputation_reliability_basis_points > 10_000
            || dynasty.runtime.generation == 0
            || dynasty.runtime.generation == u16::MAX
            || dynasty.runtime.succession_risk_basis_points > 10_000
        {
            return Err(format!(
                "dynasty {} has an invalid resource value",
                dynasty.id()
            ));
        }
    }
    for character in state.characters.iter() {
        if character.birth_day() > state.clock.day()
            || character.runtime.health_basis_points > 10_000
            || (character.status() == crate::core::CharacterStatus::Active
                && character.runtime.health_basis_points == 0)
            || character.runtime.loyalty_basis_points > 10_000
            || character.capabilities.administration > 100
            || character.capabilities.commerce > 100
            || character.capabilities.social > 100
            || character.capabilities.craft > 100
        {
            return Err(format!(
                "character {} has an invalid birth date, capability, or basis-point value",
                character.id()
            ));
        }
    }
    for household in state.households.iter() {
        if household.members == 0
            || household.cash() < Money::ZERO
            || household.weekly_income < Money::ZERO
            || household.bread_need_daily < Quantity::ZERO
            || household.ale_need_daily < Quantity::ZERO
            || household.food_satisfaction_basis_points() > 10_000
        {
            return Err(format!(
                "household {} has an invalid economic value",
                household.id()
            ));
        }
    }
    for business in state.businesses.iter() {
        if business.cash() < Money::ZERO
            || business.finance.lifetime_revenue < Money::ZERO
            || business.finance.lifetime_costs < Money::ZERO
            || business.operations.capacity_batches_per_day == 0
            || business.operations.condition_basis_points > 10_000
            || business.operations.quality_basis_points > 10_000
            || business.policy.target_input_days > 30
            || business.policy.target_output_days > 30
            || business.policy.minimum_cash_reserve < Money::ZERO
            || business.policy.maintenance_basis_points > 10_000
            || business.policy.quality_target_basis_points > 10_000
            || business.finance.version == u64::MAX
            || business
                .inventory()
                .values()
                .any(|quantity| *quantity < Quantity::ZERO)
        {
            return Err(format!(
                "business {} has an invalid economic value",
                business.id()
            ));
        }
    }
    if state.market.clearing_account < Money::ZERO {
        return Err("market clearing account has an invalid value".to_owned());
    }
    for quote in state.market.quotes.values() {
        if quote.price <= Money::ZERO
            || quote.previous_price <= Money::ZERO
            || quote.stock < Quantity::ZERO
            || quote.target_stock <= Quantity::ZERO
            || quote.demand_today < Quantity::ZERO
            || quote.supply_today < Quantity::ZERO
        {
            return Err(format!(
                "market quote {} has an invalid value",
                quote.good_id()
            ));
        }
    }
    for (good_id, price) in &state.market.month_start_prices {
        if *price <= Money::ZERO || !state.market.quotes.contains_key(good_id) {
            return Err(format!("market month-start price for {good_id} is invalid"));
        }
    }
    Ok(())
}

fn validate_financial_numeric_ranges(state: &AppState) -> Result<(), String> {
    for loan in state.loans.values() {
        if loan.principal <= Money::ZERO
            || loan.balance < Money::ZERO
            || loan.weekly_payment <= Money::ZERO
            || loan.interest_basis_points > 10_000
        {
            return Err(format!("loan {} has an invalid financial value", loan.id));
        }
        if !is_schedulable_day(loan.next_due_day)
            || (loan.status.is_repayment_active()
                && !crate::systems::is_settleable_weekly_due_day(
                    state.clock.day(),
                    loan.next_due_day,
                ))
        {
            return Err(format!("loan {} has an invalid due date", loan.id));
        }
    }
    for debt in state.civic_debts.values() {
        if debt.principal <= Money::ZERO
            || debt.balance < Money::ZERO
            || debt.weekly_payment <= Money::ZERO
            || debt.interest_basis_points > 10_000
        {
            return Err(format!(
                "civic debt {} has an invalid financial value",
                debt.id
            ));
        }
        if !is_schedulable_day(debt.next_due_day)
            || (matches!(
                debt.status,
                crate::core::CivicDebtStatus::Current | crate::core::CivicDebtStatus::Delinquent
            ) && !crate::systems::is_settleable_weekly_due_day(
                state.clock.day(),
                debt.next_due_day,
            ))
        {
            return Err(format!("civic debt {} has invalid dates", debt.id));
        }
    }
    for property in state.properties.values() {
        if property.value <= Money::ZERO
            || property.anchor_value < Money::ZERO
            || property.weekly_rent < Money::ZERO
            || property.condition_basis_points > 10_000
        {
            return Err(format!(
                "property {} has an invalid financial value",
                property.id
            ));
        }
    }
    for agreement in state.employment.values() {
        if agreement.weekly_wage <= Money::ZERO
            || agreement.workers == 0
            || agreement.conditions_basis_points > 10_000
            || agreement.loyalty_basis_points > 10_000
        {
            return Err(format!(
                "employment agreement {} has an invalid financial value",
                agreement.id
            ));
        }
    }
    for contract in state.contracts.values() {
        if contract.quantity_per_week <= Quantity::ZERO
            || contract.unit_price <= Money::ZERO
            || contract.penalty < Money::ZERO
            || contract.unpaid_breach_penalty < Money::ZERO
            || contract.unpaid_breach_penalty > contract.penalty
            || contract.collected_breach_penalty < Money::ZERO
            || contract
                .collected_breach_penalty
                .saturating_add(contract.unpaid_breach_penalty)
                > contract.penalty
            || checked_cost_for(contract.quantity_per_week, contract.unit_price).is_none()
        {
            return Err(format!(
                "supply contract {} has an invalid financial value",
                contract.id
            ));
        }
        if !is_schedulable_day(contract.next_due_day)
            || !is_schedulable_day(contract.end_day)
            || (contract.status == crate::core::ContractStatus::Active
                && !crate::systems::is_settleable_weekly_due_day(
                    state.clock.day(),
                    contract.next_due_day,
                ))
        {
            return Err(format!("supply contract {} has invalid dates", contract.id));
        }
    }
    validate_institution_numeric_ranges(state)
}

fn validate_institution_numeric_ranges(state: &AppState) -> Result<(), String> {
    for institution in state.institutions.values() {
        if institution.budget < Money::ZERO
            || institution.legitimacy_basis_points > 10_000
            || institution.term_number == 0
            || institution.term_number == u32::MAX
            || institution.term_started_day > state.clock.day()
            || !crate::systems::is_valid_institution_selection_day(
                institution.term_started_day,
                institution.next_selection_day,
            )
            || institution.active_directive.is_some_and(|directive| {
                !crate::systems::is_valid_active_directive_expiry(
                    state.clock.day(),
                    directive.expires_day,
                ) || !institution.powers.contains(&directive.power)
            })
        {
            return Err(format!(
                "institution {} has an invalid budget, term timing, or directive",
                institution.institution_id
            ));
        }
    }
    Ok(())
}

fn validate_civic_numeric_ranges(state: &AppState) -> Result<(), String> {
    for council in state.family_councils.values() {
        if council.unity_basis_points > 10_000 || council.charter_version == u64::MAX {
            return Err(format!(
                "family council {} has invalid unity or an exhausted charter version",
                council.dynasty_id
            ));
        }
    }
    for district in state.districts.values() {
        if district.rent_index_basis_points < crate::systems::MIN_DISTRICT_RENT_INDEX_BASIS_POINTS
            || district.rent_index_basis_points
                > crate::systems::MAX_DISTRICT_RENT_INDEX_BASIS_POINTS
            || district.employment_basis_points > 10_000
            || district.sanitation_basis_points > 10_000
            || district.safety_basis_points > 10_000
            || district.unrest_basis_points > 10_000
        {
            return Err(format!(
                "district {} has an invalid basis-point value",
                district.district_id
            ));
        }
    }
    for work in state.public_works.values() {
        if work.budget <= Money::ZERO
            || work.spent < Money::ZERO
            || work.spent > work.budget
            || work.progress_basis_points > 10_000
        {
            return Err(format!(
                "public work {} has an invalid progress value",
                work.id
            ));
        }
        let expected_progress =
            crate::systems::public_work_progress_basis_points(work.spent, work.budget);
        if work.progress_basis_points != expected_progress
            || (work.status == crate::core::PublicWorkStatus::Completed)
                != (work.spent == work.budget)
        {
            return Err(format!(
                "public work {} progress does not match its spending or lifecycle",
                work.id
            ));
        }
    }
    for route in state.external_routes.values() {
        if route.daily_capacity < Quantity::ZERO
            || route.risk_basis_points > 10_000
            || route.disruption_basis_points > 10_000
            || route.toll_basis_points > 10_000
        {
            return Err(format!("external route {} has an invalid value", route.id));
        }
    }
    for crisis in state.crises.values() {
        if crisis.severity_basis_points > 10_000 {
            return Err(format!("crisis {} has an invalid severity", crisis.id));
        }
    }
    validate_legal_case_numeric_ranges(state)?;
    for relationship in state.relationships.values() {
        if relationship.trust_basis_points > 10_000
            || relationship.fear_basis_points > 10_000
            || relationship.respect_basis_points > 10_000
            || relationship.resentment_basis_points > 10_000
        {
            return Err("relationship contains an invalid basis-point value".to_owned());
        }
    }
    Ok(())
}

fn validate_legal_case_numeric_ranges(state: &AppState) -> Result<(), String> {
    for legal_case in state.legal_cases.values() {
        if legal_case.evidence_basis_points > 10_000
            || legal_case.public_attention_basis_points > 10_000
            || legal_case.damages < Money::ZERO
        {
            return Err(format!(
                "legal case {} has an invalid measure or damages value",
                legal_case.id
            ));
        }
        if !crate::systems::is_valid_legal_hearing_day(legal_case.filed_day, legal_case.hearing_day)
        {
            return Err(format!("legal case {} has invalid dates", legal_case.id));
        }
    }
    Ok(())
}

fn validate_definition_references(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), String> {
    let expected_goods: BTreeSet<_> = registry
        .goods()
        .iter()
        .map(crate::registry::GoodDef::id)
        .collect();
    let actual_goods: BTreeSet<_> = state.market.quotes.keys().copied().collect();
    if actual_goods != expected_goods {
        return Err("market quote IDs do not match the scenario registry".to_owned());
    }
    if state
        .market
        .quotes
        .iter()
        .any(|(good_id, quote)| quote.good_id() != *good_id)
    {
        return Err("market quote map key differs from its record ID".to_owned());
    }
    for good in registry.goods() {
        let quote = state
            .market
            .quotes
            .get(&good.id())
            .expect("validated market quote ID set must contain every good");
        if quote.target_stock != good.target_market_stock() {
            return Err(format!(
                "market quote {} target stock does not match the scenario registry",
                good.id()
            ));
        }
    }
    let expected_districts: BTreeSet<_> = registry
        .districts()
        .iter()
        .map(crate::registry::DistrictDef::id)
        .collect();
    let actual_districts: BTreeSet<_> = state.districts.keys().copied().collect();
    if actual_districts != expected_districts {
        return Err("district runtime IDs do not match the scenario registry".to_owned());
    }
    if state
        .districts
        .iter()
        .any(|(district_id, district)| district.district_id != *district_id)
    {
        return Err("district runtime map key differs from its record ID".to_owned());
    }
    let expected_institutions: BTreeSet<_> = registry
        .institutions()
        .iter()
        .map(crate::registry::InstitutionDef::id)
        .collect();
    let actual_institutions: BTreeSet<_> = state.institutions.keys().copied().collect();
    if actual_institutions != expected_institutions {
        return Err("institution state IDs do not match the scenario registry".to_owned());
    }
    for definition in registry.institutions() {
        let institution = state
            .institutions
            .get(&definition.id())
            .expect("validated institution ID set must contain every definition");
        if institution.powers != crate::systems::institution_powers_for(definition.kind()) {
            return Err(format!(
                "institution {} powers do not match the scenario registry",
                definition.id()
            ));
        }
    }
    Ok(())
}

fn validate_primary_records(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), String> {
    if !state.dynasties.contains_key(&state.player_dynasty_id) {
        return Err("player dynasty does not exist".to_owned());
    }
    for (dynasty_id, dynasty) in &state.dynasties {
        if dynasty.id() != *dynasty_id {
            return Err(format!(
                "dynasty map key {dynasty_id} differs from record ID"
            ));
        }
        if dynasty.name().trim().is_empty() {
            return Err(format!("dynasty {dynasty_id} has a blank name"));
        }
        if dynasty.heir_id() == Some(dynasty.head_id()) {
            return Err(format!(
                "dynasty {dynasty_id} uses the same character as head and heir"
            ));
        }
        for character_id in [Some(dynasty.head_id()), dynasty.heir_id()]
            .into_iter()
            .flatten()
        {
            let character = state.characters.get(character_id).ok_or_else(|| {
                format!("dynasty {dynasty_id} references missing character {character_id}")
            })?;
            if character.dynasty_id() != *dynasty_id {
                return Err(format!(
                    "dynasty {dynasty_id} references character {character_id} from another dynasty"
                ));
            }
            if character.status() != crate::core::CharacterStatus::Active {
                return Err(format!(
                    "dynasty {dynasty_id} references inactive head or heir {character_id}"
                ));
            }
            let expected_role = if character_id == dynasty.head_id() {
                crate::core::CharacterRole::HeadOfHouse
            } else {
                crate::core::CharacterRole::Heir
            };
            if character.role() != expected_role {
                return Err(format!(
                    "dynasty {dynasty_id} head or heir {character_id} has the wrong role"
                ));
            }
        }
    }

    let mut character_index: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for (character_id, character) in state.characters.records() {
        if character.id() != *character_id || !state.dynasties.contains_key(&character.dynasty_id())
        {
            return Err(format!(
                "character {character_id} has an invalid identity reference"
            ));
        }
        if character.name().trim().is_empty() {
            return Err(format!("character {character_id} has a blank name"));
        }
        character_index
            .entry(character.dynasty_id())
            .or_default()
            .insert(*character_id);
    }
    if &character_index != state.characters.index() {
        return Err("character dynasty index is stale or incomplete".to_owned());
    }

    let mut household_index: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for (household_id, household) in state.households.records() {
        if household.id() != *household_id
            || registry.get_district(household.district_id()).is_none()
        {
            return Err(format!(
                "household {household_id} has an invalid identity reference"
            ));
        }
        household_index
            .entry(household.district_id())
            .or_default()
            .insert(*household_id);
    }
    if &household_index != state.households.index() {
        return Err("household district index is stale or incomplete".to_owned());
    }

    validate_business_records(registry, state)
}

fn validate_business_records(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), String> {
    let mut owner_index: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    let mut district_index: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    let mut administrative_load = BTreeMap::<_, u64>::new();
    for (business_id, business) in state.businesses.records() {
        let owner_id = business.owner_dynasty_id();
        if business.id() != *business_id
            || !state.dynasties.contains_key(&owner_id)
            || registry.get_district(business.district_id()).is_none()
            || registry.get_recipe(business.recipe_id()).is_none()
        {
            return Err(format!(
                "business {business_id} has an invalid definition reference"
            ));
        }
        if business.name().trim().is_empty() {
            return Err(format!("business {business_id} has a blank name"));
        }
        let manager = state
            .characters
            .get(business.manager_id())
            .ok_or_else(|| format!("business {business_id} references a missing manager"))?;
        if manager.dynasty_id() != owner_id {
            return Err(format!(
                "business {business_id} manager belongs to another dynasty"
            ));
        }
        if manager.status() != crate::core::CharacterStatus::Active {
            return Err(format!(
                "business {business_id} references an inactive manager"
            ));
        }
        // A business's premises link is the business-side half of the
        // occupancy relationship: it must resolve to an existing property
        // whose occupant is the business itself or nobody (evicted while
        // insolvent, re-occupiable on recovery).
        if let Some(premises_id) = business.premises_property_id() {
            let premises = state.properties.get(&premises_id).ok_or_else(|| {
                format!("business {business_id} references missing premises property {premises_id}")
            })?;
            if premises
                .occupant_business_id
                .is_some_and(|existing_id| existing_id != *business_id)
            {
                return Err(format!(
                    "business {business_id} premises {premises_id} are occupied by another business"
                ));
            }
        }
        if business
            .inventory()
            .keys()
            .any(|good_id| registry.get_good(*good_id).is_none())
        {
            return Err(format!(
                "business {business_id} contains an unknown inventory good"
            ));
        }
        owner_index
            .entry(owner_id)
            .or_default()
            .insert(*business_id);
        district_index
            .entry(business.district_id())
            .or_default()
            .insert(*business_id);
        let recipe = registry
            .get_recipe(business.recipe_id())
            .expect("validated business recipe must exist");
        administrative_load
            .entry(owner_id)
            .and_modify(|load| *load += u64::from(recipe.administrative_load()))
            .or_insert(u64::from(recipe.administrative_load()));
    }
    if &owner_index != state.businesses.owner_index()
        || &district_index != state.businesses.district_index()
    {
        return Err("business ownership or district index is stale or incomplete".to_owned());
    }
    for dynasty in state.dynasties.values() {
        let expected = administrative_load.get(&dynasty.id()).copied().unwrap_or(0);
        if u64::from(dynasty.administrative_load()) != expected {
            return Err(format!(
                "dynasty {} administrative load {} does not match derived load {expected}",
                dynasty.id(),
                dynasty.administrative_load()
            ));
        }
    }
    Ok(())
}

fn validate_strategic_records(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), String> {
    validate_contract_records(registry, state)?;
    let mut occupied_businesses = BTreeSet::new();
    for (property_id, property) in &state.properties {
        if property.id != *property_id || !state.districts.contains_key(&property.district_id) {
            return Err(format!(
                "property {property_id} has an invalid identity reference"
            ));
        }
        if property.name.trim().is_empty() {
            return Err(format!("property {property_id} has a blank name"));
        }
        if property.tenant_dynasty_id.is_some() && property.owner_dynasty_id.is_none() {
            return Err(format!(
                "property {property_id} has a tenant without an owner"
            ));
        }
        for dynasty_id in [property.owner_dynasty_id, property.tenant_dynasty_id]
            .into_iter()
            .flatten()
        {
            if !state.dynasties.contains_key(&dynasty_id) {
                return Err(format!(
                    "property {property_id} references a missing dynasty"
                ));
            }
        }
        if let Some(business_id) = property.occupant_business_id {
            let business = state.businesses.get(business_id).ok_or_else(|| {
                format!("property {property_id} references a missing occupant business")
            })?;
            if property.owner_dynasty_id.is_none()
                || !occupied_businesses.insert(business_id)
                || business.district_id() != property.district_id
            {
                return Err(format!(
                    "property {property_id} has an invalid or duplicate occupant"
                ));
            }
            if business.premises_property_id() != Some(*property_id) {
                return Err(format!(
                    "property {property_id} occupant business does not reference it as its premises"
                ));
            }
            let expected_tenant = property
                .owner_dynasty_id
                .filter(|owner_id| *owner_id != business.owner_dynasty_id())
                .map(|_| business.owner_dynasty_id());
            if property.tenant_dynasty_id != expected_tenant {
                return Err(format!(
                    "property {property_id} tenancy does not match its occupant business owner"
                ));
            }
        }
        if let Some(loan_id) = property.collateral_loan_id {
            let loan = state
                .loans
                .get(&loan_id)
                .ok_or_else(|| format!("property {property_id} references a missing loan"))?;
            if loan.collateral_property_id != Some(*property_id)
                || matches!(
                    loan.status,
                    crate::core::LoanStatus::Defaulted | crate::core::LoanStatus::Repaid
                )
                || property.owner_dynasty_id != Some(loan.borrower_dynasty_id)
            {
                return Err(format!(
                    "property {property_id} has an invalid collateral relationship"
                ));
            }
        }
    }
    validate_finance_and_organization_records(state)
}

fn validate_contract_records(
    registry: &crate::registry::Registry,
    state: &AppState,
) -> Result<(), String> {
    for (contract_id, contract) in &state.contracts {
        let buyer = state.businesses.get(contract.buyer_business_id);
        let seller = state.businesses.get(contract.seller_business_id);
        if contract.id != *contract_id
            || contract.buyer_business_id == contract.seller_business_id
            || buyer.is_none()
            || seller.is_none()
            || !state.market.quotes.contains_key(&contract.good_id)
        {
            return Err(format!(
                "supply contract {contract_id} has an invalid reference"
            ));
        }
        let buyer = buyer.expect("validated contract buyer must exist");
        let seller = seller.expect("validated contract seller must exist");
        if contract.status == crate::core::ContractStatus::Active
            && (matches!(
                buyer.status(),
                crate::core::BusinessStatus::Insolvent | crate::core::BusinessStatus::Closed
            ) || matches!(
                seller.status(),
                crate::core::BusinessStatus::Insolvent | crate::core::BusinessStatus::Closed
            ))
        {
            return Err(format!(
                "supply contract {contract_id} is incompatible with its business lifecycle"
            ));
        }
        let buyer_recipe = registry
            .get_recipe(buyer.recipe_id())
            .expect("validated business recipe must exist");
        let seller_recipe = registry
            .get_recipe(seller.recipe_id())
            .expect("validated business recipe must exist");
        let valid_breach_attribution = match (
            contract.breaching_dynasty_id,
            contract.breach_victim_dynasty_id,
        ) {
            (None, None) => true,
            // Attribution records the defendant for recoverable breach debt
            // from the first attributable miss, so it may outlive any status.
            (Some(breacher), Some(victim)) => {
                breacher != victim
                    && state.dynasties.contains_key(&breacher)
                    && state.dynasties.contains_key(&victim)
            }
            (Some(_), None) | (None, Some(_)) => false,
        };
        let valid_delivery_attribution = contract_delivery_attribution_is_valid(state, contract);
        if seller_recipe.output_good_id() != contract.good_id
            || !buyer_recipe
                .inputs()
                .iter()
                .any(|input| input.good_id() == contract.good_id)
            || (contract.status == crate::core::ContractStatus::Active
                && buyer.owner_dynasty_id() == seller.owner_dynasty_id())
            || (contract.status == crate::core::ContractStatus::Active
                && contract.next_due_day > contract.end_day)
            || (contract.unpaid_breach_penalty > Money::ZERO
                && (contract.breaching_dynasty_id.is_none()
                    || contract.breach_victim_dynasty_id.is_none()))
            || !valid_breach_attribution
            || !valid_delivery_attribution
        {
            return Err(format!(
                "supply contract {contract_id} is incompatible with its parties or term"
            ));
        }
    }
    Ok(())
}

fn contract_delivery_attribution_is_valid(
    state: &AppState,
    contract: &crate::core::SupplyContract,
) -> bool {
    contract
        .fulfilled_deliveries_by_dynasty
        .keys()
        .all(|dynasty_id| state.dynasties.contains_key(dynasty_id))
        && contract.has_consistent_delivery_attribution()
}

fn validate_finance_and_organization_records(state: &AppState) -> Result<(), String> {
    validate_loan_records(state)?;
    validate_civic_debt_records(state)?;
    validate_employment_records(state)?;
    validate_family_records(state)?;
    validate_institution_and_misc_records(state)
}

fn validate_civic_debt_records(state: &AppState) -> Result<(), String> {
    let mut authorizing_law_ids = BTreeSet::new();
    for (debt_id, debt) in &state.civic_debts {
        let authorizing_law = state.laws.get(&debt.authorizing_law_id);
        if debt.id != *debt_id
            || !state.dynasties.contains_key(&debt.creditor_dynasty_id)
            || debt.sponsor_dynasty_id == Some(debt.creditor_dynasty_id)
            || debt
                .sponsor_dynasty_id
                .is_some_and(|dynasty_id| !state.dynasties.contains_key(&dynasty_id))
            || !authorizing_law.is_some_and(|law| {
                law.kind == crate::core::LawKind::PublicDebtAuthorization
                    && law.sponsor_dynasty_id == debt.sponsor_dynasty_id
                    && law.value == debt.principal.copper()
            })
            || debt.issued_day > state.clock.day()
            || debt.next_due_day < debt.issued_day
        {
            return Err(format!(
                "civic debt {debt_id} has an invalid identity or authorization reference"
            ));
        }
        if !authorizing_law_ids.insert(debt.authorizing_law_id) {
            return Err(format!(
                "civic debt {debt_id} reuses consumed public-debt authorization {}",
                debt.authorizing_law_id
            ));
        }
        match debt.status {
            crate::core::CivicDebtStatus::Current => {
                if debt.balance <= Money::ZERO || debt.missed_payments != 0 {
                    return Err(format!(
                        "current civic debt {debt_id} has invalid balance or arrears"
                    ));
                }
            }
            crate::core::CivicDebtStatus::Delinquent => {
                if debt.balance <= Money::ZERO || !(1..3).contains(&debt.missed_payments) {
                    return Err(format!(
                        "delinquent civic debt {debt_id} has invalid balance or arrears"
                    ));
                }
            }
            crate::core::CivicDebtStatus::Defaulted => {
                if debt.balance <= Money::ZERO || debt.missed_payments < 3 {
                    return Err(format!(
                        "defaulted civic debt {debt_id} has invalid balance or arrears"
                    ));
                }
            }
            crate::core::CivicDebtStatus::Repaid => {
                if debt.balance != Money::ZERO || debt.missed_payments != 0 {
                    return Err(format!(
                        "repaid civic debt {debt_id} retains balance or arrears"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_loan_records(state: &AppState) -> Result<(), String> {
    let mut active_loan_pairs = BTreeSet::new();
    for (loan_id, loan) in &state.loans {
        if loan.id != *loan_id
            || loan.lender_dynasty_id == loan.borrower_dynasty_id
            || !state.dynasties.contains_key(&loan.lender_dynasty_id)
            || !state.dynasties.contains_key(&loan.borrower_dynasty_id)
        {
            return Err(format!("loan {loan_id} has an invalid dynasty reference"));
        }
        if loan.status.is_repayment_active()
            && !active_loan_pairs.insert((loan.lender_dynasty_id, loan.borrower_dynasty_id))
        {
            return Err(format!(
                "loan {loan_id} duplicates an existing repayment-active lender/borrower pair"
            ));
        }
        if !loan.status.has_consistent_arrears(loan.missed_payments) {
            return Err(format!(
                "loan {loan_id} status does not match its missed-payment count"
            ));
        }
        match loan.status {
            crate::core::LoanStatus::Current
            | crate::core::LoanStatus::Delinquent
            | crate::core::LoanStatus::Restructured
            | crate::core::LoanStatus::Defaulted => {
                if loan.balance <= Money::ZERO {
                    return Err(format!("unsettled loan {loan_id} has no remaining balance"));
                }
            }
            crate::core::LoanStatus::Repaid => {
                if loan.balance != Money::ZERO {
                    return Err(format!("repaid loan {loan_id} retains a balance"));
                }
            }
        }
        if let Some(property_id) = loan.collateral_property_id {
            let property = state.properties.get(&property_id).ok_or_else(|| {
                format!("loan {loan_id} references a missing collateral property")
            })?;
            match loan.status {
                crate::core::LoanStatus::Current
                | crate::core::LoanStatus::Delinquent
                | crate::core::LoanStatus::Restructured => {
                    if property.collateral_loan_id != Some(*loan_id)
                        || property.owner_dynasty_id != Some(loan.borrower_dynasty_id)
                    {
                        return Err(format!(
                            "loan {loan_id} has an invalid active collateral relationship"
                        ));
                    }
                }
                crate::core::LoanStatus::Defaulted => {
                    if property.collateral_loan_id == Some(*loan_id) {
                        return Err(format!(
                            "defaulted loan {loan_id} has an invalid collateral settlement"
                        ));
                    }
                }
                crate::core::LoanStatus::Repaid => {
                    if property.collateral_loan_id == Some(*loan_id) {
                        return Err(format!(
                            "repaid loan {loan_id} has an invalid collateral release"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_employment_records(state: &AppState) -> Result<(), String> {
    let mut workers_by_business = BTreeMap::<BusinessId, u64>::new();
    let mut workers_by_household = BTreeMap::<HouseholdId, u64>::new();
    for (employment_id, agreement) in &state.employment {
        let business = state.businesses.get(agreement.business_id);
        // Numeric bounds (positive wage, nonzero workers) are owned by
        // `validate_financial_numeric_ranges`; this pass owns references and
        // lifecycle agreement.
        if agreement.id != *employment_id
            || business.is_none()
            || state.households.get(agreement.household_id).is_none()
        {
            return Err(format!(
                "employment agreement {employment_id} has an invalid reference"
            ));
        }
        if business.is_some_and(|business| {
            !crate::systems::is_employment_status_compatible(business.status(), agreement.status)
        }) {
            return Err(format!(
                "employment agreement {employment_id} status is incompatible with its business lifecycle"
            ));
        }
        if agreement.status != crate::core::EmploymentStatus::Ended {
            workers_by_business
                .entry(agreement.business_id)
                .and_modify(|workers| *workers += u64::from(agreement.workers))
                .or_insert(u64::from(agreement.workers));
            workers_by_household
                .entry(agreement.household_id)
                .and_modify(|workers| *workers += u64::from(agreement.workers))
                .or_insert(u64::from(agreement.workers));
        }
    }
    for (business_id, workers) in workers_by_business {
        let business = state
            .businesses
            .get(business_id)
            .expect("validated employment business must exist");
        let supported_workers = crate::systems::supported_worker_capacity(business);
        if workers > u64::from(supported_workers) {
            return Err(format!(
                "business {business_id} employment exceeds operating capacity"
            ));
        }
    }
    for (household_id, workers) in workers_by_household {
        let members = state
            .households
            .get(household_id)
            .expect("validated employment household must exist")
            .members();
        if workers > u64::from(members) {
            return Err(format!(
                "household {household_id} employment exceeds household labor capacity"
            ));
        }
    }
    Ok(())
}

fn validate_family_records(state: &AppState) -> Result<(), String> {
    validate_family_links(state)?;
    validate_family_councils(state)
}

fn validate_family_links(state: &AppState) -> Result<(), String> {
    let mut actively_married_characters = BTreeSet::new();
    let mut active_wards = BTreeSet::new();
    let mut active_player_wards = 0_usize;
    for (link_id, link) in &state.family_links {
        validate_family_link_reference(state, *link_id, link)?;
        validate_marriage_link(state, *link_id, link, &mut actively_married_characters)?;
        validate_adoptive_or_ward_link(
            state,
            *link_id,
            link,
            &mut active_wards,
            &mut active_player_wards,
        )?;
        validate_parent_child_link(state, *link_id, link)?;
    }
    if active_player_wards > crate::systems::MAX_ACTIVE_WARDS {
        return Err(format!(
            "player dynasty has {active_player_wards} active wards, exceeding the supported maximum of {}",
            crate::systems::MAX_ACTIVE_WARDS
        ));
    }
    Ok(())
}

fn validate_family_link_reference(
    state: &AppState,
    link_id: crate::ids::FamilyLinkId,
    link: &crate::core::FamilyLink,
) -> Result<(), String> {
    if link.id != link_id
        || link.first_character_id == link.second_character_id
        || state.characters.get(link.first_character_id).is_none()
        || state.characters.get(link.second_character_id).is_none()
    {
        return Err(format!(
            "family link {link_id} has an invalid character reference"
        ));
    }
    Ok(())
}

fn validate_marriage_link(
    state: &AppState,
    link_id: crate::ids::FamilyLinkId,
    link: &crate::core::FamilyLink,
    actively_married_characters: &mut BTreeSet<crate::ids::CharacterId>,
) -> Result<(), String> {
    if !link.active || link.kind != FamilyLinkKind::Marriage {
        return Ok(());
    }
    if !actively_married_characters.insert(link.first_character_id)
        || !actively_married_characters.insert(link.second_character_id)
    {
        return Err(format!(
            "family link {link_id} gives a character multiple active marriages"
        ));
    }
    let first = state
        .characters
        .get(link.first_character_id)
        .expect("validated family link character must exist");
    let second = state
        .characters
        .get(link.second_character_id)
        .expect("validated family link character must exist");
    if first.status() != crate::core::CharacterStatus::Active
        || second.status() != crate::core::CharacterStatus::Active
    {
        return Err(format!(
            "family link {link_id} has an invalid active marriage lifecycle"
        ));
    }
    Ok(())
}

fn validate_adoptive_or_ward_link(
    state: &AppState,
    link_id: crate::ids::FamilyLinkId,
    link: &crate::core::FamilyLink,
    active_wards: &mut BTreeSet<crate::ids::CharacterId>,
    active_player_wards: &mut usize,
) -> Result<(), String> {
    if !matches!(link.kind, FamilyLinkKind::Ward) {
        return Ok(());
    }
    let first = state
        .characters
        .get(link.first_character_id)
        .expect("validated family link character must exist");
    let second = state
        .characters
        .get(link.second_character_id)
        .expect("validated family link character must exist");
    if first.dynasty_id() != second.dynasty_id() {
        return Err(format!(
            "family link {link_id} crosses dynasties for an adoptive or ward relationship"
        ));
    }
    if !link.active || link.kind != FamilyLinkKind::Ward {
        return Ok(());
    }
    if first.status() != crate::core::CharacterStatus::Active
        || second.status() != crate::core::CharacterStatus::Active
        || !state
            .family_councils
            .get(&second.dynasty_id())
            .is_some_and(|council| council.members.contains(&second.id()))
    {
        return Err(format!(
            "family link {link_id} has an invalid active ward lifecycle"
        ));
    }
    if !active_wards.insert(link.second_character_id) {
        return Err(format!(
            "family link {link_id} gives one character multiple active ward relationships"
        ));
    }
    if second.dynasty_id() == state.player_dynasty_id {
        *active_player_wards += 1;
    }
    Ok(())
}

fn validate_parent_child_link(
    state: &AppState,
    link_id: crate::ids::FamilyLinkId,
    link: &crate::core::FamilyLink,
) -> Result<(), String> {
    if link.kind != FamilyLinkKind::ParentChild {
        return Ok(());
    }
    let parent = state
        .characters
        .get(link.first_character_id)
        .expect("validated family link character must exist");
    let child = state
        .characters
        .get(link.second_character_id)
        .expect("validated family link character must exist");
    if child.birth_day().saturating_sub(parent.birth_day())
        < crate::core::MIN_PARENT_CHILD_AGE_GAP_DAYS
    {
        return Err(format!(
            "family link {link_id} has impossible parent-child chronology"
        ));
    }
    Ok(())
}

fn validate_family_councils(state: &AppState) -> Result<(), String> {
    if state.family_councils.len() != state.dynasties.len() {
        return Err("every dynasty must have exactly one family council".to_owned());
    }
    for (dynasty_id, council) in &state.family_councils {
        let dynasty = state.dynasties.get(dynasty_id);
        if council.dynasty_id != *dynasty_id
            || dynasty.is_none()
            || council.members.iter().any(|character_id| {
                !state
                    .characters
                    .get(*character_id)
                    .is_some_and(|character| {
                        character.dynasty_id() == *dynasty_id
                            && character.status() == crate::core::CharacterStatus::Active
                    })
            })
        {
            return Err(format!(
                "family council {dynasty_id} has an invalid reference"
            ));
        }
        let dynasty = dynasty.expect("validated council dynasty must exist");
        if !council.members.contains(&dynasty.head_id())
            || dynasty
                .heir_id()
                .is_some_and(|heir_id| !council.members.contains(&heir_id))
        {
            return Err(format!(
                "family council {dynasty_id} omits the dynasty head or heir"
            ));
        }
    }
    Ok(())
}

fn validate_institution_and_misc_records(state: &AppState) -> Result<(), String> {
    let mut officeholders = BTreeSet::new();
    let mut player_memberships = BTreeMap::new();
    for (institution_id, institution) in &state.institutions {
        let unsupported_player_member = institution.members.iter().any(|character_id| {
            state
                .characters
                .get(*character_id)
                .is_some_and(|character| {
                    character.dynasty_id() == state.player_dynasty_id
                        && institution.office_holder_id != Some(*character_id)
                })
                && !state.audit_log.iter().any(|record| {
                    record.kind() == AuditKind::InstitutionPatronage
                        && record
                            .audit_subject()
                            .references_institution_character(*institution_id, *character_id)
                })
        });
        if institution.institution_id != *institution_id
            || institution.members.iter().any(|character_id| {
                !state
                    .characters
                    .get(*character_id)
                    .is_some_and(|character| {
                        character.status() == crate::core::CharacterStatus::Active
                    })
            })
            || institution
                .office_holder_id
                .is_some_and(|holder| !institution.members.contains(&holder))
            || unsupported_player_member
        {
            return Err(format!(
                "institution {institution_id} has inconsistent runtime state"
            ));
        }
        if institution
            .office_holder_id
            .is_some_and(|holder_id| !officeholders.insert(holder_id))
        {
            return Err(format!(
                "institution {institution_id} duplicates an existing officeholder"
            ));
        }
        for character_id in &institution.members {
            if state
                .characters
                .get(*character_id)
                .is_some_and(|character| character.dynasty_id() == state.player_dynasty_id)
            {
                *player_memberships.entry(*character_id).or_insert(0_usize) += 1;
            }
        }
    }
    if let Some((character_id, memberships)) = player_memberships.iter().find(|(_, memberships)| {
        **memberships > crate::systems::MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER
    }) {
        return Err(format!(
            "player character {character_id} belongs to {memberships} institutions, exceeding the supported maximum of {}",
            crate::systems::MAX_INSTITUTION_MEMBERSHIPS_PER_CHARACTER
        ));
    }
    for (pair, relationship) in &state.relationships {
        if relationship.pair != *pair
            || pair.first == pair.second
            || !state.dynasties.contains_key(&pair.first)
            || !state.dynasties.contains_key(&pair.second)
            || relationship.last_interaction_day > state.clock.day()
            || relationship.memories.len() > crate::systems::MAX_RELATIONSHIP_MEMORIES
            || relationship
                .memories
                .iter()
                .any(|memory| memory.trim().is_empty())
        {
            return Err("relationship map contains an invalid dynasty pair".to_owned());
        }
    }
    let dynasty_ids: Vec<_> = state.dynasties.keys().copied().collect();
    for (index, left_dynasty_id) in dynasty_ids.iter().enumerate() {
        for right_dynasty_id in dynasty_ids.iter().skip(index + 1) {
            let pair = crate::core::DynastyPair::new(*left_dynasty_id, *right_dynasty_id);
            if !state.relationships.contains_key(&pair) {
                return Err(format!(
                    "relationship map is missing dynasty pair {left_dynasty_id}/{right_dynasty_id}"
                ));
            }
        }
    }
    validate_misc_record_ids_and_refs(state)
}

fn validate_misc_record_ids_and_refs(state: &AppState) -> Result<(), String> {
    validate_law_report_and_objective_records(state)?;
    validate_civic_event_records(state)?;
    validate_persisted_history(state)
}

fn validate_law_report_and_objective_records(state: &AppState) -> Result<(), String> {
    for (law_id, law) in &state.laws {
        if law.id != *law_id
            || !law.kind.is_value_valid(law.value)
            || law.enacted_day > state.clock.day()
            || law
                .sponsor_dynasty_id
                .is_some_and(|dynasty_id| !state.dynasties.contains_key(&dynasty_id))
        {
            return Err(format!("law {law_id} has an invalid identity reference"));
        }
        if law.active
            && state
                .laws
                .values()
                .any(|other| other.id != law.id && other.active && other.kind == law.kind)
        {
            return Err(format!(
                "law kind {:?} has multiple active records",
                law.kind
            ));
        }
        if law.active && !law.kind.remains_active_after_enactment() {
            return Err(format!(
                "active law kind {:?} is a consumed one-time authorization",
                law.kind
            ));
        }
    }
    for (report_id, report) in &state.information_reports {
        if report.id != *report_id
            || !state.dynasties.contains_key(&report.owner_dynasty_id)
            || !crate::systems::is_valid_information_report_dates(
                state.clock.day(),
                report.created_day,
                report.expires_day,
            )
            || report.subject.trim().is_empty()
            || report.source.trim().is_empty()
            || report.summary.trim().is_empty()
        {
            return Err(format!(
                "information report {report_id} has an invalid reference"
            ));
        }
        let target_exists = report.target.is_none_or(|target| match target {
            InformationTarget::Market { good_id } => state.market.quotes.contains_key(&good_id),
            InformationTarget::Counterparty { dynasty_id } => {
                state.dynasties.contains_key(&dynasty_id)
            }
            InformationTarget::District { district_id } => {
                state.districts.contains_key(&district_id)
            }
        });
        if !target_exists {
            return Err(format!(
                "information report {report_id} targets a missing record"
            ));
        }
    }
    validate_ai_objective_records(state)
}

fn validate_ai_objective_records(state: &AppState) -> Result<(), String> {
    let mut pursuing_objectives = BTreeMap::<crate::ids::DynastyId, usize>::new();
    for (objective_id, objective) in &state.ai_objectives {
        if objective.id != *objective_id
            || !state.dynasties.contains_key(&objective.dynasty_id)
            || objective.dynasty_id == state.player_dynasty_id
            || objective.created_day > state.clock.day()
            || objective
                .target_dynasty_id
                .is_some_and(|dynasty_id| !state.dynasties.contains_key(&dynasty_id))
            || objective.target_dynasty_id == Some(objective.dynasty_id)
        {
            return Err(format!(
                "AI objective {objective_id} has an invalid reference"
            ));
        }
        if objective.rationale.trim().is_empty() {
            return Err(format!("AI objective {objective_id} has no rationale"));
        }
        if objective.status == crate::core::ObjectiveStatus::Planned {
            return Err(format!(
                "AI objective {objective_id} uses an unsupported planned lifecycle state"
            ));
        }
        if objective.status == crate::core::ObjectiveStatus::Pursuing {
            let count = pursuing_objectives.entry(objective.dynasty_id).or_default();
            *count += 1;
            if *count > 1 {
                return Err(format!(
                    "dynasty {} has multiple pursuing AI objectives",
                    objective.dynasty_id
                ));
            }
        }
    }
    for dynasty_id in state
        .dynasties
        .keys()
        .copied()
        .filter(|dynasty_id| *dynasty_id != state.player_dynasty_id)
    {
        if pursuing_objectives.get(&dynasty_id).copied() != Some(1) {
            return Err(format!(
                "dynasty {dynasty_id} does not have exactly one pursuing AI objective"
            ));
        }
    }
    Ok(())
}

fn validate_civic_event_records(state: &AppState) -> Result<(), String> {
    let mut active_public_works = BTreeSet::new();
    let mut active_player_sponsored_works = 0_usize;
    for (work_id, work) in &state.public_works {
        if work.id != *work_id
            || !state.districts.contains_key(&work.district_id)
            || work
                .sponsor_dynasty_id
                .is_some_and(|dynasty_id| !state.dynasties.contains_key(&dynasty_id))
        {
            return Err(format!("public work {work_id} has an invalid reference"));
        }
        if work.status.is_unfinished() {
            if !active_public_works.insert((work.district_id, work.kind)) {
                return Err(format!(
                    "public work {work_id} duplicates an unfinished project of the same kind in district {}",
                    work.district_id
                ));
            }
            if work.sponsor_dynasty_id == Some(state.player_dynasty_id) {
                active_player_sponsored_works += 1;
            }
        }
    }
    if active_player_sponsored_works > crate::systems::MAX_ACTIVE_SPONSORED_PUBLIC_WORKS {
        return Err(format!(
            "player dynasty sponsors {active_player_sponsored_works} unfinished public works, exceeding the supported maximum of {}",
            crate::systems::MAX_ACTIVE_SPONSORED_PUBLIC_WORKS
        ));
    }
    validate_legal_case_records(state)?;
    for (route_id, route) in &state.external_routes {
        if route.id != *route_id
            || route.name.trim().is_empty()
            || !state.market.quotes.contains_key(&route.good_id)
        {
            return Err(format!(
                "external route {route_id} has an invalid reference"
            ));
        }
    }
    for (crisis_id, crisis) in &state.crises {
        if crisis.id != *crisis_id
            || crisis.started_day > state.clock.day()
            || crisis.cause.trim().is_empty()
            || crisis
                .district_id
                .is_some_and(|district_id| !state.districts.contains_key(&district_id))
            || !crisis
                .status
                .has_consistent_severity(crisis.severity_basis_points)
        {
            return Err(format!("crisis {crisis_id} has an invalid reference"));
        }
    }
    Ok(())
}

fn validate_legal_case_records(state: &AppState) -> Result<(), String> {
    let mut active_cases = BTreeSet::new();
    let mut litigated_loans = BTreeSet::new();
    let mut litigated_contracts = BTreeSet::new();
    for (case_id, legal_case) in &state.legal_cases {
        if legal_case.id != *case_id
            || legal_case.plaintiff_dynasty_id == legal_case.defendant_dynasty_id
            || !state
                .dynasties
                .contains_key(&legal_case.plaintiff_dynasty_id)
            || !state
                .dynasties
                .contains_key(&legal_case.defendant_dynasty_id)
            || legal_case.filed_day > state.clock.day()
            || legal_case.hearing_day < legal_case.filed_day
            || (matches!(
                legal_case.status,
                crate::core::LegalCaseStatus::DecidedForPlaintiff
                    | crate::core::LegalCaseStatus::DecidedForDefendant
            ) && legal_case.hearing_day > state.clock.day())
        {
            return Err(format!("legal case {case_id} has an invalid reference"));
        }
        if let Some(claim_source) = legal_case.claim_source {
            let first_use = match claim_source {
                LegalClaimSource::Loan { loan_id } => litigated_loans.insert(loan_id),
                LegalClaimSource::Contract { contract_id } => {
                    litigated_contracts.insert(contract_id)
                }
            };
            if !first_use {
                return Err(format!(
                    "legal case {case_id} reuses a grounded claim source that was already litigated"
                ));
            }
        }
        validate_legal_claim_source(state, *case_id, legal_case)?;
        if matches!(
            legal_case.status,
            crate::core::LegalCaseStatus::Filed | crate::core::LegalCaseStatus::Hearing
        ) && !active_cases.insert((
            legal_case.plaintiff_dynasty_id,
            legal_case.defendant_dynasty_id,
            legal_case.kind,
        )) {
            return Err(format!(
                "legal case {case_id} duplicates an unresolved case between the same parties"
            ));
        }
    }
    Ok(())
}

fn validate_legal_claim_source(
    state: &AppState,
    case_id: crate::ids::LegalCaseId,
    legal_case: &crate::core::LegalCase,
) -> Result<(), String> {
    let Some(claim_source) = legal_case.claim_source else {
        return Ok(());
    };
    match claim_source {
        LegalClaimSource::Loan { loan_id } => {
            let loan = state
                .loans
                .get(&loan_id)
                .ok_or_else(|| format!("legal case {case_id} references missing loan {loan_id}"))?;
            if legal_case.kind != LegalCaseKind::Debt
                || loan.lender_dynasty_id != legal_case.plaintiff_dynasty_id
                || loan.borrower_dynasty_id != legal_case.defendant_dynasty_id
            {
                return Err(format!(
                    "legal case {case_id} has a debt claim source that does not match its loan and parties"
                ));
            }
        }
        LegalClaimSource::Contract { contract_id } => {
            let contract = state.contracts.get(&contract_id).ok_or_else(|| {
                format!("legal case {case_id} references missing contract {contract_id}")
            })?;
            // Recoverable breach debt grounds on the attributed parties from
            // the first attributable miss, which exists while the contract is
            // still Active (breach status needs three misses). This mirrors
            // the runtime invariant; requiring `Breached` here would reject
            // saves the simulation itself produces.
            if legal_case.kind != LegalCaseKind::ContractBreach
                || contract.breaching_dynasty_id != Some(legal_case.defendant_dynasty_id)
                || contract.breach_victim_dynasty_id != Some(legal_case.plaintiff_dynasty_id)
            {
                return Err(format!(
                    "legal case {case_id} has a contract-breach claim source that does not match its contract and parties"
                ));
            }
        }
    }
    Ok(())
}

fn validate_persisted_history(state: &AppState) -> Result<(), String> {
    let mut outbox_ids = BTreeSet::new();
    let mut prior_outbox_id = None;
    let mut prior_outbox_day = i64::MIN;
    for message in &state.outbox {
        if !outbox_ids.insert(message.id) {
            return Err("outbox contains duplicate message IDs".to_owned());
        }
        if prior_outbox_id.is_some_and(|prior_id| message.id <= prior_id) {
            return Err("outbox message IDs are not strictly increasing".to_owned());
        }
        if message.day < prior_outbox_day || message.day > state.clock.day() {
            return Err("outbox messages are not chronologically valid".to_owned());
        }
        if message.subject.trim().is_empty() || message.body.trim().is_empty() {
            return Err("outbox message lacks user-facing content".to_owned());
        }
        prior_outbox_id = Some(message.id);
        prior_outbox_day = message.day;
    }

    let mut chronicle_ids = BTreeSet::new();
    let mut prior_chronicle_day = i64::MIN;
    for entry in &state.chronicle {
        if !chronicle_ids.insert(entry.id()) {
            return Err("chronicle contains duplicate entry IDs".to_owned());
        }
        if entry.day() < prior_chronicle_day || entry.day() > state.clock.day() {
            return Err("chronicle entries are not chronologically valid".to_owned());
        }
        if entry.summary().trim().is_empty() {
            return Err("chronicle entry lacks user-facing content".to_owned());
        }
        prior_chronicle_day = entry.day();
    }
    let mut prior_audit_day = i64::MIN;
    for record in &state.audit_log {
        if record.day() < prior_audit_day || record.day() > state.clock.day() {
            return Err("audit log is not chronologically valid".to_owned());
        }
        if record.subject().trim().is_empty() || record.detail().trim().is_empty() {
            return Err("audit record lacks diagnostic content".to_owned());
        }
        validate_audit_record_references(state, record)?;
        prior_audit_day = record.day();
    }
    Ok(())
}

fn validate_audit_record_references(
    state: &AppState,
    record: &crate::core::AuditRecord,
) -> Result<(), String> {
    if matches!(
        record.kind(),
        AuditKind::InstitutionPatronage
            | AuditKind::InstitutionWithdrawal
            | AuditKind::OfficeNomination
    ) {
        let Some((institution_id, character_id)) =
            record.audit_subject().institution_character_ids()
        else {
            return Err(format!(
                "{:?} audit record has an invalid institution/character subject",
                record.kind()
            ));
        };
        if !state.institutions.contains_key(&institution_id)
            || state.characters.get(character_id).is_none()
        {
            return Err(format!(
                "{:?} audit record references a missing institution or character",
                record.kind()
            ));
        }
    }
    if matches!(
        record.kind(),
        AuditKind::OfficeDirective | AuditKind::InstitutionEndowment
    ) {
        let Some(institution_id) = record.audit_subject().institution_id() else {
            return Err(format!(
                "{:?} audit record has an invalid institution subject",
                record.kind()
            ));
        };
        if !state.institutions.contains_key(&institution_id) {
            return Err(format!(
                "{:?} audit record references missing institution {institution_id}",
                record.kind()
            ));
        }
    }
    if record.kind() == AuditKind::OfficeDirective {
        validate_office_directive_audit_reference(state, record)?;
    }
    if record.kind() == AuditKind::InstitutionEndowment {
        validate_institution_endowment_audit_reference(state, record)?;
    }
    if matches!(
        record.kind(),
        AuditKind::OfficeDutyShortfall | AuditKind::OfficeDutyForfeiture
    ) {
        validate_office_duty_audit_reference(state, record)?;
    }
    Ok(())
}

fn validate_office_directive_audit_reference(
    state: &AppState,
    record: &crate::core::AuditRecord,
) -> Result<(), String> {
    let subject = record.audit_subject();
    let institution_id = subject
        .institution_id()
        .expect("office directive institution subject was validated above");
    let Some(dynasty_id) = subject.dynasty_id() else {
        return Err("OfficeDirective audit record lacks dynasty attribution".to_owned());
    };
    if !state.dynasties.contains_key(&dynasty_id) {
        return Err(format!(
            "OfficeDirective audit record references missing dynasty {dynasty_id}"
        ));
    }
    if subject.as_str() != format!("institution:{institution_id};dynasty:{dynasty_id}") {
        return Err("OfficeDirective audit record has an invalid dynasty attribution".to_owned());
    }
    Ok(())
}

fn validate_institution_endowment_audit_reference(
    state: &AppState,
    record: &crate::core::AuditRecord,
) -> Result<(), String> {
    let subject = record.audit_subject();
    let institution_id = subject
        .institution_id()
        .expect("institution endowment subject was validated above");
    let Some(dynasty_id) = subject.dynasty_id() else {
        return Err("InstitutionEndowment audit record lacks dynasty attribution".to_owned());
    };
    if !state.dynasties.contains_key(&dynasty_id) {
        return Err(format!(
            "InstitutionEndowment audit record references missing dynasty {dynasty_id}"
        ));
    }
    if subject.as_str() != format!("institution:{institution_id};dynasty:{dynasty_id}") {
        return Err(
            "InstitutionEndowment audit record has an invalid dynasty attribution".to_owned(),
        );
    }
    Ok(())
}

fn validate_office_duty_audit_reference(
    state: &AppState,
    record: &crate::core::AuditRecord,
) -> Result<(), String> {
    let subject = record.audit_subject();
    let Some(institution_id) = subject.institution_id() else {
        return Err(format!(
            "{:?} audit record has an invalid institution subject",
            record.kind()
        ));
    };
    let Some(dynasty_id) = subject.dynasty_id() else {
        return Err(format!(
            "{:?} audit record has an invalid dynasty subject",
            record.kind()
        ));
    };
    if !state.institutions.contains_key(&institution_id)
        || !state.dynasties.contains_key(&dynasty_id)
    {
        return Err(format!(
            "{:?} audit record references a missing institution or dynasty",
            record.kind()
        ));
    }
    if subject.as_str() != format!("institution:{institution_id};dynasty:{dynasty_id}") {
        return Err(format!(
            "{:?} audit record has an invalid office-duty subject shape",
            record.kind()
        ));
    }
    Ok(())
}

/// The only member the loader must inspect before a full deserialize.
///
/// Deserializing this probe skips every other member without building the
/// intermediate value tree, so loading does not parse the whole document
/// twice just to learn its schema version. Unknown members are deliberately
/// allowed; duplicates were already rejected before this runs.
#[derive(Deserialize)]
struct SchemaVersionProbe {
    schema_version: u64,
}

fn require_current_schema(bytes: &[u8], path: &Path) -> Result<(), PersistenceError> {
    match serde_json::from_slice::<SchemaVersionProbe>(bytes) {
        Ok(probe) => {
            if probe.schema_version != u64::from(CURRENT_SCHEMA_VERSION) {
                return Err(PersistenceError::UnsupportedSchemaVersion {
                    path: path.to_path_buf(),
                    found: probe.schema_version,
                    supported: CURRENT_SCHEMA_VERSION,
                });
            }
            Ok(())
        }
        // A well-formed document without a usable `schema_version` member
        // (missing, wrong type, non-object root) is reported exactly as the
        // previous value-tree lookup reported it; only genuine syntax
        // failures surface as parse errors.
        Err(error) if error.is_data() => Err(PersistenceError::MissingSchemaVersion {
            path: path.to_path_buf(),
        }),
        Err(source) => Err(PersistenceError::Parse {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
