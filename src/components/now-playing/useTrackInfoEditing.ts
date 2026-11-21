import { nextTick, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { usePlayerStore } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import { useCurrentLyricsStore } from '@/stores/currentLyrics'
import { toEditableLyricsText, toEditableTranslationText } from '@/modules/lyrics/lyricsFormat'
import { fetchLyrics } from '@/modules/lyrics/lyricsFetch'
import { rememberLyricSource } from '@/modules/lyrics/lyricSource'
import { persistTrackSyncPayload, withUpdatedCustomInfoPayload } from '@/modules/lyrics/syncTrackPayload'
import { createLogger } from '@/utils/logger'
import { lyricSourceForPlatform, parseLyricsFromSearchResult } from './searchResultLyrics'
import type { InfoSearchPlatform, InfoSearchResult, NowPlayingSheetContext } from './types'

const log = createLogger('now-playing')

/** 「获取歌曲信息」搜索并按字段回填，以及手工「编辑歌曲信息」 */
export function useTrackInfoEditing(context: NowPlayingSheetContext) {
  const { ipc, goToSubView, goBackToMain, cacheLyricsForTrack, commitLyricsToTrack } = context
  const player = usePlayerStore()
  const toast = useToastStore()
  const currentLyrics = useCurrentLyricsStore()
  const { t } = useI18n()

  // 获取歌曲信息（搜索）
  const searchQuery = ref('')
  const searchResults = ref<InfoSearchResult[]>([])
  const isSearching = ref(false)
  const infoSearchPlatform = ref<InfoSearchPlatform>('netease')
  const infoApplyCandidate = ref<InfoSearchResult | null>(null)
  const applyInfoFields = ref({
    title: true,
    artist: true,
    cover: true,
    lyrics: true,
  })

  /** 搜过之后才显示「无搜索结果」 */
  const infoSearched = ref(false)
  let infoSearchRequest = 0

  // 打开即按当前歌名搜一次（对齐 Android），不用再手动点搜索
  function openInfoSearch() {
    searchQuery.value = player.currentTrack?.title || ''
    searchResults.value = []
    infoApplyCandidate.value = null
    infoSearched.value = false
    applyInfoFields.value = { title: true, artist: true, cover: true, lyrics: true }
    goToSubView('search')
    void doSearch()
  }

  function selectInfoSearchPlatform(platform: InfoSearchPlatform) {
    if (infoSearchPlatform.value === platform) return
    infoSearchPlatform.value = platform
    void doSearch()
  }

  async function doSearch() {
    const q = searchQuery.value.trim()
    const request = ++infoSearchRequest
    searchResults.value = []
    infoApplyCandidate.value = null
    if (!q) {
      isSearching.value = false
      infoSearched.value = false
      return
    }
    isSearching.value = true
    const platform = infoSearchPlatform.value
    try {
      const results = await ipc.search(q, platform, platform === 'qq')
      if (request === infoSearchRequest) searchResults.value = results
    } catch (e) {
      log.error('Search failed:', e)
    } finally {
      if (request === infoSearchRequest) {
        isSearching.value = false
        infoSearched.value = true
      }
    }
  }

  const fieldPickerRef = ref<HTMLElement | null>(null)

  function applySearchResult(result: InfoSearchResult) {
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
        const direct = await parseLyricsFromSearchResult(result, ipc.parseLrc)
        if (direct && direct.lines.length > 0) {
          rememberLyricSource(player.currentTrack, source)
          currentLyrics.replace(direct.lines)
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
            const actualSource = fetched.source ? lyricSourceForPlatform(fetched.source) : source
            rememberLyricSource(player.currentTrack, actualSource)
            currentLyrics.replace(nextLyrics)
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

  return {
    searchQuery,
    searchResults,
    isSearching,
    infoSearchPlatform,
    infoApplyCandidate,
    applyInfoFields,
    infoSearched,
    fieldPickerRef,
    openInfoSearch,
    selectInfoSearchPlatform,
    doSearch,
    applySearchResult,
    confirmApplySearchResult,
    editTitle,
    editArtist,
    editCoverUrl,
    openEditInfo,
    saveEditInfo,
    restoreInfo,
  }
}
