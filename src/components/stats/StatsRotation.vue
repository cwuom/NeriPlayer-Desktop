<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { StatsRank } from '@/modules/library/playbackStatsInsights'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import { useStatsFormat } from './useStatsFormat'

// 本期艺人 / 专辑排行（对齐 Android statsRankInsights）：按收听时长占比排序，默认只展示前几项
const props = defineProps<{
  artists: StatsRank[]
  albums: StatsRank[]
}>()

type RankKind = 'artists' | 'albums'

const COLLAPSED_SIZE = 5
const EXPANDED_LIMIT = 50

const { t } = useI18n()
const format = useStatsFormat()

const selected = ref<RankKind>('artists')
const expanded = ref(false)

const kind = computed<RankKind>(() => {
  if (!props.artists.length) return 'albums'
  if (!props.albums.length) return 'artists'
  return selected.value
})
const ranking = computed(() => (kind.value === 'albums' ? props.albums : props.artists))
const visible = computed(() => ranking.value.slice(0, expanded.value ? EXPANDED_LIMIT : COLLAPSED_SIZE))
const options = computed(() => [
  { value: 'artists' as const, label: t('stats.artist_tab', { count: format.number(props.artists.length) }) },
  { value: 'albums' as const, label: t('stats.album_tab', { count: format.number(props.albums.length) }) },
])

watch(kind, () => { expanded.value = false })

function meterClip(share: number): string {
  return `inset(0 ${((1 - Math.max(0.02, share)) * 100).toFixed(2)}% 0 0 round 999px)`
}
</script>

<template>
  <section class="stats-card rotation">
    <div class="card-head">
      <div>
        <h2 class="card-title">{{ t('stats.rotation') }}</h2>
        <p class="card-subtitle">{{ t('stats.rank_description') }}</p>
      </div>
    </div>

    <SegmentedControl
      v-if="artists.length && albums.length"
      v-model="selected"
      class="rotation-tabs"
      :options="options"
      :label="t('stats.rotation')"
    />
    <div v-else class="rotation-single">{{ options[kind === 'albums' ? 1 : 0].label }}</div>

    <ol class="rank-list">
      <li v-for="(rank, index) in visible" :key="rank.key" class="rank-row">
        <span class="rank-index" :class="{ top: index < 3 }">{{ index + 1 }}</span>
        <span class="rank-art" :class="{ round: kind === 'artists' }">
          <BilibiliCoverImage v-if="rank.coverUrl" :src="rank.coverUrl" loading="lazy">
            <span class="material-symbols-rounded">{{ kind === 'artists' ? 'person' : 'album' }}</span>
          </BilibiliCoverImage>
          <span v-else class="material-symbols-rounded">{{ kind === 'artists' ? 'person' : 'album' }}</span>
        </span>
        <span class="rank-body">
          <span class="rank-line">
            <span class="rank-name" :title="rank.name">{{ rank.name }}</span>
            <span class="rank-share">{{ format.percent(rank.share) }}</span>
          </span>
          <span class="meter" aria-hidden="true">
            <span class="meter-fill" :style="{ clipPath: meterClip(rank.share) }" />
          </span>
          <span class="rank-meta">
            <span>{{ t('stats.track_total', { count: format.number(rank.trackCount) }, rank.trackCount) }} · {{ format.plays(rank.totalPlayCount) }}</span>
            <span class="rank-time">{{ format.duration(rank.totalListenMs) }}</span>
          </span>
        </span>
      </li>
    </ol>

    <button
      v-if="ranking.length > COLLAPSED_SIZE"
      type="button"
      class="rank-toggle"
      :aria-expanded="expanded"
      @click="expanded = !expanded"
    >
      {{ expanded ? t('stats.rank_less') : t('stats.rank_more', { count: Math.min(ranking.length, EXPANDED_LIMIT) }) }}
      <span class="material-symbols-rounded" aria-hidden="true">{{ expanded ? 'expand_less' : 'expand_more' }}</span>
    </button>
  </section>
</template>

<style scoped lang="scss">
@use './statsCard.scss' as card;

@include card.card;
@include card.meter;

.rotation-tabs {
  display: grid;
  width: 100%;
  margin-bottom: 8px;
}

.rotation-single {
  margin-bottom: 8px;
  font-size: var(--fs-13);
  font-weight: 600;
  color: var(--md-on-surface-variant);
}

.rank-list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.rank-row {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 64px;
  padding: 8px 0;

  & + & { border-top: 1px solid var(--hairline); }
}

.rank-index {
  flex: 0 0 22px;
  font-size: var(--fs-13);
  font-weight: 600;
  text-align: center;
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;

  &.top { color: var(--md-on-surface); }
}

.rank-art {
  @include card.artwork(40px);

  &.round { border-radius: var(--radius-full); }
}

.rank-body {
  display: flex;
  flex: 1;
  min-width: 0;
  flex-direction: column;
  gap: 6px;
}

.rank-line,
.rank-meta {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  min-width: 0;
}

.rank-name {
  overflow: hidden;
  font-size: var(--fs-14);
  font-weight: 600;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rank-share {
  flex-shrink: 0;
  font-size: var(--fs-13);
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}

.rank-meta {
  font-size: var(--fs-11);
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;

  > span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
}

.rank-time { flex-shrink: 0; }

.rank-toggle {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  margin: 6px 0 -6px auto;
  padding: 6px 8px 6px 12px;
  border-radius: var(--radius-full);
  font-size: var(--fs-13);
  font-weight: 600;
  color: var(--md-primary);
  transition: background var(--duration-short);

  .material-symbols-rounded { font-size: 20px; }
  &:hover { background: color-mix(in srgb, var(--md-primary) 10%, transparent); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 2px; }
}

.rotation { display: flex; flex-direction: column; }
</style>
