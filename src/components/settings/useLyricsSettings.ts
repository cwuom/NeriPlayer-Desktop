import { computed, ref } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { DEFAULT_LYRIC_SOURCES, useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import { DEFAULT_LYRIC_OFFSET_MS } from '@/modules/lyrics/lyricOffset'
import { closeDesktopLyricsWindow, openDesktopLyricsWindow } from '@/modules/desktopLyrics/bridge'
import type { DesktopLyricsFrameLine } from '@/modules/desktopLyrics/frame'
import {
  DEFAULT_DESKTOP_LYRICS_STYLE,
  DESKTOP_LYRICS_FONT_PRESETS,
  DESKTOP_LYRICS_SECONDARY,
  DESKTOP_LYRICS_THEMES,
  DESKTOP_LYRICS_WEIGHTS,
  applyDesktopLyricsTheme,
  normalizeDesktopLyricsStyle,
  parseCssColor,
  type DesktopLyricsStyle,
} from '@/modules/desktopLyrics/style'
import { settingsLog as log } from './settingsShared'

// 各歌词来源的默认偏移，顺序与默认值对齐 Android 设置页
export const lyricOffsetSettings = [
  { key: 'cloudMusicOffset', label: 'settings.netease_offset', defaultMs: DEFAULT_LYRIC_OFFSET_MS.netease, iconSvg: '/icons/ic_netease.svg' },
  { key: 'qqMusicOffset', label: 'settings.qq_offset', defaultMs: DEFAULT_LYRIC_OFFSET_MS.qq, iconSvg: '/icons/ic_qq_music.svg' },
  { key: 'kugouOffset', label: 'settings.kugou_offset', defaultMs: DEFAULT_LYRIC_OFFSET_MS.kugou, iconSvg: '/icons/ic_kugou.svg' },
  { key: 'lrclibOffset', label: 'settings.lrclib_offset', defaultMs: DEFAULT_LYRIC_OFFSET_MS.lrclib, iconSvg: '/icons/ic_lrclib.svg' },
  { key: 'amllTtmlOffset', label: 'settings.amll_ttml_offset', defaultMs: DEFAULT_LYRIC_OFFSET_MS.amll_ttml, iconSvg: '/icons/ic_amll.svg' },
  { key: 'biliCcSubtitleOffset', label: 'bili_subtitle.offset', defaultMs: DEFAULT_LYRIC_OFFSET_MS.bilibili_cc, iconSvg: '/icons/ic_bilibili.svg' },
] as const

export const CUSTOM_FONT = '__custom__'

// 预览：两句示例歌词循环播放，第一句逐字
const PREVIEW_LOOP_MS = 7_600

export function desktopLyricsPreviewLines(line: string, translation: string, next: string): DesktopLyricsFrameLine[] {
  const characters = Array.from(line)
  const step = Math.max(120, Math.floor(3_000 / Math.max(1, characters.length)))
  const words = characters.map((text, index) => ({ startMs: 400 + index * step, durationMs: step, text }))
  const firstEnd = 400 + characters.length * step
  return [
    { startMs: 400, durationMs: firstEnd - 400, text: characters.join(''), translation, roman: 'desktop lyrics preview', words },
    { startMs: firstEnd + 300, durationMs: 3_000, text: next, translation: '', roman: '', words: [] },
  ]
}

export function desktopLyricsThemeSwatch(id: string, accentColor: string | null): string {
  if (id === 'accent') {
    const accent = accentColor || '#d0bcff'
    return `linear-gradient(135deg, #ffffff, ${accent})`
  }
  const theme = DESKTOP_LYRICS_THEMES.find(candidate => candidate.id === id)
  return theme ? `linear-gradient(135deg, ${theme.played[0]}, ${theme.played[1]})` : 'transparent'
}

/** 用户点过「自定义」或当前字体不在预设里时，下拉框停在自定义项 */
export function desktopLyricsFontChoice(family: string, custom: boolean): string {
  if (custom) return CUSTOM_FONT
  return !family || (DESKTOP_LYRICS_FONT_PRESETS as readonly string[]).includes(family) ? family : CUSTOM_FONT
}

/** 歌词与桌面歌词分区。忙碌、自定义字体与强调色状态要跨分区切换保留，因此由设置页持有 */
export function useLyricsSettings() {
  const { t } = useI18n()
  const toast = useToastStore()
  const settings = useSettingsStore()
  const { defaultLyricSource } = storeToRefs(settings)

  const defaultLyricSourceOptions = computed(() => DEFAULT_LYRIC_SOURCES.map(value => ({
    value,
    label: t(`settings.lyric_source_${value}`),
  })))
  const defaultLyricSourceDescription = computed(() => t(`settings.lyric_source_${defaultLyricSource.value}_desc`))

  function changeDefaultLyricSource(value: string) {
    const source = DEFAULT_LYRIC_SOURCES.find(source => source === value)
    if (source) defaultLyricSource.value = source
  }

  const lyricOffsetsChanged = computed(() => lyricOffsetSettings.some(item => settings[item.key] !== item.defaultMs))

  function resetLyricOffsets() {
    for (const item of lyricOffsetSettings) settings[item.key] = item.defaultMs
  }

  // ---- 桌面歌词 ----
  const desktopLyricsStyle = computed(() => settings.desktopLyrics)

  function updateDesktopLyrics(patch: Partial<DesktopLyricsStyle>) {
    settings.desktopLyrics = normalizeDesktopLyricsStyle({ ...settings.desktopLyrics, ...patch })
  }

  /** 改任一颜色就变成自定义配色 */
  function setDesktopLyricsGradient(key: 'playedColors' | 'unplayedColors', index: 0 | 1, value: string) {
    const colors = [...desktopLyricsStyle.value[key]] as [string, string]
    colors[index] = value
    updateDesktopLyrics({ [key]: colors, theme: 'custom' })
  }

  function chooseDesktopLyricsTheme(id: string) {
    settings.desktopLyrics = applyDesktopLyricsTheme(desktopLyricsStyle.value, id)
  }

  function resetDesktopLyrics() {
    const { bounds, locked } = desktopLyricsStyle.value
    settings.desktopLyrics = normalizeDesktopLyricsStyle({ ...DEFAULT_DESKTOP_LYRICS_STYLE, bounds, locked })
  }

  const desktopLyricsBusy = ref(false)
  async function toggleDesktopLyrics(open: boolean) {
    if (desktopLyricsBusy.value) return
    desktopLyricsBusy.value = true
    try {
      if (open) await openDesktopLyricsWindow()
      else await closeDesktopLyricsWindow()
    } catch (error) {
      log.error('Desktop lyrics toggle failed:', error)
      toast.error(t('desktop_lyrics.open_failed'))
    } finally {
      desktopLyricsBusy.value = false
    }
  }

  const desktopLyricsCustomFont = ref(false)
  const desktopLyricsFontOptions = computed(() => [
    { value: '', label: t('desktop_lyrics.font_default') },
    ...DESKTOP_LYRICS_FONT_PRESETS.map(font => ({ value: font, label: font })),
    { value: CUSTOM_FONT, label: t('desktop_lyrics.font_custom') },
  ])
  const desktopLyricsFontChoiceValue = computed(() => desktopLyricsFontChoice(desktopLyricsStyle.value.fontFamily, desktopLyricsCustomFont.value))
  function chooseDesktopLyricsFont(value: string) {
    desktopLyricsCustomFont.value = value === CUSTOM_FONT
    if (value !== CUSTOM_FONT) updateDesktopLyrics({ fontFamily: value })
  }
  const desktopLyricsWeightOptions = computed(() => DESKTOP_LYRICS_WEIGHTS.map(weight => ({ value: String(weight), label: String(weight) })))
  const desktopLyricsSecondaryOptions = computed(() => DESKTOP_LYRICS_SECONDARY.map(value => ({
    value, label: t(`desktop_lyrics.secondary_${value}`),
  })))

  const desktopLyricsAccent = ref<string | null>(null)
  const previewLines = computed(() => desktopLyricsPreviewLines(
    t('desktop_lyrics.preview_line'), t('desktop_lyrics.preview_translation'), t('desktop_lyrics.preview_next'),
  ))
  function desktopLyricsPreviewClock(): number {
    return performance.now() % PREVIEW_LOOP_MS
  }
  function refreshDesktopLyricsAccent() {
    desktopLyricsAccent.value = parseCssColor(getComputedStyle(document.documentElement).getPropertyValue('--md-primary'))
  }

  return {
    defaultLyricSourceOptions,
    defaultLyricSourceDescription,
    changeDefaultLyricSource,
    lyricOffsetsChanged,
    resetLyricOffsets,
    desktopLyricsStyle,
    updateDesktopLyrics,
    setDesktopLyricsGradient,
    chooseDesktopLyricsTheme,
    resetDesktopLyrics,
    desktopLyricsBusy,
    toggleDesktopLyrics,
    desktopLyricsFontOptions,
    desktopLyricsFontChoice: desktopLyricsFontChoiceValue,
    chooseDesktopLyricsFont,
    desktopLyricsWeightOptions,
    desktopLyricsSecondaryOptions,
    themeSwatch: (id: string) => desktopLyricsThemeSwatch(id, desktopLyricsAccent.value),
    desktopLyricsAccent,
    desktopLyricsPreviewLines: previewLines,
    desktopLyricsPreviewClock,
    refreshDesktopLyricsAccent,
  }
}

export type LyricsSettings = ReturnType<typeof useLyricsSettings>
