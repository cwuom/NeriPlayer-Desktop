// 仅开发模式下、在普通浏览器（非 Tauri）里打开时加载：模拟窗口与 IPC，
// 让界面能在浏览器里完整渲染，便于做 UI 调整和多尺寸检查。生产构建不会引入本文件。
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks'

const COVER_HUES = [12, 38, 160, 205, 262, 318]

function sampleCover(index: number): string {
  const hue = COVER_HUES[index % COVER_HUES.length]
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="240" height="240"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue},62%,62%)"/><stop offset="1" stop-color="hsl(${(hue + 40) % 360},55%,32%)"/></linearGradient></defs><rect width="240" height="240" fill="url(#g)"/></svg>`
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
}

const SAMPLE_TITLES = [
  '夜に駆ける', 'Blue Hour', '晴天', 'Lemon', 'Wonderwall', '稻香', 'Clair de Lune', 'Shape of You',
  '紅蓮華', '起风了', 'Nightcall', 'Golden Hour', '可能否', 'Merry-Go-Round', 'Feather', '春日影',
]
const SAMPLE_ARTISTS = ['YOASOBI', 'TXT', '周杰伦', '米津玄師', 'Oasis', 'Debussy', 'Kavinsky', 'JVKE', '木小雅', '久石譲']

function sampleTracks(count: number) {
  return Array.from({ length: count }, (_, index) => ({
    id: `preview-${index}`,
    title: SAMPLE_TITLES[index % SAMPLE_TITLES.length],
    artist: SAMPLE_ARTISTS[index % SAMPLE_ARTISTS.length],
    album: `Album ${index % 7}`,
    duration_ms: 150_000 + index * 7_000,
    cover_url: sampleCover(index),
    url: '',
    source: 'local',
    added_at: Date.now() - index * 86_400_000,
    file_modified_ms: Date.now() - index * 86_400_000,
  }))
}

function samplePlaylists() {
  return ['通勤路上', '深夜循环', 'Lo-fi 学习', '收藏的 J-Pop'].map((name, index) => ({
    id: String(index + 1),
    name,
    track_count: 12 + index * 9,
    trackCount: 12 + index * 9,
    cover_url: sampleCover(index + 2),
    coverUrl: sampleCover(index + 2),
    created_at: Date.now() - index * 3_600_000,
  }))
}

function neteaseSongs(count: number) {
  return Array.from({ length: count }, (_, index) => ({
    id: 1000 + index,
    name: SAMPLE_TITLES[index % SAMPLE_TITLES.length],
    ar: [{ name: SAMPLE_ARTISTS[index % SAMPLE_ARTISTS.length] }],
    al: { name: `Album ${index % 7}`, picUrl: sampleCover(index) },
    dt: 180_000 + index * 5_000,
  }))
}

function neteasePlaylists(count: number) {
  const names = ['城市夜行', '雨天的 Lo-fi', '日系清新', '独立摇滚精选', '钢琴与白噪音', '复古合成器', '华语经典', '动漫原声']
  return Array.from({ length: count }, (_, index) => ({
    id: 2000 + index,
    name: names[index % names.length],
    picUrl: sampleCover(index + 3),
    trackCount: 40 + index * 3,
    playCount: 120_000 * (index + 1),
  }))
}

function homeSection(args: Record<string, unknown> | undefined): unknown {
  const source = String(args?.source ?? '')
  const songSections = ['personal_radar', 'daily_recommend', 'private_fm', 'top_soaring', 'personalized_new_songs', 'top_hot', 'top_new']
  if (songSections.includes(source)) return { code: 200, data: { dailySongs: neteaseSongs(12) } }
  return { code: 200, result: neteasePlaylists(10) }
}

const STAT_PERIOD_SIZES = { day: 4, week: 9, month: 15, year: 21, all: 24 } as const
const STAT_PERIOD_SCALE = { day: 0.08, week: 0.22, month: 0.45, year: 0.8, all: 1 } as const

// 统计页预览：按时间范围从同一批曲目里截取，范围越大曲目越多、时长越长
function playbackStatsOverview() {
  const now = Date.now()
  return (Object.keys(STAT_PERIOD_SIZES) as Array<keyof typeof STAT_PERIOD_SIZES>).map((period) => {
    const scale = STAT_PERIOD_SCALE[period]
    const items = Array.from({ length: STAT_PERIOD_SIZES[period] }, (_, index) => {
      const playCount = Math.max(1, Math.round((48 - index * 1.9) * scale))
      const artist = index % 5 === 1
        ? `${SAMPLE_ARTISTS[index % SAMPLE_ARTISTS.length]} / ${SAMPLE_ARTISTS[(index + 3) % SAMPLE_ARTISTS.length]}`
        : SAMPLE_ARTISTS[index % SAMPLE_ARTISTS.length]
      return {
        identityKey: `preview|netease|${3000 + index}`,
        id: String(3000 + index),
        name: SAMPLE_TITLES[index % SAMPLE_TITLES.length],
        artist,
        album: `Album ${index % 7}`,
        albumId: String(100 + (index % 7)),
        coverUrl: sampleCover(index),
        mediaUri: null,
        durationMs: 180_000 + index * 6_000,
        totalListenMs: Math.round(playCount * (150_000 + ((index * 37_000) % 90_000))),
        playCount,
        lastPlayedAt: now - index * 3_600_000 * (period === 'day' ? 1 : 9),
        firstPlayedAt: now - (index + 1) * 86_400_000 * 11,
      }
    }).sort((left, right) => right.playCount - left.playCount || right.totalListenMs - left.totalListenMs)
    return {
      period,
      totalPlayCount: items.reduce((sum, item) => sum + item.playCount, 0),
      totalListenMs: items.reduce((sum, item) => sum + item.totalListenMs, 0),
      trackCount: items.length,
      items,
    }
  })
}

// 探索页 B 站分类搜索预览：歌曲 / 视频为整段视频行，创作者为圆头像行
function exploreSearch(args: Record<string, unknown> | undefined): unknown {
  if (args?.platform !== 'bilibili') return { items: [], has_more: false }
  const page = Number(args.page) || 1
  if (args.kind === 'creators') {
    const fans = [1_234_567, 89_000, 4_321, 560, 12]
    return {
      items: fans.map((count, index) => ({
        kind: 'artist', platform: 'bilibili', id: String(1000 * page + index),
        name: `${SAMPLE_ARTISTS[(index + page) % SAMPLE_ARTISTS.length]} 官方频道`, cover_url: sampleCover(index),
        subtitle: '', follower_count: count, video_count: 10 + index * 37,
      })),
      has_more: page < 2,
    }
  }
  return {
    items: Array.from({ length: 10 }, (_, index) => ({
      kind: 'song', id: `bilibili:BVpreview${args.kind}${page}${index}`,
      title: `${SAMPLE_TITLES[index % SAMPLE_TITLES.length]}${args.kind === 'videos' ? ' 现场版' : ''}`,
      artist: SAMPLE_ARTISTS[index % SAMPLE_ARTISTS.length], album: '', duration_ms: 200_000 + index * 9_000,
      source: 'bilibili', cover_url: sampleCover(index + 1),
    })),
    has_more: page < 2,
  }
}

function respond(cmd: string, args?: Record<string, unknown>): unknown {
  switch (cmd) {
    case 'explore_search':
      return exploreSearch(args)
    case 'get_netease_home_section':
      return homeSection(args)
    case 'get_settings':
      throw new Error('preview: settings come from localStorage')
    case 'ui_scale_supported':
      return false
    case 'local_library_snapshot':
      return { folders: [{ path: '/Users/preview/Music' }], tracks: sampleTracks(24), scanning: false, skipped: [] }
    case 'list_playlists':
    case 'get_home_local_playlists':
      return samplePlaylists()
    case 'get_local_playlist_tracks':
      return sampleTracks(16)
    case 'get_playlist_tracks':
      return sampleTracks(18).map(track => ({ ...track, durationMs: track.duration_ms, coverUrl: track.cover_url, audioUrl: track.url }))
    case 'get_playlist_usage_stats':
      return {}
    case 'load_user_data_snapshot':
      return null
    case 'get_playback_stats_overview':
      return playbackStatsOverview()
  }
  if (cmd.startsWith('plugin:window|') || cmd.startsWith('plugin:webview|')) return null
  if (/^(list|get_.*s$|load_.*s$)/.test(cmd)) return []
  return null
}

export function installBrowserPreview() {
  mockWindows('main')
  mockIPC((cmd, args) => respond(cmd, args as Record<string, unknown> | undefined), { shouldMockEvents: true })
}
