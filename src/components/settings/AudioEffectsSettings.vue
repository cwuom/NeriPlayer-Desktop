<script setup lang="ts">
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
    await navigator.clipboard.writeText(player.exportAudioEffectsPreset(currentUserPreset.value?.id))
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
      <div class="effects-heading">
        <div class="setting-info"><div class="setting-title">{{ t('dsp.enabled') }}</div><div class="setting-desc">{{ t('dsp.enabled_help') }}</div></div>
        <label class="m3-switch"><input type="checkbox" :checked="player.audioEffectsProfile.enabled" :aria-label="t('dsp.enabled')"
          @change="player.updateAudioEffectsProfile({ enabled: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"></span></span></label>
      </div>
      <div class="effects-toolbar">
        <button :class="{ active: player.audioEffectsProfile.enabled }" :aria-pressed="player.audioEffectsProfile.enabled" @click="player.updateAudioEffectsProfile({ enabled: !player.audioEffectsProfile.enabled })">{{ t(player.audioEffectsProfile.enabled ? 'dsp.compare_original' : 'dsp.compare_effect') }}</button>
        <span class="output-label">{{ t('dsp.current_output', { name: currentOutput }) }}</span>
      </div>
    </div>

    <section class="effects-section effects-preset-section" :aria-label="t('dsp.preset_section')">
      <div class="effects-section-heading"><h3 class="setting-title">{{ t('dsp.preset_section') }}</h3><span>{{ t('dsp.current_preset', { name: presetLabel }) }}</span></div>
      <div class="effects-content">
        <div class="effects-presets"><button v-for="item in categories" :key="item" :class="{ active: category === item }" :aria-pressed="category === item" @click="category = item">{{ t(`dsp.categories.${item}`) }}</button></div>
        <div class="effects-preset-grid" role="group" :aria-label="t('player.eq_preset')">
          <div v-for="preset in presetOptions" :key="preset.value" class="effects-preset-item">
            <button class="effects-preset-card"
              :class="{ active: player.audioEffectsProfile.presetId === preset.value }"
              :aria-label="preset.label" :aria-pressed="player.audioEffectsProfile.presetId === preset.value"
              @click="player.setAudioEffectsPreset(preset.value)"><strong>{{ preset.label }}</strong><span v-if="preset.description">{{ preset.description }}</span></button>
            <button v-if="preset.user" class="effects-preset-delete danger" :aria-label="`${t('dsp.delete_preset')} ${preset.label}`"
              @click="player.deleteAudioEffectsPreset(preset.value)"><span class="material-symbols-rounded" aria-hidden="true">delete</span>{{ t('dsp.remove') }}</button>
          </div>
        </div>
        <p v-if="!presetOptions.length" class="setting-desc">{{ t('dsp.no_user_presets') }}</p>
        <p class="setting-desc">{{ t('dsp.preset_help') }}</p>
        <div class="effects-toolbar"><input v-model="presetName" class="effects-text-input" maxlength="24" :placeholder="t('dsp.preset_name')" :aria-label="t('dsp.preset_name')" />
          <button :disabled="!presetName.trim() || player.audioEffects.userPresets.length >= 50" @click="savePreset">{{ t('dsp.save_preset') }}</button></div>
        <div class="effects-transfer">
          <div class="effects-transfer-heading">
            <div class="setting-title">{{ t('dsp.transfer.title') }}</div>
            <div class="setting-desc">{{ t('dsp.transfer.help') }}</div>
          </div>
          <div class="effects-transfer-actions">
            <label class="file-button"><span class="material-symbols-rounded" aria-hidden="true">upload_file</span>{{ t('dsp.transfer.from_file') }}
              <input type="file" accept=".json,.txt,application/json,text/plain" :aria-label="t('dsp.transfer.from_file')" @change="importPresetFile" /></label>
            <button :class="{ active: pasteOpen }" :aria-expanded="pasteOpen" @click="pasteOpen = !pasteOpen">
              <span class="material-symbols-rounded" aria-hidden="true">content_paste</span>{{ t('dsp.transfer.paste') }}</button>
            <button @click="copyShareCode"><span class="material-symbols-rounded" aria-hidden="true">{{ copied ? 'check' : 'content_copy' }}</span>{{ t(copied ? 'dsp.transfer.copied' : 'dsp.transfer.copy') }}</button>
          </div>
          <div v-if="pasteOpen" class="effects-paste">
            <textarea v-model="pasteText" :aria-label="t('dsp.transfer.paste_placeholder')" :placeholder="t('dsp.transfer.paste_placeholder')" spellcheck="false"></textarea>
            <div class="effects-toolbar"><button class="active" :disabled="!pasteText.trim()" @click="runImport(pasteText)">{{ t('dsp.transfer.import') }}</button>
              <button @click="pasteOpen = false; pasteText = ''">{{ t('common.cancel') }}</button></div>
          </div>
          <p v-if="presetStatus" class="effects-status" :class="{ error: presetStatus.error }" role="status" aria-live="polite">{{ presetStatus.text }}</p>
        </div>
      </div>
    </section>

    <div class="effects-mode-switch setting-card" role="group" :aria-label="t('dsp.adjustment_mode')">
      <button class="setting-title" :class="{ active: mode === 'simple' }" :aria-pressed="mode === 'simple'" @click="mode = 'simple'">{{ t('dsp.simple_mode') }}</button>
      <button class="setting-title" :class="{ active: mode === 'advanced' }" :aria-pressed="mode === 'advanced'" @click="mode = 'advanced'">{{ t('dsp.expert_mode') }}</button>
    </div>

    <section v-show="mode === 'simple'" class="effects-section effects-listening" :aria-label="t('dsp.listening.title')">
      <div class="effects-section-heading"><h3>{{ t('dsp.listening.title') }}</h3><button @click="player.resetListeningTuning()">{{ t('dsp.listening.reset') }}</button></div>
      <div class="effects-content"><p class="setting-desc">{{ t('dsp.listening.help') }}</p>
        <div class="effects-grid"><DspNumberControl v-for="control in LISTENING_CONTROLS" :key="control.key"
          :class="{ 'listening-space': control.key === 'space' }" :label="t(`dsp.listening.${control.key}.title`)"
          :help="t(`dsp.listening.${control.key}.help`)" :model-value="player.listeningTuning[control.key]"
          :min="control.min" :max="100" :step="1" unit="%" :value-label="listeningLabel(control.key)"
          :min-label="t(`dsp.listening.${control.key}.left`)" :max-label="t(`dsp.listening.${control.key}.right`)"
          @update:model-value="player.setListeningTuning(control.key, $event)" /></div>
        <p class="effects-listening-note">{{ t('dsp.listening.protection') }}</p>
      </div>
    </section>

    <div v-show="mode === 'advanced'" class="effects-professional">

    <details class="effects-section" open>
      <summary>{{ t('dsp.equalizer_section') }}<button @click.prevent="player.resetAudioEffectsSection('equalizer')">{{ t('dsp.reset_section') }}</button></summary>
      <div class="effects-content">
        <DspFrequencyGraph :sound="sound" />
        <DspNumberControl v-for="control in SOUND_CONTROLS.filter(item => item.section === 'equalizer')" :key="control.key" :label="t(`dsp.controls.${control.key}`)" :help="t(`dsp.help.${control.key}`)" :model-value="sound[control.key]" :min="control.min" :max="control.max" :step="control.step" :unit="control.unit" @update:model-value="changeSound(control.key, $event)" />
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
            <button class="danger" :aria-label="t('dsp.remove_filter', { number: index + 1 })" @click="removePeq(index)">{{ t('dsp.remove') }}</button></div>
          <CustomSelect :model-value="band.type" :options="peqTypes.map(type => ({ value: type, label: t(`dsp.filters.${type}`) }))" :label="t('dsp.filter_type')" @update:model-value="changePeq(index, { type: $event as ParametricEqBand['type'] })" />
          <div class="effects-grid">
            <DspNumberControl :label="t('dsp.filter_frequency', { number: index + 1 })" :model-value="band.frequencyHz" :min="20" :max="20000" :step="1" unit="Hz" @update:model-value="changePeq(index, { frequencyHz: $event })" />
            <DspNumberControl v-if="['peak', 'low_shelf', 'high_shelf'].includes(band.type)" :label="t('dsp.filter_gain', { number: index + 1 })" :model-value="band.gainDb" :min="-30" :max="24" :step="0.1" unit="dB" @update:model-value="changePeq(index, { gainDb: $event })" />
            <DspNumberControl :label="t('dsp.filter_q', { number: index + 1 })" :model-value="band.q" :min="0.1" :max="20" :step="0.001" unit="Q" @update:model-value="changePeq(index, { q: $event })" />
          </div>
        </div>
        <button :disabled="sound.parametricBands.length >= 10" class="effects-add" @click="addPeq">{{ t('dsp.add_filter') }} ({{ sound.parametricBands.length }}/10)</button>
        <details class="effects-import"><summary class="setting-title">{{ t('dsp.autoeq') }}</summary><p class="setting-desc">{{ t('dsp.autoeq_help') }}</p>
          <textarea v-model="autoEqText" :aria-label="t('dsp.autoeq_text')" :placeholder="t('dsp.autoeq_text')" spellcheck="false"></textarea>
          <div class="effects-toolbar"><label class="file-button">{{ t('dsp.choose_file') }}<input type="file" accept=".txt,.csv,text/plain,text/csv" @change="readAutoEqFile" /></label><button :disabled="!autoEqText.trim()" @click="importAutoEq">{{ t('dsp.apply_autoeq') }}</button></div>
          <p v-if="autoEqStatus" class="effects-status" :class="{ error: autoEqStatus.error }" role="status" aria-live="polite">{{ autoEqStatus.text }}</p>
        </details>
      </div>
    </details>

    <details v-for="section in sections" :key="section" class="effects-section">
      <summary>{{ t(`dsp.${section}_section`) }}<button @click.prevent="player.resetAudioEffectsSection(section)">{{ t('dsp.reset_section') }}</button></summary>
      <div class="effects-content">
        <div class="effects-heading"><div class="setting-info"><div class="setting-title">{{ t(`dsp.${section}_section`) }}</div><div class="setting-desc">{{ t(`dsp.${section}_help`) }}</div></div>
          <label class="m3-switch"><input type="checkbox" :checked="sound[`${section}Enabled`]" :aria-label="t(`dsp.${section}_section`)" @change="toggleSound(`${section}Enabled`, ($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"></span></span></label></div>
        <div v-for="key in soundSwitches[section]" :key="key" class="effects-heading setting-card">
          <div class="setting-info"><div class="setting-title">{{ t(`dsp.controls.${key}`) }}</div><div class="setting-desc">{{ t(`dsp.help.${key}`) }}</div></div>
          <label class="m3-switch"><input type="checkbox" :checked="sound[key]" :aria-label="t(`dsp.controls.${key}`)" @change="toggleSound(key, ($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"></span></span></label>
        </div>
        <div class="effects-grid">
          <DspNumberControl v-for="control in SOUND_CONTROLS.filter(item => item.section === section)" :key="control.key" :label="t(`dsp.controls.${control.key}`)" :help="t(`dsp.help.${control.key}`)" :model-value="sound[control.key]" :min="control.min" :max="control.max" :step="control.step" :unit="control.unit === 'percent' ? '%' : control.unit === 'ratio' ? 'x' : control.unit" :scale="control.unit === 'percent' ? 100 : 1" @update:model-value="changeSound(control.key, $event)" />
        </div>
      </div>
    </details>

    </div>

    <details class="effects-section">
      <summary>{{ t('dsp.speaker_section') }}<button @click.prevent="player.resetAudioEffectsSection('speaker')">{{ t('dsp.reset_section') }}</button></summary>
      <div class="effects-content"><div class="effects-heading"><div class="setting-info"><div class="setting-title">{{ t('dsp.speaker_section') }}</div><div class="setting-desc">{{ t('dsp.speaker_help') }}</div></div>
        <label class="m3-switch"><input type="checkbox" :checked="player.audioEffects.speaker.enabled" :aria-label="t('dsp.speaker_section')" @change="changeSpeaker('enabled', ($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"></span></span></label></div>
        <div class="setting-card"><div class="setting-title">{{ t('dsp.speaker_size') }}</div>
          <div class="effects-size-options" role="group" :aria-label="t('dsp.speaker_size')">
            <button v-for="size in ['phone', 'tablet', 'large']" :key="size" :class="{ active: player.audioEffects.speaker.size === size }"
              :aria-pressed="player.audioEffects.speaker.size === size" @click="changeSpeaker('size', size)">{{ t(`dsp.speaker_sizes.${size}`) }}</button>
          </div>
        </div>
        <p v-if="mode === 'simple'" class="setting-desc">{{ t('dsp.speaker_simple_help') }}</p>
        <div v-show="mode === 'advanced'" class="effects-grid effects-professional-control"><DspNumberControl v-for="control in speakerControls" :key="control.key" :label="t(`dsp.speaker_controls.${control.key}`)" :help="t(`dsp.speaker_help_controls.${control.key}`)" :model-value="Number(player.audioEffects.speaker[control.key])" :min="control.min" :max="control.max" :step="control.step" :unit="control.unit" :scale="control.scale" @update:model-value="changeSpeaker(control.key, $event)" /></div>
      </div>
    </details>

    <details class="effects-section">
      <summary>{{ t('dsp.rate_section') }}<button @click.prevent="player.setSpeed(1); player.setPitch(1); player.setVinylEffect(false)">{{ t('dsp.reset_section') }}</button></summary>
      <div class="effects-content"><div class="effects-heading setting-card"><div class="setting-info"><div class="setting-title">{{ t('player.vinyl_effect') }}</div><div class="setting-desc">{{ t('dsp.vinyl_help') }}</div></div>
        <label class="m3-switch"><input type="checkbox" :checked="player.vinylEffectEnabled" :aria-label="t('player.vinyl_effect')" @change="player.setVinylEffect(($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"></span></span></label></div>
        <p v-if="player.vinylEffectEnabled" class="setting-desc">{{ t('dsp.vinyl_following', { value: player.effectivePlaybackPitch().toFixed(2) }) }}</p>
        <template v-for="control in rateControls" :key="control.key"><DspNumberControl v-if="control.key === 'speed' || !player.vinylEffectEnabled" :label="t(control.title)" :help="t(control.desc)" :model-value="control.key === 'speed' ? player.playbackSpeed : player.playbackPitch" :min="control.min" :max="control.max" :step="0.05" unit="x" @update:model-value="setRate(control.key, $event)" /></template>
        <div class="effects-presets"><button v-for="value in speedPresets" :key="value" :aria-label="`${t('player.playback_speed')} ${value}x`" :aria-pressed="player.playbackSpeed === value" :class="{ active: player.playbackSpeed === value }" @click="player.setSpeed(value)">{{ value }}x</button></div>
        <DspNumberControl v-show="mode === 'advanced'" class="effects-professional-control" :label="t('player.loudness_gain')" :help="t('settings.audio_effects_gain_desc')" :model-value="player.loudnessGainMb" :min="0" :max="1500" :step="50" :scale="0.01" unit="dB" @update:model-value="player.setLoudnessGain($event)" />
      </div>
    </details>

    <details class="effects-section">
      <summary>{{ t('dsp.advanced_section') }}</summary>
      <div class="effects-content"><div class="effects-heading setting-card"><div class="setting-info"><div class="setting-title">{{ t('dsp.per_output') }}</div><div class="setting-desc">{{ t('dsp.per_output_help', { name: currentOutput }) }}</div></div>
        <label class="m3-switch"><input type="checkbox" :checked="player.audioEffects.perOutputEnabled" :aria-label="t('dsp.per_output')" @change="player.updateAudioEffects({ perOutputEnabled: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"></span></span></label></div>
        <div class="effects-heading setting-card"><div class="setting-info"><div class="setting-title">{{ t('dsp.auto_headroom') }}</div><div class="setting-desc">{{ t('dsp.auto_headroom_help') }}</div></div>
          <label class="m3-switch"><input type="checkbox" :checked="player.audioEffects.autoHeadroom" :aria-label="t('dsp.auto_headroom')" @change="player.updateAudioEffects({ autoHeadroom: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"></span></span></label></div>
        <div class="setting-card"><div class="setting-title">{{ t('dsp.quality') }}</div><p class="setting-desc">{{ t('dsp.quality_help') }}</p>
          <CustomSelect :model-value="player.audioEffects.powerMode" :options="['eco', 'balanced', 'high'].map(mode => ({ value: mode, label: t(`dsp.quality_modes.${mode}`) }))" :label="t('dsp.quality')" @update:model-value="player.updateAudioEffects({ powerMode: $event as 'eco' | 'balanced' | 'high' })" /></div>
        <div class="setting-card effects-stats"><div class="setting-title">{{ t('dsp.runtime') }}</div>
          <template v-if="stats"><span>{{ t(stats.active ? 'dsp.runtime_active' : 'dsp.runtime_bypass') }}</span><span v-if="stats.inactiveReason">{{ t(`dsp.reasons.${stats.inactiveReason}`) }}</span>
            <div class="effects-stats-grid"><span>{{ t('dsp.cpu') }} <b>{{ stats.cpuLoadPercent.toFixed(2) }}%</b></span><span>{{ t('dsp.sample_rate') }} <b>{{ stats.sampleRate }} Hz</b></span><span>{{ t('dsp.compressor_reduction') }} <b>{{ stats.compressorReductionDb.toFixed(1) }} dB</b></span><span>{{ t('dsp.limiter_reduction') }} <b>{{ stats.limiterReductionDb.toFixed(1) }} dB</b></span></div>
          </template><p v-else class="setting-desc">{{ t('dsp.runtime_unavailable') }}</p></div>
        <div class="setting-card"><div class="setting-title">{{ t('player.reset_effects') }}</div><p class="setting-desc">{{ t('settings.audio_effects_reset_desc') }}</p><button class="danger" @click="player.resetAudioEffects()">{{ t('player.reset_effects') }}</button></div>
      </div>
    </details>
  </div>
</template>

<style scoped lang="scss">
.audio-effects-settings { min-width: 0; }
.setting-card { padding: 16px; border-radius: var(--radius-md); background: var(--md-surface-container); }
.setting-title { font-size: 14px; font-weight: 500; }
.setting-desc { margin: 4px 0 0; color: var(--md-on-surface-variant); font-size: 12px; line-height: 1.6; }
.effects-heading, .effects-toolbar { display: flex; gap: 12px; align-items: center; }
.effects-heading { justify-content: space-between; }
.effects-toolbar { flex-wrap: wrap; }
.effects-master { margin-bottom: 10px; }
.effects-master .effects-toolbar { margin-top: 12px; }
.effects-section-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 14px 16px; background: var(--md-surface-container-low); }
.effects-section-heading h3 { margin: 0; font-size: 14px; }
.effects-section-heading > span { font-size: 12px; color: var(--md-on-surface-variant); overflow-wrap: anywhere; }
.effects-preset-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; }
.effects-preset-card { display: flex; flex-direction: column; gap: 6px; padding: 12px; min-width: 0; min-height: 74px; text-align: left; }
.effects-preset-card strong { font-size: 13px; font-weight: 600; }
.effects-preset-card span { font-size: 11px; line-height: 1.45; color: var(--md-on-surface-variant); overflow-wrap: anywhere; }
.effects-preset-card.active span { color: inherit; }
.effects-mode-switch { display: flex; gap: 8px; margin: 20px 0 10px; }
.effects-mode-switch button { flex: 1; padding: 11px; font-weight: 500; }
.effects-size-options { display: flex; gap: 8px; margin-top: 12px; }
.effects-size-options button { flex: 1; }
.listening-space { grid-column: 1 / -1; }
.effects-listening-note { margin: 0; font-size: 11px; line-height: 1.6; color: var(--md-on-surface-variant); }
.setting-info { flex: 1; min-width: 0; }
.output-label { font-size: 11px; color: var(--md-on-surface-variant); overflow-wrap: anywhere; }
.effects-section { margin: 10px 0; border: 1px solid var(--md-outline-variant); border-radius: var(--radius-lg); overflow: hidden; }
.effects-section > summary { display: flex; align-items: center; gap: 8px; padding: 14px 16px; cursor: pointer; font-size: 14px; font-weight: 600; list-style: none; background: var(--md-surface-container-low); }
.effects-section > summary::before { content: 'chevron_right'; font-family: 'Material Symbols Rounded'; font-size: 22px; font-weight: 400; transition: transform .15s; }
.effects-section[open] > summary::before { transform: rotate(90deg); }
summary:focus-visible { outline: 2px solid var(--md-primary); outline-offset: -3px; }
.effects-section > summary > button { margin-left: auto; font-weight: 400; }
.effects-content { display: flex; flex-direction: column; gap: 14px; padding: 16px; }
.effects-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
.effects-presets { display: flex; flex-wrap: wrap; gap: 6px; }
button, .file-button { padding: 8px 12px; border: 1px solid var(--md-outline-variant); border-radius: var(--radius-sm); background: transparent; color: var(--md-on-surface-variant); font: inherit; font-size: 12px; cursor: pointer; text-align: center; }
button:hover, .file-button:hover { background: var(--md-surface-container-highest); }
button:focus-visible, .file-button:focus-within { outline: 2px solid var(--md-primary); outline-offset: 2px; }
button.active { border-color: transparent; background: var(--md-primary-container); color: var(--md-on-primary-container); }
button:disabled { opacity: .45; cursor: default; }
button.danger { color: var(--md-error); }
.effects-add { align-self: flex-start; }
.peq-band { display: flex; flex-direction: column; gap: 12px; padding: 12px; border: 1px solid var(--md-outline-variant); border-radius: var(--radius-md); }
.peq-band strong { font-size: 13px; font-weight: 500; }
.peq-band .effects-toolbar button { margin-left: auto; }
.peq-enabled { display: flex; align-items: center; gap: 6px; font-size: 12px; cursor: pointer; }
.peq-enabled input { accent-color: var(--md-primary); }
.effects-text-input, textarea { border: 1px solid var(--md-outline-variant); border-radius: var(--radius-sm); background: var(--md-surface-container); color: var(--md-on-surface); font: inherit; font-size: 12px; padding: 10px; }
.effects-text-input { flex: 1; min-width: 140px; }
.effects-text-input:focus, textarea:focus { outline: 2px solid var(--md-primary); outline-offset: 1px; }
.effects-import summary { color: var(--md-primary); font-size: 12px; cursor: pointer; padding: 6px 0; }
textarea { display: block; box-sizing: border-box; width: 100%; min-height: 130px; resize: vertical; margin: 10px 0; font-family: var(--font-family-mono, monospace); line-height: 1.5; }
.file-button { position: relative; overflow: hidden; }
.file-button input { position: absolute; inset: 0; opacity: 0; width: 100%; cursor: pointer; }
.effects-stats { font-size: 12px; color: var(--md-on-surface-variant); }
.effects-stats > span { display: inline-block; margin: 8px 10px 0 0; }
.effects-stats-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; margin-top: 12px; }
.effects-stats-grid b { color: var(--md-on-surface); font-weight: 500; font-variant-numeric: tabular-nums; }
.effects-preset-item { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
.effects-preset-item .effects-preset-card { flex: 1; }
.effects-preset-delete { display: inline-flex; align-items: center; justify-content: center; gap: 4px; padding: 5px 10px; }
.effects-preset-delete .material-symbols-rounded { font-size: 16px; }
.effects-transfer { display: flex; flex-direction: column; gap: 12px; padding: 14px; border-radius: var(--radius-md); background: var(--md-surface-container); }
.effects-transfer-actions { display: flex; flex-wrap: wrap; gap: 8px; }
.effects-transfer-actions > button, .effects-transfer-actions > .file-button { display: inline-flex; align-items: center; gap: 6px; }
.effects-transfer-actions .material-symbols-rounded { font-size: 18px; }
.effects-paste { display: flex; flex-direction: column; gap: 8px; animation: effects-reveal .2s ease-out; }
.effects-paste textarea { min-height: 96px; margin: 0; }
@keyframes effects-reveal { from { opacity: 0; transform: translateY(-4px); } }
.effects-status { margin: 0; color: var(--md-primary); font-size: 12px; line-height: 1.5; animation: effects-reveal .2s ease-out; }
.effects-status.error { color: var(--md-error); }
@media (max-width: 680px) { .effects-grid, .effects-stats-grid { grid-template-columns: minmax(0, 1fr); } .effects-preset-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } .effects-content { padding: 12px; } .effects-section-heading { flex-wrap: wrap; } }
</style>
