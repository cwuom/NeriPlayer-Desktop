// 音质选项表：key 与设置页、Android 一致，label 是 i18n key
const neteaseQualities = [
  { key: 'standard', label: 'settings.q_standard' },
  { key: 'higher', label: 'settings.q_high' },
  { key: 'exhigh', label: 'settings.q_exhigh' },
  { key: 'lossless', label: 'settings.q_lossless' },
  { key: 'hires', label: 'settings.q_hires' },
  { key: 'jyeffect', label: 'settings.q_surround' },
  { key: 'sky', label: 'settings.q_sky' },
  { key: 'jymaster', label: 'settings.q_master' },
]

const qqQualities = [
  { key: 'standard', label: 'settings.q_standard' },
  { key: 'high', label: 'settings.q_high_yt' },
  { key: 'lossless', label: 'settings.q_lossless' },
]

const youtubeQualities = [
  { key: 'low', label: 'settings.q_low' },
  { key: 'medium', label: 'settings.q_medium' },
  { key: 'high', label: 'settings.q_high_yt' },
  { key: 'very_high', label: 'settings.q_very_high' },
]

const biliQualities = [
  { key: 'low', label: 'settings.q_smooth' },
  { key: 'medium', label: 'settings.q_standard' },
  { key: 'high', label: 'settings.q_good' },
  { key: 'lossless', label: 'settings.q_lossless' },
  { key: 'hires', label: 'settings.q_hires' },
  { key: 'dolby', label: 'settings.q_dolby' },
]

export type QualityOption = (typeof neteaseQualities)[number]

/** 各平台当前音质设置所在的字段 */
export interface QualitySettings {
  neteaseQuality: string
  qqMusicQuality: string
  biliQuality: string
  youtubeQuality: string
}

export function qualityOptionsForSource(source: string): QualityOption[] {
  if (source === 'netease') return neteaseQualities
  if (source === 'qq') return qqQualities
  if (source === 'bilibili') return biliQualities
  if (source === 'youtube') return youtubeQualities
  return []
}

export function qualityKeyForSource(settings: QualitySettings, source: string): string {
  if (source === 'netease') return settings.neteaseQuality
  if (source === 'qq') return settings.qqMusicQuality
  if (source === 'bilibili') return settings.biliQuality
  if (source === 'youtube') return settings.youtubeQuality
  return ''
}

export function setQualityKeyForSource(settings: QualitySettings, source: string, key: string) {
  if (source === 'netease') settings.neteaseQuality = key
  else if (source === 'qq') settings.qqMusicQuality = key
  else if (source === 'bilibili') settings.biliQuality = key
  else if (source === 'youtube') settings.youtubeQuality = key
}
