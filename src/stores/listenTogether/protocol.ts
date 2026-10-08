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
/** local_safety：睡眠定时、失败跳过、自动推进等内部操作，不受一起听的成员控制限制（对齐 Android LOCAL_SAFETY） */
export type PlaybackCommandSource = 'local' | 'local_safety' | 'remote_sync'

export type LocalRoomControlRestriction = 'controller_offline' | 'member_control_disabled'

/** 听众在房主离线或关闭了成员控制时不能控制播放（对齐 Android resolveLocalRoomControlRestriction） */
export function resolveLocalRoomControlRestriction(
  roomStatus: string | null | undefined,
  allowMemberControl: boolean | null | undefined,
  isController: boolean,
): LocalRoomControlRestriction | null {
  if (isController) return null
  if (roomStatus === 'controller_offline') return 'controller_offline'
  if (allowMemberControl === false) return 'member_control_disabled'
  return null
}

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

// 与服务端 worker.js 相同：任意汉字（Unicode Han 文字，含 〇、々 与各扩展区）、ASCII 字母和数字，按码点计数
const NICKNAME_REGEX = new RegExp(`^[\\p{Script=Han}A-Za-z0-9]{1,${LT_NICKNAME_MAX_LENGTH}}$`, 'u')

/** 返回 true 表示昵称合法（对齐服务端，避免建房/加入被拒或云同步到 Android 端被 sanitize 丢弃） */
export function isValidLtNickname(nickname: string): boolean {
  return NICKNAME_REGEX.test(nickname.trim())
}

// 不匹配更长单词中间的片段；调试版客户端生成 neriplayer-debug:// 邀请（对齐 Android ListenTogetherInviteParser）
const INVITE_REGEX = /(?<![a-z0-9+.-])neriplayer(?:-debug)?:\/\/listen-together\/join\?[^\s]+/i

export interface LtInvite {
  roomId: string
  joinSecret: string
  baseUrl?: string
  /** 邀请人昵称，只用于展示；不合法的昵称直接丢弃 */
  inviter?: string
  /** 文本里匹配到的邀请链接原文 */
  link: string
  /** 邀请带了服务器地址但不是合法的 https 地址，已被忽略 */
  hasInvalidBaseUrl: boolean
}

/** 从任意文本里解析邀请链接；参数顺序无关。没有密钥的邀请服务端会拒绝，直接视为无效 */
export function parseLtInvite(text: string | null | undefined): LtInvite | null {
  const match = text?.match(INVITE_REGEX)
  if (!match) return null
  const params = new URLSearchParams(match[0].slice(match[0].indexOf('?') + 1))
  const roomId = normalizeLtRoomId(params.get('roomId') ?? '')
  if (!isValidLtRoomId(roomId)) return null
  const joinSecret = normalizeLtJoinSecret(params.get('secret'))
  if (!joinSecret) return null
  const rawBaseUrl = params.get('baseUrl')?.trim()
  const baseUrl = normalizeLtInviteBaseUrl(rawBaseUrl)
  const inviter = params.get('inviter')?.trim() ?? ''
  return {
    roomId,
    joinSecret,
    baseUrl: baseUrl ?? undefined,
    ...(isValidLtNickname(inviter) ? { inviter } : {}),
    link: match[0],
    hasInvalidBaseUrl: !!rawBaseUrl && !baseUrl,
  }
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
