import { defineStore } from 'pinia'
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { useToastStore } from '@/stores/toast'
import i18n from '@/i18n'
import { buildBiliPartTracks, type BiliVideoInfo } from '@/modules/playback/biliVideoParts'
import { isBiliVideoSkipTrack, resolveBiliVideoSkipBvid, resolveBiliVideoSkipCid } from '@/modules/playback/biliVideoSkip'

export const useBiliVideoPartsStore = defineStore('biliVideoParts', () => {
  const player = usePlayerStore()
  const open = ref(false)
  const loading = ref(false)
  const failed = ref(false)
  const info = ref<BiliVideoInfo | null>(null)
  const track = ref<TrackInfo | null>(null)
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
    const tracks = buildBiliPartTracks(info.value, track.value)
    const selected = tracks.find(item => item.syncPayload?.subAudioId === String(cid))
    if (!selected) return
    close()
    player.playAll(tracks, selected.id)
  }

  function retry() {
    if (track.value && fallback) return request(track.value, fallback)
  }

  return { open, loading, failed, info, track, request, choose, close, retry }
})
