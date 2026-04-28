#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Pin a major.minor; PBS publishes only the latest patch, so qusp's
# fuzzy matcher resolves to whatever's current.
VERSION="${PYTHON_VERSION:-3.13.0}"

isolate_qusp
step "install python ${VERSION}" run_qusp install python "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[python]
version = "$VERSION"
EOF

out=$(capture_qusp run python3 --version)
assert_contains "$out" "Python 3.13" "python --version reports 3.13.x"

ok "python ${VERSION}: install + run (resolved via PBS fuzzy match)"
