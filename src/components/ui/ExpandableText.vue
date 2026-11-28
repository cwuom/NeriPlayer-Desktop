<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

const props = withDefaults(defineProps<{
  text: string
  lines?: number
}>(), {
  lines: 3,
})

const { t } = useI18n()

const bodyRef = ref<HTMLElement>()
const expanded = ref(false)
const overflowing = ref(false)
let observer: ResizeObserver | null = null

function measure() {
  const el = bodyRef.value
  // 展开后不再裁切，scrollHeight 与 clientHeight 相等，此时测量会把按钮误判为不需要
  if (!el || expanded.value) return
  // 行数截断只让 scrollHeight 大于 clientHeight；留 1px 吸收行高的亚像素误差
  overflowing.value = el.scrollHeight > el.clientHeight + 1
}

function toggle() {
  expanded.value = !expanded.value
  if (!expanded.value) void nextTick(measure)
}

onMounted(() => {
  measure()
  // 窗口宽度变化会让同一段文字换行数不同，要重新判断是否被截断
  if (typeof ResizeObserver !== 'undefined' && bodyRef.value) {
    observer = new ResizeObserver(measure)
    observer.observe(bodyRef.value)
  }
})

watch(() => [props.text, props.lines], () => {
  expanded.value = false
  void nextTick(measure)
})

onBeforeUnmount(() => observer?.disconnect())
</script>

<template>
  <div class="expandable-text">
    <p
      ref="bodyRef"
      class="expandable-text-body"
      :class="{ expanded }"
      :style="{ '--expandable-text-lines': String(lines) }"
    >{{ text }}</p>
    <button
      v-if="overflowing || expanded"
      type="button"
      class="expandable-text-toggle"
      :aria-expanded="expanded"
      @click="toggle"
    >
      <span>{{ expanded ? t('common.collapse') : t('common.expand') }}</span>
      <span class="material-symbols-rounded" aria-hidden="true">{{ expanded ? 'expand_less' : 'expand_more' }}</span>
    </button>
  </div>
</template>

<style scoped lang="scss">
/* 字号、颜色、行高与外边距由使用方的 class 决定，这里只负责截断与展开 */
.expandable-text {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
}

.expandable-text-body {
  width: 100%;
  margin: 0;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: var(--expandable-text-lines);
  line-clamp: var(--expandable-text-lines);
  overflow: hidden;
  overflow-wrap: anywhere;
  /* 简介里的换行是作者分的段，展开后要保留 */
  white-space: pre-line;
  user-select: text;

  &.expanded {
    display: block;
    -webkit-line-clamp: unset;
    line-clamp: none;
  }
}

.expandable-text-toggle {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  margin-left: -6px;
  padding: 2px 4px 2px 6px;
  border-radius: var(--radius-sm);
  color: var(--md-primary);
  font-size: var(--fs-13);
  font-weight: 600;
  line-height: 1.4;
  transition: background var(--duration-short);

  &:hover { background: color-mix(in srgb, var(--md-primary) 8%, transparent); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 1px; }

  .material-symbols-rounded { font-size: 18px; }
}
</style>
