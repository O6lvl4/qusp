# PHP

**優先度:** Phase 4 (2.x+)
**難易度:** 高 (extension 地獄、build 時間長い)

## なぜ

2026 でも production 大量。WordPress / Laravel / Symfony / EC sites の継続需要。
mise / asdf が対応してる以上、qusp の「mainstream coverage」を主張するには必要。

## 設計

php.net は **source tarball のみ**、prebuilt は配布してない。distro packages (apt / yum / brew) が主流。

### 現実的な path

- **`php-build` (rbenv-php-build / asdf-php) を内部 lib として使う**: ruby-build と同じ哲学。spawn_blocking で source build。
- Source: `https://www.php.net/distributions/php-{ver}.tar.gz`
- Verification: php.net が GPG sig + sha256 を published page に載せる (HTML scrape 必要)
- Build: `./configure --prefix=... --with-openssl --with-curl ...` + `make`
- Build 時間: 5-15 分 (Erlang と同等)

### Extension 管理

PHP の最大の難所。`./configure` の引数で extension を有効化する。よく使われるもの:
- openssl, curl, gd, mbstring, zip, intl, pdo_mysql, pdo_pgsql, redis (extra), imagick (extra), opcache

**mise は extension 設定を `mise.toml` の php section に書ける**。qusp 同等を目指す:

```toml
[php]
version = "8.4.0"
extensions = ["openssl", "curl", "gd", "mbstring", "zip", "intl", "pdo_mysql"]
```

extensions は Backend ごとの拡張フィールド (java の `distribution` と同じパターンで `InstallOpts` を拡張)。

## 設計上の悩み

- **OS-level deps が要る**: openssl-dev, libxml2-dev, libcurl-dev 等。qusp はそれらを管理しない。`brew install pkg-config openssl@3 libxml2 libcurl libzip` 的な前提を README で明示。
- **PECL extensions** (redis, imagick 等) はソースから build に追加 step が必要。Phase 1.5 では coreのみ。
- ヒラ pin (`8.4.0`) と「LTS の最新 patch」需要。range version specs (Phase 2) と組み合わせ。

## 非ゴール

- Composer 管理。Composer は別 tool として `[php.tools] composer = "..."` で curated。
- PECL の包括サポート (Phase 4 では core extensions のみ)。
- HHVM。

## 実装ステップ

1. `php-build` 相当の Rust 実装 OR shell-out (Erlang 同様 spawn_blocking)
2. `crates/qusp-core/src/backends/php.rs`
3. `InstallOpts.extensions: Option<Vec<String>>` を追加 (もしくは backend 専用フィールド)
4. e2e/php.sh — CI 環境で OS deps install 必要
