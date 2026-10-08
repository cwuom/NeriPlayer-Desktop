import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const sourceUrl = new URL('../src/modules/playback/longFormProgress.ts', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const transpiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  fileName: sourceUrl.pathname,
  reportDiagnostics: true,
})
assert.equal(transpiled.diagnostics?.length ?? 0, 0)
const { LONG_FORM_MIN_DURATION_MS, resolveLongFormResumePosition, longFormPositionForPersistence } =
  await import(`data:text/javascript;base64,${Buffer.from(transpiled.outputText).toString('base64')}`)

const minute = 60_000
const hour = 60 * minute
assert.equal(LONG_FORM_MIN_DURATION_MS, 15 * minute)

// [enabled, duration, requested, remembered, allowRemembered] -> start
for (const [args, expected, why] of [
  [[true, hour, 0, 10 * minute], 10 * minute, 'resumes a long track'],
  [[true, hour, 42_000, 10 * minute], 42_000, 'an explicit position wins'],
  [[false, hour, 0, 10 * minute], 0, 'the setting is off'],
  [[true, 15 * minute - 1000, 0, 5 * minute], 0, 'a 14:59 track is not long-form'],
  [[true, 15 * minute, 0, 5 * minute], 5 * minute, 'exactly 15 minutes is long-form'],
  [[true, hour, 0, 4_000], 0, 'under 5 seconds is not worth resuming'],
  [[true, hour, 0, 5_000], 5_000, '5 seconds resumes'],
  [[true, hour, 0, hour - 30_000], 0, 'the last 30 seconds count as finished'],
  [[true, hour, 0, hour - 30_001], hour - 30_001, 'just before the last 30 seconds resumes'],
  [[true, hour, 0, 10 * minute, false], 0, 'navigation does not resume'],
  [[true, hour, -5, -1], 0, 'negative values are treated as zero'],
]) {
  assert.equal(resolveLongFormResumePosition(...args), expected, why)
}

// [enabled, duration, position] -> stored position (null = keep what is stored)
for (const [args, expected, why] of [
  [[true, hour, 10 * minute], 10 * minute, 'remembers a long track'],
  [[false, hour, 10 * minute], null, 'the setting is off'],
  [[true, 15 * minute - 1000, 10 * minute], null, 'a 14:59 track is not long-form'],
  [[true, hour, 4_000], null, 'the first 5 seconds are not remembered'],
  [[true, hour, hour - 30_000], 0, 'reaching the last 30 seconds clears the position'],
  [[true, hour, hour], 0, 'finishing clears the position'],
]) {
  assert.equal(longFormPositionForPersistence(...args), expected, why)
}

console.log('long-form progress policy tests passed')
