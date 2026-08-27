<script setup lang="ts">
export interface NuiChatMenuItem {
  id: string
  title: string
  meta?: string
}

const props = defineProps<{
  chats: NuiChatMenuItem[]
  activeId?: string
}>()

const emit = defineEmits<{
  'new-chat': []
  select: [id: string]
}>()
</script>

<template>
  <aside class="flex w-64 shrink-0 flex-col gap-4 overflow-clip pt-6 pb-8">
    <div class="flex w-full items-center justify-between pl-4">
      <p class="text-xs leading-normal text-nui-fg">Chats</p>
      <NuiIconButton icon="add" label="New chat" @click="emit('new-chat')" />
    </div>
    <div class="nui-scroll flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
      <NuiChatCard
        v-for="chat in props.chats"
        :key="chat.id"
        :title="chat.title"
        :meta="chat.meta"
        :active="chat.id === props.activeId"
        @click="emit('select', chat.id)"
      />
      <p v-if="props.chats.length === 0" class="px-4 py-8 text-center text-xs text-nui-muted">
        No chats yet
      </p>
    </div>
  </aside>
</template>
