#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${LUA_VERSION:-5.4.7}"

# Source-build backend — needs a host C compiler. Skip with TAP-style
# code 77 if `make` or `cc` is missing rather than reporting a real fail.
command -v make >/dev/null 2>&1 || skip "no make on host"
command -v cc >/dev/null 2>&1 || command -v clang >/dev/null 2>&1 \
    || command -v gcc >/dev/null 2>&1 || skip "no C compiler on host"

isolate_qusp
step "install lua ${VERSION}" run_qusp install lua "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[lua]
version = "$VERSION"
EOF

out=$(capture_qusp run lua -v 2>&1)
assert_contains "$out" "$VERSION" "lua -v reports the pinned version"

cat > hello.lua <<'EOF'
print("hi from qusp-managed lua")
EOF
out=$(capture_qusp run lua hello.lua 2>&1)
assert_contains "$out" "hi from qusp-managed lua" "lua script roundtrip"

# Verify luac (compiler) is also on PATH.
out=$(capture_qusp run luac -v 2>&1)
assert_contains "$out" "$VERSION" "luac -v reports the pinned version"

ok "lua ${VERSION}: source build + install + script run"
