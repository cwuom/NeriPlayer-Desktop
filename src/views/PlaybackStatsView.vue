<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { normalizeTrack, usePlayerStore, type TrackInfo } from '@/stores/player'
import { fetchPlaybackStatTracks } from '@/modules/playback/playbackStatTracks'
import {
  STATS_PERIODS,
  usePlaybackStatsStore,
  type StatsPeriod,
  type TrackStat,
} from '@/stores/playbackStats'
import {
  STATS_SORT_MODES,
  buildStatsInsights,
  sortStatTracks,
  type StatsSortMode,
} from '@/modules/library/playbackStatsInsights'
import { buildHourlyRhythm } from '@/modules/library/playbackHourlyRhythm'
import TrackContextMenu from '@/components/TrackContextMenu.vue'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import StatsHero from '@/components/stats/StatsHero.vue'
import StatsSpotlight from '@/components/stats/StatsSpotlight.vue'
import StatsRotation from '@/components/stats/StatsRotation.vue'
import StatsBarChart from '@/components/stats/StatsBarChart.vue'
import StatsHourlyRhythm from '@/components/stats/StatsHourlyRhythm.vue'
import StatsRangeCompare from '@/components/stats/StatsRangeCompare.vue'
import StatsTrackList from '@/components/stats/StatsTrackList.vue'
import { useToastStore } from '@/stores/toast'

type StatsView = 'overview' | 'tracks'

const router = useRouter()
const player = usePlayerStore()
const stats = usePlaybackStatsStore()
const toast = useToastStore()
const { t } = useI18n()

const TOP_CHART_SIZE = 5
const CHART_TITLE_KEYS: Record<StatsSortMode, string> = {
  play_count: 'stats.most_played',
  listen_time: 'stats.chart_listen_time',
  recent: 'stats.chart_recent',
  first_played: 'stats.chart_first_played',
}

const showClearConfirm = ref(false)
const view = ref<StatsView>('overview')
const sortMode = ref<StatsSortMode>('play_count')

const summary = computed(() => stats.current)
const hasAnyStats = computed(() => STATS_PERIODS.some(period => stats.summaries[period].trackCount > 0))
const insights = computed(() => buildStatsInsights(summary.value.items, summary.value))
const hourlyRhythm = computed(() => buildHourlyRhythm(summary.value.hourListenMs))
const mostPlayed = computed(() => sortStatTracks(summary.value.items, 'play_count').slice(0, TOP_CHART_SIZE))
const rankedTracks = computed(() => sortStatTracks(summary.value.items, sortMode.value))
const rankedTop = computed(() => rankedTracks.value.slice(0, TOP_CHART_SIZE))

const periodOptions = computed(() => STATS_PERIODS.map(period => ({ value: period, label: t(`stats.period_${period}`) })))
const viewOptions = computed(() => [
  { value: 'overview' as const, label: t('stats.view_overview'), icon: 'insights' },
  { value: 'tracks' as const, label: t('stats.view_tracks'), icon: 'format_list_numbered' },
])
const sortOptions = computed(() => STATS_SORT_MODES.map(mode => ({ value: mode, label: t(`stats.sort_${mode}`) })))
const rangeLabel = computed(() => t(`stats.range_${stats.activePeriod}`))

const rootRef = ref<HTMLElement | null>(null)

// 两个视图长度差很多，切换后停在旧位置会落进列表中段
watch(view, () => rootRef.value?.scrollTo({ top: 0 }))

function selectPeriod(period: StatsPeriod) {
  stats.activePeriod = period
}

function isFilesystemPath(value: string): boolean {
  return /^[a-zA-Z]:[\\/]/.test(value) || value.startsWith('/') || value.startsWith('\\\\')
}

// 本地歌曲要有本机文件路径才能播放；其他设备同步来的 content:// 等地址在桌面端打不开
function isPlayable(track: TrackInfo): boolean {
  return track.source !== 'local' || isFilesystemPath(track.audioUrl)
}

const startingIdentity = ref('')

/// 同步来的统计 id 和本机曲目 id 形态不同，同时比对同步身份键
function isCurrent(item: TrackStat): boolean {
  const current = player.currentTrack
  return !!current && (current.id === item.id || current.playlistKey === item.identityKey)
}

/// 对齐 Android（onSongClick(listOf(stat.toSongItem()), 0)）：只播点中的这一首。
/// 统计记录的身份由后端按同步模型还原（Android 同步来的是裸 id + 平台标记）
/** 按统计记录还原可播放的曲目；还原不了时提示原因并返回 null */
async function resolveStatTrack(item: TrackStat): Promise<TrackInfo | null> {
  if (startingIdentity.value) return null
  startingIdentity.value = item.identityKey
  try {
    const [raw] = await fetchPlaybackStatTracks([item.identityKey])
    const selected = raw ? normalizeTrack(raw) : null
    if (!selected || !isPlayable(selected)) {
      toast.show(t(selected?.source === 'local' ? 'stats.local_file_unavailable' : 'stats.track_unavailable'), 'info')
      return null
    }
    return selected
  } catch (error) {
    toast.error(String(error))
    return null
  } finally {
    startingIdentity.value = ''
  }
}

function playResolved(track: TrackInfo) {
  player.playAll([track], track.id, track.playlistKey)
}

async function playStat(item: TrackStat) {
  const track = await resolveStatTrack(item)
  if (track) playResolved(track)
}

const trackMenuRef = ref<InstanceType<typeof TrackContextMenu> | null>(null)

async function openStatMenu(event: MouseEvent, item: TrackStat) {
  event.preventDefault()
  const { clientX, clientY } = event
  const track = await resolveStatTrack(item)
  if (track) trackMenuRef.value?.openAt(clientX, clientY, track)
}

async function confirmClear() {
  showClearConfirm.value = false
  try {
    await stats.clearAll()
  } catch (error) {
    toast.error(String(error))
  }
}

onMounted(() => {
  void stats.flushAndRefresh()
})
</script>

<template>
  <div ref="rootRef" class="detail-view stats-view">
    <header class="detail-header" data-tauri-drag-region>
      <button type="button" class="back-btn" :aria-label="t('common.back')" @click="router.back()">
        <span class="material-symbols-rounded">arrow_back</span>
      </button>
      <h1 class="header-title">{{ t('stats.title') }}</h1>
      <div class="header-spacer" data-tauri-drag-region />
      <button
        type="button"
        class="header-icon-btn"
        :title="t('stats.clear')"
        :aria-label="t('stats.clear')"
        :disabled="!hasAnyStats"
        @click="showClearConfirm = true"
      >
        <span class="material-symbols-rounded">delete_sweep</span>
      </button>
    </header>

    <div v-if="hasAnyStats" class="stats-toolbar">
      <SegmentedControl
        :model-value="stats.activePeriod"
        :options="periodOptions"
        :label="t('stats.period_label')"
        @update:model-value="selectPeriod"
      />
      <SegmentedControl
        v-model="view"
        :options="viewOptions"
        :label="t('stats.view_label')"
      />
    </div>

    <div v-if="stats.loading && !hasAnyStats" class="state-center">
      <span class="material-symbols-rounded spinning">progress_activity</span>
    </div>

    <div v-else-if="!hasAnyStats" class="stats-empty">
      <span class="empty-icon material-symbols-rounded" aria-hidden="true">bar_chart</span>
      <p class="empty-title">{{ t('stats.empty') }}</p>
      <p class="empty-hint">{{ t('stats.empty_hint') }}</p>
    </div>

    <div v-else-if="!summary.trackCount" class="stats-period-empty">
      <div class="stats-empty compact">
        <span class="empty-icon material-symbols-rounded" aria-hidden="true">event_busy</span>
        <p class="empty-title">{{ t('stats.period_empty') }}</p>
      </div>
      <StatsRangeCompare :summaries="stats.summaries" :active="stats.activePeriod" @select="selectPeriod" />
    </div>

    <Transition v-else name="stats-swap" mode="out-in">
      <div v-if="view === 'overview'" key="overview" class="stats-overview">
        <StatsHero
          :range-label="rangeLabel"
          :total-listen-ms="summary.totalListenMs"
          :total-play-count="summary.totalPlayCount"
          :track-count="summary.trackCount"
          :artist-count="insights.artists.length"
          :average-listen-ms-per-play="insights.averageListenMsPerPlay"
          :earliest-listen-at="stats.activePeriod === 'day' ? null : insights.earliestListenAt"
        />
        <div class="overview-grid">
          <div class="overview-col">
            <StatsSpotlight
              v-if="insights.topTrack"
              :track="insights.topTrack"
              :active="isCurrent(insights.topTrack)"
              :starting="startingIdentity === insights.topTrack.identityKey"
              @play="playStat"
              @menu="openStatMenu"
            />
            <StatsRotation
              v-if="insights.artists.length || insights.albums.length"
              :artists="insights.artists"
              :albums="insights.albums"
            />
            <StatsHourlyRhythm v-if="hourlyRhythm" :rhythm="hourlyRhythm" />
          </div>
          <div class="overview-col">
            <StatsBarChart
              :title="t('stats.most_played')"
              :items="mostPlayed"
              mode="play_count"
              :is-current="isCurrent"
              @play="playStat"
              @menu="openStatMenu"
            />
            <StatsRangeCompare :summaries="stats.summaries" :active="stats.activePeriod" @select="selectPeriod" />
          </div>
        </div>
      </div>

      <div v-else key="tracks" class="stats-tracks">
        <aside class="tracks-aside">
          <StatsBarChart
            :title="t(CHART_TITLE_KEYS[sortMode])"
            :items="rankedTop"
            :mode="sortMode"
            :is-current="isCurrent"
            @play="playStat"
            @menu="openStatMenu"
          />
          <StatsRangeCompare
            class="aside-range"
            :summaries="stats.summaries"
            :active="stats.activePeriod"
            @select="selectPeriod"
          />
        </aside>
        <section class="tracks-panel">
          <div class="panel-head">
            <div>
              <h2 class="panel-title">{{ t('stats.view_tracks') }}</h2>
              <p class="panel-subtitle">{{ rangeLabel }} · {{ t('stats.track_total', { count: summary.trackCount }, summary.trackCount) }}</p>
            </div>
            <SegmentedControl v-model="sortMode" :options="sortOptions" :label="t('stats.sort_label')" />
          </div>
          <StatsTrackList
            :items="rankedTracks"
            :mode="sortMode"
            :starting-key="startingIdentity"
            :is-current="isCurrent"
            @play="playStat"
            @menu="openStatMenu"
          />
        </section>
      </div>
    </Transition>

    <M3Dialog
      v-model:open="showClearConfirm"
      :title="t('stats.clear')"
      icon="delete_sweep"
      :confirm-text="t('stats.clear_confirm_btn')"
      confirm-danger
      @confirm="confirmClear"
    >
      <p>{{ t('stats.clear_confirm_msg') }}</p>
    </M3Dialog>
    <TrackContextMenu ref="trackMenuRef" @play="playResolved" />
  </div>
</template>

<style scoped lang="scss">
@use '@/styles/detail-view.scss' as *;

/* 统计页（Clean）：顶栏只留返回、标题与清空，区间和视图切换放在下方工具栏，正文按容器宽度在单列 / 双列之间切换 */
.header-title {
  margin: 0;
  overflow: hidden;
  font-size: calc(18px * var(--text-scale));
  font-weight: 700;
  letter-spacing: var(--tracking-title);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.header-spacer {
  flex: 1;
  align-self: stretch;
}

.header-icon-btn {
  display: flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: var(--radius-full);
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short), color var(--duration-short);

  .material-symbols-rounded { font-size: 21px; }

  &:hover:not(:disabled) {
    background: color-mix(in srgb, var(--md-error) 10%, transparent);
    color: var(--md-error);
  }

  &:disabled { opacity: 0.4; cursor: default; }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 2px; }
}

.stats-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 10px 16px;
  padding: 4px 0 14px;
}

.stats-overview,
.stats-tracks {
  padding-top: 8px;
}

.stats-overview {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.overview-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 16px;
}

.overview-col {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 16px;
}

@container (min-width: 900px) {
  .overview-grid {
    grid-template-columns: minmax(0, 1.1fr) minmax(0, 1fr);
    align-items: start;
  }
}

.stats-tracks {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 16px;
}

.tracks-aside {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 16px;
}

/* 单列时列表优先，时间范围对比只在双列侧栏里补位 */
.aside-range { display: none; }

.tracks-panel {
  min-width: 0;
  padding: 18px 14px 12px;
  border-radius: var(--radius-xl);
  background: var(--md-surface-container-low);
  box-shadow: 0 0 0 1px var(--hairline);
}

.panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 12px;
  padding: 0 8px 12px;
  margin-bottom: 6px;
  border-bottom: 1px solid var(--hairline);
}

.panel-title {
  margin: 0;
  font-size: calc(17px * var(--text-scale));
  font-weight: 700;
  letter-spacing: var(--tracking-title);
}

.panel-subtitle {
  margin: 2px 0 0;
  font-size: var(--fs-12);
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;
}

@container (min-width: 1100px) {
  .aside-range { display: block; }

  .stats-tracks {
    grid-template-columns: minmax(300px, 380px) minmax(0, 1fr);
    align-items: start;
  }

  /* 列表很长时左侧图表随滚动停在顶栏下方 */
  .tracks-aside {
    position: sticky;
    top: calc(var(--titlebar-height, 48px) + 12px);
  }
}

.stats-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  min-height: 360px;
  padding: 48px 24px;
  text-align: center;

  &.compact { min-height: 0; padding: 40px 24px 28px; }
}

.empty-icon {
  display: grid;
  place-items: center;
  width: 72px;
  height: 72px;
  margin-bottom: 10px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-high);
  color: var(--md-on-surface-variant);
  font-size: 34px;
}

.empty-title {
  margin: 0;
  font-size: var(--fs-15);
  font-weight: 600;
}

.empty-hint {
  margin: 0;
  font-size: var(--fs-13);
  color: var(--md-on-surface-variant);
}

.stats-period-empty {
  display: flex;
  flex-direction: column;
  gap: 8px;
  max-width: 720px !important;
}

.stats-swap-enter-active,
.stats-swap-leave-active {
  transition: opacity 140ms var(--ease-standard), transform 140ms var(--ease-standard);
}

.stats-swap-enter-from {
  opacity: 0;
  transform: translateY(6px);
}

.stats-swap-leave-to { opacity: 0; }

@media (prefers-reduced-motion: reduce) {
  .stats-swap-enter-active,
  .stats-swap-leave-active { transition: none; }

  .stats-swap-enter-from { transform: none; }
}
</style>
