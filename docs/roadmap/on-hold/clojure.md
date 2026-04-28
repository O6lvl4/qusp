# Clojure (via Coursier or `clj`)

**優先度:** Phase 4 (2.x+)
**難易度:** 中 (Scala backend と Coursier 経由を共有)
**前提:** Java backend、Coursier-based Scala backend が先行 (resolver を共有可能)

## なぜ

JVM 上の Lisp、関数型 Web (re-frame, Pedestal, Datomic) コミュニティ。少人数だが粘り強い。

## 設計

Clojure は 2 つの主流 install path:

### A. Official Clojure CLI (`clj` / `clojure`)

- Source: `https://github.com/clojure/brew-install/releases/download/{version}/posix-install.sh` (公式 install script、bash) — qusp の哲学とは合わない
- Direct: `https://download.clojure.org/install/clojure-tools-{version}.tar.gz` 配布もある (要確認)
- 構成: `clj`, `clojure` (POSIX shell scripts) + JARs

### B. Coursier 経由 (`cs install clojure`)

- Scala backend と同じ Coursier を流用
- `cs install clojure` で公式 CLI を pull
- 一番楽

### qusp の役割

**B 案を採用** (Scala backend と同じ Coursier を使い回す)。

```toml
[java]
version = "21"

[clojure]
version = "1.12.0"
```

`requires = ["java"]`。実装的には Coursier instance を Scala backend と共有 (multi-backend-shared bootstrap installer のパターン)。

## 設計上の悩み

- **複数 backend で Coursier を共有**: 同じ Coursier binary を Scala / Clojure / Groovy が使う。content-addressed store で natural shared、ただし backend が独立に "own" する設計を変える必要があるかも
  - 妥協案: 各 backend が同じ Coursier binary を install (sha 一致するので store に 1 つしか落ちない)、それぞれの bin/ symlink 経由でアクセス
- **Leiningen** (古典的 build tool) は curated tools か、別 backend か。今は scope 外。

## 非ゴール

- Leiningen の管理 (Phase 5 で再検討)
- ClojureScript 個別管理 (Clojure CLI 経由で済む)

## 実装ステップ

1. `crates/qusp-core/src/backends/clojure.rs` (Coursier wrap)
2. Scala backend と Coursier instance を共有する仕組み
3. e2e/clojure.sh
