<template>
  <div class="flex flex-col h-full relative overflow-hidden">
    <!-- Splatter background -->
    <div class="absolute inset-0 pointer-events-none z-0" :style="{ background: pageSplatterBg }" />

    <!-- Header -->
    <header class="relative z-10 px-4 sm:px-6 py-3 sm:py-4 border-b border-white/[0.04]">
      <div class="flex items-center justify-between mb-3 sm:mb-4">
        <div>
          <h2 class="text-base sm:text-lg font-semibold text-white/90">Memory</h2>
          <p class="text-xs sm:text-sm text-white/40">
            {{ activeTab === 'history' ? 'Search conversation history' : activeTab === 'files' ? 'Workspace files' : 'Manage semantic memories' }}
          </p>
        </div>

        <!-- Scope Toggle -->
        <div class="flex items-center gap-2">
          <span class="text-xs text-white/30 hidden sm:inline">Scope:</span>
          <div class="flex rounded-lg overflow-hidden border border-white/[0.06]">
            <button
              @click="memoryScope = 'global'"
              :class="[
                'relative overflow-hidden px-3 py-1.5 text-xs font-medium transition-colors flex items-center gap-1.5',
                memoryScope === 'global'
                  ? 'text-white/90'
                  : 'text-white/40 hover:text-white/60'
              ]"
              :style="memoryScope === 'global' ? { ...btnGlassStyle, background: scopeSplatterBg } : btnGlassStyle"
              @mouseenter="() => { btnGlassEnter(); scopeEnter() }"
              @mouseleave="() => { btnGlassLeave(); scopeLeave() }"
            >
              <span class="absolute inset-0 z-0" :style="{ background: btnMeshBg }"></span>
              <Globe class="w-3 h-3 relative z-10" />
              <span class="hidden sm:inline relative z-10">Global</span>
            </button>
            <button
              @click="memoryScope = 'workspace'"
              :disabled="!currentWorkspace"
              :class="[
                'relative overflow-hidden px-3 py-1.5 text-xs font-medium transition-colors flex items-center gap-1.5 border-l border-white/[0.06]',
                memoryScope === 'workspace'
                  ? 'text-cyan-300/90'
                  : currentWorkspace
                    ? 'text-white/40 hover:text-white/60'
                    : 'text-white/20 cursor-not-allowed'
              ]"
              :style="memoryScope === 'workspace' ? { ...btnGlassStyle, background: scopeSplatterBg } : btnGlassStyle"
              :title="currentWorkspace ? currentWorkspace.name : 'No workspace selected'"
              @mouseenter="() => { btnGlassEnter(); scopeEnter() }"
              @mouseleave="() => { btnGlassLeave(); scopeLeave() }"
            >
              <span class="absolute inset-0 z-0" :style="{ background: btnMeshBg }"></span>
              <FolderKanban class="w-3 h-3 relative z-10" />
              <span class="hidden sm:inline max-w-[100px] truncate relative z-10">
                {{ currentWorkspace?.name || 'Workspace' }}
              </span>
            </button>
          </div>
        </div>
      </div>

      <!-- Tabs -->
      <div class="flex gap-2">
        <button
          @click="activeTab = 'history'"
          :class="[
            'relative overflow-hidden px-3 py-1.5 rounded-lg text-sm font-medium transition-colors',
            activeTab === 'history' ? 'text-white/90' : 'text-white/40 hover:text-white/60'
          ]"
          :style="activeTab === 'history' ? { ...btnGlassStyle, background: tabSplatterBg } : btnGlassStyle"
          @mouseenter="() => { btnGlassEnter(); tabEnter() }"
          @mouseleave="() => { btnGlassLeave(); tabLeave() }"
        >
          <span class="absolute inset-0 z-0" :style="{ background: btnMeshBg }"></span>
          <span class="relative z-10">📚 <span class="hidden sm:inline ml-1">Conversation</span> History</span>
        </button>
        <button
          @click="activeTab = 'semantic'"
          :class="[
            'relative overflow-hidden px-3 py-1.5 rounded-lg text-sm font-medium transition-colors',
            activeTab === 'semantic' ? 'text-white/90' : 'text-white/40 hover:text-white/60'
          ]"
          :style="activeTab === 'semantic' ? { ...btnGlassStyle, background: tabSplatterBg } : btnGlassStyle"
          @mouseenter="() => { btnGlassEnter(); tabEnter() }"
          @mouseleave="() => { btnGlassLeave(); tabLeave() }"
        >
          <span class="absolute inset-0 z-0" :style="{ background: btnMeshBg }"></span>
          <span class="relative z-10">
            🧠 <span class="hidden sm:inline ml-1">Semantic</span> Memory
            <span v-if="semanticMemories.length > 0" class="ml-1 text-xs text-cyan-300/80">
              {{ semanticMemories.length }}
            </span>
          </span>
        </button>
        <button
          v-if="currentWorkspace && memoryScope === 'workspace'"
          @click="activeTab = 'files'"
          :class="[
            'relative overflow-hidden px-3 py-1.5 rounded-lg text-sm font-medium transition-colors',
            activeTab === 'files' ? 'text-white/90' : 'text-white/40 hover:text-white/60'
          ]"
          :style="activeTab === 'files' ? { ...btnGlassStyle, background: tabSplatterBg } : btnGlassStyle"
          @mouseenter="() => { btnGlassEnter(); tabEnter() }"
          @mouseleave="() => { btnGlassLeave(); tabLeave() }"
        >
          <span class="absolute inset-0 z-0" :style="{ background: btnMeshBg }"></span>
          <span class="relative z-10">📁 <span class="hidden sm:inline ml-1">Workspace</span> Files</span>
        </button>
      </div>
    </header>

    <!-- History Tab -->
    <div v-if="activeTab === 'history'" class="relative z-10 flex-1 overflow-y-auto">
      <!-- Search -->
      <div class="px-4 sm:px-6 py-4 border-b border-white/[0.04]">
        <form @submit.prevent="performSearch" class="flex gap-2 sm:gap-3">
          <div class="relative flex-1">
            <input
              v-model="searchQuery"
              type="text"
              placeholder="Search conversations..."
              class="w-full rounded-lg px-3 py-2 pl-10 text-sm text-white/80 placeholder-white/20 border border-white/[0.06] outline-none focus:border-white/[0.12] transition-colors"
              style="background: transparent;"
              @input="debouncedSearch"
            />
            <span class="absolute left-3 top-1/2 -translate-y-1/2 text-white/20">🔍</span>
          </div>
          <button
            type="submit"
            :disabled="isSearching"
            class="relative overflow-hidden px-4 py-2 rounded-lg text-sm font-medium text-white/70 hover:text-white/90 transition-colors disabled:opacity-40"
            :style="btnGlassStyle"
            @mouseenter="btnGlassEnter"
            @mouseleave="btnGlassLeave"
          >
            <span class="absolute inset-0 z-0" :style="{ background: btnMeshBg }"></span>
            <span class="relative z-10">{{ isSearching ? 'Searching...' : 'Search' }}</span>
          </button>
        </form>
      </div>

      <div class="p-4 sm:p-6">
        <!-- Empty state -->
        <div v-if="!searchQuery && results.length === 0" class="flex items-center justify-center min-h-[300px] sm:min-h-[400px]">
          <div class="text-center max-w-md px-4">
            <div class="text-5xl sm:text-6xl mb-4">📚</div>
            <h3 class="text-lg sm:text-xl font-semibold text-white/80 mb-2">Conversation History</h3>
            <p class="text-sm text-white/40 mb-6">Search through all your conversations to find past discussions.</p>
            <div v-if="stats" class="grid grid-cols-2 gap-3 sm:gap-4 text-sm">
              <div
                class="p-3 sm:p-4 text-center rounded-xl border border-white/[0.04]"
                :style="{ background: statSplatterBg }"
                @mouseenter="statEnter" @mouseleave="statLeave"
              >
                <div class="text-xl sm:text-2xl font-bold text-cyan-300/80">{{ stats.total_sessions }}</div>
                <div class="text-white/30 text-xs sm:text-sm">Sessions</div>
              </div>
              <div
                class="p-3 sm:p-4 text-center rounded-xl border border-white/[0.04]"
                :style="{ background: statSplatterBg }"
                @mouseenter="statEnter" @mouseleave="statLeave"
              >
                <div class="text-xl sm:text-2xl font-bold text-violet-300/80">{{ stats.total_messages }}</div>
                <div class="text-white/30 text-xs sm:text-sm">Messages</div>
              </div>
            </div>
          </div>
        </div>

        <!-- Search results -->
        <div v-else-if="results.length > 0" class="space-y-3 sm:space-y-4">
          <div class="text-xs sm:text-sm text-white/30 mb-3 sm:mb-4">
            Found {{ results.length }} results for "{{ searchQuery }}"
          </div>

          <div
            v-for="result in results"
            :key="result.message_id"
            class="rounded-xl p-3 sm:p-4 cursor-pointer transition-all border border-white/[0.04] hover:border-white/[0.08]"
            :style="{ background: resultSplatterBg }"
            @mouseenter="resultEnter" @mouseleave="resultLeave"
            @click="goToSession(result)"
          >
            <div class="flex items-start justify-between mb-2 gap-2">
              <div class="flex items-center gap-2 min-w-0">
                <span class="text-sm">{{ result.role === 'user' ? '👤' : '🌙' }}</span>
                <span class="text-xs font-medium" :class="result.role === 'user' ? 'text-violet-300/70' : 'text-cyan-300/70'">
                  {{ result.role === 'user' ? 'You' : 'Nanna' }}
                </span>
              </div>
              <span class="text-xs text-white/20 flex-shrink-0">
                {{ formatDate(result.timestamp) }}
              </span>
            </div>
            <p class="text-sm text-white/60 line-clamp-3">{{ result.content }}</p>
            <div v-if="result.session_title" class="mt-2 text-xs text-white/20">
              📎 {{ result.session_title }}
            </div>
          </div>
        </div>

        <!-- No results -->
        <div v-else-if="searchQuery && !isSearching" class="flex items-center justify-center min-h-[300px]">
          <div class="text-center">
            <div class="text-4xl mb-3">🔍</div>
            <p class="text-white/40">No results found for "{{ searchQuery }}"</p>
          </div>
        </div>

        <!-- Loading -->
        <div v-if="isSearching" class="flex items-center justify-center py-12">
          <div class="text-white/30 text-sm">Searching...</div>
        </div>
      </div>
    </div>

    <!-- Semantic Memory Tab -->
    <div v-if="activeTab === 'semantic'" class="relative z-10 flex-1 overflow-y-auto">
      <!-- Actions bar -->
      <div class="px-4 sm:px-6 py-3 border-b border-white/[0.04] flex items-center justify-between">
        <div class="flex items-center gap-2">
          <input
            v-model="memoryFilter"
            type="text"
            placeholder="Filter memories..."
            class="rounded-lg px-3 py-1.5 text-sm text-white/80 placeholder-white/20 border border-white/[0.06] outline-none focus:border-white/[0.12] transition-colors"
            style="background: transparent;"
          />
        </div>
        <div class="flex items-center gap-2">
          <button
            @click="triggerDream"
            :disabled="isDreaming"
            class="relative overflow-hidden px-3 py-1.5 rounded-lg text-xs font-medium text-white/60 hover:text-white/80 transition-colors disabled:opacity-40"
            :style="btnGlassStyle"
            @mouseenter="btnGlassEnter"
            @mouseleave="btnGlassLeave"
          >
            <span class="absolute inset-0 z-0" :style="{ background: btnMeshBg }"></span>
            <span class="relative z-10">{{ isDreaming ? '💭 Dreaming...' : '💭 Dream Now' }}</span>
          </button>
          <button
            v-if="semanticMemories.length > 0"
            @click="clearAllMemories"
            class="relative overflow-hidden px-3 py-1.5 rounded-lg text-xs font-medium text-red-400/60 hover:text-red-400/80 transition-colors"
            :style="btnGlassStyle"
            @mouseenter="btnGlassEnter"
            @mouseleave="btnGlassLeave"
          >
            <span class="absolute inset-0 z-0" :style="{ background: btnMeshBg }"></span>
            <span class="relative z-10">🗑️ Clear All</span>
          </button>
        </div>
      </div>

      <div class="p-4 sm:p-6">
        <!-- Empty state -->
        <div v-if="filteredMemories.length === 0" class="flex items-center justify-center min-h-[300px] sm:min-h-[400px]">
          <div class="text-center max-w-md px-4">
            <div class="text-5xl sm:text-6xl mb-4">🧠</div>
            <h3 class="text-lg sm:text-xl font-semibold text-white/80 mb-2">
              {{ memoryFilter ? 'No matching memories' : 'No memories yet' }}
            </h3>
            <p class="text-sm text-white/40">
              {{ memoryFilter ? 'Try a different filter.' : 'Memories are created during conversations when Nanna learns something worth remembering.' }}
            </p>
          </div>
        </div>

        <!-- Memory list -->
        <div v-else class="space-y-2">
          <div
            v-for="memory in filteredMemories"
            :key="memory.id"
            class="group rounded-xl p-3 sm:p-4 transition-all border border-white/[0.04] hover:border-white/[0.08]"
            :style="{ background: memorySplatterBg }"
            @mouseenter="memoryEnter" @mouseleave="memoryLeave"
          >
            <div class="flex items-start justify-between gap-3">
              <div class="flex-1 min-w-0">
                <p class="text-sm text-white/70 leading-relaxed">{{ memory.content }}</p>
                <div class="flex items-center gap-3 mt-2 text-xs text-white/20">
                  <span>{{ formatDate(memory.created_at) }}</span>
                  <span v-if="memory.category" class="text-violet-300/40">{{ memory.category }}</span>
                  <span v-if="memory.importance" class="text-amber-300/40">
                    {{ '★'.repeat(Math.min(memory.importance, 5)) }}
                  </span>
                </div>
              </div>
              <button
                @click="deleteMemory(memory.id)"
                class="opacity-0 group-hover:opacity-100 text-white/20 hover:text-red-400/60 transition-all p-1"
                title="Delete memory"
              >
                <Trash2 class="w-3.5 h-3.5" />
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Workspace Files Tab -->
    <div v-if="activeTab === 'files'" class="relative z-10 flex-1 overflow-y-auto">
      <div class="p-4 sm:p-6">
        <div v-if="workspaceFiles.length === 0" class="flex items-center justify-center min-h-[300px]">
          <div class="text-center">
            <div class="text-5xl mb-4">📁</div>
            <h3 class="text-lg font-semibold text-white/80 mb-2">No workspace files</h3>
            <p class="text-sm text-white/40">This workspace doesn't have any special files yet.</p>
          </div>
        </div>

        <div v-else class="space-y-3">
          <div
            v-for="file in workspaceFiles"
            :key="file.name"
            class="rounded-xl p-4 transition-all border border-white/[0.04] hover:border-white/[0.08]"
            :style="{ background: fileSplatterBg }"
            @mouseenter="fileEnter" @mouseleave="fileLeave"
          >
            <div class="flex items-center justify-between mb-2">
              <div class="flex items-center gap-2">
                <span class="text-sm">📄</span>
                <span class="text-sm font-medium text-white/70">{{ file.name }}</span>
              </div>
              <span class="text-xs text-white/20">{{ formatFileSize(file.size) }}</span>
            </div>
            <pre v-if="file.preview" class="text-xs text-white/40 whitespace-pre-wrap line-clamp-4 font-mono">{{ file.preview }}</pre>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, watch, inject } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Globe, FolderKanban, Trash2 } from 'lucide-vue-next'
import { useSplatter } from '~/composables/useSplatter'
import { useGroundGlass } from '~/composables/useGroundGlass'

// Ground glass for all buttons
const { meshBg: btnMeshBg, containerStyle: btnGlassStyle, onEnter: btnGlassEnter, onLeave: btnGlassLeave } = useGroundGlass({
  blur: 12,
  opacity: 0.08,
  sizes: ['55%', '50%', '45%'],
  lerpSpeed: 0.008,
  interval: 2000,
})

// Splatter instances for different sections
const { splatterBg: pageSplatterBg } = useSplatter({
  colors: ['139,92,246', '34,197,94', '251,191,36'],
  opacityRanges: [[0.02, 0.03], [0.01, 0.02], [0.01, 0.02]],
  sizes: ['70%', '65%', '55%'],
})

const { splatterBg: scopeSplatterBg, onEnter: scopeEnter, onLeave: scopeLeave } = useSplatter({
  colors: ['139,92,246', '99,102,241', '167,139,250'],
  opacityRanges: [[0.08, 0.10], [0.06, 0.08], [0.04, 0.06]],
  sizes: ['65%', '60%', '50%'],
})

const { splatterBg: tabSplatterBg, onEnter: tabEnter, onLeave: tabLeave } = useSplatter({
  colors: ['139,92,246', '99,102,241', '167,139,250'],
  opacityRanges: [[0.08, 0.10], [0.06, 0.08], [0.04, 0.06]],
  sizes: ['65%', '60%', '50%'],
})

const { splatterBg: statSplatterBg, onEnter: statEnter, onLeave: statLeave } = useSplatter({
  colors: ['34,211,238', '56,189,248', '20,184,220'],
  opacityRanges: [[0.06, 0.08], [0.04, 0.06], [0.02, 0.04]],
  sizes: ['70%', '65%', '55%'],
})

const { splatterBg: resultSplatterBg, onEnter: resultEnter, onLeave: resultLeave } = useSplatter({
  colors: ['34,211,238', '139,92,246', '56,189,248'],
  opacityRanges: [[0.04, 0.06], [0.03, 0.05], [0.02, 0.04]],
  sizes: ['70%', '65%', '55%'],
})

const { splatterBg: memorySplatterBg, onEnter: memoryEnter, onLeave: memoryLeave } = useSplatter({
  colors: ['139,92,246', '34,197,94', '251,191,36'],
  opacityRanges: [[0.04, 0.06], [0.03, 0.05], [0.02, 0.04]],
  sizes: ['70%', '65%', '55%'],
})

const { splatterBg: fileSplatterBg, onEnter: fileEnter, onLeave: fileLeave } = useSplatter({
  colors: ['251,191,36', '234,179,8', '245,158,11'],
  opacityRanges: [[0.04, 0.06], [0.03, 0.05], [0.02, 0.04]],
  sizes: ['70%', '65%', '55%'],
})

// State
const activeTab = ref<'history' | 'semantic' | 'files'>('semantic')
const memoryScope = ref<'global' | 'workspace'>('global')
const searchQuery = ref('')
const memoryFilter = ref('')
const isSearching = ref(false)
const isDreaming = ref(false)
const results = ref<any[]>([])
const semanticMemories = ref<any[]>([])
const stats = ref<any>(null)
const workspaceFiles = ref<any[]>([])

const currentWorkspace = inject<any>('currentWorkspace', ref(null))

const filteredMemories = computed(() => {
  if (!memoryFilter.value) return semanticMemories.value
  const q = memoryFilter.value.toLowerCase()
  return semanticMemories.value.filter((m: any) =>
    m.content?.toLowerCase().includes(q) || m.category?.toLowerCase().includes(q)
  )
})

// Debounced search
let searchTimeout: ReturnType<typeof setTimeout> | null = null
function debouncedSearch() {
  if (searchTimeout) clearTimeout(searchTimeout)
  searchTimeout = setTimeout(performSearch, 300)
}

async function performSearch() {
  if (!searchQuery.value.trim()) {
    results.value = []
    return
  }
  isSearching.value = true
  try {
    const res = await invoke('search_messages', {
      query: searchQuery.value,
      scope: memoryScope.value,
      workspaceId: currentWorkspace?.value?.id,
    })
    results.value = res as any[]
  } catch (e) {
    console.error('Search failed:', e)
    results.value = []
  } finally {
    isSearching.value = false
  }
}

async function fetchMemories() {
  try {
    const res = await invoke('list_memories', {
      scope: memoryScope.value,
      workspaceId: currentWorkspace?.value?.id,
    })
    semanticMemories.value = res as any[]
  } catch (e) {
    console.error('Failed to fetch memories:', e)
  }
}

async function fetchStats() {
  try {
    stats.value = await invoke('get_memory_stats', {
      scope: memoryScope.value,
      workspaceId: currentWorkspace?.value?.id,
    })
  } catch (e) {
    console.error('Failed to fetch stats:', e)
  }
}

async function deleteMemory(id: string) {
  try {
    await invoke('delete_memory', { id })
    semanticMemories.value = semanticMemories.value.filter((m: any) => m.id !== id)
  } catch (e) {
    console.error('Failed to delete memory:', e)
  }
}

async function triggerDream() {
  isDreaming.value = true
  try {
    await invoke('trigger_dream', {
      scope: memoryScope.value,
      workspaceId: currentWorkspace?.value?.id,
    })
    await fetchMemories()
  } catch (e) {
    console.error('Dream failed:', e)
  } finally {
    isDreaming.value = false
  }
}

async function clearAllMemories() {
  if (!confirm('Are you sure you want to clear all memories? This cannot be undone.')) return
  try {
    await invoke('clear_memories', {
      scope: memoryScope.value,
      workspaceId: currentWorkspace?.value?.id,
    })
    semanticMemories.value = []
  } catch (e) {
    console.error('Failed to clear memories:', e)
  }
}

async function fetchWorkspaceFiles() {
  if (!currentWorkspace?.value?.id) return
  try {
    const res = await invoke('list_workspace_files', {
      workspaceId: currentWorkspace.value.id,
    })
    workspaceFiles.value = res as any[]
  } catch (e) {
    console.error('Failed to fetch workspace files:', e)
  }
}

function goToSession(result: any) {
  navigateTo(`/?session=${result.session_id}`)
}

function formatDate(ts: string | number): string {
  if (!ts) return ''
  const d = new Date(typeof ts === 'number' ? ts * 1000 : ts)
  const now = new Date()
  const diff = now.getTime() - d.getTime()
  if (diff < 60000) return 'just now'
  if (diff < 3600000) return `${Math.floor(diff / 60000)}m ago`
  if (diff < 86400000) return `${Math.floor(diff / 3600000)}h ago`
  if (diff < 604800000) return `${Math.floor(diff / 86400000)}d ago`
  return d.toLocaleDateString()
}

function formatFileSize(bytes: number): string {
  if (!bytes) return ''
  if (bytes < 1024) return `${bytes}B`
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)}KB`
  return `${(bytes / 1048576).toFixed(1)}MB`
}

watch(memoryScope, () => {
  fetchMemories()
  fetchStats()
  if (activeTab.value === 'files') fetchWorkspaceFiles()
})

watch(activeTab, (tab) => {
  if (tab === 'files') fetchWorkspaceFiles()
})

onMounted(() => {
  fetchMemories()
  fetchStats()
})
</script>

<style scoped>
.line-clamp-3 {
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.line-clamp-4 {
  display: -webkit-box;
  -webkit-line-clamp: 4;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
</style>
