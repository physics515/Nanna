//! Tool execution policy — allowlists and blocklists.
//!
//! A [`ToolPolicy`] decides whether a *canonical* tool name may execute. Two
//! properties make it a security boundary rather than a hint:
//!
//! 1. **Deny wins.** A name on the denylist is refused no matter what the
//!    allowlist says. Conflicts fail closed.
//! 2. **Overlay only narrows.** [`ToolPolicy::overlay`] composes a broader
//!    (global) policy with a narrower (per-channel, per-user) one and can only
//!    ever remove capability. A channel policy cannot re-grant a tool the global
//!    policy denied.
//!
//! # Canonical names only
//!
//! Policy decisions MUST be made on the name a call *resolved to*, never on the
//! name the model typed. [`crate::ToolRegistry::resolve_tool`] matches
//! exact → case-insensitive → fuzzy, and aliases (`bash`, `Bash`) map onto
//! canonical tools (`exec`). Checking the requested name would let `Bash`, `EXEC`,
//! or a fuzzy near-miss walk straight past a denylist entry for `exec`. The
//! registry therefore enforces policy *after* resolution and alias-canonicalization.

use std::collections::HashSet;

/// Why a tool call was refused, for logging and the model-facing error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// The tool is on the denylist.
    Blocked,
    /// An allowlist is in force and the tool is not on it.
    NotAllowed,
}

impl DenyReason {
    /// Human-readable explanation, safe to surface to the model.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked by tool policy",
            Self::NotAllowed => "not in the tool allowlist",
        }
    }
}

/// An allow/deny policy over canonical tool names.
///
/// The default policy allows every tool: `allow = None`, `deny` empty. This
/// keeps the policy layer opt-in — an unconfigured registry behaves exactly as
/// it did before policy existed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolPolicy {
    /// `None` means "no allowlist in force" (every tool passes this gate).
    /// `Some(set)` restricts execution to exactly `set`; `Some(empty)` denies all.
    allow: Option<HashSet<String>>,
    /// Names refused unconditionally. Takes precedence over `allow`.
    deny: HashSet<String>,
}

impl ToolPolicy {
    /// A policy that permits every tool. Equivalent to [`Default`].
    #[must_use]
    pub fn allow_all() -> Self {
        Self::default()
    }

    /// Restrict execution to `names` (canonical tool names).
    ///
    /// An empty iterator produces a policy that denies everything, which is a
    /// legitimate configuration (a fully locked-down channel), not an error.
    #[must_use]
    pub fn allow_only<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            allow: Some(names.into_iter().map(Into::into).collect()),
            deny: HashSet::new(),
        }
    }

    /// Refuse `names` (canonical tool names); everything else is permitted.
    #[must_use]
    pub fn deny_only<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            allow: None,
            deny: names.into_iter().map(Into::into).collect(),
        }
    }

    /// Build a policy from the `[tools] enabled` / `[tools] disabled` config
    /// lists — the single interpretation of those two settings.
    ///
    /// `enabled` is treated as a real allowlist only when it is non-empty and
    /// does not contain the `"*"` wildcard; otherwise it means "no restriction"
    /// (the default config ships `enabled = ["*"]`). `disabled` always applies
    /// as a denylist on top, and deny beats allow, so a name on both lists fails
    /// closed.
    ///
    /// Lives here rather than in a host crate so every entry point that exposes
    /// tools — the daemon, and the `nanna mcp serve` bridge — reads the user's
    /// configuration identically. A second copy is a security bug waiting to
    /// happen: the two would drift and one surface would offer a tool the user
    /// had disabled.
    #[must_use]
    pub fn from_config_lists(enabled: Option<&[String]>, disabled: &[String]) -> Self {
        // Matched by `filter` so there is no `unwrap` on this path.
        let real_allowlist = enabled.filter(|e| !e.is_empty() && !e.iter().any(|n| n == "*"));

        let base = match real_allowlist {
            Some(names) => Self::allow_only(names.iter().cloned()),
            None => Self::allow_all(),
        };

        let policy = if disabled.is_empty() {
            base
        } else {
            base.with_denied(disabled.iter().cloned())
        };

        debug_assert!(
            disabled.iter().all(|name| policy.deny.contains(name)),
            "every disabled name must end up denied"
        );
        debug_assert!(
            real_allowlist.is_some() || policy.allow.is_none(),
            "a wildcard or empty `enabled` must not produce an allowlist"
        );
        policy
    }

    /// Add denied names to this policy, returning the narrowed policy.
    #[must_use]
    pub fn with_denied<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.deny.extend(names.into_iter().map(Into::into));
        self
    }

    /// True when this policy constrains nothing (the default, allow-all).
    ///
    /// The registry uses this to skip policy work entirely on the hot path.
    #[must_use]
    pub fn is_unrestricted(&self) -> bool {
        self.allow.is_none() && self.deny.is_empty()
    }

    /// Decide whether `canonical_name` may execute.
    ///
    /// `canonical_name` must already be alias-resolved — see the module docs.
    /// Returns `Ok(())` when permitted, or the reason for refusal.
    ///
    /// # Errors
    ///
    /// Returns [`DenyReason::Blocked`] if the name is on the denylist, or
    /// [`DenyReason::NotAllowed`] if an allowlist is in force and excludes it.
    pub fn check(&self, canonical_name: &str) -> Result<(), DenyReason> {
        debug_assert!(
            !canonical_name.is_empty(),
            "policy check requires a non-empty tool name"
        );

        // Deny wins: evaluated first so a name present in BOTH lists fails closed.
        if self.deny.contains(canonical_name) {
            return Err(DenyReason::Blocked);
        }

        if let Some(ref allow) = self.allow
            && !allow.contains(canonical_name)
        {
            return Err(DenyReason::NotAllowed);
        }

        debug_assert!(
            !self.deny.contains(canonical_name),
            "permitted name must not be on the denylist"
        );
        Ok(())
    }

    /// Convenience predicate over [`Self::check`].
    #[must_use]
    pub fn permits(&self, canonical_name: &str) -> bool {
        self.check(canonical_name).is_ok()
    }

    /// Compose `self` (broader) with `narrower`, producing a policy that permits
    /// only what **both** permit.
    ///
    /// This is the per-channel / per-user layering primitive. It is monotonically
    /// restrictive by construction: denials union, allowlists intersect. A
    /// narrower layer can never re-grant capability the broader layer withheld.
    #[must_use]
    pub fn overlay(&self, narrower: &Self) -> Self {
        let mut deny = self.deny.clone();
        deny.extend(narrower.deny.iter().cloned());

        let allow = match (self.allow.as_ref(), narrower.allow.as_ref()) {
            (None, None) => None,
            (Some(a), None) => Some(a.clone()),
            (None, Some(b)) => Some(b.clone()),
            // Both constrain: only names on both lists survive.
            (Some(a), Some(b)) => Some(a.intersection(b).cloned().collect()),
        };

        let composed = Self { allow, deny };

        // Postcondition: the overlay never widens either input. Bounded by the
        // number of configured names, which is bounded by the tool registry.
        debug_assert!(
            self.deny.iter().all(|n| !composed.permits(n)),
            "overlay must preserve every denial of the broader policy"
        );
        debug_assert!(
            narrower.deny.iter().all(|n| !composed.permits(n)),
            "overlay must preserve every denial of the narrower policy"
        );
        composed
    }

    /// Names on the denylist, for diagnostics and UI display.
    pub fn denied(&self) -> impl Iterator<Item = &str> {
        self.deny.iter().map(String::as_str)
    }

    /// The allowlist, if one is in force.
    #[must_use]
    pub fn allowed(&self) -> Option<impl Iterator<Item = &str>> {
        self.allow.as_ref().map(|a| a.iter().map(String::as_str))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_permits_everything() {
        let p = ToolPolicy::default();
        assert!(p.is_unrestricted());
        assert!(p.permits("exec"));
        assert!(p.permits("anything_at_all"));
    }

    #[test]
    fn denylist_refuses_named_tool_only() {
        let p = ToolPolicy::deny_only(["exec"]);
        assert_eq!(p.check("exec"), Err(DenyReason::Blocked));
        assert!(p.permits("read_file"));
        assert!(!p.is_unrestricted());
    }

    #[test]
    fn allowlist_refuses_everything_else() {
        let p = ToolPolicy::allow_only(["read_file", "recall"]);
        assert!(p.permits("read_file"));
        assert!(p.permits("recall"));
        assert_eq!(p.check("exec"), Err(DenyReason::NotAllowed));
    }

    #[test]
    fn empty_allowlist_denies_all() {
        let p = ToolPolicy::allow_only(Vec::<String>::new());
        assert_eq!(p.check("read_file"), Err(DenyReason::NotAllowed));
        assert!(!p.is_unrestricted());
    }

    #[test]
    fn deny_wins_over_allow_for_the_same_name() {
        // A name on both lists must fail closed.
        let p = ToolPolicy::allow_only(["exec", "read_file"]).with_denied(["exec"]);
        assert_eq!(p.check("exec"), Err(DenyReason::Blocked));
        assert!(p.permits("read_file"));
    }

    #[test]
    fn overlay_unions_denials() {
        let global = ToolPolicy::deny_only(["exec"]);
        let channel = ToolPolicy::deny_only(["write_file"]);
        let effective = global.overlay(&channel);
        assert_eq!(effective.check("exec"), Err(DenyReason::Blocked));
        assert_eq!(effective.check("write_file"), Err(DenyReason::Blocked));
        assert!(effective.permits("read_file"));
    }

    #[test]
    fn overlay_cannot_regrant_a_global_denial() {
        // The security property: a channel allowlist naming `exec` must NOT
        // resurrect it once the global policy denied it.
        let global = ToolPolicy::deny_only(["exec"]);
        let channel = ToolPolicy::allow_only(["exec", "read_file"]);
        let effective = global.overlay(&channel);
        assert_eq!(effective.check("exec"), Err(DenyReason::Blocked));
        assert!(effective.permits("read_file"));
    }

    #[test]
    fn overlay_intersects_allowlists() {
        let global = ToolPolicy::allow_only(["read_file", "write_file", "recall"]);
        let channel = ToolPolicy::allow_only(["write_file", "recall", "exec"]);
        let effective = global.overlay(&channel);
        assert!(effective.permits("write_file"));
        assert!(effective.permits("recall"));
        // Only on one list each — both must drop out.
        assert_eq!(effective.check("read_file"), Err(DenyReason::NotAllowed));
        assert_eq!(effective.check("exec"), Err(DenyReason::NotAllowed));
    }

    #[test]
    fn overlay_with_unrestricted_is_identity() {
        let open = ToolPolicy::allow_all();
        let p = ToolPolicy::allow_only(["recall"]).with_denied(["exec"]);
        assert_eq!(open.overlay(&p), p);
        assert_eq!(p.overlay(&open), p);
    }

    #[test]
    fn overlay_is_associative_over_denials() {
        let a = ToolPolicy::deny_only(["exec"]);
        let b = ToolPolicy::deny_only(["write_file"]);
        let c = ToolPolicy::deny_only(["fetch"]);
        assert_eq!(a.overlay(&b).overlay(&c), a.overlay(&b.overlay(&c)));
    }

    /// `[tools] enabled = ["*"]` is the shipped default and must mean "no
    /// restriction", not "allow a tool literally named `*`".
    #[test]
    fn from_config_wildcard_enabled_is_unrestricted() {
        let enabled = vec!["*".to_string()];
        let policy = ToolPolicy::from_config_lists(Some(&enabled), &[]);
        assert!(policy.is_unrestricted());
        assert!(policy.permits("exec"));
    }

    #[test]
    fn from_config_absent_or_empty_enabled_is_unrestricted() {
        // Neither an absent list nor an empty one is an allowlist — an empty
        // `enabled` in a config file must not silently lock out every tool.
        assert!(ToolPolicy::from_config_lists(None, &[]).is_unrestricted());
        assert!(ToolPolicy::from_config_lists(Some(&[]), &[]).is_unrestricted());
    }

    #[test]
    fn from_config_real_allowlist_restricts() {
        let enabled = vec!["read_file".to_string(), "list_dir".to_string()];
        let policy = ToolPolicy::from_config_lists(Some(&enabled), &[]);
        assert!(!policy.is_unrestricted());
        assert!(policy.permits("read_file"));
        assert!(
            !policy.permits("exec"),
            "a tool outside the allowlist is denied"
        );
    }

    #[test]
    fn from_config_disabled_denies_even_when_also_enabled() {
        // Deny beats allow: a name on BOTH lists must fail closed, or a typo in
        // `enabled` could silently re-grant something the user disabled.
        let enabled = vec!["read_file".to_string(), "exec".to_string()];
        let disabled = vec!["exec".to_string()];
        let policy = ToolPolicy::from_config_lists(Some(&enabled), &disabled);
        assert!(policy.permits("read_file"));
        assert!(!policy.permits("exec"));
    }

    #[test]
    fn from_config_disabled_applies_under_a_wildcard_too() {
        // The common real-world shape: `enabled = ["*"]` with a few `disabled`.
        let enabled = vec!["*".to_string()];
        let disabled = vec!["exec".to_string()];
        let policy = ToolPolicy::from_config_lists(Some(&enabled), &disabled);
        assert!(!policy.is_unrestricted());
        assert!(!policy.permits("exec"));
        assert!(policy.permits("read_file"));
    }

    #[test]
    fn deny_reason_messages_are_distinct() {
        assert_ne!(
            DenyReason::Blocked.as_str(),
            DenyReason::NotAllowed.as_str()
        );
    }
}
