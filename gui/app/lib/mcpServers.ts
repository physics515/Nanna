/**
 * The daemon's per-server MCP state (`system.status` → `mcp_servers`).
 *
 * A configured MCP server that failed to start used to leave nothing in the
 * GUI: its tools were simply absent from the Tools list, and the reason lived
 * only in the daemon's boot log. One parser validates the payload so a
 * malformed entry renders as nothing rather than a broken row.
 */
export type McpServerStateName = 'starting' | 'started' | 'failed' | 'not_started'

export interface McpServerState {
  name: string
  state: McpServerStateName
  tools: number
  detail?: string
  /** How a started server was reached, e.g. `2026-07-28 over Streamable HTTP`. */
  link?: string
}

const STATES: ReadonlySet<string> = new Set(['starting', 'started', 'failed', 'not_started'])

/** Parse the daemon's list, dropping malformed entries. Never throws. */
export function parseMcpServers(payload: unknown): McpServerState[] {
  if (!Array.isArray(payload)) return []
  const servers: McpServerState[] = []
  for (const entry of payload) {
    if (entry === null || typeof entry !== 'object') continue
    const e = entry as Record<string, unknown>
    if (typeof e.name !== 'string' || typeof e.state !== 'string' || !STATES.has(e.state)) continue
    if (typeof e.tools !== 'number' || !Number.isInteger(e.tools) || e.tools < 0) continue
    // A not-started entry may have no name (it failed config validation); its
    // detail is then the only thing that says which entry it was.
    if (e.name === '' && typeof e.detail !== 'string') continue
    servers.push({
      name: e.name,
      state: e.state as McpServerStateName,
      tools: e.tools,
      ...(typeof e.detail === 'string' ? { detail: e.detail } : {}),
      ...(typeof e.link === 'string' && e.link !== '' ? { link: e.link } : {}),
    })
  }
  return servers
}

/** One line a person can read at a glance. Pure. */
export function describeMcpServer(server: McpServerState): string {
  switch (server.state) {
    case 'started': {
      const tools = `${server.tools} tool${server.tools === 1 ? '' : 's'}`
      return server.link ? `${tools} · ${server.link}` : tools
    }
    case 'starting':
      return 'starting…'
    case 'failed':
      return server.detail ? `failed: ${server.detail}` : 'failed'
    case 'not_started':
      return server.detail ?? 'not started'
  }
}
