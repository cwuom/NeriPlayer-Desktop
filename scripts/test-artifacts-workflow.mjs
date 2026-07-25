import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'

const repositoryRoot = path.resolve(import.meta.dirname, '..')
const workflowPath = path.join(repositoryRoot, '.github', 'workflows', 'artifacts.yml')
const workflow = fs.readFileSync(workflowPath, 'utf8')

assert.match(
  workflow,
  /name:\s+NeriPlayer-\$\{\{ matrix\.artifact_name \}\}\s*\n\s+path:\s+artifacts\/\*/,
  'the installer artifact upload must remain available',
)

assert.match(
  workflow,
  /name:\s+NeriPlayer-\$\{\{ matrix\.artifact_name \}\}-portable\s*\n\s+path:\s+portable\/?\s*\n\s+if-no-files-found:\s+error/,
  'each build must upload a separate portable artifact directory',
)

for (const marker of [
  'portable_dir="portable"',
  'portable_stage_dir="portable-stage"',
  'mkdir -p "$portable_dir/windows"',
  'mkdir -p "$portable_stage_dir/macos"',
  'mkdir -p "$portable_stage_dir/linux"',
  'NeriPlayer.exe',
  'NeriPlayer.desktop',
  'icon.png',
  'README.md',
  'LICENSE',
]) {
  assert.ok(workflow.includes(marker), `portable packaging is missing ${marker}`)
}

assert.match(
  workflow,
  /case\s+"\$\{\{ matrix\.artifact_name \}\}"\s+in[\s\S]*macOS-\*\)[\s\S]*Windows-\*\)[\s\S]*Linux-\*\)/,
  'portable packaging must cover macOS, Windows, and Linux matrix entries',
)

assert.ok(!workflow.includes('-maxdepth'), 'portable packaging must use a BSD-compatible macOS app lookup')

const portableArchives = workflow.match(
  /tar -czf "\$portable_dir\/NeriPlayer-\$\{\{ matrix\.artifact_name \}\}-portable\.tar\.gz"/g,
)
assert.equal(portableArchives?.length, 2, 'macOS and Linux portable packages must preserve executable permissions')
assert.doesNotMatch(workflow, /docs\/superpowers|\.codex/, 'portable packaging must not include Codex process files')

console.log('artifacts workflow portable packaging contract passed')
