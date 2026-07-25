<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { usePlayerStore, displayAlbum, type LyricLine, type TrackInfo } from '@/stores/player'
import { useLikedSongsStore } from '@/stores/likedSongs'
import { useAuthStore } from '@/stores/auth'
import { useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import { useDownloadStore } from '@/stores/download'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { invoke } from '@tauri-apps/api/core'
import { extractPalette, type PaletteResult } from '@/utils/paletteExtractor'
import { shouldShowDynamicBackground } from '@/utils/nowPlayingBackground'
import {
  normalizeCoverUrlForDisplay,
  normalizeProxiedCoverUrl,
  peekCoverImage,
  resolveCoverImage,
} from '@/utils/bilibiliCover'
import { clearCachedLyrics, getCachedLyrics, saveCachedLyrics } from '@/modules/lyrics/lyricsCache'
import { hasLyricsRequestInFlight, loadLyricsSingleFlight } from '@/modules/lyrics/lyricsRequest'
import {
  toEditableLyricsText,
  toEditableTranslationText,
  resolveStoredLyricStateFromPayload,
  resolveStoredTranslatedLyricStateFromPayload,
  withUpdatedLyricsPayload,
  mapBackendLyrics as mapBackendLyricsShared,
  mergeParsedLyricsWithTranslations,
} from '@/modules/lyrics/lyricsFormat'
import { offsetBucketForSource } from '@/modules/lyrics/lyricOffset'
import { persistTrackSyncPayload } from '@/modules/lyrics/syncTrackPayload'
import { useLyricOffsetStore } from '@/stores/lyricOffset'
import HyperBackground from './HyperBackground.vue'
import CoverBlurBackground from './CoverBlurBackground.vue'
import BilibiliCoverImage from './BilibiliCoverImage.vue'
import WaveformSlider from './WaveformSlider.vue'
import LyricsView from './LyricsView.vue'
import NowPlayingViewSwitch from './NowPlayingViewSwitch.vue'
import QueuePanel from './QueuePanel.vue'
import AddToPlaylistDialog from './AddToPlaylistDialog.vue'
import ListenTogetherPanel from './ListenTogetherPanel.vue'
import CustomSelect from './ui/CustomSelect.vue'
import EditableRangeValue from './ui/EditableRangeValue.vue'
import ContextMenu from './ui/ContextMenu.vue'
import type { ContextMenuActionItem } from '@/utils/contextMenu'
import { playbackSessionTrackKey } from '@/modules/playback/playbackRequest'
import {
  resolveNowPlayingViewMode,
  type NowPlayingViewMode,
} from '@/modules/nowPlaying/viewMode'
import { createLogger } from '@/utils/logger'
import { getTrackCoverUrl } from '@/utils/trackCover'
import { summarizeLogError } from '@/utils/logSanitizer'

const log = createLogger('now-playing')

const emit = defineEmits<{ collapse: [] }>()
const props = defineProps<{
  hideHeader?: boolean
  transitionState?: 'opening' | 'closing' | null
}>()
const player = usePlayerStore()
const likedSongs = useLikedSongsStore()
const auth = useAuthStore()
const settings = useSettingsStore()
const toast = useToastStore()
const downloadStore = useDownloadStore()
const lyricOffsetStore = useLyricOffsetStore()
const router = useRouter()
const { t } = useI18n()
const playViewMode = ref<NowPlayingViewMode>('cover')
const coverLoadError = ref(false)
const coverUrl = ref('')
const showVolumeSlider = ref(false)
const showQueue = ref(false)
const showAddToPlaylist = ref(false)
const showAudioFxPanel = ref(false)
const showSleepMenu = ref(false)
const showMoreSheet = ref(false)
const showLtPanel = ref(false)
const isTrackSwitchAnimating = ref(false)
const trackSwitchDirection = ref<'prev' | 'next' | 'neutral'>('neutral')
const controlFeedbackPulse = ref(0)
const lastControlDirection = ref<'prev' | 'next' | 'misc' | null>(null)
let trackSwitchAnimTimer: ReturnType<typeof setTimeout> | null = null
let controlFeedbackPulseTimer: ReturnType<typeof setTimeout> | null = null
let moreSheetSwitchTimer: ReturnType<typeof setTimeout> | null = null
let coverRenderRetryCount = 0

function coverSourceLabel(rawUrl: string): string {
  if (!rawUrl) return 'empty'
  if (rawUrl.startsWith('data:')) return `data-url(${rawUrl.length})`
  try {
    return new URL(rawUrl).hostname || 'url'
  } catch {
    return `invalid(${rawUrl.length})`
  }
}

function hideMoreSheet() {
  showMoreSheet.value = false
}

// 均衡器辅助
import { EQ_PRESETS } from '@/stores/player'
const eqPresetIds = Object.keys(EQ_PRESETS)
const eqFreqLabels = ['60', '230', '910', '3.6k', '14k']

function onEqBandChange(index: number, value: number) {
  const bands = [...player.equalizerBands]
  bands[index] = Math.round(value)
  player.setEqualizer(player.equalizerEnabled, bands)
  player.equalizerPresetId = 'custom'
  settings.equalizerPresetId = 'custom'
}

// 来源徽章（对齐 Android PlaybackSourceBadge）
const playbackSourceLabel = computed(() => {
  const id = player.currentTrack?.id || ''
  if (id.startsWith('netease:')) return t('player.source_netease')
  if (id.startsWith('qq:')) return t('player.source_qq')
  if (id.startsWith('bilibili:')) return t('player.source_bilibili')
  if (id.startsWith('youtube:')) return t('player.source_youtube')
  if (id.startsWith('local:') || player.currentTrack?.audioUrl?.startsWith('file:')) return t('player.source_local')
  return ''
})
const playbackSourceIcon = computed(() => {
  const id = player.currentTrack?.id || ''
  if (id.startsWith('netease:')) return 'netease'
  if (id.startsWith('qq:')) return 'music_note'
  if (id.startsWith('bilibili:')) return 'smart_display'
  if (id.startsWith('youtube:')) return 'play_circle'
  return 'folder'
})
const showSourceBadge = computed(() => settings.showCoverBadge && playbackSourceLabel.value !== '')
const sourceBadgeKey = computed(() => `${nowPlayingTrackKey.value}:${playbackSourceIcon.value}:${playbackSourceLabel.value || 'none'}`)

function platformLabel(source?: string) {
  switch ((source || '').toLowerCase()) {
    case 'netease': return t('player.source_netease')
    case 'qq': return t('player.source_qq')
    case 'bilibili': return t('player.source_bilibili')
    case 'youtube': return t('player.source_youtube')
    case 'lrclib': return 'LRCLIB'
    case 'local': return t('player.source_local')
    default: return source || ''
  }
}

function mapBackendLyrics(lyrics: any[]): LyricLine[] {
  return mapBackendLyricsShared(lyrics)
}

function readCachedLyrics(track: TrackInfo) {
  return getCachedLyrics(track)
}

// 同步歌词落地: 将云同步下来的 matched/original 歌词解析为本地歌词行
// 仅读取 syncPayload, 不在读取路径回写云端
// 返回 null 表示无本地覆盖 (可在线拉取); [] 表示有意清空或解析失败
async function materializeSyncedLyrics(track: TrackInfo): Promise<LyricLine[] | null> {
  const payload = track.syncPayload
  const lyricState = resolveStoredLyricStateFromPayload(payload)
  if (lyricState.kind === 'absent') return null
  if (lyricState.kind === 'cleared') return []
  try {
    const parsed = await invoke<any[]>('parse_lrc_content', { content: lyricState.text })
    const translationState = resolveStoredTranslatedLyricStateFromPayload(payload)
    let parsedTranslations: any[] = []
    if (translationState.kind === 'present' && translationState.text.trim()) {
      try {
        parsedTranslations = await invoke<any[]>('parse_lrc_content', {
          content: translationState.text,
        })
      } catch (e) {
        log.warn('Parse synced translation failed:', e)
      }
    }
    return mergeParsedLyricsWithTranslations(
      mapBackendLyrics(parsed),
      mapBackendLyrics(parsedTranslations),
    )
  } catch (e) {
    log.warn('Materialize synced lyrics failed:', e)
    return []
  }
}

function cacheLyricsForTrack(track: TrackInfo | null | undefined, lines: LyricLine[]) {
  if (!track || lines.length === 0) return
  saveCachedLyrics(track, lines)
}

function removeCachedLyricsForCurrentTrack() {
  if (!player.currentTrack) return
  clearCachedLyrics(player.currentTrack)
}

/** 编辑后的歌词写回 syncPayload + 本地歌单, 供同步上传 (对齐 Android) */
async function commitLyricsToTrack(
  nextLyric: string | null,
  nextTranslated: string | null,
  source?: string | null,
) {
  const track = player.currentTrack
  if (!track) return
  const nextPayload = withUpdatedLyricsPayload(
    track.syncPayload,
    nextLyric,
    nextTranslated,
    source ?? 'LOCAL_EDIT',
  )
  // CURRENT version: 有意清空也会上传 None, 与 Android v1 一致
  nextPayload.syncMetadataVersion = 1
  delete nextPayload.sync_metadata_version
  player.patchCurrentTrackSyncPayload(nextPayload)
  const updatedTrack = player.currentTrack
  if (updatedTrack) {
    await persistTrackSyncPayload(updatedTrack)
  }
}

// 歌词编辑器
const lyricsEditorText = ref('')
const lyricsTranslationEditorText = ref('')
const lyricsEditorTab = ref<'original' | 'translation'>('original')

function openLyricsEditor() {
  // 优先使用 syncPayload 原文(保持用户编辑/YRC 源文本), 否则从当前展示行导出
  const payload = player.currentTrack?.syncPayload
  const stored = resolveStoredLyricStateFromPayload(payload)
  const storedTranslation = resolveStoredTranslatedLyricStateFromPayload(payload)
  const lines = displayLyrics.value
  lyricsEditorTab.value = 'original'
  if (stored.kind === 'present') {
    lyricsEditorText.value = stored.text
  } else if (lines.length > 0) {
    lyricsEditorText.value = toEditableLyricsText(lines)
  } else {
    lyricsEditorText.value = ''
  }
  if (storedTranslation.kind === 'present') {
    lyricsTranslationEditorText.value = storedTranslation.text
  } else if (lines.length > 0) {
    lyricsTranslationEditorText.value = toEditableTranslationText(lines)
  } else {
    lyricsTranslationEditorText.value = ''
  }
  goToSubView('lyrics-editor')
}

async function applyLyricsFromEditor() {
  const text = lyricsEditorText.value.trim()
  const translationText = lyricsTranslationEditorText.value.trim()
  if (!text) {
    // 清除歌词: 本地 cache + syncPayload matched* 置空 (CURRENT 版本会同步清空)
    fetchedLyrics.value = []
    removeCachedLyricsForCurrentTrack()
    await commitLyricsToTrack(null, null, 'LOCAL_EDIT')
    toast.success(t('player.lyrics_cleared'))
    goBackToMain()
    return
  }
  try {
    // parse_lrc_content 已走 parse_auto, 支持 YRC 逐字往返
    const parsed = await invoke<any[]>('parse_lrc_content', { content: text })
    let parsedTranslations: any[] = []
    if (translationText) {
      try {
        parsedTranslations = await invoke<any[]>('parse_lrc_content', { content: translationText })
      } catch (e) {
        log.warn('Parse translation LRC failed, applying original only:', e)
      }
    }
    const nextLyrics = mergeParsedLyricsWithTranslations(
      mapBackendLyrics(parsed),
      mapBackendLyrics(parsedTranslations),
    )
    fetchedLyrics.value = nextLyrics
    cacheLyricsForTrack(player.currentTrack, nextLyrics)
    // 原文保留编辑器文本(YRC/LRC), 与 Android toEditableLyricsText 往返一致
    await commitLyricsToTrack(text, translationText || null, 'LOCAL_EDIT')
    toast.success(t('player.lyrics_applied'))
  } catch (e) {
    log.error('Parse lyrics failed:', e)
    toast.error(String(e))
  }
  goBackToMain()
}

// 歌词填充（搜索 + 应用歌词）
const lyricFillQuery = ref('')
const lyricFillResults = ref<any[]>([])
const isLyricFilling = ref(false)
const lyricFillPlatform = ref<'netease' | 'lrclib' | 'qq'>('netease')

async function parseLyricsFromSearchResult(result: any): Promise<{
  lines: LyricLine[]
  rawLyric: string
  rawTranslated: string | null
} | null> {
  if (result?.synced_lyrics) {
    const rawLyric = String(result.synced_lyrics)
    const rawTranslated = result?.translated_lyrics ? String(result.translated_lyrics) : null
    const parsed = await invoke<any[]>('parse_lrc_content', { content: rawLyric })
    let parsedTranslations: any[] = []
    if (rawTranslated) {
      try {
        parsedTranslations = await invoke<any[]>('parse_lrc_content', { content: rawTranslated })
      } catch {}
    }
    return {
      lines: mergeParsedLyricsWithTranslations(
        mapBackendLyrics(parsed),
        mapBackendLyrics(parsedTranslations),
      ),
      rawLyric,
      rawTranslated,
    }
  }

  if (result?.plain_lyrics) {
    const rawLyric = String(result.plain_lyrics)
    const lines = rawLyric
      .split(/\r?\n/)
      .map((line: string) => line.trim())
      .filter(Boolean)
      .map((line: string, index: number) => ({
        startMs: index * 3000,
        durationMs: 3000,
        words: [] as LyricLine['words'],
        text: line,
        translation: undefined as string | undefined,
      }))
    return { lines, rawLyric, rawTranslated: null }
  }

  return null
}

async function doLyricFillSearch() {
  const q = lyricFillQuery.value.trim()
  if (!q) return
  isLyricFilling.value = true
  lyricFillResults.value = []
  try {
    const results = await invoke<any[]>('search', {
      query: q,
      platform: lyricFillPlatform.value,
      includeLyrics: true,
    })
    lyricFillResults.value = results
  } catch (e) {
    log.error('Lyric fill search failed:', e)
  } finally {
    isLyricFilling.value = false
  }
}

async function applyLyricFill(result: any) {
  try {
    const source = String(result?.platform || lyricFillPlatform.value || 'LOCAL_EDIT').toUpperCase()
    const direct = await parseLyricsFromSearchResult(result)
    if (direct && direct.lines.length > 0) {
      fetchedLyrics.value = direct.lines
      cacheLyricsForTrack(player.currentTrack, direct.lines)
      // 搜索填充也写回本地 syncPayload, 不直接覆写云端; 下次同步上传
      await commitLyricsToTrack(direct.rawLyric, direct.rawTranslated, source)
      toast.success(t('player.lyrics_fill_applied'))
      goBackToMain()
      return
    }
    const idText = String(result.id || '')
    const neteaseId = idText.startsWith('netease:') ? parseInt(idText.replace('netease:', '')) : null
    const qqSongMid = idText.startsWith('qq:') ? idText.replace('qq:', '') : null
    const lyrics = await invoke<any[]>('fetch_lyrics', {
      title: result.title || '',
      artist: result.artist || '',
      durationSecs: Math.floor((result.duration_ms || 0) / 1000),
      audioPath: null,
      neteaseId: neteaseId,
      qqSongMid,
            youtubeVideoId: null,
      })
    const nextLyrics = mapBackendLyrics(lyrics)
    fetchedLyrics.value = nextLyrics
    cacheLyricsForTrack(player.currentTrack, nextLyrics)
    if (nextLyrics.length > 0) {
      await commitLyricsToTrack(
        toEditableLyricsText(nextLyrics),
        toEditableTranslationText(nextLyrics) || null,
        source,
      )
    }
    toast.success(t('player.lyrics_fill_applied'))
    goBackToMain()
  } catch (e) {
    log.error('Lyric fill failed:', e)
    toast.error(String(e))
  }
}

const fetchedLyrics = ref<LyricLine[]>([])
const isFetchingLyrics = ref(false)

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

// 从封面提取的动态颜色（归一化 RGBA）
function createDefaultExtractedColors(): [number[], number[], number[], number[], number[]] {
  return [
    [0.07, 0.27, 0.42, 1],
    [0.35, 0.24, 0.20, 1],
    [0.34, 0.12, 0.26, 1],
    [0.17, 0.14, 0.34, 1],
    [0.18, 0.34, 0.36, 1],
  ]
}

const extractedColors = ref(createDefaultExtractedColors())
const paletteResult = ref<PaletteResult | null>(null)
const PALETTE_COVER_DECODE_SIZE = 320
let paletteRequestToken = 0

function resetExtractedPalette() {
  paletteResult.value = null
  extractedColors.value = createDefaultExtractedColors()
}

// 使用 Android 同尺寸封面采样，避免小图把细节压成单色
function extractColorsFromCover(url: string): Promise<boolean> {
  const requestToken = ++paletteRequestToken
  return new Promise((resolve) => {
    const img = new Image()
    if (!url.startsWith('data:') && !url.startsWith('blob:')) {
      img.crossOrigin = 'anonymous'
    }
    img.referrerPolicy = 'no-referrer'
    img.onload = () => {
      if (requestToken !== paletteRequestToken) {
        resolve(false)
        return
      }
      try {
        const canvas = document.createElement('canvas')
        const size = PALETTE_COVER_DECODE_SIZE
        canvas.width = size
        canvas.height = size
        const ctx = canvas.getContext('2d', { willReadFrequently: true })
        if (!ctx) throw new Error('Canvas 2D context is unavailable')
        ctx.drawImage(img, 0, 0, size, size)
        const imageData = ctx.getImageData(0, 0, size, size)

        const palette = extractPalette(imageData, 16)
        paletteResult.value = palette
        extractedColors.value = palette.shaderColors.map(
          (c) => [c[0] / 255, c[1] / 255, c[2] / 255, 1]
        ) as [number[], number[], number[], number[], number[]]
        resolve(true)
      } catch (error) {
        if (requestToken === paletteRequestToken) resetExtractedPalette()
        log.error('color extraction failed:', error)
        resolve(false)
      }
    }
    img.onerror = () => {
      if (requestToken === paletteRequestToken) {
        resetExtractedPalette()
        log.error('cover image load failed:', coverSourceLabel(url))
      }
      resolve(false)
    }
    img.src = url
  })
}

// 支持的平台封面统一走后端代理，保证显示与取色读取同一份已验证数据
watch(
  () => [
    player.hasPlaybackSession,
    player.hasPlaybackSession ? getTrackCoverUrl(player.currentTrack) : '',
    player.hasPlaybackSession ? player.currentTrack?.id || '' : '',
  ] as const,
  async ([, rawUrl], _, onCleanup) => {
    let active = true
    onCleanup(() => { active = false })

    paletteRequestToken++
    coverRenderRetryCount = 0
    coverLoadError.value = false

    const proxiedUrl = normalizeProxiedCoverUrl(rawUrl)
    const normalizedUrl = proxiedUrl || normalizeCoverUrlForDisplay(rawUrl)
    log.info('cover resolve begin:', {
      trackId: player.currentTrack?.id,
      rawSource: coverSourceLabel(rawUrl),
      proxied: !!proxiedUrl,
      rawChars: rawUrl.length,
    })
    if (!normalizedUrl) {
      coverUrl.value = ''
      resetExtractedPalette()
      log.warn('cover resolve skipped: no usable URL')
      return
    }

    // 先显示原始地址，代理解析在后台替换，避免详情页首帧出现占位符
    const cachedUrl = proxiedUrl ? peekCoverImage(proxiedUrl) : ''
    coverUrl.value = cachedUrl || normalizedUrl
    log.info('cover fallback displayed:', {
      trackId: player.currentTrack?.id,
      source: coverSourceLabel(coverUrl.value),
      cacheHit: !!cachedUrl,
    })

    let displayUrl = cachedUrl || normalizedUrl
    if (proxiedUrl && !cachedUrl) {
      const proxyStarted = performance.now()
      try {
        displayUrl = await resolveCoverImage(proxiedUrl)
        log.info('cover proxy resolved:', {
          trackId: player.currentTrack?.id,
          elapsedMs: Math.round(performance.now() - proxyStarted),
          dataUrlChars: displayUrl.length,
        })
      } catch (error) {
        log.error('failed to resolve proxied cover:', {
          trackId: player.currentTrack?.id,
          source: coverSourceLabel(proxiedUrl),
          elapsedMs: Math.round(performance.now() - proxyStarted),
          error: summarizeLogError(error),
        })
        if (!active) return
        // 代理封面解析失败时回退到原始 URL 直接显示，而非清空封面
        displayUrl = normalizedUrl
      }
    }

    if (!active) {
      log.info('cover proxy result ignored: stale track')
      return
    }
    // 封面解析成功即显示，取色仅作背景调色板的尽力而为，不再阻塞封面渲染
    // 否则取色失败（canvas 读取异常/token 竞态）会让已解析封面永远显示不出来
    coverUrl.value = displayUrl
    coverLoadError.value = false
    log.info('cover display committed:', {
      trackId: player.currentTrack?.id,
      source: coverSourceLabel(displayUrl),
      chars: displayUrl.length,
    })
    void extractColorsFromCover(displayUrl)
  },
  { immediate: true },
)

function handleNowPlayingCoverLoad(event: Event) {
  if (!player.hasPlaybackSession) return
  const src = (event.currentTarget as HTMLImageElement).src
  coverLoadError.value = false
  log.info('cover img loaded:', {
    trackId: player.currentTrack?.id,
    source: coverSourceLabel(src),
    chars: src.length,
  })
}

async function handleNowPlayingCoverError(event: Event) {
  if (!player.hasPlaybackSession) return
  const image = event.currentTarget as HTMLImageElement
  const failedSrc = image.getAttribute('src') || image.src
  const track = player.currentTrack
  const rawUrl = getTrackCoverUrl(track)
  const proxiedUrl = normalizeProxiedCoverUrl(rawUrl)
  if (
    !track
    || !proxiedUrl
    || failedSrc !== coverUrl.value
    || coverRenderRetryCount >= 1
  ) {
    coverLoadError.value = true
    log.warn('cover img failed:', {
      trackId: track?.id,
      source: coverSourceLabel(failedSrc),
      retry: coverRenderRetryCount,
      hasProxy: !!proxiedUrl,
    })
    return
  }

  coverRenderRetryCount++
  paletteRequestToken++
  resetExtractedPalette()
  const expectedTrackId = track.id
  const expectedRawUrl = rawUrl
  const fallbackUrl = normalizeCoverUrlForDisplay(rawUrl)
  coverLoadError.value = false
  log.warn('cover img failed, refreshing proxy:', {
    trackId: expectedTrackId,
    source: coverSourceLabel(failedSrc),
    fallbackSource: coverSourceLabel(fallbackUrl),
  })

  try {
    const refreshedUrl = await resolveCoverImage(proxiedUrl, { forceRefresh: true })
    if (
      player.currentTrack?.id !== expectedTrackId
      || getTrackCoverUrl(player.currentTrack) !== expectedRawUrl
    ) {
      log.info('cover refresh ignored: stale track')
      return
    }

    // 重新解析成功即恢复封面显示，取色失败不应再次把封面隐藏
    coverUrl.value = refreshedUrl
    coverLoadError.value = false
    log.info('cover refresh committed:', {
      trackId: expectedTrackId,
      dataUrlChars: refreshedUrl.length,
    })
    void extractColorsFromCover(refreshedUrl)
  } catch (error) {
    log.error('failed to refresh proxied cover:', summarizeLogError(error))
    if (fallbackUrl) {
      coverUrl.value = fallbackUrl
      coverLoadError.value = false
      log.info('cover refresh fallback committed:', {
        trackId: expectedTrackId,
        source: coverSourceLabel(fallbackUrl),
      })
    } else {
      coverLoadError.value = true
    }
  }
}

function onSeek(progress: number) {
  player.seekTo(Math.round(progress * player.durationMs))
}

function onLyricSeek(ms: number) {
  player.seekTo(ms)
}

const isFavorite = computed(() => likedSongs.isTrackLiked(player.currentTrack))
const canToggleFavorite = computed(() => {
  const track = player.currentTrack
  return !!track && (!track.id.startsWith('netease:') || auth.canMutateNetease)
})

async function toggleFavorite() {
  if (!canToggleFavorite.value) return
  await likedSongs.toggleTrack(player.currentTrack, {
    neteaseAuthorized: auth.canMutateNetease,
  })
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

function formatSleepRemaining(seconds: number): string {
  if (seconds <= 0) return ''
  const m = Math.floor(seconds / 60)
  const s = seconds % 60
  return m > 0 ? `${m}:${s.toString().padStart(2, '0')}` : `${s}s`
}

const nowPlayingTrackKey = computed(() => (
  playbackSessionTrackKey(
    player.hasPlaybackSession,
    player.currentTrack?.playlistKey,
    player.currentTrack?.id,
  )
))
const transitionStateClass = computed(() => props.transitionState ? `np-shell--${props.transitionState}` : '')
const nowPlayingTimeKey = computed(() => `time:${nowPlayingTrackKey.value}`)
const nowPlayingAudioInfoKey = computed(() => [
  nowPlayingTrackKey.value,
  player.isPlayingFromDownload ? 'download' : 'stream',
  audioInfoDisplay.value || 'none',
].join(':'))
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
  else showAddToPlaylist.value = true
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
  const targetEl = settings.coverStyle === 'card'
    ? cardCoverRef.value
    : discRef.value
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

let lyricFetchRequestId = 0

// 当曲目切换时自动获取歌词
watch(nowPlayingTrackKey, async (trackKey) => {
  const requestId = ++lyricFetchRequestId
  const track = player.currentTrack
  if (trackKey === 'empty' || !track) {
    fetchedLyrics.value = []
    isFetchingLyrics.value = false
    log.info('lyrics cleared: no playback track')
    return
  }

  const started = performance.now()
  const cachedLyrics = readCachedLyrics(track)
  const reusedRequest = hasLyricsRequestInFlight(track)
  fetchedLyrics.value = cachedLyrics || []
  isFetchingLyrics.value = true
  log.info('lyrics load begin:', {
    requestId,
    trackId: track.id,
    cachedLines: cachedLyrics?.length || 0,
    reusedRequest,
  })
  try {
    // 本地 cache 优先; 有缓存则不再触网/回写云端
    if (cachedLyrics?.length) {
      log.info('lyrics from local cache:', {
        requestId,
        trackId: track.id,
        lines: cachedLyrics.length,
      })
      return
    }

    // 同步歌词优先落地为本地歌词(不触网, 不回写云端)
    // present -> 使用; cleared -> 空词并阻止在线回填; absent -> 继续在线
    const syncedLyrics = await materializeSyncedLyrics(track)
    if (requestId !== lyricFetchRequestId) return
    if (syncedLyrics !== null) {
      fetchedLyrics.value = syncedLyrics
      if (syncedLyrics.length > 0) {
        cacheLyricsForTrack(track, syncedLyrics)
      }
      log.info('lyrics from sync payload:', {
        requestId,
        trackId: track.id,
        lines: syncedLyrics.length,
        cleared: syncedLyrics.length === 0,
      })
      return
    }

    const neteaseId = track.id.startsWith('netease:')
      ? parseInt(track.id.replace('netease:', ''))
      : undefined
    const qqSongMid = track.id.startsWith('qq:')
      ? track.id.replace('qq:', '')
      : undefined
    const youtubeVideoId = track.id.startsWith('youtube:')
      ? track.id.replace('youtube:', '')
      : undefined

    const nextLyrics = await loadLyricsSingleFlight(track, async () => {
      const invokeStarted = performance.now()
      log.info('lyrics backend invoke:', { requestId, trackId: track.id })
      const lyrics = await invoke<any[]>('fetch_lyrics', {
        title: track.title,
        artist: track.artist,
        durationSecs: Math.floor((track.durationMs || player.durationMs || 0) / 1000),
        audioPath: track.audioUrl || null,
        neteaseId: neteaseId || null,
        qqSongMid: qqSongMid || null,
        youtubeVideoId: youtubeVideoId || null,
      })
      const mapped = mapBackendLyrics(lyrics)
      if (mapped.length > 0) cacheLyricsForTrack(track, mapped)
      log.info('lyrics backend returned:', {
        requestId,
        trackId: track.id,
        lines: mapped.length,
        elapsedMs: Math.round(performance.now() - invokeStarted),
      })
      return mapped
    })

    if (requestId !== lyricFetchRequestId) {
      log.info('lyrics result ignored: stale request', {
        requestId,
        activeRequestId: lyricFetchRequestId,
        trackId: track.id,
        lines: nextLyrics.length,
      })
      return
    }
    fetchedLyrics.value = nextLyrics.length > 0 ? nextLyrics : []
    log.info('lyrics load committed:', {
      requestId,
      trackId: track.id,
      lines: fetchedLyrics.value.length,
      elapsedMs: Math.round(performance.now() - started),
    })
  } catch (e) {
    log.error('Fetch lyrics failed:', {
      requestId,
      trackId: track.id,
      elapsedMs: Math.round(performance.now() - started),
      error: summarizeLogError(e),
    })
    if (requestId === lyricFetchRequestId) {
      fetchedLyrics.value = readCachedLyrics(track) || cachedLyrics || []
      log.info('lyrics cache restored after failure:', {
        requestId,
        trackId: track.id,
        lines: fetchedLyrics.value.length,
      })
    }
  } finally {
    if (requestId === lyricFetchRequestId) {
      isFetchingLyrics.value = false
      log.info('lyrics load finished:', {
        requestId,
        trackId: track.id,
        elapsedMs: Math.round(performance.now() - started),
      })
    }
  }
}, { immediate: true })

// 唱片旋转（JS 驱动，停止时保持角度 + 缓动）
const discRef = ref<HTMLDivElement>()
let discAngle = 0            // 当前累计角度（度）
let discAnimFrame = 0
let discLastTime = 0
const DISC_RPM = 2.4         // 每秒转过的度数 = 360 / 25s ≈ 14.4 deg/s
const DEG_PER_MS = 360 / 25000

function animateDisc(timestamp: number) {
  if (!discLastTime) discLastTime = timestamp
  const dt = timestamp - discLastTime
  discLastTime = timestamp

  if (player.isPlaying) {
    discAngle = (discAngle + DEG_PER_MS * dt) % 360
  }
  if (discRef.value) {
    discRef.value.style.transform = `rotate(${discAngle}deg)`
  }
  discAnimFrame = requestAnimationFrame(animateDisc)
}

onMounted(() => {
  discAnimFrame = requestAnimationFrame(animateDisc)
})
onUnmounted(() => {
  cancelAnimationFrame(discAnimFrame)
  if (trackSwitchAnimTimer) clearTimeout(trackSwitchAnimTimer)
  if (controlFeedbackPulseTimer) clearTimeout(controlFeedbackPulseTimer)
  if (moreSheetSwitchTimer) clearTimeout(moreSheetSwitchTimer)
  closeToolbarPopovers()
})

defineExpose({
  toggleMore() {
    if (player.hasPlaybackSession) showMoreSheet.value = !showMoreSheet.value
  },
  getCoverSnapshot,
})

// 右键菜单（歌曲名/歌手复制 + 封面保存）
const contextMenu = ref({ show: false, x: 0, y: 0, type: '' as 'title' | 'artist' | 'cover' })

watch(() => player.hasPlaybackSession, (hasSession) => {
  if (hasSession) return
  playViewMode.value = 'cover'
  closeToolbarPopovers()
  showQueue.value = false
  showMoreSheet.value = false
  showLtPanel.value = false
  contextMenu.value.show = false
})

const contextMenuItems = computed(() => {
  if (contextMenu.value.type === 'title') {
    return [{ id: 'copy-title', label: t('player.copy_title'), icon: 'content_copy' }]
  }
  if (contextMenu.value.type === 'artist') {
    return [{ id: 'copy-artist', label: t('player.copy_artist'), icon: 'content_copy' }]
  }
  return [{ id: 'save-cover', label: t('player.save_cover'), icon: 'save' }]
})

function openContextMenu(e: MouseEvent, type: 'title' | 'artist' | 'cover') {
  e.preventDefault()
  contextMenu.value = { show: true, x: e.clientX, y: e.clientY, type }
}

function closeContextMenu() {
  contextMenu.value.show = false
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text)
    toast.success(t('player.copied'))
  } catch {
    toast.error(t('player.copy_failed'))
  }
  closeContextMenu()
}

function handleContextMenuClick(item: ContextMenuActionItem) {
  if (item.id === 'copy-title') {
    void copyText(player.currentTrack?.title || '')
  } else if (item.id === 'copy-artist') {
    void copyText(player.currentTrack?.artist || '')
  } else if (item.id === 'save-cover') {
    void saveCoverArt()
  }
}

async function saveCoverArt() {
  closeContextMenu()
  const url = player.currentTrack?.coverUrl
  if (!url) return

  try {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const filePath = await save({
      defaultPath: `${player.currentTrack?.title || 'cover'}.jpg`,
      filters: [{ name: 'Image', extensions: ['jpg', 'png', 'webp'] }],
    })
    if (!filePath) return

    const response = await fetch(url, { referrerPolicy: 'no-referrer' })
    const blob = await response.blob()
    const arrayBuffer = await blob.arrayBuffer()

    await invoke('save_file_bytes', {
      path: filePath,
      data: Array.from(new Uint8Array(arrayBuffer)),
    })
    toast.success(t('player.cover_saved'))
  } catch (e) {
    log.error('Save cover failed:', e)
    toast.error(t('player.cover_save_failed'))
  }
}

const displayLyrics = computed(() => {
  if (player.lyrics.length) return player.lyrics
  if (fetchedLyrics.value.length) return fetchedLyrics.value
  return []
})

const lyricsModeAvailable = computed(
  () => displayLyrics.value.length > 0 || isFetchingLyrics.value,
)

function requestPlayViewMode(requestedMode: NowPlayingViewMode) {
  playViewMode.value = resolveNowPlayingViewMode(
    playViewMode.value,
    requestedMode,
    displayLyrics.value.length > 0,
    isFetchingLyrics.value,
  )
}

watch(lyricsModeAvailable, (available) => {
  if (!available && playViewMode.value === 'lyrics') {
    requestPlayViewMode('lyrics')
  }
})

// 更多选项面板子视图
const moreSheetView = ref<
  'main' | 'offset' | 'fontsize' | 'speed' | 'search' | 'editinfo' |
  'quality' | 'lyrics-editor' | 'lyrics-fill' | 'track-detail'
>('main')
const moreSheetTransition = ref('slide-left')

function goToSubView(view: typeof moreSheetView.value) {
  moreSheetTransition.value = 'slide-left'
  moreSheetView.value = view
}
function goBackToMain() {
  moreSheetTransition.value = 'slide-right'
  moreSheetView.value = 'main'
}

// 关闭更多选项面板时重置子视图
watch(showMoreSheet, (v) => { if (!v) setTimeout(() => { moreSheetView.value = 'main' }, 220) })

// 获取歌曲信息（搜索）
const searchQuery = ref('')
const searchResults = ref<any[]>([])
const isSearching = ref(false)
const infoSearchPlatform = ref<'netease' | 'bilibili' | 'youtube' | 'qq'>('netease')
const infoApplyCandidate = ref<any | null>(null)
const applyInfoFields = ref({
  title: true,
  artist: true,
  cover: true,
  lyrics: false,
})

function openInfoSearch() {
  searchQuery.value = player.currentTrack?.title || ''
  searchResults.value = []
  infoApplyCandidate.value = null
  applyInfoFields.value = { title: true, artist: true, cover: true, lyrics: false }
  goToSubView('search')
}

async function doSearch() {
  const q = searchQuery.value.trim()
  if (!q) return
  isSearching.value = true
  searchResults.value = []
  infoApplyCandidate.value = null
  try {
    const results = await invoke<any[]>('search', {
      query: q,
      platform: infoSearchPlatform.value,
      includeLyrics: infoSearchPlatform.value === 'qq',
    })
    searchResults.value = results
  } catch (e) {
    log.error('Search failed:', e)
  } finally {
    isSearching.value = false
  }
}

function applySearchResult(result: any) {
  infoApplyCandidate.value = result
  applyInfoFields.value = {
    title: !!result.title,
    artist: !!result.artist,
    cover: !!(result.cover_url || result.coverUrl),
    lyrics: false,
  }
}

async function confirmApplySearchResult() {
  const result = infoApplyCandidate.value
  if (!result) return
  const patch: Record<string, string> = {}
  if (applyInfoFields.value.title) patch.title = result.title || player.currentTrack?.title || ''
  if (applyInfoFields.value.artist) patch.artist = result.artist || player.currentTrack?.artist || ''
  if (applyInfoFields.value.cover) patch.coverUrl = result.cover_url || result.coverUrl || player.currentTrack?.coverUrl || ''
  if (Object.keys(patch).length > 0) player.updateCurrentTrackInfo(patch)
  if (applyInfoFields.value.lyrics) {
    try {
      const direct = await parseLyricsFromSearchResult(result)
      if (direct && direct.lines.length > 0) {
        fetchedLyrics.value = direct.lines
        cacheLyricsForTrack(player.currentTrack, direct.lines)
        const source = String(result?.platform || 'LOCAL_EDIT').toUpperCase()
        await commitLyricsToTrack(direct.rawLyric, direct.rawTranslated, source)
      } else {
        const idText = String(result.id || '')
        const neteaseId = idText.startsWith('netease:') ? parseInt(idText.replace('netease:', '')) : null
        const qqSongMid = idText.startsWith('qq:') ? idText.replace('qq:', '') : null
        const lyrics = await invoke<any[]>('fetch_lyrics', {
          title: result.title || player.currentTrack?.title || '',
          artist: result.artist || player.currentTrack?.artist || '',
          durationSecs: Math.floor((result.duration_ms || player.currentTrack?.durationMs || 0) / 1000),
          audioPath: null,
          neteaseId,
          qqSongMid,
          youtubeVideoId: null,
        })
        if (lyrics.length) {
          const nextLyrics = mapBackendLyrics(lyrics)
          fetchedLyrics.value = nextLyrics
          cacheLyricsForTrack(player.currentTrack, nextLyrics)
        }
      }
    } catch (e) {
      log.warn('Apply info lyrics failed:', e)
    }
  }
  toast.success(t('player.info_applied'))
  infoApplyCandidate.value = null
  goBackToMain()
}

// 编辑歌曲信息
const editTitle = ref('')
const editArtist = ref('')
const editCoverUrl = ref('')

function openEditInfo() {
  editTitle.value = player.currentTrack?.title || ''
  editArtist.value = player.currentTrack?.artist || ''
  editCoverUrl.value = player.currentTrack?.coverUrl || ''
  goToSubView('editinfo')
}

function saveEditInfo() {
  player.updateCurrentTrackInfo({
    title: editTitle.value,
    artist: editArtist.value,
    coverUrl: editCoverUrl.value,
  })
  toast.success(t('player.info_applied'))
  goBackToMain()
}

function restoreInfo() {
  player.restoreOriginalTrackInfo()
  toast.success(t('player.info_restored'))
  goBackToMain()
}

// 音质切换
const currentSource = computed(() => {
  const id = player.currentTrack?.id || ''
  if (id.startsWith('netease:')) return 'netease'
  if (id.startsWith('qq:')) return 'qq'
  if (id.startsWith('bilibili:')) return 'bilibili'
  if (id.startsWith('youtube:')) return 'youtube'
  return 'local'
})

// 偏移分桶: netease→cloud, qq→qq, youtube/bili/local→none(默认 0)
const currentOffsetBucket = computed(() => offsetBucketForSource(currentSource.value))

// 逐曲用户偏移(delta, 默认 0) -- 偏移面板编辑的就是它, 不再混入系统默认
const currentLyricUserOffsetMs = computed<number>({
  get: () => lyricOffsetStore.getUserOffsetMs(player.currentTrack),
  set: (value: number) => lyricOffsetStore.setUserOffsetMs(player.currentTrack, value),
})

// 系统全局默认(基线), 只读展示; youtube 等 none 桶恒为 0
const currentLyricDefaultOffsetMs = computed(() =>
  lyricOffsetStore.defaultOffsetMs(currentOffsetBucket.value),
)

// 有效偏移 = 基线 + delta, 喂给歌词渲染
const currentLyricTotalOffsetMs = computed(
  () => currentLyricDefaultOffsetMs.value + currentLyricUserOffsetMs.value,
)

// 偏移面板副标题: 标明当前生效的系统默认来源
const currentLyricOffsetSourceLabel = computed(() => {
  switch (currentOffsetBucket.value) {
    case 'qq':
      return t('player.source_qq')
    case 'cloud':
      return t('player.source_netease')
    default:
      return t(`player.source_${currentSource.value}` as 'player.source_youtube')
  }
})

const currentTrackId = computed(() => player.currentTrack?.id || '')
const currentNeteaseSongNumericId = computed(() => {
  const id = currentTrackId.value
  if (!id.startsWith('netease:')) return null
  const num = Number(id.replace('netease:', ''))
  return Number.isFinite(num) && num > 0 ? num : null
})
const currentDownloadTask = computed(() => currentTrackId.value ? downloadStore.downloading.get(currentTrackId.value) : undefined)
const currentDownloadedTrack = computed(() => currentTrackId.value ? downloadStore.getDownloadedTrack(currentTrackId.value) : undefined)
const isCurrentDownloaded = computed(() => currentTrackId.value ? downloadStore.isDownloaded(currentTrackId.value) : false)
const isCurrentDownloading = computed(() => !!currentDownloadTask.value)
const isCurrentDownloadCancellable = computed(() => {
  const status = currentDownloadTask.value?.status
  return status === 'resolving' || status === 'downloading'
})

function formatDurationMs(ms?: number) {
  if (!ms || ms <= 0) return '-'
  const totalSeconds = Math.floor(ms / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}:${String(seconds).padStart(2, '0')}`
}

function formatFileSize(bytes?: number) {
  if (!bytes || bytes <= 0) return '-'
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex += 1
  }
  return `${value.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`
}

// 歌曲详情专用: 码率/编解码/采样 等完整音频参数 (含 kbps, 与进度条纸面规格分离)
const trackDetailAudioParams = computed(() => {
  const info = player.audioInfo
  if (!info) return ''
  const parts: string[] = []
  const quality = currentAudioQualityLabel()
  if (quality) parts.push(quality)
  if (info.codec) {
    const codec = normalizeAudioDisplayToken(info.codec)
    if (codec && !parts.includes(codec)) parts.push(codec)
  }
  if (info.format) {
    const format = normalizeAudioDisplayToken(info.format)
    if (format && !parts.includes(format) && format.toLowerCase() !== (info.codec || '').toLowerCase()) {
      parts.push(format)
    }
  }
  if (info.bitrate && info.bitrate > 0) parts.push(`${Math.round(info.bitrate)} kbps`)
  for (const token of paperSpecFromAudioInfo(info)) {
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

function isRemotePlaybackSource(source: string) {
  return source === 'netease' || source === 'qq' || source === 'bilibili' || source === 'youtube'
}

function downloadTaskStatusText(status?: string) {
  switch (status) {
    case 'resolving': return t('download.resolving')
    case 'downloading': return t('download.downloading')
    case 'cancelling': return t('download.cancelling')
    case 'cancelled': return t('download.cancelled')
    case 'error': return t('download.download_failed')
    case 'already_exists': return t('download.already_exists')
    default: return ''
  }
}

const downloadActionIcon = computed(() => {
  if (player.isPlayingFromDownload) return 'download_done'
  if (isCurrentDownloading.value) {
    const status = currentDownloadTask.value?.status
    if (status === 'error') return 'error'
    if (status === 'cancelled') return 'cancel'
    if (isCurrentDownloadCancellable.value) return 'cancel'
    return 'downloading'
  }
  if (isCurrentDownloaded.value) return 'refresh'
  return 'download'
})

const downloadActionLabel = computed(() => {
  if (player.isPlayingFromDownload) return t('player.playing_from_download')
  if (isCurrentDownloadCancellable.value) return '取消下载'
  if (isCurrentDownloading.value) return downloadTaskStatusText(currentDownloadTask.value?.status) || t('download.downloading')
  if (isCurrentDownloaded.value) return t('download.redownload')
  return t('library.tab_downloads')
})

const downloadActionDesc = computed(() => {
  if (player.isPlayingFromDownload) return t('download.using_local_file')
  if (isCurrentDownloading.value) {
    const task = currentDownloadTask.value
    if (!task) return ''
    if (task.message) return task.message
    if (typeof task.progress === 'number') return `${task.progress}%`
    return ''
  }
  if (isCurrentDownloaded.value) return t('download.redownload_desc')
  return t('download.download_desc')
})

const downloadActionDisabled = computed(() =>
  !player.currentTrack
  || player.isPlayingFromDownload
  || (isCurrentDownloading.value && !isCurrentDownloadCancellable.value && currentDownloadTask.value?.status !== 'error' && currentDownloadTask.value?.status !== 'cancelled')
)

const neteaseQualities = [
  { key: 'standard', label: 'settings.q_standard' },
  { key: 'higher', label: 'settings.q_high' },
  { key: 'exhigh', label: 'settings.q_exhigh' },
  { key: 'lossless', label: 'settings.q_lossless' },
  { key: 'hires', label: 'settings.q_hires' },
  { key: 'jyeffect', label: 'settings.q_surround' },
  { key: 'sky', label: 'settings.q_sky' },
  { key: 'jymaster', label: 'settings.q_master' },
]

const qqQualities = [
  { key: 'standard', label: 'settings.q_standard' },
  { key: 'high', label: 'settings.q_high_yt' },
  { key: 'lossless', label: 'settings.q_lossless' },
]

const youtubeQualities = [
  { key: 'low', label: 'settings.q_low' },
  { key: 'medium', label: 'settings.q_medium' },
  { key: 'high', label: 'settings.q_high_yt' },
  { key: 'very_high', label: 'settings.q_very_high' },
]

const biliQualities = [
  { key: 'low', label: 'settings.q_smooth' },
  { key: 'medium', label: 'settings.q_standard' },
  { key: 'high', label: 'settings.q_good' },
  { key: 'lossless', label: 'settings.q_lossless' },
  { key: 'hires', label: 'settings.q_hires' },
  { key: 'dolby', label: 'settings.q_dolby' },
]

const isQualitySwitching = ref(false)

async function switchQuality(key: string) {
  if (isQualitySwitching.value) return
  const source = currentSource.value
  const previousKey = currentQualityKey(source)
  if (!previousKey || previousKey === key) {
    showMoreSheet.value = false
    return
  }

  isQualitySwitching.value = true
  setQualityKey(source, key)
  try {
    showMoreSheet.value = false
    await player.replayWithQuality()
  } catch (e) {
    setQualityKey(source, previousKey)
    toast.error(String(e))
  } finally {
    isQualitySwitching.value = false
  }
}

function currentQualityKey(source = currentSource.value): string {
  if (source === 'netease') return settings.neteaseQuality
  if (source === 'qq') return settings.qqMusicQuality
  if (source === 'bilibili') return settings.biliQuality
  if (source === 'youtube') return settings.youtubeQuality
  return ''
}

function setQualityKey(source: string, key: string) {
  if (source === 'netease') settings.neteaseQuality = key
  else if (source === 'qq') settings.qqMusicQuality = key
  else if (source === 'bilibili') settings.biliQuality = key
  else if (source === 'youtube') settings.youtubeQuality = key
}

function qualityOptionsForSource(source: string) {
  if (source === 'netease') return neteaseQualities
  if (source === 'qq') return qqQualities
  if (source === 'bilibili') return biliQualities
  if (source === 'youtube') return youtubeQualities
  return []
}

function qualityLabelFor(source: string, key?: string) {
  if (!key || source === 'local') return ''
  const item = qualityOptionsForSource(source).find(q => q.key === key)
  return item ? t(item.label) : key
}

// 下载/重新下载歌曲
async function handleDownloadAction() {
  const track = player.currentTrack
  if (!track) return
  showMoreSheet.value = false
  if (isCurrentDownloadCancellable.value) {
    await downloadStore.cancelDownload(track.id)
    return
  }
  if (isCurrentDownloaded.value && !player.isPlayingFromDownload) {
    player.handleDownloadedFileRemoved(track.id, downloadStore.getDownloadedTrack(track.id)?.filePath)
    await downloadStore.redownloadTrack(track)
  } else {
    await downloadStore.downloadTrack(track)
  }
}

async function openCurrentAlbum() {
  const songId = currentNeteaseSongNumericId.value
  if (!songId) return
  try {
    const detail = await invoke<any>('get_netease_song_detail', { songId })
    const song = detail?.songs?.[0]
    const albumId = song?.al?.id || song?.album?.id
    if (!albumId) {
      toast.error(t('player.not_available'))
      return
    }
    hideMoreSheet()
    // 先收起正在播放页，再跳转专辑，避免详情盖在 NP 下面
    emit('collapse')
    await router.push({ name: 'netease-album', params: { id: String(albumId) } })
  } catch (e) {
    log.error('Open album failed:', e)
    toast.error(String(e))
  }
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
    const bid = track.id.replace('bilibili:', '')
    url = `https://www.bilibili.com/video/${bid}`
  } else if (track.id.startsWith('youtube:')) {
    const vid = track.id.replace('youtube:', '')
    url = `https://music.youtube.com/watch?v=${vid}`
  }
  const text = url
    ? `${track.title} - ${track.artist}\n${url}`
    : `${track.title} - ${track.artist}`
  try {
    await navigator.clipboard.writeText(text)
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
const canViewNeteaseAlbum = computed(() => currentSource.value === 'netease' && !!albumName.value && !!currentNeteaseSongNumericId.value)

// 进度条下方音质信息（不展示 Local / download 占位）
// 纸面规格: 最高/极高/杜比… + 可选编解码; 不展示 kbps 数字
const audioInfoParts = computed(() => {
  const info = player.audioInfo
  if (!info) return []
  const parts: Array<{ text: string; accent?: boolean }> = []
  if (settings.showQualitySwitch) addAudioInfoPart(parts, currentAudioQualityLabel(), true)
  if (settings.showAudioCodec) addAudioInfoPart(parts, normalizeAudioDisplayToken(info.codec))
  // showAudioSpec: 只补 sampleRate/bitDepth 类纸面规格, 不写 kbps
  if (settings.showAudioSpec) {
    for (const token of paperSpecFromAudioInfo(info)) addAudioInfoPart(parts, token)
  }
  return parts.filter(part => !isHiddenAudioInfoToken(part.text))
})

const audioInfoDisplay = computed(() => {
  return audioInfoParts.value.map(part => part.text).join(' · ')
})

function currentAudioQualityLabel() {
  const info = player.audioInfo
  const source = info?.source && info.source !== 'local' ? info.source : currentSource.value
  // 优先已本地化的 qualityLabel; 否则用 qualityKey 映射到 标准/极高/最高…
  const labeled = info?.qualityLabel?.trim()
  if (labeled && !/kbps/i.test(labeled) && labeled !== info?.qualityKey) {
    return labeled
  }
  return qualityLabelFor(source, info?.qualityKey || currentQualityKey(source))
}

/** 从 audioInfo 抽出非码率的纸面规格 (如 48 kHz / 16 bit) */
function paperSpecFromAudioInfo(info: {
  sampleRateHz?: number
  bitDepth?: number
  specLabel?: string
}): string[] {
  const tokens: string[] = []
  if (info.sampleRateHz && info.sampleRateHz > 0) {
    const khz = info.sampleRateHz / 1000
    tokens.push(info.sampleRateHz % 1000 === 0
      ? `${khz.toFixed(0)} kHz`
      : `${khz.toFixed(1)} kHz`)
  }
  if (info.bitDepth && info.bitDepth > 0) tokens.push(`${info.bitDepth} bit`)
  // specLabel 里可能混有 kbps, 过滤掉
  if (info.specLabel) {
    for (const part of info.specLabel.split('|').map(s => s.trim())) {
      if (!part || /kbps/i.test(part)) continue
      if (!tokens.includes(part)) tokens.push(part)
    }
  }
  return tokens
}

function addAudioInfoPart(
  parts: Array<{ text: string; accent?: boolean }>,
  value?: string,
  accent = false,
) {
  const normalized = value?.trim()
  if (!normalized) return
  if (parts.some(part => isSameAudioInfoToken(part.text, normalized))) return
  parts.push({ text: normalized, accent })
}

function normalizeAudioDisplayToken(value?: string) {
  if (!value) return ''
  const raw = value.trim()
  const lower = raw.toLowerCase()
  // 占位词直接丢掉
  if (isHiddenAudioInfoToken(raw)) return ''
  const tokenMap: Record<string, string> = {
    flac: 'FLAC',
    mp3: 'MP3',
    mpeg: 'MP3',
    aac: 'AAC',
    mp4a: 'AAC',
    m4a: 'M4A',
    opus: 'Opus',
    ogg: 'OGG',
    vorbis: 'Vorbis',
    wav: 'WAV',
    aiff: 'AIFF',
    'ec-3': 'EC-3',
    ac3: 'AC-3',
  }
  return tokenMap[lower] ?? tokenMap[lower.split('.')[0]] ?? raw
}

function isHiddenAudioInfoToken(value?: string) {
  if (!value) return true
  const lower = value.trim().toLowerCase()
  return lower === 'local'
    || lower === 'download'
    || lower === 'file'
    || lower === 'offline'
    || lower === 'downloaded'
}

function isSameAudioInfoToken(left: string, right: string) {
  return left.trim().toLowerCase() === right.trim().toLowerCase()
}

// AccentBackdrop 底色（对齐 Android：主色降饱和调暗后铺底）
// 强制压暗，保证白字在亮封面/浅主题下仍可读
const accentBgStyle = computed(() => {
  if (!player.hasPlaybackSession) return { background: 'rgb(18, 18, 18)' }
  const bg = paletteResult.value?.accentBg
  if (!bg) return { background: 'rgb(18, 18, 18)' }
  const [r, g, b] = bg
  const luma = (r * 0.299 + g * 0.587 + b * 0.114) / 255
  if (luma <= 0.34) {
    return { background: `rgb(${r}, ${g}, ${b})` }
  }
  // 过亮时向中性深色混合，保留色相
  const t = Math.min(1, (luma - 0.34) / 0.4)
  const mix = 0.35 + t * 0.45
  return {
    background: `rgb(${Math.round(r * (1 - mix) + 18 * mix)}, ${Math.round(g * (1 - mix) + 18 * mix)}, ${Math.round(b * (1 - mix) + 18 * mix)})`,
  }
})
const shouldRenderDynamicBackground = computed(() => shouldShowDynamicBackground(
  player.hasPlaybackSession,
  settings.dynamicBackground,
  paletteResult.value,
))

// 动态主题 CSS 变量（对齐 Android M3 动态配色）
const dynamicColorVars = computed(() => {
  if (!player.hasPlaybackSession) return {}
  const p = paletteResult.value
  if (!p) return {}
  const lv = p.lightVibrant

  // RGB -> HSL 转换
  const r = lv[0] / 255, g = lv[1] / 255, b = lv[2] / 255
  const max = Math.max(r, g, b), min = Math.min(r, g, b)
  let h = 0, s = 0
  const l = (max + min) / 2
  if (max !== min) {
    const d = max - min
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min)
    if (max === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6
    else if (max === g) h = ((b - r) / d + 2) / 6
    else h = ((r - g) / d + 4) / 6
  }

  // HSL -> RGB
  const hsl2rgb = (h: number, s: number, l: number): [number, number, number] => {
    if (s === 0) return [Math.round(l * 255), Math.round(l * 255), Math.round(l * 255)]
    const hue2rgb = (p: number, q: number, t: number) => {
      if (t < 0) t += 1; if (t > 1) t -= 1
      if (t < 1/6) return p + (q - p) * 6 * t
      if (t < 1/2) return q
      if (t < 2/3) return p + (q - p) * (2/3 - t) * 6
      return p
    }
    const q = l < 0.5 ? l * (1 + s) : l + s - l * s
    const pp = 2 * l - q
    return [
      Math.round(hue2rgb(pp, q, h + 1/3) * 255),
      Math.round(hue2rgb(pp, q, h) * 255),
      Math.round(hue2rgb(pp, q, h - 1/3) * 255),
    ]
  }

  // 主色：提升饱和度和亮度确保在暗背景上的可见性（对齐 Android M3 primary）
  const primaryS = Math.min(1, s * 1.2 + 0.15) // 保底饱和度
  const primaryL = Math.max(0.55, Math.min(0.75, l * 0.8 + 0.35)) // 亮度 55~75% 确保对比
  const [pr, pg, pb] = hsl2rgb(h, primaryS, primaryL)
  const primary = `rgb(${pr}, ${pg}, ${pb})`

  // 主色容器：更亮、低饱和度（对齐 Android primaryContainer）
  const pcS = Math.min(1, s * 0.8 + 0.1)
  const pcL = Math.max(0.70, Math.min(0.85, primaryL + 0.15))
  const [pcr, pcg, pcb] = hsl2rgb(h, pcS, pcL)
  const primaryContainer = `rgb(${pcr}, ${pcg}, ${pcb})`

  // 主色上文字：基于 primaryContainer 亮度选深/浅色
  const pcLuma = pcr * 0.299 + pcg * 0.587 + pcb * 0.114
  const onPrimary = pcLuma > 140 ? 'rgb(20, 18, 24)' : 'rgb(255, 255, 255)'

  return {
    '--np-primary': primary,
    '--np-on-primary': onPrimary,
    '--np-primary-container': primaryContainer,
    '--np-on-surface': 'rgba(255,255,255,1)',
    '--np-on-surface-variant': 'rgba(255,255,255,0.78)',
    '--waveform-thumb-color': primary,
  }
})

// 进度条活跃色（与 --np-primary 同步）
const sliderActiveColor = computed(() => {
  const vars = dynamicColorVars.value
  return (vars as any)['--np-primary'] || '#fff'
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
      },
    ]"
    :style="dynamicColorVars"
    @click="closeToolbarPopovers()"
  >
    <!-- AccentBackdrop 底色层（对齐 Android：主色降饱和+调暗） -->
    <div class="np-bg-solid" :style="accentBgStyle" />
    <!-- 动态背景：封面模糊 OR WebGL 着色器，互斥 -->
    <CoverBlurBackground
      v-if="player.hasPlaybackSession && settings.coverBlurBg"
      :cover-url="coverUrl"
      :blur-amount="settings.coverBlurAmount * 30"
      :darken-alpha="Math.max(settings.coverBlurDarken, 0.42)"
    />
    <HyperBackground
      v-else-if="shouldRenderDynamicBackground"
      :music-level="settings.audioReactive ? player.audioLevel : 0"
      :beat-impulse="settings.audioReactive ? player.beatImpulse : 0"
      :colors="extractedColors"
      :is-dark="true"
      :light-offset="paletteResult?.lightOffset ?? 0"
      :saturate-offset="paletteResult?.saturateOffset ?? 0"
    />
    <div class="np-scrim" />
    <div class="np-ambient-glow" />

    <!-- 顶栏（融合模式下由 TitleBar 接管） -->
    <header v-if="!props.hideHeader" class="np-header">
      <button class="np-icon-btn" @click.stop="emit('collapse')">
        <span class="material-symbols-rounded">keyboard_arrow_down</span>
      </button>
      <div class="np-header-center">
        <span class="np-from-label">
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
      <section
        class="np-left"
        :inert="playViewMode === 'lyrics'"
        :aria-hidden="playViewMode === 'lyrics'"
      >
        <div class="np-left-stack">
        <div
          class="cover-wrap"
          :class="{
            'cover-wrap--card': settings.coverStyle === 'card',
            'cover-wrap--disc': settings.coverStyle !== 'card',
            'cover-wrap--switching': isTrackSwitchAnimating,
            'cover-wrap--lyrics-entry': lyricsModeAvailable,
          }"
          :role="lyricsModeAvailable ? 'button' : undefined"
          :tabindex="lyricsModeAvailable ? 0 : -1"
          :aria-label="lyricsModeAvailable ? t('player.view_mode_lyrics') : undefined"
          :title="lyricsModeAvailable ? t('player.view_mode_lyrics') : undefined"
          @click="requestPlayViewMode('lyrics')"
          @keydown.enter.prevent="requestPlayViewMode('lyrics')"
          @keydown.space.prevent="requestPlayViewMode('lyrics')"
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
          <div v-else ref="discRef" class="cover-disc">
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
            <div class="cover-groove" />
            <div class="cover-hole" />
          </div>
          <!-- 来源徽章（对齐 Android PlaybackSourceBadge） -->
          <transition name="np-badge-swap" mode="out-in">
            <div v-if="showSourceBadge && settings.coverStyle === 'card'" :key="sourceBadgeKey" class="source-badge">
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
              <p class="np-artist" @contextmenu="openContextMenu($event, 'artist')">{{ player.currentTrack?.artist || '' }}</p>
            </div>
          </transition>
        </div>

        <div class="np-slider-area">
          <WaveformSlider
            :progress="player.interpolatedProgress"
            :is-playing="player.isPlaying"
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
          <div class="np-audio-info" :key="nowPlayingAudioInfoKey">
            <span v-if="player.isPlayingFromDownload" class="np-download-chip">
              <span class="material-symbols-rounded">download_done</span>
              {{ t('player.playing_from_download') }}
            </span>
            <span v-if="audioInfoParts.length" class="np-audio-detail" :class="{ separated: player.isPlayingFromDownload }">
              <template v-for="(part, index) in audioInfoParts" :key="`${part.text}:${index}`">
                <span
                  class="np-audio-detail-part"
                  :class="{ 'np-audio-detail-part--accent': part.accent }"
                >{{ part.text }}</span>
                <span v-if="index < audioInfoParts.length - 1" class="np-audio-separator">·</span>
              </template>
            </span>
          </div>
        </div>

        <div class="np-control-deck">
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
            @click="handleToggleShuffle()"
          >
            <span class="material-symbols-rounded">shuffle</span>
          </button>

          <button class="ctrl-btn ctrl-btn--transport" :class="{ 'ctrl-btn--switching': isTrackSwitchAnimating }" @click="handlePrevClick()">
            <span class="material-symbols-rounded filled" style="font-size: 30px">skip_previous</span>
          </button>

          <!-- 播放/暂停 带动画 -->
          <button class="play-btn play-btn--transport" :class="{ 'play-btn--switching': isTrackSwitchAnimating }" @click="handleTogglePlayPause()" :disabled="player.isLoadingAudio">
            <transition name="play-icon">
              <span
                v-if="player.isLoadingAudio"
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

          <button class="ctrl-btn ctrl-btn--transport" :class="{ 'ctrl-btn--switching': isTrackSwitchAnimating }" @click="handleNextClick()">
            <span class="material-symbols-rounded filled" style="font-size: 30px">skip_next</span>
          </button>

          <button
            class="ctrl-btn"
            :class="{ active: player.repeatMode !== 'off' }"
            @click="handleToggleRepeatMode()"
          >
            <span class="material-symbols-rounded">{{ player.repeatMode === 'one' ? 'repeat_one' : 'repeat' }}</span>
          </button>
        </div>

        <!-- 工具栏（对齐 Android 底部：Favorite -> Queue -> Sleep -> Volume -> Speed -> Add） -->
        <div
          class="np-toolbar"
          @click.stop
        >
          <button
            class="tool-btn tool-btn--feedback fav-btn"
            :class="{ active: isFavorite }"
            :disabled="!canToggleFavorite"
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
          <button class="tool-btn tool-btn--feedback" @click="handleOpenQueue()">
            <span class="material-symbols-rounded">queue_music</span>
          </button>
          <!-- 睡眠定时器 -->
          <div class="sleep-wrap">
            <button
              class="tool-btn tool-btn--feedback"
              :class="{ active: player.sleepTimerMode || showSleepMenu }"
              @click="triggerControlFeedbackPulse(); toggleToolbarPanel('sleep')"
            >
              <span class="material-symbols-rounded">timer</span>
              <span v-if="player.sleepRemainingSeconds > 0" class="sleep-badge">
                {{ formatSleepRemaining(player.sleepRemainingSeconds) }}
              </span>
            </button>
            <div v-if="showSleepMenu" class="sleep-popover np-floating-popover np-floating-popover--menu">
              <button
                v-for="opt in sleepOptions"
                :key="opt.value"
                class="sleep-option"
                @click="handleSleepOption(opt.value); showSleepMenu = false"
              >
                {{ opt.label }}
              </button>
              <button
                v-if="player.sleepTimerMode"
                class="sleep-option cancel"
                @click="player.cancelSleepTimer(); showSleepMenu = false"
              >
                {{ t('player.sleep_off') }}
              </button>
            </div>
          </div>
          <div class="volume-wrap">
            <button
              class="tool-btn tool-btn--feedback"
              :class="{ active: showVolumeSlider }"
              @click="triggerControlFeedbackPulse(); toggleToolbarPanel('volume')"
            >
              <span class="material-symbols-rounded">{{ player.volume === 0 ? 'volume_off' : player.volume < 0.5 ? 'volume_down' : 'volume_up' }}</span>
            </button>
            <div v-if="showVolumeSlider" class="volume-popover np-floating-popover np-floating-popover--volume">
              <input
                type="range"
                min="0"
                max="1"
                step="0.01"
                :value="1 - player.volume"
                class="volume-slider"
                @input="player.setVolume(1 - parseFloat(($event.target as HTMLInputElement).value))"
              />
              <EditableRangeValue
                :model-value="player.volume"
                class="volume-label"
                :min="0"
                :max="1"
                :step="0.01"
                :input-scale="100"
                :input-width="32"
                :range-reversed="true"
                :display-value="`${Math.round(player.volume * 100)}%`"
                input-suffix="%"
                :aria-label="t('player.volume')"
                @update:model-value="player.setVolume($event)"
              />
            </div>
          </div>
          <!-- 音效 (AudioFX) -->
          <div class="speed-wrap">
            <button class="tool-btn tool-btn--feedback" :class="{ active: showAudioFxPanel || player.hasActiveEffects }" @click="triggerControlFeedbackPulse(); toggleToolbarPanel('audiofx')">
              <span class="material-symbols-rounded">tune</span>
            </button>
            <div v-if="showAudioFxPanel" class="audiofx-popover np-floating-popover np-floating-popover--audiofx">
              <!-- 播放速度 -->
              <div class="audiofx-section">
                <div class="audiofx-section-header">{{ t('player.speed') }}</div>
                <div class="audiofx-speed-grid">
                  <button
                    v-for="spd in [0.5, 0.75, 0.85, 1.0, 1.25, 1.5, 2.0, 3.0]"
                    :key="spd"
                    class="speed-option"
                    :class="{ active: player.playbackSpeed === spd }"
                    @click="player.setSpeed(spd)"
                  >
                    {{ spd }}x
                  </button>
                </div>
                <div class="audiofx-slider-row">
                  <span class="audiofx-slider-label">0.25x</span>
                  <input
                    type="range" min="0.25" max="3" step="0.05"
                    :value="player.playbackSpeed"
                    class="audiofx-slider"
                    @input="player.setSpeed(parseFloat(($event.target as HTMLInputElement).value))"
                  />
                  <span class="audiofx-slider-label">3x</span>
                  <EditableRangeValue
                    :model-value="player.playbackSpeed"
                    class="audiofx-slider-value"
                    :min="0.25"
                    :max="3"
                    :step="0.05"
                    :display-value="`${player.playbackSpeed.toFixed(2)}x`"
                    input-suffix="x"
                    :aria-label="t('player.playback_speed')"
                    @update:model-value="player.setSpeed($event)"
                  />
                </div>
              </div>

              <!-- 响度增益 -->
              <div class="audiofx-section">
                <div class="audiofx-section-header">{{ t('player.loudness_gain') }}</div>
                <div class="audiofx-preset-row">
                  <button v-for="db in [0, 300, 600, 900, 1200, 1500]" :key="db"
                    class="speed-option" :class="{ active: player.loudnessGainMb === db }"
                    @click="player.setLoudnessGain(db)"
                  >
                    {{ db === 0 ? '0' : '+' + (db / 100).toFixed(0) }}dB
                  </button>
                </div>
                <div class="audiofx-slider-row">
                  <span class="audiofx-slider-label">0</span>
                  <input
                    type="range" min="0" max="1500" step="50"
                    :value="player.loudnessGainMb"
                    class="audiofx-slider"
                    @input="player.setLoudnessGain(parseFloat(($event.target as HTMLInputElement).value))"
                  />
                  <span class="audiofx-slider-label">+15dB</span>
                  <EditableRangeValue
                    :model-value="player.loudnessGainMb"
                    class="audiofx-slider-value"
                    :min="0"
                    :max="1500"
                    :step="50"
                    :input-scale="0.01"
                    :input-width="44"
                    :display-value="`+${(player.loudnessGainMb / 100).toFixed(1)}dB`"
                    input-suffix="dB"
                    :aria-label="t('player.loudness_gain')"
                    @update:model-value="player.setLoudnessGain($event)"
                  />
                </div>
              </div>

              <!-- 均衡器 -->
              <div class="audiofx-section">
                <div class="audiofx-section-header">
                  {{ t('player.equalizer') }}
                  <label class="audiofx-toggle">
                    <input type="checkbox" :checked="player.equalizerEnabled"
                      @change="player.setEqualizer(($event.target as HTMLInputElement).checked, player.equalizerBands)" />
                    <span class="audiofx-toggle-slider"></span>
                  </label>
                </div>
                <CustomSelect
                  surface="dark"
                  :model-value="player.equalizerPresetId"
                  :options="eqPresetIds.map(pid => ({ value: pid, label: t('player.eq_' + pid) }))"
                  @update:model-value="player.setEqualizerPreset($event)"
                />
                <div v-if="player.equalizerEnabled" class="audiofx-eq-bands">
                  <div v-for="(freq, i) in eqFreqLabels" :key="i" class="audiofx-eq-band">
                    <EditableRangeValue
                      :model-value="player.equalizerBands[i]"
                      class="audiofx-eq-val"
                      :min="-1500"
                      :max="1500"
                      :step="50"
                      :input-scale="0.01"
                      :input-width="44"
                      :display-value="(player.equalizerBands[i] / 100).toFixed(1)"
                      input-suffix="dB"
                      :aria-label="`${freq} Hz`"
                      @update:model-value="onEqBandChange(i, $event)"
                    />
                    <input type="range" min="-1500" max="1500" step="50"
                      class="audiofx-eq-slider" orient="vertical"
                      :value="player.equalizerBands[i]"
                      @input="onEqBandChange(i, parseFloat(($event.target as HTMLInputElement).value))" />
                    <span class="audiofx-eq-freq">{{ freq }}</span>
                  </div>
                </div>
              </div>

              <!-- 重置 -->
              <button class="speed-option audiofx-reset" @click="player.resetAudioEffects()">
                {{ t('player.reset_effects') }}
              </button>
            </div>
          </div>
          <button class="tool-btn tool-btn--feedback" @click="triggerControlFeedbackPulse(); toggleToolbarPanel('add')">
            <span class="material-symbols-rounded">playlist_add</span>
          </button>
        </div>
        </div>
        </div>
      </section>

      <!-- 右侧歌词 -->
      <section class="np-right" :class="{ 'np-right--switching': isTrackSwitchAnimating, 'np-right--beat-active': isVisualBeatActive }">
        <NowPlayingViewSwitch
          class="np-view-switch-anchor"
          :model-value="playViewMode"
          :lyrics-available="lyricsModeAvailable"
          @update:model-value="requestPlayViewMode"
        />
        <LyricsView
          v-if="displayLyrics.length > 0"
          :lyrics="displayLyrics"
          :current-time-ms="player.interpolatedPositionMs"
          :preview-time-ms="previewPositionMs"
          :is-playing="player.isPlaying"
          :lyric-offset-ms="currentLyricTotalOffsetMs"
          :seek-seq="player.lastSeekCommand.seq"
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
      <QueuePanel v-if="player.hasPlaybackSession && showQueue" @close="showQueue = false" />
    </Teleport>
    <AddToPlaylistDialog v-if="player.hasPlaybackSession" v-model:open="showAddToPlaylist" :track="player.currentTrack" />
    <ListenTogetherPanel v-if="player.hasPlaybackSession && showLtPanel" class="np-lt-panel" @close="showLtPanel = false" />

    <!-- 更多选项面板（对齐 Android MoreOptionsSheet） -->
    <Teleport to="body">
      <Transition name="more-sheet">
      <div v-if="player.hasPlaybackSession && showMoreSheet" class="np-more-overlay" @click="showMoreSheet = false">
        <div class="np-more-sheet" @click.stop>

          <Transition :name="moreSheetTransition" mode="out-in">
          <div :key="moreSheetView" class="np-more-sheet-content">

          <!-- 主菜单（对齐 Android MoreOptionsSheet 顺序） -->
          <template v-if="moreSheetView === 'main'">
            <h4 class="np-more-title">{{ t('player.more_options') }}</h4>

            <!-- 获取歌曲信息 -->
            <button class="np-more-list-item" @click="openInfoSearch">
              <span class="material-symbols-rounded">info</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.get_info') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 编辑歌曲信息 -->
            <button class="np-more-list-item" @click="openEditInfo">
              <span class="material-symbols-rounded">edit</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.edit_info') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 歌曲详情 -->
            <button class="np-more-list-item" @click="goToSubView('track-detail')">
              <span class="material-symbols-rounded">article</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">歌曲详情</span>
                <span class="np-more-list-desc">来源、ID、音频与下载状态</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 查看专辑（对齐 Android：网易云来源显示） -->
            <button v-if="canViewNeteaseAlbum" class="np-more-list-item" @click="openCurrentAlbum">
              <span class="material-symbols-rounded">library_music</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.view_album', { name: albumName }) }}</span>
              </div>
            </button>

            <!-- 音质切换（仅在线来源显示） -->
            <button v-if="currentSource !== 'local'" class="np-more-list-item" @click="goToSubView('quality')">
              <span class="material-symbols-rounded">music_note</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.quality_switch') }}</span>
                <span class="np-more-list-desc">{{ currentAudioQualityLabel() || '—' }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 音频效果 -->
            <button class="np-more-list-item" @click="goToSubView('speed')">
              <span class="material-symbols-rounded">tune</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.audio_effects') }}</span>
                <span class="np-more-list-desc">{{ t('player.audio_effects_desc') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 歌词偏移 -->
            <button class="np-more-list-item" @click="goToSubView('offset')">
              <span class="material-symbols-rounded">timer</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.lyric_offset') }}</span>
                <span class="np-more-list-desc">{{ currentLyricUserOffsetMs > 0 ? '+' : '' }}{{ currentLyricUserOffsetMs }}ms · {{ currentLyricOffsetSourceLabel }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 歌词字号 -->
            <button class="np-more-list-item" @click="goToSubView('fontsize')">
              <span class="material-symbols-rounded">format_size</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.font_scale') }}</span>
                <span class="np-more-list-desc">{{ Math.round(settings.lyricFontScale * 100) }}%</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 歌词编辑器（对齐 Android LyricsEditorSheet） -->
            <button class="np-more-list-item" @click="openLyricsEditor">
              <span class="material-symbols-rounded">edit_note</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.lyrics_editor') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 歌词填充（对齐 Android FillOptionsDialog） -->
            <button class="np-more-list-item" @click="lyricFillQuery = player.currentTrack?.title || ''; lyricFillResults = []; lyricFillPlatform = currentSource === 'netease' ? 'netease' : (currentSource === 'qq' ? 'qq' : 'lrclib'); goToSubView('lyrics-fill')">
              <span class="material-symbols-rounded">lyrics</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.lyrics_fill') }}</span>
                <span class="np-more-list-desc">{{ t('player.lyrics_fill_desc') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 分享 -->
            <button class="np-more-list-item" @click="shareSong">
              <span class="material-symbols-rounded">share</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.share') }}</span>
              </div>
            </button>

            <!-- 一起听 -->
            <button class="np-more-list-item" @click="openListenTogetherFromMore">
              <span class="material-symbols-rounded">headphones</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('listen_together.title') }}</span>
                <span class="np-more-list-desc">创建或加入同步播放房间</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 下载（在线来源：下载 / 重新下载 / 本地播放状态） -->
            <button
              v-if="currentSource !== 'local'"
              class="np-more-list-item"
              :disabled="downloadActionDisabled"
              @click="handleDownloadAction"
            >
              <span class="material-symbols-rounded">{{ downloadActionIcon }}</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ downloadActionLabel }}</span>
                <span v-if="downloadActionDesc" class="np-more-list-desc">{{ downloadActionDesc }}</span>
              </div>
            </button>
          </template>

          <!-- 子视图：歌词偏移 -->
          <template v-else-if="moreSheetView === 'offset'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.lyric_offset') }}</h4>
            </div>
            <div class="np-more-item">
              <div class="np-more-label">{{ t('player.lyric_offset_song') }}</div>
              <div class="np-more-hint">{{ t('player.lyric_offset_base', { value: `${currentLyricDefaultOffsetMs >= 0 ? '+' : ''}${currentLyricDefaultOffsetMs}ms` }) }} · {{ currentLyricOffsetSourceLabel }}</div>
              <div class="np-more-row">
                <input type="range" min="-2000" max="2000" step="50"
                  :value="currentLyricUserOffsetMs"
                  class="np-more-slider"
                  @input="currentLyricUserOffsetMs = parseInt(($event.target as HTMLInputElement).value)"
                />
                <EditableRangeValue
                  v-model="currentLyricUserOffsetMs"
                  class="np-offset-value"
                  :class="{ positive: currentLyricUserOffsetMs > 0, negative: currentLyricUserOffsetMs < 0 }"
                  :min="-2000"
                  :max="2000"
                  :step="50"
                  :display-value="`${currentLyricUserOffsetMs > 0 ? '+' : ''}${currentLyricUserOffsetMs}ms`"
                  input-suffix="ms"
                  :aria-label="t('player.lyric_offset')"
                />
              </div>
            </div>
          </template>

          <!-- 子视图：字号 -->
          <template v-else-if="moreSheetView === 'fontsize'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.font_scale') }}</h4>
            </div>
            <div class="np-more-item">
              <div class="np-more-row">
                <input type="range" min="0.6" max="1.6" step="0.05"
                  :value="settings.lyricFontScale"
                  class="np-more-slider"
                  @input="settings.lyricFontScale = parseFloat(($event.target as HTMLInputElement).value)"
                />
                <EditableRangeValue
                  v-model="settings.lyricFontScale"
                  class="np-offset-value"
                  :min="0.6"
                  :max="1.6"
                  :step="0.05"
                  :input-scale="100"
                  :display-value="`${Math.round(settings.lyricFontScale * 100)}%`"
                  input-suffix="%"
                  :aria-label="t('player.font_scale')"
                />
              </div>
              <p class="np-more-preview" :style="{ fontSize: `${24 * settings.lyricFontScale}px` }">
                {{ t('player.font_preview') }}
              </p>
            </div>
          </template>

          <!-- 子视图：音频效果（对齐 Android PlaybackSoundSheet） -->
          <template v-else-if="moreSheetView === 'speed'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.audio_effects') }}</h4>
            </div>
            <!-- 播放速度 -->
            <div class="np-more-item">
              <div class="np-more-label">
                <span class="material-symbols-rounded" style="font-size: 18px">speed</span>
                {{ t('player.playback_speed') }}
              </div>
              <div class="np-more-speed-grid">
                <button
                  v-for="spd in [0.5, 0.75, 0.85, 0.9, 1, 1.1, 1.25, 1.5, 2]"
                  :key="spd"
                  class="np-more-speed-btn"
                  :class="{ active: player.playbackSpeed === spd }"
                  @click="player.setSpeed(spd)"
                >{{ spd }}x</button>
              </div>
            </div>
            <!-- 速度微调滑块 -->
            <div class="np-more-item">
              <div class="np-more-row">
                <input type="range" min="0.25" max="3" step="0.05"
                  :value="player.playbackSpeed"
                  class="np-more-slider"
                  @input="player.setSpeed(parseFloat(($event.target as HTMLInputElement).value))"
                />
                <EditableRangeValue
                  :model-value="player.playbackSpeed"
                  class="np-offset-value"
                  :min="0.25"
                  :max="3"
                  :step="0.05"
                  :display-value="`${player.playbackSpeed.toFixed(2)}x`"
                  input-suffix="x"
                  :aria-label="t('player.playback_speed')"
                  @update:model-value="player.setSpeed($event)"
                />
              </div>
            </div>
          </template>

          <!-- 子视图：获取歌曲信息 -->
          <template v-else-if="moreSheetView === 'search'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.get_info') }}</h4>
            </div>
            <div class="np-more-search-bar">
              <input
                v-model="searchQuery"
                class="np-more-input"
                :placeholder="t('player.search_song')"
                @keydown.enter="doSearch"
              />
              <button class="np-more-search-btn" @click="doSearch" :disabled="isSearching">
                <span class="material-symbols-rounded">search</span>
              </button>
            </div>
            <div class="np-more-segmented platform">
              <button
                :class="{ active: infoSearchPlatform === 'netease' }"
                @click="infoSearchPlatform = 'netease'; searchResults = []; infoApplyCandidate = null"
              >
                网易云
              </button>
              <button
                :class="{ active: infoSearchPlatform === 'qq' }"
                @click="infoSearchPlatform = 'qq'; searchResults = []; infoApplyCandidate = null"
              >
                QQ 音乐
              </button>
              <button
                :class="{ active: infoSearchPlatform === 'bilibili' }"
                @click="infoSearchPlatform = 'bilibili'; searchResults = []; infoApplyCandidate = null"
              >
                Bilibili
              </button>
              <button
                :class="{ active: infoSearchPlatform === 'youtube' }"
                @click="infoSearchPlatform = 'youtube'; searchResults = []; infoApplyCandidate = null"
              >
                YouTube
              </button>
            </div>
            <div v-if="isSearching" class="np-more-status">{{ t('player.searching') }}</div>
            <div v-else-if="searchResults.length === 0 && searchQuery" class="np-more-status">{{ t('player.no_results') }}</div>
            <div class="np-more-search-results">
              <button
                v-for="(r, ri) in searchResults"
                :key="ri"
                class="np-more-search-item"
                :class="{ active: infoApplyCandidate === r }"
                @click="applySearchResult(r)"
              >
                <BilibiliCoverImage v-if="r.cover_url" :src="r.cover_url" class="np-more-search-cover" />
                <div class="np-more-search-info">
                  <span class="np-more-search-title">{{ r.title }}</span>
                  <span class="np-more-search-artist">{{ r.artist }}</span>
                </div>
                <span class="np-more-search-source">{{ platformLabel(r.source) }}</span>
              </button>
            </div>
            <div v-if="infoApplyCandidate" class="np-more-field-picker">
              <div class="np-more-candidate-preview">
                <BilibiliCoverImage
                  v-if="infoApplyCandidate.cover_url || infoApplyCandidate.coverUrl"
                  :src="infoApplyCandidate.cover_url || infoApplyCandidate.coverUrl"
                  class="np-more-candidate-cover"
                />
                <div class="np-more-search-info">
                  <span class="np-more-search-title">{{ infoApplyCandidate.title }}</span>
                  <span class="np-more-search-artist">{{ infoApplyCandidate.artist }}</span>
                </div>
              </div>
              <div class="np-more-field-title">选择要填充的字段</div>
              <div class="np-more-field-options">
                <label class="np-more-chip">
                  <input v-model="applyInfoFields.title" type="checkbox" />
                  <span>歌曲名</span>
                </label>
                <label class="np-more-chip">
                  <input v-model="applyInfoFields.artist" type="checkbox" />
                  <span>歌手</span>
                </label>
                <label class="np-more-chip">
                  <input v-model="applyInfoFields.cover" type="checkbox" />
                  <span>封面</span>
                </label>
                <label class="np-more-chip">
                  <input v-model="applyInfoFields.lyrics" type="checkbox" />
                  <span>歌词</span>
                </label>
              </div>
              <div class="np-more-form-actions compact">
                <button class="np-more-form-btn primary" @click="confirmApplySearchResult">
                  <span class="material-symbols-rounded">check</span>
                  应用选择
                </button>
                <button class="np-more-form-btn" @click="infoApplyCandidate = null">
                  取消
                </button>
              </div>
            </div>
          </template>

          <!-- 子视图：编辑歌曲信息 -->
          <template v-else-if="moreSheetView === 'editinfo'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.edit_info') }}</h4>
            </div>
            <div class="np-more-form">
              <label class="np-more-form-label">{{ t('player.song_title') }}</label>
              <input v-model="editTitle" class="np-more-input" />

              <label class="np-more-form-label">{{ t('player.artist_name') }}</label>
              <input v-model="editArtist" class="np-more-input" />

              <label class="np-more-form-label">{{ t('player.cover_url_label') }}</label>
              <input v-model="editCoverUrl" class="np-more-input" />

              <div class="np-more-form-actions">
                <button class="np-more-form-btn primary" @click="saveEditInfo">
                  <span class="material-symbols-rounded">check</span>
                  {{ t('common.save') }}
                </button>
                <button v-if="player.hasOriginalTrackInfo()" class="np-more-form-btn" @click="restoreInfo">
                  <span class="material-symbols-rounded">restore</span>
                  {{ t('player.restore_original') }}
                </button>
              </div>
            </div>
          </template>

          <!-- 子视图：歌曲详情 -->
          <template v-else-if="moreSheetView === 'track-detail'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">歌曲详情</h4>
            </div>
            <div class="np-track-detail-card">
              <div class="np-track-detail-hero">
                <img
                  v-if="coverUrl"
                  :src="coverUrl"
                  class="np-track-detail-cover"
                  referrerpolicy="no-referrer"
                />
                <div class="np-track-detail-heading">
                  <strong>{{ player.currentTrack?.title || '-' }}</strong>
                  <span>{{ player.currentTrack?.artist || '-' }}</span>
                </div>
              </div>

              <button class="np-track-detail-row copyable" @click="copyText(player.currentTrack?.id || '')">
                <span>歌曲 ID</span>
                <strong>{{ player.currentTrack?.id || '-' }}</strong>
              </button>
              <button class="np-track-detail-row copyable" @click="copyText(player.currentTrack?.title || '')">
                <span>标题</span>
                <strong>{{ player.currentTrack?.title || '-' }}</strong>
              </button>
              <button class="np-track-detail-row copyable" @click="copyText(player.currentTrack?.artist || '')">
                <span>歌手</span>
                <strong>{{ player.currentTrack?.artist || '-' }}</strong>
              </button>
              <div class="np-track-detail-row">
                <span>专辑</span>
                <strong>{{ albumName || '-' }}</strong>
              </div>
              <div class="np-track-detail-row">
                <span>来源</span>
                <strong>{{ playbackSourceLabel || currentSource }}</strong>
              </div>
              <div class="np-track-detail-row">
                <span>{{ t('player.track_detail_duration') }}</span>
                <strong>{{ formatDurationMs(player.currentTrack?.durationMs || player.durationMs) }}</strong>
              </div>
              <div class="np-track-detail-row">
                <span>{{ t('player.track_detail_audio_params') }}</span>
                <strong>{{ trackDetailAudioParams || audioInfoDisplay || '-' }}</strong>
              </div>
              <div class="np-track-detail-row">
                <span>{{ t('player.track_detail_bitrate') }}</span>
                <strong>
                  {{ player.audioInfo?.bitrate && player.audioInfo.bitrate > 0
                    ? `${Math.round(player.audioInfo.bitrate)} kbps`
                    : '-' }}
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

          <!-- 子视图：音质切换 -->
          <template v-else-if="moreSheetView === 'quality'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.quality_switch') }}</h4>
            </div>
            <div v-if="qualityOptionsForSource(currentSource).length" class="np-more-quality-list">
              <button
                v-for="q in qualityOptionsForSource(currentSource)"
                :key="q.key"
                class="np-more-quality-item"
                :class="{ active: currentQualityKey() === q.key }"
                :disabled="isQualitySwitching"
                @click="switchQuality(q.key)"
              >
                <span>{{ t(q.label) }}</span>
                <span
                  v-if="currentQualityKey() === q.key"
                  class="material-symbols-rounded"
                  style="font-size: 18px"
                >check</span>
              </button>
              <div v-if="isQualitySwitching" class="np-more-status">{{ t('player.quality_changing') }}</div>
            </div>
            <div v-else class="np-more-status">{{ t('player.not_available') }}</div>
          </template>

          <!-- 子视图：歌词编辑器（对齐 Android LyricsEditorSheet） -->
          <template v-else-if="moreSheetView === 'lyrics-editor'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.lyrics_editor') }}</h4>
            </div>
            <div class="np-lyrics-editor">
              <div class="np-more-segmented">
                <button
                  :class="{ active: lyricsEditorTab === 'original' }"
                  @click="lyricsEditorTab = 'original'"
                >
                  原文
                </button>
                <button
                  :class="{ active: lyricsEditorTab === 'translation' }"
                  @click="lyricsEditorTab = 'translation'"
                >
                  翻译
                </button>
              </div>
              <textarea
                v-if="lyricsEditorTab === 'original'"
                v-model="lyricsEditorText"
                class="np-lyrics-textarea"
                :placeholder="t('player.lyrics_editor_placeholder')"
                spellcheck="false"
              />
              <textarea
                v-else
                v-model="lyricsTranslationEditorText"
                class="np-lyrics-textarea"
                placeholder="[00:12.34]翻译歌词，可留空"
                spellcheck="false"
              />
              <div class="np-more-form-actions">
                <button class="np-more-form-btn primary" @click="applyLyricsFromEditor">
                  <span class="material-symbols-rounded">check</span>
                  {{ t('player.lyrics_apply') }}
                </button>
                <button class="np-more-form-btn" @click="lyricsEditorText = ''; lyricsTranslationEditorText = ''; applyLyricsFromEditor()">
                  <span class="material-symbols-rounded">clear_all</span>
                  {{ t('player.lyrics_clear') }}
                </button>
              </div>
            </div>
          </template>

          <!-- 子视图：歌词填充（对齐 Android FillOptionsDialog） -->
          <template v-else-if="moreSheetView === 'lyrics-fill'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.lyrics_fill') }}</h4>
            </div>
            <div class="np-more-search-bar">
              <input
                v-model="lyricFillQuery"
                class="np-more-input"
                :placeholder="t('player.search_song')"
                @keydown.enter="doLyricFillSearch"
              />
              <button class="np-more-search-btn" @click="doLyricFillSearch" :disabled="isLyricFilling">
                <span class="material-symbols-rounded">search</span>
              </button>
            </div>
            <div class="np-more-segmented platform">
              <button
                :class="{ active: lyricFillPlatform === 'netease' }"
                @click="lyricFillPlatform = 'netease'; lyricFillResults = []"
              >
                网易云
              </button>
              <button
                :class="{ active: lyricFillPlatform === 'qq' }"
                @click="lyricFillPlatform = 'qq'; lyricFillResults = []"
              >
                QQ 音乐
              </button>
              <button
                :class="{ active: lyricFillPlatform === 'lrclib' }"
                @click="lyricFillPlatform = 'lrclib'; lyricFillResults = []"
              >
                LRCLIB
              </button>
            </div>
            <div v-if="isLyricFilling" class="np-more-status">{{ t('player.searching') }}</div>
            <div v-else-if="lyricFillResults.length === 0 && lyricFillQuery" class="np-more-status">{{ t('player.no_results') }}</div>
            <div class="np-more-search-results">
              <button
                v-for="(r, ri) in lyricFillResults"
                :key="ri"
                class="np-more-search-item"
                @click="applyLyricFill(r)"
              >
                <BilibiliCoverImage v-if="r.cover_url" :src="r.cover_url" class="np-more-search-cover" />
                <div class="np-more-search-info">
                  <span class="np-more-search-title">{{ r.title }}</span>
                  <span class="np-more-search-artist">{{ r.artist }}</span>
                </div>
                <span class="material-symbols-rounded" style="font-size: 18px; color: rgba(255,255,255,0.3)">lyrics</span>
              </button>
            </div>
          </template>

          </div>
          </Transition>

        </div>
      </div>
      </Transition>
    </Teleport>

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

<style scoped lang="scss">
.now-playing {
  position: fixed;
  inset: 0;
  z-index: 200;
  display: flex;
  flex-direction: column;
  // 确保完全不透明
  isolation: isolate;
  overflow: hidden;
  /* 与窗体圆角一致，避免全屏层直角顶出 OS 圆角 */
  border-radius: var(--radius-lg);
  user-select: none;
  -webkit-user-select: none;
  transition: transform 460ms cubic-bezier(0.22, 1, 0.36, 1), opacity 300ms ease;
}

.np-empty-state {
  position: relative;
  z-index: 3;
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  color: rgba(255, 255, 255, 0.72);
}

.np-empty-state h2 {
  margin: 0;
  font-size: 18px;
  font-weight: 500;
  letter-spacing: 0;
}

.np-empty-icon {
  font-size: 48px;
  opacity: 0.68;
}

.np-header-spacer {
  width: 40px;
  height: 40px;
}

/* 开/关由 App 层 slide-up 纯上滑负责，壳层不再二次缩放/位移 */
.np-shell--opening,
.np-shell--closing {
  animation: none;
}

.np-shell--beat-active .np-ambient-glow::before {
  animation: np-beat-shell-bloom 320ms cubic-bezier(0.22, 1, 0.36, 1);
}

// 纯色底层：由 accentBgStyle 动态控制颜色
.np-bg-solid {
  position: absolute;
  inset: 0;
  z-index: -1;
  transition: background 0.8s ease;
}

// 对齐 Android：无全局遮罩，对比度完全由文字颜色和动态配色保证
.np-scrim {
  display: none;
}

/* 环境光：纯 radial 渐变，禁止 filter:blur（滤镜矩形盒会在窗角露出方框） */
.np-ambient-glow {
  position: absolute;
  inset: 0;
  z-index: 1;
  pointer-events: none;
  overflow: hidden;
}

.np-ambient-glow::before {
  content: '';
  position: absolute;
  inset: 0;
  background:
    radial-gradient(ellipse 70% 58% at 28% 24%,
      color-mix(in srgb, var(--np-primary, rgba(255,255,255,0.34)) 38%, transparent) 0%,
      color-mix(in srgb, var(--np-primary, rgba(255,255,255,0.34)) 16%, transparent) 38%,
      transparent 68%),
    radial-gradient(ellipse 62% 54% at 78% 78%,
      color-mix(in srgb, var(--np-primary-container, rgba(255,255,255,0.30)) 30%, transparent) 0%,
      color-mix(in srgb, var(--np-primary-container, rgba(255,255,255,0.30)) 12%, transparent) 42%,
      transparent 72%),
    radial-gradient(ellipse 48% 40% at 55% 48%,
      color-mix(in srgb, var(--np-primary, rgba(255,255,255,0.20)) 10%, transparent) 0%,
      transparent 70%);
  opacity: 0.48;
  transform: scale(1);
  transition: opacity 420ms ease, transform 620ms cubic-bezier(0.22, 1, 0.36, 1);
}

.np-shell--track-switching .np-ambient-glow::before {
  animation: np-glow-breathe 620ms cubic-bezier(0.22, 1, 0.36, 1);
}

/* 打开/关闭时内容不做 settle 位移或缩放，只跟随外壳上滑 */
.np-shell--opening .np-header,
.np-shell--opening .np-body,
.np-shell--closing .np-header,
.np-shell--closing .np-body {
  animation: none;
}

/* 顶栏 */
.np-header {
  position: relative;
  z-index: 2;
  display: flex;
  align-items: center;
  padding: 44px 24px 14px; /* 顶部 44px = 36px 顶栏 + 8px 间距 */
  gap: 12px;
  flex-shrink: 0;
  transition: transform 380ms cubic-bezier(0.22, 1, 0.36, 1), opacity 260ms ease;
}

.np-header-center {
  flex: 1;
  text-align: center;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.np-from-label {
  font-size: 11px;
  font-weight: 600;
  color: rgba(255,255,255,0.60);
  text-transform: uppercase;
  letter-spacing: 1.2px;
}

.np-from-name {
  font-size: 13px;
  font-weight: 600;
  color: rgba(255,255,255,0.87);
}

.np-header-meta-swap-enter-active,
.np-header-meta-swap-leave-active {
  transition: opacity 220ms ease, transform 280ms cubic-bezier(0.22, 1, 0.36, 1);
}

.np-header-meta-swap-enter-from {
  opacity: 0;
  transform: translateY(6px);
}

.np-header-meta-swap-leave-to {
  opacity: 0;
  transform: translateY(-6px);
}

.np-icon-btn {
  width: 46px;
  height: 46px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgba(255,255,255,0.92);
  transition: background 150ms, color 220ms ease;

  &:hover { background: rgba(255,255,255,0.12); }
  .material-symbols-rounded { font-size: 28px; }
}

/* 双栏主体：五五分 */
.np-body {
  position: relative;
  z-index: 2;
  flex: 1;
  display: flex;
  padding: 0 0 16px;
  gap: 0;
  overflow: hidden;
  min-height: 0;
  transition: transform 420ms cubic-bezier(0.22, 1, 0.36, 1), opacity 300ms ease;

  &.np-body--no-header {
    /* 顶栏 36px + 呼吸空间，避免内容贴红绿灯/顶栏 */
    padding-top: 52px;
  }

  &.np-body--lyrics-mode {
    .np-left {
      flex: 0 0 0%;
      opacity: 0;
      transform: translateX(-18px) scale(0.985);
      pointer-events: none;
      overflow: hidden;
      padding: 0;
      gap: 0;
    }

    .np-right {
      flex: 1 1 100%;
      padding: 0 48px 8px;
      transform: translateX(0);
      filter: none;
    }
  }

  &.np-body--cover-mode {
    .np-left {
      flex: 0 0 46%;
      opacity: 1;
      transform: none;
      pointer-events: auto;
      justify-content: center;
      padding: 20px 36px 24px 52px;
    }

    .np-right {
      flex: 1 1 54%;
      padding: 8px 48px 16px 16px;
    }
  }
}

/* 大屏/全屏：封面更大、左右留白更均衡 */
@media (min-height: 900px) {
  .cover-wrap {
    width: min(100%, 380px, 38vh);
  }

  .np-left-stack {
    max-width: 420px;
    gap: 14px;
  }
}

@media (min-width: 1400px) {
  .np-body.np-body--cover-mode {
    .np-left {
      padding: 24px 40px 28px 64px;
    }

    .np-right {
      padding: 12px 64px 20px 24px;
    }
  }
}

.np-left {
  flex: 1;
  min-width: 0;
  max-width: 560px;
  display: flex;
  flex-direction: column;
  align-items: center;
  /* 外层垂直居中整块 stack；stack 内部固定，切歌不重排 */
  justify-content: center;
  padding: 12px 28px 16px 40px;
  transition:
    flex-basis 420ms cubic-bezier(0.22, 1, 0.36, 1),
    padding 420ms cubic-bezier(0.22, 1, 0.36, 1),
    opacity 280ms ease,
    transform 320ms cubic-bezier(0.22, 1, 0.36, 1);
}

.np-left-stack {
  width: 100%;
  max-width: 380px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  flex-shrink: 0;
}

.np-left--beat-active {
  animation: none;
}

.np-right {
  position: relative;
  flex: 1;
  min-width: 0;
  background: transparent;
  display: flex;
  align-items: stretch;
  overflow: hidden;
  padding: 0 40px 8px 8px;
  transition: transform 420ms cubic-bezier(0.22, 1, 0.36, 1), opacity 280ms ease, filter 420ms cubic-bezier(0.22, 1, 0.36, 1);
}

.np-view-switch-anchor {
  position: absolute;
  top: 8px;
  right: 48px;
  z-index: 4;
}

.np-body--lyrics-mode .np-view-switch-anchor { right: 48px; }

/* 封面：全屏时更大更稳 */
.cover-wrap {
  position: relative;
  width: min(100%, 340px, 40vh);
  max-width: 100%;
  aspect-ratio: 1;
  flex-shrink: 0;
  transition: filter 320ms cubic-bezier(0.22, 1, 0.36, 1), opacity 240ms ease;
  overflow: visible;
}

.cover-wrap--card {
  border-radius: 24px;
}

.cover-wrap--disc {
  border-radius: 50%;
}

.cover-wrap--switching {
  .cover-card,
  .cover-disc {
    filter: drop-shadow(0 22px 54px rgba(0,0,0,0.54));
  }
}

.cover-wrap--beat-active {
  animation: np-cover-beat-unison 320ms cubic-bezier(0.22, 1, 0.36, 1);
}

.cover-wrap--lyrics-entry {
  cursor: pointer;

  &:focus-visible {
    outline: 2px solid var(--np-primary-container, #fff);
    outline-offset: 5px;
  }
}

/* Card 模式（圆角矩形，对齐 Android） */
.cover-card {
  position: relative;
  width: 100%;
  height: 100%;
  border-radius: 24px;
  background: transparent;
  display: flex;
  align-items: center;
  justify-content: center;
  color: white;
  overflow: hidden;
  clip-path: inset(0 round 24px);
  box-shadow: 0 16px 48px rgba(0,0,0,0.5);
}

.cover-card-img {
  position: absolute;
  inset: 0;
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
  border-radius: inherit;
  transform: scale(1.01);
}

.cover-card-placeholder {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 48px;
  opacity: 0.35;
}

/* Disc 模式（黑胶唱片） */

.cover-disc {
  width: 100%;
  height: 100%;
  border-radius: 50%;
  background: conic-gradient(
    from 0deg,
    #2d2640,
    #1e1a2e,
    #252030,
    #2d2640
  );
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  will-change: transform;
  filter: drop-shadow(0 16px 48px rgba(0,0,0,0.5));
}

.cover-inner {
  position: relative;
  width: 78%;
  height: 78%;
  border-radius: 50%;
  background: linear-gradient(135deg,
    #2d2640 0%,
    #1a1724 50%,
    #1e1a2e 100%
  );
  display: flex;
  align-items: center;
  justify-content: center;
  color: white;
  overflow: hidden;
}

.cover-img {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  border-radius: 50%;
}

.cover-disc-placeholder {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 48px;
  opacity: 0.35;
}

.cover-groove {
  position: absolute;
  width: 90%;
  height: 90%;
  border-radius: 50%;
  border: 1px solid rgba(255,255,255,0.04);
  pointer-events: none;
}

.cover-hole {
  position: absolute;
  width: 40px;
  height: 40px;
  border-radius: 50%;
  background: rgba(0,0,0,0.6);
  border: 2px solid rgba(255,255,255,0.06);
}

/* 曲目信息：固定高度 + 绝对叠层，切歌不塌布局 */
.np-info {
  text-align: center;
  width: 100%;
  max-width: 100%;
  padding: 0 8px;
  margin: 0;
  position: relative;
  height: 52px;
  flex-shrink: 0;
  overflow: hidden;
}

.np-info--beat-active {
  animation: none;
}

.np-meta {
  position: absolute;
  inset: 0;
  width: 100%;
  text-align: center;
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-items: center;
  box-sizing: border-box;
  padding: 0 4px;
}

.np-title {
  font-size: 22px;
  font-weight: 700;
  color: white;
  line-height: 1.25;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  letter-spacing: -0.3px;
}

.np-artist {
  font-size: 14px;
  color: rgba(255,255,255,0.78);
  margin-top: 2px;
  font-weight: 500;
  line-height: 1.25;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 进度条区域：固定高度，音质行始终占位 */
.np-slider-area {
  width: 100%;
  max-width: 100%;
  /* 进度条 + 时间 + 音质/下载 chip，留足高度避免裁切 */
  height: 80px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  justify-content: flex-start;
  overflow: visible;
}

.np-slider-area--beat-active {
  animation: np-slider-beat-glide 320ms cubic-bezier(0.22, 1, 0.36, 1);
}

.np-time {
  display: flex;
  justify-content: space-between;
  font-size: 11px;
  font-weight: 600;
  color: rgba(255,255,255,0.78);
  padding: 4px 4px 0;
  font-variant-numeric: tabular-nums;
}

.np-audio-info {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-wrap: wrap;
  gap: 6px;
  text-align: center;
  font-size: 11px;
  font-weight: 600;
  color: rgba(255,255,255,0.68);
  letter-spacing: 0.2px;
  margin-top: 6px;
  /* 给下载 chip 完整高度，禁止裁切圆角 */
  min-height: 24px;
  height: auto;
  flex-shrink: 0;
  overflow: visible;
  padding: 1px 2px;
  box-sizing: border-box;
}

.np-audio-codec {
  color: var(--np-primary-container, var(--md-primary-container, #E8DEF8));
  transition: color 0.6s ease;
}

.np-audio-detail {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  color: rgba(255,255,255,0.68);

  &.separated::before {
    content: '·';
    margin-right: 0;
    color: rgba(255,255,255,0.42);
  }
}

.np-audio-detail-part {
  color: rgba(255,255,255,0.70);
  transition: color 0.45s ease;
}

.np-audio-detail-part--accent {
  /* 降低饱和与发光，避免「高清环绕声」等标签过于抢眼 */
  color: color-mix(in srgb, var(--np-primary, var(--md-primary, #D0BCFF)) 58%, rgba(255,255,255,0.78));
  text-shadow: none;
  font-weight: 600;
}

.np-audio-separator {
  color: rgba(255,255,255,0.34);
}

.np-download-chip {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 3px 9px;
  border-radius: 999px;
  line-height: 1.2;
  white-space: nowrap;
  color: var(--np-primary-container, var(--md-primary-container, #E8DEF8));
  background: rgba(255,255,255,0.10);
  border: 1px solid rgba(255,255,255,0.14);
  box-sizing: border-box;
  flex-shrink: 0;

  .material-symbols-rounded {
    font-size: 13px;
    line-height: 1;
  }
}

/* 控制栏 */
.np-control-deck {
  position: relative;
  width: 100%;
  min-height: 108px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2px;
  padding: 7px 10px 6px;
  border-radius: 8px;
}

.np-control-deck::before {
  content: '';
  position: absolute;
  inset: 0;
  border-radius: 8px;
  border: 1px solid rgba(255,255,255,0.14);
  border-color: color-mix(in srgb, var(--np-primary-container, rgba(255,255,255,0.64)) 28%, rgba(255,255,255,0.10));
  background: rgba(20,18,24,0.38);
  background: color-mix(in srgb, var(--np-primary-container, rgba(255,255,255,0.18)) 14%, rgba(20,18,24,0.46));
  box-shadow: 0 12px 30px rgba(0,0,0,0.20), inset 0 1px 0 rgba(255,255,255,0.18);
  backdrop-filter: blur(24px) saturate(1.14);
  -webkit-backdrop-filter: blur(24px) saturate(1.14);
  isolation: isolate;
  pointer-events: none;
  z-index: 0;
}

@supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
  .np-control-deck::before {
    background: rgba(48,44,58,0.88);
  }
}

.np-controls {
  position: relative;
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  width: 100%;
  margin-top: 0;
  flex-shrink: 0;
  transition: opacity 240ms ease;
}

.np-controls--switching {
  animation: none;
}

.np-controls--feedback {
  animation: np-controls-feedback-wave 260ms cubic-bezier(0.22, 1, 0.36, 1);
}

.np-controls--feedback-prev .ctrl-btn--transport:first-of-type {
  --transport-glow-shift: -16px;
  --transport-glow-rotation: -22deg;
  --transport-glow-opacity: 1;
  --transport-ring-opacity: 0.68;
  --transport-ring-scale: 1.04;
}

.np-controls--feedback-next .ctrl-btn--transport:last-of-type {
  --transport-glow-shift: 16px;
  --transport-glow-rotation: 22deg;
  --transport-glow-opacity: 1;
  --transport-ring-opacity: 0.68;
  --transport-ring-scale: 1.04;
}

.ctrl-btn {
  width: 46px;
  height: 46px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgba(255,255,255,0.87);
  position: relative;
  overflow: hidden;
  transition: color 150ms, background 150ms, transform 180ms cubic-bezier(0.22, 1, 0.36, 1), box-shadow 220ms cubic-bezier(0.22, 1, 0.36, 1);

  &:hover { background: rgba(255,255,255,0.10); color: rgba(255,255,255,0.95); }
  &.active { color: var(--np-primary, white); }
  &:active { transform: scale(0.9); }
}

.ctrl-btn--transport {
  --transport-glow-shift: 0px;
  --transport-glow-rotation: 0deg;
  --transport-glow-opacity: 0;
  --transport-ring-opacity: 0;
  --transport-ring-scale: 0.9;

  &::after {
    content: '';
    position: absolute;
    inset: 9px;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--np-primary-container, rgba(255,255,255,0.8)) 46%, rgba(255,255,255,0.14));
    opacity: var(--transport-ring-opacity);
    transform: scale(var(--transport-ring-scale));
    transition:
      opacity 220ms ease,
      transform 300ms cubic-bezier(0.22, 1, 0.36, 1),
      border-color 260ms ease;
  }

  &::before {
    content: '';
    position: absolute;
    inset: -8px;
    border-radius: 999px;
    background:
      linear-gradient(
        90deg,
        transparent 8%,
        rgba(255,255,255,0.05) 24%,
        color-mix(in srgb, var(--np-primary-container, rgba(255,255,255,0.92)) 72%, white) 48%,
        rgba(255,255,255,0.08) 72%,
        transparent 92%
      );
    opacity: var(--transport-glow-opacity);
    transform: translateX(var(--transport-glow-shift)) rotate(var(--transport-glow-rotation)) scale(1.08);
    filter: blur(10px);
    transition:
      opacity 180ms ease,
      transform 320ms cubic-bezier(0.22, 1, 0.36, 1);
    pointer-events: none;
  }

  &:hover::after,
  &:focus-visible::after {
    --transport-ring-opacity: 0.44;
    --transport-ring-scale: 1;
  }

  &:hover::before,
  &:focus-visible::before {
    --transport-glow-opacity: 0.48;
    --transport-glow-shift: 0px;
    --transport-glow-rotation: 0deg;
  }
}

.ctrl-btn--switching {
  animation: np-control-pulse 320ms cubic-bezier(0.22, 1, 0.36, 1);
}

.play-btn {
  width: 52px;
  height: 52px;
  border-radius: 50%;
  background: var(--np-primary-container, #f5f0ff);
  color: var(--np-on-primary, rgb(20, 18, 24));
  display: flex;
  align-items: center;
  justify-content: center;
  /* 轻阴影，不进 mask，避免中间发黑 / 四角 */
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.22);
  filter: none;
  margin: 0 4px;
  transition: transform 150ms var(--ease-standard), background 0.6s ease, color 0.6s ease, box-shadow 150ms ease;
  overflow: hidden;
  position: relative;
  isolation: isolate;

  &:hover {
    transform: scale(1.05);
    box-shadow: 0 8px 22px rgba(0, 0, 0, 0.28);
  }
  &:active { transform: scale(0.94); }
}

.play-btn--transport {
  overflow: hidden;
}

/* 去掉会盖住按钮中心的黑径向阴影 */
.play-btn--transport::before {
  content: none;
}

.play-btn--transport::after {
  content: none;
}

.play-btn--transport:hover::before,
.play-btn--transport:focus-visible::before,
.play-btn--transport:hover::after,
.play-btn--transport:focus-visible::after {
  content: none;
}

.play-btn--switching {
  animation: np-play-pulse 360ms cubic-bezier(0.22, 1, 0.36, 1);
}

.play-icon-inner {
  font-size: 28px;
  display: block;
  position: absolute;
  z-index: 1;
  color: inherit;

  &.spinning {
    animation: np-spin 1s linear infinite;
    color: inherit;
  }
}

@keyframes np-spin { to { transform: rotate(360deg); } }

@keyframes np-control-pulse {
  0% { transform: scale(1); }
  35% { transform: scale(0.9); }
  100% { transform: scale(1); }
}

@keyframes np-play-pulse {
  0% { transform: scale(1); }
  35% { transform: scale(1.08); }
  100% { transform: scale(1); }
}

/* 壳层/内容开合动画已禁用：仅保留 App.slide-up 的 translateY */

@keyframes np-glow-breathe {
  0% {
    opacity: 0.28;
    transform: scale(1.02);
  }
  48% {
    opacity: 0.58;
    transform: scale(1.0);
  }
  100% {
    opacity: 0.48;
    transform: scale(1);
  }
}

@keyframes np-controls-breathe {
  0% {
    transform: translateY(6px);
    opacity: 0.72;
  }
  52% {
    transform: translateY(-2px);
    opacity: 1;
  }
  100% {
    transform: translateY(0);
    opacity: 1;
  }
}

@keyframes np-controls-feedback-wave {
  0% {
    transform: scale(0.992);
    opacity: 0.88;
  }
  45% {
    transform: scale(1.008);
    opacity: 1;
  }
  100% {
    transform: scale(1);
    opacity: 1;
  }
}

@keyframes np-play-press-bounce {
  0% {
    transform: scale(1);
  }
  24% {
    transform: scale(0.9);
  }
  62% {
    transform: scale(1.08);
  }
  100% {
    transform: scale(1);
  }
}

@keyframes np-beat-shell-bloom {
  0% {
    opacity: 0.48;
    transform: scale(1);
  }
  50% {
    opacity: 0.64;
    transform: scale(1.03);
  }
  100% {
    opacity: 0.48;
    transform: scale(1);
  }
}

@keyframes np-left-beat-sway {
  0% { transform: translateY(0) scale(1); }
  50% { transform: translateY(-3px) scale(1.006); }
  100% { transform: translateY(0) scale(1); }
}

@keyframes np-cover-beat-unison {
  0% {
    transform: scale(1) translateY(0);
    filter: drop-shadow(0 16px 48px rgba(0,0,0,0.5));
  }
  50% {
    transform: scale(1.02) translateY(-4px);
    filter: drop-shadow(0 22px 60px rgba(0,0,0,0.58));
  }
  100% {
    transform: scale(1) translateY(0);
    filter: drop-shadow(0 16px 48px rgba(0,0,0,0.5));
  }
}

@keyframes np-info-beat-unison {
  0% { transform: translateY(0); opacity: 1; }
  50% { transform: translateY(-2px); opacity: 1; }
  100% { transform: translateY(0); opacity: 1; }
}

@keyframes np-slider-beat-glide {
  0% { transform: translateY(0); opacity: 1; }
  50% { transform: translateY(2px); opacity: 1; }
  100% { transform: translateY(0); opacity: 1; }
}

@keyframes np-toolbar-beat-bob {
  0% { transform: translateY(0); opacity: 1; }
  50% { transform: translateY(-2px); opacity: 1; }
  100% { transform: translateY(0); opacity: 1; }
}

@keyframes np-lyrics-beat-sway {
  0% {
    transform: translateX(0);
    opacity: 1;
  }
  50% {
    transform: translateX(-4px);
    opacity: 1;
  }
  100% {
    transform: translateX(0);
    opacity: 1;
  }
}

@keyframes np-popover-rise {
  0% {
    opacity: 0;
    transform: translateX(-50%) translateY(10px) scale(0.94);
    filter: blur(8px);
  }
  100% {
    opacity: 1;
    transform: translateX(-50%) translateY(0) scale(1);
    filter: blur(0);
  }
}

@keyframes np-toolbar-breathe {
  0% {
    opacity: 0.6;
    transform: translateY(10px);
  }
  55% {
    opacity: 1;
    transform: translateY(-1px);
  }
  100% {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes np-lyrics-fade-shift {
  0% {
    opacity: 0.56;
    transform: translateX(18px);
    filter: blur(10px);
  }
  48% {
    opacity: 1;
    transform: translateX(-4px);
    filter: blur(0);
  }
  100% {
    opacity: 1;
    transform: translateX(0);
    filter: blur(0);
  }
}

/* 播放/暂停图标切换动画（同时进出，绝对定位重叠） */
.play-icon-enter-active {
  transition: transform 200ms var(--ease-decelerate), opacity 150ms var(--ease-decelerate);
}
.play-icon-leave-active {
  transition: transform 120ms var(--ease-accelerate), opacity 80ms var(--ease-accelerate);
}
.play-icon-enter-from { transform: scale(0.5); opacity: 0; }
.play-icon-leave-to { transform: scale(0.5); opacity: 0; }

.np-cover-swap-enter-active,
.np-cover-swap-leave-active {
  position: absolute;
  inset: 0;
  transition: opacity 280ms ease, transform 520ms cubic-bezier(0.22, 1, 0.36, 1), filter 520ms cubic-bezier(0.22, 1, 0.36, 1);
}
.np-cover-swap-enter-from,
.np-cover-swap-leave-to {
  opacity: 0;
  transform: scale(0.965);
  filter: saturate(0.94) blur(1px);
}

.np-meta-swap-enter-active,
.np-meta-swap-leave-active {
  transition: opacity 200ms ease;
}
.np-meta-swap-enter-from,
.np-meta-swap-leave-to {
  opacity: 0;
  /* 不再上下位移，避免标题区看起来「跳一大截」 */
  transform: none;
  filter: none;
}

.np-cover-static-enter-active,
.np-cover-static-leave-active {
  transition: none;
}

.np-cover-flow-prev-enter-active,
.np-cover-flow-prev-leave-active,
.np-cover-flow-next-enter-active,
.np-cover-flow-next-leave-active {
  position: absolute;
  inset: 0;
  transition:
    opacity 300ms ease,
    transform 620ms cubic-bezier(0.22, 1, 0.36, 1),
    filter 620ms cubic-bezier(0.22, 1, 0.36, 1);
}

.np-cover-flow-prev-enter-from {
  opacity: 0;
  transform: translateX(-22px) scale(0.972);
  filter: saturate(0.92) blur(2px);
}

.np-cover-flow-prev-leave-to {
  opacity: 0;
  transform: translateX(18px) scale(1.022);
  filter: saturate(1.08) blur(4px);
}

.np-cover-flow-next-enter-from {
  opacity: 0;
  transform: translateX(22px) scale(0.972);
  filter: saturate(0.92) blur(2px);
}

.np-cover-flow-next-leave-to {
  opacity: 0;
  transform: translateX(-18px) scale(1.022);
  filter: saturate(1.08) blur(4px);
}

/* 切歌标题：绝对叠层交叉淡入，不做位移，避免整列重排 */
.np-meta-flow-prev-enter-active,
.np-meta-flow-prev-leave-active,
.np-meta-flow-next-enter-active,
.np-meta-flow-next-leave-active,
.np-meta-static-enter-active,
.np-meta-static-leave-active {
  transition: opacity 220ms ease;
}

.np-meta-flow-prev-enter-from,
.np-meta-flow-prev-leave-to,
.np-meta-flow-next-enter-from,
.np-meta-flow-next-leave-to,
.np-meta-static-enter-from,
.np-meta-static-leave-to {
  opacity: 0;
  transform: none;
  filter: none;
}

.np-meta-flow-prev-leave-active,
.np-meta-flow-next-leave-active,
.np-meta-static-leave-active {
  position: absolute;
  inset: 0;
}

.np-detail-swap-enter-active,
.np-detail-swap-leave-active {
  transition: opacity 160ms ease;
}

.np-detail-swap-enter-from,
.np-detail-swap-leave-to {
  opacity: 0;
  transform: none;
}

/* 工具栏：切歌时不上下呼吸，避免整列位移 */
.np-toolbar {
  position: relative;
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  width: 100%;
  margin-top: 0;
  flex-shrink: 0;
  transition: opacity 240ms ease;
}

.np-toolbar--switching,
.np-toolbar--beat-active {
  animation: none;
}

.tool-btn {
  width: 42px;
  height: 42px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgba(255,255,255,0.72);
  position: relative;
  overflow: hidden;
  transition: color 150ms, background 150ms, transform 180ms cubic-bezier(0.22, 1, 0.36, 1), box-shadow 220ms cubic-bezier(0.22, 1, 0.36, 1);

  .material-symbols-rounded { font-size: 20px; }

  &:hover { background: rgba(255,255,255,0.10); color: rgba(255,255,255,0.87); }
  &.active { color: var(--np-primary, var(--md-primary-container, #E8DEF8)); }
  &.disabled { opacity: 0.38; cursor: default; }
  &:disabled { opacity: 0.38; cursor: default; }
  &:disabled:hover { background: transparent; color: rgba(255,255,255,0.72); }
  &:active { transform: scale(0.88); }
}

.tool-btn--feedback::after {
  content: '';
  position: absolute;
  inset: 10px;
  border-radius: 999px;
  background: radial-gradient(circle, rgba(255,255,255,0.22) 0%, transparent 72%);
  opacity: 0;
  transform: scale(0.6);
  transition: opacity 200ms ease, transform 260ms cubic-bezier(0.22, 1, 0.36, 1);
}

.tool-btn--feedback:hover::after,
.tool-btn--feedback:focus-visible::after {
  opacity: 1;
  transform: scale(1);
}

// 收藏按钮活跃态保持更鲜明的红色
.fav-btn.active {
  color: #FF3B30 !important;
  text-shadow: 0 0 18px rgba(255, 59, 48, 0.28);
}

.np-favorite-swap-enter-active,
.np-favorite-swap-leave-active {
  transition: opacity 180ms ease, transform 240ms cubic-bezier(0.22, 1, 0.36, 1);
}

.np-favorite-swap-enter-from {
  opacity: 0;
  transform: scale(0.72);
}

.np-favorite-swap-leave-to {
  opacity: 0;
  transform: scale(1.2);
}

/* 来源徽章（对齐 Android PlaybackSourceBadge） */
.source-badge {
  position: absolute;
  bottom: 10px;
  right: 10px;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px 4px 7px;
  border-radius: 20px;
  background: rgba(0, 0, 0, 0.55);
  backdrop-filter: blur(12px);
  border: 1px solid rgba(255, 255, 255, 0.1);
}

.source-badge-icon {
  font-size: 14px;
  color: rgba(255, 255, 255, 0.7);
}

.source-badge-icon--netease {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  background: rgba(255, 255, 255, 0.7);
  mask: url('/icons/ic_netease.svg') center / contain no-repeat;
}

.source-badge-label {
  font-size: 11px;
  font-weight: 600;
  color: rgba(255, 255, 255, 0.8);
  letter-spacing: 0.3px;
}

@keyframes badge-in {
  from { opacity: 0; transform: scale(0.7); }
  to { opacity: 1; transform: scale(1); }
}

.np-badge-swap-enter-active,
.np-badge-swap-leave-active {
  transition: opacity 220ms ease, transform 300ms cubic-bezier(0.22, 1, 0.36, 1);
}

.np-badge-swap-enter-from {
  opacity: 0;
  transform: translateY(8px) scale(0.9);
}

.np-badge-swap-leave-to {
  opacity: 0;
  transform: translateY(-8px) scale(0.9);
}

/* 音量控制 */
.volume-wrap {
  position: relative;
}

.volume-popover {
  position: absolute;
  bottom: 46px;
  left: 50%;
  transform: translateX(-50%);
  background: rgba(30, 28, 34, 0.95);
  backdrop-filter: blur(20px);
  border-radius: 14px;
  box-sizing: border-box;
  width: 58px;
  min-width: 58px;
  max-width: 58px;
  padding: 16px 10px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  box-shadow: 0 8px 32px rgba(0,0,0,0.5);
  border: 1px solid rgba(255,255,255,0.08);
  z-index: 10;
}

.np-floating-popover {
  position: absolute;
  overflow: hidden;
  isolation: isolate;
  /* 统一毛玻璃：半透明底 + 强模糊 */
  background: rgba(22, 20, 26, 0.62);
  backdrop-filter: blur(28px) saturate(1.15);
  -webkit-backdrop-filter: blur(28px) saturate(1.15);
  border-color: color-mix(in srgb, var(--np-primary-container, rgba(255,255,255,0.14)) 26%, rgba(255,255,255,0.08));
  box-shadow:
    0 14px 40px rgba(0,0,0,0.46),
    0 0 0 1px rgba(255,255,255,0.03) inset;
  animation: np-popover-rise 260ms cubic-bezier(0.22, 1, 0.36, 1);
}

.np-floating-popover::before {
  content: '';
  position: absolute;
  inset: 0;
  background:
    radial-gradient(circle at 18% 14%, color-mix(in srgb, var(--np-primary) 16%, transparent), transparent 34%),
    linear-gradient(180deg, rgba(255,255,255,0.05), transparent 28%);
  opacity: 0.95;
  pointer-events: none;
  z-index: -1;
}

.np-floating-popover--audiofx::before {
  background:
    radial-gradient(circle at 16% 12%, color-mix(in srgb, var(--np-primary-container) 22%, transparent), transparent 36%),
    radial-gradient(circle at 82% 88%, color-mix(in srgb, var(--np-primary) 10%, transparent), transparent 40%),
    linear-gradient(180deg, rgba(255,255,255,0.05), transparent 24%);
}

.volume-slider {
  writing-mode: vertical-lr;
  appearance: none;
  width: 4px;
  height: 100px;
  background: rgba(255,255,255,0.15);
  border-radius: 2px;
  outline: none;
  cursor: pointer;
  accent-color: var(--np-primary, #fff);

  &::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--np-primary-container, white);
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
    cursor: pointer;
  }
}

.volume-label {
  display: block;
  width: 42px;
  font-size: 11px;
  font-weight: 600;
  color: color-mix(in srgb, var(--np-primary-container, rgba(255,255,255,0.7)) 56%, rgba(255,255,255,0.4));
  font-variant-numeric: tabular-nums;
  text-align: center;
  white-space: nowrap;
}

/* 播放速度 / 音效面板 */
.speed-wrap, .sleep-wrap {
  position: relative;
}

.sleep-wrap .tool-btn {
  overflow: visible;
}

.speed-label {
  font-size: 12px;
  font-weight: 700;
  letter-spacing: -0.3px;
}

.audiofx-popover {
  position: absolute;
  bottom: 46px;
  left: 50%;
  transform: translateX(-50%);
  background: rgba(22, 20, 26, 0.72);
  backdrop-filter: blur(28px) saturate(1.15);
  -webkit-backdrop-filter: blur(28px) saturate(1.15);
  border-radius: 16px;
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  box-shadow: 0 8px 40px rgba(0,0,0,0.6);
  border: 1px solid rgba(255,255,255,0.08);
  z-index: 10;
  min-width: 300px;
  max-height: 480px;
  overflow-y: auto;

  &::-webkit-scrollbar { width: 0; height: 0; display: none; }
}

.audiofx-section :deep(.custom-select) {
  width: 100%;
}

.audiofx-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.audiofx-section-header {
  font-size: 12px;
  font-weight: 600;
  color: rgba(255,255,255,0.5);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.audiofx-speed-grid, .audiofx-preset-row {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.audiofx-speed-grid .speed-option,
.audiofx-preset-row .speed-option {
  padding: 5px 10px;
  font-size: 12px;
  min-width: unset;
  flex: 0 0 auto;
}

.audiofx-slider-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.audiofx-slider-label {
  font-size: 10px;
  color: rgba(255,255,255,0.35);
  min-width: 28px;
  text-align: center;
}

.audiofx-slider-value {
  font-size: 11px;
  font-weight: 600;
  color: var(--md-primary, #D0BCFF);
  min-width: 42px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.audiofx-slider {
  flex: 1;
  appearance: none;
  height: 4px;
  background: rgba(255,255,255,0.15);
  border-radius: 2px;
  outline: none;
  cursor: pointer;

  &::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--np-primary, var(--md-primary, #D0BCFF));
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
  }

  &::-moz-range-thumb {
    width: 14px;
    height: 14px;
    border: none;
    border-radius: 50%;
    background: var(--np-primary, var(--md-primary, #D0BCFF));
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
  }
}

.audiofx-toggle {
  position: relative;
  display: inline-block;
  width: 34px;
  height: 18px;

  input { opacity: 0; width: 0; height: 0; }

  .audiofx-toggle-slider {
    position: absolute;
    cursor: pointer;
    inset: 0;
    background: rgba(255,255,255,0.15);
    border-radius: 18px;
    transition: 0.2s;

    &::before {
      content: '';
      position: absolute;
      height: 14px;
      width: 14px;
      left: 2px;
      bottom: 2px;
      background: white;
      border-radius: 50%;
      transition: 0.2s;
    }
  }

  input:checked + .audiofx-toggle-slider {
    background: var(--md-primary, #D0BCFF);
  }

  input:checked + .audiofx-toggle-slider::before {
    transform: translateX(16px);
  }
}

.audiofx-eq-bands {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 4px;
  padding: 4px 0;
}

.audiofx-eq-band {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  flex: 1;
}

.audiofx-eq-val {
  font-size: 10px;
  color: var(--np-primary, var(--md-primary, #D0BCFF));
  font-variant-numeric: tabular-nums;
  min-height: 14px;
}

.audiofx-eq-freq {
  font-size: 9px;
  color: rgba(255,255,255,0.35);
}

.audiofx-eq-slider {
  writing-mode: vertical-lr;
  direction: rtl;
  appearance: none;
  width: 4px;
  height: 80px;
  background: rgba(255,255,255,0.15);
  border-radius: 2px;
  cursor: pointer;
  outline: none;

  &::-webkit-slider-thumb {
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--np-primary, var(--md-primary, #D0BCFF));
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
  }

  &::-moz-range-thumb {
    width: 12px;
    height: 12px;
    border: none;
    border-radius: 50%;
    background: var(--np-primary, var(--md-primary, #D0BCFF));
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
  }
}

.audiofx-reset {
  width: 100%;
  text-align: center;
  color: #EF5350 !important;
  border-top: 1px solid rgba(255,255,255,0.06);
  margin-top: 2px;
  padding-top: 10px;
}

.speed-popover, .sleep-popover {
  position: absolute;
  bottom: 46px;
  left: 50%;
  transform: translateX(-50%);
  background: rgba(30, 28, 34, 0.95);
  backdrop-filter: blur(20px);
  border-radius: 14px;
  padding: 8px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  box-shadow: 0 8px 32px rgba(0,0,0,0.5);
  border: 1px solid rgba(255,255,255,0.08);
  z-index: 10;
  min-width: 120px;
}

.speed-option, .sleep-option {
  padding: 8px 16px;
  border: none;
  background: transparent;
  color: rgba(255,255,255,0.7);
  font-size: 13px;
  cursor: pointer;
  border-radius: 8px;
  text-align: left;
  white-space: nowrap;
  transition: all 0.15s;

  &:hover {
    background: color-mix(in srgb, var(--np-primary-container, rgba(255,255,255,0.12)) 16%, rgba(255,255,255,0.08));
    color: white;
  }

  &.active {
    color: var(--np-primary, var(--md-primary, #D0BCFF));
    font-weight: 600;
  }

  &.cancel {
    color: #EF5350;
    border-top: 1px solid rgba(255,255,255,0.06);
    margin-top: 4px;
    padding-top: 10px;
  }
}

.sleep-badge {
  position: absolute;
  top: -2px;
  right: -4px;
  z-index: 2;
  min-width: 22px;
  height: 14px;
  line-height: 14px;
  font-size: 9px;
  font-weight: 700;
  background: var(--md-primary, #D0BCFF);
  /* 定时器倒计时文字白色 */
  color: #fff;
  padding: 0 4px;
  border-radius: 999px;
  font-variant-numeric: tabular-nums;
  text-align: center;
  pointer-events: none;
}

/* 歌词空状态 */
.lyrics-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 100%;
  gap: 10px;
  color: rgba(255,255,255,0.18);
  font-size: 14px;
  font-weight: 500;
  transition: transform 320ms cubic-bezier(0.22, 1, 0.36, 1), opacity 240ms ease;
}

.lyrics-empty .spinning {
  animation: np-spin 1s linear infinite;
}

.np-right--switching {
  animation: np-lyrics-fade-shift 520ms cubic-bezier(0.22, 1, 0.36, 1);
}

.np-right--beat-active {
  animation: np-lyrics-beat-sway 320ms cubic-bezier(0.22, 1, 0.36, 1);
}

@media (prefers-reduced-motion: reduce) {
  .now-playing,
  .now-playing *,
  .now-playing *::before,
  .now-playing *::after {
    animation: none !important;
    animation-iteration-count: 1 !important;
    transition-duration: 1ms !important;
    transition-delay: 0ms !important;
    scroll-behavior: auto !important;
  }

  .now-playing .spinning {
    animation: np-spin 1s linear infinite !important;
  }

  .np-body.np-body--lyrics-mode .np-left {
    transform: none;
  }
}
</style>

<style lang="scss">
/* 更多选项面板：Overlay + Sheet 过渡 */
.more-sheet-enter-active {
  transition: opacity 280ms cubic-bezier(0.2, 0, 0, 1);
  .np-more-sheet {
    transition: opacity 280ms cubic-bezier(0.2, 0, 0, 1), transform 280ms cubic-bezier(0.2, 0, 0, 1);
  }
}
.more-sheet-leave-active {
  transition: opacity 200ms cubic-bezier(0.2, 0, 0, 1);
  .np-more-sheet {
    transition: opacity 200ms cubic-bezier(0.2, 0, 0, 1), transform 200ms cubic-bezier(0.2, 0, 0, 1);
  }
}
.more-sheet-enter-from,
.more-sheet-leave-to {
  opacity: 0;
  .np-more-sheet {
    opacity: 0;
    transform: scale(0.92) translateY(16px);
  }
}

/* 子视图滑动过渡 */
.slide-left-enter-active,
.slide-left-leave-active,
.slide-right-enter-active,
.slide-right-leave-active {
  transition: transform 180ms cubic-bezier(0.2, 0, 0, 1), opacity 180ms cubic-bezier(0.2, 0, 0, 1);
}
.slide-left-enter-from { transform: translateX(24px); opacity: 0; }
.slide-left-leave-to   { transform: translateX(-24px); opacity: 0; }
.slide-right-enter-from { transform: translateX(-24px); opacity: 0; }
.slide-right-leave-to   { transform: translateX(24px); opacity: 0; }

.np-more-overlay {
  position: fixed;
  inset: 0;
  z-index: 9000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  /* 高斯模糊遮罩，避免生硬压暗 */
  background: rgba(8, 8, 12, 0.28);
  backdrop-filter: blur(22px) saturate(1.08);
  -webkit-backdrop-filter: blur(22px) saturate(1.08);
  border-radius: var(--radius-lg);
  overflow: hidden;
  clip-path: inset(0 round var(--radius-lg));
}

.np-more-sheet {
  width: min(380px, 100%);
  max-width: calc(100vw - 32px);
  max-height: min(80vh, calc(100vh - 32px));
  /* 隐藏滚动条，仍允许内容滚动 */
  overflow-y: auto;
  scrollbar-width: none;
  -ms-overflow-style: none;
  background: rgba(30, 28, 34, 0.88);
  backdrop-filter: blur(28px) saturate(1.12);
  -webkit-backdrop-filter: blur(28px) saturate(1.12);
  border-radius: 24px;
  padding: 24px;
  box-shadow: 0 16px 48px rgba(0,0,0,0.5);
  border: 1px solid rgba(255,255,255,0.06);

  /* 隐藏滚动条 */
  &::-webkit-scrollbar { width: 0; height: 0; display: none; }
}

.np-more-sheet-content {
  min-height: 0;
}

.np-more-title {
  font-size: 18px;
  font-weight: 600;
  color: rgba(255,255,255,0.9);
  margin: 0 0 16px;
}

// 子视图 header（返回按钮 + 标题）
.np-more-sub-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid rgba(255,255,255,0.06);

  .np-more-title { margin: 0; }
}

.np-more-back {
  width: 36px;
  height: 36px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgba(255,255,255,0.7);
  transition: background 0.15s;
  &:hover { background: rgba(255,255,255,0.08); }
}

// Android 风格列表项
.np-more-list-item {
  display: flex;
  align-items: center;
  gap: 16px;
  width: 100%;
  min-width: 0;
  margin-top: 6px;
  padding: 13px 8px;
  border: 1px solid rgba(255,255,255,0.06);
  background: rgba(255,255,255,0.025);
  color: rgba(255,255,255,0.85);
  cursor: pointer;
  border-radius: var(--radius-md);
  transition: background 0.15s, transform 0.15s cubic-bezier(0.2, 0, 0, 1);

  &:hover {
    background: rgba(255,255,255,0.08);
    transform: translateX(2px);
    .np-more-chevron { color: rgba(255,255,255,0.45) !important; }
  }

  > .material-symbols-rounded:first-child {
    font-size: 20px;
    color: rgba(255,255,255,0.6);
    flex-shrink: 0;
    width: 36px;
    height: 36px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    background: rgba(255,255,255,0.07);
  }
}

.np-more-list-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  text-align: left;
}

.np-more-list-headline {
  font-size: 15px;
  font-weight: 500;
}

.np-more-list-desc {
  font-size: 12px;
  color: rgba(255,255,255,0.4);
}

.np-more-chevron {
  font-size: 20px !important;
  color: rgba(255,255,255,0.25) !important;
  transition: color 0.15s;
}

// 速度选择网格
.np-more-speed-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 8px;
  padding: 8px 0;
}

.np-more-speed-btn {
  padding: 12px;
  border: none;
  border-radius: 12px;
  background: rgba(255,255,255,0.06);
  color: rgba(255,255,255,0.7);
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;

  &:hover { background: rgba(255,255,255,0.10); }
  &.active {
    background: var(--md-primary-container, #E8DEF8);
    color: var(--md-on-primary-container, #1D192B);
  }
}

.np-more-item {
  margin-bottom: 20px;
}

.np-more-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 500;
  color: rgba(255,255,255,0.7);
  margin-bottom: 10px;
}

.np-more-hint {
  font-size: 12px;
  color: rgba(255,255,255,0.5);
  margin-top: -4px;
  margin-bottom: 12px;
}

.np-more-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.np-more-slider {
  flex: 1;
  appearance: none;
  height: 4px;
  background: rgba(255,255,255,0.15);
  border-radius: 2px;
  outline: none;
  cursor: pointer;

  &::-webkit-slider-thumb {
    appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--md-primary, #D0BCFF);
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
  }
}

.np-offset-value {
  font-size: 12px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  color: rgba(255,255,255,0.5);
  min-width: 60px;
  text-align: right;

  &.positive { color: #66BB6A; }
  &.negative { color: #EF5350; }
}

.np-more-preview {
  margin-top: 8px;
  font-weight: 700;
  color: rgba(255,255,255,0.5);
  line-height: 1.4;
  transition: font-size 0.15s;
}

// 搜索栏
.np-more-search-bar {
  display: flex;
  gap: 8px;
  margin-bottom: 12px;
}

.np-more-input {
  flex: 1;
  padding: 10px 14px;
  border: 1px solid rgba(255,255,255,0.12);
  border-radius: 12px;
  background: rgba(255,255,255,0.06);
  color: white;
  font-size: 14px;
  outline: none;
  transition: border-color 0.15s;

  &:focus { border-color: rgba(255,255,255,0.3); }
  &::placeholder { color: rgba(255,255,255,0.3); }
}

.np-more-search-btn {
  width: 42px;
  height: 42px;
  border-radius: 12px;
  background: rgba(255,255,255,0.08);
  color: rgba(255,255,255,0.7);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: background 0.15s;

  &:hover { background: rgba(255,255,255,0.14); }
  &:disabled { opacity: 0.4; }
}

.np-more-status {
  text-align: center;
  color: rgba(255,255,255,0.35);
  font-size: 13px;
  padding: 16px 0;
}

// 搜索结果列表
.np-more-search-results {
  max-height: 300px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  &::-webkit-scrollbar { display: none; }
}

.np-more-search-item {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  padding: 10px 4px;
  border: none;
  background: transparent;
  color: rgba(255,255,255,0.85);
  cursor: pointer;
  border-radius: 10px;
  transition: background 0.15s;
  text-align: left;

  &:hover { background: rgba(255,255,255,0.06); }

  &.active {
    background: rgba(255,255,255,0.10);
    outline: 1px solid rgba(255,255,255,0.12);
  }
}

.np-more-search-cover {
  width: 40px;
  height: 40px;
  border-radius: 6px;
  object-fit: cover;
  flex-shrink: 0;
  background: rgba(255,255,255,0.06);
}

.np-more-search-info {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.np-more-search-title {
  font-size: 14px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.np-more-search-artist {
  font-size: 12px;
  color: rgba(255,255,255,0.4);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.np-more-search-source {
  font-size: 10px;
  font-weight: 600;
  color: rgba(255,255,255,0.3);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  flex-shrink: 0;
}

// 编辑表单
.np-more-form {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.np-more-form-label {
  font-size: 12px;
  font-weight: 600;
  color: rgba(255,255,255,0.45);
  margin-top: 4px;
}

.np-more-form-actions {
  display: flex;
  gap: 8px;
  margin-top: 12px;

  &.compact {
    margin-top: 10px;
  }
}

.np-more-form-btn {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 10px;
  border: none;
  border-radius: 12px;
  background: rgba(255,255,255,0.08);
  color: rgba(255,255,255,0.7);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: background 0.15s;

  .material-symbols-rounded { font-size: 18px; }

  &:hover { background: rgba(255,255,255,0.12); }

  &.primary {
    background: var(--md-primary-container, #E8DEF8);
    color: var(--md-on-primary-container, #1D192B);
    &:hover { opacity: 0.9; }
  }
}

.np-more-field-picker {
  margin-top: 12px;
  padding: 12px;
  border-radius: 16px;
  background:
    linear-gradient(135deg, rgba(255,255,255,0.10), rgba(255,255,255,0.04));
  border: 1px solid rgba(255,255,255,0.10);
}

.np-more-candidate-preview {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
}

.np-more-candidate-cover {
  width: 46px;
  height: 46px;
  border-radius: 10px;
  object-fit: cover;
  background: rgba(255,255,255,0.08);
  flex-shrink: 0;
}

.np-more-field-title {
  margin-bottom: 8px;
  color: rgba(255,255,255,0.48);
  font-size: 12px;
  font-weight: 700;
}

.np-more-field-options {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.np-more-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 10px;
  border-radius: 999px;
  background: rgba(255,255,255,0.07);
  color: rgba(255,255,255,0.72);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;

  input {
    accent-color: var(--md-primary, #D0BCFF);
  }
}

.np-more-segmented {
  display: flex;
  padding: 3px;
  border-radius: 14px;
  background: rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.08);

  button {
    flex: 1;
    padding: 8px 10px;
    border: none;
    border-radius: 11px;
    background: transparent;
    color: rgba(255,255,255,0.50);
    font-size: 13px;
    font-weight: 700;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;

    &.active {
      background: rgba(255,255,255,0.14);
      color: rgba(255,255,255,0.90);
    }
  }

  &.platform {
    margin: -2px 0 12px;

    button {
      font-size: 12px;
      padding: 7px 8px;
    }
  }
}

.np-track-detail-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.np-track-detail-hero {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px;
  margin-bottom: 4px;
  border-radius: 18px;
  background: rgba(255,255,255,0.07);
  border: 1px solid rgba(255,255,255,0.08);
}

.np-track-detail-cover {
  width: 58px;
  height: 58px;
  border-radius: 14px;
  object-fit: cover;
  background: rgba(255,255,255,0.08);
}

.np-track-detail-heading {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;

  strong,
  span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  strong {
    color: rgba(255,255,255,0.92);
    font-size: 15px;
  }

  span {
    color: rgba(255,255,255,0.45);
    font-size: 12px;
  }
}

.np-track-detail-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  width: 100%;
  padding: 10px 4px;
  border: none;
  border-bottom: 1px solid rgba(255,255,255,0.05);
  background: transparent;
  text-align: left;

  span {
    color: rgba(255,255,255,0.42);
    font-size: 12px;
    flex-shrink: 0;
  }

  strong {
    min-width: 0;
    color: rgba(255,255,255,0.78);
    font-size: 12px;
    font-weight: 650;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  &.copyable {
    cursor: pointer;
    border-radius: 10px;
    transition: background 0.15s;

    &:hover {
      background: rgba(255,255,255,0.06);
    }
  }
}

.np-track-detail-share {
  width: 100%;
  margin-top: 8px;
  flex: none;
}

// 音质列表
.np-more-quality-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

// 歌词编辑器
.np-lyrics-editor {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.np-lyrics-textarea {
  width: 100%;
  min-height: 200px;
  max-height: 320px;
  padding: 12px 14px;
  border: 1px solid rgba(255,255,255,0.12);
  border-radius: 12px;
  background: rgba(255,255,255,0.04);
  color: rgba(255,255,255,0.85);
  font-size: 13px;
  font-family: 'Cascadia Code', 'JetBrains Mono', 'Fira Code', monospace;
  line-height: 1.6;
  resize: vertical;
  outline: none;
  transition: border-color 0.15s;

  &:focus { border-color: rgba(255,255,255,0.3); }
  &::placeholder { color: rgba(255,255,255,0.2); }
  &::-webkit-scrollbar { width: 4px; }
  &::-webkit-scrollbar-thumb {
    background: rgba(255,255,255,0.12);
    border-radius: 2px;
  }
}

.np-more-quality-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  padding: 14px 12px;
  border: none;
  background: transparent;
  color: rgba(255,255,255,0.7);
  font-size: 15px;
  cursor: pointer;
  border-radius: 10px;
  transition: background 0.15s;

  &:hover { background: rgba(255,255,255,0.06); }

  &.active {
    color: var(--md-primary-container, #E8DEF8);
    font-weight: 600;
  }
}

.np-lt-panel {
  right: 24px;
  bottom: 92px;
  z-index: 9100;
  border-radius: 20px;
  box-shadow: 0 20px 56px rgba(0,0,0,0.35);
}
</style>
