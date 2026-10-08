<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import {
  STATS_PERIODS,
  usePlaybackStatsStore,
  type StatsPeriod,
  type TrackStat,
} from '@/stores/playbackStats'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import { useEscapeClose } from '@/composables/useEscapeClose'
import { useToastStore } from '@/stores/toast'

const router = useRouter()
const player = usePlayerStore()
const stats = usePlaybackStatsStore()
const toast = useToastStore()
const { t } = useI18n()

const showClearConfirm = ref(false)
useEscapeClose(() => showClearConfirm.value, () => { showClearConfirm.value = false })
const TOP_CHART_SIZE = 5

const summary = computed(() => stats.current)
const topTracks = computed(() => summary.value.items.slice(0, TOP_CHART_SIZE))
// 条形图按最大值归一，避免第一名占满时其余项挤成一条线
const chartMax = computed(() => Math.max(1, ...topTracks.value.map((item) => item.playCount)))

function periodLabel(period: StatsPeriod): string {
  return t(`stats.period_${period}`)
}

function formatListenTime(ms: number): string {
  const totalMinutes = Math.floor(Math.max(0, ms) / 60_000)
  const hours = Math.floor(totalMinutes / 60)
  const minutes = totalMinutes % 60
  if (hours > 0) return `${hours}h ${minutes}m`
  return `${minutes}m`
}

// 用 scaleX 而不是 width：只走合成层，切区间时不会每帧触发布局
function barScale(item: TrackStat): number {
  return Math.max(0.04, item.playCount / chartMax.value)
}

function isFilesystemPath(value: string): boolean {
  return /^[a-zA-Z]:[\\/]/.test(value) || value.startsWith('/') || value.startsWith('\\\\')
}

// 本地歌曲要有文件路径才能播放：优先统计里记下的位置，旧记录从 local:<路径> 身份里取；
// 其他设备同步来的 content:// 等地址在桌面端无法打开
function localPlaybackPath(item: TrackStat): string {
  const fromId = item.id.startsWith('local:') ? item.id.slice('local:'.length) : ''
  return [item.mediaUri ?? '', fromId].find(isFilesystemPath) ?? ''
}

function statToTrack(item: TrackStat): TrackInfo | null {
  if (!item.id) return null
  const prefix = item.id.split(':')[0]
  const source = (['netease', 'qq', 'bilibili', 'youtube'].includes(prefix) ? prefix : 'local') as TrackInfo['source']
  return {
    id: item.id,
    title: item.name,
    artist: item.artist,
    album: item.album,
    durationMs: item.durationMs,
    coverUrl: item.coverUrl ?? undefined,
    audioUrl: source === 'local' ? localPlaybackPath(item) : '',
    source,
    addedAt: 0,
  } as TrackInfo
}

function playStat(item: TrackStat) {
  const track = statToTrack(item)
  if (!track) return
  if (track.source === 'local' && !track.audioUrl) {
    toast.show(t('stats.local_file_unavailable'), 'info')
    return
  }
  void player.play(track, 'local')
}

async function confirmClear() {
  showClearConfirm.value = false
  await stats.clearAll()
}

onMounted(() => {
  void stats.flushAndRefresh()
})

onUnmounted(() => {
  showClearConfirm.value = false
})
</script>

<template>
  <div class="detail-view">
    <div class="detail-header">
      <button class="back-btn" @click="router.back()">
        <span class="material-symbols-rounded">arrow_back</span>
      </button>
      <div class="header-title">{{ t('stats.title') }}</div>
      <div class="header-spacer" />
      <button
        class="action-btn danger"
        :title="t('stats.clear')"
        @click="showClearConfirm = true"
      >
        <span class="material-symbols-rounded">delete_sweep</span>
      </button>
    </div>

    <div class="period-bar">
      <button
        v-for="period in STATS_PERIODS"
        :key="period"
        class="period-chip"
        :class="{ active: stats.activePeriod === period }"
        @click="stats.activePeriod = period"
      >
        {{ periodLabel(period) }}
      </button>
    </div>

    <div class="summary-card">
      <div class="summary-metric">
        <span class="material-symbols-rounded">headphones</span>
        <div class="metric-value">{{ summary.totalPlayCount }}</div>
        <div class="metric-label">{{ t('stats.play_count') }}</div>
      </div>
      <div class="summary-metric">
        <span class="material-symbols-rounded">schedule</span>
        <div class="metric-value">{{ formatListenTime(summary.totalListenMs) }}</div>
        <div class="metric-label">{{ t('stats.listen_time') }}</div>
      </div>
      <div class="summary-metric">
        <span class="material-symbols-rounded">library_music</span>
        <div class="metric-value">{{ summary.trackCount }}</div>
        <div class="metric-label">{{ t('stats.track_count') }}</div>
      </div>
    </div>

    <div v-if="stats.loading && !summary.items.length" class="empty-state">
      <span class="material-symbols-rounded spinning">progress_activity</span>
    </div>

    <div v-else-if="!summary.items.length" class="empty-state">
      <span class="material-symbols-rounded">bar_chart</span>
      <p>{{ t('stats.empty') }}</p>
    </div>

    <template v-else>
      <div class="section-title">{{ t('stats.most_played') }}</div>
      <div class="chart">
        <div v-for="item in topTracks" :key="item.identityKey" class="chart-row">
          <div class="chart-name">{{ item.name || t('stats.unknown_track') }}</div>
          <div class="chart-track">
            <div class="chart-bar" :style="{ transform: `scaleX(${barScale(item)})` }" />
          </div>
          <div class="chart-value">{{ item.playCount }}</div>
        </div>
      </div>

      <div class="track-list">
        <div
          v-for="(item, index) in summary.items"
          :key="item.identityKey"
          class="track-item"
          :class="{ active: player.currentTrack?.id === item.id }"
          @click="playStat(item)"
        >
          <div class="track-index">
            <span class="index-num" :class="{ top: index < 3 }">{{ index + 1 }}</span>
          </div>
          <div class="track-cover">
            <BilibiliCoverImage v-if="item.coverUrl" :src="item.coverUrl" loading="lazy">
              <span class="material-symbols-rounded filled">music_note</span>
            </BilibiliCoverImage>
            <span v-else class="material-symbols-rounded filled">music_note</span>
          </div>
          <div class="track-info">
            <div class="track-title">{{ item.name || t('stats.unknown_track') }}</div>
            <div class="track-meta">{{ item.artist }}</div>
          </div>
          <div class="track-stat">
            <div class="stat-primary">{{ t('stats.times', { count: item.playCount }) }}</div>
            <div class="stat-secondary">{{ formatListenTime(item.totalListenMs) }}</div>
          </div>
        </div>
      </div>
    </template>

    <Teleport to="body">
      <div v-if="showClearConfirm" class="dialog-overlay" @click="showClearConfirm = false">
        <div class="dialog-card" @click.stop>
          <h3>{{ t('stats.clear') }}</h3>
          <p>{{ t('stats.clear_confirm_msg') }}</p>
          <div class="dialog-actions">
            <button class="dialog-btn" @click="showClearConfirm = false">
              {{ t('common.cancel') }}
            </button>
            <button class="dialog-btn danger" @click="confirmClear">
              {{ t('stats.clear_confirm_btn') }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped lang="scss">
@use '@/styles/detail-view.scss' as *;

.header-title {
  font-size: 18px;
  font-weight: 600;
  flex-shrink: 0;
}

.header-spacer { flex: 1; }

.action-btn {
  width: 40px;
  height: 40px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container-high); }
  &.danger:hover { color: var(--md-error, #FFB4AB); }
  .material-symbols-rounded { font-size: 22px; }
}

.period-bar {
  display: flex;
  gap: 8px;
  margin-bottom: 16px;
  flex-wrap: wrap;
}

.period-chip {
  padding: 7px 18px;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 500;
  color: var(--md-on-surface-variant);
  border: 1px solid var(--md-outline-variant, rgba(255, 255, 255, 0.12));
  transition:
    background var(--duration-short) var(--easing-standard, ease),
    color var(--duration-short) var(--easing-standard, ease),
    border-color var(--duration-short) var(--easing-standard, ease);

  &:hover { background: var(--md-surface-container-high); }

  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
    border-color: transparent;
  }
}

.summary-card {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 8px;
  padding: 20px 12px;
  margin-bottom: 24px;
  border-radius: var(--radius-xl, 28px);
  background: var(--md-surface-container);
}

.summary-metric {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  min-width: 0;

  .material-symbols-rounded {
    font-size: 22px;
    color: var(--md-primary);
    margin-bottom: 4px;
  }
}

.metric-value {
  font-size: 24px;
  font-weight: 600;
  line-height: 1.1;
  font-variant-numeric: tabular-nums;
}

.metric-label {
  font-size: 12px;
  color: var(--md-on-surface-variant);
}

.section-title {
  font-size: 15px;
  font-weight: 600;
  margin-bottom: 12px;
}

.chart {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-bottom: 28px;
}

.chart-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.chart-name {
  flex: 0 0 120px;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--md-on-surface-variant);
}

.chart-track {
  flex: 1;
  height: 22px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-high);
  overflow: hidden;
}

.chart-bar {
  height: 100%;
  width: 100%;
  transform-origin: left center;
  border-radius: var(--radius-full);
  background: var(--md-primary);
  // 切换区间时条形平滑重排，而不是瞬间跳变
  transition: transform 420ms var(--easing-emphasized, cubic-bezier(0.2, 0, 0, 1));
  will-change: transform;
}

.chart-value {
  flex: 0 0 44px;
  text-align: right;
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  color: var(--md-on-surface-variant);
}

.index-num.top {
  color: var(--md-primary);
  font-weight: 600;
}

.track-stat {
  flex-shrink: 0;
  text-align: right;
  min-width: 78px;
}

.stat-primary {
  font-size: 13px;
  font-variant-numeric: tabular-nums;
  color: var(--md-primary);
}

.stat-secondary {
  font-size: 11px;
  opacity: 0.6;
  color: var(--md-on-surface-variant);
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 64px 0;
  color: var(--md-on-surface-variant);

  .material-symbols-rounded { font-size: 44px; opacity: 0.4; }
  p { font-size: 14px; opacity: 0.7; }
}

.spinning { animation: stats-spin 1s linear infinite; }

@keyframes stats-spin {
  to { transform: rotate(360deg); }
}

@media (prefers-reduced-motion: reduce) {
  .chart-bar { transition: none; }
  .spinning { animation: none; }
}

.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 500;
}

.dialog-card {
  background: var(--md-surface-container-high);
  border-radius: var(--radius-xl, 28px);
  padding: 24px;
  min-width: 300px;
  max-width: 400px;

  h3 { font-size: 18px; font-weight: 600; margin-bottom: 12px; }
  p { font-size: 14px; color: var(--md-on-surface-variant); line-height: 1.5; }
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 20px;
}

.dialog-btn {
  padding: 8px 20px;
  border-radius: var(--radius-full);
  font-size: 14px;
  font-weight: 500;
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container-highest); }
  &.danger { color: var(--md-error, #FFB4AB); }
  &.danger:hover { background: color-mix(in srgb, var(--md-error, #FFB4AB) 12%, transparent); }
}
</style>
