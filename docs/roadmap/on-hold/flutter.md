# Flutter

**優先度:** Phase 4 (2.x+)
**難易度:** 中 (zip 自体は素直、ただし ~700 MB)

## なぜ

Android/iOS のクロスプラットフォーム UI フレームワーク。
Dart は v0.19.0 で先行出荷済み — Flutter は別ブランチ。

## 設計

- **Source:** `https://storage.googleapis.com/flutter_infra_release/releases/stable/<os>/flutter_<os>_<version>-stable.zip`
- 重さ: zip ~700 MB、展開後 ~2 GB
- **Verification:** `releases_<os>.json` に sha256 がリストされてる
- Channel: stable / beta / dev / master

## 設計上の悩み

- **Flutter SDK の重さ**: 700 MB zip → 2 GB 展開、content-addressed store のディスク
  圧迫が顕著。Phase 5 の reproducibility audit と合わせて「不要バージョン削除」
  コマンドが先に必要かもしれない。
- **Dart in Flutter vs standalone**: 同じプロジェクトが両方を pin するケースが
  奇妙。Flutter pin だけで内部 Dart も同時に解決するのが筋。`[flutter]` 単独で
  Dart を解決させ、`[dart]` は完全独立。`Backend::requires` には乗せない。
- **Android SDK / Xcode** は完全に qusp 範囲外。FLUTTER_ROOT を設定するだけ、
  ネイティブビルドは Flutter の責務。

## 非ゴール

- Android SDK / NDK 管理。
- Xcode 管理。
- iOS Simulator 操作。
- Flutter package (pub.dev) の管理。Pub の責務。

## 実装ステップ

1. `crates/qusp-core/src/backends/flutter.rs` — zip extract + ストア配置
2. README に「Flutter は ~700 MB」warning
3. e2e/flutter.sh (CI ディスク pressure に注意)
