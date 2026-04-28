# Scala 3

**Shipped:** v0.20.0
**Tag:** v0.20.0
**Phase 4 第六弾。Direct GitHub release tarball + cross-backend [java]。**

## 設計判断: Coursier wrap → Direct tarball

元 on-hold ノートは Coursier 経由 (`cs install scala`) を提案していたが、
実装直前の調査で `scala/scala3` が **3.7.0 以降は per-asset `.sha256`
sidecar を出している** ことが判明。Coursier infrastructure の必要性が
消滅したので、Kotlin/Crystal と完全に同形の direct download にした。

## なぜこの方向のほうが良かったか

- **Layer 削減:** Coursier wrap は「qusp が Coursier を install して
  Coursier が Scala を install」の二段。エラー origination が曖昧、
  store の "誰の install か" が曖昧、Coursier 自体の version pin が
  別次元、と複雑化要素が積層する。
- **Verification の単純さ:** Coursier 経由だと sha 検証は Coursier の
  `--repository` 経路に乗るが、qusp の "全 install を qusp の
  HttpFetcher trait 越しに verify" 原則を貫けない。Direct なら
  Crystal/Julia と同じ HttpFetcher 経路。
- **Reuse:** Clojure (v0.21.0) も同じ direct パターンで実装でき、
  共有 Coursier infra を作る必要が無くなった。

## 設計

- **Source:** `https://github.com/scala/scala3/releases/download/<v>/scala3-<v>-<triple>.tar.gz`
- **Verification:** 同 URL + `.sha256` sidecar (`<HEX>  <filename>` 形式)
- **Triple naming:** `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `aarch64-pc-linux`, `x86_64-pc-linux`
- **Layout:** `scala3-<v>-<triple>/{bin/{scala, scalac, scaladoc, ...}, lib/, maven2/}`
  - `maven2/` は ~70MB の bundled local Maven cache。ランタイム resolver で
    使うので削らない。
- **Detect:** `.scala-version`, `build.sbt`, `build.sc`
- **`requires = ["java"]`** (JVM 必須)
- **list_remote:** GitHub releases API、3.7.0 未満は除外 (未検証で install できない)

## 設計上の決定

- **検証フロア = 3.7.0**: 3.6.x 以前は per-asset sidecar が無く
  `sha256sum-<triple>.txt` の bulk file のみ。フォールバックも実装
  可能だが、qusp の "一律 sha 検証" を maintain するためにフロアを
  立てて未対応にした。ユーザーは新しい 3.x を pin する。
- **Scala 2 は対象外**: Scala 2.13.x も生きてるが distribution が別
  リポジトリ (lampepfl/dotty 等)。qusp scala backend は 3.x 専用。
  2.x 需要が出たら別 backend (`scala2`) として追加検討。
- **Tools = empty**: sbt/mill/scala-cli/Coursier は Scala 自身の
  ecosystem。qusp は build tool 競合をしない。

## 非ゴール

- sbt の管理 (Phase 5 で再検討)
- Maven Central package 解決 (sbt/mill の責務)
- Scala-CLI (scala-cli は別 binary、curated tools として後で別途)
- Scala 2.13.x (別 backend として保留)
