<script setup lang="ts">
import { writeClipboardText } from '@/utils/clipboard'
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { isTauri } from '@tauri-apps/api/core'
import { usePlayerStore } from '@/stores/player'
import {
  AUDIO_EFFECT_PRESETS, GRAPHIC_EQ_FREQUENCIES, SOUND_CONTROLS,
  type AudioEffectsSound, type ParametricEqBand, type SpeakerOptimizerSettings,
} from '@/modules/audioEffects/model'
import { LISTENING_CONTROLS, protectListeningSound, type ListeningControlKey } from '@/modules/audioEffects/listening'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import DspNumberControl from './DspNumberControl.vue'
import DspFrequencyGraph from './DspFrequencyGraph.vue'

const player = usePlayerStore()
const { t } = useI18n()
const sound = computed(() => player.audioEffectsProfile.sound)
const currentOutput = computed(() => player.audioEffectsOutputDevice === 'system_default' ? t('dsp.system_default') : player.audioEffectsOutputDevice)
const root = ref<HTMLElement | null>(null)
const mode = ref<'simple' | 'advanced'>('simple')
const category = ref('common')
const categories = ['common', 'all', 'style', 'tone', 'scene', 'device', 'user']
const commonPresets = ['flat', 'pop', 'vocal_boost', 'bass_boost', 'warm', 'live', 'night', 'headphone_hifi']
const presetOptions = computed(() => [
  ...AUDIO_EFFECT_PRESETS.filter(preset => preset.id !== 'custom' && (category.value === 'all'
    || (category.value === 'common' ? commonPresets.includes(preset.id) : preset.category === category.value)))
    .map(preset => ({ value: preset.id, label: t(`dsp.presets.${preset.id}`), description: t(`dsp.preset_notes.${preset.id}`), user: false })),
  ...(category.value === 'all' || category.value === 'user' ? player.audioEffects.userPresets.map(preset => ({ value: preset.id, label: preset.name, description: '', user: true })) : []),
])
const presetLabel = computed(() => {
  const id = player.audioEffectsProfile.presetId
  if (id === 'custom') return t('dsp.manual')
  return player.audioEffects.userPresets.find(preset => preset.id === id)?.name
    ?? (AUDIO_EFFECT_PRESETS.some(preset => preset.id === id) ? t(`dsp.presets.${id}`) : t('dsp.manual'))
})
const currentUserPreset = computed(() => player.audioEffects.userPresets.find(preset => preset.id === player.audioEffectsProfile.presetId))
function listeningLabel(key: ListeningControlKey) {
  const value = Math.round(player.listeningTuning[key])
  return value === 0 ? t('dsp.listening.neutral') : `${t(`dsp.listening.${key}.${value < 0 ? 'left' : 'right'}`)} ${Math.abs(value)}%`
}
async function revealSetting(title: string) {
  const target = title === t('player.eq_preset') ? t('dsp.preset_section') : title
  const element = [...(root.value?.querySelectorAll<HTMLElement>('.setting-title') ?? [])]
    .find(item => item.textContent?.trim() === target)
  if (title === t('dsp.expert_mode') || element?.closest('.effects-professional, .effects-professional-control')) mode.value = 'advanced'
  else if (title === t('dsp.simple_mode') || element?.closest('.effects-listening')) mode.value = 'simple'
  await nextTick()
  for (let parent = element?.parentElement; parent; parent = parent.parentElement) {
    if (parent instanceof HTMLDetailsElement) parent.open = true
    if (parent === root.value) break
  }
}
defineExpose({ revealSetting })
const presetName = ref('')
const pasteOpen = ref(false)
const pasteText = ref('')
const copied = ref(false)
const autoEqText = ref('')
const presetStatus = ref<{ text: string, error: boolean } | null>(null)
const autoEqStatus = ref<{ text: string, error: boolean } | null>(null)
let presetStatusTimer: ReturnType<typeof setTimeout> | undefined
let copiedTimer: ReturnType<typeof setTimeout> | undefined
const peqTypes = ['peak', 'low_shelf', 'high_shelf', 'low_pass', 'high_pass', 'notch', 'band_pass']
const sections = ['tone', 'space', 'dynamics'] as const
const soundSwitches = { space: ['mono', 'swapChannels'], dynamics: ['compressorEnabled', 'limiterEnabled'], tone: [] } as const
const rateControls = [
  { key: 'speed', title: 'player.playback_speed', desc: 'settings.audio_effects_speed_desc', min: 0.1, max: 4 },
  { key: 'pitch', title: 'player.pitch', desc: 'settings.audio_effects_pitch_desc', min: 0.25, max: 2 },
] as const
const speedPresets = [0.5, 0.75, 0.95, 1, 1.05, 1.25, 1.5, 2]
const sectionIcons = { equalizer: 'equalizer', tone: 'graphic_eq', space: 'surround_sound', dynamics: 'compress', speaker: 'speaker', rate: 'speed', advanced: 'memory' } as const
const speakerControls = [
  { key: 'protectionHz', min: 0, max: 400, step: 10, unit: 'Hz', scale: 1 },
  ...(['bassHarmonics', 'loudness', 'clarity', 'stereoExpand'] as const).map(key => ({ key, min: 0, max: 1, step: 0.01, unit: '%', scale: 100 })),
] as const

interface RuntimeStats {
  active: boolean
  cpuLoadPercent: number
  limiterReductionDb: number
  compressorReductionDb: number
  sampleRate: number
  nativeAvailable: boolean
  inactiveReason: string | null
}
const stats = ref<RuntimeStats | null>(null)
let statsTimer: ReturnType<typeof setInterval> | undefined
let mounted = false
let statsPending = false
async function refreshStats() {
  if (!isTauri() || statsPending) return
  statsPending = true
  try {
    const result = await player.refreshAudioEffectsStats()
    if (mounted) stats.value = result
  } catch {
    if (mounted) stats.value = null
  } finally { statsPending = false }
}
onMounted(() => { mounted = true; void refreshStats(); statsTimer = setInterval(() => void refreshStats(), 1000) })
onUnmounted(() => { mounted = false; clearInterval(statsTimer); clearTimeout(presetStatusTimer); clearTimeout(copiedTimer) })

function changeSound(key: keyof AudioEffectsSound, value: number | boolean) {
  player.updateAudioEffectsSound({ [key]: value })
}
function toggleSound(key: keyof AudioEffectsSound, enabled: boolean) {
  if (enabled && !player.audioEffectsProfile.enabled) player.updateAudioEffectsProfile({ enabled: true })
  player.updateAudioEffectsSound({ [key]: enabled, ...(key === 'parametricEnabled' && enabled ? { equalizerEnabled: true } : {}) })
}
function changeBand(index: number, value: number) {
  const bands = [...sound.value.equalizerBandsDb]
  bands[index] = value
  player.updateAudioEffectsSound({ equalizerBandsDb: bands })
}
function changePeq(index: number, patch: Partial<ParametricEqBand>) {
  player.updateAudioEffectsSound({ parametricBands: sound.value.parametricBands.map((band, position) => position === index ? { ...band, ...patch } : band) })
}
function addPeq() {
  if (sound.value.parametricBands.length >= 10) return
  if (!player.audioEffectsProfile.enabled) player.updateAudioEffectsProfile({ enabled: true })
  player.updateAudioEffectsSound({ equalizerEnabled: true, parametricEnabled: true, parametricBands: [...sound.value.parametricBands, { enabled: true, type: 'peak', frequencyHz: 1000, gainDb: 0, q: 0.707 }] })
}
function removePeq(index: number) {
  player.updateAudioEffectsSound({ parametricBands: sound.value.parametricBands.filter((_, position) => position !== index) })
}
function changeSpeaker(key: keyof SpeakerOptimizerSettings, value: number | string | boolean) {
  if (key === 'enabled' && value === true) {
    if (mode.value === 'simple') {
      player.updateAudioEffects({ autoHeadroom: true })
      player.updateAudioEffectsProfile({ enabled: true, sound: protectListeningSound(sound.value) })
    } else if (!player.audioEffectsProfile.enabled) player.updateAudioEffectsProfile({ enabled: true })
  }
  player.updateAudioEffects({ speaker: { ...player.audioEffects.speaker, [key]: value,
    ...(key === 'size' && mode.value === 'simple' ? { protectionHz: 0 } : {}) } })
}
function setRate(key: 'speed' | 'pitch', value: number) {
  if (key === 'speed') player.setSpeed(value)
  else player.setPitch(value)
}
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error)
function showPresetStatus(text: string, error = false) {
  presetStatus.value = { text, error }
  clearTimeout(presetStatusTimer)
  if (!error) presetStatusTimer = setTimeout(() => { presetStatus.value = null }, 4000)
}
function savePreset() {
  try { player.saveAudioEffectsPreset(presetName.value.trim()); presetName.value = ''; category.value = 'user'; presetStatus.value = null }
  catch (error) { showPresetStatus(errorText(error), true) }
}
function runImport(text: string) {
  try {
    const result = player.importAudioEffectsPreset(text)
    pasteOpen.value = false
    pasteText.value = ''
    if (result.addedPresets > 0) category.value = 'user'
    showPresetStatus(result.kind === 'settings'
      ? t('dsp.transfer.imported_settings', { count: result.addedPresets })
      : t('dsp.transfer.imported_profile', { name: result.name || t('dsp.manual') }))
  } catch (error) { showPresetStatus(errorText(error), true) }
}
async function readFileText(event: Event): Promise<string | null> {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  if (!file) return null
  if (file.size > 1024 * 1024) throw new Error(t('dsp.file_too_large'))
  return file.text()
}
async function importPresetFile(event: Event) {
  try { const text = await readFileText(event); if (text !== null) runImport(text) }
  catch (error) { showPresetStatus(errorText(error), true) }
}
async function copyShareCode() {
  try {
    await writeClipboardText(player.exportAudioEffectsPreset(currentUserPreset.value?.id))
    copied.value = true
    clearTimeout(copiedTimer)
    copiedTimer = setTimeout(() => { copied.value = false }, 2000)
  } catch (error) { showPresetStatus(errorText(error), true) }
}
function importAutoEq() {
  try {
    const result = player.importAutoEq(autoEqText.value)
    autoEqStatus.value = { text: [t('dsp.autoeq_result', { imported: result.imported, skipped: result.skipped }), ...result.warnings].join(' '), error: false }
  } catch (error) { autoEqStatus.value = { text: errorText(error), error: true } }
}
async function readAutoEqFile(event: Event) {
  try { const text = await readFileText(event); if (text !== null) autoEqText.value = text }
  catch (error) { autoEqStatus.value = { text: errorText(error), error: true } }
}
</script>

<template>
  <div ref="root" class="audio-effects-settings">
    <div class="setting-card effects-master">
      <div class="setting-icon-wrap" :class="{ accent: player.audioEffectsProfile.enabled }"><span class="material-symbols-rounded" aria-hidden="true">graphic_eq</span></div>
      <div class="setting-info"><div class="setting-title">{{ t('dsp.enabled') }}</div><div class="setting-desc">{{ t('dsp.enabled_help') }}</div></div>
      <label class="m3-switch"><input type="checkbox" :checked="player.audioEffectsProfile.enabled" :aria-label="t('dsp.enabled')"
        @change="player.updateAudioEffectsProfile({ enabled: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"></span></span></label>
    </div>

    <section class="effects-block effects-preset-section" :aria-label="t('dsp.preset_section')">
      <div class="section-label effects-label"><h3 class="setting-title">{{ t('dsp.preset_section') }}</h3><span class="effects-label-meta">{{ t('dsp.current_preset', { name: presetLabel }) }}</span></div>
      <div class="setting-card setting-card--stack effects-panel">
        <div class="effects-tabs" role="group" :aria-label="t('dsp.preset_section')">
          <button v-for="item in categories" :key="item" type="button" class="effects-tab" :class="{ active: category === item }" :aria-pressed="category === item" @click="category = item">{{ t(`dsp.categories.${item}`) }}</button>
        </div>
        <div class="effects-preset-grid" role="group" :aria-label="t('player.eq_preset')">
          <div v-for="preset in presetOptions" :key="preset.value" class="effects-preset-item">
            <button type="button" class="effects-preset-card"
              :class="{ active: player.audioEffectsProfile.presetId === preset.value }"
              :aria-label="preset.label" :aria-pressed="player.audioEffectsProfile.presetId === preset.value"
              @click="player.setAudioEffectsPreset(preset.value)"><strong>{{ preset.label }}</strong><span v-if="preset.description">{{ preset.description }}</span></button>
            <button v-if="preset.user" type="button" class="effects-preset-delete" :aria-label="`${t('dsp.delete_preset')} ${preset.label}`" :title="t('dsp.delete_preset')"
              @click="player.deleteAudioEffectsPreset(preset.value)"><span class="material-symbols-rounded" aria-hidden="true">delete</span></button>
          </div>
        </div>
        <p v-if="!presetOptions.length" class="effects-empty">{{ t('dsp.no_user_presets') }}</p>
        <p class="setting-desc">{{ t('dsp.preset_help') }}</p>
      </div>

      <div class="section-label">{{ t('dsp.manage_presets') }}</div>
      <div class="setting-card effects-save">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded" aria-hidden="true">bookmark_add</span></div>
        <input v-model="presetName" class="effects-text-input" maxlength="24" :placeholder="t('dsp.preset_name')" :aria-label="t('dsp.preset_name')" />
        <button type="button" class="m3-chip active" :disabled="!presetName.trim() || player.audioEffects.userPresets.length >= 50" @click="savePreset">{{ t('dsp.save_preset') }}</button>
      </div>
      <div class="setting-card setting-card--stack effects-transfer">
        <div class="card-heading">
          <div class="setting-icon-wrap"><span class="material-symbols-rounded" aria-hidden="true">swap_horiz</span></div>
          <div class="setting-info"><div class="setting-title">{{ t('dsp.transfer.title') }}</div><div class="setting-desc">{{ t('dsp.transfer.help') }}</div></div>
        </div>
        <div class="effects-transfer-actions effects-indent">
          <label class="m3-chip file-button"><span class="material-symbols-rounded" aria-hidden="true">upload_file</span>{{ t('dsp.transfer.from_file') }}
            <input type="file" accept=".json,.txt,application/json,text/plain" :aria-label="t('dsp.transfer.from_file')" @change="importPresetFile" /></label>
          <button type="button" class="m3-chip" :class="{ active: pasteOpen }" :aria-expanded="pasteOpen" @click="pasteOpen = !pasteOpen">
            <span class="material-symbols-rounded" aria-hidden="true">content_paste</span>{{ t('dsp.transfer.paste') }}</button>
          <button type="button" class="m3-chip" @click="copyShareCode"><span class="material-symbols-rounded" aria-hidden="true">{{ copied ? 'check' : 'content_copy' }}</span>{{ t(copied ? 'dsp.transfer.copied' : 'dsp.transfer.copy') }}</button>
        </div>
        <div v-if="pasteOpen" class="effects-paste effects-indent">
          <textarea v-model="pasteText" :aria-label="t('dsp.transfer.paste_placeholder')" :placeholder="t('dsp.transfer.paste_placeholder')" spellcheck="false"></textarea>
          <div class="effects-toolbar"><button type="button" class="m3-chip active" :disabled="!pasteText.trim()" @click="runImport(pasteText)">{{ t('dsp.transfer.import') }}</button>
            <button type="button" class="m3-chip" @click="pasteOpen = false; pasteText = ''">{{ t('common.cancel') }}</button></div>
        </div>
        <p v-if="presetStatus" class="effects-status effects-indent" :class="{ error: presetStatus.error }" role="status" aria-live="polite">{{ presetStatus.text }}</p>
      </div>
    </section>

    <div class="setting-card effects-mode-switch" role="group" :aria-label="t('dsp.adjustment_mode')">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded" aria-hidden="true">tune</span></div>
      <div class="setting-info"><div class="setting-title">{{ t('dsp.adjustment_mode') }}</div></div>
      <div class="effects-tabs effects-tabs--fit">
        <button type="button" class="setting-title effects-tab" :class="{ active: mode === 'simple' }" :aria-pressed="mode === 'simple'" @click="mode = 'simple'">{{ t('dsp.simple_mode') }}</button>
        <button type="button" class="setting-title effects-tab" :class="{ active: mode === 'advanced' }" :aria-pressed="mode === 'advanced'" @click="mode = 'advanced'">{{ t('dsp.expert_mode') }}</button>
      </div>
    </div>

    <section v-show="mode === 'simple'" class="effects-block effects-listening" :aria-label="t('dsp.listening.title')">
      <div class="section-label effects-label"><h3>{{ t('dsp.listening.title') }}</h3><button type="button" class="effects-text-btn" @click="player.resetListeningTuning()">{{ t('dsp.listening.reset') }}</button></div>
      <p class="effects-lead">{{ t('dsp.listening.help') }}</p>
      <div class="effects-grid"><DspNumberControl v-for="control in LISTENING_CONTROLS" :key="control.key"
        :class="{ 'listening-space': control.key === 'space' }" :label="t(`dsp.listening.${control.key}.title`)"
        :help="t(`dsp.listening.${control.key}.help`)" :model-value="player.listeningTuning[control.key]"
        :min="control.min" :max="100" :step="1" unit="%" :value-label="listeningLabel(control.key)"
        :min-label="t(`dsp.listening.${control.key}.left`)" :max-label="t(`dsp.listening.${control.key}.right`)"
        @update:model-value="player.setListeningTuning(control.key, $event)" /></div>
      <p class="effects-note"><span class="material-symbols-rounded" aria-hidden="true">shield</span>{{ t('dsp.listening.protection') }}</p>
    </section>

    <div v-show="mode === 'advanced'" class="effects-professional">
      <div class="section-label">{{ t('dsp.expert_mode') }}</div>

      <details class="effects-section" open>
        <summary>
          <span class="setting-icon-wrap"><span class="material-symbols-rounded" aria-hidden="true">{{ sectionIcons.equalizer }}</span></span>
          <span class="effects-summary-title">{{ t('dsp.equalizer_section') }}</span>
          <button type="button" class="effects-text-btn" @click.prevent="player.resetAudioEffectsSection('equalizer')">{{ t('dsp.reset_section') }}</button>
          <span class="material-symbols-rounded effects-chevron" aria-hidden="true">expand_more</span>
        </summary>
        <div class="effects-content">
          <DspFrequencyGraph :sound="sound" />
          <div class="effects-grid"><DspNumberControl v-for="control in SOUND_CONTROLS.filter(item => item.section === 'equalizer')" :key="control.key" :label="t(`dsp.controls.${control.key}`)" :help="t(`dsp.help.${control.key}`)" :model-value="sound[control.key]" :min="control.min" :max="control.max" :step="control.step" :unit="control.unit" @update:model-value="changeSound(control.key, $event)" /></div>
          <div class="effects-heading"><div class="setting-info"><div class="setting-title">{{ t('dsp.graphic_eq') }}</div><div class="setting-desc">{{ t('settings.audio_effects_eq_desc') }}</div></div>
            <label class="m3-switch"><input type="checkbox" :checked="sound.equalizerEnabled" :aria-label="t('dsp.graphic_eq')" @change="toggleSound('equalizerEnabled', ($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"></span></span></label></div>
          <div class="effects-grid effects-eq-bands">
            <DspNumberControl v-for="(frequency, index) in GRAPHIC_EQ_FREQUENCIES" :key="frequency" class="effects-band" :label="t('settings.audio_effects_band', { frequency })" :model-value="sound.equalizerBandsDb[index] ?? 0" :min="-12" :max="12" :step="0.1" unit="dB" @update:model-value="changeBand(index, $event)" />
          </div>
          <div class="effects-heading"><div class="setting-info"><div class="setting-title">{{ t('dsp.parametric_eq') }}</div><div class="setting-desc">{{ t('dsp.parametric_help') }}</div></div>
            <label class="m3-switch"><input type="checkbox" :checked="sound.parametricEnabled" :aria-label="t('dsp.parametric_eq')" @change="toggleSound('parametricEnabled', ($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"></span></span></label></div>
          <div v-for="(band, index) in sound.parametricBands" :key="index" class="peq-band">
            <div class="effects-toolbar"><strong>{{ t('dsp.filter_number', { number: index + 1 }) }}</strong>
              <label class="peq-enabled"><input type="checkbox" :checked="band.enabled" :aria-label="t('dsp.filter_enabled', { number: index + 1 })" @change="changePeq(index, { enabled: ($event.target as HTMLInputElement).checked })" />{{ t('dsp.filter_on') }}</label>
              <button type="button" class="m3-chip sm danger" :aria-label="t('dsp.remove_filter', { number: index + 1 })" @click="removePeq(index)">{{ t('dsp.remove') }}</button></div>
            <CustomSelect :model-value="band.type" :options="peqTypes.map(type => ({ value: type, label: t(`dsp.filters.${type}`) }))" :label="t('dsp.filter_type')" @update:model-value="changePeq(index, { type: $event as ParametricEqBand['type'] })" />
            <div class="effects-grid">
              <DspNumberControl :label="t('dsp.filter_frequency', { number: index + 1 })" :model-value="band.frequencyHz" :min="20" :max="20000" :step="1" unit="Hz" @update:model-value="changePeq(index, { frequencyHz: $event })" />
              <DspNumberControl v-if="['peak', 'low_shelf', 'high_shelf'].includes(band.type)" :label="t('dsp.filter_gain', { number: index + 1 })" :model-value="band.gainDb" :min="-30" :max="24" :step="0.1" unit="dB" @update:model-value="changePeq(index, { gainDb: $event })" />
              <DspNumberControl :label="t('dsp.filter_q', { number: index + 1 })" :model-value="band.q" :min="0.1" :max="20" :step="0.001" unit="Q" @update:model-value="changePeq(index, { q: $event })" />
            </div>
          </div>
          <button type="button" :disabled="sound.parametricBands.length >= 10" class="m3-chip effects-add" @click="addPeq"><span class="material-symbols-rounded" aria-hidden="true">add</span>{{ t('dsp.add_filter') }} ({{ sound.parametricBands.length }}/10)</button>
          <details class="effects-import"><summary class="setting-title">{{ t('dsp.autoeq') }}</summary><p class="setting-desc">{{ t('dsp.autoeq_help') }}</p>
            <textarea v-model="autoEqText" :aria-label="t('dsp.autoeq_text')" :placeholder="t('dsp.autoeq_text')" spellcheck="false"></textarea>
            <div class="effects-toolbar"><label class="m3-chip file-button"><span class="material-symbols-rounded" aria-hidden="true">upload_file</span>{{ t('dsp.choose_file') }}<input type="file" accept=".txt,.csv,text/plain,text/csv" @change="readAutoEqFile" /></label><button type="button" class="m3-chip active" :disabled="!autoEqText.trim()" @click="importAutoEq">{{ t('dsp.apply_autoeq') }}</button></div>
            <p v-if="autoEqStatus" class="effects-status" :class="{ error: autoEqStatus.error }" role="status" aria-live="polite">{{ autoEqStatus.text }}</p>
          </details>
        </div>
      </details>

      <details v-for="section in sections" :key="section" class="effects-section">
        <summary>
          <span class="setting-icon-wrap"><span class="material-symbols-rounded" aria-hidden="true">{{ sectionIcons[section] }}</span></span>
          <span class="effects-summary-title">{{ t(`dsp.${section}_section`) }}</span>
          <button type="button" class="effects-text-btn" @click.prevent="player.resetAudioEffectsSection(section)">{{ t('dsp.reset_section') }}</button>
          <span class="material-symbols-rounded effects-chevron" aria-hidden="true">expand_more</span>
        </summary>
        <div class="effects-content">
          <div class="effects-heading"><div class="setting-info"><div class="setting-title">{{ t(`dsp.${section}_section`) }}</div><div class="setting-desc">{{ t(`dsp.${section}_help`) }}</div></div>
            <label class="m3-switch"><input type="checkbox" :checked="sound[`${section}Enabled`]" :aria-label="t(`dsp.${section}_section`)" @change="toggleSound(`${section}Enabled`, ($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"></span></span></label></div>
          <div v-if="soundSwitches[section].length" class="effects-rows">
            <div v-for="key in soundSwitches[section]" :key="key" class="effects-heading setting-card">
              <div class="setting-info"><div class="setting-title">{{ t(`dsp.controls.${key}`) }}</div><div class="setting-desc">{{ t(`dsp.help.${key}`) }}</div></div>
              <label class="m3-switch"><input type="checkbox" :checked="sound[key]" :aria-label="t(`dsp.controls.${key}`)" @change="toggleSound(key, ($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"></span></span></label>
            </div>
          </div>
          <div class="effects-grid">
            <DspNumberControl v-for="control in SOUND_CONTROLS.filter(item => item.section === section)" :key="control.key" :label="t(`dsp.controls.${control.key}`)" :help="t(`dsp.help.${control.key}`)" :model-value="sound[control.key]" :min="control.min" :max="control.max" :step="control.step" :unit="control.unit === 'percent' ? '%' : control.unit === 'ratio' ? 'x' : control.unit" :scale="control.unit === 'percent' ? 100 : 1" @update:model-value="changeSound(control.key, $event)" />
          </div>
        </div>
      </details>
    </div>

    <div class="section-label">{{ t('dsp.output_playback') }}</div>

    <details class="effects-section">
      <summary>
        <span class="setting-icon-wrap"><span class="material-symbols-rounded" aria-hidden="true">{{ sectionIcons.speaker }}</span></span>
        <span class="effects-summary-title">{{ t('dsp.speaker_section') }}</span>
        <button type="button" class="effects-text-btn" @click.prevent="player.resetAudioEffectsSection('speaker')">{{ t('dsp.reset_section') }}</button>
        <span class="material-symbols-rounded effects-chevron" aria-hidden="true">expand_more</span>
      </summary>
      <div class="effects-content"><div class="effects-heading"><div class="setting-info"><div class="setting-title">{{ t('dsp.speaker_section') }}</div><div class="setting-desc">{{ t('dsp.speaker_help') }}</div></div>
        <label class="m3-switch"><input type="checkbox" :checked="player.audioEffects.speaker.enabled" :aria-label="t('dsp.speaker_section')" @change="changeSpeaker('enabled', ($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"></span></span></label></div>
        <div class="effects-rows">
          <div class="setting-card effects-size-card"><div class="setting-info"><div class="setting-title">{{ t('dsp.speaker_size') }}</div><div v-if="mode === 'simple'" class="setting-desc">{{ t('dsp.speaker_simple_help') }}</div></div>
            <div class="effects-tabs effects-tabs--fit effects-size-options" role="group" :aria-label="t('dsp.speaker_size')">
              <button v-for="size in ['phone', 'tablet', 'large']" :key="size" type="button" class="effects-tab" :class="{ active: player.audioEffects.speaker.size === size }"
                :aria-pressed="player.audioEffects.speaker.size === size" @click="changeSpeaker('size', size)">{{ t(`dsp.speaker_sizes.${size}`) }}</button>
            </div>
          </div>
        </div>
        <div v-show="mode === 'advanced'" class="effects-grid effects-professional-control"><DspNumberControl v-for="control in speakerControls" :key="control.key" :label="t(`dsp.speaker_controls.${control.key}`)" :help="t(`dsp.speaker_help_controls.${control.key}`)" :model-value="Number(player.audioEffects.speaker[control.key])" :min="control.min" :max="control.max" :step="control.step" :unit="control.unit" :scale="control.scale" @update:model-value="changeSpeaker(control.key, $event)" /></div>
      </div>
    </details>

    <details class="effects-section">
      <summary>
        <span class="setting-icon-wrap"><span class="material-symbols-rounded" aria-hidden="true">{{ sectionIcons.rate }}</span></span>
        <span class="effects-summary-title">{{ t('dsp.rate_section') }}</span>
        <button type="button" class="effects-text-btn" @click.prevent="player.setSpeed(1); player.setPitch(1); player.setVinylEffect(false)">{{ t('dsp.reset_section') }}</button>
        <span class="material-symbols-rounded effects-chevron" aria-hidden="true">expand_more</span>
      </summary>
      <div class="effects-content">
        <div class="effects-rows">
          <div class="effects-heading setting-card"><div class="setting-info"><div class="setting-title">{{ t('player.vinyl_effect') }}</div><div class="setting-desc">{{ t('dsp.vinyl_help') }}</div><div v-if="player.vinylEffectEnabled" class="setting-desc effects-following">{{ t('dsp.vinyl_following', { value: player.effectivePlaybackPitch().toFixed(2) }) }}</div></div>
            <label class="m3-switch"><input type="checkbox" :checked="player.vinylEffectEnabled" :aria-label="t('player.vinyl_effect')" @change="player.setVinylEffect(($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"></span></span></label></div>
        </div>
        <div class="effects-grid">
          <template v-for="control in rateControls" :key="control.key"><DspNumberControl v-if="control.key === 'speed' || !player.vinylEffectEnabled" :label="t(control.title)" :help="t(control.desc)" :model-value="control.key === 'speed' ? player.playbackSpeed : player.playbackPitch" :min="control.min" :max="control.max" :step="0.05" unit="x" @update:model-value="setRate(control.key, $event)" /></template>
        </div>
        <div class="effects-tabs effects-speed-presets" role="group" :aria-label="t('player.playback_speed')">
          <button v-for="value in speedPresets" :key="value" type="button" class="effects-tab" :aria-label="`${t('player.playback_speed')} ${value}x`" :aria-pressed="player.playbackSpeed === value" :class="{ active: player.playbackSpeed === value }" @click="player.setSpeed(value)">{{ value }}x</button>
        </div>
        <div v-show="mode === 'advanced'" class="effects-grid effects-professional-control">
          <DspNumberControl :label="t('player.loudness_gain')" :help="t('settings.audio_effects_gain_desc')" :model-value="player.loudnessGainMb" :min="0" :max="1500" :step="50" :scale="0.01" unit="dB" @update:model-value="player.setLoudnessGain($event)" />
        </div>
      </div>
    </details>

    <details class="effects-section">
      <summary>
        <span class="setting-icon-wrap"><span class="material-symbols-rounded" aria-hidden="true">{{ sectionIcons.advanced }}</span></span>
        <span class="effects-summary-title">{{ t('dsp.advanced_section') }}</span>
        <span class="material-symbols-rounded effects-chevron" aria-hidden="true">expand_more</span>
      </summary>
      <div class="effects-content">
        <div class="effects-rows">
          <div class="effects-heading setting-card"><div class="setting-info"><div class="setting-title">{{ t('dsp.per_output') }}</div><div class="setting-desc">{{ t('dsp.per_output_help', { name: currentOutput }) }}</div></div>
            <label class="m3-switch"><input type="checkbox" :checked="player.audioEffects.perOutputEnabled" :aria-label="t('dsp.per_output')" @change="player.updateAudioEffects({ perOutputEnabled: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"></span></span></label></div>
          <div class="effects-heading setting-card"><div class="setting-info"><div class="setting-title">{{ t('dsp.auto_headroom') }}</div><div class="setting-desc">{{ t('dsp.auto_headroom_help') }}</div></div>
            <label class="m3-switch"><input type="checkbox" :checked="player.audioEffects.autoHeadroom" :aria-label="t('dsp.auto_headroom')" @change="player.updateAudioEffects({ autoHeadroom: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"></span></span></label></div>
          <div class="setting-card effects-quality"><div class="setting-info"><div class="setting-title">{{ t('dsp.quality') }}</div><p class="setting-desc">{{ t('dsp.quality_help') }}</p></div>
            <CustomSelect class="effects-select" :model-value="player.audioEffects.powerMode" :options="['eco', 'balanced', 'high'].map(mode => ({ value: mode, label: t(`dsp.quality_modes.${mode}`) }))" :label="t('dsp.quality')" @update:model-value="player.updateAudioEffects({ powerMode: $event as 'eco' | 'balanced' | 'high' })" /></div>
          <div class="setting-card setting-card--stack effects-stats"><div class="effects-stats-heading"><div class="setting-title">{{ t('dsp.runtime') }}</div>
              <span v-if="stats" class="effects-stats-state" :class="{ active: stats.active }">{{ t(stats.active ? 'dsp.runtime_active' : 'dsp.runtime_bypass') }}</span></div>
            <template v-if="stats"><p v-if="stats.inactiveReason" class="setting-desc">{{ t(`dsp.reasons.${stats.inactiveReason}`) }}</p>
              <dl class="effects-stats-grid">
                <div><dt>{{ t('dsp.cpu') }}</dt><dd>{{ stats.cpuLoadPercent.toFixed(2) }}%</dd></div>
                <div><dt>{{ t('dsp.sample_rate') }}</dt><dd>{{ stats.sampleRate }} Hz</dd></div>
                <div><dt>{{ t('dsp.compressor_reduction') }}</dt><dd>{{ stats.compressorReductionDb.toFixed(1) }} dB</dd></div>
                <div><dt>{{ t('dsp.limiter_reduction') }}</dt><dd>{{ stats.limiterReductionDb.toFixed(1) }} dB</dd></div>
              </dl>
            </template><p v-else class="setting-desc">{{ t('dsp.runtime_unavailable') }}</p></div>
          <div class="setting-card"><div class="setting-info"><div class="setting-title">{{ t('player.reset_effects') }}</div><p class="setting-desc">{{ t('settings.audio_effects_reset_desc') }}</p></div><button type="button" class="m3-chip danger" @click="player.resetAudioEffects()">{{ t('player.reset_effects') }}</button></div>
        </div>
      </div>
    </details>
  </div>
</template>

<style scoped lang="scss">
@use '@/components/settings/settingsPrimitives.scss' as primitives;
@include primitives.card;
@include primitives.card-body;
@include primitives.card-text;
@include primitives.chip;

.audio-effects-settings {
  --effects-card-bg: var(--md-surface-container-low);
  min-width: 0;
  container: audio-effects / inline-size;
}

/* 行本身不可点（只有开关、按钮可点），不做整行悬停染色；折叠区段内的卡片再降一级表面形成层次 */
.audio-effects-settings .setting-card,
.audio-effects-settings .setting-card:hover { background: var(--effects-card-bg); }

.setting-card--stack { flex-direction: column; align-items: stretch; gap: 14px; }
.card-heading { display: flex; align-items: center; gap: 14px; }
.effects-indent { padding-left: 50px; }
.effects-mode-switch, .effects-size-card, .effects-quality, .effects-save { flex-wrap: wrap; }
.effects-mode-switch { margin-top: 28px; }
.effects-following { color: var(--md-primary); }

/* 分区标题：左侧标题，右侧状态或次要操作 */
.effects-label {
  justify-content: space-between;
  gap: 12px;

  h3, .setting-title { margin: 0; color: inherit; font-size: inherit; font-weight: inherit; letter-spacing: normal; }
}
.effects-label-meta { font-size: var(--fs-12); font-weight: 500; text-align: right; overflow-wrap: anywhere; }
.effects-lead { margin: -2px 6px 12px; color: var(--md-on-surface-variant); font-size: var(--fs-12); line-height: 1.6; }
.effects-note {
  display: flex;
  gap: 6px;
  margin: 10px 6px 0;
  color: var(--md-on-surface-variant);
  font-size: var(--fs-12);
  line-height: 1.6;

  .material-symbols-rounded { flex-shrink: 0; margin-top: 1px; font-size: 16px; }
}

/* 分段选择（与 ui/SegmentedControl 同一外观）。这里按钮数量多、标签长短不一，
   用自适应宽度 + 横向滚动，选中态保持按钮语义（aria-pressed） */
.effects-tabs {
  display: flex;
  gap: 2px;
  min-width: 0;
  padding: 3px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-high);
  overflow-x: auto;
  scrollbar-width: none;

  &::-webkit-scrollbar { display: none; }
}
.effects-tabs--fit { flex-shrink: 0; max-width: 100%; }
.effects-tab {
  flex: 1 0 auto;
  height: 30px;
  padding: 0 14px;
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--md-on-surface-variant);
  font: inherit;
  font-size: var(--fs-13);
  font-weight: 500;
  letter-spacing: normal;
  white-space: nowrap;
  cursor: pointer;
  transition: color var(--duration-short) var(--ease-standard), background var(--duration-short) var(--ease-standard),
    box-shadow var(--duration-short) var(--ease-standard);

  &:hover:not(.active) { color: var(--md-on-surface); }
  &.active {
    background: var(--md-surface-container-lowest);
    color: var(--md-on-surface);
    font-weight: 600;
    box-shadow: var(--shadow-1), 0 0 0 0.5px var(--hairline);
  }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: -2px; }
}

/* 预设卡片 */
.effects-preset-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 8px; }
.effects-preset-item { position: relative; display: flex; min-width: 0; }
.effects-preset-card {
  position: relative;
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  min-height: 72px;
  padding: 12px 14px;
  border: 0;
  border-radius: var(--radius-md);
  background: var(--md-surface-container);
  color: var(--md-on-surface);
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition: background var(--duration-short) var(--ease-standard), box-shadow var(--duration-short) var(--ease-standard);

  strong { padding-right: 20px; font-size: var(--fs-13); font-weight: 600; }
  span { color: var(--md-on-surface-variant); font-size: var(--fs-11); line-height: 1.45; overflow-wrap: anywhere; }

  &:hover { background: color-mix(in srgb, var(--md-on-surface) 5%, var(--md-surface-container)); }
  &.active {
    background: color-mix(in srgb, var(--md-primary) 12%, var(--md-surface-container-low));
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--md-primary) 60%, transparent);

    &::after {
      content: 'check_circle';
      position: absolute;
      top: 10px;
      right: 10px;
      color: var(--md-primary);
      font-family: 'Material Symbols Rounded';
      font-size: 18px;
      font-variation-settings: 'FILL' 1;
      line-height: 1;
    }
  }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 2px; }
}
.effects-preset-delete {
  position: absolute;
  right: 6px;
  bottom: 6px;
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--md-on-surface-variant);
  cursor: pointer;
  transition: background var(--duration-short), color var(--duration-short);

  .material-symbols-rounded { font-size: 18px; }
  &:hover { background: color-mix(in srgb, var(--md-error) 12%, transparent); color: var(--md-error); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 1px; }
}
.effects-empty {
  margin: 0;
  padding: 18px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container);
  color: var(--md-on-surface-variant);
  font-size: var(--fs-13);
  text-align: center;
}
.effects-panel > .setting-desc { margin: 0; }

/* 按钮 / 输入 */
.m3-chip {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;

  .material-symbols-rounded { margin-left: -2px; font-size: 18px; }
}
.m3-chip.active:hover:not(:disabled) { background: color-mix(in srgb, var(--md-primary) 20%, var(--md-surface-container-low)); }
.effects-text-btn {
  flex-shrink: 0;
  height: 30px;
  padding: 0 12px;
  border: 0;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--md-primary);
  font: inherit;
  font-size: var(--fs-13);
  font-weight: 600;
  white-space: nowrap;
  cursor: pointer;
  transition: background var(--duration-short);

  &:hover { background: color-mix(in srgb, var(--md-primary) 10%, transparent); }
  &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: 1px; }
}
.effects-text-input, textarea {
  box-sizing: border-box;
  border: 1px solid transparent;
  background: var(--md-surface-container-high);
  color: var(--md-on-surface);
  font: inherit;
  font-size: var(--fs-13);
  outline: none;
  transition: border-color var(--duration-short), background var(--duration-short), box-shadow var(--duration-short);

  &::placeholder { color: var(--md-on-surface-variant); opacity: 0.8; }
  &:focus {
    border-color: var(--md-primary);
    background: var(--md-surface-container-lowest);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--md-primary) 18%, transparent);
  }
}
.effects-text-input { flex: 1 1 180px; min-width: 0; height: 36px; padding: 0 14px; border-radius: var(--radius-full); }
textarea {
  display: block;
  width: 100%;
  min-height: 130px;
  margin: 10px 0;
  padding: 10px 12px;
  border-radius: var(--radius-md);
  font-family: var(--font-family-mono, monospace);
  font-size: var(--fs-12);
  line-height: 1.5;
  resize: vertical;
}
.file-button { position: relative; overflow: hidden; cursor: pointer; }
.file-button input { position: absolute; inset: 0; width: 100%; opacity: 0; cursor: pointer; }
.file-button:focus-within { outline: 2px solid var(--md-primary); outline-offset: 2px; }
.effects-toolbar { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
.effects-transfer-actions { display: flex; flex-wrap: wrap; gap: 8px; }
.effects-paste { display: flex; flex-direction: column; gap: 8px; animation: effects-reveal .2s ease-out; }
.effects-paste textarea { min-height: 96px; margin: 0; }
.effects-status { margin: 0; color: var(--md-primary); font-size: var(--fs-12); line-height: 1.5; animation: effects-reveal .2s ease-out; }
.effects-status.error { color: var(--md-error); }
@keyframes effects-reveal { from { opacity: 0; transform: translateY(-4px); } }

/* 数值控件网格：容器底色是细线色，1px 间隙就是格线，单列/双列都成为一个分组 */
.effects-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 1px;
  overflow: hidden;
  border-radius: var(--radius-lg);
  background: var(--hairline);
  box-shadow: 0 0 0 1px var(--hairline);
}
/* 网格里的控件自成分格，不参与 primitives.card 的相邻行合组（选择器与其一一对应以压过优先级） */
.effects-grid > .setting-card:not(.sub-card),
.effects-grid > .setting-card:not(.sub-card):has(+ .setting-card:not(.sub-card)),
.effects-grid > .setting-card:not(.sub-card) + .setting-card:not(.sub-card),
.effects-grid > .setting-card:not(.sub-card) + .setting-card:not(.sub-card):has(+ .setting-card:not(.sub-card)) {
  margin: 0;
  border-radius: 0;
  box-shadow: none;
}
.effects-grid > .setting-card:not(.sub-card) + .setting-card:not(.sub-card)::before { content: none; }
@container audio-effects (min-width: 540px) {
  .effects-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .effects-grid > :last-child:nth-child(odd), .effects-grid > .listening-space { grid-column: 1 / -1; }
}

/* 折叠区段内的开关行：没有图标，分隔线从左边距开始 */
.effects-rows > .setting-card:not(.sub-card) + .setting-card:not(.sub-card)::before { left: 18px; }
.effects-rows > .setting-card:last-child { margin-bottom: 0; }
.effects-heading { display: flex; align-items: center; justify-content: space-between; gap: 14px; }
.effects-content > .effects-heading { padding: 2px 2px 0; }

/* 折叠区段（专业调节与输出、播放相关的分组卡片） */
details.effects-section {
  --effects-card-bg: var(--md-surface-container-lowest);
  margin-bottom: 8px;
  border-radius: var(--radius-lg);
  background: var(--md-surface-container-low);
  box-shadow: 0 0 0 1px var(--hairline);

  > summary {
    display: flex;
    align-items: center;
    gap: 14px;
    min-height: 64px;
    padding: 12px 14px 12px 18px;
    border-radius: inherit;
    list-style: none;
    cursor: pointer;
    user-select: none;
    transition: background var(--duration-short);

    &::-webkit-details-marker { display: none; }
    &:hover { background: color-mix(in srgb, var(--md-on-surface) 2.5%, transparent); }
    &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: -2px; }
  }

  &[open] > summary { border-bottom-right-radius: 0; border-bottom-left-radius: 0; }
  &[open] > summary .effects-chevron { transform: rotate(180deg); }
}
.effects-summary-title { flex: 1; min-width: 0; font-size: var(--fs-14); font-weight: 600; letter-spacing: var(--tracking-title); }
.effects-chevron { flex-shrink: 0; color: var(--md-on-surface-variant); font-size: 22px; transition: transform var(--duration-medium) var(--ease-standard); }
.effects-content { display: flex; flex-direction: column; gap: 14px; padding: 4px 18px 18px; }
.effects-add { align-self: flex-start; }

.peq-band {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 14px;
  border-radius: var(--radius-md);
  background: var(--effects-card-bg);
  box-shadow: 0 0 0 1px var(--hairline);

  strong { font-size: var(--fs-13); font-weight: 600; }
  .effects-toolbar .m3-chip { margin-left: auto; }
}
.peq-enabled { display: flex; align-items: center; gap: 6px; font-size: var(--fs-12); cursor: pointer; }

.effects-import {
  padding: 0 14px;
  border-radius: var(--radius-md);
  background: var(--effects-card-bg);
  box-shadow: 0 0 0 1px var(--hairline);

  > summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 46px;
    color: var(--md-primary);
    font-size: var(--fs-13);
    list-style: none;
    cursor: pointer;

    &::-webkit-details-marker { display: none; }
    &::after {
      content: 'expand_more';
      font-family: 'Material Symbols Rounded';
      font-size: 20px;
      font-weight: 400;
      transition: transform var(--duration-medium) var(--ease-standard);
    }
    &:focus-visible { outline: 2px solid var(--md-primary); outline-offset: -2px; border-radius: var(--radius-sm); }
  }

  &[open] { padding-bottom: 14px; }
  &[open] > summary::after { transform: rotate(180deg); }
}

.effects-select { flex: 0 1 200px; min-width: 160px; margin-left: auto; }
.effects-stats-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.effects-stats-state {
  padding: 2px 10px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container-high);
  color: var(--md-on-surface-variant);
  font-size: var(--fs-12);
  font-weight: 600;

  &.active { background: color-mix(in srgb, var(--md-primary) 14%, transparent); color: var(--md-primary); }
}
.effects-stats-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
  margin: 0;

  > div { padding: 10px 12px; border-radius: var(--radius-md); background: var(--md-surface-container); }
  dt { color: var(--md-on-surface-variant); font-size: var(--fs-11); }
  dd { margin: 2px 0 0; color: var(--md-on-surface); font-size: var(--fs-14); font-weight: 600; font-variant-numeric: tabular-nums; }
}
.effects-stats > .setting-desc { margin: 0; }

@container audio-effects (max-width: 560px) {
  .effects-indent { padding-left: 0; }
  .effects-content { padding: 4px 12px 12px; }
  .effects-tabs--fit { flex: 1 1 100%; }
  .effects-select { flex-basis: 100%; margin-left: 0; }
}
</style>
