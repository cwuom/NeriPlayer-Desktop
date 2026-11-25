<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  HOURS_PER_DAY,
  clampHour,
  hourBarRatio,
  isHourAxisTick,
  type HourlyRhythm,
} from '@/modules/library/playbackHourlyRhythm'
import { useStatsFormat } from './useStatsFormat'

// 各时段收听柱状图（对齐 Android PlaybackStatsRhythm 的小时模式）：默认选中高峰时段
const props = defineProps<{
  rhythm: HourlyRhythm
}>()

const { t, locale } = useI18n()
const format = useStatsFormat()

const chosen = ref<number | null>(null)
const selected = computed(() => chosen.value ?? props.rhythm.peakIndex)
const hourIndexes = Array.from({ length: HOURS_PER_DAY }, (_, hour) => hour)

// 切换时间范围后回到新的高峰时段
watch(() => props.rhythm, () => { chosen.value = null })

const hourFormat = computed(() => {
  try {
    return new Intl.DateTimeFormat(locale.value, { hour: 'numeric', minute: '2-digit' })
  } catch {
    return new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' })
  }
})

function hourLabel(hour: number): string {
  return hourFormat.value.format(new Date(2000, 0, 1, hour))
}

function slotLabel(hour: number): string {
  return t('stats.hourly_selected', { hour: hourLabel(hour), duration: format.duration(props.rhythm.hours[hour] ?? 0) })
}

function select(hour: number) {
  chosen.value = clampHour(hour)
}

function onKeydown(event: KeyboardEvent) {
  const step = { ArrowLeft: -1, ArrowRight: 1, Home: -HOURS_PER_DAY, End: HOURS_PER_DAY }[event.key]
  if (step === undefined) return
  event.preventDefault()
  select(selected.value + step)
}
</script>

<template>
  <section class="stats-card hourly-rhythm">
    <div class="card-head">
      <h2 class="card-title">{{ t('stats.hourly_title') }}</h2>
      <div class="slot-nav">
        <button
          type="button"
          class="nav-btn"
          :aria-label="t('stats.hourly_previous')"
          :disabled="selected <= 0"
          @click="select(selected - 1)"
        >
          <span class="material-symbols-rounded" aria-hidden="true">chevron_left</span>
        </button>
        <button
          type="button"
          class="nav-btn"
          :aria-label="t('stats.hourly_next')"
          :disabled="selected >= HOURS_PER_DAY - 1"
          @click="select(selected + 1)"
        >
          <span class="material-symbols-rounded" aria-hidden="true">chevron_right</span>
        </button>
      </div>
    </div>

    <div class="slot-summary" aria-live="polite">
      <span class="slot-hour">{{ hourLabel(selected) }}</span>
      <span v-if="rhythm.hours[selected] === rhythm.peakMs" class="slot-peak">{{ t('stats.hourly_peak') }}</span>
      <span class="slot-duration">{{ format.duration(rhythm.hours[selected] ?? 0) }}</span>
    </div>

    <div class="hour-bars" role="group" :aria-label="t('stats.hourly_title')" @keydown="onKeydown">
      <button
        v-for="hour in hourIndexes"
        :key="hour"
        type="button"
        class="hour-bar"
        :class="{ selected: hour === selected, empty: !rhythm.hours[hour] }"
        :tabindex="hour === selected ? 0 : -1"
        :aria-pressed="hour === selected"
        :aria-label="slotLabel(hour)"
        @click="select(hour)"
      >
        <span class="hour-fill" :style="{ height: `${hourBarRatio(rhythm.hours[hour] ?? 0, rhythm.peakMs) * 100}%` }" />
      </button>
    </div>
    <div class="hour-axis" aria-hidden="true">
      <span v-for="hour in hourIndexes" :key="hour" class="axis-tick">{{ isHourAxisTick(hour) ? hour : '' }}</span>
    </div>
  </section>
</template>

<style scoped lang="scss">
@use './statsCard.scss' as card;

@include card.card;

.slot-nav {
  display: flex;
  gap: 2px;
  margin: -6px -8px 0 0;
}

.nav-btn {
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short), color var(--duration-short);

  .material-symbols-rounded { font-size: 20px; }

  &:hover:not(:disabled) {
    background: var(--state-hover);
    color: var(--md-on-surface);
  }

  &:disabled { opacity: 0.38; cursor: default; }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 2px; }
}

.slot-summary {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 4px 8px;
  margin-bottom: 14px;
}

.slot-hour {
  font-size: var(--fs-14);
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}

.slot-peak {
  padding: 1px 8px;
  border-radius: var(--radius-full);
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);
  font-size: var(--fs-12);
  font-weight: 600;
}

.slot-duration {
  font-size: var(--fs-13);
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;
}

.hour-bars {
  display: grid;
  grid-template-columns: repeat(24, minmax(0, 1fr));
  align-items: end;
  height: 96px;
  // 三条参考线，和 Android 画布上的横线一致
  background:
    linear-gradient(var(--hairline), var(--hairline)) 0 33.33% / 100% 1px no-repeat,
    linear-gradient(var(--hairline), var(--hairline)) 0 66.66% / 100% 1px no-repeat,
    linear-gradient(var(--md-outline-variant), var(--md-outline-variant)) 0 100% / 100% 1px no-repeat;
}

.hour-bar {
  display: flex;
  align-items: flex-end;
  justify-content: center;
  height: 100%;
  padding: 0 1px;
  border-radius: var(--radius-xs);
  cursor: pointer;

  &:hover .hour-fill { background: var(--md-primary); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 1px; }
  &.empty { cursor: default; }
}

.hour-fill {
  width: min(76%, 14px);
  border-radius: 4px 4px 2px 2px;
  // 未选中的柱子降一级，图表不会整片发亮
  background: color-mix(in srgb, var(--md-primary) 40%, transparent);
  transition:
    height 480ms var(--ease-emphasized, cubic-bezier(0.2, 0, 0, 1)),
    background var(--duration-short);

  .selected & { background: var(--md-primary); }
}

.hour-axis {
  display: grid;
  grid-template-columns: repeat(24, minmax(0, 1fr));
  margin-top: 6px;
}

.axis-tick {
  overflow: visible;
  font-size: var(--fs-11);
  color: var(--md-on-surface-variant);
  text-align: center;
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}

@media (prefers-reduced-motion: reduce) {
  .hour-fill { transition: none; }
}
</style>
