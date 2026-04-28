# Cargo Binstall Integration

**優先度:** Phase 3 (2.x)

## 問題

Rust ecosystem は `cargo install foo` で source build が事実上の正解。
ただし build 時間がかかる (1 分超 lots of cases)。
`cargo binstall foo` は prebuilt binary を pull する modern alternative。

## やること

`rust::knows_tool(name)` を主要 cargo CLIs (ripgrep / fd / bat / tokei / hyperfine / ...) に対して true。
`install_tool` は cargo-binstall を invoke (binstall 自体を qusp が install する必要がある — bootstrap 問題)。

## bootstrap 問題

- qusp が rust toolchain を install する
- その rust toolchain で cargo-binstall を build (1 回限り、~30 秒)
- 以後 cargo-binstall を使って prebuilt CLI を pull

これが qusp のレイヤーで OK か、レイヤー違反か、悩ましい。
