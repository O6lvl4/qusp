# Release Infra — Matrix CI + Homebrew Tap

**Shipped:** v0.6.0
**Tag:** v0.6.0

- `.github/workflows/release.yml` — tag-triggered, 5 platform matrix
  (aarch64/x86_64-apple-darwin, aarch64/x86_64-unknown-linux-musl, x86_64-pc-windows-msvc)
- Linux arm64 は `cross` で cross-compile
- SHA256SUMS publish, per-asset sha256 sidecars
- `O6lvl4/homebrew-tap` への formula 自動 bump (TAP_DEPLOY_KEY)
- packaging/homebrew/qusp.rb.template

`brew install O6lvl4/tap/qusp` 一行で global install 可能。
