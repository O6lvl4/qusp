#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${JAVA_VERSION:-21}"
DISTRIBUTION="${JAVA_DISTRIBUTION:-temurin}"

isolate_qusp

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[java]
version = "$VERSION"
distribution = "$DISTRIBUTION"
EOF

step "install (parallel form, picks up [java] distribution)" run_qusp install

# `java -version` writes to stderr; merge.
out=$(capture_qusp run java -version 2>&1)
assert_contains "$out" "21" "java -version mentions Java 21"
assert_contains "$out" "Temurin" "java -version mentions the pinned distribution"

# Tool routing: mvn (sha512) and gradle (sha256). Both must succeed.
step "add tool mvn" run_qusp add tool mvn
out=$(capture_qusp run mvn --version)
assert_contains "$out" "Apache Maven" "mvn --version"

step "add tool gradle" run_qusp add tool gradle
out=$(capture_qusp run gradle --version)
assert_contains "$out" "Gradle" "gradle --version"

ok "java ${VERSION} (${DISTRIBUTION}): install + run + mvn + gradle"
