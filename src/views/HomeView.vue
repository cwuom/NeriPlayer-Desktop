<script setup lang="ts">
import { ref, computed, onMounted, onActivated, onDeactivated, onUnmounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import { convertFileSrc, invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

defineOptions({ name: 'HomeView' })
import { usePlayerStore } from '@/stores/player'
import { useLibraryStore } from '@/stores/library'
import { useAuthStore } from '@/stores/auth'
import { useRecommendStore } from '@/stores/recommend'
import { useHistoryStore } from '@/stores/history'
import { useSettingsStore } from '@/stores/settings'
import { useI18n } from 'vue-i18n'
import type { TrackInfo } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import { normalizeContinuePlaylists, continuePlaylistRoute, type ContinuePlaylist } from '@/modules/library/homeContinue'
import { createLogger } from '@/utils/logger'
import { getPlaybackSourceKind } from '@/modules/playback/playbackSource'
import { useHomeFeedStore } from '@/stores/homeFeed'
import { NETEASE_HOME_SECTIONS } from '@/modules/library/neteaseHome'
import HomeFeedSection from '@/components/HomeFeedSection.vue'
import HomeShelf from '@/components/home/HomeShelf.vue'
import { comboLabelParts, effectiveShortcuts } from '@/modules/shortcuts/bindings'
import TrackContextMenu from '@/components/TrackContextMenu.vue'
import CollectionContextMenu from '@/components/CollectionContextMenu.vue'
import { collectionWebUrl, type CollectionMenuTarget } from '@/utils/collectionLinks'
import { buildYoutubeHomeSections } from '@/modules/youtube/youtubeHomeLayout'
import { useEscapeClose } from '@/composables/useEscapeClose'

const router = useRouter()
const player = usePlayerStore()
const library = useLibraryStore()
const auth = useAuthStore()
const recommend = useRecommendStore()
const homeFeed = useHomeFeedStore()
const history = useHistoryStore()
const settings = useSettingsStore()
const toast = useToastStore()
const { t, locale } = useI18n()
const log = createLogger('home')

const continuePlaylists = ref<ContinuePlaylist[]>([])
const continueLoading = ref(true)
const continueError = ref(false)
const availableContinuePlaylists = computed(() => continuePlaylists.value.filter(playlist =>
  settings.internationalizationEnabled || playlist.source.toLowerCase() !== 'youtubemusic').slice(0, 12))
let continueRequest = 0
let homeUnmounted = false
let unlistenPlaylistsChanged: UnlistenFn | null = null
const homeActive = ref(true)

/// 已有内容时静默刷新：切回首页不再闪加载态、转刷新按钮
async function loadContinuePlaylists() {
  const request = ++continueRequest
  continueLoading.value = continuePlaylists.value.length === 0
  continueError.value = false
  try {
    const [usage, local] = await Promise.allSettled([
      invoke<unknown>('get_playlist_usage_stats'),
      invoke<unknown>('get_home_local_playlists'),
    ])
    if (request !== continueRequest || homeUnmounted) return
    if (usage.status === 'rejected') throw usage.reason
    if (local.status === 'rejected') {
      continueError.value = true
      log.error('Load local continue playlists failed:', local.reason)
    }
    continuePlaylists.value = normalizeContinuePlaylists(usage.value, local.status === 'fulfilled' ? local.value : []).map(playlist => ({
      ...playlist,
      coverUrl: playlist.coverUrl && !/^(https?:|asset:|data:|blob:)/i.test(playlist.coverUrl)
        ? convertFileSrc(playlist.coverUrl) : playlist.coverUrl,
    }))
  } catch (error) {
    if (request !== continueRequest || homeUnmounted) return
    continueError.value = true
    log.error('Load continue playlists failed:', error)
  } finally {
    if (request === continueRequest && !homeUnmounted) continueLoading.value = false
  }
}

function openContinuePlaylist(playlist: ContinuePlaylist) {
  const target = continuePlaylistRoute(playlist)
  if (target) void router.push(target)
}

onActivated(() => {
  homeActive.value = true
  void loadContinuePlaylists()
  refreshNeteaseHome()
})
onDeactivated(() => {
  homeActive.value = false
  homeFeed.deactivate()
})
onMounted(async () => {
  try {
    const stop = await listen('playlists-changed', () => void loadContinuePlaylists())
    if (homeUnmounted) stop()
    else unlistenPlaylistsChanged = stop
  } catch (error) {
    log.error('Listen for playlist changes failed:', error)
  }
})
onUnmounted(() => {
  homeUnmounted = true
  continueRequest++
  unlistenPlaylistsChanged?.()
  homeFeed.deactivate()
})

const showNotifications = ref(false)
useEscapeClose(() => showNotifications.value, () => { showNotifications.value = false })
const homeSearchQuery = ref('')

function submitHomeSearch() {
  const q = homeSearchQuery.value.trim()
  if (!q) return
  void router.push({
    name: 'explore',
    query: { q, platform: settings.internationalizationEnabled ? 'youtube' : 'netease' },
  })
}

const neteaseHomeSections = computed(() => settings.internationalizationEnabled ? [] :
  NETEASE_HOME_SECTIONS.filter(section => !section.requiresLogin || auth.netease.loggedIn))
type NeteaseHomeDefinition = (typeof NETEASE_HOME_SECTIONS)[number]
type NeteaseHomeBlock =
  | { key: string; definition: NeteaseHomeDefinition; pair?: undefined }
  | { key: string; pair: NeteaseHomeDefinition[]; definition?: undefined }

const CHART_PAIR = new Set(['top_hot', 'top_new'])

// 热歌榜与新歌榜合成一个并排块，放在原先热歌榜的位置
const neteaseHomeBlocks = computed<NeteaseHomeBlock[]>(() => {
  const charts = neteaseHomeSections.value.filter(section => CHART_PAIR.has(section.key))
  const blocks: NeteaseHomeBlock[] = []
  for (const definition of neteaseHomeSections.value) {
    if (!CHART_PAIR.has(definition.key) || charts.length < 2) blocks.push({ key: definition.key, definition })
    else if (definition.key === charts[0].key) blocks.push({ key: charts.map(chart => chart.key).join('+'), pair: charts })
  }
  return blocks
})
let neteaseSessionRevision = 0

function refreshNeteaseHome(force = false) {
  const accountKey = JSON.stringify([auth.netease.nickname, auth.netease.avatarUrl, neteaseSessionRevision])
  if (settings.internationalizationEnabled || !homeActive.value) {
    homeFeed.deactivate(auth.netease.loggedIn, accountKey)
    return
  }
  void homeFeed.refresh(auth.netease.loggedIn, accountKey, force)
}

watch([() => settings.internationalizationEnabled, () => auth.netease], (_, previous) => {
  const accountChanged = Boolean(previous?.[1] && previous[1] !== auth.netease)
  if (accountChanged) neteaseSessionRevision++
  refreshNeteaseHome(accountChanged)
}, { immediate: true })

// 搜索框里提示当前实际绑定的快捷键（默认 Ctrl/⌘+F，可在设置里改）
const searchShortcutLabel = computed(() => {
  const combo = effectiveShortcuts(settings.shortcutBindings, 'local').search
  return combo ? comboLabelParts(combo).join(' ') : ''
})
const todayLabel = computed(() => new Intl.DateTimeFormat(locale.value, { month: 'long', day: 'numeric', weekday: 'long' }).format(new Date()))

const greeting = computed(() => {
  const h = new Date().getHours()
  if (h < 6) return t('home.greeting_night')
  if (h < 12) return t('home.greeting_morning')
  if (h < 18) return t('home.greeting_afternoon')
  return t('home.greeting_evening')
})

// 最近播放（真实历史记录）
const recentTracks = computed(() => {
  return history.entries.filter(entry =>
    settings.internationalizationEnabled || getPlaybackSourceKind(entry.track) !== 'youtube',
  ).slice(0, 12).map(e => e.track)
})

// 本地音乐
const localTracks = computed(() => {
  return library.tracks.slice(0, 12)
})

// 用户歌单（多平台合并）
const myPlaylists = computed(() => {
  const all = Object.entries(recommend.userPlaylists)
    .filter(([platform]) => platform !== 'bilibili' && (platform !== 'youtube' || settings.internationalizationEnabled))
    .flatMap(([platform, list]) => list.map(pl => ({ ...pl, platform })))
  return all.slice(0, 12)
})

interface QuickTile {
  key: string
  name: string
  coverUrl?: string
  meta: string
  open: () => void
  menu?: (event: MouseEvent) => void
}

const QUICK_TILE_LIMIT = 8

// 快速入口：继续播放优先，余下用我的歌单补齐到 8 个，去重后保持原顺序
const quickTiles = computed<QuickTile[]>(() => {
  const tiles: QuickTile[] = availableContinuePlaylists.value.map(playlist => ({
    key: `continue:${playlist.key}`,
    name: playlist.name,
    coverUrl: playlist.coverUrl,
    meta: t('player.track_count', { count: playlist.trackCount }),
    open: () => openContinuePlaylist(playlist),
  }))
  const names = new Set(tiles.map(tile => tile.name))
  for (const pl of myPlaylists.value) {
    if (tiles.length >= QUICK_TILE_LIMIT) break
    if (names.has(pl.name)) continue
    names.add(pl.name)
    tiles.push({
      key: `mine:${pl.platform}:${pl.id}`,
      name: pl.name,
      coverUrl: pl.coverUrl,
      meta: t('home.my_playlists'),
      open: () => openPlatformPlaylist(pl),
      menu: event => openPlaylistMenu(event, pl.platform, pl.id),
    })
  }
  // 双列/四列布局下奇数个会剩半行，超过 2 个时截成偶数
  const count = Math.min(QUICK_TILE_LIMIT, tiles.length)
  return tiles.slice(0, count > 2 ? count - (count % 2) : count)
})

const bilibiliPlaylists = computed(() => (recommend.userPlaylists.bilibili || []).slice(0, 12))

// 首页 YouTube 推荐随国际化开关显示，仅在当前登录会话复用
const showYoutubeFeed = computed(() => settings.internationalizationEnabled && auth.youtube.loggedIn)
const youtubeHomeSections = computed(() => buildYoutubeHomeSections(recommend.homeFeedShelves))
const homeRefreshing = computed(() => continueLoading.value || (settings.internationalizationEnabled
  ? recommend.homeFeedLoading : Object.values(homeFeed.sections).some(section => section.loading)))

function refreshHome() {
  void loadContinuePlaylists()
  refreshNeteaseHome(true)
  if (showYoutubeFeed.value) void recommend.fetchHomeFeed()
  if (!settings.internationalizationEnabled && auth.bilibili.loggedIn) void recommend.fetchUserPlaylists('bilibili')
}

function playHomeSongs(songs: TrackInfo[], index: number) {
  if (songs[index]) player.playAll(songs, songs[index].id, songs[index].playlistKey)
}

// 右键菜单里的「播放」与点击行为一致：推荐歌曲整组入队，最近播放、本地音乐卡片单曲播放
const trackMenuRef = ref<InstanceType<typeof TrackContextMenu> | null>(null)
let playFromMenu: (() => void) | null = null

function openSongMenu(event: MouseEvent, songs: TrackInfo[], index: number) {
  const track = songs[index]
  if (!track) return
  playFromMenu = () => playHomeSongs(songs, index)
  trackMenuRef.value?.open(event, track)
}

function openCardMenu(event: MouseEvent, track: TrackInfo) {
  playFromMenu = () => player.play(track)
  trackMenuRef.value?.open(event, track)
}

function platformPlaylistTarget(platform: string | undefined, rawId: string | number): CollectionMenuTarget {
  const id = String(rawId)
  if (platform === 'bilibili') {
    return { route: { name: 'bili-playlist', params: { mediaId: id } }, webUrl: collectionWebUrl('bilibili', 'playlist', id) }
  }
  if (platform === 'youtube') {
    return { route: { name: 'youtube-playlist', params: { browseId: id } }, webUrl: collectionWebUrl('youtube', 'playlist', id) }
  }
  return { route: { name: 'netease-playlist', params: { id } }, webUrl: collectionWebUrl('netease', 'playlist', id) }
}

const collectionMenuRef = ref<InstanceType<typeof CollectionContextMenu> | null>(null)

function openPlaylistMenu(event: MouseEvent, platform: string | undefined, id: string | number) {
  collectionMenuRef.value?.open(event, platformPlaylistTarget(platform, id))
}

function openPlatformPlaylist(pl: { platform?: string; id: string | number }) {
  void router.push(platformPlaylistTarget(pl.platform, pl.id).route)
}

// 启动时恢复上次扫描 + 拉取推荐
onMounted(() => {
  void library.ensureStarted()
  if (auth.netease.loggedIn) void recommend.ensureUserPlaylists('netease')
  if (auth.bilibili.loggedIn) void recommend.ensureUserPlaylists('bilibili')
})

// 登录状态变化时刷新推荐；歌单先显示缓存，本次启动第一次用到时在后台刷新
watch(() => auth.netease.loggedIn, (loggedIn) => {
  if (loggedIn) void recommend.ensureUserPlaylists('netease')
})

watch(() => auth.bilibili.loggedIn, (loggedIn) => {
  if (loggedIn) void recommend.ensureUserPlaylists('bilibili')
})

// 换号由 auth 的 notifyAuthChanged 作废缓存。这里不能拿账号对象换没换来判断：
// 启动时的登录态检查和资料刷新都会换掉它，那样每次启动都会清空缓存的 YouTube 歌单
watch([showYoutubeFeed, () => auth.youtube], ([enabled]) => {
  if (!enabled) return
  void recommend.ensureUserPlaylists('youtube')
  if (recommend.homeFeedShelves.length === 0) recommend.fetchHomeFeed()
}, { immediate: true })

// 通知历史
function openNotifications() {
  showNotifications.value = !showNotifications.value
  if (showNotifications.value) {
    toast.markAllRead()
  }
}

function formatNotifTime(ts: number): string {
  const diff = Date.now() - ts
  if (diff < 60000) return t('recent.just_now')
  if (diff < 3600000) return t('recent.minutes_ago', { count: Math.floor(diff / 60000) })
  if (diff < 86400000) return t('recent.hours_ago', { count: Math.floor(diff / 3600000) })
  return t('recent.days_ago', { count: Math.floor(diff / 86400000) })
}
</script>

<template>
  <div class="home-view">
    <div class="home-inner">
    <header class="home-topbar" data-tauri-drag-region>
      <form class="home-search" role="search" @submit.prevent="submitHomeSearch">
        <span class="material-symbols-rounded home-search-icon" aria-hidden="true">search</span>
        <input v-model="homeSearchQuery" type="search" :placeholder="t('explore.search_placeholder')" :aria-label="t('explore.search_placeholder')" data-shortcut-search />
        <kbd v-if="!homeSearchQuery && searchShortcutLabel" class="home-search-kbd" aria-hidden="true">{{ searchShortcutLabel }}</kbd>
        <button v-else type="submit" class="home-search-go" :aria-label="t('explore.title')">
          <span class="material-symbols-rounded">arrow_forward</span>
        </button>
      </form>
      <div class="notif-wrap">
        <button class="header-action" :aria-label="t('home.refresh')" :title="t('home.refresh')" :disabled="homeRefreshing" @click="refreshHome">
          <span class="material-symbols-rounded" :class="{ spinning: homeRefreshing }">refresh</span>
        </button>
        <button class="header-action" :aria-label="t('home.notifications')" :title="t('home.notifications')" @click="openNotifications">
          <span class="material-symbols-rounded">notifications</span>
          <span v-if="toast.unreadCount > 0" class="notif-badge">{{ toast.unreadCount > 99 ? '99+' : toast.unreadCount }}</span>
        </button>

        <!-- 通知历史面板 -->
        <Teleport to="body">
          <Transition name="notif-pop">
          <div v-if="showNotifications" class="notif-overlay" @click="showNotifications = false">
            <div class="notif-panel" @click.stop>
              <div class="notif-header">
                <h3>{{ t('home.notifications') }}</h3>
                <button v-if="toast.history.length > 0" class="notif-clear" @click="toast.clearHistory()">
                  {{ t('home.clear_all') }}
                </button>
              </div>
              <div v-if="toast.history.length === 0" class="notif-empty">
                <span class="material-symbols-rounded" style="font-size: 32px; opacity: 0.2">notifications_none</span>
                <p>{{ t('home.no_notifications') }}</p>
              </div>
              <div v-else class="notif-list">
                <div
                  v-for="notif in toast.history"
                  :key="notif.id"
                  class="notif-item"
                  :class="{ unread: !notif.read }"
                >
                  <span class="material-symbols-rounded notif-icon" :class="notif.type">
                    {{ notif.type === 'success' ? 'check_circle' : notif.type === 'error' ? 'error' : 'info' }}
                  </span>
                  <div class="notif-content">
                    <div class="notif-text">{{ notif.text }}</div>
                    <div class="notif-time">{{ formatNotifTime(notif.timestamp) }}</div>
                  </div>
                </div>
              </div>
            </div>
          </div>
          </Transition>
        </Teleport>
      </div>
    </header>

    <section class="home-hero">
      <h1 class="greeting">{{ greeting }}</h1>
      <p class="hero-date">{{ todayLabel }}</p>
    </section>

    <!-- 快速入口：继续播放的歌单优先，不足时用我的歌单补齐，一屏内可直接点开 -->
    <section v-if="quickTiles.length || continueLoading || continueError" class="quick-section" :aria-label="t('home.continue_play')">
      <div v-if="quickTiles.length" class="quick-grid">
        <button
          v-for="tile in quickTiles"
          :key="tile.key"
          type="button"
          class="quick-tile"
          @click="tile.open()"
          @contextmenu="tile.menu?.($event)"
        >
          <span class="quick-cover">
            <span class="material-symbols-rounded filled cover-fallback">queue_music</span>
            <BilibiliCoverImage v-if="tile.coverUrl" :src="tile.coverUrl" :alt="tile.name" />
          </span>
          <span class="quick-text">
            <span class="quick-name">{{ tile.name }}</span>
            <span class="quick-meta">{{ tile.meta }}</span>
          </span>
          <span class="quick-go material-symbols-rounded" aria-hidden="true">arrow_forward</span>
        </button>
      </div>
      <div v-else-if="continueLoading" class="section-state">
        <span class="material-symbols-rounded spinning">progress_activity</span>
        <span>{{ t('player.loading') }}</span>
      </div>
      <div v-else-if="continueError" class="section-state error">
        <span>{{ t('home.continue_load_failed') }}</span>
        <button class="section-state-action" @click="loadContinuePlaylists">{{ t('player.retry') }}</button>
      </div>
    </section>

    <template v-if="showYoutubeFeed">
      <HomeFeedSection
        v-for="section in youtubeHomeSections"
        :key="section.key"
        :definition="section"
        :title="section.title"
        :section="{ songs: section.songs, playlists: section.playlists, loading: false, error: null }"
        @play="playHomeSongs"
        @song-menu="openSongMenu"
        @playlist-menu="(event, playlist) => openPlaylistMenu(event, 'youtube', playlist.id)"
        @playlist="playlist => router.push({ name: 'youtube-playlist', params: { browseId: playlist.id } })"
      />
    </template>

    <div v-if="settings.internationalizationEnabled && !showYoutubeFeed" class="section-state">
      <span>{{ t('home.youtube_login_hint') }}</span>
      <button class="section-state-action" @click="router.push('/settings')">{{ t('settings.title') }}</button>
    </div>
    <div v-else-if="showYoutubeFeed && youtubeHomeSections.length === 0" class="section-state">
      <span v-if="recommend.homeFeedLoading" class="material-symbols-rounded spinning">progress_activity</span>
      <span>{{ recommend.homeFeedLoading ? t('player.loading') : t('home.no_recommendations') }}</span>
      <button v-if="!recommend.homeFeedLoading" class="section-state-action" @click="recommend.fetchHomeFeed()">{{ t('player.retry') }}</button>
    </div>

    <template v-for="block in neteaseHomeBlocks" :key="block.key">
      <!-- 两个榜单并排成一组，宽屏下减少同质的长列表堆叠 -->
      <div v-if="block.pair" class="chart-pair">
        <HomeFeedSection
          v-for="definition in block.pair"
          :key="definition.key"
          compact
          :definition="definition"
          :section="homeFeed.sections[definition.key]"
          @retry="homeFeed.retry(definition.key)"
          @play="playHomeSongs"
          @song-menu="openSongMenu"
        />
      </div>
      <HomeFeedSection
        v-else
        :definition="block.definition"
        :section="homeFeed.sections[block.definition.key]"
        @retry="homeFeed.retry(block.definition.key)"
        @play="playHomeSongs"
        @song-menu="openSongMenu"
        @playlist-menu="(event, playlist) => openPlaylistMenu(event, 'netease', playlist.id)"
        @playlist="playlist => router.push({ name: 'netease-playlist', params: { id: playlist.id } })"
      />
      <HomeShelf
        v-if="block.key.includes('top_new') && bilibiliPlaylists.length"
        :title="t('library.bilibili_favorites')"
        :more-label="t('home.more')"
        @more="router.push({ name: 'library', query: { tab: 'bilibili_favorites' } })"
      >
        <template #icon>
          <span class="platform-inline-icon" style="--platform-color: #00a1d6; mask-image: url('/icons/ic_bilibili.svg')" />
        </template>
        <button v-for="playlist in bilibiliPlaylists" :key="playlist.id" type="button" class="shelf-card" @click="router.push({ name: 'bili-playlist', params: { mediaId: playlist.id } })" @contextmenu="openPlaylistMenu($event, 'bilibili', playlist.id)">
          <span class="shelf-cover">
            <span class="material-symbols-rounded filled cover-fallback">video_library</span>
            <BilibiliCoverImage v-if="playlist.coverUrl" :src="playlist.coverUrl" :alt="playlist.name" />
          </span>
          <span class="shelf-name">{{ playlist.name }}</span>
          <span class="shelf-meta">{{ t('player.track_count', { count: playlist.trackCount }) }}</span>
        </button>
      </HomeShelf>
    </template>

    <HomeShelf v-if="myPlaylists.length > 0" :title="t('home.my_playlists')" :more-label="t('home.more')" @more="router.push('/library')">
      <button
        v-for="pl in myPlaylists"
        :key="`${pl.platform}:${pl.id}`"
        type="button"
        class="shelf-card"
        @click="openPlatformPlaylist(pl)"
        @contextmenu="openPlaylistMenu($event, pl.platform, pl.id)"
      >
        <span class="shelf-cover">
          <span class="material-symbols-rounded filled cover-fallback">queue_music</span>
          <BilibiliCoverImage v-if="pl.coverUrl" :src="pl.coverUrl" loading="lazy" />
        </span>
        <span class="shelf-name">{{ pl.name }}</span>
      </button>
    </HomeShelf>

    <HomeShelf v-if="recentTracks.length > 0" :title="t('home.recent_play')" :more-label="t('home.more')" @more="router.push('/recent')">
      <button
        v-for="(track, index) in recentTracks"
        :key="track.id + '-recent-' + index"
        type="button"
        class="shelf-card"
        @click="player.play(track)"
        @contextmenu="openCardMenu($event, track)"
      >
        <span class="shelf-cover">
          <span class="material-symbols-rounded filled cover-fallback">music_note</span>
          <BilibiliCoverImage v-if="track.coverUrl" :src="track.coverUrl" loading="lazy" />
          <span class="shelf-play material-symbols-rounded filled" aria-hidden="true">play_arrow</span>
        </span>
        <span class="shelf-name">{{ track.title }}</span>
        <span class="shelf-meta">{{ track.artist }}</span>
      </button>
    </HomeShelf>

    <HomeShelf v-if="localTracks.length > 0" :title="t('home.local_music')" :more-label="t('home.more')" @more="router.push('/library')">
      <button
        v-for="track in localTracks"
        :key="track.id"
        type="button"
        class="shelf-card"
        @click="player.play(track)"
        @contextmenu="openCardMenu($event, track)"
      >
        <span class="shelf-cover">
          <span class="material-symbols-rounded filled cover-fallback">music_note</span>
          <BilibiliCoverImage v-if="track.coverUrl" :src="track.coverUrl" loading="lazy" />
          <span class="shelf-play material-symbols-rounded filled" aria-hidden="true">play_arrow</span>
        </span>
        <span class="shelf-name">{{ track.title }}</span>
        <span class="shelf-meta">{{ track.artist }}</span>
      </button>
    </HomeShelf>
    <TrackContextMenu ref="trackMenuRef" @play="playFromMenu?.()" />
    <CollectionContextMenu ref="collectionMenuRef" />
    </div>
  </div>
</template>

<style scoped lang="scss">
/* 首页（Clean）：吸顶搜索栏 → 问候 → 快速入口 → 推荐与横向货架。
   内容限宽居中，超宽屏上不会把卡片拉得过散；所有列数按容器宽度而非窗口宽度计算 */
.home-view {
  container-type: inline-size;
  /* 切页淡入时整页 opacity < 1 会自成层叠上下文，若层级不高于 .content-drag-strip（z-index: 1），
     淡入全程吸顶栏都被拖动条的毛玻璃盖住，动画结束才突然冒出来；固定压在拖动条之上 */
  position: relative;
  z-index: 2;
  min-height: 100%;
  padding: 0 clamp(18px, 3.2vw, 44px) 36px;
  user-select: none;
  -webkit-user-select: none;
}

.home-inner {
  max-width: 1480px;
  margin: 0 auto;
}

/* ---------- 吸顶顶栏 ---------- */
.home-topbar {
  position: sticky;
  /* 滚动容器 .content 的顶部 padding 就是顶栏这一行，sticky 以 padding 内沿为基准，需上移一行 */
  top: calc(-1 * var(--titlebar-height, 48px));
  z-index: 5;
  display: flex;
  align-items: center;
  gap: 12px;
  /* 占用窗口顶栏这一行：与红绿灯 / 窗口按钮同一中线，右侧给 Windows 窗口按钮让位 */
  height: var(--titlebar-height, 48px);
  margin: calc(-1 * var(--titlebar-height, 48px)) calc(-1 * clamp(18px, 3.2vw, 44px)) 0;
  padding: 0 calc(clamp(18px, 3.2vw, 44px) + var(--window-controls-width, 0px)) 0 clamp(18px, 3.2vw, 44px);
  /* 渐隐玻璃底由 global.scss 统一绘制 */
}

.home-search {
  position: relative;
  display: flex;
  align-items: center;
  flex: 1 1 auto;
  max-width: 560px;
  height: 36px;
  padding: 0 6px 0 12px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-high);
  transition: background var(--duration-short), box-shadow var(--duration-short);

  &:hover { background: color-mix(in srgb, var(--md-on-surface) 5%, var(--md-surface-container-high)); }
  &:focus-within {
    background: var(--md-surface-container-lowest);
    box-shadow: 0 0 0 2px var(--md-primary), var(--shadow-1);
  }

  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 10px 0 8px;
    background: transparent;
    color: var(--md-on-surface);
    font-size: var(--fs-14);
    border: 0;
    outline: 0;

    &::placeholder { color: var(--md-on-surface-variant); opacity: 0.85; }
    &::-webkit-search-cancel-button { display: none; }
  }
}

.home-search-icon {
  font-size: 20px;
  color: var(--md-on-surface-variant);
}

.home-search-kbd {
  flex-shrink: 0;
  padding: 2px 8px;
  margin-right: 4px;
  border-radius: 6px;
  background: var(--md-surface-container-lowest);
  box-shadow: 0 0 0 1px var(--hairline), 0 1px 0 var(--hairline);
  color: var(--md-on-surface-variant);
  font-family: var(--font-family);
  font-size: var(--fs-11);
  font-weight: 600;
  letter-spacing: 0.02em;
}

.home-search-go {
  display: grid;
  place-items: center;
  flex: 0 0 30px;
  height: 30px;
  border-radius: var(--radius-full);
  background: var(--md-primary);
  color: var(--md-on-primary);

  .material-symbols-rounded { font-size: 18px; }
}

.notif-wrap {
  position: relative;
  display: flex;
  gap: 4px;
  margin-left: auto;
}

.header-action {
  position: relative;
  width: 38px;
  height: 38px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short), color var(--duration-short);

  .material-symbols-rounded { font-size: 22px; }
  &:hover { background: var(--state-hover); color: var(--md-on-surface); }
  &:disabled { cursor: default; }
}

.notif-badge {
  position: absolute;
  top: 2px;
  right: 2px;
  min-width: 17px;
  height: 17px;
  border-radius: 9px;
  background: var(--md-error);
  color: var(--md-on-error);
  font-size: var(--fs-10);
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0 4px;
  box-shadow: 0 0 0 2px var(--md-background);
  pointer-events: none;
}

/* ---------- 问候 ---------- */
.home-hero {
  padding: 22px 0 18px;
}

.greeting {
  font-size: clamp(26px, 3.4cqi, 36px);
  font-size: calc(clamp(26px, 3.4cqi, 36px) * var(--text-scale));
  font-weight: 750;
  letter-spacing: -0.02em;
  line-height: 1.15;
}

.hero-date {
  margin-top: 6px;
  font-size: var(--fs-14);
  color: var(--md-on-surface-variant);
}

/* ---------- 快速入口 ---------- */
.quick-section { margin-bottom: 36px; }

.quick-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 10px;
}

/* 快速入口只是捷径，按列数截成整行，避免最后一行孤零零一个 */
@container (max-width: 900px) {
  .quick-grid { grid-template-columns: repeat(3, minmax(0, 1fr)); }
  .quick-tile:nth-child(n + 7) { display: none; }
}
@container (max-width: 640px) {
  .quick-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}
@container (max-width: 420px) {
  .quick-grid { grid-template-columns: minmax(0, 1fr); }
  .quick-tile:nth-child(n + 5) { display: none; }
}

.quick-tile {
  position: relative;
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
  height: 60px;
  padding: 0 12px 0 0;
  overflow: hidden;
  border-radius: var(--radius-md);
  background: var(--md-surface-container-low);
  box-shadow: 0 0 0 1px var(--hairline);
  text-align: left;
  transition: background var(--duration-short), box-shadow var(--duration-short), transform var(--duration-short);

  &:hover {
    background: color-mix(in srgb, var(--md-on-surface) 3%, var(--md-surface-container-low));
    box-shadow: 0 0 0 1px var(--hairline), var(--shadow-2);

    .quick-go { opacity: 1; transform: none; }
  }

  &:active { transform: scale(0.985); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 2px; }
}

.quick-cover {
  position: relative;
  display: grid;
  place-items: center;
  flex: 0 0 60px;
  height: 60px;
  background: var(--md-surface-variant);

  :deep(img) { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
  .cover-fallback { font-size: 24px; opacity: 0.4; }
}

.quick-text {
  display: flex;
  flex: 1;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
}

.quick-name,
.quick-meta {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.quick-name { font-size: var(--fs-14); font-weight: 600; }
.quick-meta { font-size: var(--fs-12); color: var(--md-on-surface-variant); }

.quick-go {
  display: grid;
  place-items: center;
  flex: 0 0 30px;
  height: 30px;
  border-radius: var(--radius-full);
  background: var(--md-primary);
  color: var(--md-on-primary);
  font-size: 18px;
  opacity: 0;
  transform: translateX(-4px);
  transition: opacity var(--duration-short), transform var(--duration-short) var(--ease-emphasized);
}

/* 榜单并排：容器足够宽时两列，窄时回到上下排列 */
.chart-pair {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0 28px;
}

@container (max-width: 760px) {
  .chart-pair { grid-template-columns: minmax(0, 1fr); }
}

/* ---------- 货架卡片 ---------- */
.shelf-card {
  display: flex;
  flex-direction: column;
  min-width: 0;
  text-align: left;
  border-radius: var(--radius-md);

  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 3px; }

  &:hover .shelf-cover {
    transform: translateY(-3px);
    box-shadow: var(--shadow-2), inset 0 0 0 1px var(--hairline);
  }

  &:hover .shelf-play { opacity: 1; transform: none; }
}

.shelf-cover {
  position: relative;
  display: grid;
  place-items: center;
  aspect-ratio: 1;
  overflow: hidden;
  border-radius: var(--radius-md);
  background: var(--md-surface-variant);
  box-shadow: var(--shadow-1), inset 0 0 0 1px var(--hairline);
  transition: transform 220ms var(--ease-emphasized), box-shadow 220ms var(--ease-emphasized);

  :deep(img) { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
  .cover-fallback { font-size: 34px; opacity: 0.4; }
}

.shelf-play {
  position: absolute;
  right: 10px;
  bottom: 10px;
  display: grid;
  place-items: center;
  width: 40px;
  height: 40px;
  border-radius: var(--radius-full);
  background: var(--md-primary);
  color: var(--md-on-primary);
  font-size: 26px;
  box-shadow: 0 6px 16px rgb(0 0 0 / 0.28);
  opacity: 0;
  transform: translateY(6px);
  transition: opacity var(--duration-short), transform 220ms var(--ease-emphasized);
}

.shelf-name,
.shelf-meta {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.shelf-name { margin-top: 10px; font-size: var(--fs-14); font-weight: 600; }
.shelf-meta { margin-top: 2px; font-size: var(--fs-12); color: var(--md-on-surface-variant); }

.platform-inline-icon {
  display: inline-block;
  width: 20px;
  height: 20px;
  flex-shrink: 0;
  background: var(--platform-color, var(--md-primary));
  mask-size: contain;
  mask-repeat: no-repeat;
  mask-position: center;
}

/* ---------- 状态行 ---------- */
.section-state {
  min-height: 64px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  margin-bottom: 24px;
  border-radius: var(--radius-lg);
  background: var(--md-surface-container-low);
  box-shadow: 0 0 0 1px var(--hairline);
  color: var(--md-on-surface-variant);
  font-size: var(--fs-13);

  &.error { color: var(--md-error); }
  .spinning { font-size: 18px; }
}

.section-state-action {
  padding: 5px 12px;
  border-radius: var(--radius-full);
  color: var(--md-on-surface);
  background: var(--md-surface-container-high);
  font-size: var(--fs-12);
  font-weight: 600;
}

.spinning { animation: spin 1s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }

@media (prefers-reduced-motion: reduce) {
  .quick-tile, .quick-go, .shelf-cover, .shelf-play { transition: none; }
  .spinning { animation: none; }
}
</style>

<!-- 通知面板（Teleport to body，需要 non-scoped） -->
<style lang="scss">
.notif-overlay {
  position: fixed;
  inset: 0;
  z-index: 400;
  background: rgba(0, 0, 0, 0.2);
}

.notif-panel {
  position: fixed;
  top: 60px;
  right: 24px;
  width: 360px;
  max-height: 480px;
  background: var(--clean-dialog-surface);
  border-radius: var(--radius-lg);
  box-shadow: 0 8px 40px rgba(0, 0, 0, 0.3), 0 2px 8px rgba(0, 0, 0, 0.15);
  border: 1px solid var(--hairline);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  transform-origin: top right;
  z-index: 401;
}

/* 开合都走 transition，连点通知按钮时从当前状态反向，不会闪 */
.notif-pop-enter-active, .notif-pop-leave-active {
  transition: background-color 180ms ease;
  .notif-panel { transition: opacity 180ms var(--ease-decelerate, ease-out), transform 200ms var(--ease-emphasized-decel, ease-out); }
}
.notif-pop-leave-active .notif-panel { transition-duration: 120ms; transition-timing-function: var(--ease-accelerate, ease-in); }
.notif-pop-enter-from, .notif-pop-leave-to {
  background-color: transparent;
  .notif-panel { opacity: 0; transform: translateY(-8px) scale(0.97); }
}

.notif-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 20px 12px;
  border-bottom: 1px solid var(--hairline);
  flex-shrink: 0;

  h3 {
    font-size: 16px;
    font-weight: 600;
  }
}

.notif-clear {
  font-size: var(--fs-12);
  font-weight: 500;
  color: var(--md-error, #FFB4AB);
  padding: 4px 10px;
  border-radius: var(--radius-full, 999px);
  transition: background 150ms;

  &:hover { background: color-mix(in srgb, var(--md-error, #FFB4AB) 10%, transparent); }
}

.notif-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 48px 0;
  gap: 8px;
  color: var(--md-on-surface-variant);
  font-size: var(--fs-13);
}

.notif-list {
  overflow-y: auto;
  padding: 8px;
  max-height: 400px;
}

.notif-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 10px;
  transition: background 150ms;

  &:hover { background: var(--md-surface-container-high); }
  &.unread { background: color-mix(in srgb, var(--md-primary) 6%, transparent); }
}

.notif-icon {
  font-size: 18px;
  flex-shrink: 0;
  margin-top: 1px;

  &.success { color: #66BB6A; }
  &.error { color: #EF5350; }
  &.info { color: var(--md-primary); }
}

.notif-content {
  flex: 1;
  min-width: 0;
}

.notif-text {
  font-size: var(--fs-13);
  font-weight: 500;
  line-height: 1.35;
  word-break: break-word;
}

.notif-time {
  font-size: var(--fs-11);
  color: var(--md-on-surface-variant);
  opacity: 0.6;
  margin-top: 3px;
}

</style>
