# DDD Phase 1 — PinnedManifest

**Shipped:** v0.10.0
**Tag:** v0.10.0

Smart-constructed `PinnedManifest` を導入。`domain::validate(&Manifest, &registry)` が:
- 未知 lang を弾く (UnknownLanguage)
- 空 version を弾く (MissingVersion / EmptyVersion)
- cross-backend deps 検証 (MissingDependency)

を一括で行う。Orchestrator は `&PinnedManifest` だけ受け取る。
"validate したか?" の心配が型レベルで消える。

`thiserror` ベースの typed `ManifestError`。
