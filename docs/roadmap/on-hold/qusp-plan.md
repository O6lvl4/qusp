# `qusp plan`

**優先度:** Phase 2 (1.x) — **dogfood で需要が出れば**
**前提:** Phase 2 で `domain::plan::plan_sync` が pure 関数として既に存在

## 問題 (というか問い)

terraform-plan 相当の dry-run を出したい誘惑がある。
DDD Phase 2 で `plan_sync` を pure に切ったので、UI に晒すのは 30 行で書ける。

ただし:
- `qusp sync` は既に「結果」を表示する
- 「事前に何が起きるか確認」需要は CI の `--frozen` でほぼカバー
- 残るユースケース: manifest 編集後に「install しないで diff だけ確認」

→ **dogfood で本当に欲しくなったらやる**。先回りはしない。

## もしやるなら

```
qusp plan
─── plan ───
+ install  rust 1.85.0 → 1.95.0       (bump pinned in qusp.toml)
+ install  bun 1.2.0                  (new)
- prune    node/eslint                (no longer in qusp.toml)
= keep     go 1.26.2
= keep     java temurin-21
─── 2 install, 1 prune, 2 keep ───
```

`SyncPlan` を pretty-print するだけ。HTTP 叩かない (plan 純粋)。
