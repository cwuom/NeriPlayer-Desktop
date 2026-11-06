import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

// 封面取字节已抽到 src/utils/imageTransfer.ts（歌词海报共用），这里仍按声明逐个抠出来跑
const utilSource = await readFile(new URL('../src/utils/imageTransfer.ts', import.meta.url), 'utf8')
const parsed = ts.createSourceFile('imageTransfer.ts', utilSource, ts.ScriptTarget.ES2022, true)
const functions = parsed.statements
  .filter(node => (ts.isFunctionDeclaration(node) && ['readImageBytes', 'imageExtension'].includes(node.name?.text))
    || (ts.isVariableStatement(node) && node.declarationList.declarations.some(item => item.name.getText(parsed) === 'DATA_IMAGE_RE')))
  .map(node => node.getText(parsed).replace(/^export\s+/, ''))
  .join('\n')
for (const name of ['readImageBytes', 'imageExtension']) {
  assert.ok(functions.includes(`function ${name}`), `missing cover export declaration: ${name}`)
}
const cacheSource = await readFile(new URL('../src/utils/bilibiliCoverCache.ts', import.meta.url), 'utf8')
const compiledCache = ts.transpileModule(cacheSource, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 },
}).outputText
const { normalizeProxiedCoverUrl } = await import(`data:text/javascript;base64,${Buffer.from(compiledCache).toString('base64')}`)
const fetched = [], proxied = []
const compiled = ts.transpileModule(functions, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText
const { readImageBytes, imageExtension } = new Function('resolveCoverImage', 'normalizeProxiedCoverUrl', 'fetch', `${compiled}; return { readImageBytes, imageExtension }`)(
  async url => {
    if (!normalizeProxiedCoverUrl(url)) throw new Error('Cover URL is invalid')
    proxied.push(url)
    return `data:image/png;base64,${btoa('fixture-png')}`
  },
  normalizeProxiedCoverUrl,
  async url => {
    fetched.push(url)
    return { ok: true, blob: async () => new Blob(['fixture-local'], { type: 'image/webp' }) }
  },
)

const local = await readImageBytes('http://asset.localhost/C%3A/Music/cover.webp')
assert.equal(local.extension, 'webp')
assert.equal(new TextDecoder().decode(local.bytes), 'fixture-local')
assert.deepEqual(proxied, [], 'local asset addresses must bypass the online cover allowlist')
assert.equal((await readImageBytes('asset://localhost/Music/cover.webp')).extension, 'webp')
assert.equal((await readImageBytes('https://p1.music.126.net/fixture.png')).extension, 'png')
assert.equal(proxied.length, 1)
assert.equal((await readImageBytes(`data:image/jpeg;base64,${btoa('fixture-jpeg')}`)).extension, 'jpg')
assert.equal(fetched.length, 2, 'proxied and inline covers do not fall through to browser fetch')
assert.equal(imageExtension('PNG'), 'png')
console.log('local asset, online proxy and inline cover export tests passed')
