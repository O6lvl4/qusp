# Groovy

**Shipped:** v0.18.0
**Tag:** v0.18.0
**Phase 4 第四弾。Single-binary 系・Apache zip + cross-backend [java]。**

## 出荷時の落とし穴

`bin/startGroovy` は Darwin ブロックで
`JAVA_OPTS="$JAVA_OPTS -Xdock:name=$GROOVY_APP_NAME -Xdock:icon=$GROOVY_HOME/lib/groovy.icns"`
を追記してから後段で `exec "$JAVACMD" $JAVA_OPTS \ ...` と
**unquoted** で展開する。`$GROOVY_HOME` が macOS 既定の
`~/Library/Application Support/dev.O6lvl4.qusp` 配下のときに
"Application Support" のスペースで word-split が起き、
`Support/.../lib/groovy.icns` が main-class として Java に渡されて
`ClassNotFoundException: Support.dev.O6lvl4.qusp.groovy.4.0.22.lib.groovy.icns`
を出して死ぬ。

`install` 後に `bin/startGroovy` の
` -Xdock:icon=$GROOVY_HOME/lib/groovy.icns` の部分を削除する
in-place パッチで回避。Dock のアイコンバッジは捨てて、
`groovy --version` が動くほうを取る。

(以下、設計時の元メモ)

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
