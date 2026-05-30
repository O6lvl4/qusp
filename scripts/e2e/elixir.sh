#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# Elixir is arch-independent BEAM bytecode (no OS gate of its own); it
# rides on the Erlang backend. macOS is fully supported; Linux erlang
# (builds.hex.pm) is experimental — opt in with QUSP_E2E_LINUX=1.
VERSION="${ELIXIR_VERSION:-1.18.4}"
case "$(uname -s)" in
  Darwin) ERLANG_VERSION="${ERLANG_VERSION:-27.3.4.3}" ;;
  Linux)
    [ "${QUSP_E2E_LINUX:-0}" = "1" ] \
      || skip "Linux erlang (builds.hex.pm) is experimental — set QUSP_E2E_LINUX=1 to test"
    ERLANG_VERSION="${ERLANG_VERSION:-27.3}"
    ;;
  *) skip "erlang prebuilds cover macOS and Linux(glibc) only" ;;
esac

isolate_qusp

# Cross-backend dep: [elixir] without [erlang] is rejected at validation.
mkdir -p "$HOME/proj-no-erlang" && cd "$HOME/proj-no-erlang"
cat > qusp.toml <<EOF
[elixir]
version = "$VERSION"
EOF
err=$(capture_qusp install || true)
assert_contains "$err" "requires [erlang]" \
  "elixir without [erlang] errors with cross-backend message"

# With both pinned, `qusp install` must install erlang FIRST (dependency
# layering) so elixir's install-time OTP-major probe finds an OTP to match.
mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[erlang]
version = "$ERLANG_VERSION"

[elixir]
version = "$VERSION"
EOF
step "install (erlang → elixir, dependency-ordered)" run_qusp install

out=$(capture_qusp run elixir --version 2>&1)
assert_contains "$out" "$VERSION" "elixir --version reports the pinned version"

# mix shells out to erl at run time — this proves the cross-backend PATH
# merge (elixir's launchers find the qusp-managed Erlang runtime).
out=$(capture_qusp run mix --version 2>&1)
assert_contains "$out" "Mix $VERSION" "mix runs (Erlang runtime resolved via env merge)"

# End-to-end: evaluate a tiny Elixir expression.
out=$(capture_qusp run elixir -e 'IO.puts("hi from qusp-managed elixir")' 2>&1)
assert_contains "$out" "hi from qusp-managed elixir" "elixir -e roundtrip"

ok "elixir ${VERSION}: cross-backend dep + ordered install + mix + run"
