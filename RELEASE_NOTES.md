# Nanna v0.3.27-beta.36 — Environment Keys Stay in the Environment

A key your environment supplies, such as `BRAVE_API_KEY`, is no longer copied into your secure
store when you reset, import or change your settings.

## What's Fixed

**A key from the environment is never copied to the keyring.** In 0.3.26, changing some other
setting stopped copying the environment's keys, but three changes could still copy one:

- Importing settings that hold the key your environment variable already supplies, when
  config.toml held a different key at startup.
- Setting a key to that same value, in the same situation.
- A reset just after you removed a channel whose bot token comes from the environment, such as
  `TELEGRAM_BOT_TOKEN`. The reset rebuilt the channel from the variable and copied its token.

The copy then outlived the variable: after you unset or rotated it, the next start still ran on
the old key. Now a key the environment supplies is never copied, whatever change brings it in.

---

Updating from 0.3.25 or earlier? The
[0.3.26 release notes](https://github.com/physics515/Nanna/releases/tag/v0.3.26-beta.35) cover
chat falling back to your next model when the first one fails, tools that survive a restart, and
secrets kept out of config.toml, and link to 0.3.25's.
