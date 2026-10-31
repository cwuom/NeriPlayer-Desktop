<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { useEscapeClose } from '@/composables/useEscapeClose'
import { usePlayerStore } from '@/stores/player'
import { useToastStore } from '@/stores/toast'
import { getTrackCoverUrl } from '@/utils/trackCover'
import { renderPoster, loadPosterLogo, posterToBlob, createPosterMeasure, writePosterToClipboard, type RenderedPoster } from '@/modules/lyrics/posterCanvas'
import { MAX_POSTER_VISUAL_LINES, posterVisualLines } from '@/modules/lyrics/posterLayout'
import { readImageBytes, safeFileName, saveImageWithDialog } from '@/utils/imageTransfer'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'

const log = createLogger('lyric-share')

const props = defineProps<{
  open: boolean
  /** 勾选列表的数据源，由调用方从歌词行取好显示文本 */
  lines: { index: number, text: string }[]
  /** 打开时默认选中的行（右键点的那一句） */
  initialIndex: number
}>()

const emit = defineEmits<{ 'update:open': [value: boolean] }>()

const { t } = useI18n()
const toast = useToastStore()
const player = usePlayerStore()

const title = computed(() => player.currentTrack?.title || '')
const artist = computed(() => player.currentTrack?.artist || '')
/** 与保存封面同源：取原图地址，取不到再退回显示用地址 */
const coverUrl = computed(() => getTrackCoverUrl(player.currentTrack, 'original') || player.currentTrack?.coverUrl || null)

type Stage = 'select' | 'preview'
/** 打开时默认选中的行数（从右键那一句往后数） */
const DEFAULT_SELECTED_LINES = 5
const stage = ref<Stage>('select')
const selected = ref(new Set<number>())
const busy = ref(false)
/**
 * 出图会话号：封面取图和渲染都在 await 里，期间用户可以关掉弹窗再从另一句打开。
 * 每次打开与每次点"生成海报"都推进一次号，结果回来对不上就直接丢掉，
 * 否则旧会话的海报会盖掉新会话的状态，保存与复制发出去的是上一张图
 */
let renderGeneration = 0
const poster = ref<RenderedPoster | null>(null)
const previewRef = ref<HTMLCanvasElement | null>(null)
const listRef = ref<HTMLDivElement | null>(null)

const selectedCount = computed(() => selected.value.size)
const selectedLines = computed(() => props.lines
  .filter(line => selected.value.has(line.index))
  .map(line => line.text))
const measure = createPosterMeasure()
/** 版面占用按折行后的视觉行算：一句长歌词可能占两三行 */
const visualLines = computed(() => posterVisualLines(selectedLines.value, measure))
/** 版面满了就不让再勾 */
const atLimit = computed(() => visualLines.value >= MAX_POSTER_VISUAL_LINES)
/** 点下去才发现这一句装不下：用它把上限提示顶出来，否则用户只看到点了没反应 */
const rejectedAdd = ref(false)
const limitReached = computed(() => atLimit.value || rejectedAdd.value)
const canSubmit = computed(() => selectedCount.value > 0 && !busy.value)

/** 拟议的这批句子能不能排进海报预算（含"排不下会丢行"那种情况） */
function fitsBudget(texts: string[]): boolean {
  return posterVisualLines(texts, measure) <= MAX_POSTER_VISUAL_LINES
}

/** 按勾选顺序取出给定集合的歌词文本 */
function textsOf(indices: Set<number>): string[] {
  return props.lines.filter(line => indices.has(line.index)).map(line => line.text)
}

watch(() => props.open, async open => {
  if (!open) return
  stage.value = 'select'
  poster.value = null
  rejectedAdd.value = false
  // 旧会话的渲染结果会被会话号丢掉，所以这里直接放开 busy：
  // 否则新会话要等那张已经作废的海报画完才能点"生成海报"，看起来就像按钮坏了
  busy.value = false
  renderGeneration++
  selected.value = new Set()
  // 字体没就绪时 measureText 量的是后备字体宽度，默认预选会按错的行数算出图来
  await document.fonts.ready
  // 期间被关掉、切到预览、或用户已经自己勾了句子（那就尊重他的选择，不再覆盖）
  if (!props.open || stage.value !== 'select' || selected.value.size) return
  // 默认从右键那一句往后共 5 行：歌词是往前写的，往前选不符合"分享正在唱的这段"的直觉
  const anchor = props.initialIndex >= 0 ? props.initialIndex : 0
  const picked: string[] = []
  const initial: number[] = []
  for (const line of props.lines) {
    if (initial.length >= DEFAULT_SELECTED_LINES) break
    if (line.index < anchor || !line.text.trim()) continue
    if (!fitsBudget([...picked, line.text])) break
    picked.push(line.text)
    initial.push(line.index)
  }
  if (!initial.length) {
    // 连右键那句本身都排不进预算时不预选，同时把上限提示顶出来
    rejectedAdd.value = true
  }
  selected.value = new Set(initial)
  void scrollToAnchor()
})

/** 滚动定位的锚点：已选中的第一句，清空选择时退回右键那一句 */
const anchorIndex = computed(() => (selected.value.size ? Math.min(...selected.value) : props.initialIndex))

/**
 * 把锚点那句滚到列表顶部（打开时、以及从预览返回勾选页时）。
 * 用 offsetTop 而不是 getBoundingClientRect 差值：弹窗入场动画带 scale(0.9)，rect 会被整体缩掉一成；
 * 也因此 .lsd-list 要 position: relative，否则 offsetParent 落到 body 上，offsetTop 不是列表内坐标
 */
async function scrollToAnchor() {
  await nextTick()
  const list = listRef.value
  const item = list?.querySelector<HTMLElement>(`[data-line="${anchorIndex.value}"]`)
  if (!list || !item) return
  list.scrollTop = Math.max(0, item.offsetTop - 16)
  item.querySelector<HTMLElement>('.lsd-check')?.focus({ preventScroll: true })
}

watch([() => props.open, () => stage.value], ([open, currentStage]) => {
  if (open && currentStage === 'select') void scrollToAnchor()
})

function close() {
  emit('update:open', false)
}

useEscapeClose(() => props.open, close)

function toggleLine(index: number, event?: Event) {
  const next = new Set(selected.value)
  if (next.has(index)) {
    next.delete(index)
    rejectedAdd.value = false
  } else {
    // 按"加上这一句之后"的集合试算：一句长歌词可能占两三个视觉行，
    // 只看当前选择会放行到 25/24，海报就会超出书面承诺的上限
    const proposed = new Set(next)
    proposed.add(index)
    if (!fitsBudget(textsOf(proposed))) {
      rejectedAdd.value = true
      // 浏览器在 change 之前就把勾选框翻上了，这里必须按模型退回，
      // 否则会出现"框勾着、行没高亮、选择里也没有"的脱钩
      const input = event?.target as HTMLInputElement | null
      if (input) input.checked = selected.value.has(index)
      return
    }
    next.add(index)
    rejectedAdd.value = false
  }
  selected.value = next
  poster.value = null
}

/** 全选只取放得下的前若干句：碰到第一句装不下的就停，保持选段连续 */
function selectAll() {
  const picked: string[] = []
  const next = new Set<number>()
  for (const line of props.lines) {
    if (!line.text.trim()) continue
    // 第一句也要试算：整首歌词被存成一句超长文本时，短路会让它直接被选进去、出图又被截断
    if (!fitsBudget([...picked, line.text])) break
    picked.push(line.text)
    next.add(line.index)
  }
  selected.value = next
  rejectedAdd.value = false
  poster.value = null
}

function clearSelection() {
  selected.value = new Set()
  rejectedAdd.value = false
  poster.value = null
}

async function copySelectedText() {
  const text = selectedLines.value.join('\n')
  if (!text) return
  try {
    await writeText(text)
    toast.success(t('player.copied'))
  } catch (error) {
    log.warn('copy selected lyrics failed:', summarizeLogError(error))
    toast.error(t('player.copy_failed'))
  }
}

async function generatePoster() {
  if (!selectedLines.value.length || busy.value) return
  busy.value = true
  const generation = ++renderGeneration
  // 出图内容在点击这一刻定稿：封面取图要 await，期间切歌或改勾选都不能把
  // 新曲目的歌名歌手拼到旧歌词上，也不能让成图和按钮提交时的选择不一致
  const snapshot = {
    lines: selectedLines.value.slice(),
    title: title.value,
    artist: artist.value,
  }
  let coverSource: string | null = coverUrl.value
  let objectUrl = ''
  try {
    // 封面按原图取字节再转 blob：图床防盗链的直链画进画布会污染，导出取不到像素
    if (coverSource) {
      try {
        const { bytes } = await readImageBytes(coverSource)
        const blob = new Blob([bytes as unknown as BlobPart], { type: 'image/*' })
        objectUrl = URL.createObjectURL(blob)
        coverSource = objectUrl
      } catch (error) {
        log.warn('cover fetch for poster failed:', summarizeLogError(error))
        coverSource = null
      }
    }
    if (generation !== renderGeneration) return
    const rendered = await renderPoster(
      { coverUrl: coverSource, lines: snapshot.lines, title: snapshot.title, artist: snapshot.artist },
      await loadPosterLogo(),
    )
    // 期间弹窗被关掉又重开过（或又点了一次生成）：这一版结果属于旧会话，整张丢掉
    if (generation !== renderGeneration) return
    poster.value = rendered
    stage.value = 'preview'
    await nextTick()
    const host = previewRef.value
    if (host && rendered.canvas !== host) {
      host.width = rendered.canvas.width
      host.height = rendered.canvas.height
      host.getContext('2d')?.drawImage(rendered.canvas, 0, 0)
    }
  } catch (error) {
    log.error('poster render failed:', summarizeLogError(error))
    toast.error(t('lyrics.poster_render_failed'))
  } finally {
    if (objectUrl) URL.revokeObjectURL(objectUrl)
    // 只由当前会话释放 busy：旧会话收尾时新会话可能正在渲染，覆盖会把新会话的进度打掉
    if (generation === renderGeneration) busy.value = false
  }
}

function backToSelect() {
  stage.value = 'select'
  poster.value = null
}

async function exportPoster(): Promise<{ bytes: Uint8Array } | null> {
  if (!poster.value) return null
  try {
    const blob = await posterToBlob(poster.value.canvas)
    return { bytes: new Uint8Array(await blob.arrayBuffer()) }
  } catch (error) {
    log.error('poster encode failed:', summarizeLogError(error))
    toast.error(t('lyrics.poster_render_failed'))
    return null
  }
}

async function savePoster() {
  if (!poster.value || busy.value) return
  // 先占住 busy：PNG 编码要几百毫秒，空着这段双击就会弹两个保存对话框
  busy.value = true
  try {
    const exported = await exportPoster()
    if (!exported) return
    const saved = await saveImageWithDialog(exported.bytes, safeFileName(`${title.value}-${artist.value}`, 'lyrics'), 'png')
    if (saved) toast.success(t('lyrics.poster_saved'))
  } catch (error) {
    log.error('poster save failed:', summarizeLogError(error))
    toast.error(t('lyrics.poster_save_failed'))
  } finally {
    busy.value = false
  }
}

async function copyPosterImage() {
  if (!poster.value || busy.value) return
  busy.value = true
  try {
    await writePosterToClipboard(poster.value.canvas)
    toast.success(t('lyrics.poster_copied'))
  } catch (error) {
    // 剪贴板写图片失败时给出可执行的替代路径（保存成文件）
    log.warn('poster clipboard copy failed:', summarizeLogError(error))
    toast.error(t('lyrics.poster_copy_failed'))
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition name="lsd">
      <div v-if="open" class="lsd-overlay" @click="close">
        <div class="lsd-dialog" role="dialog" aria-modal="true" :aria-label="t('lyrics.select_lines')" @click.stop>
          <div class="lsd-header">
            <span class="material-symbols-rounded lsd-icon">lyrics</span>
            <h3 class="lsd-title">{{ stage === 'select' ? t('lyrics.select_lines') : t('lyrics.poster_preview') }}</h3>
            <button class="lsd-close" :title="t('common.close')" :aria-label="t('common.close')" @click="close">
              <span class="material-symbols-rounded">close</span>
            </button>
          </div>

          <div v-if="stage === 'select'" class="lsd-body">
            <div class="lsd-toolbar">
              <span class="lsd-count">{{ t('lyrics.selected_count', { count: selectedCount, visual: visualLines, max: MAX_POSTER_VISUAL_LINES }) }}</span>
              <button class="lsd-link" @click="selectAll">{{ t('lyrics.select_all') }}</button>
              <button class="lsd-link" :disabled="!selectedCount" @click="clearSelection">{{ t('lyrics.clear_selection') }}</button>
            </div>
            <div ref="listRef" class="lsd-list" role="list">
              <label
                v-for="line in lines"
                :key="line.index"
                class="lsd-item"
                :class="{ checked: selected.has(line.index), empty: !line.text.trim() }"
                :data-line="line.index"
                role="listitem"
              >
                <input
                  type="checkbox"
                  class="lsd-check"
                  :checked="selected.has(line.index)"
                  :disabled="!line.text.trim() || busy"
                  @change="toggleLine(line.index, $event)"
                />
                <span class="lsd-text">{{ line.text }}</span>
              </label>
            </div>
            <p v-if="limitReached" class="lsd-hint">{{ t('lyrics.line_limit_hint', { max: MAX_POSTER_VISUAL_LINES }) }}</p>
          </div>

          <div v-else class="lsd-body lsd-preview-body">
            <canvas ref="previewRef" class="lsd-preview" :aria-label="t('lyrics.poster_preview')" />
            <p v-if="poster && poster.plan.dropped" class="lsd-hint">{{ t('lyrics.poster_truncated', { count: poster.plan.dropped }) }}</p>
            <p v-if="poster && (poster.plan.titleTruncated || poster.plan.artistTruncated)" class="lsd-hint">
              {{ t('lyrics.poster_title_truncated') }}
            </p>
          </div>

          <div class="lsd-actions">
            <button class="lsd-btn text" @click="close">{{ t('common.cancel') }}</button>
            <template v-if="stage === 'select'">
              <button class="lsd-btn text" :disabled="!selectedCount || busy" @click="copySelectedText">
                {{ t('lyrics.copy_selected') }}
              </button>
              <button class="lsd-btn filled" :disabled="!canSubmit" @click="generatePoster">
                {{ busy ? t('lyrics.poster_generating') : t('lyrics.generate_poster') }}
              </button>
            </template>
            <template v-else>
              <button class="lsd-btn text" @click="backToSelect">{{ t('lyrics.back_to_select') }}</button>
              <button class="lsd-btn text" :disabled="busy" @click="copyPosterImage">{{ t('lyrics.copy_image') }}</button>
              <button class="lsd-btn filled" :disabled="busy" @click="savePoster">{{ t('lyrics.save_image') }}</button>
            </template>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped lang="scss">
.lsd-overlay {
  position: fixed;
  inset: 0;
  z-index: 9000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(4px);
}

.lsd-dialog {
  display: flex;
  flex-direction: column;
  width: min(680px, calc(100vw - 48px));
  max-height: calc(100vh - 48px);
  padding: 24px;
  border-radius: 28px;
  background: var(--md-surface-container-high);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.35), 0 2px 8px rgba(0, 0, 0, 0.2);
}

.lsd-header {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 8px;
  margin-bottom: 16px;
}

.lsd-icon {
  font-size: 24px;
  color: var(--md-primary);
}

.lsd-title {
  flex: 1;
  min-width: 0;
  margin: 0;
  font-size: 20px;
  font-weight: 600;
  line-height: 1.3;
  color: var(--md-on-surface);
}

.lsd-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  border: none;
  border-radius: 20px;
  background: none;
  color: var(--md-on-surface-variant);
  cursor: pointer;

  &:hover { background: rgba(255, 255, 255, 0.08); }
}

.lsd-body {
  min-height: 0;
  display: flex;
  flex-direction: column;
  margin-bottom: 16px;
}

.lsd-toolbar {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 16px;
  margin-bottom: 8px;
  font-size: 13px;
  color: var(--md-on-surface-variant);
}

.lsd-count { flex: 1; }

.lsd-link {
  border: none;
  padding: 8px;
  border-radius: 8px;
  background: none;
  color: var(--md-primary);
  font-size: 13px;
  cursor: pointer;

  &:hover:not(:disabled) { background: rgba(208, 188, 255, 0.08); }
  &:disabled { opacity: 0.38; cursor: default; }
}

.lsd-list {
  position: relative;
  flex: 1;
  min-height: 200px;
  max-height: 44vh;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.lsd-item {
  display: flex;
  align-items: baseline;
  gap: 12px;
  padding: 8px 12px;
  border-radius: 12px;
  cursor: pointer;

  &:hover { background: rgba(255, 255, 255, 0.05); }
  &.checked { background: rgba(208, 188, 255, 0.1); }
  &.empty { opacity: 0.35; cursor: default; }

  .lsd-text {
    min-width: 0;
    font-size: 15px;
    line-height: 1.5;
    color: var(--md-on-surface);
  }
}

.lsd-check {
  flex: 0 0 auto;
  width: 18px;
  height: 18px;
  accent-color: var(--md-primary);
  cursor: pointer;
}

.lsd-preview-body {
  align-items: center;
  gap: 8px;
}

/* 预览按 3:4 上限缩放，实际导出仍是 1080 宽的完整画布 */
.lsd-preview {
  max-width: 100%;
  max-height: 52vh;
  width: auto;
  height: auto;
  border-radius: 16px;
  background: #000;
  object-fit: contain;
}

.lsd-hint {
  margin: 0;
  font-size: 12px;
  line-height: 1.5;
  color: var(--md-on-surface-variant);
}

.lsd-actions {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
}

.lsd-btn {
  height: 40px;
  padding: 0 24px;
  border: none;
  border-radius: 20px;
  font-family: inherit;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: background 200ms, opacity 200ms;

  &.text {
    background: none;
    color: var(--md-primary);

    &:hover:not(:disabled) { background: rgba(208, 188, 255, 0.08); }
    &:active:not(:disabled) { background: rgba(208, 188, 255, 0.12); }
  }

  &.filled {
    background: var(--md-primary);
    color: var(--md-on-primary);

    &:hover:not(:disabled) { box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3); }
    &:active:not(:disabled) { opacity: 0.9; }
  }

  &:disabled {
    opacity: 0.38;
    pointer-events: none;
  }
}

.lsd-enter-active {
  transition: opacity 200ms ease-out;

  .lsd-dialog {
    transition: transform 300ms cubic-bezier(0.05, 0.7, 0.1, 1), opacity 200ms;
  }
}

.lsd-leave-active {
  transition: opacity 150ms ease-in;

  .lsd-dialog {
    transition: transform 200ms cubic-bezier(0.3, 0, 0.8, 0.15), opacity 200ms;
  }
}

.lsd-enter-from,
.lsd-leave-to {
  opacity: 0;

  .lsd-dialog {
    transform: scale(0.9);
    opacity: 0;
  }
}
</style>
