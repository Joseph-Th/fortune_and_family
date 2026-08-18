# Tasks

Current executable work that may be picked up asynchronously. Keep entries short; remove completed work. `STATUS.md`/`CAPABILITIES.md` owns implemented truth and `ROADMAP.md`/`DIRECTION.md` owns future direction.

## T-0001 - Reject duplicate save JSON members

- Area: persistence-admission
- Next: Make Civic Dynasty's current-schema save admission duplicate-aware before serde_json::Value/BTreeMap deserialization can collapse repeated object members. Add one raw JSON admission step (or equivalent collection-aware mechanism) that rejects duplicate members at any nested authoritative path, then preserve the existing schema-version check, typed AppState decode, release invariant validation, bounded regular-file read, deterministic continuation, and atomic-write behavior. Add raw-text tests for a duplicate top-level field and duplicate nested map key with path-specific diagnostics, plus regression coverage showing ordinary unknown/missing/current-schema errors remain unchanged. Keep this focused on authoritative save JSON; CLI PlayerCommand JSON can adopt the same helper later if its schema contains consequential map-valued inputs.
- Paths: `src/persistence.rs`, `src/persistence_tests.rs`
- Verify: `bash scripts/test.sh fast persistence && bash scripts/test.sh standard`
- Depends: none
- Basis: `8394aca439c6a71d42be24e9d8428e6dcae8fb1d`
- Reviewed: `2026-08-18T07:14:47Z`

## T-0002 - Distinguish save commit from durability

- Area: persistence-publication
- Next: Refine save_state so tempfile persist/replacement is the visibility commit point and a later Unix parent-directory sync failure is represented as committed-with-durability-degradation rather than PersistenceError::SyncDirectory implying the old save is still authoritative. Preserve current invariant validation, unique same-directory staging, strict JSON admission work from T-0001, and all pre-commit failures. Add deterministic injection around the parent-sync boundary and tests proving callers do not retry or report an already-visible save as uncommitted.
- Paths: `src/persistence.rs`
- Verify: `bash scripts/test.sh fast persistence && bash scripts/test.sh standard`
- Depends: none
- Basis: `9c3b50cbc27bf92e36e9ec983df7225188e70540`
- Reviewed: `2026-08-18T07:37:08Z`

## T-0003 - Bind campaign saves to authored registry identity

- Area: persistence-compatibility
- Next: Bind every Civic Dynasty save to the exact behavior-relevant Rivergate Registry interpretation that determined the campaign. AppState persists CURRENT_SCHEMA_VERSION and scenario_key, and load validation rebuilds the current Rivergate registry and checks references, but registry definitions also determine market prices/targets, recipes, districts, institutions, costs, capacities, and strategic behavior. Changed authored values under the same IDs and scenario key can therefore silently reinterpret a current-schema save. Add a deterministic canonical registry fingerprint in stable typed-ID order, persist it in the current save contract, reject mismatches before returning AppState, and bump CURRENT_SCHEMA_VERSION because the durable shape changes. Add identical-registry round-trip and deterministic-continuation tests, same-ID/value-change rejection, insertion-order-independent fingerprinting, and clear mismatch diagnostics. Coordinate with T-0001's strict raw admission and T-0002's publication semantics without coupling compatibility to either failure mode.
- Paths: `src/registry/mod.rs`
- Verify: `bash scripts/test.sh fast persistence && bash scripts/test.sh standard`
- Depends: T-0001
- Basis: `9c3b50cbc27bf92e36e9ec983df7225188e70540`
- Reviewed: `2026-08-18T08:00:18Z`

## T-0004 - Type generated-report post-commit degradation

- Area: artifact-publication
- Next: Apply the same truthful visibility-commit semantics required for campaign saves to the separate generated-output publisher used by playtest reports and art review exports. write_generated_output stages and syncs bytes, persists/replaces the destination, then on Unix synchronizes the parent directory; a parent-sync failure after persist currently surfaces as CliError::GameplayReportWrite/ArtReviewWrite even though the new artifact is already visible. Return a structured committed-with-durability-degradation outcome or equivalent typed result after the visibility commit, preserve all pre-commit errors and non-regular-destination rejection, and make CLI messaging/retry guidance reflect whether publication committed. Add deterministic injected post-commit sync-failure coverage for both report and art call sites. src/main.rs is currently under unrelated active work, so this task is anchored to the clean art-report owner and must be refreshed against main.rs before implementation.
- Paths: `src/art/harness.rs`
- Verify: `bash scripts/test.sh art-cli && bash scripts/test.sh gameplay-cli && bash scripts/test.sh standard`
- Depends: T-0002
- Basis: `9c3b50cbc27bf92e36e9ec983df7225188e70540`
- Reviewed: `2026-08-18T08:00:27Z`

## T-0005 - Reject dashboard source/output aliasing

- Area: operator-path-safety
- Next: Reject destructive path aliasing for the dashboard transform before generated-output publication. The Dashboard command loads an authoritative campaign JSON and then writes self-contained HTML to an independently named output; if input and output identify the same filesystem path through exact or equivalent relative spelling, the completed HTML atomically replaces the campaign source. Keep intentional in-place Simulate/Execute save semantics unchanged. Add a filesystem-aware path-equivalence check at the dashboard CLI boundary before ensure_output_parent/write_generated_output, reject exact/canonical/lexical aliases with a clear typed CLI error, and prove the source campaign bytes remain unchanged. Add CLI regression coverage for exact alias, relative-spelling alias, and distinct output success. src/main.rs currently has unrelated active edits, so anchor this task to the clean projection test owner and refresh against main.rs before implementation. Coordinate with T-0004 because both touch generated-output behavior, but alias rejection must happen before any staging or side effects.
- Paths: `src/projection_tests.rs`
- Verify: `bash scripts/test.sh cli && bash scripts/test.sh standard`
- Depends: T-0004
- Basis: `9c3b50cbc27bf92e36e9ec983df7225188e70540`
- Reviewed: `2026-08-18T08:08:40Z`

## T-0006 - Enforce campaign writer ownership

- Area: persistence-concurrency
- Next: Define and enforce an explicit writer-ownership model for user campaign files instead of treating atomic replacement as protection against lost updates or clobbering. AppState has no mutation/file revision and save_state unconditionally replaces the destination; two in-place Simulate/Execute processes can load the same campaign and the last publisher silently discards the other accepted mutation, while New can replace an existing campaign at its default or requested output without an explicit overwrite decision. Add a cross-process-safe ownership/CAS protocol for in-place mutations that covers the read-through-commit interval and rejects a stale source before publication, with a typed conflict containing the expected/current file identity or revision. Make New fail closed on an existing user campaign unless an explicit documented overwrite mode is chosen. Keep low-level atomic staging/durability behavior from T-0002, strict admission from T-0001, and source/output transform safety from T-0005. Add deterministic two-writer tests proving no lost update, stale-writer rejection with the winner preserved, lock/claim cleanup after failures, and no-clobber New behavior.
- Paths: `src/persistence.rs`, `src/persistence_tests.rs`
- Verify: `bash scripts/test.sh cli && bash scripts/test.sh fast persistence && bash scripts/test.sh standard`
- Depends: T-0002
- Basis: `9c3b50cbc27bf92e36e9ec983df7225188e70540`
- Reviewed: `2026-08-18T08:09:23Z`

## T-0007 - Preserve uncertainty under bounded gameplay probes

- Area: gameplay-evidence-uncertainty
- Next: Make bounded candidate probing preserve the distinction between validated unavailability and unverified candidates. A decision cycle generates/ranks the full candidate set, then select_probe_candidates probes at most max_candidate_probes. record_quiet_diagnostic currently marks a command kind as rejected by validation whenever none of that kind's probed representatives were viable, even when additional variants of the same kind remain unprobed; a no-action cycle can therefore turn budget-limited search into a false family-level blocked/unavailable claim. Track generated, probed, unprobed, viable, and rejected counts per command kind (or equivalent explicit coverage state), classify a family as validation-blocked only when every relevant generated candidate was actually probed and rejected, and report remaining families/options as unverified due to the probe budget. Ensure choice/blocked-cycle metrics and human/JSON findings do not count unprobed options as negative evidence. Add low-probe-budget regressions where an early representative rejects but an unprobed same-family variant is viable, plus exhaustive-budget equivalence. gameplay.rs/gameplay_tests.rs are currently under unrelated active work, so anchor this task to the clean gameplay report checker and refresh against those owners before implementation.
- Paths: `scripts/check_gameplay.py`
- Verify: `bash scripts/test.sh fast gameplay && bash scripts/test.sh gameplay && bash scripts/test.sh standard`
- Depends: none
- Basis: `9c3b50cbc27bf92e36e9ec983df7225188e70540`
- Reviewed: `2026-08-18T08:10:00Z`
