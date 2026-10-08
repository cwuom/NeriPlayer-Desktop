// 前端用户状态（播放队列、播放历史、逐曲歌词偏移）的数据库持久化
//
// 启动时在创建 store 之前一次性拉取快照，store 仍可同步恢复；旧版 localStorage
// 数据按数据域导入一次。写入经串行队列发出，先发出的命令一定先落库
import { invoke } from '@tauri-apps/api/core'

export const LEGACY_PLAYER_STATE_KEY = 'neri:player-state'
export const LEGACY_HISTORY_KEY = 'neri:play-history'
export const LEGACY_HISTORY_DELETIONS_KEY = 'neri:play-history-deletions'
export const LEGACY_LYRIC_OFFSETS_KEY = 'neri.lyric-user-offsets'

export interface PersistedHistory {
  entries: Array<{ track: unknown; playedAt: number }>
  deletions: Array<{ track: unknown; deletedAt: number }>
}

export interface UserDataSnapshot {
  playbackState: Record<string, unknown> | null
  history: PersistedHistory
  lyricOffsets: Record<string, number>
  migrated: { playbackState: boolean; history: boolean; lyricOffsets: boolean }
}

export interface LegacyUserData {
  playbackState: Record<string, unknown> | null
  history: PersistedHistory | null
  lyricOffsets: Record<string, number> | null
}

type StorageLike = Pick<Storage, 'getItem' | 'removeItem'>

let snapshot: UserDataSnapshot | null = null
let legacyPlayerStateCleanupPending = false
let writeChain: Promise<unknown> = Promise.resolve()

function parseJson(storage: StorageLike, key: string): unknown {
  const raw = storage.getItem(key)
  if (raw === null) return undefined
  try {
    return JSON.parse(raw)
  } catch {
    return null
  }
}

function finiteInteger(value: unknown): number | undefined {
  const number = typeof value === 'number' ? value : Number.NaN
  return Number.isFinite(number) ? Math.round(number) : undefined
}

/**
 * 旧快照可能缺少 hasPlaybackSession：按恢复逻辑的旧版推断提前定型，
 * 数据库里只存明确的布尔值，恢复结果与直接读取旧快照一致
 */
export function normalizeLegacyPlaybackState(raw: unknown): Record<string, unknown> | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const state = raw as Record<string, unknown>
  const queue = Array.isArray(state.queue) ? state.queue : []
  const queueIndex = finiteInteger(state.queueIndex)
  const currentTrackId = typeof state.currentTrackId === 'string' && state.currentTrackId ? state.currentTrackId : undefined
  const currentTrackPlaylistKey = typeof state.currentTrackPlaylistKey === 'string' && state.currentTrackPlaylistKey
    ? state.currentTrackPlaylistKey
    : undefined
  let hasPlaybackSession: boolean
  if (typeof state.hasPlaybackSession === 'boolean') {
    hasPlaybackSession = state.hasPlaybackSession
  } else {
    const legacyWithoutSession = !currentTrackId && !currentTrackPlaylistKey
      && typeof state.queueIndex === 'number' && state.queueIndex < 0
    hasPlaybackSession = !legacyWithoutSession
  }
  const normalized: Record<string, unknown> = {
    queue,
    queueIndex: queueIndex ?? -1,
    hasPlaybackSession,
  }
  if (currentTrackId) normalized.currentTrackId = currentTrackId
  if (currentTrackPlaylistKey) normalized.currentTrackPlaylistKey = currentTrackPlaylistKey
  if (typeof state.volume === 'number' && Number.isFinite(state.volume)) normalized.volume = state.volume
  if (typeof state.positionMs === 'number' && Number.isFinite(state.positionMs)) normalized.positionMs = state.positionMs
  if (typeof state.repeatMode === 'string') normalized.repeatMode = state.repeatMode
  if (typeof state.shuffleEnabled === 'boolean') normalized.shuffleEnabled = state.shuffleEnabled
  return normalized
}

function normalizeLegacyHistory(rawEntries: unknown, rawDeletions: unknown): PersistedHistory | null {
  if (rawEntries === undefined && rawDeletions === undefined) return null
  const entries = (Array.isArray(rawEntries) ? rawEntries : []).flatMap((entry) => {
    const item = entry as { track?: unknown; playedAt?: unknown; played_at?: unknown } | null
    const playedAt = finiteInteger(item?.playedAt ?? item?.played_at)
    return item?.track && playedAt && playedAt > 0 ? [{ track: item.track, playedAt }] : []
  })
  const deletions = (Array.isArray(rawDeletions) ? rawDeletions : []).flatMap((deletion) => {
    const item = deletion as { track?: unknown; deletedAt?: unknown; deleted_at?: unknown } | null
    const deletedAt = finiteInteger(item?.deletedAt ?? item?.deleted_at)
    return item?.track && deletedAt && deletedAt > 0 ? [{ track: item.track, deletedAt }] : []
  })
  return { entries, deletions }
}

function normalizeLegacyOffsets(raw: unknown): Record<string, number> | null {
  if (raw === undefined) return null
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {}
  const offsets: Record<string, number> = {}
  for (const [key, value] of Object.entries(raw as Record<string, unknown>)) {
    const offset = finiteInteger(typeof value === 'number' ? value : Number(value))
    if (key && offset) offsets[key] = offset
  }
  return offsets
}

/** 读取旧版 localStorage 中的用户状态（只读，不删除） */
export function readLegacyUserData(storage: StorageLike): LegacyUserData {
  return {
    playbackState: normalizeLegacyPlaybackState(parseJson(storage, LEGACY_PLAYER_STATE_KEY)),
    history: normalizeLegacyHistory(
      parseJson(storage, LEGACY_HISTORY_KEY),
      parseJson(storage, LEGACY_HISTORY_DELETIONS_KEY),
    ),
    lyricOffsets: normalizeLegacyOffsets(parseJson(storage, LEGACY_LYRIC_OFFSETS_KEY)),
  }
}

function needsLegacyImport(migrated: UserDataSnapshot['migrated']): boolean {
  return !migrated.playbackState || !migrated.history || !migrated.lyricOffsets
}

/**
 * 拉取数据库快照并完成旧数据迁移；没有 Rust 后端（浏览器开发模式）时返回 false，
 * 各 store 退回 localStorage
 */
export async function preloadUserData(storage: StorageLike | undefined = globalThis.localStorage): Promise<boolean> {
  try {
    let loaded = await invoke<UserDataSnapshot>('load_user_data_snapshot')
    if (storage && needsLegacyImport(loaded.migrated)) {
      const legacy = readLegacyUserData(storage)
      loaded = await invoke<UserDataSnapshot>('import_legacy_user_data', {
        playbackState: loaded.migrated.playbackState ? null : legacy.playbackState,
        history: loaded.migrated.history ? null : legacy.history,
        lyricOffsets: loaded.migrated.lyricOffsets ? null : legacy.lyricOffsets,
      })
    }
    if (storage) {
      // 设置迁移可能还要读取旧播放器快照里的音量，它在第一次成功写库后再删除
      if (loaded.migrated.history) {
        storage.removeItem(LEGACY_HISTORY_KEY)
        storage.removeItem(LEGACY_HISTORY_DELETIONS_KEY)
      }
      if (loaded.migrated.lyricOffsets) storage.removeItem(LEGACY_LYRIC_OFFSETS_KEY)
      legacyPlayerStateCleanupPending = loaded.migrated.playbackState
        && storage.getItem(LEGACY_PLAYER_STATE_KEY) !== null
    }
    snapshot = loaded
    return true
  } catch (error) {
    console.warn('[user-data] database unavailable, falling back to localStorage:', error)
    snapshot = null
    return false
  }
}

export function preloadedUserData(): UserDataSnapshot | null {
  return snapshot
}

export function hasUserDataBackend(): boolean {
  return snapshot !== null
}

/** 第一次成功写入播放队列后调用，删除已迁移的旧快照 */
export function finishLegacyPlayerStateCleanup(storage: StorageLike | undefined = globalThis.localStorage) {
  if (!legacyPlayerStateCleanupPending || !storage) return
  legacyPlayerStateCleanupPending = false
  storage.removeItem(LEGACY_PLAYER_STATE_KEY)
}

/** 串行发出持久化命令：异步命令在后端可能并发执行，这里保证落库顺序与调用顺序一致 */
export function persistUserData<T>(command: string, args: Record<string, unknown>): Promise<T> {
  const run = writeChain.then(() => invoke<T>(command, args))
  writeChain = run.catch(() => undefined)
  return run
}
