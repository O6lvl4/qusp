# `qusp --version` に build rev + date を載せる

**Phase 5 (Hospitality Parity)。**
**解決対象 audit row:** C2 ─ version display

## なぜ

実測 (audit 2026-04-28):

```
$ uv --version
uv 0.8.12 (36151df0e 2025-08-18)

$ qusp --version
qusp 0.24.0
```

uv は `<git-rev> <build-date>` を併記する。これは:
1. **bug report で正確な build を特定できる** (issue tracker での再現性)
2. **release tag 後の patch build / dev build と stable release を区別できる**
3. **dogfood 中の人が自分のバイナリが古いか即判定できる**

qusp は cargo tag-based version しか出さないので、同じ 0.24.0 でも
release artifact / 自前 cargo build / dev branch build の区別が
不可能。

## 設計案

### build.rs で git rev + ISO date を埋め込む

```rust
// crates/qusp-cli/build.rs (新規)
fn main() {
    println!(
        "cargo:rustc-env=QUSP_BUILD_GIT_REV={}",
        std::process::Command::new("git")
            .args(["rev-parse", "--short=10", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".into())
    );
    println!(
        "cargo:rustc-env=QUSP_BUILD_DATE={}",
        chrono::Utc::now().format("%Y-%m-%d")
    );
    println!("cargo:rerun-if-changed=.git/HEAD");
}
```

clap の `#[command(version)]` を独自 string に置換:

```rust
#[command(
    name = "qusp",
    version = concat!(
        env!("CARGO_PKG_VERSION"),
        " (",
        env!("QUSP_BUILD_GIT_REV"),
        " ",
        env!("QUSP_BUILD_DATE"),
        ")"
    ),
    ...
)]
```

出力:
```
$ qusp --version
qusp 0.24.0 (a1b2c3d4e5 2026-04-28)
```

### Dirty build の処理

git working tree が dirty (uncommitted changes あり) の場合は
`a1b2c3d4e5-dirty` を表示。release artifact は必ず clean tree から
生成されるので dirty 表示は dev build の signal になる。

## 設計上の悩み

- **build.rs vs cargo features**: `chrono` を build-dep に入れるのは
  cargo build で chrono を 1 度引くコストがある。代替: `time` 使う、
  または UNIX `date` コマンドを subprocess (既に git も呼んでるので
  類似)。
- **CI build の date**: GitHub Actions の build 時刻 = release artifact
  の "build date"。これは実 use では release tag と date が semi-同期
  なので問題なし。
- **GIT が無い環境** (tarball から build 等): `unknown` フォールバック
  でも `qusp --version` は壊れない。

## 非ゴール

- crate 依存 sha (lock の sha でビルド全体を identify) ─ Phase 6
  reproducibility audit の領域
- build profile 表示 (debug/release) ─ user-facing の意味薄い

## 実装ステップ

1. `crates/qusp-cli/build.rs` 新規作成
2. `Cargo.toml` の `[build-dependencies]` に必要な crate (chrono か time)
3. `main.rs` の clap version 文字列を更新
4. release CI で artifact の `qusp --version` を実行して期待 format
   一致を assert (regression check)
