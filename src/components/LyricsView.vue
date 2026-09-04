<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { DomLyricPlayer, type LyricLineMouseEvent } from '@amll-core/lyric-player/dom/index.ts'
import type {
  LyricLine as AmllLyricLine,
  LyricWord as AmllLyricWord,
} from '@amll-core/interfaces.ts'
import type {
  LyricLine as PlayerLyricLine,
  LyricWord as PlayerLyricWord,
} from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { useI18n } from 'vue-i18n'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import ContextMenu from '@/components/ui/ContextMenu.vue'
import {
  createContextMenuItem,
  type ContextMenuActionItem,
  type ContextMenuItem,
} from '@/utils/contextMenu'

const settings = useSettingsStore()
const { t } = useI18n()

const props = withDefaults(defineProps<{
  lyrics: PlayerLyricLine[]
  currentTimeMs: number
  previewTimeMs?: number | null
  isPlaying: boolean
  lyricOffsetMs?: number
  seekSeq?: number
}>(), {
  currentTimeMs: 0,
  previewTimeMs: null,
  isPlaying: false,
  lyricOffsetMs: undefined,
  seekSeq: 0,
})

const emit = defineEmits<{ seek: [timeMs: number] }>()

const hostRef = ref<HTMLDivElement>()
const isLayoutReady = ref(false)
let lyricPlayer: DomLyricPlayer | null = null
let rafId = 0
let layoutFrameId = 0
let layoutSyncToken = 0
let lastFrameAt = 0
let lastSyncedTime = Number.NaN
let lastFeedAt = 0
let settleTimer: ReturnType<typeof setTimeout> | null = null
let resizeObserver: ResizeObserver | null = null
let lastHostWidth = 0
let lastHostHeight = 0

const SPLIT_WHITESPACE_RE = /(\s+)/
const WHITESPACE_RE = /\s/g
const AMLL_WORD_FADE_WIDTH = 0.5
const LAYOUT_SETTLE_PASSES = 4
const LAYOUT_SETTLE_MAX_PASSES = 8
const SIZE_EPSILON = 0.5
// 时间喂入节流：后端插值时钟在 60/120Hz 显示上每帧变化，逐帧 setCurrentTime
// 会让 AMLL 每帧重算整棵歌词状态树；30ms 粒度下行切换/逐字误差不可感知，
// 弹簧与 WAAPI 词遮罩动画各自独立走时钟，不受喂入频率影响
const TIME_FEED_INTERVAL_MS = 30
// 暂停态下 seek/布局后弹簧需要短暂逐帧驱动收敛，收敛后停表省掉每帧样式写入
const PAUSE_SETTLE_MS = 500

interface PlayerRubyWord {
  startMs: number
  durationMs: number
  text: string
}

type RichPlayerLyricWord = PlayerLyricWord & {
  romanWord?: string
  obscene?: boolean
  ruby?: PlayerRubyWord[]
}

const offsetMs = computed(() => {
  // 必须由上层传入有效偏移; 不再回退到网易云全局默认,
  // 否则 YouTube/B站/本地在漏传时会被错误套上 +1000ms
  if (typeof props.lyricOffsetMs === 'number' && Number.isFinite(props.lyricOffsetMs)) {
    return props.lyricOffsetMs
  }
  return 0
})

const effectiveTimeMs = computed(() => {
  if (props.previewTimeMs != null) return props.previewTimeMs
  return props.currentTimeMs
})

const amllTimeMs = computed(() => Math.max(0, effectiveTimeMs.value + offsetMs.value))

function displayText(line: PlayerLyricLine): string {
  if (line.text) return line.text
  return (line.words || []).map(word => word.text).join('')
}

function hasWordTiming(line: PlayerLyricLine): boolean {
  return (line.words || []).some(word => word.durationMs > 0)
}

function normalizeWordTimings(line: PlayerLyricLine): PlayerLyricWord[] {
  const words = line.words || []
  const timedWords = words.filter(word => word.durationMs > 0)
  if (timedWords.length === 0) return words

  const lineStart = line.startMs
  const firstWordStart = Math.min(...timedWords.map(word => word.startMs))
  // 对齐 Android normalizeSyllableTimes: 仅以首词早于行起点判定相对时间轴, 去掉
  // lastWordEnd<=duration 上限, 否则词轨略超行长会被误判为绝对导致逐字跑偏（LY-5）
  const usesRelativeTime = firstWordStart < lineStart - 250

  if (!usesRelativeTime) return words

  return words.map(word => ({
    ...word,
    startMs: lineStart + word.startMs,
  }))
}

function restoreWhitespaceFromLineText(
  words: PlayerLyricWord[],
  lineText: string,
): PlayerLyricWord[] {
  if (!lineText || !/\s/.test(lineText) || words.some(word => /\s/.test(word.text))) {
    return words
  }

  const compactWords = words.map(word => word.text).join('').replace(/\s+/g, '')
  const compactLine = lineText.replace(/\s+/g, '')
  if (compactWords !== compactLine) return words

  const restored: PlayerLyricWord[] = []
  let cursor = 0

  for (const word of words) {
    const nextIndex = lineText.indexOf(word.text, cursor)
    if (nextIndex < 0) return words

    const between = lineText.slice(cursor, nextIndex)
    if (between) {
      if (between.trim()) return words
      restored.push({
        startMs: word.startMs,
        durationMs: 0,
        text: between,
      })
    }

    restored.push({
      ...word,
      text: lineText.slice(nextIndex, nextIndex + word.text.length),
    })
    cursor = nextIndex + word.text.length
  }

  const tail = lineText.slice(cursor)
  if (tail) {
    if (tail.trim()) return words
    const lastWord = words[words.length - 1]
    restored.push({
      startMs: lastWord ? lastWord.startMs + lastWord.durationMs : 0,
      durationMs: 0,
      text: tail,
    })
  }

  return restored
}

function splitWhitespaceAtoms(words: PlayerLyricWord[]): PlayerLyricWord[] {
  const result: RichPlayerLyricWord[] = []

  for (const word of words) {
    const richWord = word as RichPlayerLyricWord
    if (!word.text || !/\s/.test(word.text) || !word.text.trim() || (richWord.ruby?.length ?? 0) > 0) {
      result.push(word)
      continue
    }

    const parts = word.text.split(SPLIT_WHITESPACE_RE).filter(part => part.length > 0)
    const totalLength = word.text.replace(WHITESPACE_RE, '').length || 1
    const timePerUnit = word.durationMs / totalLength
    let currentOffset = 0

    for (const part of parts) {
      const startMs = word.startMs + currentOffset * timePerUnit
      if (!part.trim()) {
        result.push({
          startMs,
          durationMs: 0,
          text: part,
          obscene: richWord.obscene,
        })
        continue
      }

      const durationMs = part.length * timePerUnit
      result.push({
        startMs,
        durationMs,
        text: part,
        romanWord: richWord.romanWord,
        obscene: richWord.obscene,
      })
      currentOffset += part.length
    }
  }

  return result
}

function toAmllWord(word: PlayerLyricWord): AmllLyricWord {
  const richWord = word as RichPlayerLyricWord
  const startTime = Math.max(0, Math.round(word.startMs))
  const endTime = Math.max(startTime, Math.round(word.startMs + word.durationMs))
  const amllWord: AmllLyricWord = {
    word: word.text,
    startTime,
    endTime,
  }
  if (richWord.romanWord) amllWord.romanWord = richWord.romanWord
  if (richWord.obscene != null) amllWord.obscene = richWord.obscene
  if (richWord.ruby?.length) {
    amllWord.ruby = richWord.ruby.map(ruby => {
      const rubyStartTime = Math.max(0, Math.round(ruby.startMs))
      return {
        word: ruby.text,
        startTime: rubyStartTime,
        endTime: Math.max(rubyStartTime, Math.round(ruby.startMs + ruby.durationMs)),
      }
    })
  }
  return amllWord
}

function buildTimedWords(line: PlayerLyricLine): AmllLyricWord[] {
  if (!hasWordTiming(line)) return []

  const lineText = displayText(line)
  const normalizedWords = normalizeWordTimings(line)
  const restoredWords = restoreWhitespaceFromLineText(normalizedWords, lineText)
  return splitWhitespaceAtoms(restoredWords)
    .filter(word => word.text.length > 0)
    .map(toAmllWord)
}

function toAmllLine(line: PlayerLyricLine): AmllLyricLine {
  const startTime = Math.max(0, Math.round(line.startMs))
  const fallbackEndTime = Math.max(startTime + 1, Math.round(line.startMs + line.durationMs))
  // 关闭高级歌词动画时整行一次性显示：把逐字时间轴折叠成单个词
  const timedWords = settings.advancedLyrics ? buildTimedWords(line) : []
  const words = timedWords.length > 0
    ? timedWords
    : [{
        word: displayText(line),
        startTime,
        endTime: fallbackEndTime,
      }]
  const endTime = Math.max(
    fallbackEndTime,
    ...words.map(word => word.endTime),
    startTime + 1,
  )

  return {
    words,
    translatedLyric: settings.showTranslation ? (line.translation || '') : '',
    romanLyric: settings.showTranslation ? (line.roman || '') : '',
    startTime,
    endTime,
    isBG: false,
    isDuet: false,
  }
}

function buildAmllLines(): AmllLyricLine[] {
  return props.lyrics.map(toAmllLine)
}

function syncCurrentTime(forceSeek = false): void {
  if (!lyricPlayer) return
  const time = Math.max(0, Math.round(amllTimeMs.value))
  const drift = Math.abs(time - lastSyncedTime)
  // 只有真正的跳转（>500ms）才强制 seek。插值时钟被后端位置事件小幅
  // 回拉是常态（缓冲、事件节流都会造成 100ms 级摆动），80ms 就强跳的话
  // 每次回拉歌词都猛抖一下——「一抖一抖」就是它。500ms 以内直接喂时间，
  // AMLL 按连续播放自行平滑，行切换粒度是秒级，不会因此卡错行。
  if (!forceSeek && drift >= 500) forceSeek = true

  if (forceSeek) {
    lastFeedAt = performance.now()
  } else {
    const now = performance.now()
    if (now - lastFeedAt < TIME_FEED_INTERVAL_MS) return
    if (drift < 1) return
    lastFeedAt = now
  }

  lyricPlayer.setCurrentTime(time, forceSeek)
  lastSyncedTime = time
  if (forceSeek) requestSettleLoop()
}

function syncPlayState(): void {
  if (!lyricPlayer) return
  if (props.isPlaying) lyricPlayer.resume()
  else lyricPlayer.pause()
}

function syncLyricOptions(): void {
  if (!lyricPlayer) return
  lyricPlayer.setEnableBlur(settings.lyricBlur)
  lyricPlayer.setBlurAmount(settings.lyricBlurAmount)
  lyricPlayer.setWordFadeWidth(settings.advancedLyrics ? AMLL_WORD_FADE_WIDTH : 0)
}

function reloadLyrics(): void {
  if (!lyricPlayer) return
  isLayoutReady.value = false
  const time = Math.max(0, Math.round(amllTimeMs.value))
  lyricPlayer.setLyricLines(buildAmllLines(), time)
  lyricPlayer.setCurrentTime(time, true)
  lyricPlayer.update(0)
  lastSyncedTime = time
  syncLyricOptions()
  syncPlayState()
  scheduleLayoutSync()
}

/// 右键菜单：主路径走 AMLL 的 line-contextmenu 事件直接拿 lineIndex；
/// 命中行间空隙时兜底用行容器（currentLyricGroups[i].element）的纵坐标反查
/// 最近一行。不再按叶子节点文本反查——逐字歌词下叶子是单字/单词 span，
/// 文本永远不等于整行，旧算法恒失配
const lyricMenu = ref<{ show: boolean; x: number; y: number; index: number }>({
  show: false, x: 0, y: 0, index: -1,
})

function resolveLineIndexAt(clientY: number): number {
  if (!lyricPlayer) return -1
  const groups = lyricPlayer.currentLyricGroups
  const lineCount = Math.min(groups.length, props.lyrics.length)

  let bestIndex = -1
  let bestDistance = Number.POSITIVE_INFINITY
  for (let index = 0; index < lineCount; index++) {
    const element = groups[index]?.element
    // 虚拟化滚动会把视野外的行容器移出 DOM，跳过拿不到几何信息的行
    if (!element || !element.isConnected) continue
    const rect = element.getBoundingClientRect()
    if (rect.height <= 0) continue
    const distance = Math.abs(clientY - (rect.top + rect.height / 2))
    if (distance >= bestDistance) continue
    bestDistance = distance
    bestIndex = index
  }
  return bestIndex
}

function onLyricContextMenu(event: MouseEvent): void {
  const index = resolveLineIndexAt(event.clientY)
  if (index < 0) return
  event.preventDefault()
  lyricMenu.value = { show: true, x: event.clientX, y: event.clientY, index }
}

function onLineContextMenu(event: Event): void {
  const lineEvent = event as LyricLineMouseEvent
  const index = lineEvent.lineIndex
  if (index < 0 || index >= props.lyrics.length) return
  // preventDefault 经 AMLL 映射回原生事件抑制系统菜单；
  // stopPropagation 阻止冒泡到宿主的兜底 contextmenu，避免二次开菜单
  lineEvent.preventDefault()
  lineEvent.stopPropagation()
  lyricMenu.value = { show: true, x: lineEvent.clientX, y: lineEvent.clientY, index }
}

const lyricMenuItems = computed<ContextMenuItem[]>(() => [
  createContextMenuItem(t('lyrics.copy_line'), { id: 'copy-line', icon: 'content_copy' }),
  createContextMenuItem(t('lyrics.copy_all'), { id: 'copy-all', icon: 'copy_all' }),
  createContextMenuItem(t('lyrics.seek_here'), { id: 'seek', icon: 'play_arrow' }),
])

async function handleLyricMenuClick(item: ContextMenuActionItem): Promise<void> {
  const index = lyricMenu.value.index
  lyricMenu.value.show = false
  const line = props.lyrics[index]
  if (!line) return

  if (item.id === 'seek') {
    emit('seek', Math.max(0, Math.round(line.startMs)))
    return
  }
  const text = item.id === 'copy-all'
    ? props.lyrics.map((entry) => displayText(entry)).filter(Boolean).join('\n')
    : displayText(line)
  if (!text) return
  try {
    await writeText(text)
  } catch {
    // 剪贴板不可用时静默失败，复制不是关键路径
  }
}

function onLineClick(event: Event): void {
  const lineEvent = event as LyricLineMouseEvent
  if (!lyricPlayer || lineEvent.lineIndex < 0) return

  const line = lyricPlayer.getLyricLines()[lineEvent.lineIndex]
  if (!line) return

  lineEvent.preventDefault()
  lyricPlayer.resetScroll()
  emit('seek', Math.max(0, Math.round(line.startTime - offsetMs.value)))
}

function startFrameLoop(): void {
  if (rafId) return
  lastFrameAt = performance.now()
  rafId = requestAnimationFrame(function tick(now) {
    const delta = Math.min(64, now - lastFrameAt)
    lastFrameAt = now
    lyricPlayer?.update(delta)
    rafId = requestAnimationFrame(tick)
  })
}

function stopFrameLoop(): void {
  if (!rafId) return
  cancelAnimationFrame(rafId)
  rafId = 0
}

/// 帧循环按需启停：AMLL 的 update() 每帧会给所有已挂载歌词行写
/// transform/opacity/filter 等样式，暂停时歌词动画已冻结，继续逐帧驱动
/// 只会徒增样式重算与重绘（WebKitGTK 尤甚）。暂停态下 seek/布局等
/// 需要弹簧收敛的场景由 requestSettleLoop 短暂续跑。
function syncFrameLoop(): void {
  const shouldRun = props.isPlaying || props.previewTimeMs != null
  if (shouldRun) startFrameLoop()
  else stopFrameLoop()
}

function requestSettleLoop(): void {
  if (props.isPlaying || props.previewTimeMs != null) return
  startFrameLoop()
  if (settleTimer) clearTimeout(settleTimer)
  settleTimer = setTimeout(() => {
    settleTimer = null
    if (!props.isPlaying && props.previewTimeMs == null) stopFrameLoop()
  }, PAUSE_SETTLE_MS)
}

function clearSettleTimer(): void {
  if (settleTimer) {
    clearTimeout(settleTimer)
    settleTimer = null
  }
}

let stopManualScrollWatcher: (() => void) | null = null

/// 暂停态下手动滚动歌词：AMLL 的滚动由弹簧驱动，弹簧只在 update() 里推进，
/// 帧循环停表时滚轮/触摸拖动不会产生视觉位移。滚动事件持续刷新续跑窗口，
/// 停止滚动 500ms 后自动停表
function startManualScrollWatcher(): void {
  if (!hostRef.value || stopManualScrollWatcher) return
  const onScroll = () => requestSettleLoop()
  hostRef.value.addEventListener('wheel', onScroll, { passive: true })
  hostRef.value.addEventListener('touchmove', onScroll, { passive: true })
  stopManualScrollWatcher = () => {
    hostRef.value?.removeEventListener('wheel', onScroll)
    hostRef.value?.removeEventListener('touchmove', onScroll)
    stopManualScrollWatcher = null
  }
}

function cancelLayoutSync(): void {
  if (layoutFrameId) cancelAnimationFrame(layoutFrameId)
  layoutFrameId = 0
  layoutSyncToken += 1
}

function syncPlayerElementSize(): boolean {
  if (!lyricPlayer) return false
  const playerElement = lyricPlayer.getElement()
  const width = playerElement.clientWidth || hostRef.value?.clientWidth || 0
  const height = playerElement.clientHeight || hostRef.value?.clientHeight || 0

  if (width > 0) lyricPlayer.size[0] = width
  if (height > 0) lyricPlayer.size[1] = height

  return width > 0 && height > 0
}

function syncMountedGroupSizes(): boolean {
  if (!lyricPlayer) return false
  let changed = false

  for (const group of lyricPlayer.currentLyricGroups) {
    const element = group.element
    if (!element.parentElement) continue

    const width = element.clientWidth
    const height = element.clientHeight
    if (width <= 0 || height <= 0) continue

    const previous = lyricPlayer.lyricGroupSize.get(group)
    if (
      !previous ||
      Math.abs(previous[0] - width) > SIZE_EPSILON ||
      Math.abs(previous[1] - height) > SIZE_EPSILON
    ) {
      const nextSize: [number, number] = [width, height]
      lyricPlayer.lyricGroupSize.set(group, nextSize)
      group.onLineSizeChange(nextSize)
      changed = true
    }
  }

  return changed
}

function forceLayoutAtCurrentTime(): boolean {
  if (!lyricPlayer) return false
  const hasPlayerSize = syncPlayerElementSize()
  const time = Math.max(0, Math.round(amllTimeMs.value))

  lyricPlayer.setCurrentTime(time, true)
  const changedBeforeLayout = syncMountedGroupSizes()
  void lyricPlayer.calcLayout(true, true)
  lyricPlayer.update(0)
  const changedAfterLayout = syncMountedGroupSizes()

  if (changedBeforeLayout || changedAfterLayout) {
    void lyricPlayer.calcLayout(true, true)
    lyricPlayer.update(0)
  }

  lastSyncedTime = time
  requestSettleLoop()
  return hasPlayerSize
}

function finishLayoutSync(token: number): void {
  if (!lyricPlayer || token !== layoutSyncToken) return
  const settledTime = Math.max(0, Math.round(amllTimeMs.value))

  lyricPlayer.setCurrentTime(settledTime, true)
  forceLayoutAtCurrentTime()
  lyricPlayer.setCurrentTime(settledTime, false)
  syncPlayState()
  lyricPlayer.update(0)
  lastSyncedTime = settledTime
  isLayoutReady.value = true
}

function scheduleFontReadyLayout(token: number): void {
  const fonts = document.fonts
  if (!fonts || fonts.status === 'loaded') return

  void fonts.ready.then(() => {
    if (!lyricPlayer || token !== layoutSyncToken) return
    scheduleLayoutSync()
  })
}

function scheduleLayoutSync(): void {
  if (!lyricPlayer) return
  if (layoutFrameId) cancelAnimationFrame(layoutFrameId)

  const token = layoutSyncToken + 1
  layoutSyncToken = token
  isLayoutReady.value = false
  let pass = 0

  const runPass = () => {
    layoutFrameId = requestAnimationFrame(() => {
      layoutFrameId = 0
      if (!lyricPlayer || token !== layoutSyncToken) return

      pass += 1
      const hasPlayerSize = forceLayoutAtCurrentTime()
      const needsMorePasses =
        pass < LAYOUT_SETTLE_PASSES ||
        (!hasPlayerSize && pass < LAYOUT_SETTLE_MAX_PASSES)

      if (needsMorePasses) {
        runPass()
        return
      }

      finishLayoutSync(token)
    })
  }

  runPass()
  scheduleFontReadyLayout(token)
}

function startResizeObserver(): void {
  if (!hostRef.value || typeof ResizeObserver === 'undefined') return

  resizeObserver = new ResizeObserver(entries => {
    const entry = entries[0]
    if (!entry) return

    const width = entry.contentRect.width
    const height = entry.contentRect.height
    if (width <= 0 || height <= 0) return

    const hasSizeChanged =
      Math.abs(width - lastHostWidth) > SIZE_EPSILON ||
      Math.abs(height - lastHostHeight) > SIZE_EPSILON
    lastHostWidth = width
    lastHostHeight = height

    if (hasSizeChanged) scheduleLayoutSync()
  })
  resizeObserver.observe(hostRef.value)
}

function stopResizeObserver(): void {
  resizeObserver?.disconnect()
  resizeObserver = null
}

onMounted(() => {
  nextTick(() => {
    if (!hostRef.value || lyricPlayer) return

    lyricPlayer = new DomLyricPlayer()
    lyricPlayer.addEventListener('line-click', onLineClick as EventListener)
    lyricPlayer.addEventListener('line-contextmenu', onLineContextMenu as EventListener)
    hostRef.value.appendChild(lyricPlayer.getElement())

    startResizeObserver()
    reloadLyrics()
    startManualScrollWatcher()
    syncFrameLoop()
  })
})

onUnmounted(() => {
  clearSettleTimer()
  stopManualScrollWatcher?.()
  stopFrameLoop()
  cancelLayoutSync()
  stopResizeObserver()
  if (!lyricPlayer) return

  lyricPlayer.removeEventListener('line-click', onLineClick as EventListener)
  lyricPlayer.removeEventListener('line-contextmenu', onLineContextMenu as EventListener)
  lyricPlayer.dispose()
  lyricPlayer = null
})

watch(() => props.lyrics, () => {
  reloadLyrics()
}, { deep: false })

watch(() => settings.advancedLyrics, () => {
  reloadLyrics()
})

watch(() => settings.showTranslation, () => {
  reloadLyrics()
})

watch([() => settings.lyricBlur, () => settings.lyricBlurAmount], () => {
  syncLyricOptions()
})

watch(() => settings.lyricFontScale, () => {
  scheduleLayoutSync()
})

watch(() => props.isPlaying, () => {
  syncPlayState()
  clearSettleTimer()
  syncFrameLoop()
})

watch(() => props.previewTimeMs, () => {
  if (props.isPlaying) syncFrameLoop()
  else requestSettleLoop()
})

watch(amllTimeMs, (time, oldTime) => {
  const isPreviewing = props.previewTimeMs != null
  const isLargeJump = oldTime !== undefined && Math.abs(time - oldTime) > 1000
  const forceSeek = isPreviewing || isLargeJump
  syncCurrentTime(forceSeek)
})

watch(() => props.seekSeq, (seq, oldSeq) => {
  if (seq === oldSeq) return
  syncCurrentTime(true)
})
</script>

<template>
  <div
    ref="hostRef"
    class="lyrics-scroll"
    :class="{
      'lyrics-scroll--ready': isLayoutReady,
    }"
    :style="{ '--lyric-font-scale': settings.lyricFontScale }"
    @contextmenu="onLyricContextMenu"
  />
  <ContextMenu
    :open="lyricMenu.show"
    :x="lyricMenu.x"
    :y="lyricMenu.y"
    :items="lyricMenuItems"
    @update:open="lyricMenu.show = $event"
    @click="handleLyricMenuClick"
  />
</template>

<style scoped lang="scss">
.lyrics-scroll {
  width: 100%;
  height: 100%;
  overflow: hidden;
  position: relative;
  text-align: left;
  color: white;
  /* 底部提前淡出，避免歌词贴底难读 */
  mask-image: linear-gradient(
    to bottom,
    transparent 0%,
    black 10%,
    black 72%,
    transparent 94%
  );
  -webkit-mask-image: linear-gradient(
    to bottom,
    transparent 0%,
    black 10%,
    black 72%,
    transparent 94%
  );
  --amll-lp-color: white;
  --amll-lp-font-size: calc(max(max(5vh, 2.5vw), 12px) * var(--lyric-font-scale, 1));
  --amll-lp-emphasis-glow-opacity-boost: 5.6;
  --amll-lp-emphasis-glow-min-opacity: 0.58;
  --amll-lp-emphasis-glow-max-opacity: 0.96;
  --amll-lp-emphasis-glow-radius-boost: 3;
  --amll-lp-emphasis-glow-min-radius: 0.2;
  --amll-lp-emphasis-glow-max-radius: 0.7;
}

:deep(.amll-lyric-player [class*="interludeDots"]) {
  z-index: 2;
}

.lyrics-scroll:not(.lyrics-scroll--ready) :deep(.amll-lyric-player) {
  visibility: hidden;
  opacity: 0 !important;
  pointer-events: none;
}

:deep(.amll-lyric-player) {
  text-align: left;
  font-weight: 850;
  font-variation-settings: 'wght' 850;
  -webkit-font-smoothing: antialiased;
  --amll-lp-line-width-aspect: 0.82;
}

:deep(.amll-lyric-player [class*="lyricMainLine"]) {
  font-weight: 850;
  font-variation-settings: 'wght' 850;
  letter-spacing: -0.025em;
}

:deep(.amll-lyric-player [class*="lyricSubLine"]) {
  font-weight: 650;
  font-variation-settings: 'wght' 650;
}

</style>
