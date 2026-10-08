<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { formatEditableNumber, parseEditableNumber, roundToStepPrecision } from '@/utils/editableRange'

const props = withDefaults(defineProps<{
  modelValue: number
  min: number
  max: number
  step?: number
  displayValue?: string
  inputScale?: number
  inputSuffix?: string
  inputWidth?: number
  rangeReversed?: boolean
  ariaLabel?: string
}>(), {
  step: 1,
  displayValue: undefined,
  inputScale: 1,
  inputSuffix: '',
  inputWidth: 72,
  rangeReversed: false,
  ariaLabel: undefined,
})

const emit = defineEmits<{
  'update:modelValue': [value: number]
}>()

const { t } = useI18n()
const inputRef = ref<HTMLInputElement>()
const valueRootRef = ref<HTMLElement>()
const isEditing = ref(false)
const draftValue = ref('')
const hasError = ref(false)
let settingCard: HTMLElement | null = null
let suppressNextCardClick = false
let blurCommitTimer: ReturnType<typeof setTimeout> | null = null
const rangeAnimationFrames = new WeakMap<HTMLInputElement, number>()

const displayValue = computed(() => props.displayValue
  ?? formatEditableNumber(props.modelValue, props.inputScale))
const inputMin = computed(() => formatEditableNumber(props.min, props.inputScale))
const inputMax = computed(() => formatEditableNumber(props.max, props.inputScale))
const inputStep = computed(() => formatEditableNumber(props.step, props.inputScale))
const errorMessage = computed(() => t('common.invalid_range', {
  min: inputMin.value,
  max: inputMax.value,
}))

function startEditing(event?: MouseEvent) {
  event?.preventDefault()
  event?.stopPropagation()
  if (isEditing.value) return

  draftValue.value = formatEditableNumber(props.modelValue, props.inputScale)
  hasError.value = false
  isEditing.value = true
  void nextTick(() => {
    inputRef.value?.focus()
    inputRef.value?.select()
  })
}

function cancelEditing() {
  if (blurCommitTimer) {
    clearTimeout(blurCommitTimer)
    blurCommitTimer = null
  }
  isEditing.value = false
  hasError.value = false
  draftValue.value = ''
}

function suppressCardClick() {
  suppressNextCardClick = true
  window.setTimeout(() => {
    suppressNextCardClick = false
  }, 0)
}

function commitEditing() {
  if (!isEditing.value) return

  if (blurCommitTimer) {
    clearTimeout(blurCommitTimer)
    blurCommitTimer = null
  }

  const previousValue = props.modelValue
  const parsedValue = parseEditableNumber(
    draftValue.value,
    props.min,
    props.max,
    props.inputScale,
  )
  if (parsedValue === null) {
    hasError.value = true
    void nextTick(() => {
      inputRef.value?.focus()
      inputRef.value?.select()
    })
    return
  }
  const nextValue = roundToStepPrecision(parsedValue, props.step, props.min, props.max)

  suppressCardClick()
  cancelEditing()
  emit('update:modelValue', nextValue)
  animateAssociatedRange(previousValue, nextValue)
}

function isEnterKey(event: KeyboardEvent): boolean {
  return event.key === 'Enter'
    || event.key === 'Return'
    || event.key === 'NumpadEnter'
    || event.code === 'Enter'
    || event.code === 'NumpadEnter'
    || event.keyCode === 13
    || event.which === 13
}

function handleKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    cancelEditing()
    return
  }

  if (isEnterKey(event)) {
    event.preventDefault()
    event.stopPropagation()
    commitEditing()
  }
}

function handleKeyup(event: KeyboardEvent) {
  if (!isEditing.value || !isEnterKey(event)) return
  event.preventDefault()
  event.stopPropagation()
  commitEditing()
}

function handleInputBlur(event: FocusEvent) {
  if (!isEditing.value) return

  const relatedTarget = event.relatedTarget
  if (relatedTarget instanceof Node && valueRootRef.value?.contains(relatedTarget)) return

  if (blurCommitTimer) clearTimeout(blurCommitTimer)
  blurCommitTimer = setTimeout(() => {
    blurCommitTimer = null
    commitEditing()
  }, 0)
}

function handleDraftInput(event: Event) {
  const input = event.currentTarget
  if (!(input instanceof HTMLInputElement)) return
  draftValue.value = input.value
}

function handleSettingCardClick(event: MouseEvent) {
  if (suppressNextCardClick) {
    suppressNextCardClick = false
    return
  }
  const target = event.target
  if (target instanceof Element && target.closest('input, button, select, textarea, label, a')) {
    return
  }
  startEditing()
}

function handleConfirmPointer(event: MouseEvent) {
  event.preventDefault()
  event.stopPropagation()
  commitEditing()
}

function findAssociatedRange(): HTMLInputElement | null {
  let parent = valueRootRef.value?.parentElement ?? null
  while (parent) {
    const ranges = parent.querySelectorAll<HTMLInputElement>('input[type="range"]')
    if (ranges.length === 1) return ranges[0]
    parent = parent.parentElement
  }
  return null
}

function toRangeValue(value: number): number {
  return props.rangeReversed ? props.min + props.max - value : value
}

function animateAssociatedRange(from: number, to: number) {
  if (!Number.isFinite(from) || !Number.isFinite(to) || from === to) return

  void nextTick(() => {
    const range = findAssociatedRange()
    if (!range) return

    const previousFrame = rangeAnimationFrames.get(range)
    if (previousFrame !== undefined) cancelAnimationFrame(previousFrame)

    const startValue = toRangeValue(from)
    const endValue = toRangeValue(to)
    const startedAt = performance.now()
    const durationMs = 240
    range.value = String(startValue)

    const animate = (timestamp: number) => {
      const progress = Math.min(1, (timestamp - startedAt) / durationMs)
      const easedProgress = 1 - Math.pow(1 - progress, 3)
      const value = startValue + (endValue - startValue) * easedProgress
      range.value = String(value)

      if (progress < 1) {
        rangeAnimationFrames.set(range, requestAnimationFrame(animate))
      } else {
        range.value = String(endValue)
        rangeAnimationFrames.delete(range)
      }
    }

    rangeAnimationFrames.set(range, requestAnimationFrame(animate))
  })
}

onMounted(() => {
  settingCard = valueRootRef.value?.closest('.setting-card') as HTMLElement | null
  settingCard?.addEventListener('click', handleSettingCardClick)
})

onUnmounted(() => {
  if (blurCommitTimer) {
    clearTimeout(blurCommitTimer)
    blurCommitTimer = null
  }
  settingCard?.removeEventListener('click', handleSettingCardClick)
  settingCard = null
})

defineExpose({ startEditing })
</script>

<template>
  <span ref="valueRootRef" class="editable-range-value" :class="{ editing: isEditing, invalid: hasError }">
    <button
      v-if="!isEditing"
      type="button"
      class="editable-range-display"
      :aria-label="ariaLabel || t('common.edit_value')"
      :title="ariaLabel || t('common.edit_value')"
      @click="startEditing"
    >{{ displayValue }}</button>

    <form v-else class="editable-range-editor" novalidate @submit.prevent.stop="commitEditing">
      <input
        ref="inputRef"
        :value="draftValue"
        class="editable-range-input"
        type="number"
        inputmode="decimal"
        :min="inputMin"
        :max="inputMax"
        :step="inputStep"
        :style="{ width: `${inputWidth}px` }"
        :aria-label="ariaLabel || t('common.edit_value')"
        @click.stop
        @input="handleDraftInput"
        @keydown="handleKeydown"
        @keyup="handleKeyup"
        @blur="handleInputBlur"
      />
      <span v-if="inputSuffix" class="editable-range-suffix">{{ inputSuffix }}</span>
      <button
        type="submit"
        class="editable-range-submit"
        :aria-label="t('common.confirm')"
        :title="t('common.confirm')"
        @click="handleConfirmPointer"
      >
        <span class="material-symbols-rounded" aria-hidden="true">check</span>
      </button>
    </form>

    <span v-if="hasError" class="editable-range-error" role="alert">
      {{ errorMessage }}
    </span>
  </span>
</template>

<style scoped>
.editable-range-value {
  position: relative;
  display: inline-flex;
  align-items: center;
  min-width: 0;
  max-width: 100%;
}

.editable-range-display {
  max-width: 100%;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: inherit;
  white-space: nowrap;
  cursor: text;
}

.editable-range-display:hover,
.editable-range-display:focus-visible {
  color: var(--md-primary, currentColor);
  text-decoration: underline;
  text-underline-offset: 3px;
  outline: none;
}

.editable-range-editor {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  max-width: 100%;
}

.editable-range-submit {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: 0;
  border-radius: 50%;
  background: transparent;
  color: var(--md-primary, currentColor);
  cursor: pointer;

  &:hover,
  &:focus-visible {
    background: color-mix(in srgb, var(--md-primary, currentColor) 12%, transparent);
    outline: none;
  }

  .material-symbols-rounded { font-size: 18px; }
}

.editable-range-input {
  box-sizing: border-box;
  width: 72px;
  height: 24px;
  padding: 2px 6px;
  border: 1px solid var(--md-primary, currentColor);
  border-radius: 5px;
  background: var(--md-surface-container-highest, transparent);
  color: inherit;
  font: inherit;
  font-variant-numeric: tabular-nums;
  outline: none;
  text-align: inherit;
}

.editable-range-input::-webkit-inner-spin-button,
.editable-range-input::-webkit-outer-spin-button {
  margin: 0;
}

.editable-range-suffix {
  margin-left: 3px;
  white-space: nowrap;
}

.editable-range-error {
  position: absolute;
  top: calc(100% + 5px);
  right: 0;
  z-index: 20;
  width: max-content;
  max-width: 220px;
  padding: 4px 7px;
  border-radius: 5px;
  background: var(--md-surface-container-high, #2b2930);
  color: var(--md-error, #ba1a1a);
  font-size: 11px;
  font-weight: 500;
  line-height: 1.35;
  white-space: normal;
  pointer-events: none;
}
</style>
