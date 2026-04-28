#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# `# qusp: <lang> = <v>` inline metadata — Phase 5 audit row D2.
# Use Lua because (a) source build is fast, (b) we have multiple
# verified versions in the SHA table (5.4.4–5.4.8 + 5.5.0), letting
# us actually pin a different version via inline metadata and verify
# qusp installs *that one* not the curated default.
command -v make >/dev/null 2>&1 || skip "no make on host"
command -v cc >/dev/null 2>&1 || command -v clang >/dev/null 2>&1 \
    || command -v gcc >/dev/null 2>&1 || skip "no C compiler on host"

isolate_qusp

mkdir -p "$HOME/proj" && cd "$HOME/proj"

# Inline metadata pins Lua 5.4.5; without it qusp would pick 5.4.7
# (the curated default in script.rs).
cat > pinned.lua <<'EOF'
-- qusp: lua = 5.4.5
print(_VERSION)
EOF

step "qusp x ./pinned.lua (auto-installs lua 5.4.5 from inline metadata)" \
    run_qusp x ./pinned.lua

# Verify the install_dir is 5.4.5, not the curated default 5.4.7.
out=$(capture_qusp list lua --output-format json)
assert_contains "$out" '"version": "5.4.5"' \
    "lua 5.4.5 (from inline metadata) appears in installed list"

# And confirm the script actually ran with that version.
out=$(capture_qusp x ./pinned.lua 2>&1)
assert_contains "$out" "Lua 5.4" "lua banner from script run"

# Lang mismatch must be ignored: a python directive in a lua script
# should not derail Lua's own resolution. (Use 5.4.7 via no metadata
# in this script.)
cat > plain.lua <<'EOF'
-- qusp: python = "3.99.0"
-- (above directive should be ignored for Lua resolution)
print(_VERSION)
EOF
step "qusp x ./plain.lua (lang-mismatch metadata ignored)" \
    run_qusp x ./plain.lua

ok "inline metadata: pin honored, lang mismatch ignored, version resolution priority 0"
