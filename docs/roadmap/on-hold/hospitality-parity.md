# Hospitality Parity (Phase 5)

> 「**uv 並のホスピタリティを 18+ 言語全部に。**」

## なぜ Phase 5 が "Reproducibility" から "Hospitality" に変わったか

v0.23.0 までで mise/asdf 比較における qusp の競争 position が固まった:

| 軸 | mise | qusp v0.23 | 差 |
|---|---|---|---|
| 対応言語 native 実装 | plugin 任せ | 全 native Rust | qusp 圧勝 |
| install 検証必須 | plugin 任せ | sha256 一律必須 | qusp 圧勝 |
| cross-backend dep | 無 | `requires=["java"]` 機構 | qusp 圧勝 |
| shim 速度 | ~10ms | ~2ms (4×) | qusp 軽勝 |
| 対応言語数 | ~50 | 18 (本セッション 9 連発で到達) | mise 勝ち |
| **task runner / env / hospitality** | **mise > qusp** | **qusp 不在** | **mise 勝ち** |

uv との比較は方向違い (uv は Python 単体に特化した深さ、qusp は 18 言語横断の広さ)。
ただし **uv が Python 1 つに対してやってる "ホスピタリティの密度" を 18 言語全部に拡張する** position は誰も取ってない。

これが Phase 5 の新しい定義。
旧 Phase 5 (Reproducibility & Nix Bridge) は Phase 6 へ後ろ倒し。

## uv の "ホスピタリティ" の正体

1. **install が瞬時** — 数十秒の体験を 200ms に
2. **エラーが意味を持って読める** — "version not found" じゃなく "did you mean 3.12.7?" まで来る
3. **first-run が 0-config** — `uv run script.py` でランタイムも venv も自動
4. **失敗が再現可能** — lock がある、`--frozen` がある、reproduction が完全
5. **パッケージ管理が同じ口** — `uv add`, `uv tool install` が version manager と一体
6. **shim が無い** — `uv run` 直接 exec、stat 1 回で resolve
7. **CLI の語感** — `uv` 2 文字、サブコマンドが必ず短い動詞

## qusp v0.24.0 時点の達成度

達成済:
- ✅ shim 無し (`qusp run` 直接 exec、4× 速い)
- ✅ lockfile sha-rooted、`--frozen` あり
- ✅ first-run 0-config (`qusp x ./hello.lua` で auto install + exec、v0.24.0)
- ✅ CLI 語感 (`qusp` 4 文字 + `quspx`)

部分達成:
- 🟡 エラーの意味性: Python の fuzzy match だけで横展開してない
- 🟡 install 速度: native backend なので mise より速いが uv の "200ms 級" には届かない (network bound)

未達:
- ❌ tool 管理 (`qusp add tool ruff` 級)
- ❌ inline script metadata 自動 pin
- ❌ shellenv の auto-eval
- ❌ progress display の uv 級揃え

## Phase 5 のサブタスク

### Done

- [x] **[`qusp x <script>` extension-routing](../done/x-script-routing.md)** (v0.24.0)
  ─ 18 言語のうち単一 file で起動する 16 言語に対応。`qusp x ./hello.lua` が
    fresh machine で auto install + exec する uv 級体験を立てた。

### Active candidates (順番は dogfood で決める)

- [ ] **[Did-you-mean fuzzy: 全 backend 展開](did-you-mean-cross-backend.md)**
  ─ Python だけにある fuzzy match を全 18 backend で。version not found 時に
    "did you mean 3.12.7?" と返す。

- [ ] **[Progress display を uv 級に揃える](progress-display-uv-class.md)**
  ─ spinner / ETA / "downloaded N of M" / installing N of M のレイアウト統一。
    今は backend ごとに微妙に揃ってない。

- [ ] **[Cross-language tool install registry](tool-registry-cross-language.md)**
  ─ `qusp tool install ruff` / `gopls` / `prettier` / `scalafmt` / `cabal-fmt`
    を 1 動詞で。Phase 3 の Python tools-via-uv を内包・拡大する。

- [ ] **[Inline script metadata (PEP 723 風)](inline-script-metadata.md)**
  ─ `# qusp: lua = 5.4.7` 風に script 冒頭で version pin、`qusp x` がそれを
    優先順位 0 で読む。

- [ ] **[Error richness: distribution required → suggest defaults](error-richness-distribution-defaults.md)**
  ─ `qusp install java 21` が distribution 不在で失敗した時に
    "temurin / corretto / zulu / graalvm から選んで、おすすめは temurin" まで
    返す。Java を皮切りに各 multi-vendor backend へ。

- [ ] **[shellenv auto-eval](shellenv-auto-eval.md)**
  ─ `.zshrc` に 1 行も追加せずに qusp 単体が `cd` hook を装着する経路。
    現状の `eval "$(qusp hook --shell zsh)"` は明示的な opt-in、これを
    self-installing にする道を探る。

## 完了定義

> 「fresh laptop で qusp 1 つだけ install して、`qusp x ./<anything>.{py,lua,scala,...}`
>  と打ったら全部 ergonomic に動く」

uv の Python 体験を全 18 言語に展開したと言える瞬間。

## 非ゴール

- task runner (`qusp task` / `mise run` 相当): 別 phase / 別議論
- env / secret 管理 (direnv 代替): 別 phase
- Reproducibility & Nix bridge: Phase 6 に移動
