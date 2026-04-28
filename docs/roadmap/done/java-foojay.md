# Java Backend with Foojay Multi-Vendor

**Shipped:** v0.5.0
**Tag:** v0.5.0

Foojay disco API 経由で Temurin / Corretto / Zulu / GraalVM CE を統一的に install。
- `[java] distribution = "temurin"` で vendor 切り替え
- macOS の `Contents/Home/` ネスト吸収
- mvn (sha512) + gradle (sha256) を curated tools に
- sha256/sha512 hex-length 自動判別
- `Backend::install` に `InstallOpts { distribution }` 追加
