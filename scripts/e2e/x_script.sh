#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# `qusp x <script>` extension-routing — uv-class hospitality test.
# Use Lua because (a) source build is fast (~5s) and (b) it has no
# cross-backend dep, so we exercise the install-then-exec path
# without needing Java/etc to be pre-installed.
command -v make >/dev/null 2>&1 || skip "no make on host"
command -v cc >/dev/null 2>&1 || command -v clang >/dev/null 2>&1 \
    || command -v gcc >/dev/null 2>&1 || skip "no C compiler on host"

isolate_qusp

# No qusp.toml, no `.lua-version`, no prior `qusp install lua` —
# just a script. qusp x should detect the .lua extension, install
# the curated default Lua version, and run the script.
mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > hello.lua <<'EOF'
print("hi from qusp x routing")
EOF

step "qusp x ./hello.lua against fresh HOME (auto-installs lua)" \
    run_qusp x ./hello.lua

# Repeat invocation should reuse the installed toolchain — verify
# by confirming the second run produces the script output.
out=$(capture_qusp x ./hello.lua 2>&1)
assert_contains "$out" "hi from qusp x routing" \
    "qusp x ./hello.lua exec output"

# Argv passthrough: extra args after the script path reach the script.
cat > args.lua <<'EOF'
for i, v in ipairs(arg) do print(i, v) end
EOF
out=$(capture_qusp x ./args.lua alpha beta 2>&1)
assert_contains "$out" "alpha" "argv[1] passed through"
assert_contains "$out" "beta" "argv[2] passed through"

# Tool-dispatch path is untouched: `qusp x` with a non-script
# argv[0] still routes via the existing tool registry. Use a
# made-up name to confirm we hit the *tool* path (which will fail
# with a "no backend" error, not a "script not found" error).
err=$(capture_qusp x not-a-real-tool-or-script 2>&1 || true)
assert_contains "$err" "no backend" \
    "non-script argv falls through to tool dispatch"

ok "qusp x: extension-routing + idempotent reuse + argv passthrough + tool fallback"
