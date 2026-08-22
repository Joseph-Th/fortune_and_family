# Agent Guide

This file is the execution card for repository work. [ARCHITECTURE.md](ARCHITECTURE.md) owns implementation structure and mutation contracts, [STATUS.md](STATUS.md) owns current capability, [TESTING.md](TESTING.md) owns test policy and completion gates, and [DESIGN.md](DESIGN.md) owns product intent.

## Start here

1. Read [`../AGENTS.md`](../AGENTS.md) and preserve unrelated working-tree state.
2. Read [README.md](README.md), [STATUS.md](STATUS.md), and only the relevant section of [ARCHITECTURE.md](ARCHITECTURE.md).
3. Identify the owning source module and sibling tests; trace the public entry point to canonical mutation and invariant validation.
4. Identify the narrowest proving test before changing behavior. Run it before editing only when reproducing a failure or establishing a baseline the task actually needs; otherwise execute it when the behavior is ready for proof.
5. Use [TESTING.md](TESTING.md) to choose the smallest completion lane that owns the changed surface.
6. Update the one document that owns any changed architecture, behavior, schema, API, command, harness, or scope contract.

This project applies the Universal, Stateful Application, Deterministic System, and Automated Behavior Evaluation portfolio profiles.

## Primary owners

| Concern | Owner |
|---|---|
| Campaign construction | `src/systems/bootstrap.rs` |
| Player commands | `src/systems/commands.rs` |
| Daily/scheduled simulation | `src/systems/simulation.rs`, `src/systems/strategic.rs` |
| Legal/progression | `src/systems/legal.rs`, `src/systems/progression.rs` |
| Persistence | `src/persistence.rs` |
| Read models/HTML | `src/projection.rs` |
| Gameplay analysis | `src/gameplay/` |
| Procedural art | `src/art/` |

## Project guardrails

- `Registry` owns immutable definitions; `AppState` and its records own serializable runtime state; systems own validation and mutation; adapters translate external IO only.
- Consequential operations validate references, ownership, permission, lifecycle, capacity, ranges, and arithmetic before one atomic commit. Rejected operations preserve state.
- Multi-record work resolves the complete result before commit or uses a consumed `Validated*` with current-state revalidation.
- Preserve state-owned deterministic randomness, stable ordered iteration/tie-breaking, fixed-point `Money`/`Quantity`, checked scheduling, and explicit overflow handling.
- Persistent identity uses typed IDs; optional relationships are explicit; authoritative records and owned indexes/lifecycle memberships update coherently.
- Core systems perform no implicit IO. Durable external work is represented before the adapter performs it.
- Project-owned enums are exhaustive; consequential fields are private; domain failures use contextual typed errors; replaced internal paths are deleted.
- Repository verification is local. Do not create or depend on GitHub Actions workflows.

## Completion

Use `bash scripts/test.sh fast <filter>` (or `.\scripts\test.ps1 fast <filter>`
on Windows) for focused iteration when it shortens feedback or isolates a failure.
When ordinary work is ready for completion, go directly to `standard` instead of running `fast`
immediately beforehand merely as a prelude. For specialized surfaces, run the smallest lane from
[TESTING.md](TESTING.md) that owns the changed contract, plus only genuinely distinct evidence that lane
does not already contain. Deeper lanes are required only when their distinct contract changed. Confirm
canonical ownership, deterministic/persistence/invariant behavior, current documentation, and clean diff
hygiene.
