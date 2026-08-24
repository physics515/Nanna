import { inlineMd, markdownToHtml } from '~/lib/tiptapMarkdown'

/**
 * Characterization snapshot for the inbound markdown → HTML path.
 *
 * `tiptapMarkdown.ts` carries 26 of the GUI's 96 `vue-tsc --build` errors, all
 * of the `noUncheckedIndexedAccess` family (`lines[i]` is `string | undefined`
 * even under an `i < lines.length` guard). Fixing them means touching every
 * branch of the parser — and this file's own header says it plainly: "this is
 * the function whose output becomes the message the daemon receives, so a
 * corruption here is a corruption of what the user actually said."
 *
 * Six tests were not enough cover to refactor behind. These pin the *current*
 * output for every branch and every edge the parser has, so the type fix can be
 * proven behaviour-preserving rather than assumed to be. They were written and
 * run green BEFORE that change, which is the only order in which a
 * characterization test means anything.
 *
 * If a future change intends to alter output, update the expectation and say so
 * in the commit — that is the point of pinning it, not an obstacle to it.
 */

// Every branch the parser has, plus the edges that decide whether an index read
// can go out of range: unterminated fences, one-line inputs, trailing markers.
const CORPUS: Array<[name: string, input: string]> = [
  ['empty', ''],
  ['plain', 'plain text'],
  ['multiline plain', 'a\nb\nc'],

  ['h1', '# H1'],
  ['h2', '## H2'],
  ['h6', '###### H6'],
  ['hash without space is not a heading', '#no-space'],
  ['seven hashes is not a heading', '####### too many'],

  ['fenced, no language', '```\ncode\n```'],
  ['fenced with language', '```js\nconst a = 1\n```'],
  ['fence never closed', '```unclosed\nstill code'],
  ['fence is the last line', 'text\n```'],
  ['empty fence', '```\n```'],

  ['blockquote', '> quoted'],
  ['blockquote, two lines', '> line one\n> line two'],
  ['angle bracket without space', '>no space'],
  ['blockquote is the last line', 'text\n> q'],

  ['task unchecked', '- [ ] todo'],
  ['task checked', '- [x] done'],
  ['task list', '- [ ] a\n- [x] b'],

  ['bullet', '- bullet'],
  ['bullet list', '- a\n- b\n- c'],

  ['ordered', '1. one'],
  ['ordered list', '1. one\n2. two'],

  ['bold', '**bold**'],
  ['italic', '*ital*'],
  ['inline code', '`code`'],
  ['link', '[link](http://x)'],
  ['mixed inline', '**bold** and `code` and [l](u)'],

  ['blank lines', 'text\n\n\nmore'],
  ['everything at once', '# H\n\n- a\n- b\n\n```py\nx=1\n```\n\n> q'],

  ['trailing whitespace', 'trailing spaces   '],
  ['leading whitespace', '   leading'],
  ['html-significant characters', 'quote " and & and < >'],
  ['multi-byte', 'emoji 🎉 and em—dash and ünïcode'],
  ['single newline', '\n'],
  ['only newlines', '\n\n\n'],
]

describe('markdownToHtml (characterization — pins current output)', () => {
  for (const [name, input] of CORPUS) {
    it(`is stable for: ${name}`, () => {
      expect(markdownToHtml(input)).toMatchSnapshot()
    })
  }

  it('never throws on any corpus input', () => {
    // The index reads this parser makes are guarded by `i < lines.length`, so
    // none should be able to run off the end. A throw here would mean one can.
    for (const [name, input] of CORPUS) {
      expect(() => markdownToHtml(input), name).not.toThrow()
    }
  })

  it('never emits the string "undefined"', () => {
    // The failure mode a `?? ''` fix could introduce, and the one a `!`
    // assertion would let through at runtime: an out-of-range read
    // stringified into the user's message.
    for (const [name, input] of CORPUS) {
      expect(markdownToHtml(input), name).not.toContain('undefined')
    }
  })
})

describe('inlineMd (characterization — pins current output)', () => {
  const INLINE: Array<[name: string, input: string]> = [
    ['empty', ''],
    ['bold', '**b**'],
    ['italic', '*i*'],
    ['code', '`c`'],
    ['link', '[t](u)'],
    ['unclosed bold', '**never closed'],
    ['unclosed code', '`never closed'],
    ['bracket without paren', '[text]'],
    ['nested-looking', '**bold with `code`**'],
    ['html-significant characters', '<script>&"'],
  ]

  for (const [name, input] of INLINE) {
    it(`is stable for: ${name}`, () => {
      expect(inlineMd(input)).toMatchSnapshot()
    })
  }
})
