<script setup lang="ts">
import { DialogRoot, DialogTrigger, DialogPortal, DialogOverlay, DialogContent, DialogClose, DialogTitle, DialogDescription } from 'radix-vue'
import { X } from 'lucide-vue-next'

interface Props {
  modelValue?: boolean
  title?: string
  description?: string
  size?: 'sm' | 'md' | 'lg' | 'xl'
  closable?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  modelValue: false,
  size: 'md',
  closable: true,
})

const emit = defineEmits<{
  'update:modelValue': [value: boolean]
}>()

const open = computed({
  get: () => props.modelValue,
  set: (value) => emit('update:modelValue', value),
})

const sizeClass = computed(() => ({
  sm: 'max-w-sm',
  md: 'max-w-lg',
  lg: 'max-w-2xl',
  xl: 'max-w-4xl',
}[props.size ?? 'md']))
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogTrigger v-if="$slots.trigger" as-child>
      <slot name="trigger" />
    </DialogTrigger>
    <DialogPortal>
      <DialogOverlay class="modal-overlay" />
      <DialogContent :class="['modal-content', sizeClass]">
        <!-- Header -->
        <div v-if="title || closable" class="flex items-center justify-between">
          <DialogTitle v-if="title" class="text-lg font-semibold" style="color: #e2e8f0">
            {{ title }}
          </DialogTitle>
          <DialogClose
            v-if="closable"
            class="rounded-sm opacity-70 transition-opacity hover:opacity-100 focus:outline-none"
          >
            <X class="h-4 w-4" />
            <span class="sr-only">Close</span>
          </DialogClose>
        </div>

        <DialogDescription v-if="description" class="text-sm" style="color: #94a3b8">
          {{ description }}
        </DialogDescription>

        <!-- Content -->
        <slot />

        <!-- Footer -->
        <div v-if="$slots.footer">
          <slot name="footer" />
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style scoped>
.modal-overlay {
  position: fixed;
  inset: 0;
  z-index: 50;
  background: rgba(0, 0, 0, 0.6);
  backdrop-filter: blur(4px);
}
.modal-overlay[data-state="open"] {
  animation: fade-in 200ms ease-out forwards;
}
.modal-overlay[data-state="closed"] {
  animation: fade-out 150ms ease-in forwards;
}

.modal-content {
  position: fixed;
  left: 50%;
  top: 50%;
  z-index: 50;
  display: grid;
  width: 100%;
  gap: 1rem;
  padding: 1.5rem;
  transform: translate(-50%, -50%);
  border-radius: 0.75rem;
  background: #1e293b;
  border: 1px solid rgba(71, 85, 105, 0.3);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
}
.modal-content[data-state="open"] {
  animation: modal-in 200ms ease-out forwards;
}
.modal-content[data-state="closed"] {
  animation: modal-out 150ms ease-in forwards;
}

@keyframes fade-in {
  from { opacity: 0; }
  to { opacity: 1; }
}
@keyframes fade-out {
  from { opacity: 1; }
  to { opacity: 0; }
}
@keyframes modal-in {
  from {
    opacity: 0;
    transform: translate(-50%, -50%) scale(0.95);
  }
  to {
    opacity: 1;
    transform: translate(-50%, -50%) scale(1);
  }
}
@keyframes modal-out {
  from {
    opacity: 1;
    transform: translate(-50%, -50%) scale(1);
  }
  to {
    opacity: 0;
    transform: translate(-50%, -50%) scale(0.95);
  }
}
</style>
