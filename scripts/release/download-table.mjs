// 生成发布说明顶部的「按系统下载」表格：每个安装包一枚 shields.io 徽章，直接链到发布附件
//
// 用法：node scripts/release/download-table.mjs <owner/repo> <tag> <附件目录> [上一个 tag]
// 按实际附件文件名分类，不认识的文件（FFmpeg 源码、校验和等）不进表格。
import { readdirSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

// 顺序即表格里的行序与徽章顺序；Windows 徽章不带图标：simple-icons 已移除微软商标
const RULES = [
  { os: 'Windows', pattern: /-setup\.exe$/i, label: 'Setup', color: '3a7ca5' },
  { os: 'Windows', pattern: /\.msi$/i, label: 'MSI', color: '5ba4cf' },
  { os: 'macOS', pattern: /aarch64.*\.dmg$/i, label: 'DMG', message: 'Apple Silicon', color: '000000', logo: 'apple' },
  { os: 'macOS', pattern: /(x64|x86_64).*\.dmg$/i, label: 'DMG', message: 'Intel x64', color: '3d9ad1', logo: 'apple' },
  { os: 'Linux', pattern: /\.AppImage\.tar\.gz$/i, label: 'AppImage', color: 'e05d44', logo: 'linux' },
  { os: 'Linux', pattern: /\.deb$/i, label: 'DebPackage', color: 'e8956a', logo: 'debian' },
  { os: 'Linux', pattern: /\.rpm$/i, label: 'RpmPackage', color: 'dfb317', logo: 'redhat' },
]

function architecture(file) {
  if (/aarch64|arm64/i.test(file)) return 'ARM64'
  if (/x86_64|x64|amd64/i.test(file)) return 'x64'
  return null
}

// Tauri 的 WiX 包名以语言结尾，例如 NeriPlayer_1.0.0_x64_zh-CN.msi
function msiLanguage(file) {
  return file.match(/_([a-z]{2}-[A-Z]{2})\.msi$/)?.[1] ?? null
}

// shields.io 静态徽章路径里 - 和 _ 是分隔符，字面量要写成 -- 与 __
function shieldsText(text) {
  return encodeURIComponent(text.replace(/-/g, '--').replace(/_/g, '__'))
}

export function badgeUrl({ label, message, color, logo }) {
  const query = logo ? `?logo=${logo}&logoColor=white` : ''
  return `https://img.shields.io/badge/${shieldsText(label)}-${shieldsText(message)}-${color}${query}`
}

export function classifyAssets(files) {
  const entries = []
  for (const rule of RULES) {
    for (const file of [...files].sort()) {
      if (!rule.pattern.test(file)) continue
      const arch = rule.message ?? architecture(file)
      if (!arch) throw new Error(`cannot tell the architecture of ${file}`)
      // MSI 一个包只有一种界面语言，同一架构会有多个，徽章上要能区分
      const language = rule.label === 'MSI' ? msiLanguage(file) : null
      const message = language ? `${arch} ${language}` : arch
      entries.push({ ...rule, message, file })
    }
  }
  return entries
}

export function renderDownloadTable({ repo, tag, files, previousTag }) {
  const entries = classifyAssets(files)
  if (entries.length === 0) throw new Error('no installable release assets found')

  const base = `https://github.com/${repo}`
  const downloadUrl = file => `${base}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(file)}`
  const rows = []
  for (const os of [...new Set(entries.map(entry => entry.os))]) {
    const badges = entries
      .filter(entry => entry.os === os)
      .map(entry => `[![${entry.label} ${entry.message}](${badgeUrl(entry)})](${downloadUrl(entry.file)})`)
    rows.push(`| ${os} | ${badges.join('<br>')} |`)
  }
  const changelog = previousTag
    ? `${base}/compare/${encodeURIComponent(previousTag)}...${encodeURIComponent(tag)}`
    : `${base}/commits/${encodeURIComponent(tag)}`

  return [
    '### Download based on your OS:',
    '',
    '| OS | Download |',
    '| --- | --- |',
    ...rows,
    '',
    `List of all changes: [ChangeLog](${changelog})`,
    '',
  ].join('\n')
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [repo, tag, assetsDir, previousTag] = process.argv.slice(2)
  if (!repo || !tag || !assetsDir) {
    throw new Error('usage: download-table.mjs <owner/repo> <tag> <assets-dir> [previous-tag]')
  }
  process.stdout.write(renderDownloadTable({ repo, tag, files: readdirSync(assetsDir), previousTag }))
}
