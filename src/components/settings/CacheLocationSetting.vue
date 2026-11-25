<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import { useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import {
  cacheLocationErrorKey,
  getMediaCacheLocation,
  openMediaCacheDir,
  pickMediaCacheDir,
  resetMediaCacheDir,
  setMediaCacheDir,
  type MediaCacheLocation,
  type MediaCacheRelocation,
} from '@/modules/settings/cacheLocation'
import { formatStorageSize } from '@/utils/storage'
import { createLogger } from '@/utils/logger'

const log = createLogger('settings-view')
const { t } = useI18n()
const toast = useToastStore()
const settings = useSettingsStore()

const location = ref<MediaCacheLocation | null>(null)
const busy = ref(false)
// 待确认的目标目录；null 表示恢复默认
const pendingTarget = ref<string | null | undefined>(undefined)
const moveExisting = ref(true)

const confirmOpen = computed({
  get: () => pendingTarget.value !== undefined,
  set: (open: boolean) => { if (!open) pendingTarget.value = undefined },
})
const displayPath = computed(() => location.value?.path || '...')
const pendingDisplay = computed(() => pendingTarget.value ?? location.value?.defaultPath ?? '')

async function loadLocation() {
  try {
    location.value = await getMediaCacheLocation()
  } catch (error) {
    log.error('Failed to read media cache dir:', error)
  }
}

async function chooseDirectory() {
  try {
    const picked = await pickMediaCacheDir(location.value?.path)
    if (!picked || picked === location.value?.path) return
    moveExisting.value = true
    pendingTarget.value = picked
  } catch (error) {
    log.error('Failed to pick media cache dir:', error)
    toast.error(t(cacheLocationErrorKey(error)))
  }
}

function askReset() {
  moveExisting.value = true
  pendingTarget.value = null
}

async function applyChange() {
  const target = pendingTarget.value
  if (target === undefined || busy.value) return
  pendingTarget.value = undefined
  busy.value = true
  const existing = moveExisting.value ? 'move' : 'discard'
  try {
    const result = target === null
      ? await resetMediaCacheDir(existing)
      : await setMediaCacheDir(target, existing)
    location.value = result.location
    // 后端已保存；同步到设置 store，免得之后的普通保存带着旧值（后端也会忽略它）
    settings.mediaCacheDir = result.location.customPath
    notifyResult(result)
  } catch (error) {
    log.error('Failed to change media cache dir:', error)
    toast.error(t(cacheLocationErrorKey(error)))
  } finally {
    busy.value = false
  }
}

function notifyResult(result: MediaCacheRelocation) {
  if (result.failedCount > 0) {
    toast.show(t('settings.cache_location_partial', { count: result.failedCount }), 'info')
  } else if (result.movedFiles > 0) {
    toast.success(t('settings.cache_location_moved', { size: formatStorageSize(result.movedBytes) }))
  } else {
    toast.success(t('settings.cache_location_changed'))
  }
}

async function openDirectory() {
  try {
    await openMediaCacheDir()
  } catch (error) {
    log.error('Failed to open media cache dir:', error)
    toast.error(t('settings.cache_location_open_failed'))
  }
}

onMounted(loadLocation)
</script>

<template>
  <div class="setting-card cache-location-card">
    <div class="setting-icon-wrap"><span class="material-symbols-rounded">folder_open</span></div>
    <div class="setting-info">
      <div class="setting-title">{{ t('settings.cache_location') }}</div>
      <div class="setting-desc cache-location-path" :title="displayPath">{{ displayPath }}</div>
      <div class="setting-desc">{{ busy ? t('settings.cache_location_moving') : t('settings.cache_location_desc') }}</div>
    </div>
    <div class="chip-row">
      <button class="m3-chip sm" type="button" :disabled="busy" @click="chooseDirectory">{{ t('settings.cache_location_change') }}</button>
      <button class="m3-chip sm" type="button" :disabled="busy" @click="openDirectory">{{ t('settings.cache_location_open') }}</button>
      <button v-if="location && !location.isDefault" class="m3-chip sm" type="button" :disabled="busy" @click="askReset">{{ t('settings.cache_location_reset') }}</button>
    </div>
  </div>

  <M3Dialog
    v-model:open="confirmOpen"
    :title="t('settings.cache_location_confirm_title')"
    icon="drive_file_move"
    :confirm-text="t('common.confirm')"
    :confirm-danger="!moveExisting"
    @confirm="applyChange"
  >
    <div class="cache-location-dialog">
      <p>{{ t('settings.cache_location_confirm_target') }}</p>
      <code class="cache-location-target">{{ pendingDisplay }}</code>
      <button
        class="cache-location-check"
        type="button"
        role="checkbox"
        :aria-checked="moveExisting"
        @click="moveExisting = !moveExisting"
      >
        <span class="cache-location-checkbox" :class="{ checked: moveExisting }" aria-hidden="true">
          <span v-if="moveExisting" class="material-symbols-rounded">check</span>
        </span>
        <span class="cache-location-check-copy">
          <strong>{{ t('settings.cache_location_move_existing') }}</strong>
          <small>{{ moveExisting ? t('settings.cache_location_move_existing_desc') : t('settings.cache_location_discard_desc') }}</small>
        </span>
      </button>
    </div>
  </M3Dialog>
</template>

<style scoped lang="scss">
@use '@/components/settings/settingsPrimitives.scss' as primitives;

@include primitives.card;
@include primitives.card-body;
@include primitives.card-text;
@include primitives.chip;

.chip-row {
  display: flex;
  flex-wrap: wrap;
  flex-shrink: 0;
  gap: 6px;
}

.m3-chip:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.cache-location-path {
  word-break: break-all;
}

.cache-location-dialog {
  display: flex;
  flex-direction: column;
  gap: 10px;

  p { margin: 0; }
}

.cache-location-target {
  padding: 6px 10px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container-highest);
  font-size: var(--fs-12);
  word-break: break-all;
}

.cache-location-check {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  width: 100%;
  padding: 8px 6px;
  border: 0;
  border-radius: 10px;
  background: transparent;
  color: var(--md-on-surface);
  font: inherit;
  text-align: left;
  cursor: pointer;

  &:hover { background: color-mix(in srgb, var(--md-primary) 8%, transparent); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 1px; }

  strong { font-size: var(--fs-13); }
  small { color: var(--md-on-surface-variant); font-size: var(--fs-11); line-height: 1.35; }
}

.cache-location-checkbox {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 18px;
  width: 18px;
  height: 18px;
  margin-top: 1px;
  border: 2px solid var(--md-outline);
  border-radius: 4px;
  color: var(--md-on-primary);
  transition: background 150ms, border-color 150ms;

  &.checked {
    border-color: var(--md-primary);
    background: var(--md-primary);
  }

  .material-symbols-rounded { font-size: 14px; font-weight: 700; }
}

.cache-location-check-copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
</style>
