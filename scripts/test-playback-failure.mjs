import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/modules/playback/playbackFailure.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const {
  PLAYBACK_FAILURE_REASONS,
  PLAYBACK_REFRESH_COOLDOWN_MS,
  playbackFailureMessageKey,
  retrySongUrlResolution,
  shouldThrottlePlaybackRefresh,
} = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`)

const failure = (message = 'boom', retryable = true) => ({ type: 'failure', message, retryable })
const delays = []
const delay = async ms => { delays.push(ms) }

// 失败最多再试 5 次，间隔 250ms × 次数
let attempts = 0
const exhausted = await retrySongUrlResolution(async () => { attempts++; return failure() }, { delay })
assert.equal(exhausted.type, 'failure')
assert.equal(attempts, 6)
assert.deepEqual(delays, [250, 500, 750, 1000, 1250])

// 成功、登录、等待权威流、不可重试与被取代的失败都立即返回
for (const result of [
  { type: 'success', url: 'https://audio.example/a.mp3' },
  { type: 'requires_login' },
  { type: 'waiting_for_authoritative_stream' },
  failure('restricted', false),
  failure('Audio error: Playback request superseded'),
  failure('Playback source song identity mismatch'),
]) {
  let calls = 0
  await retrySongUrlResolution(async () => { calls++; return result }, { delay })
  assert.equal(calls, 1, JSON.stringify(result))
}

// 请求作废后不再继续重试
let cancelled = 0
let active = true
await retrySongUrlResolution(async () => { cancelled++; active = false; return failure() }, { delay, shouldContinue: () => active })
assert.equal(cancelled, 1)

// 同一曲目 10 秒内再次刷新会被节流
assert.equal(PLAYBACK_REFRESH_COOLDOWN_MS, 10_000)
const last = { key: 'netease:1', at: 1_000 }
assert.equal(shouldThrottlePlaybackRefresh(last, 'netease:1', 1_000 + 9_000), true)
assert.equal(shouldThrottlePlaybackRefresh(last, 'netease:1', 1_000 + 10_000), false)
assert.equal(shouldThrottlePlaybackRefresh(last, 'netease:2', 1_000 + 1_000), false)
assert.equal(shouldThrottlePlaybackRefresh(null, 'netease:1', 0), false)

// 每种失败原因在四种语言里都有提示文案
for (const locale of ['zh-CN', 'en', 'zh-TW', 'ja']) {
  const messages = JSON.parse(await readFile(new URL(`../src/i18n/${locale}.json`, import.meta.url), 'utf8'))
  for (const reason of [...PLAYBACK_FAILURE_REASONS, 'preview_only', 'network_error']) {
    const key = reason === 'preview_only' || reason === 'network_error'
      ? `player.playback_${reason}`
      : playbackFailureMessageKey(reason)
    const value = key.split('.').reduce((node, part) => node?.[part], messages)
    assert.equal(typeof value, 'string', `${locale} is missing ${key}`)
  }
}

console.log('playback failure retry and message tests passed')
