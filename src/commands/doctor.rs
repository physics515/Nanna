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
    checks.push(check_server_exposure());
    checks.push(check_tools_dir(config));
    checks.push(check_embeddings(config));
    checks.push(check_ollama_servers(config));

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

/// Where `nanna server` binds, stated from the one place that decides it.
///
/// The bind address comes only from the `--host` flag, which defaults to
/// loopback: `commands::serve` builds `nanna_server::ServerConfig` from it and
/// carries nothing else over but `webhook_secret`. There is deliberately no
/// config key for it. `[server].host` existed until 2026-09-11 and nothing ever
/// read it — it was shaped exactly like a security control while controlling
/// nothing, so it was deleted rather than wired (wiring it under its old
/// `0.0.0.0` default would have exposed an HTTP surface with no authentication
/// of its own). A stale key left in an old `config.toml` is still ignored.
///
/// No branch on `[server].enabled` either: `nanna server` does not read it, so
/// reporting "disabled" from it would be the false alarm this command exists to
/// replace.
fn check_server_exposure() -> Check {
    Check::ok(
        "server.bind",
        format!(
            "`nanna server` binds {} unless started with `--host`; no config key sets the bind \
             address",
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

/// Ollama's own default port, which a base URL without one means.
const OLLAMA_DEFAULT_PORT: u16 = 11434;

/// What the summarizer falls back to when `[llm].ollama_url` is unset.
const OLLAMA_DEFAULT_URL: &str = "http://localhost:11434";

/// Two Ollama servers configured without meaning to.
///
/// Chat and embeddings reach Ollama through `[memory].ollama_host`;
/// summarization — the model behind dreaming and context compression —
/// through `[llm].ollama_url`, which defaults to localhost. Point the first at a
/// GPU box and summaries still go to localhost. Found 2026-09-11 in a smoke
/// run. Which key should win is an owner call — a saved config carries the
/// default on disk, so code cannot tell a deliberate split from an untouched
/// one — so this says so rather than guessing.
fn check_ollama_servers(config: &Config) -> Check {
    let summarizes_on_ollama = config
        .llm
        .summarization_priority
        .iter()
        .any(|m| is_ollama_spec(m));
    let chats_on_ollama = config.llm.provider.eq_ignore_ascii_case("ollama")
        || config.llm.model_priority.iter().any(|m| is_ollama_spec(m))
        || config
            .memory
            .embedding_provider
            .eq_ignore_ascii_case("ollama")
        || config
            .memory
            .embedding_priority
            .iter()
            .any(|m| is_ollama_spec(m));
    if !(summarizes_on_ollama && chats_on_ollama) {
        return Check::ok("ollama.servers", "at most one Ollama server is in use");
    }
    let chat_url = config.memory.ollama_host.as_str();
    let summary_url = config
        .llm
        .ollama_url
        .as_deref()
        .unwrap_or(OLLAMA_DEFAULT_URL);
    if ollama_endpoint(chat_url) == ollama_endpoint(summary_url) {
        return Check::ok(
            "ollama.servers",
            format!("chat, embeddings and summarization share {chat_url}"),
        );
    }
    Check::warn(
        "ollama.servers",
        format!(
            "chat and embeddings use `[memory].ollama_host = \"{chat_url}\"` but summarization \
             uses `[llm].ollama_url = \"{summary_url}\"` — two different Ollama servers"
        ),
        "set both to the same URL, unless summarizing on a separate server is deliberate",
    )
}

/// Does a model spec name an Ollama model? `ollama/<model>`, or a bare
/// `name:tag` (how Ollama ids look, and how the router detects them).
fn is_ollama_spec(model: &str) -> bool {
    let model = model.trim();
    model.starts_with("ollama/") || (!model.contains('/') && model.contains(':'))
}

/// `(host, port)` of an Ollama base URL, the host lowercased and every
/// loopback spelling folded to one — `localhost`, `127.0.0.1` and `[::1]` on the
/// same port are the same server. A missing port means Ollama's default.
fn ollama_endpoint(url: &str) -> (String, u16) {
    let trimmed = url.trim();
    let rest = trimmed.split_once("://").map_or(trimmed, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or_default();
    let (host, port) = authority.strip_prefix('[').map_or_else(
        || match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), Some(port)),
            None => (authority.to_string(), None),
        },
        |bracketed| match bracketed.split_once(']') {
            Some((inner, tail)) => (format!("[{inner}]"), tail.strip_prefix(':')),
            None => (authority.to_string(), None),
        },
    );
    let port = port
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(OLLAMA_DEFAULT_PORT);
    let host = if nanna_config::is_loopback_host(&host) {
        "loopback".to_string()
    } else {
        host.to_ascii_lowercase()
    };
    debug_assert!(!host.contains('/'), "the host carries no path");
    (host, port)
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
    fn the_server_bind_check_names_the_flag_that_actually_binds() {
        // `[server].host` is gone — it never bound anything — so the check
        // states the effective answer: loopback unless `--host` says
        // otherwise. No config key can expose the server, so nothing to warn.
        let checks = run_checks(&cfg(), Path::new("/x"));
        let check = checks
            .iter()
            .find(|c| c.name == "server.bind")
            .expect("server.bind is always checked");
        assert_eq!(check.severity, Severity::Ok);
        assert!(
            check.detail.contains("--host"),
            "names the thing that binds: {}",
            check.detail
        );
        assert!(
            check.detail.contains(nanna_config::LOOPBACK_HOST),
            "and its default: {}",
            check.detail
        );
    }

    #[test]
    fn the_server_bind_check_does_not_trust_the_inert_enabled_flag() {
        // `nanna server` does not read `[server].enabled`; the old check
        // reported "HTTP server disabled" from it, which is a false claim
        // about a server that starts regardless.
        let mut config = cfg();
        config.server.enabled = false;
        let checks = run_checks(&config, Path::new("/x"));
        let check = checks
            .iter()
            .find(|c| c.name == "server.bind")
            .expect("server.bind is always checked");
        assert!(
            !check.detail.contains("disabled"),
            "an inert flag must not be reported as a fact: {}",
            check.detail
        );
        assert!(check.detail.contains("--host"));
    }

    #[test]
    fn ollama_endpoints_fold_loopback_spellings_and_the_default_port() {
        let local = ollama_endpoint("http://localhost:11434");
        assert_eq!(ollama_endpoint("http://127.0.0.1:11434/"), local);
        assert_eq!(ollama_endpoint("http://[::1]:11434"), local);
        assert_eq!(
            ollama_endpoint("http://localhost"),
            local,
            "a missing port is Ollama's default"
        );
        assert_ne!(ollama_endpoint("http://gpu-box:11434"), local);
        assert_ne!(
            ollama_endpoint("http://localhost:11435"),
            local,
            "a different port is a different server"
        );
        assert_eq!(
            ollama_endpoint("http://GPU-Box:11434"),
            ollama_endpoint("http://gpu-box:11434")
        );
    }

    fn ollama_everywhere() -> Config {
        let mut config = cfg();
        config.llm.provider = "ollama".to_string();
        config.llm.summarization_priority = vec!["ollama/qwen3:8b".to_string()];
        config
    }

    fn ollama_check(config: &Config) -> Check {
        run_checks(config, Path::new("/x"))
            .into_iter()
            .find(|c| c.name == "ollama.servers")
            .expect("ollama.servers is always checked")
    }

    /// The smoke-run finding: chat on a GPU box, summaries still on localhost.
    #[test]
    fn two_ollama_servers_are_flagged_with_the_fix() {
        let mut config = ollama_everywhere();
        config.memory.ollama_host = "http://gpu-box:11434".to_string();
        config.llm.ollama_url = None;
        let check = ollama_check(&config);
        assert_eq!(check.severity, Severity::Warn);
        assert!(check.detail.contains("gpu-box"), "{}", check.detail);
        assert!(
            check.detail.contains(OLLAMA_DEFAULT_URL),
            "an unset key shows the URL it actually falls back to: {}",
            check.detail
        );
        assert!(
            check
                .remedy
                .as_ref()
                .is_some_and(|r| r.contains("same URL")),
            "{check:?}"
        );
    }

    #[test]
    fn one_ollama_server_in_any_spelling_is_fine() {
        let mut config = ollama_everywhere();
        config.memory.ollama_host = "http://localhost:11434".to_string();
        config.llm.ollama_url = Some("http://127.0.0.1:11434/".to_string());
        let check = ollama_check(&config);
        assert_eq!(check.severity, Severity::Ok, "{}", check.detail);

        // Summarizing on Ollama while chat and embeddings are cloud is one
        // Ollama server, not two.
        let mut config = cfg();
        config.llm.provider = "anthropic".to_string();
        config.llm.model_priority = vec!["claude-opus-5".to_string()];
        config.memory.embedding_provider = "openai".to_string();
        config.memory.embedding_priority.clear();
        config.llm.summarization_priority = vec!["ollama/qwen3:8b".to_string()];
        config.llm.ollama_url = Some("http://gpu-box:11434".to_string());
        let check = ollama_check(&config);
        assert_eq!(check.severity, Severity::Ok, "{}", check.detail);
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
