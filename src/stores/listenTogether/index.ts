/**
 * 一起听 Pinia Store
 * 房间管理、WebSocket 通信、播放同步
 */
import { defineStore } from 'pinia'
import { ref, computed, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager'
import { usePlayerStore } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import i18n from '@/i18n'
import type {
  ConnectionState,
  LtRole,
  ListenTogetherRoomState,
  ListenTogetherRoomSettings,
  ListenTogetherSocketEnvelope,
  ListenTogetherEvent,
  ListenTogetherInitialSnapshot,
} from './protocol'
import { desktopRepeatToWire } from './protocol'
import { trackInfoToLtTrack, ltTrackToTrackInfo, toShareableQueueSnapshot } from './mapper'
import { createLogger } from '@/utils/logger'

const log = createLogger('listen-together')

const LT_UUID_KEY = 'neri:lt-uuid'
const LT_SESSION_GENERATION_KEY = 'neri:lt-session-generation'
const DEFAULT_BASE_URL = 'https://neriplayer.hancat.work'

// 进度纠偏阈值
const DRIFT_SOFT_MS = 800
const DRIFT_FORCE_MS = 2500
const CONTROL_EVENT_DEDUP_MS = 350
const SEEK_EVENT_DEDUP_MS = 800
const SEEK_EVENT_MIN_DELTA_MS = 300
const LOCAL_SEEK_REPORT_DEBOUNCE_MS = 450

// 心跳间隔
const HEARTBEAT_INTERVAL_MS = 10_000

// 重连配置
const RECONNECT_DELAYS = [1000, 2000, 4000, 8000, 16000, 30000]

interface LtBackendSessionEvent {
  sessionId: number
}

interface LtBackendMessageEvent extends LtBackendSessionEvent {
  envelope: ListenTogetherSocketEnvelope
}

interface LtBackendDisconnectedEvent extends LtBackendSessionEvent {
  code: number
  reason: string
}

export const useListenTogetherStore = defineStore('listenTogether', () => {
  const settings = useSettingsStore()

  // 状态
  const connectionState = ref<ConnectionState>('disconnected')
  const roomId = ref<string | null>(null)
  const userUuid = ref(loadOrCreateUuid())
  const role = ref<LtRole | null>(null)
  const roomState = ref<ListenTogetherRoomState | null>(null)
  const sessionError = ref<string | null>(null)
  const lastSyncEventType = ref<string | null>(null)
  const lastSyncAt = ref<number | null>(null)
  const lastReconnectAt = ref<number | null>(null)

  // 从 settings store 读取（双向绑定）
  const baseUrl = computed({
    get: () => settings.ltServerUrl || DEFAULT_BASE_URL,
    set: (v: string) => { settings.ltServerUrl = v },
  })
  const nickname = computed({
    get: () => settings.ltNickname || `User-${userUuid.value.slice(0, 4)}`,
    set: (v: string) => { settings.ltNickname = v },
  })
  const roomSettings = computed({
    get: () => ({
      allowMemberControl: settings.ltAllowMemberControl,
      autoPauseOnMemberChange: settings.ltAutoPauseOnMemberChange,
      shareAudioLinks: settings.ltShareAudioLinks,
    }),
    set: (v: ListenTogetherRoomSettings) => {
      settings.ltAllowMemberControl = v.allowMemberControl
      settings.ltAutoPauseOnMemberChange = v.autoPauseOnMemberChange
      settings.ltShareAudioLinks = v.shareAudioLinks
    },
  })

  // 内部状态
  let _heartbeatTimer: ReturnType<typeof setInterval> | null = null
  let _sessionGeneration = loadInitialSessionGeneration()
  let _backendSessionQueue: Promise<void> = Promise.resolve()
  let _reconnectAttempt = 0
  let _reconnectTimer: ReturnType<typeof setTimeout> | null = null
  let _pendingPlayerSyncTimer: ReturnType<typeof setTimeout> | null = null
  let _suppressPlayerWatchTimer: ReturnType<typeof setTimeout> | null = null
  let _wsUrl: string | null = null
  let _unlistenMessage: UnlistenFn | null = null
  let _unlistenConnected: UnlistenFn | null = null
  let _unlistenDisconnected: UnlistenFn | null = null
  let _suppressPlayerWatch = false
  let _lastAppliedRoomVersion = 0
  // 回环抑制：避免自己发出的事件触发回环
  const _recentOutboundEventIds = new Set<string>()
  // 记录最后上报的 track id，避免重复上报
  let _lastReportedTrackId: string | null = null
  let _lastReportedIsPlaying: boolean | null = null
  let _lastReportedRepeatMode: number | null = null
  let _lastReportedShuffle: boolean | null = null
  let _lastSentControlType: string | null = null
  let _lastSentControlAt = 0
  let _lastSentSeekPosition: number | null = null
  let _lastSentSeekAt = 0
  let _pendingSeekReport: { positionMs: number; trackId: string | null } | null = null
  let _pendingSeekTimer: ReturnType<typeof setTimeout> | null = null

  // 计算属性
  const isConnected = computed(() => connectionState.value === 'connected')
  const isController = computed(() => role.value === 'controller')
  const members = computed(() => roomState.value?.members ?? [])

  function currentStreamUrlForSharing(
    player: ReturnType<typeof usePlayerStore>,
  ): string | undefined {
    if (!roomSettings.value.shareAudioLinks) return undefined
    const streamUrl = player.currentResolvedStreamUrl?.trim()
    if (!streamUrl || !/^https?:\/\//i.test(streamUrl)) return undefined
    return streamUrl
  }

  function isCurrentSession(generation: number) {
    if (generation !== _sessionGeneration) return false
    try {
      return generation === loadSessionGeneration()
    } catch {
      return false
    }
  }

  function advanceSessionGeneration() {
    if (!Number.isSafeInteger(_sessionGeneration) || _sessionGeneration < 0) {
      throw new Error('invalid persisted session generation')
    }
    const persistedGeneration = loadSessionGeneration()
    const currentGeneration = Math.max(_sessionGeneration, persistedGeneration)
    if (currentGeneration >= Number.MAX_SAFE_INTEGER) {
      throw new Error('Listen Together session generation exhausted')
    }
    const nextGeneration = currentGeneration + 1
    persistSessionGeneration(nextGeneration)
    _sessionGeneration = nextGeneration
    return _sessionGeneration
  }

  function isCurrentRoomSession(generation: number, expectedRoomId: string | null) {
    return isCurrentSession(generation) && roomId.value === expectedRoomId
  }

  function cancelReconnect() {
    if (_reconnectTimer) {
      clearTimeout(_reconnectTimer)
      _reconnectTimer = null
    }
    _reconnectAttempt = 0
  }

  function cancelPendingPlayerSync() {
    if (_pendingPlayerSyncTimer) {
      clearTimeout(_pendingPlayerSyncTimer)
      _pendingPlayerSyncTimer = null
    }
  }

  function cancelPlayerWatchRelease() {
    if (_suppressPlayerWatchTimer) {
      clearTimeout(_suppressPlayerWatchTimer)
      _suppressPlayerWatchTimer = null
    }
  }

  function resetLocalSession() {
    stopHeartbeat()
    teardownPlayerWatch()
    teardownListeners()
    cancelReconnect()
    cancelPendingPlayerSync()
    cancelPlayerWatchRelease()
    _suppressPlayerWatch = false

    roomId.value = null
    role.value = null
    roomState.value = null
    _lastAppliedRoomVersion = 0
    sessionError.value = null
    connectionState.value = 'disconnected'
    lastSyncEventType.value = null
    lastSyncAt.value = null
    lastReconnectAt.value = null
    _wsUrl = null
    _lastReportedTrackId = null
    _lastReportedIsPlaying = null
    _lastReportedRepeatMode = null
    _lastReportedShuffle = null
    _lastSentControlType = null
    _lastSentControlAt = 0
    _lastSentSeekPosition = null
    _lastSentSeekAt = 0
    _recentOutboundEventIds.clear()
  }

  function beginSessionAttempt() {
    const departedSessionId = _sessionGeneration
    const generation = advanceSessionGeneration()
    resetLocalSession()
    connectionState.value = 'connecting'
    return { generation, departedSessionId }
  }

  function rollbackSession(generation: number, error: string) {
    if (!isCurrentSession(generation)) return
    resetLocalSession()
    sessionError.value = error
  }

  async function disconnectBackend(sessionId: number) {
    try {
      await invoke('lt_disconnect_ws', { sessionId })
    } catch (e) {
      log.error('disconnect failed:', e)
    }
  }

  function runBackendSessionOperation<T>(operation: () => Promise<T>): Promise<T> {
    const result = _backendSessionQueue.then(operation, operation)
    _backendSessionQueue = result.then(() => undefined, () => undefined)
    return result
  }

  // 房间操作
  /** 创建房间 */
  async function createRoom() {
    const player = usePlayerStore()
    const toast = useToastStore()
    const t = (i18n.global as any).t
    const { generation, departedSessionId } = beginSessionAttempt()

    try {
      // 构建初始快照
      const { queue: ltQueue, resolvedIndex } = toShareableQueueSnapshot(
        player.queue,
        player.queueIndex,
        roomSettings.value.shareAudioLinks,
        currentStreamUrlForSharing(player),
      )

      const snapshot: ListenTogetherInitialSnapshot = {
        queue: ltQueue,
        currentIndex: resolvedIndex,
        track: player.currentTrack
          ? trackInfoToLtTrack(player.currentTrack, currentStreamUrlForSharing(player))
          : undefined,
        settings: roomSettings.value,
        isPlaying: player.isPlaying,
        positionMs: player.positionMs,
        // Align Android ListenTogetherInitialSnapshot (ExoPlayer ints)
        repeatMode: desktopRepeatToWire(player.repeatMode),
        shuffleEnabled: !!player.shuffleEnabled,
      }

      const resp = await runBackendSessionOperation(async () => {
        if (!isCurrentSession(generation)) return null
        await disconnectBackend(departedSessionId)
        if (!isCurrentSession(generation)) return null
        return invoke<any>('lt_create_room', {
          baseUrl: baseUrl.value,
          userUuid: userUuid.value,
          nickname: nickname.value,
          initialSnapshot: snapshot,
        })
      })
      if (!isCurrentSession(generation) || !resp) return

      if (!resp.ok) {
        throw new Error(resp.error || 'Create room failed')
      }

      roomId.value = resp.roomId
      role.value = (resp.role as LtRole) || 'controller'
      if (resp.state) {
        roomState.value = resp.state
        _lastAppliedRoomVersion = resp.state.version || 0
        markSync('INITIAL_STATE', resp.state.updatedAt || Date.now())
      }

      // 连接 WebSocket
      const wsUrl = resp.wsUrl || buildWsUrl(baseUrl.value, resp.roomId, resp.token)
      await connectWs(wsUrl, generation)
      if (!isCurrentSession(generation)) return

      startHeartbeat()
      setupPlayerWatch()

    } catch (e) {
      if (!isCurrentSession(generation)) return
      const msg = e instanceof Error ? e.message : String(e)
      rollbackSession(generation, msg)
      await runBackendSessionOperation(() => disconnectBackend(generation))
      toast.error(t('listen_together.create_failed', { msg }))
    }
  }

  /** 加入房间 */
  async function joinRoom(targetRoomId: string) {
    const toast = useToastStore()
    const t = (i18n.global as any).t
    const { generation, departedSessionId } = beginSessionAttempt()

    try {
      const resp = await runBackendSessionOperation(async () => {
        if (!isCurrentSession(generation)) return null
        await disconnectBackend(departedSessionId)
        if (!isCurrentSession(generation)) return null
        return invoke<any>('lt_join_room', {
          baseUrl: baseUrl.value,
          roomId: targetRoomId,
          userUuid: userUuid.value,
          nickname: nickname.value,
        })
      })
      if (!isCurrentSession(generation) || !resp) return

      if (!resp.ok) {
        throw new Error(resp.error || 'Join room failed')
      }

      roomId.value = targetRoomId
      role.value = (resp.role as LtRole) || 'listener'
      const initialState = resp.state as ListenTogetherRoomState | undefined

      const wsUrl = resp.wsUrl || buildWsUrl(baseUrl.value, targetRoomId, resp.token)
      await connectWs(wsUrl, generation)
      if (!isCurrentSession(generation)) return

      if (initialState && (!roomState.value || roomState.value.version < initialState.version)) {
        roomState.value = initialState
        _lastAppliedRoomVersion = initialState.version || 0
        roomSettings.value = initialState.settings || roomSettings.value
        markSync('INITIAL_STATE', initialState.updatedAt || Date.now())
        applyRoomStateToPlayer(initialState, 'join', initialState.playback?.basePositionMs)
      } else if (roomState.value) {
        applyRoomStateToPlayer(
          roomState.value,
          'join',
          roomState.value.playback?.basePositionMs,
        )
      }

      setupPlayerWatch()

    } catch (e) {
      if (!isCurrentSession(generation)) return
      const msg = e instanceof Error ? e.message : String(e)
      rollbackSession(generation, msg)
      await runBackendSessionOperation(() => disconnectBackend(generation))
      toast.error(t('listen_together.join_failed', { msg }))
    }
  }

  /** 离开房间 */
  async function leaveRoom() {
    const departedSessionId = _sessionGeneration
    let generationError: Error | null = null
    try {
      if (!Number.isSafeInteger(departedSessionId) || departedSessionId < 0) {
        throw new Error('invalid persisted session generation')
      }
      const persistedGeneration = loadSessionGeneration()
      if (persistedGeneration < departedSessionId) {
        throw new Error('persisted session generation moved backwards')
      }
      if (persistedGeneration === departedSessionId) {
        advanceSessionGeneration()
      }
    } catch (e) {
      generationError = e instanceof Error ? e : new Error(String(e))
    }
    resetLocalSession()
    if (Number.isSafeInteger(departedSessionId) && departedSessionId >= 0) {
      await Promise.all([
        disconnectBackend(departedSessionId),
        runBackendSessionOperation(() => disconnectBackend(departedSessionId)),
      ])
    }
    if (generationError) throw generationError
  }

  // WebSocket 连接
  async function connectWs(wsUrl: string, generation: number) {
    if (!isCurrentSession(generation)) return
    _wsUrl = wsUrl
    await setupListeners(generation)
    if (!isCurrentSession(generation)) return
    await invoke('lt_connect_ws', { wsUrl, sessionId: generation })
  }

  async function setupListeners(generation: number) {
    const listeningRoomId = roomId.value
    const [unlistenMessage, unlistenConnected, unlistenDisconnected] = await Promise.all([
      listen<LtBackendMessageEvent>('lt:message', (event) => {
        if (!isCurrentRoomSession(generation, listeningRoomId)) return
        if (event.payload.sessionId !== generation) return
        handleSocketMessage(event.payload.envelope)
      }),
      listen<LtBackendSessionEvent>('lt:connected', (event) => {
        if (!isCurrentRoomSession(generation, listeningRoomId)) return
        if (event.payload.sessionId !== generation) return
        const reconnected = _reconnectAttempt > 0
        if (_reconnectTimer) {
          clearTimeout(_reconnectTimer)
          _reconnectTimer = null
        }
        connectionState.value = 'connected'
        if (reconnected) {
          lastReconnectAt.value = Date.now()
          markSync('RECONNECTED', lastReconnectAt.value)
        }
        _reconnectAttempt = 0
      }),
      listen<LtBackendDisconnectedEvent>('lt:disconnected', (event) => {
        if (!isCurrentRoomSession(generation, listeningRoomId)) return
        if (event.payload.sessionId !== generation) return
        const wasConnected = connectionState.value === 'connected'
        connectionState.value = 'disconnected'

        // 是否需要重连
        if (wasConnected) {
          scheduleReconnect(generation)
        }
      }),
    ])

    if (!isCurrentRoomSession(generation, listeningRoomId)) {
      unlistenMessage()
      unlistenConnected()
      unlistenDisconnected()
      return
    }

    teardownListeners()
    _unlistenMessage = unlistenMessage
    _unlistenConnected = unlistenConnected
    _unlistenDisconnected = unlistenDisconnected
  }

  function teardownListeners() {
    _unlistenMessage?.()
    _unlistenConnected?.()
    _unlistenDisconnected?.()
    _unlistenMessage = null
    _unlistenConnected = null
    _unlistenDisconnected = null
  }

  // 消息处理
  function handleSocketMessage(envelope: ListenTogetherSocketEnvelope) {
    switch (envelope.type) {
      case 'welcome':
        handleWelcome(envelope)
        break
      case 'room_state_updated':
        handleRoomStateUpdated(envelope)
        break
      case 'link_requested':
        handleLinkRequested(envelope)
        break
      case 'member_control_requested':
        handleMemberControlRequested(envelope)
        break
      case 'room_suspended':
        handleRoomSuspended(envelope)
        break
      case 'room_resumed':
        handleRoomResumed(envelope)
        break
      case 'room_closed':
        handleRoomClosed(envelope)
        break
      case 'pong':
        // 心跳回复，忽略
        break
      case 'event_applied':
        // 事件确认，忽略
        break
      default:
        log.debug('unknown message type:', envelope.type)
    }
  }

  function handleWelcome(envelope: ListenTogetherSocketEnvelope) {
    if (envelope.role) {
      role.value = envelope.role as LtRole
    }
    if (envelope.state) {
      if ((envelope.state.version || 0) < _lastAppliedRoomVersion) return
      roomState.value = envelope.state
      _lastAppliedRoomVersion = envelope.state.version || 0
      roomSettings.value = envelope.state.settings || roomSettings.value
      markSync('WELCOME', envelope.state.updatedAt || Date.now())
      applyRoomStateToPlayer(
        envelope.state,
        'welcome',
        envelope.state.playback?.basePositionMs,
      )
    }
  }

  function handleRoomStateUpdated(envelope: ListenTogetherSocketEnvelope) {
    if (!envelope.state) return
    if ((envelope.state.version || 0) < _lastAppliedRoomVersion) return

    // Echo suppression
    if (envelope.causedBy?.eventId && _recentOutboundEventIds.has(envelope.causedBy.eventId)) {
      _recentOutboundEventIds.delete(envelope.causedBy.eventId)
      // 仍然更新 roomState 但不应用到 player
      roomState.value = envelope.state
      _lastAppliedRoomVersion = envelope.state.version || 0
      roomSettings.value = envelope.state.settings || roomSettings.value
      markSync(envelope.causedBy?.type || 'STATE_SYNC', envelope.state.updatedAt || Date.now())
      return
    }

    roomState.value = envelope.state
    _lastAppliedRoomVersion = envelope.state.version || 0
    roomSettings.value = envelope.state.settings || roomSettings.value
    markSync(envelope.causedBy?.type || 'STATE_SYNC', envelope.state.updatedAt || Date.now())

    applyRoomStateToPlayer(
      envelope.state,
      envelope.causedBy?.type || 'state_update',
      envelope.expectedPositionMs,
    )
  }

  function handleLinkRequested(envelope: ListenTogetherSocketEnvelope) {
    if (
      !isController.value
      || connectionState.value !== 'connected'
      || !roomSettings.value.shareAudioLinks
    ) return

    const player = usePlayerStore()
    const requestedStableKey = envelope.requestTrackStableKey
    const requestedTrack = player.currentTrack
    const generation = _sessionGeneration
    if (!requestedStableKey || !requestedTrack) return

    const sendLinkReadyIfCurrent = (resolvedStreamUrl?: string) => {
      if (
        !isCurrentSession(generation)
        || !isController.value
        || connectionState.value !== 'connected'
        || !roomSettings.value.shareAudioLinks
        || player.currentTrack !== requestedTrack
      ) return

      const streamUrl = currentStreamUrlForSharing(player)
      if (!streamUrl || (resolvedStreamUrl && streamUrl !== resolvedStreamUrl)) return
      const currentLt = trackInfoToLtTrack(player.currentTrack, streamUrl)
      if (requestedStableKey !== currentLt.stableKey) return

      const { queue, resolvedIndex } = toShareableQueueSnapshot(
        player.queue,
        player.queueIndex,
        true,
        streamUrl,
      )
      const sharedTrack = queue[resolvedIndex]
      if (!sharedTrack || sharedTrack.stableKey !== requestedStableKey) return

      sendEvent({
        type: 'LINK_READY',
        requestTrackStableKey: requestedStableKey,
        track: sharedTrack,
        queue,
        currentIndex: resolvedIndex,
        state: player.isPlaying ? 'playing' : 'paused',
        positionMs: player.positionMs,
      })
    }

    if (currentStreamUrlForSharing(player)) {
      sendLinkReadyIfCurrent()
      return
    }

    if (typeof player.resolveCurrentStreamUrl !== 'function') return
    void player.resolveCurrentStreamUrl()
      .then(resolvedStreamUrl => {
        if (!resolvedStreamUrl) return
        sendLinkReadyIfCurrent(resolvedStreamUrl)
      })
      .catch(() => {})
  }

  function handleMemberControlRequested(envelope: ListenTogetherSocketEnvelope) {
    // 房主处理听众的控制请求
    if (!isController.value) return

    const player = usePlayerStore()
    const causeType = envelope.causedBy?.type

    _suppressPlayerWatch = true
    try {
      switch (causeType) {
        case 'REQUEST_PLAY':
          if (!player.isPlaying) player.togglePlayPause('local')
          reportPlayEvent()
          break
        case 'REQUEST_PAUSE':
          if (player.isPlaying) player.togglePlayPause('local')
          reportPauseEvent()
          break
        case 'REQUEST_SEEK':
          if (envelope.positionMs != null) {
            player.seekTo(envelope.positionMs, 'local')
            reportSeekEvent(envelope.positionMs)
          }
          break
        case 'REQUEST_SET_TRACK':
          if (envelope.track) {
            const trackInfo = ltTrackToTrackInfo(envelope.track)
            player.play(trackInfo, 'local')
            reportSetTrackEvent(envelope.track, envelope.currentIndex ?? 0)
          }
          break
        case 'REQUEST_PLAYBACK_MODE': {
          // Align Android: controller commits PLAYBACK_MODE for member request
          const repeatMode = envelope.repeatMode
            ?? envelope.state?.playback?.repeatMode
            ?? undefined
          const shuffleEnabled = envelope.shuffleEnabled
            ?? envelope.state?.playback?.shuffleEnabled
            ?? undefined
          player.applyListenTogetherPlaybackMode({
            repeatMode: typeof repeatMode === 'number' ? repeatMode : null,
            shuffleEnabled: typeof shuffleEnabled === 'boolean' ? shuffleEnabled : null,
          })
          reportPlaybackModeEvent()
          break
        }
      }
    } finally {
      schedulePlayerWatchRelease(350)
    }
  }

  function handleRoomSuspended(_envelope: ListenTogetherSocketEnvelope) {
    const toast = useToastStore()
    const t = (i18n.global as any).t
    markSync('ROOM_SUSPENDED')
    toast.error(t('listen_together.controller_offline'))
  }

  function handleRoomResumed(_envelope: ListenTogetherSocketEnvelope) {
    markSync('ROOM_RESUMED')
  }

  function handleRoomClosed(_envelope: ListenTogetherSocketEnvelope) {
    const toast = useToastStore()
    const t = (i18n.global as any).t
    toast.error(t('listen_together.room_closed'))
    leaveRoom()
  }

  // 播放器同步
  function applyRoomStateToPlayer(
    state: ListenTogetherRoomState,
    causeType: string,
    expectedPositionMs?: number,
  ) {
    const player = usePlayerStore()
    cancelPendingPlayerSync()
    cancelPlayerWatchRelease()
    if (!state.track) {
      _suppressPlayerWatch = false
      return
    }

    _suppressPlayerWatch = true
    const generation = _sessionGeneration
    const applyingRoomId = roomId.value

    try {
      const remoteTrack = ltTrackToTrackInfo(state.track)
      const remoteIsPlaying = state.playback.state === 'playing'

      // 计算期望位置
      let expectedPos = expectedPositionMs ?? state.playback.basePositionMs
      if (remoteIsPlaying && state.playback.baseTimestampMs > 0) {
        const elapsed = Date.now() - state.playback.baseTimestampMs
        expectedPos = state.playback.basePositionMs + Math.max(0, elapsed) * state.playback.playbackRate
      }

      // 对比当前曲目
      const currentId = player.currentTrack?.id
      if (currentId !== remoteTrack.id) {
        // 需要切歌时，同时更新队列
        if (state.queue.length > 0) {
          const newQueue = state.queue.map(ltTrackToTrackInfo)
          player.queue.splice(0, player.queue.length, ...newQueue)
          const newIndex = Math.max(0, Math.min(state.currentIndex, newQueue.length - 1))
          player.queueIndex = newIndex
        }
        // 使用 remote_sync source 播放
        player.play(remoteTrack, 'remote_sync')
        // 播放后对齐进度与播放态
        _pendingPlayerSyncTimer = setTimeout(() => {
          _pendingPlayerSyncTimer = null
          if (!isCurrentRoomSession(generation, applyingRoomId)) return
          if (roomState.value?.track?.stableKey !== state.track?.stableKey) return
          if (player.currentTrack?.id !== remoteTrack.id) return
          if (expectedPos > 1000) {
            player.seekTo(expectedPos, 'remote_sync')
          }
          if (!remoteIsPlaying) {
            player.pause('remote_sync')
          }
        }, 300)
        _lastReportedTrackId = remoteTrack.id
        _lastReportedIsPlaying = remoteIsPlaying
        player.applyListenTogetherPlaybackMode({
          repeatMode: state.playback.repeatMode,
          shuffleEnabled: state.playback.shuffleEnabled,
        })
        _lastReportedRepeatMode = typeof state.playback.repeatMode === 'number'
          ? state.playback.repeatMode
          : desktopRepeatToWire(player.repeatMode)
        _lastReportedShuffle = typeof state.playback.shuffleEnabled === 'boolean'
          ? state.playback.shuffleEnabled
          : !!player.shuffleEnabled
        return
      }

      // 对比播放状态
      if (player.isPlaying !== remoteIsPlaying) {
        if (remoteIsPlaying) {
          player.resume('remote_sync')
        } else {
          player.pause('remote_sync')
        }
        _lastReportedIsPlaying = remoteIsPlaying
      }

      // 进度纠偏
      const diff = Math.abs(player.positionMs - expectedPos)
      if (diff > DRIFT_SOFT_MS) {
        player.seekTo(expectedPos, 'remote_sync')
      }

      // Align Android applyListenTogetherPlaybackMode
      player.applyListenTogetherPlaybackMode({
        repeatMode: state.playback.repeatMode,
        shuffleEnabled: state.playback.shuffleEnabled,
      })
      _lastReportedRepeatMode = typeof state.playback.repeatMode === 'number'
        ? state.playback.repeatMode
        : desktopRepeatToWire(player.repeatMode)
      _lastReportedShuffle = typeof state.playback.shuffleEnabled === 'boolean'
        ? state.playback.shuffleEnabled
        : !!player.shuffleEnabled
    } finally {
      // 延迟恢复 watch，避免同步操作触发上报
      schedulePlayerWatchRelease(500, generation, applyingRoomId)
    }
  }

  function schedulePlayerWatchRelease(
    delayMs: number,
    generation = _sessionGeneration,
    expectedRoomId = roomId.value,
  ) {
    cancelPlayerWatchRelease()
    _suppressPlayerWatchTimer = setTimeout(() => {
      _suppressPlayerWatchTimer = null
      if (!isCurrentRoomSession(generation, expectedRoomId)) return
      _suppressPlayerWatch = false
    }, delayMs)
  }

  // 本地变化上报
  let _playerWatchStop: (() => void) | null = null
  let _seekWatchStop: (() => void) | null = null

  function setupPlayerWatch() {
    teardownPlayerWatch()
    const player = usePlayerStore()

    _playerWatchStop = watch(
      () => ({
        trackId: player.currentTrack?.id,
        isPlaying: player.isPlaying,
        repeatMode: player.repeatMode,
        shuffleEnabled: player.shuffleEnabled,
      }),
      (newVal) => {
        if (_suppressPlayerWatch || player.isRemoteSyncGuardActive() || connectionState.value !== 'connected') return

        // 曲目变化
        if (newVal.trackId && newVal.trackId !== _lastReportedTrackId) {
          _lastReportedTrackId = newVal.trackId
          if (player.currentTrack) {
            const ltTrack = trackInfoToLtTrack(player.currentTrack)
            if (isController.value) {
              reportSetTrackEvent(ltTrack, player.queueIndex)
            } else {
              sendRequestEvent('REQUEST_SET_TRACK', { track: ltTrack, currentIndex: player.queueIndex })
            }
          }
        }

        // 播放状态变化
        if (newVal.isPlaying !== _lastReportedIsPlaying) {
          _lastReportedIsPlaying = newVal.isPlaying
          if (newVal.isPlaying) {
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
          wireRepeat !== _lastReportedRepeatMode
          || wireShuffle !== _lastReportedShuffle
        ) {
          _lastReportedRepeatMode = wireRepeat
          _lastReportedShuffle = wireShuffle
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
        if (_suppressPlayerWatch || connectionState.value !== 'connected') return
        const seek = player.lastSeekCommand
        if (seek.source !== 'local') return
        if (player.isRemoteSyncGuardActive()) return

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

  // 事件发送
  function generateEventId(): string {
    return `${userUuid.value.slice(0, 8)}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`
  }

  async function sendEvent(event: ListenTogetherEvent) {
    const generation = _sessionGeneration
    const sendingRoomId = roomId.value
    if (!event.eventId) event.eventId = generateEventId()
    if (!event.clientTimeMs) event.clientTimeMs = Date.now()

    _recentOutboundEventIds.add(event.eventId!)
    // 清理过期 ID（保留最近 50 个）
    if (_recentOutboundEventIds.size > 50) {
      const iter = _recentOutboundEventIds.values()
      _recentOutboundEventIds.delete(iter.next().value!)
    }

    try {
      const sent = await invoke<boolean>('lt_send_event', {
        event,
        sessionId: generation,
      })
      if (!sent) {
        handleSendFailure(generation, sendingRoomId, 'WebSocket is not connected')
      }
    } catch (e) {
      log.error('send event failed:', e)
      handleSendFailure(
        generation,
        sendingRoomId,
        e instanceof Error ? e.message : String(e),
      )
    }
  }

  function handleSendFailure(
    generation: number,
    sendingRoomId: string | null,
    error: string,
  ) {
    if (!isCurrentRoomSession(generation, sendingRoomId) || !sendingRoomId) return
    sessionError.value = error
    const wasConnected = connectionState.value === 'connected'
    connectionState.value = 'disconnected'
    if (wasConnected) scheduleReconnect(generation)
  }

  function shouldSkipControlEvent(type: string) {
    const now = Date.now()
    if (_lastSentControlType === type && now - _lastSentControlAt < CONTROL_EVENT_DEDUP_MS) {
      return true
    }
    _lastSentControlType = type
    _lastSentControlAt = now
    return false
  }

  function shouldSkipSeekEvent(positionMs: number) {
    const now = Date.now()
    if (
      _lastSentSeekPosition !== null
      && Math.abs(_lastSentSeekPosition - positionMs) < SEEK_EVENT_MIN_DELTA_MS
      && now - _lastSentSeekAt < SEEK_EVENT_DEDUP_MS
    ) {
      return true
    }
    _lastSentSeekPosition = positionMs
    _lastSentSeekAt = now
    return false
  }

  function reportPlayEvent() {
    const player = usePlayerStore()
    if (shouldSkipControlEvent('PLAY')) return
    sendEvent({
      type: 'PLAY',
      positionMs: player.positionMs,
      state: 'playing',
    })
  }

  function reportPauseEvent() {
    const player = usePlayerStore()
    if (shouldSkipControlEvent('PAUSE')) return
    sendEvent({
      type: 'PAUSE',
      positionMs: player.positionMs,
      state: 'paused',
    })
  }

  function reportSeekEvent(positionMs: number) {
    if (shouldSkipSeekEvent(positionMs)) return
    sendEvent({
      type: 'SEEK',
      positionMs,
    })
  }

  function reportPlaybackModeEvent() {
    const player = usePlayerStore()
    if (shouldSkipControlEvent('PLAYBACK_MODE')) return
    const repeatMode = desktopRepeatToWire(player.repeatMode)
    const shuffleEnabled = !!player.shuffleEnabled
    _lastReportedRepeatMode = repeatMode
    _lastReportedShuffle = shuffleEnabled
    sendEvent({
      type: 'PLAYBACK_MODE',
      repeatMode,
      shuffleEnabled,
      positionMs: player.positionMs,
      state: player.isPlaying ? 'playing' : 'paused',
    })
  }

  function scheduleLocalSeekReport(positionMs: number) {
    const player = usePlayerStore()
    _pendingSeekReport = {
      positionMs,
      trackId: player.currentTrack?.id ?? null,
    }

    if (_pendingSeekTimer) {
      clearTimeout(_pendingSeekTimer)
    }

    _pendingSeekTimer = setTimeout(() => {
      flushPendingSeekReport()
    }, LOCAL_SEEK_REPORT_DEBOUNCE_MS)
  }

  function flushPendingSeekReport() {
    if (_pendingSeekTimer) {
      clearTimeout(_pendingSeekTimer)
      _pendingSeekTimer = null
    }

    const pending = _pendingSeekReport
    _pendingSeekReport = null
    if (!pending || connectionState.value !== 'connected') return

    const player = usePlayerStore()
    if (pending.trackId && player.currentTrack?.id !== pending.trackId) return

    if (isController.value) {
      reportSeekEvent(pending.positionMs)
    } else {
      if (shouldSkipSeekEvent(pending.positionMs)) return
      sendRequestEvent('REQUEST_SEEK', { positionMs: pending.positionMs })
    }
  }

  function clearPendingSeekReport() {
    if (_pendingSeekTimer) {
      clearTimeout(_pendingSeekTimer)
      _pendingSeekTimer = null
    }
    _pendingSeekReport = null
  }

  function reportSetTrackEvent(track: any, currentIndex: number) {
    const player = usePlayerStore()
    const streamUrl = currentStreamUrlForSharing(player)
    const { queue: ltQueue, resolvedIndex } = toShareableQueueSnapshot(
      player.queue,
      player.queueIndex,
      roomSettings.value.shareAudioLinks,
      streamUrl,
    )
    sendEvent({
      type: 'SET_TRACK',
      track: ltQueue[resolvedIndex]
        ?? (player.currentTrack ? trackInfoToLtTrack(player.currentTrack, streamUrl) : track),
      currentIndex: ltQueue.length > 0 ? resolvedIndex : currentIndex,
      queue: ltQueue,
      positionMs: 0,
      shouldPlay: player.isPlaying,
    })
  }

  function sendRequestEvent(type: string, extra: Partial<ListenTogetherEvent> = {}) {
    const player = usePlayerStore()
    sendEvent({
      type,
      positionMs: player.positionMs,
      ...extra,
    })
  }

  // 心跳
  function startHeartbeat() {
    stopHeartbeat()
    if (!isController.value) return

    _heartbeatTimer = setInterval(() => {
      if (connectionState.value !== 'connected') return

      const player = usePlayerStore()
      const streamUrl = currentStreamUrlForSharing(player)
      const { queue: ltQueue, resolvedIndex } = toShareableQueueSnapshot(
        player.queue,
        player.queueIndex,
        roomSettings.value.shareAudioLinks,
        streamUrl,
      )

      sendEvent({
        type: 'HEARTBEAT',
        positionMs: player.positionMs,
        state: player.isPlaying ? 'playing' : 'paused',
        queue: ltQueue,
        currentIndex: resolvedIndex,
        track: player.currentTrack
          ? trackInfoToLtTrack(player.currentTrack, streamUrl)
          : undefined,
        repeatMode: desktopRepeatToWire(player.repeatMode),
        shuffleEnabled: !!player.shuffleEnabled,
      })
    }, HEARTBEAT_INTERVAL_MS)
  }

  function stopHeartbeat() {
    if (_heartbeatTimer) {
      clearInterval(_heartbeatTimer)
      _heartbeatTimer = null
    }
  }

  // 断线重连
  function scheduleReconnect(generation = _sessionGeneration) {
    if (!isCurrentSession(generation)) return
    if (_reconnectTimer) return

    const reconnectRoomId = roomId.value
    const reconnectWsUrl = _wsUrl
    if (!reconnectRoomId || !reconnectWsUrl) return
    const delay = RECONNECT_DELAYS[Math.min(_reconnectAttempt, RECONNECT_DELAYS.length - 1)]
    _reconnectAttempt++

    _reconnectTimer = setTimeout(async () => {
      _reconnectTimer = null
      if (!isCurrentRoomSession(generation, reconnectRoomId)) return

      connectionState.value = 'connecting'
      try {
        await invoke('lt_connect_ws', {
          wsUrl: reconnectWsUrl,
          sessionId: generation,
        })
        if (!isCurrentRoomSession(generation, reconnectRoomId)) return
        // 重连后拉取最新 state
        const stateResp = await invoke<any>('lt_get_room_state', {
          baseUrl: baseUrl.value,
          roomId: reconnectRoomId,
        })
        if (!isCurrentRoomSession(generation, reconnectRoomId)) return
        if (stateResp.ok && stateResp.state) {
          roomState.value = stateResp.state
          _lastAppliedRoomVersion = stateResp.state.version || 0
          applyRoomStateToPlayer(stateResp.state, 'reconnect', stateResp.expectedPositionMs)
        }
        if (isController.value) startHeartbeat()
      } catch {
        if (isCurrentRoomSession(generation, reconnectRoomId)) {
          connectionState.value = 'disconnected'
          scheduleReconnect(generation)
        }
      }
    }, delay)
  }

  // 房间设置更新
  async function updateRoomSettings(newSettings: Partial<ListenTogetherRoomSettings>) {
    if (newSettings.allowMemberControl !== undefined) settings.ltAllowMemberControl = newSettings.allowMemberControl
    if (newSettings.autoPauseOnMemberChange !== undefined) settings.ltAutoPauseOnMemberChange = newSettings.autoPauseOnMemberChange
    if (newSettings.shareAudioLinks !== undefined) settings.ltShareAudioLinks = newSettings.shareAudioLinks
    if (isController.value && connectionState.value === 'connected') {
      sendEvent({
        type: 'UPDATE_SETTINGS',
        roomSettings: roomSettings.value,
      })
    }
  }

  // 邀请链接
  function getInviteLink(): string {
    return `neriplayer://listen-together/join?roomId=${roomId.value}&baseUrl=${encodeURIComponent(baseUrl.value)}`
  }

  async function copyInviteLink() {
    const link = getInviteLink()
    try {
      await writeText(link)
      const toast = useToastStore()
      const t = (i18n.global as any).t
      toast.success(t('listen_together.invite_copied'))
    } catch {}
  }

  /** 检测剪贴板中的邀请链接 */
  async function checkClipboardInvite(): Promise<{ roomId: string; baseUrl?: string } | null> {
    try {
      const text = await readText()
      if (!text) return null
      const match = text.match(/neriplayer:\/\/listen-together\/join\?roomId=([^&]+)(?:&baseUrl=([^&\s]+))?/)
      if (match) {
        return {
          roomId: match[1],
          baseUrl: match[2] ? decodeURIComponent(match[2]) : undefined,
        }
      }
    } catch {}
    return null
  }

  // 工具函数
  function buildWsUrl(base: string, roomId: string, token: string): string {
    const normalized = base.replace(/\/$/, '')
    const httpUrl = `${normalized}/api/rooms/${roomId}/ws?token=${encodeURIComponent(token)}`
    return httpUrl.replace(/^http/, 'ws')
  }

  function loadOrCreateUuid(): string {
    let uuid = localStorage.getItem(LT_UUID_KEY)
    if (!uuid) {
      uuid = crypto.randomUUID()
      localStorage.setItem(LT_UUID_KEY, uuid)
    }
    return uuid
  }

  function loadSessionGeneration(): number {
    try {
      const stored = localStorage.getItem(LT_SESSION_GENERATION_KEY)
      if (stored === null) return 0
      if (!/^(0|[1-9]\d*)$/.test(stored)) {
        throw new Error('invalid persisted session generation')
      }

      const generation = Number(stored)
      if (!Number.isSafeInteger(generation) || generation < 0) {
        throw new Error('invalid persisted session generation')
      }
      return generation
    } catch (e) {
      log.error('failed to load session generation:', e)
      throw e
    }
  }

  function loadInitialSessionGeneration(): number {
    try {
      return loadSessionGeneration()
    } catch {
      return Number.NaN
    }
  }

  function persistSessionGeneration(generation: number) {
    try {
      localStorage.setItem(LT_SESSION_GENERATION_KEY, String(generation))
    } catch (e) {
      log.error('failed to persist session generation:', e)
      throw e
    }
  }

  function markSync(eventType: string, timestamp = Date.now()) {
    lastSyncEventType.value = eventType
    lastSyncAt.value = timestamp
  }

  return {
    // 状态
    connectionState, roomId, userUuid, nickname, role,
    roomState, sessionError, baseUrl, roomSettings,
    lastSyncEventType, lastSyncAt, lastReconnectAt,
    // 计算属性
    isConnected, isController, members,
    // 方法
    createRoom, joinRoom, leaveRoom,
    updateRoomSettings, copyInviteLink, getInviteLink,
    checkClipboardInvite,
    // 暴露给外部（seek 上报）
    reportSeekEvent,
  }
})
