<script setup lang="ts">
// 歌词匹配面板：选平台、改关键字，列出打过分的候选；点开先预览，确认后再应用
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  LYRIC_MATCH_SOURCES,
  defaultLyricMatchKeyword,
  defaultLyricMatchSources,
  formatMatchDelta,
  formatMatchDuration,
  matchLyrics,
  type LyricMatchResult,
  type LyricMatchSource,
} from '@/modules/lyrics/lyricMatch'
import { createLogger } from '@/utils/logger'

const props = defineProps<{
  title: string
  artist: string
  album?: string
  durationMs: number
  playbackSource?: string | null
  preferWordTimed: boolean
  applyLabel: string
}>()

const emit = defineEmits<{ pick: [result: LyricMatchResult] }>()

const { t } = useI18n()
const log = createLogger('lyric-match')

const PREVIEW_LINES = 8
const SOURCE_ICONS: Record<LyricMatchSource, string> = {
  kugou: '/icons/ic_kugou.svg',
  netease: '/icons/ic_netease.svg',
  qq: '/icons/ic_qq_music.svg',
  lrclib: '/icons/ic_lrclib.svg',
  amll_ttml: '/icons/ic_amll.svg',
}

const keyword = ref(defaultLyricMatchKeyword(props.title, props.artist))
const selectedSources = ref<LyricMatchSource[]>(defaultLyricMatchSources(props.playbackSource))
const results = ref<LyricMatchResult[]>([])
const searching = ref(false)
const searched = ref(false)
const error = ref('')
const expandedKey = ref<string | null>(null)
let generation = 0

function sourceLabel(source: LyricMatchSource): string {
  switch (source) {
    case 'netease': return t('player.source_netease')
    case 'qq': return t('player.source_qq')
    case 'kugou': return t('player.lyric_source_kugou')
    case 'lrclib': return 'LRCLIB'
    case 'amll_ttml': return 'AMLL TTML'
  }
}

function resultKey(result: LyricMatchResult, index: number): string {
  return `${result.source}:${result.id}:${index}`
}

function toggleSource(source: LyricMatchSource) {
  const next = new Set(selectedSources.value)
  if (next.has(source)) next.delete(source)
  else next.add(source)
  selectedSources.value = LYRIC_MATCH_SOURCES.filter(value => next.has(value))
}

const noSource = computed(() => selectedSources.value.length === 0)

async function search() {
  const query = keyword.value.trim()
  if (!query || noSource.value) return
  const request = ++generation
  searching.value = true
  error.value = ''
  expandedKey.value = null
  try {
    const found = await matchLyrics({
      keyword: query,
      title: props.title,
      artist: props.artist,
      album: props.album || '',
      durationMs: props.durationMs,
      preferWordTimed: props.preferWordTimed,
      sources: [...selectedSources.value],
    })
    if (request !== generation) return
    results.value = found
    searched.value = true
  } catch (cause) {
    if (request !== generation) return
    log.warn('match_lyrics failed:', cause)
    results.value = []
    error.value = String(cause)
    searched.value = true
  } finally {
    if (request === generation) searching.value = false
  }
}

function metaParts(result: LyricMatchResult): string[] {
  const parts = [sourceLabel(result.source)]
  if (result.wordTimed) parts.push(t('player.lyrics_match_word_timed'))
  const duration = formatMatchDuration(result.durationMs)
  if (duration) parts.push(duration)
  if (result.durationDeltaMs != null && result.durationDeltaMs > 0) {
    parts.push(t('player.lyrics_match_delta', { value: formatMatchDelta(result.durationDeltaMs) }))
  }
  parts.push(t('player.lyrics_match_score', { score: result.score }))
  return parts
}

function toggleExpanded(key: string) {
  expandedKey.value = expandedKey.value === key ? null : key
}

/** 展开后把这一项连同应用按钮滚进可视区 */
function revealPreview(element: unknown) {
  if (!(element instanceof HTMLElement)) return
  element.parentElement?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
}

onMounted(() => { void search() })
onUnmounted(() => { generation++ })
</script>

<template>
  <div class="lyric-match">
    <div class="lyric-match-bar">
      <input
        v-model="keyword"
        class="lyric-match-input"
        :placeholder="t('player.lyrics_match_keyword_hint')"
        :aria-label="t('player.lyrics_match_keyword')"
        @keydown.enter="search"
      />
      <button class="lyric-match-search" :disabled="searching || noSource || !keyword.trim()" :title="t('player.search_song')" @click="search">
        <span class="material-symbols-rounded" :class="{ spinning: searching }">{{ searching ? 'progress_activity' : 'search' }}</span>
      </button>
    </div>

    <div class="lyric-match-label">{{ t('player.lyrics_match_sources') }}</div>
    <div class="lyric-match-sources">
      <button
        v-for="source in LYRIC_MATCH_SOURCES"
        :key="source"
        class="lyric-match-chip"
        :class="{ active: selectedSources.includes(source) }"
        :aria-pressed="selectedSources.includes(source)"
        @click="toggleSource(source)"
      >
        <span class="lyric-match-chip-icon" :style="{ maskImage: `url(${SOURCE_ICONS[source]})` }"></span>
        {{ sourceLabel(source) }}
      </button>
    </div>

    <div v-if="noSource" class="lyric-match-status">{{ t('player.lyrics_match_no_source') }}</div>
    <div v-else-if="searching" class="lyric-match-status">{{ t('player.lyrics_match_searching') }}</div>
    <div v-else-if="error" class="lyric-match-status error">{{ t('player.lyrics_match_failed', { error }) }}</div>
    <div v-else-if="searched && results.length === 0" class="lyric-match-status">{{ t('player.lyrics_match_empty') }}</div>

    <div v-if="!searching && results.length" class="lyric-match-results">
      <div
        v-for="(result, index) in results"
        :key="resultKey(result, index)"
        class="lyric-match-item"
        :class="{ expanded: expandedKey === resultKey(result, index) }"
      >
        <button class="lyric-match-head" @click="toggleExpanded(resultKey(result, index))">
          <span class="lyric-match-source-icon" :style="{ maskImage: `url(${SOURCE_ICONS[result.source]})` }"></span>
          <span class="lyric-match-info">
            <span class="lyric-match-title">{{ result.title || '—' }}</span>
            <span class="lyric-match-artist">{{ [result.artist, result.album].filter(Boolean).join(' · ') }}</span>
            <span class="lyric-match-meta">{{ metaParts(result).join(' · ') }}</span>
          </span>
          <span class="lyric-match-badges">
            <span class="lyric-match-confidence" :class="result.confidence">{{ t(`player.lyrics_match_confidence_${result.confidence}`) }}</span>
            <span v-if="result.hasTranslation" class="lyric-match-tag">{{ t('player.lyrics_match_has_translation') }}</span>
            <span v-if="result.hasRomanization" class="lyric-match-tag">{{ t('player.lyrics_match_has_romanization') }}</span>
          </span>
        </button>
        <div v-if="expandedKey === resultKey(result, index)" :ref="revealPreview" class="lyric-match-preview">
          <!-- 按钮放在预览前面：面板窄，放在末尾要先滚动才点得到 -->
          <div class="lyric-match-preview-actions">
            <button class="lyric-match-apply" @click="emit('pick', result)">
              <span class="material-symbols-rounded">check</span>
              {{ applyLabel }}
            </button>
            <span v-if="result.lines.length > PREVIEW_LINES" class="lyric-match-more">
              {{ t('player.lyrics_match_more_lines', { count: result.lines.length - PREVIEW_LINES }) }}
            </span>
          </div>
          <div v-for="(line, lineIndex) in result.lines.slice(0, PREVIEW_LINES)" :key="lineIndex" class="lyric-match-line">
            <span class="lyric-match-line-text">{{ line.text }}</span>
            <span v-if="line.translation" class="lyric-match-line-sub">{{ line.translation }}</span>
            <span v-if="line.roman" class="lyric-match-line-sub">{{ line.roman }}</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped lang="scss">
.lyric-match {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.lyric-match-bar {
  display: flex;
  gap: 8px;
}

.lyric-match-input {
  flex: 1;
  min-width: 0;
  padding: 10px 14px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.06);
  color: white;
  font-size: 14px;
  outline: none;
  transition: border-color 0.15s;

  &:focus { border-color: rgba(255, 255, 255, 0.3); }
  &::placeholder { color: rgba(255, 255, 255, 0.3); }
}

.lyric-match-search {
  width: 42px;
  height: 42px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.08);
  color: rgba(255, 255, 255, 0.75);
  transition: background 0.15s;

  &:hover:not(:disabled) { background: rgba(255, 255, 255, 0.14); }
  &:disabled { opacity: 0.4; }
}

.spinning { animation: lyric-match-spin 1s linear infinite; }
@keyframes lyric-match-spin { to { transform: rotate(360deg); } }

.lyric-match-label {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.45);
}

.lyric-match-sources {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.lyric-match-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px 6px 8px;
  border-radius: 999px;
  border: 1px solid rgba(255, 255, 255, 0.14);
  background: transparent;
  color: rgba(255, 255, 255, 0.6);
  font-size: 12.5px;
  transition: background 0.15s, color 0.15s, border-color 0.15s;

  &:hover { background: rgba(255, 255, 255, 0.06); }
  &.active {
    background: rgba(255, 255, 255, 0.16);
    border-color: rgba(255, 255, 255, 0.28);
    color: rgba(255, 255, 255, 0.95);
  }
}

.lyric-match-chip-icon,
.lyric-match-source-icon {
  display: block;
  flex-shrink: 0;
  background: currentColor;
  mask-size: contain;
  mask-repeat: no-repeat;
  mask-position: center;
}

.lyric-match-chip-icon { width: 16px; height: 16px; }
.lyric-match-source-icon { width: 22px; height: 22px; margin-top: 2px; color: rgba(255, 255, 255, 0.55); }

.lyric-match-status {
  text-align: center;
  color: rgba(255, 255, 255, 0.4);
  font-size: 13px;
  padding: 14px 0;

  &.error { color: #ffb4ab; }
}

.lyric-match-results {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 360px;
  overflow-y: auto;
  &::-webkit-scrollbar { display: none; }
}

.lyric-match-item {
  border-radius: 12px;
  transition: background 0.15s;

  &.expanded { background: rgba(255, 255, 255, 0.06); }
}

.lyric-match-head {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  width: 100%;
  padding: 10px 8px;
  border: none;
  border-radius: 12px;
  background: transparent;
  color: rgba(255, 255, 255, 0.88);
  text-align: left;
  cursor: pointer;

  &:hover { background: rgba(255, 255, 255, 0.05); }
}

.lyric-match-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.lyric-match-title,
.lyric-match-artist,
.lyric-match-meta {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.lyric-match-title { font-size: 14px; font-weight: 500; color: rgba(255, 255, 255, 0.92); }
.lyric-match-artist { font-size: 12px; color: rgba(255, 255, 255, 0.5); }
.lyric-match-meta { font-size: 11.5px; color: rgba(255, 255, 255, 0.38); }

.lyric-match-badges {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 4px;
  flex-shrink: 0;
}

.lyric-match-confidence,
.lyric-match-tag {
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 11px;
  white-space: nowrap;
}

.lyric-match-confidence {
  &.high { background: rgba(129, 199, 132, 0.2); color: #a5d6a7; }
  &.medium { background: rgba(255, 213, 79, 0.18); color: #ffe082; }
  &.low { background: rgba(255, 138, 101, 0.18); color: #ffab91; }
}

.lyric-match-tag {
  background: rgba(255, 255, 255, 0.08);
  color: rgba(255, 255, 255, 0.55);
}

.lyric-match-preview {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 0 12px 12px 40px;
}

.lyric-match-line {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.lyric-match-line-text { font-size: 13px; color: rgba(255, 255, 255, 0.82); }
.lyric-match-line-sub { font-size: 11.5px; color: rgba(255, 255, 255, 0.45); }
.lyric-match-more { font-size: 12px; color: rgba(255, 255, 255, 0.35); }

.lyric-match-preview-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 4px;
}

.lyric-match-apply {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 8px 16px;
  border: none;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.9);
  color: #1b1b1f;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;

  .material-symbols-rounded { font-size: 18px; }
  &:hover { background: white; }
}
</style>
