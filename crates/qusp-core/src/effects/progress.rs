//! `ProgressReporter` — explicit progress effect.
//!
//! Phase 5 (Hospitality Parity), audit row A3.
//!
//! Backends used to interleave their own `indicatif::ProgressBar`
//! constructions with download / extract / build steps. The result was
//! visually inconsistent across the 18 backends — Lua's `make` dumped
//! raw gcc output, Haskell's ghcup wrote ANSI cursor-up sequences,
//! Crystal had a silent download. uv shows one polished line per task
//! ("Downloading cpython-3.11.13-macos-x86_64-none (download) (17.5MiB)")
//! and matches across every install.
//!
//! This trait gives backends a uniform place to:
//!
//! 1. **Quantified bar** for downloads (bytes known → progress + ETA).
//! 2. **Spinner** for opaque steps (extract, sha verify, `make`,
//!    `ghcup install ghc`).
//! 3. **Switch** from quantified to spinner when a step transitions
//!    (e.g., download → extract).
//! 4. **Final line** that survives the bar/spinner clearing, so
//!    "✓ python 3.13.0 installed in 2.85s" stays visible.
//!
//! Production = `LiveProgress` (indicatif). Tests + `--quiet` = `NoopProgress`.
//!
//! TTY detection: indicatif's draw target auto-disables when stdout
//! isn't a tty (CI, pipe, redirect). The `--quiet` / `QUSP_QUIET` env
//! var hard-disables in addition.

use std::sync::Arc;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// The trait backends call when they need to show progress for a
/// long-running step.
pub trait ProgressReporter: Send + Sync {
    /// Start a quantified task. `total_bytes = None` becomes a
    /// spinner; `Some(n)` becomes a progress bar with ETA.
    fn start(&self, label: &str, total_bytes: Option<u64>) -> Box<dyn ProgressTask>;
}

/// A single progress line. Drop / `finish` / `fail` retires it.
pub trait ProgressTask: Send {
    /// Add `n` bytes of progress. No-op for spinners.
    fn advance(&mut self, n: u64);
    /// Late-discovered total (e.g., `Content-Length` header arrived
    /// after task start). Upgrades a spinner into a bar.
    fn set_total(&mut self, total: u64);
    /// Replace the visible label (e.g., "downloading" → "extracting").
    fn set_label(&mut self, label: String);
    /// Mark task done. The cleared bar is replaced by a one-line
    /// summary that persists.
    fn finish(&mut self, label: String);
    /// Mark task aborted (failure). Bar is cleared, no persistent line.
    fn fail(&mut self);
}

// ─── LiveProgress (indicatif backed) ────────────────────────────────

#[derive(Clone)]
pub struct LiveProgress {
    multi: Arc<MultiProgress>,
    enabled: bool,
}

impl LiveProgress {
    pub fn new() -> Self {
        Self::with_enabled(true)
    }

    pub fn with_enabled(enabled: bool) -> Self {
        Self {
            multi: Arc::new(MultiProgress::new()),
            enabled,
        }
    }
}

impl Default for LiveProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressReporter for LiveProgress {
    fn start(&self, label: &str, total_bytes: Option<u64>) -> Box<dyn ProgressTask> {
        if !self.enabled {
            return Box::new(NoopTask);
        }
        let pb = match total_bytes {
            Some(n) => {
                let pb = self.multi.add(ProgressBar::new(n));
                pb.set_style(
                    ProgressStyle::with_template(
                        "{msg} {bar:30.cyan/blue} {bytes}/{total_bytes} · {bytes_per_sec} · {eta}",
                    )
                    .unwrap()
                    .progress_chars("━━╸"),
                );
                pb
            }
            None => {
                let pb = self.multi.add(ProgressBar::new_spinner());
                pb.set_style(
                    ProgressStyle::with_template("{spinner:.cyan} {msg} · {elapsed}").unwrap(),
                );
                pb.enable_steady_tick(Duration::from_millis(100));
                pb
            }
        };
        pb.set_message(label.to_string());
        Box::new(LiveTask { pb })
    }
}

struct LiveTask {
    pb: ProgressBar,
}

impl ProgressTask for LiveTask {
    fn advance(&mut self, n: u64) {
        self.pb.inc(n);
    }
    fn set_total(&mut self, total: u64) {
        // If we started as a spinner, indicatif lets us set length and
        // it switches to a bar at the next redraw. Re-style for safety.
        if self.pb.length().is_none() {
            self.pb.set_style(
                ProgressStyle::with_template(
                    "{msg} {bar:30.cyan/blue} {bytes}/{total_bytes} · {bytes_per_sec} · {eta}",
                )
                .unwrap()
                .progress_chars("━━╸"),
            );
        }
        self.pb.set_length(total);
    }
    fn set_label(&mut self, label: String) {
        self.pb.set_message(label);
    }
    fn finish(&mut self, label: String) {
        self.pb.finish_with_message(label);
    }
    fn fail(&mut self) {
        self.pb.finish_and_clear();
    }
}

// ─── NoopProgress (quiet / tests) ───────────────────────────────────

pub struct NoopProgress;

impl ProgressReporter for NoopProgress {
    fn start(&self, _label: &str, _total: Option<u64>) -> Box<dyn ProgressTask> {
        Box::new(NoopTask)
    }
}

struct NoopTask;

impl ProgressTask for NoopTask {
    fn advance(&mut self, _: u64) {}
    fn set_total(&mut self, _: u64) {}
    fn set_label(&mut self, _: String) {}
    fn finish(&mut self, _: String) {}
    fn fail(&mut self) {}
}

// ─── helpers ────────────────────────────────────────────────────────

/// Run a sync subprocess inside a progress spinner. stdout / stderr
/// are captured (not inherited), keeping the user's terminal clean.
/// On failure, the captured output is replayed to stderr so the user
/// can diagnose. On success, the spinner finishes with the
/// `success_label`.
///
/// Used by source-build backends (Lua's `make`, Haskell's
/// `ghcup install ghc`, future PHP/R/etc.) to replace the previous
/// "subprocess spews everything to the terminal" UX with the
/// uv-class single-line `Building X (elapsed)` experience.
pub fn run_with_spinner(
    progress: &dyn ProgressReporter,
    label: &str,
    success_label: String,
    cmd: &mut std::process::Command,
) -> anyhow::Result<()> {
    use std::io::Write;
    let mut task = progress.start(label, None);
    let output = cmd.output();
    match output {
        Ok(out) if out.status.success() => {
            task.finish(success_label);
            Ok(())
        }
        Ok(out) => {
            task.fail();
            // Replay captured streams so the user sees what happened.
            std::io::stderr().write_all(&out.stdout).ok();
            std::io::stderr().write_all(&out.stderr).ok();
            anyhow::bail!("{label} exited with {}", out.status)
        }
        Err(e) => {
            task.fail();
            Err(anyhow::anyhow!("{label}: spawn failed: {e}"))
        }
    }
}
