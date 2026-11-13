# Play

How a campaign is played: a text-forward loop over the persisted campaign state. The command CLI is the play surface; no scripted agent plays the game for you.

`DESIGN.md` owns intent. `STATUS.md` owns the command surface. `ARCHITECTURE.md` owns the canonical mutation path. This document owns the play loop and what counts as evidence of play.

## The loop

A campaign is one saved `AppState` on disk. Every turn is ordinary text:

```bash
cargo run --locked -- new --output saves/valeri.json --seed 42 \
  --dynasty Valeri --founder "Elian Valeri" --background baker

cargo run --locked -- execute saves/valeri.json \
  --command '{"SetHouseGovernance":{"governance":"FamilyPartnership"}}'
cargo run --locked -- execute saves/valeri.json \
  --command '{"InvestInBusiness":{"business_id":1,"amount":{"copper":5000}}}'

cargo run --locked -- simulate saves/valeri.json --days 30
cargo run --locked -- summary saves/valeri.json
```

`new` builds a campaign, `execute` applies exactly one JSON-encoded `PlayerCommand` and persists it, and `simulate` advances a day count. `summary` reads back state in text, `inspect` emits the full JSON projection, `dashboard` renders the self-contained HTML view, and `validate` audits the save.

Read the summary after each turn. The CLI prints scenario and house position, needs-attention items, market movement, current intelligence, recent chronicle entries, and unread notices. Those are the feedback channels a player reads; a command that changes nothing observable is either blocked or inconsequential and should be reconsidered.

## Command discipline

- `PlayerCommand` in `src/systems/commands/` is the authoritative schema. `execute` takes its JSON encoding.
- Every command goes through `apply_player_command`: validation, ownership, permission, lifecycle, capacity, and arithmetic checks run before one atomic commit, so a rejected command leaves the save untouched and reports a typed `CommandError`.
- `execute` writes through the same compare-and-swap save path as `simulate`. If the file changed underneath you, the write fails rather than overwriting concurrent work.
- Player commands and simulated days are the only ways state changes. No adapter patches records.

## What-if probing

An agent that wants to evaluate a candidate action before committing it clones the campaign and probes on the clone instead of mutating the live save.

- `apply_player_command_scratch` applies one command to an exclusively owned scratch campaign. It skips the defensive whole-campaign copy that `apply_player_command` performs, and it is the supported probe entry point for lookahead.
- A failed probe may leave the scratch state partially mutated, so discard the clone rather than keeping it.
- Probe clones are disposable. They are never a source of truth, and only the chosen command is committed to the real save.

This keeps exploration cheap without creating a second mutation path: the probe runs the same validation and commit logic as the real command.

## Determinism and saves

- Same registry, save, command sequence, and day count reproduce the same campaign. Randomness is state-owned; nothing reads OS entropy or wall-clock time in the simulation.
- The save chain is compare-and-swap with a content-addressed revision, an atomic same-directory temp write, and an optional directory sync. `SaveOutcome` reports degraded durability rather than silently weakening it.
- Automation that needs a fresh world each run passes `--start-seed` to pin it; the `art` verb rotates its default seed by UTC day, and campaign seeds are always explicit.
- Saves live under ignored `saves/`. Scratch campaigns, probe clones, and captures belong under ignored `target/agent-output/<task>/` or an OS temp directory.

## Evidence of play

Text-play output is evidence when it is observed from a real session: a campaign created on disk, at least one accepted command, simulated days, and read-back state that shows the command's effect. Prefer that to any claim derived from a unit test alone.

Judgment of prose clarity, pacing, fairness, and whether choices feel consequential belongs to a human reading the session. Tests prove contracts; they do not prove the game reads well.
