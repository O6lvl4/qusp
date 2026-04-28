#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${NODE_VERSION:-22.9.0}"

isolate_qusp
step "install node ${VERSION}" run_qusp install node "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[node]
version = "$VERSION"
EOF

out=$(capture_qusp run node --version)
assert_contains "$out" "v${VERSION}" "node --version"

# Curated tool: tsc lives in the static registry.
step "add tool tsc" run_qusp add tool tsc
out=$(capture_qusp run tsc --version)
assert_contains "$out" "Version" "tsc --version"

ok "node ${VERSION}: install + run + tsc (npm dist.integrity verified)"
