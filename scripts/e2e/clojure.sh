#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${CLOJURE_VERSION:-1.12.4.1618}"
JAVA_VERSION="${JAVA_VERSION:-21}"

isolate_qusp

# Cross-backend dep: [clojure] without [java] should fail with a clear message.
mkdir -p "$HOME/proj-no-java" && cd "$HOME/proj-no-java"
cat > qusp.toml <<EOF
[clojure]
version = "$VERSION"
EOF
err=$(capture_qusp install || true)
assert_contains "$err" "requires [java]" \
  "clojure without [java] errors with cross-backend message"

# With [java] pinned, install both.
mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[java]
version = "$JAVA_VERSION"
distribution = "temurin"

[clojure]
version = "$VERSION"
EOF

step "install (java + clojure in parallel)" run_qusp install

out=$(capture_qusp run clojure --version 2>&1)
assert_contains "$out" "$VERSION" "clojure --version reports the pinned version"

# Inline -e expression — exercises clj.jar resolution + JVM dispatch.
out=$(capture_qusp run clojure -M -e '(println "hi from qusp-managed clojure")' 2>&1)
assert_contains "$out" "hi from qusp-managed clojure" "clojure inline -e roundtrip"

ok "clojure ${VERSION}: install + cross-backend [java] merge + inline run"
