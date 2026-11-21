/** 进度条下方音质行的一项；accent 为平台音质标签，可点击切换音质 */
export interface AudioInfoPart {
  text: string
  accent?: boolean
}

// 文件播放只采用探测字段，避免把平台规格当成实际文件参数
export function paperSpecFromAudioInfo(info: {
  sampleRateHz?: number
  bitDepth?: number
  channelCount?: number
  specLabel?: string
}, includeSpecLabel = true): string[] {
  const tokens: string[] = []
  if (info.sampleRateHz && info.sampleRateHz > 0) {
    const khz = info.sampleRateHz / 1000
    tokens.push(info.sampleRateHz % 1000 === 0
      ? `${khz.toFixed(0)} kHz`
      : `${khz.toFixed(1)} kHz`)
  }
  if (info.bitDepth && info.bitDepth > 0) tokens.push(`${info.bitDepth} bit`)
  if (!includeSpecLabel && info.channelCount && info.channelCount > 0) tokens.push(`${info.channelCount} ch`)
  // specLabel 里可能混有 kbps, 过滤掉
  if (includeSpecLabel && info.specLabel) {
    for (const part of info.specLabel.split('|').map(s => s.trim())) {
      if (!part || /kbps/i.test(part)) continue
      if (!tokens.includes(part)) tokens.push(part)
    }
  }
  return tokens
}

export function addAudioInfoPart(
  parts: AudioInfoPart[],
  value?: string,
  accent = false,
) {
  const normalized = value?.trim()
  if (!normalized) return
  if (parts.some(part => isSameAudioInfoToken(part.text, normalized))) return
  parts.push({ text: normalized, accent })
}

export function normalizeAudioDisplayToken(value?: string, local = false) {
  if (!value) return ''
  const raw = value.trim()
  // 在线流的 format 常是 MIME（audio/mp4; codecs="mp4a.40.2"），按编码参数或子类型识别
  const [essence, params = ''] = raw.toLowerCase().split(';')
  const isMime = /^(audio|video)\//.test(essence)
  const lower = /codecs\s*=\s*"?([^",]+)/.exec(params)?.[1]?.trim()
    || (isMime ? essence.trim().replace(/^(audio|video)\/(x-)?/, '') : raw.toLowerCase())
  if (local && lower === 'mpeg') return 'MPEG'
  // 占位词直接丢掉
  if (isHiddenAudioInfoToken(raw)) return ''
  const tokenMap: Record<string, string> = {
    flac: 'FLAC',
    mp3: 'MP3',
    mpeg: 'MP3',
    aac: 'AAC',
    mp4a: 'AAC',
    mp4: 'MP4',
    m4a: 'M4A',
    webm: 'WebM',
    opus: 'Opus',
    ogg: 'OGG',
    vorbis: 'Vorbis',
    wav: 'WAV',
    aiff: 'AIFF',
    'ec-3': 'E-AC-3',
    eac3: 'E-AC-3',
    ac3: 'AC-3',
  }
  return tokenMap[lower] ?? tokenMap[lower.split('.')[0]] ?? (isMime ? lower.toUpperCase() : raw)
}

export function isHiddenAudioInfoToken(value?: string) {
  if (!value) return true
  const lower = value.trim().toLowerCase()
  return lower === 'local'
    || lower === 'download'
    || lower === 'file'
    || lower === 'offline'
    || lower === 'downloaded'
}

export function isSameAudioInfoToken(left: string, right: string) {
  return left.trim().toLowerCase() === right.trim().toLowerCase()
}
