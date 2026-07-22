# Component cleanup inventory

Live notebook of near-duplicates and dead weight in `gui/app`. Goal: consolidate without big rewrites until a pass is scheduled.

## Dialogs / modals

| Component | Path | Role | Notes |
|-----------|------|------|-------|
| `ConfirmDialog` | `gui/app/components/ConfirmDialog.vue` | Global confirm via `useConfirm()` | Full featured (danger, async confirm) |
| `CloseDialog` | `gui/app/components/CloseDialog.vue` | Window close / quit preferences | Domain-specific; keep separate |
| `UiModal` | `gui/app/components/ui/modal.vue` | Generic presentational modal | Prefer this for new one-off modals |

**Recommendation:** New confirmations → `useConfirm` + `ConfirmDialog`. New content modals → `UiModal`. Do not merge `CloseDialog` (product-specific). Onboarding uses a lightweight Teleport shell (not UiModal) to avoid layout coupling; optional follow-up to switch.

**Done this pass:** None merged. Pattern documented.

## Status badges / connection chrome

| Component | Path | Role | Overlap |
|-----------|------|------|---------|
| `BackendStatus` | `gui/app/components/BackendStatus.vue` | Compact backend pill | Shared label helpers via `backendLabels` |
| `ConnectionStatus` | `gui/app/components/ConnectionStatus.vue` | Richer connection/daemon UI | Overlaps BackendStatus messaging |
| status-bar patterns | TitleBar / session chrome | Inline dots + short labels | Ad-hoc classes |
| `ModelStatusBadge` | `gui/app/components/ModelStatusBadge.vue` | Model health / readiness | Different domain — keep |
| `SessionActivityBadge` | `gui/app/components/SessionActivityBadge.vue` | Session activity | Keep |

**Recommendation:** Long-term single `StatusPill` primitive consuming `backendLabels`. Do **not** force-merge now — paths inventoried for a dedicated pass.

## Settings section chrome

| Component | Path | Role |
|-----------|------|------|
| `SettingsSection` | `gui/app/components/settings/SettingsSection.vue` | **New** shared section header + advanced fold + danger tone |
| legacy `UiCard` headings | remaining tabs | Prefer `SettingsSection` for density |

**Done this pass:** Models, Agent, Memory, Data, Scheduler use `SettingsSection`. Tools can migrate next.

## Dead CSS / style suspects

- Prefer `ui-field-label` utility over ad-hoc `block text-sm font-medium…` (partially adopted).
- Duplicate card shells (`UiCard` vs section padding) — migrate to `SettingsSection` to drop redundant `text-base font-semibold text-nanna-primary mb-4` headers.
- No worldwide CSS purge this pass.

## Copy / tone

High-visibility bare errors (`Failed:`, isolated `Error`) nudged toward calm specifics in settings toasts + logs empty/offline line. `logs.vue` Live/Paused remain truthful (Live = following sink; Paused = follow off).

## Suggested next consolidations (not done)

1. Tools tab → `SettingsSection`
2. Optional `StatusPill` shared by BackendStatus + ConnectionStatus inactive states
3. Onboarding → `UiModal` if focus-trap / ESC behavior needs parity
