# Swift (server-side)

**優先度:** Phase 4 (2.x+)
**難易度:** 低-中 (公式 prebuilt あり、macOS の .pkg 扱いに注意)

## なぜ

Vapor / Hummingbird / Apple の server-side Swift コミュニティ。
iOS スコープなら不要だが、Linux server-side Swift は地味に伸びてる。

## 設計

- **Source:** `https://download.swift.org/swift-{version}-release/{os}/swift-{version}-RELEASE/swift-{version}-RELEASE-{os}.tar.gz`
  - Linux: `swift-5.10.1-RELEASE-ubuntu22.04.tar.gz` などディストロ依存名
  - macOS: `swift-5.10.1-RELEASE-osx.pkg` — **これは pkg installer** (sudo 必要)
- **Verification:** swift.org が `.sig` (PGP) を併載。`.tar.gz.sig`。
  - sha256 直接は無い、PGP verify が公式
- **Triple naming:** Linux はディストロ別 (`ubuntu22.04`, `amazonlinux2`, `centos7`, `rhel-ubi9`, `fedora39`)、macOS は `osx`、Windows は `windows10`

## 設計上の悩み

- **macOS の .pkg は qusp で扱えない** (graphical installer + sudo 要求)。代替経路: GitHub mirror に tar.gz があるかもしれない、または **macOS は対応外** と決める (Linux server-side のみ)
- **Linux ディストロ別 tarball**: Ubuntu 22.04, Ubuntu 24.04, Amazon Linux 2, CentOS 7, RHEL 9, Fedora 39 が並行。**ホスト OS を identify する仕組みが要る** (今までの qusp backend は OS×arch だけだった)
  - `/etc/os-release` を読んで Linux distro 解決、対応 distro が無ければエラー
- **PGP verify**: sha256 と違って key 管理が要る。Apple の signing key を qusp に bundle するか、初回 import するか。**最初は sha256-only mode (PGP は警告に留める)**

## 非ゴール

- Xcode の管理 (App Store / Mac App Store の責務)
- iOS SDK / Simulator
- Swift Package Manager (SPM) の package 管理

## 実装ステップ

1. `crates/qusp-core/src/backends/swift.rs` — Linux distro detection + tarball
2. `/etc/os-release` パーサ
3. macOS をどうするかの決断 (まずは Linux only、macOS は user に "homebrew で" と推奨)
4. e2e/swift.sh — Linux のみ
