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
  ListenTogetherRoomResponse,
  ListenTogetherStateResponse,
  ListenTogetherControlResponse,
} from './protocol'
import {
  desktopRepeatToWire,
  isValidLtNickname,
  normalizeLtHttpBaseUrl,
  normalizeLtInviteBaseUrl,
  normalizeLtJoinSecret,
  normalizeLtRoomId,
  resolveLtJoinSecret,
  isValidLtRoomId,
} from './protocol'
import { trackInfoToLtTrack, ltTrackToTrackInfo, toShareableQueueSnapshot, trustedInboundStreamUrls } from './mapper'
import { queueReferences, applyListenTogetherQueueMutation, buildListenTogetherQueueMutationPlan, getLtQueueReference, setLtQueueReference } from './queue'
import { acceptRoomState, resolveExpectedPosition, resolvePositionSync, resolveSoftSyncRecheckAction, SOFT_SYNC_RECHECK_INTERVAL_MS } from './playbackSync'
import { createLogger } from '@/utils/logger'

const log = createLogger('listen-together')

const LT_UUID_KEY = 'neri:lt-uuid'
const DEFAULT_BASE_URL = 'https://neriplayer.hancat.work'

const LINK_REQUEST_THROTTLE_MS = 4000
const CONTROL_EVENT_DEDUP_MS = 350
const SEEK_EVENT_DEDUP_MS = 800
const SEEK_EVENT_MIN_DELTA_MS = 300
const LOCAL_SEEK_REPORT_DEBOUNCE_MS = 450

// 心跳间隔：对齐 Android（播放 22s）, 降低大队列全量上传频率; 控制事件仍即时下发
const HEARTBEAT_INTERVAL_MS = 22_000
const PAUSED_HEARTBEAT_INTERVAL_MS = 25_000
// listener 侧存活探测间隔（房主走 HEARTBEAT，听众用 ping 保活半开连接检测）
const LISTENER_PING_INTERVAL_MS = 20_000
// 已处理转发请求 eventId 上限（对齐 Android ForwardedRequestDeduper 语义）
const HANDLED_FORWARDED_EVENT_LIMIT = 256
const HANDLED_FORWARDED_REQUESTER_LIMIT = 64

// 重连配置
const RECONNECT_DELAYS = [1000, 2000, 4000, 8000, 16000, 30000]

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
    // 默认名从持久化 UUID 派生, 稳定不随机; 仅作展示回退, 不写入 settings
    // Android 昵称白名单仅数字/字母/汉字, 不含连字符; 默认名必须同样合法,
    // 否则建房/加入时上报的昵称经云同步到 Android 端会被 sanitize 丢弃
    get: () => settings.ltNickname
      || `NERIPC${userUuid.value.replace(/-/g, '').slice(0, 4).toUpperCase()}`,
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
  let _listenerPingTimer: ReturnType<typeof setInterval> | null = null
  let _softSyncTimer: ReturnType<typeof setInterval> | null = null
  let _softSyncRate: number | null = null
  let _watchReleaseTimer: ReturnType<typeof setTimeout> | null = null
  let _playbackApplySequence = 0
  const _pendingRemotePlaybackLoads = new Set<number>()
  let _trackSwitchAt = 0
  let _reconnectAttempt = 0
  let _reconnectTimer: ReturnType<typeof setTimeout> | null = null
  // 会话代际：leaveRoom 后递增，让在途的延迟回调失效，不再操作播放器
  let _sessionGeneration = 0
  // 出站事件排序字段：实例标识会话内生成一次，序号单调递增（对齐 Android EventFactory）
  const _clientInstanceId = crypto.randomUUID()
  let _clientSequence = 0
  // 已处理的转发请求 eventId -> 处理时间，防重复送达二次执行
  const _handledForwardedEventIds = new Map<string, number>()
  const _lastForwardedSequence = new Map<string, number>()
  let _controlRequestChain: Promise<void> = Promise.resolve()
  let _wsUrl: string | null = null
  // 后端为每条 WS 分配代际 ID，迟到的旧连接事件必须丢弃（MK-02）
  let _activeWsConnectionId: string | null = null
  let _unlistenMessage: UnlistenFn | null = null
  let _unlistenConnected: UnlistenFn | null = null
  let _unlistenDisconnected: UnlistenFn | null = null
  let _suppressPlayerWatch = false
  let _lastAppliedRoomVersion = 0
  // 回环抑制：避免自己发出的事件触发回环
  const _recentOutboundEventIds = new Set<string>()
  // 记录最后上报的 track id，避免重复上报
  let _lastReportedTrackId: string | null = null
  let _lastReportedQueueKeys: string[] = []
  let _queueEventInFlight: ListenTogetherEvent | null = null
  let _queueEventSnapshot: ListenTogetherEvent | null = null
  let _queuedQueueEvent: ListenTogetherEvent | null = null
  let _queueAckTimer: ReturnType<typeof setTimeout> | null = null
  let _lastReportedIsPlaying: boolean | null = null
  let _lastReportedRepeatMode: number | null = null
  let _lastReportedShuffle: boolean | null = null
  let _lastSentControlType: string | null = null
  let _lastSentControlAt = 0
  let _lastSentSeekPosition: number | null = null
  let _lastSentSeekAt = 0
  let _pendingSeekReport: { positionMs: number; trackId: string | null } | null = null
  let _pendingSeekTimer: ReturnType<typeof setTimeout> | null = null
  // 服务器时钟偏移估计（对齐 Android estimatedServerClockOffsetMs）：
  // 期望播放位置须用服务器时钟推算，直接用本机 Date.now() 会因两端时钟差恒定偏移
  let _serverClockOffsetMs = 0
  let _lastRequestedLinkStableKey: string | null = null
  let _lastRequestedLinkAt = 0
  let _joinSecret: string | null = null

  // 计算属性
  const isConnected = computed(() => connectionState.value === 'connected')
  const isController = computed(() => role.value === 'controller')
  const members = computed(() => roomState.value?.members ?? [])

  // 房间操作
  /** 创建房间 */
  async function createRoom() {
    const player = usePlayerStore()
    const toast = useToastStore()
    const t = (i18n.global as any).t
    const generation = _sessionGeneration

    try {
      sessionError.value = null
      if (!isValidLtNickname(nickname.value)) {
        throw new Error(t('listen_together.invalid_nickname'))
      }
      connectionState.value = 'connecting'

      // 构建初始快照
      const currentStreamUrl = player.currentTrack
        ? player.getCurrentStreamUrl(player.currentTrack.id) || undefined
        : undefined
      const { queue: ltQueue, resolvedIndex } = toShareableQueueSnapshot(
        player.queue,
        player.queueIndex,
        roomSettings.value.shareAudioLinks,
        currentStreamUrl,
      )
      const initialTrack = ltQueue[resolvedIndex]

      const snapshot: ListenTogetherInitialSnapshot = {
        queue: ltQueue,
        currentIndex: resolvedIndex,
        track: initialTrack,
        settings: roomSettings.value,
        isPlaying: !!initialTrack && player.isPlaying,
        positionMs: initialTrack ? player.positionMs : 0,
        // Align Android ListenTogetherInitialSnapshot (ExoPlayer ints)
        repeatMode: desktopRepeatToWire(player.repeatMode),
        shuffleEnabled: !!player.shuffleEnabled,
      }

      const resp = await invoke<ListenTogetherRoomResponse>('lt_create_room', {
        baseUrl: baseUrl.value,
        userUuid: userUuid.value,
        nickname: nickname.value,
        initialSnapshot: snapshot,
      })
      if (generation !== _sessionGeneration) return

      if (!resp.ok) {
        throw new Error(resp.error || 'Create room failed')
      }

      const createdRoomId = resp.roomId
      if (!createdRoomId) {
        throw new Error('Create room response did not include a room ID')
      }
      roomId.value = createdRoomId
      updateJoinSecret(resp.joinSecret)
      role.value = (resp.role as LtRole) || 'controller'
      if (resp.state) {
        roomState.value = resp.state
        _lastAppliedRoomVersion = resp.state.version || 0
        markSync('INITIAL_STATE', resp.state.updatedAt || Date.now())
      }

      // 连接 WebSocket
      const wsUrl = resolveWsUrl(resp, createdRoomId)
      await connectWs(wsUrl)
      if (generation !== _sessionGeneration) return

      startHeartbeat()
      setupPlayerWatch()

    } catch (e) {
      if (generation !== _sessionGeneration) return
      const msg = e instanceof Error ? e.message : String(e)
      sessionError.value = msg
      connectionState.value = 'disconnected'
      toast.error(t('listen_together.create_failed', { msg }))
    }
  }

  /** 加入房间 */
  async function joinRoom(targetRoomId: string, joinSecret?: string) {
    const player = usePlayerStore()
    const toast = useToastStore()
    const t = (i18n.global as any).t
    const generation = _sessionGeneration

    try {
      sessionError.value = null
      // 归一化并校验房间号（对齐 Android normalize+validate），非法直接报错不发请求
      const normalizedRoomId = normalizeLtRoomId(targetRoomId)
      if (!isValidLtRoomId(normalizedRoomId)) {
        throw new Error(t('listen_together.invalid_room_id'))
      }
      if (!isValidLtNickname(nickname.value)) {
        throw new Error(t('listen_together.invalid_nickname'))
      }
      connectionState.value = 'connecting'

      const resp = await invoke<ListenTogetherRoomResponse>('lt_join_room', {
        baseUrl: baseUrl.value,
        roomId: normalizedRoomId,
        userUuid: userUuid.value,
        nickname: nickname.value,
        joinSecret: normalizeLtJoinSecret(joinSecret),
      })
      if (generation !== _sessionGeneration) return

      if (!resp.ok) {
        throw new Error(resp.error || 'Join room failed')
      }

      roomId.value = normalizedRoomId
      updateJoinSecret(resp.joinSecret, joinSecret)
      role.value = (resp.role as LtRole) || 'listener'
      if (resp.state) {
        roomState.value = resp.state
        _lastAppliedRoomVersion = resp.state.version || 0
        roomSettings.value = resp.state.settings || roomSettings.value
        markSync('INITIAL_STATE', resp.state.updatedAt || Date.now())
        // 将服务端状态应用到本地播放器
        applyRoomStateToPlayer(resp.state, 'join')
      }

      const wsUrl = resolveWsUrl(resp, normalizedRoomId)
      await connectWs(wsUrl)
      if (generation !== _sessionGeneration) return

      startListenerPing()
      setupPlayerWatch()

    } catch (e) {
      if (generation !== _sessionGeneration) return
      const msg = e instanceof Error ? e.message : String(e)
      sessionError.value = msg
      connectionState.value = 'disconnected'
      toast.error(t('listen_together.join_failed', { msg }))
    }
  }

  /** 离开房间 */
  async function leaveRoom() {
    // 递增会话代际，让在途的延迟同步回调失效
    _sessionGeneration++
    _playbackApplySequence++
    _pendingRemotePlaybackLoads.clear()
    stopHeartbeat()
    stopListenerPing()
    teardownPlayerWatch()
    teardownListeners()
    setSyncRate(null)
    if (_watchReleaseTimer) clearTimeout(_watchReleaseTimer)
    _watchReleaseTimer = null
    _suppressPlayerWatch = false
    if (_reconnectTimer) {
      clearTimeout(_reconnectTimer)
      _reconnectTimer = null
    }

    try {
      const result = await invoke<ListenTogetherControlResponse | null>('lt_leave_room')
      if (result?.ok === false) log.warn('leave room rejected:', result.error)
    } catch (error) {
      log.warn('leave room request failed:', error)
    }
    try {
      await invoke('lt_disconnect_ws')
    } catch {}

    roomId.value = null
    _joinSecret = null
    role.value = null
    roomState.value = null
    _lastAppliedRoomVersion = 0
    sessionError.value = null
    connectionState.value = 'disconnected'
    lastSyncEventType.value = null
    lastSyncAt.value = null
    lastReconnectAt.value = null
    _wsUrl = null
    _activeWsConnectionId = null
    _reconnectAttempt = 0
    _lastReportedTrackId = null
    _lastReportedQueueKeys = []
    _queueEventInFlight = null
    _queueEventSnapshot = null
    _queuedQueueEvent = null
    if (_queueAckTimer) clearTimeout(_queueAckTimer)
    _queueAckTimer = null
    _lastReportedIsPlaying = null
    _lastReportedRepeatMode = null
    _lastReportedShuffle = null
    _lastSentControlType = null
    _lastSentControlAt = 0
    _lastSentSeekPosition = null
    _lastSentSeekAt = 0
    clearPendingSeekReport()
    _recentOutboundEventIds.clear()
    _handledForwardedEventIds.clear()
    _lastForwardedSequence.clear()
    _serverClockOffsetMs = 0
    _lastRequestedLinkStableKey = null
    _lastRequestedLinkAt = 0
  }

  // WebSocket 连接
  async function connectWs(wsUrl: string) {
    const generation = _sessionGeneration
    _wsUrl = wsUrl
    // 新连接建立期间不接受旧连接的收尾事件
    _activeWsConnectionId = null
    if (!await setupListeners(generation) || generation !== _sessionGeneration) return
    await invoke('lt_connect_ws', { wsUrl })
  }

  async function setupListeners(generation: number): Promise<boolean> {
    teardownListeners()

    const unlistenMessage = await listen<ListenTogetherSocketEnvelope>('lt:message', (event) => {
      if (generation !== _sessionGeneration) return
      if (!roomId.value || (event.payload.connectionId && event.payload.connectionId !== _activeWsConnectionId)) return
      handleSocketMessage(event.payload)
    })
    if (generation !== _sessionGeneration) { unlistenMessage(); return false }
    _unlistenMessage = unlistenMessage

    const unlistenConnected = await listen<{ connectionId?: string }>('lt:connected', (event) => {
      if (generation !== _sessionGeneration) return
      _activeWsConnectionId = event.payload.connectionId || null
      const reconnected = _reconnectAttempt > 0
      connectionState.value = 'connected'
      if (reconnected) {
        lastReconnectAt.value = Date.now()
        markSync('RECONNECTED', lastReconnectAt.value)
      }
      _reconnectAttempt = 0
      // 重连后恢复 listener 存活探测
      if (roomId.value) {
        startListenerPing()
        if (isController.value) startHeartbeat()
      }
    })
    if (generation !== _sessionGeneration) { unlistenConnected(); return false }
    _unlistenConnected = unlistenConnected

    const unlistenDisconnected = await listen<{
      connectionId?: string
      code: number
      reason: string
    }>('lt:disconnected', (event) => {
      if (generation !== _sessionGeneration) return
      // 新连接已经接管时，旧连接的 close/error 事件不能触发重连风暴。
      // 旧版后端没有 connectionId 时，仅在当前也没有代际信息时兼容接受。
      if (
        _activeWsConnectionId !== null
        && event.payload.connectionId !== _activeWsConnectionId
      ) return
      if (_activeWsConnectionId === null && connectionState.value !== 'connected') return
      const wasConnected = connectionState.value === 'connected'
      connectionState.value = 'disconnected'
      _activeWsConnectionId = null
      stopListenerPing()
      stopHeartbeat()
      setSyncRate(null)

      // 是否需要重连
      if (wasConnected && roomId.value) {
        scheduleReconnect()
      }
    })
    if (generation !== _sessionGeneration) { unlistenDisconnected(); return false }
    _unlistenDisconnected = unlistenDisconnected
    return true
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
    if (envelope.state && envelope.state.roomId !== roomId.value) return
    // 用服务端时间戳更新时钟偏移（每条带 nowMs/t 的消息都更新，对齐 Android）
    if (envelope.type !== 'np_pong') {
      const serverNow = envelope.nowMs
      if (typeof serverNow === 'number' && Number.isFinite(serverNow) && serverNow > 0) {
        _serverClockOffsetMs = serverNow - Date.now()
      }
    }
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
        {
          const generation = _sessionGeneration
          _controlRequestChain = _controlRequestChain.then(async () => {
            if (generation === _sessionGeneration) await handleMemberControlRequested(envelope, generation)
          }).catch((error) => log.warn('member control failed:', error))
        }
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
      case 'np_pong': {
        // np_ping 的 t 是客户端发送时间，使用往返中点估算服务器时钟，
        // 避免把网络延迟误判成固定进度漂移
        const sentAt = envelope.t
        const serverNow = envelope.nowMs
        if (typeof sentAt === 'number' && typeof serverNow === 'number' && Number.isFinite(serverNow)) {
          const receivedAt = Date.now()
          const rtt = receivedAt - sentAt
          if (serverNow > 0 && rtt >= 0 && rtt <= 30_000) {
            _serverClockOffsetMs = serverNow - (sentAt + rtt / 2)
          }
        }
        break
      }
      // 服务端控制应答：Android 用 control_result / ack，event_applied 仅作旧命名兼容
      case 'control_result':
      case 'ack':
      case 'event_applied':
        handleControlResult(envelope)
        break
      case 'error':
        handleErrorEnvelope(envelope)
        break
      default:
        log.debug('unknown message type:', envelope.type)
    }
  }

  /// 控制命令应答（对齐 Android SessionManager.websocket.controlResult）
  ///
  /// 两条链路都不能少：
  /// 1) 被拒绝（无权限、stale target、房间已关闭）时提示用户并回滚到服务端状态，
  ///    否则本地乐观改动会留下来，两端就此分叉；
  /// 2) 自己发起的 REQUEST_*/UPDATE_SETTINGS 被仲裁通过后，把 applied.state
  ///    作为权威状态应用到本地播放器（听众侧），否则位置/状态偏差要等下一次
  ///    room_state_updated 才能纠正。
  function handleControlResult(envelope: ListenTogetherSocketEnvelope) {
    const result = envelope.result
    if (!result) return
    const t = (i18n.global as any).t

    const applied = result.applied
    const appliedCause = applied?.causedBy
    const appliedType = appliedCause?.type
    const rejected = result.ok === false || !!result.error || envelope.ok === false

    if (rejected) {
      const err = result.error || envelope.message || t('listen_together.control_rejected')
      if (retryLegacyQueueSnapshot(err)) return
      if (_queueEventInFlight && (!appliedCause?.eventId || appliedCause.eventId === _queueEventInFlight.eventId)) {
        completeQueueEvent(_queueEventInFlight.eventId, false)
      }
      log.warn('control rejected by server:', err)
      useToastStore().error(err)
      // 以服务端状态为准重新对齐，优先用 applied.state，回退 envelope.state
      const rollback = applied?.state || envelope.state
      if (rollback) {
        commitRoomState(rollback, 'EVENT_REJECTED', applied?.expectedPositionMs ?? envelope.expectedPositionMs)
      }
      return
    }

    // 仲裁通过：仅当是本端发起的请求且带权威 state 才落地
    const queueAcknowledged = !!appliedCause?.eventId && appliedCause.eventId === _queueEventInFlight?.eventId
    if (
      applied?.state
      && (queueAcknowledged || (appliedCause?.userUuid === userUuid.value
        && (appliedType === 'UPDATE_SETTINGS' || appliedType?.startsWith('REQUEST_'))))
    ) {
      commitRoomState(applied.state, appliedType || 'CONTROL_APPLIED', applied.expectedPositionMs, !isController.value && !_queuedQueueEvent)
    }
    if (applied?.state && appliedCause?.eventId && applied.state.roomId === roomId.value
      && applied.state.version >= _lastAppliedRoomVersion) completeQueueEvent(appliedCause.eventId)
  }

  function handleErrorEnvelope(envelope: ListenTogetherSocketEnvelope) {
    const t = (i18n.global as any).t
    const err = envelope.result?.error || envelope.message || t('listen_together.control_rejected')
    if (retryLegacyQueueSnapshot(err)) return
    if (_queueEventInFlight) completeQueueEvent(_queueEventInFlight.eventId, false)
    log.warn('server error envelope:', err)
    useToastStore().error(err)
  }

  function handleWelcome(envelope: ListenTogetherSocketEnvelope) {
    if (envelope.role) {
      role.value = envelope.role as LtRole
    }
    if (envelope.state) {
      commitRoomState(envelope.state, 'WELCOME', envelope.expectedPositionMs)
    }
    if (isController.value) startHeartbeat()
    else stopHeartbeat()
  }

  function commitRoomState(state: ListenTogetherRoomState, causeType: string, expectedPositionMs?: number, apply = true) {
    const accepted = acceptRoomState(roomState.value, state, roomId.value, _lastAppliedRoomVersion)
    if (!accepted) return
    roomState.value = accepted
    _lastAppliedRoomVersion = accepted.version
    roomSettings.value = accepted.settings
    markSync(causeType, accepted.updatedAt || Date.now())
    if (apply) applyRoomStateToPlayer(accepted, causeType, expectedPositionMs)
  }

  function handleRoomStateUpdated(envelope: ListenTogetherSocketEnvelope) {
    if (!envelope.state) return
    if ((envelope.state.version || 0) < _lastAppliedRoomVersion) return
    if (_queuedQueueEvent && envelope.causedBy?.eventId === _queueEventInFlight?.eventId) {
      commitRoomState(envelope.state, envelope.causedBy?.type || 'STATE_SYNC', envelope.expectedPositionMs, false)
      completeQueueEvent(envelope.causedBy?.eventId)
      return
    }

    // 回声抑制：REQUEST_*/TRACK_FINISHED 引发的房态由权威方仲裁, 即便携带本端 eventId
    // 也必须应用到播放器（对齐 Android shouldIgnoreListenTogetherIncomingState:
    // REQUEST_ 前缀与 TRACK_FINISHED 永不忽略）, 否则服务端对位置/状态的钳制修正会被
    // 本地乐观值覆盖, 要等下一次心跳才纠正
    const causeType = envelope.causedBy?.type
    const authoritativeQueueChanged = !_queuedQueueEvent && (causeType === 'SET_QUEUE' || causeType === 'REQUEST_SET_QUEUE')
      && envelope.state.version > _lastAppliedRoomVersion
      && (envelope.state.currentIndex !== roomState.value?.currentIndex
        || envelope.state.queue.map(track => track.stableKey).join('\n') !== roomState.value?.queue.map(track => track.stableKey).join('\n'))
    const causeNeverSuppressed = !!causeType
      && (causeType.startsWith('REQUEST_') || causeType === 'TRACK_FINISHED')
    if (
      !causeNeverSuppressed && !authoritativeQueueChanged
      && envelope.causedBy?.eventId
      && _recentOutboundEventIds.has(envelope.causedBy.eventId)
    ) {
      _recentOutboundEventIds.delete(envelope.causedBy.eventId)
      // 仍然更新 roomState 但不应用到 player
      commitRoomState(envelope.state, causeType || 'STATE_SYNC', envelope.expectedPositionMs, false)
      completeQueueEvent(envelope.causedBy?.eventId)
      return
    }
    // 已应用的本端 eventId 从抑制集移除, 避免累积
    if (envelope.causedBy?.eventId) {
      _recentOutboundEventIds.delete(envelope.causedBy.eventId)
    }

    commitRoomState(envelope.state, causeType || 'state_update', envelope.expectedPositionMs)
    completeQueueEvent(envelope.causedBy?.eventId)
  }

  function handleLinkRequested(envelope: ListenTogetherSocketEnvelope) {
    // 房主收到链接请求：下发当前已解析的音频 URL
    if (!isController.value || !roomSettings.value.shareAudioLinks) return

    // 检查 requestTrackStableKey 是否与当前曲目匹配
    const player = usePlayerStore()
    if (!player.currentTrack) return

    const currentLt = trackInfoToLtTrack(player.currentTrack)
    if (envelope.requestTrackStableKey && envelope.requestTrackStableKey !== currentLt.stableKey) {
      return // 请求的曲目已不是当前播放的
    }

    const streamUrl = player.getCurrentStreamUrl(player.currentTrack.id)
    const { queue, resolvedIndex } = toShareableQueueSnapshot(
      player.queue,
      player.queueIndex,
      true,
      streamUrl || undefined,
      false,
      player.getCurrentStreamUrls(player.currentTrack.id),
    )
    const track = queue[resolvedIndex]
    if (!track || track.stableKey !== currentLt.stableKey || !track.streamUrl) {
      log.debug('link requested but current stream is unavailable:', currentLt.stableKey)
      return
    }

    // 只把与请求 stableKey 对应的当前直链发回，避免异步解析结果串到另一首歌
    sendEvent({
      type: 'LINK_READY',
      track,
      queue,
      currentIndex: resolvedIndex,
      positionMs: player.positionMs,
      state: player.isPlaying ? 'playing' : 'paused',
      requestTrackStableKey: track.stableKey,
    })
  }

  function requestLinkForTrack(track: import('./protocol').ListenTogetherTrack, currentIndex: number) {
    if (isController.value || !roomSettings.value.shareAudioLinks || currentIndex < 0) return
    const now = Date.now()
    if (
      _lastRequestedLinkStableKey === track.stableKey
      && now - _lastRequestedLinkAt < LINK_REQUEST_THROTTLE_MS
    ) return
    _lastRequestedLinkStableKey = track.stableKey
    _lastRequestedLinkAt = now
    const player = usePlayerStore()
    sendEvent({
      type: 'REQUEST_LINK',
      track,
      currentIndex,
      positionMs: player.positionMs,
      requestTrackStableKey: track.stableKey,
    })
  }

  async function handleMemberControlRequested(envelope: ListenTogetherSocketEnvelope, generation: number) {
    // 房主处理听众的控制请求
    if (!isController.value) return

    // 鉴权：关闭成员控制时，只放行房主自己发出的请求
    // （对齐 Android ListenTogetherControlBlockPolicy，防魔改客户端越权控制）
    if (!roomSettings.value.allowMemberControl && envelope.causedBy?.userUuid !== userUuid.value) {
      log.warn('member control blocked: allowMemberControl=false, requester=', envelope.causedBy?.userUuid)
      return
    }

    // 按 causedBy.eventId 去重：重复送达的转发请求不得二次执行
    const causeEventId = envelope.causedBy?.eventId
    if (envelope.requestSequence != null && envelope.requestSequence > 0) {
      const requester = envelope.causedBy?.userUuid || '__global__'
      if (envelope.requestSequence <= (_lastForwardedSequence.get(requester) ?? 0)) return
      _lastForwardedSequence.delete(requester)
      _lastForwardedSequence.set(requester, envelope.requestSequence)
      if (_lastForwardedSequence.size > HANDLED_FORWARDED_REQUESTER_LIMIT) {
        _lastForwardedSequence.delete(_lastForwardedSequence.keys().next().value!)
      }
    } else if (causeEventId) {
      if (_handledForwardedEventIds.has(causeEventId)) return
      _handledForwardedEventIds.set(causeEventId, Date.now())
      while (_handledForwardedEventIds.size > HANDLED_FORWARDED_EVENT_LIMIT) {
        const oldest = _handledForwardedEventIds.keys().next().value
        if (oldest === undefined) break
        _handledForwardedEventIds.delete(oldest)
      }
    }

    const player = usePlayerStore()
    const causeType = envelope.causedBy?.type
    const currentKey = player.currentTrack ? trackInfoToLtTrack(player.currentTrack).stableKey : null
    if (causeType && ['REQUEST_PLAY', 'REQUEST_PAUSE', 'REQUEST_SEEK'].includes(causeType)
      && (!currentKey || (envelope.requestTrackStableKey ?? envelope.track?.stableKey) !== currentKey)) {
      reportHeartbeat()
      return
    }

    _suppressPlayerWatch = true
    try {
      switch (causeType) {
        case 'REQUEST_PLAY':
          if (!player.isPlaying) await player.resume('remote_sync')
          if (generation !== _sessionGeneration) return
          reportPlayEvent()
          break
        case 'REQUEST_PAUSE':
          if (player.isPlaying) await player.pause('remote_sync')
          if (generation !== _sessionGeneration) return
          reportPauseEvent()
          break
        case 'REQUEST_SEEK':
          if (envelope.positionMs != null) {
            await player.seekTo(envelope.positionMs, 'remote_sync')
            if (generation !== _sessionGeneration) return
            reportSeekEvent(envelope.positionMs)
          }
          break
        case 'REQUEST_SET_TRACK':
          if (envelope.track) {
            const queue = resolveRequestedQueue(envelope)
            if (!queue) { reportHeartbeat(); break }
            const index = 'targetCurrentIndex' in queue && queue.targetCurrentIndex != null && queue.targetCurrentIndex >= 0
              ? queue.targetCurrentIndex : envelope.currentIndex ?? queue.currentIndex
            if (!queue.queue[index] || queue.queue[index].stableKey !== envelope.track.stableKey) {
              reportHeartbeat()
              break
            }
            replacePlayerQueue(queue.queue, index)
            await player.play(player.queue[index], 'remote_sync', envelope.positionMs ?? 0)
            if (generation !== _sessionGeneration) return
            if (envelope.shouldPlay === false) await player.pause('remote_sync')
            if (generation !== _sessionGeneration) return
            reportSetTrackEvent(envelope.track, index)
          }
          break
        case 'REQUEST_SET_QUEUE': {
          const queue = resolveRequestedQueue(envelope)
          if (!queue) { reportHeartbeat(); break }
          replacePlayerQueue(queue.queue, queue.currentIndex)
          const track = player.queue[queue.currentIndex]
          const requestedPosition = envelope.positionMs ?? envelope.expectedPositionMs ?? 0
          const shouldPlay = envelope.stateName === 'playing' || envelope.shouldPlay === true
            ? true : envelope.stateName === 'paused' || envelope.shouldPlay === false ? false : player.isPlaying
          if (track && (!player.currentTrack || trackInfoToLtTrack(player.currentTrack).stableKey !== queue.queue[queue.currentIndex].stableKey)) {
            await player.play(track, 'remote_sync', requestedPosition)
            if (generation !== _sessionGeneration) return
          }
          if (!track || !shouldPlay) await player.pause('remote_sync')
          else if (!player.isPlaying) await player.resume('remote_sync')
          if (generation !== _sessionGeneration) return
          if (track && envelope.positionMs != null && Math.abs(player.positionMs - requestedPosition) > 800) {
            await player.seekTo(requestedPosition, 'remote_sync')
            if (generation !== _sessionGeneration) return
          }
          reportQueueEvent()
          break
        }
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
      if (generation === _sessionGeneration) releasePlayerWatch(350)
    }
  }

  function handleRoomSuspended(envelope: ListenTogetherSocketEnvelope) {
    const toast = useToastStore()
    const t = (i18n.global as any).t
    markSync('ROOM_SUSPENDED')
    if (envelope.state) commitRoomState(envelope.state, 'ROOM_SUSPENDED', envelope.expectedPositionMs)
    else void usePlayerStore().pause('remote_sync')
    setSyncRate(null)
    toast.error(t('listen_together.controller_offline'))
  }

  function handleRoomResumed(envelope: ListenTogetherSocketEnvelope) {
    markSync('ROOM_RESUMED')
    if (envelope.state) commitRoomState(envelope.state, 'ROOM_RESUMED', envelope.expectedPositionMs)
  }

  function handleRoomClosed(_envelope: ListenTogetherSocketEnvelope) {
    const toast = useToastStore()
    const t = (i18n.global as any).t
    toast.error(t('listen_together.room_closed'))
    leaveRoom()
  }

  function releasePlayerWatch(delay: number) {
    if (_watchReleaseTimer) clearTimeout(_watchReleaseTimer)
    const generation = _sessionGeneration
    _watchReleaseTimer = setTimeout(() => {
      _watchReleaseTimer = null
      if (generation === _sessionGeneration) _suppressPlayerWatch = false
    }, delay)
  }

  function playbackIdentity(): string | null {
    const player = usePlayerStore()
    if (!player.currentTrack) return null
    const key = trackInfoToLtTrack(player.currentTrack).stableKey
    const references = queueReferences(player.queue.map(track => trackInfoToLtTrack(track)))
    const reference = references[player.queueIndex]
    return JSON.stringify([key, reference?.stableKey === key ? reference.occurrence : 0])
  }

  function replacePlayerQueue(queue: import('./protocol').ListenTogetherTrack[], currentIndex: number) {
    const player = usePlayerStore()
    const references = queueReferences(queue)
    const tracks = queue.map((track, index) => ({
      ...ltTrackToTrackInfo(track),
      playlistKey: JSON.stringify(references[index]),
    }))
    player.queue.splice(0, player.queue.length, ...tracks)
    player.queueIndex = currentIndex >= 0 && currentIndex < tracks.length ? currentIndex : -1
    _lastReportedQueueKeys = queue.map(track => track.stableKey)
  }

  function resolveRequestedQueue(envelope: ListenTogetherSocketEnvelope) {
    if (envelope.queueMutation && roomState.value) {
      return applyListenTogetherQueueMutation({
        roomQueue: roomState.value.queue,
        roomCurrentIndex: roomState.value.currentIndex,
        roomVersion: roomState.value.version,
        mutation: envelope.queueMutation,
        targetCurrentStableKey: envelope.requestTrackStableKey,
      })
    }
    const queue = envelope.queue ?? roomState.value?.queue
    if (!queue) return null
    return { queue, currentIndex: envelope.currentIndex ?? roomState.value?.currentIndex ?? -1 }
  }

  function setSyncRate(rate: number | null) {
    if (_softSyncRate !== rate) usePlayerStore().setListenTogetherSyncPlaybackRate(rate)
    _softSyncRate = rate
    if (rate === null) {
      if (_softSyncTimer) clearInterval(_softSyncTimer)
      _softSyncTimer = null
      return
    }
    if (_softSyncTimer) return
    _softSyncTimer = setInterval(() => {
      const player = usePlayerStore()
      const state = roomState.value
      const expectedPositionMs = state ? resolveExpectedPosition(state, Date.now() + _serverClockOffsetMs) : 0
      const currentKey = player.currentTrack ? trackInfoToLtTrack(player.currentTrack).stableKey : null
      const roomKey = state?.track?.stableKey ?? state?.queue[state.currentIndex]?.stableKey
      const action = resolveSoftSyncRecheckAction({
        currentRate: _softSyncRate ?? 1,
        sessionConnected: connectionState.value === 'connected',
        isController: isController.value,
        desiredPlaying: state?.playback.state === 'playing',
        localPlaying: player.isPlaying,
        currentTrackMatchesRoom: !!currentKey && currentKey === roomKey,
        expectedPositionMs,
        localPositionMs: player.positionMs,
      })
      if (action === 'reset_rate' || action === 'none') setSyncRate(null)
      else if (action === 'apply_state' && state) {
        setSyncRate(null)
        applyRoomStateToPlayer(state, 'SOFT_SYNC_RECHECK', expectedPositionMs)
      } else if (state) {
        setSyncRate(resolvePositionSync({ expectedPositionMs, localPositionMs: player.positionMs,
          desiredPlaying: true, isController: false, causeType: 'SOFT_SYNC_RECHECK' }).rate)
      }
    }, SOFT_SYNC_RECHECK_INTERVAL_MS)
  }

  // 播放器同步
  function applyRoomStateToPlayer(state: ListenTogetherRoomState, causeType: string, expectedPositionMs?: number) {
    if (state.roomId !== roomId.value || state.version < _lastAppliedRoomVersion) return
    const player = usePlayerStore()
    const queue = [...state.queue]
    const index = state.currentIndex
    if (state.track && index >= 0 && index < queue.length && queue[index].stableKey === state.track.stableKey) {
      queue[index] = state.track
    }
    if (queue.length === 0 && state.track && index >= 0) queue.push(state.track)
    const targetIndex = queue.length === 1 && state.queue.length === 0 ? 0 : index
    const effectiveLtTrack = queue[targetIndex]
    _suppressPlayerWatch = true
    try {
      if (!effectiveLtTrack) {
        replacePlayerQueue(queue, -1)
        setSyncRate(null)
        if (player.isPlaying) void player.pause('remote_sync')
        return
      }
      const references = queueReferences(queue)
      const targetIdentity = JSON.stringify([effectiveLtTrack.stableKey, references[targetIndex].occurrence])
      const previousIdentity = playbackIdentity()
      const previousIndex = player.queueIndex
      const previousKey = player.currentTrack ? trackInfoToLtTrack(player.currentTrack).stableKey : null
      const queueUpdate = causeType === 'SET_QUEUE' || causeType === 'REQUEST_SET_QUEUE'
      const playbackContextChanged = previousKey !== effectiveLtTrack.stableKey
        || (!queueUpdate && previousIdentity !== targetIdentity)
      const remoteIsPlaying = state.playback.state === 'playing'
      const expectedPos = resolveExpectedPosition(state, Date.now() + _serverClockOffsetMs, expectedPositionMs)
      const remoteTrack = ltTrackToTrackInfo(effectiveLtTrack)
      if (!isController.value && roomSettings.value.shareAudioLinks && !remoteTrack.audioUrl) {
        requestLinkForTrack(effectiveLtTrack, targetIndex)
      }
      const streamChanged = !!remoteTrack.audioUrl
        && !trustedInboundStreamUrls(effectiveLtTrack.channelId, effectiveLtTrack.streamUrls, effectiveLtTrack.streamUrl)
          .includes(player.getCurrentStreamUrl(remoteTrack.id) || '')
      replacePlayerQueue(queue, targetIndex)
      if (playbackContextChanged || streamChanged) {
        _trackSwitchAt = Date.now()
        setSyncRate(null)
        const generation = _sessionGeneration
        const applySequence = ++_playbackApplySequence
        _pendingRemotePlaybackLoads.add(applySequence)
        // 播放完成后读取最新房态，避免固定延时在慢速解析中暂停错误的会话
        void player.play(player.queue[targetIndex], 'remote_sync', expectedPos).then(async () => {
          if (generation !== _sessionGeneration || applySequence !== _playbackApplySequence) return
          const latest = roomState.value
          const latestKey = latest?.track?.stableKey ?? latest?.queue[latest.currentIndex]?.stableKey
          if (!latest || latestKey !== effectiveLtTrack.stableKey) return
          const latestPosition = resolveExpectedPosition(latest, Date.now() + _serverClockOffsetMs)
          if (Math.abs(latestPosition - player.positionMs) > 500) {
            await player.seekTo(latestPosition, 'remote_sync')
            if (generation !== _sessionGeneration || applySequence !== _playbackApplySequence) return
          }
          if (latest.playback.state !== 'playing') await player.pause('remote_sync')
          else if (!player.isPlaying) await player.resume('remote_sync')
        }).catch((error) => log.warn('remote playback sync failed:', error)).finally(() => {
          _pendingRemotePlaybackLoads.delete(applySequence)
          if (generation === _sessionGeneration) releasePlayerWatch(500)
        })
      } else if (!player.isLoadingAudio) {
        if (player.isPlaying !== remoteIsPlaying) {
          if (remoteIsPlaying) void player.resume('remote_sync')
          else void player.pause('remote_sync')
        }
        const sync = resolvePositionSync({
          expectedPositionMs: expectedPos,
          localPositionMs: player.positionMs,
          desiredPlaying: remoteIsPlaying,
          isController: isController.value,
          causeType,
          targetIndexChanged: !queueUpdate && previousIndex !== targetIndex,
          trackSwitchGracePeriodActive: Date.now() - _trackSwitchAt < 800,
        })
        setSyncRate(sync.rate)
        if (sync.seekTo !== undefined) void player.seekTo(sync.seekTo, 'remote_sync')
      }
      _lastReportedTrackId = targetIdentity
      _lastReportedIsPlaying = remoteIsPlaying
      player.applyListenTogetherPlaybackMode({ repeatMode: state.playback.repeatMode, shuffleEnabled: state.playback.shuffleEnabled })
      _lastReportedRepeatMode = state.playback.repeatMode ?? desktopRepeatToWire(player.repeatMode)
      _lastReportedShuffle = state.playback.shuffleEnabled ?? !!player.shuffleEnabled
    } finally {
      releasePlayerWatch(500)
    }
  }

  // 本地变化上报
  let _playerWatchStop: (() => void) | null = null
  let _seekWatchStop: (() => void) | null = null

  function setupPlayerWatch() {
    teardownPlayerWatch()
    const player = usePlayerStore()
    _lastReportedTrackId = playbackIdentity()
    _lastReportedQueueKeys = player.queue.map(track => trackInfoToLtTrack(track).stableKey)
    _lastReportedIsPlaying = player.isPlaying
    _lastReportedRepeatMode = desktopRepeatToWire(player.repeatMode)
    _lastReportedShuffle = !!player.shuffleEnabled

    _playerWatchStop = watch(
      () => ({
        trackId: playbackIdentity(),
        queueKeys: player.queue.map(track => trackInfoToLtTrack(track).stableKey),
        isPlaying: player.isPlaying,
        repeatMode: player.repeatMode,
        shuffleEnabled: player.shuffleEnabled,
      }),
      (newVal) => {
        if (_suppressPlayerWatch || _pendingRemotePlaybackLoads.size > 0 || player.isRemoteSyncGuardActive() || connectionState.value !== 'connected') return

        // 曲目变化
        let trackReported = false
        if (newVal.trackId && newVal.trackId !== _lastReportedTrackId) {
          _lastReportedTrackId = newVal.trackId
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
                shouldPlay: player.isPlaying,
              })
              trackReported = true
            }
          }
        }

        if (trackReported) _lastReportedQueueKeys = newVal.queueKeys
        else if (newVal.queueKeys.length !== _lastReportedQueueKeys.length
          || newVal.queueKeys.some((key, index) => key !== _lastReportedQueueKeys[index])) {
          reportQueueEvent()
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
        if (_suppressPlayerWatch || _pendingRemotePlaybackLoads.size > 0 || connectionState.value !== 'connected') return
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
    if (!roomId.value) return
    stripUnsharedAudioLinks(event)
    const generation = _sessionGeneration
    if (!event.eventId) event.eventId = generateEventId()
    if (!event.clientTimeMs) event.clientTimeMs = Date.now()
    // 乱序保护字段（协议已声明、Android 每事件必带）：实例标识 + 单调递增序号
    if (!event.clientInstanceId) event.clientInstanceId = _clientInstanceId
    if (event.clientSequence == null) event.clientSequence = ++_clientSequence

    _recentOutboundEventIds.add(event.eventId!)
    // 清理过期 ID（保留最近 50 个）
    if (_recentOutboundEventIds.size > 50) {
      const iter = _recentOutboundEventIds.values()
      _recentOutboundEventIds.delete(iter.next().value!)
    }

    let delivered = false
    try {
      delivered = await invoke<boolean>('lt_send_event', { event })
    } catch (e) {
      log.warn('WebSocket event send failed:', e)
    }
    if (delivered || generation !== _sessionGeneration) return
    await sendHttpFallback(event, generation)
  }

  async function sendHttpFallback(event: ListenTogetherEvent, generation: number) {
    if (generation !== _sessionGeneration) return
    stripUnsharedAudioLinks(event)
    if (_queueEventInFlight?.eventId === event.eventId && _queueAckTimer) {
      clearTimeout(_queueAckTimer)
      _queueAckTimer = null
    }
    try {
      const result = await invoke<ListenTogetherControlResponse>('lt_send_control', { event })
      if (generation !== _sessionGeneration) return
      const queueEvent = _queueEventInFlight?.eventId === event.eventId
      handleControlResult({ type: 'control_result', result })
      if (result.ok && result.applied?.state) {
        if (!queueEvent || _queueEventInFlight?.eventId === event.eventId) {
          commitRoomState(result.applied.state, result.applied.causedBy?.type || event.type,
            result.applied.expectedPositionMs, !queueEvent && !_queuedQueueEvent)
          if (result.applied.state.roomId === roomId.value && result.applied.state.version >= _lastAppliedRoomVersion) {
            completeQueueEvent(event.eventId)
          }
        }
      } else if (!result.ok) {
        completeQueueEvent(event.eventId, false)
      }
    } catch (error) {
      if (generation !== _sessionGeneration) return
      log.warn('HTTP event send failed:', error)
      const t = (i18n.global as any).t
      useToastStore().error(t('listen_together.control_not_sent'))
      completeQueueEvent(event.eventId, false)
    }
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
      ...buildControlSnapshotFields(),
      positionMs: player.positionMs,
      state: 'playing',
    })
  }

  function reportPauseEvent() {
    const player = usePlayerStore()
    if (shouldSkipControlEvent('PAUSE')) return
    sendEvent({
      type: 'PAUSE',
      ...buildControlSnapshotFields(),
      positionMs: player.positionMs,
      state: 'paused',
    })
  }

  function reportSeekEvent(positionMs: number) {
    if (shouldSkipSeekEvent(positionMs)) return
    sendEvent({
      type: 'SEEK',
      ...buildControlSnapshotFields(),
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

  /// 构建控制事件的曲目绑定快照（对齐 Android playbackSnapshotEvent）
  ///
  /// currentIndex 必须用过滤后共享队列的 resolvedIndex，不能用原始 player.queueIndex，
  /// 否则队列含本地曲目时索引错位；track 取共享队列中已解析的当前项。
  function buildControlSnapshotFields() {
    const player = usePlayerStore()
    const { queue: ltQueue, resolvedIndex } = toShareableQueueSnapshot(
      player.queue,
      player.queueIndex,
      roomSettings.value.shareAudioLinks,
      player.getCurrentStreamUrl() || undefined,
      false,
      player.getCurrentStreamUrls(),
    )
    const track = ltQueue[resolvedIndex]
    return {
      queue: ltQueue,
      currentIndex: resolvedIndex,
      track,
      requestTrackStableKey: track?.stableKey,
    }
  }

  function reportSetTrackEvent(_track: any, _currentIndex: number) {
    const player = usePlayerStore()
    const snap = buildControlSnapshotFields()
    if (!snap.track) return
    sendQueueEvent({
      type: 'SET_TRACK',
      track: snap.track,
      currentIndex: snap.currentIndex,
      queue: snap.queue,
      positionMs: player.positionMs,
      shouldPlay: player.isPlaying,
    })
  }

  function queueEventFields(queue: import('./protocol').ListenTogetherTrack[], index: number) {
    const state = roomState.value
    if (state && state.schemaVersion >= 2) {
      const plan = buildListenTogetherQueueMutationPlan(state, queue, index)
      if (!plan.requiresSnapshotFallback) return { queueMutation: plan.mutation }
    }
    return { queue }
  }

  function sendQueueEvent(snapshot: ListenTogetherEvent, forceSnapshot = false) {
    if (_queueEventInFlight) {
      _queuedQueueEvent = snapshot
      return
    }
    if (!forceSnapshot && roomState.value && snapshot.queue
      && (snapshot.type === 'SET_TRACK' || snapshot.type === 'REQUEST_SET_TRACK')) {
      const plan = buildListenTogetherQueueMutationPlan(roomState.value, snapshot.queue, snapshot.currentIndex ?? -1)
      if (plan.needsCurrentSelectionAfterApply && roomState.value.schemaVersion >= 2) {
        // 新重复项确认入队后才有可引用的 occurrence，先提交队列再选择该项
        _queuedQueueEvent = snapshot
        snapshot = { ...snapshot, type: snapshot.type === 'SET_TRACK' ? 'SET_QUEUE' : 'REQUEST_SET_QUEUE' }
      }
    }
    const event: ListenTogetherEvent = {
      ...snapshot,
      queue: undefined,
      ...(forceSnapshot ? { queue: snapshot.queue } : queueEventFields(snapshot.queue ?? [], snapshot.currentIndex ?? -1)),
      eventId: generateEventId(),
    }
    _queueEventInFlight = event
    _queueEventSnapshot = snapshot
    const generation = _sessionGeneration
    _queueAckTimer = setTimeout(() => {
      _queueAckTimer = null
      if (generation === _sessionGeneration && _queueEventInFlight?.eventId === event.eventId) {
        void sendHttpFallback(event, generation)
      }
    }, 3000)
    void sendEvent(event)
  }

  function completeQueueEvent(eventId?: string, accepted = true) {
    if (!eventId || _queueEventInFlight?.eventId !== eventId) return
    if (_queueAckTimer) clearTimeout(_queueAckTimer)
    _queueAckTimer = null
    _queueEventInFlight = null
    const previousSnapshot = _queueEventSnapshot
    _queueEventSnapshot = null
    if (accepted && roomState.value) refreshLocalQueueReferences(previousSnapshot, roomState.value)
    const queued = _queuedQueueEvent
    _queuedQueueEvent = null
    if (accepted && queued && roomState.value) {
      const references = queueReferences(roomState.value.queue)
      const queue = queued.queue?.map(track => {
        const clone = { ...track }
        const reference = getLtQueueReference(track)
        const previousIndex = reference ? previousSnapshot?.queue?.findIndex(previous => {
          const previousReference = getLtQueueReference(previous)
          return previousReference?.stableKey === reference.stableKey && previousReference.occurrence === reference.occurrence
        }) ?? -1 : -1
        if (previousIndex >= 0 && references[previousIndex]?.stableKey === track.stableKey) {
          setLtQueueReference(clone, references[previousIndex])
        }
        return clone
      })
      sendQueueEvent({ ...queued, queue, track: queue?.[queued.currentIndex ?? -1] ?? queued.track })
    }
  }

  function refreshLocalQueueReferences(snapshot: ListenTogetherEvent | null, state: ListenTogetherRoomState) {
    const player = usePlayerStore()
    const references = queueReferences(state.queue)
    const shareable = player.queue.map(track => ({ track, wire: trackInfoToLtTrack(track) }))
      .filter(({ wire }) => wire.channelId !== 'local' && wire.channelId !== 'qqMusic')
    const sameSequence = shareable.length === state.queue.length
      && shareable.every(({ wire }, index) => wire.stableKey === state.queue[index].stableKey)
    for (const [index, { track, wire }] of shareable.entries()) {
      const oldReference = getLtQueueReference(wire)
      const committedIndex = sameSequence ? index : oldReference ? snapshot?.queue?.findIndex(previous => {
        const reference = getLtQueueReference(previous)
        return reference?.stableKey === oldReference.stableKey && reference.occurrence === oldReference.occurrence
      }) ?? -1 : -1
      if (committedIndex >= 0 && references[committedIndex]?.stableKey === wire.stableKey) {
        track.playlistKey = JSON.stringify(references[committedIndex])
      }
    }
  }

  function stripUnsharedAudioLinks(event: ListenTogetherEvent) {
    if (roomSettings.value.shareAudioLinks) return
    const strip = (track: import('./protocol').ListenTogetherTrack) => ({ ...track, streamUrl: undefined, streamUrls: [] })
    if (event.track) event.track = strip(event.track)
    if (event.queue) event.queue = event.queue.map(strip)
    if (event.queueMutation) {
      event.queueMutation = { ...event.queueMutation, operations: event.queueMutation.operations.map(operation =>
        operation.track ? { ...operation, track: strip(operation.track) } : operation) }
    }
  }

  function retryLegacyQueueSnapshot(error: string): boolean {
    const normalized = error.trim().toLowerCase()
    const compatibilityError = ['queue mutation is invalid', 'queue mutation base version is ahead',
      'queue mutation event type unsupported', 'queue update queue required'].some(message => normalized.includes(message))
    if (!compatibilityError || !_queueEventInFlight?.queueMutation || !_queueEventSnapshot) return false
    const snapshot = _queueEventSnapshot
    if (_queueAckTimer) clearTimeout(_queueAckTimer)
    _queueAckTimer = null
    _queueEventInFlight = null
    _queueEventSnapshot = null
    sendQueueEvent(snapshot, true)
    return true
  }

  function reportQueueEvent() {
    const player = usePlayerStore()
    const snapshot = buildControlSnapshotFields()
    _lastReportedQueueKeys = player.queue.map(track => trackInfoToLtTrack(track).stableKey)
    sendQueueEvent({
      type: isController.value ? 'SET_QUEUE' : 'REQUEST_SET_QUEUE',
      queue: snapshot.queue,
      track: snapshot.track,
      currentIndex: snapshot.currentIndex,
      requestTrackStableKey: snapshot.requestTrackStableKey,
      positionMs: player.positionMs,
      shouldPlay: player.isPlaying,
      state: player.isPlaying ? 'playing' : 'paused',
      repeatMode: desktopRepeatToWire(player.repeatMode),
      shuffleEnabled: !!player.shuffleEnabled,
    })
  }

  /// 曲目自然播完: 上报 TRACK_FINISHED 交权威方仲裁切歌, 本地不自行推进
  /// （对齐 Android ListenTogetherEventFactory.buildTrackFinishedEvent:
  /// listener 的 TRACK_FINISHED 不带 currentIndex/track/queue）
  function reportTrackFinished(finishedTrackId: string | null) {
    const player = usePlayerStore()
    const finishedLtTrack = player.currentTrack ? trackInfoToLtTrack(player.currentTrack) : null
    if (!finishedLtTrack || finishedLtTrack.channelId === 'local' || finishedLtTrack.channelId === 'qqMusic') return
    const finishedKey = finishedTrackId
      ? finishedLtTrack.stableKey
      : undefined
    sendEvent({
      type: 'TRACK_FINISHED',
      finishedTrackStableKey: finishedKey,
    })
  }

  /// 发送成员控制请求
  ///
  /// Android 控制端对 REQUEST_PLAY/PAUSE/SEEK 走 trackBoundRequestControlEventTypes：
  /// requestedStableKey 为空即直接拒绝并只回心跳。因此每个请求都必须携带曲目绑定
  /// （track/currentIndex/queue/requestTrackStableKey），对齐 playbackSnapshotEvent，
  /// 否则桌面对 Android 房主的控制会被静默丢弃（用户实测"点了没反应"根因）。
  function sendRequestEvent(type: string, extra: Partial<ListenTogetherEvent> = {}) {
    const player = usePlayerStore()
    const snap = buildControlSnapshotFields()
    if (!snap.track) {
      log.debug('skip control event without a shareable current track:', type)
      return
    }
    const isPlaying = type === 'REQUEST_PLAY'
      ? true
      : type === 'REQUEST_PAUSE'
        ? false
        : player.isPlaying
    const event: ListenTogetherEvent = {
      type,
      positionMs: player.positionMs,
      track: snap.track,
      currentIndex: snap.currentIndex,
      queue: snap.queue,
      requestTrackStableKey: snap.requestTrackStableKey,
      shouldPlay: isPlaying,
      state: isPlaying ? 'playing' : 'paused',
      repeatMode: desktopRepeatToWire(player.repeatMode),
      shuffleEnabled: !!player.shuffleEnabled,
      ...extra,
    }
    if (type === 'REQUEST_SET_TRACK' || type === 'REQUEST_SET_QUEUE') sendQueueEvent(event)
    else void sendEvent(event)
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
    const generation = _sessionGeneration
    const tick = () => {
      if (generation !== _sessionGeneration || !isController.value || !roomId.value) return
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
    _listenerPingTimer = setInterval(() => {
      if (connectionState.value !== 'connected') return
      void invoke('lt_send_ping', { t: Date.now() }).catch(() => {})
    }, LISTENER_PING_INTERVAL_MS)
  }

  function stopListenerPing() {
    if (_listenerPingTimer) {
      clearInterval(_listenerPingTimer)
      _listenerPingTimer = null
    }
  }

  // 断线重连
  function scheduleReconnect() {
    if (_reconnectTimer) return

    const delay = RECONNECT_DELAYS[Math.min(_reconnectAttempt, RECONNECT_DELAYS.length - 1)]
    _reconnectAttempt++

    _reconnectTimer = setTimeout(async () => {
      _reconnectTimer = null
      if (!_wsUrl || !roomId.value) return
      const generation = _sessionGeneration

      connectionState.value = 'connecting'
      try {
        await invoke('lt_connect_ws', { wsUrl: _wsUrl })
        if (generation !== _sessionGeneration) return
        // 重连后拉取最新 state
        const stateResp = await invoke<ListenTogetherStateResponse>('lt_get_room_state', {
          baseUrl: baseUrl.value,
          roomId: roomId.value,
        })
        if (generation !== _sessionGeneration) return
        if (stateResp.ok === false) {
          // 房间不存在/已关闭属终态: 不再无限重连, 直接离房并提示
          const t = (i18n.global as any).t
          useToastStore().error(stateResp.error || t('listen_together.room_closed'))
          await leaveRoom()
          return
        }
        if (stateResp.ok && stateResp.state) {
          if (stateResp.serverNowMs && Number.isFinite(stateResp.serverNowMs)) {
            _serverClockOffsetMs = stateResp.serverNowMs - Date.now()
          }
          commitRoomState(stateResp.state, 'reconnect', stateResp.expectedPositionMs)
        }
        if (isController.value) startHeartbeat()
      } catch {
        if (generation !== _sessionGeneration) return
        // 反复失败且非房主时, 尝试用新 token/wsUrl 重新入房恢复成员资格
        if (!isController.value && _reconnectAttempt >= RECONNECT_DELAYS.length && roomId.value) {
          const targetRoomId = roomId.value
          try {
            const resp = await invoke<ListenTogetherRoomResponse>('lt_join_room', {
              baseUrl: baseUrl.value,
              roomId: targetRoomId,
              userUuid: userUuid.value,
              nickname: nickname.value,
              joinSecret: _joinSecret || undefined,
            })
            if (generation !== _sessionGeneration) return
            if (resp.ok) {
              _reconnectAttempt = 0
              updateJoinSecret(resp.joinSecret, _joinSecret)
              const newWsUrl = resolveWsUrl(resp, targetRoomId)
              await connectWs(newWsUrl)
              startListenerPing()
              if (resp.state) {
                commitRoomState(resp.state, 'reconnect')
              }
              return
            }
          } catch {}
        }
        scheduleReconnect()
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
    const params = new URLSearchParams({ roomId: roomId.value || '' })
    const inviter = nickname.value.trim()
    if (isValidLtNickname(inviter)) params.set('inviter', inviter)
    const normalizedBaseUrl = normalizeLtHttpBaseUrl(baseUrl.value)
    if (normalizedBaseUrl && normalizedBaseUrl !== DEFAULT_BASE_URL) {
      params.set('baseUrl', normalizedBaseUrl)
    }
    if (_joinSecret) params.set('secret', _joinSecret)
    return `neriplayer://listen-together/join?${params.toString()}`
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
  async function checkClipboardInvite(): Promise<{
    roomId: string
    baseUrl?: string
    joinSecret?: string
  } | null> {
    try {
      const text = await readText()
      if (!text) return null
      // 定位邀请 URL 主体后按 query 解析, 参数顺序无关（对齐 Android decodeInviteQuery）;
      // 旧实现假定 baseUrl 紧跟 roomId, Android 生成的含 inviter 参数的链接会丢失 baseUrl
      const urlMatch = text.match(/neriplayer:\/\/listen-together\/join\?[^\s]+/i)
      if (!urlMatch) return null
      const queryStr = urlMatch[0].slice(urlMatch[0].indexOf('?') + 1)
      const params = new URLSearchParams(queryStr)
      const rawRoomId = params.get('roomId')
      if (!rawRoomId) return null
      const roomId = normalizeLtRoomId(rawRoomId)
      if (!isValidLtRoomId(roomId)) return null
      const baseUrl = normalizeLtInviteBaseUrl(params.get('baseUrl'))
      const joinSecret = normalizeLtJoinSecret(params.get('secret'))
      return {
        roomId,
        baseUrl: baseUrl || undefined,
        joinSecret,
      }
    } catch {}
    return null
  }

  // 工具函数
  function updateJoinSecret(
    value: string | null | undefined,
    fallback?: string | null,
  ) {
    _joinSecret = resolveLtJoinSecret(value, fallback) || null
  }

  function resolveWsUrl(response: ListenTogetherRoomResponse, fallbackRoomId: string): string {
    const wsUrl = response.wsUrl?.trim()
    if (wsUrl && !isInternalRoomWsUrl(wsUrl)) return wsUrl
    const token = response.token?.trim()
    if (!token) throw new Error('Listen Together response did not include a WebSocket token')
    return buildWsUrl(baseUrl.value, fallbackRoomId, token)
  }

  function isInternalRoomWsUrl(value: string): boolean {
    const normalized = value.toLowerCase()
    return normalized.includes('://room.internal/')
      || normalized.includes('://room.internal?')
      || normalized.includes('://room.internal:')
  }

  function buildWsUrl(base: string, roomId: string, token: string): string {
    const normalized = normalizeLtHttpBaseUrl(base) || base.replace(/\/$/, '')
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
    // 曲目自然播完上报（供 player.handleTrackEnded 调用）
    reportTrackFinished,
  }
})
