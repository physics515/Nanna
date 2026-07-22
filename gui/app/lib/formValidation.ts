/**
 * Lightweight field validators for settings / channel wizards / API keys.
 * Keep failures inline; never clear sibling fields on partial failure.
 */

export type FieldError = string | null

export type ValidateResult<T = string> =
  | { ok: true; value: T }
  | { ok: false; error: string }

export function required(value: unknown, label = 'This field'): FieldError {
  if (value == null) return label + ' is required'
  if (typeof value === 'string' && value.trim() === '') return label + ' is required'
  return null
}

/** Anthropic/OpenAI-style keys: sk-... or plain non-empty secret. */
export function apiKey(value: string, provider = 'API'): FieldError {
  const v = (value ?? '').trim()
  if (!v) return provider + ' key is required'
  if (v.length < 8) return provider + ' key looks too short'
  if (/\s/.test(v)) return provider + ' key must not contain spaces'
  return null
}

/** Result-object wrapper for ApiKeyInput — prefer over bare FieldError when wiring forms. */
export function validateApiKey(value: string, provider = 'API'): ValidateResult {
  const err = apiKey(value, provider)
  if (err) return { ok: false, error: err }
  return { ok: true, value: (value ?? '').trim() }
}

export function validateRequired(value: unknown, label = 'This field'): ValidateResult {
  const err = required(value, label)
  if (err) return { ok: false, error: err }
  return { ok: true, value: typeof value === 'string' ? value.trim() : String(value ?? '') }
}

export function validatePort(value: unknown): ValidateResult<number> {
  const n = typeof value === 'number' ? value : Number(String(value ?? '').trim())
  if (!Number.isFinite(n) || !Number.isInteger(n) || n < 1 || n > 65535) {
    return { ok: false, error: 'Port must be an integer from 1 to 65535' }
  }
  return { ok: true, value: n }
}

export function url(value: string, label = 'URL'): FieldError {
  const v = (value ?? '').trim()
  if (!v) return label + ' is required'
  try {
    const u = new URL(v)
    if (!u.protocol.startsWith('http')) return label + ' must be http(s)'
    return null
  } catch {
    return label + ' is not a valid URL'
  }
}

export function nonNegativeInt(value: unknown, label = 'Value'): FieldError {
  const n = typeof value === 'number' ? value : Number(value)
  if (!Number.isFinite(n) || !Number.isInteger(n) || n < 0) {
    return label + ' must be a non-negative integer'
  }
  return null
}

/** Merge field errors; first error wins per key. */
export function collectErrors(entries: Record<string, FieldError>): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [k, v] of Object.entries(entries)) {
    if (v) out[k] = v
  }
  return out
}

export function hasErrors(errors: Record<string, string | null | undefined>): boolean {
  return Object.values(errors).some((e) => !!e)
}
