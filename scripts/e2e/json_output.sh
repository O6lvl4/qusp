#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# `--output-format json` must produce parseable JSON across every
# introspection-style subcommand. Phase 5 audit row R1.

# `jq` isn't installed everywhere; fall back to `python -c 'json.loads'`
# if missing. One of them is on a developer's box.
parse_json() {
    if command -v jq >/dev/null 2>&1; then
        jq -e . "$@"
    else
        python3 -c "import sys, json; json.loads(sys.stdin.read()); print('ok')"
    fi
}

isolate_qusp

# `qusp backends`
out=$(capture_qusp backends --output-format json)
echo "$out" | parse_json > /dev/null
assert_contains "$out" '"id": "python"' "backends JSON includes python"
assert_contains "$out" '"id": "lua"' "backends JSON includes lua"

# `qusp dir <kind>` — text mode unchanged (script-friendly bare path)
text_dir=$(capture_qusp dir cache)
assert_contains "$text_dir" "qusp" "text dir output is bare path"

# `qusp dir cache --output-format json`
out=$(capture_qusp dir cache --output-format json)
echo "$out" | parse_json > /dev/null
assert_contains "$out" '"kind": "cache"' "dir JSON has kind"
assert_contains "$out" '"path"' "dir JSON has path"

# `qusp doctor --output-format json` — qusp_version, paths, backends
out=$(capture_qusp doctor --output-format json)
echo "$out" | parse_json > /dev/null
assert_contains "$out" '"qusp_version"' "doctor JSON has qusp_version"
assert_contains "$out" '"paths"' "doctor JSON has paths"
assert_contains "$out" '"installed_count"' "doctor JSON has installed_count"

# `qusp current --output-format json` — empty cwd, no pins, every
# backend should appear with version=null.
mkdir -p "$HOME/proj-empty" && cd "$HOME/proj-empty"
out=$(capture_qusp current --output-format json)
echo "$out" | parse_json > /dev/null
assert_contains "$out" '"version": null' "current JSON null when nothing pinned"

# `qusp list <lang> --output-format json` — should be valid JSON even
# when there are 0 installed versions.
out=$(capture_qusp list python --output-format json)
echo "$out" | parse_json > /dev/null
assert_contains "$out" '"lang": "python"' "list JSON has lang"
assert_contains "$out" '"scope": "installed"' "list JSON scope=installed"

# Global flag works regardless of position (clap global = true).
out=$(capture_qusp --output-format json backends)
echo "$out" | parse_json > /dev/null
assert_contains "$out" '"backends"' "global --output-format works pre-subcommand"

ok "JSON output: backends + dir + doctor + current + list parseable, text mode preserved"
