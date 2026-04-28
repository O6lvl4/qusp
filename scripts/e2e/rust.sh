#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${RUST_VERSION:-1.85.0}"

isolate_qusp
step "install rust ${VERSION}" run_qusp install rust "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[rust]
version = "$VERSION"
EOF

# Validates the install.sh-equivalent component merge: rustc, cargo,
# rustdoc must all be present in the unified bin/.
out=$(capture_qusp run rustc --version)
assert_contains "$out" "rustc ${VERSION}" "rustc --version"

out=$(capture_qusp run cargo --version)
assert_contains "$out" "cargo" "cargo --version"

# Compile something tiny so we know the std lib was actually merged.
cat > main.rs <<'EOF'
fn main() { println!("hi"); }
EOF
out=$(capture_qusp run rustc main.rs -o hello)
test -x ./hello || fail "rustc didn't produce the binary"
out=$(./hello)
assert_eq "$out" "hi" "compiled rust binary runs"

ok "rust ${VERSION}: install + rustc + cargo + actual compilation"
