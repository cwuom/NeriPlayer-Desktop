<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'

defineOptions({ name: 'LibraryView' })
import { useI18n } from 'vue-i18n'
import LocalFilesView from '@/views/LocalFilesView.vue'
import { useRecommendStore } from '@/stores/recommend'
import CloudListState from '@/components/CloudListState.vue'
import DownloadsView from '@/views/DownloadsView.vue'
import { useToastStore } from '@/stores/toast'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import M3Input from '@/components/ui/M3Input.vue'
import ContextMenu from '@/components/ui/ContextMenu.vue'
import CollectionContextMenu from '@/components/CollectionContextMenu.vue'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'
import { collectionWebUrl } from '@/utils/collectionLinks'
import {
  ARTIST_FAVORITE_SOURCES,
  favoriteKey,
  type ArtistFavoriteSource,
  type FavoritePlaylist,
} from '@/modules/library/favoriteArtists'
import { favoriteRoute, favoriteWebUrl } from '@/components/library/favoriteLinks'
import { isProtectedPlaylist, playlistDragKey, type LocalPlaylistSummary } from '@/components/library/libraryPlaylists'
import { hasProxiedCoverFallback, toDisplayableLibraryCoverUrl, useLibraryCovers } from '@/components/library/useLibraryCovers'
import { useLibraryTabs } from '@/components/library/useLibraryTabs'
import { useLocalPlaylists } from '@/components/library/useLocalPlaylists'
import { usePlaylistActions } from '@/components/library/usePlaylistActions'
import { useLocalArtists } from '@/components/library/useLocalArtists'
import { useFavoritePlaylists } from '@/components/library/useFavoritePlaylists'
import { usePlatformPlaylistSync } from '@/components/library/usePlatformPlaylistSync'

const router = useRouter()
const { t } = useI18n()
const recommend = useRecommendStore()
const toast = useToastStore()

// UI 层新增 invoke 受 ui_invoke_baseline 约束，src/modules 里也还没有这些命令的封装：
// 资料库的后端调用集中在这里，再注入给各组合函数
const libraryIpc = {
  listPlaylists: () => invoke<LocalPlaylistSummary[]>('list_playlists'),
  reorderPlaylists: (orderedIds: string[]) => invoke('reorder_playlists', { orderedIds }),
  createPlaylist: (name: string) => invoke('create_playlist', { name }),
  renamePlaylist: (id: number, name: string) => invoke('rename_playlist', { id, name }),
  deletePlaylist: (id: number) => invoke('delete_playlist', { id }),
  listFavoritePlaylists: () => invoke<unknown>('list_favorite_playlists'),
  importFollowedArtists: (source: ArtistFavoriteSource) => invoke<number>('import_followed_artists', { source }),
}

const { tabs, neteaseCategory, activeTab, tabQuery } = useLibraryTabs()

// 本地歌单：列表、多选与拖拽排序
const playlistListRef = ref<HTMLElement | null>(null)
const localPlaylists = useLocalPlaylists({ ipc: libraryIpc, activeTab, query: tabQuery, listRef: playlistListRef })
const {
  playlists,
  filteredPlaylists,
  displayName,
  isMultiSelectMode,
  selectedPlaylists,
  enterMultiSelect,
  exitMultiSelect,
  togglePlaylistSelection,
  selectAll,
  invertSelection,
  plDragKey,
  plDragOverKey,
  plDragInsertPosition,
  plDragLandingKey,
  plDragUnderGlassKeys,
  plDragGlassActive,
  plDropIndicatorVisible,
  plDropIndicatorY,
  plDropIndicatorSnap,
  plDragItemStyle,
  startPlaylistDrag,
  loadPlaylists,
} = localPlaylists

const { isLibraryCoverFailed, markLibraryCoverFailed } = useLibraryCovers()

// 新建 / 重命名 / 删除对话框与行菜单
const inputRef = ref<InstanceType<typeof M3Input>>()
const renameInputRef = ref<InstanceType<typeof M3Input>>()
const {
  showCreateDialog,
  newPlaylistName,
  openCreateDialog,
  confirmCreate,
  contextMenu,
  openContextMenu,
  openPlaylistContextMenu,
  showDeleteDialog,
  deleteTarget,
  confirmDelete,
  showRenameDialog,
  renameTarget,
  renameValue,
  playlistMenuItems,
  handlePlaylistMenuClick,
  confirmRename,
  showBatchDeleteDialog,
  requestDeleteSelected,
  confirmDeleteSelected,
} = usePlaylistActions({ ipc: libraryIpc, local: localPlaylists, createInputRef: inputRef, renameInputRef })

const collectionMenuRef = ref<InstanceType<typeof CollectionContextMenu> | null>(null)

// 本地库分类：歌单 / 歌手 / 本地文件
const {
  localCategory,
  localArtistSort,
  localArtistQuery,
  localArtistsLoading,
  showLocalArtistSort,
  localArtistSortModes,
  localArtists,
  switchLocalCategory,
  openLocalArtist,
  playLocalArtist,
  refreshLocalArtistsAfterPlaylistChange,
} = useLocalArtists({ isMultiSelectMode, exitMultiSelect })

// 所有 tab（含下载页与本地的两个分类）共用同一搜索栏，位置固定不动
const isLocalArtistSearch = computed(() => activeTab.value === 0 && localCategory.value === 'artists')
const tabSearchModel = computed({
  get: () => (isLocalArtistSearch.value ? localArtistQuery.value : tabQuery.value),
  set: (value: string) => {
    if (isLocalArtistSearch.value) localArtistQuery.value = value
    else tabQuery.value = value
  },
})
const tabSearchHint = computed(() =>
  isLocalArtistSearch.value ? t('library.local_artist_search_hint')
  : activeTab.value === 0 && localCategory.value === 'files' ? t('library.local_scan_search')
  : activeTab.value === 2 ? t('library.download_search_hint')
  : t('library.tab_search_hint'),
)

// 收藏歌单（从同步数据中获取）
const {
  favoriteCategory,
  favoriteArtistSource,
  importingArtists,
  favoriteRenderCount,
  artistFavorites,
  filteredFavoritePlaylists,
  visibleFavoritePlaylists,
  favoriteSourceLabel,
  favoriteArtistPlatformLabel,
  loadFavorites,
  cancelPendingFavoritesLoad,
  importFollowedArtists,
} = useFavoritePlaylists({ ipc: libraryIpc, query: tabQuery })

function openFavorite(fpl: FavoritePlaylist) {
  const target = favoriteRoute(fpl)
  if (target) void router.push(target)
  else toast.show(t('player.load_failed'), 'error')
}

function openFavoriteMenu(event: MouseEvent, fpl: FavoritePlaylist) {
  const target = favoriteRoute(fpl)
  if (!target) return
  collectionMenuRef.value?.open(event, { route: target, webUrl: favoriteWebUrl(fpl) })
}

onMounted(loadPlaylists)
onMounted(loadFavorites)

// 拉取云端歌单：对（登录态 × 当前 tab）响应式补拉，登录态变化后强制刷新
const {
  neteasePlaylists,
  biliPlaylists,
  youtubePlaylists,
  filteredNeteasePlaylists,
  filteredNeteaseAlbums,
  filteredBiliPlaylists,
  filteredYoutubePlaylists,
  cloudSignedIn,
  cloudPlaylistStatus,
  neteaseAlbumsStatus,
  neteaseRefreshStatus,
  retryNeteaseRefresh,
} = usePlatformPlaylistSync({ activeTab, neteaseCategory, query: tabQuery, reloadFavorites: loadFavorites })

// 监听同步完成后的歌单变更事件
let unlistenPlaylistsChanged: UnlistenFn | null = null
let libraryUnmounted = false
onMounted(async () => {
  const stop = await listen('playlists-changed', () => {
    void loadPlaylists()
    void loadFavorites()
    refreshLocalArtistsAfterPlaylistChange()
  })
  if (libraryUnmounted) stop()
  else unlistenPlaylistsChanged = stop
})
onUnmounted(() => {
  libraryUnmounted = true
  cancelPendingFavoritesLoad()
  unlistenPlaylistsChanged?.()
})
</script>

<template>
  <div class="library-view">
    <header class="lib-header">
      <h1 class="page-title">{{ t('library.title') }}</h1>
      <div class="header-actions">
        <button
          v-if="!isMultiSelectMode"
          class="header-action"
          :title="t('stats.title')"
          :aria-label="t('stats.title')"
          @click="router.push({ name: 'playback-stats' })"
        >
          <span class="material-symbols-rounded">bar_chart</span>
        </button>
        <button
          v-if="!isMultiSelectMode"
          class="header-action"
          :title="t('home.recent_play')"
          :aria-label="t('home.recent_play')"
          @click="router.push({ name: 'recent' })"
        >
          <span class="material-symbols-rounded">history</span>
        </button>
        <button v-if="activeTab === 0 && localCategory === 'playlists' && !isMultiSelectMode" class="header-action" @click="enterMultiSelect()" :title="t('common.multi_select')">
          <span class="material-symbols-rounded">checklist</span>
        </button>
        <button v-if="isMultiSelectMode" class="header-action" @click="selectAll" :title="t('common.select_all')">
          <span class="material-symbols-rounded">select_all</span>
        </button>
        <button v-if="isMultiSelectMode" class="header-action" @click="invertSelection" :title="t('common.invert_selection')">
          <span class="material-symbols-rounded">flip</span>
        </button>
        <button v-if="isMultiSelectMode" class="header-action" @click="exitMultiSelect" :title="t('common.exit_selection')">
          <span class="material-symbols-rounded">close</span>
        </button>
      </div>
    </header>

    <div class="tab-bar">
      <button
        v-for="(tab, i) in tabs"
        :key="tab.label"
        class="tab-chip"
        :class="{ active: activeTab === i }"
        @click="activeTab = i"
      >
        <span v-if="tab.brandIcon" class="tab-brand-icon" :style="{ maskImage: `url('${tab.brandIcon}')` }" aria-hidden="true" />
        <span v-else class="material-symbols-rounded" :class="{ filled: activeTab === i }" style="font-size: 18px">{{ tab.icon }}</span>
        <span>{{ tab.label }}</span>
      </button>
    </div>

    <!-- 各 tab（含下载）共用的搜索栏：单一实例固定在分类 chips 上方，
         切换视图只换 placeholder 与绑定的关键词，位置尺寸不变 -->
      <div class="collapse-row">
        <div class="collapse-clip">
          <div class="tab-search">
            <span class="material-symbols-rounded" style="font-size: 18px">search</span>
            <input
              v-model="tabSearchModel"
              type="search"
              :placeholder="tabSearchHint"
              :aria-label="tabSearchHint"
            />
            <button
              v-if="tabSearchModel"
              class="tab-search-clear"
              :aria-label="t('common.clear')"
              @click="tabSearchModel = ''"
            >
              <span class="material-symbols-rounded" style="font-size: 18px">close</span>
            </button>
          </div>
        </div>
      </div>

    <!-- tab 内容整体用统一的 fade 过渡（复用 global.scss 的 fade 类），
         避免 v-if 直切导致整页闪变 -->
    <Transition name="fade" mode="out-in">

    <!-- Tab: 本地 -->
    <div v-if="activeTab === 0" key="tab-local" class="playlist-list">
      <!-- 歌单 / 歌手分类（对齐 Android 本地页分类切换）；
           排序按钮常驻在同一行右侧，只在歌手视图内以缩放过渡出现 -->
      <div class="local-category-bar">
        <button
          v-for="category in (['playlists', 'artists', 'files'] as const)"
          :key="category"
          class="local-category-chip"
          :class="{ active: localCategory === category }"
          @click="switchLocalCategory(category)"
        >
          <span class="material-symbols-rounded" style="font-size: 17px">
            {{ category === 'playlists' ? 'queue_music' : category === 'artists' ? 'account_circle' : 'folder_open' }}
          </span>
          <span>{{ category === 'playlists' ? t('library.local_category_playlists') : category === 'artists' ? t('library.local_category_artists') : t('library.local_files') }}</span>
        </button>
        <Transition name="lib-zoom">
          <button
            v-if="localCategory === 'artists'"
            class="artist-sort-btn"
            :class="{ active: showLocalArtistSort }"
            :title="t('library.local_artist_sort')"
            :aria-label="t('library.local_artist_sort')"
            @click="showLocalArtistSort = !showLocalArtistSort"
          >
            <span class="material-symbols-rounded" style="font-size: 20px">sort</span>
          </button>
        </Transition>
      </div>

      <!-- 歌单 / 歌手两个子视图之间交叉淡入，消除整块重建的闪变 -->
      <Transition name="fade" mode="out-in">
      <!-- 歌手视图 -->
      <div v-if="localCategory === 'artists'" key="local-artists" class="local-subview">
        <!-- 排序 chips 行：展开 / 收起走折叠过渡，按钮本身保持常驻 -->
        <Transition name="lib-collapse">
          <div v-if="showLocalArtistSort" class="collapse-row">
            <div class="collapse-clip">
              <div class="artist-sort-options">
                <button
                  v-for="mode in localArtistSortModes"
                  :key="mode"
                  class="artist-sort-option"
                  :class="{ active: localArtistSort === mode }"
                  @click="localArtistSort = mode; showLocalArtistSort = false"
                >
                  {{ t(`library.local_artist_sort_${mode}`) }}
                </button>
              </div>
            </div>
          </div>
        </Transition>

        <!-- 加载 / 空态 / 网格三态与排序重排统一走容器级交叉淡入；
             歌手数可达数百，TransitionGroup 的 FLIP move 会卡，故用整容器淡入 -->
        <Transition name="lib-fade" mode="out-in">
        <div v-if="localArtistsLoading" key="loading" class="empty-tab">
          <span class="material-symbols-rounded spinning" style="font-size: 32px">progress_activity</span>
        </div>
        <div v-else-if="localArtists.length === 0" key="empty" class="empty-tab">
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">account_circle</span></div>
          <p class="empty-title">{{ t(localArtistQuery.trim() ? 'library.artist_search_empty' : 'library.local_artist_empty') }}</p>
          <p v-if="!localArtistQuery.trim()" class="empty-desc">{{ t('library.local_artist_hint') }}</p>
        </div>
        <div v-else :key="`grid-${localArtistSort}`" class="artist-grid">
          <div
            v-for="artist in localArtists"
            :key="artist.key"
            class="artist-card"
            role="button"
            tabindex="0"
            @click="openLocalArtist(artist)"
            @contextmenu="collectionMenuRef?.open($event, { route: { name: 'local-artist', params: { name: artist.name } } })"
            @keydown.enter="openLocalArtist(artist)"
            @keydown.space.prevent="openLocalArtist(artist)"
          >
            <div class="artist-cover">
              <BilibiliCoverImage v-if="artist.coverUrl" :src="artist.coverUrl" loading="lazy">
                <span class="material-symbols-rounded filled" style="font-size: 34px">account_circle</span>
              </BilibiliCoverImage>
              <span v-else class="material-symbols-rounded filled" style="font-size: 34px">account_circle</span>
              <button
                class="artist-play"
                :title="t('player.play_all')"
                :aria-label="t('player.play_all')"
                @click.stop="playLocalArtist(artist)"
              >
                <span class="material-symbols-rounded filled" style="font-size: 20px">play_arrow</span>
              </button>
            </div>
            <div class="artist-name">{{ artist.name }}</div>
            <div class="artist-count">{{ t('player.track_count', { count: artist.tracks.length }) }}</div>
          </div>
        </div>
        </Transition>
      </div>

      <LocalFilesView v-else-if="localCategory === 'files'" key="local-files" embedded :search-query="tabQuery" />

      <!-- 歌单视图 -->
      <div v-else key="local-playlists" class="local-subview">
      <!-- 新建歌单（对齐 Android：+ 新建歌单 行） -->
      <div class="new-playlist-row" @click="openCreateDialog">
        <span class="material-symbols-rounded" style="font-size: 20px">add</span>
        <span>{{ t('library.create_playlist') }}</span>
      </div>
      <div class="list-divider" />

      <!-- 歌单列表：进入 / 移除走 TransitionGroup；拖拽重排与歌单详情页
           歌曲拖拽同一套指针交互（让路位移 + 单实例落点指示线 + 拖影） -->
      <div ref="playlistListRef" class="lib-list">
      <div
        class="drop-indicator"
        :class="{ visible: plDropIndicatorVisible, snap: plDropIndicatorSnap }"
        :style="{ transform: `translate3d(0, ${plDropIndicatorY}px, 0)` }"
        aria-hidden="true"
      />
      <!-- 拖拽中切到无动画 name，释放后行归位走 FLIP move，避免与指针跟随打架 -->
      <TransitionGroup :name="plDragKey ? 'lib-static' : 'lib-list'">
      <div
        v-for="pl in filteredPlaylists"
        :key="pl.id"
        class="playlist-item"
        :class="{
          selected: selectedPlaylists.has(pl.id),
          dragging: plDragKey === playlistDragKey(pl),
          'glass-active': plDragGlassActive && plDragKey === playlistDragKey(pl),
          landing: plDragLandingKey === playlistDragKey(pl),
          'drag-under-glass': plDragUnderGlassKeys.has(playlistDragKey(pl)),
          'drag-under-glass-active': plDragGlassActive && plDragUnderGlassKeys.has(playlistDragKey(pl)),
          'drag-over-before': plDragOverKey === playlistDragKey(pl) && plDragInsertPosition === 'before',
          'drag-over-after': plDragOverKey === playlistDragKey(pl) && plDragInsertPosition === 'after',
        }"
        :data-drag-key="playlistDragKey(pl)"
        :style="plDragItemStyle(playlistDragKey(pl))"
        @click="isMultiSelectMode ? togglePlaylistSelection(pl.id) : router.push({ name: 'local-playlist', params: { id: pl.id } })"
        @contextmenu.prevent.stop="openPlaylistContextMenu($event, pl)"
      >
        <!-- 多选模式下显示复选框 -->
        <div v-if="isMultiSelectMode" class="pl-checkbox" @click.stop="togglePlaylistSelection(pl.id)">
          <span class="material-symbols-rounded" :class="{ filled: selectedPlaylists.has(pl.id) }" style="font-size: 22px">
            {{ selectedPlaylists.has(pl.id) ? 'check_circle' : 'radio_button_unchecked' }}
          </span>
        </div>
        <div class="pl-icon" :class="{ 'has-cover': pl.cover_url && !isLibraryCoverFailed('local', pl.id, pl.cover_url) }">
          <BilibiliCoverImage v-if="hasProxiedCoverFallback(pl.cover_url) && !isLibraryCoverFailed('local', pl.id, pl.cover_url)" :src="pl.cover_url!" class="pl-cover-img">
            <span class="material-symbols-rounded filled" style="font-size: 22px">library_music</span>
          </BilibiliCoverImage>
          <img
            v-else-if="pl.cover_url && !isLibraryCoverFailed('local', pl.id, pl.cover_url)"
            :src="toDisplayableLibraryCoverUrl(pl.cover_url)"
            referrerpolicy="no-referrer"
            class="pl-cover-img"
            @error="markLibraryCoverFailed('local', pl.id, pl.cover_url)"
          />
          <span v-else class="material-symbols-rounded filled" style="font-size: 22px">library_music</span>
        </div>
        <div class="pl-info">
          <div class="pl-name">{{ displayName(pl) }}</div>
          <div class="pl-count">{{ t('player.track_count', { count: pl.track_count }) }}</div>
        </div>
        <!-- 多选模式下显示排序摇杆（对齐 Android：仅拖此手柄可排序） -->
        <span
          v-if="isMultiSelectMode && !isProtectedPlaylist(pl)"
          class="pl-drag-handle material-symbols-rounded"
          style="font-size: 22px"
          :title="t('common.drag_to_reorder')"
          @pointerdown.stop="startPlaylistDrag($event, playlistDragKey(pl))"
          @click.stop
        >drag_handle</span>
        <!-- 受保护歌单不显示三点菜单 -->
        <button v-else-if="!isProtectedPlaylist(pl) && !isMultiSelectMode" class="pl-more" @click.stop="openContextMenu($event, pl)">
          <span class="material-symbols-rounded" style="font-size: 20px">more_vert</span>
        </button>
      </div>
      </TransitionGroup>
      </div>

      <!-- 多选模式底部操作栏 -->
      <Transition name="lib-bar">
      <div v-if="isMultiSelectMode" class="multi-select-bar">
        <span class="select-count">{{ t('common.selected_playlist_count', { count: selectedPlaylists.size }) }}</span>
        <button class="multi-select-action neutral" @click="selectAll">
          <span class="material-symbols-rounded" style="font-size: 18px">select_all</span>
          <span>{{ t('common.select_all') }}</span>
        </button>
        <button class="multi-select-action neutral" @click="invertSelection">
          <span class="material-symbols-rounded" style="font-size: 18px">flip</span>
          <span>{{ t('common.invert_selection') }}</span>
        </button>
        <button class="multi-select-action danger push-right" :disabled="selectedPlaylists.size === 0" @click="requestDeleteSelected">
          <span class="material-symbols-rounded" style="font-size: 18px">delete</span>
          <span>{{ t('common.delete_selected') }}</span>
        </button>
        <button class="multi-select-action ghost" @click="exitMultiSelect">
          <span>{{ t('common.cancel') }}</span>
        </button>
      </div>
      </Transition>

      <div v-if="playlists.length === 0" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">library_music</span></div>
        <p class="empty-title">{{ t('library.playlist_empty_title') }}</p>
        <p class="empty-desc">{{ t('library.playlist_empty_desc') }}</p>
      </div>
      <div v-else-if="filteredPlaylists.length === 0" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">search_off</span></div>
        <p class="empty-title">{{ t('player.no_results') }}</p>
      </div>
      </div>
      </Transition>
    </div>

    <!-- Tab: 收藏（同步的收藏歌单） -->
    <div v-else-if="activeTab === 1" key="tab-favorites" class="playlist-list">
      <!-- 歌单 / 歌手分类 (对齐 Android 收藏页分类切换) -->
      <div class="local-category-bar">
        <button
          v-for="category in (['playlists', 'artists'] as const)"
          :key="category"
          class="local-category-chip"
          :class="{ active: favoriteCategory === category }"
          @click="favoriteCategory = category"
        >
          <span class="material-symbols-rounded" style="font-size: 17px">
            {{ category === 'playlists' ? 'queue_music' : 'account_circle' }}
          </span>
          <span>{{ category === 'playlists' ? t('library.local_category_playlists') : t('library.local_category_artists') }}</span>
        </button>
      </div>

      <section v-if="favoriteCategory === 'artists'" class="favorite-artist-header">
        <div class="favorite-artist-summary">
          <h2>{{ t('library.artist_following') }}</h2>
          <span>{{ t('library.artist_count', { count: artistFavorites.length }) }}</span>
        </div>
        <div class="favorite-artist-platforms" role="group" :aria-label="t('library.artist_following')">
          <button
            v-for="source in ARTIST_FAVORITE_SOURCES"
            :key="source"
            :class="{ active: favoriteArtistSource === source }"
            :aria-pressed="favoriteArtistSource === source"
            @click="favoriteArtistSource = source"
          >
            <span v-if="favoriteArtistSource === source" class="material-symbols-rounded">check</span>
            {{ favoriteArtistPlatformLabel(source) }}
          </button>
        </div>
        <button
          v-if="favoriteArtistSource !== 'biliArtist'"
          class="favorite-artist-import"
          :disabled="importingArtists"
          @click="importFollowedArtists"
        >
          <span class="material-symbols-rounded" :class="{ spinning: importingArtists }">
            {{ importingArtists ? 'progress_activity' : 'cloud_download' }}
          </span>
          {{ t(importingArtists ? 'library.artist_import_loading' : 'library.artist_import') }}
        </button>
      </section>

      <Transition name="fade" mode="out-in">
      <TransitionGroup
        v-if="filteredFavoritePlaylists.length > 0"
        :key="'fav-' + favoriteCategory + (favoriteCategory === 'artists' ? favoriteArtistSource : '')"
        tag="div"
        name="lib-list"
        class="lib-list"
      >
        <div
          v-for="fpl in visibleFavoritePlaylists"
          :key="favoriteKey(fpl)"
          class="playlist-item"
          :class="{ 'favorite-artist-item': favoriteCategory === 'artists' }"
          role="button"
          tabindex="0"
          @click="openFavorite(fpl)"
          @contextmenu="openFavoriteMenu($event, fpl)"
          @keydown.enter="openFavorite(fpl)"
          @keydown.space.prevent="openFavorite(fpl)"
        >
          <div class="pl-icon has-cover" v-if="fpl.coverUrl && !isLibraryCoverFailed('favorite', favoriteKey(fpl), fpl.coverUrl)">
            <BilibiliCoverImage
              :src="toDisplayableLibraryCoverUrl(fpl.coverUrl)"
              class="pl-cover-img"
              loading="lazy"
              @error="markLibraryCoverFailed('favorite', favoriteKey(fpl), fpl.coverUrl)"
            />
          </div>
          <div class="pl-icon" v-else>
            <span class="material-symbols-rounded filled" style="font-size: 22px">
              {{ favoriteCategory === 'artists' ? 'account_circle' : 'bookmark' }}
            </span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ fpl.name }}</div>
            <div class="pl-count" v-if="favoriteCategory === 'artists'">{{ fpl.subtitle || favoriteArtistPlatformLabel(fpl.source) }}</div>
            <div class="pl-count" v-else>{{ t('player.track_count', { count: fpl.trackCount }) }} · {{ favoriteSourceLabel(fpl.source) }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
        </div>
      </TransitionGroup>
      <div v-else class="empty-tab" :key="'fav-empty-' + favoriteCategory">
        <div class="empty-circle">
          <span class="material-symbols-rounded" style="font-size: 40px">
            {{ favoriteCategory === 'artists' ? 'account_circle' : 'bookmark' }}
          </span>
        </div>
        <p class="empty-title">{{ t(favoriteCategory === 'artists'
          ? (tabQuery.trim() ? 'library.artist_search_empty' : 'library.artist_empty')
          : (tabQuery.trim() ? 'player.no_results' : 'explore.no_playlists')) }}</p>
        <p v-if="!tabQuery.trim()" class="empty-desc">{{ t(favoriteCategory === 'artists' ? 'library.artist_empty_hint' : 'explore.login_for_playlists') }}</p>
      </div>
      </Transition>
      <button
        v-if="visibleFavoritePlaylists.length < filteredFavoritePlaylists.length"
        class="favorite-artist-import"
        @click="favoriteRenderCount += 100"
      >{{ t('player.artist_load_more') }}</button>
    </div>

    <!-- Tab: 下载 -->
    <DownloadsView v-else-if="activeTab === 2" key="tab-downloads" embedded :search-query="tabQuery" />

    <!-- Tab: 网易云-歌单 -->
    <div v-else-if="activeTab === 3" key="tab-netease" class="playlist-list">
      <!-- 歌单 / 专辑 分类（对齐 Android 网易云板块内部分类） -->
      <div class="local-category-bar">
        <button
          v-for="category in (['playlists', 'albums'] as const)"
          :key="category"
          class="local-category-chip"
          :class="{ active: neteaseCategory === category }"
          @click="neteaseCategory = category"
        >
          <span class="material-symbols-rounded" style="font-size: 17px">
            {{ category === 'playlists' ? 'queue_music' : 'album' }}
          </span>
          <span>{{ category === 'playlists' ? t('library.tab_netease_playlists_short') : t('library.tab_netease_albums_short') }}</span>
        </button>
        <CloudListState class="platform-sync" variant="banner" :status="neteaseRefreshStatus" @retry="retryNeteaseRefresh" />
      </div>

      <!-- 歌单 / 专辑分类切换同样走交叉淡入，与本地页保持一致 -->
      <Transition name="fade" mode="out-in">
      <div v-if="neteaseCategory === 'albums'" key="ne-albums" class="local-subview">
        <TransitionGroup v-if="filteredNeteaseAlbums.length > 0 && cloudSignedIn('netease')" tag="div" name="lib-list" class="lib-list">
          <div
            v-for="album in filteredNeteaseAlbums"
            :key="album.id"
            class="playlist-item"
            @click="router.push({ name: 'netease-album', params: { id: album.id } })"
            @contextmenu="collectionMenuRef?.open($event, { route: { name: 'netease-album', params: { id: album.id } }, webUrl: collectionWebUrl('netease', 'album', album.id) })"
          >
            <div class="pl-icon" :class="{ 'has-cover': album.coverUrl && !isLibraryCoverFailed('netease-album', album.id, album.coverUrl) }">
              <img
                v-if="album.coverUrl && !isLibraryCoverFailed('netease-album', album.id, album.coverUrl)"
                :src="toDisplayableLibraryCoverUrl(album.coverUrl)"
                class="pl-cover-img"
                loading="lazy"
                referrerpolicy="no-referrer"
                @error="markLibraryCoverFailed('netease-album', album.id, album.coverUrl)"
              />
              <span v-else class="material-symbols-rounded filled" style="font-size: 22px">album</span>
            </div>
            <div class="pl-info">
              <div class="pl-name">{{ album.name }}</div>
              <div class="pl-count">{{ album.artist }} · {{ t('player.track_count', { count: album.trackCount }) }}</div>
            </div>
            <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
          </div>
        </TransitionGroup>
        <div v-else-if="recommend.userAlbums.length > 0 && cloudSignedIn('netease')" class="empty-tab">
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">search_off</span></div>
          <p class="empty-title">{{ t('player.no_results') }}</p>
        </div>
        <CloudListState
          v-else-if="cloudSignedIn('netease')"
          variant="placeholder"
          :status="neteaseAlbumsStatus"
          :loading-text="t('library.cloud_albums_loading')"
          :failed-text="t('library.cloud_albums_load_failed')"
          @retry="recommend.fetchUserAlbums()"
        >
          <template #icon>
            <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">album</span></div>
          </template>
          <p class="empty-title">{{ t('library.empty_title', { type: t('library.albums') }) }}</p>
          <p class="empty-desc">{{ t('library.empty_desc') }}</p>
        </CloudListState>
        <div v-else class="empty-tab">
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">album</span></div>
          <p class="empty-title">{{ t('library.empty_title', { type: t('library.albums') }) }}</p>
          <p class="empty-desc">{{ t('explore.login_for_playlists') }}</p>
        </div>
      </div>

      <div v-else key="ne-playlists" class="local-subview">
      <TransitionGroup v-if="filteredNeteasePlaylists.length > 0 && cloudSignedIn('netease')" tag="div" name="lib-list" class="lib-list">
        <div
          v-for="npl in filteredNeteasePlaylists"
          :key="'ne-' + npl.id"
          class="playlist-item"
          @click="router.push({ name: 'netease-playlist', params: { id: npl.id } })"
          @contextmenu="collectionMenuRef?.open($event, { route: { name: 'netease-playlist', params: { id: npl.id } }, webUrl: collectionWebUrl('netease', 'playlist', npl.id) })"
        >
          <div class="pl-icon netease">
            <img
              v-if="npl.coverUrl && !isLibraryCoverFailed('netease-playlist', npl.id, npl.coverUrl)"
              :src="toDisplayableLibraryCoverUrl(npl.coverUrl)"
              referrerpolicy="no-referrer"
              loading="lazy"
              class="pl-cover-img"
              @error="markLibraryCoverFailed('netease-playlist', npl.id, npl.coverUrl)"
            />
            <span v-else class="material-symbols-rounded filled" style="font-size: 22px">library_music</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ npl.name }}</div>
            <div class="pl-count">{{ t('library.track_count', { count: npl.trackCount || 0 }) }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
        </div>
      </TransitionGroup>
      <div v-else-if="neteasePlaylists.length > 0 && cloudSignedIn('netease')" class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">search_off</span></div>
        <p class="empty-title">{{ t('player.no_results') }}</p>
      </div>
      <CloudListState
        v-else-if="cloudSignedIn('netease')"
        variant="placeholder"
        :status="cloudPlaylistStatus('netease')"
        @retry="recommend.fetchUserPlaylists('netease')"
      >
        <template #icon>
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">cloud_queue</span></div>
        </template>
        <p class="empty-title">{{ t('explore.no_playlists') }}</p>
      </CloudListState>
      <div v-else class="empty-tab">
        <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">cloud_queue</span></div>
        <p class="empty-title">{{ t('explore.no_playlists') }}</p>
        <p class="empty-desc">{{ t('explore.login_for_playlists') }}</p>
      </div>
      </div>
      </Transition>
    </div>

    <!-- Tab: Bili 收藏夹 -->
    <div v-else-if="activeTab === 4" key="tab-bilibili" class="playlist-list">
      <template v-if="biliPlaylists.length > 0 && cloudSignedIn('bilibili')">
        <div class="platform-summary bilibili">
          <span class="platform-icon-mask" style="mask-image: url('/icons/ic_bilibili.svg')"></span>
          <div>
            <div class="platform-title">{{ t('library.bilibili_favorites') }}</div>
            <div class="platform-desc">{{ t('player.video_count', { count: biliPlaylists.reduce((sum, p) => sum + (p.trackCount || 0), 0) }) }}</div>
          </div>
          <CloudListState
            class="platform-sync"
            variant="banner"
            :status="recommend.userPlaylistsStatus.bilibili"
            @retry="recommend.fetchUserPlaylists('bilibili')"
          />
        </div>
        <TransitionGroup tag="div" name="lib-list" class="lib-list">
        <div
          v-for="bpl in filteredBiliPlaylists"
          :key="'bili-' + bpl.id"
          class="playlist-item"
          @click="router.push({ name: 'bili-playlist', params: { mediaId: bpl.id } })"
          @contextmenu="collectionMenuRef?.open($event, { route: { name: 'bili-playlist', params: { mediaId: bpl.id } }, webUrl: collectionWebUrl('bilibili', 'playlist', bpl.id) })"
        >
          <div class="pl-icon bilibili" :class="{ 'has-cover': bpl.coverUrl }">
            <BilibiliCoverImage v-if="bpl.coverUrl" :src="bpl.coverUrl" class="pl-cover-img">
              <span class="material-symbols-rounded filled" style="font-size: 22px">video_library</span>
            </BilibiliCoverImage>
            <span v-else class="material-symbols-rounded filled" style="font-size: 22px">video_library</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ bpl.name }}</div>
            <div class="pl-count">{{ t('player.video_count', { count: bpl.trackCount || 0 }) }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
        </div>
        </TransitionGroup>
        <div v-if="filteredBiliPlaylists.length === 0" class="empty-tab">
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">search_off</span></div>
          <p class="empty-title">{{ t('player.no_results') }}</p>
        </div>
      </template>
      <CloudListState
        v-else-if="cloudSignedIn('bilibili')"
        variant="placeholder"
        :status="cloudPlaylistStatus('bilibili')"
        @retry="recommend.fetchUserPlaylists('bilibili')"
      >
        <template #icon>
          <div class="empty-circle platform-empty bilibili"><span class="platform-icon-mask" style="mask-image: url('/icons/ic_bilibili.svg')"></span></div>
        </template>
        <p class="empty-title">{{ t('library.bilibili_favorites') }}</p>
        <p class="empty-desc">{{ t('explore.no_playlists') }}</p>
      </CloudListState>
      <div v-else class="empty-tab">
        <div class="empty-circle platform-empty bilibili"><span class="platform-icon-mask" style="mask-image: url('/icons/ic_bilibili.svg')"></span></div>
        <p class="empty-title">{{ t('library.bilibili_favorites') }}</p>
        <p class="empty-desc">{{ t('explore.login_for_playlists') }}</p>
      </div>
    </div>

    <!-- Tab: YouTube Music 歌单 -->
    <div v-else-if="activeTab === 5" key="tab-youtube" class="playlist-list">
      <template v-if="youtubePlaylists.length > 0 && cloudSignedIn('youtube')">
        <div class="platform-summary youtube">
          <span class="platform-icon-mask" style="mask-image: url('/icons/ic_youtube.svg')"></span>
          <div>
            <div class="platform-title">YouTube Music</div>
            <div class="platform-desc">{{ t('library.playlist_count', { count: youtubePlaylists.length }) }}</div>
          </div>
          <CloudListState
            class="platform-sync"
            variant="banner"
            :status="recommend.userPlaylistsStatus.youtube"
            @retry="recommend.fetchUserPlaylists('youtube')"
          />
        </div>
        <TransitionGroup tag="div" name="lib-list" class="lib-list">
        <div
          v-for="ypl in filteredYoutubePlaylists"
          :key="'yt-' + ypl.id"
          class="playlist-item"
          @click="router.push({ name: 'youtube-playlist', params: { browseId: ypl.id } })"
          @contextmenu="collectionMenuRef?.open($event, { route: { name: 'youtube-playlist', params: { browseId: ypl.id } }, webUrl: collectionWebUrl('youtube', 'playlist', ypl.id) })"
        >
          <div class="pl-icon youtube" :class="{ 'has-cover': ypl.coverUrl && !isLibraryCoverFailed('youtube', ypl.id, ypl.coverUrl) }">
            <BilibiliCoverImage
              v-if="hasProxiedCoverFallback(ypl.coverUrl) && !isLibraryCoverFailed('youtube', ypl.id, ypl.coverUrl)"
              :src="ypl.coverUrl!"
              class="pl-cover-img"
            >
              <span class="material-symbols-rounded filled" style="font-size: 22px">subscriptions</span>
            </BilibiliCoverImage>
            <img
              v-else-if="ypl.coverUrl && !isLibraryCoverFailed('youtube', ypl.id, ypl.coverUrl)"
              :src="toDisplayableLibraryCoverUrl(ypl.coverUrl)"
              referrerpolicy="no-referrer"
              class="pl-cover-img"
              @error="markLibraryCoverFailed('youtube', ypl.id, ypl.coverUrl)"
            />
            <span v-else class="material-symbols-rounded filled" style="font-size: 22px">subscriptions</span>
          </div>
          <div class="pl-info">
            <div class="pl-name">{{ ypl.name }}</div>
            <div class="pl-count">{{ ypl.description || 'YouTube Music' }}</div>
          </div>
          <span class="material-symbols-rounded" style="font-size: 18px; opacity: 0.3">chevron_right</span>
        </div>
        </TransitionGroup>
        <div v-if="filteredYoutubePlaylists.length === 0" class="empty-tab">
          <div class="empty-circle"><span class="material-symbols-rounded" style="font-size: 40px">search_off</span></div>
          <p class="empty-title">{{ t('player.no_results') }}</p>
        </div>
      </template>
      <CloudListState
        v-else-if="cloudSignedIn('youtube')"
        variant="placeholder"
        :status="cloudPlaylistStatus('youtube')"
        @retry="recommend.fetchUserPlaylists('youtube')"
      >
        <template #icon>
          <div class="empty-circle platform-empty youtube"><span class="platform-icon-mask" style="mask-image: url('/icons/ic_youtube.svg')"></span></div>
        </template>
        <p class="empty-title">YouTube Music</p>
        <p class="empty-desc">{{ t('explore.no_playlists') }}</p>
      </CloudListState>
      <div v-else class="empty-tab">
        <div class="empty-circle platform-empty youtube"><span class="platform-icon-mask" style="mask-image: url('/icons/ic_youtube.svg')"></span></div>
        <p class="empty-title">YouTube Music</p>
        <p class="empty-desc">{{ t('explore.login_for_playlists') }}</p>
      </div>
    </div>
    </Transition>

    <!-- 创建播放列表对话框 -->
    <M3Dialog
      v-model:open="showCreateDialog"
      :title="t('library.create_playlist')"
      icon="playlist_add"
      :confirm-text="t('library.create_playlist')"
      :confirm-disabled="!newPlaylistName.trim()"
      @confirm="confirmCreate"
    >
      <M3Input
        ref="inputRef"
        v-model="newPlaylistName"
        :placeholder="t('library.playlist_name_placeholder')"
        :maxlength="50"
        @enter="confirmCreate"
      />
    </M3Dialog>

    <ContextMenu
      :open="contextMenu.show"
      :x="contextMenu.x"
      :y="contextMenu.y"
      :items="playlistMenuItems"
      @update:open="contextMenu.show = $event"
      @click="handlePlaylistMenuClick"
    />
    <CollectionContextMenu ref="collectionMenuRef" />

    <!-- 删除确认对话框 -->
    <M3Dialog
      v-model:open="showDeleteDialog"
      :title="t('library.delete_confirm_title')"
      icon="delete"
      :confirm-text="t('library.delete_playlist')"
      confirm-danger
      @confirm="confirmDelete"
    >
      <p class="dialog-msg">{{ t('library.delete_confirm_msg', { name: deleteTarget ? displayName(deleteTarget) : '' }) }}</p>
    </M3Dialog>

    <!-- 重命名对话框 -->
    <M3Dialog
      v-model:open="showRenameDialog"
      :title="t('library.rename_playlist')"
      icon="edit"
      :confirm-text="t('common.save')"
      :confirm-disabled="!renameValue.trim() || renameValue.trim() === renameTarget?.name"
      @confirm="confirmRename"
    >
      <M3Input
        ref="renameInputRef"
        v-model="renameValue"
        :placeholder="t('library.rename_placeholder')"
        :maxlength="50"
        @enter="confirmRename"
      />
    </M3Dialog>

    <!-- 批量删除歌单确认 -->
    <M3Dialog
      v-model:open="showBatchDeleteDialog"
      :title="t('library.delete_confirm_title')"
      icon="delete"
      :confirm-text="t('common.delete_selected')"
      confirm-danger
      @confirm="confirmDeleteSelected"
    >
      <p class="dialog-msg">{{ t('library.batch_delete_playlists_msg', { count: selectedPlaylists.size }) }}</p>
    </M3Dialog>
  </div>
</template>

<style scoped lang="scss" src="./LibraryView.scss"></style>
