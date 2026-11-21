import { computed, nextTick, onUnmounted, ref, watch } from 'vue'
import { usePlayerStore } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import type { AudioInfo } from '@/modules/playback/playbackAudioInfo'
import { actualAudioParameterLabels } from '@/modules/playback/audioQualityDisplay'
import { usePlaybackAudioInfoDisplay } from '@/composables/usePlaybackAudioInfoDisplay'
import { addAudioInfoPart, isHiddenAudioInfoToken } from './audioInfoDisplay'

/**
 * 进度条下方的音质行（不展示 Local / download 占位）。
 * 参数开关统一控制文件和在线流，平台音质标签单独显示
 */
export function useAudioInfoRow(currentAudioQualityLabel: (info: AudioInfo | null, fromDownload: boolean) => string) {
  const player = usePlayerStore()
  const settings = useSettingsStore()

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

  return { displayedAudioInfo, audioInfoParts, audioDetailEl, wrappedAudioSeparators }
}
