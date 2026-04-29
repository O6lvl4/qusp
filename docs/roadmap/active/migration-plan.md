# PC 全体言語マネージャ → qusp 移行プラン

著者の現環境を mise / SDKMAN / brew / rustup から qusp 主体に置き換える。**段階的・可逆的** に進める。

---

## 現状棚卸し (2026-04-29)

| 言語 / Tool | 現マネージャ | 入ってるバージョン |
|---|---|---|
| **deno** | mise | 2.6.3 |
| **go** | mise + brew | 1.25.0 / 1.26.2 |
| **java** | mise + SDKMAN | 21.0.2 (mise) / 17.0.11-tem (SDKMAN) |
| **node** | mise + brew | 20.19.6 / 22.18.0 / 24.15.0 |
| **python** | mise + brew + system | 3.11.13 / 3.12.11 (mise) / 3.11 (brew) / 3.9.6 (system) |
| **ruby** | mise + brew + system | 3.3.9 / 3.4.5 / 3.4.7 (mise) / 2.6.10 (system) |
| **rust** | rustup + brew | rustup 管理 / brew |
| **ghc (haskell)** | brew | (バージョン未確認) |
| **ocaml** | brew | (バージョン未確認) |
| **pnpm** | mise | 10.15.0 |
| **terraform** | mise | 1.9.5 |
| **maven** | SDKMAN | 3.9.10 |
| **gradle** | SDKMAN | 9.0.0 |

mise の global config: `~/.config/mise/config.toml`
```toml
[tools]
java = "21"
node = "lts"
pnpm = "latest"
python = "3.11"
ruby = "latest"
terraform = "1.9.5"
```

---

## qusp が今カバーしてるもの

✅ **完全カバー (18 言語)**: go / ruby / python / node / deno / bun / java / kotlin / rust / zig / julia / crystal / groovy / dart / scala / clojure / lua / haskell

✅ **Java tool registry に curated**: maven (mvn) / gradle

❌ **未対応 / 範囲外**:
- **pnpm**: Node tool registry 未整備 (Phase 5 audit D3/D4 = `tool-registry-cross-language.md`)
- **terraform**: qusp の **明示 non-goal** (Phase 1 で deprecate)
- **ocaml**: Phase 4 残り 7 言語の 1 つ (on-hold/ocaml.md)

---

## 移行戦略 — 4 フェーズ

### Phase A: qusp が完全カバーする言語を移行 (低リスク、可逆)

ターゲット: deno / go / java / node / python / ruby (rust は rustup に任せる選択も可)

1. `~/qusp.toml` を作成 (global default、mise の config を翻訳)
   ```toml
   [java]
   version = "21"
   distribution = "temurin"

   [node]
   version = "22.9.0"   # LTS、mise の "lts" 相当

   [python]
   version = "3.11.13"  # mise pin と一致

   [ruby]
   version = "3.4.7"    # mise pin と一致

   [deno]
   version = "2.7.14"

   [go]
   version = "1.26.2"
   ```

2. `qusp install` で全部 install (qusp の content-addressed store に)。これは
   mise の install と並走するだけで mise を壊さない。

3. `qusp doctor` で全 backend に installed=1 が出ることを確認。

4. **mise も qusp も両方有効** な状態で各言語を `qusp run python --version` 等で
   検証。動作問題なければ Phase B へ。

**リスク**: ディスク使用 +5GB 程度 (重複 install)。`brew uninstall mise` する
段階で mise 側を消せば回収可。

**ロールバック**: `~/qusp.toml` を消すだけ。

### Phase B: Tool ecosystem の置換 (中リスク、qusp 機能追加が前提)

ターゲット: maven / gradle / pnpm

1. `qusp add tool mvn` / `qusp add tool gradle` を試す (Java tool registry 既存)
2. pnpm は **qusp 機能不足** (P5 hospitality D3/D4)。当面は mise の pnpm を残す
   選択肢が現実的。

**ブロッカー**: pnpm の cross-language tool registry 未実装。これを 1.0 前に
入れるか、回避策 (pnpm を brew install する等) を選ぶか判断要。

### Phase C: qusp 範囲外の保留 (放置)

- **terraform**: mise の terraform pin は維持。qusp non-goal なので永続維持。
- **ocaml**: brew ocaml 維持、qusp Phase 4 で OCaml ship したら移行。
- **system /usr/bin/python3 + /usr/bin/ruby**: macOS が握ってる、触らない。

### Phase D: Shell rcfile 切替 (高リスク、慎重)

`~/.zshrc` の以下を切り替える:

旧: `eval "$(mise activate zsh)"`
新: `eval "$(qusp hook --shell zsh)"`

**並行運用フェーズ** (1 週間 dogfood):
- まず **両方 eval** した状態で 3 日。順序は qusp が後 (PATH 優先)。
- 衝突や混乱が出たら mise を deactivate。
- 安定したら mise を rcfile から削除。

**ロールバック**: `git diff ~/.zshrc` で戻す、または rcfile に `eval "$(mise activate zsh)"` を再追記。

### Phase E: クリーンアップ (1.0 前後)

- `brew uninstall mise` (依存ない確認後)
- `rm -rf ~/.local/share/mise`
- `rm -rf ~/.sdkman` (Java 用、qusp java backend 完成後)
- `brew uninstall go node ruby python@3.11 rust ghc` (qusp に存在するもののみ)

ocaml は qusp Phase 4 まで保留、terraform は永続保留。

---

## 1.0 リリース前の必達条件

dogfood-and-1.0.md の Step 1-2 と整合:

- [ ] Phase A 完了: 6 言語が qusp 経由で daily に動く
- [ ] Phase D 完了: rcfile が qusp hook only
- [ ] mise が deactivated 状態で 1 週間運用
- [ ] 実プロジェクト (qusp / almide / gv / rv) で `qusp sync --frozen` が CI で通る
- [ ] papercut 一覧に "1.0 ブロッカー" が残ってない

Phase B (pnpm) と Phase C (terraform/ocaml) は **1.0 後でも許容**。
Phase B が必要になったら Phase 5 D3/D4 を 1.0.x で着手。

---

## 今日の即実行 (Phase A 開始)

1. `~/qusp.toml` を 6 言語 pin で作成
2. `qusp install` で全部 install
3. `qusp doctor` 確認
4. **rcfile はまだ触らない**

次の dogfood サイクルで Phase A の感触を確かめる。
