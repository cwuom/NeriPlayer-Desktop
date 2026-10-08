import { spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import net from 'node:net'

const require = createRequire(import.meta.url)
const tauriCli = require.resolve('@tauri-apps/cli/tauri.js')
const DEFAULT_DEV_PORT = 1420
const args = process.argv.slice(2)

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

/** 能否在该地址上监听；地址族不可用（没有 IPv6）时返回 null */
function canListen(port, host) {
  return new Promise((resolve) => {
    const server = net.createServer()
    server.once('error', (error) => {
      resolve(error.code === 'EADDRNOTAVAIL' || error.code === 'EAFNOSUPPORT' ? null : false)
    })
    server.listen({ port, host }, () => server.close(() => resolve(true)))
  })
}

/** Vite 按 localhost 监听，可能落在 IPv4 或 IPv6 上，两边都空闲才算可用 */
async function isPortFree(port) {
  for (const host of ['127.0.0.1', '::1']) {
    if ((await canListen(port, host)) === false) return false
  }
  return true
}

async function findFreePort(start) {
  for (let port = start; port < start + 50; port++) {
    if (await isPortFree(port)) return port
  }
  throw new Error(`No free dev server port between ${start} and ${start + 49}`)
}

// 其它 Tauri 项目的开发服务器默认也占 1420：被占用时顺延到空闲端口，
// 端口经环境变量交给 Vite（beforeDevCommand 继承环境），devUrl 用 --config 覆盖成同一地址
if (args[0] === 'dev') {
  const requested = Number(process.env.NERI_DEV_PORT)
  const port = Number.isInteger(requested) && requested > 0 ? requested : await findFreePort(DEFAULT_DEV_PORT)
  if (port !== DEFAULT_DEV_PORT) {
    console.warn(`[run-tauri] Port ${DEFAULT_DEV_PORT} is in use by another program, using ${port} for the dev server.`)
  }
  environment.NERI_DEV_PORT = String(port)
  args.splice(1, 0, '--config', JSON.stringify({ build: { devUrl: `http://localhost:${port}` } }))
}

const child = spawn(process.execPath, [tauriCli, ...args], {
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
