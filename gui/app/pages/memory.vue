<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <header class="px-6 py-4 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
      <div class="flex items-center justify-between">
        <div>
          <h2 class="text-lg font-semibold text-nanna-text">Memory Browser</h2>
          <p class="text-sm text-nanna-text-muted">
            Search and browse conversation history
          </p>
        </div>
        <div v-if="stats" class="text-sm text-nanna-text-dim">
          {{ stats.total_sessions }} sessions • {{ stats.total_messages }} messages
        </div>
      </div>
    </header>

    <!-- Search bar -->
    <div class="px-6 py-4 border-b border-nanna-primary/10">
      <form @submit.prevent="performSearch" class="flex gap-3">
        <div class="relative flex-1">
          <input
            v-model="searchQuery"
            type="text"
            placeholder="Search conversations..."
            class="input w-full pl-10"
            @input="debouncedSearch"
          />
          <span class="absolute left-3 top-1/2 -translate-y-1/2 text-nanna-text-dim">
            🔍
          </span>
        </div>
        <button type="submit" class="btn-primary" :disabled="isSearching">
          {{ isSearching ? 'Searching...' : 'Search' }}
        </button>
      </form>
    </div>

    <!-- Content area -->
    <div class="flex-1 overflow-y-auto p-6">
      <!-- Empty state -->
      <div v-if="!searchQuery && results.length === 0" class="flex items-center justify-center h-full">
        <div class="text-center max-w-md">
          <div class="text-6xl mb-4">📚</div>
          <h3 class="text-xl font-semibold text-nanna-text mb-2">
            Memory Browser
          </h3>
          <p class="text-nanna-text-muted mb-6">
            Search through all your conversations to find past discussions,
            decisions, and information.
          </p>
          <div v-if="stats" class="grid grid-cols-2 gap-4 text-sm">
            <div class="bg-nanna-bg-elevated p-4 rounded-lg">
              <div class="text-2xl font-bold text-nanna-accent">{{ stats.total_sessions }}</div>
              <div class="text-nanna-text-dim">Sessions</div>
            </div>
            <div class="bg-nanna-bg-elevated p-4 rounded-lg">
              <div class="text-2xl font-bold text-nanna-secondary">{{ stats.total_messages }}</div>
              <div class="text-nanna-text-dim">Messages</div>
            </div>
          </div>
        </div>
      </div>

      <!-- Search results -->
      <div v-else-if="results.length > 0" class="space-y-4">
        <div class="text-sm text-nanna-text-dim mb-4">
          Found {{ results.length }} results for "{{ searchQuery }}"
        </div>
        
        <div
          v-for="result in results"
          :key="result.message_id"
          class="bg-nanna-bg-elevated rounded-lg p-4 hover:bg-nanna-bg-surface transition-colors cursor-pointer"
          @click="goToSession(result)"
        >
          <div class="flex items-start justify-between mb-2">
            <div class="flex items-center gap-2">
              <span :class="[
                'w-6 h-6 rounded-full flex items-center justify-center text-xs',
                result.role === 'user' 
                  ? 'bg-nanna-primary text-white' 
                  : 'bg-nanna-accent text-nanna-bg-deep'
              ]">
                {{ result.role === 'user' ? 'U' : 'N' }}
              </span>
              <span class="text-sm font-medium text-nanna-secondary">
                {{ result.session_name }}
              </span>
            </div>
            <span class="text-xs text-nanna-text-dim">
              {{ formatDate(result.timestamp) }}
            </span>
          </div>
          
          <p 
            class="text-nanna-text text-sm"
            v-html="highlightMatch(result.snippet, searchQuery)"
          />
          
          <div class="mt-2 flex items-center gap-2 text-xs text-nanna-text-dim">
            <span class="px-2 py-0.5 bg-nanna-bg-deep rounded">
              {{ result.role }}
            </span>
            <span v-if="result.relevance > 0.01" class="px-2 py-0.5 bg-nanna-accent/20 text-nanna-accent rounded">
              {{ Math.round(result.relevance * 100) }}% match
            </span>
          </div>
        </div>
      </div>

      <!-- No results -->
      <div v-else-if="searchQuery && !isSearching" class="flex items-center justify-center h-full">
        <div class="text-center">
          <div class="text-4xl mb-4">🔍</div>
          <h3 class="text-lg font-semibold text-nanna-text mb-2">No results found</h3>
          <p class="text-nanna-text-muted">
            Try a different search term
          </p>
        </div>
      </div>

      <!-- Loading -->
      <div v-else-if="isSearching" class="flex items-center justify-center h-full">
        <div class="text-center">
          <div class="animate-spin text-4xl mb-4">⏳</div>
          <p class="text-nanna-text-muted">Searching...</p>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

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

const searchQuery = ref('')
const results = ref<MemorySearchResult[]>([])
const stats = ref<MemoryStats | null>(null)
const isSearching = ref(false)

let debounceTimer: ReturnType<typeof setTimeout> | null = null

onMounted(async () => {
  try {
    stats.value = await invoke<MemoryStats>('get_memory_stats')
  } catch (e) {
    console.error('Failed to load stats:', e)
  }
})

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
  // Store the session ID and navigate
  // In a real implementation, this would switch to that session
  navigateTo(`/?session=${result.session_id}`)
}
</script>
