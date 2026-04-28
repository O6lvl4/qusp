# Force reinstall flag

**Phase 5 (Hospitality Parity)。**
**解決対象 audit row:** N1 ─ force reinstall

## なぜ

実測 (audit 2026-04-28):

```
$ uv python install --reinstall 3.11.13
$ uv python install -r 3.11.13         # 短縮形
$ uv python install -f 3.11.13         # 別 alias
```

uv は 3 つの形で reinstall を許可。idempotent な install を **明示的に
やり直す** ための escape hatch。

qusp は `qusp install python 3.11.13` を 2 度叩くと 2 回目は idempotent
で skip する (`already_present`)。reinstall hatch が無い。

実害:
- store dir が partial corrupt した時 (extract 中の SIGKILL 等) に
  qusp が "already_present" 判定で skip → broken install を使い続ける
- toolchain 自体は変わらないが publisher の sha256 sidecar が更新
  された (rebuild) ケースで再 install したい
- 自分の build pipeline で常に clean install を強制したい

## 設計案

### CLI

```
$ qusp install python 3.11.13 --reinstall
$ qusp install python 3.11.13 -r
$ qusp install python 3.11.13 -f         # uv の alias を踏襲
```

`qusp install` (no args、manifest 経由) でも `--reinstall` を
受ける、その場合は manifest 内全 lang を再 install。

### Implementation

`Backend::install` の最初の `if install_dir.exists()` 早期 return を
gate する形。

```rust
pub struct InstallOpts {
    pub distribution: Option<String>,
    pub reinstall: bool,             // 新規 field
}

async fn install(...) -> Result<InstallReport> {
    let install_dir = ...;
    if install_dir.join("bin").join(...).exists() && !opts.reinstall {
        return Ok(InstallReport { already_present: true, ... });
    }
    // re-install path: 既存 install_dir / store_dir を削除してから download
    if opts.reinstall {
        let _ = std::fs::remove_dir_all(&install_dir);
        // store_dir も削除? 実は CAS なので sha 一致時は再利用したい
        // が安全側に倒すなら全削除
    }
    ...
}
```

### Reinstall scope

- `paths.data/<lang>/<v>/` (symlink + data) は **必ず削除** + 再作成
- `paths.store/<sha>/` (content-addressed) は **default で残す**
  (sha 一致なら content も一致のはずなので不要)
- `--reinstall --deep` で store entry まで削除する変種を検討
  (corrupt 復旧用)

### Lock との整合

`--reinstall` で再 install した結果 sha が変わった場合 (publisher
側で artifact が swap された)、qusp.lock の `upstream_hash` が
mismatch する。ここは:
- (A) lock の upstream_hash を更新 → user に notify
- (B) error を出して `--allow-hash-change` を要求

uv は (A) 寄り (publisher の hash 変更は信頼)、qusp は sha mandatory
原則的に (B) の方が筋良い。

## 設計上の悩み

- **CI 用途**: CI で常に reinstall すると毎回 download、遅い。
  `--reinstall-if-stale <duration>` 風の TTL 案もあるが overkill、
  `--reinstall` のみで十分。
- **idempotency 維持**: `qusp install` の no-op 性は CI でも user 側
  でも貴重 (`qusp sync` の core)。default は idempotent、`--reinstall`
  のみ destructive、というスタンスを明確に。

## 非ゴール

- partial reinstall (lib のみ / bin のみ) ─ scope 不明瞭
- reinstall 中の rollback (failure 時に旧 install を復元) ─ 安全に
  做るなら staging dir → atomic swap だが complexity 増す、まず
  basic reinstall を ship してから検討

## 実装ステップ

1. `InstallOpts.reinstall: bool` field 追加
2. CLI flag (`-r` / `-f` / `--reinstall`) → InstallOpts
3. 各 backend の install() の早期 return を gate
4. lock 整合 logic (sha mismatch 時の挙動)
5. unit test (既存 install + reinstall flag → re-download 発火)
6. e2e: 既存 lua install → reinstall flag → store dir 再作成確認
