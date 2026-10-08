// 歌词偏移量模型：对齐 Android LyricDefaultOffset.kt / LyricOffsetDefaults.kt
//
// 两层语义:
//   - 各歌词来源的默认偏移: 设置里的五个基线（网易云、QQ 音乐、酷狗、LRCLIB、AMLL TTML）
//   - 逐曲用户偏移(delta): 默认 0, 手动微调后才非 0, 与 Android SongItem.userLyricOffsetMs 同义
// 有效偏移 = 当前歌词来源的默认偏移 + 逐曲 delta。界面上显示和编辑的都是有效偏移（绝对值），
// 存储与同步的仍是 delta，双端数据一致。
//
// 默认偏移按「当前显示的歌词实际来自哪里」选：网易云曲目显示的是 AMLL TTML 逐字歌词时用
// TTML 的默认。歌词来源未知（旧缓存）时按播放来源推断：网易云、QQ 用各自默认，其余为 0。
// YouTube 原生歌词、本地歌词文件和手动编辑的歌词没有可调的默认，按 0 计。

export const LYRIC_OFFSET_SOURCES = ['netease', 'qq', 'kugou', 'lrclib', 'amll_ttml'] as const
export type LyricOffsetSource = typeof LYRIC_OFFSET_SOURCES[number]

// 与 Android DEFAULT_*_LYRIC_OFFSET_MS 一致
export const DEFAULT_LYRIC_OFFSET_MS: Readonly<Record<LyricOffsetSource, number>> = {
  netease: 1000,
  qq: 500,
  kugou: 0,
  lrclib: 0,
  amll_ttml: 0,
}

// 与 Android MIN/MAX_LYRIC_DEFAULT_OFFSET_MS、LYRIC_DEFAULT_OFFSET_STEP_MS 一致
export const MIN_LYRIC_DEFAULT_OFFSET_MS = -5000
export const MAX_LYRIC_DEFAULT_OFFSET_MS = 5000
export const LYRIC_OFFSET_STEP_MS = 50

// 逐曲 delta 的安全存储边界, 防止脏数据写入本地映射
export const MIN_LYRIC_OFFSET_MS = -30000
export const MAX_LYRIC_OFFSET_MS = 30000

/** 默认偏移按 50 ms 对齐并夹到 ±5000 ms（Android normalizeLyricDefaultOffsetMs）；非数值回到 fallback */
export function normalizeLyricDefaultOffsetMs(value: number, fallback = 0): number {
  if (!Number.isFinite(value)) return fallback
  const aligned = Math.round(value / LYRIC_OFFSET_STEP_MS) * LYRIC_OFFSET_STEP_MS
  return Math.min(MAX_LYRIC_DEFAULT_OFFSET_MS, Math.max(MIN_LYRIC_DEFAULT_OFFSET_MS, aligned)) || 0
}

/// 把各处记下的歌词来源归一：后端 snake_case、Android MusicPlatform（CLOUD_MUSIC）、
/// LyricSourcePreference（AmllTtml）以及桌面端旧写法（LRCLIB、LOCAL_EDIT）
/// 返回 'none' 表示来源已知但没有可调默认；null 表示不知道
export function normalizeLyricSource(value: unknown): LyricOffsetSource | 'none' | null {
  if (typeof value !== 'string') return null
  const key = value.toLowerCase().replace(/[^a-z0-9]/g, '')
  switch (key) {
    case 'netease':
    case 'cloudmusic':
      return 'netease'
    case 'qq':
    case 'qqmusic':
      return 'qq'
    case 'kugou':
      return 'kugou'
    case 'lrclib':
      return 'lrclib'
    case 'amllttml':
    case 'ttml':
      return 'amll_ttml'
    case 'youtube':
    case 'local':
    case 'localedit':
      return 'none'
    default:
      return null
  }
}

/** 有效偏移该用哪个来源的默认；null 表示没有默认（按 0 计） */
export function resolveLyricOffsetSource(
  lyricSource: unknown,
  playbackSource?: string | null,
): LyricOffsetSource | null {
  const normalized = normalizeLyricSource(lyricSource)
  if (normalized === 'none') return null
  if (normalized) return normalized
  if (playbackSource === 'netease') return 'netease'
  if (playbackSource === 'qq') return 'qq'
  return null
}

/** 某来源的默认偏移；没有默认的来源恒为 0 */
export function resolveLyricDefaultOffsetMs(
  source: LyricOffsetSource | null,
  defaults: Readonly<Record<LyricOffsetSource, number>>,
): number {
  return source ? defaults[source] : 0
}

// 改动系统默认偏移时, 对已手动调过的歌曲重算 delta 以保持"绝对时序"不变
// 绝对时序 = 旧默认 + 旧delta = 新默认 + 新delta => 新delta = 旧delta + 旧默认 - 新默认
export function rebaseLyricUserOffsetMs(
  userOffsetMs: number,
  previousDefaultOffsetMs: number,
  newDefaultOffsetMs: number,
): number {
  return userOffsetMs + previousDefaultOffsetMs - newDefaultOffsetMs
}

// 仅对"来源匹配且确实调过(delta != 0)"的歌曲 rebase; 未调过的跟随新默认即可
export function shouldRebaseLyricOffset(
  songSource: LyricOffsetSource | null,
  targetSource: LyricOffsetSource,
  userOffsetMs: number,
): boolean {
  return userOffsetMs !== 0 && songSource === targetSource
}

// 写入本地前的防御性归一: 非有限值归零, 取整并夹到安全边界
export function clampLyricOffsetMs(value: number): number {
  if (!Number.isFinite(value)) return 0
  const rounded = Math.round(value)
  return Math.min(MAX_LYRIC_OFFSET_MS, Math.max(MIN_LYRIC_OFFSET_MS, rounded))
}

// 逐曲偏移的稳定键: 优先曲目 id(如 netease:123, 同步回来的曲目也会重建成同一 id), 兜底 playlistKey
export function lyricUserOffsetStorageKey(
  track: { id?: string | null; playlistKey?: string | null } | null | undefined,
): string {
  if (!track) return ''
  const id = (track.id ?? '').trim()
  if (id) return id
  return (track.playlistKey ?? '').trim()
}

// 读取同步载荷里 Android 写下的逐曲偏移(delta); 缺失或非法则 0
// 兼容 camelCase / snake_case 两种字段名
export function readSyncedUserOffsetMs(
  track: { syncPayload?: Record<string, unknown> | null } | null | undefined,
): number {
  const payload = track?.syncPayload
  if (!payload || typeof payload !== 'object') return 0
  const raw = payload.userLyricOffsetMs ?? payload.user_lyric_offset_ms
  const value = typeof raw === 'number' ? raw : Number(raw)
  return Number.isFinite(value) ? clampLyricOffsetMs(value) : 0
}

/** 同步载荷里记下的歌词来源（Android matchedLyricSource） */
export function readSyncedLyricSource(
  payload: Record<string, unknown> | null | undefined,
): string | null {
  const raw = payload?.matchedLyricSource ?? payload?.matched_lyric_source
  return typeof raw === 'string' && raw.trim() ? raw : null
}

/** 把逐曲偏移写回 syncPayload (对齐 Android SongItem.userLyricOffsetMs) */
export function withUpdatedUserOffsetPayload(
  payload: Record<string, unknown> | undefined | null,
  userOffsetMs: number,
): Record<string, unknown> {
  const base: Record<string, unknown> = { ...(payload || {}) }
  const delta = clampLyricOffsetMs(userOffsetMs)
  base.userLyricOffsetMs = delta
  delete base.user_lyric_offset_ms
  // 标记 CURRENT, 避免被 legacy fill-missing 覆盖
  const version = Number(base.syncMetadataVersion ?? base.sync_metadata_version ?? 0)
  if (!Number.isFinite(version) || version < 1) {
    base.syncMetadataVersion = 1
    delete base.sync_metadata_version
  }
  return base
}

/** 带正负号的偏移文本，如 +1000ms、-250ms、0ms */
export function formatLyricOffsetMs(value: number): string {
  return `${value > 0 ? '+' : ''}${value}ms`
}
