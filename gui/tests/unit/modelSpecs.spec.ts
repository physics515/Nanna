import { modelDisplayName } from '~/lib/modelSpecs'

describe('modelDisplayName', () => {
  it('drops the routing prefix', () => {
    expect(modelDisplayName('ollama/qwen3:14b')).toBe('qwen3:14b')
  })

  // The dated ids say nothing at a glance, and the date is the only part that
  // differs between two releases of the same model.
  it('trades a known dated id for its human name', () => {
    expect(modelDisplayName('claude-opus-4-5-20251101')).toBe('Opus 4.5')
    expect(modelDisplayName('ollama/gpt-4o-mini')).toBe('GPT-4o Mini')
  })

  // The honest answer for a model nobody has named is its id — a lookup miss
  // must not shorten anything the user could not put back.
  it('leaves an unknown bare model name alone', () => {
    expect(modelDisplayName('claude-opus-9-20991231')).toBe('claude-opus-9-20991231')
  })

  // OpenRouter ids are themselves `vendor/model`, and the vendor is the half
  // that tells two similarly-named models apart — only the router prefix goes.
  it('keeps the vendor segment of a nested spec', () => {
    expect(modelDisplayName('openrouter/anthropic/claude-sonnet-4')).toBe('anthropic/claude-sonnet-4')
  })

  // One home for the rule means the header cannot render the same id two ways
  // — the badge's `openrouter/anthropic/...` and the picker's must agree.
  it('gives a nested and a prefixed spec for one model the same name', () => {
    expect(modelDisplayName('openrouter/anthropic/claude-opus-4-5-20251101'))
      .toBe(modelDisplayName('claude-opus-4-5-20251101'))
  })
})
