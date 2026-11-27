<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { openUrl } from '@tauri-apps/plugin-opener'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import { useUpdateStore } from '@/stores/update'
import { isIncremental, progressPercent, RELEASES_URL, savedPercent } from '@/modules/update/updateModel'
import { formatStorageSize } from '@/utils/storage'
import { createLogger } from '@/utils/logger'

const { t } = useI18n()
const update = useUpdateStore()
const log = createLogger('update')

const check = computed(() => update.check)
const summary = computed(() => check.value?.summary ?? null)

const title = computed(() => {
  switch (update.phase) {
    case 'downloading': return t('update.title_downloading')
    case 'ready': return t('update.title_ready')
    case 'installing': return t('update.title_installing')
    case 'error': return t('update.title_error')
    default: return t('update.title_available')
  }
})

const icon = computed(() => {
  switch (update.phase) {
    case 'ready': return 'task_alt'
    case 'error': return 'error'
    case 'downloading':
    case 'installing': return 'downloading'
    default: return 'system_update'
  }
})

/** 不能应用内更新时的说明；可以时为空 */
const manualReason = computed(() => {
  const value = check.value
  if (!value || value.inApp) return ''
  return t(`update.manual_${value.unsupportedReason ?? 'noPackage'}`)
})

const sizeText = computed(() => {
  const value = summary.value
  if (!value) return ''
  if (value.downloadBytes === 0) return t('update.size_nothing')
  const size = formatStorageSize(value.downloadBytes)
  if (isIncremental(value)) return t('update.size_incremental', { size, percent: savedPercent(value) })
  return t('update.size_full', { size })
})

const percent = computed(() => progressPercent(update.progress))
const progressText = computed(() => {
  const progress = update.progress
  if (!progress) return ''
  return t('update.download_progress', {
    done: formatStorageSize(progress.downloaded),
    total: formatStorageSize(progress.total),
  })
})

const errorText = computed(() => t(`update.error_${update.errorCode ?? 'unknown'}`))
// 前进不了的错误（签名、权限、清单）重试多半没用，给出手动下载的出口
const offerManualDownload = computed(() =>
  update.phase === 'error' && ['signature', 'permission', 'manifest', 'unknown'].includes(update.errorCode ?? 'unknown'),
)

function close() {
  // 下载中关掉窗口不影响下载，完成后可以从设置页或通知再打开
  update.dialogOpen = false
}

async function open(url: string | null | undefined) {
  try {
    await openUrl(url || RELEASES_URL)
  } catch (error) {
    log.warn('open release page failed', error)
  }
}

function openDownloadPage() {
  void open(RELEASES_URL)
  close()
}
</script>

<template>
  <M3Dialog
    :open="update.dialogOpen"
    :title="title"
    :icon="icon"
    :confirm-danger="update.phase === 'error'"
    @update:open="close"
  >
    <div class="update-body">
      <div v-if="check?.latestVersion" class="version-row">
        <span class="version-pill current">{{ check.currentVersion }}</span>
        <span class="material-symbols-rounded version-arrow" aria-hidden="true">arrow_forward</span>
        <span class="version-pill latest">{{ check.latestVersion }}</span>
        <button v-if="check.releaseUrl" type="button" class="release-link" @click="open(check.releaseUrl)">
          {{ t('update.view_release') }}
          <span class="material-symbols-rounded" aria-hidden="true">open_in_new</span>
        </button>
      </div>

      <template v-if="update.phase === 'downloading'">
        <div
          class="progress-track"
          role="progressbar"
          :aria-valuenow="percent"
          aria-valuemin="0"
          aria-valuemax="100"
          :aria-label="t('update.title_downloading')"
        >
          <div class="progress-fill" :style="{ width: `${percent}%` }" />
        </div>
        <div class="progress-meta">
          <span>{{ progressText }}</span>
          <span class="progress-percent">{{ percent }}%</span>
        </div>
        <p class="hint">{{ t('update.downloading_hint') }}</p>
      </template>

      <p v-else-if="update.phase === 'ready'" class="lead">{{ t('update.ready_hint') }}</p>
      <p v-else-if="update.phase === 'installing'" class="lead">{{ t('update.installing_hint') }}</p>
      <p v-else-if="update.phase === 'error'" class="lead error" role="alert">{{ errorText }}</p>
      <template v-else>
        <p v-if="manualReason" class="lead">{{ manualReason }}</p>
        <p v-else-if="sizeText" class="lead">{{ sizeText }}</p>
      </template>
    </div>

    <template #actions>
      <template v-if="update.phase === 'downloading'">
        <button type="button" class="m3-btn tonal" @click="update.cancelDownload()">{{ t('update.cancel_download') }}</button>
        <button type="button" class="m3-btn filled" @click="close">{{ t('update.close') }}</button>
      </template>
      <template v-else-if="update.phase === 'ready'">
        <button type="button" class="m3-btn tonal" @click="close">{{ t('update.later') }}</button>
        <button type="button" class="m3-btn filled" @click="update.install()">{{ t('update.restart_now') }}</button>
      </template>
      <template v-else-if="update.phase === 'installing'">
        <button type="button" class="m3-btn filled disabled" aria-disabled="true">{{ t('update.status_installing') }}</button>
      </template>
      <template v-else-if="update.phase === 'error'">
        <button type="button" class="m3-btn tonal" @click="close">{{ t('update.close') }}</button>
        <button v-if="offerManualDownload" type="button" class="m3-btn tonal" @click="openDownloadPage">{{ t('update.open_download_page') }}</button>
        <button type="button" class="m3-btn filled" @click="update.retry()">{{ t('update.retry') }}</button>
      </template>
      <template v-else>
        <button type="button" class="m3-btn text skip" @click="update.skipCurrentVersion()">{{ t('update.skip_version') }}</button>
        <button type="button" class="m3-btn tonal" @click="close">{{ t('update.later') }}</button>
        <button v-if="check?.inApp" type="button" class="m3-btn filled" @click="update.download()">{{ t('update.update_now') }}</button>
        <button v-else type="button" class="m3-btn filled" @click="openDownloadPage">{{ t('update.open_download_page') }}</button>
      </template>
    </template>
  </M3Dialog>
</template>

<style scoped lang="scss">
.update-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
  white-space: normal;
}

.version-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.version-pill {
  padding: 2px 10px;
  border-radius: var(--radius-full);
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: var(--fs-13, 13px);
  font-variant-numeric: tabular-nums;

  &.current {
    background: var(--md-surface-container-high);
    color: var(--md-on-surface-variant);
  }

  &.latest {
    background: color-mix(in srgb, var(--md-primary) 16%, transparent);
    color: var(--md-primary);
    font-weight: 700;
  }
}

.version-arrow {
  font-size: 16px;
  color: var(--md-on-surface-variant);
}

.release-link {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  margin-left: auto;
  padding: 2px 4px;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--md-primary);
  font: inherit;
  font-size: var(--fs-13, 13px);
  font-weight: 600;
  cursor: pointer;

  .material-symbols-rounded { font-size: 14px; }
  &:hover { text-decoration: underline; }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 2px; }
}

.lead {
  margin: 0;
  color: var(--md-on-surface);

  &.error { color: var(--md-error); }
}

.hint {
  margin: 0;
  font-size: var(--fs-13, 13px);
  color: var(--md-on-surface-variant);
}

.progress-track {
  height: 8px;
  overflow: hidden;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-high);
}

.progress-fill {
  height: 100%;
  border-radius: inherit;
  background: var(--md-primary);
  transition: width 200ms var(--ease-standard, ease-out);
}

.progress-meta {
  display: flex;
  justify-content: space-between;
  font-size: var(--fs-13, 13px);
  color: var(--md-on-surface-variant);
  font-variant-numeric: tabular-nums;
}

.progress-percent { font-weight: 600; color: var(--md-on-surface); }

.m3-btn.text {
  background: none;
  color: var(--md-on-surface-variant);
  min-width: 0;
  padding: 0 10px;

  &:hover { color: var(--md-on-surface); background: color-mix(in srgb, var(--md-on-surface) 6%, transparent); }
}

/* 「跳过此版本」是次要操作，靠左放，和主操作拉开距离 */
.skip { margin-right: auto; }

@media (prefers-reduced-motion: reduce) {
  .progress-fill { transition: none; }
}
</style>
