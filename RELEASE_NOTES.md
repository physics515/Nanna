# Nanna v0.3.25-beta.34 — Sent When You Say Send

A small release: the chat fix that just missed 0.3.24, and a config fix that landed after it.
Pressing Ctrl+Enter the moment you finished typing could leave your message sitting in the box,
unsent, with nothing to say why. And a Telegram or Discord bot token set in the environment quietly
threw away the rest of that channel's settings, including, for Telegram, the list of people allowed
to talk to the bot.

Two fixes that are in 0.3.24 were left out of its release notes. They are described at the end.

## What's Fixed

**Ctrl+Enter right after typing always sends.** The composer judged whether it was empty from a
copy of your text that lagged a frame or two behind, so a message sent the instant it was typed
could stay in the box, unsent and unexplained. It now reads the text itself, so the shortcut and the
Send button agree with what you see.

**A channel token in the environment no longer wipes the channel's settings.** With
`TELEGRAM_BOT_TOKEN` exported, a configured `[channels.telegram]` section lost its `allowed_users`
list, so anyone could drive the bot, and its webhook URL and secret, so the webhook refused every
request. The three Discord variables rebuilt `[channels.discord]` from scratch in the same way. Each
variable now replaces only the field it names, and the rest of the section is kept. A variable left
blank changes nothing, and `DISCORD_BOT_TOKEN` on its own now overrides a configured token (before,
it was ignored unless the other two were set as well).

## Also in 0.3.24, Missing From Its Notes

**Each provider's API key stays with that provider.** A key that `nanna init` saved for OpenRouter
or OpenAI was also read as the Anthropic key, so a `claude-*` chat or an Anthropic summary could
send it to Anthropic. Every provider now has its own field and keyring entry. The first time Nanna
loads a config saved the old way, it moves the key to its provider's entry, once. A key with
Anthropic's `sk-ant-` prefix is never moved. Neither is a key that disagrees with one already in the
provider's own place: both are kept, and a warning names the conflict.

**Sending no longer leaves an invisible line break behind.** After Ctrl+Enter the composer looked
empty but still held a line break, so the Send button stayed lit.

---

Updating from 0.3.22 or earlier? The
[0.3.24 release notes](https://github.com/physics515/Nanna/releases/tag/v0.3.24-beta.33) cover the
rest of what is new to you: plain chat that answers once, pictures that reach the model, streaming
that survives any language, and the memory speedups from 0.3.23.
