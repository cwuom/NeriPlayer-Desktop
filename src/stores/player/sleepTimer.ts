import { computed, ref } from 'vue'
import type { PlaybackControl } from './types'

/** 睡眠定时器：倒计时、本曲结束、队列结束三种模式 */
export function createSleepTimer(pause: PlaybackControl) {
  const sleepTimerEndMs = ref(0) // 0 = 未启用
  const sleepTimerMode = ref<'countdown' | 'end_of_track' | 'end_of_queue' | null>(null)
  const sleepTimerNowMs = ref(Date.now())
  let _sleepTimerInterval: ReturnType<typeof setInterval> | null = null

  /** 剩余睡眠时间（秒） */
  const sleepRemainingSeconds = computed(() => {
    if (!sleepTimerMode.value || sleepTimerEndMs.value <= 0) return 0
    if (sleepTimerMode.value === 'end_of_track') return -1 // 特殊标记
    return Math.max(0, Math.ceil((sleepTimerEndMs.value - sleepTimerNowMs.value) / 1000))
  })

  function startSleepTimer(minutes: number) {
    cancelSleepTimer()
    const now = Date.now()
    sleepTimerMode.value = 'countdown'
    sleepTimerNowMs.value = now
    sleepTimerEndMs.value = now + minutes * 60 * 1000
    _sleepTimerInterval = setInterval(() => {
      const now = Date.now()
      sleepTimerNowMs.value = now
      if (now >= sleepTimerEndMs.value) {
        void pause('local_safety')
        cancelSleepTimer()
      }
    }, 1000)
  }

  function startSleepTimerEndOfTrack() {
    cancelSleepTimer()
    sleepTimerMode.value = 'end_of_track'
    sleepTimerEndMs.value = 1 // 非零表示启用
  }

  function startSleepTimerEndOfQueue() {
    cancelSleepTimer()
    sleepTimerMode.value = 'end_of_queue'
    sleepTimerEndMs.value = 1
  }

  function cancelSleepTimer() {
    if (_sleepTimerInterval) {
      clearInterval(_sleepTimerInterval)
      _sleepTimerInterval = null
    }
    sleepTimerMode.value = null
    sleepTimerEndMs.value = 0
    sleepTimerNowMs.value = Date.now()
  }

  return {
    sleepTimerMode, sleepRemainingSeconds,
    startSleepTimer, startSleepTimerEndOfTrack, startSleepTimerEndOfQueue, cancelSleepTimer,
  }
}
