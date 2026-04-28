# Daily Dogfood + 1.0 Release

**優先度:** Phase 1 完成の最後の 1 ピース
**前提:** v0.14.0 完了 (DDD migration / 9 言語 / e2e / benchmark / docs 揃い)
**原則:** 機能追加じゃなく **使って papercut を拾う**。気持ちよく使えなければ 1.0 とは言わない。

---

## ゴール

著者本人が mise を解除して qusp を daily driver にした状態で:

1. 1 週間連続で困らずに使える
2. CI / 本物のプロジェクトで `qusp sync --frozen` が通る
3. 公開して「使って」と言える整合性がある

そこで初めて **v1.0.0 タグ + 公開アナウンス**。

---

## チェックリスト

### Step 1 — 環境差し替え

- [ ] `~/.zshrc` の `mise activate` を `eval "$(qusp hook --shell zsh)"` に
- [ ] mise の global tools を qusp.toml に翻訳して各プロジェクトに配置
- [ ] mise を `brew uninstall` (or shim/state を退避)
- [ ] `qusp doctor` で path 衝突が無いことを確認

### Step 2 — 実プロジェクトで使う

- [ ] **almide** (Rust workspace) — `qusp.toml` を rust 1.85.0 で固定、`cargo build` 普通に動く?
- [ ] **gv / rv / anyv-core** — Rust 単一クレート群、CI で `qusp sync --frozen` が通る?
- [ ] **何か Node プロジェクト** — pnpm 経由で実 install、`qusp run pnpm install` が問題ないか
- [ ] **何か Python プロジェクト** — uv ベース、qusp は interpreter だけ提供
- [ ] **多言語 monorepo** (もしあれば) — 複数 `[lang]` 同時 pin、cd ホップが想定通り

### Step 3 — Papercut 収集

毎日 papercut を `papercuts.md` (gitignore 推奨) に書き留める。例:

- [ ] `qusp shellenv` の `__QUSP_LAST_KEYS` リストが大きすぎてシェル env が太る、リファクタ?
- [ ] `qusp run` の error message が「spawn xxx: No such file or directory」だけだと迷う
- [ ] `qusp install` で network failure 時の retry が無い
- [ ] `qusp tree` の「(no pin detected)」が嘘かもしれない (manifest にあるのに detect が壊れてる?)
- [ ] その他、実際に出たもの

→ 全部 issue 化、優先度判定、本当に痛いものだけ 1.0 前に fix。

### Step 4 — Performance 体感

- [ ] `cd <repo>` した瞬間の shellenv 適用に体感ラグが無いか (~50 ms 以下?)
- [ ] `qusp run go test ./...` の startup overhead が無視できる範囲か
- [ ] 大量の `qusp run` を含むスクリプトで CPU 時間がふくらまないか

### Step 5 — Public-facing 整備

- [ ] README の install 節に "via brew" / "via cargo" 両方
- [ ] CHANGELOG.md (or `git log --oneline` の整理)
- [ ] `qusp --help` 出力をもう一度精読、奇妙な wording を直す
- [ ] LICENSE / CONTRIBUTING / SECURITY ファイル
- [ ] release note v1.0.0 草稿

### Step 6 — v1.0.0 タグ

すべて緑になったら:

```bash
git tag v1.0.0
git push origin v1.0.0
```

release.yml が 5-platform バイナリと Homebrew tap bump を完走するのを待つ。

### Step 7 — 公開

- [ ] HN / lobste.rs / r/rust に短文投稿 (qusp の lane を一段落で説明、benchmark 数値 1 行、9 言語表 1 つ、`brew install O6lvl4/tap/qusp` 1 行)
- [ ] 個人ブログ / X (Twitter) で release thread
- [ ] 反応に応じて follow-up

---

## 非ゴール

- v1.0 で言語数を増やさない (Phase 4 で別扱い)
- v1.0 で plugin システムを作らない (永遠に作らない方針)
- v1.0 で sigstore は入れない (Phase 2)
- v1.0 で `qusp plan` を入れない (dogfood で需要が出れば判断)
- 機能追加は **dogfood で「これが無いと回らない」が出た時だけ**

---

## 終了条件

- 著者の手元に mise が無い状態が 1 週間以上続いている
- 実プロジェクト 3 個以上で `qusp sync --frozen` が CI で安定通過
- papercut が「将来でいい」しか残ってない
- 公開後 48 時間でクラッシュバグが出ていない
