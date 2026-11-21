import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { usePlayerStore } from '@/stores/player'
import { useLyricOffsetStore } from '@/stores/lyricOffset'
import { lyricSourceOf } from '@/modules/lyrics/lyricSource'
import {
  formatLyricOffsetMs,
  LYRIC_OFFSET_STEP_MS,
  MAX_LYRIC_DEFAULT_OFFSET_MS,
  MIN_LYRIC_DEFAULT_OFFSET_MS,
} from '@/modules/lyrics/lyricOffset'

/** 更多面板里的歌词偏移：主菜单行内加减与偏移子视图共用 */
export function useLyricOffsetControls() {
  const player = usePlayerStore()
  const lyricOffsetStore = useLyricOffsetStore()
  const { t } = useI18n()

  // 当前歌词用哪个来源的默认偏移：看正在显示的歌词来自哪里（AMLL TTML、酷狗、LRCLIB…），不知道时按播放来源
  const currentLyricOffsetSource = computed(() => lyricOffsetStore.offsetSourceFor(player.currentTrack))
  const currentLyricDefaultOffsetMs = computed(() =>
    lyricOffsetStore.defaultOffsetMs(currentLyricOffsetSource.value),
  )
  // 逐曲 delta：只用来判断这首歌是否单独调过
  const currentLyricUserOffsetMs = computed(() => lyricOffsetStore.getUserOffsetMs(player.currentTrack))

  // 有效偏移（绝对值）：歌词渲染用它，偏移面板显示和编辑的也是它；与桌面歌词同一个算法
  const currentLyricTotalOffsetMs = computed<number>({
    get: () => lyricOffsetStore.effectiveOffsetMs(player.currentTrack),
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
      case 'bilibili_cc': return t('bili_subtitle.title')
    }
    const raw = (lyricSourceOf(player.currentTrack) ?? '').toLowerCase().replace(/[^a-z]/g, '')
    if (raw === 'youtube') return t('player.source_youtube')
    if (raw === 'local') return t('player.lyric_source_local')
    if (raw === 'localedit') return t('player.lyric_source_edited')
    return ''
  })

  const lyricOffsetSummary = computed(() => [formatLyricOffsetMs(currentLyricTotalOffsetMs.value), currentLyricSourceLabel.value]
    .filter(Boolean).join(' · '))

  return {
    currentLyricDefaultOffsetMs,
    currentLyricUserOffsetMs,
    currentLyricTotalOffsetMs,
    lyricOffsetSliderMin,
    lyricOffsetSliderMax,
    nudgeLyricOffset,
    resetLyricOffsetToDefault,
    currentLyricSourceLabel,
    lyricOffsetSummary,
  }
}
