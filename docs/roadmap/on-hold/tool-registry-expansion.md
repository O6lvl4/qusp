# Tool Registry Expansion (Node / Java)

**優先度:** Phase 3 (2.x)

## 現状

- Node: 4 tools (pnpm / yarn / tsc / prettier)
- Java: 2 tools (mvn / gradle)

## 候補

### Node — 追加 self-contained CLIs

- `eslint` (peer-dep があるが本体だけで動く path もある、要検証)
- `vite` (peer-dep 重い、難しいかも)
- `tsx` (esbuild peer-dep)
- `turbo` (Rust binary)
- `npm-check-updates` (deps あり)
- `rimraf` (glob dep)

→ 「peer-dep を解決する」ロジックが要る。`npm install --prefix` を使う path に倒すか?
   倒すなら "no subprocess freeloading" 原則と衝突。

### Java — 追加 JVM tools

- `sbt` (Scala build tool だが Java 単体でも使える)
- `jbang` (single-file Java runner)
- `spotless`
- `checkstyle`

## 決断ポイント

curated breadth よりも **Phase 4 (Coursier 経由 JVM family)** の方が architectural に意味あり。
Tool 数を増やす前に、**JVM family backend のビジョン**を固める方が筋。
