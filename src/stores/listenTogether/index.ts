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
  LtInvite,
} from './protocol'
import {
  desktopRepeatToWire,
  isValidLtNickname,
  LtChannels,
  normalizeLtHttpBaseUrl,
  normalizeLtInviteBaseUrl,
  normalizeLtJoinSecret,
  normalizeLtRoomId,
  parseLtInvite,
  resolveLocalRoomControlRestriction,
  resolveLtJoinSecret,
  isValidLtRoomId,
} from './protocol'
import { trackInfoToLtTrack, ltTrackToTrackInfo, toShareableQueueSnapshot, trustedInboundStreamUrls } from './mapper'
import { queueReferences, applyListenTogetherQueueMutation, buildListenTogetherQueueMutationPlan, getLtQueueReference, setLtQueueReference } from './queue'
import { acceptRoomState, resolveExpectedPosition, resolvePositionSync, resolveSoftSyncRecheckAction, SOFT_SYNC_RECHECK_INTERVAL_MS, updateServerClockOffset } from './playbackSync'
import { isTerminalReconnectError, MAX_RECONNECT_ATTEMPTS, reconnectDelayMs } from './reconnect'
import {
  isMemberRequestSatisfied,
  pendingMemberRequestAction,
  roomCurrentStableKey,
  shouldRefreshListenerState,
  StallDetector,
  TRACKED_MEMBER_REQUESTS,
  WATCHDOG_INTERVAL_MS,
  type PendingMemberRequest,
} from './watchdog'
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
// ping 发出后这么久仍没收到任何消息就认为连接已断（对齐 Android LISTEN_TOGETHER_SOCKET_RESPONSE_TIMEOUT_MS）
const SOCKET_RESPONSE_TIMEOUT_MS = 35_000
// 已处理转发请求 eventId 上限（对齐 Android ForwardedRequestDeduper 语义）
const HANDLED_FORWARDED_EVENT_LIMIT = 256
const HANDLED_FORWARDED_REQUESTER_LIMIT = 64

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
  // 设置页里的房间选项是「新建房间的默认值」；进房后生效的是房间自己的设置
  // （建房时取默认值，加入时取房主下发的）。房间状态同步不得改写用户偏好，
  // 只有用户主动修改（updateRoomSettings）才写回 settings（对齐 Android ListenTogetherPreferences）
  function preferredRoomSettings(): ListenTogetherRoomSettings {
    return {
      allowMemberControl: settings.ltAllowMemberControl,
      autoPauseOnMemberChange: settings.ltAutoPauseOnMemberChange,
      shareAudioLinks: settings.ltShareAudioLinks,
    }
  }
  const liveRoomSettings = ref<ListenTogetherRoomSettings | null>(null)
  const roomSettings = computed<ListenTogetherRoomSettings>(() => liveRoomSettings.value ?? preferredRoomSettings())

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
  // 服务端回过 "unsupported event type: np_ping" 后改发旧版 ping
  let _legacyPing = false
  // WebSocket 和 HTTP 都发不出去的控制（对齐 Android ListenTogetherControlOutbox）：
  // 按意图只留最新一条，最多 8 条，重连后重放；队列类保存完整快照，重放时不依赖旧的版本基线
  const _outbox = new Map<string, { event: ListenTogetherEvent; queueSnapshot: boolean }>()
  // 房主断线期间自己改过播放：重连时的 welcome/房态只更新房间，不能把这些改动撤回去
  let _hostControlledOffline = false
  // 正在应答链接请求的曲目，同一首歌的重复请求只应答一次
  const _linkAnswers = new Set<string>()
  // 半开连接检测（对齐 Android SocketHealthOwner）：最后一次收到任何消息、最后一次发 ping 的时间
  let _lastSocketMessageAt = 0
  let _pingSentAt = 0
  // 听众端自检：定时对齐、卡住恢复、静默时拉房态；以及还没被房主处理的成员请求
  let _watchdogTimer: ReturnType<typeof setInterval> | null = null
  let _lastWatchdogRefreshAt = 0
  // 收到过解析不了的消息：漏掉的很可能是房态更新，下一次自检时补拉
  let _stateRepairPending = false
  let _pendingMemberRequest: PendingMemberRequest | null = null
  const _stallDetector = new StallDetector()
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
  let _unlistenProtocolError: UnlistenFn | null = null
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
  // np_ping 的发送时间 -> 发送时的单调时钟，用来算往返时间
  const _pingSentElapsed = new Map<number, number>()
  let _lastRequestedLinkStableKey: string | null = null
  let _lastRequestedLinkAt = 0
  let _joinSecret: string | null = null
  // 通过别的服务器的邀请加入时只在本次会话使用那台服务器，不改写用户自己的服务器设置
  let _sessionBaseUrl: string | null = null
  const activeBaseUrl = () => _sessionBaseUrl ?? baseUrl.value

  /** 时钟偏移按 Android 平滑：单向样本含网络延迟，往返样本取中点，往返超过 30s 的丢弃 */
  function sampleServerClock(serverNowMs: number | null | undefined, sentAtWallMs?: number, sentAtElapsedMs?: number) {
    _serverClockOffsetMs = updateServerClockOffset({
      previousOffsetMs: _serverClockOffsetMs,
      serverNowMs,
      sentAtWallMs,
      sentAtElapsedMs,
      nowWallMs: Date.now(),
      nowElapsedMs: performance.now(),
    })
  }

  // 计算属性
  const isConnected = computed(() => connectionState.value === 'connected')
  const isController = computed(() => role.value === 'controller')
  // 播放器据此在本地操作执行前就拦下，而不是先改本地、再被服务端拒绝
  const localControlRestriction = computed(() => roomId.value
    ? resolveLocalRoomControlRestriction(roomState.value?.roomStatus, roomSettings.value.allowMemberControl, isController.value)
    : null)
  const members = computed(() => roomState.value?.members ?? [])

  // 房间操作
  /** 创建房间 */
  async function createRoom() {
    const player = usePlayerStore()
    const toast = useToastStore()
    const t = (i18n.global as any).t
    const generation = _sessionGeneration
    _sessionBaseUrl = null

    try {
      sessionError.value = null
      if (!isValidLtNickname(nickname.value)) {
        throw new Error(t('listen_together.invalid_nickname'))
      }
      connectionState.value = 'connecting'
      liveRoomSettings.value = preferredRoomSettings()

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
      // 服务端要求初始快照里有可共享的当前曲目，否则只会回一条生硬的 HTTP 400
      if (!initialTrack) throw new Error(t('listen_together.create_requires_shareable_track'))

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
      // 自检只对听众生效；房主身份可能在会话中转交，所以建房时也启动
      startWatchdog()

    } catch (e) {
      if (generation !== _sessionGeneration) return
      const msg = e instanceof Error ? e.message : String(e)
      sessionError.value = msg
      connectionState.value = 'disconnected'
      liveRoomSettings.value = null
      toast.error(t('listen_together.create_failed', { msg }))
    }
  }

  /** 加入房间 */
  async function joinRoom(targetRoomId: string, joinSecret?: string, baseUrlOverride?: string) {
    const player = usePlayerStore()
    const toast = useToastStore()
    const t = (i18n.global as any).t
    const generation = _sessionGeneration
    _sessionBaseUrl = baseUrlOverride?.trim() || null

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
        baseUrl: activeBaseUrl(),
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
        if (resp.state.settings) liveRoomSettings.value = { ...resp.state.settings }
        markSync('INITIAL_STATE', resp.state.updatedAt || Date.now())
        // 将服务端状态应用到本地播放器
        applyRoomStateToPlayer(resp.state, 'join')
      }

      const wsUrl = resolveWsUrl(resp, normalizedRoomId)
      await connectWs(wsUrl)
      if (generation !== _sessionGeneration) return

      startListenerPing()
      setupPlayerWatch()
      startWatchdog()

    } catch (e) {
      if (generation !== _sessionGeneration) return
      const msg = e instanceof Error ? e.message : String(e)
      sessionError.value = msg
      connectionState.value = 'disconnected'
      liveRoomSettings.value = null
      toast.error(t('listen_together.join_failed', { msg }))
    }
  }

  /** 离开房间 */
  async function leaveRoom() {
    // 对齐 Android pauseBeforeLeave：房间开着"成员变动时暂停"（默认开）时，离开前先停下自己的播放
    if (roomId.value && (roomSettings.value.autoPauseOnMemberChange ?? true)) {
      const player = usePlayerStore()
      if (player.isPlaying) await player.pause('remote_sync')
    }
    await endSession(true)
  }

  /**
   * 服务器已经结束了这次会话（房间关闭、凭据失效、重连次数用尽）：只在本地收尾，
   * 不再向 /leave 发请求，并把原因留给界面显示（对齐 Android closeRoomLocally）
   */
  async function closeRoomLocally(reason: string) {
    if (!roomId.value) return
    await endSession(false)
    sessionError.value = reason
    useToastStore().error((i18n.global as any).t('listen_together.room_closed'))
  }

  async function endSession(notifyServer: boolean) {
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

    if (notifyServer) {
      try {
        const result = await invoke<ListenTogetherControlResponse | null>('lt_leave_room')
        if (result?.ok === false) log.warn('leave room rejected:', result.error)
      } catch (error) {
        log.warn('leave room request failed:', error)
      }
    }
    try {
      await invoke('lt_disconnect_ws')
    } catch {}

    roomId.value = null
    _joinSecret = null
    role.value = null
    roomState.value = null
    liveRoomSettings.value = null
    _lastAppliedRoomVersion = 0
    sessionError.value = null
    connectionState.value = 'disconnected'
    lastSyncEventType.value = null
    lastSyncAt.value = null
    lastReconnectAt.value = null
    _wsUrl = null
    _activeWsConnectionId = null
    _reconnectAttempt = 0
    _legacyPing = false
    _outbox.clear()
    _hostControlledOffline = false
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
    _pingSentElapsed.clear()
    _sessionBaseUrl = null
    _lastSocketMessageAt = 0
    _pingSentAt = 0
    stopWatchdog()
    _pendingMemberRequest = null
    _stateRepairPending = false
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
        replayOutbox()
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

      // 服务器以"房间已关闭"等理由关掉连接时重连没有意义
      if (roomId.value && isTerminalReconnectError(event.payload.reason)) {
        void closeRoomLocally(event.payload.reason)
        return
      }
      // 是否需要重连
      if (wasConnected && roomId.value) {
        scheduleReconnect()
      }
    })
    if (generation !== _sessionGeneration) { unlistenDisconnected(); return false }
    _unlistenDisconnected = unlistenDisconnected

    // 解析不了的消息：记下原因，漏掉的多半是房态更新，交给自检尽快补拉
    const unlistenProtocolError = await listen<{ connectionId?: string; message?: string }>('lt:protocol_error', (event) => {
      if (generation !== _sessionGeneration) return
      if (event.payload.connectionId && event.payload.connectionId !== _activeWsConnectionId) return
      sessionError.value = event.payload.message || 'Protocol error'
      _stateRepairPending = true
    })
    if (generation !== _sessionGeneration) { unlistenProtocolError(); return false }
    _unlistenProtocolError = unlistenProtocolError
    return true
  }

  function teardownListeners() {
    _unlistenMessage?.()
    _unlistenConnected?.()
    _unlistenDisconnected?.()
    _unlistenProtocolError?.()
    _unlistenMessage = null
    _unlistenConnected = null
    _unlistenDisconnected = null
    _unlistenProtocolError = null
  }

  // 消息处理
  function handleSocketMessage(envelope: ListenTogetherSocketEnvelope) {
    _lastSocketMessageAt = Date.now()
    if (envelope.state && envelope.state.roomId !== roomId.value) return
    // 用服务端时间戳更新时钟偏移（每条带 nowMs 的消息都更新并平滑，对齐 Android SocketHealthOwner）
    if (envelope.type !== 'np_pong') sampleServerClock(envelope.nowMs)
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
        const sentAtElapsed = typeof sentAt === 'number' ? _pingSentElapsed.get(sentAt) : undefined
        if (typeof sentAt === 'number') _pingSentElapsed.delete(sentAt)
        sampleServerClock(envelope.nowMs, sentAt, sentAtElapsed)
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
      // 拒绝结果没有 applied，事件 id 在信封的 causedBy 上；只让被点名的那个队列事件失败，
      // 别的拒绝（例如链接请求被限流）不能连带丢掉在途和排队的队列修改（对齐 Android）
      failRejectedQueueEvent(appliedCause?.eventId ?? envelope.causedBy?.eventId, err)
      handleSessionRejection(err)
      // 以服务端状态为准重新对齐，优先用 applied.state，回退 envelope.state
      const rollback = applied?.state || envelope.state
      if (rollback && roomId.value) {
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
    failRejectedQueueEvent(envelope.causedBy?.eventId, err)
    handleSessionRejection(err)
  }

  /**
   * 被拒绝的控制和错误信封不弹 toast，按 Android 的顺序尝试恢复：旧服务端不认识 np_ping 时
   * 退回普通 ping；房间已关闭等终态在本地结束会话；听众被移出成员时重新入房；其余记为会话错误
   */
  function handleSessionRejection(err: string) {
    log.warn('server rejected a control:', err)
    const lower = err.toLowerCase()
    if (lower.includes('np_ping') && lower.includes('unsupported')) {
      _legacyPing = true
      return
    }
    if (isTerminalReconnectError(err)) {
      void closeRoomLocally(err)
      return
    }
    if (!isController.value && lower.includes('member not in room')) {
      scheduleReconnect()
      return
    }
    sessionError.value = err
  }

  function handleWelcome(envelope: ListenTogetherSocketEnvelope) {
    if (envelope.role) {
      role.value = envelope.role as LtRole
    }
    if (envelope.state) {
      commitRoomState(envelope.state, 'WELCOME', envelope.expectedPositionMs,
        !(isController.value && _hostControlledOffline))
    }
    if (isController.value) startHeartbeat()
    else stopHeartbeat()
  }

  function commitRoomState(state: ListenTogetherRoomState, causeType: string, expectedPositionMs?: number, apply = true) {
    const accepted = acceptRoomState(roomState.value, state, roomId.value, _lastAppliedRoomVersion)
    if (!accepted) return
    roomState.value = accepted
    _lastAppliedRoomVersion = accepted.version
    if (_pendingMemberRequest
      && isMemberRequestSatisfied(_pendingMemberRequest.event, accepted, Date.now() + _serverClockOffsetMs)) {
      _pendingMemberRequest = null
    }
    if (accepted.settings) liveRoomSettings.value = { ...accepted.settings }
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
    // 房主收到链接请求：下发当前曲目的可分享直链
    if (!isController.value || !roomSettings.value.shareAudioLinks) return

    // 检查 requestTrackStableKey 是否与当前曲目匹配
    const player = usePlayerStore()
    if (!player.currentTrack) return

    const currentKey = trackInfoToLtTrack(player.currentTrack).stableKey
    const requestedKey = envelope.requestTrackStableKey ?? currentKey
    if (requestedKey !== currentKey) return // 请求的曲目已不是当前播放的
    void answerLinkRequest(requestedKey, _sessionGeneration)
  }

  function currentShareableUrls(): string[] {
    const player = usePlayerStore()
    const id = player.currentTrack?.id
    return [...new Set([player.getCurrentStreamUrl(id), ...player.getCurrentStreamUrls(id)]
      .filter((url): url is string => !!url))]
  }

  const delay = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms))

  /**
   * 对齐 Android ListenTogetherControllerLinkOwner：房主自己还在加载时每 200ms 看一次，最多 40 次；
   * 还没有（从缓存/下载播放时本来就没有直链）就单独解析，最多 3 次、间隔 2s。
   * 拿到就回 LINK_READY，彻底拿不到回 LINK_UNAVAILABLE，听众不会一直干等
   */
  async function answerLinkRequest(stableKey: string, generation: number) {
    if (_linkAnswers.has(stableKey)) return
    _linkAnswers.add(stableKey)
    try {
      const player = usePlayerStore()
      const stillCurrent = () => generation === _sessionGeneration && isController.value
        && roomSettings.value.shareAudioLinks && !!player.currentTrack
        && trackInfoToLtTrack(player.currentTrack).stableKey === stableKey
      let urls = currentShareableUrls()
      for (let attempt = 0; urls.length === 0 && player.isLoadingAudio && attempt < 40; attempt++) {
        await delay(200)
        if (!stillCurrent()) return
        urls = currentShareableUrls()
      }
      for (let attempt = 0; urls.length === 0 && attempt < 3; attempt++) {
        if (attempt > 0) await delay(2000)
        if (!stillCurrent()) return
        urls = await player.resolveShareableStreamUrls(player.currentTrack!)
        if (!stillCurrent()) return
      }
      if (urls.length === 0) {
        sendEvent({ type: 'LINK_UNAVAILABLE', requestTrackStableKey: stableKey })
        return
      }
      const { queue, resolvedIndex } = toShareableQueueSnapshot(
        player.queue, player.queueIndex, true, urls[0], false, urls,
      )
      const track = queue[resolvedIndex]
      if (!track || track.stableKey !== stableKey || !track.streamUrl) return
      // 只带这首歌和它的直链，不附整份队列（对齐 Android LINK_READY）
      sendEvent({
        type: 'LINK_READY',
        track,
        currentIndex: resolvedIndex,
        positionMs: player.positionMs,
        state: player.isPlaying ? 'playing' : 'paused',
        requestTrackStableKey: stableKey,
      })
    } finally {
      _linkAnswers.delete(stableKey)
    }
  }

  /** force 用于卡住恢复：同一首歌的上一次请求可能已经丢了，不受 4 秒节流限制 */
  function requestLinkForTrack(track: import('./protocol').ListenTogetherTrack, currentIndex: number, force = false) {
    if (isController.value || !roomSettings.value.shareAudioLinks || currentIndex < 0) return
    // 房主离线时服务端只会回 "controller offline"，没必要发（对齐 Android）
    if (connectionState.value !== 'connected' || (roomState.value?.roomStatus ?? 'active') !== 'active') return
    const now = Date.now()
    if (
      !force
      && _lastRequestedLinkStableKey === track.stableKey
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

  /** 房主暂时离线：只记下房间状态，不动播放器；听众看到一条普通提示，房主自己不需要（对齐 Android） */
  function handleRoomSuspended(envelope: ListenTogetherSocketEnvelope) {
    markSync('ROOM_SUSPENDED')
    if (envelope.state) commitRoomState(envelope.state, 'ROOM_SUSPENDED', envelope.expectedPositionMs, false)
    setSyncRate(null)
    if (!isController.value) useToastStore().show((i18n.global as any).t('listen_together.controller_offline'))
  }

  /** 房主回来了：听众按恢复后的房态对齐；房主的播放器本来就是房态的来源 */
  function handleRoomResumed(envelope: ListenTogetherSocketEnvelope) {
    markSync('ROOM_RESUMED')
    if (envelope.state) {
      commitRoomState(envelope.state, 'ROOM_RESUMED', envelope.expectedPositionMs, !isController.value)
    }
  }

  /** 房间已经关闭：按关闭时的状态停下再在本地收尾，不再向已关闭的房间发 /leave（对齐 Android） */
  function handleRoomClosed(envelope: ListenTogetherSocketEnvelope) {
    if (envelope.state?.playback?.state === 'paused') void usePlayerStore().pause('remote_sync')
    void closeRoomLocally(envelope.state?.closedReason || envelope.message || 'room_closed')
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

  /** 房主给的直链标了音质时，听众按自己的音质设置挑（对齐 Android orderListenTogetherStreamUrlsForPreference） */
  function preferredStreamQuality(channelId: string): string | undefined {
    if (channelId === LtChannels.NETEASE) return settings.neteaseQuality
    if (channelId === LtChannels.BILIBILI) return settings.biliQuality
    if (channelId === LtChannels.YOUTUBE_MUSIC) return settings.youtubeQuality
    return undefined
  }

  function replacePlayerQueue(queue: import('./protocol').ListenTogetherTrack[], currentIndex: number) {
    const player = usePlayerStore()
    const references = queueReferences(queue)
    const tracks = queue.map((track, index) => ({
      ...ltTrackToTrackInfo(track, preferredStreamQuality(track.channelId)),
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

  // 播放器同步；forceReload 用于卡在加载的同一首歌，换掉那次卡住的加载
  function applyRoomStateToPlayer(state: ListenTogetherRoomState, causeType: string, expectedPositionMs?: number, forceReload = false) {
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
      const remoteTrack = ltTrackToTrackInfo(effectiveLtTrack, preferredStreamQuality(effectiveLtTrack.channelId))
      if (!isController.value && roomSettings.value.shareAudioLinks && !remoteTrack.audioUrl) {
        requestLinkForTrack(effectiveLtTrack, targetIndex)
      }
      const streamChanged = !!remoteTrack.audioUrl
        && !trustedInboundStreamUrls(effectiveLtTrack.channelId, effectiveLtTrack.streamUrls, effectiveLtTrack.streamUrl)
          .includes(player.getCurrentStreamUrl(remoteTrack.id) || '')
      replacePlayerQueue(queue, targetIndex)
      if (playbackContextChanged || streamChanged || forceReload) {
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
        isLoadingAudio: player.isLoadingAudio,
        repeatMode: player.repeatMode,
        shuffleEnabled: player.shuffleEnabled,
      }),
      (newVal) => {
        // 断线时也照常上报：发不出去会走 HTTP，再不行进待补发队列，不能让本地操作被重连时的房态撤回
        if (_suppressPlayerWatch || _pendingRemotePlaybackLoads.size > 0 || player.isRemoteSyncGuardActive()) return

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
                shouldPlay: intendsToPlay(),
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

        // 播放状态变化；加载中的"暂停"不上报，否则每次换歌都会先提交暂停、加载完再提交播放
        const playing = newVal.isPlaying || (newVal.isLoadingAudio && _lastReportedIsPlaying === true)
        if (playing !== _lastReportedIsPlaying) {
          _lastReportedIsPlaying = playing
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
        if (_suppressPlayerWatch || _pendingRemotePlaybackLoads.size > 0) return
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

  /** 断线时也能补发的控制按意图归类（对齐 Android ListenTogetherControlOutbox 的键） */
  function outboxKey(type: string): string | null {
    switch (type) {
      case 'SET_TRACK': case 'REQUEST_SET_TRACK': return 'track'
      case 'SET_QUEUE': case 'REQUEST_SET_QUEUE': return 'queue'
      case 'PLAY': case 'PAUSE': case 'REQUEST_PLAY': case 'REQUEST_PAUSE': return 'transport'
      case 'SEEK': case 'REQUEST_SEEK': return 'seek'
      case 'PLAYBACK_MODE': case 'REQUEST_PLAYBACK_MODE': return 'playback_mode'
      case 'TRACK_FINISHED': return 'track_finished'
      default: return null
    }
  }

  function rememberForReplay(event: ListenTogetherEvent, queueSnapshot: boolean): boolean {
    const key = outboxKey(event.type)
    if (!key) return false
    _outbox.delete(key)
    _outbox.set(key, { event, queueSnapshot })
    while (_outbox.size > 8) _outbox.delete(_outbox.keys().next().value!)
    return true
  }

  /** 重连后按原顺序补发；换新的事件 id 与序号，不会被当成已处理或乱序的旧事件 */
  function replayOutbox() {
    if (_outbox.size === 0) return
    const entries = [..._outbox.values()]
    _outbox.clear()
    for (const { event, queueSnapshot } of entries) {
      if (queueSnapshot) sendQueueEvent(event, true)
      else void sendEvent({ ...event, eventId: undefined, clientTimeMs: undefined, clientSequence: undefined })
    }
  }

  async function sendEvent(event: ListenTogetherEvent) {
    if (!roomId.value) return
    stripUnsharedAudioLinks(event)
    if (isController.value && connectionState.value !== 'connected' && outboxKey(event.type)) {
      _hostControlledOffline = true
    }
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
      handleControlResult({ type: 'control_result', result, causedBy: { eventId: event.eventId, type: event.type } })
      if (result.ok && result.applied?.state) {
        if (!queueEvent || _queueEventInFlight?.eventId === event.eventId) {
          // 房主自己的控制已经在本地生效，提交结果只更新房间状态，不再反过来驱动房主的播放器
          commitRoomState(result.applied.state, result.applied.causedBy?.type || event.type,
            result.applied.expectedPositionMs, !isController.value && !queueEvent && !_queuedQueueEvent)
          if (result.applied.state.roomId === roomId.value && result.applied.state.version >= _lastAppliedRoomVersion) {
            completeQueueEvent(event.eventId)
          }
        }
      }
    } catch (error) {
      if (generation !== _sessionGeneration) return
      log.warn('HTTP event send failed:', error)
      // 两条链路都不通（离线）：留到重连后补发。队列取最新的完整快照，排在后面的修改也不会丢
      const queueEvent = _queueEventInFlight?.eventId === event.eventId
      const replay = queueEvent ? _queuedQueueEvent ?? _queueEventSnapshot : event
      const kept = replay ? rememberForReplay(replay, queueEvent) : false
      completeQueueEvent(event.eventId, false)
      if (!kept) useToastStore().error((i18n.global as any).t('listen_together.control_not_sent'))
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

  // 房主在放本地文件、QQ 音乐这类无法共享的歌时不上报播放控制：服务端会把位置和状态
  // 套到房间里上一首歌上，听众被拖着跳转或暂停（对齐 Android）
  function reportPlayEvent() {
    const player = usePlayerStore()
    const binding = controlBindingFields()
    if (!binding.track || shouldSkipControlEvent('PLAY')) return
    sendEvent({
      type: 'PLAY',
      ...binding,
      positionMs: player.positionMs,
      state: 'playing',
    })
  }

  function reportPauseEvent() {
    const player = usePlayerStore()
    const binding = controlBindingFields()
    if (!binding.track || shouldSkipControlEvent('PAUSE')) return
    sendEvent({
      type: 'PAUSE',
      ...binding,
      positionMs: player.positionMs,
      state: 'paused',
    })
  }

  function reportSeekEvent(positionMs: number) {
    const binding = controlBindingFields()
    if (!binding.track || shouldSkipSeekEvent(positionMs)) return
    sendEvent({
      type: 'SEEK',
      ...binding,
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
    if (!pending || !roomId.value) return

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

  /**
   * PLAY/PAUSE/SEEK 与成员请求只需要绑定当前曲目：schema 2 起服务端不看这些事件里的队列，
   * 每次都带上整份队列（可能上千首，还含直链）只是浪费（对齐 Android EventFactory）
   */
  function controlBindingFields() {
    const snap = buildControlSnapshotFields()
    if ((roomState.value?.schemaVersion ?? 0) < 2 || !snap.track) return snap
    const { streamUrl: _url, streamUrls: _urls, ...track } = snap.track
    return { queue: undefined, currentIndex: snap.currentIndex, track, requestTrackStableKey: snap.requestTrackStableKey }
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
      shouldPlay: intendsToPlay(),
    })
  }

  /** 换歌加载期间 isPlaying 会短暂为 false，但用户要的是播放；服务端按 shouldPlay 提交 SET_TRACK 的状态 */
  function intendsToPlay(): boolean {
    const player = usePlayerStore()
    return player.isPlaying || player.isLoadingAudio
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

  function isQueueCompatibilityError(error: string): boolean {
    const normalized = error.trim().toLowerCase()
    return ['queue mutation is invalid', 'queue mutation base version is ahead',
      'queue mutation event type unsupported', 'queue update queue required'].some(message => normalized.includes(message))
  }

  /** 被拒绝的队列事件：按 eventId 匹配；没带 id 的队列兼容性错误只可能指在途的队列事件 */
  function failRejectedQueueEvent(ackId: string | undefined, error: string) {
    const target = ackId ?? (isQueueCompatibilityError(error) ? _queueEventInFlight?.eventId : undefined)
    if (target && _queueEventInFlight?.eventId === target) completeQueueEvent(target, false)
  }

  function retryLegacyQueueSnapshot(error: string): boolean {
    if (!isQueueCompatibilityError(error) || !_queueEventInFlight?.queueMutation || !_queueEventSnapshot) return false
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
      // 服务端以它作为完成位置；不带时退回服务端推算的位置，可能还差几秒（对齐 Android EventFactory）
      positionMs: Math.max(finishedLtTrack.durationMs ?? 0, player.positionMs),
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
    // REQUEST_SET_TRACK 需要的完整队列由调用方经 extra 传入
    const snap = controlBindingFields()
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
    if (type === 'REQUEST_SET_TRACK' || type === 'REQUEST_SET_QUEUE') {
      sendQueueEvent(event)
      return
    }
    if (TRACKED_MEMBER_REQUESTS.has(type)) {
      const now = Date.now()
      _pendingMemberRequest = { event, createdAt: now, lastSentAt: now, attempts: 1 }
    }
    void sendEvent(event)
  }

  function startWatchdog() {
    stopWatchdog()
    _lastWatchdogRefreshAt = 0
    _stallDetector.reset()
    const generation = _sessionGeneration
    _watchdogTimer = setInterval(() => {
      if (generation === _sessionGeneration) void runWatchdogTick(generation)
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
    const serverNow = now + _serverClockOffsetMs
    const state = roomState.value
    let requestPending = false
    if (_pendingMemberRequest) {
      const action = pendingMemberRequestAction(_pendingMemberRequest, state, now, serverNow)
      if (action === 'satisfied' || action === 'expired') {
        _pendingMemberRequest = null
      } else {
        requestPending = true
        if (action === 'retry') {
          _pendingMemberRequest = { ..._pendingMemberRequest, lastSentAt: now, attempts: _pendingMemberRequest.attempts + 1 }
          void sendEvent(_pendingMemberRequest.event)
        }
      }
    }
    // 自己的请求还在等房主处理时不按房态回放，否则会把用户刚做的操作撤回去
    if (state && !requestPending && state.roomStatus === 'active') syncListenerToRoom(state, now, serverNow)

    if (!shouldRefreshListenerState(now, _lastSocketMessageAt, _lastWatchdogRefreshAt, _stateRepairPending)) return
    _lastWatchdogRefreshAt = now
    _stateRepairPending = false
    const targetRoomId = roomId.value
    try {
      const resp = await invoke<ListenTogetherStateResponse>('lt_get_room_state', { baseUrl: activeBaseUrl(), roomId: targetRoomId })
      if (generation !== _sessionGeneration || !resp.ok || !resp.state) return
      sampleServerClock(resp.serverNowMs)
      commitRoomState(resp.state, 'WATCHDOG_REFRESH', resp.expectedPositionMs, !_pendingMemberRequest)
    } catch (error) {
      if (generation !== _sessionGeneration) return
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
    if (player.isLoadingAudio || _pendingRemotePlaybackLoads.size > 0) return
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
      || (sync.rate !== null && _softSyncRate === null)
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
    _pingSentAt = 0
    _listenerPingTimer = setInterval(() => {
      if (connectionState.value !== 'connected') return
      // 上一个 ping 发出后还没收到任何消息：35s 内先等着，不加发；超过就当连接已经半开，主动重连
      if (_pingSentAt > 0 && _lastSocketMessageAt <= _pingSentAt) {
        if (Date.now() - _pingSentAt >= SOCKET_RESPONSE_TIMEOUT_MS) handleUnresponsiveSocket()
        return
      }
      const sentAt = Date.now()
      _pingSentAt = sentAt
      _pingSentElapsed.set(sentAt, performance.now())
      while (_pingSentElapsed.size > 8) _pingSentElapsed.delete(_pingSentElapsed.keys().next().value!)
      void invoke('lt_send_ping', { t: sentAt, legacy: _legacyPing }).catch(() => {})
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
    _activeWsConnectionId = null
    stopListenerPing()
    stopHeartbeat()
    setSyncRate(null)
    if (roomId.value) scheduleReconnect()
  }

  // 断线重连
  function scheduleReconnect() {
    if (_reconnectTimer) return
    // 对齐 Android：重连有上限，用尽后在本地结束会话，而不是一直显示"连接中"
    if (_reconnectAttempt >= MAX_RECONNECT_ATTEMPTS) {
      void closeRoomLocally('reconnect_max_attempts_exceeded')
      return
    }
    _reconnectAttempt++
    const delay = reconnectDelayMs(_reconnectAttempt)

    _reconnectTimer = setTimeout(async () => {
      _reconnectTimer = null
      if (!_wsUrl || !roomId.value) return
      const generation = _sessionGeneration
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
            joinSecret: _joinSecret || undefined,
          })
          if (generation !== _sessionGeneration) return
          if (!resp.ok) throw new Error(resp.error || 'rejoin failed')
          updateJoinSecret(resp.joinSecret, _joinSecret)
          _wsUrl = resolveWsUrl(resp, targetRoomId)
        }
        await invoke('lt_connect_ws', { wsUrl: _wsUrl })
        if (generation !== _sessionGeneration) return
        // 重连后拉取最新 state
        const stateResp = await invoke<ListenTogetherStateResponse>('lt_get_room_state', {
          baseUrl: activeBaseUrl(),
          roomId: targetRoomId,
        })
        if (generation !== _sessionGeneration) return
        if (stateResp.ok === false) throw new Error(stateResp.error || 'room closed')
        if (stateResp.ok && stateResp.state) {
          sampleServerClock(stateResp.serverNowMs)
          commitRoomState(stateResp.state, 'reconnect', stateResp.expectedPositionMs,
            !(isController.value && _hostControlledOffline))
        }
        _hostControlledOffline = false
        if (isController.value) startHeartbeat()
      } catch (error) {
        if (generation !== _sessionGeneration) return
        const message = error instanceof Error ? error.message : String(error)
        if (isTerminalReconnectError(message)) {
          await closeRoomLocally(message)
          return
        }
        scheduleReconnect()
      }
    }, delay)
  }

  // 用户主动修改房间选项：总是写回默认值；正在主持房间时同步给房间（听众无权改房间设置）
  async function updateRoomSettings(newSettings: Partial<ListenTogetherRoomSettings>) {
    if (newSettings.allowMemberControl !== undefined) settings.ltAllowMemberControl = newSettings.allowMemberControl
    if (newSettings.autoPauseOnMemberChange !== undefined) settings.ltAutoPauseOnMemberChange = newSettings.autoPauseOnMemberChange
    if (newSettings.shareAudioLinks !== undefined) settings.ltShareAudioLinks = newSettings.shareAudioLinks
    if (!liveRoomSettings.value || !isController.value) return
    liveRoomSettings.value = { ...liveRoomSettings.value, ...newSettings }
    if (connectionState.value === 'connected') {
      sendEvent({
        type: 'UPDATE_SETTINGS',
        roomSettings: liveRoomSettings.value,
      })
    }
  }

  /** 是否在房间会话中（含连接中 / 断线重连） */
  const isInSession = computed(() => roomId.value !== null || connectionState.value !== 'disconnected')

  /**
   * 重新生成身份 UUID，立即生效。房间内拒绝执行（对齐 Android：先离开房间），返回是否成功。
   * 昵称保持不变；未设置昵称时默认名随新 UUID 变化。
   */
  function resetIdentity(): boolean {
    if (isInSession.value) return false
    const next = crypto.randomUUID()
    localStorage.setItem(LT_UUID_KEY, next)
    userUuid.value = next
    return true
  }

  /** 配置导入改写了持久化的 UUID 后重新读取（会话中不切换身份） */
  function reloadIdentity() {
    if (isInSession.value) return
    userUuid.value = loadOrCreateUuid()
  }

  // 邀请链接
  function getInviteLink(): string {
    const params = new URLSearchParams({ roomId: roomId.value || '' })
    const inviter = nickname.value.trim()
    if (isValidLtNickname(inviter)) params.set('inviter', inviter)
    const normalizedBaseUrl = normalizeLtHttpBaseUrl(activeBaseUrl())
    if (normalizedBaseUrl && normalizedBaseUrl !== DEFAULT_BASE_URL) {
      params.set('baseUrl', normalizedBaseUrl)
    }
    if (_joinSecret) params.set('secret', _joinSecret)
    return `neriplayer://listen-together/join?${params.toString()}`
  }

  async function copyInviteLink() {
    const toast = useToastStore()
    const t = (i18n.global as any).t
    // 服务端要求邀请带密钥，没有密钥的链接别人用了只会被拒绝
    if (!_joinSecret) {
      toast.error(t('listen_together.invite_unavailable'))
      return
    }
    // 与 Android 相同：一句说明加链接，两端都能从整段文字里识别出链接
    const inviter = nickname.value.trim()
    const text = t('listen_together.invite_share_text', {
      inviter: isValidLtNickname(inviter) ? inviter : t('listen_together.title'),
      roomId: roomId.value || '',
    })
    try {
      await writeText(`${text}\n${getInviteLink()}`)
      toast.success(t('listen_together.invite_copied'))
    } catch {}
  }

  /** 检测剪贴板中的邀请链接 */
  async function checkClipboardInvite(): Promise<LtInvite | null> {
    try {
      const invite = parseLtInvite(await readText())
      if (!invite) return null
      if (invite.hasInvalidBaseUrl) log.warn('ignored an invite server that is not https')
      return invite
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
    return buildWsUrl(activeBaseUrl(), fallbackRoomId, token)
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
    isConnected, isController, isInSession, members, localControlRestriction,
    // 方法
    createRoom, joinRoom, leaveRoom,
    updateRoomSettings, resetIdentity, reloadIdentity, copyInviteLink, getInviteLink,
    checkClipboardInvite,
    // 暴露给外部（seek 上报）
    reportSeekEvent,
    // 曲目自然播完上报（供 player.handleTrackEnded 调用）
    reportTrackFinished,
  }
})
