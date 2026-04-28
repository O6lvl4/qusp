#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${JULIA_VERSION:-1.10.4}"

isolate_qusp
step "install julia ${VERSION}" run_qusp install julia "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[julia]
version = "$VERSION"
EOF

out=$(capture_qusp run julia --version)
assert_contains "$out" "julia version $VERSION" "julia --version"

# Run a tiny program to validate the bin + lib + share layout survived.
out=$(capture_qusp run julia -e 'println("hi from qusp-managed julia")')
assert_eq "$out" "hi from qusp-managed julia" "julia -e roundtrip"

ok "julia ${VERSION}: install + version + execute"
