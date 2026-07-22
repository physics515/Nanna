export type PaletteAction = {
  id: string
  label: string
  group: string
  keywords?: string[]
  shortcut?: string
  href?: string
  action?: string
}

/** Primary IA vs Admin — order within groups is list order. */
export const NAV_ACTIONS: PaletteAction[] = [
  // Primary
  { id: 'nav-chat', label: 'Chat', group: 'Primary', keywords: ['home', 'messages', 'conversation'], href: '/', shortcut: '⌘1' },
  { id: 'nav-tasks', label: 'Tasks', group: 'Primary', keywords: ['todo', 'work'], href: '/tasks' },
  { id: 'nav-memory', label: 'Memory', group: 'Primary', keywords: ['memories', 'knowledge', 'cards'], href: '/memory' },
  { id: 'nav-tools', label: 'Tools', group: 'Primary', keywords: ['mcp', 'functions'], href: '/tools' },
  { id: 'nav-channels', label: 'Channels', group: 'Primary', keywords: ['integrations', 'telegram', 'discord'], href: '/channels' },
  { id: 'nav-settings', label: 'Settings', group: 'Primary', keywords: ['preferences', 'config'], href: '/settings', shortcut: '⌘,' },
  // Admin
  { id: 'nav-logs', label: 'Logs', group: 'Admin', keywords: ['diagnostics', 'console', 'output'], href: '/logs' },
  { id: 'nav-workspaces', label: 'Workspaces', group: 'Admin', keywords: ['projects', 'folders'], href: '/workspaces' },
  { id: 'nav-agents', label: 'Agents', group: 'Admin', keywords: ['personas', 'bots'], href: '/agents' },
  { id: 'nav-scheduler', label: 'Scheduler', group: 'Admin', keywords: ['cron', 'jobs', 'schedule'], href: '/scheduler' },
  { id: 'nav-model-stats', label: 'Model Stats', group: 'Admin', keywords: ['usage', 'tokens', 'llm'], href: '/model-stats' },
  { id: 'nav-tool-stats', label: 'Tool Stats', group: 'Admin', keywords: ['usage', 'mcp'], href: '/tool-stats' },
]

export const QUICK_ACTIONS: PaletteAction[] = [
  { id: 'new-chat', label: 'New chat', group: 'Actions', keywords: ['create', 'session', 'conversation'], action: 'new-chat', shortcut: '⌘N' },
  { id: 'toggle-live-logs', label: 'Toggle live logs', group: 'Actions', keywords: ['pause', 'poll', 'stream'], action: 'toggle-live-logs' },
  { id: 'focus-input', label: 'Focus chat input', group: 'Actions', keywords: ['compose', 'message', 'type'], action: 'focus-input' },
  { id: 'stop-generation', label: 'Stop generation', group: 'Actions', keywords: ['cancel', 'abort', 'halt'], action: 'stop-generation', shortcut: '⌘.' },
  { id: 'open-settings-models', label: 'Open model settings', group: 'Actions', keywords: ['llm', 'provider', 'api'], action: 'open-settings-models' },
  { id: 'toggle-compact-mode', label: 'Toggle compact mode', group: 'Actions', keywords: ['density', 'ui', 'spacing'], action: 'toggle-compact-mode' },
  { id: 'toggle-chat-panel', label: 'Toggle chat panel', group: 'Actions', keywords: ['sidebar', 'sessions', 'drawer'], action: 'toggle-chat-panel' },
]

const RESULT_LIMIT = 50

/** Case-insensitive match on label, group, keywords, id. Empty query → all (order preserved). */
export function filterActions(actions: PaletteAction[], query: string): PaletteAction[] {
  const q = query.trim().toLowerCase()
  if (!q) return actions.slice(0, RESULT_LIMIT)

  const scored: Array<{ action: PaletteAction; score: number }> = []
  for (const action of actions) {
    const label = action.label.toLowerCase()
    const group = action.group.toLowerCase()
    const id = action.id.toLowerCase()
    const kw = (action.keywords ?? []).map((k) => k.toLowerCase())

    let score = -1
    if (label === q) score = 100
    else if (label.startsWith(q)) score = 80
    else if (label.includes(q)) score = 60
    else if (id === q || id.includes(q)) score = 50
    else if (group.includes(q)) score = 30
    else if (kw.some((k) => k === q || k.startsWith(q) || k.includes(q))) score = 40

    if (score >= 0) scored.push({ action, score })
  }

  scored.sort((a, b) => b.score - a.score)
  return scored.slice(0, RESULT_LIMIT).map((s) => s.action)
}
