<script setup lang="ts">
import { useI18n } from 'vue-i18n'

withDefaults(defineProps<{
  selectedCount: number
  visibleSelectedCount: number
  allVisibleSelected: boolean
  showPlay?: boolean
  showQueue?: boolean
  showPlaylist?: boolean
  showDownload?: boolean
  showDelete?: boolean
  /** 删除按钮的提示文字；歌单内是「移除」而不是删除文件 */
  deleteLabel?: string
}>(), {
  showPlay: true,
  showQueue: true,
  showPlaylist: true,
  showDownload: true,
  showDelete: false,
  deleteLabel: '',
})

const emit = defineEmits<{
  selectAll: []
  invertSelection: []
  play: []
  queue: []
  playlist: []
  download: []
  delete: []
  exit: []
}>()

const { t } = useI18n()
</script>

<template>
  <div class="selection-toolbar">
    <div class="selection-count">{{ t('common.selected_count', { count: selectedCount }) }}</div>
    <div class="selection-actions">
      <button class="selection-btn" type="button" :title="allVisibleSelected ? t('common.deselect_all') : t('common.select_all')" @click="emit('selectAll')">
        <span class="material-symbols-rounded">{{ allVisibleSelected ? 'deselect' : 'select_all' }}</span>
      </button>
      <button class="selection-btn" type="button" :title="t('common.invert_selection')" @click="emit('invertSelection')">
        <span class="material-symbols-rounded">flip</span>
      </button>
      <button v-if="showPlay" class="selection-btn" type="button" :disabled="selectedCount === 0" :title="t('common.play_selected')" @click="emit('play')">
        <span class="material-symbols-rounded">play_arrow</span>
      </button>
      <button v-if="showQueue" class="selection-btn" type="button" :disabled="selectedCount === 0" :title="t('common.queue_selected')" @click="emit('queue')">
        <span class="material-symbols-rounded">add_to_queue</span>
      </button>
      <button v-if="showPlaylist" class="selection-btn" type="button" :disabled="selectedCount === 0" :title="t('common.playlist_selected')" @click="emit('playlist')">
        <span class="material-symbols-rounded">playlist_add</span>
      </button>
      <button v-if="showDownload" class="selection-btn" type="button" :disabled="selectedCount === 0" :title="t('common.download_selected')" @click="emit('download')">
        <span class="material-symbols-rounded">download</span>
      </button>
      <button v-if="showDelete" class="selection-btn danger" type="button" :disabled="selectedCount === 0" :title="deleteLabel || t('common.delete_selected')" @click="emit('delete')">
        <span class="material-symbols-rounded">delete</span>
      </button>
      <button class="selection-btn ghost" type="button" :title="t('common.exit_selection')" @click="emit('exit')">
        <span class="material-symbols-rounded">close</span>
      </button>
    </div>
  </div>
</template>

<style scoped lang="scss">
.selection-toolbar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 12px;
  margin-bottom: 12px;
  border-radius: 14px;
  /* 与歌单详情页选择工具条同一套毛玻璃配方，保持质感一致 */
  background: color-mix(in srgb, var(--md-surface-container-high) 70%, transparent);
  -webkit-backdrop-filter: blur(24px) saturate(1.5);
  backdrop-filter: blur(24px) saturate(1.5);
  border: 1px solid color-mix(in srgb, var(--md-outline-variant) 60%, transparent);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
  color: var(--md-on-surface);
}
/* Linux WebKitGTK 无 backdrop-filter 时给不透明底色，避免列表穿透 */
@supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
  .selection-toolbar {
    background: var(--md-surface-container-high);
  }
}
.selection-count { flex: 1; font-size: 13px; font-weight: 700; color: var(--md-primary); }
.selection-actions { display: flex; align-items: center; gap: 4px; }
.selection-btn { width: 34px; height: 34px; display: inline-flex; align-items: center; justify-content: center; border: 0; border-radius: 50%; background: transparent; color: inherit; cursor: pointer; }
.selection-btn:hover:not(:disabled) { background: color-mix(in srgb, currentColor 12%, transparent); }
.selection-btn:disabled { opacity: 0.4; cursor: not-allowed; }
.selection-btn.danger { color: var(--md-error); }
.selection-btn.ghost { color: inherit; }
.material-symbols-rounded { font-size: 19px; }
</style>
