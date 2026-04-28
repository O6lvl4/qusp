# Concurrent install safety (file lock)

**Phase 5 外、correctness バグ。**
**解決対象 audit row:** W1

## なぜ

実測 (audit 2026-04-28):

```
$ grep -rn "flock\|fs2::\|FileLock\|file_lock" crates/
(no hits)
```

qusp の install path に **file lock 機構が無い**。これは hospitality
ではなく **correctness バグ**。シナリオ:

1. ユーザがターミナル A で `qusp install python 3.13.0` 実行
2. 並行してターミナル B で `qusp install python 3.13.0` 実行
3. 両方が同じ store dir (`<store>/<sha-prefix>/`) に書き込もうとする
4. 結果: 片方が tarball extract 中、もう片方が同じ dir を消して
   再作成 → partial extract / sha mismatch crash / **broken install**

uv の `-v` 出力で確認できる対比:

```
DEBUG Acquired lock for `/var/.../python`
...
DEBUG Released lock at `/var/.../python/.lock`
```

uv は per-language (or per-store-dir) で `flock` ベースの mutual
exclusion を持ってる。

qusp は singleton invocation 前提で書かれてる。multi-shell ユーザ /
CI parallel job / `qusp install` の no-arg 並列内部 (orchestrator
が join_all で fan-out) でも race は理論上起き得る。orchestrator
は backend 単位の futures を並列化するが、**同じ backend の同じ
version への複数 invocation** は protect しない。

## 影響範囲

- 並列ターミナル: 中 (実害は人為依存)
- CI matrix: **高** (同 toolchain を複数 job で同時 install)
- orchestrator 内: 低 (同一 backend 同一 version の二重 install
  call が orchestrator から発火しない設計)
- multi-process shell hook: 中 (`cd` で 2 dir に同時に入る場合等)

## 設計案

### A. Per-store-dir file lock

content-addressed store の各 dir 上に `.lock` を置き、`fs2::FileExt`
or `nix::fcntl::flock` で advisory lock。

```rust
// crates/qusp-core/src/effects/lock.rs (新規)
use fs2::FileExt;

pub struct StoreLock {
    file: std::fs::File,
}

impl StoreLock {
    pub fn acquire(store_dir: &Path) -> Result<Self> {
        anyv_core::paths::ensure_dir(store_dir)?;
        let lock_path = store_dir.join(".qusp.lock");
        let file = std::fs::OpenOptions::new()
            .create(true).read(true).write(true)
            .open(&lock_path)?;
        // Block until available, retry every 1s, log every 10s
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => break,
                Err(_) => {
                    tracing::info!("waiting for lock at {}", lock_path.display());
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }
        Ok(StoreLock { file })
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
```

### B. 各 backend の install() 冒頭で acquire

```rust
async fn install(...) -> Result<InstallReport> {
    let _guard = StoreLock::acquire(&store_dir)?;
    // ... 既存 install logic
}
```

`Drop` で auto release。

### C. install_dir (data symlink) の atomic swap

現状の install:

```rust
if install_dir.exists() { remove(&install_dir) }
symlink(&store_target, &install_dir)?;
```

これは **lock acquire 後でも** symlink → unlink → re-symlink の race
window がある (concurrent reader が消失した link を踏む)。

修正: `tempfile_in` で隣接 path に新 symlink → `rename` で atomic
replace。

```rust
let tmp_link = install_dir.with_extension(format!("tmp-{}", pid));
symlink(&store_target, &tmp_link)?;
std::fs::rename(&tmp_link, &install_dir)?;
```

### D. spawn_blocking 系の lock 範囲

Lua の `make`、Haskell の `ghcup install ghc` は spawn_blocking で
sync 実行。lock も同じ task 内で hold (acquire → spawn_blocking →
release) すればいい。`StoreLock` の drop は spawn_blocking から
出た時に発火する。

## 設計上の悩み

- **deadlock 懸念**: `Backend::requires` で Java を pull する
  Kotlin install は orchestrator が両方を future fan-out。万が一
  両方が同じ store_dir を見ると lock を取り合う。実際には backend が
  違うので store_dir が違う、deadlock は起きないが verify 必須。
- **lock file の cleanup**: `.qusp.lock` 自体は残しっぱなしで OK
  (advisory lock は process death で auto release される)。ファイルは
  store dir と同じ寿命。
- **Windows 互換**: `fs2` crate は Windows で `LockFileEx` を使うので
  ポータブル。`flock` 直接呼びは避ける。

## 実装ステップ

1. `fs2 = "0.4"` を qusp-core deps に追加
2. `crates/qusp-core/src/effects/lock.rs` 新規
3. 各 backend の `install()` 冒頭で `let _guard = StoreLock::acquire(&store_dir)?;`
4. install_dir の symlink swap を atomic rename に変更
5. unit test (concurrent install を spawn_blocking で 2 並列、両方 success)
6. e2e で 2 並列 install → 1 つは "waiting for lock" log、両方最終的に install_dir 一致を assert

## なぜ Phase 5 外か

これは hospitality (uv 並のホスピタリティ) ではなく **correctness**
の問題。

uv 並ホスピタリティの定義は「使ってて気持ちいい」だが、これは「使うと
壊れる可能性がある」issue。同じく F4 (failed install で qusp.lock
不在) と並ぶ category。

優先度的には Phase 5 と並列で、なるべく早く fix する。dogfood で
qusp daily driver にすると CI / 並列 build で踏む確率が上がる。

## 関連

- F4 (failed install で qusp.lock 不在) ─ 同じ install path の
  correctness 不備。両方を 1 個の "install path hardening" PR で
  まとめて fix する選択肢あり。
