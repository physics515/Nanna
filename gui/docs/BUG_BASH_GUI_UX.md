# GUI UX bug bash

Rolling list of open polish. Promote closed items into ROADMAP dated `[x]` lines when shipped.

## Shipped (2026-04-27 / #58 tail)
- PageState empty/loading/error/offline on agents, channels, memory, tools, stats, scheduler, workspaces, tasks, logs
- Truthful backend labels (`app/lib/backendLabels.ts`) — no bare "Disconnected"
- Toasts + ConfirmDialog Escape / outside-click; destructive paths via `useConfirm`
- Chat stick-to-bottom (`userScrolledUp`) + settings per-tab scroll restore
- Density tokens + ≥32px toolbar hit targets
- Form validation helpers + `ApiKeyInput` inline errors
- WINDOW_TRAY.md close/tray/sidecar semantics

## Shipped (2026-07-22, UX simplification track)
- **Command palette (Mod+K)** — `CommandPalette.vue` + `lib/commandPalette.ts`; navigate pages, switch sessions/workspaces, new chat, focus input, stop generation, toggle live logs, compact density, settings deep-link
- **List virtualization** — `VirtualList.vue` + pure `visibleRange` helper; wired memory (>80), logs (>100), tools sidebar (>60)
- **IA / chat-first nav** — primary activity icons (Memory, Tasks, Tools, Channels); admin (Logs, Workspaces, Agents, Scheduler, stats) under **More** flyout; Settings stays bottom
- **Progressive disclosure** — Settings `Show advanced` toggle; rarely-used knobs (compression floors, iteration/nudge, model routing, Ollama host details) folded behind it via `SettingsSection`
- **Settings shell unify** — shared `SettingsSection` (title/description/danger/advanced) on Models/Agent/Memory/Data/Scheduler
- **Onboarding compression** — 3-step `OnboardingWizard` (what → backend/key → health → chat); `nanna.onboarding.done`
- **Copy/tone** — calmer settings toasts; logs offline line names 5149; Live/Paused remain truthful counter labels
- **Component cleanup inventory** — `gui/docs/COMPONENT_CLEANUP.md`
- **Compact density** — `html.density-compact` + localStorage `nanna.ui.density`; palette toggle

## Open
- Formal **1280×720 / 1440×900** clipped-CTA visual pass (manual; VirtualList helps long lists but does not replace the viewport audit)
- Command **palette polish**: fuzzy ranking, recent-commands, tool invoke shortcuts
- Channel wizard bulk/multi-step validation still uneven vs single-field ApiKeyInput
- Windows `node_modules` / vitest file-lock flakiness under concurrent test runs
- **Onboarding**: health step uses `get_backend_status` only — does not yet validate a model is pulled (Ollama) or a cloud key works end-to-end (P0.1 remainder)
- **Settings Advanced**: Tools tab not yet folded through SettingsSection (thin already)
- Optional: push-logs channel (avoid 1s 2000-line re-serialize) — leftover from P4 logs poll
- Legacy clawd/Nanna config-path copy residual (local channels toast ref in bug-bash hotfixes)

## Build / packaging (2026-07-23)
- **[mitigated] Nuxt generate ENOENT on `dist/client/manifest.json`** — dual client Vite passes + artifacts under `node_modules/.cache/nuxt/.nuxt` raced mid-generate (`ELIFECYCLE` exit 1 while nitro kept prerendering). Mitigations: pin `buildDir: '.nuxt'`, drop crawlLinks prerender to `/` only, and wipe `.nuxt` + `node_modules/.cache/nuxt` before every `pnpm generate` (`scripts/clean-nuxt-cache.mjs`). *Still watch for dual "Building client..." lines after a cold wipe — if they reappear the root is Nuxt 4 running two vite client envs concurrently.*
- **Clippy noise:** unused `README_FILE` import in `nanna-workspace::manager` (production import) — scoped to `#[cfg(test)]`.
- **Monaco chunk size** — `DIJMKxcW.js` ~4 MB minified; deferred under GUI backlog (lazy-load Monaco).
- **Dynamic+static import of `@tauri-apps/api/window`** — TitleBar/default.vue dynamic import unused because `useCloseHandler` already static-imports it. Collapse to one style.

## Regressions to watch
- Do not splice script/composables mid-`interface` in SFCs (broke `nuxt generate` post-#58)
- P16: no embedded fallback — offline must stay honest Disconnected, never silent second backend
- VimMode extension is empty stub (`extensions/VimMode.ts`) — do not advertise until implemented
