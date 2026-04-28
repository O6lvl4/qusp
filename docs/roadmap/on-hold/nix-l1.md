# Nix L1: Detect Substitutes

**優先度:** Phase 5 (3.x)

## やること

`qusp install go 1.26.2` の前に `/nix/store` を一覧して、`go-1.26.2` 風の derivation がすでにあれば symlink で済ませる。

Nix を使ってる macOS / NixOS 環境で qusp が「重複 install を避ける良い市民」になる。
