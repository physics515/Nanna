<script setup lang="ts">
import { ref, computed, inject, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Bell } from 'lucide-vue-next'
import { useGroundGlass } from '~/composables/useGroundGlass'

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

// Window state
const maximized = ref(false)
let appWindow: any = null

// Notifications
const { unreadCount, isOpen: notifOpen } = useNotificationCenter()

// Inject workspace state from layout
const currentTab = inject<Ref<Tab>>('currentTab', ref({ type: 'global' }))
const openWorkspaces = inject<Ref<WorkspaceInfo[]>>('openWorkspaces', ref([]))
const selectTab = inject<(tab: Tab) => void>('selectTab')
const addWorkspaceTab = inject<(ws: WorkspaceInfo) => void>('addWorkspaceTab')
const showWorkspacePicker = inject<Ref<boolean>>('showWorkspacePicker')

// Dropdown state
const dropdownOpen = ref(false)
const allWorkspaces = ref<WorkspaceInfo[]>([])
const ready = ref(false)

// Ground glass effect for dropdown
const { meshBg, containerStyle, onEnter: glassEnter, onLeave: glassLeave } = useGroundGlass({
  opacity: 2.5,
})

// Ground glass effect for trigger button
const { meshBg: btnMeshBg, containerStyle: btnGlassStyle, onEnter: btnGlassEnter, onLeave: btnGlassLeave } = useGroundGlass({
  opacity: 2.0,
  sizes: ['55%', '50%', '45%'],
  lerpSpeed: 0.008,
  interval: 2000,
  blur: 8,
})

function handleDropdownEnter() {
  if (ready.value) glassEnter()
}
function handleDropdownLeave() {
  if (ready.value) glassLeave()
}
function handleBtnEnter() {
  if (ready.value) btnGlassEnter()
}
function handleBtnLeave() {
  if (ready.value) btnGlassLeave()
}

const isGlobal = computed(() => !currentTab.value || currentTab.value.type === 'global')

const currentWorkspaceName = computed(() => {
  if (isGlobal.value) return 'Global'
  const ws = openWorkspaces.value.find(w => w.id === currentTab.value?.workspaceId)
  return ws?.name || 'Workspace'
})

function toggleDropdown(e: Event) {
  e.stopPropagation()
  dropdownOpen.value = !dropdownOpen.value
  if (dropdownOpen.value) {
    loadWorkspaces()
  }
}

function closeDropdown() {
  dropdownOpen.value = false
}

async function loadWorkspaces() {
  try {
    allWorkspaces.value = await invoke<WorkspaceInfo[]>('list_workspaces')
  } catch (e) {
    console.error('Failed to load workspaces:', e)
  }
}

async function selectGlobal() {
  selectTab?.({ type: 'global' })
  dropdownOpen.value = false
  // Clear active workspace on daemon
  try {
    await invoke('clear_active_workspace')
  } catch (e) {
    console.error('Failed to clear active workspace:', e)
  }
}

async function selectWorkspace(ws: WorkspaceInfo) {
  // Add to open workspaces if not already there
  addWorkspaceTab?.(ws)
  // Switch tab
  selectTab?.({ type: 'workspace', workspaceId: ws.id })
  dropdownOpen.value = false
  // Set active workspace on daemon (for tool working directory + context)
  try {
    await invoke('set_active_workspace', { id: ws.id })
  } catch (e) {
    console.error('Failed to set active workspace:', e)
  }
}

function openPicker() {
  dropdownOpen.value = false
  if (showWorkspacePicker) showWorkspacePicker.value = true
}

function manageWorkspaces() {
  dropdownOpen.value = false
  navigateTo('/workspaces')
}

onMounted(async () => {
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  appWindow = getCurrentWindow()
  maximized.value = await appWindow.isMaximized()
  document.addEventListener('click', closeDropdown)
  setTimeout(() => { ready.value = true }, 200)
})

onUnmounted(() => {
  document.removeEventListener('click', closeDropdown)
})

async function minimize() {
  await appWindow?.minimize()
}

async function toggleMaximize() {
  await appWindow?.toggleMaximize()
  maximized.value = await appWindow?.isMaximized()
}

async function close() {
  await appWindow?.close()
}
</script>

<template>
  <div class="h-8 flex items-center select-none shrink-0" data-tauri-drag-region>

    <!-- ═══ Workspace Selector (left side of title bar) ═══ -->
    <div class="ws-selector" @click.stop>
      <button
        class="ws-trigger"
        :style="btnGlassStyle"
        @click="toggleDropdown"
        @mouseenter="handleBtnEnter"
        @mouseleave="handleBtnLeave"
      >
        <!-- Glass mesh layer -->
        <span class="ws-trigger__mesh" :style="{ background: btnMeshBg }" />
        <!-- Noise overlay -->
        <span class="ws-trigger__noise" />
        <!-- Content -->
        <span class="ws-trigger__label">
          <!-- Globe icon for global -->
          <svg v-if="isGlobal" class="ws-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="10" />
            <path d="M2 12h20" />
            <path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z" />
          </svg>
          <!-- Folder icon for workspace -->
          <svg v-else class="ws-icon ws-icon--active" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M2 20V8a2 2 0 012-2h3.9a2 2 0 011.69.9l.81 1.2a2 2 0 001.67.9H20a2 2 0 012 2v9a2 2 0 01-2 2H4a2 2 0 01-2-2z" />
          </svg>
          <span class="ws-label" :class="{ 'ws-label--active': !isGlobal }">
            {{ currentWorkspaceName }}
          </span>
          <svg class="ws-chevron" :class="{ 'ws-chevron--open': dropdownOpen }" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M2 3.5l3 3 3-3" />
          </svg>
        </span>
      </button>

      <!-- Ground Glass Dropdown -->
      <Transition name="ws-dd">
        <div
          v-if="dropdownOpen"
          class="ws-dropdown"
          :style="containerStyle"
          @click.stop
          @mouseenter="handleDropdownEnter"
          @mouseleave="handleDropdownLeave"
        >
          <!-- Layer 0: animated mesh gradient -->
          <span class="ws-dropdown__mesh" :style="{ background: meshBg }" />

          <!-- Layer 1: content viewport -->
          <div class="ws-dropdown__viewport">
            <!-- Global -->
            <button
              class="ws-item"
              :class="{ 'ws-item--active': isGlobal }"
              @click="selectGlobal"
            >
              <svg class="ws-item-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="12" r="10" />
                <path d="M2 12h20" />
                <path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z" />
              </svg>
              <span class="ws-item-label">Global</span>
              <span class="ws-item-hint">all chats</span>
              <svg v-if="isGlobal" class="ws-check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="20 6 9 17 4 12" />
              </svg>
            </button>

            <div v-if="allWorkspaces.length > 0" class="ws-divider" />

            <!-- Registered workspaces -->
            <button
              v-for="ws in allWorkspaces"
              :key="ws.id"
              class="ws-item"
              :class="{ 'ws-item--active': currentTab?.type === 'workspace' && currentTab?.workspaceId === ws.id }"
              @click="selectWorkspace(ws)"
            >
              <svg class="ws-item-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M2 20V8a2 2 0 012-2h3.9a2 2 0 011.69.9l.81 1.2a2 2 0 001.67.9H20a2 2 0 012 2v9a2 2 0 01-2 2H4a2 2 0 01-2-2z" />
              </svg>
              <div class="ws-item-info">
                <span class="ws-item-name">{{ ws.name }}</span>
                <span class="ws-item-path">{{ ws.path }}</span>
              </div>
              <svg v-if="currentTab?.type === 'workspace' && currentTab?.workspaceId === ws.id" class="ws-check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="20 6 9 17 4 12" />
              </svg>
            </button>

            <div v-if="allWorkspaces.length === 0" class="ws-empty">
              No workspaces registered
            </div>

            <div class="ws-divider" />

            <!-- Actions -->
            <button class="ws-item" @click="openPicker">
              <svg class="ws-item-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="12" y1="5" x2="12" y2="19" />
                <line x1="5" y1="12" x2="19" y2="12" />
              </svg>
              <span class="ws-item-label">Open Workspace...</span>
            </button>
            <button class="ws-item" @click="manageWorkspaces">
              <svg class="ws-item-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="12" r="3" />
                <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
              </svg>
              <span class="ws-item-label">Manage Workspaces</span>
            </button>
          </div>
        </div>
      </Transition>
    </div>

    <!-- Draggable spacer -->
    <div class="flex-1" data-tauri-drag-region></div>

    <!-- Notification bell -->
    <button class="titlebar-btn notification-bell" @click="notifOpen = !notifOpen" title="Notifications">
      <Bell class="w-[13px] h-[13px]" />
      <span v-if="unreadCount > 0" class="notif-badge">
        {{ unreadCount > 9 ? '9+' : unreadCount }}
      </span>
    </button>

    <!-- Notification Center sheet -->
    <UiSheet :open="notifOpen" side="right" @update:open="notifOpen = $event">
      <template #trigger><span /></template>
      <NotificationCenter />
    </UiSheet>

    <!-- Window controls -->
    <button class="titlebar-btn" @click="minimize">
      <svg class="w-[10px] h-[10px]" viewBox="0 0 10 10" fill="currentColor">
        <rect x="1" y="5" width="8" height="1" />
      </svg>
    </button>
    <button class="titlebar-btn" @click="toggleMaximize">
      <svg v-if="!maximized" class="w-[10px] h-[10px]" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1">
        <rect x="1" y="1" width="8" height="8" />
      </svg>
      <svg v-else class="w-[10px] h-[10px]" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1">
        <rect x="2.5" y="0.5" width="7" height="7" />
        <rect x="0.5" y="2.5" width="7" height="7" />
      </svg>
    </button>
    <button class="titlebar-btn titlebar-close" @click="close">
      <svg class="w-[10px] h-[10px]" viewBox="0 0 10 10" stroke="currentColor" stroke-width="1.2">
        <line x1="1" y1="1" x2="9" y2="9" />
        <line x1="9" y1="1" x2="1" y2="9" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
/* ═══ Workspace Selector ═══ */
.ws-selector {
  position: relative;
  margin-left: 8px;
  z-index: 100;
}

.ws-trigger {
  position: relative;
  isolation: isolate;
  display: flex;
  align-items: center;
  height: auto;
  min-width: 160px;
  padding: 5px 12px;
  border: none;
  border-radius: 0 0 10px 10px;
  background: rgba(30, 41, 59, 0.30);
  cursor: pointer;
  overflow: hidden;
  text-align: left;
  box-shadow:
    0 1.5px 3px -1px rgba(0, 0, 0, 0.25),
    0 1px 2px -1px rgba(0, 0, 0, 0.15);
  transition: box-shadow 0.15s ease;
}
.ws-trigger:hover {
  box-shadow:
    0 2px 6px -1px rgba(0, 0, 0, 0.35),
    0 2px 4px -2px rgba(0, 0, 0, 0.2);
}

/* Glass mesh layer */
.ws-trigger__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
}

/* Noise overlay */
.ws-trigger__noise {
  position: absolute;
  inset: 0;
  z-index: 1;
  pointer-events: none;
  border-radius: inherit;
  opacity: 0.14;
  background-blend-mode: soft-light;
  background: repeating-radial-gradient(
    circle,
    #1a2035,
    #1a2035 2px,
    #253050 2px 4px,
    #1a2035 4px 6px,
    #253050 6px 8px,
    #1a2035 8px 10px,
    #253050 10px 12px
  ) 0 0 / 100% 100%;
}

/* Label container */
.ws-trigger__label {
  position: relative;
  z-index: 2;
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 0;
  width: 100%;
}

.ws-icon {
  width: 12px;
  height: 12px;
  color: #64748b;
  flex-shrink: 0;
}
.ws-icon--active {
  color: #22d3ee;
}

.ws-label {
  flex: 1;
  font-size: 11px;
  color: #e2e8f0;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ws-label--active {
  color: #22d3ee;
}

.ws-chevron {
  width: 8px;
  height: 8px;
  color: rgba(148, 163, 184, 0.6);
  flex-shrink: 0;
  margin-left: auto;
  transition: transform 0.15s ease;
}
.ws-chevron--open {
  transform: rotate(180deg);
}

/* ═══ Ground Glass Dropdown ═══ */
.ws-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  min-width: 220px;
  max-width: 320px;
  z-index: 200;
  isolation: isolate;
  overflow: hidden;
  border-radius: 0.75rem;
  padding: 4px;
  background: rgba(30, 41, 59, 0.30);
  /* Glass slab borders */
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  border-left: 1px solid rgba(255, 255, 255, 0.04);
  border-bottom: 1.5px solid rgba(71, 85, 105, 0.18);
  border-right: 1px solid rgba(71, 85, 105, 0.10);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.04),
    0 1.5px 1px -0.5px rgba(0, 0, 0, 0.18),
    0 3px 8px -3px rgba(0, 0, 0, 0.12);
}

/* Layer 0: animated mesh gradient */
.ws-dropdown__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
}

/* Layer 1: scrollable viewport */
.ws-dropdown__viewport {
  position: relative;
  z-index: 1;
  overflow-y: auto;
  max-height: 320px;
}

/* Layer 2: ground glass noise overlay */
.ws-dropdown::after {
  content: '';
  position: absolute;
  inset: 0;
  z-index: 2;
  pointer-events: none;
  border-radius: 0.75rem;
  opacity: 0.18;
  background-blend-mode: soft-light;
  background: repeating-radial-gradient(
    circle,
    #1a2035,
    #1a2035 2px,
    #253050 2px 4px,
    #1a2035 4px 6px,
    #253050 6px 8px,
    #1a2035 8px 10px,
    #253050 10px 12px
  ) 0 0 / 100% 100%;
}

.ws-item {
  position: relative;
  z-index: 3;
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 6px 8px;
  font-size: 12px;
  color: #f1f5f9;
  border: none;
  background: transparent;
  border-radius: 0.375rem;
  cursor: pointer;
  text-align: left;
  outline: none;
  transition: background 0.1s, color 0.1s;
}
.ws-item:hover {
  background: rgba(139, 92, 246, 0.15);
  color: #ffffff;
}
.ws-item--active {
  color: #ffffff;
}

.ws-item-icon {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
}

.ws-item-label {
  flex: 1;
}

.ws-item-hint {
  font-size: 10px;
  color: rgba(203, 213, 225, 0.5);
  margin-left: auto;
}

.ws-item-info {
  flex: 1;
  min-width: 0;
}

.ws-item-name {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ws-item-path {
  display: block;
  font-size: 10px;
  color: rgba(203, 213, 225, 0.45);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ws-check {
  width: 12px;
  height: 12px;
  color: #22d3ee;
  flex-shrink: 0;
}

.ws-divider {
  position: relative;
  z-index: 3;
  height: 1px;
  margin: 4px 6px;
  background: rgba(255, 255, 255, 0.06);
}

.ws-empty {
  position: relative;
  z-index: 3;
  padding: 8px;
  font-size: 11px;
  color: rgba(203, 213, 225, 0.5);
  text-align: center;
}

/* ═══ Dropdown transition ═══ */
.ws-dd-enter-active {
  animation: ws-dd-in 0.12s ease-out;
}
.ws-dd-leave-active {
  animation: ws-dd-in 0.08s ease-in reverse;
}
@keyframes ws-dd-in {
  from { opacity: 0; transform: translateY(-4px) scale(0.97); }
  to { opacity: 1; transform: translateY(0) scale(1); }
}

/* ═══ Window Controls ═══ */
.titlebar-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 46px;
  height: 32px;
  color: #94a3b8;
  transition: background-color 0.1s, color 0.1s;
}

.titlebar-btn:hover {
  background-color: rgba(255, 255, 255, 0.06);
  color: #e2e8f0;
}

.titlebar-close:hover {
  background-color: #ef4444;
  color: #ffffff;
}

.notification-bell {
  position: relative;
  width: 36px;
}

.notif-badge {
  position: absolute;
  top: 4px;
  right: 5px;
  min-width: 14px;
  height: 14px;
  padding: 0 3px;
  border-radius: 7px;
  background: #8b5cf6;
  color: #fff;
  font-size: 9px;
  font-weight: 600;
  line-height: 14px;
  text-align: center;
  pointer-events: none;
}
</style>
