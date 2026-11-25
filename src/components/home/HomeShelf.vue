<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { shelfEdges, watchShelfRow } from '@/modules/layout/shelfEdges'

// 首页横向货架：标题行 + 可横向滚动的卡片行。桌面端悬停时显示左右翻页按钮，
// 滚轮的纵向增量在可横向滚动时也转成横向，宽窄屏都保持同一套卡片尺寸，不再因列数变化丢内容
defineProps<{
  title: string
  icon?: string
  /** 「查看全部」按钮文案；为空时不显示 */
  moreLabel?: string
  /** 卡片宽度：cover（方形封面卡）或 wide（横向小卡） */
  variant?: 'cover' | 'wide'
}>()

const emit = defineEmits<{ more: [] }>()
const { t } = useI18n()
const rowRef = ref<HTMLElement | null>(null)
const canPrev = ref(false)
const canNext = ref(false)
let stopWatching: (() => void) | null = null

function updateEdges() {
  const row = rowRef.value
  if (!row) return
  const edges = shelfEdges(row)
  canPrev.value = edges.canPrev
  canNext.value = edges.canNext
}

function page(direction: 1 | -1) {
  const row = rowRef.value
  if (!row) return
  row.scrollBy({ left: direction * row.clientWidth * 0.85, behavior: 'smooth' })
}

onMounted(() => {
  void nextTick(updateEdges)
  // 行尺寸变化和卡片增删都要重算：后者只改 scrollWidth，ResizeObserver 不会触发
  if (rowRef.value) stopWatching = watchShelfRow(rowRef.value, updateEdges)
})
onBeforeUnmount(() => stopWatching?.())
</script>

<template>
  <section class="home-shelf" :class="`home-shelf--${variant ?? 'cover'}`">
    <header class="shelf-header">
      <h2 class="shelf-title">
        <slot name="icon"><span v-if="icon" class="material-symbols-rounded shelf-icon" aria-hidden="true">{{ icon }}</span></slot>
        <span>{{ title }}</span>
      </h2>
      <div class="shelf-actions">
        <button v-if="moreLabel" type="button" class="shelf-more" @click="emit('more')">{{ moreLabel }}</button>
        <button type="button" class="shelf-nav" :disabled="!canPrev" :aria-label="t('home.previous_page')" @click="page(-1)">
          <span class="material-symbols-rounded">chevron_left</span>
        </button>
        <button type="button" class="shelf-nav" :disabled="!canNext" :aria-label="t('home.next_page')" @click="page(1)">
          <span class="material-symbols-rounded">chevron_right</span>
        </button>
      </div>
    </header>
    <div ref="rowRef" class="shelf-row" @scroll.passive="updateEdges">
      <slot />
    </div>
  </section>
</template>

<style scoped lang="scss">
.home-shelf {
  margin-bottom: 36px;
  --shelf-item-width: clamp(148px, 15cqi, 196px);

  &--wide { --shelf-item-width: clamp(240px, 26cqi, 320px); }
}

.shelf-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 14px;
}

.shelf-title {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  font-size: calc(19px * var(--text-scale));
  font-weight: 700;
  letter-spacing: var(--tracking-title);
  color: var(--md-on-surface);

  > span:last-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
}

.shelf-icon {
  font-size: 20px;
  color: var(--md-on-surface-variant);
}

.shelf-actions {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.shelf-more {
  height: 30px;
  padding: 0 12px;
  margin-right: 4px;
  border-radius: var(--radius-full);
  font-size: var(--fs-13);
  font-weight: 600;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short), color var(--duration-short);

  &:hover { background: var(--state-hover); color: var(--md-on-surface); }
}

.shelf-nav {
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: var(--radius-full);
  color: var(--md-on-surface-variant);
  background: var(--md-surface-container-low);
  box-shadow: var(--shadow-1);
  transition: background var(--duration-short), color var(--duration-short), opacity var(--duration-short);

  .material-symbols-rounded { font-size: 20px; }
  &:hover:not(:disabled) { background: var(--md-surface-container-high); color: var(--md-on-surface); }
  &:disabled { opacity: 0.35; cursor: default; box-shadow: none; }
}

.shelf-row {
  display: grid;
  grid-auto-flow: column;
  grid-auto-columns: var(--shelf-item-width);
  gap: 18px;
  overflow-x: auto;
  overscroll-behavior-x: contain;
  scroll-snap-type: x proximity;
  scroll-padding-inline: 2px;
  /* 卡片悬停上浮和阴影需要留出空间，不被滚动容器裁掉 */
  padding: 4px 2px 10px;
  margin: -4px -2px -10px;
  scrollbar-width: none;

  &::-webkit-scrollbar { display: none; }
  > :deep(*) { scroll-snap-align: start; }
}
</style>
