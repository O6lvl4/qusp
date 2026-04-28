#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${RUBY_VERSION:-3.4.7}"

# ruby-build needs a working C compiler + openssl + libyaml. macOS 13
# x86_64 in particular has been flaky; allow caller to skip explicitly.
if [ "${E2E_SKIP_RUBY:-0}" = "1" ]; then
    skip "E2E_SKIP_RUBY=1"
fi
if ! command -v cc >/dev/null && ! command -v gcc >/dev/null; then
    skip "no C compiler in PATH (ruby-build won't work)"
fi

isolate_qusp
step "install ruby ${VERSION} (compiles from source — slow)" \
    run_qusp install ruby "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[ruby]
version = "$VERSION"
EOF

out=$(capture_qusp run ruby --version)
assert_contains "$out" "ruby ${VERSION}" "ruby --version"

ok "ruby ${VERSION}: install (via ruby-build) + run"
