#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# macOS: erlef/otp_builds (fully supported). Linux: builds.hex.pm
# (experimental, glibc-only) — opt in with QUSP_E2E_LINUX=1 so default CI
# stays green on the unvalidated runtime.
case "$(uname -s)" in
  Darwin) VERSION="${ERLANG_VERSION:-27.3.4.3}" ;;  # erlef/otp_builds patch tag
  Linux)
    [ "${QUSP_E2E_LINUX:-0}" = "1" ] \
      || skip "Linux erlang (builds.hex.pm) is experimental — set QUSP_E2E_LINUX=1 to test"
    VERSION="${ERLANG_VERSION:-27.3}"            # builds.hex.pm / erlang.org tag
    ;;
  *) skip "erlang prebuilds cover macOS and Linux(glibc) only" ;;
esac
MAJOR="${VERSION%%.*}"

isolate_qusp
step "install erlang ${VERSION}" run_qusp install erlang "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[erlang]
version = "$VERSION"
EOF

# erl runs and reports its OTP release through `qusp run`.
out=$(capture_qusp run erl -noshell -eval "io:format(\"otp=~s~n\",[erlang:system_info(otp_release)]), halt()." 2>&1)
assert_contains "$out" "otp=$MAJOR" "erl reports the pinned OTP major under qusp run"

# escript end-to-end — proves erlc/escript + relocation actually work.
cat > hello.erl <<'EOF'
#!/usr/bin/env escript
main(_) -> io:format("hi from qusp-managed erlang~n").
EOF
out=$(capture_qusp run escript hello.erl 2>&1)
assert_contains "$out" "hi from qusp-managed erlang" "escript roundtrip"

# Farm: pin globally, then run the BARE `erl` symlink with no qusp wrapper
# and no env. This is the path the find_rootdir relocation fix protects —
# the farm symlink lives outside the OTP root, so without the rewritten
# ROOTDIR fallback `erl` would chase the build-time `/tmp/otp-...` path.
step "pin erlang ${VERSION} (farm)" run_qusp pin set erlang "$VERSION"
[ -L "$HOME/.local/bin/erl" ] || fail "expected ~/.local/bin/erl symlink after pin"
out=$(env -i HOME="$HOME" PATH="/usr/bin:/bin" "$HOME/.local/bin/erl" \
    -noshell -eval "io:format(\"farm-otp=~s~n\",[erlang:system_info(otp_release)]), halt()." 2>&1)
assert_contains "$out" "farm-otp=$MAJOR" "bare farmed erl resolves its own ROOTDIR (no env)"

ok "erlang ${VERSION}: install + erl + escript + farm relocation"
