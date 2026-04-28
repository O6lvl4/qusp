# `--help` richness + pager

**Phase 5 (Hospitality Parity)。**
**解決対象 audit row:** Q1 (subcmd help richness) + Q2 (paginate) + Z1 (`<cmd> help <subcmd>` long doc)

## なぜ

実測 (audit 2026-04-28):

```
$ uv help run
Run a command or script

Usage: uv run [OPTIONS] [COMMAND]...
...
      --extra <EXTRA>
          Include optional dependencies from the specified extra name

          May be provided more than once.

          [env: UV_EXTRA=]

      --all-extras
          Include all optional dependencies

          [env: UV_ALL_EXTRAS=]
...
  -h, --help
          Display the concise help for this command
```

(数十オプション、各オプションに 1-3 行の description + `[env: VAR=]` 表記、長い場合は pager 経由)

```
$ qusp help run
Run a command using the resolved multi-language environment

Usage: qusp run [OPTIONS] [ARGV]...

Arguments:
  [ARGV]...

Options:
  -q, --quiet
  -h, --help     Print help
  -V, --version  Print version
```

(5 行、何ができるか分からない、option の意味も書いてない)

差は決定的: uv の help は **discoverability の入り口**、qusp の help は **bare reference** に留まる。新規ユーザは uv を `--help` から覚えられる、qusp は外部 docs (README) に依存。

## 設計案

### A. clap の help richness を活用

clap は `#[arg(help = "short", long_help = "long")]` で長文 help を持てる。現状 qusp は long_help を全く使ってない。

```rust
#[derive(Subcommand)]
enum Cmd {
    /// Run a command using the resolved multi-language environment.
    ///
    /// `qusp run <cmd> [args]` looks up the project's qusp.toml,
    /// resolves toolchain versions per backend, builds a merged
    /// PATH/env, and execs the command. Useful when you want bare
    /// `python script.py` or `cargo build` against the qusp-managed
    /// toolchains without modifying your shell rcfile.
    ///
    /// Examples:
    ///   qusp run python script.py    # exec python with qusp env
    ///   qusp run -- node -v          # explicit -- to disambiguate flags
    ///   qusp run cargo build         # multi-language project: cargo + node tools
    ///
    /// See also: `qusp x` for ephemeral runs without qusp.toml,
    /// `qusp shellenv` for printing exports without execing.
    Run {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        argv: Vec<String>,
    },
}
```

`uv help run` 相当 (long help) が表示され、`uv run --help` (short help) は今と同じ簡潔さを保つ。

### B. `[env: QUSP_*=]` 表記を全 flag に

`#[arg(env = "QUSP_QUIET")]` で env var contract を clap に通知 → `--help` に自動表記される。これは S1 (audit row) の解決にも直結。

```rust
struct Cli {
    #[arg(short = 'q', long = "quiet", global = true, env = "QUSP_QUIET")]
    quiet: bool,
    ...
}
```

### C. Pager 経由

`uv help <cmd>` は long help を pager (less) で flow する、`--no-pager` で opt-out。clap には built-in pager 機能無いので `pager` crate (e.g. `pager = "0.16"`) で実装:

```rust
fn print_help_paged(content: &str) {
    if std::env::var("PAGER").is_ok() && atty::is(atty::Stream::Stdout) {
        pager::Pager::new().setup();
    }
    print!("{}", content);
}
```

`qusp help <subcmd>` を hijack して long help を pager に。

### D. `qusp help <topic>` non-command topics

uv は `uv help help` でメタ help、`uv help self` でグループ help を出す。qusp も `qusp help backends` (リスト) / `qusp help install` (詳細) のような hierarchical doc を入れる余地あり。短中期は不要、long help が埋まったら検討。

## 設計上の悩み

- **long help を全 subcmd に書く維持コスト**: 現状 qusp の subcmd は
  ~17 個。各 long help を 5-10 行書くのは初期コスト ~150 行。doc は
  `cmd_*` 関数の docstring と一致させる方針で drift を防ぐ。
- **clap の long_help と short の出し分け**: `--help` short / `help <cmd>` long の慣習を踏襲。
- **env var の contract 安定性**: `QUSP_QUIET`, `QUSP_CACHE_DIR`,
  `QUSP_DATA_DIR`, `QUSP_NO_PROGRESS`, `QUSP_NO_COLOR` 等を Phase 5
  完了時に固定、stability contract に組み込む。

## 非ゴール

- man page 生成 (clap_mangen で可能だが Phase 5 での優先度低)
- markdown export (`uv self generate-shell-completion` 風) ─ 後で

## 実装ステップ

1. 全 subcmd に long help (clap doc-comment) 追加 ─ 17 個
2. global flag (`-q`) と subcmd-local flag に env var (`QUSP_*`) attach
3. `pager` crate 追加、`qusp help <subcmd>` を pager 経由に
4. `--no-pager` global flag
5. 短い再 audit: `qusp help install` の出力長 / option 説明 / env var
   表記 / pager 起動を assert
