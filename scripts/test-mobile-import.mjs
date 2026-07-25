import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

const root = new URL('../', import.meta.url)
const read = relative => readFile(new URL(relative, root), 'utf8')

function functionSection(source, signature) {
  const start = source.indexOf(signature)
  assert.notEqual(start, -1, `missing ${signature}`)
  const openingBrace = source.indexOf('{', start + signature.length)
  assert.notEqual(openingBrace, -1, `missing body for ${signature}`)

  let depth = 0
  for (let index = openingBrace; index < source.length; index += 1) {
    if (source[index] === '{') depth += 1
    if (source[index] === '}') depth -= 1
    if (depth === 0) return source.slice(start, index + 1)
  }
  assert.fail(`unterminated body for ${signature}`)
}

const syncCommand = await read('src-tauri/src/commands/sync_cmd.rs')
const syncStore = await read('src/stores/sync.ts')
const settingsCommand = await read('src-tauri/src/commands/settings_cmd.rs')
const imageCommand = await read('src-tauri/src/commands/image_cmd.rs')
const appState = await read('src-tauri/src/state.rs')
const authState = await read('src-tauri/src/auth/state.rs')
const serializer = await read('src-tauri/src/sync/serializer.rs')

for (const marker of [
  'struct AndroidConfigFile',
  'struct AndroidTypedPreferenceSnapshot',
  'fn android_settings_to_desktop',
  'fn android_auth_to_desktop',
]) {
  assert.ok(syncCommand.includes(marker), `missing Android config compatibility marker: ${marker}`)
}
assert.match(
  syncCommand,
  /if platform\s*==\s*"pc"/,
  'config import must recognize the desktop payload discriminator',
)
assert.match(
  syncCommand,
  /serde_json::from_value\(value\)[\s\S]*?Parse Android config failed/,
  'non-desktop NeriPlayer config files must use the Android payload decoder',
)
assert.match(
  syncCommand,
  /"platform":\s*imported_platform/,
  'config import must report which platform format was restored',
)

const androidConfigFixture = {
  kind: 'moe.ouom.neriplayer.config',
  formatVersion: 1,
  settings: {
    booleans: {},
    floats: {},
    ints: {},
    longs: {
      cloud_music_lyric_default_offset_ms: 750,
      qq_music_lyric_default_offset_ms: -250,
    },
    strings: {},
  },
}
assert.equal(androidConfigFixture.settings.ints.cloud_music_lyric_default_offset_ms, undefined)
assert.equal(androidConfigFixture.settings.longs.cloud_music_lyric_default_offset_ms, 750)

const androidSettingsSection = functionSection(syncCommand, 'fn android_settings_to_desktop(')
assert.match(
  androidSettingsSection,
  /let mut settings = current_settings/,
  'phone settings must overlay the current desktop snapshot',
)
assert.doesNotMatch(
  androidSettingsSection,
  /AppSettings::default\(\)/,
  'phone import must not reset desktop-only settings to defaults',
)
assert.match(
  androidSettingsSection,
  /"cloud_music_lyric_default_offset_ms"[\s\S]*?"qq_music_lyric_default_offset_ms"[\s\S]*?let value = snapshot\s*\.longs\s*\.get\(key\)/,
  'Android lyric offsets must be restored from the longs map used by real phone exports',
)
assert.match(
  syncCommand,
  /has_restorable_content\(has_listen_together_section\)[\s\S]*?Config backup has no restorable content/,
  'header-only Android config files must be rejected before they can reset state',
)
assert.match(
  syncCommand,
  /merge_imported_auth_platforms\(previous_auth\.clone\(\), auth, &auth_platforms\)/,
  'Android auth import must overlay only platforms with convertible phone credentials',
)

const configImportSection = functionSection(syncCommand, 'pub async fn import_config(')
const androidAuthSection = functionSection(syncCommand, 'fn android_auth_to_desktop(')
assert.match(
  androidAuthSection,
  /let imported_youtube = YouTubeAuth[\s\S]*?imported_youtube\.has_login\(\)[\s\S]*?auth\.youtube = Some\(imported_youtube\)/,
  'Android YouTube cookies must replace desktop auth only when they contain login credentials',
)
assert.match(
  androidAuthSection,
  /else\s*\{[\s\S]*?youtube_guest_cookies_ignored/,
  'guest-only Android YouTube cookies must produce a preservation warning',
)
const youtubeAuthSection = functionSection(authState, 'impl YouTubeAuth')
for (const credential of ['SAPISID', '__Secure-1PAPISID', '__Secure-3PAPISID']) {
  assert.ok(
    youtubeAuthSection.includes(`c.name == "${credential}"`),
    `desktop YouTube auth must accept Android login credential ${credential}`,
  )
}
for (const marker of [
  'save_sync_preferences_checked',
  'save_github_config_checked',
  'save_webdav_config_checked',
  'save_auth_strict',
  'rollback_config_import_persistence',
]) {
  assert.ok(configImportSection.includes(marker), `config import is missing strict transaction marker: ${marker}`)
}
assert.match(
  appState,
  /pub config_persistence_gate:\s*tokio::sync::Mutex<\(\)>[\s\S]*?config_persistence_gate:\s*tokio::sync::Mutex::new\(\(\)\)/,
  'AppState must initialize the shared configuration persistence gate',
)

const settingsSaveSection = functionSection(settingsCommand, 'pub async fn save_settings(')
assert.match(
  settingsSaveSection,
  /config_persistence_gate\.lock\(\)\.await[\s\S]*?store::save_settings/,
  'ordinary settings writes must use the shared configuration persistence gate',
)

const settingsLoadSection = functionSection(settingsCommand, 'pub async fn get_settings(')
assert.match(
  settingsLoadSection,
  /config_persistence_gate\.lock\(\)\.await[\s\S]*?store::load_settings/,
  'settings reads that may normalize and persist legacy data must use the shared gate',
)

const coverFetchSection = functionSection(imageCommand, 'pub async fn fetch_bilibili_cover(')
const coverGateIndex = coverFetchSection.indexOf('config_persistence_gate.lock().await')
const coverLoadIndex = coverFetchSection.indexOf('store::load_settings')
const coverGateDropIndex = coverFetchSection.indexOf('drop(config_guard)')
assert.ok(
  coverGateIndex !== -1 && coverLoadIndex > coverGateIndex && coverGateDropIndex > coverLoadIndex,
  'cover cache setup must hold the shared gate while settings normalization can write to disk',
)

const configGateIndex = configImportSection.indexOf('config_persistence_gate.lock().await')
assert.notEqual(configGateIndex, -1, 'config import must acquire the shared persistence gate')
for (const marker of [
  'let current_settings',
  'let current_auth',
  'let previous_preferences',
  'let persistence_result',
  'clear_and_reinject_webview_cookies',
  'rollback_config_import_persistence',
]) {
  assert.ok(
    configImportSection.indexOf(marker) > configGateIndex,
    `config import must acquire the persistence gate before ${marker}`,
  )
}
const cookieGateIndex = configImportSection.indexOf('auth_cookie_gate.lock().await')
assert.ok(
  cookieGateIndex > configGateIndex,
  'config import must acquire the configuration gate before the cookie gate',
)
for (const marker of ['let current_auth', 'hydrate_android_netease_auth', 'let previous_auth']) {
  assert.ok(
    configImportSection.indexOf(marker) > cookieGateIndex,
    `config import must acquire the cookie gate before ${marker}`,
  )
}
assert.doesNotMatch(
  configImportSection,
  /drop\(_config_guard\)/,
  'config import must retain the persistence gate through commit and rollback',
)

for (const command of [
  'update_sync_preferences',
  'validate_github_token',
  'create_github_repo',
  'use_existing_github_repo',
  'configure_github_sync',
  'sync_github',
  'disconnect_github_sync',
  'configure_webdav_sync',
  'sync_webdav',
  'update_github_sync_settings',
  'update_webdav_sync_settings',
  'disconnect_webdav_sync',
]) {
  const section = functionSection(syncCommand, `pub async fn ${command}(`)
  assert.match(
    section,
    /config_persistence_gate\.lock\(\)\.await/,
    `${command} must use the shared configuration persistence gate`,
  )
}

for (const helper of [
  'save_github_config_checked',
  'save_webdav_config_checked',
  'save_sync_preferences_checked',
]) {
  const section = functionSection(syncCommand, `fn ${helper}(`)
  assert.doesNotMatch(
    section,
    /config_persistence_gate/,
    `${helper} must stay lock-free for use inside the import transaction`,
  )
}

const playlistImportSection = functionSection(syncCommand, 'pub async fn import_playlists(')
assert.match(
  playlistImportSection,
  /\.add_filter\("NeriPlayer backup",\s*&\["json",\s*"bin"\]\)/,
  'the picker must expose both backup.json and backup.bin sync snapshots',
)
assert.match(
  playlistImportSection,
  /eq_ignore_ascii_case\("bin"\)[\s\S]*?if is_binary[\s\S]*?serializer::deserialize\(&data, true\)/,
  'backup.bin must be identified by its extension and decoded as the compressed sync format',
)
assert.match(
  playlistImportSection,
  /let is_binary[\s\S]*?std::fs::metadata\(file_path\)[\s\S]*?let max_input_bytes = if is_binary\s*\{\s*24 \* 1024 \* 1024\s*\}\s*else\s*\{\s*12 \* 1024 \* 1024\s*\}[\s\S]*?metadata\.len\(\) > max_input_bytes/,
  'backup.bin must allow Base64 input up to 24 MiB while JSON remains capped at 12 MiB',
)
assert.match(
  playlistImportSection,
  /contains_key\("favoritePlaylists"\)/,
  'backup.json must use the favoritePlaylists field as the online-favorites marker',
)
assert.match(
  playlistImportSection,
  /"onlineFavoritesAvailable": online_favorites_available/,
  'playlist import must report whether the selected snapshot can carry online favorites',
)

const standardAndroidBackup = {
  version: '2.2',
  timestamp: 1,
  playlists: [],
  recentPlays: [],
}
const fullSyncBackupJson = {
  version: '2.0',
  deviceId: 'phone',
  playlists: [],
  favoritePlaylists: [],
}
const carriesOnlineFavorites = value => Object.hasOwn(value, 'favoritePlaylists')
assert.equal(
  carriesOnlineFavorites(standardAndroidBackup),
  false,
  'ordinary Android playlist backups do not contain online favorites',
)
assert.equal(
  carriesOnlineFavorites(fullSyncBackupJson),
  true,
  'backup.json remains distinguishable even when its online-favorites array is empty',
)
assert.equal(/\.bin$/i.test('backup.bin'), true, 'backup.bin must carry the binary-format marker')
assert.equal(/\.bin$/i.test('backup.json'), false, 'backup.json must stay on the JSON decoder path')

assert.match(
  syncStore,
  /result\.onlineFavoritesAvailable === false[\s\S]*?settings\.import_online_favorites_unavailable/,
  'ordinary Android backups must show the missing-online-favorites warning',
)

assert.match(
  serializer,
  /MAX_DECOMPRESSED_BYTES:\s*usize\s*=\s*16\s*\*\s*1024\s*\*\s*1024/,
  'backup.bin decompression must use the Android 16 MiB output limit',
)
assert.match(
  serializer,
  /read > MAX_DECOMPRESSED_BYTES - proto_bytes\.len\(\)[\s\S]*?too large/,
  'backup.bin decoding must reject decompressed output before unbounded allocation',
)

const warningKeys = [
  'import_online_favorites_unavailable',
  'import_config_warning_listen_together_url',
  'import_config_warning_youtube_authorization',
  'import_config_warning_youtube_guest_cookies',
  'import_config_warning_youtube_account',
  'import_config_warning_netease_auth',
  'import_config_warning_unknown',
]
for (const locale of ['zh-CN', 'zh-TW', 'en', 'ja']) {
  const messages = JSON.parse(await read(`src/i18n/${locale}.json`))
  for (const key of warningKeys) {
    assert.equal(
      typeof messages.settings?.[key],
      'string',
      `${locale} is missing settings.${key}`,
    )
    assert.notEqual(messages.settings[key].trim(), '', `${locale} has an empty settings.${key}`)
  }
}

console.log('mobile config import tests passed')
