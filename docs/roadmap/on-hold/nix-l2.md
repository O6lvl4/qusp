# Nix L2: Read flake.nix

**優先度:** Phase 5 (3.x)

## やること

`flake.nix` がプロジェクト root にあれば、`packages.<system>` 宣言を qusp の resolution source として読む。
qusp.toml と flake.nix が両立する場合の優先順位を決める。

「Nix 環境にもジワ寄っていける」というメッセージ。
