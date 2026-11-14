<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { TrackInfo } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { biliSubtitleCandidates, biliSubtitlePreferences, biliSubtitleTarget, matchesBiliSubtitleLanguage, type BiliSubtitleTrack } from '@/modules/lyrics/biliSubtitle'
import { biliSubtitleRevision, invalidateBiliSubtitleCache, loadBiliSubtitleTracks } from '@/modules/lyrics/biliSubtitleLoader'
import {
  formatLyricOffsetMs, LYRIC_OFFSET_STEP_MS, MAX_LYRIC_DEFAULT_OFFSET_MS, MIN_LYRIC_DEFAULT_OFFSET_MS,
  normalizeLyricDefaultOffsetMs,
} from '@/modules/lyrics/lyricOffset'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import EditableRangeValue from '@/components/ui/EditableRangeValue.vue'

const props = withDefaults(defineProps<{ track?: TrackInfo | null; showOffset?: boolean; variant?: 'settings' | 'player' }>(), { track: null, showOffset: true, variant: 'player' })
const { t, locale } = useI18n()
const languageLabel = computed(() => t(props.track ? 'bili_subtitle.track' : 'bili_subtitle.language'))
const settings = useSettingsStore()
const tracks = ref<BiliSubtitleTrack[]>([])
const loading = ref(false)
const loginRequired = ref(false)
const failed = ref(false)
let request = 0

watch(() => JSON.stringify([props.track?.id, props.track?.playlistKey, props.track && biliSubtitleTarget(props.track), settings.biliCcSubtitleEnabled, biliSubtitleRevision.value]), async () => {
  const id = ++request
  tracks.value = []
  loginRequired.value = false
  failed.value = false
  loading.value = false
  if (!props.track || !settings.biliCcSubtitleEnabled) return
  loading.value = true
  try {
    const result = await loadBiliSubtitleTracks(props.track)
    if (id !== request) return
    tracks.value = result.tracks
    loginRequired.value = result.loginRequired
  } catch {
    if (id === request) failed.value = true
  } finally {
    if (id === request) loading.value = false
  }
}, { immediate: true })
onUnmounted(() => { request++ })

/** Localize language tags while preserving Chinese script variants and AI labels. */
function languageName(language: string): string {
  const ai = /^ai-/i.test(language)
  const tag = language.replace(/^ai-/i, '').replace(/_/g, '-')
  const alias = /^(zh-(CN|SG|Hans))$/i.test(tag) ? 'zh-CN'
    : /^(zh-(TW|HK|MO|Hant))$/i.test(tag) ? 'zh-TW' : tag.toLowerCase()
  let label: string
  if (['zh', 'zh-CN', 'zh-TW', 'en', 'ja'].includes(alias)) label = t(`bili_subtitle.languages.${alias}`)
  else {
    try { label = new Intl.DisplayNames([locale.value], { type: 'language' }).of(tag) ?? language }
    catch { label = language }
  }
  return `${label}${ai ? ' · AI' : ''}`
}

const languageOptions = computed(() => {
  const options = [{ value: '', label: t('bili_subtitle.automatic') }]
  if (props.track) {
    for (const track of tracks.value) {
      if (options.some(option => option.value === track.language)) continue
      const label = track.label && track.label !== track.language ? track.label : languageName(track.language)
      options.push({ value: track.language, label: `${label}${track.isAi && !/(?:^|[\s·(（])AI\b/i.test(label) ? ' · AI' : ''}` })
    }
  } else {
    for (const language of ['zh', 'zh-CN', 'zh-TW', 'en', 'ja', 'ai-zh', 'ai-en']) {
      options.push({ value: language, label: languageName(language) })
    }
  }
  const selected = settings.biliCcSubtitleLanguage
  if (!props.track && selected && !options.some(option => option.value === selected)) {
    options.push({ value: selected, label: languageName(selected) })
  }
  return options
})
const selectedLanguage = computed(() => {
  const selected = settings.biliCcSubtitleLanguage
  if (!props.track || !selected) return selected
  return biliSubtitleCandidates(tracks.value, biliSubtitlePreferences(settings))
    .find(track => matchesBiliSubtitleLanguage(track.language, selected))?.language ?? ''
})
const selectedLabel = computed(() => {
  if (!props.track) return undefined
  if (loading.value) return t('bili_subtitle.loading')
  if (loginRequired.value) return t('bili_subtitle.login_required_short')
  if (failed.value) return t('bili_subtitle.failed_short')
  if (!tracks.value.length) return t('bili_subtitle.no_tracks')
  return undefined
})
/** Selecting the active preference explicitly retries expired or failed requests. */
function selectLanguage(language: string) {
  if (settings.biliCcSubtitleLanguage === language) invalidateBiliSubtitleCache()
  else settings.biliCcSubtitleLanguage = language
}
const status = computed(() => {
  if (!props.track || !settings.biliCcSubtitleEnabled) return ''
  if (loading.value) return t('bili_subtitle.loading')
  if (loginRequired.value) return t('bili_subtitle.login_required')
  if (failed.value) return t('bili_subtitle.failed')
  if (!tracks.value.length) return t('bili_subtitle.empty')
  return t('bili_subtitle.tracks_available', { count: tracks.value.length })
})
function updateOffset(value: number) {
  settings.biliCcSubtitleOffset = normalizeLyricDefaultOffsetMs(value)
}
</script>

<template>
  <section class="bili-subtitle-settings" :class="{ 'bili-subtitle-settings--page': variant === 'settings' }" :aria-label="t('bili_subtitle.title')">
    <div class="subtitle-row" :class="{ 'setting-card': variant === 'settings' }">
      <span class="subtitle-icon material-symbols-rounded" aria-hidden="true">subtitles</span>
      <div class="subtitle-info"><div class="subtitle-title" :class="{ 'setting-title': variant === 'settings' }">{{ t('bili_subtitle.title') }}</div><p>{{ t('bili_subtitle.description') }}</p></div>
      <label class="m3-switch">
        <input v-model="settings.biliCcSubtitleEnabled" type="checkbox" :aria-label="t('bili_subtitle.title')" />
        <span class="track"><span class="thumb"><span v-if="settings.biliCcSubtitleEnabled" class="material-symbols-rounded">check</span></span></span>
      </label>
    </div>
    <div class="subtitle-row subtitle-row--select" :class="{ 'setting-card': variant === 'settings' }">
      <span class="subtitle-icon material-symbols-rounded" aria-hidden="true">language</span>
      <div class="subtitle-info"><div class="subtitle-title" :class="{ 'setting-title': variant === 'settings' }">{{ languageLabel }}</div><p>{{ t(track ? 'bili_subtitle.track_description' : 'bili_subtitle.language_description') }}</p></div>
      <CustomSelect class="subtitle-select" :model-value="selectedLanguage" :selected-label="selectedLabel" :options="languageOptions" :label="languageLabel"
        :disabled="!settings.biliCcSubtitleEnabled || loading || (!!track && !tracks.length)" @update:model-value="selectLanguage" />
    </div>
    <div class="subtitle-row" :class="{ 'setting-card': variant === 'settings' }">
      <span class="subtitle-icon material-symbols-rounded" aria-hidden="true">auto_awesome</span>
      <div class="subtitle-info"><div class="subtitle-title" :class="{ 'setting-title': variant === 'settings' }">{{ t('bili_subtitle.ai_fallback') }}</div><p>{{ t('bili_subtitle.ai_description') }}</p></div>
      <label class="m3-switch">
        <input v-model="settings.biliCcSubtitleAiFallback" type="checkbox" :disabled="!settings.biliCcSubtitleEnabled" :aria-label="t('bili_subtitle.ai_fallback')" />
        <span class="track"><span class="thumb"><span v-if="settings.biliCcSubtitleAiFallback" class="material-symbols-rounded">check</span></span></span>
      </label>
    </div>
    <div v-if="showOffset" class="subtitle-row" :class="{ 'setting-card': variant === 'settings' }">
      <span class="subtitle-icon material-symbols-rounded" aria-hidden="true">timer</span>
      <div class="subtitle-info subtitle-title" :class="{ 'setting-title': variant === 'settings' }">{{ t('bili_subtitle.offset') }}</div>
      <EditableRangeValue
        v-if="settings.biliCcSubtitleEnabled"
        :model-value="settings.biliCcSubtitleOffset"
        :min="MIN_LYRIC_DEFAULT_OFFSET_MS"
        :max="MAX_LYRIC_DEFAULT_OFFSET_MS"
        :step="LYRIC_OFFSET_STEP_MS"
        :display-value="formatLyricOffsetMs(settings.biliCcSubtitleOffset)"
        :aria-label="t('bili_subtitle.offset')"
        input-suffix="ms"
        @update:model-value="updateOffset"
      />
      <span v-else class="subtitle-offset-disabled">{{ formatLyricOffsetMs(settings.biliCcSubtitleOffset) }}</span>
    </div>
    <div v-if="track" class="subtitle-row">
      <p role="status">{{ status }}</p>
      <button class="m3-chip sm" :disabled="!settings.biliCcSubtitleEnabled || loading" @click="invalidateBiliSubtitleCache">{{ t('bili_subtitle.refresh') }}</button>
    </div>
  </section>
</template>

<style scoped lang="scss">
.bili-subtitle-settings { padding: 16px; border-radius: 20px; background: var(--md-surface-container); }
.subtitle-row { display: flex; align-items: center; justify-content: space-between; gap: 20px; padding: 12px 0; }
.subtitle-row + .subtitle-row { border-top: 1px solid var(--md-outline-variant); }
.subtitle-title { font-size: 14px; font-weight: 600; }
.subtitle-info { flex: 1; min-width: 0; }
.subtitle-icon { display: none; }
.subtitle-row--select { flex-wrap: wrap; }
.subtitle-row--select .subtitle-info { flex-basis: 180px; }
.subtitle-select { flex: 0 1 200px; max-width: 100%; margin-left: auto; }
.subtitle-select :deep(.custom-select-trigger) { min-height: 44px; padding: 10px 14px; font-size: 14px; }
.bili-subtitle-settings--page {
  padding: 0;
  background: transparent;
  border-radius: 0;
  .subtitle-row {
    gap: 14px;
    padding: 14px 16px;
    margin-bottom: 8px;
    border: 0;
    border-radius: var(--radius-lg);
    background: var(--md-surface-container);
    transition: background var(--duration-short);
    &:hover { background: var(--md-surface-container-high); }
  }
  .subtitle-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    border-radius: var(--radius-md);
    background: var(--md-surface-container-high);
    color: var(--md-on-surface-variant);
  }
  .subtitle-row--select .subtitle-info { flex-basis: 240px; }
  .subtitle-title { font-weight: 500; }
  p { margin-top: 2px; }
}
p { margin: 6px 0 0; font-size: 12px; line-height: 1.6; color: var(--md-on-surface-variant); }
.m3-switch { flex-shrink: 0; }
.thumb .material-symbols-rounded { font-size: 14px; }
.subtitle-offset-disabled { opacity: 0.38; white-space: nowrap; }
</style>
