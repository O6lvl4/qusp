#!/usr/bin/env bash
# Cross-cutting commands that don't belong to any one backend.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

isolate_qusp

# init: no args writes a commented template.
mkdir -p "$HOME/proj-empty" && cd "$HOME/proj-empty"
step "init (no args)" run_qusp init
test -f qusp.toml || fail "init did not write qusp.toml"
out=$(grep -c "^\# \[" qusp.toml || true)
[ "$out" -ge 5 ] || fail "init template should comment out >= 5 backends, got $out"

# init --langs: writes only requested set, uncommented.
mkdir -p "$HOME/proj-targeted" && cd "$HOME/proj-targeted"
step "init --langs go,rust,bun" run_qusp init --langs go,rust,bun
grep -q "^\[go\]" qusp.toml      || fail "init --langs missing [go]"
grep -q "^\[rust\]" qusp.toml    || fail "init --langs missing [rust]"
grep -q "^\[bun\]" qusp.toml     || fail "init --langs missing [bun]"
grep -q "^\[node\]" qusp.toml    && fail "init --langs included unrequested [node]"

# init unknown lang errors helpfully.
out=$(capture_qusp init --langs foolang --force || true)
assert_contains "$out" "unknown language" "init rejects unknown lang"

# self-update --check should resolve a tag without writing.
out=$(capture_qusp self-update --check)
assert_contains "$out" "qusp v" "self-update --check reports a version"

# Subcommand help should mention superposition (proves about-text wired).
out=$(capture_qusp --help)
assert_contains "$out" "superposition" "--help mentions superposition"

# `backends` should list at least 8 known languages.
out=$(capture_qusp backends)
for lang in bun deno go java node python ruby rust; do
    assert_contains "$out" "$lang" "backends lists $lang"
done

ok "smoke: init / init --langs / self-update --check / backends / help"
