#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${DART_VERSION:-3.5.4}"

isolate_qusp
step "install dart ${VERSION}" run_qusp install dart "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[dart]
version = "$VERSION"
EOF

out=$(capture_qusp run dart --version 2>&1)
assert_contains "$out" "$VERSION" "dart --version reports the pinned version"

# Compile + run a Dart source file end-to-end.
cat > hello.dart <<'EOF'
void main() {
  print('hi from qusp-managed dart');
}
EOF
out=$(capture_qusp run dart run hello.dart 2>&1)
assert_contains "$out" "hi from qusp-managed dart" "dart run roundtrip"

ok "dart ${VERSION}: install + version + run"
