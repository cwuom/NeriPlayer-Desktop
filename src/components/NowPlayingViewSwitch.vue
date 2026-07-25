<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import type { NowPlayingViewMode } from '@/modules/nowPlaying/viewMode'

const props = defineProps<{
  modelValue: NowPlayingViewMode
  lyricsAvailable: boolean
}>()

const emit = defineEmits<{
  'update:modelValue': [mode: NowPlayingViewMode]
}>()

const { t } = useI18n()

function requestMode(mode: NowPlayingViewMode) {
  if (mode === 'lyrics' && !props.lyricsAvailable) return
  emit('update:modelValue', mode)
}
</script>

<template>
  <div class="np-view-switch" role="group" :aria-label="t('player.now_playing')">
    <button
      type="button"
      class="np-view-switch__button"
      :class="{ active: modelValue === 'cover' }"
      :aria-label="t('player.view_mode_cover')"
      :aria-pressed="modelValue === 'cover'"
      :title="t('player.view_mode_cover')"
      @click="requestMode('cover')"
    >
      <span class="material-symbols-rounded" aria-hidden="true">album</span>
    </button>
    <button
      type="button"
      class="np-view-switch__button"
      :class="{ active: modelValue === 'lyrics' }"
      :disabled="!lyricsAvailable"
      :aria-label="t('player.view_mode_lyrics')"
      :aria-pressed="modelValue === 'lyrics'"
      :title="t('player.view_mode_lyrics')"
      @click="requestMode('lyrics')"
    >
      <span class="material-symbols-rounded" aria-hidden="true">lyrics</span>
    </button>
  </div>
</template>

<style scoped lang="scss">
.np-view-switch {
  display: grid;
  grid-template-columns: repeat(2, 38px);
  width: 84px;
  height: 42px;
  padding: 2px;
  border: 1px solid rgba(255, 255, 255, 0.14);
  border-radius: 8px;
  background: color-mix(in srgb, var(--np-primary, #16141a) 12%, rgba(18, 17, 22, 0.72));
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.24);
  backdrop-filter: blur(22px) saturate(1.12);
  -webkit-backdrop-filter: blur(22px) saturate(1.12);
}

.np-view-switch__button {
  width: 38px;
  height: 36px;
  display: grid;
  place-items: center;
  border-radius: 6px;
  color: rgba(255, 255, 255, 0.66);
  transition: color 160ms ease, background 180ms ease, transform 160ms ease;

  .material-symbols-rounded { font-size: 20px; }
  &:hover:not(:disabled) { color: rgba(255, 255, 255, 0.94); }
  &:active:not(:disabled) { transform: scale(0.92); }
  &.active {
    color: var(--np-on-primary, #141218);
    background: var(--np-primary-container, rgba(255, 255, 255, 0.88));
  }
  &:disabled { cursor: default; opacity: 0.34; }
}

@supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
  .np-view-switch { background: rgba(24, 22, 28, 0.94); }
}

@media (prefers-reduced-motion: reduce) {
  .np-view-switch__button { transition-duration: 80ms; }
}
</style>
