import type { ComputedRef, Ref } from 'vue'
import type { LyricLine, TrackInfo } from '@/types/track'
import type { CommentTarget } from '@/modules/playback/commentTarget'
import type { useAudioQuality } from './useAudioQuality'

/** 按曲目 ID 前缀判定的播放来源，未识别的一律视为本地 */
export type PlaybackSource = 'netease' | 'qq' | 'bilibili' | 'youtube' | 'local'

/** 更多选项面板的子视图 */
export type MoreSheetView =
  | 'main' | 'offset' | 'fontsize' | 'effects' | 'search' | 'editinfo'
  | 'quality' | 'lyrics-editor' | 'lyrics-fill' | 'track-detail' | 'bili-subtitles'

export type InfoSearchPlatform = 'netease' | 'bilibili' | 'youtube' | 'qq'

/** 只取 vue-i18n 的按 key 翻译能力，纯函数可直接传入 t */
export type Translate = (key: string) => string

/** 后端 search 的条目；「获取歌曲信息」只读这些字段，部分来源带 camelCase 封面与 platform */
export interface InfoSearchResult {
  id?: string | number | null
  title?: string | null
  artist?: string | null
  source?: string | null
  platform?: string | null
  cover_url?: string | null
  coverUrl?: string | null
  duration_ms?: number | null
  synced_lyrics?: string | null
  translated_lyrics?: string | null
  plain_lyrics?: string | null
}

export interface NeteaseSongDetail {
  songs?: Array<{
    al?: { id?: number } | null
    album?: { id?: number } | null
    ar?: Array<{ id?: number; name?: string } | null> | null
  } | null> | null
}

export interface YoutubeCreator {
  id: string
  name: string
  cover_url: string | null
}

export interface ArtistLink {
  id: string
  name: string
  route: { name: string; params: Record<string, string>; query?: Record<string, string> }
}

/** 正在播放页用到的后端命令：只在 NowPlaying.vue 实现一次，子模块经注入调用 */
export interface NowPlayingIpc {
  parseLrc: (content: string) => Promise<unknown[]>
  search: (query: string, platform: InfoSearchPlatform, includeLyrics: boolean) => Promise<InfoSearchResult[]>
  getNeteaseSongDetail: (songId: number) => Promise<NeteaseSongDetail | null>
  resolveYoutubeCreators: (artist: string) => Promise<YoutubeCreator[]>
}

/** 歌词编辑器三轨的初始文本 */
export interface EditableLyrics {
  original: string
  translation: string
  romanization: string
}

/**
 * 更多选项面板与页面共享的状态和动作。面板原本是页面模板的一部分，
 * 这里沿用页面里的同名 ref 与函数，保证拆出后读写的仍是同一份状态
 */
export interface NowPlayingSheetContext {
  ipc: NowPlayingIpc
  emit: (event: 'collapse') => void
  showMoreSheet: Ref<boolean>
  hideMoreSheet: () => void
  moreSheetView: Readonly<Ref<MoreSheetView>>
  moreSheetTransition: Readonly<Ref<string>>
  goToSubView: (view: MoreSheetView) => void
  goBackTo: (view: MoreSheetView) => void
  goBackToMain: () => void
  coverUrl: Readonly<Ref<string>>
  coverLoadError: Readonly<Ref<boolean>>
  handleNowPlayingCoverError: (event: Event) => Promise<void>
  albumName: ComputedRef<string>
  playbackSourceLabel: ComputedRef<string>
  commentTarget: ComputedRef<CommentTarget | null>
  quality: ReturnType<typeof useAudioQuality>
  editableLyricsOfCurrentTrack: () => EditableLyrics
  cacheLyricsForTrack: (track: TrackInfo | null | undefined, lines: LyricLine[]) => void
  removeCachedLyricsForCurrentTrack: () => void
  commitLyricsToTrack: (
    nextLyric: string | null,
    nextTranslated: string | null,
    source?: string | null,
    nextRomanized?: string | null,
    resetOffset?: boolean,
  ) => Promise<void>
  copyText: (text: string) => Promise<void>
  shareSong: () => Promise<void>
  openQualitySwitcher: () => void
  openBiliVideoSkip: () => void
  openListenTogetherFromMore: () => void
  openComments: () => void
  openAudioEffectsSettings: () => void
}
