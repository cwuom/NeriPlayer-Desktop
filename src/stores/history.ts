import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { TrackInfo } from './player'
import {
  LEGACY_HISTORY_DELETIONS_KEY,
  LEGACY_HISTORY_KEY,
  persistUserData,
  preloadedUserData,
} from '@/modules/persistence/userData'
import { createLogger } from '@/utils/logger'

const log = createLogger('history')

export interface PlayedEntry {
  track: TrackInfo
  playedAt: number
  /** 长音频续播位置；undefined 表示本机还不知道（升级前的条目），同步时沿用存档里的值 */
  resumePositionMs?: number
}

export interface HistoryDeletion {
  track: TrackInfo
  deletedAt: number
}

interface BackendHistoryEntry {
  track?: Record<string, unknown>
  playedAt?: number
  played_at?: number
  resumePositionMs?: number | null
  resume_position_ms?: number | null
}

function resumePosition(raw: any): number | undefined {
  const value = raw?.resumePositionMs ?? raw?.resume_position_ms
  return typeof value === 'number' && Number.isFinite(value) ? Math.max(0, Math.round(value)) : undefined
}

interface BackendHistoryDeletion {
  songId?: string | number
  album?: string
  mediaUri?: string | null
  deletedAt?: number
  deleted_at?: number
}

export type HistorySyncApplyOutcome = 'applied' | 'deferred' | 'skipped'

const STORAGE_KEY = LEGACY_HISTORY_KEY
const DELETIONS_STORAGE_KEY = LEGACY_HISTORY_DELETIONS_KEY
export const HISTORY_CHANGED_EVENT = 'neri:history-changed'

/**
 * 历史条目的身份：曲目 id；同一个 B 站视频的不同分 P 再按 cid 区分（取流选分 P 的同一规则）。
 * 必须与 Rust `play_history::identity_key` 一致
 */
export function historyEntryKey(track: Pick<TrackInfo, 'id' | 'album' | 'syncPayload'>): string {
  if (!track.id.startsWith('bilibili:')) return track.id
  const cid = bilibiliPage(track)
  return cid ? `${track.id}#${cid}` : track.id
}

function bilibiliPage(track: Pick<TrackInfo, 'album' | 'syncPayload'>): string | undefined {
  for (const key of ['subAudioId', 'sub_audio_id']) {
    const value = track.syncPayload?.[key]
    if (typeof value === 'string' && value.trim()) return value.trim()
    if (typeof value === 'number' && Number.isFinite(value)) return String(value)
  }
  return track.album?.match(/^Bilibili\|(\d+)/i)?.[1]
}

/** 按身份去重，保留先出现的一条（调用方先按时间从新到旧排好） */
function uniqueByIdentity<T extends { track: TrackInfo }>(items: T[]): T[] {
  const seen = new Set<string>()
  return items.filter((item) => {
    const key = historyEntryKey(item.track)
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

function normalizeTrack(raw: any): TrackInfo {
  return {
    id: String(raw?.id ?? ''),
    title: String(raw?.title ?? ''),
    artist: String(raw?.artist ?? ''),
    album: String(raw?.album ?? ''),
    durationMs: Number(raw?.durationMs ?? raw?.duration_ms ?? 0),
    coverUrl: String(raw?.coverUrl ?? raw?.cover_url ?? ''),
    audioUrl: String(raw?.audioUrl ?? raw?.audio_url ?? raw?.url ?? ''),
    source: raw?.source,
    addedAt: Number(raw?.addedAt ?? raw?.added_at ?? 0),
    syncPayload: raw?.syncPayload ?? raw?.sync_payload,
    playlistKey: raw?.playlistKey ?? raw?.playlist_key,
  }
}

function inferSource(track: TrackInfo): string {
  if (track.source) return track.source
  if (track.id.startsWith('netease:')) return 'netease'
  if (track.id.startsWith('qq:')) return 'qq'
  if (track.id.startsWith('bilibili:')) return 'bilibili'
  if (track.id.startsWith('youtube:')) return 'youtube'
  return 'local'
}

async function stableSyncId(value: string): Promise<string> {
  if (!globalThis.crypto?.subtle) return value
  const digest = await globalThis.crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(value),
  )
  const signed = new DataView(digest).getBigInt64(0)
  return String(signed === 0n ? 1n : signed)
}

async function syncIdentity(track: TrackInfo): Promise<{ songId: string; album: string; mediaUri: string }> {
  const source = inferSource(track)
  const rawId = track.id.replace(`${source}:`, '')

  if (source === 'netease') {
    return { songId: rawId, album: track.album, mediaUri: '' }
  }
  if (source === 'qq') {
    return {
      songId: await stableSyncId(`qq|${rawId}|`),
      album: track.album,
      mediaUri: '',
    }
  }
  if (source === 'bilibili') {
    const cid = track.album.match(/^Bilibili\|(.+)$/i)?.[1] ?? ''
    return {
      songId: await stableSyncId(`bilibili|${rawId}|${cid}`),
      album: track.album,
      mediaUri: '',
    }
  }
  if (source === 'youtube') {
    return {
      songId: await stableSyncId(rawId),
      album: track.album,
      mediaUri: `ytmusic://video/${encodeURIComponent(rawId)}`,
    }
  }
  return { songId: rawId, album: track.album, mediaUri: '' }
}

function toBackendTrack(track: TrackInfo) {
  return {
    id: track.id,
    title: track.title,
    artist: track.artist,
    album: track.album,
    duration_ms: Math.max(0, Math.round(track.durationMs || 0)),
    source: inferSource(track),
    url: track.audioUrl || '',
    cover_url: track.coverUrl || null,
    added_at: Math.max(0, Math.round(track.addedAt || 0)),
    sync_payload: track.syncPayload ?? null,
    playlist_key: track.playlistKey ?? null,
  }
}

function emitHistoryChanged(type: 'record' | 'progress' | 'remove' | 'clear' | 'sync') {
  if (typeof window === 'undefined') return
  window.dispatchEvent(new CustomEvent(HISTORY_CHANGED_EVENT, {
    detail: { type, at: Date.now() },
  }))
}

export const useHistoryStore = defineStore('history', () => {
  const entries = ref<PlayedEntry[]>([])
  const deletions = ref<HistoryDeletion[]>([])
  let mutationEpoch = 0
  // 只统计用户自己的播放/删除/清空，同步应用远端结果时据此判断快照之后有没有新的本地修改
  let localMutationEpoch = 0
  const database = preloadedUserData() !== null

  /** 数据库模式下每次修改只发出对应命令；浏览器开发模式退回整表写 localStorage */
  function persist(command: string, args: Record<string, unknown>) {
    if (!database) {
      try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(entries.value))
        localStorage.setItem(DELETIONS_STORAGE_KEY, JSON.stringify(deletions.value))
      } catch {
        // 存储失败忽略
      }
      return
    }
    void persistUserData(command, args).catch((error) => {
      log.error(`${command} failed:`, error)
    })
  }

  function load() {
    const preloaded = preloadedUserData()
    if (preloaded) {
      applyStored(preloaded.history.entries, preloaded.history.deletions)
      return
    }
    try {
      const rawEntries = localStorage.getItem(STORAGE_KEY)
      const rawDeletions = localStorage.getItem(DELETIONS_STORAGE_KEY)
      applyStored(rawEntries ? JSON.parse(rawEntries) : [], rawDeletions ? JSON.parse(rawDeletions) : [])
    } catch {
      entries.value = []
      deletions.value = []
    }
  }

  function applyStored(parsedEntries: unknown, parsedDeletions: unknown) {
    try {
      entries.value = Array.isArray(parsedEntries)
        ? uniqueByIdentity(parsedEntries
          .map((entry: any) => ({
            track: normalizeTrack(entry?.track),
            playedAt: Number(entry?.playedAt ?? entry?.played_at ?? 0),
            resumePositionMs: resumePosition(entry),
          }))
          .filter((entry: PlayedEntry) => entry.track.id && entry.playedAt > 0))
        : []
      deletions.value = Array.isArray(parsedDeletions)
        ? uniqueByIdentity(parsedDeletions
          .map((deletion: any) => ({
            track: normalizeTrack(deletion?.track),
            deletedAt: Number(deletion?.deletedAt ?? deletion?.deleted_at ?? 0),
          }))
          .filter((deletion: HistoryDeletion) => deletion.track.id && deletion.deletedAt > 0))
        : []
    } catch {
      entries.value = []
      deletions.value = []
    }
  }

  function markLocalMutation() {
    mutationEpoch++
    localMutationEpoch++
  }

  /** 记一次播放；已记住的续播位置保留（对齐 Android record 只合并曲目信息） */
  function record(track: TrackInfo) {
    markLocalMutation()
    const playedAt = Date.now()
    const key = historyEntryKey(track)
    const idx = entries.value.findIndex(entry => historyEntryKey(entry.track) === key)
    const resumePositionMs = idx >= 0 ? entries.value[idx].resumePositionMs : undefined
    if (idx >= 0) entries.value.splice(idx, 1)
    deletions.value = deletions.value.filter(deletion => historyEntryKey(deletion.track) !== key)
    entries.value.unshift({ track, playedAt, resumePositionMs })
    persist('record_play_history', { track, playedAt })
    emitHistoryChanged('record')
  }

  function rememberedPosition(track: TrackInfo): number {
    const key = historyEntryKey(track)
    return entries.value.find(entry => historyEntryKey(entry.track) === key)?.resumePositionMs ?? 0
  }

  /** 记下长音频的续播位置，条目移到最前（对齐 Android updateRememberedPlaybackPosition） */
  function updateResumePosition(track: TrackInfo, positionMs: number) {
    const resumePositionMs = Math.max(0, Math.round(positionMs))
    const now = Date.now()
    const key = historyEntryKey(track)
    const idx = entries.value.findIndex(entry => historyEntryKey(entry.track) === key)
    const existing = idx >= 0 ? entries.value[idx] : undefined
    if (existing && existing.playedAt > now) return
    if (!existing && resumePositionMs === 0) return
    if (existing?.resumePositionMs === resumePositionMs) return
    markLocalMutation()
    if (idx >= 0) entries.value.splice(idx, 1)
    deletions.value = deletions.value.filter(deletion => historyEntryKey(deletion.track) !== key)
    entries.value.unshift({ track, playedAt: now, resumePositionMs })
    persist('record_play_history', { track, playedAt: now, resumePositionMs })
    emitHistoryChanged('progress')
  }

  /** 按 historyEntryKey 删除单条 */
  function remove(identityKey: string) {
    const removed = entries.value.find(entry => historyEntryKey(entry.track) === identityKey)?.track
    if (!removed) return
    markLocalMutation()
    const deletedAt = Date.now()
    entries.value = entries.value.filter(entry => historyEntryKey(entry.track) !== identityKey)
    deletions.value = [
      { track: removed, deletedAt },
      ...deletions.value.filter(deletion => historyEntryKey(deletion.track) !== identityKey),
    ]
    persist('remove_play_history', { identityKey, deletedAt })
    emitHistoryChanged('remove')
  }

  function clear() {
    markLocalMutation()
    if (entries.value.length === 0) return
    const deletedAt = Date.now()
    const current = entries.value.map(entry => ({ track: entry.track, deletedAt }))
    const currentKeys = new Set(current.map(deletion => historyEntryKey(deletion.track)))
    deletions.value = [
      ...current,
      ...deletions.value.filter(deletion => !currentKeys.has(historyEntryKey(deletion.track))),
    ]
    entries.value = []
    persist('clear_play_history', { deletedAt })
    emitHistoryChanged('clear')
  }

  function getSyncSnapshot() {
    return {
      entries: entries.value.map(entry => ({
        track: toBackendTrack(entry.track),
        playedAt: entry.playedAt,
        resumePositionMs: entry.resumePositionMs ?? null,
      })),
      deletions: deletions.value.map(deletion => ({
        track: toBackendTrack(deletion.track),
        deletedAt: deletion.deletedAt,
      })),
    }
  }

  /** 与 getSyncSnapshot 同时取，交给 applySyncPayload 判断快照之后用户是否又改过历史 */
  function syncSnapshotEpoch() {
    return localMutationEpoch
  }

  /**
   * 应用远端合并结果。快照之后用户又改过历史时返回 'deferred'：这份结果不含那些修改，
   * 不能覆盖本地，调用方应补一轮同步把它们合并上去（对齐 Android 的延迟应用）
   */
  async function applySyncPayload(
    payload: any,
    isCurrent: () => boolean = () => true,
    snapshotEpoch?: number,
  ): Promise<HistorySyncApplyOutcome> {
    if (!payload || typeof payload !== 'object' || !isCurrent()) return 'skipped'
    const startedLocalEpoch = localMutationEpoch
    if (snapshotEpoch !== undefined && snapshotEpoch !== startedLocalEpoch) return 'deferred'
    const epoch = ++mutationEpoch
    const canApply = () => epoch === mutationEpoch && isCurrent()
    // 被更新的同步结果取代时直接放弃；被用户修改打断时要求补同步
    const interrupted = (): HistorySyncApplyOutcome =>
      isCurrent() && localMutationEpoch !== startedLocalEpoch ? 'deferred' : 'skipped'
    const previousEntries = entries.value.map(entry => entry.track)
    const previousDeletions = deletions.value.map(deletion => deletion.track)
    const rawEntries: BackendHistoryEntry[] = Array.isArray(payload.entries) ? payload.entries : []
    const rawDeletions: BackendHistoryDeletion[] = Array.isArray(payload.deletions) ? payload.deletions : []
    const candidates = [...previousEntries, ...previousDeletions]
    const resolvedDeletions: HistoryDeletion[] = []

    for (const rawDeletion of rawDeletions) {
      const candidate = await findMatchingTrack(candidates, rawDeletion, canApply)
      if (!canApply()) return interrupted()
      if (candidate) {
        resolvedDeletions.push({
          track: candidate,
          deletedAt: Number(rawDeletion.deletedAt ?? rawDeletion.deleted_at ?? 0),
        })
      }
    }

    const nextEntries: PlayedEntry[] = rawEntries
      .map(entry => ({
        track: normalizeTrack(entry.track),
        playedAt: Number(entry.playedAt ?? entry.played_at ?? 0),
        resumePositionMs: resumePosition(entry),
      }))
      .filter(entry => entry.track.id && entry.playedAt > 0)
      .sort((left, right) => right.playedAt - left.playedAt)

    // 提交阶段没有异步等待，先复核账号意图和本地修改再一起保存
    if (!canApply()) return interrupted()
    mutationEpoch++
    // 合并结果按同步身份区分，同一曲目可能因专辑名不同出现多条，本地只留最新的一条
    entries.value = uniqueByIdentity(nextEntries)
    deletions.value = uniqueByIdentity(resolvedDeletions
      .filter(deletion => deletion.track.id && deletion.deletedAt > 0)
      .sort((left, right) => right.deletedAt - left.deletedAt))
    persist('replace_play_history', {
      history: {
        entries: entries.value.map(entry => ({
          track: entry.track,
          playedAt: Math.round(entry.playedAt),
          resumePositionMs: entry.resumePositionMs ?? null,
        })),
        deletions: deletions.value.map(deletion => ({
          track: deletion.track,
          deletedAt: Math.round(deletion.deletedAt),
        })),
      },
    })
    emitHistoryChanged('sync')
    return 'applied'
  }

  async function findMatchingTrack(
    candidates: TrackInfo[],
    deletion: BackendHistoryDeletion,
    isCurrent: () => boolean,
  ): Promise<TrackInfo | undefined> {
    const expectedSongId = String(deletion.songId ?? '')
    const expectedAlbum = String(deletion.album ?? '')
    const expectedMediaUri = String(deletion.mediaUri ?? '')
    for (const candidate of candidates) {
      const identity = await syncIdentity(candidate)
      if (!isCurrent()) return undefined
      if (
        identity.songId === expectedSongId &&
        (identity.album === expectedAlbum || !expectedAlbum) &&
        (identity.mediaUri === expectedMediaUri || !expectedMediaUri)
      ) return candidate
    }
    return undefined
  }

  load()

  return {
    entries,
    deletions,
    record,
    rememberedPosition,
    updateResumePosition,
    remove,
    clear,
    getSyncSnapshot,
    syncSnapshotEpoch,
    applySyncPayload,
  }
})
