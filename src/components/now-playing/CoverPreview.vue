<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useEscapeClose } from '@/composables/useEscapeClose'
import { IDENTITY, clampPan, wheelZoomFactor, zoomAround, type ViewerTransform } from '@/modules/viewer/zoom'

// 封面大图预览（#52）：单击封面打开；滚轮以光标为中心缩放，按住拖动平移，
// 双击在适应窗口与 2.5 倍之间切换，Esc / 点空白处关闭，+ - 0 键缩放与复位
const props = defineProps<{
  open: boolean
  /** 原图地址；加载完成前先显示 placeholder（当前已显示的封面），避免空白 */
  src: string
  placeholder?: string
  title?: string
}>()

const emit = defineEmits<{
  close: []
  save: []
}>()

const { t } = useI18n()
const stageRef = ref<HTMLElement | null>(null)
const transform = ref<ViewerTransform>({ ...IDENTITY })
const loaded = ref(false)
const failed = ref(false)
const dragging = ref(false)
let dragStart: { x: number; y: number; originX: number; originY: number; moved: boolean } | null = null

const displaySrc = computed(() => (loaded.value || !props.placeholder ? props.src : props.placeholder))
const zoomPercent = computed(() => `${Math.round(transform.value.scale * 100)}%`)
// 按舞台可用区域等比放大到“适应窗口”尺寸：小封面（如 300px）也能铺满预览区
const natural = ref({ width: 0, height: 0 })
const stageBox = ref({ width: 0, height: 0 })
const fittedSize = computed(() => {
  const { width, height } = natural.value
  if (!width || !height || !stageBox.value.width || !stageBox.value.height) return null
  const ratio = Math.min(stageBox.value.width / width, stageBox.value.height / height)
  return { width: Math.round(width * ratio), height: Math.round(height * ratio) }
})
const imageStyle = computed(() => ({
  ...(fittedSize.value ? { width: `${fittedSize.value.width}px`, height: `${fittedSize.value.height}px` } : {}),
  transform: `translate(${transform.value.x}px, ${transform.value.y}px) scale(${transform.value.scale})`,
}))

function measureStage() {
  const stage = stageRef.value
  if (!stage) return
  const style = getComputedStyle(stage)
  stageBox.value = {
    width: stage.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight),
    height: stage.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom),
  }
}

function onImageLoad(event: Event) {
  const image = event.target as HTMLImageElement
  natural.value = { width: image.naturalWidth, height: image.naturalHeight }
  measureStage()
}

function onWindowResize() {
  measureStage()
  transform.value = { ...IDENTITY }
}

onMounted(() => window.addEventListener('resize', onWindowResize))
onBeforeUnmount(() => window.removeEventListener('resize', onWindowResize))

watch(() => [props.open, props.src] as const, ([open]) => {
  transform.value = { ...IDENTITY }
  loaded.value = false
  failed.value = false
  if (!open || !props.src) return
  const probe = new Image()
  probe.referrerPolicy = 'no-referrer'
  probe.onload = () => { loaded.value = true }
  probe.onerror = () => { failed.value = true }
  probe.src = props.src
  void nextTick(() => {
    measureStage()
    stageRef.value?.focus()
  })
}, { immediate: true })

function stageSize() {
  const image = stageRef.value?.querySelector('img')
  return { width: image?.clientWidth ?? 0, height: image?.clientHeight ?? 0 }
}

/** 光标相对图片中心（未变换时）的坐标 */
function anchorFrom(event: { clientX: number; clientY: number }) {
  const rect = stageRef.value?.getBoundingClientRect()
  if (!rect) return { x: 0, y: 0 }
  return { x: event.clientX - (rect.left + rect.width / 2), y: event.clientY - (rect.top + rect.height / 2) }
}

function zoomTo(scale: number, anchor = { x: 0, y: 0 }) {
  transform.value = zoomAround(transform.value, scale, anchor, stageSize())
}

function onWheel(event: WheelEvent) {
  event.preventDefault()
  zoomTo(transform.value.scale * wheelZoomFactor(event.deltaY), anchorFrom(event))
}

function onDoubleClick(event: MouseEvent) {
  zoomTo(transform.value.scale > 1.05 ? 1 : 2.5, anchorFrom(event))
}

function onPointerDown(event: PointerEvent) {
  if (event.button !== 0) return
  dragStart = { x: event.clientX, y: event.clientY, originX: transform.value.x, originY: transform.value.y, moved: false }
  if (transform.value.scale > 1) {
    dragging.value = true
    ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  }
}

function onPointerMove(event: PointerEvent) {
  if (!dragStart) return
  const dx = event.clientX - dragStart.x
  const dy = event.clientY - dragStart.y
  if (Math.hypot(dx, dy) > 4) dragStart.moved = true
  if (!dragging.value) return
  transform.value = clampPan({ scale: transform.value.scale, x: dragStart.originX + dx, y: dragStart.originY + dy }, stageSize())
}

function onPointerUp(event: PointerEvent) {
  const start = dragStart
  dragStart = null
  dragging.value = false
  // 未缩放时点击图片以外的空白处关闭；拖动或点在图片上都不关
  if (start && !start.moved && transform.value.scale <= 1 && !(event.target as HTMLElement).closest('img, .cover-preview-toolbar')) {
    emit('close')
  }
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === '+' || event.key === '=') zoomTo(transform.value.scale * 1.25)
  else if (event.key === '-' || event.key === '_') zoomTo(transform.value.scale / 1.25)
  else if (event.key === '0') zoomTo(1)
  else return
  event.preventDefault()
}

useEscapeClose(() => props.open, () => emit('close'))
</script>

<template>
  <Teleport to="body">
    <Transition name="cover-preview">
      <div
        v-if="open"
        class="cover-preview"
        role="dialog"
        aria-modal="true"
        :aria-label="title || t('player.cover_preview')"
      >
        <div
          ref="stageRef"
          class="cover-preview-stage"
          :class="{ zoomed: transform.scale > 1, dragging }"
          tabindex="-1"
          @wheel="onWheel"
          @dblclick="onDoubleClick"
          @pointerdown="onPointerDown"
          @pointermove="onPointerMove"
          @pointerup="onPointerUp"
          @pointercancel="onPointerUp"
          @keydown="onKeydown"
        >
          <img
            v-if="displaySrc && !failed"
            :src="displaySrc"
            :alt="title || ''"
            class="cover-preview-image"
            :style="imageStyle"
            referrerpolicy="no-referrer"
            draggable="false"
            @load="onImageLoad"
          />
          <span v-else class="material-symbols-rounded cover-preview-empty" aria-hidden="true">image_not_supported</span>
        </div>

        <div class="cover-preview-toolbar" role="toolbar" :aria-label="t('player.cover_preview')">
          <span v-if="title" class="cover-preview-title">{{ title }}</span>
          <button type="button" :title="t('player.zoom_out')" :aria-label="t('player.zoom_out')" @click="zoomTo(transform.scale / 1.25)">
            <span class="material-symbols-rounded">zoom_out</span>
          </button>
          <button type="button" class="cover-preview-zoom" :title="t('player.zoom_reset')" @click="zoomTo(1)">{{ zoomPercent }}</button>
          <button type="button" :title="t('player.zoom_in')" :aria-label="t('player.zoom_in')" @click="zoomTo(transform.scale * 1.25)">
            <span class="material-symbols-rounded">zoom_in</span>
          </button>
          <span class="cover-preview-divider" aria-hidden="true" />
          <button type="button" :title="t('player.save_cover')" :aria-label="t('player.save_cover')" @click="emit('save')">
            <span class="material-symbols-rounded">download</span>
          </button>
          <button type="button" :title="t('common.close')" :aria-label="t('common.close')" @click="emit('close')">
            <span class="material-symbols-rounded">close</span>
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped lang="scss">
.cover-preview {
  position: fixed;
  inset: 0;
  z-index: 9500;
  background: rgb(8 8 10 / 0.86);
  -webkit-backdrop-filter: blur(18px);
  backdrop-filter: blur(18px);
  color: #fff;
}

.cover-preview-stage {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  padding: 56px 48px 96px;
  overflow: hidden;
  outline: none;
  cursor: zoom-in;

  &.zoomed { cursor: grab; }
  &.dragging { cursor: grabbing; }
}

.cover-preview-image {
  max-width: 100%;
  max-height: 100%;
  border-radius: var(--radius-md);
  box-shadow: 0 30px 80px rgb(0 0 0 / 0.5);
  user-select: none;
  transform-origin: center;
  transition: transform 140ms var(--ease-standard);

  .dragging & { transition: none; }
}

.cover-preview-empty {
  font-size: 64px;
  opacity: 0.4;
}

.cover-preview-toolbar {
  position: absolute;
  left: 50%;
  bottom: 28px;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: 2px;
  max-width: calc(100vw - 32px);
  padding: 6px;
  border-radius: var(--radius-full);
  background: rgb(40 40 44 / 0.82);
  box-shadow: 0 10px 30px rgb(0 0 0 / 0.4), inset 0 0 0 1px rgb(255 255 255 / 0.08);

  button {
    display: grid;
    place-items: center;
    min-width: 36px;
    height: 36px;
    padding: 0 8px;
    border-radius: var(--radius-full);
    color: rgb(255 255 255 / 0.86);
    font-size: var(--fs-13);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    transition: background var(--duration-short), color var(--duration-short);

    &:hover { background: rgb(255 255 255 / 0.12); color: #fff; }
    .material-symbols-rounded { font-size: 21px; }
  }
}

.cover-preview-zoom { min-width: 58px !important; }

.cover-preview-title {
  max-width: 240px;
  padding: 0 12px 0 10px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-13);
  font-weight: 600;
  color: rgb(255 255 255 / 0.9);
}

.cover-preview-divider {
  width: 1px;
  height: 20px;
  margin: 0 4px;
  background: rgb(255 255 255 / 0.16);
}

.cover-preview-enter-active,
.cover-preview-leave-active {
  transition: opacity 200ms var(--ease-standard);

  .cover-preview-image { transition: transform 260ms var(--ease-emphasized-decel); }
}

.cover-preview-enter-from,
.cover-preview-leave-to {
  opacity: 0;

  .cover-preview-image { transform: scale(0.92); }
}
</style>
