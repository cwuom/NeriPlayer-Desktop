import { invoke } from '@tauri-apps/api/core'
import type { ComputedRef, Ref } from 'vue'
import { usePlayerStore } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import i18n from '@/i18n'
import { createLogger } from '@/utils/logger'
import {
  desktopRepeatToWire,
  type ConnectionState,
  type ListenTogetherControlResponse,
  type ListenTogetherEvent,
  type ListenTogetherRoomSettings,
  type ListenTogetherRoomState,
  type ListenTogetherSocketEnvelope,
  type ListenTogetherTrack,
} from './protocol'
import { toShareableQueueSnapshot, trackInfoToLtTrack } from './mapper'
import {
  buildListenTogetherQueueMutationPlan,
  getLtQueueReference,
  queueReferences,
  setLtQueueReference,
} from './queue'
import { resolveExpectedPosition } from './playbackSync'
import { MEMBER_REQUEST_TTL_MS, TRACKED_MEMBER_REQUESTS } from './watchdog'
import type { LtSession } from './session'

const log = createLogger('listen-together')

const CONTROL_EVENT_DEDUP_MS = 350
const SEEK_EVENT_DEDUP_MS = 800
const SEEK_EVENT_MIN_DELTA_MS = 300
const LOCAL_SEEK_REPORT_DEBOUNCE_MS = 450

export interface LtOutboundContext {
  session: LtSession
  roomId: Ref<string | null>
  userUuid: Ref<string>
  roomState: Ref<ListenTogetherRoomState | null>
  connectionState: Ref<ConnectionState>
  isController: ComputedRef<boolean>
  roomSettings: ComputedRef<ListenTogetherRoomSettings>
  handleControlResult: (envelope: ListenTogetherSocketEnvelope) => void
  commitRoomState: (state: ListenTogetherRoomState, causeType: string, expectedPositionMs?: number, apply?: boolean) => void
  playbackIdentity: () => string | null
}

/**
 * 出站事件：控制与成员请求的发送（WebSocket 失败走 HTTP，再失败留待重连补发）、
 * 去重节流、队列事件的确认与排队，以及事件携带的曲目绑定快照
 */
export function createLtOutbound(ctx: LtOutboundContext) {
  const {
    session, roomId, userUuid, roomState, connectionState, isController, roomSettings,
    handleControlResult, commitRoomState, playbackIdentity,
  } = ctx

  // WebSocket 和 HTTP 都发不出去的控制（对齐 Android ListenTogetherControlOutbox）：
  // 按意图只留最新一条，最多 8 条，重连后重放；队列类保存完整快照，重放时不依赖旧的版本基线
  const _outbox = new Map<string, { event: ListenTogetherEvent; queueSnapshot: boolean }>()
  // 出站事件排序字段：实例标识会话内生成一次，序号单调递增（对齐 Android EventFactory）
  const _clientInstanceId = crypto.randomUUID()
  let _clientSequence = 0
  let _queueEventSnapshot: ListenTogetherEvent | null = null
  let _queueEventPendingSince = 0
  let _queueAckTimer: ReturnType<typeof setTimeout> | null = null
  let _lastSentControlType: string | null = null
  let _lastSentControlAt = 0
  let _lastSentSeekPosition: number | null = null
  let _lastSentSeekAt = 0
  let _pendingSeekReport: { positionMs: number; trackId: string | null } | null = null
  let _pendingSeekTimer: ReturnType<typeof setTimeout> | null = null

  /** 离开房间时清空待补发的控制、在途队列事件和去重记录 */
  function resetOutbound() {
    _outbox.clear()
    _queueEventSnapshot = null
    _queueEventPendingSince = 0
    if (_queueAckTimer) clearTimeout(_queueAckTimer)
    _queueAckTimer = null
    _lastSentControlType = null
    _lastSentControlAt = 0
    _lastSentSeekPosition = null
    _lastSentSeekAt = 0
    clearPendingSeekReport()
  }

  function hasPendingListenerQueueEvent(): boolean {
    return !isController.value && !!(session.queueEventInFlight || session.queuedQueueEvent)
      && Date.now() - _queueEventPendingSince <= MEMBER_REQUEST_TTL_MS
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
      session.hostControlledOffline = true
    }
    const generation = session.generation
    if (!event.eventId) event.eventId = generateEventId()
    if (!event.clientTimeMs) event.clientTimeMs = Date.now()
    // 乱序保护字段（协议已声明、Android 每事件必带）：实例标识 + 单调递增序号
    if (!event.clientInstanceId) event.clientInstanceId = _clientInstanceId
    if (event.clientSequence == null) event.clientSequence = ++_clientSequence

    session.recentOutboundEventIds.add(event.eventId!)
    // 清理过期 ID（保留最近 50 个）
    if (session.recentOutboundEventIds.size > 50) {
      const iter = session.recentOutboundEventIds.values()
      session.recentOutboundEventIds.delete(iter.next().value!)
    }

    let delivered = false
    try {
      delivered = await invoke<boolean>('lt_send_event', { event })
    } catch (e) {
      log.warn('WebSocket event send failed:', e)
    }
    if (delivered || generation !== session.generation) return
    await sendHttpFallback(event, generation)
  }

  async function sendHttpFallback(event: ListenTogetherEvent, generation: number) {
    if (generation !== session.generation) return
    stripUnsharedAudioLinks(event)
    if (session.queueEventInFlight?.eventId === event.eventId && _queueAckTimer) {
      clearTimeout(_queueAckTimer)
      _queueAckTimer = null
    }
    try {
      const result = await invoke<ListenTogetherControlResponse>('lt_send_control', { event })
      if (generation !== session.generation) return
      const queueEvent = session.queueEventInFlight?.eventId === event.eventId
      handleControlResult({ type: 'control_result', result, causedBy: { eventId: event.eventId, type: event.type } })
      if (result.ok && result.applied?.state) {
        if (!queueEvent || session.queueEventInFlight?.eventId === event.eventId) {
          // 房主自己的控制已经在本地生效，提交结果只更新房间状态，不再反过来驱动房主的播放器
          commitRoomState(result.applied.state, result.applied.causedBy?.type || event.type,
            result.applied.expectedPositionMs, !isController.value && !queueEvent && !session.queuedQueueEvent)
          if (result.applied.state.roomId === roomId.value && result.applied.state.version >= session.lastAppliedRoomVersion) {
            completeQueueEvent(event.eventId)
          }
        }
      }
    } catch (error) {
      if (generation !== session.generation) return
      log.warn('HTTP event send failed:', error)
      // 两条链路都不通（离线）：留到重连后补发。队列取最新的完整快照，排在后面的修改也不会丢
      const queueEvent = session.queueEventInFlight?.eventId === event.eventId
      const replay = queueEvent ? session.queuedQueueEvent ?? _queueEventSnapshot : event
      const kept = replay ? rememberForReplay(replay, queueEvent) : false
      completeQueueEvent(event.eventId, false)
      if (!kept) useToastStore().error(i18n.global.t('listen_together.control_not_sent'))
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
    session.lastReportedRepeatMode = repeatMode
    session.lastReportedShuffle = shuffleEnabled
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

  function reportSetTrackEvent(_track: ListenTogetherTrack, _currentIndex: number) {
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
    _queueEventPendingSince = Date.now()
    if (session.queueEventInFlight) {
      session.queuedQueueEvent = snapshot
      return
    }
    if (!forceSnapshot && roomState.value && snapshot.queue
      && (snapshot.type === 'SET_TRACK' || snapshot.type === 'REQUEST_SET_TRACK')) {
      const plan = buildListenTogetherQueueMutationPlan(roomState.value, snapshot.queue, snapshot.currentIndex ?? -1)
      if (plan.needsCurrentSelectionAfterApply && roomState.value.schemaVersion >= 2) {
        // 新重复项确认入队后才有可引用的 occurrence，先提交队列再选择该项
        session.queuedQueueEvent = snapshot
        snapshot = { ...snapshot, type: snapshot.type === 'SET_TRACK' ? 'SET_QUEUE' : 'REQUEST_SET_QUEUE' }
      }
    }
    const event: ListenTogetherEvent = {
      ...snapshot,
      queue: undefined,
      ...(forceSnapshot ? { queue: snapshot.queue } : queueEventFields(snapshot.queue ?? [], snapshot.currentIndex ?? -1)),
      eventId: generateEventId(),
    }
    session.queueEventInFlight = event
    _queueEventSnapshot = snapshot
    const generation = session.generation
    _queueAckTimer = setTimeout(() => {
      _queueAckTimer = null
      if (generation === session.generation && session.queueEventInFlight?.eventId === event.eventId) {
        void sendHttpFallback(event, generation)
      }
    }, 3000)
    void sendEvent(event)
  }

  function completeQueueEvent(eventId?: string, accepted = true) {
    if (!eventId || session.queueEventInFlight?.eventId !== eventId) return
    if (_queueAckTimer) clearTimeout(_queueAckTimer)
    _queueAckTimer = null
    session.queueEventInFlight = null
    _queueEventPendingSince = 0
    const previousSnapshot = _queueEventSnapshot
    _queueEventSnapshot = null
    if (accepted && roomState.value) refreshLocalQueueReferences(previousSnapshot, roomState.value)
    const queued = session.queuedQueueEvent
    session.queuedQueueEvent = null
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
    const target = ackId ?? (isQueueCompatibilityError(error) ? session.queueEventInFlight?.eventId : undefined)
    if (target && session.queueEventInFlight?.eventId === target) completeQueueEvent(target, false)
  }

  function retryLegacyQueueSnapshot(error: string): boolean {
    if (!isQueueCompatibilityError(error) || !session.queueEventInFlight?.queueMutation || !_queueEventSnapshot) return false
    const snapshot = _queueEventSnapshot
    if (_queueAckTimer) clearTimeout(_queueAckTimer)
    _queueAckTimer = null
    session.queueEventInFlight = null
    _queueEventSnapshot = null
    sendQueueEvent(snapshot, true)
    return true
  }

  function reportQueueEvent(playback?: { positionMs: number; shouldPlay: boolean }) {
    const player = usePlayerStore()
    const snapshot = buildControlSnapshotFields()
    const shouldPlay = playback?.shouldPlay ?? player.isPlaying
    session.lastReportedQueueKeys = player.queue.map(track => trackInfoToLtTrack(track).stableKey)
    sendQueueEvent({
      type: isController.value ? 'SET_QUEUE' : 'REQUEST_SET_QUEUE',
      queue: snapshot.queue,
      track: snapshot.track,
      currentIndex: snapshot.currentIndex,
      requestTrackStableKey: snapshot.requestTrackStableKey,
      positionMs: playback?.positionMs ?? player.positionMs,
      shouldPlay,
      state: shouldPlay ? 'playing' : 'paused',
      repeatMode: desktopRepeatToWire(player.repeatMode),
      shuffleEnabled: !!player.shuffleEnabled,
    })
  }

  function reportQueueReorder() {
    if (!roomId.value) return
    // 重排可能改变同一歌曲的 occurrence，只同步队列，避免被 watcher 当成切歌
    session.lastReportedTrackId = playbackIdentity()
    const player = usePlayerStore()
    const state = roomState.value
    // 远程曲目仍在加载时，界面的临时暂停不能覆盖房间希望保持的播放状态
    const loadingRemoteState = session.pendingRemotePlaybackLoads.size > 0 && state
    const loadSeekSequence = session.pendingRemotePlaybackLoads.get(session.playbackApplySequence)
    const hasLocalSeek = loadSeekSequence !== undefined
      && player.lastSeekCommand.seq > loadSeekSequence
      && player.lastSeekCommand.source === 'local'
      && !player.lastSeekCommand.fromPlaybackStart
    reportQueueEvent({
      positionMs: loadingRemoteState && player.isLoadingAudio && !hasLocalSeek
        ? resolveExpectedPosition(state, Date.now() + session.serverClockOffsetMs)
        : player.positionMs,
      shouldPlay: loadingRemoteState ? state.playback.state === 'playing' : intendsToPlay(),
    })
  }

  /// 曲目自然播完: 上报 TRACK_FINISHED 交权威方仲裁切歌, 本地不自行推进
  /// （对齐 Android ListenTogetherEventFactory.buildTrackFinishedEvent:
  /// listener 的 TRACK_FINISHED 不带 currentIndex/track/queue）
  function reportTrackFinished(finishedTrackId: string | null) {
    const player = usePlayerStore()
    // 暂停期间可能已收到下一首的房态，迟到的结束通知只属于原曲目
    if (!finishedTrackId || player.currentTrack?.id !== finishedTrackId) return
    const finishedLtTrack = player.currentTrack ? trackInfoToLtTrack(player.currentTrack) : null
    if (!finishedLtTrack || finishedLtTrack.channelId === 'local' || finishedLtTrack.channelId === 'qqMusic') return
    sendEvent({
      type: 'TRACK_FINISHED',
      finishedTrackStableKey: finishedLtTrack.stableKey,
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
      session.pendingMemberRequest = { event, createdAt: now, lastSentAt: now, attempts: 1 }
    }
    void sendEvent(event)
  }

  return {
    hasPendingListenerQueueEvent, resetOutbound, replayOutbox, sendEvent, shouldSkipControlEvent,
    reportPlayEvent, reportPauseEvent, reportSeekEvent, reportPlaybackModeEvent, scheduleLocalSeekReport,
    clearPendingSeekReport, reportSetTrackEvent, intendsToPlay, completeQueueEvent, failRejectedQueueEvent,
    retryLegacyQueueSnapshot, reportQueueEvent, reportQueueReorder, reportTrackFinished, sendRequestEvent,
  }
}
