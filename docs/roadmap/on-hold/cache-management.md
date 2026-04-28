# Cache management (`qusp cache clean / prune`)

**Phase 5 (Hospitality Parity)。**
**解決対象 audit row:** G2 ─ cache clean、G3 ─ cache prune

## なぜ

実測 (audit 2026-04-28):

uv は 3 つの cache subcommand を持つ:
- `uv cache dir` ─ パス表示
- `uv cache clean [PACKAGES...]` ─ 全削除 or 特定 pkg のみ
- `uv cache prune --ci` ─ unreachable な entry のみ削除 (CI optimized)

qusp は `qusp dir cache` のみ。`clean` / `prune` 相当無し。

実害:
- 18 言語 backend を活用すると content-addressed store (~/Library/...
  /qusp/store/<sha>/) は数 GB に膨らむ
- 古い toolchain version の install を消す手段が **`rm -rf <store>`**
  しかなく、これだと現在 lock してる install まで巻き込まれる
- CI で qusp を使うと cache 肥大が ramp up、cleanup を qusp 自身が
  提供しないと user 側で手で書く必要

uv の prune は特に賢く、**reachable analysis** で「現 lock の reference
が指してない artifact」のみ消す。qusp なら「全 manifest + 全 lock を
読んで、lock referenced 以外の store entry を削除」というのが直接
対応。

## 設計案

### `qusp cache dir`

既存 `qusp dir cache` の alias として追加。互換性維持しつつ uv 同形。

### `qusp cache clean [LANG...]`

```
$ qusp cache clean
✓ cleaned cache: 24 files, 1.2 GB freed

$ qusp cache clean python
✓ cleaned python cache entries: 3 files, 312 MB freed
```

`paths.cache` 配下 (downloaded tarball の一時 cache) と、引数指定された
lang の `paths.data/<lang>/` を全削除。`<lang>` 省略で全 cache。

注意: `clean` は **install 済み toolchain も消す**。次回 install で
再 download が必要になる。aggressive。

### `qusp cache prune`

```
$ qusp cache prune
analyzing reachability across 3 qusp.toml + 5 qusp.lock entries...
✓ kept 8 store entries (referenced by lock)
✓ pruned 12 unreachable entries: 2.1 GB freed
```

各 backend が `paths.store()/<sha-prefix>/` に hashed install dir を
持つ (現在の content-addressed store)。`qusp.lock` の `upstream_hash`
field と data dir の symlink target sha を突合して、どの store entry
が **どの lock からも参照されてないか** を計算 → 不要なら削除。

実装は orchestrator に reachable-set 計算を追加:

```rust
pub fn compute_reachable_store_entries(
    paths: &Paths,
    locks: &[Lock],
) -> HashSet<String> {
    let mut reachable = HashSet::new();
    for lock in locks {
        for backend_lock in lock.backends.values() {
            // Read the symlink at data/<lang>/<v>/ to get the
            // canonical store dir, extract the sha prefix.
            ...
        }
    }
    reachable
}
```

`--ci` flag は uv と同形 (CI 最適化、より conservative にする
─ debug entry を残す等)。

### `qusp cache prune --dry-run`

実削除前に何が消えるか preview。default `prune` 動作に preview を
含める案 (`--yes` で skip) も検討。

## 設計上の悩み

- **multi-machine reachability**: ユーザが複数 project / 複数 cwd を
  持ってる場合、`prune` は **全 qusp.lock を発見できない**。`~/Library/
  .../qusp/known-projects.txt` 風の registry を持つか、`--include
  <path>` で明示する形か。
- **partial install の半端 entry**: install 途中で死んだ tmp 状態の
  store entry はどう扱うか。reachable でも lock referenced でもない、
  かつ orphan。`--orphans-only` flag で gc 可能に。
- **cache dir size 表示**: `qusp doctor` に "cache: 3.2 GB across N
  toolchains" 行を追加、user に prune 時期を促す。

## 非ゴール

- 自動 GC (cron / launchd schedule) ─ user explicit 動作で十分
- network cache の TTL 管理 ─ それは publisher の HTTP cache header
  で十分

## 実装ステップ

1. orchestrator に `compute_reachable_store_entries` pure function
2. `qusp cache` subcommand 追加 (`dir` / `clean` / `prune`)
3. `cmd_cache_*` 実装、`--dry-run` / `--ci` / `--yes` flags
4. unit test (mock paths + mock locks)
5. e2e: 複数 install → prune dry-run → 期待 set 一致 → prune 実行 → 確認
6. `qusp doctor` に cache size summary 追加
