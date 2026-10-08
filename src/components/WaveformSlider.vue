<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'

const props = withDefaults(defineProps<{
  progress: number
  isPlaying: boolean
  activeColor?: string
  inactiveColor?: string
}>(), {
  activeColor: '#fff',
  inactiveColor: 'rgba(255,255,255,0.35)',
})

const emit = defineEmits<{
  seek: [progress: number]
  preview: [progress: number]
  'preview-end': []
}>()

const containerRef = ref<HTMLDivElement>()
const svgRef = ref<SVGSVGElement>()
const isDragging = ref(false)
const dragProgress = ref(0)

const WAVE_AMPLITUDE = 2
// 固定 500 坐标的 SVG 约显示 12 个波，与 Android 播放页的可见密度一致
const WAVE_FREQ = 0.15
const PHASE_CYCLE = 2000
const AMP_TRANSITION = 500

let phase = 0
let currentAmp = 0
let animFrame = 0
let lastTime = 0
let looping = false

let pathActive: SVGPathElement | null = null
let pathInactive: SVGPathElement | null = null
let thumbDiv: HTMLDivElement | null = null

const currentProgress = computed(() => isDragging.value ? dragProgress.value : props.progress)

function wavePath(startX: number, endX: number, cy: number): string {
  if (startX >= endX) return `M ${startX} ${cy}`
  let d = `M ${startX} ${cy + Math.sin(startX * WAVE_FREQ + phase) * currentAmp}`
  for (let x = startX + 2; x <= endX; x += 2) {
    d += ` L ${x} ${cy + Math.sin(x * WAVE_FREQ + phase) * currentAmp}`
  }
  return d
}

function animate(timestamp: number) {
  if (!lastTime) lastTime = timestamp
  const dt = timestamp - lastTime
  lastTime = timestamp

  phase += (dt / PHASE_CYCLE) * Math.PI * 2

  const targetAmp = (props.isPlaying && !isDragging.value) ? WAVE_AMPLITUDE : 0
  const ampStep = (WAVE_AMPLITUDE / AMP_TRANSITION) * dt
  if (currentAmp < targetAmp) {
    currentAmp = Math.min(targetAmp, currentAmp + ampStep)
  } else if (currentAmp > targetAmp) {
    currentAmp = Math.max(targetAmp, currentAmp - ampStep)
  }

  const p = currentProgress.value
  const px = p * 500
  if (pathInactive) pathInactive.setAttribute('d', wavePath(px, 500, 4))
  if (pathActive) pathActive.setAttribute('d', wavePath(0, px, 4))

  // thumb 用 HTML div，按百分比定位（不受 SVG preserveAspectRatio=none 影响）
  if (thumbDiv) {
    const waveY = Math.sin(px * WAVE_FREQ + phase) * currentAmp
    // waveY 范围 [-2,2]，映射到容器高度百分比
    const yPercent = 50 + (waveY / 4) * 50
    thumbDiv.style.left = `${p * 100}%`
    thumbDiv.style.top = `${yPercent}%`
    thumbDiv.style.width = thumbDiv.style.height = isDragging.value ? '12px' : '8px'
  }

  // 空闲停表：波幅已归零且目标为 0（暂停/静止）时不再逐帧重建两条 SVG path，
  // 省 CPU/电；进度或播放态变化时由 watch 重启（MK-03）
  if (currentAmp === 0 && targetAmp === 0) {
    looping = false
    lastTime = 0
    return
  }
  animFrame = requestAnimationFrame(animate)
}

function startLoop() {
  if (looping) return
  looping = true
  lastTime = 0
  animFrame = requestAnimationFrame(animate)
}

onMounted(() => {
  const svg = svgRef.value!
  pathInactive = svg.querySelector('.wave-inactive')
  pathActive = svg.querySelector('.wave-active')
  thumbDiv = containerRef.value!.querySelector('.thumb') as HTMLDivElement
  startLoop()
})

// 播放态/拖拽/进度变化时重启渲染循环（停表后需要更新一帧位置或恢复波动）
watch(() => props.isPlaying, startLoop)
watch(isDragging, startLoop)
watch(currentProgress, startLoop)
onUnmounted(() => {
  cancelAnimationFrame(animFrame)
  // 拖拽中被重建（切歌时按 key 重建）时兜底复位；pointer capture 随元素销毁自动释放
  isDragging.value = false
})

function updateDragProgress(e: PointerEvent): boolean {
  const el = containerRef.value
  if (!el) return false
  const rect = el.getBoundingClientRect()
  dragProgress.value = clamp01((e.clientX - rect.left) / rect.width)
  return true
}

// setPointerCapture 模式（与 MiniPlayer 进度条一致）：
// 指针离开组件后事件仍派发到本元素，无需在 document 上挂全局监听
function handlePointerDown(e: PointerEvent) {
  e.preventDefault()
  e.stopPropagation()
  if (!updateDragProgress(e)) return
  isDragging.value = true
  containerRef.value?.setPointerCapture(e.pointerId)
  emit('preview', dragProgress.value)
}

function handlePointerMove(e: PointerEvent) {
  if (!isDragging.value) return
  if (!updateDragProgress(e)) return
  emit('preview', dragProgress.value)
}

function handlePointerUp(e: PointerEvent) {
  if (!isDragging.value) return
  updateDragProgress(e)
  const p = dragProgress.value
  isDragging.value = false
  containerRef.value?.releasePointerCapture(e.pointerId)
  emit('seek', p)
  emit('preview-end')
}

function clamp01(v: number) { return Math.max(0, Math.min(1, v)) }
</script>

<template>
  <div
    ref="containerRef"
    class="waveform-container"
    :style="{ '--wave-active-color': activeColor, '--wave-inactive-color': inactiveColor }"
    @pointerdown="handlePointerDown"
    @pointermove="handlePointerMove"
    @pointerup="handlePointerUp"
    @pointercancel="handlePointerUp"
  >
    <svg
      ref="svgRef"
      class="waveform-svg"
      viewBox="0 0 500 8"
      preserveAspectRatio="none"
    >
      <path class="wave-inactive" fill="none" stroke-width="2" stroke-linecap="round" />
      <path class="wave-active" fill="none" stroke-width="3" stroke-linecap="round" />
    </svg>
    <!-- thumb 独立于 SVG，不受拉伸影响 -->
    <div class="thumb" />
  </div>
</template>

<style scoped>
.waveform-container {
  position: relative;
  width: 100%;
  height: 8px;
  cursor: pointer;
  touch-action: none;
}

/* 扩大点击热区的透明伪元素 */
.waveform-container::before {
  content: '';
  position: absolute;
  top: -12px;
  bottom: -12px;
  left: 0;
  right: 0;
}

.waveform-svg {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  shape-rendering: geometricPrecision;
}

.wave-inactive {
  stroke: var(--wave-inactive-color, rgba(255,255,255,0.35));
  transition: stroke 0.72s cubic-bezier(0.22, 1, 0.36, 1);
}

.wave-active {
  stroke: var(--wave-active-color, #fff);
  transition: stroke 0.72s cubic-bezier(0.22, 1, 0.36, 1);
}

.thumb {
  position: absolute;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--waveform-thumb-color, #fff);
  transform: translate(-50%, -50%);
  transition: width 150ms, height 150ms,
              background 0.72s cubic-bezier(0.22, 1, 0.36, 1),
              box-shadow 0.72s cubic-bezier(0.22, 1, 0.36, 1);
  box-shadow: 0 0 4px rgba(255, 255, 255, 0.3);
  pointer-events: none;
  z-index: 1;
  will-change: left, top;
}
</style>
