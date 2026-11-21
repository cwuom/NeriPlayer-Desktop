<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useBiliVideoPartsStore } from '@/stores/biliVideoParts'
import M3Dialog from '@/components/ui/M3Dialog.vue'
import { hasBiliPartNameSeparator } from '@/modules/playback/biliVideoParts'
import { formatTrackDuration } from '@/utils/timeFormat'

const { t } = useI18n()
const parts = useBiliVideoPartsStore()
const selectedCid = ref<number | null>(null)
const scope = ref<'whole' | 'part'>('whole')
const previewIndex = computed(() => parts.info?.pages.findIndex(page => hasBiliPartNameSeparator(page.part)) ?? -1)
const previewTrack = computed(() => parts.tracks[previewIndex.value])
watch(() => parts.info, () => { selectedCid.value = parts.info?.pages[0]?.cid ?? null })
watch(() => parts.collection, () => { scope.value = 'whole' })

const confirmText = computed(() => {
  const collection = parts.collection
  if (!collection) return t('player.play_track')
  if (!collection.isLiked) return t('common.confirm')
  return t(collection.isLiked(collection[scope.value]) ? 'player.bili_collect_remove' : 'player.bili_collect_save')
})

function confirm() {
  if (parts.collection) parts.chooseCollection(scope.value)
  else if (selectedCid.value !== null) parts.choose(selectedCid.value)
}

function handleNameOrderKeydown(event: KeyboardEvent) {
  if (event.key !== 'Tab' && event.key !== 'Escape') event.stopPropagation()
}
</script>

<template>
  <M3Dialog
    :open="parts.open"
    :title="t(parts.collection ? 'player.bili_collect_scope' : 'player.bili_parts_title')"
    :confirm-text="confirmText"
    :confirm-disabled="!parts.collection && selectedCid === null"
    @update:open="parts.close()"
    @confirm="confirm"
  >
    <div v-if="parts.collection" class="part-list" role="group" :aria-label="t('player.bili_collect_scope')">
      <button
        type="button" class="part-row scope-row" :class="{ selected: scope === 'whole' }"
        :aria-pressed="scope === 'whole'" @click="scope = 'whole'"
      >
        <strong>{{ t('player.bili_collect_whole') }}</strong>
        <span>{{ parts.collection.whole.title }}</span>
      </button>
      <button
        type="button" class="part-row scope-row" :class="{ selected: scope === 'part' }"
        :aria-pressed="scope === 'part'" @click="scope = 'part'"
      >
        <strong>{{ t('player.bili_collect_part', { page: parts.collection.partNumber }) }}</strong>
        <span>{{ parts.collection.part.title }}</span>
      </button>
    </div>
    <template v-else-if="parts.info">
      <p class="video-title">{{ parts.info.title }}</p>
      <p class="hint">{{ t('player.bili_parts_hint', { count: parts.info.pages.length }) }}</p>
      <fieldset v-if="previewIndex >= 0" class="name-order" @keydown="handleNameOrderKeydown">
        <legend>{{ t('player.bili_parts_name_order') }}</legend>
        <label>
          <input v-model="parts.nameOrder" type="radio" name="bili-part-name-order" value="title-first">
          {{ t('player.bili_parts_title_first') }}
        </label>
        <label>
          <input v-model="parts.nameOrder" type="radio" name="bili-part-name-order" value="artist-first">
          {{ t('player.bili_parts_artist_first') }}
        </label>
      </fieldset>
      <div v-if="previewTrack" class="name-preview" aria-live="polite" aria-atomic="true">
        <p class="preview-heading">{{ t('player.bili_parts_preview', { page: parts.info.pages[previewIndex].page }) }}</p>
        <dl>
          <dt>{{ t('player.song_title') }}</dt>
          <dd>{{ previewTrack.title }}</dd>
          <dt>{{ t('player.artist_name') }}</dt>
          <dd>{{ previewTrack.artist }}</dd>
        </dl>
      </div>
      <p v-if="previewIndex >= 0" class="hint">{{ t('player.bili_parts_name_order_hint') }}</p>
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
.name-order {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 16px;
  min-inline-size: 0;
  margin: 0 0 12px;
  padding: 4px 0 0;
  border: 0;
  font-size: 12px;
  legend { padding: 0; font-weight: 600; }
  label { display: inline-flex; align-items: center; gap: 6px; min-height: 28px; cursor: pointer; }
  input { margin: 0; accent-color: var(--md-primary); }
  input:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 3px; }
}
.name-preview {
  margin-bottom: 8px;
  padding: 10px 12px;
  border-radius: 12px;
  background: var(--md-surface-container-highest);
  font-size: 12px;
  .preview-heading { margin: 0 0 6px; font-weight: 600; }
  dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 4px 12px; margin: 0; }
  dt { color: var(--md-on-surface-variant); }
  dd { margin: 0; color: var(--md-on-surface); overflow-wrap: anywhere; }
}
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
.scope-row { flex-direction: column; align-items: flex-start; gap: 2px; padding-inline: 12px; }
.scope-row span { font-size: 12px; overflow-wrap: anywhere; }
</style>
