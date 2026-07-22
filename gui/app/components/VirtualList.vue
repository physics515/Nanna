<template>
  <div
    ref="rootEl"
    class="virtual-list"
    :style="{ height: '100%', overflowY: 'auto', position: 'relative' }"
    @scroll.passive="onScroll"
  >
    <div
      class="virtual-list__spacer"
      :style="{ height: `${totalHeight}px`, position: 'relative' }"
    >
      <div
        class="virtual-list__window"
        :style="{
          position: 'absolute',
          top: 0,
          left: 0,
          right: 0,
          transform: `translateY(${offsetY}px)`,
        }"
      >
        <div
          v-for="(item, i) in visibleItems"
          :key="keyOf(item, start + i)"
          class="virtual-list__row"
          :style="{ height: `${itemHeight}px`, boxSizing: 'border-box' }"
        >
          <slot :item="item" :index="start + i" />
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { visibleRange } from '~/lib/virtualList'

const props = withDefaults(
  defineProps<{
    items: any[]
    itemHeight: number
    overscan?: number
    /** Optional key extractor; falls back to index. */
    itemKey?: string | ((item: any, index: number) => string | number)
  }>(),
  { overscan: 6 },
)

const rootEl = ref<HTMLElement | null>(null)
const scrollTop = ref(0)
const viewportHeight = ref(0)

let ro: ResizeObserver | null = null

function measure() {
  const el = rootEl.value
  if (!el) return
  viewportHeight.value = el.clientHeight
  scrollTop.value = el.scrollTop
}

function onScroll() {
  if (rootEl.value) scrollTop.value = rootEl.value.scrollTop
}

const range = computed(() =>
  visibleRange(
    scrollTop.value,
    viewportHeight.value,
    props.items.length,
    props.itemHeight,
    props.overscan,
  ),
)

const start = computed(() => range.value.start)
const end = computed(() => range.value.end)
const offsetY = computed(() => range.value.offsetY)
const totalHeight = computed(() => range.value.totalHeight)
const visibleItems = computed(() => props.items.slice(start.value, end.value))

function keyOf(item: any, index: number): string | number {
  if (typeof props.itemKey === 'function') return props.itemKey(item, index)
  if (typeof props.itemKey === 'string' && item != null && props.itemKey in item) {
    return item[props.itemKey]
  }
  return index
}

function scrollToBottom() {
  const el = rootEl.value
  if (!el) return
  el.scrollTop = el.scrollHeight
}

onMounted(() => {
  measure()
  if (typeof ResizeObserver !== 'undefined' && rootEl.value) {
    ro = new ResizeObserver(() => measure())
    ro.observe(rootEl.value)
  }
})

onUnmounted(() => {
  ro?.disconnect()
  ro = null
})

watch(
  () => props.items.length,
  async () => {
    // Keep measurements honest after large list swaps
    requestAnimationFrame(measure)
  },
)

defineExpose({
  el: rootEl,
  scrollToBottom,
  measure,
})
</script>
