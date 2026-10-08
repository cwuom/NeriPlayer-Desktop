import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { effectScope, nextTick, shallowReactive } from 'vue'
import ts from 'typescript'

const source = await readFile(new URL('../src/composables/usePlaybackAudioInfoDisplay.ts', import.meta.url), 'utf8')
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
})
const moduleText = outputText.replace(/from ['"]vue['"]/, `from ${JSON.stringify(import.meta.resolve('vue'))}`)
const { usePlaybackAudioInfoDisplay } = await import(`data:text/javascript;base64,${Buffer.from(moduleText).toString('base64')}`)

const first = { source: 'netease', codec: 'FLAC', sampleRateHz: 48000 }
const second = { source: 'youtube', codec: 'Opus', qualityKey: 'high' }
const state = shallowReactive({ info: first, fromDownload: true, loading: false, hasSession: true })
const scope = effectScope()
const display = scope.run(() => usePlaybackAudioInfoDisplay(() => ({ ...state })))
assert.deepEqual(display.value, { info: first, fromDownload: true })

// 取流清空真值时，过渡显示与下载标记保持完整
Object.assign(state, { loading: true, info: null, fromDownload: false })
await nextTick()
assert.equal(state.info, null)
assert.deepEqual(display.value, { info: first, fromDownload: true })

// 连续切歌或解码期间收到新规格，都等待当前会话就绪再替换
state.info = second
await nextTick()
assert.deepEqual(display.value, { info: first, fromDownload: true })
Object.assign(state, { info: null, loading: true })
await nextTick()
assert.equal(display.value.info, first)
Object.assign(state, { info: second, loading: false })
await nextTick()
assert.deepEqual(display.value, { info: second, fromDownload: false })

// 同曲目换音质也不经历空白帧，真实结果可以降级
Object.assign(state, { info: null, loading: true })
await nextTick()
assert.equal(display.value.info, second)
const downgraded = { ...second, codec: 'AAC', qualityKey: 'medium' }
Object.assign(state, { info: downgraded, loading: false })
await nextTick()
assert.equal(display.value.info, downgraded)

// 失败和没有音频规格的本地会话不能残留旧信息
Object.assign(state, { hasSession: false, loading: false, fromDownload: true })
await nextTick()
assert.deepEqual(display.value, { info: null, fromDownload: false })
Object.assign(state, { hasSession: true, info: null, fromDownload: false })
await nextTick()
assert.deepEqual(display.value, { info: null, fromDownload: false })
scope.stop()

// 在加载途中重新打开播放页时，不凭空恢复其它页面的过渡快照
const freshScope = effectScope()
const freshDisplay = freshScope.run(() => usePlaybackAudioInfoDisplay(() => ({
  info: null, fromDownload: false, loading: true, hasSession: true,
})))
assert.deepEqual(freshDisplay.value, { info: null, fromDownload: false })
freshScope.stop()
console.log('playback audio info display tests passed')
