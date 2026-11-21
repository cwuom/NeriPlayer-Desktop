import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { TrackInfo } from './player'
import {
  LEGACY_HISTORY_DELETIONS_KEY,
  LEGACY_HISTORY_KEY,
  persistUserData,
  preloadedUserData,
} from '@/modules/persistence/userData'
import {
  buildBiliPartTracks,
  readBiliPartNameOrder,
  resolveBiliPartCid,
  resolveBiliVideoReference,
  type BiliVideoInfo,
} from '@/modules/playback/biliVideoParts'
import { normalizeCoverUrlForDisplay } from '@/utils/bilibiliCoverCache'
import { asArray, asRecord, asString, dig } from '@/utils/json'
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

function resumePosition(raw: unknown): number | undefined {
  const value = dig(raw, 'resumePositionMs') ?? dig(raw, 'resume_position_ms')
  return typeof value === 'number' && Number.isFinite(value) ? Math.max(0, Math.round(value)) : undefined
}

interface BackendHistoryDeletion {
  songId?: string | number
  album?: string
  mediaUri?: string | null
  deletedAt?: number
  deleted_at?: number
}

/** sync_github / sync_webdav 返回的 SyncResult.history（Rust sync::manager 组装的合并结果） */
export interface HistorySyncPayload {
  entries?: BackendHistoryEntry[]
  deletions?: BackendHistoryDeletion[]
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

type HistoryMetadata = Pick<TrackInfo, 'title' | 'artist' | 'coverUrl' | 'durationMs'>

function hasHistoryMetadata(track: TrackInfo): boolean {
  return !!(track.title.trim() && track.artist.trim() && track.coverUrl.trim() && track.durationMs > 0)
}

function fillHistoryMetadata(track: TrackInfo, metadata: HistoryMetadata): TrackInfo {
  const title = track.title.trim() ? track.title : metadata.title
  const artist = track.artist.trim() ? track.artist : metadata.artist
  const coverUrl = track.coverUrl.trim() ? track.coverUrl : metadata.coverUrl
  const durationMs = track.durationMs > 0 ? track.durationMs : metadata.durationMs
  if (title === track.title && artist === track.artist && coverUrl === track.coverUrl && durationMs === track.durationMs) return track
  return { ...track, title, artist, coverUrl, durationMs }
}

function historyVideoMetadata(track: TrackInfo, info: BiliVideoInfo): HistoryMetadata | undefined {
  const cid = resolveBiliPartCid(track)
  const coverUrl = normalizeCoverUrlForDisplay(info.cover)
  if (!cid) return { title: info.title, artist: info.owner, coverUrl, durationMs: info.duration * 1000 }
  const page = info.pages.find(part => part.cid === cid)
  if (!page) return undefined
  if (info.pages.length === 1) return { title: info.title, artist: info.owner, coverUrl, durationMs: (page.duration || info.duration) * 1000 }
  return buildBiliPartTracks({ ...info, pages: [page] }, { ...track, coverUrl }, readBiliPartNameOrder(info.bvid))[0]
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

function normalizeTrack(value: unknown): TrackInfo {
  const raw = asRecord(value)
  return {
    id: String(raw?.id ?? ''),
    title: String(raw?.title ?? ''),
    artist: String(raw?.artist ?? ''),
    album: String(raw?.album ?? ''),
    durationMs: Number(raw?.durationMs ?? raw?.duration_ms ?? 0),
    coverUrl: String(raw?.coverUrl ?? raw?.cover_url ?? ''),
    audioUrl: String(raw?.audioUrl ?? raw?.audio_url ?? raw?.url ?? ''),
    source: asString(raw?.source),
    addedAt: Number(raw?.addedAt ?? raw?.added_at ?? 0),
    syncPayload: asRecord(raw?.syncPayload ?? raw?.sync_payload),
    playlistKey: asString(raw?.playlistKey ?? raw?.playlist_key),
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

function isLocalTrack(track: TrackInfo): boolean {
  return track.id.startsWith('local:') || inferSource(track) === 'local'
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

type HistorySyncIdentity = { songId: string; album: string; mediaUri: string; albumAliases?: string[] }

async function syncIdentity(track: TrackInfo): Promise<HistorySyncIdentity> {
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
    const payload = track.syncPayload
    const storedAudioId = String(payload?.audioId ?? payload?.audio_id ?? '').trim()
    const audioId = storedAudioId || resolveBiliVideoReference(track)?.bvid || rawId.split(':')[0]
    const cid = resolveBiliPartCid(track) ?? ''
    return {
      songId: await stableSyncId(`bilibili|${audioId}|${cid}`),
      album: 'bilibili',
      albumAliases: [track.album, String(payload?.album ?? ''), cid ? `Bilibili|${cid}` : ''].filter(Boolean),
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
  const currentEntries = computed(() => new Set(entries.value))
  const attemptedMetadata = new WeakSet<TrackInfo>()
  const videoRequests = new Map<string, Promise<BiliVideoInfo>>()
  let videoRequestTail = Promise.resolve()
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
      entries.value = uniqueByIdentity(asArray(parsedEntries)
        .map((entry): PlayedEntry => ({
          track: normalizeTrack(dig(entry, 'track')),
          playedAt: Number(dig(entry, 'playedAt') ?? dig(entry, 'played_at') ?? 0),
          resumePositionMs: resumePosition(entry),
        }))
        .filter(entry => entry.track.id && entry.playedAt > 0))
      deletions.value = uniqueByIdentity(asArray(parsedDeletions)
        .map((deletion): HistoryDeletion => ({
          track: normalizeTrack(dig(deletion, 'track')),
          deletedAt: Number(dig(deletion, 'deletedAt') ?? dig(deletion, 'deleted_at') ?? 0),
        }))
        .filter(deletion => deletion.track.id && deletion.deletedAt > 0))
    } catch {
      entries.value = []
      deletions.value = []
    }
  }

  function requestBiliMetadata({ bvid, avid }: NonNullable<ReturnType<typeof resolveBiliVideoReference>>) {
    const aliases = [bvid, avid ? `av${avid}` : null].filter((key): key is string => !!key)
    const pending = aliases.map(key => videoRequests.get(key)).find(Boolean)
    const request = pending ?? videoRequestTail.then(() => invoke<BiliVideoInfo>('get_bili_video_info', { bvid, avid: bvid ? null : avid }))
    const newAliases = aliases.filter(key => !videoRequests.has(key))
    for (const alias of newAliases) videoRequests.set(alias, request)
    const forgetAliases = () => {
      for (const alias of newAliases) videoRequests.delete(alias)
    }
    const completed = request.then(forgetAliases, forgetAliases)
    if (!pending) videoRequestTail = completed
    return request
  }

  async function restoreBiliMetadata(visibleEntries: readonly PlayedEntry[]) {
    const grouped = new Map<Promise<BiliVideoInfo>, PlayedEntry[]>()
    for (const entry of visibleEntries) {
      const track = entry.track
      if (attemptedMetadata.has(track) || hasHistoryMetadata(track)) continue
      const reference = resolveBiliVideoReference(track)
      if (!reference) continue
      attemptedMetadata.add(track)
      const request = requestBiliMetadata(reference)
      const group = grouped.get(request)
      if (group) group.push(entry)
      else grouped.set(request, [entry])
    }

    await Promise.all([...grouped].map(async ([request, group]) => {
      try {
        const info = await request
        const updates: Array<{ track: TrackInfo; playedAt: number }> = []
        for (const entry of group) {
          if (!currentEntries.value.has(entry)) continue
          const metadata = historyVideoMetadata(entry.track, info)
          if (!metadata) continue
          const repaired = fillHistoryMetadata(entry.track, metadata)
          if (repaired === entry.track) continue
          entry.track = repaired
          attemptedMetadata.add(entry.track)
          updates.push({ track: repaired, playedAt: entry.playedAt })
        }
        if (updates.length) persist('update_play_history_metadata', { entries: updates })
      } catch (error) {
        for (const entry of group) attemptedMetadata.delete(entry.track)
        log.warn('Bilibili history metadata lookup failed:', error)
      }
    }))
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
      entries: entries.value.filter(entry => !isLocalTrack(entry.track)).map(entry => ({
        track: toBackendTrack(entry.track),
        playedAt: entry.playedAt,
        resumePositionMs: entry.resumePositionMs ?? null,
      })),
      deletions: deletions.value.filter(deletion => !isLocalTrack(deletion.track)).map(deletion => ({
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
    payload: HistorySyncPayload | null | undefined,
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
    // 本地文件的路径只属于本机，远端结果不能替换本机的记录或删除标记
    const localEntries = entries.value.filter(entry => isLocalTrack(entry.track))
    const localDeletions = deletions.value.filter(deletion => isLocalTrack(deletion.track))
    const rawEntries: BackendHistoryEntry[] = Array.isArray(payload.entries) ? payload.entries : []
    const rawDeletions: BackendHistoryDeletion[] = Array.isArray(payload.deletions) ? payload.deletions : []
    const nextEntries: PlayedEntry[] = rawEntries
      .map(entry => ({
        track: normalizeTrack(entry.track),
        playedAt: Number(entry.playedAt ?? entry.played_at ?? 0),
        resumePositionMs: resumePosition(entry),
      }))
      .filter(entry => entry.track.id && entry.playedAt > 0 && !isLocalTrack(entry.track))
    const candidates = [...previousEntries, ...previousDeletions, ...nextEntries.map(entry => entry.track)]
      .filter(track => !isLocalTrack(track))
    const findMatchingTracks = createDeletionTrackMatcher(candidates)
    const resolvedDeletions: HistoryDeletion[] = []

    // 同一曲目只留最新状态，删除和播放同一时刻时以删除为准
    const mergedEntries = uniqueByIdentity([...nextEntries, ...localEntries]
      .sort((left, right) => right.playedAt - left.playedAt))
    const playedTimes = new Map(mergedEntries.map(entry => [historyEntryKey(entry.track), entry.playedAt]))
    const deletedTimes = new Map<string, number>()
    const noteDeletion = (key: string, time: number) => deletedTimes.set(key, Math.max(deletedTimes.get(key) ?? 0, time))

    function applyMatchedDeletion(matchingTracks: TrackInfo[], deletedAt: number) {
      if (!matchingTracks.length || !(deletedAt > 0)) return
      const keys = matchingTracks.map(historyEntryKey)
      for (const key of keys) noteDeletion(key, deletedAt)
      // 一个分 P 可能有多种旧 ID，以同一同步身份下的所有播放时间判定墓碑
      if (keys.every(key => (playedTimes.get(key) ?? 0) <= deletedAt)) {
        resolvedDeletions.push({ track: matchingTracks[0], deletedAt })
      }
    }

    for (const rawDeletion of rawDeletions) {
      const matchingTracks = await findMatchingTracks(rawDeletion, canApply)
      if (!canApply()) return interrupted()
      applyMatchedDeletion(matchingTracks, Number(rawDeletion.deletedAt ?? rawDeletion.deleted_at ?? 0))
    }

    // 用同步身份保留尚未被较新重播覆盖的本机 B 站墓碑，避免不完整快照恢复旧分 P
    for (const deletion of deletions.value) {
      if (inferSource(deletion.track) !== 'bilibili' || !(deletion.deletedAt > 0)) continue
      const identity = await syncIdentity(deletion.track)
      if (!canApply()) return interrupted()
      const matchingTracks = await findMatchingTracks(identity, canApply)
      if (!canApply()) return interrupted()
      applyMatchedDeletion(matchingTracks, deletion.deletedAt)
    }

    const mergedDeletions = uniqueByIdentity([...resolvedDeletions, ...localDeletions]
      .filter(deletion => deletion.track.id && deletion.deletedAt > 0)
      .sort((left, right) => right.deletedAt - left.deletedAt))
    for (const deletion of mergedDeletions) noteDeletion(historyEntryKey(deletion.track), deletion.deletedAt)

    // 提交阶段没有异步等待，先复核账号意图和本地修改再一起保存
    if (!canApply()) return interrupted()
    mutationEpoch++
    // 合并结果按同步身份区分，同一曲目可能因专辑名不同出现多条，本地只留最新的一条
    const previousBiliTracks = new Map(entries.value.filter(entry => entry.track.id.startsWith('bilibili:'))
      .map(entry => [historyEntryKey(entry.track), entry.track]))
    entries.value = mergedEntries.filter(entry => (deletedTimes.get(historyEntryKey(entry.track)) ?? 0) < entry.playedAt)
      .map(entry => {
        const previous = previousBiliTracks.get(historyEntryKey(entry.track))
        const track = previous ? fillHistoryMetadata(entry.track, previous) : entry.track
        return track === entry.track ? entry : { ...entry, track }
      })
    deletions.value = mergedDeletions.filter(deletion => (playedTimes.get(historyEntryKey(deletion.track)) ?? 0) <= deletion.deletedAt)
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

  function createDeletionTrackMatcher(candidates: TrackInfo[]) {
    const identities = new Map<string, Awaited<ReturnType<typeof syncIdentity>>>()
    const matches = new Map<string, TrackInfo[]>()
    let nextCandidate = 0

    return async (deletion: BackendHistoryDeletion, isCurrent: () => boolean): Promise<TrackInfo[]> => {
      const songId = String(deletion.songId ?? '')
      const album = String(deletion.album ?? '') || null
      const mediaUri = String(deletion.mediaUri ?? '') || null
      const expectedKey = JSON.stringify([songId, album, mediaUri])
      if (!isCurrent()) return []

      // 沿原候选顺序逐步建立索引，B 站分 P 需要继续收集同一同步身份的其它旧 ID
      while (nextCandidate < candidates.length) {
        const match = matches.get(expectedKey)
        if (match && inferSource(match[0]) !== 'bilibili') return match
        const candidate = candidates[nextCandidate++]!
        const source = inferSource(candidate)
        const identityKey = JSON.stringify(source === 'bilibili'
          ? [source, historyEntryKey(candidate), candidate.album, candidate.syncPayload?.audioId, candidate.syncPayload?.audio_id, candidate.syncPayload?.album]
          : [source, candidate.id, candidate.album])
        let identity = identities.get(identityKey)
        if (!identity) {
          identity = await syncIdentity(candidate)
          if (!isCurrent()) return []
          identities.set(identityKey, identity)
        }
        for (const album of new Set([identity.album, ...(identity.albumAliases ?? []), null])) {
          for (const mediaUri of [identity.mediaUri, null]) {
            const key = JSON.stringify([identity.songId, album, mediaUri])
            const sameIdentity = matches.get(key)
            if (!sameIdentity) matches.set(key, [candidate])
            else if (source === 'bilibili' && inferSource(sameIdentity[0]) === 'bilibili') sameIdentity.push(candidate)
          }
        }
      }
      const match = matches.get(expectedKey)
      if (match && inferSource(match[0]) === 'bilibili') {
        return matches.get(JSON.stringify([songId, 'bilibili', mediaUri])) ?? match
      }
      return match ?? []
    }
  }

  load()

  return {
    entries,
    deletions,
    restoreBiliMetadata,
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
