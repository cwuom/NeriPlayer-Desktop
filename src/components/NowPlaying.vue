<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount, onMounted, onUnmounted } from 'vue'
import { usePlayerStore } from '@/stores/player'
import type { LyricLine, TrackInfo } from '@/types/track'
import { displayAlbum } from '@/modules/library/albumDisplay'
import { useLikedSongsStore } from '@/stores/likedSongs'
import { useBiliVideoPartsStore } from '@/stores/biliVideoParts'
import { COVER_BLUR_PX_PER_UNIT, useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { shouldShowDynamicBackground } from '@/utils/nowPlayingBackground'
import { clearCachedLyrics, saveCachedLyrics } from '@/modules/lyrics/lyricsCache'
import {
  toEditableLyricsText,
  toEditableTranslationText,
  toEditableRomanizationText,
  resolveStoredLyricStateFromPayload,
  resolveStoredTranslatedLyricStateFromPayload,
  resolveStoredRomanizedLyricStateFromPayload,
  withUpdatedLyricsPayload,
} from '@/modules/lyrics/lyricsFormat'
import { withUpdatedUserOffsetPayload } from '@/modules/lyrics/lyricOffset'
import { useCurrentLyricsStore } from '@/stores/currentLyrics'
import { isEditableTarget, isMacPlatform } from '@/modules/shortcuts/platform'
import { persistTrackSyncPayload, recordLyricOverride } from '@/modules/lyrics/syncTrackPayload'
import { useLyricOffsetStore } from '@/stores/lyricOffset'
import HyperBackground from './background/HyperBackground.vue'
import CoverBlurBackground from './background/CoverBlurBackground.vue'
import CommentsPanel from './CommentsPanel.vue'
import { resolveCommentTarget } from '@/modules/playback/commentTarget'
import WaveformSlider from './WaveformSlider.vue'
import LyricsView from './LyricsView.vue'
import QueuePanel from './QueuePanel.vue'
import AddToPlaylistDialog from './AddToPlaylistDialog.vue'
import BiliVideoSkipDialog from './bilibili/BiliVideoSkipDialog.vue'
import { resolveBiliVideoSkipBvid, resolveBiliVideoSkipCid } from '@/modules/playback/biliVideoSkip'
import ListenTogetherPanel from './ListenTogetherPanel.vue'
import EditableRangeValue from './ui/EditableRangeValue.vue'
import CoverPreview from './now-playing/CoverPreview.vue'
import { getTrackCoverUrl } from '@/utils/trackCover'
import AudioEffectsPanel from './AudioEffectsPanel.vue'
import ContextMenu from './ui/ContextMenu.vue'
import NowPlayingMoreSheet from './now-playing/NowPlayingMoreSheet.vue'
import { wheelAdjustedVolume } from '@/utils/volume'
import { writeClipboardText } from '@/utils/clipboard'
import { playbackSessionTrackKey } from '@/modules/playback/playbackRequest'
import { accentBackdropColor, buildDynamicColorVars, type DynamicColorVars } from './now-playing/dynamicColors'
import { formatSleepRemaining, playbackSourceIconFor, playbackSourceLabelFor } from './now-playing/nowPlayingHelpers'
import { provideNowPlayingSheetContext } from './now-playing/sheetContext'
import { useAudioInfoRow } from './now-playing/useAudioInfoRow'
import { useAudioQuality } from './now-playing/useAudioQuality'
import { useNowPlayingContextMenu } from './now-playing/useNowPlayingContextMenu'
import { useNowPlayingCover } from './now-playing/useNowPlayingCover'
import type {
  EditableLyrics,
  InfoSearchResult,
  MoreSheetView,
  NeteaseSongDetail,
  NowPlayingIpc,
  YoutubeCreator,
} from './now-playing/types'

const emit = defineEmits<{ collapse: [] }>()
const props = defineProps<{
  hideHeader?: boolean
  transitionState?: 'opening' | 'closing' | null
}>()
const player = usePlayerStore()
const likedSongs = useLikedSongsStore()
const biliParts = useBiliVideoPartsStore()
const settings = useSettingsStore()
const toast = useToastStore()
const lyricOffsetStore = useLyricOffsetStore()
const router = useRouter()
const { t } = useI18n()

// 组件层的 invoke 计入质量基线：播放页用到的后端命令集中在这里，子模块经参数或面板上下文调用
const nowPlayingIpc: NowPlayingIpc = {
  parseLrc: content => invoke<unknown[]>('parse_lrc_content', { content }),
  search: (query, platform, includeLyrics) => invoke<InfoSearchResult[]>('search', { query, platform, includeLyrics }),
  getNeteaseSongDetail: songId => invoke<NeteaseSongDetail | null>('get_netease_song_detail', { songId }),
  resolveYoutubeCreators: artist => invoke<YoutubeCreator[]>('resolve_youtube_creators', { artist }),
}

const playViewMode = ref<'cover' | 'lyrics'>('cover')
const showVolumeSlider = ref(false)
const showQueue = ref(false)
const showAddToPlaylist = ref(false)
const addToPlaylistTrack = ref<TrackInfo | null>(null)
const addToPlaylistAbort = new AbortController()
onBeforeUnmount(() => addToPlaylistAbort.abort())
const showBiliVideoSkip = ref(false)
const biliVideoSkipTrack = ref<TrackInfo | null>(null)
const showAudioFxPanel = ref(false)
const showComments = ref(false)
const commentTarget = computed(() => resolveCommentTarget(player.currentTrack))
const showSleepMenu = ref(false)
const showMoreSheet = ref(false)
const showLtPanel = ref(false)
const isTrackSwitchAnimating = ref(false)
const trackSwitchDirection = ref<'prev' | 'next' | 'neutral'>('neutral')

// Low performance mode (opt-in, off by default): with a full-screen overlay open the
// background shader, lyrics and waveform all sit underneath it, invisible yet still burning
// CPU/GPU. While the mode is off every per-frame animation keeps running unchanged.
const fullScreenOverlayOpen = computed(() =>
  showComments.value
  || showQueue.value
  || showMoreSheet.value
  || showLtPanel.value
  || showAddToPlaylist.value
  || !!biliParts.collection
  || showBiliVideoSkip.value)
const frameLoopSuspended = computed(() => settings.lowPerformanceMode && fullScreenOverlayOpen.value)
const controlFeedbackPulse = ref(0)
const lastControlDirection = ref<'prev' | 'next' | 'misc' | null>(null)
let trackSwitchAnimTimer: ReturnType<typeof setTimeout> | null = null
let controlFeedbackPulseTimer: ReturnType<typeof setTimeout> | null = null
let moreSheetSwitchTimer: ReturnType<typeof setTimeout> | null = null

function hideMoreSheet() {
  showMoreSheet.value = false
}

// 来源徽章（对齐 Android PlaybackSourceBadge）
const playbackSourceLabel = computed(() => playbackSourceLabelFor(player.currentTrack, t))
const playbackSourceIcon = computed(() => playbackSourceIconFor(player.currentTrack?.id || ''))
const showSourceBadge = computed(() => settings.showCoverBadge && playbackSourceLabel.value !== '')
const sourceBadgeKey = computed(() => `${nowPlayingTrackKey.value}:${playbackSourceIcon.value}:${playbackSourceLabel.value || 'none'}`)

// 当前曲目歌词的读写入口：编辑器、歌词匹配与获取信息都经由这里，syncPayload 是唯一可信来源
function cacheLyricsForTrack(track: TrackInfo | null | undefined, lines: LyricLine[]) {
  if (!track || lines.length === 0) return
  void saveCachedLyrics(track, lines)
}

function removeCachedLyricsForCurrentTrack() {
  if (!player.currentTrack) return
  void clearCachedLyrics(player.currentTrack)
}

/**
 * 编辑后的歌词写回 syncPayload + 本地歌单, 供同步上传 (对齐 Android)
 * nextRomanized 不传时音译保持不变；传 null 清空
 */
async function commitLyricsToTrack(
  nextLyric: string | null,
  nextTranslated: string | null,
  source?: string | null,
  nextRomanized?: string | null,
  resetOffset = false,
) {
  const track = player.currentTrack
  if (!track) return
  const nextPayload = withUpdatedLyricsPayload(
    resetOffset ? withUpdatedUserOffsetPayload(track.syncPayload, 0) : track.syncPayload,
    nextLyric,
    nextTranslated,
    source ?? 'LOCAL_EDIT',
    undefined,
    nextRomanized,
  )
  // CURRENT version: 有意清空也会上传 None, 与 Android v1 一致
  nextPayload.syncMetadataVersion = 1
  delete nextPayload.sync_metadata_version
  player.patchCurrentTrackSyncPayload(nextPayload)
  if (resetOffset) lyricOffsetStore.clearLocalUserOffset(track)
  const updatedTrack = player.currentTrack
  if (updatedTrack) {
    await persistTrackSyncPayload(updatedTrack)
    await recordLyricOverride(updatedTrack)
  }
}

function storedOrDisplayed(
  state: ReturnType<typeof resolveStoredLyricStateFromPayload>,
  lines: LyricLine[],
  exportText: (lines: LyricLine[]) => string,
) {
  // 优先使用 syncPayload 原文(保持用户编辑/YRC 源文本), 否则从当前展示行导出
  if (state.kind === 'present') return state.text
  return lines.length > 0 ? exportText(lines) : ''
}

// 歌词编辑器（对齐 Android LyricsEditorSheet）三轨的初始文本
function editableLyricsOfCurrentTrack(): EditableLyrics {
  const payload = player.currentTrack?.syncPayload
  const lines = displayLyrics.value
  return {
    original: storedOrDisplayed(resolveStoredLyricStateFromPayload(payload), lines, toEditableLyricsText),
    translation: storedOrDisplayed(
      resolveStoredTranslatedLyricStateFromPayload(payload), lines, toEditableTranslationText),
    romanization: storedOrDisplayed(
      resolveStoredRomanizedLyricStateFromPayload(payload), lines, toEditableRomanizationText),
  }
}

// 与桌面歌词共用同一份当前歌词
const currentLyrics = useCurrentLyricsStore()
const releaseCurrentLyrics = currentLyrics.acquire()
onUnmounted(releaseCurrentLyrics)
const isFetchingLyrics = computed(() => currentLyrics.loading)

// 歌词拖动预览状态
const previewPositionMs = ref<number | null>(null)
let previewConvergeTimer: ReturnType<typeof setTimeout> | null = null

function onSliderPreview(progress: number) {
  previewPositionMs.value = progress * player.durationMs
}

function onSliderPreviewEnd() {
  // 松手后保持预览 280ms，等待播放位置追上
  if (previewConvergeTimer) clearTimeout(previewConvergeTimer)
  previewConvergeTimer = setTimeout(() => {
    previewPositionMs.value = null
  }, 280)
}

const {
  coverUrl,
  coverLoadError,
  extractedColors,
  paletteResult,
  handleNowPlayingCoverLoad,
  handleNowPlayingCoverError,
} = useNowPlayingCover()

function onSeek(progress: number) {
  player.seekTo(Math.round(progress * player.durationMs))
}

function onLyricSeek(ms: number) {
  player.seekTo(ms)
}

const isFavorite = computed(() => likedSongs.isTrackLiked(player.currentTrack, true))

function toggleFavorite() {
  likedSongs.requestToggleTrack(player.currentTrack)
}

function openAddToPlaylist() {
  if (!player.currentTrack) return
  void biliParts.requestCollection(player.currentTrack, track => {
    addToPlaylistTrack.value = track
    showAddToPlaylist.value = true
  }, undefined, addToPlaylistAbort.signal)
}

// 睡眠定时器选项
const sleepOptions = computed(() => [
  { label: t('player.sleep_15'), value: 15 },
  { label: t('player.sleep_30'), value: 30 },
  { label: t('player.sleep_45'), value: 45 },
  { label: t('player.sleep_60'), value: 60 },
  { label: t('player.sleep_90'), value: 90 },
  { label: t('player.sleep_end_of_track'), value: -1 },
  { label: t('player.sleep_end_of_queue'), value: -2 },
])

function handleSleepOption(value: number) {
  if (value === -1) {
    player.startSleepTimerEndOfTrack()
  } else if (value === -2) {
    player.startSleepTimerEndOfQueue()
  } else {
    player.startSleepTimer(value)
  }
}

const nowPlayingTrackKey = computed(() => (
  playbackSessionTrackKey(
    player.hasPlaybackSession,
    player.currentTrack?.playlistKey,
    player.currentTrack?.id,
  )
))
const transitionStateClass = computed(() => props.transitionState ? `np-shell--${props.transitionState}` : '')
const favoriteVisualKey = computed(() => `${nowPlayingTrackKey.value}:${isFavorite.value ? 'favorite' : 'normal'}`)
const headerAlbumKey = computed(() => `${nowPlayingTrackKey.value}:${albumName.value || 'album'}`)
const coverTransitionName = computed(() => {
  if (!isTrackSwitchAnimating.value) return 'np-cover-static'
  if (trackSwitchDirection.value === 'prev') return 'np-cover-flow-prev'
  if (trackSwitchDirection.value === 'next') return 'np-cover-flow-next'
  return 'np-cover-static'
})
const metaTransitionName = computed(() => {
  if (!isTrackSwitchAnimating.value) return 'np-meta-static'
  if (trackSwitchDirection.value === 'prev') return 'np-meta-flow-prev'
  if (trackSwitchDirection.value === 'next') return 'np-meta-flow-next'
  return 'np-meta-static'
})
const controlsPulseClass = computed(() => (
  controlFeedbackPulse.value && lastControlDirection.value === 'misc'
    ? 'np-controls--feedback'
    : ''
))
const isVisualBeatActive = computed(() => isTrackSwitchAnimating.value)
const cardCoverRef = ref<HTMLDivElement>()

function closeToolbarPopovers(except?: 'queue' | 'sleep' | 'volume' | 'audiofx' | 'add') {
  if (except !== 'queue') showQueue.value = false
  if (except !== 'sleep') showSleepMenu.value = false
  if (except !== 'volume') showVolumeSlider.value = false
  if (except !== 'audiofx') showAudioFxPanel.value = false
  if (except !== 'add') showAddToPlaylist.value = false
}

function toggleToolbarPanel(panel: 'sleep' | 'volume' | 'audiofx' | 'add') {
  const nextOpen = panel === 'sleep'
    ? !showSleepMenu.value
    : panel === 'volume'
      ? !showVolumeSlider.value
      : panel === 'audiofx'
        ? !showAudioFxPanel.value
        : !showAddToPlaylist.value

  closeToolbarPopovers(nextOpen ? panel : undefined)

  if (!nextOpen) return
  if (panel === 'sleep') showSleepMenu.value = true
  else if (panel === 'volume') showVolumeSlider.value = true
  else if (panel === 'audiofx') showAudioFxPanel.value = true
  else openAddToPlaylist()
}

function triggerControlFeedbackPulse(direction: 'prev' | 'next' | 'misc' = 'misc') {
  lastControlDirection.value = direction
  controlFeedbackPulse.value = Date.now()
  if (controlFeedbackPulseTimer) clearTimeout(controlFeedbackPulseTimer)
  controlFeedbackPulseTimer = setTimeout(() => {
    controlFeedbackPulse.value = 0
    lastControlDirection.value = null
  }, 280)
}

function handlePrevClick() {
  trackSwitchDirection.value = 'prev'
  triggerControlFeedbackPulse('prev')
  player.previous()
}

function handleNextClick() {
  trackSwitchDirection.value = 'next'
  triggerControlFeedbackPulse('next')
  player.next()
}

function handleTogglePlayPause() {
  player.togglePlayPause()
}

function handleToggleShuffle() {
  triggerControlFeedbackPulse('misc')
  player.toggleShuffle()
}

function handleToggleRepeatMode() {
  triggerControlFeedbackPulse('misc')
  player.toggleRepeatMode()
}

function handleOpenQueue() {
  triggerControlFeedbackPulse('misc')
  closeToolbarPopovers('queue')
  showQueue.value = true
}

type CoverSnapshot = {
  rect: { left: number; top: number; width: number; height: number }
  borderRadius: string
  src: string
}

function getCoverSnapshot(): CoverSnapshot | null {
  const src = coverUrl.value
  if (!src) return null
  // 唱片在转，取它的包围盒会比圆盘大一圈；取不转的外层
  const targetEl = settings.coverStyle === 'card'
    ? cardCoverRef.value
    : coverWrapRef.value
  if (!targetEl) return null
  const rect = targetEl.getBoundingClientRect()
  return {
    rect: {
      left: rect.left,
      top: rect.top,
      width: rect.width,
      height: rect.height,
    },
    borderRadius: getComputedStyle(targetEl).borderRadius,
    src,
  }
}

// 封面加载错误时重置
watch(nowPlayingTrackKey, () => {
  coverLoadError.value = false
  closeToolbarPopovers()
  if (!controlFeedbackPulse.value || lastControlDirection.value === 'misc') {
    trackSwitchDirection.value = 'neutral'
  }
  isTrackSwitchAnimating.value = true
  if (trackSwitchAnimTimer) clearTimeout(trackSwitchAnimTimer)
  trackSwitchAnimTimer = setTimeout(() => {
    isTrackSwitchAnimating.value = false
    trackSwitchDirection.value = 'neutral'
  }, 560)
})

// 唱片旋转走 CSS 动画（合成线程），暂停时 animation-play-state 停在原角度
const coverWrapRef = ref<HTMLDivElement>()

// 逐帧变化的播放位置以读取函数交给进度条与歌词，本组件的模板不读它，播放时不会每帧整页重渲染
const readInterpolatedProgress = () => player.interpolatedProgress
const readInterpolatedPositionMs = () => player.interpolatedPositionMs
// Level and beat impulse change with the 200ms audio-level event, so they are read inside
// the background frame loop only
const readAudioLevel = () => (settings.audioReactive ? player.audioLevel : 0)
const readBeatImpulse = () => (settings.audioReactive ? player.beatImpulse : 0)

function onVolumeWheel(event: WheelEvent) {
  const next = wheelAdjustedVolume(player.volume, event.deltaY)
  if (next !== player.volume) void player.setVolume(next)
}

// ESC 分层关闭: 每次只收起最上层, 全部收起后才由 App 全局回退关闭本页 (对齐 Android 返回语义)
function handleEscapeLayered(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented) return
  // 输入框有焦点时交给全局逻辑先失焦
  if (isEditableTarget(event.target)) return
  if (contextMenu.value.show) {
    contextMenu.value.show = false
    event.preventDefault()
    return
  }
  if (showMoreSheet.value) {
    if (moreSheetView.value !== 'main') {
      goBackToMain()
    } else {
      showMoreSheet.value = false
    }
    event.preventDefault()
    return
  }
  if (showQueue.value || showSleepMenu.value || showVolumeSlider.value || showAudioFxPanel.value) {
    closeToolbarPopovers()
    event.preventDefault()
  }
}

onMounted(() => {
  document.addEventListener('keydown', handleEscapeLayered)
})
onUnmounted(() => {
  document.removeEventListener('keydown', handleEscapeLayered)
  if (trackSwitchAnimTimer) clearTimeout(trackSwitchAnimTimer)
  if (controlFeedbackPulseTimer) clearTimeout(controlFeedbackPulseTimer)
  if (moreSheetSwitchTimer) clearTimeout(moreSheetSwitchTimer)
  if (previewConvergeTimer) clearTimeout(previewConvergeTimer)
  closeToolbarPopovers()
})

defineExpose({
  toggleMore() {
    if (player.hasPlaybackSession) showMoreSheet.value = !showMoreSheet.value
  },
  getCoverSnapshot,
})

// 封面大图预览（#52）：原图优先，加载完成前沿用已显示的封面
const coverPreviewOpen = ref(false)
const coverPreviewSrc = computed(() => getTrackCoverUrl(player.currentTrack, 'original') || coverUrl.value)
function openCoverPreview() {
  if (coverUrl.value) coverPreviewOpen.value = true
}

const {
  contextMenu,
  artistLinksLoading,
  contextMenuItems,
  openArtistPage,
  openContextMenu,
  closeContextMenu,
  copyText,
  handleContextMenuClick,
  saveCoverArt,
} = useNowPlayingContextMenu({ ipc: nowPlayingIpc, emit, coverUrl, shareSong, openAddToPlaylist, openCoverPreview })

watch(() => player.hasPlaybackSession, (hasSession) => {
  if (hasSession) return
  closeToolbarPopovers()
  showQueue.value = false
  showMoreSheet.value = false
  showLtPanel.value = false
  contextMenu.value.show = false
})

const displayLyrics = computed(() => currentLyrics.lines)

// 更多选项面板子视图
const moreSheetView = ref<MoreSheetView>('main')
const moreSheetTransition = ref('slide-left')

function goToSubView(view: MoreSheetView) {
  moreSheetTransition.value = 'slide-left'
  moreSheetView.value = view
}

// 点击进度条旁的音质标签直接打开音质切换面板（ST-05：让该设置真实可切换而非纯展示）
function openQualitySwitcher() {
  if (!canSwitchCurrentAudioQuality.value || player.isLoadingAudio) return
  showMoreSheet.value = true
  goToSubView('quality')
}
function openBiliVideoSkip() {
  const track = player.currentTrack
  if (!track || !resolveBiliVideoSkipBvid(track)) return
  biliVideoSkipTrack.value = { ...track, syncPayload: track.syncPayload ? { ...track.syncPayload } : undefined }
  closeToolbarPopovers()
  showMoreSheet.value = false
  showBiliVideoSkip.value = true
}
function goBackToMain() {
  goBackTo('main')
}

function goBackTo(view: MoreSheetView) {
  moreSheetTransition.value = 'slide-right'
  moreSheetView.value = view
}

// 关闭更多选项面板时重置子视图
watch(showMoreSheet, (v) => { if (!v) setTimeout(() => { moreSheetView.value = 'main' }, 220) })

const audioQuality = useAudioQuality()
const { canSwitchCurrentAudioQuality, currentAudioQualityLabel } = audioQuality
watch(canSwitchCurrentAudioQuality, (canSwitch) => {
  if (!canSwitch && moreSheetView.value === 'quality') goBackToMain()
})

// 歌词渲染用有效偏移（绝对值），与偏移面板、桌面歌词同一个算法
const currentLyricTotalOffsetMs = computed(() => lyricOffsetStore.effectiveOffsetMs(player.currentTrack))

function openAudioEffectsSettings() {
  showAudioFxPanel.value = false
  showMoreSheet.value = false
  void router.push('/settings?section=audio_effects').then(() => emit('collapse'))
}

function openListenTogetherFromMore() {
  hideMoreSheet()
  if (moreSheetSwitchTimer) clearTimeout(moreSheetSwitchTimer)
  moreSheetSwitchTimer = window.setTimeout(() => {
    moreSheetSwitchTimer = null
    if (!showMoreSheet.value) showLtPanel.value = true
  }, 220)
}

// 分享歌曲
async function shareSong() {
  const track = player.currentTrack
  if (!track) return
  // 构建分享文本
  let url = ''
  if (track.id.startsWith('netease:')) {
    const nid = track.id.replace('netease:', '')
    url = `https://music.163.com/song?id=${nid}`
  } else if (track.id.startsWith('bilibili:')) {
    const bid = resolveBiliVideoSkipBvid(track)
      || String(track.syncPayload?.audioId ?? track.syncPayload?.audio_id ?? track.id.replace(/^bilibili:/i, '')).split(':')[0]
    const videoId = /^\d+$/.test(bid || '') ? `av${bid}` : bid
    const cid = resolveBiliVideoSkipCid(track)
    if (videoId) url = `https://www.bilibili.com/video/${videoId}${cid ? `?cid=${cid}` : ''}`
  } else if (track.id.startsWith('youtube:')) {
    const vid = track.id.replace('youtube:', '')
    url = `https://music.youtube.com/watch?v=${vid}`
  }
  const text = url
    ? `${track.title} - ${track.artist}\n${url}`
    : `${track.title} - ${track.artist}`
  try {
    await writeClipboardText(text)
    toast.success(t('player.share_copied'))
  } catch {
    toast.error(t('player.copy_failed'))
  }
  showMoreSheet.value = false
}

// 专辑名
const albumName = computed(() => {
  const album = player.currentTrack?.album || ''
  return displayAlbum(album)
})

const { displayedAudioInfo, audioInfoParts, audioDetailEl, wrappedAudioSeparators } = useAudioInfoRow(currentAudioQualityLabel)

// AccentBackdrop 底色：没有播放会话或尚未取色时用中性深色
const accentBgStyle = computed(() => ({
  background: accentBackdropColor(player.hasPlaybackSession ? paletteResult.value?.accentBg : null),
}))
const shouldRenderDynamicBackground = computed(() => shouldShowDynamicBackground(
  player.hasPlaybackSession,
  settings.dynamicBackground,
  paletteResult.value,
))

// 动态主题 CSS 变量（对齐 Android M3 动态配色）
const dynamicColorVars = computed<DynamicColorVars>(() => {
  if (!player.hasPlaybackSession) return {}
  const p = paletteResult.value
  if (!p) return {}
  return buildDynamicColorVars(p.lightVibrant)
})

// 进度条与按钮共用强调色，避免再次压暗后在深色背景上看不清
const sliderActiveColor = computed(() => dynamicColorVars.value['--np-primary'] || '#fff')

provideNowPlayingSheetContext({
  ipc: nowPlayingIpc,
  emit,
  showMoreSheet,
  hideMoreSheet,
  moreSheetView,
  moreSheetTransition,
  goToSubView,
  goBackTo,
  goBackToMain,
  coverUrl,
  coverLoadError,
  handleNowPlayingCoverError,
  albumName,
  playbackSourceLabel,
  commentTarget,
  quality: audioQuality,
  editableLyricsOfCurrentTrack,
  cacheLyricsForTrack,
  removeCachedLyricsForCurrentTrack,
  commitLyricsToTrack,
  copyText,
  shareSong,
  openQualitySwitcher,
  openBiliVideoSkip,
  openListenTogetherFromMore,
  openComments() {
    showMoreSheet.value = false
    showComments.value = true
  },
  openAudioEffectsSettings,
})
</script>

<template>
  <div
    class="now-playing"
    :class="[
      transitionStateClass,
      {
        'np-shell--track-switching': isTrackSwitchAnimating,
        'np-shell--beat-active': isVisualBeatActive,
        'np--rounded-window': isMacPlatform,
      },
    ]"
    :style="[dynamicColorVars, { backgroundColor: accentBgStyle.background }]"
    @click="closeToolbarPopovers()"
  >
    <!-- AccentBackdrop 底色层（对齐 Android：主色降饱和+调暗） -->
    <div class="np-bg-solid" :style="accentBgStyle" />
    <!-- 动态背景：封面模糊 OR WebGL 着色器，互斥 -->
    <CoverBlurBackground
      v-if="player.hasPlaybackSession && settings.coverBlurBg"
      :cover-url="coverUrl"
      :blur-amount="settings.coverBlurAmount * COVER_BLUR_PX_PER_UNIT"
      :darken-alpha="Math.min(Math.max(settings.coverBlurDarken, 0), 0.8)"
    />
    <!-- Level and beat impulse are passed as readers: they change with the 200ms backend
         event, and binding the values directly re-rendered this component (including the
         whole more-options panel below) every 200ms. The readers return exactly the same
         values and the background frame loop still reads them per frame, so nothing about
         the rendering changes. -->
    <Transition name="np-background" appear>
      <HyperBackground
        v-if="!settings.coverBlurBg && shouldRenderDynamicBackground"
        :playing="player.isPlaying"
        :low-performance="settings.lowPerformanceMode"
        :suspended="frameLoopSuspended"
        :music-level="readAudioLevel"
        :beat-impulse="readBeatImpulse"
        :colors="extractedColors"
        :is-dark="true"
        :light-offset="paletteResult?.lightOffset ?? 0"
        :saturate-offset="paletteResult?.saturateOffset ?? 0"
      />
    </Transition>
    <div class="np-scrim" />
    <div class="np-ambient-glow" />

    <!-- 顶栏（融合模式下由 TitleBar 接管） -->
    <header v-if="!props.hideHeader" class="np-header">
      <button class="np-icon-btn" @click.stop="emit('collapse')">
        <span class="material-symbols-rounded">keyboard_arrow_down</span>
      </button>
      <div class="np-header-center">
        <span v-if="settings.showNowPlayingTitle" class="np-from-label">
          {{ player.hasPlaybackSession ? t('player.now_playing') : t('player.not_playing') }}
        </span>
        <transition name="np-header-meta-swap" mode="out-in">
          <span :key="headerAlbumKey" class="np-from-name">
            {{ player.hasPlaybackSession ? albumName : '' }}
          </span>
        </transition>
      </div>
      <button v-if="player.hasPlaybackSession" class="np-icon-btn" @click="showMoreSheet = !showMoreSheet">
        <span class="material-symbols-rounded">more_vert</span>
      </button>
      <span v-else class="np-header-spacer" aria-hidden="true" />
    </header>

    <div v-if="!player.hasPlaybackSession" class="np-empty-state">
      <span
        class="material-symbols-rounded np-empty-icon"
        :class="{ spinning: player.isLoadingAudio }"
      >{{ player.isLoadingAudio ? 'progress_activity' : 'music_off' }}</span>
      <h2>{{ player.isLoadingAudio ? t('player.loading') : t('player.not_playing') }}</h2>
    </div>

    <!-- 双栏 -->
    <div v-else class="np-body" :class="[{ 'np-body--no-header': props.hideHeader }, playViewMode === 'lyrics' ? 'np-body--lyrics-mode' : 'np-body--cover-mode']">
      <!-- 左侧：stack 固定内部高度，外层居中，切歌不上下重排 -->
      <section class="np-left">
        <div class="np-left-stack">
        <div
          ref="coverWrapRef"
          class="cover-wrap"
          :class="{
            'cover-wrap--card': settings.coverStyle === 'card',
            'cover-wrap--disc': settings.coverStyle !== 'card',
            'cover-wrap--switching': isTrackSwitchAnimating,
            'cover-wrap--previewable': !!coverUrl,
          }"
          :title="coverUrl ? t('player.cover_preview') : undefined"
          @click="openCoverPreview"
          @contextmenu="openContextMenu($event, 'cover')"
        >
          <!-- Card 模式（圆角矩形，对齐 Android） -->
          <div v-if="settings.coverStyle === 'card'" ref="cardCoverRef" class="cover-card">
            <transition :name="coverTransitionName">
              <img
                v-if="coverUrl && !coverLoadError"
                :key="`cover:${nowPlayingTrackKey}`"
                :src="coverUrl"
                referrerpolicy="no-referrer"
                class="cover-card-img"
                @load="handleNowPlayingCoverLoad"
                @error="handleNowPlayingCoverError"
              />
              <span
                v-else
                :key="`placeholder:${nowPlayingTrackKey}`"
                class="material-symbols-rounded filled cover-card-placeholder"
              >music_note</span>
            </transition>
          </div>
          <!-- Disc 模式（黑胶唱片） -->
          <div v-else class="cover-disc" :class="{ 'cover-disc--spinning': player.isPlaying }">
            <div class="cover-inner">
              <transition :name="coverTransitionName">
                <img
                  v-if="coverUrl && !coverLoadError"
                  :key="`disc-cover:${nowPlayingTrackKey}`"
                  :src="coverUrl"
                  referrerpolicy="no-referrer"
                  class="cover-img"
                  @load="handleNowPlayingCoverLoad"
                  @error="handleNowPlayingCoverError"
                />
                <span
                  v-else
                  :key="`disc-placeholder:${nowPlayingTrackKey}`"
                  class="material-symbols-rounded filled cover-disc-placeholder"
                >music_note</span>
              </transition>
            </div>
          </div>
          <div v-if="settings.coverStyle !== 'card'" class="cover-disc-sheen" aria-hidden="true" />
          <!-- 来源徽章（对齐 Android PlaybackSourceBadge） -->
          <transition name="np-badge-swap" mode="out-in">
            <div
              v-if="showSourceBadge"
              :key="sourceBadgeKey"
              class="source-badge"
              :class="{ 'source-badge--disc': settings.coverStyle !== 'card' }"
            >
              <span
                v-if="playbackSourceIcon === 'netease'"
                class="source-badge-icon source-badge-icon--netease"
                aria-hidden="true"
              />
              <span v-else class="material-symbols-rounded source-badge-icon">{{ playbackSourceIcon }}</span>
              <span class="source-badge-label">{{ playbackSourceLabel }}</span>
            </div>
          </transition>
        </div>

        <!-- 固定高度叠层：切歌时不 out-in 塌高度，避免整列重居中上下跳 -->
        <div class="np-info">
          <transition :name="metaTransitionName">
            <div :key="nowPlayingTrackKey" class="np-meta">
              <h2 class="np-title" @contextmenu="openContextMenu($event, 'title')">{{ player.currentTrack?.title || t('player.not_playing') }}</h2>
              <button type="button" class="np-artist" :disabled="artistLinksLoading || !player.currentTrack?.artist.trim()" :aria-label="t('player.open_artist')" @click="openArtistPage" @contextmenu="openContextMenu($event, 'artist')">{{ player.currentTrack?.artist || '' }}</button>
            </div>
          </transition>
        </div>

        <div class="np-slider-area">
          <WaveformSlider
            :progress="readInterpolatedProgress"
            :is-playing="player.isPlaying"
            :suspended="frameLoopSuspended"
            :active-color="sliderActiveColor"
            @seek="onSeek"
            @preview="onSliderPreview"
            @preview-end="onSliderPreviewEnd"
          />
          <div class="np-time">
            <span>{{ player.currentTimeFormatted }}</span>
            <span>{{ player.durationFormatted }}</span>
          </div>
          <!-- 音质行始终占位，空内容也保留高度 -->
          <div class="np-audio-info" :aria-busy="player.isLoadingAudio">
            <span v-if="displayedAudioInfo.fromDownload" class="np-download-chip"
              role="img"
              :title="t('player.playing_from_download')" :aria-label="t('player.playing_from_download')">
              <span class="material-symbols-rounded" aria-hidden="true">download_done</span>
            </span>
            <span v-if="audioInfoParts.length" ref="audioDetailEl" class="np-audio-detail" :class="{ separated: displayedAudioInfo.fromDownload }">
              <template v-for="(part, index) in audioInfoParts" :key="`${part.text}:${index}`">
                <span
                  class="np-audio-detail-part"
                  :class="{
                    'np-audio-detail-part--accent': part.accent,
                    'np-audio-detail-part--clickable': part.accent && canSwitchCurrentAudioQuality && !player.isLoadingAudio,
                  }"
                  :role="part.accent && canSwitchCurrentAudioQuality && !player.isLoadingAudio ? 'button' : undefined"
                  :tabindex="part.accent && canSwitchCurrentAudioQuality && !player.isLoadingAudio ? 0 : undefined"
                  @click="part.accent && !player.isLoadingAudio && openQualitySwitcher()"
                  @keydown.enter="part.accent && !player.isLoadingAudio && openQualitySwitcher()"
                >{{ part.text }}</span>
                <template v-if="index < audioInfoParts.length - 1">
                  <span
                    class="np-audio-separator"
                    :class="{ 'np-audio-separator--wrapped': wrappedAudioSeparators.has(index) }"
                    aria-hidden="true"
                  >·</span>{{ ' ' }}
                </template>
              </template>
            </span>
          </div>
        </div>

        <div
          class="np-controls"
          :class="{
            [controlsPulseClass]: !!controlFeedbackPulse,
            'np-controls--feedback-prev': lastControlDirection === 'prev',
            'np-controls--feedback-next': lastControlDirection === 'next',
          }"
        >
          <button
            class="ctrl-btn"
            :class="{ active: player.shuffleEnabled }"
            :title="t('shortcuts.shuffle')"
            :aria-label="t('shortcuts.shuffle')"
            :aria-pressed="player.shuffleEnabled"
            @click="handleToggleShuffle()"
          >
            <span class="material-symbols-rounded">shuffle</span>
          </button>

          <button
            class="ctrl-btn ctrl-btn--transport"
            :class="{ 'ctrl-btn--switching': isTrackSwitchAnimating }"
            :title="t('tray.previous')"
            :aria-label="t('tray.previous')"
            @click="handlePrevClick()"
          >
            <span class="material-symbols-rounded filled" style="font-size: 30px">skip_previous</span>
          </button>

          <!-- 播放/暂停 带动画 -->
          <button
            class="play-btn play-btn--transport"
            :class="{ 'play-btn--switching': isTrackSwitchAnimating }"
            :title="player.isPlaying ? t('tray.pause') : t('tray.play')"
            :aria-label="player.isPlaying ? t('tray.pause') : t('tray.play')"
            :disabled="player.isLoadingAudio"
            @click="handleTogglePlayPause()"
          >
            <transition name="play-icon">
              <span
                v-if="player.isLoadingAudioSlow"
                class="material-symbols-rounded spinning play-icon-inner"
                key="loading"
              >progress_activity</span>
              <span
                v-else
                class="material-symbols-rounded filled play-icon-inner"
                :key="player.isPlaying ? 'pause' : 'play'"
              >{{ player.isPlaying ? 'pause' : 'play_arrow' }}</span>
            </transition>
          </button>

          <button
            class="ctrl-btn ctrl-btn--transport"
            :class="{ 'ctrl-btn--switching': isTrackSwitchAnimating }"
            :title="t('tray.next')"
            :aria-label="t('tray.next')"
            @click="handleNextClick()"
          >
            <span class="material-symbols-rounded filled" style="font-size: 30px">skip_next</span>
          </button>

          <button
            class="ctrl-btn"
            :class="{ active: player.repeatMode !== 'off' }"
            :title="t('shortcuts.repeat')"
            :aria-label="t('shortcuts.repeat')"
            @click="handleToggleRepeatMode()"
          >
            <span class="material-symbols-rounded">{{ player.repeatMode === 'one' ? 'repeat_one' : 'repeat' }}</span>
          </button>
        </div>

        <!-- 工具栏（对齐 Android 底部：Favorite -> Queue -> Sleep -> Volume -> Speed -> Add） -->
        <div
          v-if="settings.showToolbarDock"
          class="np-toolbar"
          @click.stop
        >
          <button
            class="tool-btn tool-btn--feedback fav-btn"
            :class="{ active: isFavorite }"
            :disabled="!player.currentTrack"
            :title="t('library.liked_songs')"
            :aria-label="t('library.liked_songs')"
            :aria-pressed="isFavorite"
            @click="toggleFavorite"
          >
            <transition name="np-favorite-swap" mode="out-in">
              <span
                :key="favoriteVisualKey"
                class="material-symbols-rounded"
                :class="{ filled: isFavorite }"
              >favorite</span>
            </transition>
          </button>
          <button class="tool-btn tool-btn--feedback" :title="t('player.queue')" :aria-label="t('player.queue')" @click="handleOpenQueue()">
            <span class="material-symbols-rounded">queue_music</span>
          </button>
          <!-- 睡眠定时器 -->
          <div class="sleep-wrap">
            <button
              class="tool-btn tool-btn--feedback"
              :class="{ active: player.sleepTimerMode || showSleepMenu }"
              :title="t('player.sleep_timer')"
              :aria-label="t('player.sleep_timer')"
              :aria-expanded="showSleepMenu"
              @click="triggerControlFeedbackPulse(); toggleToolbarPanel('sleep')"
            >
              <span class="material-symbols-rounded">timer</span>
              <span v-if="player.sleepRemainingSeconds > 0" class="sleep-badge">
                {{ formatSleepRemaining(player.sleepRemainingSeconds) }}
              </span>
            </button>
            <Transition name="np-popover">
            <div
              v-if="showSleepMenu"
              class="sleep-popover np-floating-popover np-floating-popover--menu"
              role="group"
              :aria-label="t('player.sleep_timer')"
            >
              <div class="np-popover-header">
                <span class="np-popover-title">{{ t('player.sleep_timer') }}</span>
                <span v-if="player.sleepRemainingSeconds > 0" class="np-popover-meta">
                  {{ formatSleepRemaining(player.sleepRemainingSeconds) }}
                </span>
              </div>
              <div class="np-popover-group">
                <button
                  v-for="opt in sleepOptions.filter(option => option.value > 0)"
                  :key="opt.value"
                  type="button"
                  class="sleep-option"
                  @click="handleSleepOption(opt.value); showSleepMenu = false"
                >
                  {{ opt.label }}
                </button>
              </div>
              <div class="np-popover-group">
                <button
                  v-for="opt in sleepOptions.filter(option => option.value < 0)"
                  :key="opt.value"
                  type="button"
                  class="sleep-option"
                  :class="{ active: player.sleepTimerMode === (opt.value === -1 ? 'end_of_track' : 'end_of_queue') }"
                  :aria-current="player.sleepTimerMode === (opt.value === -1 ? 'end_of_track' : 'end_of_queue') || undefined"
                  @click="handleSleepOption(opt.value); showSleepMenu = false"
                >
                  <span class="sleep-option-label">{{ opt.label }}</span>
                  <span
                    v-if="player.sleepTimerMode === (opt.value === -1 ? 'end_of_track' : 'end_of_queue')"
                    class="material-symbols-rounded sleep-option-check"
                    aria-hidden="true"
                  >check</span>
                </button>
              </div>
              <button
                v-if="player.sleepTimerMode"
                type="button"
                class="sleep-option cancel"
                @click="player.cancelSleepTimer(); showSleepMenu = false"
              >
                {{ t('player.sleep_off') }}
              </button>
            </div>
            </Transition>
          </div>
          <div class="volume-wrap">
            <button
              class="tool-btn tool-btn--feedback"
              :class="{ active: showVolumeSlider }"
              :title="t('player.volume')"
              :aria-label="t('player.volume')"
              :aria-expanded="showVolumeSlider"
              @click="triggerControlFeedbackPulse(); toggleToolbarPanel('volume')"
              @wheel.prevent="onVolumeWheel"
            >
              <span class="material-symbols-rounded">{{ player.volume === 0 ? 'volume_off' : player.volume < 0.5 ? 'volume_down' : 'volume_up' }}</span>
            </button>
            <Transition name="np-popover">
            <div
              v-if="showVolumeSlider"
              class="volume-popover np-floating-popover np-floating-popover--volume"
              role="group"
              :aria-label="t('player.volume')"
            >
              <EditableRangeValue
                :model-value="player.volume"
                class="volume-label"
                :min="0"
                :max="1"
                :step="0.01"
                :input-scale="100"
                :input-width="32"
                :display-value="`${Math.round(player.volume * 100)}%`"
                input-suffix="%"
                :aria-label="t('player.volume')"
                @update:model-value="player.setVolume($event)"
              />
              <input
                type="range"
                min="0"
                max="1"
                step="0.01"
                :value="player.volume"
                class="volume-slider"
                :style="{ '--volume-fill': `${Math.round(player.volume * 100)}%` }"
                :aria-valuetext="`${Math.round(player.volume * 100)}%`"
                @input="player.setVolume(parseFloat(($event.target as HTMLInputElement).value))"
              />
              <span class="material-symbols-rounded volume-popover-icon" aria-hidden="true">{{ player.volume === 0 ? 'volume_off' : player.volume < 0.5 ? 'volume_down' : 'volume_up' }}</span>
            </div>
            </Transition>
          </div>
          <!-- 音效 (AudioFX) -->
          <div class="speed-wrap">
            <button
              class="tool-btn tool-btn--feedback"
              :class="{ active: showAudioFxPanel || player.hasActiveEffects }"
              :title="t('player.audio_effects')"
              :aria-label="t('player.audio_effects')"
              :aria-expanded="showAudioFxPanel"
              @click="triggerControlFeedbackPulse(); toggleToolbarPanel('audiofx')"
            >
              <span class="material-symbols-rounded">tune</span>
            </button>
            <Transition name="np-popover">
              <div
                v-if="showAudioFxPanel"
                class="audiofx-popover np-floating-popover np-floating-popover--audiofx"
                role="group"
                :aria-label="t('player.audio_effects')"
              >
                <AudioEffectsPanel @details="openAudioEffectsSettings" />
              </div>
            </Transition>
          </div>
          <button
            class="tool-btn tool-btn--feedback"
            :title="t('player.add_to_playlist')"
            :aria-label="t('player.add_to_playlist')"
            @click="triggerControlFeedbackPulse(); toggleToolbarPanel('add')"
          >
            <span class="material-symbols-rounded">playlist_add</span>
          </button>
          <!-- 评论（对齐 Android：仅网易云、B 站） -->
          <button
            v-if="commentTarget"
            class="tool-btn tool-btn--feedback"
            :class="{ active: showComments }"
            :title="t('player.comments')"
            :aria-label="t('player.comments')"
            @click="triggerControlFeedbackPulse(); closeToolbarPopovers(); showComments = true"
          >
            <span class="material-symbols-rounded">chat_bubble</span>
          </button>
        </div>
        </div>
      </section>

      <!-- 右侧歌词 -->
      <section class="np-right" :class="{ 'np-right--switching': isTrackSwitchAnimating, 'np-right--beat-active': isVisualBeatActive }">
        <LyricsView
          v-if="displayLyrics.length > 0"
          :lyrics="displayLyrics"
          :current-time-ms="readInterpolatedPositionMs"
          :preview-time-ms="previewPositionMs"
          :is-playing="player.isPlaying"
          :lyric-offset-ms="currentLyricTotalOffsetMs"
          :seek-seq="player.lastSeekCommand.seq"
          :suspended="frameLoopSuspended"
          @seek="onLyricSeek"
        />
        <div v-else-if="isFetchingLyrics" class="lyrics-empty">
          <span class="material-symbols-rounded spinning" style="font-size: 36px">progress_activity</span>
          <p>{{ t('player.loading') }}</p>
        </div>
        <div v-else class="lyrics-empty">
          <span class="material-symbols-rounded" style="font-size: 36px">lyrics</span>
          <p>{{ t('player.no_lyrics') }}</p>
        </div>
      </section>
    </div>

    <!-- 播放队列面板（Teleport 到 body，避免被全屏页开合动画的 transform 包含块限制） -->
    <Teleport to="body">
      <Transition name="queue-sheet">
        <QueuePanel v-if="player.hasPlaybackSession && showQueue" @close="showQueue = false" />
      </Transition>
    </Teleport>
    <AddToPlaylistDialog v-if="player.hasPlaybackSession" v-model:open="showAddToPlaylist" :track="addToPlaylistTrack" />
    <BiliVideoSkipDialog v-model:open="showBiliVideoSkip" :track="biliVideoSkipTrack" />
    <CommentsPanel v-model:open="showComments" :track="player.currentTrack" />
    <ListenTogetherPanel v-if="player.hasPlaybackSession" v-model:open="showLtPanel" />

    <NowPlayingMoreSheet />

    <CoverPreview
      :open="coverPreviewOpen"
      :src="coverPreviewSrc"
      :placeholder="coverUrl"
      :title="player.currentTrack?.title"
      @close="coverPreviewOpen = false"
      @save="saveCoverArt"
    />

    <ContextMenu
      :open="player.hasPlaybackSession && contextMenu.show"
      :x="contextMenu.x"
      :y="contextMenu.y"
      :items="contextMenuItems"
      @update:open="contextMenu.show = $event"
      @close="closeContextMenu"
      @click="handleContextMenuClick"
    />
  </div>
</template>

<style scoped lang="scss" src="./now-playing/NowPlaying.scss"></style>

<style lang="scss" src="./now-playing/NowPlaying.global.scss"></style>
