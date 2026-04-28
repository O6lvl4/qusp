#!/usr/bin/env bash
# qusp vs mise — `<tool> version` invocation latency.
#
# Measures the cold and warm-cache path through each manager:
#   - qusp run go version           — qusp's only mode (PATH-injection at exec time)
#   - mise exec -- go version       — mise's exec mode (similar shape to qusp run)
#   - mise's shim                   — `~/.local/share/mise/shims/go version` (what users actually
#                                     hit when `mise activate` is in their rcfile)
#
# Both managers must already have go 1.26.2 installed:
#   qusp install go 1.26.2
#   mise install go@1.26.2
#
# Reports a hyperfine table. Run with `--save FILE` to persist.

set -euo pipefail

if ! command -v hyperfine >/dev/null 2>&1; then
    echo "bench: hyperfine not found. Install with \`brew install hyperfine\`." >&2
    exit 1
fi

self_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
qusp_bin="${QUSP_BIN:-$self_dir/../target/release/qusp}"
mise_bin="${MISE_BIN:-/Users/o6lvl4/.local/bin/mise}"

[ -x "$qusp_bin" ] || { echo "bench: $qusp_bin not built. Run \`cargo build --release\`." >&2; exit 1; }
[ -x "$mise_bin" ] || { echo "bench: $mise_bin not found. Set MISE_BIN= or install mise." >&2; exit 1; }

# Set up isolated project dirs that pin the same go version through each tool.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/qusp-proj" "$work/mise-proj"
cat > "$work/qusp-proj/qusp.toml" <<'EOF'
[go]
version = "1.26.2"
EOF
cat > "$work/mise-proj/mise.toml" <<'EOF'
[tools]
go = "1.26.2"
EOF
# mise refuses to read config files unless they're explicitly trusted.
"$mise_bin" trust "$work/mise-proj/mise.toml" >/dev/null 2>&1 || true

# Verify each manager actually resolves to go 1.26.2 (so we're not measuring a typo).
echo "── sanity check ──"
( cd "$work/qusp-proj" && "$qusp_bin" run go version )
( cd "$work/mise-proj" && "$mise_bin" exec -- go version )

# Locate the mise shim if it exists.
mise_shim=""
if [ -x "$HOME/.local/share/mise/shims/go" ]; then
    mise_shim="$HOME/.local/share/mise/shims/go"
elif [ -x "$mise_bin" ]; then
    candidate="$( "$mise_bin" where go 2>/dev/null | head -1 )"
    [ -n "$candidate" ] && [ -x "$candidate/bin/go" ] && mise_shim="$candidate/bin/go"
fi

# Hyperfine warmups + 50 runs per command. Each command is a fresh process.
echo
echo "── hyperfine ──"
cmds=(
    -n "qusp run go version"   "cd $work/qusp-proj && $qusp_bin run go version"
    -n "mise exec go version"  "cd $work/mise-proj && $mise_bin exec -- go version"
)
if [ -n "$mise_shim" ]; then
    cmds+=( -n "mise shim go version" "$mise_shim version" )
fi

hyperfine \
    --warmup 5 \
    --runs 50 \
    --shell=none \
    --time-unit millisecond \
    "${cmds[@]}"
