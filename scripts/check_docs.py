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
    art_harness = read("src/art/harness.rs")
    read("scripts/check_gameplay.py")
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
    art_report_schema = capture(
        r"ART_REVIEW_SCHEMA_VERSION:\s*u32\s*=\s*(\d+)",
        art_harness,
        "src/art/harness.rs",
    )
    expected_lines = (
        f"| Crate version | `{crate_version}` |",
        f"| Rust edition | {edition} |",
        f"| Minimum Rust version | {rust_version} |",
        f"| Save schema | {save_schema} |",
        "| Supported save schemas | Current schema only |",
        f"| Gameplay report schema | {report_schema} |",
        f"| Art review report schema | {art_report_schema} |",
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


def check_readme_map() -> None:
    readme = read("README.md")
    for document in REQUIRED_DOCS:
        if document == "README.md":
            continue
        if f"`{document}`" not in readme:
            fail(f"README.md must orient readers to {document}")


def check_test_runner_contract() -> None:
    runner = read("scripts/test.sh")
    testing = read("TESTING.md")
    runner_modes = set(
        re.findall(r"^  ([a-z][a-z-]*)\)$", runner, re.MULTILINE)
    )
    documented_modes = set(
        re.findall(r"`bash scripts/test\.sh ([a-z][a-z-]*)", testing)
    )
    missing_docs = sorted(runner_modes - documented_modes)
    unknown_docs = sorted(documented_modes - runner_modes)
    if missing_docs:
        fail(
            "TESTING.md does not document test runner modes: "
            + ", ".join(missing_docs)
        )
    if unknown_docs:
        fail(
            "TESTING.md references unsupported test runner modes: "
            + ", ".join(unknown_docs)
        )


def check_document_style() -> None:
    retrospective_phrases = (
        "Verified on",
        "tests pass",
        "repair history",
        "completed repair",
        "previously",
        "formerly",
        "used to",
        "no longer",
        "regression fix",
        "workaround for",
    )
    maximum_line_length = 500

    for document in REQUIRED_DOCS:
        text = read(document)
        lowered = text.lower()
        for phrase in retrospective_phrases:
            if phrase.lower() in lowered:
                fail(f"{document} contains retrospective phrase {phrase!r}")
        for line_number, line in enumerate(text.splitlines(), start=1):
            if len(line) > maximum_line_length:
                fail(
                    f"{document}:{line_number} exceeds {maximum_line_length} characters; "
                    "split long prose into readable paragraphs or lists"
                )


def main() -> None:
    for document in REQUIRED_DOCS:
        read(document)
    check_status_contracts()
    check_repository_references()
    check_readme_map()
    check_test_runner_contract()
    check_document_style()
    print("Documentation contracts are consistent.")


if __name__ == "__main__":
    main()
