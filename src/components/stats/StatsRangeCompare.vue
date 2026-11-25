<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { STATS_PERIODS, type StatsPeriod, type StatsSummary } from '@/stores/playbackStats'
import { useStatsFormat } from './useStatsFormat'

// 收听节律：桌面端没有按天分桶明细，画不出 Android 的星期/小时分布，
// 改为对比各时间范围的累计时长，同时充当区间切换入口
const props = defineProps<{
  summaries: Record<StatsPeriod, StatsSummary>
  active: StatsPeriod
}>()

const emit = defineEmits<{ select: [period: StatsPeriod] }>()

const { t } = useI18n()
const format = useStatsFormat()

const peak = computed(() => Math.max(1, ...STATS_PERIODS.map(period => props.summaries[period].totalListenMs)))
const rows = computed(() => STATS_PERIODS.map((period) => {
  const summary = props.summaries[period]
  const ratio = summary.totalListenMs > 0 ? Math.max(0.02, summary.totalListenMs / peak.value) : 0
  return {
    period,
    label: t(`stats.range_${period}`),
    duration: format.duration(summary.totalListenMs),
    plays: format.plays(summary.totalPlayCount),
    clip: `inset(0 ${((1 - ratio) * 100).toFixed(2)}% 0 0 round 999px)`,
  }
}))
</script>

<template>
  <section class="stats-card range-compare">
    <div class="card-head">
      <div>
        <h2 class="card-title">{{ t('stats.rhythm') }}</h2>
        <p class="card-subtitle">{{ t('stats.range_compare') }}</p>
      </div>
    </div>
    <div class="range-rows" role="radiogroup" :aria-label="t('stats.period_label')">
      <button
        v-for="row in rows"
        :key="row.period"
        type="button"
        class="range-row"
        :class="{ active: row.period === active }"
        role="radio"
        :aria-checked="row.period === active"
        @click="emit('select', row.period)"
      >
        <span class="range-label">{{ row.label }}</span>
        <span class="range-track" aria-hidden="true">
          <span class="range-fill" :style="{ clipPath: row.clip }" />
        </span>
        <span class="range-value">
          <span class="range-duration">{{ row.duration }}</span>
          <span class="range-plays">{{ row.plays }}</span>
        </span>
      </button>
    </div>
  </section>
</template>

<style scoped lang="scss">
@use './statsCard.scss' as card;

@include card.card;

.range-rows {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin: 0 -10px;
}

.range-row {
  display: grid;
  /* 每行是独立网格，列宽固定才能让各行的条形起止对齐 */
  grid-template-columns: 6em minmax(0, 1fr) 7.5em;
  align-items: center;
  gap: 14px;
  width: 100%;
  min-height: 44px;
  padding: 6px 10px;
  border-radius: var(--radius-md);
  text-align: left;
  transition: background var(--duration-short);

  &:hover { background: var(--state-hover); }
  &:active { background: var(--state-pressed); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: -2px; }

  &.active {
    background: color-mix(in srgb, var(--md-primary) 9%, transparent);

    .range-label { color: var(--md-primary); font-weight: 700; }
    .range-fill { background: var(--md-primary); }
  }
}

.range-label {
  overflow: hidden;
  font-size: var(--fs-13);
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.range-track {
  position: relative;
  height: 10px;
  overflow: hidden;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--md-on-surface) 7%, transparent);
}

.range-fill {
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background: color-mix(in srgb, var(--md-primary) 40%, transparent);
  transition: clip-path 520ms var(--ease-emphasized, cubic-bezier(0.2, 0, 0, 1)), background var(--duration-short);
}

.range-value {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  min-width: 0;
  font-variant-numeric: tabular-nums;
}

.range-duration {
  font-size: var(--fs-13);
  font-weight: 600;
  white-space: nowrap;
}

.range-plays {
  font-size: var(--fs-11);
  color: var(--md-on-surface-variant);
  white-space: nowrap;
}

@media (prefers-reduced-motion: reduce) {
  .range-fill { transition: none; }
}
</style>
