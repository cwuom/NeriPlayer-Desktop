// ts_complexity 的回归测试：逐函数复杂度必须与 ESLint 内置 complexity 规则一致
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import test from 'node:test'
import { analyzeSource } from './ts_complexity.mjs'

const require = createRequire(import.meta.url)
const { ESLint } = require('eslint')

const FIXTURE = `
export function branches(a?: { b?: () => number }, flag = false): number {
  if (a?.b && flag) return a.b?.() ?? 0
  for (const value of [1, 2]) { if (value > 1) continue }
  while (flag) { flag = false }
  do { flag ||= Boolean(a) } while (!flag)
  switch (Number(flag)) { case 0: return 1; case 1: return 2; default: return 3 }
}
export const arrow = (value: number) => (value > 0 ? value : -value)
class Holder {
  method({ x = 1 } = {}) { try { return x } catch { return 0 } }
  outer() { const inner = () => (this ? 1 : 2); return inner() }
}
export { Holder }
`

test('per-function complexity matches the ESLint complexity rule', async () => {
  const eslint = new ESLint({
    overrideConfigFile: true,
    overrideConfig: {
      files: ['**/*.ts'],
      languageOptions: { parser: require('typescript-eslint').parser },
      rules: { complexity: ['warn', 0] },
    },
  })
  const [result] = await eslint.lintText(FIXTURE, { filePath: 'fixture.ts' })
  const expected = result.messages
    .filter((message) => message.ruleId === 'complexity' && !message.message.startsWith('Class field'))
    .map((message) => Number(/complexity of (\d+)/.exec(message.message)[1]))
    .sort((a, b) => a - b)
  const actual = analyzeSource('fixture.ts', FIXTURE).map((entry) => entry.complexity).sort((a, b) => a - b)
  assert.deepEqual(actual, expected)
})

test('reports names and 1-based line ranges for nested functions', () => {
  const functions = analyzeSource('fixture.ts', FIXTURE)
  assert.deepEqual(functions.map((entry) => entry.name), ['branches', 'arrow', 'method', 'outer', 'inner'])
  const branches = functions.find((entry) => entry.name === 'branches')
  assert.equal(branches.start.line, 2)
  assert.equal(branches.end.line, 8)
  assert.equal(functions.find((entry) => entry.name === 'inner').start.line, 12)
})
