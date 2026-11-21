import { watch, type ComputedRef } from 'vue'
import { usePlayerStore } from '@/stores/player'
import { desktopRepeatToWire, type ListenTogetherEvent, type ListenTogetherRoomSettings, type ListenTogetherTrack } from './protocol'
import { toShareableQueueSnapshot, trackInfoToLtTrack } from './mapper'
import type { LtSession } from './session'

export interface LtPlayerWatchContext {
  session: LtSession
  isController: ComputedRef<boolean>
  roomSettings: ComputedRef<ListenTogetherRoomSettings>
  playbackIdentity: () => string | null
  reportSetTrackEvent: (track: ListenTogetherTrack, currentIndex: number) => void
  sendRequestEvent: (type: string, extra?: Partial<ListenTogetherEvent>) => void
  intendsToPlay: () => boolean
  reportQueueEvent: () => void
  reportPlayEvent: () => void
  reportPauseEvent: () => void
  reportPlaybackModeEvent: () => void
  shouldSkipControlEvent: (type: string) => boolean
  scheduleLocalSeekReport: (positionMs: number) => void
  clearPendingSeekReport: () => void
}

/** 本地变化上报：观察播放器的切歌、队列、播放状态、循环/随机与 seek，按身份上报给房间 */
export function createLtPlayerWatch(ctx: LtPlayerWatchContext) {
  const {
    session, isController, roomSettings, playbackIdentity, reportSetTrackEvent, sendRequestEvent, intendsToPlay,
    reportQueueEvent, reportPlayEvent, reportPauseEvent, reportPlaybackModeEvent, shouldSkipControlEvent,
    scheduleLocalSeekReport, clearPendingSeekReport,
  } = ctx

  let _playerWatchStop: (() => void) | null = null
  let _seekWatchStop: (() => void) | null = null

  function setupPlayerWatch() {
    teardownPlayerWatch()
    const player = usePlayerStore()
    session.lastReportedTrackId = playbackIdentity()
    session.lastReportedQueueKeys = player.queue.map(track => trackInfoToLtTrack(track).stableKey)
    session.lastReportedIsPlaying = player.isPlaying
    session.lastReportedRepeatMode = desktopRepeatToWire(player.repeatMode)
    session.lastReportedShuffle = !!player.shuffleEnabled
    let observedCommandSequence = player.lastCommandSequence
    let observedSeekSequence = player.lastSeekCommand.seq

    _playerWatchStop = watch(
      () => ({
        trackId: playbackIdentity(),
        queueKeys: player.queue.map(track => trackInfoToLtTrack(track).stableKey),
        isPlaying: player.isPlaying,
        isLoadingAudio: player.isLoadingAudio,
        repeatMode: player.repeatMode,
        shuffleEnabled: player.shuffleEnabled,
        commandSource: player.lastCommandSource,
        commandSequence: player.lastCommandSequence,
        seekSequence: player.lastSeekCommand.seq,
      }),
      (newVal) => {
        const newLocalCommand = newVal.commandSource === 'local'
          && newVal.commandSequence !== observedCommandSequence
        const newLocalSeek = player.lastSeekCommand.source === 'local'
          && !player.lastSeekCommand.fromPlaybackStart
          && newVal.seekSequence !== observedSeekSequence
        observedCommandSequence = newVal.commandSequence
        observedSeekSequence = newVal.seekSequence
        // 断线时也照常上报：发不出去会走 HTTP，再不行进待补发队列，不能让本地操作被重连时的房态撤回
        if (!newLocalCommand && (session.suppressPlayerWatch || session.pendingRemotePlaybackLoads.size > 0 || player.isRemoteSyncGuardActive())) return
        if (newLocalCommand && (!newLocalSeek || newVal.trackId !== session.lastReportedTrackId)) {
          // 后来的本地播放意图取代旧加载，旧 Promise 完成后不能再调整新歌曲
          session.playbackApplySequence++
          session.pendingRemotePlaybackLoads.clear()
          session.suppressPlayerWatch = false
          if (session.watchReleaseTimer) clearTimeout(session.watchReleaseTimer)
          session.watchReleaseTimer = null
        }

        // 曲目变化
        let trackReported = false
        if (newVal.trackId && newVal.trackId !== session.lastReportedTrackId) {
          session.lastReportedTrackId = newVal.trackId
          if (player.currentTrack) {
            const ltTrack = trackInfoToLtTrack(player.currentTrack)
            if (isController.value) {
              reportSetTrackEvent(ltTrack, player.queueIndex)
              trackReported = true
            } else {
              // REQUEST_SET_TRACK 需带完整共享队列与 resolvedIndex（对齐 Android buildRequestSetTrackEvent）
              const { queue: ltQueue, resolvedIndex } = toShareableQueueSnapshot(
                player.queue,
                player.queueIndex,
                roomSettings.value.shareAudioLinks,
              )
              const track = ltQueue[resolvedIndex]
              if (!track) return
              sendRequestEvent('REQUEST_SET_TRACK', {
                track,
                currentIndex: resolvedIndex,
                queue: ltQueue,
                requestTrackStableKey: track.stableKey,
                shouldPlay: intendsToPlay(),
              })
              trackReported = true
            }
          }
        }

        if (trackReported) session.lastReportedQueueKeys = newVal.queueKeys
        else if (newVal.queueKeys.length !== session.lastReportedQueueKeys.length
          || newVal.queueKeys.some((key, index) => key !== session.lastReportedQueueKeys[index])) {
          reportQueueEvent()
        }

        // 播放状态变化；加载中的"暂停"不上报，否则每次换歌都会先提交暂停、加载完再提交播放
        const playing = newVal.isPlaying || (newVal.isLoadingAudio && session.lastReportedIsPlaying === true)
        if (playing !== session.lastReportedIsPlaying) {
          session.lastReportedIsPlaying = playing
          if (playing) {
            if (isController.value) reportPlayEvent()
            else if (!shouldSkipControlEvent('REQUEST_PLAY')) sendRequestEvent('REQUEST_PLAY')
          } else {
            if (isController.value) reportPauseEvent()
            else if (!shouldSkipControlEvent('REQUEST_PAUSE')) sendRequestEvent('REQUEST_PAUSE')
          }
        }

        // 循环/随机变化 -> PLAYBACK_MODE (Android-aligned)
        const wireRepeat = desktopRepeatToWire(newVal.repeatMode)
        const wireShuffle = !!newVal.shuffleEnabled
        if (
          wireRepeat !== session.lastReportedRepeatMode
          || wireShuffle !== session.lastReportedShuffle
        ) {
          session.lastReportedRepeatMode = wireRepeat
          session.lastReportedShuffle = wireShuffle
          if (isController.value) {
            reportPlaybackModeEvent()
          } else if (!shouldSkipControlEvent('REQUEST_PLAYBACK_MODE')) {
            sendRequestEvent('REQUEST_PLAYBACK_MODE', {
              repeatMode: wireRepeat,
              shuffleEnabled: wireShuffle,
            })
          }
        }
      },
      { deep: false },
    )

    _seekWatchStop = watch(
      () => player.lastSeekCommand.seq,
      () => {
        const seek = player.lastSeekCommand
        if (seek.source !== 'local') return

        scheduleLocalSeekReport(seek.positionMs)
      },
    )
  }

  function teardownPlayerWatch() {
    _playerWatchStop?.()
    _seekWatchStop?.()
    _playerWatchStop = null
    _seekWatchStop = null
    clearPendingSeekReport()
  }

  return { setupPlayerWatch, teardownPlayerWatch }
}
