<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import EditableRangeValue from '@/components/ui/EditableRangeValue.vue'
import { audioEffectsFrequencyResponse, GRAPHIC_EQ_FREQUENCIES, type AudioEffectsSound } from '@/modules/audioEffects/model'

const props = withDefaults(defineProps<{ sound: AudioEffectsSound, volume?: number, disabled?: boolean }>(),
  { volume: 1, disabled: false })
const emit = defineEmits<{ band: [index: number, value: number] }>()
const { t } = useI18n()

const WIDTH = 600
const HEIGHT = 220
const TOP = 16
const BOTTOM = 196
const BAND_LIMIT = 12
// 10 段正好按倍频程排列，横轴取首末频段各外扩半个倍频程，频段中心与下方滑块列严格对齐
const F_MIN = GRAPHIC_EQ_FREQUENCIES[0] / Math.SQRT2
const F_MAX = GRAPHIC_EQ_FREQUENCIES[GRAPHIC_EQ_FREQUENCIES.length - 1] * Math.SQRT2

const response = computed(() => audioEffectsFrequencyResponse(props.sound, props.volume, 140)
  .filter(point => point.frequencyHz >= F_MIN && point.frequencyHz <= F_MAX))
const limit = computed(() => Math.max(BAND_LIMIT, Math.min(30,
  Math.ceil(Math.max(...response.value.map(point => Math.abs(point.gainDb)), 0) / 6) * 6)))
const x = (frequency: number) => Math.log(frequency / F_MIN) / Math.log(F_MAX / F_MIN) * WIDTH
const y = (gain: number) => {
  const middle = (TOP + BOTTOM) / 2
  return middle - Math.max(-limit.value, Math.min(limit.value, gain)) / limit.value * (BOTTOM - TOP) / 2
}
const curve = computed(() => response.value.map((point, index) =>
  `${index ? 'L' : 'M'}${x(point.frequencyHz).toFixed(1)},${y(point.gainDb).toFixed(1)}`).join(' '))
const area = computed(() => `${curve.value} L${WIDTH},${y(0).toFixed(1)} L0,${y(0).toFixed(1)} Z`)
const gridGains = computed(() => [-limit.value, -limit.value / 2, 0, limit.value / 2, limit.value])
const bands = computed(() => GRAPHIC_EQ_FREQUENCIES.map((frequency, index) => ({
  index, frequency, gain: props.sound.equalizerBandsDb[index] ?? 0,
  label: frequency >= 1000 ? `${frequency / 1000}k` : String(Math.round(frequency)),
})))
const formatGain = (gain: number) => `${gain > 0 ? '+' : ''}${Number(gain.toFixed(1))}`

const svgRef = ref<SVGSVGElement | null>(null)
const activeBand = ref<number | null>(null)
let fine = false

function pointerTarget(event: PointerEvent) {
  const rect = svgRef.value?.getBoundingClientRect()
  if (!rect || rect.width <= 0 || rect.height <= 0) return null
  const px = (event.clientX - rect.left) / rect.width * WIDTH
  const py = (event.clientY - rect.top) / rect.height * HEIGHT
  const index = Math.max(0, Math.min(GRAPHIC_EQ_FREQUENCIES.length - 1, Math.floor(px / (WIDTH / GRAPHIC_EQ_FREQUENCIES.length))))
  const middle = (TOP + BOTTOM) / 2
  const raw = (middle - py) / ((BOTTOM - TOP) / 2) * limit.value
  const step = fine ? 0.1 : 0.5
  const gain = Math.max(-BAND_LIMIT, Math.min(BAND_LIMIT, Math.round(raw / step) * step))
  return { index, gain: Object.is(gain, -0) ? 0 : gain }
}
function paint(event: PointerEvent) {
  fine = event.shiftKey || event.altKey
  const target = pointerTarget(event)
  if (!target) return
  activeBand.value = target.index
  if (target.gain !== bands.value[target.index].gain) emit('band', target.index, target.gain)
}
function onPointerDown(event: PointerEvent) {
  if (props.disabled || event.button !== 0) return
  event.preventDefault()
  svgRef.value?.setPointerCapture?.(event.pointerId)
  paint(event)
}
function onPointerMove(event: PointerEvent) {
  if (activeBand.value === null) return
  paint(event)
}
function onPointerUp(event: PointerEvent) {
  if (activeBand.value === null) return
  svgRef.value?.releasePointerCapture?.(event.pointerId)
  activeBand.value = null
}
function onDoubleClick(event: MouseEvent) {
  if (props.disabled) return
  const rect = svgRef.value?.getBoundingClientRect()
  if (!rect || rect.width <= 0) return
  const index = Math.max(0, Math.min(9, Math.floor((event.clientX - rect.left) / rect.width * 10)))
  emit('band', index, 0)
}
function onSlider(index: number, event: Event) {
  emit('band', index, Number((event.target as HTMLInputElement).value))
}
const fillStyle = (gain: number) => {
  const percent = (gain + BAND_LIMIT) / (BAND_LIMIT * 2) * 100
  return { '--fill-from': `${Math.min(50, percent)}%`, '--fill-to': `${Math.max(50, percent)}%` }
}
</script>

<template>
  <div class="dsp-eq-editor" :class="{ disabled, dragging: activeBand !== null }">
    <div class="dsp-eq-graph">
      <svg ref="svgRef" :viewBox="`0 0 ${WIDTH} ${HEIGHT}`" preserveAspectRatio="none" role="img"
        :aria-label="t('dsp.eq_editor.graph')" @pointerdown="onPointerDown" @pointermove="onPointerMove"
        @pointerup="onPointerUp" @pointercancel="onPointerUp" @dblclick="onDoubleClick">
        <defs>
          <linearGradient id="dsp-eq-fill" x1="0" x2="0" y1="0" y2="1">
            <stop offset="0%" stop-color="var(--md-primary)" stop-opacity=".38" />
            <stop offset="50%" stop-color="var(--md-primary)" stop-opacity=".08" />
            <stop offset="100%" stop-color="var(--md-primary)" stop-opacity=".38" />
          </linearGradient>
        </defs>
        <rect v-for="band in bands" :key="`col-${band.index}`" class="dsp-eq-column"
          :class="{ active: activeBand === band.index }" :x="band.index * WIDTH / 10" :y="TOP - 8"
          :width="WIDTH / 10" :height="BOTTOM - TOP + 16" rx="8" />
        <line v-for="gain in gridGains" :key="`g-${gain}`" x1="0" :x2="WIDTH" :y1="y(gain)" :y2="y(gain)"
          :class="{ zero: gain === 0 }" />
        <path class="dsp-eq-area" :d="area" />
        <path class="dsp-eq-curve" :d="curve" />
        <g v-for="band in bands" :key="`dot-${band.index}`" class="dsp-eq-dot" :class="{ active: activeBand === band.index, raised: band.gain !== 0 }">
          <line :x1="x(band.frequency)" :x2="x(band.frequency)" :y1="y(0)" :y2="y(band.gain)" />
          <circle :cx="x(band.frequency)" :cy="y(band.gain)" r="6" />
        </g>
      </svg>
      <div class="dsp-eq-scale" aria-hidden="true">
        <span v-for="gain in [limit, 0, -limit]" :key="`s-${gain}`">{{ formatGain(gain) }}</span>
      </div>
      <div v-if="activeBand !== null" class="dsp-eq-tooltip" aria-hidden="true"
        :style="{ left: `${(activeBand + 0.5) * 10}%` }">
        {{ bands[activeBand].label }} Hz · {{ formatGain(bands[activeBand].gain) }} dB
      </div>
    </div>
    <p class="dsp-eq-hint">{{ t('dsp.eq_editor.hint') }}</p>
    <div class="dsp-eq-strip" role="group" :aria-label="t('dsp.graphic_eq')">
      <div v-for="band in bands" :key="band.frequency" class="setting-card effects-band" :class="{ changed: band.gain !== 0 }">
        <EditableRangeValue :model-value="band.gain" :min="-BAND_LIMIT" :max="BAND_LIMIT" :step="0.1"
          :display-value="formatGain(band.gain)" input-suffix="dB" :input-width="56"
          :aria-label="t('settings.audio_effects_band', { frequency: band.frequency })"
          @update:model-value="emit('band', band.index, $event)" />
        <div class="dsp-eq-slider" :style="fillStyle(band.gain)">
          <input type="range" :min="-BAND_LIMIT" :max="BAND_LIMIT" step="0.1" :value="band.gain" :disabled="disabled"
            :aria-label="t('settings.audio_effects_band', { frequency: band.frequency })"
            @input="onSlider(band.index, $event)" @dblclick="emit('band', band.index, 0)" />
        </div>
        <span class="setting-title">{{ band.label }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped lang="scss">
.dsp-eq-editor { display: flex; flex-direction: column; gap: 10px; transition: opacity .25s ease; }
.dsp-eq-editor.disabled { opacity: .5; }
.dsp-eq-graph {
  position: relative; padding: 12px 12px 8px 40px; border-radius: var(--radius-md);
  background: radial-gradient(120% 140% at 50% 0%, color-mix(in srgb, var(--md-primary) 10%, var(--md-surface-container)) 0%, var(--md-surface-container) 70%);
}
svg { display: block; width: 100%; height: 200px; overflow: visible; cursor: crosshair; touch-action: none; user-select: none; }
.disabled svg { cursor: default; }
.dsp-eq-column { fill: transparent; transition: fill .2s ease; }
.dsp-eq-column:hover, .dsp-eq-column.active { fill: color-mix(in srgb, var(--md-primary) 7%, transparent); }
line { stroke: var(--md-outline-variant); stroke-width: .7; stroke-dasharray: 3 5; vector-effect: non-scaling-stroke; }
line.zero { stroke: var(--md-on-surface-variant); stroke-opacity: .5; stroke-dasharray: none; }
.dsp-eq-area { fill: url(#dsp-eq-fill); transition: d .18s ease-out; pointer-events: none; }
.dsp-eq-curve {
  fill: none; stroke: var(--md-primary); stroke-width: 2.6; stroke-linejoin: round; stroke-linecap: round;
  vector-effect: non-scaling-stroke; transition: d .18s ease-out; pointer-events: none;
  filter: drop-shadow(0 2px 6px color-mix(in srgb, var(--md-primary) 45%, transparent));
}
.dsp-eq-dot { pointer-events: none; }
.dsp-eq-dot line { stroke: var(--md-primary); stroke-opacity: 0; stroke-dasharray: none; transition: y2 .18s ease-out, stroke-opacity .2s; }
.dsp-eq-dot.raised line { stroke-opacity: .35; }
.dsp-eq-dot circle {
  fill: var(--md-surface-container); stroke: var(--md-primary); stroke-width: 2.4; vector-effect: non-scaling-stroke;
  transition: cy .18s ease-out, r .18s ease, fill .2s;
}
.dsp-eq-dot.raised circle { fill: var(--md-primary); }
.dsp-eq-dot.active circle { r: 9; }
.dsp-eq-scale {
  position: absolute; top: 12px; bottom: 8px; left: 6px; display: flex; flex-direction: column; justify-content: space-between;
  padding: 10px 0; color: var(--md-on-surface-variant); font-size: var(--fs-10); font-variant-numeric: tabular-nums;
}
.dsp-eq-tooltip {
  position: absolute; top: 2px; transform: translateX(calc(-50% + 14px)); padding: 4px 9px; border-radius: 999px;
  background: var(--md-inverse-surface, var(--md-on-surface)); color: var(--md-inverse-on-surface, var(--md-surface));
  font-size: var(--fs-11); font-weight: 600; white-space: nowrap; pointer-events: none; animation: dsp-pop .16s ease-out;
}
@keyframes dsp-pop { from { opacity: 0; transform: translate(calc(-50% + 14px), 4px) scale(.94); } }
.dsp-eq-hint { margin: 0 2px; color: var(--md-on-surface-variant); font-size: var(--fs-11-5); line-height: 1.5; }
.dsp-eq-strip { display: grid; grid-template-columns: repeat(10, minmax(0, 1fr)); gap: 6px; padding-left: 40px; }
.effects-band {
  display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 10px 2px 8px; min-width: 0;
  border-radius: var(--radius-md); background: var(--md-surface-container); border: 1px solid transparent;
  transition: border-color .25s ease, background-color .25s ease;
}
.effects-band.changed { border-color: color-mix(in srgb, var(--md-primary) 30%, transparent); }
.effects-band :deep(.editable-range-value) { color: var(--md-primary); font-size: var(--fs-11-5); font-weight: 600; font-variant-numeric: tabular-nums; }
.effects-band .setting-title { color: var(--md-on-surface-variant); font-size: var(--fs-11); font-weight: 500; }
.dsp-eq-slider { position: relative; width: 22px; height: 116px; display: flex; justify-content: center; }
.dsp-eq-slider::before, .dsp-eq-slider::after {
  content: ''; position: absolute; left: 50%; width: 4px; border-radius: 4px; transform: translateX(-50%); pointer-events: none;
}
.dsp-eq-slider::before { top: 0; bottom: 0; background: var(--md-surface-container-highest); }
.dsp-eq-slider::after {
  bottom: var(--fill-from); height: calc(var(--fill-to) - var(--fill-from)); background: var(--md-primary);
  transition: bottom .12s ease-out, height .12s ease-out;
}
.dsp-eq-slider input {
  position: relative; z-index: 1; margin: 0; width: 22px; height: 116px; appearance: none; background: transparent;
  writing-mode: vertical-lr; direction: rtl; cursor: pointer;
}
.dsp-eq-slider input:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 2px; border-radius: 6px; }
.dsp-eq-slider input::-webkit-slider-thumb {
  appearance: none; width: 18px; height: 18px; border-radius: 50%; background: var(--md-primary);
  border: 3px solid var(--md-surface-container); transition: transform .15s ease, box-shadow .15s ease;
}
.dsp-eq-slider input:hover::-webkit-slider-thumb { box-shadow: 0 0 0 5px color-mix(in srgb, var(--md-primary) 18%, transparent); }
.dsp-eq-slider input:active::-webkit-slider-thumb { transform: scale(1.15); }
@media (max-width: 680px) {
  .dsp-eq-graph { padding-left: 32px; }
  .dsp-eq-strip { padding-left: 0; gap: 3px; }
  .dsp-eq-slider, .dsp-eq-slider input { height: 96px; }
}
</style>
