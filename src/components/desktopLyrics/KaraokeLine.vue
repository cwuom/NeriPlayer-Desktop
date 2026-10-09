<script setup lang="ts">
// 一行卡拉 OK 歌词：描边垫底层、未唱渐变层、已唱渐变层叠在一起，已唱层按进度裁剪
// 进度每帧由父组件调用 update() 直接写样式，不走响应式，避免每秒 60 次重渲染
import { computed, inject, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { KARAOKE_REGISTRY, KARAOKE_WAKE } from './registry'
import type { DesktopLyricsFrameLine } from '@/modules/desktopLyrics/frame'
import type { DesktopLyricsAlign, DesktopLyricsKaraoke } from '@/modules/desktopLyrics/style'
import { hasWordTiming, highlightWidth } from '@/modules/desktopLyrics/timeline'

const props = defineProps<{
  line: DesktopLyricsFrameLine | null
  /** 没有歌词时显示的普通文字（歌名等），不做卡拉 OK */
  fallback?: string
  endMs: number
  karaoke: DesktopLyricsKaraoke
  align: DesktopLyricsAlign
  /** 尚未开始的下一句：整行用未唱颜色 */
  upcoming?: boolean
}>()

const container = ref<HTMLElement | null>(null)
const fit = ref<HTMLElement | null>(null)
const base = ref<HTMLElement | null>(null)
const fill = ref<HTMLElement | null>(null)
let wordOffsets: Array<readonly [number, number]> | null = null
let textWidth = 0
let lastWidth = -1
let observer: ResizeObserver | null = null

const timedWords = computed(() => !!props.line && props.karaoke === 'word' && hasWordTiming(props.line))

/** 三层用同样的分段，保证字形位置完全一致 */
const segments = computed(() => {
  const line = props.line
  if (!line) return [props.fallback || '\u00a0']
  if (timedWords.value) return line.words.map(word => word.text)
  return [line.text || line.words.map(word => word.text).join('') || '\u00a0']
})

function measure() {
  const element = base.value
  const box = container.value
  const scaler = fit.value
  if (!element || !box || !scaler) return
  textWidth = element.offsetWidth
  const spans = [...element.children] as HTMLElement[]
  wordOffsets = timedWords.value ? spans.map(span => [span.offsetLeft, span.offsetWidth] as const) : null
  const available = box.clientWidth
  // 太长的行整体缩小，最多缩到一半，再长才截掉
  const scale = textWidth > available && textWidth > 0 ? Math.max(0.5, available / textWidth) : 1
  scaler.style.transform = scale < 1 ? `scale(${scale})` : ''
  lastWidth = -1
  requestFrame?.()
}

/** 父组件每帧调用；timeMs 已加歌词偏移 */
function update(timeMs: number) {
  const element = fill.value
  const line = props.line
  if (!element) return
  let width = 0
  if (line && !props.upcoming) {
    width = props.karaoke === 'off'
      ? (timeMs >= line.startMs ? textWidth : 0)
      : highlightWidth(line, props.endMs, timeMs, textWidth, wordOffsets, props.karaoke === 'word')
  }
  const rounded = Math.round(width * 2) / 2
  if (rounded === lastWidth) return
  lastWidth = rounded
  element.style.clipPath = `inset(0 ${Math.max(0, textWidth - rounded)}px 0 0)`
}

const register = inject(KARAOKE_REGISTRY, null)
const requestFrame = inject(KARAOKE_WAKE, null)
let unregister: (() => void) | null = null

watch(() => [props.line, props.karaoke, props.fallback], () => { void nextTick(measure) })

onMounted(() => {
  void nextTick(measure)
  unregister = register?.({ update }) ?? null
  // 字体加载完成或窗口缩放后重新量
  observer = new ResizeObserver(() => measure())
  if (container.value) observer.observe(container.value)
  void document.fonts?.ready.then(() => measure())
})

onBeforeUnmount(() => {
  unregister?.()
  observer?.disconnect()
})
</script>

<template>
  <div ref="container" class="karaoke-line" :class="[`align-${align}`, { upcoming }]">
    <div ref="fit" class="karaoke-fit">
      <span class="karaoke-text karaoke-stroke" aria-hidden="true"><span v-for="(segment, index) in segments" :key="index">{{ segment }}</span></span>
      <span ref="base" class="karaoke-text karaoke-unplayed"><span v-for="(segment, index) in segments" :key="index">{{ segment }}</span></span>
      <span ref="fill" class="karaoke-text karaoke-played" aria-hidden="true"><span v-for="(segment, index) in segments" :key="index">{{ segment }}</span></span>
    </div>
  </div>
</template>

<style scoped>
.karaoke-line {
  display: flex;
  width: 100%;
  min-width: 0;
  overflow: hidden;
  padding: 0.12em 0.2em;
  box-sizing: border-box;
}

.karaoke-line.align-left { justify-content: flex-start; }
.karaoke-line.align-center { justify-content: center; }
.karaoke-line.align-right { justify-content: flex-end; }

.karaoke-fit {
  position: relative;
  flex: 0 0 auto;
  line-height: 1.3;
  transform-origin: center;
}

.align-left .karaoke-fit { transform-origin: left center; }
.align-right .karaoke-fit { transform-origin: right center; }

.karaoke-text {
  display: block;
  white-space: pre;
  font-family: var(--dl-font-family);
  font-size: var(--dl-font-size);
  font-weight: var(--dl-font-weight);
  letter-spacing: var(--dl-letter-spacing);
}

/* 描边画在最底下，被上面的填充盖住内半边，只露出外半边 */
.karaoke-stroke {
  position: absolute;
  inset: 0;
  color: transparent;
  -webkit-text-stroke: var(--dl-stroke);
  filter: var(--dl-shadow);
}

.karaoke-unplayed,
.karaoke-played {
  color: transparent;
  -webkit-background-clip: text;
  background-clip: text;
}

.karaoke-unplayed {
  position: relative;
  background-image: var(--dl-unplayed);
}

.karaoke-played {
  position: absolute;
  inset: 0;
  background-image: var(--dl-played);
  filter: var(--dl-glow);
  clip-path: inset(0 100% 0 0);
  will-change: clip-path;
}

.upcoming .karaoke-played { visibility: hidden; }
</style>
