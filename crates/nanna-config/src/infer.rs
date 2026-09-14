//! Local (on-device) inference configuration and the boot-time decisions
//! derived from it.
//!
//! The runner itself lives in [`physics515/Mummu`](https://github.com/physics515/Mummu)
//! and stays there — this module is only the `[infer]` surface plus the
//! translation `Provider::Local` will sit behind. Keeping the translation in
//! `nanna-config` rather than behind `nanna-llm`'s optional `local-infer`
//! feature is deliberate: CI compiles default features only
//! (`cargo test --no-run --workspace`), so a feature-gated decision table
//! would never be built or tested.
//!
//! Two facts shape everything here:
//!
//! 1. **Mummu owns the VRAM policy.** `plan::pick_precision` walks
//!    `Precision::descending()` against a `DeviceBudget`, and
//!    `DeviceBudget::usable_bytes()` is what leaves the display its share.
//!    Nanna does not re-derive any of it; it only forwards an override when
//!    the operator set one.
//! 2. **That budget is unknowable on this host.** `DeviceBudget::from_adapter`
//!    returns `None` unless `GpuAdapter::vram_bytes` is populated, and that
//!    happens only through Mummu's Windows DXGI walk —
//!    `backend::vram_by_adapter_name()` returns an empty vec on Linux and
//!    macOS, marking Vulkan memory heaps a follow-up. So
//!    [`InferPrecision::Auto`] cannot resolve to f16 here, and
//!    [`resolve_precision`] says so out loud instead of silently handing back
//!    f32 and leaving someone to wonder why the fast path never engages.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ConfigError;

/// Which device the local runner should place the model on.
///
/// `Auto` is the honest default: Mummu owns the probe (`backend::gpu_device`
/// / `gpu_device_f16` / `cpu_device`) and Nanna has no business second-
/// guessing it. The explicit variants exist for the two cases where the
/// operator knows something the probe cannot: a machine whose GPU is claimed
/// by something else, and a bisect that needs the CPU path pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferDevice {
    /// Let Mummu's cached runtime probe choose (GPU when one is usable).
    #[default]
    Auto,
    /// Force the discrete-GPU path.
    Gpu,
    /// Force the `burn-flex` CPU path.
    Cpu,
}

/// Weight/KV-cache precision for the local runner, mirroring
/// `mummu::plan::Precision` plus an `Auto` that defers to
/// `mummu::plan::pick_precision`.
///
/// **`Auto` is not currently decidable on Linux**, and that is why this knob
/// is not cosmetic. `pick_precision` needs a `DeviceBudget`, which is built
/// from `GpuAdapter::vram_bytes`; that field is filled only by Mummu's
/// Windows DXGI walk — `backend::vram_by_adapter_name()` returns an empty
/// vec on Linux and macOS (its own comment marks Vulkan memory heaps a P6
/// follow-up), so `DeviceBudget::from_adapter` returns `None` and the planner
/// correctly refuses to guess a fit it cannot size. On this host, choosing
/// f16 therefore has to be *stated*. Revisit the default when Mummu can size
/// VRAM from Vulkan heaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferPrecision {
    /// Defer to `pick_precision` against the probed VRAM budget. Falls back
    /// to `F32` wherever that budget is unknown (all non-Windows hosts today).
    #[default]
    Auto,
    /// f16 weights and KV cache. Needs an adapter advertising `SHADER_F16`.
    F16,
    /// f32 weights and KV cache — always available, twice the VRAM.
    F32,
}

/// Local on-device inference (the Mummu runner).
///
/// This is the config surface only. It is deliberately inert until a runner
/// is actually linked in: `enabled` defaults to `false`, so a build without
/// the `local-infer` feature and a config that never mentions `[infer]`
/// behave exactly as before.
///
/// Model names are Mummu **catalog** names (`mummu::registry::catalog()`),
/// not `HuggingFace` repo ids — the catalog is what maps a short name to a
/// pinned repo + revision + architecture, and going through it is what keeps
/// a model reference reproducible.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct InferConfig {
    /// Master switch for the local runner. Default `false`: the runner is a
    /// large optional dependency and an unconfigured machine has no weights
    /// on disk, so defaulting it on would mean every daemon boot tries to
    /// fetch gigabytes.
    pub enabled: bool,

    /// Catalog name of the chat/completion model, e.g.
    /// `qwen2.5-1.5b-instruct-q4km`. Empty means "no local chat model" — the
    /// router keeps using its cloud tiers. There is no default here on
    /// purpose: which model fits is a property of the machine, and picking
    /// one for the operator would either not fit or not be the one they want.
    pub model: String,

    /// Catalog name of the sentence embedder backing the memory `embed_fn`.
    /// Defaults to the `MiniLM` entry because it is the one model whose cost is
    /// knowable in advance — CPU-only, ~91 MB, no VRAM to negotiate — which
    /// is exactly why the roadmap makes it the first local consumer.
    pub embedding_model: String,

    /// Where model weights are cached. `None` = Mummu's own default under the
    /// platform data dir.
    pub models_root: Option<PathBuf>,

    /// Device placement preference.
    pub device: InferDevice,

    /// Weight/KV-cache precision. See [`InferPrecision`] for why `Auto`
    /// cannot resolve to f16 on Linux today.
    pub precision: InferPrecision,

    /// Optional override, in bytes, for how much VRAM a plan may claim.
    ///
    /// `None` — the default — means Mummu's `DeviceBudget::usable_bytes()`
    /// decides, and that is the right answer: the display-share policy lives
    /// with the planner that has the adapter in hand. This exists only so a
    /// machine with a known extra consumer (the Tauri webview here costs
    /// several GB) can hand the planner a smaller number, and it is
    /// deliberately not a fraction — a fraction of an unknown total is not a
    /// quantity.
    pub vram_budget_bytes: Option<u64>,
}

impl Default for InferConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: String::new(),
            embedding_model: DEFAULT_LOCAL_EMBEDDING_MODEL.to_string(),
            models_root: None,
            device: InferDevice::default(),
            precision: InferPrecision::default(),
            vram_budget_bytes: None,
        }
    }
}

/// Mummu catalog name of the default local sentence embedder.
pub const DEFAULT_LOCAL_EMBEDDING_MODEL: &str = "all-minilm-l6-v2";

impl InferConfig {
    /// Whether a local chat model is configured *and* switched on.
    ///
    /// Both halves matter: `enabled` with an empty `model` is a valid state
    /// (local embeddings, cloud chat), so the router must not read `enabled`
    /// alone as "a local completion tier exists".
    #[must_use]
    pub fn has_local_chat_model(&self) -> bool {
        self.enabled && !self.model.trim().is_empty()
    }

    /// Whether local embeddings should back the memory `embed_fn`.
    #[must_use]
    pub fn has_local_embedding_model(&self) -> bool {
        self.enabled && !self.embedding_model.trim().is_empty()
    }

    /// Reject configurations that cannot be honored, so a bad `[infer]` block
    /// fails at load with a sentence instead of at first token with a panic.
    ///
    /// # Errors
    /// Returns [`ConfigError::MissingField`] when `enabled` is set but no
    /// model of either kind is named, and when `vram_budget_bytes` is present
    /// but zero.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !self.enabled {
            return Ok(());
        }
        if self.model.trim().is_empty() && self.embedding_model.trim().is_empty() {
            return Err(ConfigError::MissingField(
                "infer.model or infer.embedding_model (infer.enabled = true names neither)".into(),
            ));
        }
        if self.vram_budget_bytes == Some(0) {
            return Err(ConfigError::MissingField(
                "infer.vram_budget_bytes = 0 claims no VRAM at all; omit it to let the planner decide".into(),
            ));
        }
        Ok(())
    }
}

/// Why a precision ended up where it did — so a demotion is never silent.
///
/// The `Unsized` case is the one that matters operationally: it is not a
/// failure, it is the planner correctly refusing to guess a fit on a host that
/// cannot report VRAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecisionReason {
    /// The operator named it explicitly in `[infer].precision`.
    Stated,
    /// `pick_precision` chose it against a real VRAM budget.
    Planned,
    /// `Auto`, but no VRAM budget is available on this platform, so the
    /// always-available precision was taken.
    Unsized,
}

/// A resolved precision plus the reason, ready to be logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPrecision {
    pub precision: InferPrecision,
    pub reason: PrecisionReason,
}

impl ResolvedPrecision {
    /// A one-line announcement for the boot log.
    ///
    /// Every degradation in this codebase announces itself; a precision that
    /// quietly fell back to f32 would otherwise read as a Mummu performance
    /// regression rather than a missing platform capability.
    #[must_use]
    pub fn announcement(&self) -> String {
        match self.reason {
            PrecisionReason::Stated => {
                format!(
                    "local inference: precision {:?} (set in [infer])",
                    self.precision
                )
            }
            PrecisionReason::Planned => format!(
                "local inference: precision {:?} (planned against the probed VRAM budget)",
                self.precision
            ),
            PrecisionReason::Unsized => format!(
                "local inference: precision {:?} — [infer].precision is \"auto\" but this platform \
                 reports no VRAM size (Mummu fills GpuAdapter::vram_bytes only through its Windows \
                 DXGI walk), so the planner has nothing to size a fit against. Set \
                 [infer].precision = \"f16\" to take the half-precision path deliberately.",
                self.precision
            ),
        }
    }
}

/// Whether this platform can report a VRAM budget for the planner.
///
/// Kept as one named predicate rather than scattered `cfg!` checks so the
/// reason is stated once. Mirrors `mummu::backend::vram_by_adapter_name`,
/// which returns an empty vec off Windows.
#[must_use]
pub const fn platform_reports_vram() -> bool {
    cfg!(windows)
}

/// Resolve `[infer].precision` into the precision the runner will actually use.
///
/// `budget_known` is whether a `DeviceBudget` could be built for the chosen
/// adapter. It is a parameter rather than a probe call so this is testable
/// without a GPU — the same reason the rest of the planner takes a budget
/// instead of reading one.
#[must_use]
pub const fn resolve_precision(cfg: &InferConfig, budget_known: bool) -> ResolvedPrecision {
    match cfg.precision {
        InferPrecision::F16 => ResolvedPrecision {
            precision: InferPrecision::F16,
            reason: PrecisionReason::Stated,
        },
        InferPrecision::F32 => ResolvedPrecision {
            precision: InferPrecision::F32,
            reason: PrecisionReason::Stated,
        },
        InferPrecision::Auto if budget_known => ResolvedPrecision {
            // The actual descending walk is Mummu's; from Nanna's side the
            // planned outcome is whatever it hands back. F16 is the ceiling
            // `plan::Precision` offers, so a known budget can only land on
            // f16 or f32 and the caller re-reads the real answer from `Fit`.
            precision: InferPrecision::F16,
            reason: PrecisionReason::Planned,
        },
        InferPrecision::Auto => ResolvedPrecision {
            precision: InferPrecision::F32,
            reason: PrecisionReason::Unsized,
        },
    }
}

/// What the runner should be asked to do at boot, derived from config alone.
///
/// Separated from any Mummu call so the decision is inspectable (and testable)
/// on a machine with no GPU and no weights on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPlan {
    /// Catalog name of the chat model, when one is configured.
    pub chat_model: Option<String>,
    /// Catalog name of the embedding model, when one is configured.
    pub embedding_model: Option<String>,
    pub device: InferDevice,
    pub precision: ResolvedPrecision,
    /// Operator VRAM override, forwarded verbatim; `None` leaves the policy
    /// with `DeviceBudget::usable_bytes()`.
    pub vram_budget_bytes: Option<u64>,
}

impl LocalPlan {
    /// Build the plan for a config. Returns `None` when local inference is off
    /// or names nothing — callers use that to skip registering the tier
    /// entirely rather than registering a tier that would fail on first use.
    #[must_use]
    pub fn from_config(cfg: &InferConfig, budget_known: bool) -> Option<Self> {
        if !cfg.enabled {
            return None;
        }
        let chat_model = cfg
            .has_local_chat_model()
            .then(|| cfg.model.trim().to_string());
        let embedding_model = cfg
            .has_local_embedding_model()
            .then(|| cfg.embedding_model.trim().to_string());
        if chat_model.is_none() && embedding_model.is_none() {
            return None;
        }
        debug_assert!(
            chat_model.as_ref().is_none_or(|m| !m.is_empty()),
            "a chat model in the plan is never the empty string"
        );
        Some(Self {
            chat_model,
            embedding_model,
            device: cfg.device,
            precision: resolve_precision(cfg, budget_known),
            vram_budget_bytes: cfg.vram_budget_bytes,
        })
    }

    /// Whether this plan adds a local *completion* tier to the router.
    ///
    /// The router is frozen at boot, so this is read once while building the
    /// provider map — a tier that appears later would be invisible.
    #[must_use]
    pub const fn provides_completions(&self) -> bool {
        self.chat_model.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Config;

    #[test]
    fn infer_defaults_are_inert() {
        let c = InferConfig::default();
        assert!(!c.enabled, "the local runner must be opt-in");
        assert!(
            c.model.is_empty(),
            "no chat model is chosen for the operator"
        );
        assert!(!c.has_local_chat_model());
        assert!(
            !c.has_local_embedding_model(),
            "a named embedding model must still not be used while disabled"
        );
        assert!(c.validate().is_ok());
    }

    #[test]
    fn infer_enabled_naming_no_model_is_rejected() {
        let c = InferConfig {
            enabled: true,
            embedding_model: String::new(),
            ..Default::default()
        };
        let err = c
            .validate()
            .expect_err("enabled with no model at all is not honorable");
        assert!(format!("{err}").contains("infer.model"), "got: {err}");
    }

    #[test]
    fn infer_zero_vram_budget_is_rejected() {
        let c = InferConfig {
            enabled: true,
            vram_budget_bytes: Some(0),
            ..Default::default()
        };
        assert!(c.validate().is_err(), "a zero budget can never fit a model");
        let ok = InferConfig {
            enabled: true,
            vram_budget_bytes: None,
            ..Default::default()
        };
        assert!(
            ok.validate().is_ok(),
            "omitting it is the documented default"
        );
    }

    #[test]
    fn infer_whitespace_model_name_is_not_a_model() {
        let c = InferConfig {
            enabled: true,
            model: "   ".to_string(),
            ..Default::default()
        };
        assert!(
            !c.has_local_chat_model(),
            "a whitespace-only name would reach the catalog lookup as a miss"
        );
    }

    #[test]
    fn infer_roundtrips_through_toml_and_absent_section_defaults() {
        let mut cfg = Config::default();
        cfg.infer.enabled = true;
        cfg.infer.model = "qwen2.5-1.5b-instruct-q4km".to_string();
        cfg.infer.precision = InferPrecision::F16;
        cfg.infer.device = InferDevice::Gpu;
        cfg.infer.vram_budget_bytes = Some(8 * 1024 * 1024 * 1024);

        let text = toml::to_string_pretty(&cfg).expect("serialize");
        assert!(text.contains("[infer]"), "section must be written: {text}");
        let back: Config = toml::from_str(&text).expect("deserialize");
        assert_eq!(back.infer.model, "qwen2.5-1.5b-instruct-q4km");
        assert_eq!(back.infer.precision, InferPrecision::F16);
        assert_eq!(back.infer.device, InferDevice::Gpu);
        assert_eq!(back.infer.vram_budget_bytes, Some(8 * 1024 * 1024 * 1024));

        // A config.toml predating this section must still load unchanged.
        let legacy: Config = toml::from_str("[general]\nname = \"Nanna\"\n").expect("legacy load");
        assert!(!legacy.infer.enabled);
        assert_eq!(legacy.infer.embedding_model, DEFAULT_LOCAL_EMBEDDING_MODEL);
    }

    #[test]
    fn infer_precision_and_device_serialize_snake_case() {
        // The names an operator types into config.toml are part of the contract.
        #[derive(Serialize)]
        struct P {
            v: InferPrecision,
        }
        #[derive(Serialize)]
        struct D {
            v: InferDevice,
        }
        assert_eq!(
            toml::to_string(&P {
                v: InferPrecision::F16
            })
            .unwrap()
            .trim(),
            "v = \"f16\""
        );
        assert_eq!(
            toml::to_string(&P {
                v: InferPrecision::Auto
            })
            .unwrap()
            .trim(),
            "v = \"auto\""
        );
        assert_eq!(
            toml::to_string(&D {
                v: InferDevice::Cpu
            })
            .unwrap()
            .trim(),
            "v = \"cpu\""
        );
    }

    fn enabled() -> InferConfig {
        InferConfig {
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn disabled_config_yields_no_plan() {
        let cfg = InferConfig::default();
        assert!(LocalPlan::from_config(&cfg, true).is_none());
    }

    #[test]
    fn embedder_only_plan_does_not_offer_completions() {
        let plan = LocalPlan::from_config(&enabled(), false).expect("embedder is configured");
        assert!(!plan.provides_completions());
        assert_eq!(plan.embedding_model.as_deref(), Some("all-minilm-l6-v2"));
        assert!(plan.chat_model.is_none());
    }

    #[test]
    fn a_named_chat_model_provides_completions() {
        let cfg = InferConfig {
            model: "qwen2.5-1.5b-instruct-q4km".into(),
            ..enabled()
        };
        let plan = LocalPlan::from_config(&cfg, false).expect("plan");
        assert!(plan.provides_completions());
        assert_eq!(
            plan.chat_model.as_deref(),
            Some("qwen2.5-1.5b-instruct-q4km")
        );
    }

    #[test]
    fn enabled_but_naming_nothing_yields_no_plan() {
        let cfg = InferConfig {
            embedding_model: String::new(),
            ..enabled()
        };
        assert!(
            LocalPlan::from_config(&cfg, true).is_none(),
            "registering a tier that names no model would fail on first use"
        );
    }

    #[test]
    fn auto_precision_without_a_budget_is_f32_and_says_why() {
        let r = resolve_precision(&enabled(), false);
        assert_eq!(r.precision, InferPrecision::F32);
        assert_eq!(r.reason, PrecisionReason::Unsized);
        let msg = r.announcement();
        assert!(msg.contains("reports no VRAM size"), "got: {msg}");
        assert!(
            msg.contains("[infer].precision = \"f16\""),
            "the announcement must name the remedy: {msg}"
        );
    }

    #[test]
    fn a_stated_precision_is_never_overridden_by_the_planner() {
        let cfg = InferConfig {
            precision: InferPrecision::F16,
            ..enabled()
        };
        // Even with no budget to plan against, an explicit choice stands —
        // that is the entire point of the override on a host that cannot size.
        let r = resolve_precision(&cfg, false);
        assert_eq!(r.precision, InferPrecision::F16);
        assert_eq!(r.reason, PrecisionReason::Stated);
    }

    #[test]
    fn vram_override_is_forwarded_verbatim() {
        let cfg = InferConfig {
            vram_budget_bytes: Some(6 * 1024 * 1024 * 1024),
            ..enabled()
        };
        let plan = LocalPlan::from_config(&cfg, false).expect("plan");
        assert_eq!(plan.vram_budget_bytes, Some(6 * 1024 * 1024 * 1024));
    }

    #[test]
    fn platform_vram_predicate_matches_mummus_own_gate() {
        // Mummu fills GpuAdapter::vram_bytes only on Windows; if that ever
        // changes upstream this assertion is where the mismatch surfaces.
        assert_eq!(platform_reports_vram(), cfg!(windows));
    }
}
