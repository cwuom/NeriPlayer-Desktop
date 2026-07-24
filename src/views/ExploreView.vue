<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import { useRouter } from 'vue-router'

defineOptions({ name: 'ExploreView' })
import { useI18n } from 'vue-i18n'
import { useSearchStore } from '@/stores/search'
import { usePlayerStore } from '@/stores/player'
import { useRecommendStore, type PlaylistInfo } from '@/stores/recommend'
import BilibiliCoverImage from '@/components/BilibiliCoverImage.vue'

const router = useRouter()
const { t } = useI18n()
const searchStore = useSearchStore()
const player = usePlayerStore()
const recommend = useRecommendStore()

// 搜索
type PlatformTab = 'netease' | 'bilibili' | 'youtube'

const selectedPlatform = ref<PlatformTab>('netease')
const platformTabs = computed(() => [
  { key: 'netease' as const, label: t('player.source_netease'), icon: 'cloud' },
  { key: 'bilibili' as const, label: t('player.source_bilibili'), icon: 'smart_display' },
  { key: 'youtube' as const, label: t('player.source_youtube'), icon: 'play_circle' },
])
const searchQuery = ref('')
const isFocused = ref(false)

// 网易云歌单 Tag
const TAG_KEYS = [
  'tag_all', 'tag_pop', 'tag_soundtrack', 'tag_chinese', 'tag_nostalgia', 'tag_rock',
  'tag_acg', 'tag_western', 'tag_fresh', 'tag_night', 'tag_children', 'tag_folk',
  'tag_japanese', 'tag_romantic', 'tag_study', 'tag_korean', 'tag_work', 'tag_electronic',
  'tag_cantonese', 'tag_dance', 'tag_sad', 'tag_game', 'tag_afternoon_tea', 'tag_healing',
  'tag_rap', 'tag_light_music',
] as const

// Tag key -> 网易云 API 的 cat 参数值
const TAG_TO_CAT: Record<string, string> = {
  tag_all: '全部', tag_pop: '流行', tag_soundtrack: '影视原声', tag_chinese: '华语',
  tag_nostalgia: '怀旧', tag_rock: '摇滚', tag_acg: 'ACG', tag_western: '欧美',
  tag_fresh: '清新', tag_night: '夜晚', tag_children: '儿童', tag_folk: '民谣',
  tag_japanese: '日语', tag_romantic: '浪漫', tag_study: '学习', tag_korean: '韩语',
  tag_work: '工作', tag_electronic: '电子', tag_cantonese: '粤语', tag_dance: '舞曲',
  tag_sad: '伤感', tag_game: '游戏', tag_afternoon_tea: '下午茶', tag_healing: '治愈',
  tag_rap: '说唱', tag_light_music: '轻音乐',
}

const selectedTag = ref('tag_all')

// 精品歌单（按 Tag）
const qualityPlaylists = ref<PlaylistInfo[]>([])
const isLoadingPlaylists = ref(false)
const qualityError = ref<string | null>(null)
let qualityRequestGeneration = 0

async function loadQualityByTag(tagKey: string) {
  const requestGeneration = ++qualityRequestGeneration
  selectedTag.value = tagKey
  isLoadingPlaylists.value = true
  qualityError.value = null
  try {
    const cat = TAG_TO_CAT[tagKey] || '全部'
    const playlists = await recommend.fetchHighQualityPlaylists(cat, 30)
    if (requestGeneration !== qualityRequestGeneration) return
    qualityPlaylists.value = playlists
  } catch (e) {
    if (requestGeneration !== qualityRequestGeneration) return
    qualityError.value = e instanceof Error ? e.message : String(e)
  } finally {
    if (requestGeneration === qualityRequestGeneration) {
      isLoadingPlaylists.value = false
    }
  }
}

function retryQualityPlaylists() {
  loadQualityByTag(selectedTag.value)
}

// 搜索逻辑
const isSearching = computed(() => !!searchQuery.value.trim())

let searchTimer: ReturnType<typeof setTimeout> | null = null

function selectPlatform(platform: PlatformTab) {
  if (platform === selectedPlatform.value) return
  searchStore.clear()
  selectedPlatform.value = platform
}

watch([searchQuery, selectedPlatform], ([q]) => {
  if (searchTimer) clearTimeout(searchTimer)
  if (!q.trim()) { searchStore.clear(); return }
  searchTimer = setTimeout(() => {
    searchStore.search(q, selectedPlatform.value)
  }, 300)
})

// 工具函数
function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000)
  return `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, '0')}`
}

function playResult(r: any) {
  player.play({
    id: r.id,
    title: r.title,
    artist: r.artist,
    album: r.album || '',
    durationMs: r.duration_ms,
    coverUrl: r.cover_url || '',
    audioUrl: '',
    source: r.source || selectedPlatform.value,
  })
}

function goToPlaylist(pl: PlaylistInfo) {
  router.push({ name: 'netease-playlist', params: { id: pl.id } })
}

function platformLabel(source?: string) {
  switch ((source || selectedPlatform.value).toLowerCase()) {
    case 'bilibili': return t('player.source_bilibili')
    case 'youtube': return t('player.source_youtube')
    default: return t('player.source_netease')
  }
}

function retrySearch() {
  const q = searchQuery.value.trim()
  if (q) searchStore.search(q, selectedPlatform.value)
}

// 初始化
onMounted(() => {
  // 首次加载网易云精品歌单
  if (qualityPlaylists.value.length === 0) {
    loadQualityByTag('tag_all')
  }
})
</script>

<template>
  <div class="explore-view">
    <h1 class="page-title">{{ t('explore.title') }}</h1>

    <div class="platform-tabs" role="tablist" :aria-label="t('explore.title')">
      <button
        v-for="platform in platformTabs"
        :key="platform.key"
        class="platform-tab"
        :class="{ active: selectedPlatform === platform.key }"
        type="button"
        role="tab"
        :aria-selected="selectedPlatform === platform.key"
        @click="selectPlatform(platform.key)"
      >
        <span class="material-symbols-rounded" aria-hidden="true">{{ platform.icon }}</span>
        <span>{{ platform.label }}</span>
      </button>
    </div>

    <!-- 搜索栏 -->
    <div class="search-bar" :class="{ focused: isFocused }">
      <span class="material-symbols-rounded search-icon">search</span>
      <input
        v-model="searchQuery"
        type="text"
        :placeholder="t('explore.search_placeholder')"
        @focus="isFocused = true"
        @blur="isFocused = false"
      />
      <button v-if="searchQuery" class="clear-btn" @click="searchQuery = ''; searchStore.clear()">
        <span class="material-symbols-rounded" style="font-size: 20px">close</span>
      </button>
    </div>

    <!-- 加载状态 -->
    <div v-if="isSearching && searchStore.isSearching" class="loading-state">
      <span class="material-symbols-rounded spinning">progress_activity</span>
    </div>

    <div v-else-if="isSearching && searchStore.error" class="error-state">
      <span class="material-symbols-rounded" style="font-size: 32px; opacity: 0.6">error</span>
      <p class="empty-desc">{{ t('player.load_failed') }}</p>
      <button class="retry-btn" type="button" @click="retrySearch">
        <span class="material-symbols-rounded">refresh</span>
        {{ t('player.retry') }}
      </button>
    </div>

    <!-- 搜索结果 -->
    <div v-else-if="isSearching && searchStore.results.length > 0" class="search-results">
      <div
        v-for="r in searchStore.results"
        :key="r.id"
        class="result-item"
        @click="playResult(r)"
      >
        <div class="result-cover">
          <BilibiliCoverImage v-if="r.cover_url" :src="r.cover_url" loading="lazy">
            <span class="material-symbols-rounded filled">music_note</span>
          </BilibiliCoverImage>
          <span v-else class="material-symbols-rounded filled">music_note</span>
        </div>
        <div class="result-info">
          <div class="result-title">{{ r.title }}</div>
          <div class="result-meta">{{ r.artist }}<span v-if="r.album"> · {{ r.album }}</span></div>
        </div>
        <div class="result-source">{{ platformLabel(r.source) }}</div>
        <div class="result-duration">{{ formatDuration(r.duration_ms) }}</div>
      </div>
    </div>

    <!-- 搜索无结果 -->
    <div v-else-if="isSearching && searchStore.results.length === 0" class="empty-state" style="padding: 40px 0">
      <span class="material-symbols-rounded" style="font-size: 32px; opacity: 0.4">search_off</span>
      <p class="empty-desc" style="margin-top: 8px">{{ t('explore.empty_desc') }}</p>
    </div>

    <!-- 默认内容 -->
    <template v-else-if="selectedPlatform === 'netease'">

      <!-- 网易云 Tag 选择 + 精品歌单 -->
      <div class="tag-section">
        <div class="tag-flow">
          <button
            v-for="tagKey in TAG_KEYS"
            :key="tagKey"
            class="tag-chip"
            :class="{ active: selectedTag === tagKey }"
            @click="loadQualityByTag(tagKey)"
          >
            {{ t(`explore.${tagKey}`) }}
          </button>
        </div>
      </div>

      <!-- 精品歌单网格 -->
      <div v-if="isLoadingPlaylists" class="loading-state">
        <span class="material-symbols-rounded spinning">progress_activity</span>
      </div>
      <div v-else-if="qualityError" class="error-state">
        <span class="material-symbols-rounded" style="font-size: 32px; opacity: 0.6">error</span>
        <p class="empty-desc">{{ t('player.load_failed') }}</p>
        <button class="retry-btn" type="button" @click="retryQualityPlaylists">
          <span class="material-symbols-rounded">refresh</span>
          {{ t('player.retry') }}
        </button>
      </div>
      <div v-else-if="qualityPlaylists.length > 0" class="playlist-grid">
        <div
          v-for="pl in qualityPlaylists"
          :key="pl.id"
          class="playlist-card"
          @click="goToPlaylist(pl)"
        >
          <div class="playlist-cover">
            <BilibiliCoverImage v-if="pl.coverUrl" :src="pl.coverUrl" loading="lazy" />
            <span v-else class="material-symbols-rounded filled">queue_music</span>
          </div>
          <div class="playlist-name">{{ pl.name }}</div>
          <div v-if="pl.trackCount" class="playlist-count">{{ t('library.track_count', { count: pl.trackCount }) }}</div>
        </div>
      </div>

    </template>
  </div>
</template>

<style scoped lang="scss">
.explore-view { padding: 20px 28px 32px; }

.page-title {
  font-size: 28px;
  font-weight: 700;
  letter-spacing: -0.5px;
  margin-bottom: 20px;
}

.platform-tabs {
  display: inline-grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 4px;
  width: min(100%, 440px);
  min-height: 40px;
  padding: 4px;
  margin-bottom: 12px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container);
}

.platform-tab {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  min-width: 0;
  min-height: 32px;
  padding: 0 10px;
  border-radius: var(--radius-sm);
  color: var(--md-on-surface-variant);
  font-size: 12px;
  font-weight: 600;
  transition: background var(--duration-short), color var(--duration-short);

  .material-symbols-rounded {
    flex: 0 0 auto;
    font-size: 18px;
  }

  span:last-child {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  &:hover { background: var(--md-surface-container-high); }

  &.active {
    color: var(--md-on-secondary-container);
    background: var(--md-secondary-container);
  }
}

/* 搜索栏 */
.search-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 16px;
  height: 48px;
  background: var(--md-surface-container-high);
  border-radius: var(--radius-xl);
  border: 2px solid transparent;
  transition: background var(--duration-short), border-color var(--duration-short), box-shadow var(--duration-medium);
  margin-bottom: 16px;

  &.focused {
    background: var(--md-surface-container-highest);
    border-color: var(--md-primary);
    box-shadow: 0 0 0 4px rgba(208, 188, 255, 0.08);
  }

  .search-icon {
    color: var(--md-on-surface-variant);
    font-size: 22px;
    flex-shrink: 0;
  }

  input {
    flex: 1;
    border: none;
    background: none;
    color: var(--md-on-surface);
    font-size: 14px;
    outline: none;
    font-family: inherit;

    &::placeholder { color: var(--md-on-surface-variant); }
  }
}

.clear-btn {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-variant); }
}

/* Tag 选择区 */
.tag-section {
  margin-bottom: 20px;
}

.tag-flow {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 8px;
}

.tag-chip {
  height: 32px;
  padding: 0 14px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 500;
  border: 1px solid var(--md-outline-variant);
  background: var(--md-surface);
  color: var(--md-on-surface);
  cursor: pointer;
  transition: background var(--duration-short), border-color var(--duration-short), color var(--duration-short);
  user-select: none;

  &:hover {
    background: var(--md-surface-container-high);
    border-color: var(--md-outline);
  }

  &.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
    border-color: var(--md-secondary);
  }
}

/* 歌单网格 */
.playlist-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
  gap: 14px;
}

.playlist-card {
  cursor: pointer;
  border-radius: var(--radius-md);
  overflow: hidden;
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

  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .material-symbols-rounded {
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

.playlist-count {
  font-size: 11px;
  color: var(--md-on-surface-variant);
  margin-top: 2px;
}

/* 空状态 */
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 60px 0;
}

.empty-circle {
  width: 80px;
  height: 80px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface-variant);
  margin-bottom: 20px;
  opacity: 0.6;
}

.empty-title {
  font-size: 18px;
  font-weight: 600;
  margin-bottom: 6px;
  color: var(--md-on-surface-variant);
}

.empty-desc {
  font-size: 13px;
  color: var(--md-on-surface-variant);
  opacity: 0.6;
}

/* 加载 & 搜索结果 */
.loading-state {
  display: flex;
  justify-content: center;
  padding: 40px 0;
  color: var(--md-on-surface-variant);
}

.error-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 40px 0;
  color: var(--md-on-surface-variant);
}

.retry-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 32px;
  padding: 0 12px;
  border-radius: var(--radius-full);
  color: var(--md-on-primary);
  background: var(--md-primary);
  font: inherit;
  font-size: 12px;
  cursor: pointer;

  .material-symbols-rounded { font-size: 16px; }
}

.spinning {
  font-size: 32px;
  animation: spin 1s linear infinite;
}

@keyframes spin { to { transform: rotate(360deg); } }

.search-results {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.result-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 14px;
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container); }
  &:active { transform: scale(0.99); }
}

.result-cover {
  width: 44px;
  height: 44px;
  border-radius: var(--radius-sm);
  background: var(--md-surface-container-high);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  overflow: hidden;
  color: var(--md-on-surface-variant);

  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
}

.result-info {
  flex: 1;
  min-width: 0;
}

.result-title {
  font-size: 14px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.result-meta {
  font-size: 12px;
  color: var(--md-on-surface-variant);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.result-source {
  font-size: 11px;
  font-weight: 500;
  color: var(--md-primary);
  padding: 2px 8px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--md-primary) 10%, transparent);
  text-transform: capitalize;
  flex-shrink: 0;
}

.result-duration {
  font-size: 12px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}
</style>
