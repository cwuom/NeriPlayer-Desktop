import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useHistoryStore } from './history'
import { useToastStore } from './toast'
import { useSettingsStore } from './settings'
import i18n from '@/i18n'

export interface TrackInfo {
  id: string
  title: string
  artist: string
  album: string
  durationMs: number
  coverUrl: string
  audioUrl: string
}

/**
 * 将后端返回的 snake_case TrackInfo 映射为前端 camelCase。
 * 前端手动构造的对象已经是 camelCase，此函数同时兼容两种格式
 */
export function normalizeTrack(raw: any): TrackInfo {
  return {
    id: raw.id ?? '',
    title: raw.title ?? '',
    artist: raw.artist ?? '',
    album: raw.album ?? '',
    durationMs: raw.durationMs ?? raw.duration_ms ?? 0,
    coverUrl: raw.coverUrl ?? raw.cover_url ?? '',
    audioUrl: raw.audioUrl ?? raw.audio_url ?? raw.url ?? '',
  }
}

/** UI 显示用的专辑名：清理 B站 "Bilibili|{cid}" 等内部格式 */
export function displayAlbum(album: string): string {
  if (album.startsWith('Bilibili|') || album === 'Bilibili') return 'Bilibili'
  if (album.startsWith('Netease')) return album.replace(/^Netease/, '').trim() || album
  return album
}

export interface LyricWord {
  startMs: number
  durationMs: number
  text: string
}

export interface LyricLine {
  startMs: number
  durationMs: number
  words: LyricWord[]
  text: string
  translation?: string
}

export type RepeatMode = 'off' | 'all' | 'one'

export const usePlayerStore = defineStore('player', () => {
  const isPlaying = ref(false)
  const currentTrack = ref<TrackInfo | null>(null)
  const positionMs = ref(0)
  const durationMs = ref(0)
  const queue = ref<TrackInfo[]>([])
  const queueIndex = ref(-1)
  const repeatMode = ref<RepeatMode>('off')
  const shuffleEnabled = ref(false)
  const volume = ref(1)
  const lyrics = ref<LyricLine[]>([])

  // 播放错误信息（供 UI 展示）
  const playError = ref<string | null>(null)
  // 是否正在加载音频（下载/解码中）
  const isLoadingAudio = ref(false)

  // 当前音频质量信息
  interface AudioInfo {
    bitrate?: number  // kbps
    codec?: string    // e.g. "MP3", "FLAC", "AAC", "Opus"
    format?: string   // 原始格式标识
  }
  const audioInfo = ref<AudioInfo | null>(null)

  // 睡眠定时器
  const sleepTimerEndMs = ref(0) // 0 = 未启用
  const sleepTimerMode = ref<'countdown' | 'end_of_track' | 'end_of_queue' | null>(null)
  let _sleepTimerInterval: ReturnType<typeof setInterval> | null = null

  /** 剩余睡眠时间（秒） */
  const sleepRemainingSeconds = computed(() => {
    if (!sleepTimerMode.value || sleepTimerEndMs.value <= 0) return 0
    if (sleepTimerMode.value === 'end_of_track') return -1 // 特殊标记
    return Math.max(0, Math.ceil((sleepTimerEndMs.value - Date.now()) / 1000))
  })

  function startSleepTimer(minutes: number) {
    cancelSleepTimer()
    sleepTimerMode.value = 'countdown'
    sleepTimerEndMs.value = Date.now() + minutes * 60 * 1000
    _sleepTimerInterval = setInterval(() => {
      if (Date.now() >= sleepTimerEndMs.value) {
        pause()
        cancelSleepTimer()
      }
    }, 1000)
  }

  function startSleepTimerEndOfTrack() {
    cancelSleepTimer()
    sleepTimerMode.value = 'end_of_track'
    sleepTimerEndMs.value = 1 // 非零表示启用
  }

  function startSleepTimerEndOfQueue() {
    cancelSleepTimer()
    sleepTimerMode.value = 'end_of_queue'
    sleepTimerEndMs.value = 1
  }

  function cancelSleepTimer() {
    if (_sleepTimerInterval) {
      clearInterval(_sleepTimerInterval)
      _sleepTimerInterval = null
    }
    sleepTimerMode.value = null
    sleepTimerEndMs.value = 0
  }

  // 音频分析数据
  const audioLevel = ref(0)
  const beatImpulse = ref(0)

  const progress = computed(() =>
    durationMs.value > 0 ? positionMs.value / durationMs.value : 0
  )
  const currentTimeFormatted = computed(() => formatTime(positionMs.value))
  const durationFormatted = computed(() => formatTime(durationMs.value))

  // 是否已初始化事件监听
  let eventsInitialized = false
  // seek 后忽略 position 事件的时间窗口
  let seekGuardUntil = 0

  function initEvents() {
    if (eventsInitialized) return
    eventsInitialized = true

    // 监听后端播放位置更新
    listen<{ positionMs: number; durationMs: number }>('player:position', (e) => {
      // seek 后 500ms 内忽略回推的旧位置
      if (Date.now() < seekGuardUntil) return
      positionMs.value = e.payload.positionMs
      durationMs.value = e.payload.durationMs
    })

    // 监听音频电平
    listen<{ level: number; beat: number }>('player:audio-level', (e) => {
      audioLevel.value = e.payload.level
      beatImpulse.value = e.payload.beat
    })

    // 监听播放完成
    listen('player:track-ended', () => {
      // 睡眠定时器：播完当前曲目模式
      if (sleepTimerMode.value === 'end_of_track') {
        pause()
        cancelSleepTimer()
        return
      }
      // 睡眠定时器：播完当前歌单模式
      if (sleepTimerMode.value === 'end_of_queue') {
        const isLast = queueIndex.value >= queue.value.length - 1
        if (isLast && repeatMode.value !== 'all') {
          pause()
          cancelSleepTimer()
          return
        }
      }
      next()
    })
  }

  async function play(track: TrackInfo) {
    initEvents()

    // 加入队列
    if (!queue.value.find(t => t.id === track.id)) {
      queue.value.push(track)
    }
    currentTrack.value = track
    queueIndex.value = queue.value.findIndex(t => t.id === track.id)

    try {
      let dur: number
      playError.value = null
      isLoadingAudio.value = true
      audioInfo.value = null

      if (track.id.startsWith('netease:')) {
        // 网易云：先获取播放 URL，再调用 play_url
        const settings = useSettingsStore()
        const songId = parseInt(track.id.replace('netease:', ''))
        const urlResult = await invoke<{ url: string | null; bitrate: number; format: string }>('get_netease_song_url', {
          songId, quality: settings.neteaseQuality,
        })
        if (urlResult.url) {
          dur = await invoke<number>('play_url', { url: urlResult.url, durationHintMs: track.durationMs })
          audioInfo.value = {
            bitrate: urlResult.bitrate > 0 ? Math.round(urlResult.bitrate / 1000) : undefined,
            codec: urlResult.format ? urlResult.format.toUpperCase() : undefined,
            format: urlResult.format || undefined,
          }
        } else {
          throw new Error('No playback URL')
        }
      } else if (track.id.startsWith('bilibili:')) {
        // B站：id 可能是 bvid 或 avid（同步歌曲）
        const biliId = track.id.replace('bilibili:', '')
        const isAvid = /^\d+$/.test(biliId)
        // 从 album 提取 cid（Android 格式："Bilibili|{cid}"）
        const cidMatch = track.album?.match(/^Bilibili\|(\d+)/)
        const cid = cidMatch ? parseInt(cidMatch[1]) : undefined
        const result = await invoke<{ url: string; bandwidth: number; codecs: string }>('get_bili_audio_url', {
          bvid: isAvid ? '' : biliId,
          avid: isAvid ? parseInt(biliId) : null,
          cid: cid || null,
        })
        dur = await invoke<number>('play_url', { url: result.url, durationHintMs: track.durationMs })
        audioInfo.value = {
          bitrate: result.bandwidth > 0 ? Math.round(result.bandwidth / 1000) : undefined,
          codec: result.codecs || undefined,
        }
      } else if (track.id.startsWith('youtube:')) {
        // YouTube：获取音频流，选最高码率
        const videoId = track.id.replace('youtube:', '')
        const streams = await invoke<{ url: string; bitrate: number; mime_type: string }[]>('get_youtube_audio_url', { videoId })
        const best = streams?.[0]
        if (!best?.url) throw new Error('No YouTube audio stream')
        dur = await invoke<number>('play_url', { url: best.url, durationHintMs: track.durationMs || 0 })
        audioInfo.value = {
          bitrate: best.bitrate > 0 ? Math.round(best.bitrate / 1000) : undefined,
          codec: extractCodecFromMime(best.mime_type),
        }
      } else {
        // 本地文件
        dur = await invoke<number>('play_file', { path: track.audioUrl })
      }

      durationMs.value = dur || track.durationMs
      isPlaying.value = true
      isLoadingAudio.value = false
      positionMs.value = 0

      // 记录播放历史
      const history = useHistoryStore()
      history.record(track)
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      console.error('Play failed:', msg)
      playError.value = msg
      isPlaying.value = false
      isLoadingAudio.value = false
      // 通过 Toast 通知用户
      const toast = useToastStore()
      toast.error((i18n.global as any).t('player.play_failed', { msg }))
    }
  }

  async function togglePlayPause() {
    try {
      const playing = await invoke<boolean>('toggle_play_pause')
      isPlaying.value = playing
    } catch {
      isPlaying.value = !isPlaying.value
    }
  }

  async function pause() {
    try { await invoke('pause') } catch {}
    isPlaying.value = false
  }

  async function resume() {
    try { await invoke('resume') } catch {}
    isPlaying.value = true
  }

  async function seekTo(ms: number) {
    const posMs = Math.round(ms)
    positionMs.value = posMs
    seekGuardUntil = Date.now() + 1500
    try {
      await invoke('seek', { positionMs: posMs })
      positionMs.value = posMs
      seekGuardUntil = Date.now() + 800
    } catch (e) {
      console.error('Seek failed:', e)
    }
  }

  async function next() {
    if (queue.value.length === 0) return
    // 单曲循环：重播当前
    if (repeatMode.value === 'one') {
      seekTo(0)
      resume()
      return
    }
    let nextIdx: number
    if (shuffleEnabled.value) {
      // 随机选一首（排除当前）
      if (queue.value.length === 1) {
        nextIdx = 0
      } else {
        do {
          nextIdx = Math.floor(Math.random() * queue.value.length)
        } while (nextIdx === queueIndex.value)
      }
    } else {
      nextIdx = queueIndex.value + 1
      if (nextIdx >= queue.value.length) {
        if (repeatMode.value === 'all') nextIdx = 0
        else { isPlaying.value = false; return }
      }
    }
    await play(queue.value[nextIdx])
  }

  async function previous() {
    if (queue.value.length === 0) return
    if (positionMs.value > 3000) {
      seekTo(0)
    } else {
      const prevIdx = queueIndex.value > 0 ? queueIndex.value - 1 : queue.value.length - 1
      await play(queue.value[prevIdx])
    }
  }

  async function toggleRepeatMode() {
    try {
      const mode = await invoke<string>('cycle_repeat')
      repeatMode.value = mode as RepeatMode
    } catch {
      const modes: RepeatMode[] = ['off', 'all', 'one']
      const idx = modes.indexOf(repeatMode.value)
      repeatMode.value = modes[(idx + 1) % modes.length]
    }
  }

  async function toggleShuffle() {
    try {
      const enabled = await invoke<boolean>('toggle_shuffle')
      shuffleEnabled.value = enabled
    } catch {
      shuffleEnabled.value = !shuffleEnabled.value
    }
  }

  async function setVolume(vol: number) {
    volume.value = vol
    try { await invoke('set_volume', { level: vol }) } catch {}
  }

  // 播放速度
  const playbackSpeed = ref(1.0)
  async function setSpeed(spd: number) {
    playbackSpeed.value = spd
    try { await invoke('set_speed', { speed: spd }) } catch {}
  }

  // 批量替换队列并播放第一首
  function playAll(tracks: TrackInfo[]) {
    if (tracks.length === 0) return
    queue.value = [...tracks]
    queueIndex.value = 0
    play(tracks[0])
  }

  // 洗牌后替换队列并播放
  function shufflePlay(tracks: TrackInfo[]) {
    if (tracks.length === 0) return
    const shuffled = [...tracks]
    for (let i = shuffled.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]]
    }
    queue.value = shuffled
    queueIndex.value = 0
    play(shuffled[0])
  }

  // 插入到当前曲目之后
  function addToQueueNext(track: TrackInfo) {
    const existing = queue.value.findIndex(t => t.id === track.id)
    if (existing !== -1) {
      queue.value.splice(existing, 1)
      if (existing < queueIndex.value) queueIndex.value--
    }
    const idx = queueIndex.value + 1
    queue.value.splice(idx, 0, track)
  }

  // 追加到队列末尾
  function addToQueueEnd(track: TrackInfo) {
    if (!queue.value.find(t => t.id === track.id)) {
      queue.value.push(track)
    }
  }

  // 从队列移除指定索引
  function removeFromQueue(index: number) {
    if (index < 0 || index >= queue.value.length) return
    queue.value.splice(index, 1)
    if (queue.value.length === 0) {
      queueIndex.value = -1
      return
    }
    if (index < queueIndex.value) {
      queueIndex.value--
    } else if (index === queueIndex.value) {
      queueIndex.value = Math.min(queueIndex.value, queue.value.length - 1)
    }
  }

  // 清空队列
  function clearQueue() {
    queue.value = []
    queueIndex.value = -1
  }

  // 编辑当前曲目信息（仅前端状态，不持久化）
  let originalTrackInfo: TrackInfo | null = null

  function updateCurrentTrackInfo(patch: Partial<TrackInfo>) {
    if (!currentTrack.value) return
    if (!originalTrackInfo) {
      originalTrackInfo = { ...currentTrack.value }
    }
    currentTrack.value = { ...currentTrack.value, ...patch }
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

  // 用指定音质重新播放当前曲目（保持进度）
  async function replayWithQuality() {
    const track = currentTrack.value
    if (!track) return
    const pos = positionMs.value
    const wasPlaying = isPlaying.value
    await play(track)
    if (pos > 1000) {
      // 等一小段让播放开始后再 seek
      setTimeout(() => seekTo(pos), 300)
    }
  }

  return {
    isPlaying, currentTrack, positionMs, durationMs, queue, queueIndex,
    repeatMode, shuffleEnabled, volume, lyrics, playError, isLoadingAudio,
    audioLevel, beatImpulse, audioInfo,
    playbackSpeed, sleepTimerMode, sleepRemainingSeconds,
    progress, currentTimeFormatted, durationFormatted,
    play, togglePlayPause, pause, resume, seekTo, next, previous,
    toggleRepeatMode, toggleShuffle, setVolume, setSpeed,
    startSleepTimer, startSleepTimerEndOfTrack, startSleepTimerEndOfQueue, cancelSleepTimer,
    playAll, shufflePlay, addToQueueNext, addToQueueEnd, removeFromQueue, clearQueue,
    updateCurrentTrackInfo, restoreOriginalTrackInfo, hasOriginalTrackInfo, replayWithQuality,
  }
})

function formatTime(ms: number): string {
  const totalSeconds = Math.floor(Math.max(0, ms) / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}:${seconds.toString().padStart(2, '0')}`
}

/** 从 MIME type 提取编解码器名称，例如 'audio/webm; codecs="opus"' -> 'Opus' */
function extractCodecFromMime(mime: string): string | undefined {
  // 尝试从 codecs 参数提取
  const codecsMatch = mime.match(/codecs="?([^";\s]+)"?/)
  if (codecsMatch) {
    const raw = codecsMatch[1].split('.')[0] // 去除 profile，如 mp4a.40.2 -> mp4a
    const codecMap: Record<string, string> = {
      opus: 'Opus', vorbis: 'Vorbis', mp4a: 'AAC', flac: 'FLAC', mp3: 'MP3',
    }
    return codecMap[raw.toLowerCase()] ?? raw.toUpperCase()
  }
  // 从 MIME 主类型推断
  const typeMatch = mime.match(/^audio\/(\w+)/)
  if (typeMatch) {
    const codecMap: Record<string, string> = {
      webm: 'WebM', mp4: 'AAC', mpeg: 'MP3', ogg: 'Vorbis', flac: 'FLAC', wav: 'WAV',
    }
    return codecMap[typeMatch[1].toLowerCase()] ?? typeMatch[1].toUpperCase()
  }
  return undefined
}
