/**
 * 一起听 Pinia Store
 * 房间管理、WebSocket 通信、播放同步
 */
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager'
import { usePlayerStore } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import i18n from '@/i18n'
import {
  type ConnectionState,
  type LtRole,
  type ListenTogetherRoomState,
  type ListenTogetherRoomSettings,
  type ListenTogetherSocketEnvelope,
  type ListenTogetherInitialSnapshot,
  type ListenTogetherRoomResponse,
  type ListenTogetherControlResponse,
  type LtInvite,
  desktopRepeatToWire,
  isValidLtNickname,
  LtChannels,
  normalizeLtHttpBaseUrl,
  normalizeLtJoinSecret,
  normalizeLtRoomId,
  parseLtInvite,
  resolveLocalRoomControlRestriction,
  resolveLtJoinSecret,
  isValidLtRoomId,
} from './protocol'
import { trackInfoToLtTrack, ltTrackToTrackInfo, toShareableQueueSnapshot } from './mapper'
import { trustedInboundStreamUrls } from '@/modules/playback/streamTrust'
import { queueReferences, applyListenTogetherQueueMutation } from './queue'
import { acceptRoomState, resolveExpectedPosition, resolvePositionSync, resolveSoftSyncRecheckAction, SOFT_SYNC_RECHECK_INTERVAL_MS, updateServerClockOffset } from './playbackSync'
import { isTerminalReconnectError } from './reconnect'
import { isMemberRequestSatisfied } from './watchdog'
import { createLogger } from '@/utils/logger'
import { createLtKeepalive } from './keepalive'
import { createLtOutbound } from './outbound'
import { createLtPlayerWatch } from './playerWatch'
import { createLtSession } from './session'

const log = createLogger('listen-together')

const LT_UUID_KEY = 'neri:lt-uuid'
const DEFAULT_BASE_URL = 'https://neriplayer.hancat.work'

const LINK_REQUEST_THROTTLE_MS = 4000

// 已处理转发请求 eventId 上限（对齐 Android ForwardedRequestDeduper 语义）
const HANDLED_FORWARDED_EVENT_LIMIT = 256
const HANDLED_FORWARDED_REQUESTER_LIMIT = 64
const PASSIVE_ROOM_UPDATE_TYPES = new Set([
  'HEARTBEAT', 'LINK_READY', 'LINK_UNAVAILABLE', 'MEMBER_JOINED', 'MEMBER_LEFT', 'WATCHDOG_REFRESH',
])

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
  let _softSyncTimer: ReturnType<typeof setInterval> | null = null
  let _trackSwitchAt = 0
  // 正在应答链接请求的曲目，同一首歌的重复请求只应答一次
  const _linkAnswers = new Set<string>()
  // 已处理的转发请求 eventId -> 处理时间，防重复送达二次执行
  const _handledForwardedEventIds = new Map<string, number>()
  const _lastForwardedSequence = new Map<string, number>()
  let _controlRequestChain: Promise<void> = Promise.resolve()
  let _unlistenMessage: UnlistenFn | null = null
  let _unlistenConnected: UnlistenFn | null = null
  let _unlistenDisconnected: UnlistenFn | null = null
  let _unlistenProtocolError: UnlistenFn | null = null
  let _lastAppliedSharedStreamSignature: string | null = null
  let _lastRequestedLinkStableKey: string | null = null
  let _lastRequestedLinkAt = 0
  // 通过别的服务器的邀请加入时只在本次会话使用那台服务器，不改写用户自己的服务器设置
  let _sessionBaseUrl: string | null = null
  const session = createLtSession()
  const activeBaseUrl = () => _sessionBaseUrl ?? baseUrl.value

  /** 时钟偏移按 Android 平滑：单向样本含网络延迟，往返样本取中点，往返超过 30s 的丢弃 */
  function sampleServerClock(serverNowMs: number | null | undefined, sentAtWallMs?: number, sentAtElapsedMs?: number) {
    session.serverClockOffsetMs = updateServerClockOffset({
      previousOffsetMs: session.serverClockOffsetMs,
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

  const outbound = createLtOutbound({
    session, roomId, userUuid, roomState, connectionState, isController, roomSettings,
    handleControlResult, commitRoomState, playbackIdentity,
  })
  const {
    hasPendingListenerQueueEvent, resetOutbound, replayOutbox, sendEvent, reportPlayEvent, reportPauseEvent,
    reportSeekEvent, reportPlaybackModeEvent, reportSetTrackEvent, completeQueueEvent, failRejectedQueueEvent,
    retryLegacyQueueSnapshot, reportQueueEvent, reportQueueReorder, reportTrackFinished,
  } = outbound
  const { setupPlayerWatch, teardownPlayerWatch } = createLtPlayerWatch({
    ...outbound, session, isController, roomSettings, playbackIdentity,
  })
  const {
    startWatchdog, stopWatchdog, reportHeartbeat, startHeartbeat, stopHeartbeat,
    startListenerPing, stopListenerPing, scheduleReconnect, cancelReconnect,
  } = createLtKeepalive({
    session, roomId, userUuid, nickname, roomState, connectionState, isController, roomSettings,
    activeBaseUrl, sampleServerClock, commitRoomState, applyRoomStateToPlayer, requestLinkForTrack, setSyncRate,
    closeRoomLocally, updateJoinSecret, resolveWsUrl, sendEvent, hasPendingListenerQueueEvent,
  })

  // 房间操作
  /** 创建房间 */
  async function createRoom() {
    const player = usePlayerStore()
    const toast = useToastStore()
    const t = i18n.global.t
    const generation = session.generation
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
      if (generation !== session.generation) return

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
        session.lastAppliedRoomVersion = resp.state.version || 0
        markSync('INITIAL_STATE', resp.state.updatedAt || Date.now())
      }

      // 连接 WebSocket
      const wsUrl = resolveWsUrl(resp, createdRoomId)
      await connectWs(wsUrl)
      if (generation !== session.generation) return

      startHeartbeat()
      setupPlayerWatch()
      // 自检只对听众生效；房主身份可能在会话中转交，所以建房时也启动
      startWatchdog()

    } catch (e) {
      if (generation !== session.generation) return
      const msg = e instanceof Error ? e.message : String(e)
      sessionError.value = msg
      connectionState.value = 'disconnected'
      liveRoomSettings.value = null
      toast.error(t('listen_together.create_failed', { msg }))
    }
  }

  /** 加入房间 */
  async function joinRoom(targetRoomId: string, joinSecret?: string, baseUrlOverride?: string) {
    const toast = useToastStore()
    const t = i18n.global.t
    const generation = session.generation
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
      if (generation !== session.generation) return

      if (!resp.ok) {
        throw new Error(resp.error || 'Join room failed')
      }

      roomId.value = normalizedRoomId
      updateJoinSecret(resp.joinSecret, joinSecret)
      role.value = (resp.role as LtRole) || 'listener'
      if (resp.state) {
        roomState.value = resp.state
        session.lastAppliedRoomVersion = resp.state.version || 0
        if (resp.state.settings) liveRoomSettings.value = { ...resp.state.settings }
        markSync('INITIAL_STATE', resp.state.updatedAt || Date.now())
        // 将服务端状态应用到本地播放器
        applyRoomStateToPlayer(resp.state, 'join')
      }

      const wsUrl = resolveWsUrl(resp, normalizedRoomId)
      await connectWs(wsUrl)
      if (generation !== session.generation) return

      startListenerPing()
      setupPlayerWatch()
      startWatchdog()

    } catch (e) {
      if (generation !== session.generation) return
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
    useToastStore().error(i18n.global.t('listen_together.room_closed'))
  }

  async function endSession(notifyServer: boolean) {
    // 递增会话代际，让在途的延迟同步回调失效
    session.generation++
    session.playbackApplySequence++
    session.pendingRemotePlaybackLoads.clear()
    stopHeartbeat()
    stopListenerPing()
    teardownPlayerWatch()
    teardownListeners()
    setSyncRate(null)
    if (session.watchReleaseTimer) clearTimeout(session.watchReleaseTimer)
    session.watchReleaseTimer = null
    session.suppressPlayerWatch = false
    cancelReconnect()

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
    } catch {
      // 连接可能早已断开，断开失败不影响本地收尾
    }

    roomId.value = null
    session.joinSecret = null
    role.value = null
    roomState.value = null
    liveRoomSettings.value = null
    session.lastAppliedRoomVersion = 0
    sessionError.value = null
    connectionState.value = 'disconnected'
    lastSyncEventType.value = null
    lastSyncAt.value = null
    lastReconnectAt.value = null
    session.wsUrl = null
    session.activeWsConnectionId = null
    session.reconnectAttempt = 0
    session.legacyPing = false
    resetOutbound()
    session.hostControlledOffline = false
    session.lastReportedTrackId = null
    _lastAppliedSharedStreamSignature = null
    session.lastReportedQueueKeys = []
    session.queueEventInFlight = null
    session.queuedQueueEvent = null
    session.lastReportedIsPlaying = null
    session.lastReportedRepeatMode = null
    session.lastReportedShuffle = null
    session.recentOutboundEventIds.clear()
    _handledForwardedEventIds.clear()
    _lastForwardedSequence.clear()
    session.serverClockOffsetMs = 0
    session.pingSentElapsed.clear()
    _sessionBaseUrl = null
    session.lastSocketMessageAt = 0
    session.pingSentAt = 0
    stopWatchdog()
    session.pendingMemberRequest = null
    session.stateRepairPending = false
    _lastRequestedLinkStableKey = null
    _lastRequestedLinkAt = 0
  }

  // WebSocket 连接
  async function connectWs(wsUrl: string) {
    const generation = session.generation
    session.wsUrl = wsUrl
    // 新连接建立期间不接受旧连接的收尾事件
    session.activeWsConnectionId = null
    if (!await setupListeners(generation) || generation !== session.generation) return
    await invoke('lt_connect_ws', { wsUrl })
  }

  async function setupListeners(generation: number): Promise<boolean> {
    teardownListeners()

    const unlistenMessage = await listen<ListenTogetherSocketEnvelope>('lt:message', (event) => {
      if (generation !== session.generation) return
      if (!roomId.value || (event.payload.connectionId && event.payload.connectionId !== session.activeWsConnectionId)) return
      handleSocketMessage(event.payload)
    })
    if (generation !== session.generation) { unlistenMessage(); return false }
    _unlistenMessage = unlistenMessage

    const unlistenConnected = await listen<{ connectionId?: string }>('lt:connected', (event) => {
      if (generation !== session.generation) return
      session.activeWsConnectionId = event.payload.connectionId || null
      _lastAppliedSharedStreamSignature = null
      const reconnected = session.reconnectAttempt > 0
      connectionState.value = 'connected'
      if (reconnected) {
        lastReconnectAt.value = Date.now()
        markSync('RECONNECTED', lastReconnectAt.value)
      }
      session.reconnectAttempt = 0
      // 重连后恢复 listener 存活探测
      if (roomId.value) {
        startListenerPing()
        if (isController.value) startHeartbeat()
        replayOutbox()
      }
    })
    if (generation !== session.generation) { unlistenConnected(); return false }
    _unlistenConnected = unlistenConnected

    const unlistenDisconnected = await listen<{
      connectionId?: string
      code: number
      reason: string
    }>('lt:disconnected', (event) => {
      if (generation !== session.generation) return
      // 新连接已经接管时，旧连接的 close/error 事件不能触发重连风暴。
      // 旧版后端没有 connectionId 时，仅在当前也没有代际信息时兼容接受。
      if (
        session.activeWsConnectionId !== null
        && event.payload.connectionId !== session.activeWsConnectionId
      ) return
      if (session.activeWsConnectionId === null && connectionState.value !== 'connected') return
      const wasConnected = connectionState.value === 'connected'
      connectionState.value = 'disconnected'
      session.activeWsConnectionId = null
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
    if (generation !== session.generation) { unlistenDisconnected(); return false }
    _unlistenDisconnected = unlistenDisconnected

    // 解析不了的消息：记下原因，漏掉的多半是房态更新，交给自检尽快补拉
    const unlistenProtocolError = await listen<{ connectionId?: string; message?: string }>('lt:protocol_error', (event) => {
      if (generation !== session.generation) return
      if (event.payload.connectionId && event.payload.connectionId !== session.activeWsConnectionId) return
      sessionError.value = event.payload.message || 'Protocol error'
      session.stateRepairPending = true
    })
    if (generation !== session.generation) { unlistenProtocolError(); return false }
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
    session.lastSocketMessageAt = Date.now()
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
          const generation = session.generation
          _controlRequestChain = _controlRequestChain.then(async () => {
            if (generation === session.generation) await handleMemberControlRequested(envelope, generation)
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
        const sentAtElapsed = typeof sentAt === 'number' ? session.pingSentElapsed.get(sentAt) : undefined
        if (typeof sentAt === 'number') session.pingSentElapsed.delete(sentAt)
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
    const t = i18n.global.t

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
    const queueAcknowledged = !!appliedCause?.eventId && appliedCause.eventId === session.queueEventInFlight?.eventId
    if (
      applied?.state
      && (queueAcknowledged || (appliedCause?.userUuid === userUuid.value
        && (appliedType === 'UPDATE_SETTINGS' || appliedType?.startsWith('REQUEST_'))))
    ) {
      commitRoomState(applied.state, appliedType || 'CONTROL_APPLIED', applied.expectedPositionMs, !isController.value && !session.queuedQueueEvent)
    }
    if (applied?.state && appliedCause?.eventId && applied.state.roomId === roomId.value
      && applied.state.version >= session.lastAppliedRoomVersion) completeQueueEvent(appliedCause.eventId)
  }

  function handleErrorEnvelope(envelope: ListenTogetherSocketEnvelope) {
    const t = i18n.global.t
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
      session.legacyPing = true
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
        !(isController.value && session.hostControlledOffline))
    }
    if (isController.value) startHeartbeat()
    else stopHeartbeat()
  }

  function commitRoomState(state: ListenTogetherRoomState, causeType: string, expectedPositionMs?: number, apply = true) {
    const accepted = acceptRoomState(roomState.value, state, roomId.value, session.lastAppliedRoomVersion)
    if (!accepted) return
    roomState.value = accepted
    session.lastAppliedRoomVersion = accepted.version
    if (session.pendingMemberRequest
      && isMemberRequestSatisfied(session.pendingMemberRequest.event, accepted, Date.now() + session.serverClockOffsetMs)) {
      session.pendingMemberRequest = null
    }
    if (accepted.settings) liveRoomSettings.value = { ...accepted.settings }
    markSync(causeType, accepted.updatedAt || Date.now())
    // 请求确认前的心跳仍描述旧队列，不能撤回听众刚选中的歌曲
    if (apply && !(hasPendingListenerQueueEvent() && PASSIVE_ROOM_UPDATE_TYPES.has(causeType))) {
      applyRoomStateToPlayer(accepted, causeType, expectedPositionMs)
    }
  }

  function handleRoomStateUpdated(envelope: ListenTogetherSocketEnvelope) {
    if (!envelope.state) return
    if ((envelope.state.version || 0) < session.lastAppliedRoomVersion) return
    if (session.queuedQueueEvent && envelope.causedBy?.eventId === session.queueEventInFlight?.eventId) {
      commitRoomState(envelope.state, envelope.causedBy?.type || 'STATE_SYNC', envelope.expectedPositionMs, false)
      completeQueueEvent(envelope.causedBy?.eventId)
      return
    }

    // 回声抑制：REQUEST_*/TRACK_FINISHED 引发的房态由权威方仲裁, 即便携带本端 eventId
    // 也必须应用到播放器（对齐 Android shouldIgnoreListenTogetherIncomingState:
    // REQUEST_ 前缀与 TRACK_FINISHED 永不忽略）, 否则服务端对位置/状态的钳制修正会被
    // 本地乐观值覆盖, 要等下一次心跳才纠正
    const causeType = envelope.causedBy?.type
    const authoritativeQueueChanged = !session.queuedQueueEvent && (causeType === 'SET_QUEUE' || causeType === 'REQUEST_SET_QUEUE')
      && envelope.state.version > session.lastAppliedRoomVersion
      && (envelope.state.currentIndex !== roomState.value?.currentIndex
        || envelope.state.queue.map(track => track.stableKey).join('\n') !== roomState.value?.queue.map(track => track.stableKey).join('\n'))
    const causeNeverSuppressed = !!causeType
      && (causeType.startsWith('REQUEST_') || causeType === 'TRACK_FINISHED')
    if (
      !causeNeverSuppressed && !authoritativeQueueChanged
      && envelope.causedBy?.eventId
      && session.recentOutboundEventIds.has(envelope.causedBy.eventId)
    ) {
      session.recentOutboundEventIds.delete(envelope.causedBy.eventId)
      // 仍然更新 roomState 但不应用到 player
      commitRoomState(envelope.state, causeType || 'STATE_SYNC', envelope.expectedPositionMs, false)
      completeQueueEvent(envelope.causedBy?.eventId)
      return
    }
    // 已应用的本端 eventId 从抑制集移除, 避免累积
    if (envelope.causedBy?.eventId) {
      session.recentOutboundEventIds.delete(envelope.causedBy.eventId)
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
    void answerLinkRequest(requestedKey, session.generation)
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
      const stillCurrent = () => generation === session.generation && isController.value
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

    const applySequence = session.playbackApplySequence
    const seekSequence = player.lastSeekCommand.seq
    const requestStillCurrent = () => generation === session.generation
      && applySequence === session.playbackApplySequence
      && !(player.lastSeekCommand.source === 'local' && !player.lastSeekCommand.fromPlaybackStart
        && player.lastSeekCommand.seq > seekSequence)

    session.suppressPlayerWatch = true
    try {
      switch (causeType) {
        case 'REQUEST_PLAY':
          if (!player.isPlaying) await player.resume('remote_sync')
          if (!requestStillCurrent()) return
          reportPlayEvent()
          break
        case 'REQUEST_PAUSE':
          if (player.isPlaying) await player.pause('remote_sync')
          if (!requestStillCurrent()) return
          reportPauseEvent()
          break
        case 'REQUEST_SEEK':
          if (envelope.positionMs != null) {
            await player.seekTo(envelope.positionMs, 'remote_sync')
            if (!requestStillCurrent()) return
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
            if (!requestStillCurrent()) return
            if (envelope.shouldPlay === false) await player.pause('remote_sync')
            if (!requestStillCurrent()) return
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
            if (!requestStillCurrent()) return
          }
          if (!track || !shouldPlay) await player.pause('remote_sync')
          else if (!player.isPlaying) await player.resume('remote_sync')
          if (!requestStillCurrent()) return
          if (track && envelope.positionMs != null && Math.abs(player.positionMs - requestedPosition) > 800) {
            await player.seekTo(requestedPosition, 'remote_sync')
            if (!requestStillCurrent()) return
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
      if (generation === session.generation) releasePlayerWatch(350)
    }
  }

  /** 房主暂时离线：只记下房间状态，不动播放器；听众看到一条普通提示，房主自己不需要（对齐 Android） */
  function handleRoomSuspended(envelope: ListenTogetherSocketEnvelope) {
    markSync('ROOM_SUSPENDED')
    if (envelope.state) commitRoomState(envelope.state, 'ROOM_SUSPENDED', envelope.expectedPositionMs, false)
    setSyncRate(null)
    if (!isController.value) useToastStore().show(i18n.global.t('listen_together.controller_offline'))
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
    if (session.watchReleaseTimer) clearTimeout(session.watchReleaseTimer)
    const generation = session.generation
    session.watchReleaseTimer = setTimeout(() => {
      session.watchReleaseTimer = null
      if (generation === session.generation) session.suppressPlayerWatch = false
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
    session.lastReportedQueueKeys = queue.map(track => track.stableKey)
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
    if (session.softSyncRate !== rate) usePlayerStore().setListenTogetherSyncPlaybackRate(rate)
    session.softSyncRate = rate
    if (rate === null) {
      if (_softSyncTimer) clearInterval(_softSyncTimer)
      _softSyncTimer = null
      return
    }
    if (_softSyncTimer) return
    _softSyncTimer = setInterval(() => {
      const player = usePlayerStore()
      const state = roomState.value
      const expectedPositionMs = state ? resolveExpectedPosition(state, Date.now() + session.serverClockOffsetMs) : 0
      const currentKey = player.currentTrack ? trackInfoToLtTrack(player.currentTrack).stableKey : null
      const roomKey = state?.track?.stableKey ?? state?.queue[state.currentIndex]?.stableKey
      const action = resolveSoftSyncRecheckAction({
        currentRate: session.softSyncRate ?? 1,
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
    if (state.roomId !== roomId.value || state.version < session.lastAppliedRoomVersion) return
    const player = usePlayerStore()
    const queue = [...state.queue]
    const index = state.currentIndex
    if (state.track && index >= 0 && index < queue.length && queue[index].stableKey === state.track.stableKey) {
      queue[index] = state.track
    }
    if (queue.length === 0 && state.track && index >= 0) queue.push(state.track)
    const targetIndex = queue.length === 1 && state.queue.length === 0 ? 0 : index
    const effectiveLtTrack = queue[targetIndex]
    session.suppressPlayerWatch = true
    try {
      if (!effectiveLtTrack) {
        _lastAppliedSharedStreamSignature = null
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
      const expectedPos = resolveExpectedPosition(state, Date.now() + session.serverClockOffsetMs, expectedPositionMs)
      const remoteTrack = ltTrackToTrackInfo(effectiveLtTrack, preferredStreamQuality(effectiveLtTrack.channelId))
      if (!isController.value && roomSettings.value.shareAudioLinks && !remoteTrack.audioUrl) {
        requestLinkForTrack(effectiveLtTrack, targetIndex)
      }
      const sharedStreamUrls = trustedInboundStreamUrls(effectiveLtTrack.channelId,
        effectiveLtTrack.streamUrls, effectiveLtTrack.streamUrl)
      const sharedStreamSignature = JSON.stringify(sharedStreamUrls.slice().sort())
      const playbackUnavailable = !player.hasPlaybackSession && !player.isLoadingAudio
        && session.pendingRemotePlaybackLoads.size === 0
      // 保留已成功的备用音源；失败会话需要重载，进行中的加载仍等待原请求完成
      const streamChanged = !!remoteTrack.audioUrl
        && (playbackUnavailable || (!sharedStreamUrls.includes(player.getCurrentStreamUrl(remoteTrack.id) || '')
          && sharedStreamSignature !== _lastAppliedSharedStreamSignature))
      _lastAppliedSharedStreamSignature = sharedStreamSignature
      replacePlayerQueue(queue, targetIndex)
      if (playbackContextChanged || streamChanged || forceReload) {
        _trackSwitchAt = Date.now()
        setSyncRate(null)
        const generation = session.generation
        const applySequence = ++session.playbackApplySequence
        const seekSequence = player.lastSeekCommand.seq
        session.pendingRemotePlaybackLoads.set(applySequence, seekSequence)
        // 播放完成后读取最新房态，避免固定延时在慢速解析中暂停错误的会话
        void player.play(player.queue[targetIndex], 'remote_sync', expectedPos).then(async () => {
          if (generation !== session.generation || applySequence !== session.playbackApplySequence) return
          const latest = roomState.value
          const latestKey = latest?.track?.stableKey ?? latest?.queue[latest.currentIndex]?.stableKey
          if (!latest || latestKey !== effectiveLtTrack.stableKey) return
          const latestPosition = resolveExpectedPosition(latest, Date.now() + session.serverClockOffsetMs)
          const newerLocalSeek = player.lastSeekCommand.source === 'local'
            && !player.lastSeekCommand.fromPlaybackStart
            && player.lastSeekCommand.seq > seekSequence
          if (!newerLocalSeek && Math.abs(latestPosition - player.positionMs) > 500) {
            await player.seekTo(latestPosition, 'remote_sync')
            if (generation !== session.generation || applySequence !== session.playbackApplySequence) return
          }
          if (latest.playback.state !== 'playing') await player.pause('remote_sync')
          else if (!player.isPlaying) await player.resume('remote_sync')
        }).catch((error) => log.warn('remote playback sync failed:', error)).finally(() => {
          session.pendingRemotePlaybackLoads.delete(applySequence)
          if (generation === session.generation) releasePlayerWatch(500)
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
      session.lastReportedTrackId = targetIdentity
      session.lastReportedIsPlaying = remoteIsPlaying
      player.applyListenTogetherPlaybackMode({ repeatMode: state.playback.repeatMode, shuffleEnabled: state.playback.shuffleEnabled })
      session.lastReportedRepeatMode = state.playback.repeatMode ?? desktopRepeatToWire(player.repeatMode)
      session.lastReportedShuffle = state.playback.shuffleEnabled ?? !!player.shuffleEnabled
    } finally {
      releasePlayerWatch(500)
    }
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
    if (session.joinSecret) params.set('secret', session.joinSecret)
    return `neriplayer://listen-together/join?${params.toString()}`
  }

  async function copyInviteLink() {
    const toast = useToastStore()
    const t = i18n.global.t
    // 服务端要求邀请带密钥，没有密钥的链接别人用了只会被拒绝
    if (!session.joinSecret) {
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
    } catch {
      // 写剪贴板失败（权限或平台限制）时不打断用户，房间号仍可手动分享
    }
  }

  /** 检测剪贴板中的邀请链接 */
  async function checkClipboardInvite(): Promise<LtInvite | null> {
    try {
      const invite = parseLtInvite(await readText())
      if (!invite) return null
      if (invite.hasInvalidBaseUrl) log.warn('ignored an invite server that is not https')
      return invite
    } catch {
      // 读不到剪贴板时按没有邀请处理
    }
    return null
  }

  // 工具函数
  function updateJoinSecret(
    value: string | null | undefined,
    fallback?: string | null,
  ) {
    session.joinSecret = resolveLtJoinSecret(value, fallback) || null
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
    reportQueueReorder,
    // 曲目自然播完上报（供 player.handleTrackEnded 调用）
    reportTrackFinished,
  }
})