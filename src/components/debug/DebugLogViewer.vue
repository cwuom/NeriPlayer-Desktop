<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { useToastStore } from '@/stores/toast'
import { createLogger } from '@/utils/logger'
import CustomSelect from '@/components/ui/CustomSelect.vue'

/// 运行日志（对齐 Android LogViewer）：后端环形缓冲 + 前端日志，等宽字体按时间顺序显示，
/// 支持级别 / 来源筛选、全文搜索、自动跟随最新、展开单行与复制
interface RecentLogEntry { timestamp_ms: number; level: string; target: string; message: string }
type Level = 'ERROR' | 'WARN' | 'INFO' | 'DEBUG' | 'TRACE'

const LEVELS: Level[] = ['ERROR', 'WARN', 'INFO', 'DEBUG']
const REFRESH_MS = 1500
const FETCH_LIMIT = 800

const { t } = useI18n()
const toast = useToastStore()
const log = createLogger('debug-logs')

const entries = ref<RecentLogEntry[]>([])
const paused = ref(false)
const follow = ref(true)
const query = ref('')
const levelFilter = ref<Set<Level>>(new Set(LEVELS))
const targetFilter = ref('')
const expanded = ref<string | null>(null)
/** 「清空视图」只隐藏这个时间点之前的日志，不动后端缓冲和日志文件 */
const clearedBefore = ref(0)
const listEl = ref<HTMLElement | null>(null)
let timer: number | null = null
let refreshing = false
let disposed = false

/// 行 key 不能用列表下标：环形缓冲满了以后旧日志从头部出队，下标整体前移会让每一行都重建
const entryKeys = computed(() => {
  const seen = new Map<string, number>()
  const keys = new WeakMap<RecentLogEntry, string>()
  for (const entry of entries.value) {
    const identity = JSON.stringify([entry.timestamp_ms, entry.level, entry.target, entry.message])
    const occurrence = seen.get(identity) ?? 0
    seen.set(identity, occurrence + 1)
    keys.set(entry, `${identity}-${occurrence}`)
  }
  return keys
})

function entryKey(entry: RecentLogEntry): string {
  return entryKeys.value.get(entry) ?? String(entry.timestamp_ms)
}

function sameEntries(next: RecentLogEntry[]): boolean {
  const current = entries.value
  if (current.length !== next.length) return false
  return current.every((entry, index) => {
    const other = next[index]!
    return entry.timestamp_ms === other.timestamp_ms
      && entry.level === other.level
      && entry.target === other.target
      && entry.message === other.message
  })
}

/// 前端日志的 target 带完整源码地址（webview:anonymous@http://…/logger.ts:53:55），只留有意义的部分
function shortTarget(target: string): string {
  const webview = target.match(/^webview:(.*?)@.*?([\w-]+)\.(?:ts|js|vue)(?::\d+){0,2}$/)
  if (webview) return `web:${webview[2]}`
  return target.replace(/^neri_player_desktop::/, '').replace(/^webview:/, 'web:')
}

async function refresh() {
  if (disposed || refreshing) return
  refreshing = true
  try {
    const result = await invoke<RecentLogEntry[]>('get_recent_logs', { limit: FETCH_LIMIT, minLevel: null })
    if (disposed) return
    // 后端最新在前，展示按时间正序，最新在底部
    const next = [...result].reverse()
    if (!sameEntries(next)) entries.value = next
  } catch {
    // 浏览器开发模式没有这个命令
  } finally {
    refreshing = false
  }
}

const visible = computed(() => {
  const keyword = query.value.trim().toLowerCase()
  return entries.value.filter(entry =>
    entry.timestamp_ms >= clearedBefore.value
    && levelFilter.value.has(normalizeLevel(entry.level))
    && (!targetFilter.value || shortTarget(entry.target) === targetFilter.value)
    && (!keyword || `${entry.message} ${entry.target}`.toLowerCase().includes(keyword)),
  )
})

const levelCounts = computed(() => {
  const counts: Record<Level, number> = { ERROR: 0, WARN: 0, INFO: 0, DEBUG: 0, TRACE: 0 }
  for (const entry of entries.value) {
    if (entry.timestamp_ms >= clearedBefore.value) counts[normalizeLevel(entry.level)]++
  }
  return counts
})

const targets = computed(() => [...new Set(entries.value.map(entry => shortTarget(entry.target)))].sort())
const targetOptions = computed(() => [
  { value: '', label: t('settings.debug_logs_all_targets') },
  ...targets.value.map(target => ({ value: target, label: target })),
])

function normalizeLevel(level: string): Level {
  const upper = level.toUpperCase()
  if (upper === 'TRACE') return 'DEBUG'
  return (LEVELS as string[]).includes(upper) ? upper as Level : 'INFO'
}

function toggleLevel(level: Level) {
  const next = new Set(levelFilter.value)
  if (next.has(level) && next.size > 1) next.delete(level)
  else next.add(level)
  levelFilter.value = next
}

function soloLevel(level: Level) {
  levelFilter.value = levelFilter.value.size === 1 && levelFilter.value.has(level) ? new Set(LEVELS) : new Set([level])
}

function logTime(ms: number): string {
  const date = new Date(ms)
  const pad = (value: number, length = 2) => String(value).padStart(length, '0')
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}.${pad(date.getMilliseconds(), 3)}`
}

function formatLine(entry: RecentLogEntry): string {
  return `${logTime(entry.timestamp_ms)} [${entry.level}] [${entry.target}] ${entry.message}`
}

async function copyText(text: string) {
  try {
    await writeText(text)
    toast.success(t('settings.debug_logs_copied'))
  } catch (error) {
    log.error('copy logs failed:', error)
  }
}

async function exportReport() {
  try {
    const path = await invoke<string>('export_debug_report')
    toast.success(t('settings.debug_logs_exported'))
    await invoke('reveal_in_file_manager', { path })
  } catch (error) {
    log.error('export report failed:', error)
  }
}

async function openLogDir() {
  try {
    const dir = await invoke<string>('get_log_dir')
    await invoke('reveal_in_file_manager', { path: dir })
  } catch (error) {
    log.error('open log dir failed:', error)
  }
}

function clearView() {
  clearedBefore.value = Date.now()
  expanded.value = null
}

function onScroll() {
  const el = listEl.value
  if (!el) return
  // 用户往上翻就停止跟随，回到底部自动恢复
  follow.value = el.scrollHeight - el.scrollTop - el.clientHeight < 24
}

function scrollToBottom() {
  const el = listEl.value
  if (el) el.scrollTop = el.scrollHeight
}

watch(() => visible.value.length, () => {
  if (follow.value) void nextTick(scrollToBottom)
})

onMounted(() => {
  void refresh().then(() => { if (!disposed) void nextTick(scrollToBottom) })
  timer = window.setInterval(() => {
    if (!paused.value && document.visibilityState === 'visible') void refresh()
  }, REFRESH_MS)
})

onUnmounted(() => {
  disposed = true
  if (timer) window.clearInterval(timer)
})

const levelClass = (level: string) => `level-${normalizeLevel(level).toLowerCase()}`
</script>

<template>
  <div class="log-viewer">
    <div class="log-toolbar">
      <div class="log-levels" role="group" :aria-label="t('settings.debug_logs_level')">
        <button
          v-for="level in LEVELS"
          :key="level"
          type="button"
          class="log-level-chip"
          :class="[levelClass(level), { active: levelFilter.has(level) }]"
          :title="t('settings.debug_logs_level_hint')"
          @click="toggleLevel(level)"
          @dblclick="soloLevel(level)"
        >
          {{ level }}
          <span class="log-level-count">{{ levelCounts[level] }}</span>
        </button>
      </div>
      <CustomSelect v-model="targetFilter" class="log-target-select" :options="targetOptions" :label="t('settings.debug_logs_target')" />
      <label class="log-search">
        <span class="material-symbols-rounded" aria-hidden="true">search</span>
        <input v-model="query" type="search" :placeholder="t('settings.debug_logs_search')" />
      </label>
    </div>

    <div class="log-actions">
      <button type="button" class="log-btn" :class="{ active: paused }" @click="paused = !paused">
        <span class="material-symbols-rounded">{{ paused ? 'play_arrow' : 'pause' }}</span>
        {{ paused ? t('settings.debug_logs_resume') : t('settings.debug_logs_pause') }}
      </button>
      <button type="button" class="log-btn" :class="{ active: follow }" @click="follow = !follow; follow && scrollToBottom()">
        <span class="material-symbols-rounded">vertical_align_bottom</span>
        {{ t('settings.debug_logs_follow') }}
      </button>
      <button type="button" class="log-btn" @click="clearView">
        <span class="material-symbols-rounded">clear_all</span>
        {{ t('settings.debug_logs_clear_view') }}
      </button>
      <span class="log-spacer" />
      <button type="button" class="log-btn" @click="copyText(visible.map(formatLine).join('\n'))">
        <span class="material-symbols-rounded">content_copy</span>
        {{ t('settings.debug_logs_copy') }}
      </button>
      <button type="button" class="log-btn" @click="exportReport">
        <span class="material-symbols-rounded">ios_share</span>
        {{ t('settings.debug_logs_export') }}
      </button>
      <button type="button" class="log-btn" @click="openLogDir">
        <span class="material-symbols-rounded">folder_open</span>
        {{ t('settings.debug_logs_open_dir') }}
      </button>
    </div>

    <div ref="listEl" class="log-list" @scroll.passive="onScroll">
      <p v-if="!visible.length" class="log-empty">
        {{ t(entries.length ? 'settings.debug_logs_no_match' : 'settings.debug_logs_empty') }}
      </p>
      <div
        v-for="entry in visible"
        :key="entryKey(entry)"
        class="log-line"
        :class="[levelClass(entry.level), { expanded: expanded === entryKey(entry) }]"
        @click="expanded = expanded === entryKey(entry) ? null : entryKey(entry)"
      >
        <span class="log-time">{{ logTime(entry.timestamp_ms) }}</span>
        <span class="log-level">{{ normalizeLevel(entry.level) }}</span>
        <span class="log-target" :title="entry.target">{{ shortTarget(entry.target) }}</span>
        <span class="log-message">{{ entry.message }}</span>
        <div v-if="expanded === entryKey(entry)" class="log-detail" @click.stop>
          <span class="log-detail-target">{{ entry.target }}</span>
          <button type="button" class="log-btn small" @click="copyText(formatLine(entry))">
            <span class="material-symbols-rounded">content_copy</span>
            {{ t('settings.debug_logs_copy_line') }}
          </button>
        </div>
      </div>
    </div>
    <div class="log-footer">
      <span>{{ t('settings.debug_logs_count', { shown: visible.length, total: entries.length }) }}</span>
      <span v-if="paused" class="log-paused">{{ t('settings.debug_logs_paused') }}</span>
    </div>
  </div>
</template>

<style scoped lang="scss">
.log-viewer {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 10px;
  padding: 14px 16px;
  min-height: 0;
  margin-bottom: 8px;
  border-radius: var(--radius-lg);
  background: var(--md-surface-container);
}

.log-toolbar,
.log-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.log-levels { display: flex; gap: 6px; }

.log-level-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px;
  border-radius: var(--radius-full);
  border: 1px solid var(--hairline);
  font-family: var(--font-mono);
  font-size: var(--fs-11);
  font-weight: 600;
  color: var(--md-on-surface-variant);
  opacity: 0.55;
  transition: opacity 140ms var(--ease-standard), background 140ms var(--ease-standard), border-color 140ms var(--ease-standard);

  &.active { opacity: 1; border-color: currentColor; background: color-mix(in srgb, currentColor 12%, transparent); }
  &.level-error { color: var(--md-error); }
  &.level-warn { color: #f59e0b; }
  &.level-info { color: var(--md-primary); }
  &.level-debug { color: var(--md-on-surface-variant); }
}

.log-level-count {
  min-width: 18px;
  padding: 0 5px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, currentColor 18%, transparent);
  text-align: center;
  font-variant-numeric: tabular-nums;
}

.log-target-select {
  max-width: 200px;
  font-family: var(--font-mono);

  :deep(.custom-select-trigger) {
    height: 30px;
    padding: 0 8px;
    border-radius: var(--radius-sm);
    font-size: var(--fs-12);
  }
}

.log-search {
  flex: 1;
  min-width: 160px;
  display: flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--hairline);
  background: var(--md-surface-container-high);
  color: var(--md-on-surface-variant);
  .material-symbols-rounded { font-size: 18px; }
  &:focus-within { border-color: var(--md-primary); }
  input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--md-on-surface);
    font: inherit;
    font-size: var(--fs-12);
  }
}

.log-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 30px;
  padding: 0 10px;
  border-radius: var(--radius-full);
  font-size: var(--fs-12);
  color: var(--md-on-surface-variant);
  background: var(--md-surface-container-high);
  transition: background 140ms var(--ease-standard), color 140ms var(--ease-standard);
  .material-symbols-rounded { font-size: 16px; }
  &:hover { color: var(--md-on-surface); background: var(--md-surface-container-highest); }
  &.active { color: var(--md-on-primary-container); background: var(--md-primary-container); }
  &.small { height: 26px; padding: 0 8px; }
}

.log-spacer { flex: 1; }

.log-list {
  height: clamp(320px, calc(100vh - 360px), 900px);
  overflow: auto;
  overscroll-behavior: contain;
  padding: 6px 0;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--md-surface-container-lowest) 80%, transparent);
  font-family: var(--font-mono);
  font-size: var(--fs-12);
  line-height: 1.55;
  user-select: text;
}

.log-empty {
  padding: 40px 12px;
  text-align: center;
  font-family: var(--font-family);
  color: var(--md-on-surface-variant);
}

/* 时间 / 级别 / 来源固定列宽，消息独占剩余宽度：长来源再也挤不扁消息列 */
.log-line {
  display: grid;
  grid-template-columns: 92px 46px minmax(0, 150px) minmax(0, 1fr);
  column-gap: 10px;
  padding: 2px 12px;
  border-left: 3px solid transparent;
  cursor: pointer;

  &:hover { background: color-mix(in srgb, var(--md-on-surface) 5%, transparent); }
  &.expanded { background: color-mix(in srgb, var(--md-primary) 8%, transparent); }
  &.level-error { border-left-color: var(--md-error); .log-level { color: var(--md-error); } }
  &.level-warn { border-left-color: #f59e0b; .log-level { color: #f59e0b; } }
  &.level-info .log-level { color: var(--md-primary); }
  &.level-debug { color: var(--md-on-surface-variant); }
}

.log-time { color: var(--md-outline); font-variant-numeric: tabular-nums; }
.log-level { font-weight: 700; }

.log-target {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--md-on-surface-variant);
}

.log-message {
  min-width: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.log-line:not(.expanded) .log-message {
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.log-detail {
  grid-column: 1 / -1;
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 4px 0 2px;
  cursor: default;
}

.log-detail-target {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
  color: var(--md-outline);
}

.log-footer {
  display: flex;
  justify-content: space-between;
  font-size: var(--fs-11);
  color: var(--md-on-surface-variant);
}

.log-paused { color: #f59e0b; font-weight: 600; }
</style>
