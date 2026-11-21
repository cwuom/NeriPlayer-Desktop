<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'

/// 自定义背景图：模糊只在加载、尺寸或参数变化时画进 canvas 一次。
/// 以前整窗元素挂实时 CSS blur，播放页这类全屏层盖住再移开时整层要重新栅格化，
/// 来不及画完的那几帧会透出底色（打开/关闭播放页闪白），且每次重绘都要重算一遍模糊
const props = defineProps<{
  src: string
  blurPx: number
  opacity: number
  /** 0–1 压暗，直接画进 canvas，不额外叠一层全屏遮罩 */
  dim: number
}>()

const canvasRef = ref<HTMLCanvasElement | null>(null)
const isReady = ref(false)
const supportsCanvasFilter = typeof CanvasRenderingContext2D !== 'undefined'
  && 'filter' in CanvasRenderingContext2D.prototype

let image: HTMLImageElement | null = null
let loadToken = 0
let resizeTimer: ReturnType<typeof setTimeout> | null = null
const RESIZE_SETTLE_MS = 160

const fallbackStyle = computed(() => (
  supportsCanvasFilter || props.blurPx <= 0
    ? {}
    : { filter: `blur(${props.blurPx}px)`, transform: 'scale(1.1)' }
))

function draw(): void {
  const canvas = canvasRef.value
  const source = image
  if (!canvas || !source || !source.naturalWidth) return
  const viewportWidth = Math.max(1, window.innerWidth)
  const viewportHeight = Math.max(1, window.innerHeight)
  const blur = Math.max(0, props.blurPx)
  // 模糊后高频细节本来就没了，半分辨率绘制肉眼无差别，省一半以上的填充和显存
  const scale = blur >= 8 ? 0.5 : Math.min(window.devicePixelRatio || 1, 2)
  const width = Math.round(viewportWidth * scale)
  const height = Math.round(viewportHeight * scale)
  if (canvas.width !== width) canvas.width = width
  if (canvas.height !== height) canvas.height = height

  const context = canvas.getContext('2d')
  if (!context) return
  context.clearRect(0, 0, width, height)

  // 向外多画一圈，模糊后边缘不会向透明渐隐
  const bleed = supportsCanvasFilter ? blur * scale * 2 : 0
  const targetWidth = width + bleed * 2
  const targetHeight = height + bleed * 2
  const cover = Math.max(targetWidth / source.naturalWidth, targetHeight / source.naturalHeight)
  const drawWidth = source.naturalWidth * cover
  const drawHeight = source.naturalHeight * cover
  context.filter = supportsCanvasFilter && blur > 0 ? `blur(${blur * scale}px)` : 'none'
  context.imageSmoothingQuality = 'high'
  context.drawImage(
    source,
    (width - drawWidth) / 2,
    (height - drawHeight) / 2,
    drawWidth,
    drawHeight,
  )
  context.filter = 'none'
  const dim = Math.min(Math.max(props.dim, 0), 1)
  if (dim > 0) {
    context.fillStyle = `rgba(0, 0, 0, ${dim})`
    context.fillRect(0, 0, width, height)
  }
  isReady.value = true
}

function load(src: string): void {
  const token = ++loadToken
  if (!src) {
    image = null
    isReady.value = false
    return
  }
  const next = new Image()
  next.decoding = 'async'
  next.src = src
  next.decode()
    .then(() => {
      if (token !== loadToken) return
      image = next
      draw()
    })
    .catch(() => {
      if (token !== loadToken) return
      image = null
      isReady.value = false
    })
}

function onResize(): void {
  if (resizeTimer) clearTimeout(resizeTimer)
  // 拖动窗口期间 canvas 按 CSS 拉伸即可，停下后再按新尺寸重画
  resizeTimer = setTimeout(() => {
    resizeTimer = null
    draw()
  }, RESIZE_SETTLE_MS)
}

watch(() => props.src, load)
watch(() => [props.blurPx, props.dim], () => draw())

onMounted(() => {
  window.addEventListener('resize', onResize)
  load(props.src)
})

onUnmounted(() => {
  loadToken++
  window.removeEventListener('resize', onResize)
  if (resizeTimer) clearTimeout(resizeTimer)
  image = null
})
</script>

<template>
  <canvas
    ref="canvasRef"
    class="app-bg-image"
    :style="{ ...fallbackStyle, opacity: isReady ? opacity : 0 }"
    aria-hidden="true"
  />
</template>

<style scoped>
.app-bg-image {
  position: fixed;
  inset: 0;
  z-index: 0;
  width: 100%;
  height: 100%;
  display: block;
  pointer-events: none;
  /* 常驻独立合成层：全屏层盖住再移开时直接复用纹理，不重新栅格化 */
  will-change: transform;
  transition: opacity 360ms var(--ease-standard);
}
</style>
