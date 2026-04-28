# Dart / Flutter

**優先度:** Phase 4 (2.x+)
**難易度:** 中 (Dart 単体は容易、Flutter SDK は重い)

## なぜ

mobile / web / クロスプラットフォーム UI で需要。
特に Flutter は 2026 でも Android/iOS のデフォルト選択肢のひとつ。

## 設計

### Dart 単体 backend

- **Source:** `https://storage.googleapis.com/dart-archive/channels/stable/release/{version}/sdk/dartsdk-{os}-{arch}-release.zip`
- **Verification:** 同 URL + `.sha256sum` sidecar
- **Triple naming:** `linux-x64`, `linux-arm64`, `macos-x64`, `macos-arm64`, `windows-x64`
- **Layout:** zip → `dart-sdk/{bin/{dart, dartdoc, ...}, lib/, include/}`
- **Detect:** `.dart-version` or `pubspec.yaml` (より複雑、まずは `.dart-version` のみ)

### Flutter backend

`requires = ["dart"]`? 実は Flutter SDK には Dart が同梱されてる (separate version)。pin 関係を避けるため:
- **Flutter は独立 backend、Dart に depends しない** (Flutter 内部 Dart を使う)
- `[dart]` と `[flutter]` を別個に pin (両方使う場合)

- **Source:** `https://storage.googleapis.com/flutter_infra_release/releases/stable/{os}/flutter_{os}_{version}-stable.zip`
- 重さ: zip ~700 MB
- **Verification:** `releases_<os>.json` に sha256 がリストされてる
- Channel: stable / beta / dev / master

## 設計上の悩み

- **Flutter SDK の重さ**: 700 MB の zip + 展開後 ~2 GB。content-addressed store の disk 圧迫が顕著。Phase 5 の reproducibility audit で「不要バージョン削除」コマンドが必要になる。
- **Dart in Flutter vs standalone**: 同じ project が両方を pin するケースが奇妙。Flutter pin だけで内部 Dart も同時に解決するのが筋。
- **Android SDK / Xcode** は完全に qusp 範囲外。FLUTTER_HOME を設定するだけ、build は Flutter の責務。

## 非ゴール

- Android SDK / NDK 管理。
- Xcode 管理。
- iOS Simulator 操作。
- Flutter package (pub.dev) の管理。Pub の責務。

## 実装ステップ

1. `crates/qusp-core/src/backends/dart.rs` — シンプル、bun/deno と同形
2. `crates/qusp-core/src/backends/flutter.rs` — zip extract + ストア配置
3. README に「Flutter は ~700 MB」warning
4. e2e/dart.sh、e2e/flutter.sh (後者は CI でディスク食う)
