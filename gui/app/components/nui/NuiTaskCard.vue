<script setup lang="ts">
import { computed } from 'vue'

const props = withDefaults(defineProps<{
  title: string
  status: 'upcoming' | 'in-progress' | 'complete'
  description?: string
  assignee?: string
  /** Fraction complete, 0–1. Defaults from status when omitted. */
  progress?: number
}>(), {
  assignee: 'Nanna',
})

const emit = defineEmits<{
  complete: []
  edit: []
  delete: []
}>()

const statusLabel = computed(() => ({
  'upcoming': 'Upcoming',
  'in-progress': 'In Progress',
  'complete': 'Complete',
}[props.status]))

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
          <p class="whitespace-nowrap text-sm font-semibold leading-normal">
            <span :class="complete ? 'text-nui-bg' : 'text-nui-fg'">{{ props.title }}</span>
            <span :class="complete ? 'text-nui-accent' : 'text-nui-muted'">{{ ' ' + statusLabel }}</span>
          </p>
          <span class="min-w-0 flex-1" />
          <NuiIconButton
            icon="circle-check"
            label="Mark complete"
            class="!p-0"
            :class="complete ? 'text-nui-bg hover:text-nui-bg/70' : ''"
            @click="emit('complete')"
          />
          <NuiIconButton
            icon="edit-task"
            label="Edit task"
            class="!p-0"
            :class="complete ? 'text-nui-bg hover:text-nui-bg/70' : ''"
            @click="emit('edit')"
          />
          <NuiIconButton
            icon="delete"
            label="Delete task"
            class="!p-0"
            :class="complete ? 'text-nui-bg hover:text-nui-bg/70' : ''"
            @click="emit('delete')"
          />
        </div>
        <p class="whitespace-nowrap text-xs font-semibold leading-normal">
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
    </div>
    <NuiProgress :value="progressValue" />
  </div>
</template>
