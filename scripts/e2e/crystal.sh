#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${CRYSTAL_VERSION:-1.20.0}"

isolate_qusp
step "install crystal ${VERSION}" run_qusp install crystal "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[crystal]
version = "$VERSION"
EOF

out=$(capture_qusp run crystal --version 2>&1)
assert_contains "$out" "Crystal $VERSION" "crystal --version"

# Compile + run a Crystal source file end-to-end.
cat > hello.cr <<'EOF'
puts "hi from qusp-managed crystal"
EOF
out=$(capture_qusp run crystal run hello.cr 2>&1)
assert_contains "$out" "hi from qusp-managed crystal" "crystal run roundtrip"

ok "crystal ${VERSION}: install + version + compile"
