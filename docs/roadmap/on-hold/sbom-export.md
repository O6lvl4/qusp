# SBOM Export (`qusp sbom`)

**優先度:** Phase 5 (3.x)

## やること

`qusp sbom --format spdx` / `--format cyclonedx` で qusp.lock から SBOM を出力。
toolchain と installed tool を SPDX の Package として列挙。upstream_hash を checksum field に。

サプライチェーン audit / コンプライアンス用。
