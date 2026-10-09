<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onUnmounted } from 'vue'
import { usePlayerStore, displayAlbum, type AudioInfo, type LyricLine, type TrackInfo } from '@/stores/player'
import { useLikedSongsStore } from '@/stores/likedSongs'
import {
  COVER_BLUR_PX_PER_UNIT,
  LYRIC_FONT_SCALE_MAX,
  LYRIC_FONT_SCALE_MIN,
  LYRIC_FONT_SCALE_STEP,
  useSettingsStore,
} from '@/stores/settings'
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
import { hasLyricsRequestInFlight, hasWordTimedLyrics, loadLyricsSingleFlight } from '@/modules/lyrics/lyricsRequest'
import {
  toEditableLyricsText,
  toEditableTranslationText,
  toEditableRomanizationText,
  resolveStoredLyricStateFromPayload,
  resolveStoredTranslatedLyricStateFromPayload,
  resolveStoredRomanizedLyricStateFromPayload,
  materializeStoredLyrics,
  withUpdatedLyricsPayload,
  mapBackendLyrics as mapBackendLyricsShared,
  mergeParsedLyricsWithTranslations,
  mergeParsedLyricsWithRomanization,
  mergeWordTimedLyricsWithBaseline,
  resolveKnownNeteaseLyricSongId,
  shouldBackfillNeteaseRomanization,
} from '@/modules/lyrics/lyricsFormat'
import { lyricMatchSourceTag, type LyricMatchResult } from '@/modules/lyrics/lyricMatch'
import {
  formatLyricOffsetMs,
  MAX_LYRIC_DEFAULT_OFFSET_MS,
  MIN_LYRIC_DEFAULT_OFFSET_MS,
  LYRIC_OFFSET_STEP_MS,
  normalizeLyricSource,
  readSyncedLyricSource,
} from '@/modules/lyrics/lyricOffset'
import {
  fetchAutomaticLyrics,
  fetchLyrics,
  fetchNeteaseRomanization,
  fetchPreferredSourceLyrics,
  fetchWordTimedLyrics,
  preferredLyricMatchSource,
} from '@/modules/lyrics/lyricsFetch'
import { lyricSourceOf, rememberLyricSource } from '@/modules/lyrics/lyricSource'
import { isEditableTarget, isMacPlatform } from '@/modules/shortcuts/platform'
import {
  persistTrackSyncPayload,
  recordLyricOverride,
  withUpdatedCustomInfoPayload,
} from '@/modules/lyrics/syncTrackPayload'
import { useLyricOffsetStore } from '@/stores/lyricOffset'
import HyperBackground from './HyperBackground.vue'
import CoverBlurBackground from './CoverBlurBackground.vue'
import BilibiliCoverImage from './BilibiliCoverImage.vue'
import WaveformSlider from './WaveformSlider.vue'
import LyricsView from './LyricsView.vue'
import QueuePanel from './QueuePanel.vue'
import AddToPlaylistDialog from './AddToPlaylistDialog.vue'
import ListenTogetherPanel from './ListenTogetherPanel.vue'
import EditableRangeValue from './ui/EditableRangeValue.vue'
import AudioEffectsPanel from './AudioEffectsPanel.vue'
import LyricsMatchPanel from './LyricsMatchPanel.vue'
import ContextMenu from './ui/ContextMenu.vue'
import type { ContextMenuActionItem } from '@/utils/contextMenu'
import { playbackSessionTrackKey } from '@/modules/playback/playbackRequest'
import { createLogger } from '@/utils/logger'
import { getTrackCoverUrl } from '@/utils/trackCover'
import { summarizeLogError } from '@/utils/logSanitizer'
import { neteaseSongArtists } from '@/modules/library/artistNavigation'
import { splitArtistNames } from '@/modules/library/localArtists'
import { closeDesktopLyricsWindow, desktopLyricsOpen, openDesktopLyricsWindow } from '@/modules/desktopLyrics/bridge'
import { normalizeDesktopLyricsStyle } from '@/modules/desktopLyrics/style'
import { getPlaybackSourceKind } from '@/modules/playback/playbackSource'
import { usePlaybackAudioInfoDisplay } from '@/composables/usePlaybackAudioInfoDisplay'
import {
  actualAudioBitrateLabel,
  actualAudioParameterLabels,
  canSwitchAudioQuality,
  isLocalAudioPlayback,
  resolveAudioQualityLabel,
} from '@/modules/playback/audioQualityDisplay'

const log = createLogger('now-playing')

const emit = defineEmits<{ collapse: [] }>()
const props = defineProps<{
  hideHeader?: boolean
  transitionState?: 'opening' | 'closing' | null
}>()
const player = usePlayerStore()
const likedSongs = useLikedSongsStore()
const settings = useSettingsStore()
const toast = useToastStore()
const downloadStore = useDownloadStore()
const lyricOffsetStore = useLyricOffsetStore()
const router = useRouter()
const { t } = useI18n()
async function toggleDesktopLyrics() {
  try {
    if (desktopLyricsOpen.value) {
      await closeDesktopLyricsWindow()
    } else {
      await openDesktopLyricsWindow()
      hideMoreSheet()
    }
  } catch (error) {
    log.warn('desktop lyrics window failed:', summarizeLogError(error))
    toast.error(t('player.desktop_lyrics_failed'))
  }
}

function toggleDesktopLyricsLock() {
  settings.desktopLyrics = normalizeDesktopLyricsStyle({ ...settings.desktopLyrics, locked: !settings.desktopLyrics.locked })
}
const playViewMode = ref<'cover' | 'lyrics'>('cover')
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
  try {
    const lines = await materializeStoredLyrics(
      payload,
      async (content, part) => mapBackendLyrics(await invoke<any[]>('parse_lrc_content', part === 'original'
        ? { content, title: track.title, artist: track.artist }
        : { content })),
      error => log.warn('Parse synced translation or romanization failed:', error),
    )
    if (lines?.length) rememberLyricSource(track, readSyncedLyricSource(payload))
    return lines
  } catch (e) {
    log.warn('Materialize synced lyrics failed:', e)
    return []
  }
}

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
) {
  const track = player.currentTrack
  if (!track) return
  const nextPayload = withUpdatedLyricsPayload(
    track.syncPayload,
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
  const updatedTrack = player.currentTrack
  if (updatedTrack) {
    await persistTrackSyncPayload(updatedTrack)
    await recordLyricOverride(updatedTrack)
  }
}

// 歌词编辑器（对齐 Android LyricsEditorSheet：原文 / 翻译 / 音译三轨 + 匹配）
const lyricsEditorText = ref('')
const lyricsTranslationEditorText = ref('')
const lyricsRomanizationEditorText = ref('')
/** 打开编辑器时音译页的初始内容：一开始就空、用户也没动过时，应用不能把音译记成「有意清空」 */
const lyricsRomanizationEditorInitial = ref('')
let lyricsEditorSession = 0
const lyricsEditorTab = ref<'original' | 'translation' | 'romanization'>('original')
/** 从「编辑歌曲信息」进来时返回那里 */
const lyricsEditorReturnView = ref<'main' | 'editinfo'>('main')
/** 编辑器里填的是匹配来的歌词时记下来源，应用后默认偏移量按它算 */
const lyricsEditorSource = ref<string | null>(null)

function storedOrDisplayed(
  state: ReturnType<typeof resolveStoredLyricStateFromPayload>,
  lines: LyricLine[],
  exportText: (lines: LyricLine[]) => string,
) {
  // 优先使用 syncPayload 原文(保持用户编辑/YRC 源文本), 否则从当前展示行导出
  if (state.kind === 'present') return state.text
  return lines.length > 0 ? exportText(lines) : ''
}

function openLyricsEditor(returnView: 'main' | 'editinfo' = 'main') {
  const payload = player.currentTrack?.syncPayload
  const lines = displayLyrics.value
  lyricsEditorTab.value = 'original'
  lyricsEditorReturnView.value = returnView
  lyricsEditorSource.value = null
  lyricsEditorText.value = storedOrDisplayed(resolveStoredLyricStateFromPayload(payload), lines, toEditableLyricsText)
  lyricsTranslationEditorText.value = storedOrDisplayed(
    resolveStoredTranslatedLyricStateFromPayload(payload), lines, toEditableTranslationText)
  lyricsRomanizationEditorText.value = storedOrDisplayed(
    resolveStoredRomanizedLyricStateFromPayload(payload), lines, toEditableRomanizationText)
  lyricsRomanizationEditorInitial.value = lyricsRomanizationEditorText.value
  const session = ++lyricsEditorSession
  if (!lyricsRomanizationEditorText.value.trim()) void prefillEditorRomanization(session)
  goToSubView('lyrics-editor')
}

// 音译页为空时从网易云取音译填进去（Android 编辑器同样带上已加载的音译）
async function prefillEditorRomanization(session: number) {
  const track = player.currentTrack
  if (!track) return
  try {
    const text = await fetchNeteaseRomanization(track, resolveKnownNeteaseLyricSongId(track))
    const stillEditing = session === lyricsEditorSession && player.currentTrack?.id === track.id
      && moreSheetView.value === 'lyrics-editor'
    if (!text || !stillEditing || lyricsRomanizationEditorText.value.trim()) return
    lyricsRomanizationEditorText.value = text
    lyricsRomanizationEditorInitial.value = text
  } catch (error) {
    log.warn('editor romanization prefill unavailable:', summarizeLogError(error))
  }
}

function leaveLyricsEditor() {
  if (lyricsEditorReturnView.value === 'editinfo') goBackTo('editinfo')
  else goBackToMain()
}

async function parseOptionalLyricTrack(text: string, label: string): Promise<LyricLine[]> {
  if (!text) return []
  try {
    return mapBackendLyrics(await invoke<any[]>('parse_lrc_content', { content: text }))
  } catch (e) {
    log.warn(`Parse ${label} failed, applying the other tracks:`, e)
    return []
  }
}

async function applyLyricsFromEditor() {
  const text = lyricsEditorText.value.trim()
  const translationText = lyricsTranslationEditorText.value.trim()
  const romanizationText = lyricsRomanizationEditorText.value.trim()
  if (!text) {
    // 清除歌词: 本地 cache + syncPayload matched* 置空 (CURRENT 版本会同步清空)
    fetchedLyrics.value = []
    removeCachedLyricsForCurrentTrack()
    await commitLyricsToTrack(null, null, 'LOCAL_EDIT', null)
    toast.success(t('player.lyrics_cleared'))
    leaveLyricsEditor()
    return
  }
  try {
    // parse_lrc_content 已走 parse_auto, 支持 YRC 逐字往返
    const parsed = await invoke<any[]>('parse_lrc_content', { content: text })
    const nextLyrics = mergeParsedLyricsWithRomanization(
      mergeParsedLyricsWithTranslations(
        mapBackendLyrics(parsed),
        await parseOptionalLyricTrack(translationText, 'translation'),
      ),
      await parseOptionalLyricTrack(romanizationText, 'romanization'),
    )
    const source = lyricsEditorSource.value ?? 'LOCAL_EDIT'
    fetchedLyrics.value = nextLyrics
    rememberLyricSource(player.currentTrack, source)
    cacheLyricsForTrack(player.currentTrack, nextLyrics)
    // 原文保留编辑器文本(YRC/LRC), 与 Android toEditableLyricsText 往返一致
    const romanizedForCommit = romanizationText
      || (lyricsRomanizationEditorInitial.value.trim() ? null : undefined)
    await commitLyricsToTrack(text, translationText || null, source, romanizedForCommit)
    toast.success(t('player.lyrics_applied'))
  } catch (e) {
    log.error('Parse lyrics failed:', e)
    toast.error(String(e))
  }
  leaveLyricsEditor()
}

// 歌词匹配（对齐 Android 编辑器「匹配」）：从编辑器进来时填回编辑器，从菜单进来时直接应用
const lyricMatchTarget = ref<'track' | 'editor'>('track')
const lyricMatchSession = ref(0)

function openLyricMatch(target: 'track' | 'editor') {
  lyricMatchTarget.value = target
  lyricMatchSession.value++
  goToSubView('lyrics-fill')
}

function leaveLyricMatch() {
  if (lyricMatchTarget.value === 'editor') goBackTo('lyrics-editor')
  else goBackToMain()
}

async function onLyricMatchPicked(result: LyricMatchResult) {
  const source = lyricMatchSourceTag(result.source)
  const lyricText = toEditableLyricsText(result.lines)
  const translationText = toEditableTranslationText(result.lines)
  const romanizationText = toEditableRomanizationText(result.lines)
  if (lyricMatchTarget.value === 'editor') {
    lyricsEditorText.value = lyricText
    lyricsTranslationEditorText.value = translationText
    // 候选没有音译时保留用户已填的音译
    if (romanizationText) lyricsRomanizationEditorText.value = romanizationText
    lyricsEditorSource.value = source
    lyricsEditorTab.value = 'original'
    toast.success(t('player.lyrics_match_filled'))
    goBackTo('lyrics-editor')
    return
  }
  try {
    fetchedLyrics.value = result.lines
    rememberLyricSource(player.currentTrack, source)
    cacheLyricsForTrack(player.currentTrack, result.lines)
    // 换了一份歌词：候选没有音译时清掉旧音译，免得挂到新歌词上
    await commitLyricsToTrack(lyricText, translationText || null, source, romanizationText || null)
    toast.success(t('player.lyrics_fill_applied'))
    goBackToMain()
  } catch (e) {
    log.error('Apply matched lyrics failed:', e)
    toast.error(String(e))
  }
}

// 「获取歌曲信息」的搜索结果自带歌词时直接解析
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

async function toggleFavorite() {
  await likedSongs.toggleTrack(player.currentTrack)
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
onUnmounted(() => { lyricFetchRequestId++ })
/** 只补了音译的歌词 → 补之前那份；逐字升级据此认出「歌词没被换过」 */
const romanizationBackfilledFrom = new WeakMap<object, LyricLine[]>()

function upgradeWordTimedLyrics(track: TrackInfo, requestId: number) {
  const baseline = fetchedLyrics.value
  if (!settings.preferWordTimedLyrics || !getPlaybackSourceKind(track) || hasWordTimedLyrics(baseline)) return
  if (resolveStoredLyricStateFromPayload(track.syncPayload).kind !== 'absent') return
  const identity = JSON.stringify([track.id, track.title, track.artist, track.durationMs])
  void loadLyricsSingleFlight(track, () => fetchWordTimedLyrics({
    title: track.title, artist: track.artist, durationMs: track.durationMs || 0,
  }), 'word-timed').then(({ lines, source }) => {
    const current = player.currentTrack
    if (!current || requestId !== lyricFetchRequestId || !settings.preferWordTimedLyrics) return
    if (identity !== JSON.stringify([current.id, current.title, current.artist, current.durationMs])) return
    const shown = fetchedLyrics.value
    const unchanged = shown === baseline || romanizationBackfilledFrom.get(shown) === baseline
    if (!unchanged || !hasWordTimedLyrics(lines)) return
    if (resolveStoredLyricStateFromPayload(current.syncPayload).kind !== 'absent') return
    const merged = mergeWordTimedLyricsWithBaseline(shown, lines)
    fetchedLyrics.value = merged
    // 逐字时间轴来自 AMLL TTML / 酷狗，偏移按它们的默认算
    rememberLyricSource(current, source)
    cacheLyricsForTrack(current, merged)
  }).catch(error => log.warn('word timed lyric upgrade unavailable:', summarizeLogError(error)))
}

// 歌词没带音译时（同步载荷、旧缓存、非网易云来源），后台从网易云补上（显示与编辑器音译页都靠它）
function backfillNeteaseRomanization(track: TrackInfo, requestId: number) {
  if (!shouldBackfillNeteaseRomanization(track.syncPayload, fetchedLyrics.value)) return
  // 等待期间逐字升级可能已换上新时间轴，音译并到届时显示的那份上
  const stillWanted = () => requestId === lyricFetchRequestId && player.currentTrack?.id === track.id
    && shouldBackfillNeteaseRomanization(player.currentTrack?.syncPayload, fetchedLyrics.value)
  const songId = resolveKnownNeteaseLyricSongId(track)
  void fetchNeteaseRomanization(track, songId).then(async (text) => {
    if (!text) {
      log.info('romanization backfill: none found', { trackId: track.id, songId })
      return
    }
    if (!stillWanted()) return
    const roman = mapBackendLyrics(await invoke<any[]>('parse_lrc_content', { content: text }))
    if (!stillWanted()) return
    const shown = fetchedLyrics.value
    const merged = mergeParsedLyricsWithRomanization(shown, roman)
    if (!merged.some(line => line.roman)) {
      log.info('romanization backfill: no line matched', { trackId: track.id, romanLines: roman.length })
      return
    }
    fetchedLyrics.value = merged
    romanizationBackfilledFrom.set(fetchedLyrics.value, romanizationBackfilledFrom.get(shown) ?? shown)
    cacheLyricsForTrack(track, merged)
  }).catch(error => log.warn('netease romanization backfill unavailable:', summarizeLogError(error)))
}

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

  // 换曲瞬间立即撤下旧词：此刻播放位置已归零而旧词还挂着，
  // LyricsView 会判定大幅回跳、在旧词上硬跳回第一行——开播歌词
  // 「有概率抽一下」就是这个窗口。缓存命中只需一次本地数据库读取，
  // 随后赋回新词；在线获取则显示空态而不是旧词。
  fetchedLyrics.value = []

  const started = performance.now()
  const cachedLyrics = await readCachedLyrics(track)
  if (requestId !== lyricFetchRequestId) return
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
    // 同步歌词最优先(不触网, 不回写云端): Android 匹配的歌词经云同步落在 syncPayload,
    // 必须压过本地旧缓存, 否则历史在线歌词会永久屏蔽同步歌词
    // present -> 使用; cleared -> 空词并阻止在线回填; absent -> 缓存/在线
    const syncedLyrics = await materializeSyncedLyrics(track)
    if (requestId !== lyricFetchRequestId) return
    if (syncedLyrics !== null) {
      fetchedLyrics.value = syncedLyrics
      if (syncedLyrics.length > 0) {
        cacheLyricsForTrack(track, syncedLyrics)
        backfillNeteaseRomanization(track, requestId)
      }
      log.info('lyrics from sync payload:', {
        requestId,
        trackId: track.id,
        lines: syncedLyrics.length,
        cleared: syncedLyrics.length === 0,
      })
      return
    }

    // 设了默认歌词源时先按它匹配（Android tryGetPreferredLyricSourceResult），缓存已是该来源就直接用
    const preferredSource = preferredLyricMatchSource(getPlaybackSourceKind(track), settings.defaultLyricSource)
    if (preferredSource && !(cachedLyrics?.length && normalizeLyricSource(lyricSourceOf(track)) === preferredSource)) {
      const preferred = await fetchPreferredSourceLyrics(track, preferredSource, settings.preferWordTimedLyrics)
        .catch(error => { log.warn('preferred lyric source unavailable:', summarizeLogError(error)); return null })
      if (requestId !== lyricFetchRequestId) return
      if (preferred) {
        fetchedLyrics.value = preferred.lines
        rememberLyricSource(track, preferred.source)
        cacheLyricsForTrack(track, preferred.lines)
        log.info('lyrics from preferred source:', { requestId, trackId: track.id, source: preferred.source })
        backfillNeteaseRomanization(track, requestId)
        return
      }
    }

    // 先显示缓存，缺少逐字时间时再后台升级
    if (cachedLyrics?.length) {
      log.info('lyrics from local cache:', {
        requestId,
        trackId: track.id,
        lines: cachedLyrics.length,
      })
      if (!preferredSource) upgradeWordTimedLyrics(track, requestId)
      backfillNeteaseRomanization(track, requestId)
      return
    }

    const { lines: nextLyrics } = await loadLyricsSingleFlight(track, async () => {
      const invokeStarted = performance.now()
      log.info('lyrics backend invoke:', { requestId, trackId: track.id })
      const fetched = await fetchAutomaticLyrics(track, getPlaybackSourceKind(track), settings.preferWordTimedLyrics)
      if (fetched.lines.length > 0) {
        rememberLyricSource(track, fetched.source)
        cacheLyricsForTrack(track, fetched.lines)
      }
      // 后端目前把明确无词与任一歌词源临时失败都归为 []，不能据此写负缓存。
      // 否则一次网络/API 波动会让后续 24 小时都跳过在线歌词查询
      log.info('lyrics backend returned:', {
        requestId,
        trackId: track.id,
        source: fetched.source,
        lines: fetched.lines.length,
        elapsedMs: Math.round(performance.now() - invokeStarted),
      })
      return fetched
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
    upgradeWordTimedLyrics(track, requestId)
    backfillNeteaseRomanization(track, requestId)
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
    const restored = await readCachedLyrics(track)
    if (requestId === lyricFetchRequestId) {
      fetchedLyrics.value = restored || cachedLyrics || []
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
  // 暂停或卡片封面时停表，角度留在原处；恢复由 startDiscLoop 接上
  if (!player.isPlaying || !discRef.value) {
    discAnimFrame = 0
    discLastTime = 0
    return
  }
  if (!discLastTime) discLastTime = timestamp
  const dt = Math.min(64, timestamp - discLastTime)
  discLastTime = timestamp

  discAngle = (discAngle + DEG_PER_MS * dt) % 360
  discRef.value.style.transform = `rotate(${discAngle}deg)`
  discAnimFrame = requestAnimationFrame(animateDisc)
}

function startDiscLoop() {
  if (discAnimFrame) return
  if (discRef.value) discRef.value.style.transform = `rotate(${discAngle}deg)`
  discAnimFrame = requestAnimationFrame(animateDisc)
}

watch([() => player.isPlaying, discRef], startDiscLoop, { flush: 'post' })

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
  startDiscLoop()
  document.addEventListener('keydown', handleEscapeLayered)
})
onUnmounted(() => {
  document.removeEventListener('keydown', handleEscapeLayered)
  cancelAnimationFrame(discAnimFrame)
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

// 右键菜单（歌曲名/歌手复制 + 封面保存）
const contextMenu = ref({ show: false, x: 0, y: 0, type: '' as 'title' | 'artist' | 'cover' | 'artist-page' })
const artistLinks = ref<Array<{ id: string; name: string; route: { name: string; params: Record<string, string> } }>>([])
const artistLinkTrackId = ref('')
const artistLinksLoading = ref(false)

async function openArtistPage(event: MouseEvent) {
  const track = player.currentTrack
  if (!track?.artist.trim() || artistLinksLoading.value) return
  const source = getPlaybackSourceKind(track)
  if (source && source !== 'netease') {
    await router.push({ name: 'explore', query: { q: track.artist, platform: source === 'qq' ? 'netease' : source } })
    emit('collapse')
    return
  }
  artistLinksLoading.value = true
  try {
    let links: typeof artistLinks.value
    if (source === 'netease') {
      const rawId = track.syncPayload?.audioId ?? track.syncPayload?.audio_id ?? track.id.replace(/^netease:/i, '')
      const songId = Number(rawId)
      if (!Number.isSafeInteger(songId) || songId <= 0) return
      const detail = await invoke('get_netease_song_detail', { songId })
      links = neteaseSongArtists(detail, songId).map(artist => ({
        id: String(artist.id), name: artist.name,
        route: { name: 'netease-artist', params: { id: String(artist.id) } },
      }))
    } else {
      links = splitArtistNames(track.artist).map(name => ({
        id: name, name, route: { name: 'local-artist', params: { name } },
      }))
    }
    if (player.currentTrack?.id !== track.id) return
    if (links.length === 1) {
      await router.push(links[0]!.route)
      emit('collapse')
    } else if (links.length > 1) {
      artistLinks.value = links
      artistLinkTrackId.value = track.id
      contextMenu.value = { show: true, x: event.clientX, y: event.clientY, type: 'artist-page' }
    }
  } catch (error) {
    log.warn('failed to open artist page:', error)
    toast.error(t('player.artist_load_failed'))
  } finally {
    artistLinksLoading.value = false
  }
}

watch(() => player.hasPlaybackSession, (hasSession) => {
  if (hasSession) return
  closeToolbarPopovers()
  showQueue.value = false
  showMoreSheet.value = false
  showLtPanel.value = false
  contextMenu.value.show = false
})

const contextMenuItems = computed(() => {
  if (contextMenu.value.type === 'artist-page') {
    return artistLinks.value.map(artist => ({ id: artist.id, label: artist.name, icon: 'person' }))
  }
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
  if (contextMenu.value.type === 'artist-page') {
    const artist = artistLinks.value.find(link => link.id === item.id)
    if (artist && player.currentTrack?.id === artistLinkTrackId.value) {
      void router.push(artist.route).then(() => emit('collapse'))
    }
    closeContextMenu()
  } else if (item.id === 'copy-title') {
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

// 更多选项面板子视图
const moreSheetView = ref<
  'main' | 'offset' | 'fontsize' | 'effects' | 'search' | 'editinfo' |
  'quality' | 'lyrics-editor' | 'lyrics-fill' | 'track-detail'
>('main')
const moreSheetTransition = ref('slide-left')

function goToSubView(view: typeof moreSheetView.value) {
  moreSheetTransition.value = 'slide-left'
  moreSheetView.value = view
}

// 点击进度条旁的音质标签直接打开音质切换面板（ST-05：让该设置真实可切换而非纯展示）
function openQualitySwitcher() {
  if (!canSwitchCurrentAudioQuality.value || player.isLoadingAudio) return
  showMoreSheet.value = true
  goToSubView('quality')
}
function goBackToMain() {
  goBackTo('main')
}

function goBackTo(view: typeof moreSheetView.value) {
  moreSheetTransition.value = 'slide-right'
  moreSheetView.value = view
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
  lyrics: true,
})

function openInfoSearch() {
  searchQuery.value = player.currentTrack?.title || ''
  searchResults.value = []
  infoApplyCandidate.value = null
  applyInfoFields.value = { title: true, artist: true, cover: true, lyrics: true }
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

const fieldPickerRef = ref<HTMLElement | null>(null)

function applySearchResult(result: any) {
  infoApplyCandidate.value = result
  applyInfoFields.value = {
    title: !!result.title,
    artist: !!result.artist,
    cover: !!(result.cover_url || result.coverUrl),
    lyrics: true,
  }
  // 字段选择面板默认可能在滚动区外, 选中后滚到可见位置
  void nextTick(() => {
    fieldPickerRef.value?.scrollIntoView({ behavior: 'smooth', block: 'nearest' })
  })
}

/** 歌词来源标记: 对齐 Android MusicPlatform 枚举名, 其余平台名 Android 侧解析为 null 亦无害 */
function lyricSourceForPlatform(source?: string | null): string {
  const key = String(source || '').toLowerCase()
  if (key.includes('netease')) return 'CLOUD_MUSIC'
  if (key.includes('qq')) return 'QQ_MUSIC'
  return key ? key.toUpperCase() : 'LOCAL_EDIT'
}

async function confirmApplySearchResult() {
  const result = infoApplyCandidate.value
  const track = player.currentTrack
  if (!result || !track) return
  // 首次覆盖前的原始展示信息, 供 original* 回退
  const original = {
    title: track.title || '',
    artist: track.artist || '',
    coverUrl: track.coverUrl || '',
  }
  const patch: Record<string, string> = {}
  if (applyInfoFields.value.title) patch.title = result.title || track.title || ''
  if (applyInfoFields.value.artist) patch.artist = result.artist || track.artist || ''
  if (applyInfoFields.value.cover) patch.coverUrl = result.cover_url || result.coverUrl || track.coverUrl || ''
  if (Object.keys(patch).length > 0) {
    // 展示字段与 syncPayload custom* 一起更新, 并写回本地歌单供同步上传 (对齐 Android)
    const nextPayload = withUpdatedCustomInfoPayload(
      track.syncPayload,
      {
        title: patch.title,
        artist: patch.artist,
        coverUrl: patch.coverUrl,
        matchedSongId: String(result.id || '') || undefined,
      },
      original,
    )
    player.updateCurrentTrackInfo(patch)
    player.patchCurrentTrackSyncPayload(nextPayload)
    await persistTrackSyncPayload(player.currentTrack)
  }
  if (applyInfoFields.value.lyrics) {
    try {
      const source = lyricSourceForPlatform(result.source || result.platform)
      const direct = await parseLyricsFromSearchResult(result)
      if (direct && direct.lines.length > 0) {
        fetchedLyrics.value = direct.lines
        rememberLyricSource(player.currentTrack, source)
        cacheLyricsForTrack(player.currentTrack, direct.lines)
        await commitLyricsToTrack(direct.rawLyric, direct.rawTranslated, source)
      } else {
        const idText = String(result.id || '')
        const neteaseId = idText.startsWith('netease:') ? parseInt(idText.replace('netease:', '')) : null
        const qqSongMid = idText.startsWith('qq:') ? idText.replace('qq:', '') : null
        const fetched = await fetchLyrics({
          title: result.title || player.currentTrack?.title || '',
          artist: result.artist || player.currentTrack?.artist || '',
          durationSecs: Math.floor((result.duration_ms || player.currentTrack?.durationMs || 0) / 1000),
          audioPath: null,
          neteaseId,
          qqSongMid,
          youtubeVideoId: null,
        })
        if (fetched.lines.length) {
          const nextLyrics = fetched.lines
          fetchedLyrics.value = nextLyrics
          const actualSource = fetched.source ? lyricSourceForPlatform(fetched.source) : source
          rememberLyricSource(player.currentTrack, actualSource)
          cacheLyricsForTrack(player.currentTrack, nextLyrics)
          // 兜底在线歌词同样写回 syncPayload, 否则不同步且重启即丢
          await commitLyricsToTrack(
            toEditableLyricsText(nextLyrics),
            toEditableTranslationText(nextLyrics) || null,
            actualSource,
          )
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

async function saveEditInfo() {
  const track = player.currentTrack
  if (!track) return
  const original = {
    title: track.title || '',
    artist: track.artist || '',
    coverUrl: track.coverUrl || '',
  }
  // 手工编辑与匹配同路: custom* 进 syncPayload 并落盘, 否则重启即丢且不同步
  const nextPayload = withUpdatedCustomInfoPayload(
    track.syncPayload,
    {
      title: editTitle.value,
      artist: editArtist.value,
      coverUrl: editCoverUrl.value,
    },
    original,
  )
  player.updateCurrentTrackInfo({
    title: editTitle.value,
    artist: editArtist.value,
    coverUrl: editCoverUrl.value,
  })
  player.patchCurrentTrackSyncPayload(nextPayload)
  await persistTrackSyncPayload(player.currentTrack)
  toast.success(t('player.info_applied'))
  goBackToMain()
}

function restoreInfo() {
  player.restoreOriginalTrackInfo()
  toast.success(t('player.info_restored'))
  goBackToMain()
}

// 音质切换
// 下面的计算属性在 setup 时就会被 watch 求值，选项表必须先于它们声明
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

const currentSource = computed(() => {
  const id = player.currentTrack?.id || ''
  if (id.startsWith('netease:')) return 'netease'
  if (id.startsWith('qq:')) return 'qq'
  if (id.startsWith('bilibili:')) return 'bilibili'
  if (id.startsWith('youtube:')) return 'youtube'
  return 'local'
})
// 音质列表按播放层报告的可选项过滤（B 站只列这条视频实际提供的音质）；只有一档时不显示切换（对齐 Android）
const switchableQualities = computed(() => {
  const all = qualityOptionsForSource(currentSource.value)
  const info = player.audioInfo
  const offered = info?.source === currentSource.value ? info.qualityOptions?.map(option => option.key) : undefined
  return offered?.length ? all.filter(option => offered.includes(option.key)) : all
})
const canSwitchCurrentAudioQuality = computed(() => canSwitchAudioQuality({
  source: currentSource.value, fromDownload: player.isPlayingFromDownload, info: player.audioInfo,
}) && switchableQualities.value.length > 1)
watch(canSwitchCurrentAudioQuality, (canSwitch) => {
  if (!canSwitch && moreSheetView.value === 'quality') goBackToMain()
})

// 当前歌词用哪个来源的默认偏移：看正在显示的歌词来自哪里（AMLL TTML、酷狗、LRCLIB…），不知道时按播放来源
const currentLyricOffsetSource = computed(() => lyricOffsetStore.offsetSourceFor(player.currentTrack))
const currentLyricDefaultOffsetMs = computed(() =>
  lyricOffsetStore.defaultOffsetMs(currentLyricOffsetSource.value),
)
// 逐曲 delta：只用来判断这首歌是否单独调过
const currentLyricUserOffsetMs = computed(() => lyricOffsetStore.getUserOffsetMs(player.currentTrack))

// 有效偏移（绝对值）：歌词渲染用它，偏移面板显示和编辑的也是它
const currentLyricTotalOffsetMs = computed<number>({
  get: () => currentLyricDefaultOffsetMs.value + currentLyricUserOffsetMs.value,
  set: value => lyricOffsetStore.setEffectiveOffsetMs(player.currentTrack, value),
})

// 滑条范围 ±5000ms，当前值越界时随之放宽（Android resolveLyricOffsetSliderRange）
const lyricOffsetSliderMin = computed(() => Math.min(MIN_LYRIC_DEFAULT_OFFSET_MS, currentLyricTotalOffsetMs.value))
const lyricOffsetSliderMax = computed(() => Math.max(MAX_LYRIC_DEFAULT_OFFSET_MS, currentLyricTotalOffsetMs.value))

function nudgeLyricOffset(steps: number) {
  currentLyricTotalOffsetMs.value += steps * LYRIC_OFFSET_STEP_MS
}

function resetLyricOffsetToDefault() {
  lyricOffsetStore.setUserOffsetMs(player.currentTrack, 0)
}

// 当前歌词来源的名字，用于「网易云 · 默认 +1000ms」；没有可调默认的来源也标出来
const currentLyricSourceLabel = computed(() => {
  if (currentLyricOffsetSource.value === 'netease' && lyricOffsetStore.offsetSourceIsGuessed(player.currentTrack)) {
    return t('player.lyric_source_unknown_as', { source: t('player.source_netease') })
  }
  switch (currentLyricOffsetSource.value) {
    case 'netease': return t('player.source_netease')
    case 'qq': return t('player.source_qq')
    case 'kugou': return t('player.lyric_source_kugou')
    case 'lrclib': return 'LRCLIB'
    case 'amll_ttml': return 'AMLL TTML'
  }
  const raw = (lyricSourceOf(player.currentTrack) ?? '').toLowerCase().replace(/[^a-z]/g, '')
  if (raw === 'youtube') return t('player.source_youtube')
  if (raw === 'local') return t('player.lyric_source_local')
  if (raw === 'localedit') return t('player.lyric_source_edited')
  return ''
})

const lyricOffsetSummary = computed(() => [formatLyricOffsetMs(currentLyricTotalOffsetMs.value), currentLyricSourceLabel.value]
  .filter(Boolean).join(' · '))

function nudgeLyricFontScale(steps: number) {
  const next = Math.round((settings.lyricFontScale + steps * LYRIC_FONT_SCALE_STEP) / LYRIC_FONT_SCALE_STEP) * LYRIC_FONT_SCALE_STEP
  settings.lyricFontScale = Math.min(LYRIC_FONT_SCALE_MAX, Math.max(LYRIC_FONT_SCALE_MIN, Number(next.toFixed(2))))
}

// 「更多」里音频效果一行直接写出当前生效的设置，都是默认时显示说明
const audioEffectsSummary = computed(() => {
  const parts: string[] = []
  if (player.playbackSpeed !== 1) parts.push(`${player.playbackSpeed.toFixed(2)}x`)
  if (player.loudnessGainMb !== 0) parts.push(`+${(player.loudnessGainMb / 100).toFixed(1)}dB`)
  if (player.equalizerEnabled) parts.push(t(`player.eq_${player.equalizerPresetId}`))
  return parts.length ? parts.join(' · ') : t('player.audio_effects_desc')
})

function openLyricsFill() {
  openLyricMatch('track')
}

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

function isRemotePlaybackSource(source: string) {
  return source === 'netease' || source === 'qq' || source === 'bilibili' || source === 'youtube'
}

function downloadTaskStatusText(status?: string) {
  switch (status) {
    case 'queued': return t('download.queued')
    case 'processing': return t('download.processing')
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
  if (isCurrentDownloadCancellable.value) return t('download.cancel_task')
  if (isCurrentDownloading.value) return downloadTaskStatusText(currentDownloadTask.value?.status) || t('download.downloading')
  if (isCurrentDownloaded.value) return t('download.redownload')
  return t('library.tab_downloads')
})

const downloadActionDesc = computed(() => {
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
  || (isCurrentDownloading.value && !isCurrentDownloadCancellable.value && currentDownloadTask.value?.status !== 'error' && currentDownloadTask.value?.status !== 'cancelled')
)

const isQualitySwitching = ref(false)

async function switchQuality(key: string) {
  if (isQualitySwitching.value || !canSwitchCurrentAudioQuality.value || player.isLoadingAudio) return
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
  if (isCurrentDownloaded.value) {
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

async function openCurrentArtist() {
  const songId = currentNeteaseSongNumericId.value
  if (!songId) return
  try {
    const detail = await invoke<any>('get_netease_song_detail', { songId })
    const song = detail?.songs?.[0]
    const artist = (song?.ar || []).find((a: any) => a?.id)
    if (!artist?.id) {
      toast.error(t('player.not_available'))
      return
    }
    hideMoreSheet()
    // 先收起正在播放页，再跳转歌手页，避免详情盖在 NP 下面
    emit('collapse')
    await router.push({
      name: 'netease-artist',
      params: { id: String(artist.id) },
      query: { name: artist.name || primaryArtistName.value },
    })
  } catch (e) {
    log.error('Open artist failed:', e)
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

// 主歌手名 (展示用, 多歌手取第一个)
const primaryArtistName = computed(() => {
  const artist = player.currentTrack?.artist || ''
  return artist.split(/\s*[/,、·]\s*/)[0] || ''
})
const canViewNeteaseArtist = computed(() =>
  currentSource.value === 'netease' && !!primaryArtistName.value && !!currentNeteaseSongNumericId.value)

// 进度条下方音质信息（不展示 Local / download 占位）
// 参数开关统一控制文件和在线流，平台音质标签单独显示
const displayedAudioInfo = usePlaybackAudioInfoDisplay(() => ({
  info: player.audioInfo,
  fromDownload: player.isPlayingFromDownload,
  loading: player.isLoadingAudio,
  hasSession: player.hasPlaybackSession,
}))
const audioInfoParts = computed(() => {
  const info = displayedAudioInfo.value.info
  if (!info) return []
  const parts: Array<{ text: string; accent?: boolean }> = []
  if (settings.showQualitySwitch) addAudioInfoPart(parts, currentAudioQualityLabel(info, displayedAudioInfo.value.fromDownload), true)
  for (const token of actualAudioParameterLabels(info, settings)) addAudioInfoPart(parts, token)
  return parts.filter(part => !isHiddenAudioInfoToken(part.text))
})

// 音质行放不下时整项换行；正好落在换行处的分隔点隐藏，不让「·」挂在行首或行尾。
// 只隐藏不移除，占位不变，避免隐藏后空出位置又把下一项拉回上一行来回跳
const audioDetailEl = ref<HTMLElement | null>(null)
const wrappedAudioSeparators = ref<ReadonlySet<number>>(new Set())

function updateWrappedAudioSeparators() {
  const container = audioDetailEl.value
  const wrapped = new Set<number>()
  container?.querySelectorAll<HTMLElement>('.np-audio-separator').forEach((separator, index) => {
    const before = separator.previousElementSibling as HTMLElement | null
    const after = separator.nextElementSibling as HTMLElement | null
    if (before && after && Math.abs(before.offsetTop - after.offsetTop) > 2) wrapped.add(index)
  })
  const current = wrappedAudioSeparators.value
  if (wrapped.size !== current.size || [...wrapped].some(index => !current.has(index))) {
    wrappedAudioSeparators.value = wrapped
  }
}

// 放到下一帧再量：在观察回调里直接改状态会触发浏览器的 ResizeObserver 循环告警
let wrappedSeparatorsFrame = 0
const audioDetailResize = typeof ResizeObserver === 'undefined'
  ? null
  : new ResizeObserver(() => {
    cancelAnimationFrame(wrappedSeparatorsFrame)
    wrappedSeparatorsFrame = requestAnimationFrame(updateWrappedAudioSeparators)
  })
watch(audioDetailEl, (element, previous) => {
  if (previous) audioDetailResize?.unobserve(previous)
  if (element) audioDetailResize?.observe(element)
})
watch(audioInfoParts, () => { void nextTick(updateWrappedAudioSeparators) }, { flush: 'post' })
onUnmounted(() => {
  audioDetailResize?.disconnect()
  cancelAnimationFrame(wrappedSeparatorsFrame)
})

function currentAudioQualityLabel(info: AudioInfo | null = player.audioInfo, fromDownload = player.isPlayingFromDownload) {
  return resolveAudioQualityLabel({ source: currentSource.value, fromDownload, info },
    (source, key) => qualityLabelFor(source, key || currentQualityKey(source)))
}

// 文件播放只采用探测字段，避免把平台规格当成实际文件参数
function paperSpecFromAudioInfo(info: {
  sampleRateHz?: number
  bitDepth?: number
  channelCount?: number
  specLabel?: string
}, includeSpecLabel = true): string[] {
  const tokens: string[] = []
  if (info.sampleRateHz && info.sampleRateHz > 0) {
    const khz = info.sampleRateHz / 1000
    tokens.push(info.sampleRateHz % 1000 === 0
      ? `${khz.toFixed(0)} kHz`
      : `${khz.toFixed(1)} kHz`)
  }
  if (info.bitDepth && info.bitDepth > 0) tokens.push(`${info.bitDepth} bit`)
  if (!includeSpecLabel && info.channelCount && info.channelCount > 0) tokens.push(`${info.channelCount} ch`)
  // specLabel 里可能混有 kbps, 过滤掉
  if (includeSpecLabel && info.specLabel) {
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

function normalizeAudioDisplayToken(value?: string, local = false) {
  if (!value) return ''
  const raw = value.trim()
  // 在线流的 format 常是 MIME（audio/mp4; codecs="mp4a.40.2"），按编码参数或子类型识别
  const [essence, params = ''] = raw.toLowerCase().split(';')
  const isMime = /^(audio|video)\//.test(essence)
  const lower = /codecs\s*=\s*"?([^",]+)/.exec(params)?.[1]?.trim()
    || (isMime ? essence.trim().replace(/^(audio|video)\/(x-)?/, '') : raw.toLowerCase())
  if (local && lower === 'mpeg') return 'MPEG'
  // 占位词直接丢掉
  if (isHiddenAudioInfoToken(raw)) return ''
  const tokenMap: Record<string, string> = {
    flac: 'FLAC',
    mp3: 'MP3',
    mpeg: 'MP3',
    aac: 'AAC',
    mp4a: 'AAC',
    mp4: 'MP4',
    m4a: 'M4A',
    webm: 'WebM',
    opus: 'Opus',
    ogg: 'OGG',
    vorbis: 'Vorbis',
    wav: 'WAV',
    aiff: 'AIFF',
    'ec-3': 'E-AC-3',
    eac3: 'E-AC-3',
    ac3: 'AC-3',
  }
  return tokenMap[lower] ?? tokenMap[lower.split('.')[0]] ?? (isMime ? lower.toUpperCase() : raw)
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
// 主色直出在深色播放页上过亮刺眼：混入黑色压一档亮度，保持色相不变
const sliderActiveColor = computed(() => {
  const vars = dynamicColorVars.value
  const primary = (vars as any)['--np-primary'] || '#fff'
  return `color-mix(in srgb, ${primary} 72%, black)`
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
    :style="dynamicColorVars"
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
    <HyperBackground
      v-else-if="shouldRenderDynamicBackground"
      :playing="player.isPlaying"
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
          class="cover-wrap"
          :class="{
            'cover-wrap--card': settings.coverStyle === 'card',
            'cover-wrap--disc': settings.coverStyle !== 'card',
            'cover-wrap--switching': isTrackSwitchAnimating,
          }"
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
          v-if="settings.showToolbarDock"
          class="np-toolbar"
          @click.stop
        >
          <button
            class="tool-btn tool-btn--feedback fav-btn"
            :class="{ active: isFavorite }"
            :disabled="!player.currentTrack"
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
              <AudioEffectsPanel />
            </div>
          </div>
          <button class="tool-btn tool-btn--feedback" @click="triggerControlFeedbackPulse(); toggleToolbarPanel('add')">
            <span class="material-symbols-rounded">playlist_add</span>
          </button>
        </div>
        </div>
      </section>

      <!-- 右侧歌词 -->
      <section class="np-right" :class="{ 'np-right--switching': isTrackSwitchAnimating, 'np-right--beat-active': isVisualBeatActive }">
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
    <ListenTogetherPanel v-if="player.hasPlaybackSession" v-model:open="showLtPanel" />

    <!-- 更多选项面板（对齐 Android MoreOptionsSheet） -->
    <Teleport to="body">
      <Transition name="more-sheet">
      <div v-if="player.hasPlaybackSession && showMoreSheet" class="np-more-overlay" @click="showMoreSheet = false">
        <div class="np-more-sheet" @click.stop>

          <Transition :name="moreSheetTransition" mode="out-in">
          <div :key="moreSheetView" class="np-more-sheet-content">

          <!-- 主菜单：分组与顺序对齐 Android MoreOptionsMainContent；常用的偏移、字号直接在行内加减 -->
          <template v-if="moreSheetView === 'main'">
            <h4 class="np-more-title">{{ t('player.more_options') }}</h4>

            <!-- 歌曲信息 -->
            <div class="np-more-group">
              <button class="np-more-list-item" @click="openInfoSearch">
                <span class="material-symbols-rounded">info</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.get_info') }}</span>
                </div>
                <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
              </button>
              <button class="np-more-list-item" @click="openEditInfo">
                <span class="material-symbols-rounded">edit</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.edit_info') }}</span>
                </div>
                <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
              </button>
            </div>

            <!-- 播放：音质、音频效果、下载 -->
            <div class="np-more-group">
              <button v-if="canSwitchCurrentAudioQuality" class="np-more-list-item" :disabled="player.isLoadingAudio" @click="openQualitySwitcher()">
                <span class="material-symbols-rounded">music_note</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.quality_switch') }}</span>
                  <span class="np-more-list-desc">{{ currentAudioQualityLabel() || '—' }}</span>
                </div>
                <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
              </button>
              <button class="np-more-list-item" @click="goToSubView('effects')">
                <span class="material-symbols-rounded">tune</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.audio_effects') }}</span>
                  <span class="np-more-list-desc">{{ audioEffectsSummary }}</span>
                </div>
                <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
              </button>
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
            </div>

            <!-- 歌词 -->
            <div class="np-more-group">
              <div class="np-more-list-item np-more-list-item--stepper">
                <button class="np-more-stepper-main" @click="goToSubView('offset')">
                  <span class="material-symbols-rounded">timer</span>
                  <div class="np-more-list-info">
                    <span class="np-more-list-headline">{{ t('player.lyric_offset') }}</span>
                    <span class="np-more-list-desc" :class="{ 'np-more-list-desc--custom': currentLyricUserOffsetMs !== 0 }">{{ lyricOffsetSummary }}</span>
                  </div>
                </button>
                <div class="np-more-stepper">
                  <button :title="t('player.lyric_offset_later')" :aria-label="t('player.lyric_offset_later')" @click="nudgeLyricOffset(-1)">
                    <span class="material-symbols-rounded">remove</span>
                  </button>
                  <button :title="t('player.lyric_offset_earlier')" :aria-label="t('player.lyric_offset_earlier')" @click="nudgeLyricOffset(1)">
                    <span class="material-symbols-rounded">add</span>
                  </button>
                </div>
              </div>
              <div class="np-more-list-item np-more-list-item--stepper">
                <button class="np-more-stepper-main" @click="goToSubView('fontsize')">
                  <span class="material-symbols-rounded">format_size</span>
                  <div class="np-more-list-info">
                    <span class="np-more-list-headline">{{ t('player.font_scale') }}</span>
                    <span class="np-more-list-desc">{{ Math.round(settings.lyricFontScale * 100) }}%</span>
                  </div>
                </button>
                <div class="np-more-stepper">
                  <button
                    :disabled="settings.lyricFontScale <= LYRIC_FONT_SCALE_MIN"
                    :title="t('player.font_scale_smaller')" :aria-label="t('player.font_scale_smaller')"
                    @click="nudgeLyricFontScale(-1)"
                  >
                    <span class="material-symbols-rounded">text_decrease</span>
                  </button>
                  <button
                    :disabled="settings.lyricFontScale >= LYRIC_FONT_SCALE_MAX"
                    :title="t('player.font_scale_larger')" :aria-label="t('player.font_scale_larger')"
                    @click="nudgeLyricFontScale(1)"
                  >
                    <span class="material-symbols-rounded">text_increase</span>
                  </button>
                </div>
              </div>
              <button class="np-more-list-item" :class="{ active: desktopLyricsOpen }" @click="toggleDesktopLyrics">
                <span class="material-symbols-rounded">picture_in_picture_alt</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.desktop_lyrics') }}</span>
                </div>
                <span v-if="desktopLyricsOpen" class="material-symbols-rounded np-more-list-check">check</span>
              </button>
              <button v-if="desktopLyricsOpen" class="np-more-list-item" @click="toggleDesktopLyricsLock">
                <span class="material-symbols-rounded">{{ settings.desktopLyrics.locked ? 'lock' : 'lock_open' }}</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t(settings.desktopLyrics.locked ? 'desktop_lyrics.unlock' : 'desktop_lyrics.lock') }}</span>
                </div>
              </button>
              <button class="np-more-list-item" @click="openLyricsFill">
                <span class="material-symbols-rounded">lyrics</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.lyrics_fill') }}</span>
                  <span class="np-more-list-desc">{{ t('player.lyrics_fill_desc') }}</span>
                </div>
                <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
              </button>
              <button class="np-more-list-item" @click="openLyricsEditor()">
                <span class="material-symbols-rounded">edit_note</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.lyrics_editor') }}</span>
                </div>
                <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
              </button>
            </div>

            <!-- 浏览与分享 -->
            <div class="np-more-group">
              <button v-if="canViewNeteaseAlbum" class="np-more-list-item" @click="openCurrentAlbum">
                <span class="material-symbols-rounded">library_music</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.view_album', { name: albumName }) }}</span>
                </div>
              </button>
              <button v-if="canViewNeteaseArtist" class="np-more-list-item" @click="openCurrentArtist">
                <span class="material-symbols-rounded">artist</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.view_artist', { name: primaryArtistName }) }}</span>
                </div>
              </button>
              <button class="np-more-list-item" @click="shareSong">
                <span class="material-symbols-rounded">share</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.share') }}</span>
                </div>
              </button>
              <button class="np-more-list-item" @click="openListenTogetherFromMore">
                <span class="material-symbols-rounded">headphones</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('listen_together.title') }}</span>
                  <span class="np-more-list-desc">{{ t('listen_together.entry_desc') }}</span>
                </div>
                <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
              </button>
              <button class="np-more-list-item" @click="goToSubView('track-detail')">
                <span class="material-symbols-rounded">article</span>
                <div class="np-more-list-info">
                  <span class="np-more-list-headline">{{ t('player.track_detail') }}</span>
                  <span class="np-more-list-desc">{{ t('player.track_detail_desc') }}</span>
                </div>
                <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
              </button>
            </div>
          </template>

          <!-- 子视图：歌词偏移（显示与编辑的都是实际生效的偏移） -->
          <template v-else-if="moreSheetView === 'offset'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.lyric_offset') }}</h4>
            </div>
            <div class="np-more-item">
              <div class="np-more-hint">
                {{ currentLyricSourceLabel
                  ? t('player.lyric_offset_default_of', { source: currentLyricSourceLabel, value: formatLyricOffsetMs(currentLyricDefaultOffsetMs) })
                  : t('player.lyric_offset_default', { value: formatLyricOffsetMs(currentLyricDefaultOffsetMs) }) }}
              </div>
              <div class="np-more-row">
                <button class="np-more-step-btn" :title="t('player.lyric_offset_later')" :aria-label="t('player.lyric_offset_later')" @click="nudgeLyricOffset(-1)">
                  <span class="material-symbols-rounded">remove</span>
                </button>
                <input type="range"
                  :min="lyricOffsetSliderMin" :max="lyricOffsetSliderMax" :step="LYRIC_OFFSET_STEP_MS"
                  :value="currentLyricTotalOffsetMs"
                  class="np-more-slider"
                  :aria-label="t('player.lyric_offset')"
                  @input="currentLyricTotalOffsetMs = parseInt(($event.target as HTMLInputElement).value)"
                />
                <button class="np-more-step-btn" :title="t('player.lyric_offset_earlier')" :aria-label="t('player.lyric_offset_earlier')" @click="nudgeLyricOffset(1)">
                  <span class="material-symbols-rounded">add</span>
                </button>
                <EditableRangeValue
                  v-model="currentLyricTotalOffsetMs"
                  class="np-offset-value"
                  :class="{ positive: currentLyricTotalOffsetMs > 0, negative: currentLyricTotalOffsetMs < 0 }"
                  :min="lyricOffsetSliderMin"
                  :max="lyricOffsetSliderMax"
                  :step="LYRIC_OFFSET_STEP_MS"
                  :display-value="formatLyricOffsetMs(currentLyricTotalOffsetMs)"
                  input-suffix="ms"
                  :aria-label="t('player.lyric_offset')"
                />
              </div>
              <p class="np-more-hint np-more-hint--below">{{ t('player.lyric_offset_hint') }}</p>
              <button v-if="currentLyricUserOffsetMs !== 0" class="np-more-text-btn" @click="resetLyricOffsetToDefault">
                <span class="material-symbols-rounded">restart_alt</span>
                {{ t('player.lyric_offset_reset', { value: formatLyricOffsetMs(currentLyricDefaultOffsetMs) }) }}
              </button>
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
                <input type="range" :min="LYRIC_FONT_SCALE_MIN" :max="LYRIC_FONT_SCALE_MAX" :step="LYRIC_FONT_SCALE_STEP"
                  :value="settings.lyricFontScale"
                  class="np-more-slider"
                  :aria-label="t('player.font_scale')"
                  @input="settings.lyricFontScale = parseFloat(($event.target as HTMLInputElement).value)"
                />
                <EditableRangeValue
                  v-model="settings.lyricFontScale"
                  class="np-offset-value"
                  :min="LYRIC_FONT_SCALE_MIN"
                  :max="LYRIC_FONT_SCALE_MAX"
                  :step="LYRIC_FONT_SCALE_STEP"
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

          <!-- 子视图：音频效果，与工具栏的音效弹层是同一个面板 -->
          <template v-else-if="moreSheetView === 'effects'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.audio_effects') }}</h4>
            </div>
            <AudioEffectsPanel class="np-more-effects" />
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
                {{ t('player.source_netease') }}
              </button>
              <button
                :class="{ active: infoSearchPlatform === 'qq' }"
                @click="infoSearchPlatform = 'qq'; searchResults = []; infoApplyCandidate = null"
              >
                {{ t('player.source_qq') }}
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
            <div class="np-more-search-results" :class="{ compact: !!infoApplyCandidate }">
              <button
                v-for="(r, ri) in searchResults"
                :key="ri"
                class="np-more-search-item"
                :class="{ active: infoApplyCandidate === r }"
                @click="applySearchResult(r)"
              >
                <BilibiliCoverImage :src="r.cover_url" class="np-more-search-cover"><span class="np-more-search-cover np-more-search-cover-fallback material-symbols-rounded filled">music_note</span></BilibiliCoverImage>
                <div class="np-more-search-info">
                  <span class="np-more-search-title">{{ r.title }}</span>
                  <span class="np-more-search-artist">{{ r.artist }}</span>
                </div>
                <span class="np-more-search-source">{{ platformLabel(r.source) }}</span>
              </button>
            </div>
            <div v-if="infoApplyCandidate" ref="fieldPickerRef" class="np-more-field-picker">
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
              <div class="np-more-field-title">{{ t('player.fill_fields_title') }}</div>
              <div class="np-more-field-options">
                <label class="np-more-chip">
                  <input v-model="applyInfoFields.title" type="checkbox" />
                  <span>{{ t('player.song_title') }}</span>
                </label>
                <label class="np-more-chip">
                  <input v-model="applyInfoFields.artist" type="checkbox" />
                  <span>{{ t('player.artist_name') }}</span>
                </label>
                <label class="np-more-chip">
                  <input v-model="applyInfoFields.cover" type="checkbox" />
                  <span>{{ t('player.fill_field_cover') }}</span>
                </label>
                <label class="np-more-chip">
                  <input v-model="applyInfoFields.lyrics" type="checkbox" />
                  <span>{{ t('player.fill_field_lyrics') }}</span>
                </label>
              </div>
              <div class="np-more-form-actions compact">
                <button class="np-more-form-btn primary" @click="confirmApplySearchResult">
                  <span class="material-symbols-rounded">check</span>
                  {{ t('player.apply_selection') }}
                </button>
                <button class="np-more-form-btn" @click="infoApplyCandidate = null">
                  {{ t('common.cancel') }}
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

              <!-- 对齐 Android：歌曲信息里直接进歌词编辑 -->
              <div class="np-more-form-actions">
                <button class="np-more-form-btn" @click="openLyricsEditor('editinfo')">
                  <span class="material-symbols-rounded">edit_note</span>
                  {{ t('player.edit_lyrics') }}
                </button>
              </div>

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
                <strong>{{ playbackSourceLabel || platformLabel(currentSource) || '-' }}</strong>
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

          <!-- 子视图：音质切换 -->
          <template v-else-if="moreSheetView === 'quality'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="goBackToMain()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.quality_switch') }}</h4>
            </div>
            <div v-if="canSwitchCurrentAudioQuality" class="np-more-quality-list">
              <button
                v-for="q in switchableQualities"
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
              <button class="np-more-back" @click="leaveLyricsEditor()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.lyrics_editor') }}</h4>
              <button class="np-more-header-action" @click="openLyricMatch('editor')">
                <span class="material-symbols-rounded">manage_search</span>
                {{ t('player.lyrics_match_action') }}
              </button>
            </div>
            <div class="np-lyrics-editor">
              <div class="np-more-segmented">
                <button
                  :class="{ active: lyricsEditorTab === 'original' }"
                  @click="lyricsEditorTab = 'original'"
                >
                  {{ t('player.lyrics_editor_original') }}
                </button>
                <button
                  :class="{ active: lyricsEditorTab === 'translation' }"
                  @click="lyricsEditorTab = 'translation'"
                >
                  {{ t('player.lyrics_editor_translation') }}
                </button>
                <button
                  :class="{ active: lyricsEditorTab === 'romanization' }"
                  @click="lyricsEditorTab = 'romanization'"
                >
                  {{ t('player.lyrics_editor_romanization') }}
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
                v-else-if="lyricsEditorTab === 'translation'"
                v-model="lyricsTranslationEditorText"
                class="np-lyrics-textarea"
                :placeholder="t('player.lyrics_translation_placeholder')"
                spellcheck="false"
              />
              <textarea
                v-else
                v-model="lyricsRomanizationEditorText"
                class="np-lyrics-textarea"
                :placeholder="t('player.lyrics_romanization_placeholder')"
                spellcheck="false"
              />
              <div class="np-more-form-actions">
                <button class="np-more-form-btn primary" @click="applyLyricsFromEditor">
                  <span class="material-symbols-rounded">check</span>
                  {{ t('player.lyrics_apply') }}
                </button>
                <button class="np-more-form-btn" @click="lyricsEditorText = ''; lyricsTranslationEditorText = ''; lyricsRomanizationEditorText = ''; applyLyricsFromEditor()">
                  <span class="material-symbols-rounded">clear_all</span>
                  {{ t('player.lyrics_clear') }}
                </button>
              </div>
            </div>
          </template>

          <!-- 子视图：歌词匹配（对齐 Android 歌词编辑器「匹配」；从菜单进来时直接应用） -->
          <template v-else-if="moreSheetView === 'lyrics-fill'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="leaveLyricMatch()">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t(lyricMatchTarget === 'editor' ? 'player.lyrics_match' : 'player.lyrics_fill') }}</h4>
            </div>
            <LyricsMatchPanel
              v-if="player.currentTrack"
              :key="`${lyricMatchSession}:${player.currentTrack.id}`"
              :title="player.currentTrack.title"
              :artist="player.currentTrack.artist"
              :album="player.currentTrack.album"
              :duration-ms="player.currentTrack.durationMs || 0"
              :playback-source="currentSource"
              :prefer-word-timed="settings.preferWordTimedLyrics"
              :apply-label="t(lyricMatchTarget === 'editor' ? 'player.lyrics_match_fill_editor' : 'player.lyrics_match_apply')"
              @pick="onLyricMatchPicked"
            />
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
  // Windows/Linux 窗口为方形，圆角会让四角透出下层页面；
  // macOS 原生窗口自带圆角，仅在 macOS 保持圆角（np--rounded-window）
  border-radius: 0;
  user-select: none;
  -webkit-user-select: none;
  transition: transform 460ms cubic-bezier(0.22, 1, 0.36, 1), opacity 300ms ease;
}

.now-playing.np--rounded-window {
  /* 与 macOS 窗体圆角一致，避免全屏层直角顶出 OS 圆角 */
  border-radius: var(--radius-lg);
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
  padding: calc(var(--titlebar-height, 36px) + 8px) 24px 14px; /* 顶栏高度 + 8px 间距 */
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
    /* 顶栏高度 + 16px 呼吸空间，避免内容贴红绿灯/顶栏 */
    padding-top: calc(var(--titlebar-height, 36px) + 16px);
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
      flex: 1 1 46%;
      max-width: 560px;
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
  display: flex;
  flex-direction: column;
  align-items: center;
  /* 外层垂直居中整块 stack；stack 内部固定，切歌不重排 */
  justify-content: center;
  padding: 12px 28px 16px 40px;
  transition: opacity 280ms ease;
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
  flex: 1;
  min-width: 0;
  background: transparent;
  display: flex;
  align-items: stretch;
  overflow: hidden;
  padding: 0 40px 8px 8px;
  transition: transform 420ms cubic-bezier(0.22, 1, 0.36, 1), opacity 280ms ease, filter 420ms cubic-bezier(0.22, 1, 0.36, 1);
}

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
  display: block;
  cursor: pointer;
  text-align: left;
  font-size: 14px;
  color: rgba(255,255,255,0.78);
  margin-top: 2px;
  font-weight: 500;
  line-height: 1.25;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  &:hover { text-decoration: underline; }
  &:disabled { cursor: default; }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 3px; }
}

/* 进度条区域：固定高度，音质行始终占位 */
.np-slider-area {
  width: 100%;
  max-width: 100%;
  /* 进度条 + 时间 + 音质/下载 chip，留足高度避免裁切；窄栏里音质行换成两行时随之长高，不压到控制栏 */
  min-height: 80px;
  padding-top: 10px;
  flex-shrink: 0;
  container-type: inline-size;
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
  margin-top: 0;
  /* 空参数行保持高度，切换播放来源时不推动控制栏 */
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

/* 行内排版：放不下时在分隔点后的空格处整项换行，并让两行长度接近，不在第二行孤零零剩一项 */
.np-audio-detail {
  display: block;
  min-width: 0;
  max-width: 100%;
  text-align: center;
  text-wrap: balance;
  line-height: 1.45;
  color: rgba(255,255,255,0.68);

  &.separated::before {
    content: '·';
    margin-right: 5px;
    color: rgba(255,255,255,0.42);
  }
}

.np-audio-detail-part {
  /* 「1025 kbps」「多声道（E-AC-3）」这类整项不从中间断开，放不下就整项换到下一行 */
  white-space: nowrap;
  color: rgba(255,255,255,0.70);
  transition: color 0.45s ease;
}

.np-audio-detail-part--accent {
  /* 降低饱和与发光，避免「高清环绕声」等标签过于抢眼 */
  color: color-mix(in srgb, var(--np-primary, var(--md-primary, #D0BCFF)) 58%, rgba(255,255,255,0.78));
  text-shadow: none;
  font-weight: 600;
}

.np-audio-detail-part--clickable {
  cursor: pointer;
  border-radius: 4px;
  transition: color 0.45s ease, background-color 0.2s ease;
}

.np-audio-detail-part--clickable:hover,
.np-audio-detail-part--clickable:focus-visible {
  color: var(--np-primary, var(--md-primary, #D0BCFF));
  background-color: rgba(255, 255, 255, 0.08);
  outline: none;
}

/* 左边距加上后面的空格，两侧间隔相当 */
.np-audio-separator {
  margin: 0 2px 0 5px;
  color: rgba(255,255,255,0.34);
}

.np-audio-separator--wrapped {
  visibility: hidden;
}

/* 左栏较窄时收紧字号，常见的六项参数尽量仍排在一行 */
@container (max-width: 360px) {
  .np-audio-info {
    font-size: 10.5px;
    letter-spacing: 0;
  }

  .np-audio-separator {
    margin-left: 3px;
  }
}

.np-download-chip {
  display: inline-flex;
  align-items: center;
  line-height: 1;
  color: var(--np-primary-container, var(--md-primary-container, #E8DEF8));
  flex-shrink: 0;

  .material-symbols-rounded {
    font-size: 16px;
    line-height: 1;
  }
}

/* 控制栏 */
.np-controls {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  width: 100%;
  margin-top: 4px;
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
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  width: 100%;
  margin-top: 4px;
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

/* 黑胶是圆形，右下角落在唱片外：改为底部居中（不用 transform，避免与切换动画冲突） */
.source-badge--disc {
  left: 0;
  right: 0;
  bottom: 2px;
  width: fit-content;
  margin: 0 auto;
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
  // 竖排滑条不能用 writing-mode（WebKitGTK 圆点错位且拖不动），
  // 用原生横向滑条整体旋转：几何/命中计算保持原生，视觉为竖向。
  // value = 1 - volume：rotate(90deg) 使 min 在上，即音量满格在上
  appearance: none;
  width: 100px;
  height: 4px;
  transform: rotate(90deg);
  margin: 48px 0;
  background: rgba(255, 255, 255, 0.15);
  border-radius: 2px;
  outline: none;
  cursor: pointer;

  &::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--np-primary-container, white);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.3);
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
  /* 面板恒为深色玻璃, 文字不能继承浅色主题的全局黑字 */
  color: rgba(255,255,255,0.9);

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

  .np-more-header-action {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 6px 12px 6px 10px;
    border: none;
    border-radius: 999px;
    background: rgba(255,255,255,0.1);
    color: rgba(255,255,255,0.85);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s;

    .material-symbols-rounded { font-size: 18px; }
    &:hover { background: rgba(255,255,255,0.16); }
  }
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

.np-more-list-check {
  margin-left: auto;
  font-size: 20px;
  color: var(--md-primary, #d0bcff);
}

// Android 风格列表项
.np-more-list-item {
  display: flex;
  align-items: center;
  gap: 14px;
  width: 100%;
  min-width: 0;
  margin-top: 4px;
  padding: 12px 14px 12px 12px;
  border: none;
  background: rgba(255,255,255,0.04);
  color: rgba(255,255,255,0.88);
  cursor: pointer;
  border-radius: 16px;
  transition: background 0.18s, transform 0.18s cubic-bezier(0.2, 0, 0, 1);

  &:hover {
    background: rgba(255,255,255,0.09);
    .np-more-chevron {
      color: rgba(255,255,255,0.55) !important;
      transform: translateX(2px);
    }
    > .material-symbols-rounded:first-child {
      background: color-mix(in srgb, var(--md-primary, #D0BCFF) 26%, transparent);
      color: var(--md-primary, #D0BCFF);
    }
  }

  &:active { transform: scale(0.985); }

  > .material-symbols-rounded:first-child {
    font-size: 21px;
    color: rgba(255,255,255,0.72);
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 13px;
    background: rgba(255,255,255,0.08);
    transition: background 0.18s, color 0.18s;
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
  font-size: 14.5px;
  font-weight: 600;
  letter-spacing: 0.1px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.np-more-list-desc {
  font-size: 12px;
  color: rgba(255,255,255,0.45);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.np-more-chevron {
  font-size: 20px !important;
  color: rgba(255,255,255,0.28) !important;
  transition: color 0.18s, transform 0.18s cubic-bezier(0.2, 0, 0, 1);
}

// 分组：组与组之间用细线隔开，组内是 Android 式列表项
.np-more-group + .np-more-group {
  margin-top: 10px;
  padding-top: 6px;
  border-top: 1px solid rgba(255,255,255,0.06);
}

// 带行内加减的列表项：左边整块点进详细页，右边直接调整，不用来回进出子页
.np-more-list-item--stepper {
  padding: 0 10px 0 0;
  cursor: default;

  &:active { transform: none; }
}

.np-more-stepper-main {
  display: flex;
  align-items: center;
  gap: 14px;
  flex: 1;
  min-width: 0;
  padding: 12px 0 12px 12px;
  border: none;
  background: transparent;
  color: inherit;
  cursor: pointer;
  text-align: left;

  > .material-symbols-rounded:first-child {
    font-size: 21px;
    color: rgba(255,255,255,0.72);
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 13px;
    background: rgba(255,255,255,0.08);
    transition: background 0.18s, color 0.18s;
  }

  &:hover > .material-symbols-rounded:first-child {
    background: color-mix(in srgb, var(--md-primary, #D0BCFF) 26%, transparent);
    color: var(--md-primary, #D0BCFF);
  }
}

.np-more-stepper {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}

.np-more-stepper button,
.np-more-step-btn {
  width: 34px;
  height: 34px;
  flex-shrink: 0;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgba(255,255,255,0.78);
  background: rgba(255,255,255,0.08);
  transition: background 0.15s, color 0.15s;

  .material-symbols-rounded { font-size: 20px; }

  &:hover:not(:disabled) {
    background: color-mix(in srgb, var(--md-primary, #D0BCFF) 26%, transparent);
    color: var(--md-primary, #D0BCFF);
  }

  &:disabled {
    opacity: 0.35;
    cursor: default;
  }
}

// 这首歌单独调过偏移时用主色标出
.np-more-list-desc--custom {
  color: var(--md-primary, #D0BCFF);
}

.np-more-text-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-top: 4px;
  padding: 6px 14px;
  border-radius: var(--radius-full, 999px);
  color: var(--md-primary, #D0BCFF);
  background: rgba(255,255,255,0.06);
  font-size: 13px;
  font-weight: 600;
  transition: background 0.15s;

  .material-symbols-rounded { font-size: 18px; }

  &:hover { background: rgba(255,255,255,0.1); }
}

.np-more-effects {
  padding: 0 2px 4px;
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

  &--below { margin-top: 12px; }
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
  transition: max-height 240ms cubic-bezier(0.2, 0, 0, 1);
  &::-webkit-scrollbar { display: none; }

  /* 选中候选后收紧列表, 给字段选择面板让出高度, 避免按钮被顶出可视区 */
  &.compact {
    max-height: 176px;
  }
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

.np-more-search-cover-fallback {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 20px;
  color: rgba(255,255,255,0.5);
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
  /* 显式取色: 候选预览等非按钮容器里不能依赖继承(浅色主题下会变黑) */
  color: rgba(255,255,255,0.92);
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

.np-track-detail-cover-fallback {
  display: flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  font-size: 28px;
  color: rgba(255,255,255,0.6);
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

</style>
