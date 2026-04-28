# Kotlin Backend (Cross-Backend Dep の実証)

**Shipped:** v0.9.0
**Tag:** v0.9.0

JetBrains/kotlin GitHub releases から `kotlin-compiler-X.Y.Z.zip` を sha256 検証で install。
`Backend::requires = &["java"]`。

実機で:
- `[kotlin]` のみ pin → 親切エラー
- `[java] + [kotlin]` 両方 pin → 並列 install
- `qusp run kotlinc -version` → kotlinc-jvm が qusp の Java を picks up = env merge OK
- Hello.kt → kotlinc → Hello.jar → java -jar Hello.jar → 出力 OK
