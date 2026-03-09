<template>
  <div class="app-shell" style="display: flex; height: 100vh; overflow: hidden;">

    <!-- ═══ Activity Bar (icon-only sidebar) ═══ -->
    <aside class="activity-bar">
      <!-- Logo -->
      <div class="activity-logo" data-tauri-drag-region>
        <img src="/logo.svg" alt="Nanna" style="width: 46px; height: 46px; object-fit: contain; pointer-events: none;" />
      </div>

      <!-- Navigation icons -->
      <nav class="activity-nav">
        <!-- Chat (toggles session panel) -->
        <button
          :class="['activity-icon', { active: chatPanelOpen || route.path === '/' }]"
          @click="toggleChatPanel"
        >
          <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5">
            <path d="M4 4h12a2 2 0 012 2v7a2 2 0 01-2 2H8l-4 3v-3a2 2 0 01-2-2V6a2 2 0 012-2z" />
          </svg>
          <svg
            class="chat-arrow"
            :class="{ 'chat-arrow--open': chatPanelOpen }"
            viewBox="0 0 6 10"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M1 1l4 4-4 4" />
          </svg>
          <span class="tooltip">Chats</span>
        </button>

        <NuxtLink
          v-for="item in navItems" :key="item.to" :to="item.to"
          :class="['activity-icon', { active: isNavActive(item.to) }]"
          @click="chatPanelOpen = false"
        >
          <component :is="item.icon" />
          <span class="tooltip">{{ item.label }}</span>
        </NuxtLink>
      </nav>

      <!-- Bottom: settings + hide -->
      <div class="activity-bottom">
        <NuxtLink to="/settings" :class="['activity-icon', { active: route.path === '/settings' }]" @click="chatPanelOpen = false">
          <Settings />
          <span class="tooltip">Settings</span>
        </NuxtLink>
        <button class="activity-icon" @click="hideToTray">
          <ChevronDown />
          <span class="tooltip">Hide to Tray</span>
        </button>
      </div>
    </aside>

    <!-- ═══ Right column: chat panel + main + status bar ═══ -->
    <div style="flex: 1; display: flex; flex-direction: column; min-height: 0; min-width: 0;">

      <!-- Top row: chat panel + main content (resizable) -->
      <UiResizableGroup direction="horizontal" style="flex: 1; min-height: 0;">

        <!-- Chat Panel -->
        <UiResizablePanel
          v-if="chatPanelOpen"
          :default-size="18"
          :min-size="12"
          :max-size="35"
          :order="1"
          class="chat-panel"
        >
          <!-- Workspace Switcher -->
          <WorkspaceSwitcher />

          <!-- Header -->
          <div style="padding: 0.25rem 0.75rem; display: flex; align-items: center; justify-content: space-between;">
            <span style="font-size: 0.7rem; font-weight: 500; color: rgba(196,205,214,0.5); text-transform: uppercase; letter-spacing: 0.06em;">Chats</span>
            <button class="panel-icon-btn" @click="createNewSession" title="New chat">
              <Plus style="width: 14px; height: 14px;" />
            </button>
          </div>

          <!-- Session list -->
          <nav style="flex: 1; overflow-y: auto; min-height: 0; padding: 0 0.375rem;">
            <SessionItem
              v-for="session in sessions"
              :key="session.id"
              :session="session"
              :is-active="currentSessionId === session.id"
              @select="(s) => { switchSession(s); }"
              @deleted="onSessionDeleted"
              @renamed="onSessionRenamed"
            />
            <div v-if="sessions.length === 0" style="font-size: 0.7rem; color: rgba(100,116,139,0.5); padding: 2rem 0.5rem; text-align: center;">
              No chats yet
            </div>
          </nav>
        </UiResizablePanel>

        <UiResizableHandle v-if="chatPanelOpen" />

        <!-- Main content column -->
        <UiResizablePanel :default-size="chatPanelOpen ? 82 : 100" :order="2" style="display: flex; flex-direction: column; min-height: 0; min-width: 0;">
          <TitleBar />
          <main style="flex: 1; overflow: hidden;">
            <slot />
          </main>
        </UiResizablePanel>

      </UiResizableGroup>

      <!-- ═══ Bottom Status Bar (full width except activity bar) ═══ -->
      <div class="status-bar">
        <div class="status-left">
          <span class="status-version">v0.1.0</span>
          <span v-if="backendStatus" :class="['status-badge', isDaemon ? 'status-badge-accent' : '']">
            {{ isDaemon ? 'daemon' : 'embedded' }}
          </span>
        </div>
        <div class="status-right">
          <span :class="['status-dot', backendStatus?.connected ? 'dot-ok' : (apiKeySet ? 'dot-warn' : 'dot-err')]"></span>
          <span class="status-label">
            {{ backendStatus?.connected ? 'Connected' : (apiKeySet ? 'Disconnected' : 'No API Key') }}
          </span>
        </div>
      </div>

    </div>

    <!-- Workspace Picker Modal -->
    <WorkspacePicker
      v-if="showWorkspacePicker"
      v-model="showWorkspacePicker"
      :open-tab-ids="openTabIds"
      @select="openWorkspaceTab"
    />

    <!-- Close confirmation dialog -->
    <CloseDialog />

    <!-- Global confirmation dialog -->
    <ConfirmDialog />
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, provide } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Plus, Brain, Radio, Settings, ChevronDown, FolderKanban, Bot, Wrench, Clock, FileText, BarChart3 } from 'lucide-vue-next'

const navItems = [
  { to: '/memory', label: 'Memory', icon: Brain },
  { to: '/logs', label: 'Logs', icon: FileText },
  { to: '/workspaces', label: 'Workspaces', icon: FolderKanban },
  { to: '/agents', label: 'Agents', icon: Bot },
  { to: '/channels', label: 'Channels', icon: Radio },
  { to: '/tools', label: 'Tools', icon: Wrench },
  { to: '/scheduler', label: 'Scheduler', icon: Clock },
  { to: '/model-stats', label: 'Model Stats', icon: BarChart3 },
]

interface SessionInfo {
  id: string
  name: string
  created_at: string
  updated_at: string
  message_count: number
  workspace_id: string | null
  workspace_name: string | null
}

interface WorkspaceInfo {
  id: string
  name: string
  path: string
  active?: boolean
}

interface Tab {
  type: 'global' | 'workspace'
  workspaceId?: string
}

interface AppConfig {
  theme: string
  model: string
  api_key_set: boolean
  available_models: string[]
}

const route = useRoute()

const sessions = ref<SessionInfo[]>([])
const currentSessionId = ref<string | null>(null)
const apiKeySet = ref(false)
const showWorkspacePicker = ref(false)
const chatPanelOpen = ref(false)

const openWorkspaces = ref<WorkspaceInfo[]>([])
const currentTab = ref<Tab>({ type: 'global' })

let unlistenTrayNewChat: UnlistenFn | null = null
let unlistenCloseRequested: UnlistenFn | null = null
let unlistenSessionsCleared: UnlistenFn | null = null
let unlistenSessionRenamed: UnlistenFn | null = null

function isNavActive(path: string) {
  return route.path === path || (path !== '/' && route.path.startsWith(path))
}

function toggleChatPanel() {
  chatPanelOpen.value = !chatPanelOpen.value
  if (chatPanelOpen.value && route.path !== '/') {
    navigateTo('/')
  }
}

const openTabIds = computed(() => openWorkspaces.value.map(w => w.id))

const activeWorkspace = computed(() => {
  if (currentTab.value?.type === 'workspace') {
    return openWorkspaces.value.find(w => w.id === currentTab.value.workspaceId) || null
  }
  return null
})

function addWorkspaceTab(ws: WorkspaceInfo) {
  if (!openWorkspaces.value.some(w => w.id === ws.id)) {
    openWorkspaces.value.push(ws)
    saveTabsToStorage()
  }
}

function selectWorkspaceTab(workspaceId: string) {
  const ws = openWorkspaces.value.find(w => w.id === workspaceId)
  if (!ws) {
    loadOpenWorkspaces().then(() => {
      const found = openWorkspaces.value.find(w => w.id === workspaceId)
      if (found) currentTab.value = { type: 'workspace', workspaceId }
    })
  } else {
    currentTab.value = { type: 'workspace', workspaceId }
  }
}

function selectGlobalTab() {
  currentTab.value = { type: 'global' }
}

provide('currentSessionId', currentSessionId)
provide('sessions', sessions)
provide('activeWorkspace', activeWorkspace)
provide('currentTab', currentTab)
provide('openWorkspaces', openWorkspaces)
provide('addWorkspaceTab', addWorkspaceTab)
provide('selectWorkspaceTab', selectWorkspaceTab)
provide('selectGlobalTab', selectGlobalTab)
provide('selectTab', selectTab)
provide('closeWorkspaceTab', closeWorkspaceTab)
provide('showWorkspacePicker', showWorkspacePicker)

const { checkPermission } = useNotifications()
const { init: initBackend, status: backendStatus, isDaemon } = useBackend()
const { handleClose, loadCloseMode } = useCloseHandler()

const TABS_STORAGE_KEY = 'nanna-workspace-tabs'
const CURRENT_TAB_KEY = 'nanna-current-tab'

onMounted(async () => {
  const mode = await initBackend()
  console.log(`Nanna running in ${mode} mode`)
  loadTabsFromStorage()
  await loadOpenWorkspaces()
  await loadSessions()
  await loadConfig()

  const urlSessionId = route.query.session as string | undefined
  if (urlSessionId && sessions.value.some(s => s.id === urlSessionId)) {
    currentSessionId.value = urlSessionId
  }

  unlistenTrayNewChat = await listen('tray-new-chat', () => createNewSession())
  unlistenSessionsCleared = await listen('sessions-cleared', async () => {
    await loadSessions()
    currentSessionId.value = sessions.value[0]?.id || null
  })
  unlistenSessionRenamed = await listen<{ id: string, name: string }>('session-renamed', (event) => {
    const { id, name } = event.payload
    const idx = sessions.value.findIndex(s => s.id === id)
    if (idx !== -1) sessions.value[idx] = { ...sessions.value[idx], name }
  })

  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  const window = getCurrentWindow()
  unlistenCloseRequested = await window.onCloseRequested(async (event) => {
    event.preventDefault()
    await handleClose()
  })

  await loadCloseMode()
  await checkPermission()
})

onUnmounted(() => {
  unlistenTrayNewChat?.()
  unlistenCloseRequested?.()
  unlistenSessionsCleared?.()
  unlistenSessionRenamed?.()
})

watch(() => route.query.session, (newSessionId) => {
  if (typeof newSessionId === 'string' && sessions.value.some(s => s.id === newSessionId)) {
    currentSessionId.value = newSessionId
  }
})

// Close chat panel when navigating away from chat
watch(() => route.path, (path) => {
  if (path !== '/' && path !== '') chatPanelOpen.value = false
})

watch(currentTab, async () => {
  await loadSessions()
  currentSessionId.value = sessions.value[0]?.id || null
  if (currentSessionId.value) navigateTo(`/?session=${currentSessionId.value}`)
  saveTabsToStorage()
}, { deep: true })

function loadTabsFromStorage() {
  try {
    const savedTabs = localStorage.getItem(TABS_STORAGE_KEY)
    const savedCurrent = localStorage.getItem(CURRENT_TAB_KEY)
    if (savedTabs) {
      const tabIds: string[] = JSON.parse(savedTabs)
      openWorkspaces.value = tabIds.map(id => ({ id, name: '', path: '' }))
    }
    if (savedCurrent) currentTab.value = JSON.parse(savedCurrent)
  } catch (e) { console.error('Failed to load tabs from storage:', e) }
}

function saveTabsToStorage() {
  try {
    localStorage.setItem(TABS_STORAGE_KEY, JSON.stringify(openWorkspaces.value.map(w => w.id)))
    localStorage.setItem(CURRENT_TAB_KEY, JSON.stringify(currentTab.value))
  } catch (e) { console.error('Failed to save tabs to storage:', e) }
}

async function loadOpenWorkspaces() {
  try {
    const allWorkspaces = await invoke<WorkspaceInfo[]>('list_workspaces')
    const savedIds = openWorkspaces.value.map(w => w.id)
    openWorkspaces.value = allWorkspaces.filter(ws => savedIds.includes(ws.id))
    if (currentTab.value?.type === 'workspace') {
      if (!openWorkspaces.value.some(w => w.id === currentTab.value.workspaceId)) {
        currentTab.value = { type: 'global' }
      }
    }
    saveTabsToStorage()
  } catch (e) { console.error('Failed to load workspaces:', e); openWorkspaces.value = [] }
}

async function loadSessions() {
  try {
    const workspaceId = currentTab.value?.type === 'workspace' ? currentTab.value.workspaceId ?? null : null
    sessions.value = await invoke<SessionInfo[]>('list_sessions', { workspaceId })
    if (sessions.value[0] && !currentSessionId.value) currentSessionId.value = sessions.value[0].id
  } catch (e) { console.error('Failed to load sessions:', e) }
}

async function loadConfig() {
  try {
    const config = await invoke<AppConfig>('get_config')
    apiKeySet.value = config.api_key_set
  } catch (e) { console.error('Failed to load config:', e) }
}

function selectTab(tab: Tab) { currentTab.value = tab }

function openWorkspaceTab(ws: WorkspaceInfo) {
  if (!openWorkspaces.value.some(w => w.id === ws.id)) openWorkspaces.value.push(ws)
  currentTab.value = { type: 'workspace', workspaceId: ws.id }
  saveTabsToStorage()
}

function closeWorkspaceTab(workspaceId: string) {
  openWorkspaces.value = openWorkspaces.value.filter(w => w.id !== workspaceId)
  if (currentTab.value?.type === 'workspace' && currentTab.value.workspaceId === workspaceId) {
    currentTab.value = { type: 'global' }
  }
  saveTabsToStorage()
}

async function createNewSession() {
  try {
    const workspaceId = currentTab.value?.type === 'workspace' ? currentTab.value.workspaceId ?? null : null
    const session = await invoke<SessionInfo>('create_session', { name: null, workspaceId })
    currentSessionId.value = session.id
    await loadSessions()
    navigateTo(`/?session=${session.id}`)
  } catch (e) { console.error('Failed to create session:', e) }
}

function switchSession(session: SessionInfo) {
  currentSessionId.value = session.id
  navigateTo(`/?session=${session.id}`)
}

function onSessionDeleted(sessionId: string) {
  sessions.value = sessions.value.filter(s => s.id !== sessionId)
  if (currentSessionId.value === sessionId) {
    currentSessionId.value = sessions.value[0]?.id || null
    if (currentSessionId.value) navigateTo(`/?session=${currentSessionId.value}`)
  }
}

function onSessionRenamed(updated: SessionInfo) {
  const idx = sessions.value.findIndex(s => s.id === updated.id)
  if (idx !== -1) sessions.value[idx] = updated
}

async function hideToTray() {
  try { await invoke('hide_to_tray') } catch (e) { console.error('Failed to hide to tray:', e) }
}
</script>

<style scoped>
/* ═══ Activity Bar ═══ */
.activity-bar {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 64px;
  /* no border, no background — inherits shell gradient */
}

.activity-logo {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 52px;
  height: 52px;
  margin-top: 8px;
  margin-bottom: 8px;
}

.activity-nav {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  width: 100%;
  padding: 0 8px;
}

.activity-icon {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 40px;
  border-radius: 8px;
  color: rgba(196, 205, 214, 0.4);
  background: transparent;
  border: none;
  cursor: pointer;
  transition: color 0.15s, background 0.15s;
  text-decoration: none;
}
.activity-icon svg,
.activity-icon :deep(svg) {
  width: 20px;
  height: 20px;
}
.activity-icon:hover {
  color: #c4cdd6;
  background: rgba(255, 255, 255, 0.04);
}
.activity-icon.active {
  color: #e2e8f0;
}
/* Active indicator bar */
.activity-icon.active::before {
  content: '';
  position: absolute;
  left: 0;
  top: 6px;
  bottom: 6px;
  width: 2px;
  background: #8b5cf6;
  border-radius: 0 2px 2px 0;
}

/* Chat drawer arrow */
.chat-arrow {
  position: absolute;
  right: 4px;
  top: 50%;
  transform: translateY(-50%);
  width: 4px !important;
  height: 7px !important;
  opacity: 0.35;
  transition: transform 0.2s ease, opacity 0.2s ease;
}
.activity-icon:hover .chat-arrow {
  opacity: 0.7;
}
.chat-arrow--open {
  transform: translateY(-50%) rotate(180deg);
}

.activity-bottom {
  margin-top: auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  width: 100%;
  padding: 0 8px 12px;
}

/* Tooltip */
.tooltip {
  position: absolute;
  left: 100%;
  top: 50%;
  transform: translateY(-50%);
  margin-left: 8px;
  padding: 4px 10px;
  background: #1a1a2e;
  color: #e2e8f0;
  font-size: 12px;
  white-space: nowrap;
  border-radius: 4px;
  pointer-events: none;
  opacity: 0;
  transition: opacity 0.15s ease;
  z-index: 100;
}
.activity-icon:hover .tooltip {
  opacity: 1;
}

/* ═══ Chat Panel (secondary slide-out) ═══ */
.chat-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  /* borderless — no background, no border, inherits shell gradient */
}

.panel-icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: 6px;
  border: none;
  background: transparent;
  color: #c4cdd6;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}
.panel-icon-btn:hover {
  background: rgba(255, 255, 255, 0.06);
  color: #e2e8f0;
}

/* ═══ Bottom Status Bar ═══ */
.status-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 16px;
  height: 28px;
  flex-shrink: 0;
  border: none;
  font-size: 11px;
  color: #64748b;
  background: transparent;
}
.status-left, .status-right {
  display: flex;
  align-items: center;
  gap: 8px;
}
.status-version {
  color: #64748b;
}
.status-badge {
  padding: 1px 6px;
  border-radius: 3px;
  font-size: 10px;
  background: rgba(51, 65, 85, 0.6);
  color: #64748b;
}
.status-badge-accent {
  background: rgba(34, 211, 238, 0.15);
  color: #22d3ee;
}
.status-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
}
.dot-ok { background: #34d399; }
.dot-warn { background: #fbbf24; }
.dot-err { background: #fb7185; }
.status-label {
  color: #94a3b8;
}
</style>
