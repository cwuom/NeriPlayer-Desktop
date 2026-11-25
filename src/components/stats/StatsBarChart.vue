<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import type { TrackStat } from '@/stores/playbackStats'
import { statChartValue, type StatsSortMode } from '@/modules/library/playbackStatsInsights'
import { useStatsFormat } from './useStatsFormat'

// 前几名横向条形图（对齐 Android TopTracksBarChart）：时长排序比时长，其余排序比播放次数
const props = defineProps<{
  title: string
  items: TrackStat[]
  mode: StatsSortMode
  isCurrent: (item: TrackStat) => boolean
}>()

const emit = defineEmits<{
  play: [item: TrackStat]
  menu: [event: MouseEvent, item: TrackStat]
}>()

const { t } = useI18n()
const format = useStatsFormat()

// 按最大值归一，避免第一名占满时其余项挤成一条线
const peak = computed(() => Math.max(1, ...props.items.map(item => statChartValue(item, props.mode))))

// 用 clip-path 裁出长度：两端圆角不会像 scaleX 那样被压扁
function fillClip(item: TrackStat): string {
  const ratio = Math.max(0.03, statChartValue(item, props.mode) / peak.value)
  return `inset(0 ${((1 - ratio) * 100).toFixed(2)}% 0 0 round 999px)`
}

function valueLabel(item: TrackStat): string {
  return props.mode === 'listen_time' ? format.duration(item.totalListenMs) : format.number(item.playCount)
}

function rowLabel(item: TrackStat): string {
  const value = props.mode === 'listen_time' ? format.duration(item.totalListenMs) : format.plays(item.playCount)
  return `${t('stats.play_track', { name: item.name || t('stats.unknown_track') })}, ${value}`
}
</script>

<template>
  <section class="stats-card bar-chart">
    <div class="card-head">
      <h2 class="card-title">{{ title }}</h2>
    </div>
    <div class="bars">
      <button
        v-for="(item, index) in items"
        :key="item.identityKey"
        type="button"
        class="bar-row"
        :class="{ active: isCurrent(item), lead: index === 0 }"
        :aria-label="rowLabel(item)"
        @click="emit('play', item)"
        @contextmenu="emit('menu', $event, item)"
      >
        <span class="bar-head">
          <span class="bar-name">
            <span class="bar-rank">{{ index + 1 }}</span>
            <span class="bar-title">{{ item.name || t('stats.unknown_track') }}</span>
            <span v-if="item.artist" class="bar-artist">{{ item.artist }}</span>
          </span>
          <span class="bar-value">{{ valueLabel(item) }}</span>
        </span>
        <span class="bar-track" aria-hidden="true">
          <span class="bar-fill" :style="{ clipPath: fillClip(item), transitionDelay: `${index * 50}ms` }" />
        </span>
      </button>
    </div>
  </section>
</template>

<style scoped lang="scss">
@use './statsCard.scss' as card;

@include card.card;

.bars {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin: 0 -10px;
}

.bar-row {
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 100%;
  padding: 10px;
  border-radius: var(--radius-md);
  text-align: left;
  transition: background var(--duration-short);

  &:hover {
    background: var(--state-hover);

    .bar-fill { background: var(--md-primary); }
  }

  &:active { background: var(--state-pressed); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: -2px; }
  &.active .bar-title { color: var(--md-primary); }
}

.bar-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  min-width: 0;
}

.bar-name {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}

.bar-rank {
  flex: 0 0 auto;
  min-width: 14px;
  font-size: var(--fs-12);
  font-weight: 600;
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;
}

.bar-title {
  flex: 0 1 auto;
  overflow: hidden;
  font-size: var(--fs-14);
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
  transition: color var(--duration-short);
}

.bar-artist {
  flex: 1 1 0;
  min-width: 40px;
  overflow: hidden;
  font-size: var(--fs-12);
  color: var(--md-on-surface-variant);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.bar-value {
  flex-shrink: 0;
  font-size: var(--fs-13);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.bar-track {
  position: relative;
  height: 8px;
  margin-left: 22px;
  overflow: hidden;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--md-on-surface) 7%, transparent);
}

.bar-fill {
  position: absolute;
  inset: 0;
  border-radius: inherit;
  // 只有第一名和悬停项用满强调色，其余降一级，图表不会整片发亮
  background: color-mix(in srgb, var(--md-primary) 45%, transparent);
  transition:
    clip-path 560ms var(--ease-emphasized, cubic-bezier(0.2, 0, 0, 1)),
    background var(--duration-short);

  .lead & { background: var(--md-primary); }
}

@media (prefers-reduced-motion: reduce) {
  .bar-fill { transition: none; }
}
</style>
