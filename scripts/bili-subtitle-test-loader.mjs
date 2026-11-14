import { readFileSync } from 'node:fs'
import ts from 'typescript'

export function loadModule(file, dependencies = {}) {
  const source = readFileSync(new URL(file, import.meta.url), 'utf8')
  const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const result = {}
  new Function('require', 'exports', compiled)(name => {
    if (!(name in dependencies)) throw new Error(`Unexpected dependency: ${name}`)
    return dependencies[name]
  }, result)
  return result
}

export const format = loadModule('../src/modules/lyrics/lyricsFormat.ts')
export const subtitle = loadModule('../src/modules/lyrics/biliSubtitle.ts', {
  '@/modules/playback/biliVideoSkip': loadModule('../src/modules/playback/biliVideoSkip.ts'),
  './lyricsFormat': format,
})
export function deferred() {
  let resolve, reject
  const promise = new Promise((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
export async function flush() { for (let i = 0; i < 30; i++) await Promise.resolve() }
