import { jsonToMarkdown, markdownToHtml } from '~/lib/tiptapMarkdown'

/**
 * Composer content integrity: the text that leaves the composer must be the
 * text the user typed or pasted. `jsonToMarkdown` is the outbound half — its
 * output IS the message the daemon receives.
 *
 * The regression these cover: Tiptap's Link extension ships `autolink: true`,
 * which rewrites bare text into link marks while it is being entered. A pasted
 * mission line containing `test_01.sh` was turned into
 * `test_[01.sh](http://01.sh)` (".sh" is a live TLD), and the model was asked
 * to run a script that never existed. The editable editor now configures
 * `autolink: false`; these tests pin both sides of that decision.
 */
describe('jsonToMarkdown (composer outbound path)', () => {
  it('round-trips a pasted shell filename unchanged', () => {
    // The document Tiptap produces with autolink disabled: one plain text node.
    const doc = {
      type: 'doc',
      content: [
        { type: 'paragraph', content: [{ type: 'text', text: 'run test_01.sh and report' }] },
      ],
    }
    expect(jsonToMarkdown(doc)).toBe('run test_01.sh and report')
  })

  it('shows the corruption autolink would introduce (why it is disabled)', () => {
    // The document autolink produced: the filename split, with a link mark
    // carrying an invented http:// href.
    const autolinked = {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            { type: 'text', text: 'run test_' },
            {
              type: 'text',
              text: '01.sh',
              marks: [{ type: 'link', attrs: { href: 'http://01.sh' } }],
            },
            { type: 'text', text: ' and report' },
          ],
        },
      ],
    }
    expect(jsonToMarkdown(autolinked)).toBe('run test_[01.sh](http://01.sh) and report')
  })

  it('keeps a link the user actually wrote', () => {
    const doc = {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            { type: 'text', text: 'see ' },
            {
              type: 'text',
              text: 'the docs',
              marks: [{ type: 'link', attrs: { href: 'https://example.com/docs' } }],
            },
          ],
        },
      ],
    }
    expect(jsonToMarkdown(doc)).toBe('see [the docs](https://example.com/docs)')
  })

  it('leaves a path with dots and underscores intact through a full round trip', () => {
    const typed = 'edit src/app_v2.config.ts then run build_01.sh'
    const doc = {
      type: 'doc',
      content: [{ type: 'paragraph', content: [{ type: 'text', text: typed }] }],
    }
    expect(jsonToMarkdown(doc)).toBe(typed)
    // And the inbound half does not invent markup for it either.
    expect(markdownToHtml(typed)).toBe(`<p>${typed}</p>`)
  })

  it('preserves code blocks and their language', () => {
    const doc = {
      type: 'doc',
      content: [
        { type: 'paragraph', content: [{ type: 'text', text: 'run this:' }] },
        {
          type: 'monacoCodeBlock',
          attrs: { language: 'bash', content: './test_01.sh --all' },
        },
      ],
    }
    expect(jsonToMarkdown(doc)).toBe('run this:\n```bash\n./test_01.sh --all\n```')
  })

  it('returns empty string for an empty document', () => {
    expect(jsonToMarkdown({ type: 'doc' })).toBe('')
    expect(jsonToMarkdown(null)).toBe('')
  })
})
