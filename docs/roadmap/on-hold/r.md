# R

**優先度:** Phase 4 (2.x+)
**難易度:** 高 (prebuilt が薄い、distro 依存性が強い)

## なぜ

統計・データ分析の第一選択肢。bioinformatics / academic / 政策分析の固定需要。
mise が対応してる主要言語のうち qusp が「やる気がない」と思われたくない領域。

## 設計

R の distribution 状況は qusp の他 backend と毛色が違う:

- **macOS:** CRAN が `.pkg` (graphical installer) 配布。`.tar.gz` は source のみ。
- **Linux:** distro repos (apt/yum) が主流。CRAN の `.deb`/`.rpm` 直接 install もある。
- **Source build:** CRAN の `https://cran.r-project.org/src/base/R-{major}/R-{version}.tar.gz`、`./configure && make`、build 時間 ~10 分

### 現実的な path

mise / asdf は **`r-build`** という ruby-build フォークを内部利用してる。同じ手を使う:

- Backend: `crates/qusp-core/src/backends/r.rs`
- Source: CRAN tarball + sha256 (CRAN は MD5 / SHA-256 を published page に併載)
- Build: spawn_blocking で `./configure && make` (BLAS / LAPACK / readline / curl などの OS deps が必要)
- Layout: `bin/{R, Rscript}`, `lib64/R/`, `share/man/`

## 設計上の悩み

- **OS-level deps の多さ**: BLAS, LAPACK, libreadline, libcurl, libxml2, …。qusp の責務外、README で前提を列挙
- **macOS の `.pkg` を使えるか**: 使うと `sudo` を要求するインストーラを起動することになる。**避ける**。CRAN macOS binary を tarball 展開する path を別途確立する (`https://mac.r-project.org/`)
- **CRAN package (R 内 packages, devtools 経由)** は qusp 範囲外
- **R-devel / R-patched** などの非-stable channel もある、Phase 1.5 では stable only

## 非ゴール

- R packages (`install.packages()`) 管理。
- Bioconductor 管理。
- RStudio IDE 管理。

## 実装ステップ

1. `crates/qusp-core/src/backends/r.rs` (source build path)
2. macOS は `mac.r-project.org` の prebuilt tar.gz を試す (まだ可能性確認要)
3. e2e/r.sh — CI で BLAS/LAPACK 等 prereq install 必要
4. README に「R は重い deps が要る」 warning
