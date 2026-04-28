# Error richness: 不足情報の代わりに具体的な next step を返す

**Phase 5 (Hospitality Parity)。**

## なぜ

uv の error メッセージは「足りない情報を指摘して、次に打つコマンドの
候補まで言う」のが徹底してる。qusp の error は現状「事実を述べる」止まり
が多い。これを 18 backend 全部で uv 級に上げる。

## 代表ケース

### Java distribution required

現状:
```
$ qusp install java 21
error: java backend requires a distribution (e.g. temurin, corretto)
```

uv 級:
```
$ qusp install java 21
error: java backend requires a `distribution` to disambiguate vendors
       Available: temurin (default), corretto, zulu, graalvm_community

       qusp.toml example:
         [java]
         version = "21"
         distribution = "temurin"

       Or via CLI:
         qusp install java 21 --distribution=temurin
```

### Cross-backend dep missing

現状:
```
$ qusp install
error: kotlin requires [java] but it is not pinned
```

uv 級:
```
$ qusp install
error: [kotlin] requires [java] but it is not pinned

       Add to qusp.toml:
         [java]
         version = "21"
         distribution = "temurin"

         [kotlin]
         version = "2.1.20"
```

### Version not found (-> did-you-mean fuzzy と統合)

`did-you-mean-cross-backend.md` 参照。

### Network failure

現状:
```
$ qusp install ruby 3.4.7
error: download failed: connection timeout
```

uv 級:
```
$ qusp install ruby 3.4.7
error: cannot reach cache.ruby-lang.org (5 retries, last: connection timeout)
       The download is mandatory because qusp requires sha256-verified installs.

       Diagnose:
         curl -fsSL https://cache.ruby-lang.org -o /dev/null
         qusp doctor
```

## 設計案

### Error type の richness

`anyhow::Error` のままだと添加情報が散らばる。専用 error enum:

```rust
pub enum InstallErr {
    DistributionRequired { backend: String, available: Vec<&'static str>, default: &'static str },
    CrossBackendMissing { lang: String, requires: Vec<String> },
    VersionNotFound { backend: String, asked: String, suggested: Vec<String> },
    NetworkFailure { url: String, retries: u32, last: String },
    ShaMismatch { asset: String, expected: String, got: String },
    BuildFailed { backend: String, command: String, log_path: PathBuf },
    ...
}
```

`Display` impl で uv 級メッセージを描画。`anyhow::Error` への
conversion で既存 path も互換維持。

### Hint の出力レイアウト

```
error: <one-line summary>

       <2-3 lines of why this happened / what's missing>

       <fix block 1>:
         <code or command>

       <fix block 2 (optional)>:
         <code or command>

       <see also: docs URL or `qusp doctor`>
```

明示的にブロック分割。indent / blank line で視覚的に区切る。

## 設計上の悩み

- **Internationalization**: 現状の error は日本語混在もある。
  uv は英語のみ。qusp は global 語感重視で英語に統一する方向か、
  日本語併記か議論。
- **Stack trace との両立**: anyhow context chain は debug で重要。
  rich error は user-facing、anyhow chain は `--verbose` で出す形に。
- **`qusp doctor` との連携**: error が `qusp doctor` を suggest する
  ケースが多くなりそう。doctor 側で「最後の install 失敗を再分析」
  みたいな機能と pair で価値が増す。

## 非ゴール

- 全 anyhow!() を typed error に置換 (まずは install path 限定)
- Localization framework (英語固定で出す)

## 実装ステップ

1. `crates/qusp-core/src/error.rs` ─ InstallErr enum + Display
2. backends の `install` から InstallErr を bubble up
3. CLI 側で InstallErr を catch して rich format で出力
4. `qusp doctor` への hint 統合
5. 既存 e2e で error path をいくつか追加 (cross-backend missing,
   distribution required) して message 内容を assert
