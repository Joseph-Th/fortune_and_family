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
