import { spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'

const require = createRequire(import.meta.url)
const tauriCli = require.resolve('@tauri-apps/cli/tauri.js')

// linuxdeploy bundles an old strip (no SHT_RELR support) that fails on
// distros whose system libs use RELR relocations (Arch/Fedora families):
// "unknown type [0x13] section `.relr.dyn'". NO_STRIP skips stripping
// entirely; only affects Linux AppImage bundling.
function isRelrDistro() {
  try {
    const release = readFileSync('/etc/os-release', 'utf8')
    return /^ID(?:_LIKE)?=.*\b(?:arch|fedora)\b/m.test(release)
  } catch {
    return false
  }
}

// linuxdeploy-plugin-gtk 的符号链接兜底（linuxdeploy/linuxdeploy-plugin-gtk#24
// 方案）不带 -f：linuxdeploy 已把 GTK 模块铺平到 $APPDIR/usr/lib 时，
// `ln: 文件已存在` 会让插件 set -e 直接失败（Arch 等发行版必现）。
// 幂等打补丁：ln -s -> ln -sf，符号链接目标不变，可安全覆盖平铺副本。
function patchGtkPluginSymlinks() {
  try {
    const cacheDir = process.env.XDG_CACHE_HOME
      ? `${process.env.XDG_CACHE_HOME}/tauri`
      : `${process.env.HOME || ''}/.cache/tauri`
    const pluginPath = `${cacheDir}/linuxdeploy-plugin-gtk.sh`
    if (!existsSync(pluginPath)) return
    const content = readFileSync(pluginPath, 'utf8')
    const patched = content.replace('ln $verbose -s ', 'ln $verbose -sf ')
    if (patched !== content) writeFileSync(pluginPath, patched)
  } catch {
    // 打补丁失败不阻塞构建（仅影响 AppImage 打包）
  }
}

if (process.platform === 'linux') patchGtkPluginSymlinks()

const environment = {
  ...process.env,
  NERI_BUILD_EPOCH: process.env.NERI_BUILD_EPOCH || Math.floor(Date.now() / 1000).toString(),
  ...(isRelrDistro() ? { NO_STRIP: '1' } : {}),
}

const child = spawn(process.execPath, [tauriCli, ...process.argv.slice(2)], {
  env: environment,
  stdio: 'inherit',
})

child.on('error', (error) => {
  console.error(`Failed to start Tauri CLI: ${error.message}`)
  process.exitCode = 1
})

child.on('exit', (code) => {
  process.exitCode = code ?? 1
})
