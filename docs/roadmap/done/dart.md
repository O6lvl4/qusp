# Dart

**Shipped:** v0.19.0
**Tag:** v0.19.0
**Phase 4 第五弾。Single-binary 系・Google Cloud Storage zip + BSD `sha256sum` sidecar。**

## なぜ

mobile / web / クロスプラットフォーム UI で需要。
特に Flutter は 2026 でも Android/iOS のデフォルト選択肢のひとつ。
Dart 単体も Bazel/Build-tool スクリプト用途で需要が独立してある。

## 設計

- **Source:** `https://storage.googleapis.com/dart-archive/channels/stable/release/<v>/sdk/dartsdk-<os>-<arch>-release.zip`
- **Verification:** 同 URL + `.sha256sum` sidecar
  - 形式は BSD `coreutils sha256sum`-style: `<HEX> *<filename>` (binary mode の `*`)。
    `split_whitespace().next()` で最初のトークンを取れば十分。
- **Triple naming:** `macos-arm64`, `macos-x64`, `linux-x64`, `linux-arm64`
  - Windows は v0.19.0 では out-of-scope。
- **Layout:** zip → `dart-sdk/{bin/{dart, dartdoc, ...}, lib/, include/}`
- **Detect:** `.dart-version`
- **list_remote:** Google の archive は release index JSON を出してないので
  GitHub mirror (`dart-lang/sdk`) の releases API を使う。version 番号は同じ。

## 非ゴール

- pubspec.yaml ベースの version detect (まずは `.dart-version` のみ)
- pub.dev package 管理 (Pub の責務)
- Flutter SDK (`on-hold/flutter.md` 参照)
