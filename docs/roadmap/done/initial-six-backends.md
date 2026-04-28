# Initial 6 Backends — Go / Ruby / Python / Node / Deno / Bun

**Shipped:** v0.1.0 → v0.4.0
**Tags:** v0.1.0..v0.4.0

最初の 6 言語を pure-Rust native backends として実装。
- gv-core / rv-core を Cargo deps として組み込み (subprocess freeloading 排除)
- python-build-standalone / nodejs.org / denoland / oven-sh の publisher CDN 直接、sha256 検証
- e2e で全 install/run/tool routing 実証
