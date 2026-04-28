# Haskell

**Shipped:** v0.23.0
**Tag:** v0.23.0
**Phase 4 第九弾。Bootstrap-installer wrap パターン初投入。OCaml/opam で再利用予定。**

## 設計判断: ghcup wrap

GHC を qusp が自前で build する方向性は明確に却下した。30 分超の
bootstrap、platform-specific patches、複雑な moving target — qusp が
追従する credible position が無い。**ghcup は Haskell Foundation 公式
の bootstrap installer** なので、信頼ベクタとして採用するのが妥当。

qusp が own するもの:
1. **ghcup binary 自体の install + 検証** — `downloads.haskell.org/ghcup/<v>/SHA256SUMS` から sha256 を取って verify
2. **install root のコントロール** — `GHCUP_INSTALL_BASE_PREFIX` で qusp store dir を指定、`~/.ghcup` には書かせない
3. **PATH 合成** — `qusp run ghc` が `data/haskell/<v>/bin/` 経由で動く、shell rcfile の編集なし

qusp が delegate するもの:
- GHC binary の distribution + 検証 (ghcup の metadata source 経由)
- cabal-install / stack / HLS (将来的に `[haskell.tools]` curated 化、未着手)

## 設計

- **ghcup binary source:** `https://downloads.haskell.org/ghcup/<v>/<triple>-ghcup-<v>`
- **ghcup verification:** 同 `<v>/SHA256SUMS` (BSD-style `<HEX>  ./<filename>`)
- **Triples:** aarch64-apple-darwin, x86_64-apple-darwin, aarch64-linux, x86_64-linux
- **Pinned ghcup version (qusp v0.23.0 release prep):** 0.1.50.2
- **GHC install:** `ghcup install ghc <ghc_v>` を spawn_blocking で起動、`GHCUP_INSTALL_BASE_PREFIX=<store>` で redirect、`GHCUP_USE_XDG_DIRS` は **解除** (それを 1 にすると XDG path に飛んで qusp store の外に出る)
- **Layout post-install:**
  ```
  <store_dir>/
    bin/ghcup                      (qusp-verified)
    .ghcup/                        (ghcup-managed, INSTALL_BASE_PREFIX)
      ghc/<ghc_v>/
        bin/{ghc, ghci, runghc, ...}
        lib/...
  ```
  `data/haskell/<ghc_v>` symlinks to `<store_dir>/.ghcup/ghc/<ghc_v>`。

## 落とし穴: macOS Application Support space trap (4 度目!)

GHC の `./configure` (autoconf-generated) は `--prefix=<path>` を unquoted
で展開する path に弱い。

  ./configure --prefix=/var/.../Library/Application Support/.../ghc/9.10.1

→ `Application Support` の space で word-split が起きて
`./configure: error` で exit 1。

Lua の "stage then move" パターンは **使えない**: GHC は wrapper
scripts と `package.conf.d/*.conf` に prefix を baked-in で書くので、
post-install relocation すると compiler が壊れる。Up-front で
no-space path に install するしかない。

修正: macOS では Haskell 専用の no-space store を使う:

  $HOME/.qusp/haskell-store/<ghcup-hash>/

- `qusp` namespace、persistent (Caches と違って OS が purge しない)
- 接頭辞 `~/Library/Application Support/...` を意図的に避ける
- `$HOME` 自体に space がある場合 (技術系 macOS user では稀) は
  install 前に明確に bail、silent 破損を避ける

Linux/BSD は `~/.local/share/qusp/store/` がもともと no-space なので
diversion 不要。

これで Application Support space trap は **4 ケース目**:
1. v0.18.0 Groovy: upstream `bin/startGroovy` の `JAVA_OPTS` 裸展開
2. v0.21.0 Clojure: 自前 sed 置換時の `install_dir=PREFIX` 裸代入
3. v0.22.0 Lua: upstream `Makefile install:` の `$(INSTALL_BIN)` 裸展開
4. v0.23.0 Haskell: upstream GHC `./configure --prefix=` 裸展開

memory `project_qusp_macos_appsupport_space_trap.md` に記録更新済。
今後の patterns 全てに適用される観察:
- 単純な launcher script は in-place patch で fix できる (Groovy/Clojure)
- Makefile install rule は staging dir で逃げられる (Lua)
- autotools-generated configure は up-front no-space path しか効かない (Haskell)

OCaml も autoconf を経由する build なので同じ no-space store
パターンを再利用する見込み。

## list_remote 設計

ghcup の metadata source (YAML) は schema が rich (release status,
deprecation, viTags) で正確に解釈するのは大変。v0.23.0 では curated
list (9.10.1, 9.8.4, 9.6.6, 9.4.8, 9.2.8) を返す。pin できる version
は ghcup が知ってるもの全て (このリスト外でも install 可能)、新
qusp release で list を refresh する。

## 5 unit tests

- `picks_sha256_for` が `./<filename>` の `./` 接頭辞付き / 無し両方を抽出
- 無関係な test- / test-optparse- variants を skip して exact match
- 不在 filename で None
- ghcup_triple が macOS/Linux x4 + 非対応 OS x2 を網羅
- version_cmp が 9.10.1 > 9.8.4 (10 進比較) を維持

## Smoke-tested

- qusp install haskell 9.10.1 → ghcup binary verify (qusp curated) +
  ghcup install ghc 9.10.1 (~3-5min 初回 download 214MB GHC tarball)
- qusp run ghc --version → "The Glorious Glasgow Haskell Compilation System, version 9.10.1"
- qusp run runghc Hello.hs → "hi from qusp-managed haskell"

cargo test --release: 57/57 (5 new + 52 既存)。
e2e/haskell.sh は GHC 初回 download に時間かかるので DEFAULT のみ
(FAST には載せない)。`E2E_SKIP_HASKELL=1` で CI 個別 skip 可能。

## 非ゴール

- ghcup の置き換え (qusp が GHC を自前 build)
- Cabal package 管理 (Cabal の責務)
- Hackage との直接対話
- HLS の cross-version selection (将来の `[haskell.tools]` で別途)
