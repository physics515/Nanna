<template>
  <!-- The chat's window into the task store (P19): the planner seeds it, the
       harness drains it, the agent's todo skill writes to it. This is a live
       view of the run's checklist, not a task-management UI — mutations
       happen through chat. Hidden entirely when the session has no tasks. -->
  <aside v-if="tasks.length > 0" class="task-checklist" :class="collapsed ? 'w-10' : 'w-64 xl:w-72'">
    <!-- Collapsed rail -->
    <button
      v-if="collapsed"
      class="rail glass-tag"
      :title="`Tasks: ${doneCount}/${tasks.length} done — click to expand`"
      @click="collapsed = false"
    >
      <ListChecks class="w-4 h-4" />
      <span class="rail-count">{{ doneCount }}/{{ tasks.length }}</span>
    </button>

    <!-- Expanded panel -->
    <div v-else class="panel glass-panel">
      <div class="panel-header">
        <div class="flex items-center gap-2 min-w-0">
          <ListChecks class="w-4 h-4 text-nanna-primary shrink-0" />
          <span class="text-sm font-medium text-nanna-text truncate">Tasks</span>
          <span class="text-xs font-mono text-nanna-text-dim">{{ doneCount }}/{{ tasks.length }}</span>
        </div>
        <button
          class="text-nanna-text-dim hover:text-nanna-text transition-colors"
          title="Collapse"
          @click="collapsed = true"
        >
          <ChevronRight class="w-4 h-4" />
        </button>
      </div>

      <div class="panel-body">
        <div
          v-for="task in tasks"
          :key="task.id"
          class="task-row"
          :class="{ 'pl-6': task.parent_id != null, 'opacity-50': task.status === 'cancelled' }"
          :title="task.description || task.title"
        >
          <!-- Status glyph -->
          <span class="glyph" :class="glyphClass(task.status)">
            <CheckCircle2 v-if="task.status === 'done'" class="w-3.5 h-3.5" />
            <Loader2 v-else-if="task.status === 'in_progress'" class="w-3.5 h-3.5 animate-spin" />
            <XCircle v-else-if="task.status === 'cancelled'" class="w-3.5 h-3.5" />
            <Circle v-else class="w-3.5 h-3.5" />
          </span>
          <span
            class="task-title"
            :class="{
              'line-through text-nanna-text-dim': task.status === 'done' || task.status === 'cancelled',
              'text-nanna-text': task.status === 'in_progress',
            }"
          >{{ task.title }}</span>
          <span v-if="task.priority === 1 && task.status !== 'done'" class="prio" title="Interjected / urgent">!</span>
        </div>
      </div>
    </div>
  </aside>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { CheckCircle2, ChevronRight, Circle, ListChecks, Loader2, XCircle } from 'lucide-vue-next'

interface TaskItem {
  id: number
  parent_id: number | null
  title: string
  description?: string | null
  status: string
  priority: number
  completed_at?: string | null
}

const props = defineProps<{ sessionId: string }>()

const tasks = ref<TaskItem[]>([])
const collapsed = ref(false)
const doneCount = computed(() => tasks.value.filter(t => t.status === 'done').length)

function glyphClass(status: string): string {
  switch (status) {
    case 'done': return 'text-emerald-400'
    case 'in_progress': return 'text-nanna-primary'
    case 'cancelled': return 'text-nanna-text-dim'
    default: return 'text-nanna-text-muted'
  }
}

async function load() {
  const sessionId = props.sessionId
  if (!sessionId) { tasks.value = []; return }
  try {
    const res = await invoke<{ tasks?: TaskItem[]; error?: string }>('list_tasks', {
      scope: 'session',
      sessionId,
      includeClosed: true,
    })
    // Session switched while the request was in flight — drop the stale result.
    if (props.sessionId !== sessionId) return
    tasks.value = res?.tasks ?? []
  } catch {
    // Daemon unreachable: keep whatever we have; the next event retries.
  }
}

// Refetch on task events for this session, debounced — a run completing
// several items in a burst should cost one reload, not five.
let reloadTimer: ReturnType<typeof setTimeout> | null = null
function scheduleReload() {
  if (reloadTimer) clearTimeout(reloadTimer)
  reloadTimer = setTimeout(() => { reloadTimer = null; void load() }, 250)
}

let unlisten: UnlistenFn | null = null
onMounted(async () => {
  await load()
  unlisten = await listen<{ scope?: string; scope_id?: string | null }>('task-event', (event) => {
    const { scope, scope_id } = event.payload ?? {}
    if (scope === 'session' && scope_id && scope_id !== props.sessionId) return
    scheduleReload()
  })
})
onUnmounted(() => {
  unlisten?.()
  if (reloadTimer) clearTimeout(reloadTimer)
})

watch(() => props.sessionId, () => { tasks.value = []; void load() })
</script>

<style scoped>
.task-checklist {
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  padding: 0.5rem 0.75rem 0.75rem 0;
  transition: width 0.15s ease;
  min-height: 0;
}
.rail {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 10px 6px;
  border-radius: 9999px;
  color: var(--color-nanna-text-dim, rgba(148, 163, 184, 0.8));
  cursor: pointer;
}
.rail:hover {
  color: var(--color-nanna-text, #e2e8f0);
}
.rail-count {
  font-size: 10px;
  font-family: var(--font-mono, monospace);
  writing-mode: vertical-rl;
}
.panel {
  display: flex;
  flex-direction: column;
  border-radius: 0.75rem;
  min-height: 0;
  max-height: 100%;
  overflow: hidden;
}
.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 10px 12px;
  border-bottom: 1px solid rgba(148, 163, 184, 0.1);
}
.panel-body {
  overflow-y: auto;
  padding: 6px 6px 10px;
}
.task-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 5px 8px;
  border-radius: 0.5rem;
  font-size: 13px;
  line-height: 1.35;
}
.task-row:hover {
  background: rgba(148, 163, 184, 0.06);
}
.glyph {
  flex-shrink: 0;
  margin-top: 1px;
}
.task-title {
  min-width: 0;
  overflow-wrap: anywhere;
  color: var(--color-nanna-text-muted, rgba(203, 213, 225, 0.85));
}
.prio {
  margin-left: auto;
  flex-shrink: 0;
  font-weight: 700;
  font-size: 11px;
  color: rgb(251 191 36);
}
</style>
