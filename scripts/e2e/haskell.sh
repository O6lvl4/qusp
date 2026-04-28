#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Haskell ships via ghcup which downloads ~150MB of GHC binaries on a
# fresh install. Allow override of GHC version (default to recent
# stable) and let CI skip if the user explicitly opts out.
if [ "${E2E_SKIP_HASKELL:-0}" = "1" ]; then
    skip "E2E_SKIP_HASKELL=1"
fi

VERSION="${HASKELL_VERSION:-9.10.1}"

isolate_qusp
step "install haskell ${VERSION} (downloads GHC via ghcup, ~150 MB)" run_qusp install haskell "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[haskell]
version = "$VERSION"
EOF

out=$(capture_qusp run ghc --version 2>&1)
assert_contains "$out" "$VERSION" "ghc --version reports the pinned version"

# End-to-end: compile + run a Haskell source file via runghc.
cat > Hello.hs <<'EOF'
main :: IO ()
main = putStrLn "hi from qusp-managed haskell"
EOF
out=$(capture_qusp run runghc Hello.hs 2>&1)
assert_contains "$out" "hi from qusp-managed haskell" "runghc roundtrip"

ok "haskell ${VERSION}: ghcup wrap + GHC install + runghc roundtrip"
