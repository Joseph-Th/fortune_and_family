# AGENTS.md -- Coding Design Law and Architecture

Read before changing code. Current work is in STATUS.md.

## Coding Design Law

A deterministic, explicit, maintainable codebase in Rust.

| Constraint | Rule |
|---|---|
| Systems first | Behavior emerges from system functions operating on explicit state, not scripts, callbacks, or special cases. |
| Explicit records | Core records are concrete structs with clear ownership. Avoid hidden framework behavior and hidden mutation paths. |
| Domain-neutral rules | Architecture rules apply across subsystems. Do not encode one feature's assumptions as global design law. |

### Program Model

Registry / AppState / Record / System split:

- **Registries** hold immutable definitions and lookup tables loaded once at startup.
- **AppState** holds generated mutable runtime state that must survive execution and restart boundaries.
- **Records** hold identity, local data, and lifecycle state. They do not own business logic.
- **Systems** perform validation, derive outcomes, mutate state, and enforce invariants through canonical paths.

### Data Ownership

- **Registry** -- immutable static definitions loaded once: schemas, templates, policies, capabilities, operation definitions, validation rules, type metadata, lookup tables, and registry-owned defaults. Defined in Rust builders under `src/content/`, `src/registry/`, or the owning subsystem -- never TOML/JSON/YAML unless the file is explicitly external operator configuration.
- **AppState** -- generated mutable application state: runtime records, indexes, relationships, scheduled work, derived summaries, transaction history, generated IDs, cacheable projections, and persistent runtime configuration.
- **Records** -- individual runtime state: identity, ownership, lifecycle, references, local properties, timestamps, version counters, status, and subsystem-owned payloads.
- **Indexes** -- derived lookup structures owned by the module responsible for keeping them synchronized. Indexes are never independent sources of truth.
- **External resources** -- files, network handles, database connections, processes, and service clients live behind adapter boundaries. Core systems receive explicit data and return explicit outcomes.

### System Rules

**State mutation.** Mutate consequential state only through system functions. A caller may request an operation, but the owning system validates, resolves, commits, logs, and preserves indexes. Do not let callers directly patch fields that affect invariants.

**Canonical pipelines.** Every operation class has one path: input -> validation -> resolution -> plan/outcome -> atomic commit -> side effects at explicit boundaries -> invariant validation. Do not add parallel shortcuts for tests, UI, migrations, importers, or administrative tools.

**Validation.** Validate every cross-reference, permission, lifecycle state, range, ownership claim, and capacity constraint before mutation. Fallible multi-resource operations use validated tokens and commit exactly once.

**Determinism.** Same seed, state, inputs, and external snapshots produce the same result. All randomness comes from `state.rng` or an explicitly injected RNG owned by the state. Sort order-dependent inputs before making choices. Stable tie-breaking is mandatory.

**Definitions versus runtime state.** Static definitions describe what can exist. Runtime state records what does exist. Do not store mutable runtime values in registry definitions. Any generated value that must survive restart is serializable and owned by `AppState` or a record.

**Identity.** Use typed IDs for persistent references. Raw strings are allowed only for genuinely authored external identifiers or user-facing text. Internal IDs use newtypes with the narrowest sufficient backing type.

**References.** Validate all registry and record references at load, import, migration, or operation-validation time. Missing required IDs fail immediately with the offending ID. Optional references are explicit in the type.

**Derived data.** Derived values are either recomputed on demand or owned by one synchronizing module. If two collections must agree, they are private fields of one owner and updated by one atomic method.

**Side effects.** Core systems do not perform implicit IO. They return outcomes or commands that adapters execute at explicit boundaries. Side effects that must be durable are represented in state before external execution.

**UI and adapters.** UI, CLI, API, storage, and network layers translate input and output. They do not own business rules, invariants, or mutation semantics.

**Persistence.** Save/load preserves IDs, relationships, lifecycle state, version counters, generated definitions, and all runtime data needed to resume without recomputation changing behavior. Migrations are explicit and tested.

**Errors.** New fallible operations return dedicated error enums. Do not add `Result<_, String>` or `Result<_, &'static str>`. Error variants include enough context to identify the failed precondition without parsing text.

### Runtime Invariants

1. **Registry Reference Validity** -- every required registry reference resolves before runtime use.
2. **Record Reference Validity** -- every stored record ID resolves to an existing record unless the field is explicitly optional.
3. **Index Completeness** -- every record that should appear in an index appears in that index.
4. **Index Uniqueness** -- every indexed record appears at most once per index key unless the index explicitly supports duplicates.
5. **Ownership Exclusivity** -- a record with exclusive ownership belongs to exactly one owner, container, parent, or root collection.
6. **Lifecycle Validity** -- inactive, removed, failed, or completed records cannot be scheduled, mutated, or exposed as active unless reactivation is an explicit operation.
7. **Transaction Atomicity** -- multi-record operations either commit all intended changes or commit none.
8. **No Lost Runtime State** -- no generated record, command, event, ID, external handle, or durable outcome is created without an owner, location, registry reference, or persistence path.
9. **Deterministic Decision Ordering** -- selection among valid choices uses deterministic scoring and stable tie-breaking.
10. **Definition/Runtime Separation** -- static definitions contain no mutable runtime state, and runtime records do not duplicate immutable definitions except as validated references.
11. **Serialization Completeness** -- save/load round-trips preserve all state required for deterministic continuation.
12. **Derived Data Consistency** -- cached projections, summaries, counters, and indexes match their source records.
13. **External Boundary Explicitness** -- IO effects are represented as explicit adapter calls, commands, or durable outcomes. Core systems do not hide external mutation.
14. **Error State Consistency** -- failed operations leave state unchanged except for explicitly documented diagnostics or audit records.

Enforce via `validate_invariants(state)` (see Invariant Enforcement) plus the soak test. New invariant -> add its assertion in the same change.

### Core Constraints

| Constraint | Rule |
|---|---|
| Explicit state flow | Mutable runtime state flows through `AppState` or the owning state type. No global mutable state. |
| Determinism | Same seed + state + inputs produce the same result. All randomness comes from state-owned RNG. |
| Synchronous execution | Consequences resolve in the same call stack. No deferred queues unless the queue is an explicit state record with defined processing semantics. |
| Data-driven definitions | Static definitions live in code-owned registries/builders, not runtime logic. Runtime state never lives in a registry def; generated state that must survive restart is serializable. |
| Unified pipelines | One canonical path per operation type. No parallel special cases. Callers never bypass it to mutate consequential state directly. |
| Logic in systems | Business logic in system functions, not record methods. |
| Compiler-enforced invariants | Make invalid states unrepresentable or fail at compile time. |

### Architecture Rules

- Use explicit data structures with clear ownership. Avoid dynamic dispatch on core domain records without a measured need.
- Fields driving consequential behavior are private, exposed via read-only getters, mutated only through system functions.
- No `_ =>` arms on project-owned enums. Match every variant. Exception: third-party/primitive types.
- When mapping struct to struct, destructure explicitly (`let MyStruct { a, b, c } = source;`). No `..` unless ignored fields are named in a comment explaining why.
- Group structs past ~10 fields into nested profiles (`identity`, `configuration`, `relationships`, `runtime`, `transient`). Top-level destructuring names every profile; no `..`.
- Collections that must stay synchronized are private fields of one struct, in their own module. Expose only atomic methods (`insert`, `remove`, `move_to`, `reassign`) that update every backing collection in one call. A `pub` backing collection does not satisfy this.
- Prefer `match` over `HashMap<String, Box<dyn Trait>>` for project-owned behavior.
- Validate all cross-references at load time. Missing IDs panic immediately with the offending ID.
- Pass the narrowest context a phase needs. Read-only phases take immutable refs; mutation phases take only the mutable access they need.
- Static registries own immutable definitions; application state owns generated mutable state; records own their own runtime state (see Data Ownership).

### Naming Conventions

One vocabulary project-wide, so agents can predict a name without reading the file first.

- **Lookups.** `get_*` for a direct/keyed lookup (map, registry, slot). `find_*` for a search that scans or applies a condition. `resolve_*` for deriving a final value from a template or set of inputs (e.g. `resolve_template`, `resolve_policy`). A plain field accessor with no search or derivation is a bare noun method (`fn status(&self)`), never `get_status`.
- **Construction.** `new()` for a plain struct constructor. `build_*` for procedural or aggregate assembly that is not a single record (`build_registry`, `build_projection`). `insert_*` for adding an already-constructed value to an owned collection. `register_*` for adding a definition, handler, or capability to a registry. Do not add new `create_*`/`make_*` functions outside `#[cfg(test)]` fixtures unless an external API contract requires that exact name.
- **Removal.** `remove_*` is canonical for taking something out of a collection or state owner. `destroy_*` only when destruction fires consequential effects. Do not use `delete_*` except for literal file/save deletion or external APIs that use that term.
- **Booleans.** Predicates are always prefixed `is_`, `has_`, or `can_`. A function returning `bool` with a bare noun name is a naming bug, not a style choice.
- **Calculate Then Apply pairs.** The decision half is `decide_*`, returning a `*Plan`/`*Outcome`/`*Delta`. The mutation half is `apply_*`. Do not introduce `execute_*`/`perform_*`/`attempt_*` for this role in new code; where they already exist, migrate to `decide_*`/`apply_*` opportunistically when touching that code for other reasons.
- **Check Then Mutate pairs.** `validate_*` returns a `Validated*` token; `token.commit(...)` is the only mutation path (see below). New fallible multi-step operations return a dedicated error enum. Existing stringly-typed `Result`s are legacy debt (see STATUS.md); do not add more of them.
- **ID newtypes.** Wrap the narrowest sufficient type: `u32` for small generated-state registries, a slotmap key for dense runtime records, `String` only when the ID is genuinely content-authored or external. Every ID newtype derives at minimum `Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize`, plus `Copy, PartialOrd, Ord` whenever the backing type supports them.
- **Multi-file subsystems.** Shared suffix vocabulary: `_execution` (mutates through the canonical pipeline), `_integration` (wires the subsystem into other systems), `_loader` (builds the registry from Rust builders at startup), `_ui` (user-facing rendering/dispatch), `_adapter` (external IO boundary). A subsystem may add extra suffixes for its own concerns, but must explain the split in that subsystem's `//!` module doc.
- **Module docs.** Every file under `src/` has a `//!` header: one sentence on the file's purpose, and for multi-file subsystems, one clause on how it relates to its sibling files.

### Calculate Then Apply

Separate decision from mutation. A system reading broad state to decide takes immutable refs and returns a plain `Outcome`/`Delta`/`Plan`. A second function consumes it with `&mut AppState` or the owning mutable state and applies it in the same call stack. This is not a queue. Use for any new system whose decision reads more state than it writes.

### Check Then Mutate

All validation before any mutation. Fallible functions return `Result` or a `#[must_use]` bool; validate with early-return `?`. Never partially mutate before all checks pass.

Multi-resource transactions use a validated token:

1. `validate_*` checks preconditions, returns a `Validated*` token holding resolved targets.
2. The only mutation path is `token.commit(self, state: &mut AppState)`, consuming `self`.

No loose `execute_*_with_token(...)` functions. Single-resource paths may use early-return `?`.

### System Design Rules

- Any create/move/transfer/link/remove/reassign operation preserves all indexes and ownership atomically.
- Order-dependent results sort first or use a stable ordering. Selection among valid choices uses deterministic scoring with stable tie-breaking.
- Keep top-level system execution order in one visible sequence. Comment load-bearing ordering.
- External side effects occur only after internal state is valid or through an explicit durable command/outbox record.
- Importers, migrations, tests, and administrative tools use the same canonical mutation paths as production code.

### Invariant Enforcement

Maintain a debug-only `validate_invariants(state)` asserting every cheap invariant with `debug_assert!`. Call it at the end of the top-level pipeline. Messages name the broken invariant. Add the assertion in the same change that adds the invariant.

Keep one deterministic headless soak test: seed the RNG, build a mixed scenario, run the pipeline for thousands of ticks with `validate_invariants` active.

### Comments

Explain non-obvious intent: hidden constraints, ordering, safety reasoning, workarounds. Do not restate code, add banners, or leave commented-out code.

### Warnings and Dead Code

`cargo check` is silent. Prefix intentionally unused params with `_`. No broad `#[allow(...)]` on structs/functions. Delete dead code and replaced paths. No back-compat shims unless required.

A `dead_code` warning is not by itself proof that code should be deleted. First classify the item. Production-shaped logic that is only called from `#[cfg(test)]` code is orphaned behavior:

- If the behavior is intended to happen in the application, wire it into the canonical production pipeline.
- If the code is only a test helper, fixture builder, assertion helper, or test-only adapter, move it under `#[cfg(test)]`.
- If the behavior is obsolete, delete the code and delete or rewrite the tests that describe it.
- Do not add fake production call sites, broad `#[allow(dead_code)]`, public shims, or test-only production APIs to satisfy the compiler.

A test proves logic is meaningful, not that it is reachable in the application. When a tested function is reported as unused, ask: what user/system behavior should call it, and is the test covering that canonical path or only a private helper?

### Tests

Keep tests of real logic: calculations, state transitions, transactions, invariants, serialization boundaries, failure paths. Delete tests that only prove code can be called: content-count asserts, CRUD round-trips, and broad generator integration tests with no behavioral assertion. Prefer testing through the canonical system path; helper-only coverage must not hide disconnected production behavior.

Co-locate as `#[cfg(test)] mod tests` at the bottom of the file under test. No separate `tests/` tree, no `_test.rs` files. Name test functions descriptively without a redundant `test_` prefix (`transaction_rolls_back_on_invalid_target`, not `test_transaction...`). The `#[test]` attribute and `tests::` module path already say it is a test. Reserve the `test_` prefix for canonical soak/stress scenarios in `src/core/state.rs`.

Fixture helpers that touch a shared production registry and would panic on duplicate registration are named `*_for_test` and are idempotent register-or-update calls, matching `register_or_update_policy_for_test`. Fixtures that build a throwaway local `AppState` or record are named `make_test_*`. Do not invent a third naming scheme for the same job.

## Before Committing

- [ ] `cargo check` silent; `cargo test` passes.
- [ ] Every consequential mutation resolves before the function returns; no mutation bypasses the canonical system path.
- [ ] All randomness comes from state-owned RNG; result-affecting iteration is deterministic.
- [ ] Runtime state is not stored in static definitions; new generated state is serializable.
- [ ] New cross-references are validated at load time or operation-validation time.
- [ ] Project-owned enum matches are exhaustive; struct mappings are explicitly destructured.
- [ ] New Runtime Invariant has a matching `debug_assert!` in `validate_invariants`.
- [ ] Related collections are updated atomically by one owner.
- [ ] External side effects cross explicit adapter boundaries.
- [ ] Old replaced paths are deleted; one implementation per concern.
- [ ] STATUS.md updated, plain ASCII.
