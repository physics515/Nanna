<script setup lang="ts">
import { computed } from 'vue'

const props = withDefaults(defineProps<{
  title: string
  status: 'upcoming' | 'in-progress' | 'complete'
  /** Override the derived status word (e.g. "Cancelled"). */
  statusLabel?: string
  description?: string
  assignee?: string
  /** Fraction complete, 0–1. Defaults from status when omitted. */
  progress?: number
  /** Interjected / urgent task — pink "!" marker in the header. */
  urgent?: boolean
  /** Show the edit action. */
  editable?: boolean
  /** An action is in flight — buttons disable. */
  busy?: boolean
}>(), {
  assignee: 'Nanna',
  editable: true,
})

const emit = defineEmits<{
  complete: []
  edit: []
  delete: []
}>()

const statusLabel = computed(() => props.statusLabel ?? {
  'upcoming': 'Upcoming',
  'in-progress': 'In Progress',
  'complete': 'Complete',
}[props.status])

const complete = computed(() => props.status === 'complete')

const progressValue = computed(() => {
  if (props.progress !== undefined) return props.progress
  return props.status === 'upcoming' ? 0 : 1
})
</script>

<template>
  <div
    class="flex w-full flex-col gap-4 px-2 pb-4"
    :class="complete && 'bg-nui-pink'"
  >
    <div class="flex w-full flex-col gap-4 py-4 pl-4">
      <div class="flex w-full items-center gap-4">
        <div class="flex min-w-0 flex-1 items-center gap-2.5">
          <p class="min-w-0 truncate text-sm font-semibold leading-normal" :title="props.title">
            <span :class="complete ? 'text-nui-bg' : 'text-nui-fg'">{{ props.title }}</span>
            <span :class="complete ? 'text-nui-accent' : 'text-nui-muted'">{{ ' ' + statusLabel }}</span>
          </p>
          <span v-if="props.urgent && !complete" class="shrink-0 text-sm font-semibold text-nui-pink" title="Interjected / urgent">!</span>
          <span class="min-w-0 flex-1" />
          <NuiIconButton
            v-if="!complete"
            icon="circle-check"
            label="Mark complete"
            class="!p-0"
            :disabled="props.busy"
            @click="emit('complete')"
          />
          <NuiIconButton
            v-if="props.editable"
            icon="edit-task"
            label="Edit task"
            class="!p-0"
            :class="complete ? 'text-nui-bg hover:text-nui-bg/70' : ''"
            :disabled="props.busy"
            @click="emit('edit')"
          />
          <NuiIconButton
            icon="delete"
            label="Delete task"
            class="!p-0"
            :class="complete ? 'text-nui-bg hover:text-nui-bg/70' : ''"
            :disabled="props.busy"
            @click="emit('delete')"
          />
        </div>
        <p class="shrink-0 whitespace-nowrap text-xs font-semibold leading-normal">
          <span :class="complete ? 'text-nui-bg' : 'text-nui-fg'">{{ complete ? 'Completed by' : 'Assigned to' }}</span>
          <span :class="complete ? 'text-nui-accent' : 'text-nui-muted'">{{ ' ' + props.assignee }}</span>
        </p>
      </div>
      <p
        v-if="props.description"
        class="w-full break-words text-xs leading-normal"
        :class="complete ? 'text-nui-bg line-through' : 'text-nui-fg'"
      >
        {{ props.description }}
      </p>
      <!-- Subtasks or any extra rows the caller wants inside the card -->
      <slot />
    </div>
    <NuiProgress :value="progressValue" />
  </div>
</template>
