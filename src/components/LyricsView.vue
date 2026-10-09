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
let resizeObserver: ResizeObserver | null = null
let lastHostWidth = 0
let lastHostHeight = 0
let idleDeadline = 0
let wakeTarget: HTMLElement | null = null
let loadedTimelineKey: string | null = null

const SPLIT_WHITESPACE_RE = /(\s+)/
const WHITESPACE_RE = /\s/g
const AMLL_WORD_FADE_WIDTH = 0.5
const LAYOUT_SETTLE_PASSES = 4
const LAYOUT_SETTLE_MAX_PASSES = 8
const SIZE_EPSILON = 0.5
// 覆盖 AMLL 弹簧（posY/scale）从任意位移收敛所需的时长
const IDLE_FRAME_GRACE_MS = 2500
const WAKE_EVENTS = ['wheel', 'pointerdown', 'touchstart', 'keydown'] as const
// 播放中后端插值时钟每帧都变，逐帧 setCurrentTime 会让 AMLL 每帧重算整棵歌词状态树；
// 30ms 粒度下行切换/逐字误差不可感知，弹簧与词遮罩动画各自走时钟，不受喂入频率影响
const TIME_FEED_INTERVAL_MS = 30

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
  if (richWord.romanWord && settings.showRomanization) amllWord.romanWord = richWord.romanWord
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
    romanLyric: settings.showRomanization ? (line.roman || '') : '',
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
  if (!forceSeek && drift < 1) return
  // 只有真正的跳转（>500ms）才强制 seek。插值时钟被后端位置事件小幅
  // 回拉是常态（缓冲、事件节流都会造成 100ms 级摆动），80ms 就强跳的话
  // 每次回拉歌词都猛抖一下——「一抖一抖」就是它。500ms 以内直接喂时间，
  // AMLL 按连续播放自行平滑，行切换粒度是秒级，不会因此卡错行。
  if (!forceSeek && drift >= 500) forceSeek = true

  const now = performance.now()
  if (!forceSeek && props.isPlaying && now - lastFeedAt < TIME_FEED_INTERVAL_MS) return
  lastFeedAt = now

  lyricPlayer.setCurrentTime(time, forceSeek)
  lastSyncedTime = time
  if (!props.isPlaying) wakeFrameLoop()
}

function syncPlayState(): void {
  if (!lyricPlayer) return
  if (props.isPlaying) lyricPlayer.resume()
  else lyricPlayer.pause()
  wakeFrameLoop()
}

function syncLyricOptions(): void {
  if (!lyricPlayer) return
  lyricPlayer.setEnableBlur(settings.lyricBlur)
  lyricPlayer.setBlurAmount(settings.lyricBlurAmount)
  lyricPlayer.setWordFadeWidth(settings.advancedLyrics ? AMLL_WORD_FADE_WIDTH : 0)
  wakeFrameLoop()
}

function lyricTimelineKey(lines: PlayerLyricLine[]): string {
  return lines.map(line => line.startMs).join(',')
}

function reloadLyrics(): void {
  if (!lyricPlayer) return
  // 换了一首（时间轴不同）才先隐藏等排版落定；同一份时间轴补上音译、翻译、逐字，
  // 或切换显示选项时原地重排，不让整屏歌词闪一下
  const timelineKey = lyricTimelineKey(props.lyrics)
  const isNewTimeline = timelineKey !== loadedTimelineKey
  loadedTimelineKey = timelineKey
  const time = Math.max(0, Math.round(amllTimeMs.value))
  lyricPlayer.setLyricLines(buildAmllLines(), time)
  lyricPlayer.setCurrentTime(time, true)
  lyricPlayer.update(0)
  lastSyncedTime = time
  syncLyricOptions()
  syncPlayState()
  // 新建的行起始在屏幕外，原地重排要在这一帧就摆好位置
  if (!isNewTimeline && isLayoutReady.value) forceLayoutAtCurrentTime()
  scheduleLayoutSync(isNewTimeline || !isLayoutReady.value)
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

/// 暂停时没有逐字/间奏动画需要推进，只剩弹簧收尾；收尾后停表，
/// 避免暂停挂机时每帧重写所有可见行的样式。布局、滚动、交互会再唤醒
function wakeFrameLoop(): void {
  idleDeadline = performance.now() + IDLE_FRAME_GRACE_MS
  startFrameLoop()
}

function startFrameLoop(): void {
  if (rafId) return
  lastFrameAt = performance.now()
  rafId = requestAnimationFrame(function tick(now) {
    const delta = Math.min(64, now - lastFrameAt)
    lastFrameAt = now
    lyricPlayer?.update(delta)
    if (!props.isPlaying && now >= idleDeadline) {
      rafId = 0
      return
    }
    rafId = requestAnimationFrame(tick)
  })
}

function stopFrameLoop(): void {
  if (!rafId) return
  cancelAnimationFrame(rafId)
  rafId = 0
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

/// hide：新歌词首次排版时先隐藏，避免行从屏幕外飞入；缩放、字号等重排保持可见
function scheduleLayoutSync(hide = false): void {
  if (!lyricPlayer) return
  if (layoutFrameId) cancelAnimationFrame(layoutFrameId)

  const token = layoutSyncToken + 1
  layoutSyncToken = token
  if (hide) isLayoutReady.value = false
  wakeFrameLoop()
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
    // AMLL 默认把每行开始时间最多提前 600ms，行会在唱到之前就高亮滚动；
    // Android 按原始时间轴切行，偏移量也是按原始时间轴标定的
    lyricPlayer.setOptimizeOptions({ tryAdvanceStartTime: false })
    lyricPlayer.addEventListener('line-click', onLineClick as EventListener)
    lyricPlayer.addEventListener('line-contextmenu', onLineContextMenu as EventListener)
    hostRef.value.appendChild(lyricPlayer.getElement())
    wakeTarget = hostRef.value
    for (const type of WAKE_EVENTS) {
      wakeTarget.addEventListener(type, wakeFrameLoop, { passive: true })
    }

    startResizeObserver()
    reloadLyrics()
  })
})

onUnmounted(() => {
  stopFrameLoop()
  cancelLayoutSync()
  stopResizeObserver()
  for (const type of WAKE_EVENTS) {
    wakeTarget?.removeEventListener(type, wakeFrameLoop)
  }
  wakeTarget = null
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

watch([() => settings.showTranslation, () => settings.showRomanization], () => {
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
