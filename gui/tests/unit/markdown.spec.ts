import { describe, it, expect } from 'vitest'
import {
  ALLOWED_IMAGE_SCHEMES,
  ALLOWED_LINK_SCHEMES,
  MAX_URL_LENGTH,
  escapeHtml,
  isSafeUrl,
  renderMarkdown,
} from '~/lib/markdown'

/**
 * Everything rendered here is untrusted and lands in `v-html`, inside a webview
 * that holds `__TAURI_INTERNALS__.invoke`. The Tauri CSP (`script-src 'self'`)
 * is one header away from being the only thing between injected markup and a
 * local command; these tests are the second layer.
 */
/**
 * Assert against what the browser actually builds, not against the string.
 * `v-html` sets `innerHTML`, so "did an element get created" is the real
 * question — checking for a substring would pass on escaped text that still
 * parses, and fail on escaped text that plainly does not.
 */
function parsed(markdown: string): HTMLElement {
  const host = document.createElement('div')
  host.innerHTML = renderMarkdown(markdown)
  return host
}

describe('renderMarkdown — injected markup', () => {
  it('does not build an img element from an event-handler tag', () => {
    // `innerHTML` will not run a <script> tag, but it *will* fire an <img>
    // onerror. This is the one that actually executes.
    const host = parsed('<img src=x onerror="alert(1)">')
    expect(host.querySelector('img')).toBeNull()
    expect(host.textContent).toContain('<img src=x onerror="alert(1)">')
  })

  it('does not build a script element', () => {
    const host = parsed('<script>alert(1)</script>')
    expect(host.querySelector('script')).toBeNull()
    expect(host.textContent).toContain('<script>alert(1)</script>')
  })

  it('does not build an element carrying an inline handler', () => {
    const host = parsed('text with <div onclick="x">a div</div> in it')
    expect(host.querySelectorAll('[onclick]')).toHaveLength(0)
    expect(host.textContent).toContain('<div onclick="x">')
  })

  it('refuses every dangerous tag alike — the escape is not tag-specific', () => {
    for (const tag of ['iframe', 'object', 'form', 'svg', 'math', 'style', 'base']) {
      const host = parsed(`<${tag}>x</${tag}>`)
      expect(host.querySelector(tag), tag).toBeNull()
    }
  })

  it('leaves no element with an on* attribute, whatever the markup', () => {
    const host = parsed(
      '<body onload="a"><a href="#" onmouseover="b">x</a><svg/onload="c">',
    )
    for (const el of Array.from(host.querySelectorAll('*'))) {
      for (const attr of Array.from(el.attributes)) {
        expect(attr.name.startsWith('on'), `${el.tagName}[${attr.name}]`).toBe(false)
      }
    }
  })
})

describe('renderMarkdown — URL schemes', () => {
  it('strips the anchor from a javascript: link but keeps the words', () => {
    const html = renderMarkdown('[click me](javascript:alert(1))')
    expect(html).not.toContain('href')
    expect(html).not.toContain('javascript:')
    expect(html).toContain('click me')
  })

  it('catches a javascript: URL split by a control character', () => {
    // The reason the check parses with `URL` instead of matching a prefix: the
    // platform parser strips embedded tabs and newlines before reading the
    // scheme, so this resolves to javascript: rather than to "unrecognised".
    const html = renderMarkdown('[x](java\tscript:alert(1))')
    expect(html).not.toContain('href')
  })

  it('catches a mixed-case javascript: URL', () => {
    expect(isSafeUrl('JaVaScRiPt:alert(1)', ALLOWED_LINK_SCHEMES)).toBe(false)
  })

  it('refuses data: and vbscript: URLs in links', () => {
    expect(isSafeUrl('data:text/html,<script>alert(1)</script>', ALLOWED_LINK_SCHEMES)).toBe(false)
    expect(isSafeUrl('vbscript:msgbox(1)', ALLOWED_LINK_SCHEMES)).toBe(false)
  })

  it('refuses a javascript: image src but keeps the alt text', () => {
    const html = renderMarkdown('![the alt](javascript:alert(1))')
    expect(html).not.toContain('<img')
    expect(html).toContain('the alt')
  })

  it('refuses a data: image src — data:image is still an HTML-parsing surface here', () => {
    expect(isSafeUrl('data:image/svg+xml;base64,AAA', ALLOWED_IMAGE_SCHEMES)).toBe(false)
  })

  it('keeps ordinary http, https and mailto links', () => {
    expect(renderMarkdown('[ok](https://example.com)')).toContain('href="https://example.com"')
    expect(renderMarkdown('[ok](http://example.com)')).toContain('href="http://example.com"')
    expect(renderMarkdown('[mail](mailto:a@b.co)')).toContain('href="mailto:a@b.co"')
  })

  it('keeps an ordinary image', () => {
    const html = renderMarkdown('![a cat](https://example.com/cat.png)')
    expect(html).toContain('<img src="https://example.com/cat.png"')
    expect(html).toContain('alt="a cat"')
  })

  it('refuses a URL past the length ceiling rather than embedding it', () => {
    const long = `https://example.com/${'a'.repeat(MAX_URL_LENGTH)}`
    expect(isSafeUrl(long, ALLOWED_LINK_SCHEMES)).toBe(false)
  })

  it('escapes quotes inside an otherwise allowed URL, so it cannot end the attribute', () => {
    const html = renderMarkdown('[x](https://example.com/"onmouseover="alert(1))')
    expect(html).not.toContain('onmouseover="alert')
  })
})

/**
 * Characterization: the ordinary markdown a chat actually contains must keep
 * rendering the way it does today. These pin the output so the marked 17 -> 18
 * bump (and any future renderer change) has to state what it changed.
 */
describe('renderMarkdown — ordinary content still renders', () => {
  it('renders emphasis, strong and inline code', () => {
    const html = renderMarkdown('plain *em* **bold** and `code`')
    expect(html).toContain('<em>em</em>')
    expect(html).toContain('<strong>bold</strong>')
    expect(html).toContain('<code>code</code>')
  })

  it('renders headings', () => {
    expect(renderMarkdown('## A heading')).toContain('<h2>A heading</h2>')
  })

  it('renders bullet and ordered lists', () => {
    expect(renderMarkdown('- one\n- two')).toContain('<li>one</li>')
    expect(renderMarkdown('1. one\n2. two')).toContain('<ol>')
  })

  it('renders GFM tables', () => {
    const html = renderMarkdown('| a | b |\n| - | - |\n| 1 | 2 |')
    expect(html).toContain('<table>')
    expect(html).toContain('<th>a</th>')
    expect(html).toContain('<td>1</td>')
  })

  it('renders GFM strikethrough and task lists', () => {
    expect(renderMarkdown('~~gone~~')).toContain('<del>gone</del>')
    expect(renderMarkdown('- [x] done')).toContain('type="checkbox"')
  })

  it('honours the breaks option — a single newline is a <br>', () => {
    // Chat messages are written with soft line breaks and are expected to keep
    // them; this is why `breaks: true` is set.
    expect(renderMarkdown('line one\nline two')).toContain('<br>')
  })

  it('renders blockquotes', () => {
    expect(renderMarkdown('> quoted')).toContain('<blockquote>')
  })

  it('escapes ampersands and angle brackets in plain prose', () => {
    const html = renderMarkdown('a < b && c > d')
    expect(html).toContain('&lt;')
    expect(html).toContain('&gt;')
  })

  it('returns something for empty input rather than throwing', () => {
    expect(renderMarkdown('')).toBe('')
  })
})

describe('escapeHtml', () => {
  it('escapes all five characters that can break out of text or an attribute', () => {
    expect(escapeHtml(`&<>"'`)).toBe('&amp;&lt;&gt;&quot;&#39;')
  })

  it('leaves ordinary text untouched', () => {
    expect(escapeHtml('hello world')).toBe('hello world')
  })
})
