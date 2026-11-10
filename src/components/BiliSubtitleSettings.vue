<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import type { TrackInfo } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { biliSubtitleTarget, type BiliSubtitleTrack } from '@/modules/lyrics/biliSubtitle'
import { biliSubtitleRevision, invalidateBiliSubtitleCache, loadBiliSubtitleTracks } from '@/modules/lyrics/biliSubtitleLoader'
import { normalizeLyricDefaultOffsetMs } from '@/modules/lyrics/lyricOffset'
import CustomSelect from '@/components/ui/CustomSelect.vue'

const props = withDefaults(defineProps<{ track?: TrackInfo | null; showOffset?: boolean }>(), { track: null, showOffset: true })
const { t } = useI18n()
const settings = useSettingsStore()
const tracks = ref<BiliSubtitleTrack[]>([])
const loading = ref(false)
const loginRequired = ref(false)
const failed = ref(false)
let request = 0

watch(() => JSON.stringify([props.track && biliSubtitleTarget(props.track), settings.biliCcSubtitleEnabled, biliSubtitleRevision.value]), async () => {
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

const languageOptions = computed(() => {
  const options = [{ value: '', label: t('bili_subtitle.automatic') }]
  if (props.track) {
    for (const track of tracks.value) {
      if (options.some(option => option.value === track.language)) continue
      options.push({ value: track.language, label: `${track.label}${track.isAi ? ' · AI' : ''}` })
    }
  } else {
    for (const language of ['zh-CN', 'zh-TW', 'en', 'ja', 'ai-zh', 'ai-en']) {
      options.push({ value: language, label: t(`bili_subtitle.languages.${language}`) })
    }
  }
  const selected = settings.biliCcSubtitleLanguage
  if (selected && !options.some(option => option.value === selected)) {
    options.push({ value: selected, label: selected })
  }
  return options
})
const status = computed(() => {
  if (!props.track || !settings.biliCcSubtitleEnabled) return ''
  if (loading.value) return t('bili_subtitle.loading')
  if (loginRequired.value) return t('bili_subtitle.login_required')
  if (failed.value) return t('bili_subtitle.failed')
  if (!tracks.value.length) return t('bili_subtitle.empty')
  return t('bili_subtitle.tracks_available', { count: tracks.value.length })
})
function updateOffset(event: Event) {
  settings.biliCcSubtitleOffset = normalizeLyricDefaultOffsetMs(Number((event.target as HTMLInputElement).value))
}
</script>

<template>
  <section class="bili-subtitle-settings" :aria-label="t('bili_subtitle.title')">
    <div class="subtitle-row">
      <div><div class="subtitle-title">{{ t('bili_subtitle.title') }}</div><p>{{ t('bili_subtitle.description') }}</p></div>
      <label class="m3-switch">
        <input v-model="settings.biliCcSubtitleEnabled" type="checkbox" :aria-label="t('bili_subtitle.title')" />
        <span class="track"><span class="thumb"><span v-if="settings.biliCcSubtitleEnabled" class="material-symbols-rounded">check</span></span></span>
      </label>
    </div>
    <div class="subtitle-row">
      <div class="subtitle-title">{{ t('bili_subtitle.language') }}</div>
      <CustomSelect v-model="settings.biliCcSubtitleLanguage" :options="languageOptions" :label="t('bili_subtitle.language')"
        :disabled="!settings.biliCcSubtitleEnabled || loading" />
    </div>
    <div class="subtitle-row">
      <div><div class="subtitle-title">{{ t('bili_subtitle.ai_fallback') }}</div><p>{{ t('bili_subtitle.ai_description') }}</p></div>
      <label class="m3-switch">
        <input v-model="settings.biliCcSubtitleAiFallback" type="checkbox" :disabled="!settings.biliCcSubtitleEnabled" :aria-label="t('bili_subtitle.ai_fallback')" />
        <span class="track"><span class="thumb"><span v-if="settings.biliCcSubtitleAiFallback" class="material-symbols-rounded">check</span></span></span>
      </label>
    </div>
    <div v-if="showOffset" class="subtitle-row">
      <label class="subtitle-title" for="bili-cc-offset">{{ t('bili_subtitle.offset') }}</label>
      <div class="subtitle-offset"><input id="bili-cc-offset" type="number" :value="settings.biliCcSubtitleOffset" min="-5000" max="5000" step="50"
        :disabled="!settings.biliCcSubtitleEnabled" @change="updateOffset" /><span>ms</span></div>
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
p { margin: 6px 0 0; font-size: 12px; line-height: 1.6; color: var(--md-on-surface-variant); }
.m3-switch { flex-shrink: 0; }
.thumb .material-symbols-rounded { font-size: 14px; }
.subtitle-offset { display: flex; align-items: center; gap: 6px; }
.subtitle-offset input { width: 90px; padding: 8px; color: var(--md-on-surface); background: var(--md-surface-container-high); border: 1px solid var(--md-outline-variant); border-radius: 8px; }
</style>
