//! Java backend — Foojay-resolved JDK + Maven/Gradle as tools.
//!
//! JDK distribution is fragmented (Temurin, Corretto, Zulu, GraalVM
//! Community, …). qusp picks **Temurin by default** and lets the user
//! override per-project via `qusp.toml`:
//!
//! ```toml
//! [java]
//! version = "21.0.5"
//! distribution = "temurin"   # or "corretto" | "zulu" | "graalvm_community"
//! ```
//!
//! Resolution goes through the [Foojay disco API], which normalizes
//! every major distribution behind one schema and exposes per-asset
//! checksums. qusp downloads from the publisher's CDN (Adoptium, Amazon,
//! Azul, Oracle Labs), sha256-verifies, extracts to a content-addressed
//! store, and symlinks `versions/java/<version>` at the JAVA_HOME inside.
//!
//! Curated tool registry: **`mvn`** (Apache Maven) and **`gradle`**
//! (Gradle). Both ship as JVM-launching shell scripts that pick up the
//! ambient JAVA_HOME — exactly what `qusp run`'s merged env provides.
//!
//! [Foojay disco API]: https://api.foojay.io/swagger-ui/

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use anyv_core::extract::extract_archive;
use anyv_core::Paths as AnyvPaths;
use async_trait::async_trait;
use serde::Deserialize;
use sha2::Digest;

use crate::backend::*;

pub struct JavaBackend;

const FOOJAY_BASE: &str = "https://api.foojay.io/disco/v3.0";
const DEFAULT_DISTRIBUTION: &str = "temurin";

/// Curated tools. Both are JVM launchers — they pick up JAVA_HOME from
/// the env qusp builds.
const REGISTRY: &[(&str, &str)] = &[("mvn", "maven"), ("gradle", "gradle")];

pub fn registry_lookup(name: &str) -> Option<&'static str> {
    REGISTRY.iter().find(|(k, _)| *k == name).map(|(_, p)| *p)
}

fn paths() -> Result<AnyvPaths> {
    AnyvPaths::discover("qusp")
}

fn java_root(p: &AnyvPaths, version: &str, distribution: &str) -> PathBuf {
    p.data
        .join("java")
        .join(format!("{distribution}-{version}"))
}

fn tools_root(p: &AnyvPaths) -> PathBuf {
    p.data.join("java-tools")
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("qusp-java/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn foojay_os() -> Option<&'static str> {
    Some(match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        _ => return None,
    })
}

fn foojay_arch() -> Option<&'static str> {
    Some(match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "aarch64",
        _ => return None,
    })
}

fn foojay_archive_type() -> &'static str {
    if cfg!(windows) {
        "zip"
    } else {
        "tar.gz"
    }
}

#[derive(Deserialize, Debug)]
struct PackagesResp {
    result: Vec<FoojayPackage>,
}

#[derive(Deserialize, Debug)]
struct FoojayPackage {
    id: String,
    #[serde(default)]
    java_version: String,
    #[serde(default)]
    release_status: String,
}

#[derive(Deserialize, Debug)]
struct PackageDetailResp {
    result: Vec<FoojayPackageDetail>,
}

#[derive(Deserialize, Debug)]
struct FoojayPackageDetail {
    filename: String,
    direct_download_uri: String,
    #[serde(default)]
    checksum: String,
    #[serde(default)]
    checksum_uri: String,
    #[serde(default)]
    checksum_type: String,
}

#[async_trait]
impl Backend for JavaBackend {
    fn id(&self) -> &'static str {
        "java"
    }
    fn manifest_files(&self) -> &[&'static str] {
        &[
            ".java-version",
            ".sdkmanrc",
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
        ]
    }
    fn knows_tool(&self, name: &str) -> bool {
        registry_lookup(name).is_some()
    }

    async fn detect_version(&self, cwd: &Path) -> Result<Option<DetectedVersion>> {
        let mut dir: Option<&Path> = Some(cwd);
        while let Some(d) = dir {
            let f = d.join(".java-version");
            if f.is_file() {
                let raw = std::fs::read_to_string(&f).unwrap_or_default();
                let v = raw.trim().to_string();
                if !v.is_empty() {
                    return Ok(Some(DetectedVersion {
                        version: v,
                        source: ".java-version".into(),
                        origin: f,
                    }));
                }
            }
            dir = d.parent();
        }
        Ok(None)
    }

    async fn install(
        &self,
        _qusp_paths: &AnyvPaths,
        version: &str,
        opts: &InstallOpts,
    ) -> Result<InstallReport> {
        let paths = paths()?;
        paths.ensure_dirs()?;
        let distribution = opts
            .distribution
            .clone()
            .unwrap_or_else(|| DEFAULT_DISTRIBUTION.to_string());
        let install_dir = java_root(&paths, version, &distribution);
        if java_home_bin(&install_dir).is_some() {
            return Ok(InstallReport {
                version: version.to_string(),
                install_dir,
                already_present: true,
            });
        }

        let os = foojay_os()
            .ok_or_else(|| anyhow!("Foojay has no JDK packaging for this OS"))?;
        let arch = foojay_arch()
            .ok_or_else(|| anyhow!("Foojay has no JDK packaging for this architecture"))?;
        let archive_type = foojay_archive_type();
        let client = http_client()?;

        // Step 1 — search packages.
        let search_url = format!(
            "{FOOJAY_BASE}/packages?\
             version={version}&distribution={distribution}\
             &operating_system={os}&architecture={arch}\
             &archive_type={archive_type}&package_type=jdk\
             &directly_downloadable=true&latest=available\
             &javafx_bundled=false"
        );
        let pkgs: PackagesResp = client
            .get(&search_url)
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("Foojay search failed: {search_url}"))?
            .json()
            .await
            .context("parse Foojay packages response")?;

        let pkg = pkgs
            .result
            .iter()
            .find(|p| p.release_status.eq_ignore_ascii_case("ga"))
            .or_else(|| pkgs.result.first())
            .ok_or_else(|| {
                anyhow!(
                    "Foojay returned no JDK matching {distribution} {version} on {os}/{arch}"
                )
            })?;

        // Step 2 — fetch package details (download URL + checksum).
        let detail_url = format!("{FOOJAY_BASE}/ids/{}", pkg.id);
        let detail: PackageDetailResp = client
            .get(&detail_url)
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("Foojay detail fetch failed: {detail_url}"))?
            .json()
            .await
            .context("parse Foojay detail response")?;
        let d = detail
            .result
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Foojay returned an empty detail array for {}", pkg.id))?;

        // Pull the expected sha256. Foojay sometimes inlines `checksum`,
        // sometimes only `checksum_uri`. Try inline first, fall back to GET.
        let expected = if d.checksum_type.eq_ignore_ascii_case("sha256") && !d.checksum.is_empty()
        {
            d.checksum.clone()
        } else if !d.checksum_uri.is_empty() {
            let text = client
                .get(&d.checksum_uri)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            // Files are typically `<sha>  <filename>` or `<sha>` alone.
            text.split_whitespace()
                .next()
                .ok_or_else(|| anyhow!("checksum_uri returned empty body"))?
                .to_string()
        } else {
            bail!(
                "Foojay package {} for {} {distribution} has no sha256 checksum — refusing to install",
                d.filename,
                version
            );
        };

        // Step 3 — download.
        let bytes = client
            .get(&d.direct_download_uri)
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("download {}", d.direct_download_uri))?
            .bytes()
            .await?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let actual = hex::encode(hasher.finalize());
        if !expected.eq_ignore_ascii_case(&actual) {
            bail!(
                "sha256 mismatch for {}: expected {expected}, got {actual}",
                d.filename
            );
        }

        // Step 4 — extract into staging then promote to the store.
        let cache_path = paths.cache.join(&d.filename);
        anyv_core::paths::ensure_dir(&paths.cache)?;
        std::fs::write(&cache_path, &bytes)?;
        let store_dir = paths.store().join(&actual[..16]);
        if store_dir.exists() {
            std::fs::remove_dir_all(&store_dir).ok();
        }
        anyv_core::paths::ensure_dir(&store_dir)?;
        extract_archive(&cache_path, &store_dir)?;
        let _ = std::fs::remove_file(&cache_path);

        // Find the actual JAVA_HOME inside the extracted tree.
        let java_home = locate_java_home(&store_dir).with_context(|| {
            format!(
                "could not find a `bin/java` inside extracted {}",
                store_dir.display()
            )
        })?;

        if let Some(parent) = install_dir.parent() {
            anyv_core::paths::ensure_dir(parent)?;
        }
        if install_dir.exists() || install_dir.is_symlink() {
            let _ = std::fs::remove_file(&install_dir);
            let _ = std::fs::remove_dir_all(&install_dir);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&java_home, &install_dir).with_context(|| {
            format!(
                "symlink {} → {}",
                install_dir.display(),
                java_home.display()
            )
        })?;
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&java_home, &install_dir).with_context(|| {
            format!(
                "symlink {} → {}",
                install_dir.display(),
                java_home.display()
            )
        })?;

        Ok(InstallReport {
            version: pkg.java_version.clone(),
            install_dir,
            already_present: false,
        })
    }

    fn uninstall(&self, _: &AnyvPaths, version: &str) -> Result<()> {
        let paths = paths()?;
        // We don't know which distribution was used at uninstall time;
        // remove every install matching this version.
        let dir = paths.data.join("java");
        if !dir.exists() {
            bail!("java {version} is not installed via qusp");
        }
        let mut removed = 0;
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            let n = e.file_name().to_string_lossy().into_owned();
            if n.ends_with(&format!("-{version}")) {
                let p = e.path();
                std::fs::remove_file(&p)
                    .or_else(|_| std::fs::remove_dir_all(&p))
                    .with_context(|| format!("remove {}", p.display()))?;
                removed += 1;
            }
        }
        if removed == 0 {
            bail!("java {version} is not installed via qusp");
        }
        Ok(())
    }

    fn list_installed(&self, _: &AnyvPaths) -> Result<Vec<String>> {
        let paths = paths()?;
        let dir = paths.data.join("java");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for e in std::fs::read_dir(&dir)? {
            let e = e?;
            out.push(e.file_name().to_string_lossy().into_owned());
        }
        out.sort();
        out.reverse();
        Ok(out)
    }

    async fn list_remote(&self, client: &reqwest::Client) -> Result<Vec<String>> {
        // Surface major + LTS versions known to Foojay so users see what's
        // actually available without paginating thousands of point releases.
        let url = format!("{FOOJAY_BASE}/major_versions?ga=true");
        #[derive(Deserialize)]
        struct R {
            result: Vec<MV>,
        }
        #[derive(Deserialize)]
        struct MV {
            major_version: u32,
            #[serde(default)]
            term_of_support: String,
            versions: Vec<String>,
        }
        let r: R = client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut out = Vec::new();
        for mv in r.result {
            let lts = mv.term_of_support.eq_ignore_ascii_case("lts");
            let suffix = if lts { " (LTS)" } else { "" };
            // Take the freshest patch line for this major version.
            if let Some(top) = mv.versions.first() {
                out.push(format!("{}{suffix}", top));
            } else {
                out.push(format!("{}{suffix}", mv.major_version));
            }
        }
        Ok(out)
    }

    async fn resolve_tool(
        &self,
        client: &reqwest::Client,
        name: &str,
        spec: &ToolSpec,
    ) -> Result<ResolvedTool> {
        let pkg = spec
            .package_override()
            .map(String::from)
            .or_else(|| registry_lookup(name).map(String::from))
            .ok_or_else(|| {
                anyhow!(
                    "java tool '{name}' is not in the curated registry (mvn, gradle). \
                     Pin manually under [java.tools]."
                )
            })?;
        match pkg.as_str() {
            "maven" => resolve_maven(client, name, spec.version()).await,
            "gradle" => resolve_gradle(client, name, spec.version()).await,
            other => bail!("internal: unknown java tool package '{other}'"),
        }
    }

    async fn install_tool(
        &self,
        _qusp_paths: &AnyvPaths,
        toolchain_version: &str,
        resolved: &ResolvedTool,
    ) -> Result<LockedTool> {
        let paths = paths()?;
        let client = http_client()?;
        let bytes = client
            .get(&resolved.bin)
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("download {}", resolved.bin))?
            .bytes()
            .await?;
        // Maven publishes sha512, Gradle publishes sha256. Disambiguate
        // by hex-length (128 vs 64). Anything else is an error.
        let actual = match resolved.upstream_hash.len() {
            64 => hex::encode(sha2::Sha256::digest(&bytes)),
            128 => hex::encode(sha2::Sha512::digest(&bytes)),
            other => bail!(
                "java tool {}@{}: upstream hash length {} is neither sha256 (64) nor sha512 (128)",
                resolved.package,
                resolved.version,
                other
            ),
        };
        if !resolved.upstream_hash.eq_ignore_ascii_case(&actual) {
            bail!(
                "checksum mismatch for {} {}: expected {}, got {actual}",
                resolved.package,
                resolved.version,
                resolved.upstream_hash
            );
        }
        let store_dir = tools_root(&paths)
            .join(&resolved.package)
            .join(&resolved.version)
            .join(&actual[..16]);
        if !store_dir.join("__extracted__").exists() {
            anyv_core::paths::ensure_dir(&store_dir)?;
            let cache_name = filename_from_url(&resolved.bin)
                .unwrap_or_else(|| format!("{}-{}.archive", resolved.package, resolved.version));
            let cache_path = paths.cache.join(&cache_name);
            anyv_core::paths::ensure_dir(&paths.cache)?;
            std::fs::write(&cache_path, &bytes)?;
            extract_archive(&cache_path, &store_dir)?;
            let _ = std::fs::remove_file(&cache_path);
            let _ = std::fs::write(store_dir.join("__extracted__"), b"");
        }
        let bin_path = locate_tool_bin(&store_dir, &resolved.package, &resolved.name)
            .with_context(|| {
                format!(
                    "could not find {}/bin/{} after extracting to {}",
                    resolved.package,
                    resolved.name,
                    store_dir.display()
                )
            })?;
        Ok(LockedTool {
            name: resolved.name.clone(),
            package: resolved.package.clone(),
            version: resolved.version.clone(),
            bin: bin_path.to_string_lossy().into_owned(),
            upstream_hash: resolved.upstream_hash.clone(),
            built_with: toolchain_version.to_string(),
        })
    }

    fn tool_bin_path(&self, _: &AnyvPaths, locked: &LockedTool) -> PathBuf {
        PathBuf::from(&locked.bin)
    }

    fn build_run_env(&self, _: &AnyvPaths, version: &str, _cwd: &Path) -> Result<RunEnv> {
        // `version` may or may not embed the distribution prefix
        // ("temurin-21.0.5"). Try the embedded form first; fall back to
        // the default.
        let paths = paths()?;
        let candidates: Vec<PathBuf> = if version.contains('-') {
            vec![paths.data.join("java").join(version)]
        } else {
            vec![
                paths
                    .data
                    .join("java")
                    .join(format!("{DEFAULT_DISTRIBUTION}-{version}")),
                // Fall back to whichever distribution is installed for this version.
                find_any_distribution(&paths, version)?
                    .unwrap_or_else(|| paths.data.join("java").join(version)),
            ]
        };
        let java_home = candidates.into_iter().find(|p| p.exists()).ok_or_else(|| {
            anyhow!(
                "no java toolchain installed for version '{version}' \
                 (looked under {})",
                paths.data.join("java").display()
            )
        })?;
        let mut env = std::collections::BTreeMap::new();
        env.insert("JAVA_HOME".into(), java_home.to_string_lossy().into_owned());
        env.insert("JDK_HOME".into(), java_home.to_string_lossy().into_owned());
        Ok(RunEnv {
            path_prepend: vec![java_home.join("bin")],
            env,
        })
    }
}

fn java_home_bin(install_dir: &Path) -> Option<PathBuf> {
    let candidate = install_dir.join("bin").join("java");
    if candidate.is_file() {
        Some(candidate)
    } else {
        None
    }
}

/// After extracting a JDK archive, find the JAVA_HOME inside. macOS
/// JDKs nest `Contents/Home/`; Linux JDKs are flat. The archive always
/// contains a single top-level dir (`jdk-…`).
fn locate_java_home(store_dir: &Path) -> Result<PathBuf> {
    let mut top = None;
    for e in std::fs::read_dir(store_dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            top = Some(e.path());
            break;
        }
    }
    let top = top.ok_or_else(|| anyhow!("no top-level dir in extracted archive"))?;
    let macos = top.join("Contents/Home");
    if macos.join("bin/java").is_file() {
        return Ok(macos);
    }
    if top.join("bin/java").is_file() {
        return Ok(top);
    }
    bail!(
        "extracted archive layout did not match (no `bin/java` at {} or {})",
        top.display(),
        macos.display()
    )
}

/// Walk the install dirs to find a JDK matching `version` for any distribution.
fn find_any_distribution(p: &AnyvPaths, version: &str) -> Result<Option<PathBuf>> {
    let dir = p.data.join("java");
    if !dir.exists() {
        return Ok(None);
    }
    for e in std::fs::read_dir(&dir)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(&format!("-{version}")) {
            return Ok(Some(e.path()));
        }
    }
    Ok(None)
}

fn locate_tool_bin(store_dir: &Path, package: &str, bin_name: &str) -> Result<PathBuf> {
    // tools extract to `<store_dir>/<top>/bin/<bin_name>`. Find it.
    for e in std::fs::read_dir(store_dir)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            let candidate = e.path().join("bin").join(bin_name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    bail!(
        "no {package}/bin/{bin_name} found inside {}",
        store_dir.display()
    )
}

fn filename_from_url(url: &str) -> Option<String> {
    url.rsplit('/').next().map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Tool resolvers (Maven + Gradle)
// ---------------------------------------------------------------------------

async fn resolve_maven(
    client: &reqwest::Client,
    tool_name: &str,
    version: &str,
) -> Result<ResolvedTool> {
    let v = if version == "latest" {
        latest_maven_version(client).await?
    } else {
        version.to_string()
    };
    let line = if v.starts_with("4.") { "maven-4" } else { "maven-3" };
    let asset = format!("apache-maven-{v}-bin.tar.gz");
    let asset_url = format!("https://archive.apache.org/dist/maven/{line}/{v}/binaries/{asset}");
    let sha_url = format!("{asset_url}.sha512");
    let sha_text = client
        .get(&sha_url)
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("fetch {sha_url}"))?
        .text()
        .await?;
    let sha = sha_text
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("empty Maven .sha512"))?
        .to_string();
    Ok(ResolvedTool {
        name: tool_name.to_string(),
        package: "maven".into(),
        version: v,
        bin: asset_url,
        // Apache publishes sha512 for Maven binaries.
        upstream_hash: sha,
    })
}

async fn resolve_gradle(
    client: &reqwest::Client,
    tool_name: &str,
    version: &str,
) -> Result<ResolvedTool> {
    let v = if version == "latest" {
        latest_gradle_version(client).await?
    } else {
        version.to_string()
    };
    let asset = format!("gradle-{v}-bin.zip");
    let asset_url = format!("https://services.gradle.org/distributions/{asset}");
    let sha_url = format!("{asset_url}.sha256");
    let sha_text = client
        .get(&sha_url)
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("fetch {sha_url}"))?
        .text()
        .await?;
    let sha = sha_text
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("empty Gradle .sha256"))?
        .to_string();
    Ok(ResolvedTool {
        name: tool_name.to_string(),
        package: "gradle".into(),
        version: v,
        bin: asset_url,
        upstream_hash: sha,
    })
}

async fn latest_maven_version(client: &reqwest::Client) -> Result<String> {
    // Apache's `maven-metadata.xml` has the canonical "latest" / "release"
    // tags; cheaper to scrape the directory listing.
    let html = client
        .get("https://archive.apache.org/dist/maven/maven-3/")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut versions: Vec<String> = html
        .split('"')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.strip_suffix('/'))
        .filter(|s| s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
        .map(String::from)
        .collect();
    versions.sort_by(|a, b| version_cmp(b, a));
    versions
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("could not parse latest Maven version from Apache index"))
}

async fn latest_gradle_version(client: &reqwest::Client) -> Result<String> {
    #[derive(Deserialize)]
    struct R {
        version: String,
    }
    let r: R = client
        .get("https://services.gradle.org/versions/current")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(r.version)
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(s: &str) -> (u64, u64, u64, u64) {
        let mut p = s.split('.').map(|x| x.parse::<u64>().unwrap_or(0));
        (
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
            p.next().unwrap_or(0),
        )
    }
    parts(a).cmp(&parts(b))
}
