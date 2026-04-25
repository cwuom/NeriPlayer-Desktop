<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { usePlayerStore } from '@/stores/player'
import { useSyncStore } from '@/stores/sync'
import { useAuthStore } from '@/stores/auth'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import MiniPlayer from '@/components/MiniPlayer.vue'
import NowPlaying from '@/components/NowPlaying.vue'
import SideNav from '@/components/SideNav.vue'
import AppToast from '@/components/AppToast.vue'
import TitleBar from '@/components/TitleBar.vue'

const player = usePlayerStore()
const isNowPlayingOpen = ref(false)

const hasMiniPlayer = computed(() => !!player.currentTrack)

// 防抖自动同步（歌单变更后 30s 触发，批量合并快速操作）
const DEBOUNCE_SYNC_MS = 30_000
let debounceSyncTimer: ReturnType<typeof setTimeout> | null = null
let unlistenPlaylistChanged: UnlistenFn | null = null

function scheduleDebouncedSync() {
  const syncStore = useSyncStore()
  // 同步进行中不调度
  if (syncStore.isSyncing) return

  if (debounceSyncTimer) clearTimeout(debounceSyncTimer)
  debounceSyncTimer = setTimeout(() => {
    debounceSyncTimer = null
    if (syncStore.github.configured && syncStore.github.autoSync) {
      syncStore.syncGitHub(true)
    } else if (syncStore.webdav.configured && syncStore.webdav.autoSync) {
      syncStore.syncWebDav(true)
    }
  }, DEBOUNCE_SYNC_MS)
}

// 启动时初始化：加载同步配置 + 检查登录状态 + 自动同步
onMounted(async () => {
  const syncStore = useSyncStore()
  const authStore = useAuthStore()

  // 并行加载
  await Promise.allSettled([
    syncStore.loadConfigs(),
    authStore.checkStatus(),
  ])

  // 自动同步（配置开启且已配置），静默模式
  if (syncStore.github.configured && syncStore.github.autoSync) {
    syncStore.syncGitHub(true)
  } else if (syncStore.webdav.configured && syncStore.webdav.autoSync) {
    syncStore.syncWebDav(true)
  }

  // 监听后端 playlists-changed 事件，防抖触发自动同步
  unlistenPlaylistChanged = await listen('playlists-changed', () => {
    scheduleDebouncedSync()
  })
})

onUnmounted(() => {
  if (debounceSyncTimer) clearTimeout(debounceSyncTimer)
  if (unlistenPlaylistChanged) unlistenPlaylistChanged()
})
</script>

<template>
  <div class="app-layout">
    <SideNav />
    <main class="content" :class="{ 'has-mini-player': hasMiniPlayer }">
      <router-view v-slot="{ Component, route }">
        <transition name="fade" mode="out-in">
          <keep-alive :include="['HomeView', 'ExploreView', 'LibraryView']">
            <component :is="Component" :key="route.path" />
          </keep-alive>
        </transition>
      </router-view>
    </main>

    <!-- MiniPlayer 动画 -->
    <transition name="mini-enter">
      <MiniPlayer
        v-if="hasMiniPlayer && !isNowPlayingOpen"
        @expand="isNowPlayingOpen = true"
      />
    </transition>

    <!-- NowPlaying 全屏覆盖 -->
    <transition name="slide-up">
      <NowPlaying
        v-if="isNowPlayingOpen"
        @collapse="isNowPlayingOpen = false"
      />
    </transition>

    <!-- 顶栏浮于所有内容之上，背景透明融入主体 -->
    <TitleBar :force-light="isNowPlayingOpen" />

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
  padding-top: 36px; /* 让出顶栏高度 */
}

.content {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  transition: padding-bottom 300ms var(--ease-standard);

  &.has-mini-player { padding-bottom: 72px; }
}

/* MiniPlayer 入场退场 */
.mini-enter-enter-active {
  transition: transform 350ms var(--ease-emphasized-decel),
              opacity 250ms var(--ease-decelerate);
}
.mini-enter-leave-active {
  transition: transform 200ms var(--ease-emphasized-accel),
              opacity 150ms var(--ease-accelerate);
}
.mini-enter-enter-from {
  transform: translateY(100%);
  opacity: 0;
}
.mini-enter-leave-to {
  transform: translateY(100%);
  opacity: 0;
}
</style>
