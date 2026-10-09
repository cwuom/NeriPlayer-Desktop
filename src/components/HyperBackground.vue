<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch } from 'vue'
import { hyperBackgroundVertexShader, hyperBackgroundFragmentShader } from '@/shaders/hyperBackground'
import { createLogger } from '@/utils/logger'

const log = createLogger('hyper-background')

const props = withDefaults(defineProps<{
  musicLevel?: number
  beatImpulse?: number
  colors?: [number[], number[], number[], number[], number[]]
  isDark?: boolean
  lightOffset?: number
  saturateOffset?: number
  /** 暂停时律动和调色过渡收敛后停表，画面停在最后一帧 */
  playing?: boolean
}>(), {
  playing: true,
  musicLevel: 0,
  beatImpulse: 0,
  colors: () => [
    [0.07, 0.27, 0.42, 1],
    [0.35, 0.24, 0.20, 1],
    [0.34, 0.12, 0.26, 1],
    [0.17, 0.14, 0.34, 1],
    [0.18, 0.34, 0.36, 1],
  ],
  isDark: true,
  lightOffset: 0,
  saturateOffset: 0,
})

const canvas = ref<HTMLCanvasElement>()
let gl: WebGLRenderingContext | null = null
let program: WebGLProgram | null = null
let quadBuffer: WebGLBuffer | null = null
let animFrame = 0
// 只在渲染时累加，停表期间不走，恢复播放时动画从停下的位置接着动
let shaderTime = 0
let lastFrameMs = 0
let resizeObserver: ResizeObserver | null = null
const SETTLED_EPSILON = 1e-3

// —— 自适应渲染质量 ——
// WebKitGTK 的 WebGL 在不同驱动下性能差异巨大：NVIDIA 混合显卡 EGL 路径、
// 软件渲染（llvmpipe）时全屏 2x 分辨率逐帧重绘会严重卡顿，且 CPU/GPU 占用
// 看着不高（瓶颈在 WebKit 内部光栅/合成路径）。按实测帧耗时动态调整内部
// 渲染分辨率与帧率：GPU 富余时维持满质量，吃紧时逐级降档，恢复后再升回。
// 衡量指标是 rAF 回调间隔（跳帧的回调几乎零开销，不会把有意降低的帧率误判为卡顿）
const QUALITY_ADJUST_INTERVAL_MS = 1000
const QUALITY_SLOW_INTERVAL_MS = 33
const QUALITY_FAST_INTERVAL_MS = 20
// 连续几次检查都富余才升档，避免在临界点来回抖动
const QUALITY_RECOVER_CHECKS = 3
// 停表、切后台之后的第一个间隔不代表渲染压力
const FRAME_GAP_IGNORE_MS = 250
const QUALITY_SCALES = [1.0, 0.75, 0.5]
// 取显示器刷新率的整数分频，避免非整除档位被 vsync 量化成更低的实际帧率
const QUALITY_FPS_TIERS = [60, 30, 20]
let scaleTier = 0
let fpsTier = 0
let lastQualityCheckAt = 0
let lastRenderedAt = 0
let lastTickAt = 0
let avgFrameInterval = 0
let healthyChecks = 0
let qualityDowns = 0

function rendererIsSoftware(renderer: string): boolean {
  return /llvmpipe|softpipe|swiftshader|software/i.test(renderer)
}

function sampleFrameInterval(nowMs: number): void {
  const gap = lastTickAt ? nowMs - lastTickAt : 0
  lastTickAt = nowMs
  if (gap <= 0 || gap > FRAME_GAP_IGNORE_MS) return
  avgFrameInterval = avgFrameInterval ? avgFrameInterval * 0.9 + gap * 0.1 : gap
}

function adjustQuality(nowMs: number): void {
  if (nowMs - lastQualityCheckAt < QUALITY_ADJUST_INTERVAL_MS) return
  lastQualityCheckAt = nowMs
  if (!avgFrameInterval) return
  const interval = avgFrameInterval

  if (interval > QUALITY_SLOW_INTERVAL_MS) {
    healthyChecks = 0
    // 低于 ~30fps：先降内部分辨率，再降帧率
    if (scaleTier < QUALITY_SCALES.length - 1) scaleTier += 1
    else if (fpsTier < QUALITY_FPS_TIERS.length - 1) fpsTier += 1
    else return
    avgFrameInterval = 0
    qualityDowns += 1
    if (qualityDowns <= 3) {
      log.warn('WebGL 帧耗时偏高，降档渲染:', {
        frameIntervalMs: Math.round(interval),
        qualityScale: QUALITY_SCALES[scaleTier],
        qualityFps: QUALITY_FPS_TIERS[fpsTier],
      })
    }
    return
  }

  if (interval >= QUALITY_FAST_INTERVAL_MS || (scaleTier === 0 && fpsTier === 0)) {
    healthyChecks = 0
    return
  }
  healthyChecks += 1
  if (healthyChecks < QUALITY_RECOVER_CHECKS) return
  healthyChecks = 0
  // 恢复顺序与降档相反：先找回流畅度，再找回清晰度
  if (fpsTier > 0) fpsTier -= 1
  else scaleTier -= 1
  avgFrameInterval = 0
  qualityDowns = 0
}

// 音乐律动第二级非对称平滑（对齐 Android HyperBackground 帧循环）
// Rust analyzer.rs 已做第一级帧率无关衰减，这里补一层 attack/release，
// 并统一换算为与帧率无关的系数，避免 120Hz(ProMotion)/30fps 上手感不同。
const REF_FPS = 60            // 参考帧率：Android 每帧系数按 ~60fps 调校
const LEVEL_ATTACK = 0.12     // level 上升系数（@REF_FPS 每帧）
const LEVEL_RELEASE = 0.045   // level 下降系数
const BEAT_ATTACK = 0.46      // beat 上升系数
const BEAT_RELEASE = 0.12     // beat 下降系数
const BEAT_SCALE = 0.94       // 对齐 Android targetBeat = beat * 0.94
let smoothLevel = 0
let smoothBeat = 0

// 调色板过渡：基于时间的 520ms smoothStep（对齐 Android，帧率无关）
const PALETTE_TRANSITION_MS = 520
const smoothColors: number[][] = [
  [0.07, 0.27, 0.42, 1],
  [0.35, 0.24, 0.20, 1],
  [0.34, 0.12, 0.26, 1],
  [0.17, 0.14, 0.34, 1],
  [0.18, 0.34, 0.36, 1],
]
let smoothLightOffset = 0
let smoothSaturateOffset = 0
// 过渡起点快照 + 当前目标记录
const transStartColors: number[][] = smoothColors.map((c) => c.slice())
let transStartLight = 0
let transStartSaturate = 0
let transStartMs = 0
let transitioning = false
const targetColors: number[][] = smoothColors.map((c) => c.slice())
let targetLight = 0
let targetSaturate = 0

function lerpVal(current: number, target: number, t: number): number {
  return current + (target - current) * t
}

// 将「@REF_FPS 每帧系数」换算为与帧率无关的等效系数
function frameIndependentRate(perFrameRate: number, dt: number): number {
  return 1 - Math.pow(1 - perFrameRate, dt * REF_FPS)
}

// 3t^2 - 2t^3，对齐 Android smoothStep01
function smoothStep01(t: number): number {
  const x = t < 0 ? 0 : t > 1 ? 1 : t
  return x * x * (3 - 2 * x)
}

// 目标调色板是否变化（切歌）-> 启动一次定时过渡
function colorsChanged(): boolean {
  for (let i = 0; i < 5; i++) {
    for (let j = 0; j < 4; j++) {
      if (targetColors[i][j] !== props.colors[i][j]) return true
    }
  }
  return targetLight !== props.lightOffset || targetSaturate !== props.saturateOffset
}

// Uniform locations
let uResolution: WebGLUniformLocation | null = null
let uTime: WebGLUniformLocation | null = null
let uMusicLevel: WebGLUniformLocation | null = null
let uBeat: WebGLUniformLocation | null = null
let uColor0: WebGLUniformLocation | null = null
let uColor1: WebGLUniformLocation | null = null
let uColor2: WebGLUniformLocation | null = null
let uColor3: WebGLUniformLocation | null = null
let uColor4: WebGLUniformLocation | null = null
let uDarkMode: WebGLUniformLocation | null = null
let uLightOffset: WebGLUniformLocation | null = null
let uSaturateOffset: WebGLUniformLocation | null = null

function compileShader(gl: WebGLRenderingContext, src: string, type: number): WebGLShader | null {
  const shader = gl.createShader(type)
  if (!shader) return null
  gl.shaderSource(shader, src)
  gl.compileShader(shader)
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    log.error('Shader compile error:', gl.getShaderInfoLog(shader))
    gl.deleteShader(shader)
    return null
  }
  return shader
}

function initGL() {
  if (!canvas.value) return
  gl = canvas.value.getContext('webgl', { alpha: true, antialias: false, premultipliedAlpha: false })
  if (!gl) { log.error('WebGL not supported'); return }

  // 诊断：记录实际 GL 渲染器，软件渲染（llvmpipe 等）直接预降档
  // RENDERER 在多数 WebView 里被屏蔽成通用字符串，真实型号要走调试扩展
  const debugInfo = gl.getExtension('WEBGL_debug_renderer_info')
  const glRenderer = String(gl.getParameter(debugInfo ? debugInfo.UNMASKED_RENDERER_WEBGL : gl.RENDERER) ?? '')
  const glVendor = String(gl.getParameter(debugInfo ? debugInfo.UNMASKED_VENDOR_WEBGL : gl.VENDOR) ?? '')
  log.info('WebGL context:', { vendor: glVendor, renderer: glRenderer })
  if (rendererIsSoftware(glRenderer)) {
    log.warn('WebGL 为软件渲染，直接降到低档以保流畅:', { renderer: glRenderer })
    scaleTier = QUALITY_SCALES.length - 1
    fpsTier = QUALITY_FPS_TIERS.length - 1
  }

  const vs = compileShader(gl, hyperBackgroundVertexShader, gl.VERTEX_SHADER)
  const fs = compileShader(gl, hyperBackgroundFragmentShader, gl.FRAGMENT_SHADER)
  if (!vs || !fs) return

  program = gl.createProgram()!
  gl.attachShader(program, vs)
  gl.attachShader(program, fs)
  gl.linkProgram(program)
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    log.error('Program link error:', gl.getProgramInfoLog(program))
    gl.deleteShader(vs)
    gl.deleteShader(fs)
    gl.deleteProgram(program)
    program = null
    return
  }
  // link 成功后 shader 对象即可释放，避免随组件反复创建而累积
  gl.detachShader(program, vs)
  gl.detachShader(program, fs)
  gl.deleteShader(vs)
  gl.deleteShader(fs)
  gl.useProgram(program)

  // 全屏四边形
  quadBuffer = gl.createBuffer()
  gl.bindBuffer(gl.ARRAY_BUFFER, quadBuffer)
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1, 1,-1, -1,1, 1,1]), gl.STATIC_DRAW)
  const aPos = gl.getAttribLocation(program, 'a_position')
  gl.enableVertexAttribArray(aPos)
  gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0)

  // Uniforms
  uResolution = gl.getUniformLocation(program, 'u_resolution')
  uTime = gl.getUniformLocation(program, 'u_time')
  uMusicLevel = gl.getUniformLocation(program, 'u_musicLevel')
  uBeat = gl.getUniformLocation(program, 'u_beat')
  uColor0 = gl.getUniformLocation(program, 'u_color0')
  uColor1 = gl.getUniformLocation(program, 'u_color1')
  uColor2 = gl.getUniformLocation(program, 'u_color2')
  uColor3 = gl.getUniformLocation(program, 'u_color3')
  uColor4 = gl.getUniformLocation(program, 'u_color4')
  uDarkMode = gl.getUniformLocation(program, 'u_darkMode')
  uLightOffset = gl.getUniformLocation(program, 'u_lightOffset')
  uSaturateOffset = gl.getUniformLocation(program, 'u_saturateOffset')

  // 初始化平滑颜色为当前 props
  for (let i = 0; i < 5; i++) {
    for (let j = 0; j < 4; j++) {
      smoothColors[i][j] = props.colors[i][j]
      targetColors[i][j] = props.colors[i][j]
    }
  }
  smoothLightOffset = props.lightOffset
  smoothSaturateOffset = props.saturateOffset
  targetLight = props.lightOffset
  targetSaturate = props.saturateOffset

  lastFrameMs = performance.now()
  render()
}

function render() {
  if (!gl || !program) return
  const nowMs = performance.now()
  sampleFrameInterval(nowMs)
  adjustQuality(nowMs)

  // 帧率档位低于满速时按目标帧间隔跳过绘制（保留 rAF 保证恢复及时）；
  // 留 2ms 余量吸收 vsync 抖动，否则 30fps 档会被量化成 20fps
  const qualityFps = QUALITY_FPS_TIERS[fpsTier]
  if (fpsTier > 0 && nowMs - lastRenderedAt < 1000 / qualityFps - 2) {
    animFrame = requestAnimationFrame(render)
    return
  }

  const c = canvas.value!
  // mac/GPU 富余场景允许到 2.0，保证 Retina/HiDPI 清晰度；其余仍限制以省 GPU
  const dprCap = window.devicePixelRatio >= 2 ? 2.0 : 1.5
  // 内部渲染分辨率 = 设备缩放 × 自适应档位（低档时由 CSS 拉伸，背景本就模糊无感知）
  const dpr = Math.min(window.devicePixelRatio || 1, dprCap) * QUALITY_SCALES[scaleTier]
  // canvas 尺寸只存整数；不取整的话小数 DPR 下每帧都判定变化，重设尺寸会重新分配绘图缓冲
  const w = Math.max(1, Math.round(c.clientWidth * dpr))
  const h = Math.max(1, Math.round(c.clientHeight * dpr))
  if (c.width !== w || c.height !== h) { c.width = w; c.height = h }
  gl.viewport(0, 0, w, h)

  const dt = Math.min((nowMs - lastFrameMs) / 1000, 0.1) // 秒，钳制避免卡顿后跳变
  lastFrameMs = nowMs

  // 调色板过渡：检测目标变化 -> 启动 520ms 定时 smoothStep 过渡
  if (colorsChanged()) {
    for (let i = 0; i < 5; i++) transStartColors[i] = smoothColors[i].slice()
    transStartLight = smoothLightOffset
    transStartSaturate = smoothSaturateOffset
    for (let i = 0; i < 5; i++) {
      for (let j = 0; j < 4; j++) targetColors[i][j] = props.colors[i][j]
    }
    targetLight = props.lightOffset
    targetSaturate = props.saturateOffset
    transStartMs = nowMs
    transitioning = true
  }
  if (transitioning) {
    const raw = (nowMs - transStartMs) / PALETTE_TRANSITION_MS
    const f = smoothStep01(raw)
    for (let i = 0; i < 5; i++) {
      for (let j = 0; j < 4; j++) {
        smoothColors[i][j] = lerpVal(transStartColors[i][j], targetColors[i][j], f)
      }
    }
    smoothLightOffset = lerpVal(transStartLight, targetLight, f)
    smoothSaturateOffset = lerpVal(transStartSaturate, targetSaturate, f)
    if (raw >= 1) transitioning = false
  }

  shaderTime += dt
  gl.uniform2f(uResolution, w, h)
  gl.uniform1f(uTime, shaderTime)

  // 音乐律动第二级非对称平滑（帧率无关）
  const targetLevel = props.playing ? Math.min(Math.max(props.musicLevel, 0), 1) : 0
  const targetBeat = props.playing ? Math.min(Math.max(props.beatImpulse * BEAT_SCALE, 0), 1) : 0
  const levelRate = frameIndependentRate(
    targetLevel > smoothLevel ? LEVEL_ATTACK : LEVEL_RELEASE, dt)
  const beatRate = frameIndependentRate(
    targetBeat > smoothBeat ? BEAT_ATTACK : BEAT_RELEASE, dt)
  smoothLevel += (targetLevel - smoothLevel) * levelRate
  smoothBeat += (targetBeat - smoothBeat) * beatRate

  gl.uniform1f(uMusicLevel, smoothLevel)
  gl.uniform1f(uBeat, smoothBeat)
  gl.uniform4fv(uColor0, smoothColors[0])
  gl.uniform4fv(uColor1, smoothColors[1])
  gl.uniform4fv(uColor2, smoothColors[2])
  gl.uniform4fv(uColor3, smoothColors[3])
  gl.uniform4fv(uColor4, smoothColors[4])
  gl.uniform1f(uDarkMode, props.isDark ? 1.0 : 0.0)
  gl.uniform1f(uLightOffset, smoothLightOffset)
  gl.uniform1f(uSaturateOffset, smoothSaturateOffset)

  gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4)
  lastRenderedAt = nowMs

  const settled = !transitioning
    && smoothLevel < SETTLED_EPSILON
    && smoothBeat < SETTLED_EPSILON
  if (!props.playing && settled) {
    animFrame = 0
    return
  }
  animFrame = requestAnimationFrame(render)
}

/// 停表后由播放态、调色板、律动、尺寸变化重新拉起；窗口不可见时不拉
function wake() {
  if (!gl || !program || animFrame || document.hidden) return
  // 重置帧时钟，避免 dt 巨大导致平滑量跳变
  lastFrameMs = performance.now()
  lastTickAt = 0
  animFrame = requestAnimationFrame(render)
}

// 窗口不可见时暂停 rAF（全屏 shader 是主要功耗项），可见时恢复
function handleVisibilityChange() {
  if (document.hidden) {
    cancelAnimationFrame(animFrame)
    animFrame = 0
  } else {
    wake()
  }
}

// 睡眠唤醒、显卡驱动重置会丢上下文；不接管的话背景会一直是空白
function handleContextLost(event: Event) {
  event.preventDefault()
  cancelAnimationFrame(animFrame)
  animFrame = 0
  program = null
  quadBuffer = null
}

function handleContextRestored() {
  initGL()
}

watch(
  () => [props.playing, props.colors, props.isDark, props.lightOffset, props.saturateOffset],
  wake,
)

onMounted(() => {
  initGL()
  document.addEventListener('visibilitychange', handleVisibilityChange)
  canvas.value?.addEventListener('webglcontextlost', handleContextLost)
  canvas.value?.addEventListener('webglcontextrestored', handleContextRestored)
  if (canvas.value && typeof ResizeObserver !== 'undefined') {
    resizeObserver = new ResizeObserver(wake)
    resizeObserver.observe(canvas.value)
  }
})

onUnmounted(() => {
  document.removeEventListener('visibilitychange', handleVisibilityChange)
  canvas.value?.removeEventListener('webglcontextlost', handleContextLost)
  canvas.value?.removeEventListener('webglcontextrestored', handleContextRestored)
  resizeObserver?.disconnect()
  resizeObserver = null
  cancelAnimationFrame(animFrame)
  animFrame = 0
  // 释放 WebGL 资源并主动丢弃 context，防止反复开关正在播放页耗尽 context 配额
  if (gl) {
    if (quadBuffer) gl.deleteBuffer(quadBuffer)
    if (program) gl.deleteProgram(program)
    gl.getExtension('WEBGL_lose_context')?.loseContext()
  }
  quadBuffer = null
  program = null
  gl = null
})
</script>

<template>
  <canvas ref="canvas" class="hyper-bg" />
</template>

<style scoped>
.hyper-bg {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  z-index: 0;
  opacity: 0.80; /* 对齐 Android graphicsLayer { alpha = 0.80f }，让底色透出 */
}
</style>
