import type { AudioInfo } from './playbackAudioInfo'
import { normalizeBitrateKbps, type ResolvedPlaybackSource } from './playbackSource'

// 纸面音质名: 与 NowPlaying 音质列表 / 设置页一致, 供缓存命中与 UI 共用
const NETEASE_QUALITY_I18N: Record<string, string> = {
  standard: '标准',
  higher: '较高',
  exhigh: '极高',
  lossless: '无损',
  hires: 'Hi-Res',
  jyeffect: '高清环绕声',
  sky: '沉浸环绕声',
  jymaster: '超清母带',
}
const QQ_QUALITY_I18N: Record<string, string> = {
  standard: '标准',
  high: '高',
  lossless: '无损',
}
const YOUTUBE_QUALITY_I18N: Record<string, string> = {
  low: '低',
  medium: '中',
  high: '高',
  very_high: '最高',
}
const BILI_QUALITY_I18N: Record<string, string> = {
  low: '流畅',
  medium: '标准',
  high: '较好',
  lossless: '无损',
  hires: 'Hi-Res',
  dolby: '多声道（E-AC-3）',
}

export function qualityLabelFromKey(source?: string | null, key?: string | null): string | undefined {
  if (!key) return undefined
  const k = key.trim().toLowerCase()
  if (!k) return undefined
  const map =
    source === 'netease' ? NETEASE_QUALITY_I18N
    : source === 'qq' ? QQ_QUALITY_I18N
    : source === 'youtube' ? YOUTUBE_QUALITY_I18N
    : source === 'bilibili' ? BILI_QUALITY_I18N
    : null
  return map?.[k] ?? key
}

export function qualityOptionsFromSource(source?: string | null): Array<{ key: string; label: string }> | undefined {
  const map =
    source === 'netease' ? NETEASE_QUALITY_I18N
    : source === 'qq' ? QQ_QUALITY_I18N
    : source === 'youtube' ? YOUTUBE_QUALITY_I18N
    : source === 'bilibili' ? BILI_QUALITY_I18N
    : null
  if (!map) return undefined
  return Object.entries(map).map(([key, label]) => ({ key, label }))
}

/** 在线解析结果的音质展示信息 */
export function resolvedPlaybackAudioInfo(result: ResolvedPlaybackSource): AudioInfo {
  const qKey = result.audioInfo?.qualityKey ?? result.qualityKey
  const rawLabel = result.audioInfo?.qualityLabel
  const paperLabel =
    (rawLabel && rawLabel !== qKey && !/kbps/i.test(rawLabel))
      ? rawLabel
      : qualityLabelFromKey(result.source, qKey)
  return {
    bitrate: result.audioInfo?.bitrateKbps
      ?? normalizeBitrateKbps(result.bitrate),
    codec: result.audioInfo?.codecLabel ?? result.codec,
    format: result.format || result.audioInfo?.mimeType,
    source: result.source,
    qualityKey: qKey,
    qualityLabel: paperLabel,
    qualityOptions: result.audioInfo?.qualityOptions
      ?? qualityOptionsFromSource(result.source),
    mimeType: result.audioInfo?.mimeType,
    sampleRateHz: result.audioInfo?.sampleRateHz,
    bitDepth: result.audioInfo?.bitDepth,
    channelCount: result.audioInfo?.channelCount,
    // 过滤 kbps, 只保留纸面规格
    specLabel: result.audioInfo?.specLabel
      ?.split('|')
      .map(s => s.trim())
      .filter(s => s && !/kbps/i.test(s))
      .join(' | ') || undefined,
  }
}
