<template>
  <!-- The chat's window into the task store (P19): the planner seeds it, the
       harness drains it, the agent's todo skill writes to it. Now with
       interactive editing: mark complete, delete, reorder, and create. -->
  <aside v-if="tasks.length > 0 || showCreateInput" class="task-checklist" :class="collapsed ? 'w-10' : 'w-64 xl:w-72'">
    <!-- Collapsed rail -->
    <button
      v-if="collapsed"
      class="rail"
      :title="`Tasks: ${doneCount}/${tasks.length} done — click to expand`"
      @click="collapsed = false"
    >
      <ListChecks class="w-4 h-4" />
      <span class="rail-count">{{ doneCount }}/{{ tasks.length }}</span>
    </button>

    <!-- Expanded panel -->
    <div v-else class="panel">
      <div class="panel-header">
        <div class="flex items-center gap-2 min-w-0">
          <ListChecks class="w-4 h-4 text-nanna-primary shrink-0" />
          <span class="text-sm font-medium text-nanna-text truncate">Tasks</span>
          <span class="text-xs font-mono text-nanna-text-dim">{{ doneCount }}/{{ tasks.length }}</span>
        </div>
        <div class="flex items-center gap-1">
          <button
            class="text-nanna-text-dim hover:text-nanna-primary transition-colors p-1"
            title="Add task"
            @click="showCreateInput = true"
          >
            <Plus class="w-4 h-4" />
          </button>
          <button
            class="text-nanna-text-dim hover:text-nanna-text transition-colors p-1"
            title="Collapse"
            @click="collapsed = true"
          >
            <ChevronRight class="w-4 h-4" />
          </button>
        </div>
      </div>

      <div class="panel-body">
        <!-- Create new task input -->
        <div v-if="showCreateInput" class="create-task-row">
          <input
            ref="createInput"
            v-model="newTaskTitle"
            type="text"
            placeholder="New task title..."
            class="create-input"
            @keydown.enter="handleCreate"
            @keydown.escape="cancelCreate"
          />
          <button
            class="create-btn"
            :disabled="!newTaskTitle.trim() || creating"
            @click="handleCreate"
          >
            <Loader2 v-if="creating" class="w-3.5 h-3.5 animate-spin" />
            <Check v-else class="w-3.5 h-3.5" />
          </button>
          <button class="cancel-btn" @click="cancelCreate">
            <X class="w-3.5 h-3.5" />
          </button>
        </div>

        <!-- Task list with drag-and-drop -->
        <div
          v-for="(task, index) in tasks"
          :key="task.id"
          class="task-row"
          :class="{
            'pl-6': task.parent_id != null,
            'opacity-50': task.status === 'cancelled',
            'dragging': draggedIndex === index,
            'drag-over': dragOverIndex === index && draggedIndex !== index
          }"
          :title="task.description || task.title"
          draggable="true"
          @dragstart="handleDragStart($event, index)"
          @dragover="handleDragOver($event, index)"
          @dragleave="handleDragLeave"
          @drop="handleDrop($event, index)"
          @dragend="handleDragEnd"
        >
          <!-- Drag handle -->
          <span class="drag-handle" title="Drag to reorder">
            <GripVertical class="w-3 h-3" />
          </span>

          <!-- Status glyph / checkbox -->
          <button
            class="glyph-btn"
            :class="glyphClass(task.status)"
            :disabled="task.status === 'done' || task.status === 'cancelled' || completing === task.id"
            :title="task.status === 'done' ? 'Completed' : task.status === 'cancelled' ? 'Cancelled' : 'Mark as complete'"
            @click="handleComplete(task)"
          >
            <Loader2 v-if="completing === task.id" class="w-3.5 h-3.5 animate-spin" />
            <CheckCircle2 v-else-if="task.status === 'done'" class="w-3.5 h-3.5" />
            <XCircle v-else-if="task.status === 'cancelled'" class="w-3.5 h-3.5" />
            <Circle v-else class="w-3.5 h-3.5" />
          </button>

          <span class="flex-1 min-w-0">
            <span
              class="task-title"
              :class="{
                'line-through text-nanna-text-dim': task.status === 'done' || task.status === 'cancelled',
                'text-nanna-text': task.status === 'in_progress',
              }"
            >{{ task.title }}</span>
            <!-- Whose responsibility: the store's assignee. Sub-agents own
                 items they were delegated; unassigned items are Nanna's. -->
            <span
              v-if="assigneeLabel(task)"
              class="owner"
              :class="isDelegated(task) ? 'owner-agent' : 'owner-self'"
              :title="`Owner: ${task.assignee}`"
            >
              <Bot v-if="isDelegated(task)" class="w-3 h-3" />
              {{ assigneeLabel(task) }}
            </span>
          </span>

          <span v-if="task.priority === 1 && task.status !== 'done'" class="prio" title="Interjected / urgent">!</span>

          <!-- Delete button -->
          <button
            class="delete-btn"
            :disabled="deleting === task.id"
            title="Delete task"
            @click.stop="handleDelete(task)"
          >
            <Loader2 v-if="deleting === task.id" class="w-3 h-3 animate-spin" />
            <Trash2 v-else class="w-3 h-3" />
          </button>
        </div>
      </div>
    </div>
  </aside>

  <!-- Delete confirmation dialog -->
  <Teleport to="body">
    <div v-if="confirmDelete" class="confirm-overlay" @click="confirmDelete = null">
      <div class="confirm-dialog" @click.stop>
        <p class="confirm-text">Delete task "{{ confirmDelete.title }}"?</p>
        <p class="confirm-subtext">This will also delete any subtasks.</p>
        <div class="confirm-actions">
          <button class="confirm-cancel" @click="confirmDelete = null">Cancel</button>
          <button class="confirm-delete" @click="confirmDeleteTask">Delete</button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Bot, Check, CheckCircle2, ChevronRight, Circle, GripVertical, ListChecks, Loader2, Plus, Trash2, X, XCircle } from 'lucide-vue-next'

interface TaskItem {
  id: number
  parent_id: number | null
  title: string
  description?: string | null
  status: string
  priority: number
  /// Who owns this item: "chat" (Nanna's own run) or a sub-agent label.
  assignee?: string | null
  completed_at?: string | null
}

const props = defineProps<{ sessionId: string }>()

const tasks = ref<TaskItem[]>([])
const collapsed = ref(false)
const doneCount = computed(() => tasks.value.filter(t => t.status === 'done').length)

// Create new task state
const showCreateInput = ref(false)
const newTaskTitle = ref('')
const creating = ref(false)
const createInput = ref<HTMLInputElement | null>(null)

// Action states
const completing = ref<number | null>(null)
const deleting = ref<number | null>(null)
const confirmDelete = ref<TaskItem | null>(null)

// Drag and drop state
const draggedIndex = ref<number | null>(null)
const dragOverIndex = ref<number | null>(null)

// The harness runs chat items as actor "chat"; anything else was delegated
// (a sub-agent, the scheduler, a channel). Only the delegations are worth
// labelling — tagging every row "chat" would be noise.
function isDelegated(task: TaskItem): boolean {
  const owner = task.assignee?.trim()
  return !!owner && owner !== 'chat'
}

function assigneeLabel(task: TaskItem): string {
  return isDelegated(task) ? (task.assignee as string) : ''
}

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

// --- Create task ---
async function handleCreate() {
  const title = newTaskTitle.value.trim()
  if (!title || creating.value) return

  creating.value = true
  try {
    await invoke('create_task', {
      title,
      scope: 'session',
      sessionId: props.sessionId,
      parentId: null,
      description: null,
      priority: null,
    })
    newTaskTitle.value = ''
    showCreateInput.value = false
    await load()
  } catch (e) {
    console.error('Failed to create task:', e)
  } finally {
    creating.value = false
  }
}

function cancelCreate() {
  showCreateInput.value = false
  newTaskTitle.value = ''
}

watch(showCreateInput, async (show) => {
  if (show) {
    await nextTick()
    createInput.value?.focus()
  }
})

// --- Complete task ---
async function handleComplete(task: TaskItem) {
  if (task.status === 'done' || task.status === 'cancelled') return
  completing.value = task.id
  try {
    await invoke('complete_task', { id: task.id, workdir: null })
    await load()
  } catch (e) {
    console.error('Failed to complete task:', e)
  } finally {
    completing.value = null
  }
}

// --- Delete task ---
function handleDelete(task: TaskItem) {
  confirmDelete.value = task
}

async function confirmDeleteTask() {
  if (!confirmDelete.value) return
  const task = confirmDelete.value
  confirmDelete.value = null
  deleting.value = task.id
  try {
    await invoke('delete_task', { id: task.id })
    await load()
  } catch (e) {
    console.error('Failed to delete task:', e)
  } finally {
    deleting.value = null
  }
}

// --- Drag and drop reordering ---
function handleDragStart(e: DragEvent, index: number) {
  draggedIndex.value = index
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move'
    e.dataTransfer.setData('text/plain', String(index))
  }
}

function handleDragOver(e: DragEvent, index: number) {
  e.preventDefault()
  if (e.dataTransfer) {
    e.dataTransfer.dropEffect = 'move'
  }
  dragOverIndex.value = index
}

function handleDragLeave() {
  dragOverIndex.value = null
}

async function handleDrop(e: DragEvent, targetIndex: number) {
  e.preventDefault()
  dragOverIndex.value = null

  if (draggedIndex.value === null || draggedIndex.value === targetIndex) {
    draggedIndex.value = null
    return
  }

  const draggedTask = tasks.value[draggedIndex.value]
  const targetTask = tasks.value[targetIndex]

  if (!draggedTask || !targetTask) {
    draggedIndex.value = null
    return
  }

  // Calculate new priority based on position
  // If moving up, take target's priority; if moving down, take target's priority + 1
  const newPriority = draggedIndex.value < targetIndex
    ? targetTask.priority + 1
    : targetTask.priority

  try {
    await invoke('reorder_task', {
      id: draggedTask.id,
      newPriority,
    })
    await load()
  } catch (e) {
    console.error('Failed to reorder task:', e)
  }

  draggedIndex.value = null
}

function handleDragEnd() {
  draggedIndex.value = null
  dragOverIndex.value = null
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
}
.panel-body {
  overflow-y: auto;
  padding: 6px 6px 10px;
}

/* Create task input */
.create-task-row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  margin-bottom: 4px;
}
.create-input {
  flex: 1;
  min-width: 0;
  padding: 4px 8px;
  font-size: 12px;
  border-radius: 4px;
  background: rgba(148, 163, 184, 0.1);
  border: 1px solid rgba(148, 163, 184, 0.2);
  color: var(--color-nanna-text, #e2e8f0);
}
.create-input:focus {
  outline: none;
  border-color: var(--color-nanna-primary, #7c3aed);
}
.create-input::placeholder {
  color: var(--color-nanna-text-dim, rgba(148, 163, 184, 0.6));
}
.create-btn, .cancel-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 4px;
  border-radius: 4px;
  transition: all 0.15s ease;
}
.create-btn {
  color: var(--color-nanna-primary, #7c3aed);
}
.create-btn:hover:not(:disabled) {
  background: rgba(124, 58, 237, 0.1);
}
.create-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.cancel-btn {
  color: var(--color-nanna-text-dim, rgba(148, 163, 184, 0.6));
}
.cancel-btn:hover {
  color: var(--color-nanna-text, #e2e8f0);
}

/* Task row */
.task-row {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 5px 8px;
  border-radius: 0.5rem;
  font-size: 13px;
  line-height: 1.35;
  cursor: grab;
  transition: all 0.15s ease;
}
.task-row:hover {
  background: rgba(148, 163, 184, 0.04);
}
.task-row.dragging {
  opacity: 0.5;
  background: rgba(124, 58, 237, 0.1);
}
.task-row.drag-over {
  border-top: 2px solid var(--color-nanna-primary, #7c3aed);
}

/* Drag handle */
.drag-handle {
  flex-shrink: 0;
  margin-top: 2px;
  color: var(--color-nanna-text-dim, rgba(148, 163, 184, 0.4));
  cursor: grab;
  opacity: 0;
  transition: opacity 0.15s ease;
}
.task-row:hover .drag-handle {
  opacity: 1;
}
.drag-handle:active {
  cursor: grabbing;
}

/* Status glyph button */
.glyph-btn {
  flex-shrink: 0;
  margin-top: 1px;
  padding: 0;
  background: none;
  border: none;
  cursor: pointer;
  transition: all 0.15s ease;
}
.glyph-btn:disabled {
  cursor: default;
}
.glyph-btn:not(:disabled):hover {
  transform: scale(1.1);
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

/* Delete button */
.delete-btn {
  flex-shrink: 0;
  padding: 2px;
  color: var(--color-nanna-text-dim, rgba(148, 163, 184, 0.4));
  opacity: 0;
  transition: all 0.15s ease;
}
.task-row:hover .delete-btn {
  opacity: 1;
}
.delete-btn:hover:not(:disabled) {
  color: rgb(239 68 68);
}
.delete-btn:disabled {
  cursor: not-allowed;
}

.owner {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  margin-left: 6px;
  padding: 0 5px;
  border-radius: 9999px;
  font-size: 10px;
  font-family: var(--font-mono, monospace);
  vertical-align: middle;
  white-space: nowrap;
}
.owner-agent {
  color: rgb(129 140 248);
}
.owner-self {
  color: var(--color-nanna-text-dim, rgba(148, 163, 184, 0.7));
}

/* Confirm dialog */
.confirm-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 9999;
}
.confirm-dialog {
  background: var(--color-nanna-bg, #1e1e2e);
  border: 1px solid rgba(148, 163, 184, 0.2);
  border-radius: 12px;
  padding: 20px 24px;
  max-width: 320px;
  box-shadow: 0 20px 40px rgba(0, 0, 0, 0.3);
}
.confirm-text {
  font-size: 14px;
  font-weight: 500;
  color: var(--color-nanna-text, #e2e8f0);
  margin-bottom: 4px;
}
.confirm-subtext {
  font-size: 12px;
  color: var(--color-nanna-text-dim, rgba(148, 163, 184, 0.7));
  margin-bottom: 16px;
}
.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.confirm-cancel, .confirm-delete {
  padding: 6px 14px;
  font-size: 13px;
  font-weight: 500;
  border-radius: 6px;
  transition: all 0.15s ease;
}
.confirm-cancel {
  color: var(--color-nanna-text-dim, rgba(148, 163, 184, 0.8));
  background: transparent;
}
.confirm-cancel:hover {
  background: rgba(148, 163, 184, 0.1);
  color: var(--color-nanna-text, #e2e8f0);
}
.confirm-delete {
  background: rgb(239 68 68);
  color: white;
}
.confirm-delete:hover {
  background: rgb(220 38 38);
}
</style>
