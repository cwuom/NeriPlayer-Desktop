import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const root = new URL('../', import.meta.url)
const read = relative => readFile(new URL(relative, root), 'utf8')

const settingsView = await read('src/views/SettingsView.vue')
const capabilities = JSON.parse(await read('src-tauri/capabilities/default.json'))
const settingsCommand = await read('src-tauri/src/commands/settings_cmd.rs')
const tauriMain = await read('src-tauri/src/main.rs')

assert.match(
  settingsView,
  /async function openLogDir\(\)[\s\S]*?await invoke\(['"]open_log_dir['"]\)/,
  'the settings action must call the restricted backend log-directory command',
)
assert.match(
  settingsView,
  /<button type="button" class="setting-card setting-card-action" @click="openLogDir">/,
  'the log-directory action must expose native keyboard button semantics',
)
assert.match(
  settingsCommand,
  /pub async fn open_log_dir\(app: AppHandle\)[\s\S]*?create_dir_all\(&dir\)[\s\S]*?app\.opener\(\)[\s\S]*?\.open_path\(/,
  'the backend must create and open the exact application log directory',
)
assert.match(
  tauriMain,
  /settings_cmd::open_log_dir/,
  'the restricted log-directory command must be registered',
)
assert.equal(
  capabilities.permissions.includes('opener:allow-open-path'),
  false,
  'the frontend must not receive broad filesystem opener access',
)

console.log('log opening tests passed')
