<script setup lang="ts" generic="T extends string">
import { computed } from 'vue'

// Clean 分段选择器（对齐 Android CleanSegmentedTabs）：等宽分段 + 滑动拇指，
// 键盘左右方向键切换，语义为 radiogroup
const props = defineProps<{
  modelValue: T
  options: { value: T; label: string; icon?: string }[]
  label: string
  disabled?: boolean
}>()

const emit = defineEmits<{
  'update:modelValue': [value: T]
  select: [value: T, event: MouseEvent | KeyboardEvent]
}>()

const activeIndex = computed(() => Math.max(0, props.options.findIndex(option => option.value === props.modelValue)))
const thumbStyle = computed(() => ({
  width: `calc((100% - 6px) / ${props.options.length})`,
  transform: `translateX(${activeIndex.value * 100}%)`,
}))

function choose(value: T, event: MouseEvent | KeyboardEvent) {
  if (props.disabled || value === props.modelValue) return
  emit('select', value, event)
  emit('update:modelValue', value)
}

function onKeydown(event: KeyboardEvent) {
  const step = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0
  if (!step) return
  event.preventDefault()
  const next = (activeIndex.value + step + props.options.length) % props.options.length
  choose(props.options[next].value, event)
  const buttons = (event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>('button')
  buttons[next]?.focus()
}
</script>

<template>
  <div
    class="segmented"
    :class="{ disabled }"
    role="radiogroup"
    :aria-label="label"
    :aria-disabled="disabled || undefined"
    @keydown="onKeydown"
  >
    <span class="segmented-thumb" :style="thumbStyle" aria-hidden="true" />
    <button
      v-for="option in options"
      :key="option.value"
      type="button"
      class="segmented-item"
      :class="{ active: option.value === modelValue }"
      role="radio"
      :aria-checked="option.value === modelValue"
      :tabindex="option.value === modelValue ? 0 : -1"
      :disabled="disabled"
      @click="choose(option.value, $event)"
    >
      <span v-if="option.icon" class="material-symbols-rounded" aria-hidden="true">{{ option.icon }}</span>
      <span class="segmented-label">{{ option.label }}</span>
    </button>
  </div>
</template>

<style scoped lang="scss">
.segmented {
  position: relative;
  /* 等宽列按最长标签撑开，拇指宽度才能用 1/n 计算 */
  display: inline-grid;
  grid-auto-flow: column;
  grid-auto-columns: 1fr;
  padding: 3px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-high);
  isolation: isolate;

  &.disabled { opacity: 0.45; }
}

.segmented-thumb {
  position: absolute;
  top: 3px;
  bottom: 3px;
  left: 3px;
  z-index: -1;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-lowest);
  box-shadow: var(--shadow-1), 0 0 0 0.5px var(--hairline);
  transition: transform 320ms cubic-bezier(0.3, 1.2, 0.5, 1);
}

.segmented-item {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  min-width: 0;
  height: 30px;
  padding: 0 14px;
  border-radius: var(--radius-full);
  color: var(--md-on-surface-variant);
  font-size: var(--fs-13);
  font-weight: 500;
  white-space: nowrap;
  transition: color var(--duration-short) var(--ease-standard);

  .material-symbols-rounded { font-size: 17px; }

  &:hover:not(:disabled):not(.active) { color: var(--md-on-surface); }

  &.active {
    color: var(--md-on-surface);
    font-weight: 600;

    .material-symbols-rounded { color: var(--md-primary); }
  }

  &:disabled { cursor: default; }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: -2px; }
}

.segmented-label {
  overflow: hidden;
  text-overflow: ellipsis;
}

@media (prefers-reduced-motion: reduce) {
  .segmented-thumb { transition: none; }
}
</style>
