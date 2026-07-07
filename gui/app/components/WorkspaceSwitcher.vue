<script setup lang="ts">
import { ref, computed, inject } from 'vue'
import {
  SelectRoot, SelectTrigger, SelectPortal, SelectContent,
  SelectItem, SelectItemText, SelectItemIndicator,
  SelectSeparator, SelectGroup,
} from 'radix-vue'
import { Globe, FolderKanban, ChevronDown, Plus, Settings, Check } from 'lucide-vue-next'

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

const currentTab = inject<Ref<Tab>>('currentTab', ref({ type: 'global' }))
const openWorkspaces = inject<Ref<WorkspaceInfo[]>>('openWorkspaces', ref([]))
const selectTab = inject<(tab: Tab) => void>('selectTab')
const showWorkspacePicker = inject<Ref<boolean>>('showWorkspacePicker')

const currentValue = computed({
  get: () => {
    if (!currentTab.value || currentTab.value.type === 'global') return 'global'
    return currentTab.value.workspaceId || 'global'
  },
  set: (val: string) => {
    if (val === '__add__') {
      if (showWorkspacePicker) showWorkspacePicker.value = true
      return
    }
    if (val === '__manage__') {
      navigateTo('/workspaces')
      return
    }
    if (val === 'global') {
      selectTab?.({ type: 'global' })
    } else {
      selectTab?.({ type: 'workspace', workspaceId: val })
    }
  },
})

const currentLabel = computed(() => {
  if (currentValue.value === 'global') return 'Global'
  const ws = openWorkspaces.value.find(w => w.id === currentValue.value)
  return ws?.name || 'Workspace'
})

const isGlobal = computed(() => currentValue.value === 'global')
</script>

<template>
  <div class="ws-switcher">
    <SelectRoot v-model="currentValue">
      <SelectTrigger class="ws-trigger" aria-label="Workspace">
        <component
          :is="isGlobal ? Globe : FolderKanban"
          class="ws-trigger__icon"
          :class="{ 'ws-trigger__icon--accent': !isGlobal }"
        />
        <span class="ws-trigger__name" :class="{ 'ws-trigger__name--accent': !isGlobal }">
          {{ currentLabel }}
        </span>
        <ChevronDown class="ws-trigger__chevron" />
      </SelectTrigger>

      <SelectPortal>
        <SelectContent class="ws-content" position="popper" :side-offset="4" side="bottom" align="start">
          <div class="ws-content__viewport">
            <SelectGroup>
              <SelectItem value="global" class="ws-item">
                <Globe class="ws-item__icon" />
                <SelectItemText>Global</SelectItemText>
                <SelectItemIndicator class="ws-item__check">
                  <Check class="w-3 h-3" />
                </SelectItemIndicator>
              </SelectItem>
            </SelectGroup>

            <SelectSeparator v-if="openWorkspaces.length > 0" class="ws-separator" />

            <SelectGroup v-if="openWorkspaces.length > 0">
              <SelectItem
                v-for="ws in openWorkspaces"
                :key="ws.id"
                :value="ws.id"
                class="ws-item"
              >
                <FolderKanban class="ws-item__icon ws-item__icon--accent" />
                <SelectItemText>{{ ws.name }}</SelectItemText>
                <SelectItemIndicator class="ws-item__check">
                  <Check class="w-3 h-3" />
                </SelectItemIndicator>
              </SelectItem>
            </SelectGroup>

            <SelectSeparator class="ws-separator" />

            <SelectGroup>
              <SelectItem value="__add__" class="ws-item">
                <Plus class="ws-item__icon" />
                <SelectItemText>Open Workspace</SelectItemText>
              </SelectItem>
              <SelectItem value="__manage__" class="ws-item">
                <Settings class="ws-item__icon" />
                <SelectItemText>Manage Workspaces</SelectItemText>
              </SelectItem>
            </SelectGroup>
          </div>
        </SelectContent>
      </SelectPortal>
    </SelectRoot>
  </div>
</template>

<style scoped>
.ws-switcher {
  padding: 0.5rem 0.75rem 0.25rem;
}

/* ═══ Trigger ═══ */
.ws-trigger {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 5px 8px;
  font-size: 12px;
  color: rgba(176, 190, 201, 0.8);
  background: transparent;
  border: none;
  border-radius: 0.375rem;
  cursor: pointer;
  outline: none;
  transition: color 0.15s ease;
}
.ws-trigger:hover {
  color: rgba(226, 232, 240, 1);
}

.ws-trigger__icon {
  width: 12px;
  height: 12px;
  flex-shrink: 0;
  color: rgba(100, 116, 139, 0.7);
}
.ws-trigger__icon--accent {
  color: #22d3ee;
}

.ws-trigger__name {
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-align: left;
}
.ws-trigger__name--accent {
  color: #22d3ee;
}

.ws-trigger__chevron {
  width: 10px;
  height: 10px;
  flex-shrink: 0;
  color: rgba(100, 116, 139, 0.5);
}

/* ═══ Content (dropdown panel) ═══ */
.ws-content {
  z-index: 100;
  min-width: 180px;
  max-width: 260px;
  border-radius: 0.75rem;
  padding: 6px;
  background: rgba(15, 23, 42, 0.92);
  backdrop-filter: blur(12px);
  -webkit-backdrop-filter: blur(12px);
  border: 1px solid rgba(255, 255, 255, 0.06);
  box-shadow:
    0 4px 16px -4px rgba(0, 0, 0, 0.4),
    0 1px 2px rgba(0, 0, 0, 0.2);
  animation: ws-dropdown-in 0.12s ease-out;
}

.ws-content__viewport {
  max-height: 300px;
  overflow-y: auto;
}

/* ═══ Items ═══ */
.ws-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 8px 12px;
  font-size: 12px;
  color: rgba(196, 205, 214, 0.7);
  border: none;
  background: transparent;
  border-radius: 0.375rem;
  cursor: pointer;
  outline: none;
  transition: background 0.1s ease, color 0.1s ease;
  position: relative;
  user-select: none;
  text-align: left;
}
.ws-item:hover,
.ws-item[data-highlighted] {
  background: rgba(139, 92, 246, 0.12);
  color: #e2e8f0;
}
.ws-item[data-state="checked"] {
  color: #e2e8f0;
}

.ws-item__icon {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  color: rgba(148, 163, 184, 0.5);
}
.ws-item__icon--accent {
  color: rgba(34, 211, 238, 0.7);
}

.ws-item__check {
  margin-left: auto;
  color: rgba(139, 92, 246, 0.8);
}

/* ═══ Separator ═══ */
.ws-separator {
  height: 1px;
  margin: 4px 6px;
  background: rgba(255, 255, 255, 0.06);
}

/* ═══ Animation ═══ */
@keyframes ws-dropdown-in {
  from { opacity: 0; transform: translateY(-4px) scale(0.97); }
  to { opacity: 1; transform: translateY(0) scale(1); }
}
</style>
