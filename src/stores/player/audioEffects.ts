import { computed, ref, watch, type Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import i18n from '@/i18n'
import {
  applyAudioEffectPreset,
  AUDIO_EFFECT_PRESETS,
  ANDROID_OUTPUT_ROUTES,
  audioEffectsProfileFor,
  DEFAULT_AUDIO_EFFECTS_SETTINGS,
  exportAudioEffectsProfile,
  importAudioEffects,
  importAutoEq as parseAutoEq,
  normalizeAudioEffectsSettings,
  normalizeAudioEffectsSound,
  replaceAudioEffectsProfile,
  resetAudioEffectsSoundSection,
  type AudioEffectsProfile,
  type AudioEffectsSettings,
  type AudioEffectsSound,
  type SoundSection,
} from '@/modules/audioEffects/model'
import {
  applyListeningTuning,
  LISTENING_CONTROLS,
  listeningTuningValues,
  resetListeningTuning as resetListeningTuningSound,
  type ListeningControlKey,
} from '@/modules/audioEffects/listening'
import { EQ_PRESETS } from '@/modules/audioEffects/eqPresets'
import { setDecodableCodecs } from '@/modules/playback/playbackSource'
import { createLogger } from '@/utils/logger'
import type { AudioEffectsRuntimeStats, DecoderCapabilities, PlayerSettings } from './types'

const log = createLogger('player')

export interface AudioEffectsContext {
  settings: PlayerSettings
  volume: Ref<number>
  playbackSpeed: Ref<number>
  playbackPitch: Ref<number>
  vinylEffectEnabled: Ref<boolean>
  savePlayerState: () => void
  effectivePlaybackSpeed: () => number
  applyPlaybackParameter: (parameter: 'speed' | 'pitch' | 'loudness') => Promise<void>
  scheduleNativeAudioEffects: () => void
  setSpeed: (spd: number) => Promise<void>
  setPitch: (pitch: number) => Promise<void>
  setVinylEffect: (enabled: boolean) => Promise<void>
}

/**
 * 音量、音效链配置（预设、听感微调、导入导出）、旧版响度与均衡器，以及解码能力探测。
 * 倍速、音调与黑胶效果会改动播放时钟，留在 store 里与插值状态一起维护
 */
export function createAudioEffects(ctx: AudioEffectsContext) {
  const {
    settings, volume, playbackSpeed, playbackPitch, vinylEffectEnabled, savePlayerState,
    effectivePlaybackSpeed, applyPlaybackParameter, scheduleNativeAudioEffects, setSpeed, setPitch, setVinylEffect,
  } = ctx

  async function setVolume(vol: number) {
    volume.value = Math.max(0, Math.min(1, vol))
    settings.volume = volume.value
    try {
      await invoke('set_volume', { level: volume.value })
    } catch {
      // 音量已写入设置，下次启动或重新装载时会再次下发
    }
    savePlayerState()
  }

  function effectivePlaybackPitch(): number {
    return vinylEffectEnabled.value ? effectivePlaybackSpeed() : playbackPitch.value
  }

  const audioEffects = computed(() => settings.audioEffects)
  const audioEffectsOutputDevice = ref(settings.audioOutputDevice || 'system_default')
  const audioEffectsProfile = computed(() => audioEffectsProfileFor(audioEffects.value, audioEffectsOutputDevice.value))
  const listeningTuning = computed(() => listeningTuningValues(audioEffectsProfile.value.sound))
  let audioEffectsStatsRevision = 0

  function updateAudioEffects(patch: Partial<AudioEffectsSettings>) {
    settings.audioEffects = normalizeAudioEffectsSettings({ ...settings.audioEffects, ...patch })
  }

  function updateAudioEffectsProfile(patch: Partial<AudioEffectsProfile>) {
    settings.audioEffects = replaceAudioEffectsProfile(settings.audioEffects, audioEffectsOutputDevice.value,
      { ...audioEffectsProfile.value, ...patch })
  }

  function updateAudioEffectsSound(patch: Partial<AudioEffectsSound>) {
    updateAudioEffectsProfile({ presetId: 'custom', sound: { ...audioEffectsProfile.value.sound, ...patch } })
  }

  function setAudioEffectsPreset(id: string) {
    updateAudioEffectsProfile(applyAudioEffectPreset(audioEffectsProfile.value, id, settings.audioEffects.userPresets))
  }

  function setListeningTuning(key: ListeningControlKey, value: number) {
    if (!Number.isFinite(value) || !LISTENING_CONTROLS.some(control => control.key === key)) return
    updateAudioEffects({ autoHeadroom: true })
    updateAudioEffectsProfile({ enabled: true, presetId: 'custom',
      sound: applyListeningTuning(audioEffectsProfile.value.sound, key, value) })
  }

  function resetListeningTuning() {
    updateAudioEffectsProfile({ presetId: 'custom', sound: resetListeningTuningSound(audioEffectsProfile.value.sound) })
  }

  function resetAudioEffectsSection(section: SoundSection | 'speaker') {
    if (section === 'speaker') updateAudioEffects({ speaker: { ...DEFAULT_AUDIO_EFFECTS_SETTINGS.speaker } })
    else updateAudioEffectsProfile({ presetId: 'custom',
      sound: resetAudioEffectsSoundSection(audioEffectsProfile.value.sound, section) })
  }

  function saveAudioEffectsPreset(name: string) {
    const normalizedName = Array.from(name.trim()).slice(0, 24).join('')
    if (!normalizedName) throw new Error(i18n.global.t('dsp.preset_name'))
    if (settings.audioEffects.userPresets.length >= 50) throw new Error(i18n.global.t('dsp.preset_limit'))
    const id = `user_${crypto.randomUUID()}`
    updateAudioEffects({ userPresets: [...settings.audioEffects.userPresets,
      { id, name: normalizedName, sound: audioEffectsProfile.value.sound }] })
    updateAudioEffectsProfile({ presetId: id })
    return id
  }

  function deleteAudioEffectsPreset(id: string) {
    updateAudioEffects({ userPresets: settings.audioEffects.userPresets.filter(preset => preset.id !== id) })
    if (audioEffectsProfile.value.presetId === id) updateAudioEffectsProfile({ presetId: 'custom' })
  }

  function exportAudioEffectsPreset(id?: string): string {
    const preset = id ? settings.audioEffects.userPresets.find(item => item.id === id) : undefined
    return exportAudioEffectsProfile(preset ? { enabled: true, presetId: preset.id, sound: preset.sound }
      : audioEffectsProfile.value, preset?.name)
  }

  interface ImportedPresetEntry { name: string, sound: AudioEffectsSound, sourceId?: string }

  // 同名同音色的预设视为已有，重复导入同一份备份不会堆出一串副本
  function mergeImportedPresets(entries: ImportedPresetEntry[]) {
    const presets = [...settings.audioEffects.userPresets]
    const fingerprint = (sound: AudioEffectsSound) => JSON.stringify(normalizeAudioEffectsSound(sound))
    const idMap = new Map<string, string>()
    let added = 0
    let skipped = 0
    for (const entry of entries) {
      const name = Array.from(entry.name.trim()).slice(0, 24).join('')
      if (!name) { skipped++; continue }
      const existing = presets.find(preset => preset.name === name && fingerprint(preset.sound) === fingerprint(entry.sound))
      if (existing) {
        if (entry.sourceId) idMap.set(entry.sourceId, existing.id)
        continue
      }
      if (presets.length >= 50) { skipped++; continue }
      const id = `user_${crypto.randomUUID()}`
      presets.push({ id, name, sound: normalizeAudioEffectsSound(entry.sound) })
      if (entry.sourceId) idMap.set(entry.sourceId, id)
      added++
    }
    if (added) updateAudioEffects({ userPresets: presets })
    return { added, skipped, idMap }
  }

  function importedPresetId(id: string, idMap: Map<string, string>) {
    return idMap.get(id) ?? (AUDIO_EFFECT_PRESETS.some(preset => preset.id === id) ? id : 'custom')
  }

  function androidRouteLabel(route: string) {
    const known = (ANDROID_OUTPUT_ROUTES as readonly string[]).includes(route)
    return i18n.global.t('dsp.import.route_preset', { route: known ? i18n.global.t(`dsp.import.routes.${route}`) : route })
  }

  /** 导入 Android 配置备份、完整音效设置、Android 预设分享码或 PC 导出的配置 */
  function importAudioEffectsPreset(text: string) {
    const imported = importAudioEffects(text)
    if (imported.kind === 'profile') {
      const merged = imported.name
        ? mergeImportedPresets([{ name: imported.name, sound: imported.profile.sound, sourceId: imported.profile.presetId }])
        : { added: 0, skipped: 0, idMap: new Map<string, string>() }
      updateAudioEffectsProfile({ ...imported.profile, presetId: merged.idMap.get(imported.profile.presetId) ?? 'custom' })
      return { kind: imported.kind, source: imported.source, name: imported.name,
        addedPresets: merged.added, skippedPresets: merged.skipped, routes: 0 }
    }
    const { settings: incoming, routeProfiles } = imported
    const merged = mergeImportedPresets([
      ...incoming.userPresets.map(preset => ({ name: preset.name, sound: preset.sound, sourceId: preset.id })),
      ...routeProfiles.map(item => ({ name: androidRouteLabel(item.route), sound: item.profile.sound })),
    ])
    updateAudioEffects({ speaker: incoming.speaker, powerMode: incoming.powerMode, autoHeadroom: incoming.autoHeadroom })
    updateAudioEffectsProfile({ ...incoming.main, presetId: importedPresetId(incoming.main.presetId, merged.idMap) })
    return { kind: imported.kind, source: imported.source, name: '',
      addedPresets: merged.added, skippedPresets: merged.skipped, routes: routeProfiles.length }
  }

  function importAutoEq(text: string) {
    const imported = parseAutoEq(text)
    updateAudioEffectsProfile({ enabled: true, presetId: 'custom',
      sound: { ...audioEffectsProfile.value.sound, ...imported.sound, equalizerEnabled: true } })
    return imported
  }

  async function refreshAudioEffectsStats(): Promise<AudioEffectsRuntimeStats> {
    const revision = ++audioEffectsStatsRevision
    const stats = await invoke<AudioEffectsRuntimeStats>('get_audio_effects_stats')
    if (revision === audioEffectsStatsRevision && stats?.outputDevice) audioEffectsOutputDevice.value = stats.outputDevice
    return stats
  }

  watch(() => settings.audioEffects, scheduleNativeAudioEffects, { deep: true })
  watch(() => settings.audioOutputDevice, () => {
    void refreshAudioEffectsStats().catch(error => log.warn('audio effects output unavailable:', error))
  })

  // 旧版响度参数继续保留，均衡器数据仅供兼容旧配置
  const loudnessGainMb = ref(settings.loudnessGainMb)
  const equalizerEnabled = ref(settings.equalizerEnabled)
  const equalizerPresetId = ref(settings.equalizerPresetId)
  const equalizerBands = ref([...settings.equalizerBands]) // 5 bands, mB values

  /** 是否有任何非默认音效 */
  const hasActiveEffects = computed(() =>
    playbackSpeed.value !== 1.0
    || playbackPitch.value !== 1.0
    || vinylEffectEnabled.value
    || loudnessGainMb.value !== 0
    || equalizerEnabled.value
    || audioEffectsProfile.value.enabled
    || audioEffects.value.speaker.enabled
  )

  async function setLoudnessGain(mb: number) {
    loudnessGainMb.value = Number.isFinite(mb) ? Math.round(Math.max(0, Math.min(1500, mb))) : 0
    settings.loudnessGainMb = loudnessGainMb.value
    await applyPlaybackParameter('loudness')
  }

  // 音量均衡由设置页直接改 settings，这里跟随下发到音频链
  watch(() => settings.normalizeVolume, (enabled) => {
    void invoke('set_normalize_volume', { enabled })
      .catch(error => log.warn('volume normalization not applied:', error))
  })

  // 多声道音轨的动态范围压缩：对之后打开的音轨生效
  watch(() => settings.multichannelDrc, (enabled) => {
    void invoke('set_multichannel_drc', { enabled })
      .catch(error => log.warn('multichannel dynamic range compression not applied:', error))
  })

  // 声道平衡同样在设置页里改，拖动时实时下发，音频链按 30 ms 平滑过渡
  watch(() => settings.volumeBalance, (balance) => {
    void invoke('set_volume_balance', { balance })
      .catch(error => log.warn('channel balance not applied:', error))
  })

  async function setEqualizer(enabled: boolean, bands: number[]) {
    equalizerEnabled.value = enabled
    equalizerBands.value = Array.from({ length: 5 }, (_, i) => {
      const value = bands[i]
      return Number.isFinite(value) ? Math.round(Math.max(-1500, Math.min(1500, value))) : 0
    })
    settings.equalizerEnabled = enabled
    settings.equalizerBands = [...equalizerBands.value]
    try {
      await invoke('set_equalizer', { enabled, bandLevelsMb: equalizerBands.value })
    } catch (error) {
      log.warn('equalizer not applied:', error)
    }
  }

  async function setEqualizerPreset(presetId: string) {
    if (!Object.prototype.hasOwnProperty.call(EQ_PRESETS, presetId)) presetId = 'flat'
    equalizerPresetId.value = presetId
    settings.equalizerPresetId = presetId
    const bands = presetId === 'custom' ? equalizerBands.value : EQ_PRESETS[presetId]
    await setEqualizer(presetId !== 'flat', [...bands])
  }

  // 导入配置和后端规整后的值也要及时作用于当前播放会话
  watch(() => settings.playbackSpeed, value => {
    if (value !== playbackSpeed.value) void setSpeed(value)
  })
  watch(() => settings.playbackPitch, value => {
    if (value !== playbackPitch.value) void setPitch(value)
  })
  watch(() => settings.vinylEffectEnabled, value => {
    if (value !== vinylEffectEnabled.value) void setVinylEffect(value)
  })
  watch(() => settings.loudnessGainMb, value => {
    if (value !== loudnessGainMb.value) void setLoudnessGain(value)
  })
  watch(() => [settings.equalizerEnabled, settings.equalizerPresetId, ...settings.equalizerBands], () => {
    equalizerPresetId.value = settings.equalizerPresetId
    if (settings.equalizerEnabled !== equalizerEnabled.value
      || settings.equalizerBands.some((value, i) => value !== equalizerBands.value[i])
      || settings.equalizerBands.length !== equalizerBands.value.length) {
      void setEqualizer(settings.equalizerEnabled, settings.equalizerBands)
    }
  })

  // 取流按后端实际能解的编码挑选；查询失败或 FFmpeg 没加载上时只认内置解码器，
  // Opus、E-AC-3 这类流排到后面，选出来的流一定能播
  const decoderCapabilities = ref<DecoderCapabilities | null>(null)
  async function loadDecoderCapabilities() {
    try {
      const capabilities = await invoke<DecoderCapabilities>('get_decoder_capabilities')
      setDecodableCodecs(capabilities.codecs)
      decoderCapabilities.value = capabilities
      if (capabilities.ffmpegError) {
        log.warn('FFmpeg unavailable, streams that need it will be avoided:', capabilities.ffmpegError)
      }
    } catch (error) {
      log.warn('decoder capabilities unavailable, using built-in decoders only:', error)
    }
  }

  return {
    setVolume, effectivePlaybackPitch,
    audioEffects, audioEffectsOutputDevice, audioEffectsProfile, listeningTuning,
    updateAudioEffects, updateAudioEffectsProfile, updateAudioEffectsSound, setAudioEffectsPreset,
    setListeningTuning, resetListeningTuning, resetAudioEffectsSection, saveAudioEffectsPreset,
    deleteAudioEffectsPreset, exportAudioEffectsPreset, importAudioEffectsPreset, importAutoEq, refreshAudioEffectsStats,
    loudnessGainMb, equalizerEnabled, equalizerPresetId, equalizerBands, hasActiveEffects,
    setLoudnessGain, setEqualizer, setEqualizerPreset, decoderCapabilities, loadDecoderCapabilities,
  }
}
