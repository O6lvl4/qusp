# JVM Family via Coursier (Scala / Clojure / Groovy)

**優先度:** Phase 4 (2.x+)
**前提:** Java backend / `Backend::requires` 機構

## 設計

- 新 backend `coursier` (or `cs`)。`requires = ["java"]`。
- `coursier` は `cs install` で scala / scalac / clojure / groovy / sbt を入れられる。
- qusp の役割: coursier 自体の install + 各 sub-tool への dispatch。

`qusp.toml`:
```toml
[java]
version = "21"

[scala]
version = "3.6.2"

[clojure]
version = "1.12.0"
```

→ scala / clojure は内部で coursier 経由で resolve / install。

## 設計上の悩み

- `Backend::requires` で chain できるか? scala は coursier を要求、coursier は java を要求
- それとも scala backend が coursier を internal lib として使う? (こちらが筋良さそう)
