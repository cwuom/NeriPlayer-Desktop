<script setup lang="ts">
// 更多面板的歌曲详情子视图：曲目标识、音频参数与下载 / 缓存状态
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { usePlayerStore } from '@/stores/player'
import { actualAudioBitrateLabel, isLocalAudioPlayback } from '@/modules/playback/audioQualityDisplay'
import { normalizeAudioDisplayToken, paperSpecFromAudioInfo } from './audioInfoDisplay'
import { formatDurationMs, formatFileSize, isRemotePlaybackSource, platformLabel } from './nowPlayingHelpers'
import { useNowPlayingSheetContext } from './sheetContext'
import { useCurrentTrackDownload } from './useCurrentTrackDownload'

const player = usePlayerStore()
const { t } = useI18n()
const {
  showMoreSheet, goBackToMain, coverUrl, coverLoadError, handleNowPlayingCoverError, albumName,
  playbackSourceLabel, quality, copyText, shareSong,
} = useNowPlayingSheetContext()
const { currentSource, currentAudioQualityLabel } = quality
const { currentDownloadTask, currentDownloadedTrack, isCurrentDownloaded, downloadTaskStatusText } =
  useCurrentTrackDownload(showMoreSheet)

// 歌曲详情专用: 码率/编解码/采样 等完整音频参数 (含 kbps, 与进度条纸面规格分离)
const trackDetailAudioParams = computed(() => {
  const info = player.audioInfo
  if (!info) return ''
  const local = isLocalAudioPlayback({ source: currentSource.value, fromDownload: player.isPlayingFromDownload, info })
  const parts: string[] = []
  const quality = currentAudioQualityLabel()
  if (quality) parts.push(quality)
  if (info.codec) {
    const codec = normalizeAudioDisplayToken(info.codec, local)
    if (codec && !parts.includes(codec)) parts.push(codec)
  }
  if (info.format) {
    const format = normalizeAudioDisplayToken(info.format, local)
    if (format && !parts.includes(format) && format.toLowerCase() !== (info.codec || '').toLowerCase()) {
      parts.push(format)
    }
  }
  const bitrate = actualAudioBitrateLabel(info)
  if (bitrate) parts.push(bitrate)
  for (const token of paperSpecFromAudioInfo(info, !local)) {
    if (!parts.includes(token)) parts.push(token)
  }
  return parts.join(' · ')
})

// 播放缓存 / 下载 / 在线流 三态
const trackDetailCacheStatus = computed(() => {
  if (player.isPlayingFromDownload) return t('player.cache_status_download')
  if (player.isPlayingFromCache) return t('player.cache_status_playback_cache')
  if (isRemotePlaybackSource(currentSource.value)) return t('player.cache_status_stream')
  return t('player.cache_status_local')
})
</script>

<template>
  <div class="np-more-sub-header">
    <button class="np-more-back" @click="goBackToMain()">
      <span class="material-symbols-rounded">arrow_back</span>
    </button>
    <h4 class="np-more-title">{{ t('player.track_detail') }}</h4>
  </div>
  <div class="np-track-detail-card">
    <div class="np-track-detail-hero">
      <img
        v-if="coverUrl && !coverLoadError"
        :src="coverUrl"
        class="np-track-detail-cover"
        referrerpolicy="no-referrer"
        @error="handleNowPlayingCoverError"
      />
      <span v-else class="np-track-detail-cover np-track-detail-cover-fallback material-symbols-rounded filled">music_note</span>
      <div class="np-track-detail-heading">
        <strong>{{ player.currentTrack?.title || '-' }}</strong>
        <span>{{ player.currentTrack?.artist || '-' }}</span>
      </div>
    </div>

    <button class="np-track-detail-row copyable" @click="copyText(player.currentTrack?.id || '')">
      <span>{{ t('player.track_detail_id') }}</span>
      <strong>{{ player.currentTrack?.id || '-' }}</strong>
    </button>
    <button class="np-track-detail-row copyable" @click="copyText(player.currentTrack?.title || '')">
      <span>{{ t('player.track_detail_title') }}</span>
      <strong>{{ player.currentTrack?.title || '-' }}</strong>
    </button>
    <button class="np-track-detail-row copyable" @click="copyText(player.currentTrack?.artist || '')">
      <span>{{ t('player.track_detail_artist') }}</span>
      <strong>{{ player.currentTrack?.artist || '-' }}</strong>
    </button>
    <div class="np-track-detail-row">
      <span>{{ t('player.track_detail_album') }}</span>
      <strong>{{ albumName || '-' }}</strong>
    </div>
    <div class="np-track-detail-row">
      <span>{{ t('player.track_detail_source') }}</span>
      <strong>{{ playbackSourceLabel || platformLabel(currentSource, t) || '-' }}</strong>
    </div>
    <div class="np-track-detail-row">
      <span>{{ t('player.track_detail_duration') }}</span>
      <strong>{{ formatDurationMs(player.currentTrack?.durationMs || player.durationMs) }}</strong>
    </div>
    <div class="np-track-detail-row">
      <span>{{ t('player.track_detail_audio_params') }}</span>
      <strong>{{ trackDetailAudioParams || '-' }}</strong>
    </div>
    <div class="np-track-detail-row">
      <span>{{ t('player.track_detail_bitrate') }}</span>
      <strong>
        {{ actualAudioBitrateLabel(player.audioInfo) || '-' }}
      </strong>
    </div>
    <div class="np-track-detail-row">
      <span>{{ t('player.track_detail_cache_status') }}</span>
      <strong>{{ trackDetailCacheStatus }}</strong>
    </div>
    <div class="np-track-detail-row">
      <span>{{ t('player.track_detail_local_download_play') }}</span>
      <strong>{{ player.isPlayingFromDownload ? t('player.yes') : t('player.no') }}</strong>
    </div>
    <div class="np-track-detail-row">
      <span>{{ t('player.track_detail_download_status') }}</span>
      <strong>
        {{ currentDownloadTask
          ? downloadTaskStatusText(currentDownloadTask.status)
          : (isCurrentDownloaded ? t('download.downloaded') : t('download.not_downloaded')) }}
      </strong>
    </div>
    <div v-if="currentDownloadedTrack" class="np-track-detail-row">
      <span>{{ t('player.track_detail_file_size') }}</span>
      <strong>{{ formatFileSize(currentDownloadedTrack.fileSize) }}</strong>
    </div>
    <button class="np-more-form-btn primary np-track-detail-share" @click="shareSong">
      <span class="material-symbols-rounded">share</span>
      {{ t('player.copy_share_info') }}
    </button>
  </div>
</template>
