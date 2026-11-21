import { computed, ref, watch, type Ref } from 'vue'
import { formatTimeMs as formatTime } from '@/utils/timeFormat'

/** 加载持续超过 1s 才置真, 供播放按钮转圈; 短加载不闪 spinner */
export function createSlowLoadingFlag(isLoadingAudio: Ref<boolean>) {
  const isLoadingAudioSlow = ref(false)
  let loadingSpinnerTimer: number | null = null
  watch(isLoadingAudio, (loading) => {
    if (loading) {
      if (loadingSpinnerTimer === null) {
        loadingSpinnerTimer = window.setTimeout(() => {
          loadingSpinnerTimer = null
          isLoadingAudioSlow.value = isLoadingAudio.value
        }, 1000)
      }
    } else {
      if (loadingSpinnerTimer !== null) {
        clearTimeout(loadingSpinnerTimer)
        loadingSpinnerTimer = null
      }
      isLoadingAudioSlow.value = false
    }
  })
  return isLoadingAudioSlow
}

/** 音频分析数据：后端推送的电平与节拍，供背景律动 */
export function createAudioLevel(isPlaying: Ref<boolean>) {
  const audioLevel = ref(0)
  const beatImpulse = ref(0)
  // 后端只在播放中推电平；不清零的话暂停后背景律动会定格在最后一拍
  watch(isPlaying, (playing) => {
    if (playing) return
    audioLevel.value = 0
    beatImpulse.value = 0
  })
  return { audioLevel, beatImpulse }
}

/** 进度展示：插值位置由 store 的 rAF 循环写入，这里只做派生 */
export function createProgressDisplay(positionMs: Ref<number>, durationMs: Ref<number>) {
  // 插值后的播放位置（rAF 驱动，60fps 平滑）
  const interpolatedPositionMs = ref(0)
  const interpolatedProgress = computed(() =>
    durationMs.value > 0 ? interpolatedPositionMs.value / durationMs.value : 0
  )

  const progress = computed(() =>
    durationMs.value > 0 ? positionMs.value / durationMs.value : 0
  )
  const currentTimeFormatted = computed(() => formatTime(interpolatedPositionMs.value))
  const durationFormatted = computed(() => formatTime(durationMs.value))
  return { interpolatedPositionMs, interpolatedProgress, progress, currentTimeFormatted, durationFormatted }
}
