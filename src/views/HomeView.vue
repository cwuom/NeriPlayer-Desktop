<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRouter } from 'vue-router'

defineOptions({ name: 'HomeView' })
import { usePlayerStore } from '@/stores/player'
import { useLibraryStore } from '@/stores/library'
import { useAuthStore } from '@/stores/auth'
import { useRecommendStore, type HomeRecommendationSong } from '@/stores/recommend'
import { useHistoryStore } from '@/stores/history'
import { useSettingsStore } from '@/stores/settings'
import { useI18n } from 'vue-i18n'
import type { TrackInfo } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'

const router = useRouter()
const player = usePlayerStore()
const library = useLibraryStore()
const auth = useAuthStore()
const recommend = useRecommendStore()
const history = useHistoryStore()
const settings = useSettingsStore()
const toast = useToastStore()
const { t } = useI18n()

const showNotifications = ref(false)
const homeSearchQuery = ref('')

function submitHomeSearch() {
  const q = homeSearchQuery.value.trim()
  if (!q) return
  void router.push({
    name: 'explore',
    query: { q, platform: settings.internationalizationEnabled ? 'youtube' : 'netease' },
  })
}

const hotSection = computed(() => recommend.homeHotSongs)
const radarSection = computed(() => recommend.homeRadarSongs)
const hotSongs = computed(() => recommend.homeHotSongs.items)
const radarSongs = computed(() => recommend.homeRadarSongs.items)
const isHomeSearchLoading = computed(() => recommend.homeHotSongs.loading || recommend.homeRadarSongs.loading)

// 首页数据全为空时显示骨架屏
const showSkeleton = computed(() =>
  (recommend.isLoading || isHomeSearchLoading.value) &&
  recommend.recommendedPlaylists.length === 0 &&
  Object.keys(recommend.userPlaylists).length === 0 &&
  hotSongs.value.length === 0 &&
  radarSongs.value.length === 0
)

const greeting = computed(() => {
  const h = new Date().getHours()
  if (h < 6) return t('home.greeting_night')
  if (h < 12) return t('home.greeting_morning')
  if (h < 18) return t('home.greeting_afternoon')
  return t('home.greeting_evening')
})

const quickAccess = computed(() => [
  { title: t('home.recent_play'), icon: 'history', color: 'var(--md-primary-container)', action: 'recent' },
  { title: t('home.favorites'), icon: 'favorite', color: 'var(--md-tertiary-container)', action: 'favorites' },
  { title: t('home.local_music'), icon: 'folder_open', color: 'var(--md-secondary-container)', action: 'local' },
  { title: t('home.downloads'), icon: 'download', color: 'var(--md-surface-container-highest)', action: 'downloads' },
])

// 最近播放（真实历史记录）
const recentTracks = computed(() => {
  return history.entries.slice(0, 12).map(e => e.track)
})

// 本地音乐
const localTracks = computed(() => {
  return library.tracks.slice(0, 20)
})

// 每日推荐歌曲（网易云）
const dailySongs = computed(() => {
  return recommend.recommendedSongs.slice(0, 20)
})

// 用户歌单（多平台合并）
const myPlaylists = computed(() => {
  const all = Object.entries(recommend.userPlaylists).flatMap(([platform, list]) =>
    list.map(pl => ({ ...pl, platform })),
  )
  return all.slice(0, 12)
})

const platformHubs = computed(() => [
  {
    key: 'netease',
    title: t('settings.netease_account'),
    subtitle: recommend.recommendedPlaylists.length > 0 ? t('home.recommended_playlists') : t('explore.login_for_recommend'),
    icon: '/icons/ic_netease.svg',
    color: '#e74c3c',
    action: () => router.push({ name: 'explore' }),
  },
  {
    key: 'bilibili',
    title: t('settings.bilibili_account'),
    subtitle: auth.bilibili.loggedIn
      ? t('home.recommended_videos')
      : t('explore.bili_hint'),
    icon: '/icons/ic_bilibili.svg',
    color: '#00a1d6',
    action: () => router.push({ name: 'explore', query: { platform: 'bilibili' } }),
  },
  {
    key: 'youtube',
    title: t('settings.youtube_account'),
    subtitle: auth.youtube.loggedIn
      ? t('player.track_count', { count: recommend.userPlaylists.youtube?.length || 0 })
      : t('explore.yt_hint'),
    icon: '/icons/ic_youtube.svg',
    color: '#ff0033',
    action: () => router.push({ name: 'explore', query: { platform: 'youtube' } }),
  },
])

const youtubeHomeItems = computed(() => recommend.homeFeedShelves.flatMap(s => s.items).slice(0, 14))

// 国际化开启时 YTM 信息流置顶按分区展示 (对齐 Android HomeScreen isInternational 分支)
const ytFeedFirst = computed(() => settings.internationalizationEnabled && auth.youtube.loggedIn)
const youtubeHomeShelves = computed(() =>
  recommend.homeFeedShelves.filter(shelf => shelf.items.length > 0).slice(0, 3))

// 三列网格分页（每页 3列 x 4行 = 12 项）
const GRID_PAGE_SIZE = 12
const hotPage = ref(0)
const radarPage = ref(0)

const hotPageItems = computed(() => {
  const start = hotPage.value * GRID_PAGE_SIZE
  return hotSongs.value.slice(start, start + GRID_PAGE_SIZE)
})
const hotTotalPages = computed(() => Math.ceil(hotSongs.value.length / GRID_PAGE_SIZE))

const radarPageItems = computed(() => {
  const start = radarPage.value * GRID_PAGE_SIZE
  return radarSongs.value.slice(start, start + GRID_PAGE_SIZE)
})
const radarTotalPages = computed(() => Math.ceil(radarSongs.value.length / GRID_PAGE_SIZE))

function searchResultToTrack(s: HomeRecommendationSong): TrackInfo {
  return {
    id: s.id,
    title: s.title,
    artist: s.artist,
    album: s.album,
    durationMs: s.duration_ms,
    coverUrl: s.cover_url || '',
    audioUrl: '',
  }
}

async function handleQuickAction(action: string) {
  if (action === 'local') {
    router.push({ name: 'library', query: { tab: 'local' } })
  } else if (action === 'recent') {
    router.push('/recent')
  } else if (action === 'favorites') {
    router.push({ name: 'library', query: { tab: 'favorites' } })
  } else if (action === 'downloads') {
    router.push({ name: 'downloads' })
  }
}

// 播放每日推荐歌曲
function playDailySong(song: any) {
  const track = {
    id: `netease:${song.id}`,
    title: song.name || '',
    artist: song.ar?.map((a: any) => a.name).join(', ') || '',
    album: song.al?.name || '',
    durationMs: song.dt || 0,
    coverUrl: song.al?.picUrl || '',
    audioUrl: '',
  }
  player.play(track)
}

function openPlatformPlaylist(pl: any) {
  if (pl.platform === 'bilibili') {
    router.push({ name: 'bili-playlist', params: { mediaId: pl.id } })
  } else if (pl.platform === 'youtube') {
    router.push({ name: 'youtube-playlist', params: { browseId: pl.id } })
  } else {
    router.push({ name: 'netease-playlist', params: { id: pl.id } })
  }
}

function openYoutubeHomeItem(item: any) {
  if (item.browseId) {
    router.push({ name: 'youtube-playlist', params: { browseId: item.browseId } })
    return
  }
  if (item.videoId) {
    player.play({
      id: `youtube:${item.videoId}`,
      title: item.title,
      artist: item.subtitle || 'YouTube Music',
      album: '',
      durationMs: 0,
      coverUrl: item.coverUrl || '',
      audioUrl: '',
    })
  }
}

// 启动时恢复上次扫描 + 拉取推荐
onMounted(() => {
  if (library.tracks.length === 0) library.restoreLastScan()
  if (recommend.recommendedPlaylists.length === 0) recommend.fetchRecommendedPlaylists()
  if (auth.netease.loggedIn) {
    if (recommend.recommendedSongs.length === 0) recommend.fetchRecommendedSongs()
    if (!recommend.userPlaylists['netease']?.length) recommend.fetchUserPlaylists('netease')
    recommend.fetchHomeSearchRecommendations()
  } else {
    recommend.clearHomeSearchRecommendations()
  }
  if (auth.bilibili.loggedIn && !recommend.userPlaylists.bilibili?.length) {
    recommend.fetchUserPlaylists('bilibili')
  }
  if (auth.youtube.loggedIn) {
    if (!recommend.userPlaylists.youtube?.length) recommend.fetchUserPlaylists('youtube')
    if (recommend.homeFeedShelves.length === 0) recommend.fetchHomeFeed()
  }
})

// 登录状态变化时刷新推荐
watch(() => auth.netease.loggedIn, (loggedIn) => {
  if (loggedIn) {
    recommend.fetchRecommendedPlaylists()
    recommend.fetchRecommendedSongs()
    recommend.fetchUserPlaylists('netease')
    recommend.fetchHomeSearchRecommendations(true)
  } else {
    recommend.clearHomeSearchRecommendations()
  }
})

watch(() => recommend.homeHotSongs.items.length, () => {
  hotPage.value = 0
})

watch(() => recommend.homeRadarSongs.items.length, () => {
  radarPage.value = 0
})

watch(() => auth.bilibili.loggedIn, (loggedIn) => {
  if (loggedIn) recommend.fetchUserPlaylists('bilibili')
})

watch(() => auth.youtube.loggedIn, (loggedIn) => {
  if (loggedIn) {
    recommend.fetchUserPlaylists('youtube')
    recommend.fetchHomeFeed()
  }
})

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
    <header class="home-header">
      <h1 class="greeting">{{ greeting }}</h1>
      <form class="home-search" role="search" @submit.prevent="submitHomeSearch">
        <input v-model="homeSearchQuery" type="search" :placeholder="t('explore.search_placeholder')" :aria-label="t('explore.search_placeholder')" data-shortcut-search />
        <button type="submit" :aria-label="t('explore.title')" :disabled="!homeSearchQuery.trim()">
          <span class="material-symbols-rounded">search</span>
        </button>
      </form>
      <div class="notif-wrap">
        <button class="header-action" @click="openNotifications">
          <span class="material-symbols-rounded">notifications</span>
          <span v-if="toast.unreadCount > 0" class="notif-badge">{{ toast.unreadCount > 99 ? '99+' : toast.unreadCount }}</span>
        </button>

        <!-- 通知历史面板 -->
        <Teleport to="body">
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
        </Teleport>
      </div>
    </header>

    <section class="quick-access">
      <div
        v-for="item in quickAccess"
        :key="item.title"
        class="quick-card"
        :style="{ '--card-bg': item.color }"
        @click="handleQuickAction(item.action)"
      >
        <div class="quick-icon-wrap">
          <span class="material-symbols-rounded filled">{{ item.icon }}</span>
        </div>
        <span class="quick-title">{{ item.title }}</span>
        <span class="material-symbols-rounded quick-arrow">chevron_right</span>
      </div>
    </section>

    <!-- 多平台入口 -->
    <section class="section platform-hubs">
      <div
        v-for="hub in platformHubs"
        :key="hub.key"
        class="platform-hub-card"
        :style="{ '--hub-color': hub.color }"
        @click="hub.action()"
      >
        <span class="platform-hub-icon" :style="{ maskImage: `url(${hub.icon})` }"></span>
        <div class="platform-hub-copy">
          <div class="platform-hub-title">{{ hub.title }}</div>
          <div class="platform-hub-subtitle">{{ hub.subtitle }}</div>
        </div>
        <span class="material-symbols-rounded">arrow_forward</span>
      </div>
    </section>

    <!-- YouTube Music 信息流（国际化开启时置顶按分区展示, 对齐 Android isInternational 分支） -->
    <template v-if="ytFeedFirst">
      <section
        v-for="shelf in youtubeHomeShelves"
        :key="'ytm-shelf-' + shelf.title"
        class="section"
      >
        <div class="section-header">
          <h2 class="section-title">
            <span class="platform-inline-icon" style="--platform-color: #ff0033; mask-image: url('/icons/ic_youtube.svg')"></span>
            {{ shelf.title || 'YouTube Music' }}
          </h2>
          <button class="section-more" @click="router.push({ name: 'explore', query: { platform: 'youtube' } })">
            <span>{{ t('home.more') }}</span>
            <span class="material-symbols-rounded" style="font-size: 18px">arrow_forward</span>
          </button>
        </div>
        <div class="daily-scroll">
          <div
            v-for="item in shelf.items.slice(0, 14)"
            :key="item.browseId || item.videoId || item.title"
            class="daily-card"
            @click="openYoutubeHomeItem(item)"
          >
            <div class="daily-cover">
              <span class="material-symbols-rounded filled cover-fallback">music_note</span>
              <BilibiliCoverImage v-if="item.coverUrl" :src="item.coverUrl" loading="lazy" />
            </div>
            <div class="daily-name">{{ item.title }}</div>
            <div class="daily-artist">{{ item.subtitle }}</div>
          </div>
        </div>
      </section>
    </template>

    <!-- 骨架屏（首次加载且无缓存时） -->
    <section v-if="showSkeleton" class="section">
      <div class="skeleton-title" />
      <div class="skeleton-grid">
        <div v-for="n in 6" :key="n" class="skeleton-card">
          <div class="skeleton-cover" />
          <div class="skeleton-text" />
          <div class="skeleton-text-short" />
        </div>
      </div>
    </section>

    <!-- 热力飙升：三列网格 + 分页箭头 -->
    <section v-if="auth.netease.loggedIn && (hotSongs.length > 0 || hotSection.loading || hotSection.error)" class="section">
      <div class="section-header">
        <h2 class="section-title">
          <span class="material-symbols-rounded filled" style="font-size: 22px; color: var(--md-error); vertical-align: middle; margin-right: 6px">bolt</span>
          {{ t('home.trending') }}
        </h2>
        <div class="grid-nav" v-if="hotTotalPages > 1">
          <button class="grid-nav-btn" :disabled="hotPage === 0" @click="hotPage--">
            <span class="material-symbols-rounded">chevron_left</span>
          </button>
          <button class="grid-nav-btn" :disabled="hotPage >= hotTotalPages - 1" @click="hotPage++">
            <span class="material-symbols-rounded">chevron_right</span>
          </button>
        </div>
      </div>
      <div v-if="hotSection.loading && hotSongs.length === 0" class="section-state">
        <span class="material-symbols-rounded spinning">progress_activity</span>
        <span>{{ t('player.loading') }}</span>
      </div>
      <div v-else-if="hotSection.error && hotSongs.length === 0" class="section-state error">
        <span>{{ hotSection.error || t('home.recommend_load_failed') }}</span>
        <button class="section-state-action" @click="recommend.fetchHomeSearchRecommendations(true)">{{ t('player.retry') }}</button>
      </div>
      <div v-else class="song-grid">
        <div
          v-for="song in hotPageItems"
          :key="song.id"
          class="song-grid-item"
          @click="player.play(searchResultToTrack(song))"
        >
          <div class="song-grid-cover">
            <span class="material-symbols-rounded filled cover-fallback">music_note</span>
            <BilibiliCoverImage v-if="song.cover_url" :src="song.cover_url" loading="lazy" />
          </div>
          <div class="song-grid-info">
            <div class="song-grid-title">{{ song.title }}</div>
            <div class="song-grid-meta">{{ song.artist }}<template v-if="song.album"> · {{ song.album }}</template></div>
          </div>
        </div>
      </div>
    </section>

    <!-- 私人雷达：三列网格 + 分页箭头 -->
    <section v-if="auth.netease.loggedIn && (radarSongs.length > 0 || radarSection.loading || radarSection.error)" class="section">
      <div class="section-header">
        <h2 class="section-title">
          <span class="material-symbols-rounded filled" style="font-size: 22px; color: var(--md-primary); vertical-align: middle; margin-right: 6px">radar</span>
          {{ t('home.radar') }}
        </h2>
        <div class="grid-nav" v-if="radarTotalPages > 1">
          <button class="grid-nav-btn" :disabled="radarPage === 0" @click="radarPage--">
            <span class="material-symbols-rounded">chevron_left</span>
          </button>
          <button class="grid-nav-btn" :disabled="radarPage >= radarTotalPages - 1" @click="radarPage++">
            <span class="material-symbols-rounded">chevron_right</span>
          </button>
        </div>
      </div>
      <div v-if="radarSection.loading && radarSongs.length === 0" class="section-state">
        <span class="material-symbols-rounded spinning">progress_activity</span>
        <span>{{ t('player.loading') }}</span>
      </div>
      <div v-else-if="radarSection.error && radarSongs.length === 0" class="section-state error">
        <span>{{ radarSection.error || t('home.recommend_load_failed') }}</span>
        <button class="section-state-action" @click="recommend.fetchHomeSearchRecommendations(true)">{{ t('player.retry') }}</button>
      </div>
      <div v-else class="song-grid">
        <div
          v-for="song in radarPageItems"
          :key="song.id"
          class="song-grid-item"
          @click="player.play(searchResultToTrack(song))"
        >
          <div class="song-grid-cover">
            <span class="material-symbols-rounded filled cover-fallback">music_note</span>
            <BilibiliCoverImage v-if="song.cover_url" :src="song.cover_url" loading="lazy" />
          </div>
          <div class="song-grid-info">
            <div class="song-grid-title">{{ song.title }}</div>
            <div class="song-grid-meta">{{ song.artist }}<template v-if="song.album"> · {{ song.album }}</template></div>
          </div>
        </div>
      </div>
    </section>

    <!-- 为你推荐（登录网易云后显示） -->
    <section v-if="recommend.recommendedPlaylists.length > 0" class="section">
      <div class="section-header">
        <h2 class="section-title">{{ t('home.for_you') }}</h2>
        <button class="section-more" @click="router.push('/explore')">
          <span>{{ t('home.more') }}</span>
          <span class="material-symbols-rounded" style="font-size: 18px">arrow_forward</span>
        </button>
      </div>
      <div class="daily-scroll">
        <div
          v-for="pl in recommend.recommendedPlaylists.slice(0, 18)"
          :key="pl.id"
          class="playlist-card"
          @click="router.push({ name: 'netease-playlist', params: { id: pl.id } })"
        >
          <div class="playlist-cover">
            <span class="material-symbols-rounded filled cover-fallback">queue_music</span>
            <BilibiliCoverImage v-if="pl.coverUrl" :src="pl.coverUrl" loading="lazy" />
          </div>
          <div class="playlist-name">{{ pl.name }}</div>
        </div>
      </div>
    </section>

    <!-- 每日推荐歌曲 -->
    <section v-if="dailySongs.length > 0" class="section">
      <div class="section-header">
        <h2 class="section-title">{{ t('home.daily_recommend') }}</h2>
      </div>
      <div class="daily-scroll">
        <div
          v-for="song in dailySongs"
          :key="song.id"
          class="daily-card"
          @click="playDailySong(song)"
        >
          <div class="daily-cover">
            <span class="material-symbols-rounded filled cover-fallback">music_note</span>
            <BilibiliCoverImage v-if="song.al?.picUrl" :src="song.al.picUrl" loading="lazy" />
          </div>
          <div class="daily-name">{{ song.name }}</div>
          <div class="daily-artist">{{ song.ar?.map((a: any) => a.name).join(', ') }}</div>
        </div>
      </div>
    </section>

    <!-- YouTube Music 首页 Feed（国际化关闭时保持在底部的单分区形态） -->
    <section v-if="!ytFeedFirst && youtubeHomeItems.length > 0" class="section">
      <div class="section-header">
        <h2 class="section-title">
          <span class="platform-inline-icon" style="--platform-color: #ff0033; mask-image: url('/icons/ic_youtube.svg')"></span>
          YouTube Music
        </h2>
        <button class="section-more" @click="router.push({ name: 'explore', query: { platform: 'youtube' } })">
          <span>{{ t('home.more') }}</span>
          <span class="material-symbols-rounded" style="font-size: 18px">arrow_forward</span>
        </button>
      </div>
      <div class="daily-scroll">
        <div
          v-for="item in youtubeHomeItems"
          :key="item.browseId || item.videoId || item.title"
          class="daily-card"
          @click="openYoutubeHomeItem(item)"
        >
          <div class="daily-cover">
            <span class="material-symbols-rounded filled cover-fallback">music_note</span>
            <BilibiliCoverImage v-if="item.coverUrl" :src="item.coverUrl" loading="lazy" />
          </div>
          <div class="daily-name">{{ item.title }}</div>
          <div class="daily-artist">{{ item.subtitle }}</div>
        </div>
      </div>
    </section>

    <!-- 我的歌单 -->
    <section v-if="myPlaylists.length > 0" class="section">
      <div class="section-header">
        <h2 class="section-title">{{ t('home.my_playlists') }}</h2>
        <button class="section-more" @click="router.push('/library')">
          <span>{{ t('home.more') }}</span>
          <span class="material-symbols-rounded" style="font-size: 18px">arrow_forward</span>
        </button>
      </div>
      <div class="daily-scroll">
        <div
          v-for="pl in myPlaylists"
          :key="pl.id"
          class="playlist-card"
          @click="openPlatformPlaylist(pl)"
        >
          <div class="playlist-cover">
            <span class="material-symbols-rounded filled cover-fallback">queue_music</span>
            <BilibiliCoverImage v-if="pl.coverUrl" :src="pl.coverUrl" loading="lazy">
              <span class="material-symbols-rounded filled">queue_music</span>
            </BilibiliCoverImage>
          </div>
          <div class="playlist-name">{{ pl.name }}</div>
        </div>
      </div>
    </section>

    <!-- 最近播放 -->
    <section v-if="recentTracks.length > 0" class="section">
      <div class="section-header">
        <h2 class="section-title">{{ t('home.recent_play') }}</h2>
        <button class="section-more" @click="router.push('/recent')">
          <span>{{ t('home.more') }}</span>
          <span class="material-symbols-rounded" style="font-size: 18px">arrow_forward</span>
        </button>
      </div>
      <div class="daily-scroll">
        <div
          v-for="(track, index) in recentTracks"
          :key="track.id + '-recent-' + index"
          class="daily-card"
          @click="player.play(track)"
        >
          <div class="daily-cover">
            <span class="material-symbols-rounded filled cover-fallback">music_note</span>
            <BilibiliCoverImage
              v-if="track.coverUrl"
              :src="track.coverUrl"
              loading="lazy"
            />
          </div>
          <div class="daily-name">{{ track.title }}</div>
          <div class="daily-artist">{{ track.artist }}</div>
        </div>
      </div>
    </section>

    <!-- 本地音乐 -->
    <section v-if="localTracks.length > 0" class="section">
      <div class="section-header">
        <h2 class="section-title">{{ t('home.local_music') }}</h2>
        <button class="section-more" @click="router.push('/library')">
          <span>{{ t('home.more') }}</span>
          <span class="material-symbols-rounded" style="font-size: 18px">arrow_forward</span>
        </button>
      </div>
      <div class="daily-scroll">
        <div
          v-for="track in localTracks"
          :key="track.id"
          class="daily-card"
          @click="player.play(track)"
        >
          <div class="daily-cover">
            <span class="material-symbols-rounded filled cover-fallback">music_note</span>
            <BilibiliCoverImage
              v-if="track.coverUrl"
              :src="track.coverUrl"
              loading="lazy"
            />
          </div>
          <div class="daily-name">{{ track.title }}</div>
          <div class="daily-artist">{{ track.artist }}</div>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped lang="scss">
.home-view {
  padding: 20px 28px 32px;
  user-select: none;
  -webkit-user-select: none;
}

.home-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 24px;
  gap: 16px;
  flex-wrap: wrap;
}

.home-search {
  display: flex;
  align-items: center;
  flex: 1 1 220px;
  max-width: 440px;
  height: 42px;
  padding-left: 16px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-high);

  &:focus-within { outline: 2px solid var(--md-primary); }
  input { width: 100%; min-width: 0; background: transparent; color: var(--md-on-surface); border: 0; outline: 0; }
  button { display: grid; place-items: center; flex: 0 0 42px; height: 42px; color: var(--md-primary); }
  button:disabled { opacity: 0.4; }
}

.greeting {
  font-size: 28px;
  font-weight: 700;
  letter-spacing: -0.5px;
}

.header-action {
  width: 40px;
  height: 40px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container-high); }
}


/* 多平台入口 */
.platform-hubs {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 10px;
}

.platform-hub-card {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 72px;
  padding: 14px;
  border-radius: 20px;
  cursor: pointer;
  overflow: hidden;
  position: relative;
  background:
    radial-gradient(circle at 18% 16%, color-mix(in srgb, var(--hub-color) 28%, transparent), transparent 38%),
    var(--md-surface-container);
  border: 1px solid color-mix(in srgb, var(--hub-color) 20%, var(--md-outline-variant));
  transition: transform var(--duration-short) var(--ease-standard), background var(--duration-short);

  &:hover {
    transform: translateY(-2px);
    background:
      radial-gradient(circle at 18% 16%, color-mix(in srgb, var(--hub-color) 34%, transparent), transparent 42%),
      var(--md-surface-container-high);
  }

  > .material-symbols-rounded {
    color: color-mix(in srgb, var(--hub-color) 70%, var(--md-on-surface));
    font-size: 20px;
    opacity: 0.75;
  }
}

.platform-hub-icon,
.platform-inline-icon {
  display: inline-block;
  background: var(--hub-color, var(--platform-color, var(--md-primary)));
  mask-size: contain;
  mask-repeat: no-repeat;
  mask-position: center;
  -webkit-mask-size: contain;
  -webkit-mask-repeat: no-repeat;
  -webkit-mask-position: center;
  flex-shrink: 0;
}

.platform-hub-icon {
  width: 28px;
  height: 28px;
}

.platform-hub-copy {
  flex: 1;
  min-width: 0;
}

.platform-hub-title {
  font-size: 13px;
  font-weight: 800;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.platform-hub-subtitle {
  margin-top: 3px;
  font-size: 11px;
  color: var(--md-on-surface-variant);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.platform-inline-icon {
  width: 20px;
  height: 20px;
  vertical-align: -4px;
  margin-right: 6px;
}

@media (max-width: 900px) {
  .platform-hubs { grid-template-columns: 1fr; }
}

/* 快捷卡片 */
.quick-access {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 10px;
  margin-bottom: 32px;
}

.quick-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  border-radius: var(--radius-lg);
  background: var(--card-bg);
  cursor: pointer;
  position: relative;
  overflow: hidden;
  transition: transform var(--duration-short) var(--ease-standard);

  // M3 state layer：叠加半透明层而非 brightness 滤镜
  &::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: var(--md-on-surface);
    opacity: 0;
    transition: opacity var(--duration-short);
    pointer-events: none;
  }

  &:hover {
    transform: translateY(-1px);
    &::after { opacity: 0.08; }
  }
  &:active {
    transform: scale(0.98);
    &::after { opacity: 0.12; }
  }
}

.quick-icon-wrap {
  width: 36px;
  height: 36px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--md-on-surface) 10%, transparent);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;

  .material-symbols-rounded { font-size: 20px; }
}

.quick-title {
  flex: 1;
  font-size: 13px;
  font-weight: 600;
  letter-spacing: 0.1px;
}

.quick-arrow {
  font-size: 18px !important;
  opacity: 0.4;
}

/* 段落 */
.section { margin-bottom: 24px; }

/* 三列歌曲网格（YouTube Music 风格） */
.song-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 4px 16px;
}

.song-grid-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 10px;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background var(--duration-short);
  min-width: 0;

  &:hover { background: var(--md-surface-container-high); }
  &:active { background: var(--md-surface-container-highest); }
}

.song-grid-cover {
  width: 48px;
  height: 48px;
  border-radius: var(--radius-sm);
  background: var(--md-surface-variant);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  position: relative;

  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
  .cover-fallback { font-size: 24px; opacity: 0.4; }
}

.song-grid-info {
  flex: 1;
  min-width: 0;
}

.song-grid-title {
  font-size: 14px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  line-height: 1.45;
}

.song-grid-meta {
  font-size: 12px;
  color: var(--md-on-surface-variant);
  margin-top: 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  line-height: 1.45;
}

/* 分页导航箭头 */
.grid-nav {
  display: flex;
  align-items: center;
  gap: 4px;
}

.grid-nav-btn {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface);
  transition: background var(--duration-short);
  border: 1px solid var(--md-outline-variant);

  &:hover:not(:disabled) { background: var(--md-surface-container-high); }
  &:disabled {
    opacity: 0.3;
    cursor: default;
  }

  .material-symbols-rounded { font-size: 20px; }
}

.section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}

.section-title {
  font-size: 18px;
  font-weight: 600;
}

.section-more {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 13px;
  color: var(--md-primary);
  font-weight: 500;
  padding: 6px 12px;
  border-radius: var(--radius-full);
  transition: background var(--duration-short);

  &:hover { background: color-mix(in srgb, var(--md-primary) 8%, transparent); }
}

.section-state {
  min-height: 72px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  color: var(--md-on-surface-variant);
  font-size: 13px;

  &.error {
    color: var(--md-error);
  }

  .spinning {
    font-size: 18px;
    animation: spin 1s linear infinite;
  }
}

.section-state-action {
  padding: 5px 10px;
  border-radius: var(--radius-full);
  color: var(--md-primary);
  background: color-mix(in srgb, var(--md-primary) 10%, transparent);
  font-size: 12px;
  font-weight: 600;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

/* 网格容器（所有板块统一使用，自动换行） */
.daily-scroll {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
  gap: 14px 12px;
}

.daily-card {
  cursor: pointer;
  border-radius: var(--radius-md);
  transition: transform var(--duration-short) var(--ease-standard);

  &:hover { transform: translateY(-2px); }
}

.daily-cover {
  aspect-ratio: 1;
  border-radius: var(--radius-md);
  background: var(--md-surface-variant);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;

  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
  .cover-fallback { font-size: 32px; opacity: 0.4; }
}

.daily-name {
  font-size: 12px;
  font-weight: 500;
  margin-top: 6px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.daily-artist {
  font-size: 11px;
  color: var(--md-on-surface-variant);
  margin-top: 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 歌单卡片 */
.playlist-card {
  cursor: pointer;
  transition: transform var(--duration-short) var(--ease-standard);

  &:hover { transform: translateY(-2px); }
}

.playlist-cover {
  aspect-ratio: 1;
  border-radius: var(--radius-md);
  background: var(--md-surface-variant);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;

  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .cover-fallback {
    font-size: 32px;
    opacity: 0.4;
  }
}

.playlist-name {
  font-size: 12px;
  font-weight: 500;
  margin-top: 6px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 通知按钮包裹 */
.notif-wrap {
  position: relative;
}

.notif-badge {
  position: absolute;
  top: -2px;
  right: -2px;
  min-width: 18px;
  height: 18px;
  border-radius: 9px;
  background: var(--md-error);
  color: var(--md-on-error);
  font-size: 10px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0 4px;
  pointer-events: none;
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
  background: var(--md-surface-container);
  border-radius: 16px;
  box-shadow: 0 8px 40px rgba(0, 0, 0, 0.3), 0 2px 8px rgba(0, 0, 0, 0.15);
  border: 1px solid var(--md-outline-variant);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: notif-in 180ms var(--ease-decelerate, ease-out);
  z-index: 401;
}

@keyframes notif-in {
  from { opacity: 0; transform: translateY(-8px) scale(0.97); }
  to { opacity: 1; transform: translateY(0) scale(1); }
}

.notif-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 20px 12px;
  border-bottom: 1px solid var(--md-outline-variant);
  flex-shrink: 0;

  h3 {
    font-size: 16px;
    font-weight: 600;
  }
}

.notif-clear {
  font-size: 12px;
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
  font-size: 13px;
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
  font-size: 13px;
  font-weight: 500;
  line-height: 1.35;
  word-break: break-word;
}

.notif-time {
  font-size: 11px;
  color: var(--md-on-surface-variant);
  opacity: 0.6;
  margin-top: 3px;
}

// 骨架屏
.skeleton-title {
  width: 120px;
  height: 22px;
  border-radius: var(--radius-sm);
  background: var(--md-surface-container-high);
  margin-bottom: 16px;
  animation: skeleton-shimmer 1.5s ease-in-out infinite;
}

.skeleton-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 14px;
}

.skeleton-card {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.skeleton-cover {
  width: 100%;
  aspect-ratio: 1;
  border-radius: var(--radius-md);
  background: var(--md-surface-container-high);
  animation: skeleton-shimmer 1.5s ease-in-out infinite;
}

.skeleton-text {
  width: 80%;
  height: 13px;
  border-radius: var(--radius-xs);
  background: var(--md-surface-container-high);
  animation: skeleton-shimmer 1.5s ease-in-out 0.1s infinite;
}

.skeleton-text-short {
  width: 50%;
  height: 11px;
  border-radius: var(--radius-xs);
  background: var(--md-surface-container-high);
  animation: skeleton-shimmer 1.5s ease-in-out 0.2s infinite;
}

@keyframes skeleton-shimmer {
  0%, 100% { opacity: 0.4; }
  50% { opacity: 0.8; }
}
</style>
