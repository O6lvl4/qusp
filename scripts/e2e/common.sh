#!/usr/bin/env bash
# Shared helpers for qusp e2e tests.
#
# Each backend script sources this, then calls:
#   isolate_qusp           # set HOME to a temp dir so installs don't pollute
#   run_qusp <args...>     # invoke the qusp binary under test
#   assert_contains <output> <substring> <message>
#   skip "reason"          # exit 77 (TAP-style skip)
#   ok "message"           # green "ok" line
#   fail "message"         # red "fail" line + exit 1
#
# Test scripts must `set -euo pipefail` themselves; this file does not.

# Resolve the qusp binary under test. Honor $QUSP_BIN if exported (CI
# can pin it to e.g. target/release/qusp); otherwise look for the
# release build relative to this file, then fall back to PATH.
_e2e_self_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_e2e_repo_root="$(cd "$_e2e_self_dir/../.." && pwd)"
if [ -z "${QUSP_BIN:-}" ]; then
    if [ -x "$_e2e_repo_root/target/release/qusp" ]; then
        QUSP_BIN="$_e2e_repo_root/target/release/qusp"
    else
        QUSP_BIN="$(command -v qusp || true)"
    fi
fi
if [ -z "${QUSP_BIN:-}" ] || [ ! -x "$QUSP_BIN" ]; then
    echo "e2e: cannot find a qusp binary. Set QUSP_BIN, or run \`cargo build --release\` first." >&2
    exit 1
fi

# Colors. Disabled when stdout is not a tty.
if [ -t 1 ]; then
    _C_GREEN=$'\033[32m'
    _C_RED=$'\033[31m'
    _C_YELLOW=$'\033[33m'
    _C_DIM=$'\033[2m'
    _C_BOLD=$'\033[1m'
    _C_RESET=$'\033[0m'
else
    _C_GREEN=""; _C_RED=""; _C_YELLOW=""; _C_DIM=""; _C_BOLD=""; _C_RESET=""
fi

_E2E_TEST_NAME="${E2E_TEST_NAME:-$(basename "$0" .sh)}"

ok()   { echo "${_C_GREEN}✓${_C_RESET} ${_E2E_TEST_NAME}: $*"; }
fail() { echo "${_C_RED}✗${_C_RESET} ${_E2E_TEST_NAME}: $*" >&2; exit 1; }
skip() { echo "${_C_YELLOW}⊘${_C_RESET} ${_E2E_TEST_NAME}: skip — $*"; exit 77; }
info() { echo "${_C_DIM}…${_C_RESET} ${_E2E_TEST_NAME}: $*"; }

_E2E_TMPDIRS=()

# Set HOME to an isolated temp dir so qusp/gv/rv installs don't touch
# the user's real `~/Library/Application Support/dev.O6lvl4.*` (or the
# Linux XDG equivalent). Both `qusp` itself and the gv/rv libraries it
# pulls in honour HOME.
isolate_qusp() {
    local tmp
    tmp="$(mktemp -d)"
    _E2E_TMPDIRS+=("$tmp")
    export HOME="$tmp"
    # Some HTTPS clients still consult the user's CA bundle via SSL_CERT_FILE,
    # so don't clobber that. We only redirect data/cache/config dirs.
    export XDG_DATA_HOME="$tmp/.local/share"
    export XDG_CACHE_HOME="$tmp/.cache"
    export XDG_CONFIG_HOME="$tmp/.config"
    info "isolated under HOME=$tmp"
}

cleanup_qusp() {
    # Stash and restore exit code so cleanup never masks the script's
    # actual result (and never makes a passing test look like a failure).
    local rc=$?
    if [ "${E2E_KEEP:-0}" = "1" ]; then
        info "E2E_KEEP=1 — leaving ${_E2E_TMPDIRS[*]}"
        return "$rc"
    fi
    for d in "${_E2E_TMPDIRS[@]:-}"; do
        if [ -n "${d:-}" ] && [ -d "$d" ]; then
            # Go's module cache chmods deps read-only so users can't
            # accidentally edit them. Restore write bits before rm.
            chmod -R u+w "$d" 2>/dev/null || true
            rm -rf "$d" 2>/dev/null || true
        fi
    done
    return "$rc"
}
trap cleanup_qusp EXIT

run_qusp() {
    "$QUSP_BIN" "$@"
}

# Run qusp, capture stdout, return both exit code and the captured text.
# Usage: out=$(capture_qusp install go 1.26.2)
capture_qusp() {
    "$QUSP_BIN" "$@" 2>&1
}

assert_contains() {
    local haystack="$1" needle="$2" msg="${3:-output should contain '$2'}"
    if [[ "$haystack" == *"$needle"* ]]; then
        return 0
    fi
    echo "${_C_RED}assert_contains failed:${_C_RESET} $msg" >&2
    echo "  expected substring: $needle" >&2
    echo "  actual output:" >&2
    echo "$haystack" | sed 's/^/    /' >&2
    exit 1
}

assert_eq() {
    local got="$1" want="$2" msg="${3:-}"
    if [ "$got" = "$want" ]; then
        return 0
    fi
    echo "${_C_RED}assert_eq failed:${_C_RESET} ${msg:-values should match}" >&2
    echo "  got:  $got" >&2
    echo "  want: $want" >&2
    exit 1
}

# Run a step with a header line, fail on non-zero, hide output unless E2E_VERBOSE=1.
step() {
    local label="$1"; shift
    info "$label"
    if [ "${E2E_VERBOSE:-0}" = "1" ]; then
        "$@"
    else
        local out
        if ! out=$("$@" 2>&1); then
            echo "$out" >&2
            fail "$label failed"
        fi
    fi
}
