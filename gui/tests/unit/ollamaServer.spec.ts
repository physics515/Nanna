import { describe, expect, it } from 'vitest'
import { isLoopbackUrl, sameOllamaServer, sendsTokenInClear } from '~/lib/ollamaServer'

/**
 * The page's copy of the rule `nanna_config::same_ollama_server` enforces in
 * Rust — the same cases, so what Settings says about where the token goes
 * matches where the daemon sends it.
 */
describe('sameOllamaServer', () => {
  it.each([
    ['https://Host/ollama/', 'https://host/ollama'],
    ['http://localhost:11434', ' http://127.0.0.1:11434/ '],
    ['http://[::1]:11434', 'http://localhost:11434'],
    ['https://host:443/ollama', 'https://host/ollama'],
    ['http://host:80', 'http://host/'],
    ['https://user:pw@host/ollama', 'https://host/ollama'],
  ])('%s and %s are one server', (a, b) => {
    expect(sameOllamaServer(a, b)).toBe(true)
    expect(sameOllamaServer(b, a)).toBe(true)
  })

  it.each([
    ['https://host/ollama', 'https://host/api'],
    ['https://evil.example', 'https://host/ollama'],
    ['http://host/ollama', 'https://host/ollama'],
    ['https://host:8443/ollama', 'https://host/ollama'],
    ['http://localhost:11434', 'http://localhost:11435'],
    ['https://host/Ollama', 'https://host/ollama'],
    ['http://localhost:11434', 'http://gpu-box:11434'],
  ])('%s and %s are different servers', (a, b) => {
    expect(sameOllamaServer(a, b)).toBe(false)
  })

  it('an address that is not a URL matches nothing', () => {
    expect(sameOllamaServer('localhost:11434', 'localhost:11434')).toBe(false)
    expect(sameOllamaServer('', '')).toBe(false)
  })
})

describe('where a token would travel in the clear', () => {
  it('plain http to another machine does', () => {
    expect(sendsTokenInClear('http://gpu-box:11434')).toBe(true)
    expect(sendsTokenInClear('http://192.168.1.20:11434/ollama')).toBe(true)
  })

  it('https, or this machine, does not', () => {
    expect(sendsTokenInClear('https://gpu-box/ollama')).toBe(false)
    expect(sendsTokenInClear('http://localhost:11434')).toBe(false)
    expect(sendsTokenInClear('http://127.0.0.1:11434')).toBe(false)
    expect(sendsTokenInClear('http://[::1]:11434')).toBe(false)
    expect(sendsTokenInClear('not a url')).toBe(false)
  })

  it('reads loopback spellings as this machine', () => {
    expect(isLoopbackUrl('http://LOCALHOST:11434')).toBe(true)
    expect(isLoopbackUrl('http://127.0.0.2:11434')).toBe(true)
    expect(isLoopbackUrl('https://host/ollama')).toBe(false)
  })
})
