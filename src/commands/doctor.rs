//! `nanna doctor` — offline configuration diagnosis.
//!
//! **Root cause, not availability.** The existing `status` command answers "is
//! a thing configured", which is the question that hid the failure class this
//! is written against: our own loopback stream faults were read as provider
//! 502s and answered with restart spirals, because every surface reported
//! *availability* and none reported *why*. Each check here therefore carries a
//! remedy, not just a verdict.
//!
//! **Offline by default.** Nothing in the default pass opens a socket. Network
//! probes are slow, they fail for reasons that have nothing to do with
//! configuration, and mixing them in means a machine with no internet reports
//! its config as broken. The default pass runs in milliseconds, is
//! deterministic, and is safe to run anywhere.
//!
//! **`--online` adds the one probe that needs no credential**: each Ollama
//! server in use is asked for its model list ([`run_online_checks`]). Provider
//! key checks are absent on purpose — they would read the keyring and send a
//! key off the machine.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use nanna_config::Config;
use nanna_config::credentials::{ClaudeCredentialManager, OAuthCredential};
use nanna_llm::{OllamaProbe, probe_ollama};

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
    checks.push(check_mcp_servers(config, command_resolves, |key| {
        nanna_config::credentials::SecureStore::new().get(key).ok()
    }));

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
            why,
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

/// The `[mcp]` servers the daemon will try to start, judged the way it will:
/// entries it skips are named, and a command that is not on `PATH` is a
/// failure — the daemon would log a spawn error at boot and the server's tools
/// would silently never appear.
///
/// `resolves` answers "can this command be spawned" and `secret` looks a
/// `secret_env` value up by store key, so the verdict logic is testable without
/// a real `PATH` or keyring. A server missing a secret is a failure for the same
/// reason as a missing command: the daemon will not start it. Offline: nothing
/// is spawned or contacted, and no secret value is printed.
fn check_mcp_servers(
    config: &Config,
    resolves: impl Fn(&str) -> bool,
    secret: impl Fn(&str) -> Option<String>,
) -> Check {
    const NAME: &str = "mcp.servers";
    if config.mcp.servers.is_empty() {
        return Check::ok(NAME, "no MCP servers configured");
    }
    let (start, skipped) = config.mcp.startable();
    let missing: Vec<String> = start
        .iter()
        .filter(|entry| !resolves(entry.command.trim()))
        .map(|entry| {
            let command = entry.command.trim();
            let why = if command.contains('/') || command.contains(std::path::MAIN_SEPARATOR) {
                "which is not an executable file"
            } else {
                "which is not on PATH"
            };
            format!("'{}' runs `{command}`, {why}", entry.name.trim())
        })
        .collect();
    debug_assert!(missing.len() <= start.len());
    let unsecreted: Vec<String> = start
        .iter()
        .filter_map(|entry| entry.resolve_secret_env(&secret).err())
        .collect();
    if missing.is_empty() && !unsecreted.is_empty() {
        let mut detail = unsecreted.join("; ");
        if !skipped.is_empty() {
            detail = format!("{detail}; also not started: {}", skipped.join("; "));
        }
        return Check::fail(
            NAME,
            detail,
            "store each named secret with `nanna mcp secret set <server> <VAR>`, or remove it from \
             that server's `secret_env`",
        );
    }
    if !missing.is_empty() {
        let mut detail = missing
            .iter()
            .chain(&unsecreted)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        if !skipped.is_empty() {
            detail = format!("{detail}; also not started: {}", skipped.join("; "));
        }
        return Check::fail(
            NAME,
            detail,
            "install the command, or give its absolute path as `command` in `[[mcp.servers]]` \
             (the daemon's PATH can differ from this shell's when the GUI starts it), or set \
             `enabled = false`",
        );
    }
    if !skipped.is_empty() {
        return Check::warn(
            NAME,
            format!(
                "{} will start; not started: {}",
                start.len(),
                skipped.join("; ")
            ),
            "fix or remove the named `[[mcp.servers]]` entries",
        );
    }
    let names: Vec<&str> = start.iter().map(|entry| entry.name.trim()).collect();
    Check::ok(
        NAME,
        format!(
            "{} will start at daemon boot: {}",
            start.len(),
            names.join(", ")
        ),
    )
}

/// Whether `command` names something spawnable: an existing file when it
/// contains a path separator, otherwise a file in some `PATH` directory.
fn command_resolves(command: &str) -> bool {
    let is_spawnable = |path: &Path| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            path.metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            path.is_file()
                || ["exe", "cmd", "bat"]
                    .iter()
                    .any(|ext| path.with_extension(ext).is_file())
        }
    };
    if command.contains('/') || command.contains(std::path::MAIN_SEPARATOR) {
        return is_spawnable(Path::new(command));
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| is_spawnable(&dir.join(command)))
    })
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
/// Split an embedding spec the way the daemon's `split_embedding_spec` does.
/// `None` means malformed — and a malformed entry is skipped, not repaired.
fn split_embedding_spec(spec: &str) -> Option<(String, String)> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    match spec.split_once('/') {
        Some((provider, model)) => {
            let (provider, model) = (provider.trim(), model.trim());
            if provider.is_empty() || model.is_empty() {
                return None;
            }
            Some((provider.to_ascii_lowercase(), model.to_string()))
        }
        // A bare name means the keyless local provider.
        None => Some(("ollama".to_string(), spec.to_string())),
    }
}

/// Why one embedding spec cannot become a live client.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SpecFault {
    /// Not `provider/model`, or a half of it is empty.
    Malformed,
    /// A provider the daemon has no arm for.
    UnknownProvider(String),
    /// A real provider whose credential is absent.
    MissingCredential(String),
}

/// Reproduce the daemon's `embedding_provider_for` decision for one spec.
///
/// This mirrors `DaemonServer::embedding_provider_for` rather than calling it —
/// the same tradeoff `check_clustering` already makes, and for the same reason:
/// the daemon builds that decision from live state a diagnostic cannot
/// construct. The coupling is real, so the rules are written out plainly and
/// `the_resolution_rules_match_the_daemons` pins them.
fn embedding_spec_fault(config: &Config, spec: &str) -> Option<SpecFault> {
    let Some((provider, _model)) = split_embedding_spec(spec) else {
        return Some(SpecFault::Malformed);
    };
    let configured = |value: &Option<String>, env: &str| {
        value.as_deref().is_some_and(|k| !k.trim().is_empty())
            || std::env::var(env).is_ok_and(|k| !k.trim().is_empty())
    };
    match provider.as_str() {
        // Keyless and local: it always resolves. Whether it ANSWERS is
        // `ollama.online`'s question, not this one.
        "ollama" => None,
        "openai" => (!configured(&config.llm.openai_api_key, "OPENAI_API_KEY"))
            .then(|| SpecFault::MissingCredential("OpenAI".to_string())),
        "openrouter" => (!configured(&config.llm.openrouter_api_key, "OPENROUTER_API_KEY"))
            .then(|| SpecFault::MissingCredential("OpenRouter".to_string())),
        other => Some(SpecFault::UnknownProvider(other.to_string())),
    }
}

/// The specs the daemon will actually try, in its order: `embedding_priority`
/// when set, otherwise the single `provider/model` pair.
fn embedding_specs(config: &Config) -> Vec<String> {
    if config.memory.embedding_priority.is_empty() {
        vec![format!(
            "{}/{}",
            config.memory.embedding_provider.trim(),
            config.memory.embedding_model.trim()
        )]
    } else {
        config.memory.embedding_priority.clone()
    }
}

/// Is memory actually going to be searchable?
///
/// The old check asked only whether a provider was *named*, and answered `ok`
/// for the shipped defaults — `openai` / `text-embedding-3-small` with an empty
/// priority list. On a machine with no `OpenAI` key the daemon then says, at boot
/// and never again:
///
/// > Embedding provider 'openai/text-embedding-3-small' skipped: no OpenAI API key
/// > No embedding provider available — memory runs WITHOUT vectors: writes
/// > persist and queue for backfill, recall is unavailable
///
/// So the doctor was reporting `ok` for a configuration its own daemon had
/// already announced as broken (observed live 2026-09-14). Naming a provider and
/// being able to reach one are different questions, and only the second one
/// matters.
fn check_embeddings(config: &Config) -> Check {
    if !config.memory.enabled {
        return Check::ok("memory.embeddings", "memory disabled");
    }
    let provider = config.memory.embedding_provider.trim();
    if config.memory.embedding_priority.is_empty()
        && (provider.is_empty() || provider.eq_ignore_ascii_case("disabled"))
    {
        return Check::warn(
            "memory.embeddings",
            "memory is enabled but no embedding provider is set — memories will be stored and \
             will not be searchable",
            "set `[memory].embedding_provider`, or disable memory if that is intended",
        );
    }

    let specs = embedding_specs(config);
    let faults: Vec<(String, SpecFault)> = specs
        .iter()
        .filter_map(|spec| embedding_spec_fault(config, spec).map(|f| (spec.clone(), f)))
        .collect();

    if faults.len() < specs.len() {
        // At least one entry resolves, so the router has a primary.
        let live = specs.len() - faults.len();
        return Check::ok(
            "memory.embeddings",
            format!("{live} of {} configured embedders resolve", specs.len()),
        );
    }

    // Nothing resolves. A typo can never resolve; a missing key resolves the
    // moment it is supplied and the queued backfill then drains — so the first
    // is a FAIL and the second a WARN, and the detail says which entry is which.
    let described: Vec<String> = faults
        .iter()
        .map(|(spec, fault)| match fault {
            SpecFault::Malformed => format!("`{spec}` is not `provider/model`"),
            SpecFault::UnknownProvider(p) => format!("`{spec}` names unknown provider `{p}`"),
            SpecFault::MissingCredential(p) => format!("`{spec}` has no {p} API key"),
        })
        .collect();
    let detail = format!(
        "no configured embedder resolves ({}) — memory will be written WITHOUT vectors and \
         queued for backfill; recall is unavailable until one does",
        described.join("; ")
    );

    let only_credentials = faults
        .iter()
        .all(|(_, f)| matches!(f, SpecFault::MissingCredential(_)));
    if only_credentials {
        Check::warn(
            "memory.embeddings",
            detail,
            "set the provider's API key, or point `[memory].embedding_priority` at a keyless \
             local embedder such as `ollama/nomic-embed-text`",
        )
    } else {
        Check::fail(
            "memory.embeddings",
            detail,
            "fix the spelling — an entry that is not `provider/model` for a known provider can \
             never resolve, no credential will help",
        )
    }
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
        .any(|m| is_ollama_summary_spec(m));
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

/// Does a `summarization_priority` entry go to Ollama? The summarizer's own
/// rule, which is looser than the chat router's: an `ollama/` prefix, or **no
/// provider prefix at all** — `qwen3` with no tag is Ollama there
/// (`nanna_agent::context`, `create_client_for_model`).
fn is_ollama_summary_spec(model: &str) -> bool {
    let model = model.trim();
    match model.split_once('/') {
        Some((provider, _)) => provider.eq_ignore_ascii_case("ollama"),
        None => !model.is_empty(),
    }
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

/// How long one Ollama probe may take, connect and answer together. A local
/// server answers `/api/tags` in milliseconds and a LAN one in tens; 3 s is
/// past both with room, and a dead host still cannot stall the report.
const ONLINE_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Most missing model names one check lists before counting the rest.
const MISSING_MODELS_SHOWN_MAX: usize = 8;

/// One Ollama server the configuration points at, and the models configured
/// against it, as the tags Ollama lists them under (`name:tag`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct OllamaServer {
    url: String,
    models: Vec<String>,
}

/// The network leg of `doctor`: probe every Ollama server the configuration
/// uses, and check that each has the models configured against it.
///
/// Ollama is the only thing probed over the *network*, by design: it is the one
/// dependency that answers without a credential. Probing a provider key would
/// send that key off the machine, which a diagnostic must not do.
///
/// The Anthropic credential check reads the keyring — locally, and only for the
/// credential's metadata (does it exist, has it expired, can it be refreshed).
/// The token is never printed, logged, or sent anywhere. That read is why it
/// lives behind `--online` rather than in the offline pass, whose whole promise
/// is that it touches nothing but the config file.
pub async fn run_online_checks(config: &Config) -> Vec<Check> {
    let servers = ollama_servers_in_use(config);
    let mut checks = Vec::with_capacity(servers.len() * 2 + 2);
    checks.push(check_anthropic_credential(config));
    if servers.is_empty() {
        checks.push(Check::ok(
            "ollama.online",
            "no Ollama model is configured; there was nothing to probe",
        ));
    }
    for server in &servers {
        let probe = probe_ollama(&server.url, ONLINE_PROBE_TIMEOUT).await;
        checks.extend(judge_ollama_server(server, &probe));
    }
    debug_assert!(
        !checks.is_empty(),
        "the online leg always reports something"
    );
    debug_assert!(
        checks
            .iter()
            .all(|c| c.severity == Severity::Ok || c.remedy.is_some()),
        "every non-ok check must carry a remedy"
    );
    checks
}

/// Does the configuration route anything at Anthropic? Chat model, the priority
/// lists, and the summarizer all count — any one of them makes an Anthropic
/// credential load-bearing.
fn anthropic_is_in_use(config: &Config) -> bool {
    let names_anthropic = |model: &str| {
        let model = model.trim().to_ascii_lowercase();
        model.starts_with("claude") || model.starts_with("anthropic")
    };
    config.llm.provider.eq_ignore_ascii_case("anthropic")
        || names_anthropic(&config.llm.model)
        || config.llm.model_priority.iter().any(|m| names_anthropic(m))
        || config
            .llm
            .summarization_priority
            .iter()
            .any(|m| names_anthropic(m))
}

/// Verdict on a stored Anthropic OAuth credential.
///
/// Pure, so every branch is testable without a keyring: the caller does the one
/// read and hands the result here.
///
/// Why this check exists. When the credential expires, what the user actually
/// sees is the model router's message — observed live 2026-09-14:
/// `No provider for model: claude-sonnet-5 (detected: Anthropic, available:
/// [Ollama])`. That sentence names the model and the provider list and says
/// nothing about a credential, so it reads as "your model name is wrong" when
/// the truth is "your token expired four hours ago". The daemon logs the real
/// cause once, at WARN, at boot, and never again. This check is the place those
/// two facts get connected.
fn judge_anthropic_credential(
    anthropic_in_use: bool,
    credential: Option<&OAuthCredential>,
    api_key_configured: bool,
) -> Check {
    const NAME: &str = "auth.anthropic";
    const REMINT: &str = "re-mint it with `claude setup-token` and store it with \
                          `nanna auth login`, or set an API key in `[llm].api_key`";

    let Some(credential) = credential else {
        if !anthropic_in_use {
            return Check::ok(NAME, "no Anthropic model is configured");
        }
        if api_key_configured {
            return Check::ok(NAME, "no stored OAuth credential; an API key is configured");
        }
        return Check::fail(
            NAME,
            "an Anthropic model is configured and there is neither a stored OAuth \
             credential nor an API key",
            REMINT,
        );
    };

    // Never interpolate the token itself — not even a prefix. A diagnostic that
    // prints half a credential is a credential in the terminal scrollback.
    let expiry = credential.seconds_until_expiry();

    if !credential.is_expired() {
        let detail = expiry.map_or_else(
            || "stored OAuth credential carries no expiry".to_string(),
            |secs| format!("stored OAuth credential valid for {}h", secs / 3600),
        );
        return Check::ok(NAME, detail);
    }

    let ago = expiry.map_or_else(
        || "expired".to_string(),
        |secs| format!("expired {}h ago", (-secs) / 3600),
    );

    if !credential.can_refresh() {
        return Check::fail(
            NAME,
            format!("stored OAuth credential {ago} and carries no refresh token"),
            REMINT,
        );
    }

    // It can refresh on paper. Say what actually happens, because on this tree
    // it does not: the refresh POST carries `grant_type` and `refresh_token`
    // and no `client_id`, and the endpoint answers 400 `invalid_request_error:
    // Invalid request format` (observed live 2026-09-14). Until that is fixed a
    // refreshable credential is not a working one, and a `warn` that implied
    // otherwise would be the same lie the router's message tells.
    Check::fail(
        NAME,
        format!(
            "stored OAuth credential {ago}; a refresh token is present, but refresh is \
             known to fail on this build with 400 `Invalid request format`"
        ),
        REMINT,
    )
}

/// Read the stored Anthropic credential and judge it. The keyring read happens
/// here and nowhere else in this module.
fn check_anthropic_credential(config: &Config) -> Check {
    let anthropic_in_use = anthropic_is_in_use(config);

    // Short-circuit BEFORE the read. A configuration that never routes to
    // Anthropic has no business unlocking the user's keyring, and on a desktop
    // running libsecret that read can raise an unlock prompt — which in a test
    // or a headless run is a hang, not a diagnostic.
    if !anthropic_in_use {
        return judge_anthropic_credential(false, None, false);
    }

    let manager = ClaudeCredentialManager::new();
    let loaded = manager.load().ok();
    let api_key_configured = config
        .llm
        .api_key
        .as_deref()
        .is_some_and(|k| !k.trim().is_empty());

    judge_anthropic_credential(
        anthropic_in_use,
        loaded.as_ref().map(|l| &l.credential),
        api_key_configured,
    )
}

/// Every Ollama server in use, each with the models expected there. Chat and
/// embeddings go to `[memory].ollama_host`, summarization to
/// `[llm].ollama_url`; when both name one server it is probed once.
fn ollama_servers_in_use(config: &Config) -> Vec<OllamaServer> {
    let mut chat_models: Vec<&str> = config
        .llm
        .model_priority
        .iter()
        .chain(&config.memory.embedding_priority)
        .map(String::as_str)
        .filter(|m| is_ollama_spec(m))
        .collect();
    if config.llm.provider.eq_ignore_ascii_case("ollama") {
        chat_models.push(&config.llm.model);
    }
    if config
        .memory
        .embedding_provider
        .eq_ignore_ascii_case("ollama")
    {
        chat_models.push(&config.memory.embedding_model);
    }
    let summary_models: Vec<&str> = config
        .llm
        .summarization_priority
        .iter()
        .map(String::as_str)
        .filter(|m| is_ollama_summary_spec(m))
        .collect();
    let summary_url = config
        .llm
        .ollama_url
        .as_deref()
        .unwrap_or(OLLAMA_DEFAULT_URL);
    let mut servers: Vec<OllamaServer> = Vec::with_capacity(2);
    for (url, models) in [
        (config.memory.ollama_host.as_str(), chat_models),
        (summary_url, summary_models),
    ] {
        if models.is_empty() {
            continue;
        }
        let tags = models.iter().map(|m| ollama_tag(m));
        let same = servers
            .iter()
            .position(|s| ollama_endpoint(&s.url) == ollama_endpoint(url));
        match same {
            Some(index) => servers[index].models.extend(tags),
            None => servers.push(OllamaServer {
                url: url.trim().to_string(),
                models: tags.collect(),
            }),
        }
    }
    for server in &mut servers {
        server.models.sort_unstable();
        server.models.dedup();
    }
    debug_assert!(servers.len() <= 2, "two keys name at most two servers");
    debug_assert!(
        servers.iter().all(|s| !s.models.is_empty()),
        "only a server with models configured against it is probed"
    );
    servers
}

/// A configured model as the tag Ollama lists it under: no `ollama/` prefix,
/// and `:latest` when no tag is given — Ollama's own default.
fn ollama_tag(model: &str) -> String {
    let model = model.trim();
    let model = match model.split_once('/') {
        Some((prefix, rest)) if prefix.eq_ignore_ascii_case("ollama") => rest,
        _ => model,
    };
    let name = model.rsplit('/').next().unwrap_or(model);
    if name.contains(':') {
        model.to_string()
    } else {
        format!("{model}:latest")
    }
}

/// What one probe says about one server: whether it answered, and whether it
/// has every model configured against it.
fn judge_ollama_server(server: &OllamaServer, probe: &OllamaProbe) -> Vec<Check> {
    debug_assert!(
        !server.models.is_empty(),
        "only servers with work are probed"
    );
    let installed = match probe {
        OllamaProbe::Unreachable { reason } => {
            return vec![Check::fail(
                "ollama.online",
                format!("{}: {reason}", server.url),
                "start Ollama there (`ollama serve`), or point the config at the server that runs it",
            )];
        }
        OllamaProbe::Reachable { .. } => probe.model_names(),
    };
    let answered = Check::ok(
        "ollama.online",
        format!(
            "{} answered; {} models installed",
            server.url,
            installed.len()
        ),
    );
    let missing: Vec<&str> = server
        .models
        .iter()
        .filter(|want| !installed.iter().any(|have| have.eq_ignore_ascii_case(want)))
        .map(String::as_str)
        .collect();
    let models = if missing.is_empty() {
        Check::ok(
            "ollama.models",
            format!(
                "all {} configured models are installed on {}",
                server.models.len(),
                server.url
            ),
        )
    } else {
        let shown = missing[..missing.len().min(MISSING_MODELS_SHOWN_MAX)].join(", ");
        let more = missing.len().saturating_sub(MISSING_MODELS_SHOWN_MAX);
        let more = if more == 0 {
            String::new()
        } else {
            format!(" and {more} more")
        };
        Check::fail(
            "ollama.models",
            format!(
                "configured but not installed on {}: {shown}{more}",
                server.url
            ),
            "`ollama pull <name>` each one on that host, or drop it from the config",
        )
    };
    let checks = vec![answered, models];
    debug_assert_eq!(checks.len(), 2, "a server that answered gets both verdicts");
    checks
}

/// `nanna doctor [--online]`: every offline check, then the Ollama probe when
/// `online`, then the printed report. Returns the worst severity seen.
pub async fn run(config: &Config, config_path: &Path, online: bool) -> Severity {
    let mut checks = run_checks(config, config_path);
    let offline = checks.len();
    if online {
        checks.extend(run_online_checks(config).await);
    }
    debug_assert!(offline > 0, "the offline pass always runs");
    debug_assert!(
        online || checks.len() == offline,
        "no network check without --online"
    );
    report(&checks, online)
}

/// Print the report. Returns the worst severity seen, so the caller can choose
/// an exit code. `online` says whether [`run_online_checks`] contributed.
pub fn report(checks: &[Check], online: bool) -> Severity {
    let worst = checks
        .iter()
        .map(|c| c.severity)
        .max()
        .unwrap_or(Severity::Ok);

    if online {
        println!("Nanna doctor — configuration checks and an Ollama probe");
    } else {
        println!("Nanna doctor — offline configuration checks");
    }
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
    if online {
        println!(
            "\nOllama was probed over the network. The keyring was read locally for the \
             Anthropic credential's expiry only — no provider key was tested, printed or sent."
        );
    } else {
        println!(
            "\nThis pass is offline: no provider, network or keyring probe ran. \
             Availability is a separate question from configuration; \
             `nanna doctor --online` also probes Ollama."
        );
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::default()
    }

    fn no_secrets(_: &str) -> Option<String> {
        None
    }

    fn with_mcp(servers: &[(&str, &str)]) -> Config {
        let mut config = cfg();
        config.mcp.servers = servers
            .iter()
            .map(|(name, command)| nanna_config::McpServerEntry {
                name: (*name).to_string(),
                command: (*command).to_string(),
                args: Vec::new(),
                enabled: true,
                secret_env: Vec::new(),
            })
            .collect();
        config
    }

    #[test]
    fn mcp_servers_are_judged_the_way_the_daemon_will_start_them() {
        let installed = |command: &str| command == "npx";

        let none = check_mcp_servers(&cfg(), installed, no_secrets);
        assert_eq!(none.severity, Severity::Ok);

        let fine = check_mcp_servers(&with_mcp(&[("files", "npx")]), installed, no_secrets);
        assert_eq!(fine.severity, Severity::Ok, "{fine:?}");
        assert!(
            fine.detail.contains("1 will start at daemon boot: files"),
            "{fine:?}"
        );

        let missing = check_mcp_servers(
            &with_mcp(&[("files", "npx"), ("git", "uvx")]),
            installed,
            no_secrets,
        );
        assert_eq!(missing.severity, Severity::Fail, "{missing:?}");
        assert!(
            missing
                .detail
                .contains("'git' runs `uvx`, which is not on PATH"),
            "{missing:?}"
        );
        assert!(!missing.detail.contains("'files'"), "{missing:?}");

        let both = check_mcp_servers(
            &with_mcp(&[("files", "npx"), ("git", "/opt/uvx"), ("files", "npx")]),
            installed,
            no_secrets,
        );
        assert_eq!(both.severity, Severity::Fail, "{both:?}");
        assert!(
            both.detail
                .contains("`/opt/uvx`, which is not an executable file"),
            "{both:?}"
        );
        assert!(
            both.detail
                .contains("also not started: MCP server name 'files' is used twice"),
            "{both:?}"
        );

        let duplicate = check_mcp_servers(
            &with_mcp(&[("files", "npx"), ("files", "npx")]),
            installed,
            no_secrets,
        );
        assert_eq!(duplicate.severity, Severity::Warn, "{duplicate:?}");
        assert!(duplicate.detail.contains("is used twice"), "{duplicate:?}");
    }

    #[test]
    fn an_mcp_server_missing_its_secret_fails_without_printing_any_value() {
        let installed = |command: &str| command == "npx";
        let mut config = with_mcp(&[("github", "npx")]);
        config.mcp.servers[0].secret_env = vec!["GITHUB_TOKEN".into()];

        let unset = check_mcp_servers(&config, installed, no_secrets);
        assert_eq!(unset.severity, Severity::Fail, "{unset:?}");
        assert!(
            unset
                .detail
                .contains("run `nanna mcp secret set github GITHUB_TOKEN`"),
            "{unset:?}"
        );

        let stored =
            |key: &str| (key == "mcp.github.GITHUB_TOKEN").then(|| "ghp_value".to_string());
        let set = check_mcp_servers(&config, installed, stored);
        assert_eq!(set.severity, Severity::Ok, "{set:?}");
        assert!(!set.detail.contains("ghp_value"), "{set:?}");
    }

    #[test]
    fn command_resolution_reads_path_and_absolute_paths() {
        #[cfg(unix)]
        assert!(command_resolves("sh"), "sh is on every unix PATH");
        assert!(!command_resolves("definitely-not-a-command-nanna-7f3"));
        assert!(!command_resolves("/nonexistent/mcp-server"));
        #[cfg(unix)]
        assert!(command_resolves("/bin/sh"));
    }

    /// Build a credential `hours` from expiry (negative = already expired).
    fn credential(hours: i64, refreshable: bool) -> OAuthCredential {
        OAuthCredential {
            access_token: "not-a-real-token".to_string(),
            refresh_token: refreshable.then(|| "not-a-real-refresh".to_string()),
            expires_at: Some(chrono::Utc::now().timestamp_millis() + hours * 3600 * 1000),
            subscription_type: None,
            account_id: None,
            organization_id: None,
        }
    }

    #[test]
    fn a_live_credential_is_ok_and_reports_its_remaining_hours() {
        let check = judge_anthropic_credential(true, Some(&credential(10, true)), false);
        assert_eq!(check.severity, Severity::Ok);
        assert!(
            check.detail.contains("9h") || check.detail.contains("10h"),
            "{check:?}"
        );
    }

    #[test]
    fn an_expired_credential_without_a_refresh_token_is_terminal() {
        let check = judge_anthropic_credential(true, Some(&credential(-4, false)), false);
        assert_eq!(check.severity, Severity::Fail);
        assert!(check.detail.contains("no refresh token"), "{check:?}");
        assert!(check.remedy.is_some());
    }

    /// The live finding this check was written for: refreshable on paper, and
    /// refresh does not work on this build. A `warn` here would repeat the
    /// router's own misleading message.
    #[test]
    fn an_expired_but_refreshable_credential_still_fails_while_refresh_is_broken() {
        let check = judge_anthropic_credential(true, Some(&credential(-4, true)), false);
        assert_eq!(check.severity, Severity::Fail);
        assert!(check.detail.contains("Invalid request format"), "{check:?}");
    }

    #[test]
    fn no_credential_and_no_key_fails_only_when_anthropic_is_actually_used() {
        assert_eq!(
            judge_anthropic_credential(true, None, false).severity,
            Severity::Fail
        );
        assert_eq!(
            judge_anthropic_credential(false, None, false).severity,
            Severity::Ok
        );
        // An API key is the other way to authenticate; OAuth's absence is then fine.
        assert_eq!(
            judge_anthropic_credential(true, None, true).severity,
            Severity::Ok
        );
    }

    #[test]
    fn no_verdict_ever_contains_token_material() {
        // Negative space: the tokens above are distinctive strings, so a check
        // that leaked any part of them would be caught here rather than in a
        // user's scrollback.
        for check in [
            judge_anthropic_credential(true, Some(&credential(10, true)), false),
            judge_anthropic_credential(true, Some(&credential(-4, true)), false),
            judge_anthropic_credential(true, Some(&credential(-4, false)), false),
        ] {
            let printed = format!("{} {:?}", check.detail, check.remedy);
            assert!(!printed.contains("not-a-real-token"), "{printed}");
            assert!(!printed.contains("not-a-real-refresh"), "{printed}");
        }
    }

    #[test]
    fn anthropic_is_detected_from_any_of_the_places_a_model_can_be_named() {
        let mut config = cfg();
        config.llm.provider = "ollama".to_string();
        config.llm.model = "qwen3.5:9b".to_string();
        config.llm.model_priority.clear();
        config.llm.summarization_priority.clear();
        assert!(!anthropic_is_in_use(&config), "nothing names Anthropic");

        config.llm.summarization_priority = vec!["claude-haiku-4-5".to_string()];
        assert!(
            anthropic_is_in_use(&config),
            "the summarizer alone makes the credential load-bearing"
        );
    }

    /// A config whose embedders are all keyless and local, so tests never
    /// depend on whether the developer happens to export an `OpenAI` key.
    fn local_embedding_config() -> Config {
        let mut config = cfg();
        config.memory.embedding_priority = vec!["ollama/nomic-embed-text".to_string()];
        config
    }

    #[test]
    fn a_keyless_local_embedder_resolves() {
        let check = check_embeddings(&local_embedding_config());
        assert_eq!(check.severity, Severity::Ok, "{check:?}");
        assert!(check.detail.contains("1 of 1"), "{check:?}");
    }

    #[test]
    fn a_typo_can_never_resolve_and_is_a_failure() {
        let mut config = cfg();
        config.memory.embedding_priority = vec!["openrouter/".to_string()];
        let check = check_embeddings(&config);
        assert_eq!(check.severity, Severity::Fail, "{check:?}");
        assert!(check.detail.contains("not `provider/model`"), "{check:?}");

        config.memory.embedding_priority = vec!["opeanai/text-embedding-3-small".to_string()];
        let check = check_embeddings(&config);
        assert_eq!(check.severity, Severity::Fail, "{check:?}");
        assert!(check.detail.contains("unknown provider"), "{check:?}");
    }

    /// One working entry is enough — the router only needs a primary.
    #[test]
    fn one_resolvable_entry_rescues_a_list_with_a_broken_one() {
        let mut config = cfg();
        config.memory.embedding_priority = vec![
            "openai/text-embedding-3-small".to_string(),
            "ollama/nomic-embed-text".to_string(),
        ];
        let check = check_embeddings(&config);
        assert_eq!(check.severity, Severity::Ok, "{check:?}");
    }

    /// The daemon's rules, pinned. `check_embeddings` mirrors
    /// `DaemonServer::embedding_provider_for` rather than calling it, so the
    /// mirror is what needs a test.
    #[test]
    fn the_resolution_rules_match_the_daemons() {
        let config = cfg();
        // Keyless and local: always resolvable, regardless of any key.
        assert_eq!(
            embedding_spec_fault(&config, "ollama/nomic-embed-text"),
            None
        );
        // A bare name means ollama, so it resolves too.
        assert_eq!(
            embedding_spec_fault(&config, "nomic-embed-text:latest"),
            None
        );
        // Malformed halves are rejected, never repaired.
        assert_eq!(
            embedding_spec_fault(&config, "openrouter/"),
            Some(SpecFault::Malformed)
        );
        assert_eq!(
            embedding_spec_fault(&config, "  "),
            Some(SpecFault::Malformed)
        );
        // An unknown provider is a typo, not a credential problem.
        assert!(matches!(
            embedding_spec_fault(&config, "cohere/embed-v3"),
            Some(SpecFault::UnknownProvider(_))
        ));
    }

    /// The specs the daemon will actually try: the priority list wins, and the
    /// legacy pair is used only when it is empty.
    #[test]
    fn the_priority_list_replaces_the_single_pair_rather_than_extending_it() {
        let mut config = cfg();
        config.memory.embedding_provider = "openai".to_string();
        config.memory.embedding_model = "text-embedding-3-small".to_string();
        assert_eq!(
            embedding_specs(&config),
            vec!["openai/text-embedding-3-small".to_string()]
        );

        config.memory.embedding_priority = vec!["ollama/nomic-embed-text".to_string()];
        assert_eq!(
            embedding_specs(&config),
            vec!["ollama/nomic-embed-text".to_string()],
            "the single pair must not be appended to the list"
        );
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

    /// Chat, embeddings and summarization all on the default local Ollama.
    fn probe_config() -> Config {
        let mut config = cfg();
        config.llm.provider = "ollama".to_string();
        config.llm.model = "qwen3:8b".to_string();
        config.llm.model_priority = Vec::new();
        config.llm.summarization_priority = vec!["ollama/qwen3:4b".to_string()];
        config.llm.ollama_url = None;
        config.memory.embedding_provider = "ollama".to_string();
        config.memory.embedding_model = "nomic-embed-text".to_string();
        config.memory.embedding_priority = Vec::new();
        config
    }

    #[test]
    fn one_shared_server_is_probed_once_for_every_model() {
        let servers = ollama_servers_in_use(&probe_config());
        assert_eq!(servers.len(), 1, "{servers:?}");
        assert_eq!(
            servers[0].models,
            vec!["nomic-embed-text:latest", "qwen3:4b", "qwen3:8b"]
        );
    }

    #[test]
    fn a_split_config_probes_each_server_for_its_own_models() {
        let mut config = probe_config();
        config.llm.ollama_url = Some("http://gpu-box:11434".to_string());
        let servers = ollama_servers_in_use(&config);
        assert_eq!(servers.len(), 2, "{servers:?}");
        assert_eq!(servers[1].url, "http://gpu-box:11434");
        assert_eq!(servers[1].models, vec!["qwen3:4b"]);
    }

    #[test]
    fn nothing_on_ollama_means_nothing_to_probe() {
        let mut config = probe_config();
        config.llm.provider = "anthropic".to_string();
        config.llm.summarization_priority = Vec::new();
        config.memory.embedding_provider = "openai".to_string();
        assert_eq!(ollama_servers_in_use(&config), Vec::new());
    }

    #[test]
    fn model_tags_are_normalized_the_way_ollama_lists_them() {
        assert_eq!(ollama_tag("ollama/qwen3:8b"), "qwen3:8b");
        assert_eq!(ollama_tag(" nomic-embed-text "), "nomic-embed-text:latest");
        assert_eq!(ollama_tag("hf.co/org/model"), "hf.co/org/model:latest");
    }

    fn local_server(models: &[&str]) -> OllamaServer {
        OllamaServer {
            url: "http://localhost:11434".to_string(),
            models: models.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn an_unreachable_server_fails_with_a_remedy() {
        let probe = OllamaProbe::Unreachable {
            reason: "timed out".to_string(),
        };
        let checks = judge_ollama_server(&local_server(&["qwen3:8b"]), &probe);
        assert_eq!(checks.len(), 1, "no model verdict without an answer");
        assert_eq!(checks[0].severity, Severity::Fail);
        assert!(
            checks[0].detail.contains("timed out"),
            "{}",
            checks[0].detail
        );
        assert!(checks[0].remedy.is_some());
    }

    fn installed(names: &[&str]) -> OllamaProbe {
        OllamaProbe::Reachable {
            models: names
                .iter()
                .map(|n| nanna_llm::OllamaModel {
                    name: (*n).to_string(),
                    size_bytes: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn a_missing_model_fails_and_present_ones_pass() {
        let server = local_server(&["nomic-embed-text:latest", "qwen3:8b"]);
        let all = installed(&["Qwen3:8b", "nomic-embed-text:latest"]);
        let checks = judge_ollama_server(&server, &all);
        assert!(
            checks.iter().all(|c| c.severity == Severity::Ok),
            "{checks:?}"
        );
        let one = installed(&["qwen3:8b"]);
        let checks = judge_ollama_server(&server, &one);
        let models = checks
            .iter()
            .find(|c| c.name == "ollama.models")
            .expect("a models verdict");
        assert_eq!(models.severity, Severity::Fail);
        assert!(
            models.detail.contains("nomic-embed-text:latest")
                && !models.detail.contains("qwen3:8b"),
            "only the missing model is named: {}",
            models.detail
        );
    }

    /// The whole online leg against a port nothing listens on: one FAIL, and it
    /// returns rather than hangs.
    #[tokio::test]
    async fn the_online_leg_reports_a_dead_server() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr").port()
        };
        let mut config = probe_config();
        config.memory.ollama_host = format!("http://127.0.0.1:{port}");
        config.llm.ollama_url = Some(format!("http://localhost:{port}"));
        let checks = run_online_checks(&config).await;
        let probed: Vec<_> = checks
            .iter()
            .filter(|c| c.name == "ollama.online")
            .collect();
        assert_eq!(probed.len(), 1, "one server, probed once: {checks:?}");
        assert_eq!(probed[0].severity, Severity::Fail);
    }

    /// The summarizer sends any spec without a provider prefix to Ollama —
    /// an untagged `qwen3` included — so a split with one must still be
    /// flagged, and the server summaries really go to must be probed.
    #[test]
    fn an_untagged_summary_model_still_counts_as_ollama() {
        assert!(is_ollama_summary_spec("qwen3"));
        assert!(is_ollama_summary_spec("Ollama/qwen3:4b"));
        assert!(!is_ollama_summary_spec("anthropic/claude-haiku-4-5"));
        assert!(!is_ollama_summary_spec("  "));
        assert_eq!(ollama_tag("Ollama/qwen3:4b"), "qwen3:4b");
        let mut config = probe_config();
        config.llm.summarization_priority = vec!["qwen3".to_string()];
        config.llm.ollama_url = Some("http://gpu-box:11434".to_string());
        assert_eq!(ollama_check(&config).severity, Severity::Warn);
        let servers = ollama_servers_in_use(&config);
        assert_eq!(servers.len(), 2, "{servers:?}");
        assert_eq!(servers[1].models, vec!["qwen3:latest"]);
    }
}
