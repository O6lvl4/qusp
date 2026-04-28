# Scala (via Coursier)

**優先度:** Phase 4 (2.x+)
**難易度:** 中 (Coursier wrap、JVM 入れ子)
**前提:** Java backend、`Backend::requires`、ghcup/opam と同じ「qusp が install した bootstrap に dispatch」判断

## なぜ

JVM 上の最重要 alt-language。Spark / Akka / Play / ZIO / Cats Effect が Scala 中心。

## 設計

Scala は build tool が複雑 (sbt / mill / scala-cli) だが、**Coursier** が Scala コミュニティの公式 bootstrap installer:
- `cs install scala` で scalac / scala-cli / scalafmt / metals (LSP) など全部入る
- Coursier 自身は単一 binary、`https://github.com/coursier/coursier/releases`

### qusp の役割

- **新 backend `scala`** で Coursier を制御 (haskell/ghcup, ocaml/opam と同形):
  - Source (Coursier): `https://github.com/coursier/coursier/releases/download/v{version}/cs-{triple}.gz`
  - Verification: GitHub release sha256
  - qusp は Coursier を `versions/scala/coursier-{ver}/cs` に置く
  - `qusp install scala 3.5.0` → `<store>/cs install --install-dir=<store> scala:3.5.0` 経由

`requires = ["java"]` (Coursier 自身が JVM-bootstrapped、Java 必須)

```toml
[java]
version = "21"

[scala]
version = "3.5.0"
coursier_version = "2.1.10"  # optional
```

## 設計上の悩み

- **Coursier の中で Scala をビルドするのか prebuilt なのか**: `cs install scala` は version-specific scala compiler を pull する。実体は Maven Central から JAR (重さ ~200 MB)、JVM 上で動く。build は無い。
- **Scala 2.x vs 3.x**: 互換性破壊。`[scala] version = "2.13.14"` か `"3.5.0"` をユーザーが選ぶ。Coursier 経由なら自然に対応。
- **scala-cli vs scalac**: 2026 では scala-cli が事実上のデフォルト。両方を tools として curated に乗せる。

## 非ゴール

- sbt の管理 (sbt は別 backend or `[scala.tools]` curated)
- Maven Central package の解決 (sbt/mill の責務)

## 実装ステップ

1. `crates/qusp-core/src/backends/scala.rs` (Coursier wrap、`requires = ["java"]`)
2. Coursier binary install path
3. `cs install` dispatch
4. e2e/scala.sh
