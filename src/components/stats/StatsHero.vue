<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useStatsFormat } from './useStatsFormat'

// 总览卡（对齐 Android Clean StatsOverviewCard）：累计时长做主数字，其余指标收在右侧
const props = defineProps<{
  rangeLabel: string
  totalListenMs: number
  totalPlayCount: number
  trackCount: number
  artistCount: number
  averageListenMsPerPlay: number
  earliestListenAt: number | null
}>()

const { t } = useI18n()
const format = useStatsFormat()

const parts = computed(() => format.durationParts(props.totalListenMs))
const metrics = computed(() => [
  { key: 'plays', icon: 'headphones', label: t('stats.play_count'), value: format.number(props.totalPlayCount) },
  { key: 'tracks', icon: 'library_music', label: t('stats.track_count'), value: format.number(props.trackCount) },
  { key: 'artists', icon: 'person', label: t('stats.artist_count'), value: format.number(props.artistCount) },
  {
    key: 'average',
    icon: 'timelapse',
    label: t('stats.play_average'),
    value: props.totalPlayCount > 0 ? format.duration(props.averageListenMsPerPlay) : '—',
  },
])
</script>

<template>
  <section class="stats-hero" :aria-label="t('stats.listen_time')">
    <div class="hero-main">
      <div class="hero-eyebrow">
        <span class="material-symbols-rounded" aria-hidden="true">graphic_eq</span>
        <span>{{ t('stats.listen_time') }}</span>
        <span class="hero-range">{{ rangeLabel }}</span>
      </div>
      <div class="hero-value">
        <template v-for="part in parts" :key="part.unit">
          <span class="hero-number">{{ part.value }}</span>
          <span class="hero-unit">{{ part.unit }}</span>
        </template>
      </div>
      <div v-if="earliestListenAt" class="hero-since">
        {{ t('stats.since_date', { date: format.date(earliestListenAt) }) }}
      </div>
    </div>
    <dl class="hero-metrics">
      <div v-for="metric in metrics" :key="metric.key" class="hero-metric">
        <dt>
          <span class="material-symbols-rounded" aria-hidden="true">{{ metric.icon }}</span>
          {{ metric.label }}
        </dt>
        <dd>{{ metric.value }}</dd>
      </div>
    </dl>
  </section>
</template>

<style scoped lang="scss">
.stats-hero {
  /* 列数跟随统计页容器（.detail-view）宽度 */
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1.15fr);
  align-items: center;
  gap: 20px 32px;
  padding: 24px 26px;
  border-radius: var(--radius-xl);
  background: var(--md-surface-container-low);
  box-shadow: 0 0 0 1px var(--hairline);
}

.hero-main { min-width: 0; }

.hero-eyebrow {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-13);
  font-weight: 600;
  color: var(--md-primary);

  .material-symbols-rounded { font-size: 18px; }
}

.hero-range {
  padding: 1px 8px;
  border-radius: var(--radius-full);
  background: var(--state-hover);
  color: var(--md-on-surface-variant);
  font-size: var(--fs-11);
  font-weight: 600;
}

.hero-value {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  column-gap: 4px;
  margin-top: 10px;
  font-variant-numeric: tabular-nums;
}

.hero-number {
  font-size: calc(clamp(40px, 6cqi, 60px) * var(--text-scale));
  font-weight: 750;
  line-height: 1;
  letter-spacing: -0.03em;
}

.hero-unit {
  margin-right: 10px;
  font-size: calc(17px * var(--text-scale));
  font-weight: 600;
  color: var(--md-on-surface-variant);

  &:last-child { margin-right: 0; }
}

.hero-since {
  margin-top: 12px;
  font-size: var(--fs-12);
  color: var(--md-on-surface-variant);
}

.hero-metrics {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
  margin: 0;
}

.hero-metric {
  min-width: 0;
  padding: 12px 14px;
  border-radius: var(--radius-md);
  background: var(--md-surface);
  box-shadow: 0 0 0 1px var(--hairline);

  dt {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
    font-size: var(--fs-12);
    color: var(--md-on-surface-variant);
    white-space: nowrap;
    text-overflow: ellipsis;

    .material-symbols-rounded { font-size: 16px; color: var(--md-primary); }
  }

  dd {
    margin: 4px 0 0;
    overflow: hidden;
    font-size: calc(21px * var(--text-scale));
    font-weight: 700;
    letter-spacing: var(--tracking-tight);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
}

@container (min-width: 1080px) {
  .hero-metrics { grid-template-columns: repeat(4, minmax(0, 1fr)); }
  .stats-hero { grid-template-columns: minmax(0, 0.9fr) minmax(0, 1.6fr); }
}

@container (max-width: 640px) {
  .stats-hero { grid-template-columns: minmax(0, 1fr); padding: 20px; }
}
</style>
