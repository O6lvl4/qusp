# shellenv auto-eval

**Phase 5 (Hospitality Parity)。**
**優先度:** 低 (`qusp x` で代替できるユースケースが多い)

## なぜ

uv は **shell rcfile に何も追加せず** に `uv run` 単体で動く。これは
shim を持たない設計の自然な帰結 (uv 自体がエントリポイント)。

qusp も `qusp run` / `qusp x` だけで生活する分には rcfile 編集不要だが、
"bare command" (`python script.py`, `cargo build`, `node app.js`) が
shell 側で resolve されてほしい人向けには `eval "$(qusp hook --shell zsh)"`
を rcfile に書く必要がある。これが mise と同じ温度感の opt-in。

uv 並のホスピタリティ視点だと「rcfile 編集を全く要求しない経路」を
探る価値はある。

## 設計選択肢

### A. self-installing shellenv

```
$ qusp init-shell        # or 初回 `qusp install` 時に offer
qusp will append the following line to ~/.zshrc:

  eval "$(qusp hook --shell zsh)"

Continue? [y/N]
```

ユーザの明示的な confirm を取った上で書き込む。uv は self-install
しない (Python ecosystem 内で完結する) ので uv 模倣ではないが、
「ユーザが rcfile を直接編集しなくていい」という価値は等価。

### B. shim-via-quspx symlink farm

`~/.local/bin/qusp-shims/{python, ruby, node, ...}` への symlink を
作って `quspx <args>` に dispatch する。`~/.local/bin` は多くの distro
で既に PATH に入ってるので rcfile 編集ゼロ。

ただしこれは元の **shim 速度を捨てる** 路線で、qusp の "no-shim 4× 速い"
強みと矛盾する。**却下。**

### C. macOS launchd / Linux systemd unit

ユーザの login shell 起動時に PATH を inject する仕組み。OS 依存度が
高すぎる、副作用も大きい。**却下。**

### D. 何もしない (現状維持)

`qusp run` / `qusp x` で生活する人にはそもそも shellenv 不要。bare
command を求める人は明示的に `qusp hook` を rcfile に書く (1 行)。

「uv 級ホスピタリティ」を strict に解釈すると D も妥当な選択。uv
自体が `uv run` 必須経路なので、shellenv なしでも uv は uv 級。

## 評価

A は実装可能だが副作用が大きい (rcfile を qusp が触る)。 dogfood の
中で需要が出るかで決める。**現状は D で行く前提**、本 doc は将来
判断のための論点を残す目的。

## 非ゴール

- B / C 経路の実装
- D 以外の自動化を default にすること

## 実装ステップ (A を選ぶ場合)

1. `qusp init-shell --shell zsh` サブコマンド追加
2. ~/.zshrc / ~/.bashrc / ~/.config/fish/config.fish の検出 + idempotent
   append
3. `qusp uninstall-shell` で除去
4. `qusp install` の初回成功時に opt-in offer (今後 install しない場合
   は `--no-shell-prompt`)
