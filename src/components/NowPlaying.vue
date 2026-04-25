<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { usePlayerStore, displayAlbum } from '@/stores/player'
import { useRecommendStore } from '@/stores/recommend'
import { useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { extractPalette, type PaletteResult } from '@/utils/paletteExtractor'
import HyperBackground from './HyperBackground.vue'
import CoverBlurBackground from './CoverBlurBackground.vue'
import WaveformSlider from './WaveformSlider.vue'
import LyricsView from './LyricsView.vue'
import QueuePanel from './QueuePanel.vue'
import AddToPlaylistDialog from './AddToPlaylistDialog.vue'

const emit = defineEmits<{ collapse: [] }>()
const player = usePlayerStore()
const recommend = useRecommendStore()
const settings = useSettingsStore()
const toast = useToastStore()
const { t } = useI18n()
const showLyrics = ref(false)
const coverLoadError = ref(false)
const showVolumeSlider = ref(false)
const showQueue = ref(false)
const showAddToPlaylist = ref(false)
const showSpeedMenu = ref(false)
const showSleepMenu = ref(false)
const showMoreSheet = ref(false)
const fetchedLyrics = ref<any[]>([])
const isFetchingLyrics = ref(false)

// 歌词拖动预览状态
const previewPositionMs = ref<number | null>(null)
let previewConvergeTimer: ReturnType<typeof setTimeout> | null = null

function onSliderPreview(progress: number) {
  previewPositionMs.value = progress * player.durationMs
}

function onSliderPreviewEnd() {
  // 松手后保持预览 280ms，等待播放位置追上
  if (previewConvergeTimer) clearTimeout(previewConvergeTimer)
  previewConvergeTimer = setTimeout(() => {
    previewPositionMs.value = null
  }, 280)
}

// 从封面提取的动态颜色（归一化 RGBA）
const extractedColors = ref<[number[], number[], number[], number[]]>([
  [0.4, 0.31, 0.64, 1],
  [0.49, 0.36, 0.75, 1],
  [0.56, 0.49, 0.69, 1],
  [0.29, 0.24, 0.43, 1],
])
const paletteResult = ref<PaletteResult | null>(null)

// 使用 Median-Cut 调色板算法从封面提取主色
function extractColorsFromCover(url: string) {
  const img = new Image()
  img.crossOrigin = 'anonymous'
  img.referrerPolicy = 'no-referrer'
  img.onload = () => {
    try {
      const canvas = document.createElement('canvas')
      const size = 64
      canvas.width = size
      canvas.height = size
      const ctx = canvas.getContext('2d', { willReadFrequently: true })!
      ctx.drawImage(img, 0, 0, size, size)
      const imageData = ctx.getImageData(0, 0, size, size)

      const palette = extractPalette(imageData, 16)
      paletteResult.value = palette

      // 归一化到 0..1 RGBA
      const toNorm = (c: [number, number, number]): number[] => [
        c[0] / 255,
        c[1] / 255,
        c[2] / 255,
        1,
      ]
      extractedColors.value = [
        toNorm(palette.dominant),
        toNorm(palette.lightVibrant),
        toNorm(palette.muted),
        toNorm(palette.darkMuted),
      ] as [number[], number[], number[], number[]]
    } catch (e) {
      console.error('[NowPlaying] color extraction failed:', e)
    }
  }
  img.onerror = () => {
    console.error('[NowPlaying] cover image load failed:', url)
  }
  img.src = url
}

// 封面变化时提取颜色
watch(() => player.currentTrack?.coverUrl, (url) => {
  if (url) {
    extractColorsFromCover(url)
  }
}, { immediate: true })

function onSeek(progress: number) {
  player.seekTo(Math.round(progress * player.durationMs))
}

// 当前歌曲的网易云 ID（用于网易云收藏 API）
const currentNeteaseId = computed(() => {
  const id = player.currentTrack?.id
  if (id?.startsWith('netease:')) return parseInt(id.replace('netease:', ''))
  return null
})

// 本地"我喜欢的音乐"歌单 ID
const LIKED_PLAYLIST_NAMES = ['我喜欢的音乐', '我喜歡的音樂', 'お気に入りの曲', 'Liked Songs', 'My Favorite Music']
const localLikedPlaylistId = ref<number | null>(null)
const localLikedTrackIds = ref<Set<string>>(new Set())

// 加载本地"我喜欢的音乐"歌单数据
async function loadLocalLikedPlaylist() {
  try {
    const playlists = await invoke<any[]>('list_playlists')
    const liked = playlists.find(p => LIKED_PLAYLIST_NAMES.includes(p.name))
    if (liked) {
      localLikedPlaylistId.value = liked.id
      const tracks = await invoke<any[]>('get_playlist_tracks', { id: liked.id })
      localLikedTrackIds.value = new Set(tracks.map((t: any) => t.id))
    }
  } catch (e) {
    console.error('loadLocalLikedPlaylist:', e)
  }
}

// 收藏状态：检查本地"我喜欢的音乐"歌单
const isFavorite = computed(() => {
  const trackId = player.currentTrack?.id
  if (!trackId) return false
  return localLikedTrackIds.value.has(trackId)
})

async function toggleFavorite() {
  const track = player.currentTrack
  if (!track) return

  // 网易云歌曲：同时调用 Netease API
  const nid = currentNeteaseId.value
  if (nid) {
    recommend.toggleLikeSong(nid, !isFavorite.value).catch(() => {})
  }

  // 本地歌单操作
  if (!localLikedPlaylistId.value) {
    // 自动创建"我喜欢的音乐"歌单
    try {
      const created = await invoke<any>('create_playlist', { name: '我喜欢的音乐' })
      localLikedPlaylistId.value = created.id
    } catch (e) {
      console.error('create liked playlist:', e)
      return
    }
  }

  try {
    if (isFavorite.value) {
      await invoke('remove_from_playlist', {
        playlistId: localLikedPlaylistId.value,
        trackId: track.id,
      })
      localLikedTrackIds.value.delete(track.id)
    } else {
      await invoke('add_to_playlist', {
        playlistId: localLikedPlaylistId.value,
        track: {
          id: track.id,
          title: track.title,
          artist: track.artist,
          album: track.album,
          duration_ms: track.durationMs,
          cover_url: track.coverUrl || null,
          url: track.audioUrl || '',
          source: 'local',
        },
      })
      localLikedTrackIds.value.add(track.id)
    }
  } catch (e) {
    console.error('toggleFavorite local:', e)
  }
}

// 曲目切换或打开时加载
watch(() => player.currentTrack?.id, () => {
  loadLocalLikedPlaylist()
}, { immediate: true })

// 睡眠定时器选项
const sleepOptions = computed(() => [
  { label: t('player.sleep_15'), value: 15 },
  { label: t('player.sleep_30'), value: 30 },
  { label: t('player.sleep_45'), value: 45 },
  { label: t('player.sleep_60'), value: 60 },
  { label: t('player.sleep_90'), value: 90 },
  { label: t('player.sleep_end_of_track'), value: -1 },
  { label: t('player.sleep_end_of_queue'), value: -2 },
])

function handleSleepOption(value: number) {
  if (value === -1) {
    player.startSleepTimerEndOfTrack()
  } else if (value === -2) {
    player.startSleepTimerEndOfQueue()
  } else {
    player.startSleepTimer(value)
  }
}

function formatSleepRemaining(seconds: number): string {
  if (seconds <= 0) return ''
  const m = Math.floor(seconds / 60)
  const s = seconds % 60
  return m > 0 ? `${m}:${s.toString().padStart(2, '0')}` : `${s}s`
}

// 封面 URL
const coverUrl = computed(() => player.currentTrack?.coverUrl || '')

// 封面加载错误时重置
watch(() => player.currentTrack?.id, () => {
  coverLoadError.value = false
})

// 当曲目切换时自动获取歌词
watch(() => player.currentTrack?.id, async (id) => {
  if (!id || !player.currentTrack) {
    fetchedLyrics.value = []
    return
  }

  isFetchingLyrics.value = true
  try {
    const track = player.currentTrack
    const neteaseId = id.startsWith('netease:') ? parseInt(id.replace('netease:', '')) : undefined

    const lyrics = await invoke<any[]>('fetch_lyrics', {
      title: track.title,
      artist: track.artist,
      durationSecs: Math.floor(track.durationMs / 1000),
      audioPath: track.audioUrl || null,
      neteaseId: neteaseId || null,
    })

    fetchedLyrics.value = lyrics.map(l => ({
      startMs: l.start_ms,
      durationMs: l.duration_ms,
      words: (l.words || []).map((w: any) => ({
        startMs: w.start_ms, durationMs: w.duration_ms, text: w.text,
      })),
      text: l.text,
      translation: l.translation || undefined,
    }))
  } catch (e) {
    console.error('Fetch lyrics failed:', e)
    fetchedLyrics.value = []
  } finally {
    isFetchingLyrics.value = false
  }
}, { immediate: true })

// --- 唱片旋转（JS 驱动，停止时保持角度 + 缓动） ---
const discRef = ref<HTMLDivElement>()
let discAngle = 0            // 当前累计角度（度）
let discAnimFrame = 0
let discLastTime = 0
const DISC_RPM = 2.4         // 每秒转过的度数 = 360 / 25s ≈ 14.4 deg/s
const DEG_PER_MS = 360 / 25000

function animateDisc(timestamp: number) {
  if (!discLastTime) discLastTime = timestamp
  const dt = timestamp - discLastTime
  discLastTime = timestamp

  if (player.isPlaying) {
    discAngle = (discAngle + DEG_PER_MS * dt) % 360
  }
  if (discRef.value) {
    discRef.value.style.transform = `rotate(${discAngle}deg)`
  }
  discAnimFrame = requestAnimationFrame(animateDisc)
}

onMounted(() => {
  discAnimFrame = requestAnimationFrame(animateDisc)
})
onUnmounted(() => {
  cancelAnimationFrame(discAnimFrame)
})

// --- 右键菜单（歌曲名/歌手复制 + 封面保存） ---
const contextMenu = ref({ show: false, x: 0, y: 0, type: '' as 'title' | 'artist' | 'cover' })

function openContextMenu(e: MouseEvent, type: 'title' | 'artist' | 'cover') {
  e.preventDefault()
  const x = Math.min(e.clientX, window.innerWidth - 220)
  const y = Math.min(e.clientY, window.innerHeight - 100)
  contextMenu.value = { show: true, x, y, type }
}

function closeContextMenu() {
  contextMenu.value.show = false
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text)
    toast.success(t('player.copied'))
  } catch {
    toast.error(t('player.copy_failed'))
  }
  closeContextMenu()
}

async function saveCoverArt() {
  closeContextMenu()
  const url = player.currentTrack?.coverUrl
  if (!url) return

  try {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const filePath = await save({
      defaultPath: `${player.currentTrack?.title || 'cover'}.jpg`,
      filters: [{ name: 'Image', extensions: ['jpg', 'png', 'webp'] }],
    })
    if (!filePath) return

    const response = await fetch(url, { referrerPolicy: 'no-referrer' })
    const blob = await response.blob()
    const arrayBuffer = await blob.arrayBuffer()

    await invoke('save_file_bytes', {
      path: filePath,
      data: Array.from(new Uint8Array(arrayBuffer)),
    })
    toast.success(t('player.cover_saved'))
  } catch (e) {
    console.error('Save cover failed:', e)
    toast.error(t('player.cover_save_failed'))
  }
}

const displayLyrics = computed(() => {
  if (player.lyrics.length) return player.lyrics
  if (fetchedLyrics.value.length) return fetchedLyrics.value
  return []
})

// 更多选项面板子视图
const moreSheetView = ref<'main' | 'offset' | 'fontsize' | 'speed' | 'search' | 'editinfo' | 'quality'>('main')

// 关闭更多选项面板时重置子视图
watch(showMoreSheet, (v) => { if (!v) moreSheetView.value = 'main' })

// --- 获取歌曲信息（搜索） ---
const searchQuery = ref('')
const searchResults = ref<any[]>([])
const isSearching = ref(false)

async function doSearch() {
  const q = searchQuery.value.trim()
  if (!q) return
  isSearching.value = true
  searchResults.value = []
  try {
    const results = await invoke<any[]>('search', { query: q, platform: 'netease' })
    searchResults.value = results
  } catch (e) {
    console.error('Search failed:', e)
  } finally {
    isSearching.value = false
  }
}

function applySearchResult(result: any) {
  player.updateCurrentTrackInfo({
    title: result.title || player.currentTrack?.title,
    artist: result.artist || player.currentTrack?.artist,
    coverUrl: result.cover_url || player.currentTrack?.coverUrl,
  })
  toast.success(t('player.info_applied'))
  moreSheetView.value = 'main'
}

// --- 编辑歌曲信息 ---
const editTitle = ref('')
const editArtist = ref('')
const editCoverUrl = ref('')

function openEditInfo() {
  editTitle.value = player.currentTrack?.title || ''
  editArtist.value = player.currentTrack?.artist || ''
  editCoverUrl.value = player.currentTrack?.coverUrl || ''
  moreSheetView.value = 'editinfo'
}

function saveEditInfo() {
  player.updateCurrentTrackInfo({
    title: editTitle.value,
    artist: editArtist.value,
    coverUrl: editCoverUrl.value,
  })
  toast.success(t('player.info_applied'))
  moreSheetView.value = 'main'
}

function restoreInfo() {
  player.restoreOriginalTrackInfo()
  toast.success(t('player.info_restored'))
  moreSheetView.value = 'main'
}

// --- 音质切换 ---
const currentSource = computed(() => {
  const id = player.currentTrack?.id || ''
  if (id.startsWith('netease:')) return 'netease'
  if (id.startsWith('bilibili:')) return 'bilibili'
  if (id.startsWith('youtube:')) return 'youtube'
  return 'local'
})

const neteaseQualities = [
  { key: 'standard', label: 'settings.q_standard' },
  { key: 'higher', label: 'settings.q_high' },
  { key: 'exhigh', label: 'settings.q_exhigh' },
  { key: 'lossless', label: 'settings.q_lossless' },
  { key: 'hires', label: 'settings.q_hires' },
  { key: 'jyeffect', label: 'settings.q_surround' },
  { key: 'sky', label: 'settings.q_sky' },
  { key: 'jymaster', label: 'settings.q_master' },
]

const youtubeQualities = [
  { key: 'low', label: 'settings.q_low' },
  { key: 'medium', label: 'settings.q_medium' },
  { key: 'high', label: 'settings.q_high_yt' },
  { key: 'very_high', label: 'settings.q_very_high' },
]

async function switchQuality(key: string) {
  if (currentSource.value === 'netease') {
    settings.neteaseQuality = key
  } else if (currentSource.value === 'youtube') {
    settings.youtubeQuality = key
  }
  showMoreSheet.value = false
  await player.replayWithQuality()
}

function currentQualityKey(): string {
  if (currentSource.value === 'netease') return settings.neteaseQuality
  if (currentSource.value === 'youtube') return settings.youtubeQuality
  return ''
}

// 分享歌曲
async function shareSong() {
  const track = player.currentTrack
  if (!track) return
  // 构建分享文本
  let url = ''
  if (track.id.startsWith('netease:')) {
    const nid = track.id.replace('netease:', '')
    url = `https://music.163.com/song?id=${nid}`
  } else if (track.id.startsWith('bilibili:')) {
    const bid = track.id.replace('bilibili:', '')
    url = `https://www.bilibili.com/video/${bid}`
  } else if (track.id.startsWith('youtube:')) {
    const vid = track.id.replace('youtube:', '')
    url = `https://music.youtube.com/watch?v=${vid}`
  }
  const text = url
    ? `${track.title} - ${track.artist}\n${url}`
    : `${track.title} - ${track.artist}`
  try {
    await navigator.clipboard.writeText(text)
    toast.success(t('player.share_copied'))
  } catch {
    toast.error(t('player.copy_failed'))
  }
  showMoreSheet.value = false
}

// 专辑名
const albumName = computed(() => {
  const album = player.currentTrack?.album || ''
  return displayAlbum(album)
})

// 进度条下方音质信息
const audioInfoDisplay = computed(() => {
  const info = player.audioInfo
  if (!info) return ''
  const parts: string[] = []
  if (settings.showAudioCodec && info.codec) parts.push(info.codec)
  if (settings.showQualitySwitch && info.bitrate) parts.push(`${info.bitrate}kbps`)
  return parts.join(' \u00B7 ')
})

// AccentBackdrop 底色（对齐 Android：主色降饱和调暗后铺底）
const accentBgStyle = computed(() => {
  const bg = paletteResult.value?.accentBg
  if (!bg) return { background: 'rgb(18, 18, 18)' }
  return { background: `rgb(${bg[0]}, ${bg[1]}, ${bg[2]})` }
})
</script>

<template>
  <div class="now-playing">
    <!-- AccentBackdrop 底色层（对齐 Android：主色降饱和+调暗） -->
    <div class="np-bg-solid" :style="accentBgStyle" />
    <!-- 动态背景：封面模糊 OR WebGL 着色器，互斥 -->
    <CoverBlurBackground
      v-if="settings.coverBlurBg"
      :cover-url="coverUrl"
      :blur-amount="settings.coverBlurAmount * 30"
      :darken-alpha="settings.coverBlurDarken"
    />
    <HyperBackground
      v-else-if="settings.dynamicBackground"
      :music-level="settings.audioReactive ? player.audioLevel : 0"
      :beat-impulse="settings.audioReactive ? player.beatImpulse : 0"
      :colors="extractedColors"
      :is-dark="true"
      :light-offset="paletteResult?.lightOffset ?? 0"
      :saturate-offset="paletteResult?.saturateOffset ?? 0"
    />
    <div class="np-scrim" />

    <!-- 顶栏 -->
    <header class="np-header">
      <button class="np-icon-btn" @click.stop="emit('collapse')">
        <span class="material-symbols-rounded">keyboard_arrow_down</span>
      </button>
      <div class="np-header-center">
        <span class="np-from-label">{{ t('player.now_playing') }}</span>
        <span class="np-from-name">{{ displayAlbum(player.currentTrack?.album || '') }}</span>
      </div>
      <button class="np-icon-btn" @click="showMoreSheet = !showMoreSheet">
        <span class="material-symbols-rounded">more_vert</span>
      </button>
    </header>

    <!-- 双栏 -->
    <div class="np-body">
      <!-- 左侧 -->
      <section class="np-left">
        <div class="cover-wrap" @contextmenu="openContextMenu($event, 'cover')">
          <div ref="discRef" class="cover-disc">
            <div class="cover-inner">
              <img
                v-if="coverUrl && !coverLoadError"
                :src="coverUrl"
                referrerpolicy="no-referrer"
                class="cover-img"
                @error="coverLoadError = true"
              />
              <span v-else class="material-symbols-rounded filled" style="font-size: 48px; opacity: 0.35">music_note</span>
            </div>
            <div class="cover-groove" />
            <div class="cover-hole" />
          </div>
        </div>

        <div class="np-info">
          <h2 class="np-title" @contextmenu="openContextMenu($event, 'title')">{{ player.currentTrack?.title || t('player.not_playing') }}</h2>
          <p class="np-artist" @contextmenu="openContextMenu($event, 'artist')">{{ player.currentTrack?.artist || '' }}</p>
        </div>

        <div class="np-slider-area">
          <WaveformSlider
            :progress="player.progress"
            :is-playing="player.isPlaying"
            @seek="onSeek"
            @preview="onSliderPreview"
            @preview-end="onSliderPreviewEnd"
          />
          <div class="np-time">
            <span>{{ player.currentTimeFormatted }}</span>
            <span>{{ player.durationFormatted }}</span>
          </div>
          <div v-if="player.audioInfo" class="np-audio-info">
            <span v-if="settings.showAudioCodec && player.audioInfo.codec" class="np-audio-codec">{{ player.audioInfo.codec }}</span>
            <template v-if="settings.showAudioCodec && player.audioInfo.codec && settings.showQualitySwitch && player.audioInfo.bitrate"> · </template>
            <span v-if="settings.showQualitySwitch && player.audioInfo.bitrate">{{ player.audioInfo.bitrate }}kbps</span>
          </div>
        </div>

        <div class="np-controls">
          <button
            class="ctrl-btn"
            :class="{ active: player.repeatMode !== 'off' }"
            @click="player.toggleRepeatMode()"
          >
            <span class="material-symbols-rounded">{{ player.repeatMode === 'one' ? 'repeat_one' : 'repeat' }}</span>
          </button>

          <button class="ctrl-btn" @click="player.previous()">
            <span class="material-symbols-rounded filled" style="font-size: 30px">skip_previous</span>
          </button>

          <!-- 播放/暂停 带动画 -->
          <button class="play-btn" @click="player.togglePlayPause()" :disabled="player.isLoadingAudio">
            <transition name="play-icon" mode="out-in">
              <span
                v-if="player.isLoadingAudio"
                class="material-symbols-rounded spinning play-icon-inner"
                key="loading"
              >progress_activity</span>
              <span
                v-else
                class="material-symbols-rounded filled play-icon-inner"
                :key="player.isPlaying ? 'pause' : 'play'"
              >{{ player.isPlaying ? 'pause' : 'play_arrow' }}</span>
            </transition>
          </button>

          <button class="ctrl-btn" @click="player.next()">
            <span class="material-symbols-rounded filled" style="font-size: 30px">skip_next</span>
          </button>

          <button
            class="ctrl-btn"
            :class="{ active: player.shuffleEnabled }"
            @click="player.toggleShuffle()"
          >
            <span class="material-symbols-rounded">shuffle</span>
          </button>
        </div>

        <!-- 工具栏 -->
        <div class="np-toolbar">
          <button
            class="tool-btn"
            :class="{ active: isFavorite }"
            :disabled="!player.currentTrack"
            @click="toggleFavorite"
          >
            <span class="material-symbols-rounded" :class="{ filled: isFavorite }">favorite</span>
          </button>
          <button class="tool-btn" @click="showAddToPlaylist = true">
            <span class="material-symbols-rounded">add_circle</span>
          </button>
          <button class="tool-btn" @click="showQueue = true">
            <span class="material-symbols-rounded">queue_music</span>
          </button>
          <!-- 播放速度 -->
          <div class="speed-wrap">
            <button class="tool-btn" :class="{ active: showSpeedMenu }" @click="showSpeedMenu = !showSpeedMenu">
              <span class="speed-label">{{ player.playbackSpeed === 1 ? '1x' : player.playbackSpeed + 'x' }}</span>
            </button>
            <div v-if="showSpeedMenu" class="speed-popover" @mouseleave="showSpeedMenu = false">
              <button
                v-for="spd in [0.5, 0.75, 0.85, 0.9, 1, 1.1, 1.25, 1.5, 2]"
                :key="spd"
                class="speed-option"
                :class="{ active: player.playbackSpeed === spd }"
                @click="player.setSpeed(spd); showSpeedMenu = false"
              >
                {{ spd }}x
              </button>
            </div>
          </div>
          <div class="volume-wrap">
            <button class="tool-btn" @click="showVolumeSlider = !showVolumeSlider">
              <span class="material-symbols-rounded">{{ player.volume === 0 ? 'volume_off' : player.volume < 0.5 ? 'volume_down' : 'volume_up' }}</span>
            </button>
            <div v-if="showVolumeSlider" class="volume-popover" @mouseleave="showVolumeSlider = false">
              <input
                type="range"
                min="0"
                max="1"
                step="0.01"
                :value="player.volume"
                class="volume-slider"
                @input="player.setVolume(parseFloat(($event.target as HTMLInputElement).value))"
              />
              <span class="volume-label">{{ Math.round(player.volume * 100) }}%</span>
            </div>
          </div>
          <!-- 睡眠定时器 -->
          <div class="sleep-wrap">
            <button
              class="tool-btn"
              :class="{ active: player.sleepTimerMode || showSleepMenu }"
              @click="showSleepMenu = !showSleepMenu"
            >
              <span class="material-symbols-rounded">timer</span>
              <span v-if="player.sleepRemainingSeconds > 0" class="sleep-badge">
                {{ formatSleepRemaining(player.sleepRemainingSeconds) }}
              </span>
            </button>
            <div v-if="showSleepMenu" class="sleep-popover" @mouseleave="showSleepMenu = false">
              <button
                v-for="opt in sleepOptions"
                :key="opt.value"
                class="sleep-option"
                @click="handleSleepOption(opt.value); showSleepMenu = false"
              >
                {{ opt.label }}
              </button>
              <button
                v-if="player.sleepTimerMode"
                class="sleep-option cancel"
                @click="player.cancelSleepTimer(); showSleepMenu = false"
              >
                {{ t('player.sleep_off') }}
              </button>
            </div>
          </div>
        </div>
      </section>

      <!-- 右侧歌词 -->
      <section class="np-right">
        <LyricsView
          v-if="displayLyrics.length > 0"
          :lyrics="displayLyrics"
          :current-time-ms="player.positionMs"
          :preview-time-ms="previewPositionMs"
          :is-playing="player.isPlaying"
          @seek="(ms) => player.seekTo(ms)"
        />
        <div v-else class="lyrics-empty">
          <span class="material-symbols-rounded" style="font-size: 36px">lyrics</span>
          <p>{{ t('player.no_lyrics') }}</p>
        </div>
      </section>
    </div>

    <!-- 播放队列面板 -->
    <QueuePanel v-if="showQueue" @close="showQueue = false" />
    <AddToPlaylistDialog v-model:open="showAddToPlaylist" :track="player.currentTrack" />

    <!-- 更多选项面板（对齐 Android MoreOptionsSheet） -->
    <Teleport to="body">
      <div v-if="showMoreSheet" class="np-more-overlay" @click="showMoreSheet = false">
        <div class="np-more-sheet" @click.stop>

          <!-- 主菜单（对齐 Android MoreOptionsSheet 顺序） -->
          <template v-if="moreSheetView === 'main'">
            <h4 class="np-more-title">{{ t('player.more_options') }}</h4>

            <!-- 获取歌曲信息 -->
            <button class="np-more-list-item" @click="searchQuery = player.currentTrack?.title || ''; searchResults = []; moreSheetView = 'search'">
              <span class="material-symbols-rounded">info</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.get_info') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 编辑歌曲信息 -->
            <button class="np-more-list-item" @click="openEditInfo">
              <span class="material-symbols-rounded">edit</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.edit_info') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 音质切换（仅在线来源显示） -->
            <button v-if="currentSource !== 'local'" class="np-more-list-item" @click="moreSheetView = 'quality'">
              <span class="material-symbols-rounded">music_note</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.quality_switch') }}</span>
                <span class="np-more-list-desc">{{ player.audioInfo?.codec }} · {{ player.audioInfo?.bitrate }}kbps</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 音频效果 -->
            <button class="np-more-list-item" @click="moreSheetView = 'speed'">
              <span class="material-symbols-rounded">tune</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.audio_effects') }}</span>
                <span class="np-more-list-desc">{{ t('player.audio_effects_desc') }}</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 歌词偏移 -->
            <button class="np-more-list-item" @click="moreSheetView = 'offset'">
              <span class="material-symbols-rounded">timer</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.lyric_offset') }}</span>
                <span class="np-more-list-desc">{{ settings.cloudMusicOffset > 0 ? '+' : '' }}{{ settings.cloudMusicOffset }}ms</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 歌词字号 -->
            <button class="np-more-list-item" @click="moreSheetView = 'fontsize'">
              <span class="material-symbols-rounded">format_size</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.font_scale') }}</span>
                <span class="np-more-list-desc">{{ Math.round(settings.lyricFontScale * 100) }}%</span>
              </div>
              <span class="material-symbols-rounded np-more-chevron">chevron_right</span>
            </button>

            <!-- 分享 -->
            <button class="np-more-list-item" @click="shareSong">
              <span class="material-symbols-rounded">share</span>
              <div class="np-more-list-info">
                <span class="np-more-list-headline">{{ t('player.share') }}</span>
              </div>
            </button>
          </template>

          <!-- 子视图：歌词偏移 -->
          <template v-else-if="moreSheetView === 'offset'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="moreSheetView = 'main'">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.lyric_offset') }}</h4>
            </div>
            <div class="np-more-item">
              <div class="np-more-row">
                <input type="range" min="-2000" max="2000" step="50"
                  :value="settings.cloudMusicOffset"
                  class="np-more-slider"
                  @input="settings.cloudMusicOffset = parseInt(($event.target as HTMLInputElement).value)"
                />
                <span class="np-offset-value" :class="{ positive: settings.cloudMusicOffset > 0, negative: settings.cloudMusicOffset < 0 }">
                  {{ settings.cloudMusicOffset > 0 ? '+' : '' }}{{ settings.cloudMusicOffset }}ms
                </span>
              </div>
            </div>
          </template>

          <!-- 子视图：字号 -->
          <template v-else-if="moreSheetView === 'fontsize'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="moreSheetView = 'main'">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.font_scale') }}</h4>
            </div>
            <div class="np-more-item">
              <div class="np-more-row">
                <input type="range" min="0.6" max="1.6" step="0.05"
                  :value="settings.lyricFontScale"
                  class="np-more-slider"
                  @input="settings.lyricFontScale = parseFloat(($event.target as HTMLInputElement).value)"
                />
                <span class="np-offset-value">{{ Math.round(settings.lyricFontScale * 100) }}%</span>
              </div>
              <p class="np-more-preview" :style="{ fontSize: `${24 * settings.lyricFontScale}px` }">
                {{ t('player.font_preview') }}
              </p>
            </div>
          </template>

          <!-- 子视图：播放速度 -->
          <template v-else-if="moreSheetView === 'speed'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="moreSheetView = 'main'">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.playback_speed') }}</h4>
            </div>
            <div class="np-more-speed-grid">
              <button
                v-for="spd in [0.5, 0.75, 0.85, 0.9, 1, 1.1, 1.25, 1.5, 2]"
                :key="spd"
                class="np-more-speed-btn"
                :class="{ active: player.playbackSpeed === spd }"
                @click="player.setSpeed(spd)"
              >{{ spd }}x</button>
            </div>
          </template>

          <!-- 子视图：获取歌曲信息 -->
          <template v-else-if="moreSheetView === 'search'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="moreSheetView = 'main'">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.get_info') }}</h4>
            </div>
            <div class="np-more-search-bar">
              <input
                v-model="searchQuery"
                class="np-more-input"
                :placeholder="t('player.search_song')"
                @keydown.enter="doSearch"
              />
              <button class="np-more-search-btn" @click="doSearch" :disabled="isSearching">
                <span class="material-symbols-rounded">search</span>
              </button>
            </div>
            <div v-if="isSearching" class="np-more-status">{{ t('player.searching') }}</div>
            <div v-else-if="searchResults.length === 0 && searchQuery" class="np-more-status">{{ t('player.no_results') }}</div>
            <div class="np-more-search-results">
              <button
                v-for="(r, ri) in searchResults"
                :key="ri"
                class="np-more-search-item"
                @click="applySearchResult(r)"
              >
                <img v-if="r.cover_url" :src="r.cover_url" class="np-more-search-cover" referrerpolicy="no-referrer" />
                <div class="np-more-search-info">
                  <span class="np-more-search-title">{{ r.title }}</span>
                  <span class="np-more-search-artist">{{ r.artist }}</span>
                </div>
                <span class="np-more-search-source">{{ r.source }}</span>
              </button>
            </div>
          </template>

          <!-- 子视图：编辑歌曲信息 -->
          <template v-else-if="moreSheetView === 'editinfo'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="moreSheetView = 'main'">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.edit_info') }}</h4>
            </div>
            <div class="np-more-form">
              <label class="np-more-form-label">{{ t('player.song_title') }}</label>
              <input v-model="editTitle" class="np-more-input" />

              <label class="np-more-form-label">{{ t('player.artist_name') }}</label>
              <input v-model="editArtist" class="np-more-input" />

              <label class="np-more-form-label">{{ t('player.cover_url_label') }}</label>
              <input v-model="editCoverUrl" class="np-more-input" />

              <div class="np-more-form-actions">
                <button class="np-more-form-btn primary" @click="saveEditInfo">
                  <span class="material-symbols-rounded">check</span>
                  {{ t('common.save') }}
                </button>
                <button v-if="player.hasOriginalTrackInfo()" class="np-more-form-btn" @click="restoreInfo">
                  <span class="material-symbols-rounded">restore</span>
                  {{ t('player.restore_original') }}
                </button>
              </div>
            </div>
          </template>

          <!-- 子视图：音质切换 -->
          <template v-else-if="moreSheetView === 'quality'">
            <div class="np-more-sub-header">
              <button class="np-more-back" @click="moreSheetView = 'main'">
                <span class="material-symbols-rounded">arrow_back</span>
              </button>
              <h4 class="np-more-title">{{ t('player.quality_switch') }}</h4>
            </div>
            <div v-if="currentSource === 'netease'" class="np-more-quality-list">
              <button
                v-for="q in neteaseQualities"
                :key="q.key"
                class="np-more-quality-item"
                :class="{ active: currentQualityKey() === q.key }"
                @click="switchQuality(q.key)"
              >
                <span>{{ t(q.label) }}</span>
                <span v-if="currentQualityKey() === q.key" class="material-symbols-rounded" style="font-size: 18px">check</span>
              </button>
            </div>
            <div v-else-if="currentSource === 'youtube'" class="np-more-quality-list">
              <button
                v-for="q in youtubeQualities"
                :key="q.key"
                class="np-more-quality-item"
                :class="{ active: currentQualityKey() === q.key }"
                @click="switchQuality(q.key)"
              >
                <span>{{ t(q.label) }}</span>
                <span v-if="currentQualityKey() === q.key" class="material-symbols-rounded" style="font-size: 18px">check</span>
              </button>
            </div>
            <div v-else class="np-more-status">{{ t('player.not_available') }}</div>
          </template>

        </div>
      </div>
    </Teleport>

    <!-- 右键菜单 -->
    <Teleport to="body">
      <div v-if="contextMenu.show" class="np-ctx-overlay" @click="closeContextMenu" @contextmenu.prevent="closeContextMenu">
        <div class="np-ctx-menu" :style="{ left: contextMenu.x + 'px', top: contextMenu.y + 'px' }">
          <template v-if="contextMenu.type === 'title'">
            <button class="np-ctx-item" @click="copyText(player.currentTrack?.title || '')">
              <span class="material-symbols-rounded" style="font-size: 20px">content_copy</span>
              <span>{{ t('player.copy_title') }}</span>
            </button>
          </template>
          <template v-else-if="contextMenu.type === 'artist'">
            <button class="np-ctx-item" @click="copyText(player.currentTrack?.artist || '')">
              <span class="material-symbols-rounded" style="font-size: 20px">content_copy</span>
              <span>{{ t('player.copy_artist') }}</span>
            </button>
          </template>
          <template v-else-if="contextMenu.type === 'cover'">
            <button class="np-ctx-item" @click="saveCoverArt">
              <span class="material-symbols-rounded" style="font-size: 20px">save</span>
              <span>{{ t('player.save_cover') }}</span>
            </button>
          </template>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped lang="scss">
.now-playing {
  position: fixed;
  inset: 0;
  z-index: 200;
  display: flex;
  flex-direction: column;
  // 确保完全不透明
  isolation: isolate;
}

// 纯色底层 — 由 accentBgStyle 动态控制颜色
.np-bg-solid {
  position: absolute;
  inset: 0;
  z-index: -1;
  transition: background 0.8s ease;
}

.np-scrim {
  position: absolute;
  inset: 0;
  background: linear-gradient(
    180deg,
    rgba(0,0,0,0.05) 0%,
    rgba(0,0,0,0.12) 40%,
    rgba(0,0,0,0.25) 100%
  );
  z-index: 1;
  pointer-events: none;
}

/* 顶栏 */
.np-header {
  position: relative;
  z-index: 2;
  display: flex;
  align-items: center;
  padding: 14px 24px;
  gap: 12px;
  flex-shrink: 0;
}

.np-header-center {
  flex: 1;
  text-align: center;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.np-from-label {
  font-size: 11px;
  font-weight: 600;
  color: rgba(255,255,255,0.45);
  text-transform: uppercase;
  letter-spacing: 1.2px;
}

.np-from-name {
  font-size: 13px;
  font-weight: 600;
  color: rgba(255,255,255,0.75);
}

.np-icon-btn {
  width: 46px;
  height: 46px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgba(255,255,255,0.8);
  transition: background 150ms;

  &:hover { background: rgba(255,255,255,0.08); }
  .material-symbols-rounded { font-size: 28px; }
}

/* 双栏主体 — 五五分 */
.np-body {
  position: relative;
  z-index: 2;
  flex: 1;
  display: flex;
  padding: 0 0 20px;
  gap: 0;
  overflow: hidden;
  min-height: 0;
}

.np-left {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 0 40px;
}

.np-right {
  flex: 1;
  border-radius: 20px;
  background: transparent;
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
  border: none;
  display: flex;
  align-items: stretch;
  overflow: hidden;
  padding: 0 40px 0 0;
}

/* 封面 */
.cover-wrap {
  width: 280px;
  height: 280px;
  filter: drop-shadow(0 16px 48px rgba(0,0,0,0.5));
  flex-shrink: 0;
}

.cover-disc {
  width: 100%;
  height: 100%;
  border-radius: 50%;
  background: conic-gradient(
    from 0deg,
    #2d2640,
    #1e1a2e,
    #252030,
    #2d2640
  );
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  will-change: transform;
}

.cover-inner {
  width: 78%;
  height: 78%;
  border-radius: 50%;
  background: linear-gradient(135deg,
    #2d2640 0%,
    #1a1724 50%,
    #1e1a2e 100%
  );
  display: flex;
  align-items: center;
  justify-content: center;
  color: white;
  overflow: hidden;
}

.cover-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  border-radius: 50%;
}

.cover-groove {
  position: absolute;
  width: 90%;
  height: 90%;
  border-radius: 50%;
  border: 1px solid rgba(255,255,255,0.04);
  pointer-events: none;
}

.cover-hole {
  position: absolute;
  width: 40px;
  height: 40px;
  border-radius: 50%;
  background: rgba(0,0,0,0.6);
  border: 2px solid rgba(255,255,255,0.06);
}

/* 曲目信息 */
.np-info {
  text-align: center;
  width: 100%;
  padding: 2px 12px 0;
  margin-bottom: 8px;
}

.np-title {
  font-size: 22px;
  font-weight: 700;
  color: white;
  line-height: 1.3;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  letter-spacing: -0.3px;
}

.np-artist {
  font-size: 14px;
  color: rgba(255,255,255,0.55);
  margin-top: 3px;
  font-weight: 500;
}

/* 进度条区域 */
.np-slider-area {
  width: 100%;
}

.np-time {
  display: flex;
  justify-content: space-between;
  font-size: 11px;
  font-weight: 600;
  color: rgba(255,255,255,0.4);
  padding: 2px 4px 0;
  font-variant-numeric: tabular-nums;
}

.np-audio-info {
  text-align: center;
  font-size: 10px;
  font-weight: 600;
  color: rgba(255,255,255,0.28);
  letter-spacing: 0.5px;
  margin-top: 0px;
}

.np-audio-codec {
  color: var(--md-primary-container, #E8DEF8);
}

/* 控制栏 */
.np-controls {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  width: 100%;
  margin-top: 50px;
}

.ctrl-btn {
  width: 46px;
  height: 46px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgba(255,255,255,0.4);
  transition: color 150ms, background 150ms;

  &:hover { background: rgba(255,255,255,0.07); color: rgba(255,255,255,0.75); }
  &.active { color: white; }
  &:active { transform: scale(0.9); }
}

.play-btn {
  width: 52px;
  height: 52px;
  border-radius: var(--radius-full);
  background: white;
  color: rgb(20, 18, 24);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 4px 24px rgba(0,0,0,0.35);
  margin: 0 4px;
  transition: transform 150ms var(--ease-standard), box-shadow 150ms;
  overflow: hidden;

  &:hover { transform: scale(1.05); box-shadow: 0 6px 28px rgba(0,0,0,0.4); }
  &:active { transform: scale(0.94); }
}

.play-icon-inner {
  font-size: 28px;
  display: block;

  &.spinning {
    animation: np-spin 1s linear infinite;
    color: inherit; // 继承 .play-btn 的深色文字
  }
}

@keyframes np-spin { to { transform: rotate(360deg); } }

/* 播放/暂停图标切换动画 */
.play-icon-enter-active {
  transition: transform 150ms var(--ease-decelerate), opacity 100ms var(--ease-decelerate);
}
.play-icon-leave-active {
  transition: transform 80ms var(--ease-accelerate), opacity 60ms var(--ease-accelerate);
}
.play-icon-enter-from { transform: scale(0.5); opacity: 0; }
.play-icon-leave-to { transform: scale(0.5); opacity: 0; }

/* 工具栏 */
.np-toolbar {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  width: 100%;
  margin-top: 4px;
}

.tool-btn {
  width: 42px;
  height: 42px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgba(255,255,255,0.35);
  transition: color 150ms, background 150ms;

  .material-symbols-rounded { font-size: 20px; }

  &:hover { background: rgba(255,255,255,0.06); color: rgba(255,255,255,0.65); }
  &.active { color: var(--md-primary-container, #E8DEF8); }
  &.disabled { opacity: 0.25; cursor: default; }
  &:active { transform: scale(0.88); }
}

/* 音量控制 */
.volume-wrap {
  position: relative;
}

.volume-popover {
  position: absolute;
  bottom: 46px;
  left: 50%;
  transform: translateX(-50%);
  background: rgba(30, 28, 34, 0.95);
  backdrop-filter: blur(20px);
  border-radius: 14px;
  padding: 16px 12px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  box-shadow: 0 8px 32px rgba(0,0,0,0.5);
  border: 1px solid rgba(255,255,255,0.08);
  z-index: 10;
}

.volume-slider {
  writing-mode: vertical-lr;
  direction: rtl;
  appearance: none;
  width: 4px;
  height: 100px;
  background: rgba(255,255,255,0.15);
  border-radius: 2px;
  outline: none;
  cursor: pointer;

  &::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: white;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
    cursor: pointer;
  }
}

.volume-label {
  font-size: 11px;
  font-weight: 600;
  color: rgba(255,255,255,0.5);
  font-variant-numeric: tabular-nums;
}

/* 播放速度 */
.speed-wrap, .sleep-wrap {
  position: relative;
}

.speed-label {
  font-size: 12px;
  font-weight: 700;
  letter-spacing: -0.3px;
}

.speed-popover, .sleep-popover {
  position: absolute;
  bottom: 46px;
  left: 50%;
  transform: translateX(-50%);
  background: rgba(30, 28, 34, 0.95);
  backdrop-filter: blur(20px);
  border-radius: 14px;
  padding: 8px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  box-shadow: 0 8px 32px rgba(0,0,0,0.5);
  border: 1px solid rgba(255,255,255,0.08);
  z-index: 10;
  min-width: 120px;
}

.speed-option, .sleep-option {
  padding: 8px 16px;
  border: none;
  background: transparent;
  color: rgba(255,255,255,0.7);
  font-size: 13px;
  cursor: pointer;
  border-radius: 8px;
  text-align: left;
  white-space: nowrap;
  transition: all 0.15s;

  &:hover {
    background: rgba(255,255,255,0.1);
    color: white;
  }

  &.active {
    color: var(--md-primary, #D0BCFF);
    font-weight: 600;
  }

  &.cancel {
    color: #EF5350;
    border-top: 1px solid rgba(255,255,255,0.06);
    margin-top: 4px;
    padding-top: 10px;
  }
}

.sleep-badge {
  position: absolute;
  top: -2px;
  right: -4px;
  font-size: 9px;
  font-weight: 700;
  background: var(--md-primary, #D0BCFF);
  color: #1C1B1F;
  padding: 1px 4px;
  border-radius: 6px;
  font-variant-numeric: tabular-nums;
}

/* 歌词空状态 */
.lyrics-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 100%;
  gap: 10px;
  color: rgba(255,255,255,0.18);
  font-size: 14px;
  font-weight: 500;
}
</style>

<!-- 右键菜单样式（Teleport 到 body，需 unscoped） -->
<style lang="scss">
.np-ctx-overlay {
  position: fixed;
  inset: 0;
  z-index: 500;
}

.np-ctx-menu {
  position: fixed;
  min-width: 200px;
  background: rgba(30, 28, 34, 0.95);
  backdrop-filter: blur(20px);
  border-radius: 12px;
  border: 1px solid rgba(255,255,255,0.08);
  padding: 4px 0;
  z-index: 501;
  box-shadow: 0 8px 32px rgba(0,0,0,0.5);
  animation: np-ctx-in 120ms ease-out;
}

@keyframes np-ctx-in {
  from { opacity: 0; transform: scale(0.95) translateY(-4px); }
  to { opacity: 1; transform: scale(1) translateY(0); }
}

.np-ctx-item {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  padding: 10px 16px;
  border: none;
  background: transparent;
  color: rgba(255,255,255,0.8);
  font-size: 14px;
  cursor: pointer;
  transition: background 0.1s;

  &:hover {
    background: rgba(255,255,255,0.08);
  }
}

/* 更多选项面板 */
.np-more-overlay {
  position: fixed;
  inset: 0;
  z-index: 9000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0,0,0,0.35);
  backdrop-filter: blur(4px);
}

.np-more-sheet {
  width: 380px;
  max-height: 80vh;
  overflow-y: auto;
  background: rgba(30, 28, 34, 0.95);
  backdrop-filter: blur(20px);
  border-radius: 28px;
  padding: 24px;
  box-shadow: 0 16px 48px rgba(0,0,0,0.5);
  border: 1px solid rgba(255,255,255,0.06);
  animation: np-sheet-in 200ms cubic-bezier(0.2, 0, 0, 1);
}

@keyframes np-sheet-in {
  from { opacity: 0; transform: scale(0.92); }
  to { opacity: 1; transform: scale(1); }
}

.np-more-title {
  font-size: 18px;
  font-weight: 600;
  color: rgba(255,255,255,0.9);
  margin: 0 0 16px;
}

// 子视图 header（返回按钮 + 标题）
.np-more-sub-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 16px;

  .np-more-title { margin: 0; }
}

.np-more-back {
  width: 36px;
  height: 36px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgba(255,255,255,0.7);
  transition: background 0.15s;
  &:hover { background: rgba(255,255,255,0.08); }
}

// Android 风格列表项
.np-more-list-item {
  display: flex;
  align-items: center;
  gap: 16px;
  width: 100%;
  padding: 14px 4px;
  border: none;
  background: transparent;
  color: rgba(255,255,255,0.85);
  cursor: pointer;
  border-radius: 12px;
  transition: background 0.15s;

  &:hover { background: rgba(255,255,255,0.06); }

  > .material-symbols-rounded {
    font-size: 22px;
    color: rgba(255,255,255,0.5);
    flex-shrink: 0;
  }
}

.np-more-list-info {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  text-align: left;
}

.np-more-list-headline {
  font-size: 15px;
  font-weight: 500;
}

.np-more-list-desc {
  font-size: 12px;
  color: rgba(255,255,255,0.4);
}

.np-more-chevron {
  font-size: 20px !important;
  color: rgba(255,255,255,0.25) !important;
}

// 速度选择网格
.np-more-speed-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 8px;
  padding: 8px 0;
}

.np-more-speed-btn {
  padding: 12px;
  border: none;
  border-radius: 12px;
  background: rgba(255,255,255,0.06);
  color: rgba(255,255,255,0.7);
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;

  &:hover { background: rgba(255,255,255,0.10); }
  &.active {
    background: var(--md-primary-container, #E8DEF8);
    color: var(--md-on-primary-container, #1D192B);
  }
}

.np-more-item {
  margin-bottom: 20px;
}

.np-more-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 500;
  color: rgba(255,255,255,0.7);
  margin-bottom: 10px;
}

.np-more-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.np-more-slider {
  flex: 1;
  appearance: none;
  height: 4px;
  background: rgba(255,255,255,0.15);
  border-radius: 2px;
  outline: none;
  cursor: pointer;

  &::-webkit-slider-thumb {
    appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--md-primary, #D0BCFF);
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
  }
}

.np-offset-value {
  font-size: 12px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  color: rgba(255,255,255,0.5);
  min-width: 60px;
  text-align: right;

  &.positive { color: #66BB6A; }
  &.negative { color: #EF5350; }
}

.np-more-preview {
  margin-top: 8px;
  font-weight: 700;
  color: rgba(255,255,255,0.5);
  line-height: 1.4;
  transition: font-size 0.15s;
}

// 搜索栏
.np-more-search-bar {
  display: flex;
  gap: 8px;
  margin-bottom: 12px;
}

.np-more-input {
  flex: 1;
  padding: 10px 14px;
  border: 1px solid rgba(255,255,255,0.12);
  border-radius: 12px;
  background: rgba(255,255,255,0.06);
  color: white;
  font-size: 14px;
  outline: none;
  transition: border-color 0.15s;

  &:focus { border-color: rgba(255,255,255,0.3); }
  &::placeholder { color: rgba(255,255,255,0.3); }
}

.np-more-search-btn {
  width: 42px;
  height: 42px;
  border-radius: 12px;
  background: rgba(255,255,255,0.08);
  color: rgba(255,255,255,0.7);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: background 0.15s;

  &:hover { background: rgba(255,255,255,0.14); }
  &:disabled { opacity: 0.4; }
}

.np-more-status {
  text-align: center;
  color: rgba(255,255,255,0.35);
  font-size: 13px;
  padding: 16px 0;
}

// 搜索结果列表
.np-more-search-results {
  max-height: 300px;
  overflow-y: auto;
  &::-webkit-scrollbar { display: none; }
}

.np-more-search-item {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  padding: 10px 4px;
  border: none;
  background: transparent;
  color: rgba(255,255,255,0.85);
  cursor: pointer;
  border-radius: 10px;
  transition: background 0.15s;
  text-align: left;

  &:hover { background: rgba(255,255,255,0.06); }
}

.np-more-search-cover {
  width: 40px;
  height: 40px;
  border-radius: 6px;
  object-fit: cover;
  flex-shrink: 0;
  background: rgba(255,255,255,0.06);
}

.np-more-search-info {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.np-more-search-title {
  font-size: 14px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.np-more-search-artist {
  font-size: 12px;
  color: rgba(255,255,255,0.4);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.np-more-search-source {
  font-size: 10px;
  font-weight: 600;
  color: rgba(255,255,255,0.3);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  flex-shrink: 0;
}

// 编辑表单
.np-more-form {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.np-more-form-label {
  font-size: 12px;
  font-weight: 600;
  color: rgba(255,255,255,0.45);
  margin-top: 4px;
}

.np-more-form-actions {
  display: flex;
  gap: 8px;
  margin-top: 12px;
}

.np-more-form-btn {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 10px;
  border: none;
  border-radius: 12px;
  background: rgba(255,255,255,0.08);
  color: rgba(255,255,255,0.7);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: background 0.15s;

  .material-symbols-rounded { font-size: 18px; }

  &:hover { background: rgba(255,255,255,0.12); }

  &.primary {
    background: var(--md-primary-container, #E8DEF8);
    color: var(--md-on-primary-container, #1D192B);
    &:hover { opacity: 0.9; }
  }
}

// 音质列表
.np-more-quality-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.np-more-quality-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  padding: 14px 12px;
  border: none;
  background: transparent;
  color: rgba(255,255,255,0.7);
  font-size: 15px;
  cursor: pointer;
  border-radius: 10px;
  transition: background 0.15s;

  &:hover { background: rgba(255,255,255,0.06); }

  &.active {
    color: var(--md-primary-container, #E8DEF8);
    font-weight: 600;
  }
}
</style>
