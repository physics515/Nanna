<template>
  <div class="flex items-center gap-1 px-2 py-1.5 bg-nanna-bg-deep/50 border-b border-nanna-primary/10 overflow-x-auto scrollbar-thin">
    <!-- Global Tab -->
    <button
      :class="[
        'flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm font-medium transition-all whitespace-nowrap',
        'hover:bg-nanna-bg-elevated',
        isGlobalActive 
          ? 'bg-nanna-bg-surface text-nanna-text border border-nanna-primary/20' 
          : 'text-nanna-text-muted'
      ]"
      @click="$emit('select', { type: 'global' })"
    >
      <Globe class="w-3.5 h-3.5" />
      <span>Global</span>
    </button>

    <!-- Divider -->
    <div v-if="openWorkspaces.length > 0" class="w-px h-5 bg-nanna-primary/20 mx-1" />

    <!-- Workspace Tabs -->
    <div
      v-for="ws in openWorkspaces"
      :key="ws.id"
      :class="[
        'group flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm font-medium transition-all whitespace-nowrap',
        'hover:bg-nanna-bg-elevated',
        currentTab?.type === 'workspace' && currentTab?.workspaceId === ws.id
          ? 'bg-nanna-bg-surface text-nanna-accent border border-nanna-accent/30'
          : 'text-nanna-text-muted'
      ]"
    >
      <button
        class="flex items-center gap-1.5"
        @click="$emit('select', { type: 'workspace', workspaceId: ws.id })"
      >
        <FolderKanban class="w-3.5 h-3.5" />
        <span class="max-w-[120px] truncate">{{ ws.name }}</span>
      </button>
      
      <!-- Close button -->
      <button
        class="p-0.5 rounded opacity-0 group-hover:opacity-100 hover:bg-nanna-bg-elevated transition-opacity"
        @click.stop="$emit('close', ws.id)"
        title="Close tab"
      >
        <X class="w-3 h-3" />
      </button>
    </div>

    <!-- Add Workspace Button -->
    <button
      class="flex items-center justify-center w-7 h-7 rounded-lg text-nanna-text-dim hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors ml-1"
      @click="$emit('add')"
      title="Open workspace"
    >
      <Plus class="w-4 h-4" />
    </button>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { Globe, FolderKanban, X, Plus } from 'lucide-vue-next'

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

const props = defineProps<{
  openWorkspaces: WorkspaceInfo[]
  currentTab: Tab | null
}>()

defineEmits<{
  select: [tab: Tab]
  close: [workspaceId: string]
  add: []
}>()

const isGlobalActive = computed(() => 
  !props.currentTab || props.currentTab.type === 'global'
)
</script>

<style scoped>
.scrollbar-thin::-webkit-scrollbar {
  height: 4px;
}
.scrollbar-thin::-webkit-scrollbar-track {
  background: transparent;
}
.scrollbar-thin::-webkit-scrollbar-thumb {
  background: rgba(139, 92, 246, 0.3);
  border-radius: 2px;
}
</style>
