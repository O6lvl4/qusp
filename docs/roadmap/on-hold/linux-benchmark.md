# Linux Benchmark

**優先度:** Phase 2 (1.x)
**前提:** scripts/bench.sh 既存 (macOS-13 x86_64 の数値あり)

## やること

1. e2e workflow に bench job を追加 (workflow_dispatch のみ、cron では無し — flaky)
2. ubuntu-latest と macos-14 (arm64) で同じ bench を流す
3. 結果を `docs/bench/` に json で commit
4. README の latency 表を 3 platform に拡張

## 期待される値

予想:
- Linux x86_64: macOS と近い
- macOS arm64 (M1 / M2 / M3): startup が早い分、qusp run はもっと速い (~5 ms?)
