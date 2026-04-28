#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# v0.28.0 install-path hardening:
#   W1 — concurrent installs of same lang+version serialise via flock
#   F4 — `qusp install` (no-args) writes qusp.lock on success
#   atomic_symlink_swap — install_dir symlink replaces atomically

isolate_qusp

# F4: `qusp install` no-args produces a qusp.lock
mkdir -p "$HOME/proj-f4" && cd "$HOME/proj-f4"
cat > qusp.toml <<EOF
[zig]
version = "0.16.0"
EOF
[ ! -f qusp.lock ] || fail "F4 setup: qusp.lock already exists"
step "F4: qusp install (no-args) creates qusp.lock" run_qusp install
[ -f qusp.lock ] || fail "F4 broken: qusp install did not produce qusp.lock"
assert_contains "$(cat qusp.lock)" "0.16.0" "qusp.lock contains pinned version"
assert_contains "$(cat qusp.lock)" "[zig]" "qusp.lock has [zig] section"

# W1: install lock file exists alongside install_dir
data_dir=$(capture_qusp dir data)
lock_file="$data_dir/zig/0.16.0.qusp-lock"
[ -f "$lock_file" ] || fail "W1 broken: install lock not created at $lock_file"

# atomic_symlink_swap: install_dir is a symlink (not a regular dir)
install_dir="$data_dir/zig/0.16.0"
[ -L "$install_dir" ] || fail "atomic swap: install_dir should be a symlink"

# W1 contention: a second `qusp install` of same lang+version blocks on
# the lock. We can't easily race two qusp processes from a single bash
# script with timing precision, but we can verify the lock-file
# acquire/release works by hot-running it twice in series — must stay
# idempotent (already-present skip path).
step "W1: second invocation idempotent under lock" run_qusp install zig 0.16.0

# F4 partial-success edge: a multi-lang install where one fails should
# still write the surviving entries to lock. Use a deliberately-bad
# version on one entry while the other succeeds.
mkdir -p "$HOME/proj-partial" && cd "$HOME/proj-partial"
cat > qusp.toml <<EOF
[zig]
version = "0.16.0"

[crystal]
version = "99.99.99"
EOF
# Expect non-zero exit (one backend fails) but lock still includes zig.
set +e
run_qusp install
rc=$?
set -e
[ "$rc" -ne 0 ] || fail "expected partial-success exit non-zero"
[ -f qusp.lock ] || fail "F4 partial: qusp.lock missing despite zig success"
assert_contains "$(cat qusp.lock)" "[zig]" "partial qusp.lock has [zig]"
# Crystal failed → may or may not appear; we don't assert on it.

ok "install hardening: F4 lock-on-success + W1 lock file + atomic symlink + partial-success persistence"
