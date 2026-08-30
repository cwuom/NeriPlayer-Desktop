import { spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import { readFileSync } from 'node:fs'

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
