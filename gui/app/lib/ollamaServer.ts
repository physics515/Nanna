/**
 * Which Ollama server an address names, for what Settings and onboarding SAY
 * about the saved bearer token.
 *
 * Where the token actually goes is decided in Rust (`same_ollama_server` in
 * nanna-config): the daemon loads it only for the server it was saved for.
 * This mirrors that rule so a page can tell the user, before anything is
 * saved, that the token will not be sent to the address in the field.
 */

/** `localhost`, the IPv4 loopback block, or `::1` — this machine. */
function isLoopbackHostname(hostname: string): boolean {
  const host = hostname.replace(/^\[|\]$/g, '').toLowerCase()
  return host === 'localhost' || host === '::1' || /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host)
}

interface ServerIdentity {
  protocol: string
  host: string
  port: string
  path: string
}

function identify(url: string): ServerIdentity | null {
  let parsed: URL
  try {
    parsed = new URL(url.trim())
  } catch {
    return null
  }
  if (!parsed.hostname) return null
  return {
    protocol: parsed.protocol,
    host: isLoopbackHostname(parsed.hostname) ? 'this machine' : parsed.hostname.toLowerCase(),
    // `URL` already reads a scheme's default port as no port.
    port: parsed.port,
    path: parsed.pathname.replace(/\/+$/, ''),
  }
}

/**
 * Whether two Ollama base URLs name the same server: scheme, host (case and
 * loopback spellings aside), port (a scheme's default written or not) and
 * path without trailing slashes. An address that is not a URL matches none.
 */
export function sameOllamaServer(a: string, b: string): boolean {
  const left = identify(a)
  const right = identify(b)
  if (!left || !right) return false
  return (
    left.protocol === right.protocol &&
    left.host === right.host &&
    left.port === right.port &&
    left.path === right.path
  )
}

/** Whether an Ollama base URL points at this machine. */
export function isLoopbackUrl(url: string): boolean {
  try {
    return isLoopbackHostname(new URL(url.trim()).hostname)
  } catch {
    return false
  }
}

/**
 * Whether a bearer token sent to `url` would cross the network unencrypted:
 * plain `http://` to another machine.
 */
export function sendsTokenInClear(url: string): boolean {
  try {
    const parsed = new URL(url.trim())
    return parsed.protocol === 'http:' && !isLoopbackHostname(parsed.hostname)
  } catch {
    return false
  }
}
