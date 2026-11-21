// 前端共享的曲目与歌词模型（IPC 契约）。放在最底层，modules 与 stores 都从这里引用，
// 避免逻辑层反向依赖 store

export interface TrackInfo {
  id: string
  title: string
  artist: string
  album: string
  durationMs: number
  coverUrl: string
  audioUrl: string
  source?: string
  addedAt?: number
  fileModifiedMs?: number
  syncPayload?: Record<string, unknown>
  playlistKey?: string
}

export interface LyricWord {
  startMs: number
  durationMs: number
  text: string
}

export interface LyricLine {
  startMs: number
  durationMs: number
  words: LyricWord[]
  text: string
  translation?: string
  roman?: string
}

export type RepeatMode = 'off' | 'all' | 'one'
