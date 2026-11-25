<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import type { TrackStat } from '@/stores/playbackStats'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import { useStatsFormat } from './useStatsFormat'

// 本期收听最久的一首（对齐 Android statsSpotlight），整卡可点即播
defineProps<{
  track: TrackStat
  active: boolean
  starting: boolean
}>()

const emit = defineEmits<{
  play: [track: TrackStat]
  menu: [event: MouseEvent, track: TrackStat]
}>()

const { t } = useI18n()
const format = useStatsFormat()
</script>

<template>
  <button
    type="button"
    class="spotlight"
    :class="{ active }"
    :aria-label="t('stats.play_track', { name: track.name || t('stats.unknown_track') })"
    @click="emit('play', track)"
    @contextmenu="emit('menu', $event, track)"
  >
    <span class="spotlight-cover">
      <BilibiliCoverImage v-if="track.coverUrl" :src="track.coverUrl" loading="lazy">
        <span class="material-symbols-rounded filled">music_note</span>
      </BilibiliCoverImage>
      <span v-else class="material-symbols-rounded filled">music_note</span>
    </span>
    <span class="spotlight-text">
      <span class="spotlight-eyebrow">{{ t('stats.spotlight') }}</span>
      <span class="spotlight-title">{{ track.name || t('stats.unknown_track') }}</span>
      <span class="spotlight-meta">
        <span v-if="track.artist" class="spotlight-artist">{{ track.artist }}</span>
        <span>{{ format.duration(track.totalListenMs) }}</span>
        <span>{{ format.plays(track.playCount) }}</span>
      </span>
    </span>
    <span class="spotlight-play" aria-hidden="true">
      <span class="material-symbols-rounded filled" :class="{ spinning: starting }">{{ starting ? 'progress_activity' : 'play_arrow' }}</span>
    </span>
  </button>
</template>

<style scoped lang="scss">
@use './statsCard.scss' as card;

.spotlight {
  display: flex;
  align-items: center;
  gap: 18px;
  width: 100%;
  min-width: 0;
  padding: 18px;
  border-radius: var(--radius-xl);
  background: var(--md-surface-container-low);
  box-shadow: 0 0 0 1px var(--hairline);
  text-align: left;
  transition: background var(--duration-short), box-shadow var(--duration-short), transform var(--duration-short);

  &:hover {
    background: color-mix(in srgb, var(--md-on-surface) 3%, var(--md-surface-container-low));
    box-shadow: 0 0 0 1px var(--hairline), var(--shadow-2);

    .spotlight-play { transform: scale(1.06); }
  }

  &:active { transform: scale(0.99); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 2px; }
  &.active .spotlight-title { color: var(--md-primary); }
}

.spotlight-cover {
  @include card.artwork(84px, var(--radius-md));

  box-shadow: inset 0 0 0 1px var(--hairline), var(--shadow-1);
}

.spotlight-text {
  display: flex;
  flex: 1;
  min-width: 0;
  flex-direction: column;
  gap: 4px;
}

.spotlight-eyebrow {
  font-size: var(--fs-12);
  font-weight: 600;
  color: var(--md-on-surface-variant);
}

.spotlight-title {
  display: -webkit-box;
  overflow: hidden;
  font-size: calc(21px * var(--text-scale));
  font-weight: 700;
  letter-spacing: var(--tracking-tight);
  line-height: 1.25;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  transition: color var(--duration-short);
}

.spotlight-meta {
  display: flex;
  min-width: 0;
  flex-wrap: wrap;
  font-size: var(--fs-13);
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;

  > span + span::before {
    content: '·';
    margin: 0 6px;
    opacity: 0.6;
  }
}

.spotlight-artist {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.spotlight-play {
  display: grid;
  place-items: center;
  flex: 0 0 44px;
  height: 44px;
  border-radius: var(--radius-full);
  background: var(--md-primary);
  color: var(--md-on-primary);
  box-shadow: 0 4px 14px color-mix(in srgb, var(--md-primary) 30%, transparent);
  transition: transform var(--duration-short) var(--ease-emphasized);

  .material-symbols-rounded { font-size: 26px; }
}

.spinning { animation: spotlight-spin 1s linear infinite; }

@keyframes spotlight-spin { to { transform: rotate(360deg); } }

@media (prefers-reduced-motion: reduce) {
  .spotlight,
  .spotlight-play { transition: none; }
  .spinning { animation: none; }
}
</style>
