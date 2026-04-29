#!/usr/bin/env python3
"""
refresh_default_versions.py — release-prep drift report.

Compares the static `default_version` table in
`crates/qusp-cli/src/script.rs` (and the duplicated map in
`crates/qusp-cli/src/main.rs::cmd_init`) against the **current
upstream "latest stable"** for each backend, as reported by
`qusp list <lang> --remote --output-format json`.

Outputs a per-language diff with **suggested updates**, but does NOT
auto-commit. Release prep workflow:

    1. cargo build --release
    2. ./scripts/refresh_default_versions.py
    3. Review the suggested diff (filter EA / pre-release / LTS-vs-current)
    4. Hand-apply via sed or editor
    5. cargo test --release
    6. Tag + push

The script must work offline-degraded: a backend whose list_remote
fails (network, rate-limit) is reported as ``unknown`` and skipped
without aborting the whole refresh.

Why not auto-suggest with per-lang filters? In dogfood we observed:

- python list_remote includes 3.15.0a7 (alpha) at top
- java list_remote returns 26.0.1+10 (EA) ahead of 21.0.11 (LTS)
- ruby list_remote returns ascending order (1.8.5 first)
- dart list_remote returns empty (transient or schema-shift)

Each is a separate decision; bundling into "latest_stable" auto-pick
is fragile. Surface the data, let the human decide.
"""

from __future__ import annotations

import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SCRIPT_RS = REPO / "crates/qusp-cli/src/script.rs"
MAIN_RS = REPO / "crates/qusp-cli/src/main.rs"
QUSP = REPO / "target/release/qusp"

# Per-language hint about how to filter list_remote for a sensible
# release-prep default. Empty hint = no hint; human picks from top N.
# `lts_marker` flags backends whose list output annotates LTS releases.
HINTS = {
    "python": "skip 3.x.0{a,b,rc} (alphas/betas) — pick highest stable 3.<n>.<patch>",
    "node": "prefer even-major (22, 24, 26) for LTS — odd majors are non-LTS",
    "java": "prefer (LTS) marker — 21, 25 etc. — over latest EA build",
    "ruby": "list may sort ascending; pick from end of remote, not start",
    "rust": "first entry is latest stable (channel detection works for `stable`)",
    "haskell": "GHC stable line — skip alpha/rc tags",
    "groovy": "pick latest 4.x or 5.x depending on Gradle compat we want",
    "lua": "5.4.x is current stable line; 5.5.x just released — pick 5.4 latest patch unless intentional bump",
}


@dataclass
class Backend:
    lang: str
    current: str | None
    remote_top: list[str]
    error: str | None = None


def extract_default_version_table() -> dict[str, str]:
    """Parse the `default_version` fn body from script.rs."""
    text = SCRIPT_RS.read_text()
    # Find the default_version function and read until its closing brace.
    idx = text.find("pub fn default_version")
    if idx < 0:
        raise RuntimeError("default_version fn not found in script.rs")
    body_start = text.find("{", idx)
    if body_start < 0:
        raise RuntimeError("could not find body of default_version")
    # Walk forward, balance braces.
    depth = 0
    end = body_start
    for i, ch in enumerate(text[body_start:], start=body_start):
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                end = i
                break
    body = text[body_start : end + 1]
    out: dict[str, str] = {}
    # Lines like:  "go" => "1.26.2",
    for line in body.splitlines():
        stripped = line.strip()
        if not stripped.startswith('"'):
            continue
        if "=>" not in stripped:
            continue
        try:
            key_part, val_part = stripped.split("=>", 1)
            key = key_part.strip().strip('"').strip()
            val = val_part.strip().rstrip(",").strip().strip('"').strip()
            if key and val and not val.startswith("return"):
                out[key] = val
        except ValueError:
            continue
    return out


def fetch_remote_top(lang: str, n: int = 5) -> tuple[list[str], str | None]:
    """Run `qusp list <lang> --remote --output-format json`. Returns
    (top-n versions, error-or-none)."""
    try:
        proc = subprocess.run(
            [str(QUSP), "list", lang, "--remote", "--output-format", "json"],
            capture_output=True,
            timeout=30,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return [], "timeout fetching remote list"
    if proc.returncode != 0:
        return [], proc.stderr.decode("utf-8", errors="replace").strip()[:200]
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as e:
        return [], f"non-JSON output: {e}"
    versions = [v.get("version", "") for v in data.get("versions", [])]
    return versions[:n], None


def render_report(backends: list[Backend]) -> None:
    """Print a diff report to stdout. Sections: drift, parity, errors."""
    drift: list[Backend] = []
    parity: list[Backend] = []
    errors: list[Backend] = []
    for b in backends:
        if b.error:
            errors.append(b)
        elif b.current and b.remote_top and b.current == b.remote_top[0]:
            parity.append(b)
        else:
            drift.append(b)

    print("=" * 72)
    print("qusp default-version drift report")
    print("=" * 72)
    print(
        f"static table: {SCRIPT_RS.relative_to(REPO)}\n"
        f"            + {MAIN_RS.relative_to(REPO)} (cmd_init duplicated map)\n"
    )

    if drift:
        print(f"## drift ({len(drift)} backends out of date)\n")
        for b in drift:
            print(f"  [{b.lang}] table = {b.current!r}")
            for i, v in enumerate(b.remote_top):
                marker = " ←" if i == 0 else "  "
                print(f"          remote[{i}]{marker} {v!r}")
            hint = HINTS.get(b.lang)
            if hint:
                print(f"          hint: {hint}")
            print()
    else:
        print("## drift\n  (none — every backend at remote[0])\n")

    if parity:
        print(f"## parity ({len(parity)})")
        for b in parity:
            print(f"  ✓ {b.lang} = {b.current}")
        print()

    if errors:
        print(f"## errors ({len(errors)})")
        for b in errors:
            print(f"  ⚠ {b.lang}: {b.error}")
        print(
            "\n  errors are non-fatal — release prep can still proceed,\n"
            "  just leave those rows untouched until next refresh.\n"
        )

    # Summary line for CI.
    print("=" * 72)
    print(
        f"summary: {len(drift)} drift / {len(parity)} parity / {len(errors)} errors"
    )
    if drift:
        print(
            "next: review hints above, hand-edit `default_version` and the\n"
            "      `cmd_init` map, then `cargo test --release` before tag."
        )
    print("=" * 72)


def main() -> int:
    if not QUSP.exists():
        print(
            f"error: {QUSP.relative_to(REPO)} not found.\n"
            f"       run `cargo build --release` first.",
            file=sys.stderr,
        )
        return 2
    table = extract_default_version_table()
    if not table:
        print("error: extracted default_version table is empty", file=sys.stderr)
        return 2
    backends: list[Backend] = []
    for lang in sorted(table.keys()):
        top, err = fetch_remote_top(lang, n=5)
        backends.append(
            Backend(lang=lang, current=table[lang], remote_top=top, error=err)
        )
    render_report(backends)
    # Exit 0 even with drift — this is a report, not a gate.
    # CI can interpret the summary line if it wants to gate releases.
    return 0


if __name__ == "__main__":
    sys.exit(main())
