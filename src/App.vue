<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { usePlayerStore, displayAlbum } from '@/stores/player'
import { usePlaybackStatsStore } from '@/stores/playbackStats'
import { installGlobalShortcuts } from '@/modules/shortcuts/globalShortcuts'
import { syncFrequencyDelayMs, useSyncStore } from '@/stores/sync'
import { useAuthStore } from '@/stores/auth'
import { useSettingsStore } from '@/stores/settings'
import { HISTORY_CHANGED_EVENT } from '@/stores/history'
import { useLikedSongsStore } from '@/stores/likedSongs'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { convertFileSrc } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import MiniPlayer from '@/components/MiniPlayer.vue'
import NowPlaying from '@/components/NowPlaying.vue'
import SideNav from '@/components/SideNav.vue'
import AppToast from '@/components/AppToast.vue'
import TitleBar from '@/components/TitleBar.vue'
import { setLocale } from '@/i18n'
import { applyTheme } from '@/utils/theme'
import { applyThemeColor } from '@/utils/themeColor'
import { getTrackCoverUrl } from '@/utils/trackCover'
import { applyDynamicColorFromCover, applyDynamicColorFromSeed, clearDynamicColor, resolveSystemAccentSeed } from '@/utils/colorExtractor'
import { hasVisiblePlaybackSession } from '@/modules/playback/playbackRequest'

type CoverSnapshot = {
  rect: { left: number; top: number; width: number; height: number }
  borderRadius: string
  src: string
}

// macOS 标题栏更高（52px，红绿灯垂直居中），根类驱动 --titlebar-height 分支；
// 与 TitleBar.vue 相同的 navigator 平台判定，窗口在 mount 后才显示，不会闪
const isMacPlatform = /Mac|iPhone|iPad/.test(
  (navigator as any).userAgentData?.platform || navigator.platform || navigator.userAgent
)
if (isMacPlatform) document.documentElement.classList.add('platform-mac')

const player = usePlayerStore()
const settingsStore = useSettingsStore()
const likedSongs = useLikedSongsStore()
const route = useRoute()
const router = useRouter()
const isNowPlayingOpen = ref(false)
// 静音前的音量，取消静音时还原
let volumeBeforeMute = 0.5
let uninstallShortcuts: (() => void) | null = null
const contentRef = ref<HTMLElement | null>(null)
const miniPlayerRef = ref<InstanceType<typeof MiniPlayer> | null>(null)
const nowPlayingRef = ref<InstanceType<typeof NowPlaying> | null>(null)
const playerTransitionPulse = ref(0)
const flipOverlay = ref<CoverSnapshot | null>(null)
const flipOverlayStyle = ref<Record<string, string> | null>(null)
const flipTransitionMode = ref<'opening' | 'closing' | null>(null)
const isFlipAnimating = ref(false)
const nowPlayingMotionState = ref<'opening' | 'closing' | null>(null)
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

function scheduleNowPlayingMotionCleanup(delay = 520) {
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
  scheduleNowPlayingMotionCleanup(520)
  playerTransitionPulse.value = Date.now()
  isNowPlayingOpen.value = true
}

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
  scheduleNowPlayingMotionCleanup(520)
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
const bgImageStyle = computed(() => {
  const uri = settingsStore.backgroundImageUri
  if (!uri) return null
  const src = uri.startsWith('http') ? uri : convertFileSrc(uri)
  return {
    backgroundImage: `url("${src}")`,
    filter: `blur(${settingsStore.backgroundImageBlur}px)`,
    opacity: settingsStore.backgroundImageAlpha,
  }
})

// 对齐 Android：数据变更后短暂延迟，给连续操作留出合并时间
const DEBOUNCE_SYNC_MS = 5_000
const PERIODIC_SYNC_MS = 60 * 60 * 1000
let debounceSyncTimer: ReturnType<typeof setTimeout> | null = null
let historyBatchedTimer: ReturnType<typeof setTimeout> | null = null
let periodicSyncTimer: ReturnType<typeof setInterval> | null = null
let unlistenPlaylistChanged: UnlistenFn | null = null
let unlistenCloseRequested: UnlistenFn | null = null
let unlistenTrayNowPlaying: UnlistenFn | null = null
let unlistenTrayHome: UnlistenFn | null = null

function handleBeforeUnload() {
  player.flushPlayerState()
  // 结算最后一段收听，否则关窗前听的时长会丢
  void usePlaybackStatsStore().flushFinal()
}

// 关窗流程标志位：flush 完成后放行；窗口隐藏/关闭拦截全部由 Rust 侧完成
let closeFlushDone = false

async function handleCloseRequested(event: { preventDefault: () => void }) {
  event.preventDefault()
  if (closeFlushDone) return
  closeFlushDone = true
  try {
    // 同步保存播放器状态 + 等待统计落盘（隐藏后播放继续，下次关闭不再有卸载事件）
    player.flushPlayerState()
    await usePlaybackStatsStore().flushFinal()
  } catch {
    // 落盘失败不阻塞
  }
  closeFlushDone = false
}

function scheduleDebouncedSync() {
  const syncStore = useSyncStore()
  // 同步进行中不调度
  if (syncStore.isSyncing) return

  if (debounceSyncTimer) clearTimeout(debounceSyncTimer)
  debounceSyncTimer = setTimeout(() => {
    debounceSyncTimer = null
    void syncStore.syncAuto(true)
  }, DEBOUNCE_SYNC_MS)
}

function triggerSilentSync() {
  const syncStore = useSyncStore()
  if (syncStore.isSyncing) return
  void syncStore.syncAuto(true)
}

function scheduleHistorySync() {
  const syncStore = useSyncStore()
  if (syncStore.isSyncing) return

  if (!(
    (syncStore.github.configured && syncStore.github.autoSync) ||
    (syncStore.webdav.configured && syncStore.webdav.autoSync)
  )) return

  const delay = syncFrequencyDelayMs(syncStore.syncFrequency)
  if (delay === 0) {
    triggerSilentSync()
    return
  }

  if (historyBatchedTimer) clearTimeout(historyBatchedTimer)
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
watch(() => route.fullPath, async () => {
  // flush:'pre'——此刻 DOM 尚未切换，contentRef.scrollTop 仍是离开页的真实滚动量
  if (_prevRouteName && contentRef.value) {
    _scrollPositions.set(_prevRouteName, contentRef.value.scrollTop)
  }
  const entering = route.name as string | undefined
  _prevRouteName = entering
  await nextTick()
  requestAnimationFrame(() => {
    if (!contentRef.value) return
    const restore = entering && KEEP_ALIVE_ROUTE_NAMES.includes(entering)
      ? _scrollPositions.get(entering) ?? 0
      : 0
    contentRef.value.scrollTo({ top: restore, left: 0 })
  })
}, { flush: 'pre' })

// 动态取色：跟随封面主题色。解析当前深浅色，供令牌生成使用
function resolveDynamicIsDark(): boolean {
  const mode = settingsStore.darkMode
  if (mode === 'dark') return true
  if (mode === 'light') return false
  return window.matchMedia('(prefers-color-scheme: dark)').matches
}

// 取色方式、深浅色或封面变化时重算；default 或无封面则还原预设主题色
watch(
  () => [
    settingsStore.colorMode,
    settingsStore.darkMode,
    player.hasPlaybackSession ? getTrackCoverUrl(player.currentTrack) : '',
  ] as const,
  ([mode, , cover]) => {
    // 圆形扩散中由 theme 路径同步重算，避免圆外提前换色
    if (document.documentElement.classList.contains('theme-ripple-active')) return
    const dark = resolveDynamicIsDark()
    if (mode === 'cover') {
      if (cover) {
        void applyDynamicColorFromCover(cover as string, dark)
        return
      }
      clearDynamicColor(dark)
      return
    }
    if (mode === 'system') {
      const seed = resolveSystemAccentSeed()
      if (seed) {
        applyDynamicColorFromSeed(seed, dark)
        return
      }
      // 引擎不支持解析系统强调色时回退默认取色
    }
    clearDynamicColor(dark)
  },
)

// 启动时初始化：加载同步配置 + 检查登录状态 + 自动同步
onMounted(async () => {
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
  uninstallShortcuts = installGlobalShortcuts({
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
      void router.push({ name: 'explore' }).then(() => {
        requestAnimationFrame(() => {
          const input = document.querySelector<HTMLInputElement>('[data-shortcut-search]')
          input?.focus()
          input?.select()
        })
      })
    },
    toggleShuffle: () => player.toggleShuffle(),
    cycleRepeat: () => player.toggleRepeatMode(),
    // 覆盖所有实际存在的弹层根类；旧选择器 .dialog-overlay/.m3-dialog-scrim 均不存在，
    // 导致弹层打开时全局播放快捷键仍生效（UI-002）
    isOverlayOpen: () => document.querySelector(
      '.m3-dialog-overlay, .atp-overlay, .lt-overlay, .queue-overlay, .notif-overlay, .debug-dialog-overlay',
    ) !== null,
  })

  await settingsStore.hydrate()
  applyTheme(settingsStore.darkMode, false)
  applyThemeColor(settingsStore.themeColor, undefined, false)
  // 首屏在预设主题之后应用动态取色，避免被 applyThemeColor 覆盖
  if (settingsStore.colorMode === 'cover') {
    const cover = player.hasPlaybackSession ? getTrackCoverUrl(player.currentTrack) : ''
    if (cover) void applyDynamicColorFromCover(cover, resolveDynamicIsDark())
  } else if (settingsStore.colorMode === 'system') {
    const seed = resolveSystemAccentSeed()
    if (seed) applyDynamicColorFromSeed(seed, resolveDynamicIsDark())
  }
  setLocale(settingsStore.locale, false)
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

  // 监听后端 playlists-changed 事件，防抖触发自动同步
  unlistenPlaylistChanged = await listen('playlists-changed', () => {
    scheduleDebouncedSync()
  })

  // 托盘菜单：正在播放 / 打开主页面（窗口可能处于隐藏状态，先由后端 show）
  unlistenTrayNowPlaying = await listen('tray:open-now-playing', () => {
    if (player.hasPlaybackSession) void openNowPlaying()
  })
  unlistenTrayHome = await listen('tray:open-home', () => {
    void router.push({ name: 'home' })
  })

  // 监听前端播放历史变更事件，触发历史自动同步
  window.addEventListener(HISTORY_CHANGED_EVENT, scheduleHistorySync as EventListener)
})

onUnmounted(() => {
  player.flushPlayerState()
  uninstallShortcuts?.()
  uninstallShortcuts = null
  window.removeEventListener('beforeunload', handleBeforeUnload)
  window.removeEventListener('pagehide', handleBeforeUnload)
  resetFlipState()
  if (nowPlayingMotionTimer) clearTimeout(nowPlayingMotionTimer)
  if (debounceSyncTimer) clearTimeout(debounceSyncTimer)
  if (historyBatchedTimer) clearTimeout(historyBatchedTimer)
  if (periodicSyncTimer) clearInterval(periodicSyncTimer)
  likedSongs.stop()
  window.removeEventListener(HISTORY_CHANGED_EVENT, scheduleHistorySync as EventListener)
  if (unlistenPlaylistChanged) unlistenPlaylistChanged()
  if (unlistenCloseRequested) unlistenCloseRequested()
  if (unlistenTrayNowPlaying) unlistenTrayNowPlaying()
  if (unlistenTrayHome) unlistenTrayHome()
})
</script>

<template>
  <div
    class="app-layout"
    :class="{
      'app-layout--np-active': isNowPlayingOpen || isNowPlayingMotionActive,
      'app-layout--np-opening': nowPlayingMotionState === 'opening',
      'app-layout--np-closing': nowPlayingMotionState === 'closing',
    }"
  >
    <!-- 自定义背景图 -->
    <div v-if="bgImageStyle" class="app-bg-image" :style="bgImageStyle"></div>
    <SideNav class="app-side-nav" :class="{ 'app-side-nav--dimmed': isNowPlayingOpen }" />
    <main
      ref="contentRef"
      class="content"
      :class="{
        'has-mini-player': hasMiniPlayer,
        'content--np-dimmed': isNowPlayingOpen,
      }"
    >
      <router-view v-slot="{ Component, route }">
        <transition name="fade" mode="out-in">
          <keep-alive :include="['HomeView', 'ExploreView', 'LibraryView']">
            <component :is="Component" :key="route.path" />
          </keep-alive>
        </transition>
      </router-view>
    </main>

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
    <transition name="slide-up">
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
  </div>
</template>

<style scoped lang="scss">
.app-layout {
  display: flex;
  width: 100%;
  height: 100%;
  overflow: hidden;
  position: relative;
  border-radius: var(--radius-lg);
  padding-top: var(--titlebar-height, 36px); /* 让出顶栏高度（与 TitleBar / mac 红绿灯对齐） */
  isolation: isolate;
}

.app-bg-image {
  position: fixed;
  inset: 0;
  z-index: 0;
  background-size: cover;
  background-position: center;
  background-repeat: no-repeat;
  pointer-events: none;
  /* scale slightly to hide blur edges */
  transform: scale(1.1);
}

.app-side-nav {
  position: relative;
  z-index: 2;
  transition: opacity 200ms ease;

  &.app-side-nav--dimmed {
    /* 仅透明度，避免背景缩放干扰详情页纯上滑 */
    transition: opacity 320ms ease;
    opacity: 0.55;
    pointer-events: none;
  }
}

.content {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  position: relative;
  z-index: 2;
  /* 恢复态用较快的 transition */
  transition:
    padding-bottom 300ms var(--ease-standard),
    opacity 200ms ease;

  /* 预留迷你播放器高度 + 少量余量避开进度条热区；余量过大会在列表下方露出白带 */
  &.has-mini-player { padding-bottom: calc(var(--mini-player-height, 76px) + 6px); }

  &.content--np-dimmed {
    /* 仅透明度，禁止缩放/位移 */
    transition:
      padding-bottom 300ms var(--ease-standard),
      opacity 320ms ease;
    opacity: 0.55;
    pointer-events: none;
  }
}

/* MiniPlayer 入场退场 */
.mini-enter-enter-active {
  transition: transform 350ms var(--ease-emphasized-decel),
              opacity 250ms var(--ease-decelerate),
              filter 350ms var(--ease-emphasized-decel);
}
.mini-enter-leave-active {
  transition: transform 200ms var(--ease-emphasized-accel),
              opacity 150ms var(--ease-accelerate),
              filter 200ms var(--ease-emphasized-accel);
}
.mini-enter-enter-from {
  transform: translateY(100%) scale(0.98);
  opacity: 0;
  filter: blur(12px);
}
.mini-enter-leave-to {
  transform: translateY(100%);
  opacity: 0;
  filter: blur(10px);
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
