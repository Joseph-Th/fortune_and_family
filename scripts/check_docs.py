#!/usr/bin/env python3
"""Validate repository documentation contracts without external dependencies."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REQUIRED_DOCS = (
    "README.md",
    "ARCHITECTURE.md",
    "AGENTS.md",
    "DESIGN.md",
    "STATUS.md",
    "TESTING.md",
    "GAMEPLAY_HARNESS.md",
)


def fail(message: str) -> None:
    print(f"documentation check failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def read(relative_path: str) -> str:
    path = ROOT / relative_path
    if not path.is_file():
        fail(f"missing {relative_path}")
    text = path.read_text(encoding="utf-8")
    if not text.strip():
        fail(f"{relative_path} is empty")
    return text


def capture(pattern: str, text: str, source: str) -> str:
    match = re.search(pattern, text, re.MULTILINE)
    if match is None:
        fail(f"could not read expected value from {source}")
    return match.group(1)


def check_status_contracts() -> None:
    cargo = read("Cargo.toml")
    state = read("src/core/state.rs")
    gameplay = read("src/gameplay.rs")
    status = read("STATUS.md")

    crate_version = capture(r'^version\s*=\s*"([^"]+)"', cargo, "Cargo.toml")
    edition = capture(r'^edition\s*=\s*"([^"]+)"', cargo, "Cargo.toml")
    rust_version = capture(r'^rust-version\s*=\s*"([^"]+)"', cargo, "Cargo.toml")
    save_schema = capture(
        r"CURRENT_SCHEMA_VERSION:\s*u32\s*=\s*(\d+)", state, "src/core/state.rs"
    )
    report_schema = capture(
        r"GAMEPLAY_REPORT_SCHEMA_VERSION:\s*u16\s*=\s*(\d+)",
        gameplay,
        "src/gameplay.rs",
    )
    previous_schema = str(int(save_schema) - 1)

    expected_lines = (
        f"| Crate version | `{crate_version}` |",
        f"| Rust edition | {edition} |",
        f"| Minimum Rust version | {rust_version} |",
        f"| Save schema | {save_schema} |",
        f"| Supported save migrations | Versions 0 through {previous_schema} |",
        f"| Gameplay report schema | {report_schema} |",
    )
    for line in expected_lines:
        if line not in status:
            fail(f"STATUS.md must contain: {line}")


def check_repository_references() -> None:
    reference_pattern = re.compile(
        r"`((?:src|scripts)/[^`\s]+|[A-Z][A-Z_]+\.md)`"
    )
    for document in REQUIRED_DOCS:
        text = read(document)
        for raw_target in reference_pattern.findall(text):
            target = raw_target.rstrip(".,;:")
            if "*" in target:
                continue
            if not (ROOT / target).exists():
                fail(f"{document} references missing path {target}")


def check_forward_facing_status() -> None:
    status = read("STATUS.md")
    forbidden = (
        "Verified on",
        "tests pass",
        "repair history",
        "release gameplay matrix",
    )
    for phrase in forbidden:
        if phrase.lower() in status.lower():
            fail(f"STATUS.md contains retrospective phrase {phrase!r}")


def main() -> None:
    for document in REQUIRED_DOCS:
        read(document)
    check_status_contracts()
    check_repository_references()
    check_forward_facing_status()
    print("Documentation contracts are consistent.")


if __name__ == "__main__":
    main()
