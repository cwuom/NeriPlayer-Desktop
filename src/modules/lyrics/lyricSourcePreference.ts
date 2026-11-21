/** 与 Android LyricSourcePreference.storageValue 一致 */
export const DEFAULT_LYRIC_SOURCES = [
  'automatic', 'cloud_music', 'kugou', 'qq_music', 'lrclib', 'amll_ttml',
] as const
export type DefaultLyricSource = typeof DEFAULT_LYRIC_SOURCES[number]
