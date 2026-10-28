<script setup lang="ts">
import {
  ref, computed, nextTick, onErrorCaptured, onMounted, onUnmounted, watch,
  type ComponentInternalInstance, type ComponentPublicInstance,
} from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { usePlayerStore, displayAlbum } from '@/stores/player'
import { usePlaybackStatsStore } from '@/stores/playbackStats'
import { installGlobalShortcuts, type ShortcutActions } from '@/modules/shortcuts/globalShortcuts'
import { applySystemShortcuts, clearSystemShortcuts } from '@/modules/shortcuts/systemShortcuts'
import { detectPlatform } from '@/modules/shortcuts/platform'
import { shortcutRecording } from '@/modules/shortcuts/recording'
import { closeDesktopLyricsWindow, desktopLyricsOpen, installDesktopLyricsBridge, openDesktopLyricsWindow } from '@/modules/desktopLyrics/bridge'
import { installTrayBridge, quitApp } from '@/modules/tray/bridge'
import { syncFrequencyDelayMs, useSyncStore } from '@/stores/sync'
import { useAuthStore } from '@/stores/auth'
import { useRecommendStore } from '@/stores/recommend'
import { useSettingsStore } from '@/stores/settings'
import { applyFontSettings } from '@/modules/settings/fonts'
import { LOW_PERFORMANCE_INTERVAL_MS, setFrameInterval } from '@/utils/frameLoop'
import { HISTORY_CHANGED_EVENT } from '@/stores/history'
import { useLikedSongsStore } from '@/stores/likedSongs'
import { useDownloadStore } from '@/stores/download'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { convertFileSrc } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import AppBackgroundImage from '@/components/AppBackgroundImage.vue'
import MiniPlayer from '@/components/MiniPlayer.vue'
import NowPlaying from '@/components/NowPlaying.vue'
import SideNav from '@/components/SideNav.vue'
import AppToast from '@/components/AppToast.vue'
import BiliVideoPartsDialog from '@/components/BiliVideoPartsDialog.vue'
import TitleBar from '@/components/TitleBar.vue'
import i18n, { setLocale } from '@/i18n'
import { useToastStore } from '@/stores/toast'
import { applyTheme } from '@/utils/theme'
import { applyThemeColor } from '@/utils/themeColor'
import { getTrackCoverUrl } from '@/utils/trackCover'
import { applyDynamicColorFromCover, applyDynamicColorFromSeed, clearDynamicColor, resolveSystemAccentSeed } from '@/utils/colorExtractor'
import { createLogger } from '@/utils/logger'
import { hasVisiblePlaybackSession } from '@/modules/playback/playbackRequest'
import { useUpcomingLyricsPrefetch } from '@/composables/useUpcomingLyricsPrefetch'

type CoverSnapshot = {
  rect: { left: number; top: number; width: number; height: number }
  borderRadius: string
  src: string
}

// 根类 platform-mac / platform-windows / platform-linux 驱动平台样式分支（如 macOS 更高的
// --titlebar-height，红绿灯垂直居中）；窗口在 mount 后才显示，不会闪
document.documentElement.classList.add(`platform-${detectPlatform()}`)

const appLog = createLogger('app')
const player = usePlayerStore()
const settingsStore = useSettingsStore()
watch(
  () => [settingsStore.appFontFamily, settingsStore.lyricFontFamily] as const,
  ([appFontFamily, lyricFontFamily]) => applyFontSettings(appFontFamily, lyricFontFamily),
  { immediate: true },
)
// Low performance mode is a global rendering preference, applied as soon as the main
// window mounts instead of waiting for the settings page to be opened. The three per-frame
// animations (background shader, lyrics, waveform) share one scheduler, so capping the
// frame rate is a single call here.
watch(
  () => settingsStore.lowPerformanceMode,
  on => setFrameInterval(on ? LOW_PERFORMANCE_INTERVAL_MS : 0),
  { immediate: true },
)
const likedSongs = useLikedSongsStore()
const route = useRoute()
const router = useRouter()
useUpcomingLyricsPrefetch()
const isNowPlayingOpen = ref(false)
// 静音前的音量，取消静音时还原
let volumeBeforeMute = 0.5
let uninstallShortcuts: (() => void) | null = null
let stopSystemShortcuts: (() => void) | null = null
let uninstallDesktopLyrics: (() => void) | null = null
const contentRef = ref<HTMLElement | null>(null)
const miniPlayerRef = ref<InstanceType<typeof MiniPlayer> | null>(null)
const nowPlayingRef = ref<InstanceType<typeof NowPlaying> | null>(null)
const playerTransitionPulse = ref(0)
const flipOverlay = ref<CoverSnapshot | null>(null)
const flipOverlayStyle = ref<Record<string, string> | null>(null)
const flipTransitionMode = ref<'opening' | 'closing' | null>(null)
const isFlipAnimating = ref(false)
const nowPlayingMotionState = ref<'opening' | 'closing' | null>(null)
// NVIDIA + WebKitGTK：整页滑入/滑出会重绘巨大图层并闪烁，改为瞬时切换
// NVIDIA + WebKitGTK: sliding the full-screen layer repaints a huge area and
// flickers, so switch instantly instead.
const nvidiaCompat = ref(false)
const lastCoverSnapshot = ref<{ trackId: string; src: string } | null>(null)

const hasMiniPlayer = computed(() => hasVisiblePlaybackSession(
  player.hasPlaybackSession,
  player.currentTrack?.id,
))
const transitionTrackKey = computed(() => player.currentTrack?.id || 'empty')
const hideMiniCoverForFlip = computed(() => isFlipAnimating.value)
const isNowPlayingMotionActive = computed(() => nowPlayingMotionState.value !== null)
const skipMiniEnter = ref(false)
const miniTransitionName = computed(() => skipMiniEnter.value ? 'mini-none' : 'mini-enter')
let flipAnimationFrame = 0
let flipCleanupTimer: ReturnType<typeof setTimeout> | null = null
let nowPlayingMotionTimer: ReturnType<typeof setTimeout> | null = null

function cancelFlipAnimation() {
  if (flipAnimationFrame) {
    cancelAnimationFrame(flipAnimationFrame)
    flipAnimationFrame = 0
  }
  if (flipCleanupTimer) {
    clearTimeout(flipCleanupTimer)
    flipCleanupTimer = null
  }
}

function resetFlipState() {
  cancelFlipAnimation()
  isFlipAnimating.value = false
  flipTransitionMode.value = null
  flipOverlay.value = null
  flipOverlayStyle.value = null
}

function scheduleNowPlayingMotionCleanup(delay = nvidiaCompat.value ? 0 : 520) {
  if (nowPlayingMotionTimer) {
    clearTimeout(nowPlayingMotionTimer)
  }
  nowPlayingMotionTimer = setTimeout(() => {
    nowPlayingMotionTimer = null
    nowPlayingMotionState.value = null
  }, delay)
}

function setFlipStyle(snapshot: CoverSnapshot) {
  flipOverlayStyle.value = {
    left: `${snapshot.rect.left}px`,
    top: `${snapshot.rect.top}px`,
    width: `${snapshot.rect.width}px`,
    height: `${snapshot.rect.height}px`,
    borderRadius: snapshot.borderRadius,
  }
}

function runFlipTransition(from: CoverSnapshot, to: CoverSnapshot, mode: 'opening' | 'closing') {
  cancelFlipAnimation()
  flipTransitionMode.value = mode
  flipOverlay.value = from
  setFlipStyle(from)
  isFlipAnimating.value = true

  flipAnimationFrame = requestAnimationFrame(() => {
    flipAnimationFrame = requestAnimationFrame(() => {
      flipOverlay.value = { ...to, src: from.src || to.src }
      flipOverlayStyle.value = {
        left: `${to.rect.left}px`,
        top: `${to.rect.top}px`,
        width: `${to.rect.width}px`,
        height: `${to.rect.height}px`,
        borderRadius: to.borderRadius,
      }
    })
  })

  flipCleanupTimer = window.setTimeout(() => {
    flipCleanupTimer = null
    resetFlipState()
  }, mode === 'opening' ? 420 : 520)
}

async function openNowPlaying() {
  if (!hasMiniPlayer.value) return
  // 动画进行中允许打断，避免连点卡住
  resetFlipState()
  nowPlayingMotionState.value = 'opening'
  scheduleNowPlayingMotionCleanup()
  playerTransitionPulse.value = Date.now()
  isNowPlayingOpen.value = true
}

// 播放页里的组件挂载失败后，外层 <transition> 之后的每次更新都会因为它没有渲染结果而抛错，
// 整个界面就此卡住：关掉播放页，并换 key 重建这层 transition
const nowPlayingBoundaryKey = ref(0)

function failedToMountInNowPlaying(instance: ComponentPublicInstance | null): boolean {
  let current: ComponentInternalInstance | null = instance?.$ ?? null
  if (!current || current.isMounted) return false
  for (; current; current = current.parent) {
    if (current.type === NowPlaying) return true
  }
  return false
}

onErrorCaptured((error, instance) => {
  if (!failedToMountInNowPlaying(instance)) return
  appLog.error('now playing failed to mount:', error)
  isNowPlayingOpen.value = false
  nowPlayingMotionState.value = null
  nowPlayingBoundaryKey.value++
  useToastStore().error((i18n.global as any).t('player.now_playing_failed'))
  return false
})

async function closeNowPlaying() {
  if (!isNowPlayingOpen.value && nowPlayingMotionState.value !== 'opening') return
  const from = nowPlayingRef.value?.getCoverSnapshot?.() as CoverSnapshot | null
  if (from?.src && player.currentTrack?.id) {
    lastCoverSnapshot.value = {
      trackId: player.currentTrack.id,
      src: from.src,
    }
  }
  resetFlipState()
  nowPlayingMotionState.value = 'closing'
  scheduleNowPlayingMotionCleanup()
  skipMiniEnter.value = true
  isNowPlayingOpen.value = false
  await nextTick()
  requestAnimationFrame(() => { skipMiniEnter.value = false })
}

const miniCoverFallbackSrc = computed(() => {
  const snapshot = lastCoverSnapshot.value
  if (!snapshot || snapshot.trackId !== player.currentTrack?.id) return ''
  return snapshot.src
})

// 背景图片
// 旧版直接引用原图路径，资源作用域外的图片重启后会丢失：启动时复制进应用数据目录
async function adoptManagedBackgroundImage() {
  const uri = settingsStore.backgroundImageUri
  if (!uri || /^https?:/i.test(uri)) return
  try {
    const managed = await invoke<string>('import_background_image', { source: uri })
    if (managed !== uri && settingsStore.backgroundImageUri === uri) settingsStore.backgroundImageUri = managed
  } catch (error) {
    appLog.warn('Custom background could not be copied:', error)
  }
}

const bgImageSrc = computed(() => {
  const uri = settingsStore.backgroundImageUri
  if (!uri) return ''
  return /^https?:/i.test(uri) ? uri : convertFileSrc(uri)
})
watch(
  () => [!!bgImageSrc.value, settingsStore.enhancedAdvancedBlur, settingsStore.enhancedAdvancedBlurRadius] as const,
  ([active, glass, radius]) => {
    const root = document.documentElement
    root.classList.toggle('has-custom-bg', active)
    root.classList.toggle('enhanced-blur', active && glass)
    root.style.setProperty('--glass-blur', `${radius}px`)
  },
  { immediate: true },
)

// 对齐 Android：数据变更后短暂延迟，给连续操作留出合并时间
const DEBOUNCE_SYNC_MS = 5_000
// 长音频进度播放时每 15 秒写一次：比写入间隔长，连续播放时不同步，暂停或停下后再同步
const HISTORY_PROGRESS_SETTLE_MS = 30_000
const PERIODIC_SYNC_MS = 60 * 60 * 1000
let debounceSyncTimer: ReturnType<typeof setTimeout> | null = null
let historyBatchedTimer: ReturnType<typeof setTimeout> | null = null
let historyProgressTimer: ReturnType<typeof setTimeout> | null = null
let periodicSyncTimer: ReturnType<typeof setInterval> | null = null
let unlistenPlaylistChanged: UnlistenFn | null = null
let unlistenPlaylistUsage: UnlistenFn | null = null
let unlistenCloseRequested: UnlistenFn | null = null
let uninstallTray: (() => void) | null = null

function handleBeforeUnload() {
  void player.flushPlayerState()
  // 结算最后一段收听，否则关窗前听的时长会丢
  void usePlaybackStatsStore().flushFinal()
}

function flushBeforeExit() {
  return Promise.allSettled([player.flushPlayerState(), usePlaybackStatsStore().flushFinal()])
}

// 关窗流程进行中：重复的关闭请求不再触发落盘
let closeFlushing = false

// 主窗口的隐藏由 Rust 侧完成（关闭 = 收进托盘）；这里必须 preventDefault，
// 否则 JS 侧会接着 destroy 窗口。关闭即退出时落盘后再结束进程
async function handleCloseRequested(event: { preventDefault: () => void }) {
  event.preventDefault()
  if (closeFlushing) return
  closeFlushing = true
  try {
    if (settingsStore.closeToTray) await flushBeforeExit()
    else await quitApp(flushBeforeExit)
  } catch {
    // 落盘失败不阻塞
  } finally {
    closeFlushing = false
  }
}

function scheduleDebouncedSync() {
  const syncStore = useSyncStore()
  // 这一轮同步的快照里没有这次修改，等它结束后补一轮
  if (syncStore.isSyncing) {
    syncStore.requestFollowUpSync()
    return
  }

  if (debounceSyncTimer) clearTimeout(debounceSyncTimer)
  debounceSyncTimer = setTimeout(() => {
    debounceSyncTimer = null
    void syncStore.syncAuto(true)
  }, DEBOUNCE_SYNC_MS)
}

function triggerSilentSync() {
  const syncStore = useSyncStore()
  if (syncStore.isSyncing) {
    syncStore.requestFollowUpSync()
    return
  }
  void syncStore.syncAuto(true)
}

function scheduleHistorySync(event: Event) {
  const type = (event as CustomEvent<{ type?: string }>).detail?.type
  // 同步自己写回的历史不需要再同步一次
  if (type === 'sync') return
  const syncStore = useSyncStore()

  if (!(
    (syncStore.github.configured && syncStore.github.autoSync) ||
    (syncStore.webdav.configured && syncStore.webdav.autoSync)
  )) return

  const delay = syncFrequencyDelayMs(syncStore.syncFrequency)
  if (delay === 0) {
    if (type === 'progress') {
      if (historyProgressTimer) clearTimeout(historyProgressTimer)
      historyProgressTimer = setTimeout(() => {
        historyProgressTimer = null
        triggerSilentSync()
      }, HISTORY_PROGRESS_SETTLE_MS)
      return
    }
    if (historyProgressTimer) {
      clearTimeout(historyProgressTimer)
      historyProgressTimer = null
    }
    triggerSilentSync()
    return
  }

  // 批量窗口从第一条修改开始计时；每次播放都重新计时的话，连续听歌时永远等不到同步
  if (historyBatchedTimer) return
  historyBatchedTimer = setTimeout(() => {
    historyBatchedTimer = null
    triggerSilentSync()
  }, delay)
}

// 滚动位置保持（UI-009）：.content 是各路由共享的滚动容器，keep-alive 只缓存组件状态、
// 不保存该容器的 scrollTop。这里在离开时按 route.name 记录、进入时恢复：
// keep-alive 页（home/explore/library）恢复原滚动，其余页归零，避免"新页从上一页偏移量开始"
const KEEP_ALIVE_ROUTE_NAMES = ['home', 'explore', 'library']
const _scrollPositions = new Map<string, number>()
let _prevRouteName = route.name as string | undefined
let pendingScrollTop = 0
watch(() => route.fullPath, () => {
  // flush:'pre'——此刻 DOM 尚未切换，contentRef.scrollTop 仍是离开页的真实滚动量
  if (_prevRouteName && contentRef.value) {
    _scrollPositions.set(_prevRouteName, contentRef.value.scrollTop)
  }
  const entering = route.name as string | undefined
  _prevRouteName = entering
  pendingScrollTop = entering && KEEP_ALIVE_ROUTE_NAMES.includes(entering)
    ? _scrollPositions.get(entering) ?? 0
    : 0
}, { flush: 'pre' })
// out-in 过渡要等离开页淡出后才插入新页；在 enter 钩子里滚动，才作用在新页上
function restoreContentScroll() {
  contentRef.value?.scrollTo({ top: pendingScrollTop, left: 0 })
}

// 动态取色：跟随封面主题色。解析当前深浅色，供令牌生成使用
function resolveDynamicIsDark(): boolean {
  const mode = settingsStore.darkMode
  if (mode === 'dark') return true
  if (mode === 'light') return false
  return window.matchMedia('(prefers-color-scheme: dark)').matches
}

let systemAccentRequest = 0
let appliedSystemAccent = ''

/** 跟随系统强调色；探测是异步的，期间切了取色方式或又发起新探测时丢弃旧结果 */
async function applySystemAccent() {
  const request = ++systemAccentRequest
  const seed = await resolveSystemAccentSeed()
  if (request !== systemAccentRequest || settingsStore.colorMode !== 'system') return
  if (document.documentElement.classList.contains('theme-ripple-active')) return
  const dark = resolveDynamicIsDark()
  const key = seed ? `${seed.join(',')}|${dark}` : ''
  if (key && key === appliedSystemAccent) return
  appliedSystemAccent = key
  // 引擎不支持解析系统强调色时回退默认取色
  if (seed) applyDynamicColorFromSeed(seed, dark)
  else clearDynamicColor(dark)
}

// 切回窗口时重新读一次，用户在系统设置里换了强调色能跟上
function handleWindowFocus() {
  if (settingsStore.colorMode === 'system') void applySystemAccent()
}

// 取色方式、深浅色或封面变化时重算；default 或无封面则还原预设主题色
watch(
  () => [
    settingsStore.colorMode,
    settingsStore.darkMode,
    settingsStore.colorMode === 'cover' && player.hasPlaybackSession ? getTrackCoverUrl(player.currentTrack) : '',
  ] as const,
  ([mode, , cover]) => {
    systemAccentRequest++
    appliedSystemAccent = ''
    // 圆形扩散中由 theme 路径同步重算，避免圆外提前换色
    if (document.documentElement.classList.contains('theme-ripple-active')) return
    if (mode === 'system') {
      void applySystemAccent()
      return
    }
    const dark = resolveDynamicIsDark()
    if (mode === 'cover' && cover) {
      void applyDynamicColorFromCover(cover, dark)
      return
    }
    clearDynamicColor(dark)
  },
)

// 启动时初始化：加载同步配置 + 检查登录状态 + 自动同步
onMounted(async () => {
  // NVIDIA + WebKitGTK 下整页 opacity/transform 过渡会触发大面积重绘并闪烁；
  // 后端确认是 NVIDIA 后加类，让 CSS 把路由切换降为瞬时切换（不动画整页）。
  // Full-page opacity/transform transitions repaint a large area and flicker on
  // WebKitGTK + NVIDIA; after the backend confirms NVIDIA, add a class that makes
  // route switches instant instead of animating the whole page.
  if (detectPlatform() === 'linux') {
    void invoke<{ nvidia: boolean }>('get_linux_graphics_compat')
      .then((compat) => {
        if (compat.nvidia) {
          nvidiaCompat.value = true
          document.documentElement.classList.add('nvidia-compat')
        }
      })
      .catch(() => { /* 旧后端或非 Linux：保持默认过渡 */ })
  }
  uninstallDesktopLyrics = installDesktopLyricsBridge({
    openSettings: () => {
      if (isNowPlayingOpen.value) closeNowPlaying()
      // 带上时间戳：已经在设置页时也能再次跳到桌面歌词分区
      void router.push({ name: 'settings', query: { section: 'desktop_lyrics', focus: 'desktop-lyrics', at: String(Date.now()) } })
    },
  })
  uninstallTray = installTrayBridge({
    openNowPlaying: () => void openNowPlaying(),
    flushBeforeQuit: flushBeforeExit,
  })
  const syncStore = useSyncStore()
  const authStore = useAuthStore()
  window.addEventListener('beforeunload', handleBeforeUnload)
  window.addEventListener('pagehide', handleBeforeUnload)
  try {
    unlistenCloseRequested = await getCurrentWindow().onCloseRequested(handleCloseRequested)
  } catch {
    // 浏览器开发模式没有原生窗口事件时依赖 pagehide/beforeunload
  }

  // Rust 配置是启动后的规范来源，本地影子只负责首屏快速显示
  // 播放统计要在任何播放发生之前接上，否则首曲不计数
  usePlaybackStatsStore().attach()
  const shortcutActions: ShortcutActions = {
    togglePlay: () => void player.togglePlayPause(),
    next: () => void player.next(),
    previous: () => void player.previous(),
    seekBy: (deltaMs) => {
      if (!player.hasPlaybackSession) return
      const target = Math.max(0, Math.min(player.durationMs, player.positionMs + deltaMs))
      void player.seekTo(target)
    },
    adjustVolume: (delta) => {
      void player.setVolume(Math.max(0, Math.min(1, player.volume + delta)))
    },
    toggleMute: () => {
      if (player.volume > 0) {
        volumeBeforeMute = player.volume
        void player.setVolume(0)
      } else {
        void player.setVolume(volumeBeforeMute || 0.5)
      }
    },
    toggleNowPlaying: () => {
      if (isNowPlayingOpen.value) void closeNowPlaying()
      else if (player.hasPlaybackSession) openNowPlaying()
    },
    closeOverlay: () => {
      if (!isNowPlayingOpen.value) return false
      void closeNowPlaying()
      return true
    },
    focusSearch: () => {
      const focusInput = () => requestAnimationFrame(() => {
        const input = document.querySelector<HTMLInputElement>('[data-shortcut-search]')
        input?.focus()
        input?.select()
      })
      // 已在探索页时直接聚焦，不能重新 push 把 ?q= 冲掉
      if (router.currentRoute.value.name === 'explore') focusInput()
      else void router.push({ name: 'explore' }).then(focusInput)
    },
    toggleShuffle: () => player.toggleShuffle(),
    cycleRepeat: () => player.toggleRepeatMode(),
    toggleLike: () => {
      if (player.currentTrack) void likedSongs.toggleTrack(player.currentTrack)
    },
    toggleDesktopLyrics: () => {
      const toggle = desktopLyricsOpen.value ? closeDesktopLyricsWindow() : openDesktopLyricsWindow()
      void toggle.catch(() => useToastStore().error((i18n.global as any).t('player.desktop_lyrics_failed')))
    },
    showWindow: () => {
      const appWindow = getCurrentWindow()
      void appWindow.unminimize().then(() => appWindow.show()).then(() => appWindow.setFocus()).catch(() => {})
    },
    // 覆盖所有实际存在的弹层根类，弹层打开时不响应全局播放快捷键（UI-002）
    isOverlayOpen: () => document.querySelector(
      '.m3-dialog-overlay, .dialog-overlay, .context-menu-overlay, .atp-overlay, .lt-overlay, .queue-overlay, .comments-overlay, .notif-overlay, .debug-dialog-overlay',
    ) !== null,
  }
  uninstallShortcuts = installGlobalShortcuts(shortcutActions, () => settingsStore.shortcutBindings)
  stopSystemShortcuts = watch(
    () => [settingsStore.shortcutBindings, settingsStore.globalShortcutsEnabled, shortcutRecording.value] as const,
    ([bindings, enabled, recording]) => void applySystemShortcuts(bindings, enabled && !recording, shortcutActions),
    { immediate: true },
  )

  await settingsStore.hydrate()
  applyTheme(settingsStore.darkMode, false)
  applyThemeColor(settingsStore.themeColor, undefined, false)
  // 首屏在预设主题之后应用动态取色，避免被 applyThemeColor 覆盖
  if (settingsStore.colorMode === 'cover') {
    const cover = player.hasPlaybackSession ? getTrackCoverUrl(player.currentTrack) : ''
    if (cover) void applyDynamicColorFromCover(cover, resolveDynamicIsDark())
  } else if (settingsStore.colorMode === 'system') {
    void applySystemAccent()
  }
  window.addEventListener('focus', handleWindowFocus)
  setLocale(settingsStore.locale, false)
  void adoptManagedBackgroundImage()
  await player.applyPersistedSettings()
  if (route.name === 'home' && settingsStore.defaultScreen !== 'home') {
    await router.replace({ name: settingsStore.defaultScreen })
  }
  try {
    await invoke('set_bypass_proxy', { bypass: settingsStore.bypassProxy })
  } catch {
    // 浏览器开发模式没有 Rust bridge 时忽略
  }

  // 并行加载
  await Promise.allSettled([
    syncStore.loadConfigs(),
    authStore.checkStatus(),
    likedSongs.start(),
  ])

  periodicSyncTimer = setInterval(() => {
    void syncStore.syncAuto(true)
  }, PERIODIC_SYNC_MS)

  // 自动同步（配置开启且已配置），静默模式
  void syncStore.syncAuto(true)

  // 账号状态就绪后再接着下上次没下完的任务（解析地址需要登录态）
  void useDownloadStore().resumePendingDownloads().catch(error => appLog.warn('Resume pending downloads failed:', error))

  // 云端歌单先用上次的缓存显示，这里在后台按已登录的账号各刷新一份，进音乐库时就是新的。
  // 关了国际化就没有 YouTube 入口，不去请求它
  const recommend = useRecommendStore()
  const prefetched = settingsStore.internationalizationEnabled
    ? ['netease', 'bilibili', 'youtube'] as const
    : ['netease', 'bilibili'] as const
  for (const platform of prefetched) {
    if (authStore[platform].loggedIn) void recommend.ensureUserPlaylists(platform, { quiet: true })
  }
  if (authStore.netease.loggedIn) void recommend.ensureUserAlbums({ quiet: true })

  // 监听后端 playlists-changed 事件，防抖触发自动同步；同步自己写回的歌单带 "sync" 标记，不再触发
  unlistenPlaylistChanged = await listen<string | null>('playlists-changed', (event) => {
    if (event.payload === 'sync') return
    scheduleDebouncedSync()
  })
  // 打开歌单的记录写进了同步扩展段（对齐 Android recordOpen 之后 triggerSync）
  unlistenPlaylistUsage = await listen('playlist-usage-changed', () => scheduleDebouncedSync())

  // 监听前端播放历史变更事件，触发历史自动同步
  window.addEventListener(HISTORY_CHANGED_EVENT, scheduleHistorySync)
})

onUnmounted(() => {
  uninstallDesktopLyrics?.()
  uninstallDesktopLyrics = null
  void player.flushPlayerState()
  uninstallShortcuts?.()
  uninstallShortcuts = null
  stopSystemShortcuts?.()
  stopSystemShortcuts = null
  void clearSystemShortcuts()
  window.removeEventListener('beforeunload', handleBeforeUnload)
  window.removeEventListener('pagehide', handleBeforeUnload)
  window.removeEventListener('focus', handleWindowFocus)
  resetFlipState()
  if (nowPlayingMotionTimer) clearTimeout(nowPlayingMotionTimer)
  if (debounceSyncTimer) clearTimeout(debounceSyncTimer)
  if (historyBatchedTimer) clearTimeout(historyBatchedTimer)
  if (historyProgressTimer) clearTimeout(historyProgressTimer)
  if (periodicSyncTimer) clearInterval(periodicSyncTimer)
  likedSongs.stop()
  window.removeEventListener(HISTORY_CHANGED_EVENT, scheduleHistorySync)
  if (unlistenPlaylistChanged) unlistenPlaylistChanged()
  if (unlistenPlaylistUsage) unlistenPlaylistUsage()
  if (unlistenCloseRequested) unlistenCloseRequested()
  uninstallTray?.()
  uninstallTray = null
})
</script>

<template>
  <div class="app-layout">
    <!-- 自定义背景图 -->
    <AppBackgroundImage
      v-if="bgImageSrc"
      :src="bgImageSrc"
      :blur-px="settingsStore.backgroundImageBlur"
      :opacity="settingsStore.backgroundImageAlpha"
      :dim="settingsStore.backgroundImageDim"
    />
    <SideNav class="app-side-nav" :inert="isNowPlayingOpen || undefined" />
    <main
      ref="contentRef"
      class="content"
      :class="{ 'has-mini-player': hasMiniPlayer }"
      :inert="isNowPlayingOpen || undefined"
    >
      <router-view v-slot="{ Component, route }">
        <transition name="fade" mode="out-in" @enter="restoreContentScroll">
          <keep-alive :include="['HomeView', 'ExploreView', 'LibraryView']">
            <component :is="Component" :key="route.path" />
          </keep-alive>
        </transition>
      </router-view>
    </main>

    <!-- 播放页开合时压暗底层：用独立遮罩而不是给页面设透明度，页面里的玻璃卡片
         （backdrop-filter）不会因祖先透明度变化失去背景采样而闪烁 -->
    <div class="np-underlay-scrim" :class="{ 'np-underlay-scrim--visible': isNowPlayingOpen }" aria-hidden="true" />

    <!-- MiniPlayer 动画 -->
    <transition :name="miniTransitionName">
      <MiniPlayer
        v-if="hasMiniPlayer && !isNowPlayingOpen"
        ref="miniPlayerRef"
        :cover-hidden-by-flip="hideMiniCoverForFlip"
        :cover-fallback-src="miniCoverFallbackSrc"
        :key="`mini:${transitionTrackKey}:${playerTransitionPulse}`"
        @expand="openNowPlaying()"
      />
    </transition>

    <!-- NowPlaying 全屏覆盖 -->
    <transition :key="nowPlayingBoundaryKey" name="slide-up">
      <NowPlaying
        v-if="isNowPlayingOpen"
        ref="nowPlayingRef"
        hide-header
        :transition-state="nowPlayingMotionState"
        @collapse="closeNowPlaying()"
      />
    </transition>

    <!-- 顶栏浮于所有内容之上，与播放器融合 -->
    <TitleBar
      :force-light="isNowPlayingOpen"
      :now-playing="isNowPlayingOpen"
      :has-playback-session="player.hasPlaybackSession"
      :track-name="player.hasPlaybackSession ? player.currentTrack?.title : ''"
      :album-name="player.hasPlaybackSession && player.currentTrack?.album ? displayAlbum(player.currentTrack.album) : ''"
      :transitioning="isNowPlayingOpen || isNowPlayingMotionActive"
      :transition-state="nowPlayingMotionState"
      @collapse="closeNowPlaying()"
      @toggle-more="nowPlayingRef?.toggleMore?.()"
    />

    <Transition name="flip-cover-fade">
      <div
        v-if="flipOverlay && flipOverlayStyle"
        class="flip-cover-overlay"
        :class="{
          'flip-cover-overlay--opening': flipTransitionMode === 'opening',
          'flip-cover-overlay--closing': flipTransitionMode === 'closing',
        }"
        :style="flipOverlayStyle"
      >
        <img :src="flipOverlay.src" class="flip-cover-img" referrerpolicy="no-referrer" />
      </div>
    </Transition>

    <AppToast />
    <BiliVideoPartsDialog />
  </div>
</template>

<style scoped lang="scss">
.app-layout {
  display: flex;
  width: 100%;
  height: 100%;
  overflow: hidden;
  position: relative;
  padding-top: var(--titlebar-height, 36px); /* 让出顶栏高度（与 TitleBar / mac 红绿灯对齐） */
  isolation: isolate;
}

.app-side-nav {
  position: relative;
  z-index: 2;
}

.content {
  flex: 1;
  max-height: 100%;
  overflow-y: auto;
  overflow-x: hidden;
  position: relative;
  z-index: 2;
  transition: padding-bottom 300ms var(--ease-standard);

  /* 缩小滚动视口，避免正文透过透明底栏，底部少量余量避开进度条热区 */
  &.has-mini-player {
    max-height: calc(100% - var(--mini-player-height, 76px));
    padding-bottom: 6px;
  }
}

.np-underlay-scrim {
  position: fixed;
  inset: 0;
  z-index: 150;
  background: rgb(0 0 0 / 0.45);
  opacity: 0;
  pointer-events: none;
  transition: opacity 200ms ease;

  &.np-underlay-scrim--visible {
    opacity: 1;
    transition-duration: 320ms;
  }
}

/* MiniPlayer 入场退场：只动 transform/opacity，滤镜动画会让玻璃底栏逐帧重算模糊 */
.mini-enter-enter-active {
  transition: transform 350ms var(--ease-emphasized-decel),
              opacity 250ms var(--ease-decelerate);
}
.mini-enter-leave-active {
  transition: transform 200ms var(--ease-emphasized-accel),
              opacity 150ms var(--ease-accelerate);
}
.mini-enter-enter-from {
  transform: translateY(100%) scale(0.98);
  opacity: 0;
}
.mini-enter-leave-to {
  transform: translateY(100%);
  opacity: 0;
}

.flip-cover-overlay {
  position: fixed;
  z-index: 350;
  overflow: hidden;
  pointer-events: none;
  box-shadow: 0 20px 56px rgba(0, 0, 0, 0.36);
  transition:
    left 520ms cubic-bezier(0.22, 1, 0.36, 1),
    top 520ms cubic-bezier(0.22, 1, 0.36, 1),
    width 520ms cubic-bezier(0.22, 1, 0.36, 1),
    height 520ms cubic-bezier(0.22, 1, 0.36, 1),
    border-radius 520ms cubic-bezier(0.22, 1, 0.36, 1),
    opacity 180ms ease;
}

.flip-cover-overlay--opening {
  box-shadow: none;
  transition:
    left 360ms cubic-bezier(0.22, 1, 0.36, 1),
    top 360ms cubic-bezier(0.22, 1, 0.36, 1),
    width 360ms cubic-bezier(0.22, 1, 0.36, 1),
    height 360ms cubic-bezier(0.22, 1, 0.36, 1),
    border-radius 360ms cubic-bezier(0.22, 1, 0.36, 1),
    opacity 120ms ease;
}

.flip-cover-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.flip-cover-fade-enter-active,
.flip-cover-fade-leave-active {
  transition: opacity 160ms ease;
}

.flip-cover-fade-enter-from,
.flip-cover-fade-leave-to {
  opacity: 0;
}
</style>
