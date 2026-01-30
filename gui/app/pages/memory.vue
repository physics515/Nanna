<template>
  <div class="flex flex-col h-full">
    <!-- Header with tabs -->
    <header class="px-4 sm:px-6 py-3 sm:py-4 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
      <div class="flex items-center justify-between mb-3 sm:mb-4">
        <div>
          <h2 class="text-base sm:text-lg font-semibold text-nanna-text">Memory</h2>
          <p class="text-xs sm:text-sm text-nanna-text-muted">
            {{ activeTab === 'history' ? 'Search conversation history' : 'Manage semantic memories' }}
          </p>
        </div>
      </div>
      
      <!-- Tabs -->
      <div class="flex gap-2">
        <UiButton
          @click="activeTab = 'history'"
          :variant="activeTab === 'history' ? 'default' : 'ghost'"
          size="sm"
        >
          📚 <span class="hidden sm:inline ml-1">Conversation</span> History
        </UiButton>
        <UiButton
          @click="activeTab = 'semantic'"
          :variant="activeTab === 'semantic' ? 'default' : 'ghost'"
          size="sm"
        >
          🧠 <span class="hidden sm:inline ml-1">Semantic</span> Memory
          <UiBadge v-if="semanticMemories.length > 0" variant="accent" class="ml-1 text-xs">
            {{ semanticMemories.length }}
          </UiBadge>
        </UiButton>
      </div>
    </header>

    <!-- History Tab Content -->
    <div v-if="activeTab === 'history'" class="flex-1 overflow-y-auto">
      <!-- Search bar -->
      <div class="px-4 sm:px-6 py-4 border-b border-nanna-primary/10">
        <form @submit.prevent="performSearch" class="flex gap-2 sm:gap-3">
          <div class="relative flex-1">
            <UiInput
              v-model="searchQuery"
              type="text"
              placeholder="Search conversations..."
              class="pl-10"
              @input="debouncedSearch"
            />
            <span class="absolute left-3 top-1/2 -translate-y-1/2 text-nanna-text-dim">
              🔍
            </span>
          </div>
          <UiButton type="submit" :disabled="isSearching">
            {{ isSearching ? 'Searching...' : 'Search' }}
          </UiButton>
        </form>
      </div>

      <!-- Content area -->
      <div class="p-4 sm:p-6">
        <!-- Empty state -->
        <div v-if="!searchQuery && results.length === 0" class="flex items-center justify-center min-h-[300px] sm:min-h-[400px]">
          <div class="text-center max-w-md px-4">
            <div class="text-5xl sm:text-6xl mb-4">📚</div>
            <h3 class="text-lg sm:text-xl font-semibold text-nanna-text mb-2">
              Conversation History
            </h3>
            <p class="text-sm text-nanna-text-muted mb-6">
              Search through all your conversations to find past discussions.
            </p>
            <div v-if="stats" class="grid grid-cols-2 gap-3 sm:gap-4 text-sm">
              <UiCard class="p-3 sm:p-4 text-center">
                <div class="text-xl sm:text-2xl font-bold text-nanna-accent">{{ stats.total_sessions }}</div>
                <div class="text-nanna-text-dim text-xs sm:text-sm">Sessions</div>
              </UiCard>
              <UiCard class="p-3 sm:p-4 text-center">
                <div class="text-xl sm:text-2xl font-bold text-nanna-secondary">{{ stats.total_messages }}</div>
                <div class="text-nanna-text-dim text-xs sm:text-sm">Messages</div>
              </UiCard>
            </div>
          </div>
        </div>

        <!-- Search results -->
        <div v-else-if="results.length > 0" class="space-y-3 sm:space-y-4">
          <div class="text-xs sm:text-sm text-nanna-text-dim mb-3 sm:mb-4">
            Found {{ results.length }} results for "{{ searchQuery }}"
          </div>
          
          <div
            v-for="result in results"
            :key="result.message_id"
            class="bg-nanna-bg-elevated rounded-lg p-3 sm:p-4 hover:bg-nanna-bg-surface transition-colors cursor-pointer"
            @click="goToSession(result)"
          >
            <div class="flex items-start justify-between mb-2 gap-2">
              <div class="flex items-center gap-2 min-w-0">
                <UiAvatar 
                  size="sm"
                  :class="result.role === 'user' ? 'bg-nanna-primary' : 'bg-nanna-accent'"
                >
                  {{ result.role === 'user' ? 'U' : 'N' }}
                </UiAvatar>
                <span class="text-xs sm:text-sm font-medium text-nanna-secondary truncate">
                  {{ result.session_name }}
                </span>
              </div>
              <span class="text-xs text-nanna-text-dim shrink-0">
                {{ formatDate(result.timestamp) }}
              </span>
            </div>
            
            <p 
              class="text-nanna-text text-xs sm:text-sm line-clamp-3"
              v-html="highlightMatch(result.snippet, searchQuery)"
            />
          </div>
        </div>

        <!-- No results -->
        <div v-else-if="searchQuery && !isSearching" class="flex items-center justify-center min-h-[300px] sm:min-h-[400px]">
          <div class="text-center px-4">
            <div class="text-4xl mb-4">🔍</div>
            <h3 class="text-lg font-semibold text-nanna-text mb-2">No results found</h3>
            <p class="text-sm text-nanna-text-muted">Try a different search term</p>
          </div>
        </div>
      </div>
    </div>

    <!-- Semantic Memory Tab Content -->
    <div v-else-if="activeTab === 'semantic'" class="flex-1 overflow-y-auto">
      <!-- Stats bar -->
      <div class="px-4 sm:px-6 py-3 sm:py-4 border-b border-nanna-primary/10 flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
        <div v-if="cognitiveStats" class="flex flex-wrap gap-3 sm:gap-4 text-xs sm:text-sm">
          <span class="text-nanna-text-dim">
            <span class="text-nanna-success font-medium">{{ cognitiveStats.active }}</span> active
          </span>
          <span class="text-nanna-text-dim">
            <span class="text-nanna-warning font-medium">{{ cognitiveStats.dormant }}</span> dormant
          </span>
          <span class="text-nanna-text-dim">
            <span class="text-nanna-text-muted font-medium">{{ cognitiveStats.silent }}</span> silent
          </span>
        </div>
        <div class="flex gap-2">
          <UiButton 
            @click="triggerConsolidation" 
            variant="secondary"
            size="sm"
            :disabled="isConsolidating"
          >
            {{ isConsolidating ? '💭 Dreaming...' : '💭 Consolidate' }}
          </UiButton>
          <UiButton 
            v-if="semanticMemories.length > 0"
            @click="showClearConfirm = true" 
            variant="destructive"
            size="sm"
          >
            🗑️ Clear All
          </UiButton>
        </div>
      </div>

      <!-- Memory list -->
      <div class="p-4 sm:p-6">
        <!-- Empty state -->
        <div v-if="semanticMemories.length === 0" class="flex items-center justify-center min-h-[300px] sm:min-h-[400px]">
          <div class="text-center max-w-md px-4">
            <div class="text-5xl sm:text-6xl mb-4">🧠</div>
            <h3 class="text-lg sm:text-xl font-semibold text-nanna-text mb-2">
              No Memories Yet
            </h3>
            <p class="text-sm text-nanna-text-muted">
              As you chat, Nanna will automatically extract and remember important facts about you.
            </p>
          </div>
        </div>

        <!-- Memory cards -->
        <div v-else class="space-y-3">
          <UiCard
            v-for="memory in semanticMemories"
            :key="memory.id"
            class="p-3 sm:p-4 group"
          >
            <div class="flex items-start justify-between gap-3 sm:gap-4">
              <div class="flex-1 min-w-0">
                <!-- Content (editable) -->
                <div v-if="editingId === memory.id" class="mb-2">
                  <UiTextarea
                    v-model="editContent"
                    class="text-sm"
                    :rows="2"
                    @keydown.escape="cancelEdit"
                  />
                  <div class="flex gap-2 mt-2">
                    <UiButton @click="saveEdit(memory.id)" size="sm">Save</UiButton>
                    <UiButton @click="cancelEdit" variant="secondary" size="sm">Cancel</UiButton>
                  </div>
                </div>
                <p v-else class="text-nanna-text text-xs sm:text-sm mb-2">{{ memory.content }}</p>
                
                <!-- Metadata -->
                <div class="flex flex-wrap items-center gap-1.5 sm:gap-2 text-xs">
                  <UiBadge :variant="memory.fact_type === 'observed' ? 'warning' : 'accent'">
                    {{ memory.fact_type === 'observed' ? '👁️ observed' : '💬 stated' }}
                  </UiBadge>
                  <UiBadge :variant="
                    memory.state === 'active' ? 'success' :
                    memory.state === 'dormant' ? 'warning' : 'secondary'
                  ">
                    {{ memory.state }}
                  </UiBadge>
                  <span class="text-nanna-text-dim hidden sm:inline">
                    importance: {{ memory.importance.toFixed(1) }}
                  </span>
                  <span class="text-nanna-text-dim hidden sm:inline">
                    recall: {{ (memory.retrievability * 100).toFixed(0) }}%
                  </span>
                  <span class="text-nanna-text-dim">
                    {{ memory.created_at }}
                  </span>
                </div>
              </div>
              
              <!-- Actions -->
              <div class="flex gap-1 sm:opacity-0 sm:group-hover:opacity-100 transition-opacity shrink-0">
                <UiButton 
                  v-if="editingId !== memory.id"
                  @click="startEdit(memory)" 
                  variant="ghost"
                  size="sm"
                  title="Edit"
                >
                  ✏️
                </UiButton>
                <UiButton 
                  @click="deleteMemory(memory.id)" 
                  variant="ghost"
                  size="sm"
                  title="Delete"
                  class="hover:text-nanna-error"
                >
                  🗑️
                </UiButton>
              </div>
            </div>
          </UiCard>
        </div>
      </div>
    </div>

    <!-- Clear confirmation modal -->
    <div v-if="showClearConfirm" class="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
      <UiCard class="max-w-md w-full">
        <h3 class="text-lg font-semibold text-nanna-text mb-2">Clear All Memories?</h3>
        <p class="text-sm text-nanna-text-muted mb-4">
          This will permanently delete all {{ semanticMemories.length }} memories. This action cannot be undone.
        </p>
        <div class="flex gap-2 justify-end">
          <UiButton @click="showClearConfirm = false" variant="secondary">Cancel</UiButton>
          <UiButton @click="clearAllMemories" variant="destructive">Clear All</UiButton>
        </div>
      </UiCard>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

// Types
interface MemorySearchResult {
  session_id: string
  session_name: string
  message_id: string
  role: string
  content: string
  timestamp: string
  snippet: string
  relevance: number
}

interface MemoryStats {
  total_sessions: number
  total_messages: number
  oldest_session: string | null
  newest_session: string | null
}

interface CognitiveStats {
  total_memories: number
  active: number
  dormant: number
  silent: number
  unavailable: number
  consolidation_enabled: boolean
  last_consolidation: string | null
}

interface SemanticMemory {
  id: string
  content: string
  fact_type: string
  importance: number
  state: string
  weight: number
  retrievability: number
  access_count: number
  created_at: string
  session_id: string | null
}

// State
const activeTab = ref<'history' | 'semantic'>('semantic')
const searchQuery = ref('')
const results = ref<MemorySearchResult[]>([])
const stats = ref<MemoryStats | null>(null)
const cognitiveStats = ref<CognitiveStats | null>(null)
const semanticMemories = ref<SemanticMemory[]>([])
const isSearching = ref(false)
const isConsolidating = ref(false)
const showClearConfirm = ref(false)
const editingId = ref<string | null>(null)
const editContent = ref('')

let debounceTimer: ReturnType<typeof setTimeout> | null = null

// Load data on mount
onMounted(async () => {
  await Promise.all([
    loadHistoryStats(),
    loadCognitiveStats(),
    loadSemanticMemories(),
  ])
})

async function loadHistoryStats() {
  try {
    stats.value = await invoke<MemoryStats>('get_memory_stats')
  } catch (e) {
    console.error('Failed to load history stats:', e)
  }
}

async function loadCognitiveStats() {
  try {
    cognitiveStats.value = await invoke<CognitiveStats>('get_cognitive_memory_stats')
  } catch (e) {
    console.error('Failed to load cognitive stats:', e)
  }
}

async function loadSemanticMemories() {
  try {
    semanticMemories.value = await invoke<SemanticMemory[]>('list_memories')
  } catch (e) {
    console.error('Failed to load semantic memories:', e)
  }
}

// History search
function debouncedSearch() {
  if (debounceTimer) clearTimeout(debounceTimer)
  debounceTimer = setTimeout(performSearch, 300)
}

async function performSearch() {
  if (!searchQuery.value.trim()) {
    results.value = []
    return
  }

  isSearching.value = true
  try {
    results.value = await invoke<MemorySearchResult[]>('search_memory', {
      query: searchQuery.value,
      limit: 50,
    })
  } catch (e) {
    console.error('Search failed:', e)
    results.value = []
  } finally {
    isSearching.value = false
  }
}

// Memory management
async function triggerConsolidation() {
  isConsolidating.value = true
  try {
    await invoke('trigger_consolidation')
    await loadCognitiveStats()
    await loadSemanticMemories()
  } catch (e) {
    console.error('Consolidation failed:', e)
  } finally {
    isConsolidating.value = false
  }
}

function startEdit(memory: SemanticMemory) {
  editingId.value = memory.id
  editContent.value = memory.content
}

function cancelEdit() {
  editingId.value = null
  editContent.value = ''
}

async function saveEdit(id: string) {
  try {
    await invoke('update_memory', { id, content: editContent.value })
    await loadSemanticMemories()
    cancelEdit()
  } catch (e) {
    console.error('Failed to update memory:', e)
  }
}

async function deleteMemory(id: string) {
  try {
    await invoke('delete_memory', { id })
    await loadSemanticMemories()
    await loadCognitiveStats()
  } catch (e) {
    console.error('Failed to delete memory:', e)
  }
}

async function clearAllMemories() {
  try {
    await invoke('clear_all_memories')
    await loadSemanticMemories()
    await loadCognitiveStats()
    showClearConfirm.value = false
  } catch (e) {
    console.error('Failed to clear memories:', e)
  }
}

// Helpers
function formatDate(timestamp: string): string {
  try {
    const date = new Date(timestamp)
    const now = new Date()
    const diff = now.getTime() - date.getTime()
    const days = Math.floor(diff / (1000 * 60 * 60 * 24))
    
    if (days === 0) {
      return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    } else if (days === 1) {
      return 'Yesterday'
    } else if (days < 7) {
      return `${days} days ago`
    } else {
      return date.toLocaleDateString()
    }
  } catch {
    return timestamp
  }
}

function highlightMatch(text: string, query: string): string {
  if (!query) return text
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const regex = new RegExp(`(${escaped})`, 'gi')
  return text.replace(regex, '<mark class="bg-nanna-warning/30 text-nanna-warning px-0.5 rounded">$1</mark>')
}

function goToSession(result: MemorySearchResult) {
  navigateTo(`/?session=${result.session_id}`)
}
</script>
