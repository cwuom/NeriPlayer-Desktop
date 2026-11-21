// 发布版本号：手动触发的发布用构建时间生成 YY.MMDD.HHMM（UTC+8），例如 2026-11-21 13:40 → 26.1121.1340
//
// 用法：
//   node scripts/release/version.mjs compute <unix 秒>
//     输出 app_version=… 与 msi_version=…，可直接追加到 $GITHUB_OUTPUT
//   node scripts/release/version.mjs stamp <时间戳版本> <Tauri 配置输出文件>
//     把版本写进 package.json、Cargo 工作区与 Cargo.lock，再生成带 MSI 版本的 `tauri build --config` 配置
//
// 语义化版本不允许数字段有前导零，所以 1 月 5 日 09:05 是 27.105.905 而不是 27.0105.0905；
// 各段按数值比较时与时间先后一致，升级判断不受影响。
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const TIMEZONE_OFFSET_SECONDS = 8 * 60 * 60
const SEMVER_CORE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/

function wallClock(epochSeconds) {
  if (!Number.isInteger(epochSeconds) || epochSeconds < 0) {
    throw new Error(`invalid epoch: ${epochSeconds}`)
  }
  // 东八区没有夏令时，固定偏移后按 UTC 取字段即可
  const date = new Date((epochSeconds + TIMEZONE_OFFSET_SECONDS) * 1000)
  return {
    year: date.getUTCFullYear() % 100,
    month: date.getUTCMonth() + 1,
    day: date.getUTCDate(),
    hour: date.getUTCHours(),
    minute: date.getUTCMinutes(),
  }
}

export function timestampVersion(epochSeconds) {
  const { year, month, day, hour, minute } = wallClock(epochSeconds)
  return `${year}.${month * 100 + day}.${hour * 100 + minute}`
}

/**
 * MSI 版本要求 major/minor ≤ 255、其余段 ≤ 65535，YY.MMDD.HHMM 的第二段放不下，
 * 拆成 YY.M.DDHH.MM 后仍随时间单调递增。年初的 27.105.905 本身就放得下，但也必须换算，
 * 否则会和年末换算后的 27.11.… 比较顺序颠倒。
 */
export function msiVersion(appVersion) {
  const match = SEMVER_CORE.exec(appVersion)
  if (!match) throw new Error(`not a plain semver version: ${appVersion}`)
  const [major, minor, patch] = match.slice(1).map(Number)
  const month = Math.floor(minor / 100)
  const day = minor % 100
  const hour = Math.floor(patch / 100)
  const minute = patch % 100
  const isTimestamp = month >= 1 && month <= 12 && day >= 1 && day <= 31
    && hour <= 23 && minute <= 59 && major <= 255
  if (!isTimestamp) throw new Error(`${appVersion} is not a YY.MMDD.HHMM timestamp version`)
  return `${major}.${month}.${day * 100 + hour}.${minute}`
}

export function stampPackageJson(text, version) {
  let replaced = false
  const next = text.replace(/("version"\s*:\s*")[^"]*(")/, (_, head, tail) => {
    replaced = true
    return `${head}${version}${tail}`
  })
  if (!replaced) throw new Error('package.json has no version field')
  return next
}

// Windows 构建机按 core.autocrlf=true 检出时清单是 CRLF：先按 LF 处理，写回时还原原有换行
function preservingLineEndings(text, transform) {
  const crlf = text.includes('\r\n')
  const result = transform(crlf ? text.replace(/\r\n/g, '\n') : text)
  return crlf ? result.replace(/\n/g, '\r\n') : result
}

export function stampCargoToml(text, version) {
  return preservingLineEndings(text, lf => stampCargoTomlLf(lf, version))
}

function stampCargoTomlLf(text, version) {
  let replaced = false
  const next = text.replace(
    /(^\[workspace\.package\]\n(?:(?!\[)[^\n]*\n)*?version\s*=\s*")[^"]*(")/m,
    (_, head, tail) => {
      replaced = true
      return `${head}${version}${tail}`
    },
  )
  if (!replaced) throw new Error('Cargo.toml has no [workspace.package] version')
  return next
}

/** 只改工作区成员（没有 source 字段的包），第三方依赖的版本保持原样 */
export function stampCargoLock(text, fromVersion, toVersion) {
  return preservingLineEndings(text, lf => stampCargoLockLf(lf, fromVersion, toVersion))
}

function stampCargoLockLf(text, fromVersion, toVersion) {
  let count = 0
  const blocks = text.split('\n\n').map(block => {
    if (!block.startsWith('[[package]]') || /\nsource = /.test(block)) return block
    // 没有依赖的成员块以 version 行结尾，后面没有换行，按整行匹配
    const versionLine = new RegExp(`^version = "${fromVersion.replace(/\./g, '\\.')}"$`, 'm')
    if (!versionLine.test(block)) return block
    count++
    return block.replace(versionLine, `version = "${toVersion}"`)
  })
  if (count === 0) throw new Error(`Cargo.lock has no workspace package at ${fromVersion}`)
  return blocks.join('\n\n')
}

export function stampVersion(root, version) {
  if (!SEMVER_CORE.test(version)) throw new Error(`not a plain semver version: ${version}`)
  const packagePath = join(root, 'package.json')
  const cargoTomlPath = join(root, 'src-tauri/Cargo.toml')
  const cargoLockPath = join(root, 'src-tauri/Cargo.lock')

  const packageText = readFileSync(packagePath, 'utf8')
  const current = JSON.parse(packageText).version
  writeFileSync(packagePath, stampPackageJson(packageText, version))
  writeFileSync(cargoTomlPath, stampCargoToml(readFileSync(cargoTomlPath, 'utf8'), version))
  if (current !== version) {
    writeFileSync(cargoLockPath, stampCargoLock(readFileSync(cargoLockPath, 'utf8'), current, version))
  }
}

export function tauriVersionConfig(version) {
  return { bundle: { windows: { wix: { version: msiVersion(version) } } } }
}

function main([command, ...args]) {
  if (command === 'compute') {
    const version = timestampVersion(Number(args[0]))
    console.log(`app_version=${version}`)
    console.log(`msi_version=${msiVersion(version)}`)
    return
  }
  if (command === 'stamp') {
    const [version, configPath] = args
    if (!version || !configPath) throw new Error('usage: stamp <version> <config-output>')
    stampVersion(process.cwd(), version)
    mkdirSync(dirname(resolve(configPath)), { recursive: true })
    writeFileSync(configPath, `${JSON.stringify(tauriVersionConfig(version), null, 2)}\n`)
    return
  }
  throw new Error('usage: version.mjs compute <epoch> | stamp <version> <config-output>')
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2))
}
