# GUI UX bug bash log

Rolling short list for P4 UI/UX quality. Promote fixed items to dated `[x]` lines in `ROADMAP.md`.

## Open

- [ ] Channel wizard multi-step forms: bulk inline validation coverage beyond token/url fields (Telegram secrets, Discord snowflakes) still uneven across wizards.
- [ ] Long lists (memory browser, logs when paused with thousands of rows, tools registry): virtualization not yet applied; large workspaces may jank on 1280×720.
- [ ] Command palette UI itself is reserved (`Ctrl/Cmd+K` swallows) but not implemented — only the shortcut reservation landed.
- [ ] GUI unit tests depend on a healthy `pnpm`/`node_modules` tree; Windows file locks under concurrent installs can leave `vitest` unlinkable (environment; query `lru-cache` / antivirus holds).

## Fixed this pass (2026-04-27)

- [x] Bare "Disconnected" chrome next to live log data — replaced with endpoint-aware offline/reconnecting/starting/crashed labels via `backendLabels`.
- [x] Silent blank panels on agents/memory/tools/channels/stats/scheduler/workspaces/tasks — shared `PageState` + loadError/offline/empty.
- [x] Escape did not reliably dismiss confirm dialogs — `pushEscapeHandler` stack + ConfirmDialog wiring.
- [x] API key save could submit empty/short keys without inline error — `formValidation` + `ApiKeyInput`.
- [x] Close-to-tray vs quit semantics undocumented — `gui/docs/WINDOW_TRAY.md`.
- [x] Global shortcuts for new chat / focus input / stop / reserved palette.
