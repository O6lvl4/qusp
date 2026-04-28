#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${GO_VERSION:-1.26.2}"

isolate_qusp
step "install go ${VERSION}" run_qusp install go "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[go]
version = "$VERSION"
EOF

out=$(capture_qusp run go version)
assert_contains "$out" "go${VERSION}" "go version output"

# Tool routing: gopls is in gv-core's static registry.
step "add tool gopls" run_qusp add tool gopls
out=$(capture_qusp run gopls version)
assert_contains "$out" "golang.org/x/tools/gopls" "gopls reports its module path"

ok "go ${VERSION}: install + run + tool routing"
