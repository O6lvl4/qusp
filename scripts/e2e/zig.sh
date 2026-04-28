#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

VERSION="${ZIG_VERSION:-0.16.0}"

isolate_qusp
step "install zig ${VERSION}" run_qusp install zig "$VERSION"

mkdir -p "$HOME/proj" && cd "$HOME/proj"
cat > qusp.toml <<EOF
[zig]
version = "$VERSION"
EOF

out=$(capture_qusp run zig version)
assert_eq "$out" "$VERSION" "zig version"

# Compile + run a Zig source file end-to-end. Validates that the unified
# bin layout + lib/std/ symlink chain survived the xz extract.
cat > hello.zig <<'EOF'
const std = @import("std");
pub fn main() !void {
    std.debug.print("hi from qusp-managed zig\n", .{});
}
EOF
out=$(capture_qusp run zig run hello.zig 2>&1)
assert_contains "$out" "hi from qusp-managed zig" "compiled zig program ran"

ok "zig ${VERSION}: install + run + compile"
