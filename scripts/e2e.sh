#!/usr/bin/env bash
# qusp end-to-end test driver.
#
# Usage:
#   scripts/e2e.sh                    # run every test
#   scripts/e2e.sh go rust            # run a subset
#   scripts/e2e.sh --fast             # skip the slow ones (java, ruby)
#
# Env:
#   QUSP_BIN=path/to/qusp             # binary to test (default: target/release/qusp, then PATH)
#   E2E_VERBOSE=1                     # don't suppress per-step output
#   E2E_KEEP=1                        # don't clean temp HOMEs (for debugging)
#   E2E_SKIP_RUBY=1                   # skip ruby (compile-from-source flake)
set -euo pipefail

self_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
e2e_dir="$self_dir/e2e"

# Order matters: smoke first (fast, no network beyond GitHub releases for
# self-update), then fast toolchain installs, then slow ones.
DEFAULT_TESTS=(smoke go node deno bun rust python java kotlin groovy scala clojure zig julia crystal dart lua haskell install_hardening x_script inline_metadata farm json_output ruby)
FAST_TESTS=(smoke go node deno bun rust python zig julia crystal dart lua install_hardening x_script inline_metadata farm json_output)

fast=0
specific=()
for a in "$@"; do
    case "$a" in
        --fast) fast=1 ;;
        -h|--help)
            grep '^#' "$0" | sed 's/^# //; s/^#//'
            exit 0
            ;;
        *) specific+=("$a") ;;
    esac
done

if [ "${#specific[@]}" -gt 0 ]; then
    tests=("${specific[@]}")
elif [ "$fast" = 1 ]; then
    tests=("${FAST_TESTS[@]}")
else
    tests=("${DEFAULT_TESTS[@]}")
fi

pass=0
fail=0
skip=0
fail_list=()
skip_list=()

for t in "${tests[@]}"; do
    script="$e2e_dir/$t.sh"
    if [ ! -f "$script" ]; then
        echo "e2e: unknown test '$t' (no $script)" >&2
        exit 2
    fi
    echo
    echo "─── running $t ───"
    set +e
    bash "$script"
    rc=$?
    set -e
    case "$rc" in
        0)  pass=$((pass + 1)) ;;
        77) skip=$((skip + 1)); skip_list+=("$t") ;;
        *)  fail=$((fail + 1)); fail_list+=("$t") ;;
    esac
done

echo
echo "═══ summary ═══"
echo "  pass: $pass"
echo "  skip: $skip${skip_list:+  (${skip_list[*]})}"
echo "  fail: $fail${fail_list:+  (${fail_list[*]})}"
echo

if [ "$fail" -gt 0 ]; then
    exit 1
fi
exit 0
