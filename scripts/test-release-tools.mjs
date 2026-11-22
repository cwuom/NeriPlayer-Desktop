import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { cpSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  stampCargoLock,
  stampCargoToml,
  stampVersion,
  tauriVersionConfig,
  timestampVersion,
} from './release/version.mjs'
import { badgeUrl, classifyAssets, renderDownloadTable } from './release/download-table.mjs'

const epochAt = iso => Date.parse(iso) / 1000
const versionScript = fileURLToPath(new URL('./release/version.mjs', import.meta.url))

function run(name, test) {
  test()
  console.log(`ok - ${name}`)
}

run('compute emits matching public and MSI versions for consecutive minutes', () => {
  for (const [time, version] of [
    ['2026-11-21T13:40:00+08:00', '26.11.29620'],
    ['2026-11-21T13:41:00+08:00', '26.11.29621'],
  ]) {
    const output = execFileSync(process.execPath, [versionScript, 'compute', String(epochAt(time))], { encoding: 'utf8' })
    assert.equal(output.trimEnd(), `app_version=${version}\nmsi_version=${version}`)
  }
})

run('UTC+8 month and year boundaries fit the three MSI fields', () => {
  assert.equal(timestampVersion(epochAt('2026-10-31T15:59:00Z')), '26.10.44639')
  assert.equal(timestampVersion(epochAt('2026-10-31T16:00:00Z')), '26.11.0')
  assert.equal(timestampVersion(epochAt('2026-12-31T15:59:00Z')), '26.12.44639')
  assert.equal(timestampVersion(epochAt('2026-12-31T16:00:00Z')), '27.1.0')
  assert.throws(() => timestampVersion(-1))
  assert.throws(() => timestampVersion(Number.NaN))
  assert.throws(() => timestampVersion(Number.MAX_SAFE_INTEGER))
})

run('manual releases reject out-of-range version fields', () => {
  assert.equal(tauriVersionConfig('28.2.41759').bundle.windows.wix.version, '28.2.41759')
  for (const version of ['100.1.0', '26.0.0', '26.13.0', '26.12.44640', '26.2.40320', '26.01.0', '26.1121.1340']) {
    assert.throws(() => tauriVersionConfig(version))
  }
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
  const stamped = stampCargoToml(toml, '26.11.29620')
  assert.match(stamped, /\[workspace\.package\]\nversion = "26\.11\.29620"/)
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
  const stamped = stampCargoLock(lock, '1.0.0', '26.11.29620')
  assert.match(stamped, /name = "neri-common"\nversion = "26\.11\.29620"/)
  assert.match(stamped, /name = "neri-player-desktop"\nversion = "26\.11\.29620"/)
  assert.match(stamped, /name = "third-party"\nversion = "1\.0\.0"/)
  assert.throws(() => stampCargoLock(lock, '9.9.9', '26.11.29620'))
})

run('Cargo manifests checked out with CRLF are stamped and keep CRLF', () => {
  const crlf = text => text.replace(/\r?\n/g, '\r\n')
  const toml = stampCargoToml(crlf(readFileSync('src-tauri/Cargo.toml', 'utf8')), '26.11.29620')
  assert.match(toml, /\[workspace\.package\]\r\nversion = "26\.11\.29620"\r\n/)
  assert.ok(!/[^\r]\n/.test(toml))
  const lock = stampCargoLock(crlf(readFileSync('src-tauri/Cargo.lock', 'utf8')), '1.0.0', '26.11.29620')
  assert.match(lock, /name = "neri-player-desktop"\r\nversion = "26\.11\.29620"\r\n/)
  assert.match(lock, /name = "neri-common"\r\nversion = "26\.11\.29620"/)
  assert.ok(!/[^\r]\n/.test(lock))
})

run('stamp writes the same version to the manifests and WiX config', () => {
  const root = mkdtempSync(join(tmpdir(), 'neri-release-version-'))
  try {
    cpSync('package.json', join(root, 'package.json'))
    cpSync('src-tauri/Cargo.toml', join(root, 'src-tauri/Cargo.toml'))
    cpSync('src-tauri/Cargo.lock', join(root, 'src-tauri/Cargo.lock'))
    const configPath = join(root, 'config', 'tauri.version.json')
    execFileSync(process.execPath, [versionScript, 'stamp', '26.11.29620', configPath], { cwd: root })
    assert.equal(JSON.parse(readFileSync(configPath, 'utf8')).bundle.windows.wix.version, '26.11.29620')
    assert.equal(JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version, '26.11.29620')
    assert.match(readFileSync(join(root, 'src-tauri/Cargo.toml'), 'utf8'), /\[workspace\.package\]\r?\nversion = "26\.11\.29620"/)
    const lock = readFileSync(join(root, 'src-tauri/Cargo.lock'), 'utf8')
    assert.match(lock, /name = "neri-player-desktop"\r?\nversion = "26\.11\.29620"/)
    assert.throws(() => stampVersion(root, '26.0105.0905'))
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

const ASSETS = [
  'NeriPlayer_26.11.29620_x64-setup.exe',
  'NeriPlayer_26.11.29620_x64_en-US.msi',
  'NeriPlayer_26.11.29620_aarch64-adhoc.dmg',
  'NeriPlayer_26.11.29620_x64-adhoc.dmg',
  'NeriPlayer_26.11.29620_amd64.AppImage.tar.gz',
  'NeriPlayer_26.11.29620_amd64.deb',
  'NeriPlayer-26.11.29620-1.x86_64.rpm',
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
    tag: 'NeriPlayer-26.11.29620',
    files: ASSETS,
    previousTag: 'NeriPlayer-26.11.27900',
  })
  assert.match(markdown, /^### Download based on your OS:/)
  assert.match(markdown, /\| Windows \| \[!\[Setup x64\]/)
  assert.match(markdown, /releases\/download\/NeriPlayer-26\.11\.29620\/NeriPlayer_26\.11\.29620_x64-setup\.exe\)/)
  assert.match(markdown, /\| macOS \| .*Apple Silicon.*<br>.*Intel x64/)
  assert.ok(!markdown.includes('ffmpeg-8.0'))
  assert.match(markdown, /\[ChangeLog\]\(https:\/\/github\.com\/owner\/repo\/compare\/NeriPlayer-26\.11\.27900\.\.\.NeriPlayer-26\.11\.29620\)/)
  const withoutPrevious = renderDownloadTable({ repo: 'owner/repo', tag: 'v1.0.0', files: ASSETS })
  assert.match(withoutPrevious, /\[ChangeLog\]\(https:\/\/github\.com\/owner\/repo\/commits\/v1\.0\.0\)/)
  assert.throws(() => renderDownloadTable({ repo: 'owner/repo', tag: 'v1', files: ['notes.txt'] }))
})
