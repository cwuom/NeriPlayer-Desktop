import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import type { LyricLine } from '@/types/track'
import { usePlayerStore } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import { useCurrentLyricsStore } from '@/stores/currentLyrics'
import { useLyricOffsetStore } from '@/stores/lyricOffset'
import {
  mapBackendLyrics,
  mergeParsedLyricsWithRomanization,
  mergeParsedLyricsWithTranslations,
  resolveKnownNeteaseLyricSongId,
  toEditableLyricsText,
  toEditableRomanizationText,
  toEditableTranslationText,
} from '@/modules/lyrics/lyricsFormat'
import { lyricMatchSourceTag, type LyricMatchResult } from '@/modules/lyrics/lyricMatch'
import { fetchNeteaseRomanization } from '@/modules/lyrics/lyricsFetch'
import { rememberLyricSource } from '@/modules/lyrics/lyricSource'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'
import type { NowPlayingSheetContext } from './types'

const log = createLogger('now-playing')

/** 歌词编辑器（对齐 Android LyricsEditorSheet：原文 / 翻译 / 音译三轨）与歌词匹配 */
export function useLyricsEditing(context: NowPlayingSheetContext) {
  const {
    ipc, moreSheetView, goToSubView, goBackTo, goBackToMain,
    editableLyricsOfCurrentTrack, cacheLyricsForTrack, removeCachedLyricsForCurrentTrack, commitLyricsToTrack,
  } = context
  const player = usePlayerStore()
  const toast = useToastStore()
  const currentLyrics = useCurrentLyricsStore()
  const lyricOffsetStore = useLyricOffsetStore()
  const { t } = useI18n()

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

  function openLyricsEditor(returnView: 'main' | 'editinfo' = 'main') {
    const initial = editableLyricsOfCurrentTrack()
    lyricsEditorTab.value = 'original'
    lyricsEditorReturnView.value = returnView
    lyricsEditorSource.value = null
    lyricsEditorText.value = initial.original
    lyricsTranslationEditorText.value = initial.translation
    lyricsRomanizationEditorText.value = initial.romanization
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
      return mapBackendLyrics(await ipc.parseLrc(text))
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
      currentLyrics.replace([])
      removeCachedLyricsForCurrentTrack()
      await commitLyricsToTrack(null, null, 'LOCAL_EDIT', null)
      toast.success(t('player.lyrics_cleared'))
      leaveLyricsEditor()
      return
    }
    try {
      // parse_lrc_content 已走 parse_auto, 支持 YRC 逐字往返
      const parsed = await ipc.parseLrc(text)
      const nextLyrics = mergeParsedLyricsWithRomanization(
        mergeParsedLyricsWithTranslations(
          mapBackendLyrics(parsed),
          await parseOptionalLyricTrack(translationText, 'translation'),
        ),
        await parseOptionalLyricTrack(romanizationText, 'romanization'),
      )
      const source = lyricsEditorSource.value ?? 'LOCAL_EDIT'
      rememberLyricSource(player.currentTrack, source)
      currentLyrics.replace(nextLyrics)
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
    const track = player.currentTrack
    if (!track) return
    try {
      const resetOffset = lyricOffsetStore.shouldResetOffsetForMatch(track, result.source)
      rememberLyricSource(track, source)
      currentLyrics.replace(result.lines)
      cacheLyricsForTrack(track, result.lines)
      // 换了一份歌词：候选没有音译时清掉旧音译，免得挂到新歌词上
      await commitLyricsToTrack(lyricText, translationText || null, source, romanizationText || null, resetOffset)
      toast.success(t('player.lyrics_fill_applied'))
      goBackToMain()
    } catch (e) {
      log.error('Apply matched lyrics failed:', e)
      toast.error(String(e))
    }
  }

  return {
    lyricsEditorText,
    lyricsTranslationEditorText,
    lyricsRomanizationEditorText,
    lyricsEditorTab,
    openLyricsEditor,
    leaveLyricsEditor,
    applyLyricsFromEditor,
    lyricMatchTarget,
    lyricMatchSession,
    openLyricMatch,
    leaveLyricMatch,
    onLyricMatchPicked,
  }
}
