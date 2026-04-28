# Verbose `-v` flag + env var contract

**Phase 5 (Hospitality Parity)。**
**解決対象 audit row:** Y1 (-v debug log) + S1 (env var 露出)

## なぜ

実測 (audit 2026-04-28):

```
$ uv -v python install 3.11.13
DEBUG uv 0.8.12 (36151df0e 2025-08-18)
DEBUG Acquired lock for `/var/.../python`
DEBUG No installation found for request `3.11.13 (cpython-3.11.13-macos-x86_64-none)`
DEBUG Found download `cpython-3.11.13-macos-x86_64-none` for request `3.11.13 ...`
DEBUG Using request timeout of 30s
DEBUG Downloading https://github.com/astral-sh/python-build-standalone/releases/download/...
DEBUG Extracting cpython-3.11.13-...tar.gz to temporary location: /var/.../tmp7VDNIg
DEBUG Moving /var/.../python to /var/.../cpython-3.11.13-macos-x86_64-none
DEBUG Installed executable at `/var/.../bin/python3.11`
DEBUG Released lock at `/var/.../python/.lock`
```

uv の `-v` は **install の全 phase を構造化 trace** する。問題発生時の
diagnosis (なぜ download が遅い? どの URL を叩いた? どの段階で失敗?)
が即可能。

qusp は `--verbose` / `-v` flag そのものが無い。`--quiet` のみ存在。
debug 必要時は **コードに `eprintln!` を足してリビルド** が現実解。

加えて S1 (env vars exposed in `--help`):
- uv は `--cache-dir [env: UV_CACHE_DIR=]` 風に全 path/flag に env var contract を `--help` に併記
- qusp は env var contract が `--help` に出てこない (HOME/XDG_DATA_HOME を実際は読んでるが、`QUSP_*` の慣習が doc 化されてない)

両方とも **diagnostic surface** の問題で根が同じなので 1 doc に統合。

## 設計案

### A. `-v` / `--verbose` flag (multi-level)

clap で `-v` の出現回数で level を上げる:

```rust
#[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, global = true)]
verbose: u8,
```

- `-v` = INFO (現状のデフォルト output 相当の level を tracing に乗せる)
- `-vv` = DEBUG (uv の `-v` 相当)
- `-vvv` = TRACE (HTTP body / sha bytes など、滅多に必要ない)

### B. `tracing` crate 導入

現状 qusp は `eprintln!` / `say!` / `spinner` で ad-hoc に出力。これを
`tracing` ベースに refactor:

```rust
use tracing::{debug, info, instrument};

#[instrument(skip(http))]
async fn download_asset(url: &str, http: &dyn HttpFetcher) -> Result<Vec<u8>> {
    debug!("downloading {url}");
    let bytes = http.get_bytes(url).await?;
    debug!("downloaded {} bytes", bytes.len());
    Ok(bytes)
}
```

`tracing_subscriber` で verbose level に応じた filter:

```rust
let level = match cli.verbose {
    0 => Level::WARN,
    1 => Level::INFO,
    2 => Level::DEBUG,
    _ => Level::TRACE,
};
```

これで `qusp -v install python 3.13.0` が uv 同形 trace を出す。

### C. env var contract の `--help` 表記 (S1)

clap の `#[arg(env = "QUSP_*")]` を全 path/flag に attach:

```rust
struct Cli {
    #[arg(short = 'q', long, global = true, env = "QUSP_QUIET")]
    quiet: bool,

    #[arg(short = 'v', long, action = ArgAction::Count, global = true, env = "QUSP_VERBOSE")]
    verbose: u8,

    #[arg(long, global = true, env = "QUSP_NO_COLOR")]
    no_color: bool,

    #[arg(long, global = true, env = "QUSP_NO_PROGRESS")]
    no_progress: bool,
}
```

clap が自動的に `[env: QUSP_QUIET=]` を `--help` に出す → S1 解決。

主要 env var (Phase 5 完了時固定):

| env var | 役割 |
|---|---|
| `QUSP_QUIET` | `--quiet` 同等 |
| `QUSP_VERBOSE` | `--verbose` レベル数値 |
| `QUSP_NO_COLOR` | color 出力抑制 |
| `QUSP_NO_PROGRESS` | progress bar 抑制 |
| `QUSP_DATA_DIR` | data dir override (XDG_DATA_HOME より優先) |
| `QUSP_CACHE_DIR` | cache dir override |
| `QUSP_CONFIG_FILE` | qusp.toml の明示パス |
| `QUSP_NO_NETWORK` | offline mode (Phase 6 の reproducibility に繋がる) |

### D. JSON 出力との整合 (R1 と連動)

`--output-format json` 時は trace は stderr に流す、stdout は JSON のみ。`tracing` の writer は分離可能なので問題無い。

## 設計上の悩み

- **既存 `say!` macro との共存**: spinner / colored output は既に
  presentation layer に分離されてる。tracing 導入時は say! を tracing
  の INFO level に redirect する形で cohabitate。
- **DEBUG log の volume**: install 1 回で debug 行が 50+ になる可能性
  あり。filter / `RUST_LOG` 風の per-target filter (`tracing_subscriber`
  の `EnvFilter`) を用意しておく。
- **TRACE level の安全性**: HTTP body / sha bytes を流すので、token
  等が混ざらないよう careful redaction (Authorization header はマスク)。

## 非ゴール

- structured log → 別 sink (Loki / OpenTelemetry) ─ overkill
- log file への persist ─ user は `qusp -vv ... 2> qusp.log` で十分

## 実装ステップ

1. `tracing` + `tracing_subscriber` を qusp-cli/qusp-core に追加
2. `cli.verbose` count → tracing level filter
3. 既存 `eprintln!` / `say!` を tracing macro に段階的に移行
4. install path に `instrument` + `debug!` を埋める (uv level の trace)
5. 全 global flag に `env = "QUSP_*"` 付与
6. `--help` で env var が表示されることを e2e で assert
7. `docs/STABILITY.md` に env var contract を追加
