<script setup lang="ts">
import { ref, computed } from 'vue'
import { AlertCircle, AlertTriangle, Info, CheckCircle, Copy, Check, Trash2, CheckCheck, X } from 'lucide-vue-next'
import type { NotificationType, AppNotification } from '~/composables/useNotificationCenter'

const {
  notifications,
  unreadCount,
  markRead,
  markAllRead,
  clearAll,
  removeNotification,
  copyAsMarkdown,
  relativeTime,
} = useNotificationCenter()

const filter = ref<'all' | NotificationType>('all')
const expandedId = ref<string | null>(null)
const copiedId = ref<string | null>(null)

const filteredNotifications = computed(() => {
  if (filter.value === 'all') return notifications.value
  return notifications.value.filter(n => n.type === filter.value)
})

const filterCounts = computed(() => ({
  all: notifications.value.length,
  error: notifications.value.filter(n => n.type === 'error').length,
  warning: notifications.value.filter(n => n.type === 'warning').length,
  info: notifications.value.filter(n => n.type === 'info').length,
  success: notifications.value.filter(n => n.type === 'success').length,
}))

function toggleExpand(n: AppNotification) {
  if (expandedId.value === n.id) {
    expandedId.value = null
  } else {
    expandedId.value = n.id
    if (!n.read) markRead(n.id)
  }
}

async function copyNotification(id: string) {
  const md = copyAsMarkdown(id)
  if (!md) return
  try {
    await navigator.clipboard.writeText(md)
    copiedId.value = id
    setTimeout(() => { copiedId.value = null }, 2000)
  } catch (e) {
    console.error('Failed to copy:', e)
  }
}

const typeIcon: Record<NotificationType, any> = {
  error: AlertCircle,
  warning: AlertTriangle,
  info: Info,
  success: CheckCircle,
}

const typeColor: Record<NotificationType, string> = {
  error: 'text-rose-400',
  warning: 'text-amber-400',
  info: 'text-cyan-400',
  success: 'text-emerald-400',
}

const typeBg: Record<NotificationType, string> = {
  error: 'bg-rose-400/10',
  warning: 'bg-amber-400/10',
  info: 'bg-cyan-400/10',
  success: 'bg-emerald-400/10',
}

const typeBorder: Record<NotificationType, string> = {
  error: 'border-rose-400/20',
  warning: 'border-amber-400/20',
  info: 'border-cyan-400/20',
  success: 'border-emerald-400/20',
}
</script>

<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <div class="flex items-center justify-between pb-4 border-b border-white/[0.06]">
      <div class="flex items-center gap-2">
        <h2 class="text-sm font-medium text-slate-200">Notifications</h2>
        <span
          v-if="unreadCount > 0"
          class="inline-flex items-center justify-center min-w-[18px] h-[18px] px-1 text-[10px] font-medium rounded-full bg-violet-500 text-white"
        >
          {{ unreadCount }}
        </span>
      </div>
      <div class="flex items-center gap-1">
        <button
          v-if="unreadCount > 0"
          @click="markAllRead"
          class="p-1.5 rounded-md text-slate-400 hover:text-slate-200 hover:bg-white/[0.04] transition-colors"
          title="Mark all read"
        >
          <CheckCheck class="w-3.5 h-3.5" />
        </button>
        <button
          v-if="notifications.length > 0"
          @click="clearAll"
          class="p-1.5 rounded-md text-slate-400 hover:text-slate-200 hover:bg-white/[0.04] transition-colors"
          title="Clear all"
        >
          <Trash2 class="w-3.5 h-3.5" />
        </button>
      </div>
    </div>

    <!-- Filter bar -->
    <div class="flex gap-1 py-2 border-b border-white/[0.04]">
      <button
        v-for="f in (['all', 'error', 'warning', 'info', 'success'] as const)"
        :key="f"
        @click="filter = f"
        :class="[
          'px-2 py-0.5 text-[10px] rounded-md transition-colors',
          filter === f
            ? 'bg-violet-500/20 text-violet-300'
            : 'text-slate-500 hover:text-slate-300 hover:bg-white/[0.04]'
        ]"
      >
        {{ f === 'all' ? 'All' : f.charAt(0).toUpperCase() + f.slice(1) }}
        <span v-if="filterCounts[f] > 0" class="ml-1 opacity-60">{{ filterCounts[f] }}</span>
      </button>
    </div>

    <!-- Notification list -->
    <div class="flex-1 overflow-y-auto mt-2 space-y-1.5">
      <div
        v-for="n in filteredNotifications"
        :key="n.id"
        :class="[
          'rounded-lg border p-2.5 cursor-pointer transition-colors',
          typeBorder[n.type],
          expandedId === n.id ? typeBg[n.type] : 'bg-slate-800/50 hover:bg-slate-800/80',
          !n.read ? 'ring-1 ring-violet-500/30' : ''
        ]"
        @click="toggleExpand(n)"
      >
        <!-- Top row: icon + title + time + actions -->
        <div class="flex items-start gap-2">
          <component
            :is="typeIcon[n.type]"
            :class="['w-4 h-4 mt-0.5 shrink-0', typeColor[n.type]]"
          />
          <div class="flex-1 min-w-0">
            <div class="flex items-center justify-between gap-2">
              <span class="text-xs font-medium text-slate-200 truncate">{{ n.title }}</span>
              <span class="text-[10px] text-slate-500 shrink-0">{{ relativeTime(n.timestamp) }}</span>
            </div>
            <p class="text-[11px] text-slate-400 mt-0.5 line-clamp-2">{{ n.summary }}</p>
            <!-- Source badge -->
            <span class="inline-block mt-1 px-1.5 py-0.5 text-[9px] rounded bg-slate-700/50 text-slate-500">
              {{ n.source }}
            </span>
          </div>
          <!-- Unread dot -->
          <span
            v-if="!n.read"
            class="w-2 h-2 rounded-full bg-violet-500 shrink-0 mt-1"
          />
        </div>

        <!-- Expanded detail -->
        <Transition name="expand">
          <div v-if="expandedId === n.id" class="mt-2 pt-2 border-t border-white/[0.06]">
            <!-- Detail text -->
            <div v-if="n.detail && n.detail !== n.summary" class="mb-2">
              <div class="text-[10px] text-slate-500 mb-1 uppercase tracking-wider">Detail</div>
              <pre class="text-[11px] text-slate-300 bg-slate-900/60 rounded p-2 overflow-x-auto whitespace-pre-wrap font-mono max-h-[200px] overflow-y-auto">{{ n.detail }}</pre>
            </div>

            <!-- Metadata -->
            <div v-if="n.metadata && Object.keys(n.metadata).length > 0" class="mb-2">
              <div class="text-[10px] text-slate-500 mb-1 uppercase tracking-wider">Metadata</div>
              <div class="space-y-0.5">
                <div
                  v-for="(value, key) in n.metadata"
                  :key="String(key)"
                  class="text-[11px]"
                >
                  <span class="text-slate-500">{{ key }}:</span>
                  <span class="text-slate-300 ml-1 font-mono">{{ typeof value === 'object' ? JSON.stringify(value) : value }}</span>
                </div>
              </div>
            </div>

            <!-- Actions -->
            <div class="flex items-center gap-2 mt-2" @click.stop>
              <button
                @click="copyNotification(n.id)"
                class="flex items-center gap-1 px-2 py-1 text-[10px] rounded bg-slate-700/50 text-slate-400 hover:text-slate-200 hover:bg-slate-700 transition-colors"
              >
                <component :is="copiedId === n.id ? Check : Copy" class="w-3 h-3" />
                {{ copiedId === n.id ? 'Copied!' : 'Copy as Markdown' }}
              </button>
              <button
                @click="removeNotification(n.id)"
                class="flex items-center gap-1 px-2 py-1 text-[10px] rounded bg-slate-700/50 text-slate-400 hover:text-rose-300 hover:bg-rose-500/10 transition-colors"
              >
                <X class="w-3 h-3" />
                Dismiss
              </button>
            </div>
          </div>
        </Transition>
      </div>

      <!-- Empty state -->
      <div
        v-if="filteredNotifications.length === 0"
        class="text-center py-12"
      >
        <Info class="w-8 h-8 mx-auto mb-2 text-slate-600" />
        <p class="text-xs text-slate-500">
          {{ filter === 'all' ? 'No notifications yet' : `No ${filter} notifications` }}
        </p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.expand-enter-active,
.expand-leave-active {
  transition: all 0.2s ease;
  overflow: hidden;
}
.expand-enter-from,
.expand-leave-to {
  opacity: 0;
  max-height: 0;
}
.expand-enter-to,
.expand-leave-from {
  opacity: 1;
  max-height: 500px;
}

.line-clamp-2 {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
</style>
