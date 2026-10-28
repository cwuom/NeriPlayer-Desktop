<script setup lang="ts">
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useBiliVideoPartsStore } from '@/stores/biliVideoParts'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import { formatTrackDuration } from '@/utils/timeFormat'

const { t } = useI18n()
const parts = useBiliVideoPartsStore()
const selectedCid = ref<number | null>(null)
watch(() => parts.info, () => { selectedCid.value = parts.info?.pages[0]?.cid ?? null })
</script>

<template>
  <M3Dialog
    :open="parts.open"
    :title="t('player.bili_parts_title')"
    :confirm-text="t('player.play_track')"
    :confirm-disabled="parts.loading || selectedCid === null || parts.failed"
    @update:open="parts.close()"
    @confirm="selectedCid !== null && parts.choose(selectedCid)"
  >
    <p class="video-title">{{ parts.info?.title || parts.track?.title }}</p>
    <p v-if="parts.loading" role="status">{{ t('common.loading') }}</p>
    <div v-else-if="parts.failed" class="load-error" role="alert">
      <p>{{ t('player.bili_parts_load_failed') }}</p>
      <button type="button" class="retry-button" @click="parts.retry()">{{ t('player.retry') }}</button>
    </div>
    <template v-else-if="parts.info">
      <p class="hint">{{ t('player.bili_parts_hint', { count: parts.info.pages.length }) }}</p>
      <div class="part-list" role="listbox" :aria-label="t('player.bili_parts_title')">
        <button
          v-for="page in parts.info.pages" :key="page.cid" type="button"
          class="part-row" :class="{ selected: selectedCid === page.cid }"
          role="option" :aria-selected="selectedCid === page.cid"
          @click="parts.choose(page.cid)"
        >
          <span class="part-number">P{{ page.page }}</span>
          <span class="part-title">{{ page.part || parts.info.title }}</span>
          <span class="part-duration">{{ formatTrackDuration(page.duration * 1000) }}</span>
        </button>
      </div>
    </template>
  </M3Dialog>
</template>

<style scoped lang="scss">
.video-title { margin: 0 0 12px; font-weight: 600; overflow-wrap: anywhere; color: var(--md-on-surface); }
.hint { margin: 0 0 12px; font-size: 12px; }
.part-list { display: flex; flex-direction: column; gap: 4px; }
.part-row {
  display: flex; align-items: center; gap: 10px; width: 100%; padding: 12px 8px;
  border: 0; border-radius: 12px; background: transparent; color: var(--md-on-surface);
  font: inherit; text-align: left; cursor: pointer;
  &:hover { background: var(--md-surface-container-highest); }
  &.selected { background: var(--md-secondary-container); color: var(--md-on-secondary-container); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: -2px; }
}
.part-number { min-width: 28px; font-size: 12px; opacity: 0.75; }
.part-title { flex: 1; min-width: 0; overflow-wrap: anywhere; }
.part-duration { flex-shrink: 0; font-size: 12px; opacity: 0.75; }
.retry-button { border: 0; border-radius: 16px; padding: 8px 16px; background: var(--md-secondary-container); color: var(--md-on-secondary-container); font: inherit; cursor: pointer; }
</style>
