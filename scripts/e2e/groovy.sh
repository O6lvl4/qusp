#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${GROOVY_VERSION:-4.0.22}"
JAVA_VERSION="${JAVA_VERSION:-21}"

isolate_qusp

# Cross-backend dep: [groovy] without [java] should fail with a clear message.
mkdir -p "$HOME/proj-no-java" && cd "$HOME/proj-no-java"
cat > qusp.toml <<EOF
[groovy]
version = "$VERSION"
EOF
err=$(capture_qusp install || true)
assert_contains "$err" "requires [java]" \
  "groovy without [java] errors with cross-backend message"

# With [java] pinned, install both. Java install fans out in parallel.
mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[java]
version = "$JAVA_VERSION"
distribution = "temurin"

[groovy]
version = "$VERSION"
EOF

step "install (java + groovy in parallel)" run_qusp install

out=$(capture_qusp run groovy --version 2>&1)
assert_contains "$out" "$VERSION" "groovy --version reports the pinned version"
# Cross-backend env merge: groovy's launcher prints "JVM: <vendor>" because
# it found java on PATH. Proves [java] env merged through.
assert_contains "$out" "JVM" "groovy reports a JVM (env merge present)"

# End-to-end: compile + run a .groovy script in process via `groovy`.
cat > hello.groovy <<'EOF'
println "hi from qusp-managed groovy"
EOF
out=$(capture_qusp run groovy hello.groovy 2>&1)
assert_contains "$out" "hi from qusp-managed groovy" "groovy script roundtrip"

ok "groovy ${VERSION}: install + cross-backend [java] merge + script run"
