# Range Version Specs

**優先度:** Phase 2 (1.x)
**前提:** v1.0 出荷後

## 問題

`[go] version = "1.26.2"` だけの exact pin。
`^1.26`, `~1.26.0`, `>=1.26 <2.0` が書けない。
LTS スイッチ ("Java 21 LTS なら何でも") も書けない。

## やること

1. `Version` newtype は今 opaque string。`VersionConstraint` enum を別途追加。
   - `Exact("1.26.2")`
   - `Caret("1.26")` → `^1.26`
   - `Tilde("1.26.0")` → `~1.26.0`
   - `Range { min, max }`
   - `LtsLatest` (Java 専用?)
2. `Backend::resolve_concrete(constraint, list_remote_result) -> Version` を新規。
3. lock には resolved exact が落ちる。manifest に書いた range は lock を見れば実態が分かる。

## 非ゴール

- semver の厳密性: 各 publisher の version 形式は揃ってない (`go1.26.2` vs `21.0.5+11`)。
  最大公約数で「範囲指定をサポートする backend」と「exact only backend」に分ける。
