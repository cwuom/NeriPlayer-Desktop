import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

// B 站收藏的四种歌单：合集、视频列表要走 UP 主空间归档，不能当收藏夹 id 去查
const source = await readFile(new URL('../src/modules/library/biliPlaylistReference.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const { parseBiliPlaylistReference, biliPlaylistRoute } =
  await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

// 用户数据库里同步来的三条真实记录
assert.deepEqual(parseBiliPlaylistReference('bili-playlist/v1/SERIES/150096/26087398'), { kind: 'SERIES', fid: '150096', mid: '26087398' })
assert.deepEqual(parseBiliPlaylistReference('bili-playlist/v1/COLLECTION/1097758/26087398'), { kind: 'COLLECTION', fid: '1097758', mid: '26087398' })
assert.deepEqual(parseBiliPlaylistReference('bili-playlist/v1/CREATED_FAVORITE/7273337/473400804'), { kind: 'CREATED_FAVORITE', fid: '7273337', mid: '473400804' })

// 与 Android 一样严格：缺段、多段、未知种类、非整数都不认
for (const invalid of [
  null, '', 'VLPL123', 'bili-playlist/v1/SERIES/150096', 'bili-playlist/v1/SERIES/150096/1/2',
  'bili-playlist/v1/PLAYLIST/1/2', 'bili-playlist/v1/SERIES/abc/2', 'bili-playlist/v2/SERIES/1/2',
]) {
  assert.equal(parseBiliPlaylistReference(invalid), null, String(invalid))
}

assert.deepEqual(
  biliPlaylistRoute({ id: '150096', kind: 'SERIES', mid: '26087398', name: '直播回放', coverUrl: 'c.jpg', trackCount: 764, uploader: '猫屋敷梨梨Official' }),
  {
    name: 'bili-playlist', params: { mediaId: '150096' },
    query: { kind: 'series', mid: '26087398', name: '直播回放', cover: 'c.jpg', count: '764', uploader: '猫屋敷梨梨Official' },
  },
  '视频列表带上 UP 主 mid 和种类，详情页才知道该调空间归档接口',
)
assert.deepEqual(
  biliPlaylistRoute({ id: '1097758', kind: 'COLLECTION', mid: '26087398', name: '合集' }).query.kind,
  'collection',
)
assert.deepEqual(biliPlaylistRoute({ id: '727333704', kind: 'CREATED_FAVORITE', mid: '473400804' }), {
  name: 'bili-playlist', params: { mediaId: '727333704' },
})
assert.deepEqual(biliPlaylistRoute({ id: '727333704' }), { name: 'bili-playlist', params: { mediaId: '727333704' } },
  '没有 browseId 的旧收藏按收藏夹打开，与 Android 默认 CREATED_FAVORITE 一致')
assert.equal(biliPlaylistRoute({ id: '150096', kind: 'SERIES' }), null, '缺 UP 主 mid 的视频列表打不开，不能退回收藏夹接口')
assert.equal(biliPlaylistRoute({ id: '150096', kind: 'SERIES', mid: '99999999999999999999' }), null)
assert.equal(biliPlaylistRoute({ id: '0' }), null)
assert.equal(biliPlaylistRoute({ id: '1', kind: 'UNKNOWN' }), null)

console.log('bili playlist reference tests passed')
