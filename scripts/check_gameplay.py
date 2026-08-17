#!/usr/bin/env python3
"""Validate long-running gameplay gate reports with stable diagnostics."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any, Callable

STRANDED_SUCCESSION_FINDING = "Political succession can strand institutional recovery"


def fail(message: str) -> None:
    raise SystemExit(f"gameplay report check failed: {message}")


def load_report(path_text: str) -> dict[str, Any]:
    path = Path(path_text)
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"could not read {path}: {error}")
    if not isinstance(report, dict):
        fail(f"{path} must contain a JSON object")
    return report


def campaigns(report: dict[str, Any]) -> list[dict[str, Any]]:
    values = report.get("campaigns")
    if not isinstance(values, list) or not values:
        fail("report must contain at least one campaign")
    if not all(isinstance(value, dict) for value in values):
        fail("campaign entries must be JSON objects")
    return values


def require_generation(report: dict[str, Any]) -> None:
    entries = campaigns(report)
    first = entries[0]
    arc = first.get("fantasy_arc")
    if not isinstance(arc, dict) or arc.get("first_succession_day") is None:
        fail("generation-length gate did not reach succession")
    phase = report.get("aggregate", {}).get("phase_stats", {}).get("SuccessionLegacy", {})
    if phase.get("decision_cycles", 0) == 0:
        fail("generation-length gate did not observe succession-and-legacy decisions")
    print(
        "Generation gate passed: "
        f"{len(entries)} campaign, succession day {arc['first_succession_day']}"
    )


def require_credit_stress(report: dict[str, Any]) -> None:
    entries = campaigns(report)
    credit_actions = sum(
        campaign.get("commands", {}).get("ExtendCredit", {}).get("executed", 0)
        for campaign in entries
    )
    minimum_credit_sample = len(entries) * 2
    if credit_actions < minimum_credit_sample:
        fail(
            f"credit stress audit observed only {credit_actions} player loans; "
            f"requires at least {minimum_credit_sample}"
        )
    distressed = [
        campaign
        for campaign in entries
        if campaign.get("maximum_player_delinquent_lending", 0) > 0
        or campaign.get("maximum_player_defaulted_lending", 0) > 0
    ]
    if not distressed:
        fail("credit stress audit observed no distress on player-issued loans")
    enforcement_cases = sum(
        campaign.get("player_debt_enforcement_cases", 0) for campaign in entries
    )
    if enforcement_cases == 0:
        fail("credit stress audit observed lending distress but no debt enforcement")
    print(
        "Credit stress passed: "
        f"{credit_actions} loans, {len(distressed)} distressed campaign(s), "
        f"{enforcement_cases} enforcement case(s)"
    )


def require_generation_matrix(report: dict[str, Any]) -> None:
    entries = campaigns(report)
    configured = report.get("config", {}).get("personas")
    expected_personas = set(configured) if isinstance(configured, list) else set()
    observed_personas = {campaign.get("persona") for campaign in entries}
    if expected_personas and observed_personas != expected_personas:
        fail(
            "generation audit personas differ from configured personas: "
            f"expected {sorted(expected_personas)}, observed {sorted(observed_personas)}"
        )
    missing = [
        campaign.get("persona", "unknown")
        for campaign in entries
        if campaign.get("fantasy_arc", {}).get("first_succession_day") is None
    ]
    if missing:
        fail(f"generation audit did not reach succession for: {missing}")
    missing_transitions = [
        campaign.get("persona", "unknown")
        for campaign in entries
        if campaign.get("succession_transition") is None
    ]
    if missing_transitions:
        fail(f"generation audit did not capture succession transitions for: {missing_transitions}")
    for campaign in entries:
        transition = campaign["succession_transition"]
        succession_day = campaign["fantasy_arc"]["first_succession_day"]
        if transition.get("day") != succession_day:
            fail(
                f"{campaign.get('persona', 'unknown')} succession transition day "
                f"{transition.get('day')} did not match fantasy milestone day {succession_day}"
            )
    stranded = [
        finding
        for finding in report.get("findings", [])
        if finding.get("title") == STRANDED_SUCCESSION_FINDING
    ]
    if stranded:
        fail(stranded[0].get("evidence", STRANDED_SUCCESSION_FINDING))
    phase = report.get("aggregate", {}).get("phase_stats", {}).get("SuccessionLegacy", {})
    if phase.get("decision_cycles", 0) == 0 or phase.get("substantive_actions", 0) == 0:
        fail("generation audit did not observe substantive succession-and-legacy play")
    print(
        "Generation matrix passed: "
        f"{len(entries)} campaigns, {len(observed_personas)} configured persona(s)"
    )


CHECKS: dict[str, Callable[[dict[str, Any]], None]] = {
    "generation": require_generation,
    "credit-stress": require_credit_stress,
    "generation-matrix": require_generation_matrix,
}


def main() -> None:
    if len(sys.argv) != 3 or sys.argv[1] not in CHECKS:
        raise SystemExit(
            "usage: check_gameplay.py <generation|credit-stress|generation-matrix> <report.json>"
        )
    CHECKS[sys.argv[1]](load_report(sys.argv[2]))


if __name__ == "__main__":
    main()
