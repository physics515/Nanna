# Window, tray, and close semantics (Windows primary)

Nanna is a pure daemon client (P16). The GUI never owns agent state; closing the
window must not quietly kill the daemon unless the user chose **Quit**.

## Title bar (Windows)

| Control | Behavior |
|--------|----------|
| Minimize | OS minimize (taskbar). Daemon keeps running. |
| Maximize / restore | Standard. |
| Close (X) | Honors the saved **close mode** (see below). |

## Close modes

Stored via `get_close_mode` / `set_close_mode` (OS preference / config):

| Mode | Close (X) does | Daemon |
|------|----------------|--------|
| `ask` (default) | Opens CloseDialog: Hide to tray / Quit | Unchanged until Quit |
| `minimize_to_tray` | Hide window to tray immediately | Keeps running (sidecar) |
| `quit_completely` | Tear down window **and** stop the managed daemon sidecar | Stops |

`useCloseHandler` routes through the Tauri `handle_window_close` command so the
Rust side can hide vs exit consistently. Closing to tray must **not** orphan a
GUI-spawned sidecar on next launch: login_item / single-instance + the daemon
PID lock prevent double-start; quitting explicitly stops the sidecar.

## Tray

- Left-click / "Show": restores the main window.
- "Quit Nanna": same path as Quit in CloseDialog — stops managed sidecar.
- Tooltip should say whether the daemon is attached (`Daemon` / specific offline reason), never a bare "Disconnected" next to live Logs.

## Tests / smoke

- Unit: close-mode invoke path (`useCloseHandler` load/save).
- Manual: Close → tray → reopen attaches same daemon on 5149; Quit → `/health` down; relaunch starts fresh sidecar.
- E2E (soft): window close hygiene in `gui/e2e/tauri-driver.md`.

## Bugs found during P4 UX (logged)

See ROADMAP **Bug bash (gui-ux)** for rolling items (e.g. push channel for logs poll, command palette still swallowed).
