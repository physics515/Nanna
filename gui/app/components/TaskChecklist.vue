<template>
  <!-- The chat's window into the task store (P19): the planner seeds it, the
       harness drains it, the agent's todo skill writes to it. Task cards +
       the action rail from the nui design; mark complete, delete, reorder
       (drag), and create all still here. -->
  <aside class="hidden shrink-0 items-start gap-4 self-stretch lg:flex">
    <!-- Task pane -->
    <div
      v-if="paneOpen && (tasks.length > 0 || showCreateInput)"
      class="nui-scroll flex min-h-0 w-[512px] flex-col gap-8 self-stretch overflow-y-auto"
    >
      <div class="flex w-full items-center justify-between pl-4">
        <p class="text-xs leading-normal text-nui-fg">
          Tasks <span class="text-nui-muted">{{ doneCount }}/{{ tasks.length }}</span>
        </p>
        <NuiIconButton icon="add" label="Add task" @click="showCreateInput = true" />
      </div>

      <!-- Create new task input -->
      <div v-if="showCreateInput" class="flex items-center gap-2 px-2">
        <input
          ref="createInput"
          v-model="newTaskTitle"
          type="text"
          placeholder="New task title..."
          class="min-w-0 flex-1 border-b border-dashed border-nui-muted bg-transparent p-2 font-nui text-xs font-[450] leading-normal text-nui-fg outline-none placeholder:text-nui-muted focus:border-nui-accent"
          @keydown.enter="handleCreate"
          @keydown.escape="cancelCreate"
        />
        <NuiIconButton
          icon="circle-check"
          label="Create task"
          class="text-nui-green"
          :disabled="!newTaskTitle.trim() || creating"
          @click="handleCreate"
        />
        <NuiIconButton icon="close" label="Cancel" @click="cancelCreate" />
      </div>

      <!-- Top-level tasks as cards; subtasks as compact rows inside -->
      <div
        v-for="task in topLevelTasks"
        :key="task.id"
        :class="{
          'opacity-50': task.status === 'cancelled',
          'opacity-60': draggedId === task.id,
          'border-t-2 border-solid border-nui-accent': dragOverId === task.id && draggedId !== task.id,
        }"
        :title="task.description || task.title"
        draggable="true"
        @dragstart="handleDragStart($event, task)"
        @dragover="handleDragOver($event, task)"
        @dragleave="dragOverId = null"
        @drop="handleDrop($event, task)"
        @dragend="handleDragEnd"
      >
        <NuiTaskCard
          :title="task.title"
          :status="cardStatus(task)"
          :status-label="task.status === 'cancelled' ? 'Cancelled' : undefined"
          :description="task.description ?? undefined"
          :assignee="assigneeLabel(task)"
          :progress="cardProgress(task)"
          :urgent="task.priority === 1 && task.status !== 'done'"
          :editable="false"
          :busy="completing === task.id || deleting === task.id"
          @complete="handleComplete(task)"
          @delete="handleDelete(task)"
        >
          <div v-if="childrenOf(task.id).length" class="flex w-full flex-col gap-1">
            <div
              v-for="sub in childrenOf(task.id)"
              :key="sub.id"
              class="flex w-full items-center gap-2"
            >
              <button
                type="button"
                class="shrink-0 disabled:pointer-events-none"
                :class="subGlyphClass(task, sub)"
                :disabled="sub.status === 'done' || sub.status === 'cancelled' || completing === sub.id"
                :title="sub.status === 'done' ? 'Completed' : sub.status === 'cancelled' ? 'Cancelled' : 'Mark as complete'"
                @click="handleComplete(sub)"
              >
                <NuiIcon :name="sub.status === 'done' ? 'circle-check' : 'dot'" :size="16" />
              </button>
              <span
                class="min-w-0 flex-1 truncate text-xs leading-normal"
                :class="[
                  task.status === 'done' ? 'text-nui-bg' : 'text-nui-fg',
                  (sub.status === 'done' || sub.status === 'cancelled') && 'line-through opacity-70',
                ]"
                :title="sub.description || sub.title"
              >{{ sub.title }}</span>
              <span
                v-if="isDelegated(sub)"
                class="shrink-0 whitespace-nowrap text-xs leading-normal"
                :class="task.status === 'done' ? 'text-nui-bg' : 'text-nui-info'"
                :title="`Owner: ${sub.assignee}`"
              >{{ sub.assignee }}</span>
              <NuiIconButton
                icon="delete"
                label="Delete subtask"
                class="!p-0"
                :class="task.status === 'done' ? 'text-nui-bg hover:text-nui-bg/70' : ''"
                :disabled="deleting === sub.id"
                @click="handleDelete(sub)"
              />
            </div>
          </div>
        </NuiTaskCard>
      </div>
    </div>

    <!-- Action rail -->
    <div class="flex shrink-0 flex-col items-start">
      <NuiRailButton
        icon="tasks"
        :label="tasks.length ? `Tasks: ${doneCount}/${tasks.length} done` : 'Tasks'"
        :active="paneOpen && (tasks.length > 0 || showCreateInput)"
        accent="green"
        @click="toggleRail"
      />
      <span
        v-if="tasks.length > 0"
        class="w-full pt-1 text-center text-xs leading-normal text-nui-muted"
      >{{ doneCount }}/{{ tasks.length }}</span>
    </div>
  </aside>

  <!-- Delete confirmation dialog -->
  <Teleport to="body">
    <div v-if="confirmDelete" class="fixed inset-0 z-[9999] flex items-center justify-center bg-black/50 font-nui" @click="confirmDelete = null">
      <div class="w-80 border border-solid border-nui-muted/40 bg-nui-bg p-6" @click.stop>
        <p class="mb-1 text-sm font-semibold leading-normal text-nui-fg">Delete task "{{ confirmDelete.title }}"?</p>
        <p class="mb-4 text-xs leading-normal text-nui-muted">This will also delete any subtasks.</p>
        <div class="flex justify-end gap-2">
          <button class="px-4 py-2 text-xs text-nui-muted transition-colors hover:text-nui-fg" @click="confirmDelete = null">Cancel</button>
          <button class="bg-nui-pink px-4 py-2 text-xs text-nui-bg transition-opacity hover:opacity-80" @click="confirmDeleteTask">Delete</button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

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
const paneOpen = ref(true)
const doneCount = computed(() => tasks.value.filter(t => t.status === 'done').length)

const topLevelTasks = computed(() => tasks.value.filter(t => t.parent_id == null))

function childrenOf(id: number): TaskItem[] {
  return tasks.value.filter(t => t.parent_id === id)
}

// Create new task state
const showCreateInput = ref(false)
const newTaskTitle = ref('')
const creating = ref(false)
const createInput = ref<HTMLInputElement | null>(null)

// Action states
const completing = ref<number | null>(null)
const deleting = ref<number | null>(null)
const confirmDelete = ref<TaskItem | null>(null)

// Drag and drop state (top-level cards reorder by priority)
const draggedId = ref<number | null>(null)
const dragOverId = ref<number | null>(null)

// Opening the rail with no tasks yet goes straight to creating one — the
// pane has nothing else to show.
function toggleRail() {
  if (paneOpen.value && (tasks.value.length > 0 || showCreateInput.value)) {
    paneOpen.value = false
    showCreateInput.value = false
  } else {
    paneOpen.value = true
    if (tasks.value.length === 0) showCreateInput.value = true
  }
}

// The harness runs chat items as actor "chat"; anything else was delegated
// (a sub-agent, the scheduler, a channel). Only the delegations are worth
// labelling — tagging every row "chat" would be noise.
function isDelegated(task: TaskItem): boolean {
  const owner = task.assignee?.trim()
  return !!owner && owner !== 'chat'
}

function assigneeLabel(task: TaskItem): string {
  return isDelegated(task) ? (task.assignee as string) : 'Nanna'
}

function cardStatus(task: TaskItem): 'upcoming' | 'in-progress' | 'complete' {
  if (task.status === 'done') return 'complete'
  if (task.status === 'in_progress') return 'in-progress'
  return 'upcoming'
}

/**
 * The card's bar: subtask completion where subtasks exist, otherwise the
 * task's own binary state (half-full while in progress — the store tracks
 * no finer grain for a leaf task).
 */
function cardProgress(task: TaskItem): number {
  const children = childrenOf(task.id)
  if (children.length > 0) {
    return children.filter(c => c.status === 'done').length / children.length
  }
  if (task.status === 'done') return 1
  if (task.status === 'in_progress') return 0.5
  return 0
}

function subGlyphClass(parent: TaskItem, sub: TaskItem): string {
  if (parent.status === 'done') return 'text-nui-bg'
  switch (sub.status) {
    case 'done': return 'text-nui-green'
    case 'in_progress': return 'text-nui-accent'
    case 'cancelled': return 'text-nui-muted'
    default: return 'text-nui-muted hover:text-nui-fg'
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

// --- Drag and drop reordering (top-level cards) ---
function handleDragStart(e: DragEvent, task: TaskItem) {
  draggedId.value = task.id
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move'
    e.dataTransfer.setData('text/plain', String(task.id))
  }
}

function handleDragOver(e: DragEvent, task: TaskItem) {
  e.preventDefault()
  if (e.dataTransfer) {
    e.dataTransfer.dropEffect = 'move'
  }
  dragOverId.value = task.id
}

async function handleDrop(e: DragEvent, targetTask: TaskItem) {
  e.preventDefault()
  dragOverId.value = null

  if (draggedId.value === null || draggedId.value === targetTask.id) {
    draggedId.value = null
    return
  }

  const list = topLevelTasks.value
  const draggedIndex = list.findIndex(t => t.id === draggedId.value)
  const targetIndex = list.findIndex(t => t.id === targetTask.id)
  const draggedTask = list[draggedIndex]

  if (!draggedTask || targetIndex === -1) {
    draggedId.value = null
    return
  }

  // Calculate new priority based on position
  // If moving up, take target's priority; if moving down, take target's priority + 1
  const newPriority = draggedIndex < targetIndex
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

  draggedId.value = null
}

function handleDragEnd() {
  draggedId.value = null
  dragOverId.value = null
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
