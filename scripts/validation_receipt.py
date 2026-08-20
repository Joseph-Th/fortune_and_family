#!/usr/bin/env python3
"""Record and verify content-addressed local validation receipts.

The fingerprint describes repository file contents, not HEAD/index placement,
so a successful pre-commit validation remains valid after those exact bytes are
committed. Ignored build artifacts and the receipt itself are excluded by Git.
"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path


LANE_RANK = {"quick": 1, "standard": 2}


def git(*args: str) -> bytes:
    return subprocess.check_output(("git", *args), stderr=subprocess.DEVNULL)


def repository_root() -> Path:
    return Path(os.fsdecode(git("rev-parse", "--show-toplevel").strip())).resolve()


def receipt_path(root: Path) -> Path:
    raw = Path(os.fsdecode(git("rev-parse", "--git-path", "git-wizard/validation-receipt.json").strip()))
    return raw if raw.is_absolute() else root / raw


def repository_fingerprint(root: Path) -> str:
    digest = hashlib.sha256()
    paths = sorted(
        path
        for path in git("ls-files", "--cached", "--others", "--exclude-standard", "-z").split(b"\0")
        if path
    )
    for raw_path in paths:
        path = root / os.fsdecode(raw_path)
        digest.update(len(raw_path).to_bytes(4, "big"))
        digest.update(raw_path)
        if path.is_symlink():
            target = os.fsencode(os.readlink(path))
            digest.update(b"L")
            digest.update(len(target).to_bytes(8, "big"))
            digest.update(target)
            continue
        if not path.exists():
            digest.update(b"D")
            continue
        digest.update(b"F")
        digest.update(path.stat().st_size.to_bytes(8, "big"))
        with path.open("rb") as handle:
            while chunk := handle.read(1024 * 1024):
                digest.update(chunk)
    return digest.hexdigest()


def record(required_lane: str, expected_fingerprint: str) -> int:
    if required_lane not in LANE_RANK:
        print(f"unsupported receipt lane: {required_lane}", file=sys.stderr)
        return 2
    root = repository_root()
    current_fingerprint = repository_fingerprint(root)
    if current_fingerprint != expected_fingerprint:
        print(
            "repository content changed while validation was running; no receipt recorded",
            file=sys.stderr,
        )
        return 3
    destination = receipt_path(root)
    destination.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "schema": 1,
        "lane": required_lane,
        "fingerprint": current_fingerprint,
    }
    temporary = destination.with_suffix(".tmp")
    temporary.write_text(json.dumps(payload, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, destination)
    print(f"recorded {required_lane} validation receipt")
    return 0


def check(required_lane: str) -> int:
    if required_lane not in LANE_RANK:
        return 2
    root = repository_root()
    destination = receipt_path(root)
    try:
        payload = json.loads(destination.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return 1
    recorded_lane = payload.get("lane")
    if payload.get("schema") != 1 or recorded_lane not in LANE_RANK:
        return 1
    if LANE_RANK[recorded_lane] < LANE_RANK[required_lane]:
        return 1
    if payload.get("fingerprint") != repository_fingerprint(root):
        return 1
    print(f"reusing {recorded_lane} validation receipt for {required_lane} pre-push gate")
    return 0


def main() -> int:
    if len(sys.argv) == 2 and sys.argv[1] == "fingerprint":
        print(repository_fingerprint(repository_root()))
        return 0
    if len(sys.argv) == 3 and sys.argv[1] == "check":
        return check(sys.argv[2])
    if len(sys.argv) == 4 and sys.argv[1] == "record":
        return record(sys.argv[2], sys.argv[3])
    print(
        "usage: validation_receipt.py fingerprint | check LANE | record LANE START_FINGERPRINT",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
