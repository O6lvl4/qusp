# Dogfood Papercuts (live-collected)

> 2026-04-29 dogfood session — qusp daily-driver の qusp 自身リポジトリでの初日。

各 papercut に **音感** (痛みの強さ) と **既存 audit row との対応** を付記。新規 row の場合は doc 化候補。

---

## P1: rust 1.85.0 default が ecosystem 最新 (1.86+) より古い ⚠️

**音感: 中-高 (新規ユーザの最初の cargo build が落ちる)**

`script.rs::default_version("rust") == "1.85.0"` だが、現代の crate
ecosystem は icu_properties@2.2.0 等で MSRV 1.86 が要求される。`qusp x ./hello.rs`
や `qusp init --langs rust` で新規ユーザに渡るのは 1.85.0 → 即 cargo
で MSRV エラー。

**Fix:** `default_version` table を release prep で更新する手順を確立、
あるいは `"latest-stable"` 風の実 latest 解決にする。後者は cmd_init や
script.rs から離れた pure resolver として整備可能。

**Audit row 関連:** 既存なし → **新規候補 default-versions-currency.md** 作成 OR
`Phase 5 release prep checklist` の一部にする。

---

## P2: `qusp install rust stable` (with arg) が lock を更新しない ⚠️

**音感: 中 (sync を別途叩く必要があると気づくまで時間)**

v0.28.0 の F4 fix は `qusp install` (no-args, manifest 経由) のみ対象だった。
`qusp install <lang> <ver>` (引数あり) は依然 lock を触らない。

**Fix:** `cmd_install` の引数あり経路でも単一 backend について lock を upsert する。
`distribution` も単一引数経路では当時 manifest 由来で取ってる、同様に lock 化。

**Audit row 関連:** F4 の sub-fix。同 doc に追記、または `lock-on-arged-install.md` 新設。

---

## P3: `qusp current` の `(from rust-toolchain.toml)` vs qusp.toml の二重表示 🤔

**音感: 低 (機能的には正しい、見た目だけ)**

qusp.toml で `rust = "stable"` と書いてるのに `qusp current rust` は
`(from rust-toolchain.toml)` と表示する。両者が一致してるので動作は
正しいが、**どっちが優先か表示されない**。

**Fix:** `qusp current` の出力に "X (from A; matches qusp.toml)" 形式で
両 source 表示。Audit row B3 (Resolve current) に既存対応 doc あり、これに追記。

**Audit row 関連:** B3 / `current-resolved-path.md`

---

## P4: `qusp outdated` が rolling channel を理解しない ⚠️

**音感: 中-高 (false positive で「stable は古い」と表示)**

`[rust] version = "stable"` を pin してると `outdated` は `stable → 1.95.0`
と表示。ROLLING channel と semver pin の区別がついてない。Pin が `stable`
なら常に最新を意味するので outdated はあり得ないが、qusp は
literal string 比較してる。

**Fix:** `outdated` で rolling channel keyword (`stable`, `beta`, `nightly`,
`latest`) を pre-filter。
backend に `is_rolling_channel(s) -> bool` を追加するのが筋。

**Audit row 関連:** 既存なし → **新規候補 rolling-channel-handling.md**。
Phase 5 hospitality 分類だが outdated 限定の small fix。

---

## P5: `outdated` の文法 single/plural mismatch 🤏

**音感: 極低**

`1 toolchain have newer upstream versions.` ─ "1 toolchain HAVE" は文法ミス。

**Fix:** `cmd_outdated` の format string を `if hits == 1 { "has" } else { "have" }`
で出し分け。1-line patch。

**Audit row 関連:** 軽 hospitality、別 row 不要。

---

## P6: `qusp x /tmp/hello.rs` が tool dispatch にフォールスルーして誤誘導 ⚠️

**音感: 中 (ユーザは file を渡したのに tool 名扱いされる)**

`.rs` 拡張子は extension routing 表に無い (rust scripts は意図的 unsupport)。
そこで tool dispatch にフォールスルーし、エラーは
`no backend recognized tool '/tmp/hello.rs'` ─ tool 名扱いの error message。

**Fix:** `cmd_x` の routing 時、argv[0] が **存在するファイル** で、その
**拡張子が extension table に無い** 場合は専用エラー:
"qusp x doesn't support .rs scripts (rust requires cargo project context).
Did you mean `qusp run cargo run ...`?"

**Audit row 関連:** D1 (cold-cache run) と K1 (error syntax) の交差点。
`x-script-routing` (done) の followup として `done/x-script-routing.md` の
"Known limitation" セクションに記録 + `qusp x` 改善 doc 作成。

---

## P7: `qusp install rust 99.99.99` の error が raw HTTP 404 URL ⚠️

**音感: 中 (uv 比較で audit 既知)**

```
error: fetch https://static.rust-lang.org/dist/rust-99.99.99-x86_64-apple-darwin.tar.gz.sha256:
       response error for https://static.rust-lang.org/dist/rust-99.99.99-...:
       HTTP status client error (404 Not Found) for url (...)
```

ユーザに「99.99.99 という rust は存在しません。rust list --remote から選んで」
と返すべき。

**Fix:** Audit row A5/A5b/K1 の `did-you-mean-cross-backend.md` で扱う。
Phase 5 high priority に既登録。dogfood で実需確認 → 優先度上昇。

**Audit row 関連:** A5 / A5b / K1 / `did-you-mean-cross-backend.md`

---

## P8: `qusp run pnpm` が pinning 無しでも system PATH の pnpm を実行する 🤔

**音感: 低-中 (期待動作だが surprising)**

pnpm は qusp.toml で pin してないが、`qusp run pnpm --version` は system
の pnpm (homebrew/mise 経由) を呼んで成功する。

これは `qusp run` の正しい動作 (PATH を temporary に prepend するだけ、
system command は通る)。ただし、ユーザ視点で
「qusp が pnpm を管理してるのか system のを使ってるのか」が不明。

**Fix:** 不要。dogfood で誰かが confused になったら Phase 5 hospitality
の `qusp doctor` に "command resolution: <cmd> → <path>" trace を出す経路を検討。

**Audit row 関連:** 不要。

---

## P9: `qusp doctor` が "rust : 2 installed" を返す ─ 1.85.0 + stable 🤏

**音感: 極低 (技術的には正しい、stable は別 dir なので別 install)**

qusp の rust backend は `1.85.0` と `stable` を別 install dir に置く
(`data/rust/1.85.0` vs `data/rust/stable`)。doctor は dir 数 = installed
count なので "2 installed"。stable は alias っぽい性質なので解釈は微妙。

**Fix:** 不要。意図通り。doctor の text 表示 + JSON 出力で `1.85.0 + stable`
の中身が見えるように list で詳細出すなら Phase 5 B1 (list-remote-richness.md) で
扱える。

**Audit row 関連:** B1。

---

## P10: shellenv の `_QUSP_LAST_KEYS` が単一 var で comma-sep 想定っぽい 🤏

**音感: 極低**

`export _QUSP_LAST_KEYS=RUSTUP_TOOLCHAIN`. 単一値ならいいが複数 lang
pin で複数 env var 出るときは comma-separated になる想定 (おそらく既存
そう書かれてる)。現状 1 言語だけなので問題なし。

**Fix:** 多言語 manifest を pin したら確認。今は OK。

**Audit row 関連:** dogfood multi-lang 環境で再観察。

---

## P11: `qusp current` が "from global" と出す go の出処不明 🤔

**音感: 低**

qusp.toml に go 無し、`go.mod` 無し、`.go-version` 無し。それでも
`qusp current` で go = `go1.26.2 (from global)`。gv-core 内部の
"global" detect が走ってる。

**Fix:** `(from global)` の意味が user に伝わらない。doctor / current の
explanation を具体化:
- `(from global)` → `(from $HOME/.go/version)` 等、実 source path を表示。

**Audit row 関連:** B3 / `current-resolved-path.md` の既存 doc に追記。

---

## P12-15: minor 整形 / wording

- P12: `qusp tree` の manifest と resolution 行の繋がりが視覚的に薄い (`├──` と `└──` が混在気味)
- P13: install success メッセージ `→ /Users/.../qusp/rust/stable` の path が長すぎてターミナル折り返し
- P14: `qusp.toml` の init template に rust 例が `version = "1.85.0"` (P1 と同じ)
- P15: `qusp doctor` の `qusp : 0.28.1` 行と `data dir : ...` の縦揃え微妙

全部 wording / display polish。Phase 5 後半の Q1 (help richness) でまとめて。

---

## 集計

| Priority | 数 | Audit row 既存 | 新規 doc 候補 |
|---|---|---|---|
| 中-高 ⚠️ | **3** (P1, P4, P6) | P1 → 新規 / P4 → 新規 / P6 → done/x-script-routing followup | 2 件 |
| 中 ⚠️ | **3** (P2, P3, P7) | P2 → F4 sub-fix / P3 → B3 / P7 → A5 既存 | 0 件 (既存 doc に追記) |
| 低 🤔 | **2** (P8, P11) | P11 → B3 既存 / P8 → 不要 | 0 件 |
| 極低 🤏 | **5** (P5, P9, P10, P12-15) | misc polish | 0 件 |

**実需で炙り出された "1.0 前に絶対 fix" Tier 1 (3 件):**
- P1 default rust version currency
- P4 rolling channel outdated false positive
- P6 qusp x の `.rs` (extension table 外) error message 改善

**1.0 前に推奨 Tier 2 (3 件):**
- P2 install with-arg の lock 更新
- P3 / P11 current の source 表示
- P7 did-you-mean fuzzy 全 backend

これらを潰してから dogfood Step 2 (almide / gv / rv 等の他 repo に展開) に進む。
