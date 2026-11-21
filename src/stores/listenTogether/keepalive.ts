import { invoke } from '@tauri-apps/api/core'
import type { ComputedRef, Ref } from 'vue'
import { usePlayerStore } from '@/stores/player'
import { createLogger } from '@/utils/logger'
import {
  desktopRepeatToWire,
  type ConnectionState,
  type ListenTogetherEvent,
  type ListenTogetherRoomResponse,
  type ListenTogetherRoomSettings,
  type ListenTogetherRoomState,
  type ListenTogetherStateResponse,
  type ListenTogetherTrack,
} from './protocol'
import { toShareableQueueSnapshot, trackInfoToLtTrack } from './mapper'
import { resolveExpectedPosition, resolvePositionSync } from './playbackSync'
import { isTerminalReconnectError, MAX_RECONNECT_ATTEMPTS, reconnectDelayMs } from './reconnect'
import {
  pendingMemberRequestAction,
  roomCurrentStableKey,
  shouldRefreshListenerState,
  StallDetector,
  WATCHDOG_INTERVAL_MS,
} from './watchdog'
import type { LtSession } from './session'

const log = createLogger('listen-together')

// 心跳间隔：对齐 Android（播放 22s）, 降低大队列全量上传频率; 控制事件仍即时下发
const HEARTBEAT_INTERVAL_MS = 22_000
const PAUSED_HEARTBEAT_INTERVAL_MS = 25_000
// listener 侧存活探测间隔（房主走 HEARTBEAT，听众用 ping 保活半开连接检测）
const LISTENER_PING_INTERVAL_MS = 20_000
// ping 发出后这么久仍没收到任何消息就认为连接已断（对齐 Android LISTEN_TOGETHER_SOCKET_RESPONSE_TIMEOUT_MS）
const SOCKET_RESPONSE_TIMEOUT_MS = 35_000

export interface LtKeepaliveContext {
  session: LtSession
  roomId: Ref<string | null>
  userUuid: Ref<string>
  nickname: ComputedRef<string>
  roomState: Ref<ListenTogetherRoomState | null>
  connectionState: Ref<ConnectionState>
  isController: ComputedRef<boolean>
  roomSettings: ComputedRef<ListenTogetherRoomSettings>
  activeBaseUrl: () => string
  sampleServerClock: (serverNowMs: number | null | undefined, sentAtWallMs?: number, sentAtElapsedMs?: number) => void
  commitRoomState: (state: ListenTogetherRoomState, causeType: string, expectedPositionMs?: number, apply?: boolean) => void
  applyRoomStateToPlayer: (state: ListenTogetherRoomState, causeType: string, expectedPositionMs?: number, forceReload?: boolean) => void
  requestLinkForTrack: (track: ListenTogetherTrack, currentIndex: number, force?: boolean) => void
  setSyncRate: (rate: number | null) => void
  closeRoomLocally: (reason: string) => Promise<void>
  updateJoinSecret: (value: string | null | undefined, fallback?: string | null) => void
  resolveWsUrl: (response: ListenTogetherRoomResponse, fallbackRoomId: string) => string
  sendEvent: (event: ListenTogetherEvent) => Promise<void>
  hasPendingListenerQueueEvent: () => boolean
}

/** 连接保活：房主心跳、存活探测与半开连接检测、断线重连，以及听众端定时自检 */
export function createLtKeepalive(ctx: LtKeepaliveContext) {
  const {
    session, roomId, userUuid, nickname, roomState, connectionState, isController, roomSettings,
    activeBaseUrl, sampleServerClock, commitRoomState, applyRoomStateToPlayer, requestLinkForTrack, setSyncRate,
    closeRoomLocally, updateJoinSecret, resolveWsUrl, sendEvent, hasPendingListenerQueueEvent,
  } = ctx

  let _heartbeatTimer: ReturnType<typeof setInterval> | null = null
  let _listenerPingTimer: ReturnType<typeof setInterval> | null = null
  let _reconnectTimer: ReturnType<typeof setTimeout> | null = null
  // 听众端自检：定时对齐、卡住恢复、静默时拉房态
  let _watchdogTimer: ReturnType<typeof setInterval> | null = null
  let _lastWatchdogRefreshAt = 0
  const _stallDetector = new StallDetector()

  /** 离开房间时取消还没触发的重连 */
  function cancelReconnect() {
    if (_reconnectTimer) {
      clearTimeout(_reconnectTimer)
      _reconnectTimer = null
    }
  }

  function startWatchdog() {
    stopWatchdog()
    _lastWatchdogRefreshAt = 0
    _stallDetector.reset()
    const generation = session.generation
    _watchdogTimer = setInterval(() => {
      if (generation === session.generation) void runWatchdogTick(generation)
    }, WATCHDOG_INTERVAL_MS)
  }

  function stopWatchdog() {
    if (_watchdogTimer) clearInterval(_watchdogTimer)
    _watchdogTimer = null
  }

  /**
   * 听众端每 8 秒自检一次（对齐 Android ListenTogetherListenerWatchdogOwner）。
   * 漏掉的房态、丢掉的请求和卡住的加载不用等 22–25 秒后的下一次心跳才纠正
   */
  async function runWatchdogTick(generation: number) {
    if (isController.value || connectionState.value !== 'connected' || !roomId.value) return
    const now = Date.now()
    const serverNow = now + session.serverClockOffsetMs
    const state = roomState.value
    let requestPending = false
    if (session.pendingMemberRequest) {
      const action = pendingMemberRequestAction(session.pendingMemberRequest, state, now, serverNow)
      if (action === 'satisfied' || action === 'expired') {
        session.pendingMemberRequest = null
      } else {
        requestPending = true
        if (action === 'retry') {
          session.pendingMemberRequest = { ...session.pendingMemberRequest, lastSentAt: now, attempts: session.pendingMemberRequest.attempts + 1 }
          void sendEvent(session.pendingMemberRequest.event)
        }
      }
    }
    // 自己的请求还在等房主处理时不按房态回放，否则会把用户刚做的操作撤回去
    if (state && !requestPending && !hasPendingListenerQueueEvent() && state.roomStatus === 'active') {
      syncListenerToRoom(state, now, serverNow)
    }

    if (!shouldRefreshListenerState(now, session.lastSocketMessageAt, _lastWatchdogRefreshAt, session.stateRepairPending)) return
    _lastWatchdogRefreshAt = now
    session.stateRepairPending = false
    const targetRoomId = roomId.value
    try {
      const resp = await invoke<ListenTogetherStateResponse>('lt_get_room_state', { baseUrl: activeBaseUrl(), roomId: targetRoomId })
      if (generation !== session.generation || !resp.ok || !resp.state) return
      sampleServerClock(resp.serverNowMs)
      commitRoomState(resp.state, 'WATCHDOG_REFRESH', resp.expectedPositionMs, !session.pendingMemberRequest)
    } catch (error) {
      if (generation !== session.generation) return
      const message = error instanceof Error ? error.message : String(error)
      if (isTerminalReconnectError(message)) void closeRoomLocally(message)
      else log.warn('listener state refresh failed:', message)
    }
  }

  function syncListenerToRoom(state: ListenTogetherRoomState, now: number, serverNow: number) {
    const player = usePlayerStore()
    const roomKey = roomCurrentStableKey(state) ?? null
    const currentKey = player.currentTrack ? trackInfoToLtTrack(player.currentTrack).stableKey : null
    const roomPlaying = state.playback.state === 'playing'
    const expectedPositionMs = resolveExpectedPosition(state, serverNow)
    // 房间在播、本地同一首歌却一直在加载：重新加载并强制要一次链接（对齐 Android ListenerStallRecovery）
    const stalled = roomPlaying && !player.isPlaying && player.isLoadingAudio && !!roomKey && currentKey === roomKey
    if (_stallDetector.shouldRecover(stalled, roomKey, now)) {
      log.warn('listener playback stalled, reloading the room track:', roomKey)
      applyRoomStateToPlayer(state, 'WATCHDOG_STALL', expectedPositionMs, true)
      const index = state.currentIndex
      const track = state.track ?? state.queue[index]
      if (track) requestLinkForTrack(track, index, true)
      return
    }
    if (player.isLoadingAudio || session.pendingRemotePlaybackLoads.size > 0) return
    // 只在确实不一致时回放，免得每次自检都替换队列、打断用户操作的上报
    const sync = resolvePositionSync({
      expectedPositionMs,
      localPositionMs: player.positionMs,
      desiredPlaying: roomPlaying,
      isController: false,
      causeType: 'WATCHDOG',
    })
    if (
      currentKey !== roomKey
      || player.isPlaying !== roomPlaying
      || sync.seekTo !== undefined
      || (sync.rate !== null && session.softSyncRate === null)
    ) {
      applyRoomStateToPlayer(state, 'WATCHDOG', expectedPositionMs)
    }
  }

  // 心跳
  function reportHeartbeat() {
    if (!isController.value || !roomId.value) return
    const player = usePlayerStore()
    const { queue, resolvedIndex } = toShareableQueueSnapshot(
      player.queue, player.queueIndex, roomSettings.value.shareAudioLinks,
      player.currentTrack ? player.getCurrentStreamUrl(player.currentTrack.id) || undefined : undefined,
      false, player.getCurrentStreamUrls(),
    )
    // 当前歌无法共享时只保活：不带位置和状态的心跳不会改动房间里那首歌的进度
    if (!queue[resolvedIndex]) {
      void sendEvent({ type: 'HEARTBEAT' })
      return
    }
    void sendEvent({
      type: 'HEARTBEAT',
      positionMs: player.positionMs,
      state: player.isPlaying ? 'playing' : 'paused',
      queue: (roomState.value?.schemaVersion ?? 1) < 2 ? queue : undefined,
      currentIndex: resolvedIndex,
      track: queue[resolvedIndex],
      repeatMode: desktopRepeatToWire(player.repeatMode),
      shuffleEnabled: !!player.shuffleEnabled,
    })
  }

  function startHeartbeat() {
    stopHeartbeat()
    if (!isController.value || !roomId.value) return
    const generation = session.generation
    const tick = () => {
      if (generation !== session.generation || !isController.value || !roomId.value) return
      if (connectionState.value === 'connected') reportHeartbeat()
      _heartbeatTimer = setTimeout(tick, usePlayerStore().isPlaying ? HEARTBEAT_INTERVAL_MS : PAUSED_HEARTBEAT_INTERVAL_MS)
    }
    _heartbeatTimer = setTimeout(tick, usePlayerStore().isPlaying ? HEARTBEAT_INTERVAL_MS : PAUSED_HEARTBEAT_INTERVAL_MS)
  }

  function stopHeartbeat() {
    if (_heartbeatTimer) {
      clearInterval(_heartbeatTimer)
      _heartbeatTimer = null
    }
  }

  // 定期 ping 也用于估计时钟偏移，房主和听众都需要保持探测
  function startListenerPing() {
    stopListenerPing()
    session.pingSentAt = 0
    _listenerPingTimer = setInterval(() => {
      if (connectionState.value !== 'connected') return
      // 上一个 ping 发出后还没收到任何消息：35s 内先等着，不加发；超过就当连接已经半开，主动重连
      if (session.pingSentAt > 0 && session.lastSocketMessageAt <= session.pingSentAt) {
        if (Date.now() - session.pingSentAt >= SOCKET_RESPONSE_TIMEOUT_MS) handleUnresponsiveSocket()
        return
      }
      const sentAt = Date.now()
      session.pingSentAt = sentAt
      session.pingSentElapsed.set(sentAt, performance.now())
      while (session.pingSentElapsed.size > 8) session.pingSentElapsed.delete(session.pingSentElapsed.keys().next().value!)
      void invoke('lt_send_ping', { t: sentAt, legacy: session.legacyPing }).catch(() => {})
    }, LISTENER_PING_INTERVAL_MS)
  }

  function stopListenerPing() {
    if (_listenerPingTimer) {
      clearInterval(_listenerPingTimer)
      _listenerPingTimer = null
    }
  }

  /** TCP 要很久才会发现半开连接，这期间界面会一直显示已连接却收不到任何同步 */
  function handleUnresponsiveSocket() {
    log.warn('listen together socket stopped responding, reconnecting')
    connectionState.value = 'disconnected'
    session.activeWsConnectionId = null
    stopListenerPing()
    stopHeartbeat()
    setSyncRate(null)
    if (roomId.value) scheduleReconnect()
  }

  // 断线重连
  function scheduleReconnect() {
    if (_reconnectTimer) return
    // 对齐 Android：重连有上限，用尽后在本地结束会话，而不是一直显示"连接中"
    if (session.reconnectAttempt >= MAX_RECONNECT_ATTEMPTS) {
      void closeRoomLocally('reconnect_max_attempts_exceeded')
      return
    }
    session.reconnectAttempt++
    const delay = reconnectDelayMs(session.reconnectAttempt)

    _reconnectTimer = setTimeout(async () => {
      _reconnectTimer = null
      if (!session.wsUrl || !roomId.value) return
      const generation = session.generation
      const targetRoomId = roomId.value

      connectionState.value = 'connecting'
      try {
        // 听众每次重连前都重新入房：凭据过期或被移出成员时才能拿到新的连接地址（对齐 Android）
        if (!isController.value) {
          const resp = await invoke<ListenTogetherRoomResponse>('lt_join_room', {
            baseUrl: activeBaseUrl(),
            roomId: targetRoomId,
            userUuid: userUuid.value,
            nickname: nickname.value,
            joinSecret: session.joinSecret || undefined,
          })
          if (generation !== session.generation) return
          if (!resp.ok) throw new Error(resp.error || 'rejoin failed')
          updateJoinSecret(resp.joinSecret, session.joinSecret)
          session.wsUrl = resolveWsUrl(resp, targetRoomId)
        }
        await invoke('lt_connect_ws', { wsUrl: session.wsUrl })
        if (generation !== session.generation) return
        // 重连后拉取最新 state
        const stateResp = await invoke<ListenTogetherStateResponse>('lt_get_room_state', {
          baseUrl: activeBaseUrl(),
          roomId: targetRoomId,
        })
        if (generation !== session.generation) return
        if (stateResp.ok === false) throw new Error(stateResp.error || 'room closed')
        if (stateResp.ok && stateResp.state) {
          sampleServerClock(stateResp.serverNowMs)
          commitRoomState(stateResp.state, 'reconnect', stateResp.expectedPositionMs,
            !(isController.value && session.hostControlledOffline))
        }
        session.hostControlledOffline = false
        if (isController.value) startHeartbeat()
      } catch (error) {
        if (generation !== session.generation) return
        const message = error instanceof Error ? error.message : String(error)
        if (isTerminalReconnectError(message)) {
          await closeRoomLocally(message)
          return
        }
        scheduleReconnect()
      }
    }, delay)
  }

  return {
    startWatchdog, stopWatchdog, reportHeartbeat, startHeartbeat, stopHeartbeat,
    startListenerPing, stopListenerPing, scheduleReconnect, cancelReconnect,
  }
}
