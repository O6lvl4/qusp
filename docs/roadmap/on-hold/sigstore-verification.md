# Sigstore / SLSA Signature Verification

**優先度:** Phase 2 (1.x)
**前提:** v1.0 出荷後
**問題:** 現状の sha256 / sha512 検証は **publisher CDN と同じチャネル**経由で来る。
CDN が compromised なら hash も改竄できる。supply-chain attack 耐性が無い。

## 対象

- Go: `cosign verify-blob` (golang.org が cosign 署名を出してる、確認要)
- Node: nodejs.org が GPG sig を併載 (.sig ファイル)
- Python (PBS): `Sigstore` 署名あり、SLSA provenance あり
- Bun / Deno: GPG-signed sums file
- Java (Foojay): publisher 各社で signature の出し方が違う、現実的には sha だけ
- Rust: rust-lang.org が `.asc` 併載

## やること

1. `crate::effects::Verifier` trait — `verify(asset_bytes, sig_url, pub_keys)`
2. backend ごとに verify policy を持つ
3. `qusp.toml [security] verify = "sigstore" | "gpg" | "sha-only"` で opt-in
4. デフォルトで verify_strict は backend が出来るときだけ有効

## 非ゴール

- 全 publisher 強制。strict 対応していない publisher (Maven Central 等) は警告のみ。
