# PATH not-on-path 案内

**Phase 5 (Hospitality Parity)。**
**解決対象 audit row:** D6 ─ PATH not-on-path warning

## なぜ

実測 (audit 2026-04-28) で uv が install 後に出す案内:

```
$ uv python install 3.11.13
warning: `/var/folders/.../tmp/.local/bin` is not on your PATH.
         To use installed Python executables, run
         `export PATH="/var/folders/.../tmp/.local/bin:$PATH"`
         or `uv python update-shell`.
```

uv の install path は `~/.local/bin/` で、bash/zsh でも fish でも
PATH に入ってないことがある。ここで warn せずに silent install すると
ユーザは「install 完了したのに `python` が動かない」体験をする。
uv はこれを **install の最後で必ず check + warn** する。

qusp は構造上「`qusp run` / `qusp x` 経由で動かす」前提なので、
PATH に shim を入れない設計。**ただし bare command で `python` を
動かしたい派** には rcfile に `eval "$(qusp hook --shell zsh)"` を
書く必要があり、その案内が install フローに無い。

実測:

```
$ qusp install python 3.11.13
✓ python 3.11.15+20260414 installed
  → /var/folders/.../python/3.11.13
```

完。ユーザが次に `python --version` と打って失敗するまで気づかない。

## 設計案

### A. install 後の hint print

```
$ qusp install python 3.13.0
✓ python 3.13.0 installed
  → /Users/.../qusp/python/3.13.0

  Run via:
    qusp run python              # explicit (no PATH change needed)
    qusp x ./script.py           # ephemeral

  Or to enable bare `python` in this shell session:
    eval "$(qusp hook --shell zsh)"

  See `qusp doctor` for diagnostics.
```

3 行ブロック (上 = recommended、下 = opt-in)。`qusp run` / `qusp x`
を一段上に置くことで qusp 哲学を beep する。

### B. `qusp doctor` の hint 充実

doctor 出力に「shell hook がインストールされてるか」セクションを足す:

```
$ qusp doctor
qusp doctor
  data dir   : /Users/.../qusp
  shell hook : ✗ not eval'd in current shell
  → To enable bare commands (`python`, `node`, ...) install the hook:
       echo 'eval "$(qusp hook --shell zsh)"' >> ~/.zshrc
       source ~/.zshrc
  ...
```

shell hook の検出は env var (e.g. `QUSP_HOOK_INSTALLED=1` を hook 自身
が export する) で判定。

### C. quiet モード (`-q`) では hint suppress

CI / pipeline 用。`-q` で warning と hint 全部抑制。

## 設計上の悩み

- **過剰なお節介**: install のたびに 5 行 hint を出すと verbose 気味。
  `--no-hint` で抑制可能、もしくは `qusp` 初回 install 時のみ出して
  あと sticky に suppress (config file に `hint_shown_at = "<date>"`)。
- **shell ごとの hook 形式差**: zsh/bash/fish/pwsh で eval 構文が
  違う。hint は detected `$SHELL` を読んで適切な eval を出す。
- **`qusp install` 後の hint vs `qusp init` 後の hint**: 重複しないよう
  init は hint 出さず、install 1 回目のみ出す形。

## 非ゴール

- shell hook の自動 install (それは
  `shellenv-auto-eval.md` で扱う、現状 D 案 = no-op)
- shim の自動配置 (qusp 哲学に反する、却下)

## 実装ステップ

1. `cmd_install` の成功 path に hint block (`-q` で suppress)
2. `cmd_doctor` に shell hook detection セクション
3. config file (`~/.config/qusp/config.toml`) に `hint_shown_at` を持つ
4. install 1 回目以外は hint 出さない logic (TTL or 完全 suppress)
5. e2e: install 直後の出力に "qusp run" "qusp x" "eval qusp hook"
   3 つが含まれることを assert
