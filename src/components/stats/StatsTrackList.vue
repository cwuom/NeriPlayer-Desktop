<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import type { TrackStat } from '@/stores/playbackStats'
import type { StatsSortMode } from '@/modules/library/playbackStatsInsights'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import { displayAlbum } from '@/modules/library/albumDisplay'
import { useStatsFormat } from './useStatsFormat'

// 歌曲排行列表（对齐 Android StatTrackRow）：右侧主数值跟随当前排序方式
const props = defineProps<{
  items: TrackStat[]
  mode: StatsSortMode
  startingKey: string
  isCurrent: (item: TrackStat) => boolean
}>()

const emit = defineEmits<{
  play: [item: TrackStat]
  menu: [event: MouseEvent, item: TrackStat]
}>()

const { t } = useI18n()
const format = useStatsFormat()

function primary(item: TrackStat): string {
  switch (props.mode) {
    case 'listen_time': return format.duration(item.totalListenMs)
    case 'recent': return format.date(item.lastPlayedAt) || '—'
    case 'first_played': return format.date(item.firstPlayedAt) || '—'
    default: return format.plays(item.playCount)
  }
}

function secondary(item: TrackStat): string {
  return props.mode === 'play_count' ? format.duration(item.totalListenMs) : format.plays(item.playCount)
}

function onKeydown(event: KeyboardEvent, item: TrackStat) {
  if (event.key !== 'Enter' && event.key !== ' ') return
  event.preventDefault()
  emit('play', item)
}
</script>

<template>
  <ol class="stat-list">
    <li
      v-for="(item, index) in items"
      :key="item.identityKey"
      class="stat-row"
      :class="{ active: isCurrent(item) }"
      tabindex="0"
      :aria-label="t('stats.play_track', { name: item.name || t('stats.unknown_track') })"
      @click="emit('play', item)"
      @keydown="onKeydown($event, item)"
      @contextmenu="emit('menu', $event, item)"
    >
      <span class="stat-index">
        <span v-if="startingKey === item.identityKey" class="material-symbols-rounded spinning">progress_activity</span>
        <span v-else class="stat-rank" :class="{ top: index < 3 }">{{ index + 1 }}</span>
      </span>
      <span class="stat-cover">
        <BilibiliCoverImage v-if="item.coverUrl" :src="item.coverUrl" loading="lazy">
          <span class="material-symbols-rounded filled">music_note</span>
        </BilibiliCoverImage>
        <span v-else class="material-symbols-rounded filled">music_note</span>
      </span>
      <span class="stat-info">
        <span class="stat-title">{{ item.name || t('stats.unknown_track') }}</span>
        <span class="stat-artist">{{ item.artist }}</span>
      </span>
      <span class="stat-album">{{ displayAlbum(item.album) }}</span>
      <span class="stat-values">
        <span class="stat-primary">{{ primary(item) }}</span>
        <span class="stat-secondary">{{ secondary(item) }}</span>
      </span>
    </li>
  </ol>
</template>

<style scoped lang="scss">
@use './statsCard.scss' as card;

.stat-list {
  display: flex;
  flex-direction: column;
  gap: 1px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.stat-row {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 60px;
  padding: 7px 12px 7px 6px;
  border-radius: var(--radius-md);
  cursor: pointer;
  // 长列表交给浏览器跳过屏幕外行的渲染
  content-visibility: auto;
  contain-intrinsic-size: auto 60px;
  transition: background 120ms ease;

  &:hover { background: var(--state-hover); }
  &:active { background: var(--state-pressed); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: -2px; }

  &.active {
    background: color-mix(in srgb, var(--md-primary) 9%, transparent);

    .stat-title { color: var(--md-primary); }
  }
}

.stat-index {
  display: grid;
  place-items: center;
  flex: 0 0 32px;

  .spinning {
    font-size: 18px;
    color: var(--md-primary);
    animation: stat-spin 1s linear infinite;
  }
}

.stat-rank {
  font-size: var(--fs-13);
  font-weight: 500;
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;

  &.top {
    font-weight: 700;
    color: var(--md-on-surface);
  }
}

.stat-cover { @include card.artwork(44px); }

.stat-info {
  display: flex;
  flex: 1 1 0;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
}

.stat-title,
.stat-artist,
.stat-album {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.stat-title {
  font-size: var(--fs-14);
  font-weight: 500;
  transition: color var(--duration-short);
}

.stat-artist,
.stat-album {
  font-size: var(--fs-12);
  color: var(--md-on-surface-variant);
}

/* 专辑列只在内容区足够宽时出现 */
.stat-album {
  display: none;
  flex: 0 1 28%;
  min-width: 0;
}

@container (min-width: 1100px) {
  .stat-album { display: block; }
}

.stat-values {
  display: flex;
  flex: 0 0 auto;
  min-width: 96px;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
  font-variant-numeric: tabular-nums;
}

.stat-primary {
  font-size: var(--fs-13);
  font-weight: 600;
}

.stat-secondary {
  font-size: var(--fs-11);
  color: var(--md-on-surface-variant);
}

@keyframes stat-spin { to { transform: rotate(360deg); } }

@media (prefers-reduced-motion: reduce) {
  .stat-index .spinning { animation: none; }
}
</style>
