#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${BUN_VERSION:-1.2.0}"

isolate_qusp
step "install bun ${VERSION}" run_qusp install bun "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[bun]
version = "$VERSION"
EOF

out=$(capture_qusp run bun --version)
assert_contains "$out" "$VERSION" "bun --version"

# Bun upstream symlinks bunx → bun; qusp mirrors that.
out=$(capture_qusp run bunx --version)
assert_contains "$out" "$VERSION" "bunx mirror works"

ok "bun ${VERSION}: install + run + bunx"
