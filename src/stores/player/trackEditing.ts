import type { Ref } from 'vue'
import type { TrackInfo } from '@/types/track'

/** 编辑当前曲目信息，保留编辑前的副本以便还原 */
export function createTrackEditing(currentTrack: Ref<TrackInfo | null>, queue: Ref<TrackInfo[]>) {
  // 编辑当前曲目信息
  let originalTrackInfo: TrackInfo | null = null

  function updateCurrentTrackInfo(patch: Partial<TrackInfo>) {
    if (!currentTrack.value) return
    if (!originalTrackInfo) {
      originalTrackInfo = { ...currentTrack.value }
    }
    currentTrack.value = { ...currentTrack.value, ...patch }
  }

  /** 更新当前曲目 syncPayload (合并字段), 并同步到队列中同 id 曲目 */
  function patchCurrentTrackSyncPayload(
    nextPayload: Record<string, unknown> | null | undefined,
  ) {
    if (!currentTrack.value) return
    const payload = nextPayload ? { ...nextPayload } : undefined
    updateCurrentTrackInfo({ syncPayload: payload })
    const trackId = currentTrack.value.id
    if (!trackId) return
    queue.value = queue.value.map((item) =>
      item.id === trackId ? { ...item, syncPayload: payload } : item,
    )
  }

  function restoreOriginalTrackInfo() {
    if (originalTrackInfo && currentTrack.value) {
      currentTrack.value = { ...originalTrackInfo }
      originalTrackInfo = null
    }
  }

  function hasOriginalTrackInfo() {
    return originalTrackInfo !== null
  }

  return { updateCurrentTrackInfo, patchCurrentTrackSyncPayload, restoreOriginalTrackInfo, hasOriginalTrackInfo }
}
