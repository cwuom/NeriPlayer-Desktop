import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const sourceUrl = new URL('../src/views/HomeView.vue', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')

assert.match(source, /key:\s*['"]netease['"]/)
assert.doesNotMatch(source, /key:\s*['"]bilibili['"]/)
assert.doesNotMatch(source, /key:\s*['"]youtube['"]/)
assert.doesNotMatch(source, /query:\s*\{\s*platform:\s*['"](?:bilibili|youtube)['"]\s*\}/)
assert.doesNotMatch(source, /youtubeHomeItems/)
assert.doesNotMatch(source, /fetchHomeFeed\s*\(/)
assert.doesNotMatch(source, /auth\.(?:bilibili|youtube)/)
assert.match(source, /userPlaylists\[['"]netease['"]\]/)
assert.match(
  source,
  /subtitle:\s*!auth\.netease\.loggedIn[\s\S]*login_for_recommend/,
  'the NetEase hub must use auth state, not playlist length, for its login message',
)
assert.doesNotMatch(
  source,
  /recommendedPlaylists\.length\s*>\s*0\s*\?[^\n]*login_for_recommend/,
  'an empty or still-loading recommendation response must not look logged out',
)
assert.match(
  source,
  /const\s+neteaseSessionFingerprint\s*=\s*computed\(\(\)\s*=>\s*`\$\{auth\.netease\.loggedIn\s*\?\s*['"]1['"]\s*:\s*['"]0['"]\}:\$\{auth\.neteaseSessionVersion\}`\s*,?\s*\)/,
  'Home must observe account/session changes even when loggedIn stays true',
)
assert.match(source, /watch\(neteaseSessionFingerprint\s*,/)
assert.doesNotMatch(
  source,
  /watch\(\(\)\s*=>\s*auth\.netease\.loggedIn\s*,/,
  'Home must not reduce the NetEase session boundary to a login boolean',
)

const homeMount = source.slice(
  source.indexOf('onMounted(() =>'),
  source.indexOf('// 登录状态变化'),
)
assert.match(
  homeMount,
  /if\s*\(auth\.netease\.loggedIn\)\s*\{[\s\S]*fetchRecommendedPlaylists\(\)/,
  'Home must wait for verified Netease auth before requesting personalized shelves',
)
for (const section of ['recommend\\.recommendedPlaylists', 'dailySongs', 'myPlaylists']) {
  assert.match(
    source,
    new RegExp(`<section v-if="auth\\.netease\\.loggedIn && ${section}\\.length > 0"`),
    `${section} must not render before Netease auth has been verified`,
  )
}

console.log('home netease surface tests passed')
