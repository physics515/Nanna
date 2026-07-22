<template>
  <div class="flex h-full">
    <!-- Main: Task tree -->
    <main class="flex-1 flex flex-col min-w-0">
      <!-- Header -->
      <header class="px-4 py-2 border-b border-white/[0.04] bg-nanna-bg-surface/80">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-3">
            <div class="flex items-center gap-2">
              <ListChecks class="w-5 h-5 text-nanna-accent" />
              <h2 class="font-semibold text-nanna-text text-sm">Tasks</h2>
            </div>
            <!-- Scope selector -->
            <div class="flex rounded-lg overflow-hidden border border-white/[0.06]">
              <button
                v-for="s in scopes"
                :key="s.id"
                @click="scope = s.id"
                class="px-3 py-1 text-xs transition-colors"
                :class="scope === s.id
                  ? 'bg-nanna-accent/20 text-nanna-accent'
                  : 'text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-primary/10'"
              >
                {{ s.label }}
              </button>
            </div>
            <label class="flex items-center gap-1.5 text-xs text-nanna-text-muted cursor-pointer select-none">
              <input type="checkbox" v-model="includeClosed" class="accent-nanna-accent" />
              Show closed
            </label>
          </div>
          <div class="flex items-center gap-2">
            <button @click="refreshTasks" class="p-1 rounded hover:bg-nanna-primary/20 text-nanna-text-muted hover:text-nanna-text transition-colors" title="Refresh">
              <RefreshCw class="w-4 h-4" :class="{ 'animate-spin': refreshing }" />
            </button>
            <UiButton @click="showCreateModal = true" size="sm">
              <Plus class="w-4 h-4 mr-1" /> New Task
            </UiButton>
          </div>
        </div>
      </header>

      <!-- Filter -->
      <div class="px-4 py-2 border-b border-white/[0.04] bg-nanna-bg-surface/30 flex items-center gap-2">
        <UiInput
          v-model="filterText"
          placeholder="Filter query (e.g. p1 & !done, @bug, overdue, search: parser)"
          size="sm"
          class="flex-1 text-xs font-mono"
          @keyup.enter="applyFilter"
        />
        <UiButton @click="applyFilter" variant="ghost" size="sm" :disabled="!filterText.trim()">Filter</UiButton>
        <UiButton v-if="filterActive" @click="clearFilter" variant="ghost" size="sm" class="text-nanna-text-muted" title="Clear filter">
          <X class="w-4 h-4" />
        </UiButton>
      </div>

      <!-- Tree -->
      <div class="flex-1 overflow-y-auto p-2">
        <div v-if="loading" class="flex items-center justify-center py-8">
          <Loader2 class="w-5 h-5 animate-spin text-nanna-text-muted" />
        </div>

        <PageState
          v-else-if="!isOnline || loadError || treeRows.length === 0"
          :state="!isOnline ? 'offline' : (loadError ? 'error' : 'empty')"
          :title="!isOnline ? 'Daemon offline' : (loadError ? 'Could not load tasks' : (filterActive ? 'No tasks match this filter' : 'No tasks yet'))"
          :description="!isOnline
            ? 'The task store lives in Turso on the daemon. Reconnect to plan or resume a run.'
            : (loadError || (filterActive ? 'Try clearing the filter.' : 'Create a task to start a long-horizon plan.'))"
          :primary-action="!isOnline || loadError ? 'Retry' : (filterActive ? '' : 'Create task')"
          @primary="onTasksPrimary"
          compact
        />

        <div v-else class="space-y-0.5">
          <div
            v-for="row in treeRows"
            :key="row.task.id"
            @click="selectTask(row.task)"
            class="group flex items-center gap-2 pr-2 py-1.5 rounded cursor-pointer transition-colors"
            :style="{ paddingLeft: `${8 + row.depth * 20}px` }"
            :class="selectedTask?.id === row.task.id ? 'bg-nanna-accent/20' : 'hover:bg-nanna-primary/10'"
          >
            <span class="text-sm flex-shrink-0" :title="row.task.status">{{ statusIcon[row.task.status] || '⬜' }}</span>
            <span class="text-[10px] text-nanna-text-dim font-mono flex-shrink-0">#{{ row.task.id }}</span>
            <span
              class="text-sm truncate flex-1"
              :class="row.task.status === 'done' || row.task.status === 'cancelled'
                ? 'text-nanna-text-dim line-through'
                : 'text-nanna-text'"
            >{{ row.task.title }}</span>
            <span v-if="row.task.blocked" class="px-1.5 py-0.5 rounded text-[10px] bg-red-500/15 text-red-400 flex-shrink-0" title="Blocked by dependencies">⛔ blocked</span>
            <span class="px-1.5 py-0.5 rounded text-[10px] flex-shrink-0" :class="priorityClass(row.task.priority)">p{{ row.task.priority }}</span>
            <span
              v-for="label in row.task.labels"
              :key="label"
              class="px-1.5 py-0.5 rounded text-[10px] bg-nanna-primary/15 text-nanna-text-muted flex-shrink-0"
            >{{ label }}</span>
            <span v-if="row.task.due_at" class="text-[10px] text-nanna-text-dim flex-shrink-0" :title="row.task.due_at">
              due {{ formatDate(row.task.due_at) }}
            </span>
            <!-- Quick actions -->
            <div class="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
              <button
                v-if="row.task.status !== 'done' && row.task.status !== 'cancelled'"
                @click.stop="completeTask(row.task)"
                class="p-1 rounded hover:bg-nanna-primary/20 text-nanna-text-muted hover:text-green-400"
                title="Complete"
              >
                <Check class="w-3.5 h-3.5" />
              </button>
              <button
                @click.stop="removeTask(row.task)"
                class="p-1 rounded hover:bg-nanna-primary/20 text-nanna-text-muted hover:text-red-400"
                title="Delete"
              >
                <Trash2 class="w-3.5 h-3.5" />
              </button>
            </div>
          </div>
        </div>
      </div>

      <footer class="px-4 py-2 border-t border-white/[0.04] text-[10px] text-nanna-text-dim">
        {{ tasks.length }} tasks{{ filterActive ? ' (filtered)' : '' }} · scope: {{ scope }}
      </footer>
    </main>

    <!-- Right panel: details + run -->
    <aside class="w-96 border-l border-white/[0.04] bg-nanna-bg-surface/80 flex flex-col min-h-0">
      <!-- Task details -->
      <section class="flex-1 overflow-y-auto border-b border-white/[0.04] min-h-0">
        <div v-if="!selectedTask" class="p-4 py-10 text-center text-sm text-nanna-text-muted">
          Select a task to see details
        </div>
        <div v-else class="p-4 space-y-4">
          <div class="flex items-start justify-between gap-2">
            <div class="min-w-0">
              <div class="flex items-center gap-2">
                <span class="text-sm">{{ statusIcon[selectedTask.status] || '⬜' }}</span>
                <span class="font-mono text-xs text-nanna-text-dim">#{{ selectedTask.id }}</span>
                <span class="px-1.5 py-0.5 rounded text-[10px]" :class="priorityClass(selectedTask.priority)">p{{ selectedTask.priority }}</span>
                <span v-if="selectedTask.blocked" class="px-1.5 py-0.5 rounded text-[10px] bg-red-500/15 text-red-400">⛔ blocked</span>
              </div>
              <h3 class="text-sm font-semibold text-nanna-text mt-1 break-words">{{ selectedTask.title }}</h3>
            </div>
            <button @click="deselectTask" class="p-1 rounded hover:bg-nanna-primary/20 flex-shrink-0">
              <X class="w-4 h-4 text-nanna-text-muted" />
            </button>
          </div>

          <!-- Acceptance verdict (shown after a failed complete) -->
          <div v-if="verdictMsg" class="p-2 rounded bg-amber-500/10 border border-amber-500/30 text-xs text-amber-300 whitespace-pre-wrap">
            {{ verdictMsg }}
          </div>

          <!-- Quick actions -->
          <div class="flex gap-2">
            <UiButton
              v-if="selectedTask.status !== 'done' && selectedTask.status !== 'cancelled'"
              @click="completeTask(selectedTask)"
              size="sm"
              :disabled="actionBusy"
            >
              <Check class="w-4 h-4 mr-1" /> Complete
            </UiButton>
            <UiButton @click="removeTask(selectedTask)" variant="ghost" size="sm" class="text-red-400 hover:text-red-300" :disabled="actionBusy">
              <Trash2 class="w-4 h-4 mr-1" /> Delete
            </UiButton>
          </div>

          <div v-if="loadingDetails" class="flex items-center justify-center py-4">
            <Loader2 class="w-4 h-4 animate-spin text-nanna-text-muted" />
          </div>

          <template v-else>
            <!-- Description -->
            <div v-if="detailTask?.description">
              <h4 class="text-xs font-semibold text-nanna-text mb-1">Description</h4>
              <p class="text-xs text-nanna-text-muted whitespace-pre-wrap">{{ detailTask.description }}</p>
            </div>

            <!-- Acceptance criteria -->
            <div v-if="detailTask?.acceptance">
              <h4 class="text-xs font-semibold text-nanna-text mb-1">Acceptance</h4>
              <pre class="p-2 bg-nanna-bg-elevated/50 rounded text-[11px] font-mono text-nanna-text-muted overflow-x-auto">{{ JSON.stringify(detailTask.acceptance, null, 2) }}</pre>
            </div>

            <!-- Notes -->
            <div>
              <h4 class="text-xs font-semibold text-nanna-text mb-1">Notes</h4>
              <div v-if="taskNotes.length === 0" class="text-xs text-nanna-text-dim">No notes yet</div>
              <div v-else class="space-y-1.5">
                <div v-for="(note, i) in taskNotes" :key="i" class="p-2 bg-nanna-bg-elevated/50 rounded text-xs text-nanna-text-muted">
                  <div class="whitespace-pre-wrap break-words">{{ entryText(note) }}</div>
                  <div v-if="entryTime(note)" class="text-[10px] text-nanna-text-dim mt-0.5">{{ entryTime(note) }}</div>
                </div>
              </div>
              <div class="flex gap-2 mt-2">
                <UiInput v-model="newNote" placeholder="Add a note..." size="sm" class="flex-1 text-xs" @keyup.enter="addNote" />
                <UiButton @click="addNote" variant="ghost" size="sm" :disabled="!newNote.trim() || actionBusy">Add</UiButton>
              </div>
            </div>

            <!-- Activity -->
            <div>
              <h4 class="text-xs font-semibold text-nanna-text mb-1">Activity</h4>
              <div v-if="taskActivity.length === 0" class="text-xs text-nanna-text-dim">No activity yet</div>
              <div v-else class="space-y-1">
                <div v-for="(entry, i) in taskActivity" :key="i" class="text-[11px] text-nanna-text-muted flex gap-2">
                  <span v-if="entryTime(entry)" class="text-nanna-text-dim flex-shrink-0">{{ entryTime(entry) }}</span>
                  <span class="break-words min-w-0">{{ entryText(entry) }}</span>
                </div>
              </div>
            </div>
          </template>
        </div>
      </section>

      <!-- Long-horizon run panel -->
      <section class="flex-1 overflow-y-auto min-h-0 p-4 space-y-3">
        <div class="flex items-center justify-between">
          <h3 class="text-sm font-semibold text-nanna-text">Long-Horizon Run</h3>
          <span v-if="runStatus?.running" class="flex items-center gap-1.5 text-xs text-green-400">
            <span class="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse"></span> running
          </span>
          <span v-else class="text-xs text-nanna-text-dim">idle</span>
        </div>

        <div v-if="runStatus?.running" class="text-xs text-nanna-text-muted space-y-0.5">
          <div class="truncate" :title="runStatus.goal">Goal: {{ runStatus.goal }}</div>
          <div v-if="runStatus.started_at">Started: {{ formatDate(runStatus.started_at) }}</div>
        </div>

        <template v-if="!runStatus?.running">
          <UiTextarea v-model="runGoal" placeholder="What should the agent work toward?" :rows="3" class="text-sm" />
          <UiInput v-model="runMaxTokens" placeholder="Token budget (optional)" size="sm" class="text-xs" />
        </template>

        <div v-if="runError" class="p-2 rounded bg-red-500/10 border border-red-500/30 text-xs text-red-400 whitespace-pre-wrap">
          {{ runError }}
        </div>

        <div class="flex gap-2">
          <UiButton v-if="!runStatus?.running" @click="startRun" size="sm" :disabled="runBusy || !runGoal.trim()">
            <Play class="w-4 h-4 mr-1" /> {{ runBusy ? 'Starting...' : 'Start Run' }}
          </UiButton>
          <UiButton v-else @click="cancelRun" variant="ghost" size="sm" class="text-red-400 hover:text-red-300" :disabled="runBusy">
            <Square class="w-4 h-4 mr-1" /> {{ runBusy ? 'Cancelling...' : 'Cancel Run' }}
          </UiButton>
        </div>

        <!-- Final report -->
        <div v-if="finalReport" class="p-3 rounded-lg bg-nanna-bg-elevated space-y-2">
          <div class="text-xs font-medium text-nanna-text-muted">Last Report</div>
          <div class="grid grid-cols-3 gap-2 text-center">
            <div>
              <div class="text-lg font-semibold text-nanna-text">{{ finalReport.items_completed ?? 0 }}</div>
              <div class="text-[10px] text-nanna-text-dim">completed</div>
            </div>
            <div>
              <div class="text-lg font-semibold text-nanna-text">{{ formatNum(finalReport.tokens_per_completed_item) }}</div>
              <div class="text-[10px] text-nanna-text-dim">tokens / item</div>
            </div>
            <div>
              <div class="text-sm font-semibold text-nanna-text pt-1 break-words">{{ formatStop(finalReport.stop) }}</div>
              <div class="text-[10px] text-nanna-text-dim">stop reason</div>
            </div>
          </div>
          <div class="text-[10px] text-nanna-text-dim">
            steps {{ finalReport.steps_taken ?? 0 }}
            · unverified {{ finalReport.items_completed_unverified ?? 0 }}
            · abandoned {{ finalReport.items_abandoned ?? 0 }}
            · replans {{ finalReport.replans ?? 0 }}
            · tokens {{ (finalReport.input_tokens ?? 0) + (finalReport.output_tokens ?? 0) }}
            · {{ finalReport.wall_clock_secs ?? 0 }}s
          </div>
        </div>

        <!-- Live event feed -->
        <div>
          <div class="text-xs font-medium text-nanna-text-muted mb-1">Events</div>
          <div ref="feedEl" class="h-40 overflow-y-auto p-2 bg-nanna-bg-elevated/50 rounded text-[11px] font-mono space-y-0.5">
            <div v-if="runFeed.length === 0" class="text-nanna-text-dim">No run events yet</div>
            <div v-for="(line, i) in runFeed" :key="i" class="text-nanna-text-muted break-words">
              <span class="text-nanna-text-dim">{{ line.ts }}</span> {{ line.text }}
            </div>
          </div>
        </div>
      </section>
    </aside>

    <!-- Create Task Modal -->
    <UiModal v-model="showCreateModal" title="New Task">
      <div class="space-y-4">
        <div>
          <label class="block text-xs font-medium text-nanna-text-muted mb-1">Title</label>
          <UiInput v-model="newTask.title" placeholder="What needs doing?" class="text-sm" @keyup.enter="createTask" />
        </div>
        <div>
          <label class="block text-xs font-medium text-nanna-text-muted mb-1">Description (optional)</label>
          <UiTextarea v-model="newTask.description" placeholder="Details, context, links..." :rows="2" class="text-sm" />
        </div>
        <div class="flex gap-3">
          <div class="flex-1">
            <label class="block text-xs font-medium text-nanna-text-muted mb-1">Priority</label>
            <select v-model="newTask.priority" class="w-full px-3 py-2 bg-nanna-bg-elevated/30 border border-white/[0.06] rounded-lg text-sm text-nanna-text">
              <option value="1">p1 — urgent</option>
              <option value="2">p2 — high</option>
              <option value="3">p3 — normal</option>
              <option value="4">p4 — low</option>
            </select>
          </div>
          <div class="flex-1 min-w-0">
            <label class="block text-xs font-medium text-nanna-text-muted mb-1">Parent (optional)</label>
            <select v-model="newTask.parentId" class="w-full px-3 py-2 bg-nanna-bg-elevated/30 border border-white/[0.06] rounded-lg text-sm text-nanna-text">
              <option value="">None (top-level)</option>
              <option v-for="t in openTasks" :key="t.id" :value="String(t.id)">#{{ t.id }} {{ t.title }}</option>
            </select>
          </div>
        </div>
        <div>
          <label class="block text-xs font-medium text-nanna-text-muted mb-1">Acceptance criteria JSON (optional)</label>
          <textarea
            v-model="newTask.acceptance"
            class="w-full h-20 p-2 bg-nanna-bg-elevated/30 border border-white/[0.06] rounded text-xs font-mono text-nanna-text resize-none"
            placeholder='{ "kind": "command", "command": "cargo test" }  |  { "kind": "file_exists", "path": "out.txt" }'
          ></textarea>
        </div>
        <div v-if="createError" class="p-2 rounded bg-red-500/10 border border-red-500/30 text-xs text-red-400">
          {{ createError }}
        </div>
        <div class="flex justify-end gap-2 pt-2 border-t border-white/[0.04]">
          <UiButton @click="showCreateModal = false" variant="ghost">Cancel</UiButton>
          <UiButton @click="createTask" :disabled="createBusy || !newTask.title.trim()">
            {{ createBusy ? 'Creating...' : 'Create' }}
          </UiButton>
        </div>
      </div>
    </UiModal>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  Plus, RefreshCw, Loader2, ListChecks, Trash2, Check, Play, Square, X
} from 'lucide-vue-next'

const { isOnline } = useBackend()
const toast = useToast()
const { confirm } = useConfirm()

interface Task {
  id: number
  parent_id: number | null
  scope: string
  scope_id: string | null
  project: string | null
  title: string
  description: string | null
  status: 'pending' | 'in_progress' | 'done' | 'cancelled'
  blocked: boolean
  priority: number
  labels: string[]
  tool_scope: string[]
  due_at: string | null
  recurrence: string | null
  depends_on: number[]
  acceptance: Record<string, unknown> | null
  assignee: string | null
  sort_order: number
  created_at: string
  updated_at: string
  completed_at: string | null
}

interface RunStatus {
  running: boolean
  goal?: string
  started_at?: string
  last_report?: Record<string, any>
}

interface TaskEventPayload {
  kind: 'run_started' | 'run_progress' | 'run_completed'
  scope: string
  scope_id: string | null
  goal?: string
  task_id?: number | null
  progress_kind?: string
  detail?: unknown
  report?: Record<string, any>
}

// State
const scopes = [
  { id: 'workspace', label: 'Workspace' },
  { id: 'global', label: 'Global' },
] as const
const scope = ref<'workspace' | 'global'>('workspace')
const includeClosed = ref(false)

const loading = ref(true)
const loadError = ref<string | null>(null)
const refreshing = ref(false)
const actionBusy = ref(false)
const tasks = ref<Task[]>([])

const filterText = ref('')
const filterActive = ref(false)

const selectedTask = ref<Task | null>(null)
const taskDetails = ref<{ task?: Task; notes?: any[]; activity?: any[] } | null>(null)


const loadingDetails = ref(false)
const verdictMsg = ref<string | null>(null)
const newNote = ref('')

const showCreateModal = ref(false)
const createBusy = ref(false)
const createError = ref<string | null>(null)
const newTask = ref({ title: '', description: '', priority: '3', parentId: '', acceptance: '' })

const runGoal = ref('')
const runMaxTokens = ref('')
const runBusy = ref(false)
const runError = ref<string | null>(null)
const runStatus = ref<RunStatus | null>(null)
const runFeed = ref<{ ts: string; text: string }[]>([])
const finalReport = ref<Record<string, any> | null>(null)
const feedEl = ref<HTMLElement | null>(null)

let unlistenTaskEvent: UnlistenFn | null = null

const statusIcon: Record<string, string> = {
  pending: '⬜',
  in_progress: '🔄',
  done: '✅',
  cancelled: '🚫',
}

// Computed
const treeRows = computed(() => {
  const byParent = new Map<number | null, Task[]>()
  const ids = new Set(tasks.value.map(t => t.id))
  for (const t of tasks.value) {
    // Orphans (parent not in the current result set) render as roots
    const key = t.parent_id !== null && ids.has(t.parent_id) ? t.parent_id : null
    if (!byParent.has(key)) byParent.set(key, [])
    byParent.get(key)!.push(t)
  }
  const rows: { task: Task; depth: number }[] = []
  const visit = (parent: number | null, depth: number) => {
    const children = byParent.get(parent) || []
    children.sort((a, b) => (a.sort_order - b.sort_order) || (a.id - b.id))
    for (const t of children) {
      rows.push({ task: t, depth })
      visit(t.id, depth + 1)
    }
  }
  visit(null, 0)
  return rows
})

const openTasks = computed(() =>
  tasks.value.filter(t => t.status === 'pending' || t.status === 'in_progress')
)

const detailTask = computed(() => taskDetails.value?.task ?? selectedTask.value)
const taskNotes = computed(() => taskDetails.value?.notes ?? [])
const taskActivity = computed(() => taskDetails.value?.activity ?? [])

// Helpers
/** The daemon reports domain errors as {error, message} inside a success envelope. */
function checkError<T = any>(res: T): T {
  const r = res as any
  if (r && typeof r === 'object' && !Array.isArray(r) && r.error) {
    throw new Error(r.message || String(r.error))
  }
  return res
}

function priorityClass(p: number): string {
  switch (p) {
    case 1: return 'bg-red-500/15 text-red-400'
    case 2: return 'bg-amber-500/15 text-amber-400'
    case 3: return 'bg-sky-500/15 text-sky-400'
    default: return 'bg-slate-500/15 text-slate-400'
  }
}

function formatDate(iso: string): string {
  try {
    const d = new Date(iso)
    if (Number.isNaN(d.getTime())) return iso
    return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
  } catch {
    return iso
  }
}

function formatNum(n: unknown): string {
  if (typeof n !== 'number' || !Number.isFinite(n)) return '—'
  return Math.round(n).toLocaleString()
}

function formatStop(stop: unknown): string {
  if (stop == null) return '—'
  if (typeof stop === 'string') return stop
  return JSON.stringify(stop)
}

function entryText(e: any): string {
  if (typeof e === 'string') return e
  if (e?.content != null) return String(e.content)
  const kind = e?.kind ?? e?.action ?? e?.event ?? ''
  const detail = e?.detail == null ? '' : (typeof e.detail === 'string' ? e.detail : JSON.stringify(e.detail))
  const base = [kind, detail].filter(Boolean).join(' — ')
  return base || JSON.stringify(e)
}

function entryTime(e: any): string {
  const ts = e?.created_at ?? e?.at ?? e?.timestamp
  return typeof ts === 'string' ? formatDate(ts) : ''
}

// Task list
function onTasksPrimary() {
  if (!isOnline.value || loadError.value) {
    void refreshTasks()
    return
  }
  showCreateModal.value = true
}

async function refreshTasks() {
  refreshing.value = true
  try {
    let res: any
    if (filterActive.value && filterText.value.trim()) {
      res = checkError(await invoke('query_tasks', {
        filter: filterText.value.trim(),
        scope: scope.value,
        sessionId: null,
      }))
    } else {
      res = checkError(await invoke('list_tasks', {
        scope: scope.value,
        sessionId: null,
        includeClosed: includeClosed.value,
      }))
    }
    tasks.value = res.tasks || []
  } catch (e) {
    console.error('Failed to load tasks:', e)
    loadError.value = e instanceof Error ? e.message : String(e)
  } finally {
    refreshing.value = false
    loading.value = false
  }
}

function applyFilter() {
  if (!filterText.value.trim()) return
  filterActive.value = true
  refreshTasks()
}

function clearFilter() {
  filterActive.value = false
  filterText.value = ''
  refreshTasks()
}

// Details
async function selectTask(t: Task) {
  selectedTask.value = t
  verdictMsg.value = null
  loadingDetails.value = true
  try {
    taskDetails.value = checkError(await invoke('get_task', { id: t.id }))
  } catch (e) {
    console.error('Failed to load task details:', e)
    taskDetails.value = null
  } finally {
    loadingDetails.value = false
  }
}

function deselectTask() {
  selectedTask.value = null
  taskDetails.value = null
  verdictMsg.value = null
}

async function refreshDetails() {
  if (!selectedTask.value) return
  try {
    taskDetails.value = checkError(await invoke('get_task', { id: selectedTask.value.id }))
  } catch (e) {
    console.error('Failed to refresh task details:', e)
  }
}

// Actions
async function completeTask(t: Task) {
  actionBusy.value = true
  try {
    const res: any = checkError(await invoke('complete_task', { id: t.id, workdir: null }))
    if (res.done === false) {
      // Acceptance verification failed — surface the verdict on the task
      if (selectedTask.value?.id !== t.id) await selectTask(t)
      verdictMsg.value = 'Acceptance check failed'
        + (res.verdict != null
          ? ': ' + (typeof res.verdict === 'string' ? res.verdict : JSON.stringify(res.verdict, null, 2))
          : '')
    } else {
      verdictMsg.value = null
    }
    await refreshTasks()
    await refreshDetails()
  } catch (e: any) {
    if (selectedTask.value?.id === t.id) verdictMsg.value = e.toString()
    else alert('Failed to complete task: ' + e.toString())
  } finally {
    actionBusy.value = false
  }
}

async function removeTask(t: Task) {
  if (!confirm(`Delete task #${t.id} "${t.title}" and its subtree?`)) return
  actionBusy.value = true
  try {
    checkError(await invoke('delete_task', { id: t.id }))
    if (selectedTask.value?.id === t.id) deselectTask()
    await refreshTasks()
  } catch (e: any) {
    alert('Failed to delete task: ' + e.toString())
  } finally {
    actionBusy.value = false
  }
}

async function addNote() {
  if (!selectedTask.value || !newNote.value.trim()) return
  actionBusy.value = true
  try {
    checkError(await invoke('add_task_note', { id: selectedTask.value.id, content: newNote.value.trim() }))
    newNote.value = ''
    await refreshDetails()
  } catch (e: any) {
    alert('Failed to add note: ' + e.toString())
  } finally {
    actionBusy.value = false
  }
}

async function createTask() {
  createError.value = null
  if (!newTask.value.title.trim()) {
    createError.value = 'Title is required'
    return
  }
  const payload: Record<string, unknown> = {
    title: newTask.value.title.trim(),
    scope: scope.value,
    priority: Number(newTask.value.priority),
  }
  if (newTask.value.description.trim()) payload.description = newTask.value.description.trim()
  if (newTask.value.parentId) payload.parent_id = Number(newTask.value.parentId)
  if (newTask.value.acceptance.trim()) {
    try {
      payload.acceptance = JSON.parse(newTask.value.acceptance)
    } catch {
      createError.value = 'Acceptance must be valid JSON'
      return
    }
  }
  createBusy.value = true
  try {
    checkError(await invoke('create_task', { payload }))
    showCreateModal.value = false
    newTask.value = { title: '', description: '', priority: '3', parentId: '', acceptance: '' }
    await refreshTasks()
  } catch (e: any) {
    createError.value = e.toString()
  } finally {
    createBusy.value = false
  }
}

// Long-horizon run
async function loadRunStatus() {
  try {
    const status = checkError(await invoke<RunStatus>('get_task_run_status', {
      scope: scope.value,
      sessionId: null,
    }))
    runStatus.value = status
    // Surface the last report when idle and nothing fresher arrived via events
    if (!status.running && status.last_report && !finalReport.value) {
      finalReport.value = status.last_report
    }
  } catch (e) {
    console.error('Failed to load run status:', e)
  }
}

async function startRun() {
  runError.value = null
  if (!runGoal.value.trim()) return
  const payload: Record<string, unknown> = {
    goal: runGoal.value.trim(),
    scope: scope.value,
  }
  const budget = Number(runMaxTokens.value)
  if (runMaxTokens.value.trim() && Number.isFinite(budget) && budget > 0) {
    payload.max_total_tokens = Math.floor(budget)
  }
  runBusy.value = true
  try {
    checkError(await invoke('start_task_run', { payload }))
    finalReport.value = null
    runFeed.value = []
    await loadRunStatus()
  } catch (e: any) {
    runError.value = e.toString()
  } finally {
    runBusy.value = false
  }
}

async function cancelRun() {
  runError.value = null
  runBusy.value = true
  try {
    checkError(await invoke('cancel_task_run', { scope: scope.value, sessionId: null }))
    await loadRunStatus()
  } catch (e: any) {
    runError.value = e.toString()
  } finally {
    runBusy.value = false
  }
}

function pushFeed(text: string) {
  runFeed.value.push({ ts: new Date().toLocaleTimeString(), text })
  if (runFeed.value.length > 200) runFeed.value.splice(0, runFeed.value.length - 200)
  nextTick(() => {
    if (feedEl.value) feedEl.value.scrollTop = feedEl.value.scrollHeight
  })
}

// Watchers
watch([scope, includeClosed], () => {
  deselectTask()
  refreshTasks()
  loadRunStatus()
})

// Lifecycle
onMounted(async () => {
  unlistenTaskEvent = await listen<TaskEventPayload>('task-event', (event) => {
    const p = event.payload
    if (p.kind === 'run_started') {
      finalReport.value = null
      pushFeed(`run started — ${p.goal ?? ''}`)
    } else if (p.kind === 'run_progress') {
      const taskPart = p.task_id != null ? `#${p.task_id} ` : ''
      const detailPart = p.detail == null
        ? ''
        : (typeof p.detail === 'string' ? p.detail : JSON.stringify(p.detail))
      pushFeed(`${taskPart}${p.progress_kind ?? 'progress'}${detailPart ? ' — ' + detailPart : ''}`)
    } else if (p.kind === 'run_completed') {
      finalReport.value = p.report ?? null
      pushFeed('run completed')
    }
    // Any run event can change task state — refresh list + status + details
    refreshTasks()
    loadRunStatus()
    refreshDetails()
  })

  await refreshTasks()
  await loadRunStatus()
})

onUnmounted(() => {
  unlistenTaskEvent?.()
})
</script>
