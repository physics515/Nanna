/**
 * Markdown rendering for chat content.
 *
 * Everything this renders is untrusted: assistant output, user input, and — via
 * the assistant quoting what a tool returned — the contents of any web page or
 * file the agent touched. The result is handed to `v-html`, so whatever comes
 * out of here is parsed as HTML by the webview that also holds
 * `__TAURI_INTERNALS__.invoke`.
 *
 * `marked` does not sanitize (the `sanitize` option was removed in v5), so by
 * default it passes author HTML straight through:
 *
 *   `<img src=x onerror="alert(1)">`  ->  `<img src=x onerror="alert(1)">`
 *   `[click](javascript:alert(1))`    ->  `<a href="javascript:alert(1)">`
 *
 * `innerHTML` will not run a `<script>` tag, but it *will* fire an `<img>`
 * `onerror`. Today the only thing stopping that is the Tauri CSP
 * (`script-src 'self'`, no `unsafe-inline`) — one header, on one config file,
 * standing between injected markup and local command execution. This module is
 * the second layer:
 *
 * 1. **Author HTML is escaped, never emitted.** The block and inline `html`
 *    renderers return escaped text, so a tag written in the source is *shown*
 *    rather than parsed. Every tag in the output is one marked itself
 *    generated, from a fixed set.
 * 2. **Link and image URLs are scheme-checked.** Only `http:`, `https:` and
 *    `mailto:` survive (images narrow that to `http:`/`https:`); anything else
 *    — most of all `javascript:` — renders as inert text.
 *
 * This is a deliberate behaviour change: a message containing `<div>` now shows
 * the literal characters instead of laying out a div. That is what a chat client
 * should do with markup it did not author, and it is what every mainstream chat
 * UI does.
 */

import { Marked } from 'marked'
import type { Tokens } from 'marked'

/**
 * URL schemes a link may carry.
 *
 * An allowlist rather than a `javascript:` denylist: the ways to spell a script
 * URL are open-ended (`java\tscript:`, `JaVaScRiPt:`, `&#106;avascript:`,
 * `vbscript:`, `data:text/html`), while the set of schemes chat content has any
 * business linking to is small and closed.
 */
export const ALLOWED_LINK_SCHEMES = ['http:', 'https:', 'mailto:'] as const

/** Schemes an image `src` may carry, beyond the link set. */
export const ALLOWED_IMAGE_SCHEMES = ['http:', 'https:'] as const

/** Longest URL kept. Beyond this the link renders as plain text. */
export const MAX_URL_LENGTH = 8192

const HTML_ESCAPES: Record<string, string> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
}

/** Escape the five characters that can break out of text or an attribute. */
export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (ch) => HTML_ESCAPES[ch] ?? ch)
}

/**
 * A URL's scheme in lowercase, or `null` when it has none (a relative URL) or
 * cannot be parsed at all.
 *
 * Parsing with `URL` rather than a regex is what makes the check robust: the
 * platform parser already strips the leading/embedded whitespace and control
 * characters that a hand-rolled matcher misses, so `java\tscript:alert(1)`
 * resolves to the `javascript:` scheme here instead of slipping through as an
 * unrecognised string.
 */
function schemeOf(url: string): string | null {
  try {
    return new URL(url, 'https://nanna.invalid/').protocol.toLowerCase()
  } catch {
    return null
  }
}

/**
 * True when `url` is safe to place in an `href`/`src`.
 *
 * Relative URLs resolve against a base with an allowed scheme, so they pass —
 * a relative link in chat content cannot escape the app's own origin, and the
 * CSP bounds where it could go anyway.
 */
export function isSafeUrl(url: string, allowed: readonly string[]): boolean {
  if (url.length > MAX_URL_LENGTH) return false
  const scheme = schemeOf(url)
  return scheme !== null && allowed.includes(scheme)
}

/** Build the `title="..."` attribute, or nothing when there is no title. */
function titleAttr(title: string | null | undefined): string {
  return title ? ` title="${escapeHtml(title)}"` : ''
}

/**
 * A marked instance whose renderer cannot emit author HTML or an unsafe URL.
 *
 * A private `Marked` instance rather than the shared default export, because the
 * default is process-global: configuring it here would push this policy onto
 * every other caller of `marked` in the app, and — the direction that actually
 * matters — a `marked.setOptions` or `marked.use` anywhere else could quietly
 * replace the renderer this module depends on. A security property must not be
 * reachable by unrelated code.
 */
const safeMarked = new Marked({
  breaks: true,
  gfm: true,
  renderer: {
    /**
     * Block-level and inline raw HTML. Both arrive here; returning escaped text
     * is what turns "parse this markup" into "show this markup".
     */
    html({ text }: Tokens.HTML | Tokens.Tag): string {
      return escapeHtml(text)
    },

    link(this: { parser: { parseInline: (t: Tokens.Generic[]) => string } }, token: Tokens.Link): string {
      const label = this.parser.parseInline(token.tokens)
      if (!isSafeUrl(token.href, ALLOWED_LINK_SCHEMES)) {
        // Keep the label visible — dropping it would silently delete content —
        // but strip the anchor so the refused URL is not clickable.
        return label
      }
      return `<a href="${escapeHtml(token.href)}"${titleAttr(token.title)}>${label}</a>`
    },

    image(token: Tokens.Image): string {
      const alt = escapeHtml(token.text ?? '')
      if (!isSafeUrl(token.href, ALLOWED_IMAGE_SCHEMES)) {
        return alt
      }
      return `<img src="${escapeHtml(token.href)}" alt="${alt}"${titleAttr(token.title)}>`
    },
  },
})

/**
 * Render chat markdown to HTML that is safe to hand to `v-html`.
 *
 * Never throws: a parser failure returns the escaped source, so a malformed
 * message degrades to plain text instead of blanking the bubble.
 */
export function renderMarkdown(text: string): string {
  try {
    return safeMarked.parse(text, { async: false }) as string
  } catch {
    return escapeHtml(text)
  }
}
