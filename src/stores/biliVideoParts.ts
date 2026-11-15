import { defineStore } from 'pinia'
import { computed, ref, shallowRef } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { useToastStore } from '@/stores/toast'
import i18n from '@/i18n'
import {
  buildBiliPartTracks, readBiliPartNameOrder, resolveBiliPartCid, resolveBiliVideoReference, saveBiliPartNameOrder,
  type BiliPartNameOrder, type BiliVideoInfo,
} from '@/modules/playback/biliVideoParts'
import { isBiliVideoSkipTrack } from '@/modules/playback/biliVideoSkip'

type PlayParts = (tracks: TrackInfo[], startTrackId: string) => void
type CollectionScope = 'whole' | 'part'
type VideoReference = NonNullable<ReturnType<typeof resolveBiliVideoReference>>
export interface BiliCollectionIdentity {
  video: VideoReference
  allowWholeFavorite?: boolean
  singlePageCid?: number
}

type CollectionSave = (track: TrackInfo, identity?: BiliCollectionIdentity) => void

interface CollectionChoice {
  whole: TrackInfo
  part: TrackInfo
  partNumber: number
  isLiked?: (track: TrackInfo) => boolean
  save: (track: TrackInfo) => void
}

export const useBiliVideoPartsStore = defineStore('biliVideoParts', () => {
  const player = usePlayerStore()
  const info = shallowRef<BiliVideoInfo | null>(null)
  const collection = shallowRef<CollectionChoice | null>(null)
  const open = computed(() => !!(info.value || collection.value))
  const selection = shallowRef<{ parent: TrackInfo; playParts: PlayParts } | null>(null)
  const nameOrder = ref<BiliPartNameOrder>('title-first')
  const tracks = computed(() => info.value && selection.value
    ? buildBiliPartTracks(info.value, selection.value.parent, nameOrder.value)
    : [])
  let generation = 0
  let lastDetail: BiliVideoInfo | null = null
  let releaseCollectionSignal: (() => void) | undefined

  function blockedByRoom(): boolean {
    const restriction = useListenTogetherStore().localControlRestriction
    if (!restriction) return false
    useToastStore().error(i18n.global.t(restriction === 'controller_offline'
      ? 'listen_together.control_blocked_controller_offline'
      : 'listen_together.control_blocked_member_control'))
    return true
  }

  function close() {
    generation++
    releaseCollectionSignal?.()
    releaseCollectionSignal = undefined
    info.value = null
    collection.value = null
    selection.value = null
    nameOrder.value = 'title-first'
  }

  async function loadDetail({ bvid, avid }: VideoReference) {
    const detail = await invoke<BiliVideoInfo>('get_bili_video_info', { bvid, avid: bvid ? null : avid })
    if (!detail.bvid || !detail.aid || !detail.pages.length) throw new Error('Missing video parts')
    return detail
  }

  async function request(parent: TrackInfo, play: () => void, playParts: PlayParts = player.playAll) {
    close()
    if (isBiliVideoSkipTrack(parent) && blockedByRoom()) return
    const reference = resolveBiliVideoReference(parent)
    if (!reference || resolveBiliPartCid(parent)) {
      play()
      return
    }
    const token = generation
    const previousTrack = player.currentTrack
    let detail: BiliVideoInfo
    try {
      detail = await loadDetail(reference)
    } catch {
      // 音频仍可能已下载或缓存，交给播放器沿用原来的取流和错误处理
      if (token === generation && player.currentTrack === previousTrack && !blockedByRoom()) play()
      return
    }
    if (token !== generation || player.currentTrack !== previousTrack || blockedByRoom()) return
    lastDetail = detail
    if (detail.pages.length === 1) {
      play()
      return
    }
    selection.value = { parent, playParts }
    nameOrder.value = readBiliPartNameOrder(detail.bvid)
    info.value = detail
  }

  function choose(cid: number) {
    if (!info.value || !selection.value) return
    if (blockedByRoom()) {
      close()
      return
    }
    const { playParts } = selection.value
    const queue = tracks.value
    const selected = queue.find(item => resolveBiliPartCid(item) === cid)
    if (!selected) return
    saveBiliPartNameOrder(info.value.bvid, nameOrder.value)
    close()
    playParts(queue, selected.id)
  }

  async function requestCollection(
    track: TrackInfo,
    save: CollectionSave,
    isLiked?: (track: TrackInfo, video: VideoReference) => boolean,
    signal?: AbortSignal,
  ) {
    if (signal?.aborted) return
    close()
    const reference = resolveBiliVideoReference(track)
    if (!reference) {
      save(track)
      return
    }
    const token = generation
    if (signal) {
      const abort = () => { if (token === generation) close() }
      signal.addEventListener('abort', abort, { once: true })
      releaseCollectionSignal = () => signal.removeEventListener('abort', abort)
    }
    function saveCurrent(video: VideoReference, singlePageCid?: number) {
      if (token !== generation) return
      close()
      save(track, { video, allowWholeFavorite: true, singlePageCid })
    }

    let detail: BiliVideoInfo
    try {
      const cached = lastDetail
      detail = cached && (reference.bvid ? cached.bvid === reference.bvid : cached.aid === reference.avid)
        ? cached : await loadDetail(reference)
    } catch {
      // 歌单保存在本机，视频详情不可用时仍可按当前条目的身份保存
      saveCurrent(reference)
      return
    }
    if (token !== generation) return
    const video = { bvid: detail.bvid, avid: detail.aid }
    const fixedCid = resolveBiliPartCid(track)
    const cid = fixedCid ?? detail.cid
    const page = detail.pages.find(item => item.cid === cid)
    if (!page) {
      saveCurrent(video)
      return
    }
    if (detail.pages.length === 1) {
      lastDetail = detail
      saveCurrent(video, page.cid)
      return
    }
    const [part] = buildBiliPartTracks({ ...detail, pages: [page] }, track, readBiliPartNameOrder(detail.bvid))
    if (!part) return
    lastDetail = detail
    // 批量播放时，视频条目可能带着整集的标题和时长，但已被播放器补上了 CID
    const hasWholeVideoMetadata = track.title === detail.title && track.durationMs === detail.duration * 1000
    collection.value = {
      whole: {
        id: `bilibili:${detail.bvid}`, title: detail.title, artist: detail.owner, coverUrl: detail.cover,
        durationMs: detail.duration * 1000, album: '', source: 'bilibili', audioUrl: '',
      },
      part: fixedCid && !hasWholeVideoMetadata ? {
        ...track,
        id: part.id,
        title: track.title.trim() || part.title,
        artist: track.artist.trim() || part.artist,
        coverUrl: track.coverUrl.trim() || part.coverUrl,
        durationMs: track.durationMs || part.durationMs,
        album: part.album,
        syncPayload: { ...part.syncPayload, ...track.syncPayload, album: part.album },
      } : part,
      partNumber: page.page,
      isLiked: isLiked && (target => isLiked(target, video)),
      save: target => save(target, { video }),
    }
  }

  function chooseCollection(scope: CollectionScope) {
    const choice = collection.value
    if (!choice) return
    close()
    choice.save(choice[scope])
  }

  return { open, info, collection, nameOrder, tracks, request, choose, requestCollection, chooseCollection, close }
})
