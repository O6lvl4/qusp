# Clojure

**Shipped:** v0.21.0
**Tag:** v0.21.0
**Phase 4 第七弾。Direct GitHub release tarball + cross-backend [java]、Scala v0.20.0 と同形。**

## 設計判断: Coursier wrap 不要

元 on-hold ノートは Scala backend と Coursier instance を共有する案
を提案していたが、Scala v0.20.0 で Coursier 自体を捨てた (per-asset
.sha256 sidecar が出てたので不要に)。Clojure も `clojure/brew-install`
が `clojure-tools-<v>.tar.gz` + `.sha256` sidecar を素直に出していて、
完全に Direct download パターンで成立した。

結果として Scala / Clojure の 2 backend は共通する infrastructure を
持たない、独立 backend として実装。Layer 削減 + Coursier の trust 不要。

## 設計

- **Source:** `https://github.com/clojure/brew-install/releases/download/<v>/clojure-tools-<v>.tar.gz`
- **Verification:** 同 URL + `.sha256` sidecar (bare hex single line)
- **Layout (post-extract):** flat `clojure-tools/{clojure, clj, deps.edn,
  example-deps.edn, tools.edn, exec.jar, clojure-tools-<v>.jar,
  clojure.1, clj.1, install.sh}`
- **Detect:** `.clojure-version`, `deps.edn`
- **`requires = ["java"]`**: launcher は内部で `exec java -cp ... clojure.main`
  する pure-bash script。JDK 必須。
- **list_remote:** GitHub releases API、pre-release は除外。

## 上流の install.sh を Rust で再実装

upstream の `posix-install.sh` は flat な tarball を FHS 風に
再配置する shell script だが、qusp は subprocess で動かさず Rust で
忠実に再実装する (ファイルコピー + sed 置換)。理由は:

- subprocess 起動の environment leak を避ける。
- `posix-install.sh` は `prefix=$HOME` 既定で動くが、qusp は store
  hash ディレクトリに置きたい。`-p` 引数経由で渡せばいいが、shell
  trust が増えるだけ。
- macOS では `sed -i ''` (empty backup arg) と Linux の `sed -i` で
  syntax が違う。Rust なら平和。

再配置:
```
clojure-tools/<flat>      →  prefix/bin/{clojure, clj}
                              prefix/lib/clojure/{deps.edn, example-deps.edn, tools.edn}
                              prefix/lib/clojure/libexec/{exec.jar, clojure-tools-<v>.jar}
                              prefix/share/man/man1/{clojure.1, clj.1}
```
launcher の `PREFIX` / `BINDIR` プレースホルダは sed 同様に文字列
置換、ただし **single-quote-escape** で wrap する (下記)。

## 落とし穴: macOS Application Support の space trap (再演)

upstream の `clojure` launcher は冒頭で

  install_dir=PREFIX

と裸代入して、後段の使用は `"$install_dir/..."` と quoted。空の
`PREFIX` を qusp data dir (`~/Library/Application Support/dev.O6lvl4.qusp/...`)
で置換すると、代入行で word-split が起き

  install_dir=/Users/.../Library/Application
  Support/dev.O6lvl4.qusp/store/.../clojure: command not found

を出す (Groovy v0.18.0 で踏んだ Application Support space trap の
launcher 版)。

修正: `shell_single_quote()` ヘルパで `'<path>'` 形式に wrap して
sed 置換するので、結果は

  install_dir='/Users/.../Application Support/.../clojure'

になり bash assignment 安全。embed された `'` は `'\''` で
close-escape-reopen する (qusp data path には現実的に出てこないが
未来の堅牢性のため)。

clj 側の `bin_dir=BINDIR` も同形なので同じ処理。

4 unit tests:
- parses_clojure_sha256_sidecar: bare hex 形式 (Dart の BSD `*`-suffix と異なる)
- shell_single_quote_handles_application_support_path: 実 prefix path quote
- shell_single_quote_escapes_embedded_apostrophe: `a'b` → `'a'\''b'`
- version_cmp_orders_clojure_4_segment: 4-segment Maven 風 `1.12.4.1618` の比較

## Smoke-tested

- `qusp install` → java + clojure 並列 install、8.49s
- `qusp run clojure --version` → "Clojure CLI version 1.12.4.1618"
- `qusp run clojure -M -e '(println "hi from qusp-managed clojure")'` →
  Maven Central から clojure-1.12.4 を初回 download 後 "hi from ..."

## 非ゴール

- Leiningen の管理 (Phase 5 で再検討)
- ClojureScript 個別管理 (Clojure CLI 経由で済む)
- Maven Central package のローカル resolve (Clojure CLI の責務)
