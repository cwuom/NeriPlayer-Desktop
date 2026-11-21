import type { ListenTogetherEvent } from './protocol'
import type { PendingMemberRequest } from './watchdog'

/** 一次房间会话里由多个模块共同读写的状态，随 store 创建；离开房间时由 endSession 复位 */
export interface LtSession {
  // 会话代际：leaveRoom 后递增，让在途的延迟回调失效，不再操作播放器
  generation: number
  playbackApplySequence: number
  pendingRemotePlaybackLoads: Map<number, number>
  watchReleaseTimer: ReturnType<typeof setTimeout> | null
  softSyncRate: number | null
  reconnectAttempt: number
  // 服务端回过 "unsupported event type: np_ping" 后改发旧版 ping
  legacyPing: boolean
  // 房主断线期间自己改过播放：重连时的 welcome/房态只更新房间，不能把这些改动撤回去
  hostControlledOffline: boolean
  // 半开连接检测（对齐 Android SocketHealthOwner）：最后一次收到任何消息、最后一次发 ping 的时间
  lastSocketMessageAt: number
  pingSentAt: number
  // 收到过解析不了的消息：漏掉的很可能是房态更新，下一次自检时补拉
  stateRepairPending: boolean
  pendingMemberRequest: PendingMemberRequest | null
  wsUrl: string | null
  // 后端为每条 WS 分配代际 ID，迟到的旧连接事件必须丢弃（MK-02）
  activeWsConnectionId: string | null
  suppressPlayerWatch: boolean
  lastAppliedRoomVersion: number
  // 回环抑制：避免自己发出的事件触发回环
  recentOutboundEventIds: Set<string>
  // 记录最后上报的 track id，避免重复上报
  lastReportedTrackId: string | null
  lastReportedQueueKeys: string[]
  queueEventInFlight: ListenTogetherEvent | null
  queuedQueueEvent: ListenTogetherEvent | null
  lastReportedIsPlaying: boolean | null
  lastReportedRepeatMode: number | null
  lastReportedShuffle: boolean | null
  // 服务器时钟偏移估计（对齐 Android estimatedServerClockOffsetMs）：
  // 期望播放位置须用服务器时钟推算，直接用本机 Date.now() 会因两端时钟差恒定偏移
  serverClockOffsetMs: number
  // np_ping 的发送时间 -> 发送时的单调时钟，用来算往返时间
  pingSentElapsed: Map<number, number>
  joinSecret: string | null
}

export function createLtSession(): LtSession {
  return {
    generation: 0,
    playbackApplySequence: 0,
    pendingRemotePlaybackLoads: new Map<number, number>(),
    watchReleaseTimer: null,
    softSyncRate: null,
    reconnectAttempt: 0,
    legacyPing: false,
    hostControlledOffline: false,
    lastSocketMessageAt: 0,
    pingSentAt: 0,
    stateRepairPending: false,
    pendingMemberRequest: null,
    wsUrl: null,
    activeWsConnectionId: null,
    suppressPlayerWatch: false,
    lastAppliedRoomVersion: 0,
    recentOutboundEventIds: new Set<string>(),
    lastReportedTrackId: null,
    lastReportedQueueKeys: [],
    queueEventInFlight: null,
    queuedQueueEvent: null,
    lastReportedIsPlaying: null,
    lastReportedRepeatMode: null,
    lastReportedShuffle: null,
    serverClockOffsetMs: 0,
    pingSentElapsed: new Map<number, number>(),
    joinSecret: null,
  }
}
