# Hospitality Parity (Phase 5)

> 「**uv 並のホスピタリティを 18+ 言語全部に。**」

---

## Audit snapshot

- **Date:** 2026-04-28
- **uv:** 0.8.12 (36151df0e 2025-08-18)
- **qusp:** 0.24.0
- **Host:** macOS (darwin x86_64)
- **方法:** 各カテゴリで uv と qusp を実コマンドで叩き、出力 / 時間を取得。
  cold-cache のシナリオは isolated `HOME` + `UV_CACHE_DIR` / `XDG_DATA_HOME`
  で疑似 fresh-machine を作って計測。
- **更新:** uv が major upgrade した時、qusp が新機能を出した時の 2 タイミングで再 audit。
  古い snapshot は git history で復元可。

---

## Why Phase 5 = Hospitality (not Reproducibility)

v0.23.0 までで mise/asdf 比較における qusp の競争 position が固まった:

| 軸 | mise | qusp v0.23 | 差 |
|---|---|---|---|
| 対応言語 native 実装 | plugin 任せ | 全 native Rust | qusp 圧勝 |
| install 検証必須 | plugin 任せ | sha256 一律必須 | qusp 圧勝 |
| cross-backend dep | 無 | `requires=["java"]` 機構 | qusp 圧勝 |
| shim 速度 | ~10ms | ~2ms (4×) | qusp 軽勝 |
| 対応言語数 | ~50 | 18 | mise 勝ち |
| **task runner / env / hospitality** | **mise > qusp** | **qusp 不在** | **mise 勝ち** |

uv は方向違い: Python 単体に対する深さ。qusp は 18 言語横断の広さ。

**「uv が Python 1 つに対してやってる ergonomic 密度を 18+ 言語全部に拡張する」 ─ この position は誰も取ってない。**

これが Phase 5 の定義。旧 Phase 5 (Reproducibility & Nix Bridge) は Phase 6 へ後ろ倒し。

---

## uv 並判定表 (audit 結果)

凡例:
- ✅ **Parity**: uv と同等または上
- 🟡 **Partial**: 動くが質で劣る
- ❌ **Missing**: 機能不在
- N/A: uv 専用 / qusp の方向と合わず計測不要

### A. Runtime install (toolchain)

| ID | 項目 | uv | qusp | 判定 |
|---|---|---|---|---|
| A1 | Cold install latency (Python 3.11.13、isolated HOME) | 2.28s download → install (17.5 MiB) | ~2-3s 同等 | ✅ Parity |
| A2 | Warm install (idempotent) | 0.05s no-op | ~0.05s 同等 (`already_present`) | ✅ Parity |
| A3 | Progress display during install | 1 line "Downloading cpython-3.11.13 (17.5MiB)" + "Installed Python 3.11.13 in 2.28s" | bare success line、download 中 silent | ❌ Missing |
| A4 | sha verification の透明性 | publisher trust (PyPI / python-build-standalone) | 全 install で表示無し but 検証は実施 | 🟡 Partial (見えてない) |
| A5 | Error on missing version | "No download found for request: cpython-3.99.0-..." (素っ気ない) | "no python-build-standalone asset found ... Try a different patch like..." (誘導あり、文面は改善余地) | 🟡 Mixed |
| A5b | Typo に対する fuzzy match | 無し (3.13.71 → 即 error) | 3.13.71 → **silent に** 3.13.13+20260414 を install (python-build-standalone 上流の latest-patch 解決の副作用、qusp 側の意図的 fuzzy ではない)。動くが「ユーザに違うことを通告しない」点で UX バグ気味 | 🟡 Mixed (qusp は forgiving だが silent、uv は厳格だが通告ゼロ) |
| A6 | Multi-vendor (Java distribution 風) | N/A (Python は 1 vendor) | Foojay 経由で temurin / corretto / zulu / graalvm、未指定で temurin default | qusp 専有 ✓ |
| A7 | Network 不安定時の挙動 | retry あり (uv は libcurl のリトライ込み) | parse error が ad-hoc に user に出る (`parse python-build-standalone release index: EOF`) | ❌ qusp 劣勢 |

### B. Runtime list / introspection

| ID | 項目 | uv | qusp | 判定 |
|---|---|---|---|---|
| B1 | List remote versions | `cpython-3.13.7+freethreaded`, `<download available>`, `/usr/local/bin/python3.13 -> ...` の rich layout | bare semver list | 🟡 Partial |
| B2 | List installed | system / brew / mise / uv 全部 discover | qusp-managed のみ | ❌ Missing (ただし設計上の意図 ─ "no subprocess freeloading") |
| B3 | Resolve current | `/Users/.../bin/python` 絶対 path | `python (none)` または version 文字列 | 🟡 Partial |
| B4 | Pin version (write `.<lang>-version`) | `uv python pin 3.12` で `.python-version` 書込 | コマンド無し | ❌ Missing |

### C. Self management

| ID | 項目 | uv | qusp | 判定 |
|---|---|---|---|---|
| C1 | self-update | `uv self update --dry-run` (dry run flag) | `qusp self-update --check` (専用 flag) | ✅ Parity |
| C2 | version display | `uv 0.8.12 (36151df0e 2025-08-18)` git rev + date | `qusp 0.24.0` のみ | 🟡 Partial |
| C3 | Shell completions | zsh 4947 行 | zsh 636 行 | ✅ (qusp は単純で十分) |

### D. First-run / hospitality flows

| ID | 項目 | uv | qusp | 判定 |
|---|---|---|---|---|
| D1 | Cold-cache run script (no install) | `uv run hello.py` 5.95s (Python 3.12 download + run) | `qusp x ./hello.py` 16.95s (但 1 回目 EOF parse error、retry で成功)。Lua の場合 12s (source build)、Zig 0.16.0 の場合 44s (cold prebuilt + script コンパイル時 API mismatch) | 🟡 Mixed |
| D2 | Inline script metadata | PEP 723 (`# /// script ... ///`) 完全 honor。`requires-python = "==3.10.*"` で 3.10 を pull | `# qusp: lua = X` を **ignore**、curated default で run | ❌ Missing |
| D3 | Tool install (persistent) | `uv tool install ruff` で `~/.local/bin/ruff` 設置 | `qusp add tool ruff` exists がほぼ Go 専用、Python tool route 未実装 | ❌ Missing |
| D4 | Tool run (ephemeral) | `uvx ruff --version` cold 1.96s、Installed in 4ms | `qusp x ruff` → "no backend recognized tool 'ruff'" | ❌ Missing |
| D5 | Doctor / health check | `uv` には無い | `qusp doctor` で data dir / config / cache / 各 backend installed 数を rich 出力 | qusp 専有 ✓ |
| D6 | PATH not-on-path warning | "warning: ... is not on your PATH. ... `uv python update-shell`" | 無し (qusp の哲学は `qusp run`/`qusp x` 経由なので PATH 不要、ただし bare command 派には案内が無い) | 🟡 Partial |

### E. Init / scaffold

| ID | 項目 | uv | qusp | 判定 |
|---|---|---|---|---|
| E1 | `init` minimalism | 5 行 pyproject.toml | 798 byte 例示満載の qusp.toml | 🟡 (好み分かれる、qusp は親切すぎ?) |
| E2 | First-time setup hospitality | "Initialized project `init-uv`" 1 行 | "✓ wrote /path/qusp.toml" 1 行 + 編集テンプレ | ✅ Parity |

---

## Verdict 集計 (15 項目, A6/A5b/D5 は qusp 専有)

| 状態 | 件数 |
|---|---|
| ✅ Parity (or qusp 勝ち) | A1 / A2 / A6 / C1 / C3 / D5 / E2 = **7 件** |
| 🟡 Partial (動くが劣る) | A4 / A5 / A5b / B1 / B3 / C2 / D1 / D6 / E1 = **9 件** |
| ❌ Missing (不在 or 大幅劣る) | A3 / A7 / B2 / B4 / D2 / D3 / D4 = **7 件** |
| qusp 専有 | A6 / D5 (上記 ✅ に含む) | |

**Phase 5 完了基準:** 全 ❌ 項目を 🟡 以上に、🟡 項目の半数以上を ✅ に。
B2 は qusp 哲学で deliberate non-goal なので例外、B4 は議論余地。

---

## Phase 5 サブタスクと audit row のマッピング

各 ❌ / 🟡 を解決する on-hold doc 群:

### Done

- ✅ **[`qusp x <script>` extension-routing](../done/x-script-routing.md)** (v0.24.0)
  ─ D1 を立てた最初の証拠。残課題は performance + reliability (A7) と
    inline metadata (D2)。

### High priority (audit で明確に劣勢)

- ❌ **[Did-you-mean fuzzy: 全 backend 展開](did-you-mean-cross-backend.md)**
  ─ 解決対象: **A5** (qusp 側 message 改善)、**A5b** (silent 版交換の
    通告 path)。Python は python-build-standalone 上流が latest-patch
    解決を勝手にやってくれてるが qusp 側は「user 入力 → 実 install
    version」差分を通告しない。両方を fix する: missing version は
    "did you mean" 候補表示 (uv に勝てる)、近接 substitution は
    "asked X, installed Y, was that intended?" の確認 print を出す。

- ❌ **[Progress display を uv 級に揃える](progress-display-uv-class.md)**
  ─ 解決対象: **A3** (download 中 silent)、**A4** (sha verify 不可視)。
    uv の `Downloading cpython-... (17.5MiB)` に並ぶ単一行 progress。
    Lua/Haskell の build subprocess も同じ枠で見せる。

- ❌ **[Cross-language tool install registry](tool-registry-cross-language.md)**
  ─ 解決対象: **D3** (`tool install` persistent)、**D4** (`tool run`
    ephemeral)。uv の `uvx ruff` 1.96s 体験を qusp で。Python は uv
    routing で先行、他言語は curated registry で。

- ❌ **[Inline script metadata (PEP 723 風)](inline-script-metadata.md)**
  ─ 解決対象: **D2** (現状 ignore)。PEP 723 完全互換 + qusp 専用 prefix
    `# qusp: lua = X` を共存。

- ❌ **[Network resilience + retry](#)** ← 新規 on-hold doc 候補
  ─ 解決対象: **A7** (parse error 露出)。`HttpFetcher` trait に retry
    layer を入れて transient error を user に見せない。
    *(まだ doc なし、Phase 5 で要追加)*

### Medium priority (Partial → Parity 押し上げ)

- 🟡 **[Error richness: distribution defaults](error-richness-distribution-defaults.md)**
  ─ 解決対象: **A5 (qusp 側文面)**。InstallErr enum で uv 級 message。

- 🟡 **List remote の richness 強化** ← 既存 doc 無し、追加候補
  ─ 解決対象: **B1**。`<download available>` 風の install-status 表示、
    impl タグ (`cpython-...`)、distribution variants。

- 🟡 **Resolve current の絶対 path 表示** ← 既存 doc 無し、追加候補
  ─ 解決対象: **B3**。`qusp current python --resolved` で絶対 path。

- 🟡 **version 文字列に build rev + date を載せる** ← 既存 doc 無し、追加候補
  ─ 解決対象: **C2**。`build.rs` で git rev / build date を埋め込む。

- 🟡 **`qusp x` の reliability** (parse error retry)
  ─ A7 の延長、Network resilience に内包。

- 🟡 **PATH not-on-path 案内** ← 既存 doc 無し、追加候補
  ─ 解決対象: **D6**。`qusp doctor` で `qusp hook` の eval 提案、
    bare command 派への案内 path。

### Low priority / Defer

- 🟡 **E1 init minimalism**: 親切な examples を消すかどうかは UX 議論
  待ち。dogfood でテンプレが重い feedback が出たら decide。

- ❌ **[shellenv auto-eval](shellenv-auto-eval.md)** (元から低 priority)
  ─ 解決対象: **D6 の延長**。rcfile 編集ゼロは uv 級ホスピタリティの
    上限ではなく、qusp 側は doctor 経由案内で 80% 達成可能。

### Deliberate non-goals

- **B2 List installed: 全 manager discover**: qusp の "no subprocess
  freeloading" 原則と矛盾。uv が `/usr/local/bin/python3.14` を
  自動 discover するのはホスピタリティ上は ✅ だが、qusp は **own した
  install 以外 trust しない** stance を取る。`qusp doctor` で competing
  manager 検出 → 案内する path はあり得る (将来的)。

- **B4 pin command**: uv の `python pin` は `.python-version` を書く
  だけ。qusp では `qusp.toml` 編集 (or 将来 `qusp pin`) で代替。
  優先度低、dogfood 需要次第。

---

## 完了定義 (re-audit ベース)

> 「fresh laptop で qusp 1 つだけ install して、`qusp x ./<anything>.{py,lua,scala,...}` と打ったら全部 ergonomic に動く」
>
> + 上記 audit 表を再走したとき:
>
> - ✅ 列が **15 件中 12 件以上**
> - ❌ 列が **0 件** (deliberate non-goal を除く)
> - 全 🟡 項目に明示的な reasoning (なぜ ✅ にならないか) が doc に残ってる

uv の Python 体験を全 18 言語に展開したと言える瞬間、かつ
mise/asdf に対する **設計品質 + 機能網羅 両軸** での圧倒的優位が成立する瞬間。

---

## 非ゴール

- task runner (`qusp task` / `mise run` 相当): 別 phase / 別議論
- env / secret 管理 (direnv 代替): 別 phase
- Reproducibility & Nix bridge: Phase 6 に移動
- Python の package 解決機能 (`uv add` / `uv sync` / `uv pip`): qusp の
  方向違い、uv に dispatch する形のみ検討 (D3 の中で扱う)
