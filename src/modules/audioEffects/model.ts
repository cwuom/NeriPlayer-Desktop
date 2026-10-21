// 基于 NeriPlayer Android c54884d 的纯音效模型、预设和 RBJ 频响计算移植
// 原作者 cwuom / ouom，来源 https://github.com/cwuom/NeriPlayer，按 GPL-3.0 保留来源

export type ParametricEqBandType = 'peak' | 'low_shelf' | 'high_shelf' | 'low_pass' | 'high_pass' | 'notch' | 'band_pass'
export type AudioEffectsPowerMode = 'eco' | 'balanced' | 'high'
export type SoundSection = 'equalizer' | 'tone' | 'space' | 'dynamics'

export interface ParametricEqBand {
  enabled: boolean
  type: ParametricEqBandType
  frequencyHz: number
  gainDb: number
  q: number
}

export interface AudioEffectsSound {
  preampDb: number
  equalizerEnabled: boolean
  equalizerBandsDb: number[]
  /** 补偿相邻频段的叠加，让实际频响经过每个滑块的数值；Android 没有这个字段，缺省按原样叠加 */
  equalizerAccurate: boolean
  parametricEnabled: boolean
  parametricBands: ParametricEqBand[]
  bassDb: number
  bassFrequencyHz: number
  trebleDb: number
  trebleFrequencyHz: number
  /** 播放器音量调低时按等响曲线补回低频和高频，0 为关闭 */
  loudnessCompensation: number
  virtualBass: number
  virtualBassFrequencyHz: number
  warmth: number
  clarity: number
  clarityFrequencyHz: number
  vocal: number
  vocalRemoval: number
  stereoWidth: number
  monoBassHz: number
  mono: boolean
  swapChannels: boolean
  crossfeed: number
  surround: number
  reverb: number
  reverbRoom: number
  reverbDamping: number
  reverbPreDelayMs: number
  compressorEnabled: boolean
  compressorThresholdDb: number
  compressorRatio: number
  compressorAttackMs: number
  compressorReleaseMs: number
  compressorKneeDb: number
  compressorMakeupDb: number
  outputGainDb: number
  limiterEnabled: boolean
  limiterCeilingDb: number
  limiterReleaseMs: number
  toneEnabled: boolean
  spaceEnabled: boolean
  dynamicsEnabled: boolean
}

export interface AudioEffectsProfile {
  enabled: boolean
  presetId: string
  sound: AudioEffectsSound
}

export interface AudioEffectsUserPreset {
  id: string
  name: string
  sound: AudioEffectsSound
}

export interface SpeakerOptimizerSettings {
  enabled: boolean
  size: 'phone' | 'tablet' | 'large'
  protectionHz: number
  bassHarmonics: number
  loudness: number
  clarity: number
  stereoExpand: number
}

export interface AudioEffectsSettings {
  version: 2
  main: AudioEffectsProfile
  perOutputEnabled: boolean
  outputProfiles: Record<string, AudioEffectsProfile>
  userPresets: AudioEffectsUserPreset[]
  speaker: SpeakerOptimizerSettings
  powerMode: AudioEffectsPowerMode
  autoHeadroom: boolean
}

export const GRAPHIC_EQ_FREQUENCIES = [31.25, 62.5, 125, 250, 500, 1000, 2000, 4000, 8000, 16000]

export const DEFAULT_AUDIO_EFFECTS_SOUND: AudioEffectsSound = {
  preampDb: 0,
  equalizerEnabled: true,
  equalizerBandsDb: Array(10).fill(0),
  equalizerAccurate: false,
  parametricEnabled: false,
  parametricBands: [],
  bassDb: 0,
  bassFrequencyHz: 100,
  trebleDb: 0,
  trebleFrequencyHz: 8000,
  loudnessCompensation: 0,
  virtualBass: 0,
  virtualBassFrequencyHz: 100,
  warmth: 0,
  clarity: 0,
  clarityFrequencyHz: 5000,
  vocal: 0,
  vocalRemoval: 0,
  stereoWidth: 1,
  monoBassHz: 0,
  mono: false,
  swapChannels: false,
  crossfeed: 0,
  surround: 0,
  reverb: 0,
  reverbRoom: 0.5,
  reverbDamping: 0.5,
  reverbPreDelayMs: 10,
  compressorEnabled: false,
  compressorThresholdDb: -18,
  compressorRatio: 2,
  compressorAttackMs: 10,
  compressorReleaseMs: 150,
  compressorKneeDb: 6,
  compressorMakeupDb: 0,
  outputGainDb: 0,
  limiterEnabled: true,
  limiterCeilingDb: -1,
  limiterReleaseMs: 80,
  toneEnabled: true,
  spaceEnabled: true,
  dynamicsEnabled: true,
}

type NumericSoundKey = {
  [K in keyof AudioEffectsSound]: AudioEffectsSound[K] extends number ? K : never
}[keyof AudioEffectsSound]

export interface SoundControl {
  key: NumericSoundKey
  section: SoundSection
  min: number
  max: number
  step: number
  unit: 'dB' | 'Hz' | 'ms' | 'percent' | 'ratio'
  defaultValue: number
}

function control(key: NumericSoundKey, section: SoundSection, min: number, max: number,
  step: number, unit: SoundControl['unit']): SoundControl {
  return { key, section, min, max, step, unit, defaultValue: DEFAULT_AUDIO_EFFECTS_SOUND[key] }
}

export const SOUND_CONTROLS: SoundControl[] = [
  control('preampDb', 'equalizer', -24, 6, 0.5, 'dB'),
  control('bassDb', 'tone', -12, 12, 0.5, 'dB'),
  control('bassFrequencyHz', 'tone', 40, 250, 5, 'Hz'),
  control('trebleDb', 'tone', -12, 12, 0.5, 'dB'),
  control('trebleFrequencyHz', 'tone', 2000, 14000, 250, 'Hz'),
  control('loudnessCompensation', 'tone', 0, 1, 0.05, 'percent'),
  control('virtualBass', 'tone', 0, 1, 0.05, 'percent'),
  control('virtualBassFrequencyHz', 'tone', 50, 200, 5, 'Hz'),
  control('warmth', 'tone', 0, 1, 0.05, 'percent'),
  control('clarity', 'tone', 0, 1, 0.05, 'percent'),
  control('clarityFrequencyHz', 'tone', 2000, 10000, 250, 'Hz'),
  control('vocal', 'tone', 0, 1, 0.05, 'percent'),
  control('vocalRemoval', 'tone', 0, 1, 0.05, 'percent'),
  control('stereoWidth', 'space', 0, 2, 0.05, 'ratio'),
  control('monoBassHz', 'space', 0, 250, 10, 'Hz'),
  control('crossfeed', 'space', 0, 1, 0.05, 'percent'),
  control('surround', 'space', 0, 1, 0.05, 'percent'),
  control('reverb', 'space', 0, 1, 0.05, 'percent'),
  control('reverbRoom', 'space', 0, 1, 0.05, 'percent'),
  control('reverbDamping', 'space', 0, 1, 0.05, 'percent'),
  control('reverbPreDelayMs', 'space', 0, 100, 1, 'ms'),
  control('compressorThresholdDb', 'dynamics', -50, 0, 0.5, 'dB'),
  control('compressorRatio', 'dynamics', 1, 20, 0.1, 'ratio'),
  control('compressorAttackMs', 'dynamics', 0.5, 100, 0.5, 'ms'),
  control('compressorReleaseMs', 'dynamics', 20, 1000, 10, 'ms'),
  control('compressorKneeDb', 'dynamics', 0, 18, 0.5, 'dB'),
  control('compressorMakeupDb', 'dynamics', 0, 18, 0.5, 'dB'),
  control('outputGainDb', 'dynamics', -12, 12, 0.5, 'dB'),
  control('limiterCeilingDb', 'dynamics', -6, 0, 0.1, 'dB'),
  control('limiterReleaseMs', 'dynamics', 20, 500, 10, 'ms'),
]

const BOOLEAN_SOUND_KEYS = ['equalizerEnabled', 'equalizerAccurate', 'parametricEnabled', 'mono', 'swapChannels',
  'compressorEnabled', 'limiterEnabled', 'toneEnabled', 'spaceEnabled', 'dynamicsEnabled'] as const
const PARAMETRIC_TYPES: ParametricEqBandType[] = ['peak', 'low_shelf', 'high_shelf', 'low_pass', 'high_pass', 'notch', 'band_pass']
const OUTPUT_PROFILE_LIMIT = 64
const OUTPUT_DEVICE_NAME_LIMIT = 256
const USER_PRESET_LIMIT = 50
const PRESET_NAME_LIMIT = 24
const PRESET_ID_LIMIT = 128
const IMPORT_TEXT_LIMIT = 128 * 1024
const PROFILE_FORMAT = 'neriplayer-audio-effects-profile'
const DEFAULT_OUTPUT_DEVICE = 'system_default'

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown> : {}
}

function own(value: Record<string, unknown>, key: string): unknown {
  return Object.prototype.hasOwnProperty.call(value, key) ? value[key] : undefined
}

function finite(value: unknown, min: number, max: number, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value)
    ? Math.max(min, Math.min(max, value)) : fallback
}

function flag(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback
}

function textValue(value: unknown, limit: number, fallback = ''): string {
  return typeof value === 'string' ? Array.from(value.trim()).slice(0, limit).join('') || fallback : fallback
}

function safeKey(key: string): boolean {
  return key !== '__proto__' && !Object.prototype.hasOwnProperty.call(Object.prototype, key)
}

function outputKey(value: string): string {
  const key = typeof value === 'string'
    ? Array.from(value.trim()).slice(0, OUTPUT_DEVICE_NAME_LIMIT).join('') || DEFAULT_OUTPUT_DEVICE
    : DEFAULT_OUTPUT_DEVICE
  return safeKey(key) ? key : DEFAULT_OUTPUT_DEVICE
}

function normalizeParametricBand(value: unknown): ParametricEqBand {
  const input = record(value)
  const type = own(input, 'type')
  return {
    enabled: flag(own(input, 'enabled'), true),
    type: PARAMETRIC_TYPES.includes(type as ParametricEqBandType) ? type as ParametricEqBandType : 'peak',
    frequencyHz: finite(own(input, 'frequencyHz'), 20, 20000, 1000),
    gainDb: finite(own(input, 'gainDb'), -30, 24, 0),
    q: finite(own(input, 'q'), 0.1, 20, 0.707),
  }
}

export function normalizeAudioEffectsSound(value: unknown): AudioEffectsSound {
  const input = record(value)
  const sound: AudioEffectsSound = { ...DEFAULT_AUDIO_EFFECTS_SOUND, equalizerBandsDb: Array(10).fill(0), parametricBands: [] }
  for (const item of SOUND_CONTROLS) sound[item.key] = finite(own(input, item.key), item.min, item.max, item.defaultValue)
  for (const key of BOOLEAN_SOUND_KEYS) sound[key] = flag(own(input, key), DEFAULT_AUDIO_EFFECTS_SOUND[key])
  const graphic = own(input, 'equalizerBandsDb')
  if (Array.isArray(graphic)) sound.equalizerBandsDb = GRAPHIC_EQ_FREQUENCIES.map((_, index) => finite(graphic[index], -12, 12, 0))
  const parametric = own(input, 'parametricBands')
  if (Array.isArray(parametric)) sound.parametricBands = parametric.slice(0, 10).map(normalizeParametricBand)
  if (sound.monoBassHz < 40) sound.monoBassHz = 0
  return sound
}

export function normalizeAudioEffectsProfile(value: unknown): AudioEffectsProfile {
  const input = record(value)
  return {
    enabled: flag(own(input, 'enabled'), false),
    presetId: textValue(own(input, 'presetId'), PRESET_ID_LIMIT, 'flat'),
    sound: normalizeAudioEffectsSound(own(input, 'sound')),
  }
}

function normalizeSpeaker(value: unknown): SpeakerOptimizerSettings {
  const input = record(value)
  const size = own(input, 'size')
  const protectionHz = finite(own(input, 'protectionHz'), 0, 400, 0)
  return {
    enabled: flag(own(input, 'enabled'), false),
    size: size === 'tablet' || size === 'large' ? size : 'phone',
    protectionHz: protectionHz < 40 ? 0 : protectionHz,
    bassHarmonics: finite(own(input, 'bassHarmonics'), 0, 1, 0.5),
    loudness: finite(own(input, 'loudness'), 0, 1, 0.4),
    clarity: finite(own(input, 'clarity'), 0, 1, 0.3),
    stereoExpand: finite(own(input, 'stereoExpand'), 0, 1, 0.3),
  }
}

function normalizeUserPresets(value: unknown): AudioEffectsUserPreset[] {
  if (!Array.isArray(value)) return []
  const presets: AudioEffectsUserPreset[] = []
  const ids = new Set<string>()
  for (const item of value) {
    const input = record(item)
    const id = textValue(own(input, 'id'), PRESET_ID_LIMIT)
    const name = textValue(own(input, 'name'), PRESET_NAME_LIMIT)
    if (!id || !name || !safeKey(id) || ids.has(id)) continue
    ids.add(id)
    presets.push({ id, name, sound: normalizeAudioEffectsSound(own(input, 'sound')) })
    if (presets.length === USER_PRESET_LIMIT) break
  }
  return presets
}

export function normalizeAudioEffectsSettings(value: unknown): AudioEffectsSettings {
  const input = record(value)
  const outputProfiles: Record<string, AudioEffectsProfile> = {}
  const sourceProfiles = record(own(input, 'outputProfiles'))
  let profileCount = 0
  for (const sourceKey in sourceProfiles) {
    if (!Object.prototype.hasOwnProperty.call(sourceProfiles, sourceKey)) continue
    if (!safeKey(sourceKey.trim())) continue
    const key = outputKey(sourceKey)
    if (Object.prototype.hasOwnProperty.call(outputProfiles, key)) continue
    outputProfiles[key] = normalizeAudioEffectsProfile(sourceProfiles[sourceKey])
    if (++profileCount === OUTPUT_PROFILE_LIMIT) break
  }
  const powerMode = own(input, 'powerMode')
  return {
    version: 2,
    main: normalizeAudioEffectsProfile(own(input, 'main')),
    perOutputEnabled: flag(own(input, 'perOutputEnabled'), false),
    outputProfiles,
    userPresets: normalizeUserPresets(own(input, 'userPresets')),
    speaker: normalizeSpeaker(own(input, 'speaker')),
    powerMode: powerMode === 'eco' || powerMode === 'high' ? powerMode : 'balanced',
    autoHeadroom: flag(own(input, 'autoHeadroom'), true),
  }
}

export const DEFAULT_AUDIO_EFFECTS_SETTINGS: AudioEffectsSettings = normalizeAudioEffectsSettings(undefined)

export function audioEffectsProfileFor(settings: AudioEffectsSettings, outputDevice: string): AudioEffectsProfile {
  const key = outputKey(outputDevice)
  const profile = settings.perOutputEnabled
    ? Object.prototype.hasOwnProperty.call(settings.outputProfiles, key)
      ? settings.outputProfiles[key]
      : Object.prototype.hasOwnProperty.call(settings.outputProfiles, DEFAULT_OUTPUT_DEVICE)
        ? settings.outputProfiles[DEFAULT_OUTPUT_DEVICE] : settings.main
    : settings.main
  return normalizeAudioEffectsProfile(profile)
}

export function replaceAudioEffectsProfile(settings: AudioEffectsSettings, device: string,
  profile: AudioEffectsProfile): AudioEffectsSettings {
  const next = normalizeAudioEffectsSettings(settings)
  const normalized = normalizeAudioEffectsProfile(profile)
  if (!next.perOutputEnabled) next.main = normalized
  else {
    const key = outputKey(device)
    const keys = Object.keys(next.outputProfiles)
    if (!Object.prototype.hasOwnProperty.call(next.outputProfiles, key) && keys.length >= OUTPUT_PROFILE_LIMIT) {
      delete next.outputProfiles[keys[0]]
    }
    next.outputProfiles[key] = normalized
  }
  return next
}

function compressorAmount(amount: number): Partial<AudioEffectsSound> {
  const threshold = -10 - 26 * amount
  const ratio = 1.5 + 4.5 * amount
  return { compressorEnabled: amount > 0, compressorThresholdDb: threshold, compressorRatio: ratio,
    compressorAttackMs: 12 - 6 * amount, compressorReleaseMs: 180, compressorKneeDb: 8,
    compressorMakeupDb: 0.5 * -threshold * (1 - 1 / ratio) }
}

export interface AudioEffectPreset {
  id: string
  category: 'style' | 'tone' | 'scene' | 'device'
  sound: AudioEffectsSound
}

function preset(id: string, category: AudioEffectPreset['category'], sound: Partial<AudioEffectsSound> = {}): AudioEffectPreset {
  return { id, category, sound: normalizeAudioEffectsSound(sound) }
}

export const AUDIO_EFFECT_PRESETS: AudioEffectPreset[] = [
  preset('flat', 'style'),
  preset('pop', 'style', { equalizerBandsDb: [0, 1, 2, 1, 0, -0.5, 1, 2, 2.5, 2], vocal: 0.2 }),
  preset('rock', 'style', { equalizerBandsDb: [3, 3, 2, 0, -1, -0.5, 1, 2.5, 3, 3], warmth: 0.15 }),
  preset('classical', 'style', { equalizerBandsDb: [1, 1, 0.5, 0, 0, 0, -0.5, 0.5, 1.5, 2],
    stereoWidth: 1.1, reverb: 0.12, reverbRoom: 0.75, reverbDamping: 0.45, reverbPreDelayMs: 20 }),
  preset('jazz', 'style', { equalizerBandsDb: [1.5, 1, 1, 1.5, 0, -0.5, 0.5, 1.5, 2, 1.5], warmth: 0.25 }),
  preset('electronic', 'style', { equalizerBandsDb: [4, 3.5, 2, 0, -1, 0, 1, 2, 3, 3], virtualBass: 0.25, stereoWidth: 1.15 }),
  preset('hip_hop', 'style', { equalizerBandsDb: [4.5, 4, 2.5, 1, -0.5, -0.5, 1, 1, 1.5, 1], virtualBass: 0.3 }),
  preset('rnb', 'style', { equalizerBandsDb: [3, 3, 2, 1, -0.5, 0, 1, 1.5, 1.5, 1], warmth: 0.2 }),
  preset('folk', 'style', { equalizerBandsDb: [0.5, 1, 1.5, 1, 0, 0, 1, 2, 2, 1.5], warmth: 0.15 }),
  preset('metal', 'style', { equalizerBandsDb: [3, 2.5, 1, -1, -2, -1, 1, 3, 3.5, 3] }),
  preset('acg', 'style', { equalizerBandsDb: [1, 1, 0.5, 0, 0, 0.5, 1.5, 2.5, 3, 2.5], vocal: 0.3, clarity: 0.2 }),
  preset('piano', 'style', { equalizerBandsDb: [0, 0, 0.5, 1, 0.5, 0, 0.5, 1, 1.5, 1], reverb: 0.08 }),
  preset('vocal_boost', 'tone', { equalizerBandsDb: [-2, -1.5, -1, 0, 1, 2.5, 3, 2.5, 1, 0], vocal: 0.4 }),
  preset('bass_boost', 'tone', { equalizerBandsDb: [1, 1, 0.5, 0, 0, 0, 0, 0, 0, 0], bassDb: 7, virtualBass: 0.25 }),
  preset('deep_bass', 'tone', { equalizerBandsDb: [2, 1.5, 0.5, 0, -0.5, 0, 0, 0, 0, 0], bassDb: 8,
    bassFrequencyHz: 80, virtualBass: 0.45, monoBassHz: 120 }),
  preset('treble_boost', 'tone', { trebleDb: 5, trebleFrequencyHz: 6000, clarity: 0.2 }),
  preset('warm', 'tone', { warmth: 0.6, trebleDb: -3, trebleFrequencyHz: 9000, crossfeed: 0.1 }),
  preset('live', 'scene', { stereoWidth: 1.3, surround: 0.2, reverb: 0.35, reverbRoom: 0.85,
    reverbDamping: 0.4, reverbPreDelayMs: 25 }),
  preset('concert_hall', 'scene', { stereoWidth: 1.2, reverb: 0.4, reverbRoom: 0.95,
    reverbDamping: 0.3, reverbPreDelayMs: 40 }),
  preset('cinema', 'scene', { bassDb: 4, virtualBass: 0.3, surround: 0.6, reverb: 0.15, reverbRoom: 0.7, ...compressorAmount(0.25) }),
  preset('surround_3d', 'scene', { surround: 0.8, stereoWidth: 1.5 }),
  preset('karaoke', 'scene', { vocalRemoval: 0.9, reverb: 0.25, reverbRoom: 0.6 }),
  preset('night', 'scene', { bassDb: -2, trebleDb: -1, ...compressorAmount(0.7) }),
  preset('spoken', 'scene', { equalizerBandsDb: [-6, -4, -2, 0, 1, 2, 3, 2, 0, -1], mono: true, vocal: 0.5, ...compressorAmount(0.5) }),
  preset('loudness_max', 'scene', { limiterCeilingDb: -0.5, ...compressorAmount(0.85) }),
  preset('headphone_hifi', 'device', { equalizerBandsDb: [1, 0.5, 0, 0, 0, 0, 0.5, 1, 1, 0.5], crossfeed: 0.35, clarity: 0.1 }),
  preset('studio', 'device', { crossfeed: 0.3, reverb: 0.05, reverbRoom: 0.25 }),
  preset('car', 'device', { bassDb: 3, trebleDb: 2, vocal: 0.2, ...compressorAmount(0.3) }),
  preset('small_speaker', 'device', { equalizerBandsDb: [0, 0, 1, 1, 0, 0, 1, 2, 2, 1],
    virtualBass: 0.5, virtualBassFrequencyHz: 140, monoBassHz: 150, ...compressorAmount(0.35) }),
]

export function applyAudioEffectPreset(profile: AudioEffectsProfile, presetId: string,
  userPresets: AudioEffectsUserPreset[] = []): AudioEffectsProfile {
  const current = normalizeAudioEffectsProfile(profile)
  const builtIn = AUDIO_EFFECT_PRESETS.find(item => item.id === presetId)
  if (builtIn) return { enabled: true, presetId, sound: normalizeAudioEffectsSound({ ...builtIn.sound,
    parametricEnabled: current.sound.parametricEnabled, parametricBands: current.sound.parametricBands }) }
  const user = normalizeUserPresets(userPresets).find(item => item.id === presetId)
  return user ? { enabled: true, presetId, sound: normalizeAudioEffectsSound(user.sound) } : current
}

export function resetAudioEffectsSoundSection(sound: AudioEffectsSound, section: SoundSection): AudioEffectsSound {
  const next = normalizeAudioEffectsSound(sound)
  for (const item of SOUND_CONTROLS) if (item.section === section) next[item.key] = item.defaultValue
  switch (section) {
    case 'equalizer':
      next.equalizerEnabled = true
      next.equalizerBandsDb = Array(10).fill(0)
      next.equalizerAccurate = false
      next.parametricEnabled = false
      next.parametricBands = []
      break
    case 'tone':
      next.toneEnabled = true
      break
    case 'space':
      next.spaceEnabled = true
      next.mono = false
      next.swapChannels = false
      break
    case 'dynamics':
      next.dynamicsEnabled = true
      next.compressorEnabled = false
      next.limiterEnabled = true
      break
  }
  return next
}

function effectiveSound(value: AudioEffectsSound): AudioEffectsSound {
  let sound = normalizeAudioEffectsSound(value)
  for (const [section, enabled] of [
    ['equalizer', sound.equalizerEnabled], ['tone', sound.toneEnabled],
    ['space', sound.spaceEnabled], ['dynamics', sound.dynamicsEnabled],
  ] as const) if (!enabled) sound = resetAudioEffectsSoundSection(sound, section)
  return sound
}

export interface AutoEqImport {
  sound: Partial<AudioEffectsSound>
  imported: number
  skipped: number
  warnings: string[]
}

const APO_TYPES: Record<string, ParametricEqBandType> = {
  PK: 'peak', PEQ: 'peak', LS: 'low_shelf', LSC: 'low_shelf', LSQ: 'low_shelf',
  HS: 'high_shelf', HSC: 'high_shelf', HSQ: 'high_shelf', LP: 'low_pass', LPQ: 'low_pass',
  HP: 'high_pass', HPQ: 'high_pass', NO: 'notch', BP: 'band_pass',
}
const NUMBER_TOKEN = '[-+]?(?:\\d+(?:\\.\\d*)?|\\.\\d+)(?:[eE][-+]?\\d+)?'

function importText(text: string, limit = IMPORT_TEXT_LIMIT): string {
  if (typeof text !== 'string' || !text.trim()) throw new Error('导入内容为空')
  if (text.length > limit || new TextEncoder().encode(text).length > limit) {
    throw new Error(`导入内容超过 ${limit / 1024} KiB 限制`)
  }
  return text.trim().replace(/^\uFEFF/, '')
}

function importedNumber(token: string, label: string): number {
  const value = Number(token)
  if (!Number.isFinite(value)) throw new Error(`${label}必须是有限数值`)
  return value
}

function clampImported(value: number, min: number, max: number, label: string, warnings: string[]): number {
  const clamped = Math.max(min, Math.min(max, value))
  if (clamped !== value) warnings.push(`${label}超出范围，已限制到 ${clamped}`)
  return clamped
}

function interpolateLog(frequency: number, points: Array<[number, number]>): number {
  if (frequency <= points[0][0]) return points[0][1]
  if (frequency >= points[points.length - 1][0]) return points[points.length - 1][1]
  const index = points.findIndex(point => point[0] >= frequency)
  const [leftHz, leftGain] = points[index - 1], [rightHz, rightGain] = points[index]
  const progress = Math.log(frequency / leftHz) / Math.log(rightHz / leftHz)
  return leftGain + progress * (rightGain - leftGain)
}

export function importAutoEq(text: string): AutoEqImport {
  const content = importText(text)
  const lines = content.split(/\r?\n/).map(line => line.replace(/\s*#.*$/, '').trim()).filter(Boolean)
  const warnings: string[] = []
  let preampDb = 0, preampFound = false, skipped = 0
  const bands: ParametricEqBand[] = []
  let graphic: string | undefined
  for (const line of lines) {
    if (/\b(?:NaN|Infinity)\b/i.test(line)) throw new Error('AutoEQ 参数必须是有限数值')
    if (/^preamp\s*:/i.test(line)) {
      const match = new RegExp(`^preamp\\s*:\\s*(${NUMBER_TOKEN})\\s*dB\\s*$`, 'i').exec(line)
      if (!match) throw new Error('前级增益格式无效')
      if (preampFound) warnings.push('存在多个前级增益，以最后一个为准')
      preampFound = true
      preampDb = clampImported(importedNumber(match[1], '前级增益'), -24, 6, '前级增益', warnings)
      continue
    }
    const graphicMatch = /^graphiceq\s*:(.*)$/i.exec(line)
    if (graphicMatch) {
      if (graphic !== undefined) throw new Error('只支持一个 GraphicEQ 定义')
      graphic = graphicMatch[1]
      continue
    }
    if (!/^filter(?:\s|\d|:|$)/i.test(line)) { warnings.push(`已忽略不支持的指令: ${line.slice(0, 80)}`); continue }
    const filter = /^filter\s*\d*\s*:\s*(on|off)\s+([a-z]+)\b(.*)$/i.exec(line)
    if (!filter) throw new Error('Filter 格式无效')
    const type = APO_TYPES[filter[2].toUpperCase()]
    if (!type) { skipped++; warnings.push(`已跳过不支持的滤波器 ${filter[2]}`); continue }
    const readParameter = (key: string, unit: string, required: boolean, fallback: number): number => {
      const match = new RegExp(`\\b${key}\\s+(${NUMBER_TOKEN})${unit ? `\\s*${unit}` : ''}(?=\\s|$)`, 'i').exec(filter[3])
      if (!match) {
        if (required || new RegExp(`\\b${key}\\b`, 'i').test(filter[3])) throw new Error(`Filter 的 ${key} 参数无效或缺失`)
        return fallback
      }
      return importedNumber(match[1], key)
    }
    const gainType = type === 'peak' || type === 'low_shelf' || type === 'high_shelf'
    const frequencyHz = readParameter('Fc', 'Hz', true, 1000)
    if (frequencyHz <= 0) throw new Error('滤波器频率必须大于 0')
    const band: ParametricEqBand = {
      enabled: filter[1].toLowerCase() === 'on', type,
      frequencyHz: clampImported(frequencyHz, 20, 20000, '滤波器频率', warnings),
      gainDb: clampImported(readParameter('Gain', 'dB', gainType, 0), -30, 24, '滤波器增益', warnings),
      q: clampImported(readParameter('Q', '', false, 0.707), 0.1, 20, '滤波器 Q', warnings),
    }
    if (bands.length >= 10) { skipped++; warnings.push('参数均衡最多 10 段，已跳过额外滤波器') }
    else bands.push(band)
  }
  if (graphic !== undefined) {
    if (bands.length) throw new Error('不能同时导入 GraphicEQ 和参数 Filter')
    const points: Array<[number, number]> = graphic.split(';').filter(entry => entry.trim()).map(entry => {
      const match = new RegExp(`^\\s*(${NUMBER_TOKEN})\\s+(${NUMBER_TOKEN})\\s*$`).exec(entry)
      if (!match) throw new Error('GraphicEQ 频率和增益格式无效')
      const frequency = importedNumber(match[1], 'GraphicEQ 频率')
      if (frequency <= 0) throw new Error('GraphicEQ 频率必须大于 0')
      return [frequency, importedNumber(match[2], 'GraphicEQ 增益')]
    })
    points.sort((left, right) => left[0] - right[0])
    if (points.length < 2 || points.some((point, index) => index > 0 && point[0] === points[index - 1][0])) {
      throw new Error('GraphicEQ 至少需要两个不同频率的点')
    }
    const equalizerBandsDb = GRAPHIC_EQ_FREQUENCIES.map(frequency => clampImported(
      interpolateLog(frequency, points), -12, 12, `${frequency} Hz 增益`, warnings))
    return { sound: { preampDb, equalizerEnabled: true, equalizerBandsDb }, imported: points.length, skipped, warnings }
  }
  if (!bands.length) throw new Error('未找到有效的 AutoEQ 滤波器或 GraphicEQ')
  return { sound: { preampDb, parametricEnabled: true, parametricBands: bands }, imported: bands.length, skipped, warnings }
}

function validateImportedJson(value: unknown, depth = 0, budget = { remaining: 12000 }): void {
  if (depth > 16 || --budget.remaining < 0) throw new Error('音效 JSON 结构过深或过大')
  if (typeof value === 'number' && !Number.isFinite(value)) throw new Error('音效参数必须是有限数值')
  if (value === null || typeof value !== 'object') return
  if (Array.isArray(value)) {
    for (const item of value) validateImportedJson(item, depth + 1, budget)
  } else {
    for (const [key, item] of Object.entries(value)) {
      if (!safeKey(key)) throw new Error('音效 JSON 含不允许的属性')
      validateImportedJson(item, depth + 1, budget)
    }
  }
}

function validateImportedSound(value: Record<string, unknown>): void {
  const requireNumber = (input: Record<string, unknown>, key: string) => {
    const number = own(input, key)
    if (number !== undefined && (typeof number !== 'number' || !Number.isFinite(number))) {
      throw new Error(`${key}必须是有限数值`)
    }
  }
  const requireFlag = (input: Record<string, unknown>, key: string) => {
    const enabled = own(input, key)
    if (enabled !== undefined && typeof enabled !== 'boolean') throw new Error(`${key}必须为布尔值`)
  }
  for (const item of SOUND_CONTROLS) requireNumber(value, item.key)
  for (const key of BOOLEAN_SOUND_KEYS) requireFlag(value, key)
  const graphic = own(value, 'equalizerBandsDb')
  if (graphic !== undefined && (!Array.isArray(graphic) || graphic.length > 10
    || graphic.some(gain => typeof gain !== 'number' || !Number.isFinite(gain)))) {
    throw new Error('图形均衡器必须是最多 10 段的有限数值数组')
  }
  const parametric = own(value, 'parametricBands')
  if (parametric === undefined) return
  if (!Array.isArray(parametric) || parametric.length > 10) throw new Error('参数均衡器必须是最多 10 段的数组')
  for (const item of parametric) {
    if (item === null || typeof item !== 'object' || Array.isArray(item)) throw new Error('参数均衡器段格式无效')
    const band = record(item)
    const type = own(band, 'type')
    if (type !== undefined && !PARAMETRIC_TYPES.includes(type as ParametricEqBandType)) throw new Error('参数均衡器滤波类型无效')
    for (const key of ['frequencyHz', 'gainDb', 'q']) requireNumber(band, key)
    requireFlag(band, 'enabled')
  }
}

const DEFAULT_EXPORT_NAME = 'NeriPlayer PC'

export function exportAudioEffectsProfile(profile: AudioEffectsProfile, name = ''): string {
  const normalized = normalizeAudioEffectsProfile(profile)
  const exportName = textValue(name, PRESET_NAME_LIMIT)
  // 顶层 id/name/sound 是 Android 预设分享码的必填字段，Android 会忽略其余键
  return JSON.stringify({ format: PROFILE_FORMAT, version: 2,
    id: normalized.presetId === 'custom' ? 'pc_custom' : normalized.presetId,
    name: exportName || DEFAULT_EXPORT_NAME, displayName: exportName,
    sound: normalized.sound, profile: normalized }, null, 2)
}

function parseImportedJson(text: string, limit = IMPORT_TEXT_LIMIT): unknown {
  let parsed: unknown
  try { parsed = JSON.parse(importText(text, limit)) } catch (error) {
    throw new Error(error instanceof SyntaxError ? '音效 JSON 格式无效' : error instanceof Error ? error.message : '音效导入失败')
  }
  return parsed
}

export function importAudioEffectsProfile(text: string): { name: string, profile: AudioEffectsProfile } {
  const parsed = parseImportedJson(text)
  validateImportedJson(parsed)
  return importProfileObject(parsed)
}

function importProfileObject(parsed: unknown): { name: string, profile: AudioEffectsProfile } {
  const input = record(parsed)
  if (own(input, 'format') !== undefined && own(input, 'format') !== PROFILE_FORMAT) throw new Error('不是 NeriPlayer 音效配置')
  const version = own(input, 'version')
  if (version !== undefined && version !== 1 && version !== 2) throw new Error('不支持的音效配置版本')
  const candidate = record(own(input, 'profile') ?? own(input, 'main') ?? input)
  const rawSound = own(candidate, 'sound')
  if (rawSound === null || typeof rawSound !== 'object' || Array.isArray(rawSound)) throw new Error('音效配置缺少有效的 sound 对象')
  validateImportedSound(record(rawSound))
  const userId = textValue(own(candidate, 'id'), PRESET_ID_LIMIT)
  const normalized = normalizeAudioEffectsProfile({ ...candidate,
    enabled: own(candidate, 'enabled') ?? true,
    presetId: own(candidate, 'presetId') ?? (userId || 'custom'),
  })
  const displayName = own(input, 'displayName')
  return { name: textValue(typeof displayName === 'string' ? displayName : own(input, 'name'), PRESET_NAME_LIMIT),
    profile: normalized }
}

export const ANDROID_CONFIG_KIND = 'moe.ouom.neriplayer.config'
const ANDROID_BACKUP_TEXT_LIMIT = 1024 * 1024
const ANDROID_AUDIO_EFFECTS_KEY = 'audio_effects_settings'
export const ANDROID_OUTPUT_ROUTES = ['speaker', 'wired', 'bluetooth', 'usb', 'other'] as const

export interface AudioEffectsRouteProfile {
  route: string
  profile: AudioEffectsProfile
}

export type AudioEffectsImport =
  | { kind: 'profile', source: 'profile', name: string, profile: AudioEffectsProfile }
  | { kind: 'settings', source: 'android_backup' | 'settings', settings: AudioEffectsSettings,
    routeProfiles: AudioEffectsRouteProfile[] }

function looksLikeFullSettings(input: Record<string, unknown>): boolean {
  if (own(input, 'format') !== undefined || own(input, 'sound') !== undefined) return false
  return own(input, 'main') !== undefined && ['userPresets', 'outputProfiles', 'speaker', 'powerMode',
    'perOutputEnabled', 'autoHeadroom', 'speedPitchLinked'].some(key => own(input, key) !== undefined)
}

function importSettingsObject(input: Record<string, unknown>,
  source: 'android_backup' | 'settings'): AudioEffectsImport {
  const version = own(input, 'version')
  if (version !== undefined && (typeof version !== 'number' || version < 0 || version > 2)) throw new Error('不支持的音效配置版本')
  const main = own(input, 'main')
  if (main !== undefined) {
    if (main === null || typeof main !== 'object' || Array.isArray(main)) throw new Error('音效配置的 main 格式无效')
    const sound = own(record(main), 'sound')
    if (sound !== undefined) validateImportedSound(record(sound))
  }
  const routeProfiles: AudioEffectsRouteProfile[] = []
  const sourceProfiles = record(own(input, 'outputProfiles'))
  for (const route of Object.keys(sourceProfiles)) {
    if (!(ANDROID_OUTPUT_ROUTES as readonly string[]).includes(route)) continue
    const sound = own(record(sourceProfiles[route]), 'sound')
    if (sound !== undefined) validateImportedSound(record(sound))
    routeProfiles.push({ route: outputKey(route), profile: normalizeAudioEffectsProfile(sourceProfiles[route]) })
  }
  const userPresets = own(input, 'userPresets')
  if (Array.isArray(userPresets)) {
    for (const preset of userPresets) {
      const sound = own(record(preset), 'sound')
      if (sound !== undefined) validateImportedSound(record(sound))
    }
  }
  // 手机的输出路由（扬声器/耳机/蓝牙）和电脑的声卡名不是一回事，路由配置单独交给调用方处理
  const settings = normalizeAudioEffectsSettings({ ...input, outputProfiles: {}, perOutputEnabled: false })
  return { kind: 'settings', source, settings, routeProfiles: own(input, 'perOutputEnabled') === true ? routeProfiles : [] }
}

/**
 * 识别所有能拿到的音效来源：Android 配置备份、完整音效设置、Android 预设分享码和 PC 导出的单个配置
 */
export function importAudioEffects(text: string): AudioEffectsImport {
  const parsed = parseImportedJson(text, ANDROID_BACKUP_TEXT_LIMIT)
  const input = record(parsed)
  if (own(input, 'kind') === ANDROID_CONFIG_KIND) {
    // 备份里其余设置（账号、同步……）一概不读，只取音效这一项再按普通导入校验
    const raw = own(record(own(record(own(input, 'settings')), 'strings')), ANDROID_AUDIO_EFFECTS_KEY)
    if (typeof raw !== 'string' || !raw.trim()) throw new Error('这份 Android 备份里没有音效设置，请先在手机上调过音效再导出')
    const inner = parseImportedJson(raw)
    validateImportedJson(inner)
    return importSettingsObject(record(inner), 'android_backup')
  }
  importText(text)
  validateImportedJson(parsed)
  if (looksLikeFullSettings(input)) return importSettingsObject(input, 'settings')
  const profile = importProfileObject(parsed)
  return { kind: 'profile', source: 'profile', name: profile.name, profile: profile.profile }
}

interface BiquadCoefficients {
  b0: number
  b1: number
  b2: number
  a1: number
  a2: number
}

const RESPONSE_SAMPLE_RATE = 48000

function coefficients(b0: number, b1: number, b2: number, a0: number, a1: number, a2: number): BiquadCoefficients {
  return { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
}

function designBiquad(type: ParametricEqBandType, frequencyHz: number, gainDb: number, q: number): BiquadCoefficients {
  const frequency = Math.max(5, Math.min(RESPONSE_SAMPLE_RATE * 0.45, frequencyHz))
  const omega = 2 * Math.PI * frequency / RESPONSE_SAMPLE_RATE
  const c = Math.cos(omega), alpha = Math.sin(omega) / (2 * Math.max(0.1, Math.min(24, q)))
  const a = 10 ** (gainDb / 40), root = 2 * Math.sqrt(a) * alpha
  switch (type) {
    case 'peak': return coefficients(1 + alpha * a, -2 * c, 1 - alpha * a, 1 + alpha / a, -2 * c, 1 - alpha / a)
    case 'low_shelf': return coefficients(a * (a + 1 - (a - 1) * c + root), 2 * a * (a - 1 - (a + 1) * c),
      a * (a + 1 - (a - 1) * c - root), a + 1 + (a - 1) * c + root, -2 * (a - 1 + (a + 1) * c), a + 1 + (a - 1) * c - root)
    case 'high_shelf': return coefficients(a * (a + 1 + (a - 1) * c + root), -2 * a * (a - 1 + (a + 1) * c),
      a * (a + 1 + (a - 1) * c - root), a + 1 - (a - 1) * c + root, 2 * (a - 1 - (a + 1) * c), a + 1 - (a - 1) * c - root)
    case 'low_pass': return coefficients((1 - c) / 2, 1 - c, (1 - c) / 2, 1 + alpha, -2 * c, 1 - alpha)
    case 'high_pass': return coefficients((1 + c) / 2, -(1 + c), (1 + c) / 2, 1 + alpha, -2 * c, 1 - alpha)
    case 'notch': return coefficients(1, -2 * c, 1, 1 + alpha, -2 * c, 1 - alpha)
    case 'band_pass': return coefficients(alpha, 0, -alpha, 1 + alpha, -2 * c, 1 - alpha)
  }
}

function magnitudeDb(filter: BiquadCoefficients, frequency: number): number {
  const omega = 2 * Math.PI * frequency / RESPONSE_SAMPLE_RATE
  const realNumerator = filter.b0 + filter.b1 * Math.cos(omega) + filter.b2 * Math.cos(2 * omega)
  const imaginaryNumerator = -filter.b1 * Math.sin(omega) - filter.b2 * Math.sin(2 * omega)
  const realDenominator = 1 + filter.a1 * Math.cos(omega) + filter.a2 * Math.cos(2 * omega)
  const imaginaryDenominator = -filter.a1 * Math.sin(omega) - filter.a2 * Math.sin(2 * omega)
  const numerator = realNumerator ** 2 + imaginaryNumerator ** 2
  const denominator = realDenominator ** 2 + imaginaryDenominator ** 2
  return numerator <= 0 || denominator <= 0 ? -120 : 10 * Math.log10(numerator / denominator)
}

const GRAPHIC_EQ_Q = 1.41
const GRAPHIC_EQ_NATIVE_LIMIT_DB = 15
const GRAPHIC_EQ_PROBE_DB = 6
const GRAPHIC_EQ_REGULARIZATION = 0.01
const GRAPHIC_EQ_REFINEMENTS = 3
let graphicInteraction: number[][] | null = null

// interaction[j][i]：第 i 段每 dB 增益在第 j 段中心频率处产生的 dB 数
function graphicEqInteraction(): number[][] {
  graphicInteraction ??= GRAPHIC_EQ_FREQUENCIES.map(center => GRAPHIC_EQ_FREQUENCIES.map(frequency =>
    magnitudeDb(designBiquad('peak', frequency, GRAPHIC_EQ_PROBE_DB, GRAPHIC_EQ_Q), center) / GRAPHIC_EQ_PROBE_DB))
  return graphicInteraction
}

function graphicCenterResponse(gains: number[]): number[] {
  const filters = gains.map((gain, index) => designBiquad('peak', GRAPHIC_EQ_FREQUENCIES[index], gain, GRAPHIC_EQ_Q))
  return GRAPHIC_EQ_FREQUENCIES.map(center => filters.reduce((total, filter) => total + magnitudeDb(filter, center), 0))
}

// 正则化最小二乘：(BᵀB + λI)x = Bᵀr，λ 防止相邻段为凑精度互相拉出极端的正负增益
function solveGraphicCorrection(residual: number[]): number[] {
  const interaction = graphicEqInteraction()
  const size = residual.length
  const matrix = Array.from({ length: size }, (_, row) => Array.from({ length: size + 1 }, (_, column) => {
    if (column === size) return interaction.reduce((total, line, k) => total + line[row] * residual[k], 0)
    const value = interaction.reduce((total, line) => total + line[row] * line[column], 0)
    return row === column ? value + GRAPHIC_EQ_REGULARIZATION : value
  }))
  for (let pivot = 0; pivot < size; pivot++) {
    let best = pivot
    for (let row = pivot + 1; row < size; row++) if (Math.abs(matrix[row][pivot]) > Math.abs(matrix[best][pivot])) best = row
    ;[matrix[pivot], matrix[best]] = [matrix[best], matrix[pivot]]
    for (let row = 0; row < size; row++) {
      if (row === pivot) continue
      const factor = matrix[row][pivot] / matrix[pivot][pivot]
      for (let column = pivot; column <= size; column++) matrix[row][column] -= factor * matrix[pivot][column]
    }
  }
  return matrix.map((row, index) => row[size] / row[index])
}

/** 送进原生图形均衡的实际增益；精确模式下让各中心频率的合成频响等于滑块值 */
export function graphicEqDesignGains(targets: readonly number[], accurate: boolean): number[] {
  const gains = GRAPHIC_EQ_FREQUENCIES.map((_, index) => Number.isFinite(targets[index]) ? targets[index] : 0)
  if (!accurate || gains.every(gain => gain === 0)) return gains
  const clamp = (value: number) => Math.max(-GRAPHIC_EQ_NATIVE_LIMIT_DB, Math.min(GRAPHIC_EQ_NATIVE_LIMIT_DB, value))
  let design = solveGraphicCorrection(gains).map(clamp)
  // 峰值滤波器的 dB 响应只是近似线性，用真实频响再修正几轮
  for (let step = 0; step < GRAPHIC_EQ_REFINEMENTS; step++) {
    const response = graphicCenterResponse(design)
    const correction = solveGraphicCorrection(gains.map((target, index) => target - response[index]))
    design = design.map((gain, index) => clamp(gain + correction[index]))
  }
  return design
}

/** 响度补偿：音量每降 1 dB，低频补约 0.3 dB、高频补约 0.1 dB，分别封顶 10 dB / 4 dB */
export function loudnessCompensationDb(amount: number, volume: number): { bassDb: number, trebleDb: number } {
  const strength = Number.isFinite(amount) ? Math.max(0, Math.min(1, amount)) : 0
  if (strength <= 0) return { bassDb: 0, trebleDb: 0 }
  const linear = Number.isFinite(volume) ? Math.max(0.001, Math.min(1, volume)) : 1
  const attenuation = Math.min(40, Math.max(0, -20 * Math.log10(linear)))
  return { bassDb: strength * Math.min(10, attenuation * 0.3), trebleDb: strength * Math.min(4, attenuation * 0.1) }
}

export const LOUDNESS_BASS_HZ = 120
export const LOUDNESS_TREBLE_HZ = 10000

function responseFilters(sound: AudioEffectsSound, volume: number): BiquadCoefficients[] {
  const filters: BiquadCoefficients[] = []
  if (sound.equalizerEnabled) graphicEqDesignGains(sound.equalizerBandsDb, sound.equalizerAccurate).forEach((gain, index) => {
    if (gain !== 0) filters.push(designBiquad('peak', GRAPHIC_EQ_FREQUENCIES[index], gain, GRAPHIC_EQ_Q))
  })
  if (sound.parametricEnabled) for (const band of sound.parametricBands) {
    if (band.enabled) filters.push(designBiquad(band.type, band.frequencyHz, band.gainDb, band.q))
  }
  if (sound.bassDb !== 0) {
    filters.push(designBiquad('low_shelf', sound.bassFrequencyHz, sound.bassDb, 0.707))
    if (sound.bassDb > 0) {
      // 与原生低音一致，只在提升时逐渐收紧次声高通
      const targetHz = Math.max(16, Math.min(30, 0.25 * sound.bassFrequencyHz))
      const subsonicHz = 5 + (targetHz - 5) * Math.min(1, sound.bassDb / 4)
      filters.push(designBiquad('high_pass', subsonicHz, 0, 0.707))
    }
  }
  if (sound.trebleDb !== 0) filters.push(designBiquad('high_shelf', sound.trebleFrequencyHz, sound.trebleDb, 0.707))
  const loudness = loudnessCompensationDb(sound.loudnessCompensation, volume)
  if (loudness.bassDb !== 0) filters.push(designBiquad('low_shelf', LOUDNESS_BASS_HZ, loudness.bassDb, 0.707))
  if (loudness.trebleDb !== 0) filters.push(designBiquad('high_shelf', LOUDNESS_TREBLE_HZ, loudness.trebleDb, 0.707))
  return filters
}

export function audioEffectsFrequencyResponse(value: AudioEffectsSound, volume = 1,
  points = 96): Array<{ frequencyHz: number, gainDb: number }> {
  const sound = effectiveSound(value)
  const filters = responseFilters(sound, volume)
  const count = Math.max(2, Math.round(points))
  return Array.from({ length: count }, (_, index) => {
    const frequencyHz = 20 * (1000 ** (index / (count - 1)))
    const gainDb = filters.reduce((total, filter) => total + magnitudeDb(filter, frequencyHz), sound.preampDb)
    return { frequencyHz, gainDb }
  })
}

const LEGACY_PRESETS: Record<string, number[]> = {
  flat: [0, 0, 0, 0, 0], acoustic: [300, 200, 100, 200, 300], bass_boost: [700, 500, 200, -100, -300],
  bass_reduce: [-700, -500, -200, 0, 100], bass_reducer: [-700, -500, -200, 0, 100],
  classical: [400, 200, -100, 300, 500], club: [500, 300, 0, 300, 400], dance: [600, 400, 100, 400, 500],
  deep: [800, 500, 100, 0, 200], electronic: [500, 200, -100, 400, 600], folk: [100, 300, 0, 400, 500],
  hip_hop: [700, 400, 100, 200, 400], jazz: [300, 200, 100, 300, 400], latin: [400, 300, 100, 400, 500],
  loudness: [500, 200, 0, -100, -200], lounge: [200, 100, 100, 300, 400], piano: [200, 100, 0, 300, 400],
  pop: [-100, 300, 500, 300, 0], rnb: [400, 300, 200, 400, 300], rock: [600, 300, -100, 300, 600],
  small_speakers: [400, 100, -200, 400, 700], spoken_word: [-400, -200, 400, 500, 200],
  treble_boost: [-300, -100, 200, 500, 800], treble_reduce: [100, 0, -200, -500, -800],
  treble_reducer: [100, 0, -200, -500, -800], vocal_boost: [-200, 100, 500, 500, 100],
}

export function migrateLegacyAudioEffects(value: {
  equalizerEnabled?: boolean
  equalizerPresetId?: string
  equalizerBands?: number[]
  loudnessGainMb?: number
}): AudioEffectsSettings {
  const settings = normalizeAudioEffectsSettings(undefined)
  if (value.equalizerEnabled !== true) return settings
  const rawBands = Array.isArray(value.equalizerBands) && value.equalizerBands.length === 5
    ? value.equalizerBands : safeKey(value.equalizerPresetId ?? '') ? LEGACY_PRESETS[value.equalizerPresetId ?? ''] : undefined
  const gains = [0, 1, 2, 3, 4].map(index => finite(rawBands?.[index], -1500, 1500, 0) / 100)
  const headroom = Math.max(0, ...gains)
  // 旧管线逐段减去最大增益；保留原 Q 和余量策略，避免升级后听感突然变亮或再次降低音量
  const bands = [60, 230, 910, 3600, 14000].map((frequencyHz, index): ParametricEqBand => ({
    enabled: true, type: 'peak', frequencyHz, gainDb: gains[index] - headroom, q: 1,
  }))
  settings.main = { enabled: true, presetId: 'custom',
    sound: normalizeAudioEffectsSound({ parametricEnabled: true, parametricBands: bands }) }
  settings.autoHeadroom = false
  return settings
}
