<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import {
  formatStorageSize,
  type StorageCacheClearOptions,
  type StorageUsageItem,
  type StorageUsageSection,
  type StorageUsageSummary,
} from '@/utils/storage'
import { useI18n } from 'vue-i18n'

const props = withDefaults(defineProps<{
  open: boolean
  loading?: boolean
  clearing?: boolean
  activeDownloadCount?: number
  summary?: StorageUsageSummary | null
}>(), {
  loading: false,
  clearing: false,
  activeDownloadCount: 0,
  summary: null,
})

const emit = defineEmits<{
  'update:open': [value: boolean]
  clear: [options: StorageCacheClearOptions]
}>()

const { t } = useI18n()
const showClearDialog = reactive({ value: false })
const clearOptions = reactive<StorageCacheClearOptions>({
  audioCache: true,
  imageCache: true,
  downloadStaging: false,
  sharedMedia: false,
  platformList: false,
  lyricsCache: false,
})

type CacheOptionKey = keyof StorageCacheClearOptions

const canClear = computed(() => Object.values(clearOptions).some(Boolean))
const downloadStagingDisabled = computed(() => props.activeDownloadCount > 0)

watch(() => props.open, open => {
  if (!open) showClearDialog.value = false
})

function closeDetails() {
  emit('update:open', false)
}

function openClearDialog() {
  clearOptions.audioCache = true
  clearOptions.imageCache = true
  clearOptions.downloadStaging = false
  clearOptions.sharedMedia = false
  clearOptions.platformList = false
  clearOptions.lyricsCache = false
  showClearDialog.value = true
}

function toggleCacheOption(key: CacheOptionKey) {
  if (key === 'downloadStaging' && downloadStagingDisabled.value) return
  clearOptions[key] = !clearOptions[key]
}

function confirmClear() {
  if (!canClear.value) return
  emit('clear', { ...clearOptions })
  showClearDialog.value = false
}

function sectionTitle(section: StorageUsageSection) {
  return t(`settings.storage_group_${section.id.replace('cleanable_cache', 'cleanable_cache')}`)
}

function itemTitle(item: StorageUsageItem) {
  return t(`settings.storage_type_${item.id}`)
}

function itemDescription(item: StorageUsageItem) {
  return t(`settings.storage_desc_${item.id}`)
}

function cacheOptionTitle(kind: string) {
  return t(`settings.storage_type_${kind}`)
}

function cacheOptionDescription(kind: string) {
  return t(`settings.storage_desc_${kind}`)
}
</script>

<template>
  <M3Dialog
    :open="open"
    :title="t('settings.storage_usage_title')"
    icon="database"
    :confirm-text="t('common.close')"
    @update:open="emit('update:open', $event)"
    @confirm="closeDetails"
  >
    <div class="storage-dialog-body">
      <div v-if="loading" class="storage-loading">
        <span class="material-symbols-rounded spinning">progress_activity</span>
        <span>{{ t('settings.storage_loading') }}</span>
      </div>
      <template v-else-if="summary">
        <div v-for="section in summary.sections" :key="section.id" class="storage-section">
          <div class="storage-section-title">{{ sectionTitle(section) }}</div>
          <div v-for="item in section.items" :key="item.id" class="storage-row">
            <div class="storage-row-info">
              <div class="storage-row-title">{{ itemTitle(item) }}</div>
              <div class="storage-row-desc">{{ itemDescription(item) }}</div>
            </div>
            <div class="storage-row-value">
              <strong>{{ formatStorageSize(item.sizeBytes) }}</strong>
              <span v-if="item.recordCount != null">{{ t('settings.storage_record_count', { count: item.recordCount }) }}</span>
              <span v-else>{{ t('settings.storage_file_count', { count: item.fileCount }) }}</span>
            </div>
          </div>
        </div>
        <div class="storage-total">
          <span>{{ t('settings.storage_total') }}</span>
          <strong>{{ formatStorageSize(summary.totalSizeBytes) }}</strong>
        </div>
        <button class="storage-clear-entry" type="button" :disabled="clearing" @click="openClearDialog">
          <span class="material-symbols-rounded">delete_sweep</span>
          <span>{{ t('settings.clear_cache') }}</span>
        </button>
      </template>
    </div>
  </M3Dialog>

  <M3Dialog
    :open="showClearDialog.value"
    :title="t('settings.storage_clear_title')"
    icon="delete_sweep"
    :confirm-text="t('common.confirm')"
    :confirm-disabled="clearing || !canClear"
    confirm-danger
    @update:open="showClearDialog.value = $event"
    @confirm="confirmClear"
  >
    <div class="storage-clear-dialog">
      <p>{{ t('settings.storage_clear_warning') }}</p>
      <div class="storage-clear-label">{{ t('settings.storage_select_cache_types') }}</div>
      <button
        class="storage-check-row"
        type="button"
        role="checkbox"
        :aria-checked="clearOptions.audioCache"
        @click="toggleCacheOption('audioCache')"
      >
        <span class="storage-checkbox" :class="{ checked: clearOptions.audioCache }" aria-hidden="true">
          <span v-if="clearOptions.audioCache" class="material-symbols-rounded">check</span>
        </span>
        <span class="storage-check-copy"><strong>{{ cacheOptionTitle('audio_cache') }}</strong><small>{{ cacheOptionDescription('audio_cache') }}</small></span>
      </button>
      <button
        class="storage-check-row"
        type="button"
        role="checkbox"
        :aria-checked="clearOptions.imageCache"
        @click="toggleCacheOption('imageCache')"
      >
        <span class="storage-checkbox" :class="{ checked: clearOptions.imageCache }" aria-hidden="true">
          <span v-if="clearOptions.imageCache" class="material-symbols-rounded">check</span>
        </span>
        <span class="storage-check-copy"><strong>{{ cacheOptionTitle('image_cache') }}</strong><small>{{ cacheOptionDescription('image_cache') }}</small></span>
      </button>
      <button
        class="storage-check-row"
        type="button"
        role="checkbox"
        :aria-checked="clearOptions.downloadStaging"
        :disabled="downloadStagingDisabled"
        :class="{ disabled: downloadStagingDisabled }"
        @click="toggleCacheOption('downloadStaging')"
      >
        <span class="storage-checkbox" :class="{ checked: clearOptions.downloadStaging }" aria-hidden="true">
          <span v-if="clearOptions.downloadStaging" class="material-symbols-rounded">check</span>
        </span>
        <span class="storage-check-copy"><strong>{{ cacheOptionTitle('download_staging') }}</strong><small>{{ downloadStagingDisabled ? t('settings.storage_download_staging_active') : cacheOptionDescription('download_staging') }}</small></span>
      </button>
      <button
        class="storage-check-row"
        type="button"
        role="checkbox"
        :aria-checked="clearOptions.sharedMedia"
        @click="toggleCacheOption('sharedMedia')"
      >
        <span class="storage-checkbox" :class="{ checked: clearOptions.sharedMedia }" aria-hidden="true">
          <span v-if="clearOptions.sharedMedia" class="material-symbols-rounded">check</span>
        </span>
        <span class="storage-check-copy"><strong>{{ cacheOptionTitle('shared_media') }}</strong><small>{{ cacheOptionDescription('shared_media') }}</small></span>
      </button>
      <button
        class="storage-check-row"
        type="button"
        role="checkbox"
        :aria-checked="clearOptions.platformList"
        @click="toggleCacheOption('platformList')"
      >
        <span class="storage-checkbox" :class="{ checked: clearOptions.platformList }" aria-hidden="true">
          <span v-if="clearOptions.platformList" class="material-symbols-rounded">check</span>
        </span>
        <span class="storage-check-copy"><strong>{{ cacheOptionTitle('platform_list_cache') }}</strong><small>{{ cacheOptionDescription('platform_list_cache') }}</small></span>
      </button>
      <button
        class="storage-check-row"
        type="button"
        role="checkbox"
        :aria-checked="clearOptions.lyricsCache"
        @click="toggleCacheOption('lyricsCache')"
      >
        <span class="storage-checkbox" :class="{ checked: clearOptions.lyricsCache }" aria-hidden="true">
          <span v-if="clearOptions.lyricsCache" class="material-symbols-rounded">check</span>
        </span>
        <span class="storage-check-copy"><strong>{{ cacheOptionTitle('lyrics_cache') }}</strong><small>{{ cacheOptionDescription('lyrics_cache') }}</small></span>
      </button>
    </div>
  </M3Dialog>
</template>

<style scoped lang="scss">
.storage-dialog-body,
.storage-clear-dialog {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.storage-dialog-body { max-height: min(56vh, 560px); overflow-y: auto; }
.storage-loading { display: flex; align-items: center; justify-content: center; gap: 8px; min-height: 120px; }
.storage-section { display: flex; flex-direction: column; gap: 6px; }
.storage-section-title { color: var(--md-primary); font-size: 13px; font-weight: 700; margin-top: 4px; }
.storage-row { display: flex; align-items: center; gap: 12px; padding: 8px 0; border-bottom: 1px solid color-mix(in srgb, var(--md-outline-variant) 26%, transparent); }
.storage-row-info { min-width: 0; flex: 1; }
.storage-row-title { color: var(--md-on-surface); font-size: 13px; font-weight: 600; }
.storage-row-desc { color: var(--md-on-surface-variant); font-size: 11px; line-height: 1.35; }
.storage-row-value { display: flex; flex-direction: column; align-items: flex-end; flex: 0 0 auto; color: var(--md-on-surface-variant); font-size: 11px; }
.storage-row-value strong { color: var(--md-on-surface); font-size: 13px; }
.storage-total { display: flex; justify-content: space-between; padding-top: 4px; color: var(--md-on-surface); font-size: 14px; }
.storage-total strong { color: var(--md-primary); }
.storage-clear-entry { display: inline-flex; align-items: center; justify-content: center; gap: 8px; min-height: 40px; border-radius: 20px; background: var(--md-error-container); color: var(--md-on-error-container); font: inherit; font-weight: 600; cursor: pointer; }
.storage-clear-entry:disabled { opacity: 0.45; cursor: not-allowed; }
.storage-clear-dialog p { margin: 0; color: var(--md-on-surface-variant); }
.storage-clear-label { color: var(--md-on-surface); font-size: 13px; font-weight: 700; }
.storage-check-row { display: flex; align-items: flex-start; gap: 10px; width: 100%; padding: 8px 6px; border: 0; border-radius: 10px; background: transparent; color: var(--md-on-surface); font: inherit; text-align: left; cursor: pointer; }
.storage-check-row:hover:not(:disabled) { background: color-mix(in srgb, var(--md-primary) 8%, transparent); }
.storage-check-row:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 1px; }
.storage-check-row.disabled,
.storage-check-row:disabled { opacity: 0.45; cursor: not-allowed; }
.storage-checkbox { width: 18px; height: 18px; display: inline-flex; align-items: center; justify-content: center; flex: 0 0 18px; margin-top: 1px; border: 2px solid var(--md-outline); border-radius: 4px; color: var(--md-on-primary); transition: background 150ms, border-color 150ms; }
.storage-checkbox.checked { border-color: var(--md-primary); background: var(--md-primary); }
.storage-checkbox .material-symbols-rounded { font-size: 14px; font-weight: 700; }
.storage-check-copy { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.storage-check-row strong { font-size: 13px; }
.storage-check-row small { color: var(--md-on-surface-variant); font-size: 11px; line-height: 1.35; }
.spinning { animation: storage-spin 1s linear infinite; }
@keyframes storage-spin { to { transform: rotate(360deg); } }
</style>
