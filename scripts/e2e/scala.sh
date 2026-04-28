#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${SCALA_VERSION:-3.8.3}"
JAVA_VERSION="${JAVA_VERSION:-21}"

isolate_qusp

# Cross-backend dep: [scala] without [java] should fail with a clear message.
mkdir -p "$HOME/proj-no-java" && cd "$HOME/proj-no-java"
cat > qusp.toml <<EOF
[scala]
version = "$VERSION"
EOF
err=$(capture_qusp install || true)
assert_contains "$err" "requires [java]" \
  "scala without [java] errors with cross-backend message"

# With [java] pinned, install both.
mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[java]
version = "$JAVA_VERSION"
distribution = "temurin"

[scala]
version = "$VERSION"
EOF

step "install (java + scala in parallel)" run_qusp install

out=$(capture_qusp run scala --version 2>&1)
assert_contains "$out" "$VERSION" "scala --version reports the pinned version"

# End-to-end: compile + run a Scala source file.
cat > Hello.scala <<'EOF'
@main def hello = println("hi from qusp-managed scala")
EOF
out=$(capture_qusp run scala run Hello.scala 2>&1)
assert_contains "$out" "hi from qusp-managed scala" "scala run roundtrip"

ok "scala ${VERSION}: install + cross-backend [java] merge + run"
