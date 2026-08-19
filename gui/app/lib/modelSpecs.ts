/**
 * Rendering for router model specs — `ollama/qwen3:14b`,
 * `openrouter/anthropic/claude-…`, or a bare `claude-…`. This is the same
 * string shape `llm.model_priority` holds and the daemon's router consumes,
 * so the spec itself is stored and sent verbatim; only its label is shortened.
 *
 * This is the ONE home for the label rule. The chat header shows the global
 * model badge and this chat's pin side by side, and two rules meant the same
 * id could render two ways in the same row — which reads as two models.
 */

/**
 * Ids that say nothing at a glance get a human name. Anthropic and OpenAI
 * date-stamp theirs, and the date is the only part that differs between two
 * releases of the same model. Everything else — every Ollama tag, every model
 * released after this list was written — keeps its id, which is the honest
 * answer when there is nothing shorter to say.
 */
const FRIENDLY_NAMES: Record<string, string> = {
  'claude-opus-4-5-20251101': 'Opus 4.5',
  'claude-opus-4-20250514': 'Opus 4',
  'claude-sonnet-4-20250514': 'Sonnet 4',
  'claude-3-5-sonnet-20241022': 'Sonnet 3.5',
  'claude-3-5-haiku-20241022': 'Haiku 3.5',
  'gpt-4o': 'GPT-4o',
  'gpt-4o-mini': 'GPT-4o Mini',
  'gpt-4-turbo': 'GPT-4 Turbo',
}

/**
 * The label for a model spec: its routing prefix dropped, and a known id
 * traded for its human name. A chat header has room for the model, not the
 * route, and the full spec stays one hover away in every title that uses
 * this — so the label is shortened without ever hiding which model is in play.
 */
export function modelDisplayName(spec: string): string {
  const slash = spec.indexOf('/')
  const routeless = slash === -1 ? spec : spec.slice(slash + 1)
  // A friendly name is keyed by the bare model id, which for a nested
  // OpenRouter spec (`openrouter/anthropic/claude-…`) is the last segment.
  // An unnamed spec keeps its vendor, because the vendor is what tells two
  // similarly-named models apart.
  const bare = routeless.slice(routeless.lastIndexOf('/') + 1)
  return FRIENDLY_NAMES[bare] ?? routeless
}
