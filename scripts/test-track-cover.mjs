import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const sourceUrl = new URL('../src/utils/trackCover.ts', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ES2022,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const { COVER_SIZE_DETAIL, COVER_SIZE_LIST, getTrackCoverUrl, upgradeYouTubeThumbnailUrl, withCoverSize } = await import(moduleUrl)

assert.equal(getTrackCoverUrl(null), '')
assert.equal(getTrackCoverUrl({ coverUrl: '  https://img.example/direct.jpg  ' }), 'https://img.example/direct.jpg')
assert.equal(
  getTrackCoverUrl({
    coverUrl: '',
    syncPayload: { custom_cover_url: 'https://img.example/custom.jpg' },
  }),
  'https://img.example/custom.jpg',
)
assert.equal(
  getTrackCoverUrl({
    syncPayload: { originalCoverUrl: 'https://img.example/original.jpg' },
  }),
  'https://img.example/original.jpg',
)
assert.equal(
  getTrackCoverUrl({
    coverUrl: 'https://img.example/direct.jpg',
    syncPayload: { customCoverUrl: 'https://img.example/custom.jpg' },
  }),
  'https://img.example/direct.jpg',
)

// YouTube 缩略图尺寸：列表 226（解码约 0.2 MB），详情 1200
assert.equal(
  upgradeYouTubeThumbnailUrl('https://lh3.googleusercontent.com/abc123=w226-h226-l90-rj'),
  'https://lh3.googleusercontent.com/abc123=w226-h226',
)
assert.equal(
  upgradeYouTubeThumbnailUrl('https://lh3.googleusercontent.com/abc123=w60-h60-l90-rj'),
  'https://lh3.googleusercontent.com/abc123=w226-h226',
)
assert.equal(
  upgradeYouTubeThumbnailUrl('https://lh3.googleusercontent.com/abc123'),
  'https://lh3.googleusercontent.com/abc123=w226-h226',
)
assert.equal(
  upgradeYouTubeThumbnailUrl('https://yt3.ggpht.com/abc123=w120-h120'),
  'https://yt3.ggpht.com/abc123=w226-h226',
)
assert.equal(
  upgradeYouTubeThumbnailUrl('https://yt3.ggpht.com/avatar=s88'),
  'https://yt3.ggpht.com/avatar=w226-h226',
)
assert.equal(
  upgradeYouTubeThumbnailUrl('https://lh3.googleusercontent.com/abc123=w60-h60', 'detail'),
  'https://lh3.googleusercontent.com/abc123=w1200-h1200',
)
assert.equal(
  upgradeYouTubeThumbnailUrl('https://i.ytimg.com/vi/abc123/maxresdefault.jpg'),
  'https://i.ytimg.com/vi/abc123/maxresdefault.jpg',
)
assert.equal(upgradeYouTubeThumbnailUrl(''), '')
assert.equal(upgradeYouTubeThumbnailUrl('  '), '  ')

// getTrackCoverUrl 应对 YouTube 封面自动升清
assert.equal(
  getTrackCoverUrl({
    coverUrl: 'https://lh3.googleusercontent.com/cover=w60-h60-l90-rj',
  }),
  'https://lh3.googleusercontent.com/cover=w226-h226',
)
assert.equal(
  getTrackCoverUrl({ coverUrl: 'https://lh3.googleusercontent.com/cover=w60-h60' }, 'detail'),
  'https://lh3.googleusercontent.com/cover=w1200-h1200',
)

// 网易云封面：按展示尺寸追加 ?param，幂等且只升不降
assert.equal(
  withCoverSize('https://p1.music.126.net/abc.jpg', COVER_SIZE_LIST),
  'https://p1.music.126.net/abc.jpg?param=300y300',
)
assert.equal(
  withCoverSize('https://p1.music.126.net/abc.jpg?param=300y300', COVER_SIZE_LIST),
  'https://p1.music.126.net/abc.jpg?param=300y300',
)
assert.equal(
  withCoverSize('https://p1.music.126.net/abc.jpg?param=300y300', COVER_SIZE_DETAIL),
  'https://p1.music.126.net/abc.jpg?param=640y640',
)
assert.equal(
  withCoverSize('https://p1.music.126.net/abc.jpg?param=1200y1200', COVER_SIZE_LIST),
  'https://p1.music.126.net/abc.jpg?param=1200y1200',
)
assert.equal(
  withCoverSize('https://p1.music.126.net/abc.jpg?param=1200y1200', COVER_SIZE_LIST, true),
  'https://p1.music.126.net/abc.jpg?param=300y300',
  'list requests downgrade an oversized cover to the exact list size',
)
assert.equal(
  withCoverSize('https://p1.music.126.net/abc.jpg?param=1200y1200', COVER_SIZE_DETAIL),
  'https://p1.music.126.net/abc.jpg?param=1200y1200',
  'detail requests keep a larger existing parameter',
)
assert.equal(
  getTrackCoverUrl({ coverUrl: 'https://p1.music.126.net/abc.jpg?param=1200y1200' }),
  'https://p1.music.126.net/abc.jpg?param=300y300',
)
assert.equal(
  withCoverSize('https://img.example/abc.jpg', COVER_SIZE_LIST),
  'https://img.example/abc.jpg',
)

// original export removes only the suffix this app manages, on known hosts
assert.equal(
  getTrackCoverUrl({ coverUrl: 'https://lh3.googleusercontent.com/abc=w1200-h1200-l90-rj' }, 'original'),
  'https://lh3.googleusercontent.com/abc',
)
assert.equal(
  getTrackCoverUrl({ coverUrl: 'https://yt3.ggpht.com/avatar=s120' }, 'original'),
  'https://yt3.ggpht.com/avatar',
)
assert.equal(
  getTrackCoverUrl({ coverUrl: 'https://p1.music.126.net/abc.jpg?param=300y300&foo=1' }, 'original'),
  'https://p1.music.126.net/abc.jpg?foo=1',
)
assert.equal(
  getTrackCoverUrl({ coverUrl: 'https://img.example/abc.jpg?param=300y300' }, 'original'),
  'https://img.example/abc.jpg?param=300y300',
)

console.log('track cover tests passed')
