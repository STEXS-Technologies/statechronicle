#!/usr/bin/env python3
"""Fail closed when the reviewed dependency-maintenance exception expires."""
from datetime import date, datetime, timezone
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[1]
ADVISORY = "RUSTSEC-2026-0543"
EXPIRY = date(2026, 12, 17)


def check(root=ROOT, today=None):
    today = today or datetime.now(timezone.utc).date()
    ignored = []
    for name in (".cargo/audit.toml", "deny.toml"):
        with (root / name).open("rb") as config:
            ignored.append(tomllib.load(config)["advisories"]["ignore"])
    if ignored == [[], []]:
        return  # The dependency was migrated and both exceptions were removed.
    if ignored != [[ADVISORY], [ADVISORY]]:
        raise ValueError("advisory exceptions differ from the reviewed maintenance policy")
    if today >= EXPIRY:
        raise ValueError(f"{ADVISORY} exception expired on {EXPIRY}; see DEPENDENCY_MAINTENANCE.md")


if __name__ == "__main__":
    check()
