<script setup lang="ts">
import { ref, computed, inject, onMounted, onUnmounted } from 'vue'
import { Globe, FolderKanban, ChevronDown, Plus, Settings } from 'lucide-vue-next'
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

// Inject workspace state from layout
const currentTab = inject<Ref<Tab>>('currentTab', ref({ type: 'global' }))
const openWorkspaces = inject<Ref<WorkspaceInfo[]>>('openWorkspaces', ref([]))
const selectTab = inject<(tab: Tab) => void>('selectTab')
const closeWorkspaceTab = inject<(workspaceId: string) => void>('closeWorkspaceTab')
const showWorkspacePicker = inject<Ref<boolean>>('showWorkspacePicker')

const dropdownOpen = ref(false)

// Ground glass for the dropdown
const { meshBg, containerStyle, onEnter: glassEnter, onLeave: glassLeave } = useGroundGlass({
  opacity: 2.5,
})

// Prevent animation on mount when cursor is already over the dropdown area
const ready = ref(false)

const currentTabName = computed(() => {
  if (!currentTab.value || currentTab.value.type === 'global') return 'Global'
  const ws = openWorkspaces.value.find(w => w.id === currentTab.value.workspaceId)
  return ws?.name || 'Workspace'
})

const isGlobal = computed(() => !currentTab.value || currentTab.value.type === 'global')

function handleDropdownEnter() {
  if (ready.value) glassEnter()
}
function handleDropdownLeave() {
  if (ready.value) glassLeave()
}

onMounted(() => {
  document.addEventListener('click', closeDropdown)
  setTimeout(() => { ready.value = true }, 200)
})

onUnmounted(() => {
  document.removeEventListener('click', closeDropdown)
})

function closeDropdown() {
  dropdownOpen.value = false
}

function toggleDropdown(e: Event) {
  e.stopPropagation()
  dropdownOpen.value = !dropdownOpen.value
}

function selectGlobal() {
  selectTab?.({ type: 'global' })
  dropdownOpen.value = false
}

function selectWorkspace(wsId: string) {
  selectTab?.({ type: 'workspace', workspaceId: wsId })
  dropdownOpen.value = false
}

function removeWorkspace(e: Event, wsId: string) {
  e.stopPropagation()
  closeWorkspaceTab?.(wsId)
}

function openPicker() {
  dropdownOpen.value = false
  if (showWorkspacePicker) showWorkspacePicker.value = true
}

function manageWorkspaces() {
  dropdownOpen.value = false
  navigateTo('/workspaces')
}
</script>

<template>
  <div class="ws-switcher">
    <!-- Trigger -->
    <button class="ws-trigger" @click="toggleDropdown">
      <component
        :is="isGlobal ? Globe : FolderKanban"
        :style="{
          width: '12px',
          height: '12px',
          color: isGlobal ? '#64748b' : '#22d3ee',
          flexShrink: 0,
        }"
      />
      <span class="ws-name" :style="{ color: isGlobal ? 'rgba(176,190,201,0.8)' : '#22d3ee' }">
        {{ currentTabName }}
      </span>
      <ChevronDown
        :style="{
          width: '10px',
          height: '10px',
          color: 'rgba(100,116,139,0.5)',
          flexShrink: 0,
          transition: 'transform 0.15s ease',
          transform: dropdownOpen ? 'rotate(180deg)' : 'rotate(0)',
        }"
      />
    </button>

    <!-- Ground Glass Dropdown -->
    <Transition name="ws-dropdown">
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

        <!-- Layer 1: content -->
        <div class="ws-dropdown__viewport">
          <!-- Global -->
          <button
            :class="['ws-item', { 'ws-item-active': isGlobal }]"
            @click="selectGlobal"
          >
            <Globe :style="{ width: '14px', height: '14px', flexShrink: 0 }" />
            <span>Global</span>
          </button>

          <!-- Open workspaces -->
          <template v-if="openWorkspaces.length > 0">
            <div class="ws-divider" />
            <div
              v-for="ws in openWorkspaces"
              :key="ws.id"
              :class="['ws-item', { 'ws-item-active': currentTab?.type === 'workspace' && currentTab?.workspaceId === ws.id }]"
              @click="selectWorkspace(ws.id)"
            >
              <FolderKanban :style="{ width: '14px', height: '14px', flexShrink: 0 }" />
              <span class="ws-item-name">{{ ws.name }}</span>
              <button class="ws-item-close" @click="removeWorkspace($event, ws.id)" title="Close">
                <svg viewBox="0 0 10 10" stroke="currentColor" stroke-width="1.5" style="width: 8px; height: 8px;">
                  <line x1="2" y1="2" x2="8" y2="8" />
                  <line x1="8" y1="2" x2="2" y2="8" />
                </svg>
              </button>
            </div>
          </template>

          <!-- Actions -->
          <div class="ws-divider" />
          <button class="ws-item" @click="openPicker">
            <Plus :style="{ width: '14px', height: '14px', flexShrink: 0 }" />
            <span>Open Workspace</span>
          </button>
          <button class="ws-item" @click="manageWorkspaces">
            <Settings :style="{ width: '14px', height: '14px', flexShrink: 0 }" />
            <span>Manage Workspaces</span>
          </button>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.ws-switcher {
  position: relative;
  padding: 0.5rem 0.75rem 0.25rem;
}

/* ═══ Trigger ═══ */
.ws-trigger {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 4px 8px;
  font-size: 12px;
  color: rgba(176, 190, 201, 0.8);
  background: transparent;
  border: none;
  border-radius: 0.375rem;
  cursor: pointer;
  outline: none;
  transition: color 0.2s ease;
}
.ws-trigger:hover {
  color: rgba(176, 190, 201, 1);
}

.ws-name {
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-align: left;
}

/* ═══ Ground Glass Dropdown ═══ */
.ws-dropdown {
  position: absolute;
  top: 100%;
  left: 0.75rem;
  right: 0.75rem;
  margin-top: 4px;
  z-index: 100;
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
  max-height: 280px;
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

/* ═══ Items ═══ */
.ws-item {
  position: relative;
  z-index: 3;
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 6px 10px;
  font-size: 12px;
  color: rgba(196, 205, 214, 0.7);
  border: none;
  background: transparent;
  border-radius: 0.375rem;
  cursor: pointer;
  text-align: left;
  outline: none;
  transition: background 0.1s ease, color 0.1s ease;
}
.ws-item:hover {
  background: rgba(139, 92, 246, 0.12);
  color: #e2e8f0;
}
.ws-item-active {
  color: #e2e8f0;
}

.ws-item-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ws-item-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border-radius: 4px;
  border: none;
  background: transparent;
  color: rgba(100, 116, 139, 0.5);
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.1s, background 0.1s, color 0.1s;
  flex-shrink: 0;
}
.ws-item:hover .ws-item-close {
  opacity: 1;
}
.ws-item-close:hover {
  background: rgba(255, 255, 255, 0.08);
  color: #e2e8f0;
}

.ws-divider {
  position: relative;
  z-index: 3;
  height: 1px;
  margin: 4px 6px;
  background: rgba(255, 255, 255, 0.06);
}

/* ═══ Dropdown transition ═══ */
.ws-dropdown-enter-active {
  animation: ws-dropdown-in 0.15s ease-out;
}
.ws-dropdown-leave-active {
  animation: ws-dropdown-in 0.1s ease-in reverse;
}
@keyframes ws-dropdown-in {
  from { opacity: 0; transform: translateY(-4px) scale(0.97); }
  to { opacity: 1; transform: translateY(0) scale(1); }
}
</style>
