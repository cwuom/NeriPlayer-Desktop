export interface AudioQualityState {
  source: string
  fromDownload: boolean
  info: {
    source?: string
    qualityKey?: string
    qualityLabel?: string
  } | null
}

export function isLocalAudioPlayback(state: AudioQualityState): boolean {
  return state.fromDownload || state.source === 'local' || state.info?.source === 'local'
}

export function canSwitchAudioQuality(state: AudioQualityState): boolean {
  return !isLocalAudioPlayback(state) && ['netease', 'qq', 'bilibili', 'youtube'].includes(state.source)
}

export function resolveAudioQualityLabel(
  state: AudioQualityState,
  resolveFallback: (source: string, key?: string) => string,
): string {
  if (isLocalAudioPlayback(state)) return ''
  const info = state.info
  const source = info?.source || state.source
  const key = info?.qualityKey
  // 已知档位走界面语言的译名；播放层写入的 qualityLabel 是固定中文，只用于界面不认识的档位
  if (key) {
    const translated = resolveFallback(source, key)
    if (translated && translated !== key) return translated
  }
  const labeled = info?.qualityLabel?.trim()
  if (labeled && !/kbps/i.test(labeled) && labeled !== key) return labeled
  return resolveFallback(source, key)
}

export function actualAudioBitrateLabel(info: { bitrate?: number } | null): string {
  const bitrate = info?.bitrate
  return typeof bitrate === 'number' && Number.isFinite(bitrate) && bitrate > 0
    ? `${Math.round(bitrate)} kbps`
    : ''
}

interface ActualAudioParameters {
  bitrate?: number
  format?: string
  codec?: string
  channelCount?: number
  sampleRateHz?: number
  bitDepth?: number
}

interface AudioParameterVisibility {
  showAudioBitrate: boolean
  showAudioFormat: boolean
  showAudioChannels: boolean
  showAudioSampleRate: boolean
  showAudioBitDepth: boolean
}

export function actualAudioParameterLabels(
  info: ActualAudioParameters,
  visibility: AudioParameterVisibility,
): string[] {
  const labels: string[] = []
  if (visibility.showAudioBitrate) {
    const bitrate = actualAudioBitrateLabel(info)
    if (bitrate) labels.push(bitrate)
  }
  if (visibility.showAudioFormat) {
    const format = actualAudioFormatLabel(info.format)
    const codec = actualAudioFormatLabel(info.codec)
    // MP4/WebM/OGG 只是封装，同一个 audio/mp4 里可能是 AAC 也可能是 E-AC-3，认得出编码就展示编码
    const label = format && codec && CONTAINER_FORMATS.has(format) ? codec : format || codec
    if (label) labels.push(label)
  }
  if (visibility.showAudioChannels && isPositiveInteger(info.channelCount)) labels.push(`${info.channelCount} ch`)
  if (visibility.showAudioSampleRate && typeof info.sampleRateHz === 'number'
    && Number.isFinite(info.sampleRateHz) && info.sampleRateHz > 0) {
    const khz = info.sampleRateHz / 1000
    labels.push(`${info.sampleRateHz % 1000 === 0 ? khz.toFixed(0) : khz.toFixed(1)} kHz`)
  }
  if (visibility.showAudioBitDepth && isPositiveInteger(info.bitDepth)) labels.push(`${info.bitDepth} bit`)
  return labels
}

function isPositiveInteger(value?: number): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value > 0
}

const CONTAINER_FORMATS = new Set(['MP4', 'M4A', 'WebM', 'OGG', 'MKA'])

const AUDIO_FORMAT_LABELS: Record<string, string> = {
  flac: 'FLAC', mp3: 'MP3', mpeg: 'MPEG', aac: 'AAC', mp4a: 'AAC',
  mp4: 'MP4', m4a: 'M4A', opus: 'Opus', ogg: 'OGG', vorbis: 'Vorbis',
  wav: 'WAV', wave: 'WAV', aiff: 'AIFF', 'ec-3': 'E-AC-3', eac3: 'E-AC-3', 'e-ac-3': 'E-AC-3',
  ac3: 'AC-3', 'ac-3': 'AC-3', alac: 'ALAC', webm: 'WebM', matroska: 'MKA',
}

function actualAudioFormatLabel(value?: string): string {
  const raw = value?.trim() || ''
  const lower = raw.toLowerCase()
  if (!raw || ['unknown', 'local', 'download', 'file', 'offline', 'downloaded'].includes(lower)) return ''
  // 在线流常给 MIME（audio/mp4; codecs="mp4a.40.2"），按编码参数或子类型识别，不把 MIME 原样露给用户
  const [essence, params = ''] = lower.split(';')
  const codecs = /codecs\s*=\s*"?([^",]+)/.exec(params)?.[1]?.trim()
  const isMime = /^(audio|video)\//.test(essence)
  const token = codecs || (isMime ? essence.trim().replace(/^(audio|video)\/(x-)?/, '') : lower)
  return AUDIO_FORMAT_LABELS[token] ?? AUDIO_FORMAT_LABELS[token.split('.')[0]] ?? (isMime ? token.toUpperCase() : raw)
}
