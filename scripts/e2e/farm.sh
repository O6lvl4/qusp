#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# v0.29.0 symlink farm + global pin (Phase 1 active dogfood Step 1).
# Verifies:
#   - install of a backend with only unversioned bins doesn't farm
#   - `qusp pin set <lang> <v>` materialises unversioned symlinks
#   - bare command via ~/.local/bin/<name> works without activation
#   - pin list / rm flow works
#   - Foreign symlink is preserved (qusp doesn't clobber)

# Lua is the cleanest test (unversioned only, fast source build).
command -v make >/dev/null 2>&1 || skip "no make on host"
command -v cc >/dev/null 2>&1 || command -v clang >/dev/null 2>&1 \
    || command -v gcc >/dev/null 2>&1 || skip "no C compiler on host"

isolate_qusp

step "install lua 5.4.7" run_qusp install lua 5.4.7

# Without a pin, no farm symlinks for unversioned bins.
[ ! -e "$HOME/.local/bin/lua" ] || fail "expected no ~/.local/bin/lua before pin"

# Pin globally → expose unversioned bins.
step "pin lua 5.4.7 globally" run_qusp pin set lua 5.4.7

[ -L "$HOME/.local/bin/lua" ] || fail "expected ~/.local/bin/lua symlink after pin"
[ -L "$HOME/.local/bin/luac" ] || fail "expected ~/.local/bin/luac symlink after pin"

# Bare command works via the symlink (no activation).
out=$("$HOME/.local/bin/lua" -v 2>&1)
assert_contains "$out" "Lua 5.4.7" "bare lua via ~/.local/bin works"

# pin list shows the pin.
out=$(capture_qusp pin list)
assert_contains "$out" "lua" "pin list includes lua"
assert_contains "$out" "5.4.7" "pin list includes version"

# Foreign symlink (pre-existing, not pointing at qusp store) preserved.
mkdir -p "$HOME/foreign-bin" && touch "$HOME/foreign-bin/luarocks"
ln -sf "$HOME/foreign-bin/luarocks" "$HOME/.local/bin/luarocks"
# Re-run pin to attempt re-materialise (luarocks isn't in our farm list anyway,
# this just confirms the foreign-link policy doesn't get triggered for unrelated names)
step "re-pin lua (idempotent)" run_qusp pin set lua 5.4.7
[ -L "$HOME/.local/bin/luarocks" ] || fail "foreign symlink luarocks should be untouched"

# Unpin removes the entry from global config (links not auto-removed by design).
step "pin rm lua" run_qusp pin rm lua
out=$(capture_qusp pin list)
assert_contains "$out" "no global pins set" "pin list empty after rm"

ok "farm + pin: install / pin / bare exec / list / rm / foreign-preserve"
