<script setup lang="ts">
import { DialogRoot, DialogTrigger, DialogPortal, DialogOverlay, DialogContent, DialogClose } from 'radix-vue'
import { X } from 'lucide-vue-next'

interface Props {
  open?: boolean
  side?: 'left' | 'right' | 'top' | 'bottom'
}

const props = withDefaults(defineProps<Props>(), {
  side: 'left',
})

const emit = defineEmits<{
  'update:open': [value: boolean]
}>()
</script>

<template>
  <DialogRoot :open="props.open" @update:open="emit('update:open', $event)">
    <DialogTrigger as-child>
      <slot name="trigger" />
    </DialogTrigger>
    <DialogPortal>
      <DialogOverlay class="sheet-overlay" />
      <DialogContent :class="['sheet-content', `sheet-${props.side}`]">
        <slot />
        <DialogClose
          class="absolute right-4 top-4 rounded-sm opacity-70 transition-opacity hover:opacity-100 focus:outline-none"
        >
          <X class="h-4 w-4" />
          <span class="sr-only">Close</span>
        </DialogClose>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style scoped>
.sheet-overlay {
  position: fixed;
  inset: 0;
  z-index: 50;
  background: rgba(0, 0, 0, 0.6);
  backdrop-filter: blur(4px);
}
.sheet-overlay[data-state="open"] {
  animation: fade-in 300ms ease-out forwards;
}
.sheet-overlay[data-state="closed"] {
  animation: fade-out 200ms ease-in forwards;
}

.sheet-content {
  position: fixed;
  z-index: 50;
  background: #0f172a;
  padding: 1.5rem;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
}

/* Left */
.sheet-left {
  inset: 0 auto 0 0;
  height: 100%;
  width: 75%;
  max-width: 24rem;
  border-right: 1px solid rgba(255, 255, 255, 0.04);
}
.sheet-left[data-state="open"] {
  animation: enter-from-left 400ms ease-out forwards;
}
.sheet-left[data-state="closed"] {
  animation: exit-to-left 300ms ease-in forwards;
}

/* Right */
.sheet-right {
  inset: 0 0 0 auto;
  height: 100%;
  width: 75%;
  max-width: 24rem;
  border-left: 1px solid rgba(255, 255, 255, 0.04);
}
.sheet-right[data-state="open"] {
  animation: enter-from-right 400ms ease-out forwards;
}
.sheet-right[data-state="closed"] {
  animation: exit-to-right 300ms ease-in forwards;
}

/* Top */
.sheet-top {
  inset: 0 0 auto 0;
  width: 100%;
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
}
.sheet-top[data-state="open"] {
  animation: slide-in-from-top 400ms ease-out forwards;
}
.sheet-top[data-state="closed"] {
  animation: slide-out-to-top 300ms ease-in forwards;
}

/* Bottom */
.sheet-bottom {
  inset: auto 0 0 0;
  width: 100%;
  border-top: 1px solid rgba(255, 255, 255, 0.04);
}
.sheet-bottom[data-state="open"] {
  animation: slide-in-from-bottom 400ms ease-out forwards;
}
.sheet-bottom[data-state="closed"] {
  animation: slide-out-to-bottom 300ms ease-in forwards;
}

@keyframes fade-in {
  from { opacity: 0; }
  to { opacity: 1; }
}
@keyframes fade-out {
  from { opacity: 1; }
  to { opacity: 0; }
}
@keyframes enter-from-left {
  from { transform: translateX(-100%); }
  to { transform: translateX(0); }
}
@keyframes exit-to-left {
  from { transform: translateX(0); }
  to { transform: translateX(-100%); }
}
@keyframes enter-from-right {
  from { transform: translateX(100%); }
  to { transform: translateX(0); }
}
@keyframes exit-to-right {
  from { transform: translateX(0); }
  to { transform: translateX(100%); }
}
@keyframes slide-in-from-top {
  from { transform: translateY(-100%); }
  to { transform: translateY(0); }
}
@keyframes slide-out-to-top {
  from { transform: translateY(0); }
  to { transform: translateY(-100%); }
}
@keyframes slide-in-from-bottom {
  from { transform: translateY(100%); }
  to { transform: translateY(0); }
}
@keyframes slide-out-to-bottom {
  from { transform: translateY(0); }
  to { transform: translateY(100%); }
}
</style>
