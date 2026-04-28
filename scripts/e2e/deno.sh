#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${DENO_VERSION:-2.0.0}"

isolate_qusp
step "install deno ${VERSION}" run_qusp install deno "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[deno]
version = "$VERSION"
EOF

out=$(capture_qusp run deno --version)
assert_contains "$out" "deno ${VERSION}" "deno --version"

# Deno's add-tool path is intentionally a clear error.
err_out=$(capture_qusp add tool definitely-not-real || true)
assert_contains "$err_out" "no backend recognized tool" \
  "unknown-tool error mentions backend routing"

ok "deno ${VERSION}: install + run, clear no-tools error"
