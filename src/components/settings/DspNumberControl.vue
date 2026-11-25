<script setup lang="ts">
import { computed } from 'vue'
import EditableRangeValue from '@/components/ui/EditableRangeValue.vue'

const props = withDefaults(defineProps<{
  modelValue: number
  label: string
  help?: string
  min: number
  max: number
  step?: number
  unit?: string
  scale?: number
  minLabel?: string
  maxLabel?: string
  valueLabel?: string
  /** 双击滑块恢复的值；同时作为填充轨道的起点 */
  defaultValue?: number
  disabled?: boolean
}>(), { step: 0.1, unit: '', scale: 1, help: undefined, minLabel: undefined, maxLabel: undefined,
  valueLabel: undefined, defaultValue: undefined, disabled: false })
const emit = defineEmits<{ 'update:modelValue': [value: number] }>()
const displayValue = computed(() => {
  const value = props.modelValue * props.scale
  const decimals = props.unit === 'Hz' ? 2 : 3
  return `${Number(value.toFixed(decimals))}${props.unit ? ` ${props.unit}` : ''}`
})
const percent = (value: number) => props.max === props.min ? 0
  : Math.max(0, Math.min(100, (value - props.min) / (props.max - props.min) * 100))
const origin = computed(() => props.defaultValue ?? (props.min < 0 && props.max > 0 ? 0 : props.min))
const fill = computed(() => {
  const start = percent(origin.value), end = percent(props.modelValue)
  return { '--fill-from': `${Math.min(start, end)}%`, '--fill-to': `${Math.max(start, end)}%`, '--thumb': `${end}%` }
})
const changed = computed(() => Math.abs(props.modelValue - origin.value) > props.step / 2)
function reset() {
  if (props.defaultValue !== undefined && !props.disabled) emit('update:modelValue', props.defaultValue)
}
</script>

<template>
  <div class="setting-card dsp-number-control" :class="{ changed, disabled }">
    <div class="dsp-number-heading">
      <div class="dsp-number-info">
        <div class="setting-title">{{ label }}</div>
        <div v-if="help" class="setting-desc">{{ help }}</div>
      </div>
      <EditableRangeValue :model-value="modelValue" :min="min" :max="max" :step="step" :input-scale="scale"
        :display-value="valueLabel ?? displayValue" :input-suffix="unit" :aria-label="label"
        @update:model-value="emit('update:modelValue', $event)" />
    </div>
    <div class="dsp-number-range" :style="fill">
      <span>{{ minLabel ?? Number((min * scale).toFixed(2)) }}</span>
      <div class="dsp-number-track">
        <input type="range" :min="min" :max="max" :step="step" :value="modelValue" :aria-label="label" :disabled="disabled"
          @input="emit('update:modelValue', Number(($event.target as HTMLInputElement).value))" @dblclick="reset" />
      </div>
      <span>{{ maxLabel ?? `${Number((max * scale).toFixed(2))} ${unit}` }}</span>
    </div>
  </div>
</template>

<style scoped lang="scss">
/* 根节点同时带父组件的 scope，父级的 primitives.card 会作用到这里；用双类提高优先级，只接管内部排版 */
.setting-card.dsp-number-control {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 10px;
  min-height: 0;
  padding: 14px 18px 12px;
  transition: opacity .25s ease;
}
.setting-card.dsp-number-control.disabled { opacity: .5; }
.dsp-number-heading { display: flex; gap: 12px; align-items: flex-start; justify-content: space-between; }
.dsp-number-info { flex: 1; min-width: 0; }
.setting-title { font-size: var(--fs-14); font-weight: 600; letter-spacing: var(--tracking-title); }
.setting-desc { margin-top: 3px; color: var(--md-on-surface-variant); font-size: var(--fs-12); line-height: 1.5; }
.dsp-number-heading :deep(.editable-range-value) {
  flex-shrink: 0; min-width: 44px; padding: 3px 10px; border-radius: var(--radius-full);
  background: var(--md-surface-container-high); color: var(--md-on-surface);
  font-size: var(--fs-12); font-weight: 600; font-variant-numeric: tabular-nums; text-align: center;
  transition: background-color .25s ease, color .25s ease;
}
/* 偏离默认值时数值胶囊染强调色，一眼看出哪些参数动过 */
.changed .dsp-number-heading :deep(.editable-range-value) {
  background: color-mix(in srgb, var(--md-primary) 14%, transparent); color: var(--md-primary);
}
.dsp-number-range { display: flex; gap: 10px; align-items: center; }
.dsp-number-range > span {
  flex-shrink: 0; color: var(--md-on-surface-variant); font-size: var(--fs-11);
  font-variant-numeric: tabular-nums; white-space: nowrap;
}
.dsp-number-track { position: relative; flex: 1; min-width: 0; height: 20px; display: flex; align-items: center; }
.dsp-number-track::before, .dsp-number-track::after {
  content: ''; position: absolute; top: 50%; height: 4px; border-radius: var(--radius-full); transform: translateY(-50%); pointer-events: none;
}
.dsp-number-track::before { left: 0; right: 0; background: color-mix(in srgb, var(--md-on-surface) 14%, transparent); }
/* 填充从默认值（双极参数为 0）画到当前值，而不是从最小值开始 */
.dsp-number-track::after {
  left: var(--fill-from); width: calc(var(--fill-to) - var(--fill-from)); background: var(--md-primary);
  transition: left .12s ease-out, width .12s ease-out;
}
input {
  position: relative; z-index: 1; flex: 1; min-width: 0; margin: 0; appearance: none; height: 20px;
  background: transparent; outline: none; cursor: pointer;
}
input:disabled { cursor: default; }
/* 拇指与全局 .m3-slider 一致：白色圆点 + 投影 */
input::-webkit-slider-thumb {
  appearance: none; width: 18px; height: 18px; border-radius: 50%; background: #fff;
  box-shadow: 0 0 0 0.5px rgb(0 0 0 / 0.12), 0 2px 6px rgb(0 0 0 / 0.22);
  transition: transform .15s ease, box-shadow .15s ease;
}
input:hover::-webkit-slider-thumb { transform: scale(1.08); }
input:active::-webkit-slider-thumb { transform: scale(0.94); }
input:focus-visible::-webkit-slider-thumb {
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--md-primary) 40%, transparent), 0 2px 6px rgb(0 0 0 / 0.22);
}
input::-moz-range-thumb { width: 18px; height: 18px; border: 0; border-radius: 50%; background: #fff; box-shadow: 0 2px 6px rgb(0 0 0 / 0.22); }
</style>
