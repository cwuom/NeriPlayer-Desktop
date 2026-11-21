import { computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { resolvePlaybackQueueStartIndex } from '@/modules/playback/playbackQueue'
import { buildQueueDisplayOrder } from '@/modules/playback/queueDisplayOrder'
import { createLogger } from '@/utils/logger'
import type { RepeatMode, TrackInfo } from '@/types/track'
import { tracePlaybackUi } from './trace'
import type { PlaybackCommandSource, PlaybackControl, PlayerCoreState, PlayFunction } from './types'

const log = createLogger('player')

type ShuffleState = Pick<PlayerCoreState, 'queue' | 'queueIndex' | 'shuffleBag' | 'shuffleHistory' | 'shuffleFuture'>

/** Shuffle 三栈（未播放池 bag、回退历史 history、待播 future）随队列变动的索引维护 */
export function createShuffleOrder(ctx: ShuffleState) {
  const { queue, queueIndex, shuffleBag, shuffleHistory, shuffleFuture } = ctx

  // Shuffle 三栈辅助函数
  /** Fisher-Yates 洗牌重建未播放池，排除当前索引 */
  function rebuildShuffleBag() {
    shuffleBag.value = []
    for (let i = 0; i < queue.value.length; i++) {
      if (i !== queueIndex.value) shuffleBag.value.push(i)
    }
    for (let i = shuffleBag.value.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [shuffleBag.value[i], shuffleBag.value[j]] = [shuffleBag.value[j], shuffleBag.value[i]]
    }
  }

  /**
   * Shifts shuffle indices after a "play next" insertion.
   *
   * The new slot must join the queued-future stack, not the unplayed bag: the next
   * shuffled track only pops from `shuffleFuture` (LIFO), so a slot pushed into the bag
   * is shuffled to a random position and the insertion looks like it did nothing.
   */
  function shiftShuffleIndicesForPlayNext(insertIdx: number) {
    shuffleBag.value = shuffleBag.value.map(i => i >= insertIdx ? i + 1 : i)
    shuffleHistory.value = shuffleHistory.value.map(i => i >= insertIdx ? i + 1 : i)
    shuffleFuture.value = shuffleFuture.value.map(i => i >= insertIdx ? i + 1 : i)
    shuffleFuture.value.push(insertIdx)
  }

  /** 队列移除后更新 shuffle 索引 */
  function shiftShuffleIndicesForRemove(removeIdx: number) {
    shuffleBag.value = shuffleBag.value.filter(i => i !== removeIdx).map(i => i > removeIdx ? i - 1 : i)
    shuffleHistory.value = shuffleHistory.value.filter(i => i !== removeIdx).map(i => i > removeIdx ? i - 1 : i)
    shuffleFuture.value = shuffleFuture.value.filter(i => i !== removeIdx).map(i => i > removeIdx ? i - 1 : i)
  }

  return { rebuildShuffleBag, shiftShuffleIndicesForPlayNext, shiftShuffleIndicesForRemove }
}

/**
 * 统一播放模式循环切换：顺序播放 -> 列表循环 -> 单曲循环 -> 随机播放 -> 顺序播放
 * 合并 repeat + shuffle 为一个按钮的逻辑
 */
type PlayMode = 'sequential' | 'repeat_all' | 'repeat_one' | 'shuffle'

export interface QueueEditingContext extends PlayerCoreState {
  play: PlayFunction
  pause: PlaybackControl
  savePlayerState: () => void
  maybePrefetchNext: () => void
  blockedByListenTogether: (commandSource: PlaybackCommandSource) => boolean
  blocksLocalSongInRoom: (track: TrackInfo, commandSource: PlaybackCommandSource) => boolean
  rebuildShuffleBag: () => void
  shiftShuffleIndicesForRemove: (removeIdx: number) => void
  toggleShuffle: () => Promise<void>
}

/** 队列编辑、播放模式与队列展示顺序 */
export function createQueueEditing(ctx: QueueEditingContext) {
  const {
    queue, queueIndex, currentTrack, queueSelectionNeedsPlayback, repeatMode, shuffleEnabled,
    shuffleBag, shuffleHistory, shuffleFuture, play, pause, savePlayerState, maybePrefetchNext,
    blockedByListenTogether, blocksLocalSongInRoom, rebuildShuffleBag, shiftShuffleIndicesForRemove, toggleShuffle,
  } = ctx

  const queueDisplayOrder = computed(() => buildQueueDisplayOrder(
    queue.value.length, queueIndex.value, shuffleEnabled.value,
    shuffleHistory.value, shuffleFuture.value, shuffleBag.value,
  ))
  const queueDisplayIndices = computed(() => queueDisplayOrder.value.indices)
  const queueDisplayCurrentIndex = computed(() => queueDisplayOrder.value.currentIndex)

  /** play 在异步解析前和加载完成后各提交一次选中曲目，同一次请求只生效一次 */
  function createTrackCommit(track: TrackInfo, requestedQueueIndex: number | undefined, shuffleNavigation: boolean) {
    let trackCommitted = false

    function commitTrack() {
      if (trackCommitted) return
      const previousTrack = currentTrack.value
      const matchesQueueIdentity = (item: TrackInfo | undefined, target: TrackInfo | null) => !!target
        && item?.id === target.id && (item.playlistKey ?? '') === (target.playlistKey ?? '')
      const previousQueueIndex = matchesQueueIdentity(queue.value[queueIndex.value], previousTrack)
        ? queueIndex.value
        : queue.value.findIndex(item => matchesQueueIdentity(item, previousTrack))
      const nextQueueIndex = requestedQueueIndex !== undefined && matchesQueueIdentity(queue.value[requestedQueueIndex], track)
        ? requestedQueueIndex
        : matchesQueueIdentity(queue.value[previousQueueIndex], track)
          ? previousQueueIndex
          : resolvePlaybackQueueStartIndex(queue.value, track.id, track.playlistKey)
      if (shuffleEnabled.value && !shuffleNavigation) {
        if (nextQueueIndex !== previousQueueIndex || queueSelectionNeedsPlayback.value) {
          shuffleBag.value = shuffleBag.value.filter(index => index !== nextQueueIndex)
        }
        if (previousQueueIndex >= 0 && nextQueueIndex !== previousQueueIndex && !queueSelectionNeedsPlayback.value) {
          shuffleHistory.value.push(previousQueueIndex)
        }
      }
      queueSelectionNeedsPlayback.value = false
      currentTrack.value = track
      queueIndex.value = nextQueueIndex
      // 推送元数据到后端镜像, 供系统媒体会话(SMTC/MPRIS)展示标题/歌手/封面/时长（PB-01）
      void invoke('update_media_metadata', {
        id: track.id,
        title: track.title,
        artist: track.artist,
        album: track.album || '',
        coverUrl: track.coverUrl || null,
        durationMs: track.durationMs || 0,
      }).catch(() => {})
      trackCommitted = true
    }

    return commitTrack
  }

  async function toggleRepeatMode() {
    // 播放模式在前端本地推进（对齐 Android cycleRepeatModeImpl）, 不再采信后端
    // PlayQueue 独立状态机的返回值: 后端进程启动恒为 Off, 与前端从 localStorage 恢复的
    // 模式不同步, 会导致重启后首次点击跳档/无效（PB-03）
    const modes: RepeatMode[] = ['off', 'all', 'one']
    const idx = modes.indexOf(repeatMode.value)
    repeatMode.value = modes[(idx + 1) % modes.length]
    log.info('repeat mode ->', repeatMode.value)
    savePlayerState()
  }

  const playMode = computed<PlayMode>(() => {
    if (shuffleEnabled.value) return 'shuffle'
    if (repeatMode.value === 'all') return 'repeat_all'
    if (repeatMode.value === 'one') return 'repeat_one'
    return 'sequential'
  })

  /** 后端的循环状态只是镜像，以前端为准（PB-03） */
  async function mirrorRepeatModeToBackend() {
    try {
      await invoke<string>('cycle_repeat')
    } catch {
      // 镜像同步失败不回滚前端已切换的模式
    }
  }

  async function cyclePlayMode() {
    const current = playMode.value
    switch (current) {
      case 'sequential':
        // -> 列表循环
        if (shuffleEnabled.value) await toggleShuffle()
        repeatMode.value = 'all'
        await mirrorRepeatModeToBackend()
        break
      case 'repeat_all':
        // -> 单曲循环
        repeatMode.value = 'one'
        await mirrorRepeatModeToBackend()
        break
      case 'repeat_one':
        // -> 随机播放
        repeatMode.value = 'off'
        await mirrorRepeatModeToBackend()
        if (!shuffleEnabled.value) await toggleShuffle()
        break
      case 'shuffle':
        // -> 顺序播放
        if (shuffleEnabled.value) await toggleShuffle()
        repeatMode.value = 'off'
        break
    }
    savePlayerState()
  }

  /**
   * 一起听远端模式应用 (对齐 Android applyListenTogetherPlaybackMode)
   * 仅本地落状态, 不反向上报, 由调用方负责 suppress watch
   */
  function applyListenTogetherPlaybackMode(options: {
    repeatMode?: number | null
    shuffleEnabled?: boolean | null
  }) {
    let changed = false
    if (options.repeatMode !== null && options.repeatMode !== undefined) {
      const mapped =
        options.repeatMode === 1 ? 'one'
          : options.repeatMode === 2 ? 'all'
            : options.repeatMode === 0 ? 'off'
              : null
      if (mapped && repeatMode.value !== mapped) {
        repeatMode.value = mapped
        changed = true
      }
    }
    if (typeof options.shuffleEnabled === 'boolean' && shuffleEnabled.value !== options.shuffleEnabled) {
      shuffleEnabled.value = options.shuffleEnabled
      if (shuffleEnabled.value) {
        rebuildShuffleBag()
        shuffleHistory.value = []
        shuffleFuture.value = []
      } else {
        shuffleBag.value = []
        shuffleHistory.value = []
        shuffleFuture.value = []
      }
      changed = true
    }
    if (changed) savePlayerState()
  }

  // 批量替换队列，并且只发起一次目标曲目的播放请求
  // 队列从哪个本地歌单开始播：其中的歌计入一次播放时，歌单也计一次（对齐 Android localPlaylistPlaybackSource）
  let localPlaylistSource: { id: string; members: Set<string> } | null = null

  function localPlaylistMemberKey(track: TrackInfo): string {
    return track.playlistKey || track.id
  }

  function setLocalPlaylistSource(tracks: TrackInfo[], localPlaylistId?: string) {
    localPlaylistSource = localPlaylistId
      ? { id: localPlaylistId, members: new Set(tracks.map(localPlaylistMemberKey)) }
      : null
  }

  /** 这首歌属于当前队列的来源本地歌单时返回歌单 id；之后加进队列的别处歌曲不算 */
  function localPlaylistIdFor(track: TrackInfo): string | null {
    return localPlaylistSource?.members.has(localPlaylistMemberKey(track)) ? localPlaylistSource.id : null
  }

  /** @param localPlaylistId 从本地歌单开始播放时传入，用于歌单播放统计 */
  function playAll(
    tracks: TrackInfo[],
    requestedTrackId?: string,
    requestedPlaylistKey?: string,
    localPlaylistId?: string,
  ) {
    const startIndex = resolvePlaybackQueueStartIndex(
      tracks,
      requestedTrackId,
      requestedPlaylistKey,
    )
    if (startIndex < 0) {
      tracePlaybackUi(
        'queue_start_rejected',
        undefined,
        `tracks=${tracks.length}, requestedId=${requestedTrackId || '-'}, requestedKey=${requestedPlaylistKey || '-'}`,
      )
      return
    }
    tracePlaybackUi(
      'queue_start_resolved',
      tracks[startIndex],
      `index=${startIndex}, tracks=${tracks.length}, requestedId=${requestedTrackId || '-'}`,
    )
    queue.value = [...tracks]
    queueIndex.value = startIndex
    shuffleHistory.value = []
    shuffleFuture.value = []
    setLocalPlaylistSource(tracks, localPlaylistId)
    // 队列整体替换后未播放池必须按新队列重建（排除当前索引）；
    // 否则曲目自然结束/手动 next 走 bag 空 -> 提前 return，自动切歌永久卡死
    if (shuffleEnabled.value) {
      rebuildShuffleBag()
    } else {
      shuffleBag.value = []
    }
    void play(queue.value[startIndex], 'local', 0, false, true, startIndex, true)
  }

  // 洗牌后替换队列并播放
  function shufflePlay(tracks: TrackInfo[], localPlaylistId?: string) {
    if (tracks.length === 0) return
    setLocalPlaylistSource(tracks, localPlaylistId)
    const shuffled = [...tracks]
    for (let i = shuffled.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]]
    }
    queue.value = shuffled
    queueIndex.value = 0
    shuffleHistory.value = []
    shuffleFuture.value = []
    if (shuffleEnabled.value) {
      rebuildShuffleBag()
    } else {
      shuffleBag.value = []
    }
    void play(shuffled[0], 'local', 0, false, true, 0, true)
  }

  async function playQueueItem(index: number, commandSource: PlaybackCommandSource = 'local') {
    const track = queue.value[index]
    if (!Number.isInteger(index) || !track || blockedByListenTogether(commandSource)
      || blocksLocalSongInRoom(track, commandSource)) return
    await play(track, commandSource, 0, false, true, index)
  }

  // 追加到队列末尾
  function addToQueueEnd(track: TrackInfo) {
    if (!queue.value.find(t => t.id === track.id)) {
      queue.value.push(track)
      if (shuffleEnabled.value) {
        shuffleBag.value.push(queue.value.length - 1)
      }
    }
    savePlayerState()
  }

  function moveQueueItem(fromIndex: number, toIndex: number, expectedQueue?: readonly TrackInfo[]): boolean {
    if (!Number.isInteger(fromIndex) || !Number.isInteger(toIndex)
      || fromIndex < 0 || toIndex < 0
      || fromIndex >= queue.value.length || toIndex >= queue.value.length
      || fromIndex === toIndex) return false
    if (expectedQueue && (expectedQueue.length !== queue.value.length
      || expectedQueue.some((track, index) => track !== queue.value[index]))) return false
    if (blockedByListenTogether('local')) return false

    // 按槽位映射，重复歌曲也保留各自的播放索引和随机播放历史
    function remapIndex(index: number): number {
      if (index === fromIndex) return toIndex
      if (fromIndex < toIndex && index > fromIndex && index <= toIndex) return index - 1
      if (fromIndex > toIndex && index >= toIndex && index < fromIndex) return index + 1
      return index
    }

    const nextQueue = [...queue.value]
    const [moved] = nextQueue.splice(fromIndex, 1)
    nextQueue.splice(toIndex, 0, moved)
    queueIndex.value = remapIndex(queueIndex.value)
    shuffleBag.value = shuffleBag.value.map(remapIndex)
    shuffleHistory.value = shuffleHistory.value.map(remapIndex)
    shuffleFuture.value = shuffleFuture.value.map(remapIndex)
    queue.value = nextQueue
    useListenTogetherStore().reportQueueReorder()
    savePlayerState()
    maybePrefetchNext()
    return true
  }

  // 从队列移除指定索引
  function removeFromQueue(index: number) {
    if (index < 0 || index >= queue.value.length) return
    const wasCurrentTrack = index === queueIndex.value

    if (shuffleEnabled.value) shiftShuffleIndicesForRemove(index)

    queue.value.splice(index, 1)
    if (queue.value.length === 0) {
      queueIndex.value = -1
      queueSelectionNeedsPlayback.value = false
      currentTrack.value = null
      shuffleBag.value = []
      shuffleHistory.value = []
      shuffleFuture.value = []
      void pause('local_safety')
      savePlayerState()
      return
    }
    if (index < queueIndex.value) {
      queueIndex.value--
    } else if (wasCurrentTrack) {
      // 被删除的是当前曲目，索引保持（指向下一首），但不超界
      queueIndex.value = Math.min(queueIndex.value, queue.value.length - 1)
      // 这里只更新选中项，尚未加载替代歌曲，不能提前消费它的待播记录
      queueSelectionNeedsPlayback.value = true
      currentTrack.value = queue.value[queueIndex.value]
    }
    savePlayerState()
  }

  // 清空队列
  function clearQueue() {
    queue.value = []
    queueIndex.value = -1
    queueSelectionNeedsPlayback.value = false
    shuffleBag.value = []
    shuffleHistory.value = []
    shuffleFuture.value = []
    savePlayerState()
  }

  return {
    queueDisplayIndices, queueDisplayCurrentIndex, createTrackCommit,
    toggleRepeatMode, playMode, cyclePlayMode, applyListenTogetherPlaybackMode, localPlaylistIdFor,
    playAll, shufflePlay, playQueueItem, addToQueueEnd, moveQueueItem, removeFromQueue, clearQueue,
  }
}

