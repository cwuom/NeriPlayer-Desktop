// 生成随包分发 FFmpeg 的 Tauri 配置，出安装包时用 `tauri build --config <文件>` 合并
//
// 用法：node scripts/ffmpeg/bundle-config.mjs <target> [构建目录] [输出文件]
//   target：linux-x86_64、windows-x86_64、macos-aarch64、macos-x86_64
//   构建目录：build-ffmpeg.sh 的输出目录，默认 .cache/ffmpeg-build
//   输出文件：默认 <构建目录>/<target>/tauri.bundle.json
//
// 动态库按 BUILDINFO.txt 的清单逐个核对存在与 SHA-256；Windows、Linux 作为资源放进
// 资源目录的 ffmpeg/，macOS 放进 Frameworks（签名应用时一并签名）。
// 不写进 tauri.conf.json：那里的资源每次 cargo build 都会检查，没编 FFmpeg 的开发环境就编不过了。
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const FFMPEG_TARGETS = ['linux-x86_64', 'windows-x86_64', 'macos-aarch64', 'macos-x86_64']

/** 读 BUILDINFO.txt 里的库清单：每行「sha256  文件名  字节数 bytes」 */
export function bundledLibraries(buildInfo) {
  const lines = buildInfo.split(/\r?\n/)
  const start = lines.indexOf('libraries:')
  if (start < 0) return []
  return lines.slice(start + 1)
    .map(line => line.trim().match(/^([0-9a-f]{64})\s+(\S+)\s+\d+ bytes$/))
    .filter(Boolean)
    .map(([, sha256, file]) => ({ sha256, file }))
}

export function ffmpegBundleConfig(target, buildRoot) {
  if (!FFMPEG_TARGETS.includes(target)) throw new Error(`unknown FFmpeg target: ${target}`)
  const directory = resolve(buildRoot, target)
  const buildInfoPath = join(directory, 'BUILDINFO.txt')
  const libraries = bundledLibraries(readFileSync(buildInfoPath, 'utf8'))
  if (libraries.length === 0) throw new Error(`${buildInfoPath} lists no libraries`)
  for (const { sha256, file } of libraries) {
    const actual = createHash('sha256').update(readFileSync(join(directory, file))).digest('hex')
    if (actual !== sha256) throw new Error(`${file} does not match BUILDINFO.txt (sha256 ${actual})`)
  }

  const resources = {
    [join(directory, 'LICENSE.txt')]: 'ffmpeg/LICENSE.txt',
    [buildInfoPath]: 'ffmpeg/BUILDINFO.txt',
  }
  if (target.startsWith('macos-')) {
    return {
      bundle: {
        resources,
        macOS: { frameworks: libraries.map(({ file }) => join(directory, file)) },
      },
    }
  }
  for (const { file } of libraries) resources[join(directory, file)] = `ffmpeg/${file}`
  return { bundle: { resources } }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [target, buildRoot = '.cache/ffmpeg-build', output] = process.argv.slice(2)
  if (!target) {
    console.error('usage: node scripts/ffmpeg/bundle-config.mjs <target> [build-dir] [output]')
    process.exit(2)
  }
  const destination = output ?? join(buildRoot, target, 'tauri.bundle.json')
  writeFileSync(destination, `${JSON.stringify(ffmpegBundleConfig(target, buildRoot), null, 2)}\n`)
  console.log(destination)
}
