# Reproducibility Audit (`qusp verify`)

**優先度:** Phase 5 (3.x)

## やること

`qusp verify` で qusp.lock の `upstream_hash` と手元の installed binary の hash を突合。
mismatch なら「lock と install がドリフトしてる」と報告。

CI で「成果物が lock 通りか」を保証する。
