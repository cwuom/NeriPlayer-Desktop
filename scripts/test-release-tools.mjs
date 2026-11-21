import assert from 'node:assert/strict'
import { cpSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  msiVersion,
  stampCargoLock,
  stampCargoToml,
  stampVersion,
  tauriVersionConfig,
  timestampVersion,
} from './release/version.mjs'
import { badgeUrl, classifyAssets, renderDownloadTable } from './release/download-table.mjs'

const epochAt = iso => Date.parse(iso) / 1000

function run(name, test) {
  test()
  console.log(`ok - ${name}`)
}

run('timestamp version follows UTC+8 wall clock', () => {
  assert.equal(timestampVersion(epochAt('2026-11-21T13:40:00+08:00')), '26.1121.1340')
  // UTC 当天 16:30 已是东八区次日
  assert.equal(timestampVersion(epochAt('2026-12-31T16:30:00Z')), '27.101.30')
})

run('timestamp version drops leading zeros and stays monotonic', () => {
  const earlier = timestampVersion(epochAt('2027-01-05T09:05:00+08:00'))
  const later = timestampVersion(epochAt('2027-01-05T10:00:00+08:00'))
  assert.equal(earlier, '27.105.905')
  const parts = version => version.split('.').map(Number)
  assert.ok(parts(later)[2] > parts(earlier)[2])
  assert.throws(() => timestampVersion(-1))
  assert.throws(() => timestampVersion(Number.NaN))
})

run('MSI version fits Windows Installer limits', () => {
  assert.equal(msiVersion('26.1121.1340'), '26.11.2113.40')
  assert.equal(msiVersion('27.105.905'), '27.1.509.5')
  // 年初的版本本身不超限也要换算，否则会排在年末换算结果之后
  const msiParts = version => msiVersion(version).split('.').map(Number)
  assert.ok(msiParts('27.105.905')[1] < msiParts('27.1121.1340')[1])
  assert.throws(() => msiVersion('1.2.3'))
  assert.throws(() => msiVersion('26.1121.2460'))
  assert.throws(() => msiVersion('26.0105.0905'))
  assert.deepEqual(tauriVersionConfig('26.1121.1340'), { bundle: { windows: { wix: { version: '26.11.2113.40' } } } })
})

run('Cargo.toml stamping only touches the workspace package version', () => {
  const toml = [
    '[package]',
    'name = "app"',
    'version.workspace = true',
    '',
    '[workspace.package]',
    'version = "1.0.0"',
    'edition = "2021"',
    '',
    '[workspace.dependencies]',
    'serde = { version = "1" }',
    '',
  ].join('\n')
  const stamped = stampCargoToml(toml, '26.1121.1340')
  assert.match(stamped, /\[workspace\.package\]\nversion = "26\.1121\.1340"/)
  assert.match(stamped, /serde = \{ version = "1" \}/)
  assert.throws(() => stampCargoToml('[package]\nversion = "1.0.0"\n', '2.0.0'))
})

run('Cargo.lock stamping skips registry packages with the same version', () => {
  const lock = [
    'version = 4',
    '',
    '[[package]]\nname = "neri-common"\nversion = "1.0.0"',
    '[[package]]\nname = "third-party"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"',
    '[[package]]\nname = "neri-player-desktop"\nversion = "1.0.0"\ndependencies = [\n "neri-common",\n]',
    '',
  ].join('\n\n')
  const stamped = stampCargoLock(lock, '1.0.0', '26.1121.1340')
  assert.match(stamped, /name = "neri-common"\nversion = "26\.1121\.1340"/)
  assert.match(stamped, /name = "neri-player-desktop"\nversion = "26\.1121\.1340"/)
  assert.match(stamped, /name = "third-party"\nversion = "1\.0\.0"/)
  assert.throws(() => stampCargoLock(lock, '9.9.9', '26.1121.1340'))
})

run('Cargo manifests checked out with CRLF are stamped and keep CRLF', () => {
  const crlf = text => text.replace(/\r?\n/g, '\r\n')
  const toml = stampCargoToml(crlf(readFileSync('src-tauri/Cargo.toml', 'utf8')), '26.1121.1340')
  assert.match(toml, /\[workspace\.package\]\r\nversion = "26\.1121\.1340"\r\n/)
  assert.ok(!/[^\r]\n/.test(toml))
  const lock = stampCargoLock(crlf(readFileSync('src-tauri/Cargo.lock', 'utf8')), '1.0.0', '26.1121.1340')
  assert.match(lock, /name = "neri-player-desktop"\r\nversion = "26\.1121\.1340"\r\n/)
  assert.match(lock, /name = "neri-common"\r\nversion = "26\.1121\.1340"/)
  assert.ok(!/[^\r]\n/.test(lock))
})

run('stampVersion keeps the repository manifests consistent', () => {
  const root = mkdtempSync(join(tmpdir(), 'neri-release-version-'))
  try {
    cpSync('package.json', join(root, 'package.json'))
    cpSync('src-tauri/Cargo.toml', join(root, 'src-tauri/Cargo.toml'))
    cpSync('src-tauri/Cargo.lock', join(root, 'src-tauri/Cargo.lock'))
    stampVersion(root, '26.1121.1340')
    assert.equal(JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version, '26.1121.1340')
    assert.match(readFileSync(join(root, 'src-tauri/Cargo.toml'), 'utf8'), /\[workspace\.package\]\nversion = "26\.1121\.1340"/)
    const lock = readFileSync(join(root, 'src-tauri/Cargo.lock'), 'utf8')
    assert.match(lock, /name = "neri-player-desktop"\nversion = "26\.1121\.1340"/)
    assert.throws(() => stampVersion(root, '26.0105.0905'))
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

const ASSETS = [
  'NeriPlayer_26.1121.1340_x64-setup.exe',
  'NeriPlayer_26.1121.1340_x64_en-US.msi',
  'NeriPlayer_26.1121.1340_aarch64-adhoc.dmg',
  'NeriPlayer_26.1121.1340_x64-adhoc.dmg',
  'NeriPlayer_26.1121.1340_amd64.AppImage.tar.gz',
  'NeriPlayer_26.1121.1340_amd64.deb',
  'NeriPlayer-26.1121.1340-1.x86_64.rpm',
  'ffmpeg-8.0.tar.xz',
  'SHA256SUMS.txt',
  'macOS-install.txt',
]

run('assets are classified per OS and unrelated files are skipped', () => {
  const entries = classifyAssets(ASSETS)
  assert.deepEqual(entries.map(entry => `${entry.os}:${entry.label}:${entry.message}`), [
    'Windows:Setup:x64',
    'Windows:MSI:x64',
    'macOS:DMG:Apple Silicon',
    'macOS:DMG:Intel x64',
    'Linux:AppImage:x64',
    'Linux:DebPackage:x64',
    'Linux:RpmPackage:x64',
  ])
})

run('badge text escapes shields.io separators', () => {
  assert.equal(
    badgeUrl({ label: 'DMG', message: 'Apple Silicon', color: '000000', logo: 'apple' }),
    'https://img.shields.io/badge/DMG-Apple%20Silicon-000000?logo=apple&logoColor=white',
  )
  assert.equal(badgeUrl({ label: 'a-b_c', message: 'x64', color: 'fff' }), 'https://img.shields.io/badge/a--b__c-x64-fff')
})

run('download table links every installer to the release tag', () => {
  const markdown = renderDownloadTable({
    repo: 'owner/repo',
    tag: 'NeriPlayer-26.1121.1340',
    files: ASSETS,
    previousTag: 'NeriPlayer-26.1120.900',
  })
  assert.match(markdown, /^### Download based on your OS:/)
  assert.match(markdown, /\| Windows \| \[!\[Setup x64\]/)
  assert.match(markdown, /releases\/download\/NeriPlayer-26\.1121\.1340\/NeriPlayer_26\.1121\.1340_x64-setup\.exe\)/)
  assert.match(markdown, /\| macOS \| .*Apple Silicon.*<br>.*Intel x64/)
  assert.ok(!markdown.includes('ffmpeg-8.0'))
  assert.match(markdown, /\[ChangeLog\]\(https:\/\/github\.com\/owner\/repo\/compare\/NeriPlayer-26\.1120\.900\.\.\.NeriPlayer-26\.1121\.1340\)/)
  const withoutPrevious = renderDownloadTable({ repo: 'owner/repo', tag: 'v1.0.0', files: ASSETS })
  assert.match(withoutPrevious, /\[ChangeLog\]\(https:\/\/github\.com\/owner\/repo\/commits\/v1\.0\.0\)/)
  assert.throws(() => renderDownloadTable({ repo: 'owner/repo', tag: 'v1', files: ['notes.txt'] }))
})
