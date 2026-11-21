<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { usePlayerStore, type TrackInfo } from '@/stores/player'
import { useBiliVideoSkipStore } from '@/stores/biliVideoSkip'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { useToastStore } from '@/stores/toast'
import {
  formatBiliSkipTime,
  MAX_BILI_VIDEO_SKIP_RULES,
  normalizeBiliSkipIntervals,
  parseBiliSkipTime,
  resolveBiliVideoSkipBvid,
  resolveBiliVideoSkipTarget,
  type BiliVideoSkipInterval,
  type BiliVideoSkipTarget,
} from '@/modules/playback/biliVideoSkip'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import M3Input from '@/components/ui/M3Input.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'

const props = defineProps<{ open: boolean; track: TrackInfo | null }>()
const emit = defineEmits<{ 'update:open': [value: boolean] }>()
const { t } = useI18n()
const player = usePlayerStore()
const skipStore = useBiliVideoSkipStore()
const listenTogether = useListenTogetherStore()
const toast = useToastStore()

interface TargetOption extends BiliVideoSkipTarget {
  label: string
  durationMs: number
}
interface IntervalDraft {
  intervals: BiliVideoSkipInterval[]
  startText: string
  endText: string
  dirty: boolean
  inputError: string
}
interface InputDraft extends BiliVideoSkipTarget {
  startText: string
  endText: string
  modifiedAt: number
}
const INPUT_DRAFT_STORAGE_KEY = 'neriplayer-bili-video-skip-input-drafts'
const MAX_INPUT_DRAFT_TEXT_LENGTH = 128

function readInputDrafts(): InputDraft[] {
  try {
    const document = JSON.parse(localStorage.getItem(INPUT_DRAFT_STORAGE_KEY) || '{}')
    if (!Array.isArray(document?.drafts)) return []
    const byTarget = new Map<string, InputDraft>()
    for (const raw of document.drafts) {
      if (!raw || typeof raw.bvid !== 'string' || !raw.bvid.trim() || !Number.isSafeInteger(raw.cid) || raw.cid <= 0) continue
      const draft: InputDraft = {
        bvid: raw.bvid.trim(), cid: raw.cid,
        startText: typeof raw.startText === 'string' ? raw.startText.trim().slice(0, MAX_INPUT_DRAFT_TEXT_LENGTH) : '',
        endText: typeof raw.endText === 'string' ? raw.endText.trim().slice(0, MAX_INPUT_DRAFT_TEXT_LENGTH) : '',
        modifiedAt: Number.isFinite(raw.modifiedAt) ? Math.max(0, Math.round(raw.modifiedAt)) : 0,
      }
      if (!draft.startText && !draft.endText) continue
      const key = `${draft.bvid}|${draft.cid}`
      const previous = byTarget.get(key)
      if (!previous || draft.modifiedAt >= previous.modifiedAt) byTarget.set(key, draft)
    }
    return [...byTarget.values()]
      .sort((left, right) => (left.bvid < right.bvid ? -1 : left.bvid > right.bvid ? 1 : 0) || left.cid - right.cid)
      .slice(0, MAX_BILI_VIDEO_SKIP_RULES)
  } catch {
    return []
  }
}

function persistInputDraft(key: string, start: string, end: string) {
  const separator = key.lastIndexOf('|')
  const bvid = key.slice(0, separator)
  const cid = Number(key.slice(separator + 1))
  if (separator <= 0 || !Number.isSafeInteger(cid) || cid <= 0) return
  try {
    const currentDrafts = readInputDrafts()
    const previous = currentDrafts.find(draft => draft.bvid === bvid && draft.cid === cid)
    const startText = start.trim().slice(0, MAX_INPUT_DRAFT_TEXT_LENGTH)
    const endText = end.trim().slice(0, MAX_INPUT_DRAFT_TEXT_LENGTH)
    if (previous?.startText === startText && previous.endText === endText) return
    const next = currentDrafts.filter(draft => draft.bvid !== bvid || draft.cid !== cid)
    if (startText || endText) next.push({ bvid, cid, startText, endText, modifiedAt: Math.max(Date.now(), (previous?.modifiedAt ?? 0) + 1) })
    next.sort((left, right) => (left.bvid < right.bvid ? -1 : left.bvid > right.bvid ? 1 : 0) || left.cid - right.cid)
    localStorage.setItem(INPUT_DRAFT_STORAGE_KEY, JSON.stringify({ version: 1, drafts: next.slice(0, MAX_BILI_VIDEO_SKIP_RULES) }))
  } catch {
    // 输入草稿不可用时仍允许用户编辑和保存正式规则
  }
}

const targetOptions = ref<TargetOption[]>([])
const selectedKey = ref('')
const loading = ref(false)
const saving = ref(false)
const loadError = ref('')
const inputError = ref('')
const draftIntervals = ref<BiliVideoSkipInterval[]>([])
const startText = ref('')
const endText = ref('')
const hasLocalEdits = ref(false)
const pendingDeletion = ref<number | null>(null)
const showClearConfirmation = ref(false)
const drafts = new Map<string, IntervalDraft>()
let sessionGeneration = 0
let loadGeneration = 0

const targetKey = (target: BiliVideoSkipTarget) => `${target.bvid}|${target.cid}`
const selectedTarget = computed(() => targetOptions.value.find(target => targetKey(target) === selectedKey.value) ?? null)
const selectOptions = computed(() => targetOptions.value.map(target => ({
  value: targetKey(target),
  label: `${target.label}${target.durationMs > 0 ? ` · ${formatBiliSkipTime(target.durationMs)}` : ''}`,
})))
const matchesPlayback = computed(() => {
  const target = selectedTarget.value
  const current = player.currentBiliVideoSkipTarget
  return !!player.currentTrack && !!target && !!current && target.bvid === current.bvid && target.cid === current.cid
})
const canControlPlayback = computed(() => matchesPlayback.value && !player.isLoadingAudio && !listenTogether.localControlRestriction)
const playbackPosition = computed(() => Math.max(0, Math.round(
  Number.isFinite(player.interpolatedPositionMs) ? player.interpolatedPositionMs : player.positionMs || 0,
)))

function explicitTargetFor(track: TrackInfo): BiliVideoSkipTarget | null {
  return resolveBiliVideoSkipTarget(track)
    ?? (track.id === player.currentTrack?.id ? player.currentBiliVideoSkipTarget : null)
}

function rememberDraft(key: string) {
  if (!key) return
  persistInputDraft(key, startText.value, endText.value)
  drafts.set(key, {
    intervals: draftIntervals.value.map(interval => ({ ...interval })),
    startText: startText.value,
    endText: endText.value,
    dirty: hasLocalEdits.value,
    inputError: inputError.value,
  })
}

function loadSelectedDraft() {
  const target = selectedTarget.value
  const draft = drafts.get(selectedKey.value)
  const inputDraft = readInputDrafts().find(value => targetKey(value) === selectedKey.value)
  const rule = target ? skipStore.ruleFor(target) : null
  draftIntervals.value = (draft?.dirty ? draft.intervals : rule?.isDeleted ? [] : rule?.intervals ?? [])
    .map(interval => ({ ...interval }))
  startText.value = draft?.startText ?? inputDraft?.startText ?? ''
  endText.value = draft?.endText ?? inputDraft?.endText ?? ''
  hasLocalEdits.value = draft?.dirty ?? false
  inputError.value = draft?.inputError ?? ''
  pendingDeletion.value = null
  showClearConfirmation.value = false
}

watch(selectedKey, (_key, previous) => {
  rememberDraft(previous)
  loadSelectedDraft()
}, { flush: 'sync' })

watch([startText, endText], () => {
  if (props.open && selectedTarget.value) persistInputDraft(selectedKey.value, startText.value, endText.value)
}, { flush: 'post' })

watch(() => skipStore.rules, () => {
  if (!props.open || hasLocalEdits.value || !selectedTarget.value) return
  const rule = skipStore.ruleFor(selectedTarget.value)
  draftIntervals.value = (rule?.isDeleted ? [] : rule?.intervals ?? []).map(interval => ({ ...interval }))
}, { deep: true })

watch(() => [props.open, props.track] as const, ([open]) => {
  sessionGeneration++
  loadGeneration++
  selectedKey.value = ''
  drafts.clear()
  targetOptions.value = []
  draftIntervals.value = []
  startText.value = ''
  endText.value = ''
  inputError.value = ''
  loadError.value = ''
  loading.value = false
  saving.value = false
  hasLocalEdits.value = false
  pendingDeletion.value = null
  showClearConfirmation.value = false
  if (open) void loadTargets()
}, { immediate: true })

async function loadTargets() {
  const track = props.track
  const bvid = track ? resolveBiliVideoSkipBvid(track) : null
  if (!props.open || saving.value) return
  if (!track || !bvid) {
    loadError.value = t('player.bili_skip_identity_missing')
    loading.value = false
    return
  }
  const request = ++loadGeneration
  const session = sessionGeneration
  const fallback = explicitTargetFor(track)
  const isCurrent = () => props.open && session === sessionGeneration && request === loadGeneration
  loading.value = true
  loadError.value = ''
  try {
    const result = await invoke<TargetOption[]>('get_bili_video_skip_targets', { bvid })
    if (!isCurrent()) return
    const seen = new Set<string>()
    const options = result.filter(option => {
      if (option.bvid !== bvid || !Number.isSafeInteger(option.cid) || option.cid <= 0) return false
      const key = targetKey(option)
      if (seen.has(key)) return false
      seen.add(key)
      return true
    }).map((option, index) => ({
      ...option,
      label: option.label?.trim() || `P${index + 1}`,
      durationMs: Number.isFinite(option.durationMs) ? Math.max(0, Math.round(option.durationMs)) : 0,
    }))
    if (!options.length) throw new Error('No video parts available')
    targetOptions.value = options
    if (!options.some(option => targetKey(option) === selectedKey.value)) {
      selectedKey.value = fallback && options.some(option => targetKey(option) === targetKey(fallback))
        ? targetKey(fallback)
        : targetKey(options[0]!)
    }
  } catch {
    if (!isCurrent()) return
    loadError.value = t('player.bili_skip_load_failed')
    if (!targetOptions.value.length && fallback?.bvid === bvid) {
      const durationMs = track.id === player.currentTrack?.id && player.durationMs > 0
        ? player.durationMs
        : track.durationMs
      targetOptions.value = [{ ...fallback, label: track.title || bvid, durationMs: Math.max(0, durationMs || 0) }]
      selectedKey.value = targetKey(fallback)
    }
  } finally {
    if (isCurrent()) loading.value = false
  }
}

function addInterval() {
  const target = selectedTarget.value
  if (!target || saving.value) return
  const startMs = parseBiliSkipTime(startText.value)
  const endMs = parseBiliSkipTime(endText.value)
  inputError.value = startMs === null || endMs === null
    ? t('player.bili_skip_invalid_time')
    : endMs <= startMs
      ? t('player.bili_skip_invalid_range')
      : target.durationMs > 0 && endMs > target.durationMs
        ? t('player.bili_skip_exceeds_duration')
        : ''
  if (inputError.value || startMs === null || endMs === null) return
  draftIntervals.value = normalizeBiliSkipIntervals([...draftIntervals.value, { startMs, endMs }], target.durationMs)
  hasLocalEdits.value = true
  startText.value = ''
  endText.value = ''
}

function setCurrentTime(field: 'start' | 'end') {
  if (!matchesPlayback.value || saving.value) return
  if (field === 'start') startText.value = formatBiliSkipTime(playbackPosition.value)
  else endText.value = formatBiliSkipTime(playbackPosition.value)
  inputError.value = ''
}

async function movePlayback(deltaMs: number) {
  if (!canControlPlayback.value || saving.value) return
  const durationMs = player.durationMs || selectedTarget.value?.durationMs || 0
  const positionMs = Math.max(0, playbackPosition.value + deltaMs)
  await player.seekTo(durationMs > 0 ? Math.min(durationMs, positionMs) : positionMs)
}

async function togglePlayback() {
  if (canControlPlayback.value && !saving.value) await player.togglePlayPause()
}

function confirmDelete() {
  if (pendingDeletion.value === null || saving.value) return
  draftIntervals.value = draftIntervals.value.filter((_interval, index) => index !== pendingDeletion.value)
  hasLocalEdits.value = true
  pendingDeletion.value = null
}

function confirmClear() {
  if (saving.value) return
  draftIntervals.value = []
  hasLocalEdits.value = true
  showClearConfirmation.value = false
}

async function saveIntervals() {
  const target = selectedTarget.value
  if (!props.open || !target || loading.value || saving.value) return
  const session = sessionGeneration
  const intervals = draftIntervals.value.map(interval => ({ ...interval }))
  saving.value = true
  inputError.value = ''
  try {
    await skipStore.setIntervals({ bvid: target.bvid, cid: target.cid }, intervals, target.durationMs)
    if (props.open && session === sessionGeneration) {
      toast.success(t('player.bili_skip_saved'))
      emit('update:open', false)
    }
  } catch {
    if (props.open && session === sessionGeneration) inputError.value = t('player.bili_skip_save_failed')
  } finally {
    if (session === sessionGeneration) saving.value = false
  }
}

function close() {
  rememberDraft(selectedKey.value)
  emit('update:open', false)
}
onUnmounted(() => { rememberDraft(selectedKey.value); sessionGeneration++; loadGeneration++ })
</script>

<template>
  <M3Dialog
    :open="open"
    :title="t('player.bili_skip_title')"
    icon="skip_next"
    :confirm-text="saving ? t('common.loading') : t('common.save')"
    :confirm-disabled="loading || saving || !selectedTarget"
    @update:open="close"
    @confirm="saveIntervals"
  >
    <div class="bili-skip-content">
      <p class="track-title">{{ track?.title }}</p>
      <p class="skip-hint">{{ t('player.bili_skip_hint') }}</p>
      <div v-if="loading" class="load-status" role="status">{{ t('common.loading') }}</div>
      <div v-if="loadError" class="load-status" role="alert">
        <span>{{ loadError }}</span>
        <button type="button" class="skip-button subtle" :disabled="loading || saving" @click="loadTargets">{{ t('player.retry') }}</button>
      </div>
      <fieldset v-if="selectedTarget" class="skip-editor" :disabled="saving">
        <label class="part-label" for="bili-skip-part">{{ t('player.bili_skip_part') }}</label>
        <CustomSelect id="bili-skip-part" v-model="selectedKey" :options="selectOptions" :label="t('player.bili_skip_part')" :disabled="loading || saving" />
        <div class="time-inputs">
          <M3Input v-model="startText" :label="t('player.bili_skip_start')" placeholder="00:10" :maxlength="128" @enter="addInterval" />
          <M3Input v-model="endText" :label="t('player.bili_skip_end')" placeholder="00:20" :maxlength="128" @enter="addInterval" />
        </div>
        <div v-if="matchesPlayback" class="current-time-buttons">
          <button type="button" class="skip-button subtle" @click="setCurrentTime('start')">{{ t('player.bili_skip_set_start') }}</button>
          <span class="current-time">{{ formatBiliSkipTime(playbackPosition) }}</span>
          <button type="button" class="skip-button subtle" @click="setCurrentTime('end')">{{ t('player.bili_skip_set_end') }}</button>
        </div>
        <div v-if="matchesPlayback" class="playback-buttons">
          <button v-for="step in [-5000, -1000]" :key="step" type="button" class="skip-button subtle" :disabled="!canControlPlayback" :aria-label="t('player.bili_skip_seek', { seconds: step / 1000 })" @click="movePlayback(step)">{{ step / 1000 }}s</button>
          <button type="button" class="skip-button subtle playback-toggle" :disabled="!canControlPlayback" :aria-label="t(player.isPlaying ? 'player.bili_skip_pause' : 'player.bili_skip_play')" @click="togglePlayback">
            <span class="material-symbols-rounded" aria-hidden="true">{{ player.isPlaying ? 'pause' : 'play_arrow' }}</span>
          </button>
          <button v-for="step in [1000, 5000]" :key="step" type="button" class="skip-button subtle" :disabled="!canControlPlayback" :aria-label="t('player.bili_skip_seek', { seconds: step / 1000 })" @click="movePlayback(step)">+{{ step / 1000 }}s</button>
        </div>
        <p v-else class="skip-hint">{{ t('player.bili_skip_other_part') }}</p>
        <button type="button" class="skip-button add-button" @click="addInterval">{{ t('player.bili_skip_add') }}</button>
        <p v-if="inputError" class="input-error" role="alert">{{ inputError }}</p>
        <p v-if="!draftIntervals.length" class="empty-intervals">{{ t('player.bili_skip_empty') }}</p>
        <ul v-else class="interval-list">
          <li v-for="(interval, index) in draftIntervals" :key="`${interval.startMs}:${interval.endMs}`">
            <span>{{ formatBiliSkipTime(interval.startMs) }} – {{ formatBiliSkipTime(interval.endMs) }}</span>
            <button type="button" class="skip-button subtle delete-button" :aria-label="t('common.delete')" @click="pendingDeletion = index"><span class="material-symbols-rounded" aria-hidden="true">delete</span></button>
          </li>
        </ul>
        <button type="button" class="skip-button subtle clear-button" :disabled="!draftIntervals.length" @click="showClearConfirmation = true">{{ t('player.bili_skip_clear') }}</button>
      </fieldset>
    </div>
  </M3Dialog>
  <M3Dialog :open="open && pendingDeletion !== null" :title="t('common.delete')" :confirm-text="t('common.delete')" :confirm-danger="true" @update:open="pendingDeletion = null" @confirm="confirmDelete">{{ t('player.bili_skip_delete_confirm') }}</M3Dialog>
  <M3Dialog :open="open && showClearConfirmation" :title="t('player.bili_skip_clear')" :confirm-text="t('player.bili_skip_clear')" :confirm-danger="true" @update:open="showClearConfirmation = false" @confirm="confirmClear">{{ t('player.bili_skip_clear_confirm') }}</M3Dialog>
</template>

<style scoped lang="scss">
.bili-skip-content { max-height: min(65vh, 600px); overflow-y: auto; padding-right: 4px; }
.track-title { color: var(--md-on-surface); font-weight: 600; overflow-wrap: anywhere; margin: 0 0 8px; }
.skip-hint { font-size: 12px; margin: 8px 0 12px; }
.load-status { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin: 12px 0; }
.skip-editor { min-width: 0; border: 0; padding: 0; margin: 0; }
.part-label { display: block; font-size: 12px; margin-bottom: 6px; }
.time-inputs { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin: 8px 0; }
.current-time-buttons, .playback-buttons { display: flex; align-items: center; justify-content: space-between; gap: 4px; margin: 4px 0; }
.current-time { font-variant-numeric: tabular-nums; }
.skip-button {
  border: 0; border-radius: 20px; min-height: 36px; padding: 8px 12px; background: var(--md-primary); color: var(--md-on-primary); font: inherit; cursor: pointer;
  &.subtle { background: transparent; color: var(--md-primary); }
  &:hover:not(:disabled) { opacity: 0.8; }
  &:disabled { opacity: 0.38; cursor: default; }
}
.playback-toggle, .delete-button { display: inline-flex; align-items: center; justify-content: center; }
.playback-buttons .skip-button { padding-inline: 10px; }
.add-button { width: 100%; margin: 10px 0; }
.input-error { color: var(--md-error); margin: 6px 0; font-size: 12px; }
.empty-intervals { text-align: center; margin: 18px 0; }
.interval-list { padding: 0; margin: 4px 0; list-style: none; }
.interval-list li { display: flex; align-items: center; justify-content: space-between; min-height: 44px; border-bottom: 1px solid var(--md-outline-variant); font-variant-numeric: tabular-nums; }
.clear-button { display: block; margin: 8px 0 0 auto; }
</style>
