//! `nanna doctor` — offline configuration diagnosis.
//!
//! **Root cause, not availability.** The existing `status` command answers "is
//! a thing configured", which is the question that hid the failure class this
//! is written against: our own loopback stream faults were read as provider
//! 502s and answered with restart spirals, because every surface reported
//! *availability* and none reported *why*. Each check here therefore carries a
//! remedy, not just a verdict.
//!
//! **Deliberately offline.** Nothing in this module opens a socket. Network
//! probes (provider reachability, an Ollama ping, key validity) are the other
//! half of the roadmap item and are a separate leg: they are slow, they fail
//! for reasons that have nothing to do with configuration, and mixing them in
//! means a machine with no internet reports its config as broken. What is here
//! runs in milliseconds, is deterministic, and is safe to run anywhere.

use std::fmt;
use std::path::Path;

use nanna_config::Config;

/// How bad a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Checked and correct.
    Ok,
    /// Works, but will surprise someone.
    Warn,
    /// Will not work; this is the root cause of something.
    Fail,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `f.pad`, not `write_str`: width/alignment in the format string is
        // silently ignored by a Display impl that writes directly, which is
        // what left the report's severity column ragged.
        f.pad(match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "FAIL",
        })
    }
}

/// One diagnosis.
#[derive(Debug, Clone)]
pub struct Check {
    /// Short stable name, e.g. `infer.config`.
    pub name: &'static str,
    pub severity: Severity,
    /// What was observed — always a measured fact, never a guess.
    pub detail: String,
    /// What to do about it. `None` only when there is nothing to do.
    pub remedy: Option<String>,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            severity: Severity::Ok,
            detail: detail.into(),
            remedy: None,
        }
    }

    fn warn(name: &'static str, detail: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            name,
            severity: Severity::Warn,
            detail: detail.into(),
            remedy: Some(remedy.into()),
        }
    }

    fn fail(name: &'static str, detail: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            name,
            severity: Severity::Fail,
            detail: detail.into(),
            remedy: Some(remedy.into()),
        }
    }
}

/// Run every offline check against `config`.
///
/// `config_path` is reported rather than read again, so the caller stays the
/// single place that decides where configuration comes from.
#[must_use]
pub fn run_checks(config: &Config, config_path: &Path) -> Vec<Check> {
    let mut checks = Vec::new();

    checks.push(if config_path.exists() {
        Check::ok("config.file", format!("{}", config_path.display()))
    } else {
        Check::warn(
            "config.file",
            format!(
                "{} does not exist; built-in defaults are in use",
                config_path.display()
            ),
            "run `nanna init`, or `nanna config --generate > <path>` to write one",
        )
    });

    checks.push(check_clustering(config));
    checks.push(check_infer(config));
    checks.push(check_server_exposure(config));
    checks.push(check_tools_dir(config));
    checks.push(check_embeddings(config));

    debug_assert!(
        checks
            .iter()
            .all(|c| c.severity == Severity::Ok || c.remedy.is_some()),
        "every non-ok check must carry a remedy — a verdict without one is the thing this replaces"
    );
    checks
}

/// The clustering invariant, checked against the configuration the daemon will
/// actually build.
///
/// `cluster_threshold` is not a `config.toml` field: the daemon constructs a
/// `ConsolidationConfig::default()` and overrides only the two knobs that are
/// (`max_compression_ratio`, `min_remaining_memories`). So this reconstructs
/// that same value rather than reading one, and a mismatch here means the
/// shipped defaults are wrong — which is exactly the state that shipped until
/// 2026-09-09.
fn check_clustering(config: &Config) -> Check {
    let consolidation = nanna_core::ConsolidationConfig {
        max_compression_ratio: config.memory.max_compression_ratio,
        min_remaining_memories: config.memory.min_remaining_memories,
        ..nanna_core::ConsolidationConfig::default()
    };
    match consolidation.validate() {
        Ok(()) => Check::ok(
            "memory.clustering",
            format!(
                "semantic veto intact — a merge needs cosine >= {:.2}",
                consolidation.min_required_similarity()
            ),
        ),
        Err(why) => Check::fail(
            "memory.clustering",
            why.to_string(),
            "dreaming will refuse to run until this is fixed; raise the threshold or lower the \
             non-similarity clustering weights",
        ),
    }
}

/// Local inference config, including the states that are legal but inert.
fn check_infer(config: &Config) -> Check {
    if let Err(why) = config.infer.validate() {
        return Check::fail(
            "infer.config",
            why.to_string(),
            "correct the `[infer]` section, or set `enabled = false` to turn the local tier off",
        );
    }
    if !config.infer.enabled {
        return Check::ok("infer.config", "local inference disabled");
    }
    if !config.infer.has_local_chat_model() {
        return Check::warn(
            "infer.config",
            "local inference is enabled but names no chat model, so only embeddings run locally",
            "set `[infer].model` to a Mummu catalog name, or leave it empty deliberately",
        );
    }
    Check::ok(
        "infer.config",
        format!("local chat model `{}`", config.infer.model),
    )
}

/// `[server].host` is **not** the bind address, and saying so is the check.
///
/// Verified 2026-09-09: nothing reads `nanna_config::ServerConfig::host`. The
/// bind in `nanna_server::start_server` takes `nanna_server::ServerConfig` — a
/// *different* struct — which `commands::serve` builds from the `--host` CLI
/// flag, defaulting to loopback. Only `webhook_secret` is carried over from the
/// config.
///
/// That makes the field worse than unused: it is shaped exactly like a security
/// control. Someone who sets it to `127.0.0.1` has secured nothing, and someone
/// reading the shipped `0.0.0.0` default reasonably concludes the server is
/// exposed when it is not. A doctor that warned about the default would be
/// raising a false alarm — which is the failure this command exists to avoid —
/// so it reports the effective answer instead.
fn check_server_exposure(config: &Config) -> Check {
    if !config.server.enabled {
        return Check::ok("server.bind", "HTTP server disabled");
    }
    if nanna_config::is_loopback_host(&config.server.host) {
        // Harmless, but still not what binds — say so rather than implying the
        // field did the securing.
        return Check::ok(
            "server.bind",
            format!(
                "binds loopback (from --host, default {}); `[server].host = \"{}\"` is not read",
                nanna_config::LOOPBACK_HOST,
                config.server.host
            ),
        );
    }
    Check::warn(
        "server.bind",
        format!(
            "`[server].host = \"{}\"` is NOT read by anything — the bind address comes from the \
             `--host` flag, which defaults to loopback. Setting this field secures nothing and \
             exposes nothing",
            config.server.host
        ),
        format!(
            "pass `--host` to `nanna server` to change the bind; use {} unless exposure is \
             deliberate (the HTTP surface has no authentication of its own)",
            nanna_config::LOOPBACK_HOST
        ),
    )
}

/// A configured tools directory that does not exist means every script tool is
/// silently missing.
fn check_tools_dir(config: &Config) -> Check {
    match config.tools.tools_dir.as_ref() {
        None => Check::ok("tools.dir", "using the default tools directory"),
        Some(dir) if dir.is_dir() => Check::ok("tools.dir", format!("{}", dir.display())),
        Some(dir) => Check::fail(
            "tools.dir",
            format!("{} is configured but is not a directory", dir.display()),
            "create it, or clear `[tools].tools_dir` to fall back to the default",
        ),
    }
}

/// Memory without an embedding provider still stores, but cannot search — the
/// one memory failure a person can actually fix.
fn check_embeddings(config: &Config) -> Check {
    if !config.memory.enabled {
        return Check::ok("memory.embeddings", "memory disabled");
    }
    let provider = config.memory.embedding_provider.trim();
    if provider.is_empty() || provider.eq_ignore_ascii_case("disabled") {
        return Check::warn(
            "memory.embeddings",
            "memory is enabled but no embedding provider is set — memories will be stored and \
             will not be searchable",
            "set `[memory].embedding_provider`, or disable memory if that is intended",
        );
    }
    Check::ok("memory.embeddings", format!("provider `{provider}`"))
}

/// Print the report. Returns the worst severity seen, so the caller can choose
/// an exit code.
pub fn report(checks: &[Check]) -> Severity {
    let worst = checks
        .iter()
        .map(|c| c.severity)
        .max()
        .unwrap_or(Severity::Ok);

    println!("Nanna doctor — offline configuration checks");
    println!("{}", "─".repeat(44));
    for check in checks {
        println!(
            "[{:>4}] {:<20} {}",
            check.severity, check.name, check.detail
        );
        if let Some(remedy) = &check.remedy {
            println!("       {:<20} → {remedy}", "");
        }
    }
    println!("{}", "─".repeat(44));
    match worst {
        Severity::Ok => println!("All {} checks passed.", checks.len()),
        Severity::Warn => println!("Some checks warned; nothing is broken."),
        Severity::Fail => println!("Something will not work — see the FAIL lines above."),
    }
    println!(
        "\nThis pass is offline: no provider, network or keyring probe ran. \
         Availability is a separate question from configuration."
    );
    worst
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn the_shipped_defaults_are_healthy() {
        let checks = run_checks(&cfg(), Path::new("/nonexistent/config.toml"));
        let failures: Vec<_> = checks
            .iter()
            .filter(|c| c.severity == Severity::Fail)
            .collect();
        assert!(
            failures.is_empty(),
            "shipped defaults must not fail their own doctor: {failures:?}"
        );
    }

    #[test]
    fn every_non_ok_check_carries_a_remedy() {
        // The whole point of this command over `status`: a verdict with no
        // remedy is the thing it replaces.
        let mut config = cfg();
        config.server.host = "0.0.0.0".to_string();
        config.tools.tools_dir = Some("/definitely/not/a/real/dir".into());
        config.memory.embedding_provider = "disabled".to_string();
        config.infer.enabled = true;
        config.infer.vram_budget_bytes = Some(0);

        let checks = run_checks(&config, Path::new("/nonexistent/config.toml"));
        for check in &checks {
            if check.severity != Severity::Ok {
                assert!(
                    check.remedy.as_ref().is_some_and(|r| !r.trim().is_empty()),
                    "{} has no remedy",
                    check.name
                );
            }
        }
        assert!(
            checks.iter().any(|c| c.severity == Severity::Fail),
            "that config has real faults and the doctor must say so"
        );
    }

    #[test]
    fn a_missing_tools_dir_is_a_failure_not_a_warning() {
        let mut config = cfg();
        config.tools.tools_dir = Some("/definitely/not/a/real/dir".into());
        let checks = run_checks(&config, Path::new("/nonexistent/config.toml"));
        let check = checks
            .iter()
            .find(|c| c.name == "tools.dir")
            .expect("tools.dir is always checked");
        assert_eq!(
            check.severity,
            Severity::Fail,
            "every script tool would be silently missing"
        );
    }

    #[test]
    fn the_server_host_field_is_reported_as_the_dead_field_it_is() {
        let mut config = cfg();
        config.server.enabled = true;
        config.server.host = "0.0.0.0".to_string();
        let checks = run_checks(&config, Path::new("/x"));
        let check = checks.iter().find(|c| c.name == "server.bind").unwrap();
        assert_eq!(check.severity, Severity::Warn);
        assert!(
            check.detail.contains("NOT read"),
            "the point is that the field is inert, not that 0.0.0.0 is exposed: {}",
            check.detail
        );
        assert!(
            check.remedy.as_ref().is_some_and(|r| r.contains("--host")),
            "the remedy must name the thing that actually binds: {check:?}"
        );
    }

    #[test]
    fn the_clustering_check_reports_the_real_semantic_bar() {
        let checks = run_checks(&cfg(), Path::new("/x"));
        let check = checks
            .iter()
            .find(|c| c.name == "memory.clustering")
            .unwrap();
        assert_eq!(check.severity, Severity::Ok);
        // The number a person needs in order to reason about merges — not
        // `cluster_threshold`, which reads higher than what it demands.
        assert!(
            check.detail.contains("cosine >="),
            "must report the effective bar: {}",
            check.detail
        );
    }

    #[test]
    fn severity_honours_format_width() {
        // Regression: a Display impl using `write_str` ignores width, which
        // left the report column ragged.
        assert_eq!(format!("[{:>4}]", Severity::Ok), "[  ok]");
        assert_eq!(format!("[{:>4}]", Severity::Warn), "[warn]");
        assert_eq!(format!("[{:>4}]", Severity::Fail), "[FAIL]");
    }

    #[test]
    fn severity_orders_so_the_worst_wins() {
        assert!(Severity::Fail > Severity::Warn);
        assert!(Severity::Warn > Severity::Ok);
    }
}
