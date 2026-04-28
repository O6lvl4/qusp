# Benchmark vs mise

**Shipped:** scripts/bench.sh (commit c215740)

hyperfine 50-run, 5 warmup, --shell=none, macOS-13 x86_64:

| Mode | Mean | User+Sys CPU |
|---|---|---|
| `qusp run go version` | 12.0 ms | 9 ms |
| `mise exec go version` | 12.1 ms | 9 ms |
| `mise shim go version` | 49.4 ms | 39 ms |

mise の `activate` 経由 (shim) は qusp run / mise exec の **約 4 倍遅い**。
qusp は shim 層を持たない設計上の選択がそのまま latency 優位に変換される。

README "Latency" セクションに数値添付。
