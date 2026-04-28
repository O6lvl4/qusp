# Groovy

**優先度:** Phase 4 (2.x+)
**難易度:** 低 (Apache 公式 zip、シンプル)
**前提:** Java backend

## なぜ

Gradle の DSL 言語、Spring Cloud のスクリプト、Jenkins の Pipeline。
2026 でも Gradle ユーザーが間接的に大量に使う。

## 設計

Coursier 経由でも install できるが、Groovy は **Apache 公式 zip distribution** が安定してる:

- **Source:** `https://archive.apache.org/dist/groovy/{version}/distribution/apache-groovy-binary-{version}.zip`
- **Verification:** 同 URL + `.sha256` (or `.sha512`)
- **Layout:** zip → `groovy-{version}/{bin/{groovy, groovysh, groovyc}, lib/, conf/}`
- **Detect:** 一般的な `.groovy-version` ファイル convention
- `requires = ["java"]`

Coursier 経由ではなく、Java backend / Maven / Gradle と同じパターンで apache.org から直接。

```toml
[java]
version = "21"

[groovy]
version = "4.0.22"
```

## 設計上の悩み

- **Groovy 4.x vs 3.x**: 4.x が現行、3.x は Gradle <8 系のレガシー。両方 supported なので exact pin。
- **Apache のミラー切り替え**: archive.apache.org は古いバージョン、最新は dlcdn.apache.org がデフォ。version-aware に解決。

## 非ゴール

- Grails framework 管理 (別物)。
- Groovy 系 DSL (Gradle の build.gradle) 管理。

## 実装ステップ

1. `crates/qusp-core/src/backends/groovy.rs` (mvn/gradle と同じ shape)
2. mirror 自動切換 (latest は dlcdn、archive は archive.apache.org)
3. e2e/groovy.sh
