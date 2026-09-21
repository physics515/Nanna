# Nanna v0.3.26-beta.35 — Chats That Carry On, Keys That Stay Put

When the first model in your list fails, chat now moves on to the next one instead of giving up.
Every API key, bot token and webhook secret now lives only in your system's secure store, never in
config.toml, and a key you change, clear or reset stays that way.

## What's New

**Chat falls back to your next model when the first one fails.** Chat used to run every step on
the first model in your priority list. When that model's provider failed, chat kept retrying it
until the reply ended in an error such as `error decoding response body`, even with working models
listed after it. Now a step that fails on one model runs again on the next model in the list, and
the rest of that reply stays on it. The next message starts from the top of the list again. Each
switch is shown in the chat and on the model badge. A chat pinned to one model is never moved off
it.

**Tools you create survive a restart.** A tool made on the Tools page was saved to disk but never
loaded again, so after the daemon restarted it disappeared from the list, the model could no
longer call it, and it could not be enabled or disabled. Saved tools now load at startup.

## What's Fixed

**Secrets are no longer written to config.toml.** Channel secrets (the Telegram, Discord, Slack,
Signal and WhatsApp tokens and signing secrets), the `nanna server` webhook secret, and API keys
typed into config.toml by hand were kept there in plain text. Each one is now filed in the secure
store (the OS keyring, or its encrypted-file fallback) when config.toml is loaded. The next save of
your settings removes it from the file, and until then a warning says the line can be deleted.

**A key you change or clear stays changed.** A key or token changed through the daemon's settings
(`config.set`, import or reset) reached the running app but not the secure store, and the daemon
reloads its settings a couple of seconds after each save. So a new key vanished moments later, a
key you cleared came back, and a reset or import briefly left chat without your saved keys. Each
change is now recorded where the next load reads it.

**A key set in the environment is respected, and never copied to disk.** When a key comes from an
environment variable, such as `BRAVE_API_KEY`, the daemon now refuses to change it and names the
variable to unset, instead of accepting a change that would be undone within seconds. Setting some
other key no longer copies the environment's keys into the keyring either.

**Settings lists your OpenAI models with a stored key.** Listing OpenAI models failed with "No
OpenAI API key configured" unless `OPENAI_API_KEY` was set, even with a key saved in Settings.

**Your data folder is used for everything.** With `[general] data_dir` set, the daemon kept user
tools in the default location instead of your data folder.

### For command-line users

- A key entered at the first-run prompt now works for the chat it was entered for. Before, that
  chat started without it.
- `nanna serve` uses the chosen provider's key. It read the Anthropic key for every provider.
  `OPENROUTER_API_KEY` now overrides the stored OpenRouter key, and a variable that is set but empty
  no longer wipes a stored key.
- `nanna daemon start` runs the same daemon as the desktop app and stops cleanly on a shutdown
  signal, and starting a second daemon no longer takes over the first one's PID file.

---

Updating from 0.3.24 or earlier? The
[0.3.25 release notes](https://github.com/physics515/Nanna/releases/tag/v0.3.25-beta.34) cover
Ctrl+Enter sending reliably and channel settings kept with a token in the environment, and link to
0.3.24's.
