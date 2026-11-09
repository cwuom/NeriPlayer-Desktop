import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { useToastStore } from '@/stores/toast'
import i18n from '@/i18n'
import { buildBiliPartTracks, type BiliPartNameOrder, type BiliVideoInfo } from '@/modules/playback/biliVideoParts'
import { isBiliVideoSkipTrack, resolveBiliVideoSkipBvid, resolveBiliVideoSkipCid } from '@/modules/playback/biliVideoSkip'
import { createLogger } from '@/utils/logger'

const log = createLogger('bili-video-parts')
const NAME_ORDER_STORAGE_PREFIX = 'neri:bili-part-name-order:v1:'

function readNameOrder(bvid: string): BiliPartNameOrder {
  try {
    return localStorage.getItem(`${NAME_ORDER_STORAGE_PREFIX}${bvid}`) === 'artist-first'
      ? 'artist-first'
      : 'title-first'
  } catch (error) {
    log.warn('Failed to read part name order:', error)
    return 'title-first'
  }
}

function saveNameOrder(bvid: string, order: BiliPartNameOrder) {
  try {
    const key = `${NAME_ORDER_STORAGE_PREFIX}${bvid}`
    if (order === 'artist-first') localStorage.setItem(key, order)
    else localStorage.removeItem(key)
  } catch (error) {
    log.warn('Failed to save part name order:', error)
  }
}

export const useBiliVideoPartsStore = defineStore('biliVideoParts', () => {
  const player = usePlayerStore()
  const open = ref(false)
  const loading = ref(false)
  const failed = ref(false)
  const info = ref<BiliVideoInfo | null>(null)
  const track = ref<TrackInfo | null>(null)
  const nameOrder = ref<BiliPartNameOrder>('title-first')
  const tracks = computed(() => info.value && track.value
    ? buildBiliPartTracks(info.value, track.value, nameOrder.value)
    : [])
  let generation = 0
  let fallback: (() => void) | null = null

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
    open.value = false
    loading.value = false
    failed.value = false
    info.value = null
    track.value = null
    nameOrder.value = 'title-first'
    fallback = null
  }

  async function request(parent: TrackInfo, play: () => void) {
    close()
    const isBili = isBiliVideoSkipTrack(parent)
    if (isBili && blockedByRoom()) return
    if (!isBili || resolveBiliVideoSkipCid(parent)) {
      play()
      return
    }
    const bvid = resolveBiliVideoSkipBvid(parent)
    const audioId = String(parent.syncPayload?.audioId ?? parent.syncPayload?.audio_id
      ?? parent.id.replace(/^bilibili:/i, '').split(':')[0] ?? '')
    const avid = /^\d+$/.test(audioId) ? Number(audioId) : null
    if (!bvid && (!avid || !Number.isSafeInteger(avid))) {
      play()
      return
    }
    const token = generation
    const previousTrack = player.currentTrack
    track.value = parent
    fallback = play
    open.value = true
    loading.value = true
    try {
      const detail = await invoke<BiliVideoInfo>('get_bili_video_info', { bvid, avid: bvid ? null : avid })
      if (token !== generation) return
      if (player.currentTrack !== previousTrack || blockedByRoom()) {
        close()
        return
      }
      if (!detail.bvid || !detail.aid || !detail.pages.length) throw new Error('Missing video parts')
      if (detail.pages.length === 1) {
        close()
        play()
        return
      }
      nameOrder.value = readNameOrder(detail.bvid)
      info.value = detail
    } catch {
      if (token === generation) failed.value = true
    } finally {
      if (token === generation) loading.value = false
    }
  }

  function choose(cid: number) {
    if (!info.value || !track.value || loading.value) return
    if (blockedByRoom()) {
      close()
      return
    }
    const queue = tracks.value
    const selected = queue.find(item => item.syncPayload?.subAudioId === String(cid))
    if (!selected) return
    saveNameOrder(info.value.bvid, nameOrder.value)
    close()
    player.playAll(queue, selected.id)
  }

  function retry() {
    if (track.value && fallback) return request(track.value, fallback)
  }

  return { open, loading, failed, info, track, nameOrder, tracks, request, choose, close, retry }
})
