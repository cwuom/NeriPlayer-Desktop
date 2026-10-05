/**
 * 一起听协议 DTO 类型定义
 * 与安卓端 ListenTogether protocol 包完全对齐
 */

export const LtChannels = {
  NETEASE: 'netease',
  // 保留用于桌面本地映射，但跨端共享队列会排除该频道
  QQ_MUSIC: 'qqMusic',
  BILIBILI: 'bilibili',
  YOUTUBE_MUSIC: 'youtubeMusic',
  LOCAL: 'local',
} as const

/** ExoPlayer-aligned repeat integers used on the wire */
export const LtRepeatMode = {
  OFF: 0,
  ONE: 1,
  ALL: 2,
} as const

export type LtRepeatModeValue = (typeof LtRepeatMode)[keyof typeof LtRepeatMode]

export interface ListenTogetherTrack {
  stableKey: string
  channelId: string
  audioId: string
  subAudioId?: string
  playlistContextId?: string
  mediaUri?: string
  streamUrl?: string
  streamUrls?: string[]
  name: string
  artist: string
  album?: string
  durationMs: number
  coverUrl?: string
}

export interface ListenTogetherRoomSettings {
  allowMemberControl: boolean
  autoPauseOnMemberChange: boolean
  shareAudioLinks: boolean
}

export interface ListenTogetherMember {
  userUuid: string
  nickname: string
  userId?: string
  role: string
  joinedAt: number
}

export interface ListenTogetherPlaybackState {
  state: 'playing' | 'paused' | string
  basePositionMs: number
  baseTimestampMs: number
  playbackRate: number
  /** ExoPlayer: 0=OFF 1=ONE 2=ALL */
  repeatMode?: number | null
  shuffleEnabled?: boolean | null
}

export interface ListenTogetherRoomState {
  roomId: string
  version: number
  schemaVersion: number
  controllerUserUuid?: string
  controllerUserId?: string
  controllerHeartbeatAt?: number
  settings: ListenTogetherRoomSettings
  members: ListenTogetherMember[]
  queue: ListenTogetherTrack[]
  currentIndex: number
  track?: ListenTogetherTrack
  playback: ListenTogetherPlaybackState
  controllerOfflineSince?: number
  roomStatus: string
  closedReason?: string
  updatedAt: number
}

export interface ListenTogetherCause {
  userUuid?: string
  userId?: string
  nickname?: string
  eventId?: string
  type?: string
}

export interface ListenTogetherQueueReference {
  stableKey: string
  occurrence: number
}

export interface ListenTogetherQueueOperation {
  type: string
  target?: ListenTogetherQueueReference | null
  anchor?: ListenTogetherQueueReference | null
  placement?: string | null
  track?: ListenTogetherTrack | null
  order?: ListenTogetherQueueReference[] | null
}

export interface ListenTogetherQueueMutation {
  baseRoomVersion: number
  operations: ListenTogetherQueueOperation[]
  targetCurrent?: ListenTogetherQueueReference | null
}

export interface ListenTogetherEvent {
  type: string
  eventId?: string
  clientTimeMs?: number
  clientInstanceId?: string
  clientSequence?: number
  positionMs?: number
  currentIndex?: number
  nextIndex?: number
  track?: ListenTogetherTrack
  queue?: ListenTogetherTrack[]
  roomSettings?: ListenTogetherRoomSettings
  shouldPlay?: boolean
  state?: string
  /** PLAYBACK_MODE / REQUEST_PLAYBACK_MODE */
  repeatMode?: number
  shuffleEnabled?: boolean
  queueMutation?: ListenTogetherQueueMutation
  requestTrackStableKey?: string
  forceRefresh?: boolean
  finishedTrackStableKey?: string
}

export interface ListenTogetherSocketEnvelope {
  connectionId?: string
  type: string
  sessionId?: string
  userUuid?: string
  userId?: string
  nickname?: string
  role?: string
  autoPauseOnJoin?: boolean
  state?: ListenTogetherRoomState
  expectedPositionMs?: number
  nowMs?: number
  t?: number
  ok?: boolean
  /// 控制命令应答：服务端拒绝时才有 error，缺它会让被拒绝的控制静默生效
  /// applied.state/causedBy 对齐 Android ListenTogetherAppliedEvent：
  /// 自己发起的 REQUEST_* 被仲裁通过后要据此把权威状态应用到本地播放器
  result?: {
    ok: boolean
    error?: string
    applied?: {
      type: string
      roomId?: string
      version?: number
      state?: ListenTogetherRoomState
      expectedPositionMs?: number
      nowMs?: number
      causedBy?: ListenTogetherCause
    }
  }
  message?: string
  roomId?: string
  version?: number
  causedBy?: ListenTogetherCause
  track?: ListenTogetherTrack
  queue?: ListenTogetherTrack[]
  positionMs?: number
  currentIndex?: number
  requestTrackStableKey?: string
  shouldPlay?: boolean
  stateName?: string
  repeatMode?: number
  shuffleEnabled?: boolean
  queueMutation?: ListenTogetherQueueMutation
  clientTimeMs?: number
  clientInstanceId?: string
  clientSequence?: number
  requestSequence?: number
}

export interface ListenTogetherInitialSnapshot {
  queue: ListenTogetherTrack[]
  currentIndex: number
  track?: ListenTogetherTrack
  settings: ListenTogetherRoomSettings
  isPlaying: boolean
  positionMs: number
  repeatMode: number
  shuffleEnabled: boolean
  shuffleRestoreQueue?: ListenTogetherTrack[]
}

export interface ListenTogetherControlResponse {
  ok: boolean
  error?: string
  applied?: NonNullable<ListenTogetherSocketEnvelope['result']>['applied']
}

export interface ListenTogetherRoomResponse {
  ok: boolean
  roomId?: string
  userUuid?: string
  userId?: string
  nickname?: string
  role?: string
  memberSecret?: string
  joinSecret?: string
  autoPauseOnJoin?: boolean
  token?: string
  state?: ListenTogetherRoomState
  wsUrl?: string
  error?: string
}

export interface ListenTogetherStateResponse {
  ok: boolean
  state?: ListenTogetherRoomState
  expectedPositionMs?: number
  serverNowMs?: number
  autoPauseOnJoin?: boolean
  error?: string
}

export type ConnectionState = 'disconnected' | 'connecting' | 'connected'
export type LtRole = 'controller' | 'listener'

/** 播放命令来源 */
export type PlaybackCommandSource = 'local' | 'remote_sync'

/** Desktop string mode <-> wire int (ExoPlayer) */
export function desktopRepeatToWire(mode: string | undefined | null): number {
  switch (mode) {
    case 'one':
      return LtRepeatMode.ONE
    case 'all':
      return LtRepeatMode.ALL
    default:
      return LtRepeatMode.OFF
  }
}

export function wireRepeatToDesktop(mode: number | null | undefined): 'off' | 'one' | 'all' | null {
  if (mode === null || mode === undefined || Number.isNaN(mode)) return null
  if (mode === LtRepeatMode.ONE) return 'one'
  if (mode === LtRepeatMode.ALL) return 'all'
  if (mode === LtRepeatMode.OFF) return 'off'
  return null
}

// 昵称/房间号校验（对齐 Android ListenTogetherNicknameValidation / RoomIdValidation）
// Android 昵称白名单仅数字/大小写字母/汉字, 长度 1..24, 不含连字符
export const LT_NICKNAME_MAX_LENGTH = 24
const ROOM_ID_REGEX = /^[ABCDEFGHJKLMNPQRSTUVWXYZ23456789]{6}$/

function isAllowedNicknameChar(cp: number): boolean {
  if (cp >= 0x30 && cp <= 0x39) return true // 0-9
  if (cp >= 0x41 && cp <= 0x5a) return true // A-Z
  if (cp >= 0x61 && cp <= 0x7a) return true // a-z
  // CJK 统一表意文字（含扩展 A/B 常见区段），近似 Android HAN script 判定
  if (cp >= 0x4e00 && cp <= 0x9fff) return true
  if (cp >= 0x3400 && cp <= 0x4dbf) return true
  if (cp >= 0x20000 && cp <= 0x2a6df) return true
  return false
}

/** 返回 true 表示昵称合法（对齐 Android，避免云同步到 Android 端被 sanitize 丢弃） */
export function isValidLtNickname(nickname: string): boolean {
  const normalized = nickname.trim()
  if (normalized.length < 1 || normalized.length > LT_NICKNAME_MAX_LENGTH) return false
  for (const ch of normalized) {
    const cp = ch.codePointAt(0)
    if (cp === undefined || !isAllowedNicknameChar(cp)) return false
  }
  return true
}

/** roomId 归一化：去空白并大写（对齐 Android normalizeListenTogetherRoomId） */
export function normalizeLtRoomId(value: string): string {
  return value.trim().toUpperCase()
}

/** 返回 true 表示房间号合法 */
export function isValidLtRoomId(roomId: string): boolean {
  return ROOM_ID_REGEX.test(normalizeLtRoomId(roomId))
}

export function normalizeLtHttpBaseUrl(value: string | null | undefined): string | null {
  const candidate = value?.trim().replace(/\/+$/, '')
  if (!candidate) return null
  try {
    const parsed = new URL(candidate)
    const protocol = parsed.protocol.toLowerCase()
    if ((protocol !== 'http:' && protocol !== 'https:') || !parsed.host || parsed.search || parsed.hash) {
      return null
    }
    const path = parsed.pathname.replace(/\/+$/, '')
    return `${protocol}//${parsed.host}${path === '/' ? '' : path}`
  } catch {
    return null
  }
}

export function normalizeLtInviteBaseUrl(value: string | null | undefined): string | null {
  const normalized = normalizeLtHttpBaseUrl(value)
  return normalized?.startsWith('https://') ? normalized : null
}

export function normalizeLtJoinSecret(value: string | null | undefined): string | undefined {
  const normalized = value?.trim()
  return normalized && normalized.length <= 256 ? normalized : undefined
}

export function resolveLtJoinSecret(
  value: string | null | undefined,
  fallback: string | null | undefined = undefined,
): string | undefined {
  return normalizeLtJoinSecret(value) ?? normalizeLtJoinSecret(fallback)
}
