import { readFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'

const manifest = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'))
const tests = Object.entries(manifest.scripts).filter(([name]) => name.startsWith('test:'))
let failed = 0
for (const [name, command] of tests) {
  const match = /^node (scripts\/[a-z0-9.-]+\.mjs)$/.exec(command)
  if (!match) throw new Error(`Unsupported regression command: ${name}`)
  const result = spawnSync(process.execPath, [match[1]], {
    cwd: new URL('../', import.meta.url),
    encoding: 'utf8',
    maxBuffer: 16 * 1024 * 1024,
  })
  if (result.stderr) process.stderr.write(result.stderr)
  if (result.error || result.status !== 0) {
    failed++
    console.error(`FAIL ${name}`)
    if (result.error) console.error(result.error)
    process.stdout.write(result.stdout || '')
  } else {
    console.log(`PASS ${name}`)
  }
}
console.log(`Frontend regression scripts: ${tests.length - failed} passed, ${failed} failed`)
if (failed) process.exitCode = 1
