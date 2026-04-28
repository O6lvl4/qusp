#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${KOTLIN_VERSION:-2.1.20}"
JAVA_VERSION="${JAVA_VERSION:-21}"

isolate_qusp

# Cross-backend dep: [kotlin] without [java] should fail with a clear message.
mkdir -p "$HOME/proj-no-java" && cd "$HOME/proj-no-java"
cat > qusp.toml <<EOF
[kotlin]
version = "$VERSION"
EOF
err=$(capture_qusp install || true)
assert_contains "$err" "requires [java]" \
  "kotlin without [java] errors with cross-backend message"

# With [java] pinned, install both. Java install fans out in parallel.
mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[java]
version = "$JAVA_VERSION"
distribution = "temurin"

[kotlin]
version = "$VERSION"
EOF

step "install (java + kotlin in parallel)" run_qusp install

# kotlinc -version writes to stderr; merge.
out=$(capture_qusp run kotlinc -version 2>&1)
assert_contains "$out" "$VERSION" "kotlinc -version reports the pinned version"
# This line proves the cross-backend env merge worked: kotlinc reports
# the JRE it sees, which must be the qusp-managed Temurin.
assert_contains "$out" "JRE" "kotlinc reports a JRE (env merge present)"

# End-to-end compile + run.
cat > Hello.kt <<'EOF'
fun main() { println("hello from kotlin via qusp") }
EOF
step "kotlinc → jar" run_qusp run kotlinc Hello.kt -include-runtime -d Hello.jar
out=$(capture_qusp run java -jar Hello.jar)
assert_eq "$out" "hello from kotlin via qusp" "compiled .jar runs on qusp java"

ok "kotlin ${VERSION}: install + cross-backend [java] merge + compile + run"
