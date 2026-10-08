import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { bundledLibraries, ffmpegBundleConfig } from './ffmpeg/bundle-config.mjs'

const root = mkdtempSync(join(tmpdir(), 'neri-ffmpeg-bundle-'))
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')
const LIBRARIES = {
  'windows-x86_64': ['avutil-61.dll', 'swresample-7.dll', 'avcodec-63.dll', 'avformat-63.dll'],
  'linux-x86_64': ['libavutil.so.61', 'libswresample.so.7', 'libavcodec.so.63', 'libavformat.so.63'],
  'macos-aarch64': ['libavutil.61.dylib', 'libswresample.7.dylib', 'libavcodec.63.dylib', 'libavformat.63.dylib'],
}

// 与 build-ffmpeg.sh 输出的目录结构和 BUILDINFO.txt 格式一致
function stage(target) {
  const directory = join(root, target)
  mkdirSync(directory, { recursive: true })
  const lines = LIBRARIES[target].map(file => {
    const bytes = Buffer.from(`fake ${file}`)
    writeFileSync(join(directory, file), bytes)
    return `  ${sha256(bytes)}  ${file}  ${bytes.length} bytes`
  })
  writeFileSync(join(directory, 'LICENSE.txt'), 'GNU LESSER GENERAL PUBLIC LICENSE')
  writeFileSync(join(directory, 'BUILDINFO.txt'), [
    `FFmpeg 9.0.2 for NeriPlayer (${target})`,
    `source sha256: ${'a'.repeat(64)}`,
    'configure: --disable-everything',
    'libraries:',
    ...lines,
    '',
  ].join('\n'))
  return directory
}

async function run(name, test) {
  await test()
  console.log(`ok - ${name}`)
}

try {
  await run('Windows and Linux ship the libraries as resources under ffmpeg/', () => {
    for (const target of ['windows-x86_64', 'linux-x86_64']) {
      const directory = stage(target)
      const config = ffmpegBundleConfig(target, root)
      const expected = Object.fromEntries([
        [join(directory, 'LICENSE.txt'), 'ffmpeg/LICENSE.txt'],
        [join(directory, 'BUILDINFO.txt'), 'ffmpeg/BUILDINFO.txt'],
        ...LIBRARIES[target].map(file => [join(directory, file), `ffmpeg/${file}`]),
      ])
      assert.deepEqual(config, { bundle: { resources: expected } })
    }
  })

  await run('macOS puts the libraries into Frameworks so they are signed with the app', () => {
    const directory = stage('macos-aarch64')
    const config = ffmpegBundleConfig('macos-aarch64', root)
    assert.deepEqual(config.bundle.macOS.frameworks, LIBRARIES['macos-aarch64'].map(file => join(directory, file)))
    assert.deepEqual(Object.values(config.bundle.resources), ['ffmpeg/LICENSE.txt', 'ffmpeg/BUILDINFO.txt'])
  })

  await run('a library that differs from BUILDINFO.txt or is missing stops the bundle', () => {
    const directory = stage('windows-x86_64')
    writeFileSync(join(directory, 'avcodec-63.dll'), 'tampered')
    assert.throws(() => ffmpegBundleConfig('windows-x86_64', root), /avcodec-63\.dll does not match BUILDINFO\.txt/)
    rmSync(join(directory, 'avcodec-63.dll'))
    assert.throws(() => ffmpegBundleConfig('windows-x86_64', root), /ENOENT/)
    assert.throws(() => ffmpegBundleConfig('freebsd-x86_64', root), /unknown FFmpeg target/)
  })

  await run('the library list is read only from the libraries section, with any line endings', () => {
    const text = `source sha256: ${'b'.repeat(64)}\r\nlibraries:\r\n  ${'c'.repeat(64)}  avutil-61.dll  10 bytes\r\n`
    assert.deepEqual(bundledLibraries(text), [{ sha256: 'c'.repeat(64), file: 'avutil-61.dll' }])
    assert.deepEqual(bundledLibraries('no libraries here'), [])
  })
} finally {
  rmSync(root, { recursive: true, force: true })
}

console.log('ffmpeg bundle config tests passed')
