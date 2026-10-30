<script setup lang="ts">
import { computed, reactive, ref, onMounted, onUnmounted } from 'vue'
import DebugLogViewer from '@/components/debug/DebugLogViewer.vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { invoke } from '@tauri-apps/api/core'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { usePlayerStore } from '@/stores/player'
import { useSyncStore } from '@/stores/sync'
import { useSettingsStore } from '@/stores/settings'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { useListenTogetherStore } from '@/stores/listenTogether'
import { createLogger } from '@/utils/logger'
import { formatTimeMs } from '@/utils/timeFormat'

const log = createLogger('debug-view')

const router = useRouter()
const { t, locale } = useI18n()
const player = usePlayerStore()
const syncStore = useSyncStore()
const settingsStore = useSettingsStore()
const authStore = useAuthStore()
const toast = useToastStore()

const buildInfo = ref<{
  app_version?: string
  version?: string
  uuid: string
  timestamp: string
} | null>(null)
const appDataDir = ref('')
const debugCookieStorage = ref<{ available: boolean; stored: boolean } | null>(null)
const clearingDebugCookies = ref(false)
const showClearDebugCookiesConfirm = ref(false)

const probes = ref<Record<string, 'idle' | 'testing' | 'success' | 'failed'>>({
  netease: 'idle',
  bilibili: 'idle',
  youtube: 'idle',
})

const platformName = (() => {
  const ua = navigator.userAgent
  if (/Mac/.test(ua)) return 'macOS'
  if (/Win/.test(ua)) return 'Windows'
  if (/Linux/.test(ua)) return 'Linux'
  return navigator.platform || 'Unknown'
})()

const webviewVersion = (() => {
  const match = navigator.userAgent.match(/Chrome\/([\d.]+)/)
  return match ? `Chromium ${match[1]}` : navigator.userAgent.slice(0, 60)
})()

async function testNetease() {
  probes.value.netease = 'testing'
  probes.value.netease = await probePlatform('netease')
}

async function testBilibili() {
  probes.value.bilibili = 'testing'
  probes.value.bilibili = await probePlatform('bilibili')
}

async function testYouTube() {
  probes.value.youtube = 'testing'
  probes.value.youtube = await probePlatform('youtube')
}

// 探针语义：收到任何 HTTP 响应都算连通，只有传输层失败才算不通。
// 之前调「取具体视频播放地址」，视频下架/缺参这类业务失败被误报成不通
async function probePlatform(platform: string): Promise<'success' | 'failed'> {
  try {
    const reachable = await invoke<boolean>('probe_platform_connectivity', { platform })
    return reachable ? 'success' : 'failed'
  } catch {
    return 'failed'
  }
}

function testAll() {
  testNetease()
  testBilibili()
  testYouTube()
}

function probeStatusText(status: string): string {
  switch (status) {
    case 'testing': return t('settings.probe_testing')
    case 'success': return t('settings.probe_success')
    case 'failed': return t('settings.probe_failed')
    default: return '—'
  }
}

function probeStatusColor(status: string): string {
  switch (status) {
    case 'success': return 'var(--md-primary)'
    case 'failed': return 'var(--md-error)'
    default: return 'var(--md-on-surface-variant)'
  }
}

// 未配置或从未同步时显示占位符，避免「未配置 + 历史时间」的矛盾展示
function formatSyncTime(ts: number, configured: boolean): string {
  if (!configured || !ts) return '—'
  return new Date(ts).toLocaleString(locale.value)
}

function goBack() {
  router.push('/')
}

function hideDebugMode() {
  settingsStore.devModeEnabled = false
  toast.success(t('settings.debug_hidden_toast'))
  router.push('/settings')
}

async function loadDebugCookieStorageStatus() {
  try {
    debugCookieStorage.value = await invoke('get_debug_cookie_storage_status')
  } catch {
    debugCookieStorage.value = null
  }
}

async function clearDebugCookies() {
  showClearDebugCookiesConfirm.value = false
  clearingDebugCookies.value = true
  try {
    await invoke('clear_debug_cookie_storage')
    await authStore.checkStatus({ invalidatePending: ['netease', 'bilibili', 'youtube'] })
    await loadDebugCookieStorageStatus()
    toast.success(t('settings.debug_cookie_clear_success'))
  } catch (error) {
    log.error('Failed to clear debug cookies:', error)
    toast.error(t('settings.debug_cookie_clear_failed'))
  } finally {
    clearingDebugCookies.value = false
  }
}

async function copyBuildValue(value: string | undefined | null) {
  const text = value?.trim()
  if (!text) return
  try {
    await writeText(text)
    toast.success(t('settings.build_info_copied'))
  } catch (error) {
    log.error('Failed to copy build info:', error)
    toast.error(t('settings.build_info_copy_failed'))
  }
}

onMounted(async () => {
  try {
    const info = await invoke<{
      app_version?: string
      build_uuid: string
      build_timestamp: string
      version: string
    }>('get_build_info')
    buildInfo.value = {
      app_version: info.app_version,
      version: info.version,
      uuid: info.build_uuid,
      timestamp: info.build_timestamp,
    }
  } catch { /* 命令不存在时忽略 */ }
  try {
    appDataDir.value = await invoke('get_app_data_dir')
  } catch { /* 忽略 */ }
  await loadDebugCookieStorageStatus()
  await refreshCrashes()
  startRuntimeSampling()
})

onUnmounted(() => {
  if (crashRefreshTimer) window.clearTimeout(crashRefreshTimer)
  stopRuntimeSampling()
})

// ===== 标签页 =====
type DebugTab = 'overview' | 'network' | 'logs' | 'crashes' | 'lt'
const DEBUG_TAB_KEY = 'neri:debug-tab'
const storedTab = sessionStorage.getItem(DEBUG_TAB_KEY) as DebugTab | null
const activeTab = ref<DebugTab>(storedTab && ['overview', 'network', 'logs', 'crashes', 'lt'].includes(storedTab) ? storedTab : 'overview')
/** 日志每 1.5s 轮询一次，第一次切到日志页才挂载 */
const logsMounted = ref(activeTab.value === 'logs')

const debugTabs = computed(() => [
  { id: 'overview' as const, icon: 'dashboard', label: t('settings.debug_tab_overview'), badge: '' },
  { id: 'network' as const, icon: 'sensors', label: t('settings.debug_tab_network'), badge: '' },
  { id: 'logs' as const, icon: 'terminal', label: t('settings.debug_logs'), badge: '' },
  { id: 'crashes' as const, icon: 'report', label: t('settings.debug_tab_crashes'), badge: crashes.value.length ? String(crashes.value.length) : '' },
  { id: 'lt' as const, icon: 'group', label: t('settings.debug_lt_panel'), badge: '' },
])

function selectTab(tab: DebugTab) {
  activeTab.value = tab
  if (tab === 'logs') logsMounted.value = true
  sessionStorage.setItem(DEBUG_TAB_KEY, tab)
}

// ===== 运行时诊断 =====
const runtime = reactive({ fps: 0, heap: '—', uptime: '—', viewport: '', locale: '', online: navigator.onLine })
let fpsFrame = 0
let fpsFrames = 0
let fpsWindowStart = 0
let runtimeTimer: number | null = null

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '—'
  const units = ['B', 'KB', 'MB', 'GB']
  const exponent = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)))
  return `${(bytes / 1024 ** exponent).toFixed(exponent ? 1 : 0)} ${units[exponent]}`
}

function formatUptime(ms: number): string {
  const seconds = Math.floor(ms / 1000)
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  const s = seconds % 60
  return h ? `${h}h ${m}m ${s}s` : m ? `${m}m ${s}s` : `${s}s`
}

function sampleRuntime() {
  const memory = (performance as Performance & { memory?: { usedJSHeapSize: number; jsHeapSizeLimit: number } }).memory
  runtime.heap = memory ? `${formatBytes(memory.usedJSHeapSize)} / ${formatBytes(memory.jsHeapSizeLimit)}` : '—'
  runtime.uptime = formatUptime(performance.now())
  runtime.viewport = `${window.innerWidth}×${window.innerHeight} @${window.devicePixelRatio}x · ${screen.width}×${screen.height}`
  runtime.locale = `${locale.value} · ${Intl.DateTimeFormat().resolvedOptions().timeZone}`
  runtime.online = navigator.onLine
}

function countFrame(now: number) {
  if (!fpsWindowStart) fpsWindowStart = now
  fpsFrames++
  if (now - fpsWindowStart >= 1000) {
    runtime.fps = Math.round((fpsFrames * 1000) / (now - fpsWindowStart))
    fpsFrames = 0
    fpsWindowStart = now
  }
  fpsFrame = requestAnimationFrame(countFrame)
}

function startRuntimeSampling() {
  sampleRuntime()
  fpsFrame = requestAnimationFrame(countFrame)
  runtimeTimer = window.setInterval(sampleRuntime, 1000)
}

function stopRuntimeSampling() {
  cancelAnimationFrame(fpsFrame)
  if (runtimeTimer) window.clearInterval(runtimeTimer)
}

const decoderText = computed(() => {
  const capabilities = player.decoderCapabilities
  if (!capabilities) return '—'
  if (capabilities.ffmpeg) return `FFmpeg avcodec ${capabilities.ffmpeg.avcodec}`
  return capabilities.ffmpegError ? `FFmpeg ✕ ${capabilities.ffmpegError}` : 'FFmpeg ✕'
})

const audioText = computed(() => [
  settingsStore.audioOutputDevice || t('settings.debug_runtime_default_device'),
  `vol ${Math.round(player.volume * 100)}%`,
  `×${settingsStore.playbackSpeed}`,
  settingsStore.normalizeVolume ? 'loudness' : '',
  settingsStore.equalizerEnabled ? `EQ ${settingsStore.equalizerPresetId}` : '',
].filter(Boolean).join(' · '))

// ===== API 探针详情（对齐 Android 各平台探针页：调真实接口、看/复制返回）=====
interface ProbeAction { id: string; label: string; run: () => Promise<unknown> }
interface ProbeResult { status: 'idle' | 'running' | 'success' | 'failed'; elapsedMs: number; body: string }
const probeResults = ref<Record<string, ProbeResult>>({})
const expandedProbePlatform = ref('')

// 采样参数与 Android 探针一致：网易 33894312（歌词样例曲）、B站 BV1GJ411x7h7
const probeGroups: Array<{ platform: string; label: string; actions: ProbeAction[] }> = [
  {
    platform: 'netease', label: '网易云',
    actions: [
      { id: 'account', label: '账号信息', run: () => invoke('get_user_account', { platform: 'netease' }) },
      { id: 'playlists', label: '用户歌单', run: () => invoke('get_user_playlists', { platform: 'netease' }) },
      { id: 'detail', label: '歌曲详情 33894312', run: () => invoke('get_netease_song_detail', { songId: 33894312 }) },
      { id: 'url', label: '播放地址 33894312', run: () => invoke('get_netease_song_url', { songId: 33894312, quality: 'exhigh' }) },
      { id: 'search', label: '搜索「鹿乃」', run: () => invoke('search', { query: '鹿乃', platform: 'netease' }) },
    ],
  },
  {
    platform: 'bilibili', label: '哔哩哔哩',
    actions: [
      { id: 'account', label: '账号信息', run: () => invoke('get_user_account', { platform: 'bilibili' }) },
      { id: 'favs', label: '收藏夹列表', run: () => invoke('get_user_playlists', { platform: 'bilibili' }) },
      { id: 'audio', label: '音频流 BV1GJ411x7h7', run: () => invoke('get_bili_audio_url', { bvid: 'BV1GJ411x7h7', avid: null, cid: null }) },
      { id: 'search', label: '搜索「鹿乃」', run: () => invoke('search', { query: '鹿乃', platform: 'bilibili' }) },
    ],
  },
  {
    platform: 'youtube', label: 'YouTube',
    actions: [
      { id: 'account', label: '账号信息', run: () => invoke('refresh_youtube_profile') },
      { id: 'library', label: '云端歌单', run: () => invoke('get_user_playlists', { platform: 'youtube' }) },
      { id: 'player', label: '播放解析 dQw4w9WgXcQ', run: () => invoke('get_youtube_audio_url', { videoId: 'dQw4w9WgXcQ' }) },
      { id: 'search', label: '搜索「kano」', run: () => invoke('search', { query: 'kano', platform: 'youtube' }) },
    ],
  },
]

function probeKey(platform: string, id: string): string { return `${platform}:${id}` }
function probeState(platform: string, id: string): ProbeResult {
  return probeResults.value[probeKey(platform, id)] ?? { status: 'idle', elapsedMs: 0, body: '' }
}

async function runProbe(platform: string, action: ProbeAction) {
  const key = probeKey(platform, action.id)
  probeResults.value = { ...probeResults.value, [key]: { status: 'running', elapsedMs: 0, body: '' } }
  const started = performance.now()
  try {
    const result = await action.run()
    probeResults.value = {
      ...probeResults.value,
      [key]: {
        status: 'success',
        elapsedMs: Math.round(performance.now() - started),
        body: JSON.stringify(result, null, 2) ?? 'null',
      },
    }
  } catch (e) {
    probeResults.value = {
      ...probeResults.value,
      [key]: { status: 'failed', elapsedMs: Math.round(performance.now() - started), body: String(e) },
    }
  }
}

async function copyProbeResult(platform: string, id: string) {
  const body = probeState(platform, id).body
  if (!body) return
  try {
    const { writeText } = await import('@tauri-apps/plugin-clipboard-manager')
    await writeText(body)
    toast.success(t('settings.debug_logs_copied'))
  } catch (e) {
    log.error('copy probe result failed:', e)
  }
}

// ===== 测试异常（对齐 Android「测试异常」）=====
let crashRefreshTimer: number | null = null

async function triggerCrash(kind: string) {
  if (kind === 'frontend') {
    // 出了当前调用栈再抛，确保走 window.onerror 而不是被 Vue 捕获
    setTimeout(() => {
      throw new Error('debug test: frontend uncaught exception')
    }, 0)
    return
  }
  try {
    await invoke('debug_trigger_crash', { kind })
  } catch (e) {
    // panic_command 预期走到这里：IPC 报错但应用存活
    log.warn('crash trigger returned error (expected for panic_command):', e)
  }
  // 给 panic 钩子落盘留点时间再刷新列表；句柄纳入卸载清理
  if (crashRefreshTimer) window.clearTimeout(crashRefreshTimer)
  crashRefreshTimer = window.setTimeout(() => {
    crashRefreshTimer = null
    void refreshCrashes()
  }, 600)
}

// ===== 一起听调试 =====
const ltStore = useListenTogetherStore()

// ===== 崩溃报告 =====
interface CrashReportInfo { file_name: string; size_bytes: number; modified_ms: number }
const crashes = ref<CrashReportInfo[]>([])
const expandedCrash = ref('')
const crashContent = ref('')

async function refreshCrashes() {
  try {
    crashes.value = await invoke<CrashReportInfo[]>('list_crash_reports')
  } catch { /* 静默 */ }
}

async function toggleCrash(name: string) {
  if (expandedCrash.value === name) {
    expandedCrash.value = ''
    return
  }
  try {
    crashContent.value = await invoke<string>('read_crash_report', { fileName: name })
    expandedCrash.value = name
  } catch (e) {
    log.error('read crash failed:', e)
  }
}

async function clearCrashes() {
  try {
    await invoke('clear_crash_reports')
    expandedCrash.value = ''
    await refreshCrashes()
    toast.success(t('settings.debug_crashes_cleared'))
  } catch (e) {
    log.error('clear crashes failed:', e)
  }
}
</script>

<template>
  <div class="debug-view">
    <header class="debug-header">
      <button class="debug-back-btn" @click="goBack">
        <span class="material-symbols-rounded">arrow_back</span>
      </button>
      <h1 class="page-title">{{ t('settings.debug_title') }}</h1>
    </header>

    <nav class="debug-tabs" role="tablist" :aria-label="t('settings.debug_title')">
      <button
        v-for="tab in debugTabs"
        :key="tab.id"
        type="button"
        role="tab"
        class="debug-tab"
        :class="{ active: activeTab === tab.id }"
        :aria-selected="activeTab === tab.id"
        @click="selectTab(tab.id)"
      >
        <span class="material-symbols-rounded">{{ tab.icon }}</span>
        {{ tab.label }}
        <span v-if="tab.badge" class="debug-tab-badge">{{ tab.badge }}</span>
      </button>
    </nav>

    <section v-show="activeTab === 'overview'" class="debug-panel debug-columns">
      <div class="debug-column">
    <!-- 构建信息 -->
    <div v-if="buildInfo" class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">info</span>
      <span>{{ t('settings.build_uuid').split(' ')[0] }}</span>
    </div>
    <div v-if="buildInfo" class="setting-card">
      <div class="setting-info">
        <div class="state-grid">
          <div class="state-item copyable-state-item" @click="copyBuildValue(buildInfo.version)">
            <span class="state-label">{{ t('settings.build_version_label') }}</span>
            <span class="state-value mono">{{ buildInfo.version || '—' }}</span>
          </div>
          <div class="state-item copyable-state-item" @click="copyBuildValue(buildInfo.uuid)">
            <span class="state-label">{{ t('settings.build_uuid') }}</span>
            <span class="state-value mono">{{ buildInfo.uuid }}</span>
          </div>
          <div class="state-item copyable-state-item" @click="copyBuildValue(buildInfo.timestamp)">
            <span class="state-label">{{ t('settings.build_time') }}</span>
            <span class="state-value">{{ buildInfo.timestamp }}</span>
          </div>
        </div>
      </div>
    </div>

    <!-- 系统信息 -->
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">computer</span>
      <span>{{ t('settings.debug_system_info') }}</span>
    </div>
    <div class="setting-card">
      <div class="setting-info">
        <div class="state-grid">
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_platform') }}</span>
            <span class="state-value">{{ platformName }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_webview') }}</span>
            <span class="state-value mono">{{ webviewVersion }}</span>
          </div>
          <div v-if="appDataDir" class="state-item full-width">
            <span class="state-label">{{ t('settings.debug_data_dir') }}</span>
            <span class="state-value mono">{{ appDataDir }}</span>
          </div>
        </div>
      </div>
    </div>

    <!-- 运行时诊断 -->
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">monitor_heart</span>
      <span>{{ t('settings.debug_runtime') }}</span>
    </div>
    <div class="setting-card">
      <div class="setting-info">
        <div class="state-grid">
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_runtime_fps') }}</span>
            <span class="state-value mono" :class="{ warn: runtime.fps > 0 && runtime.fps < 45 }">{{ runtime.fps || '—' }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_runtime_heap') }}</span>
            <span class="state-value mono">{{ runtime.heap }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_runtime_uptime') }}</span>
            <span class="state-value mono">{{ runtime.uptime }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_runtime_window') }}</span>
            <span class="state-value mono">{{ runtime.viewport }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_runtime_locale') }}</span>
            <span class="state-value mono">{{ runtime.locale }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_runtime_network') }}</span>
            <span class="state-value" :style="{ color: runtime.online ? 'var(--md-primary)' : 'var(--md-error)' }">{{ runtime.online ? t('settings.debug_runtime_online') : t('settings.debug_runtime_offline') }}</span>
          </div>
          <div class="state-item full-width">
            <span class="state-label">{{ t('settings.debug_runtime_decoder') }}</span>
            <span class="state-value mono">{{ decoderText }}</span>
          </div>
          <div class="state-item full-width">
            <span class="state-label">{{ t('settings.debug_runtime_audio') }}</span>
            <span class="state-value mono">{{ audioText }}</span>
          </div>
        </div>
      </div>
    </div>

      </div>
      <div class="debug-column">
    <!-- 播放器状态 -->
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">queue_music</span>
      <span>{{ t('settings.player_state') }}</span>
    </div>
    <div class="setting-card">
      <div class="setting-info">
        <div class="state-grid">
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_player_track') }}</span>
            <span class="state-value">{{ player.currentTrack?.title || '—' }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_player_artist') }}</span>
            <span class="state-value">{{ player.currentTrack?.artist || '—' }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_player_source') }}</span>
            <span class="state-value mono">{{ player.currentTrack?.id?.split(':')[0] || '—' }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_player_playing') }}</span>
            <span class="state-value">{{ player.isPlaying ? t('common.yes') : t('common.no') }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_player_position') }}</span>
            <span class="state-value mono">{{ formatTimeMs(player.positionMs) }} / {{ formatTimeMs(player.durationMs) }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_player_queue') }}</span>
            <span class="state-value">{{ player.queue?.length || 0 }}</span>
          </div>
        </div>
      </div>
    </div>

    <!-- 同步状态 -->
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">sync</span>
      <span>{{ t('settings.debug_sync_status') }}</span>
    </div>
    <div class="setting-card">
      <div class="setting-info">
        <div class="state-grid">
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_github_sync') }}</span>
            <span class="state-value" :style="{ color: syncStore.github.configured ? 'var(--md-primary)' : 'var(--md-on-surface-variant)' }">
              {{ syncStore.github.configured
                ? (syncStore.github.autoSync ? t('settings.debug_auto_sync_on') : t('settings.debug_auto_sync_off'))
                : t('settings.debug_not_configured') }}
            </span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_last_sync') }} (GitHub)</span>
            <span class="state-value">{{ formatSyncTime(syncStore.github.lastSyncTime, syncStore.github.configured) }}</span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_webdav_sync') }}</span>
            <span class="state-value" :style="{ color: syncStore.webdav.configured ? 'var(--md-primary)' : 'var(--md-on-surface-variant)' }">
              {{ syncStore.webdav.configured
                ? (syncStore.webdav.autoSync ? t('settings.debug_auto_sync_on') : t('settings.debug_auto_sync_off'))
                : t('settings.debug_not_configured') }}
            </span>
          </div>
          <div class="state-item">
            <span class="state-label">{{ t('settings.debug_last_sync') }} (WebDAV)</span>
            <span class="state-value">{{ formatSyncTime(syncStore.webdav.lastSyncTime, syncStore.webdav.configured) }}</span>
          </div>
        </div>
      </div>
    </div>

      </div>
    </section>

    <section v-show="activeTab === 'network'" class="debug-panel">
    <!-- Debug Cookie 存储 -->
    <template v-if="debugCookieStorage?.available">
      <div class="section-label">
        <span class="material-symbols-rounded" style="font-size: 18px">cookie</span>
        <span>{{ t('settings.debug_cookie_storage') }}</span>
      </div>
      <div class="setting-card">
        <div class="setting-icon-wrap">
          <span class="material-symbols-rounded">delete_sweep</span>
        </div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.debug_cookie_storage') }}</div>
          <div class="setting-desc">
            {{ debugCookieStorage.stored
              ? t('settings.debug_cookie_storage_present')
              : t('settings.debug_cookie_storage_empty') }}
          </div>
        </div>
        <button
          class="debug-cookie-clear-btn"
          :disabled="clearingDebugCookies"
          @click="showClearDebugCookiesConfirm = true"
        >
          <span v-if="clearingDebugCookies" class="material-symbols-rounded spinning">progress_activity</span>
          <span v-else class="material-symbols-rounded">delete</span>
          {{ t('settings.debug_cookie_clear') }}
        </button>
      </div>
    </template>

    <!-- API 探针 -->
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">sensors</span>
      <span>{{ t('settings.api_probe') }}</span>
    </div>

    <div class="setting-card" style="cursor: pointer" @click="testAll">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">play_arrow</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.api_probe') }}</div>
        <div class="setting-desc">{{ t('settings.api_probe_desc') }}</div>
      </div>
      <span class="material-symbols-rounded" style="font-size: 20px; opacity: 0.5">chevron_right</span>
    </div>

    <div class="setting-card sub-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">cloud</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.probe_netease') }}</div>
        <div class="setting-desc" :style="{ color: probeStatusColor(probes.netease) }">
          <span v-if="probes.netease === 'testing'" class="material-symbols-rounded spinning" style="font-size: 14px; vertical-align: middle">progress_activity</span>
          {{ probeStatusText(probes.netease) }}
        </div>
      </div>
    </div>

    <div class="setting-card sub-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">smart_display</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.probe_bilibili') }}</div>
        <div class="setting-desc" :style="{ color: probeStatusColor(probes.bilibili) }">
          <span v-if="probes.bilibili === 'testing'" class="material-symbols-rounded spinning" style="font-size: 14px; vertical-align: middle">progress_activity</span>
          {{ probeStatusText(probes.bilibili) }}
        </div>
      </div>
    </div>

    <div class="setting-card sub-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">play_circle</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.probe_youtube') }}</div>
        <div class="setting-desc" :style="{ color: probeStatusColor(probes.youtube) }">
          <span v-if="probes.youtube === 'testing'" class="material-symbols-rounded spinning" style="font-size: 14px; vertical-align: middle">progress_activity</span>
          {{ probeStatusText(probes.youtube) }}
        </div>
      </div>
    </div>

    <!-- API 探针详情 -->
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">api</span>
      <span>{{ t('settings.debug_probe_detail') }}</span>
    </div>
    <div class="setting-card stacked">
      <div class="setting-desc" style="margin-bottom: 8px">{{ t('settings.debug_probe_detail_desc') }}</div>
      <div v-for="group in probeGroups" :key="group.platform" class="probe-group">
        <button
          class="probe-group-header"
          @click="expandedProbePlatform = expandedProbePlatform === group.platform ? '' : group.platform"
        >
          <span class="material-symbols-rounded" style="font-size: 18px">
            {{ expandedProbePlatform === group.platform ? 'expand_less' : 'expand_more' }}
          </span>
          <span>{{ group.label }}</span>
        </button>
        <Transition name="probe-expand">
          <div v-if="expandedProbePlatform === group.platform" class="probe-actions">
            <div v-for="action in group.actions" :key="action.id" class="probe-action">
              <div class="probe-action-row">
                <span class="probe-action-label">{{ action.label }}</span>
                <span
                  v-if="probeState(group.platform, action.id).status !== 'idle'"
                  class="probe-action-status"
                  :class="probeState(group.platform, action.id).status"
                >
                  {{ probeState(group.platform, action.id).status === 'running'
                    ? t('settings.debug_probe_running')
                    : `${probeState(group.platform, action.id).elapsedMs}ms` }}
                </span>
                <button
                  class="debug-log-btn"
                  :disabled="probeState(group.platform, action.id).status === 'running'"
                  @click="runProbe(group.platform, action)"
                >
                  <span class="material-symbols-rounded">play_arrow</span>
                  {{ t('settings.debug_probe_run') }}
                </button>
                <button
                  v-if="probeState(group.platform, action.id).body"
                  class="debug-log-btn"
                  @click="copyProbeResult(group.platform, action.id)"
                >
                  <span class="material-symbols-rounded">content_copy</span>
                  {{ t('settings.debug_probe_copy_result') }}
                </button>
              </div>
              <pre
                v-if="probeState(group.platform, action.id).body"
                class="probe-result"
                :class="{ failed: probeState(group.platform, action.id).status === 'failed' }"
              >{{ probeState(group.platform, action.id).body.slice(0, 4000) }}</pre>
            </div>
          </div>
        </Transition>
      </div>
    </div>

    </section>

    <section v-show="activeTab === 'logs'" class="debug-panel">
    <!-- 始终记录日志（对齐 Android settings_always_record_logs）-->
    <div class="setting-card">
      <div class="setting-info" style="display: flex; align-items: center; gap: 12px">
        <div style="flex: 1; min-width: 0">
          <div class="setting-title">{{ t('settings.debug_always_log') }}</div>
          <div class="setting-desc">{{ t('settings.debug_always_log_desc') }}</div>
        </div>
        <label class="debug-switch">
          <input v-model="settingsStore.logToFile" type="checkbox" />
          <span class="debug-switch-track"><span class="debug-switch-thumb" /></span>
        </label>
      </div>
    </div>

    <!-- 运行日志 -->
    <DebugLogViewer v-if="logsMounted" />

    </section>

    <section v-show="activeTab === 'crashes'" class="debug-panel">
    <!-- 测试异常 -->
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">warning</span>
      <span>{{ t('settings.debug_test_exception') }}</span>
    </div>
    <div class="setting-card stacked">
      <div class="setting-desc" style="margin-bottom: 10px">{{ t('settings.debug_test_exception_desc') }}</div>
      <div class="crash-test-grid">
        <button class="crash-test-btn" @click="triggerCrash('handled')">
          <span class="crash-test-title">{{ t('settings.debug_test_handled') }}</span>
          <span class="crash-test-desc">{{ t('settings.debug_test_handled_desc') }}</span>
        </button>
        <button class="crash-test-btn" @click="triggerCrash('frontend')">
          <span class="crash-test-title">{{ t('settings.debug_test_frontend') }}</span>
          <span class="crash-test-desc">{{ t('settings.debug_test_frontend_desc') }}</span>
        </button>
        <button class="crash-test-btn danger" @click="triggerCrash('panic_command')">
          <span class="crash-test-title">{{ t('settings.debug_test_panic_command') }}</span>
          <span class="crash-test-desc">{{ t('settings.debug_test_panic_command_desc') }}</span>
        </button>
        <button class="crash-test-btn danger" @click="triggerCrash('panic_thread')">
          <span class="crash-test-title">{{ t('settings.debug_test_panic_thread') }}</span>
          <span class="crash-test-desc">{{ t('settings.debug_test_panic_thread_desc') }}</span>
        </button>
      </div>
    </div>

    <!-- 崩溃报告 -->
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">report</span>
      <span>{{ t('settings.debug_crashes') }}</span>
    </div>
    <div class="setting-card stacked">
      <div class="setting-info" style="display: flex; align-items: center; gap: 12px">
        <div style="flex: 1; min-width: 0">
          <div class="setting-title">{{ t('settings.debug_crashes') }}</div>
          <div class="setting-desc">{{ t('settings.debug_crashes_desc') }}</div>
        </div>
        <button v-if="crashes.length > 0" class="debug-log-btn debug-crash-clear" @click="clearCrashes">
          <span class="material-symbols-rounded">delete</span>
          {{ t('settings.debug_crashes_clear') }}
        </button>
      </div>
      <div v-if="crashes.length === 0" class="debug-logs-empty">{{ t('settings.debug_crashes_empty') }}</div>
      <div v-else class="debug-crash-list">
        <div v-for="report in crashes" :key="report.file_name" class="debug-crash-item">
          <button class="debug-crash-row" @click="toggleCrash(report.file_name)">
            <span class="material-symbols-rounded" style="font-size: 18px">
              {{ expandedCrash === report.file_name ? 'expand_less' : 'expand_more' }}
            </span>
            <span class="debug-crash-name">{{ report.file_name }}</span>
            <span class="debug-crash-size">{{ (report.size_bytes / 1024).toFixed(1) }} KB</span>
          </button>
          <pre v-if="expandedCrash === report.file_name" class="debug-crash-body">{{ crashContent }}</pre>
        </div>
      </div>
    </div>

    </section>

    <section v-show="activeTab === 'lt'" class="debug-panel">
    <!-- 一起听调试 -->
    <div class="section-label">
      <span class="material-symbols-rounded" style="font-size: 18px">group</span>
      <span>{{ t('settings.debug_lt_panel') }}</span>
    </div>
    <div class="setting-card">
      <div class="state-grid">
        <div class="state-item">
          <span class="state-label">{{ t('settings.debug_lt_connection') }}</span>
          <span class="state-value" :style="{ color: ltStore.isConnected ? 'var(--md-primary)' : 'var(--md-on-surface-variant)' }">
            {{ ltStore.connectionState }}
          </span>
        </div>
        <div class="state-item">
          <span class="state-label">{{ t('settings.debug_lt_room') }}</span>
          <span class="state-value mono">{{ ltStore.roomId || '—' }}</span>
        </div>
        <div class="state-item">
          <span class="state-label">{{ t('settings.debug_lt_role') }}</span>
          <span class="state-value">{{ ltStore.role || '—' }}</span>
        </div>
        <div class="state-item">
          <span class="state-label">{{ t('settings.debug_lt_version') }}</span>
          <span class="state-value mono">{{ ltStore.roomState?.version ?? '—' }}</span>
        </div>
        <div class="state-item">
          <span class="state-label">{{ t('settings.debug_lt_members') }}</span>
          <span class="state-value">{{ ltStore.members.length }}</span>
        </div>
      </div>
    </div>

    </section>

    <Teleport to="body">
      <div
        v-if="showClearDebugCookiesConfirm"
        class="debug-dialog-overlay"
        @click.self="showClearDebugCookiesConfirm = false"
      >
        <div class="debug-dialog-card">
          <h3>{{ t('settings.debug_cookie_clear_confirm_title') }}</h3>
          <p>{{ t('settings.debug_cookie_clear_confirm_desc') }}</p>
          <div class="debug-dialog-actions">
            <button @click="showClearDebugCookiesConfirm = false">
              {{ t('settings.cancel') }}
            </button>
            <button class="danger" @click="clearDebugCookies">
              {{ t('settings.debug_cookie_clear') }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- 隐藏调试模式 -->
    <div class="hide-debug-section">
      <button class="hide-debug-btn" @click="hideDebugMode">
        <span class="material-symbols-rounded">visibility_off</span>
        <span>{{ t('settings.debug_hide') }}</span>
      </button>
    </div>
  </div>
</template>

<style scoped lang="scss">
.debug-view {
  padding: 20px 28px 32px;
  max-width: 1180px;
  /* 宽屏下水平居中，避免固定左对齐右侧大空白 */
  margin-inline: auto;
}

.debug-tabs {
  position: sticky;
  top: 0;
  z-index: 3;
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: -8px 0 20px;
  padding: 8px 0;
  background: color-mix(in srgb, var(--md-background, var(--md-surface)) 88%, transparent);
  backdrop-filter: blur(12px);
}

.debug-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 36px;
  padding: 0 14px;
  border-radius: var(--radius-full);
  font-size: 13px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
  background: var(--md-surface-container);
  transition: background 160ms var(--ease-standard), color 160ms var(--ease-standard);
  .material-symbols-rounded { font-size: 18px; }
  &:hover { background: var(--md-surface-container-high); color: var(--md-on-surface); }
  &.active { background: var(--md-secondary-container); color: var(--md-on-secondary-container); }
}

.debug-tab-badge {
  min-width: 18px;
  padding: 0 5px;
  border-radius: var(--radius-full);
  background: var(--md-error);
  color: var(--md-on-error);
  font-size: 11px;
  line-height: 18px;
  text-align: center;
}

.debug-panel { animation: debug-panel-in 220ms var(--ease-emphasized-decel); }

/* 概览宽屏双列：左边构建 / 系统 / 运行时，右边播放器 / 同步 */
.debug-columns {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(420px, 1fr));
  gap: 0 24px;
  align-items: start;
}

.debug-column { min-width: 0; }

.state-value.warn { color: #f59e0b; }

@keyframes debug-panel-in {
  from { opacity: 0; transform: translateY(6px); }
}

.debug-header {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 24px;
  padding-top: 8px;
}

.debug-back-btn {
  width: 40px;
  height: 40px;
  border-radius: var(--radius-full);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--md-on-surface);
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container-high); }
}

.page-title {
  font-size: 28px;
  font-weight: 700;
  letter-spacing: -0.5px;
}

.section-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.8px;
  color: var(--md-primary);
  margin: 24px 0 10px;
  padding: 0 4px;

  &:first-of-type { margin-top: 0; }
}

.setting-card {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px 16px;
  border-radius: var(--radius-lg);
  background: var(--md-surface-container);
  margin-bottom: 8px;
  transition: background var(--duration-short);

  &:hover { background: var(--md-surface-container-high); }

  /* 纵向内容卡变体：探针详情 / 测试异常 / 崩溃报告共用，
     修掉 flex-row 单行卡被误用作纵向容器导致的竖排压缩 */
  &.stacked {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }
}

.sub-card {
  margin-left: 54px;
  background: var(--md-surface-container-low) !important;
}

.setting-icon-wrap {
  width: 40px;
  height: 40px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container-high);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: var(--md-on-surface-variant);
}

.setting-info { flex: 1; min-width: 0; }
.setting-title { font-size: 14px; font-weight: 500; }
.setting-desc { font-size: 12px; color: var(--md-on-surface-variant); margin-top: 2px; }

.spinning {
  animation: spin 1s linear infinite;
}

@keyframes spin { to { transform: rotate(360deg); } }

.state-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 12px 24px;
  padding: 4px 0;
}

.state-item {
  display: flex;
  flex-direction: column;
  gap: 2px;

  &.full-width {
    grid-column: 1 / -1;
  }

  &.copyable-state-item {
    cursor: pointer;
    border-radius: 8px;
    padding: 4px 6px;
    margin: -4px -6px;
    transition: background var(--duration-short, 0.15s);

    &:hover {
      background: var(--md-surface-container-high, rgba(255, 255, 255, 0.06));
    }
  }
}

.state-label {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--md-on-surface-variant);
  opacity: 0.7;
}

.state-value {
  font-size: 13px;
  font-weight: 500;
  color: var(--md-on-surface);
  word-break: break-all;
  line-height: 1.45;

  &.mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }
}

.debug-cookie-clear-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  border-radius: var(--radius-full);
  background: var(--md-error-container, rgba(255, 80, 80, 0.12));
  color: var(--md-on-error-container, #ff5050);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;

  &:disabled {
    cursor: wait;
    opacity: 0.6;
  }

  .material-symbols-rounded { font-size: 17px; }
}

.debug-dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 10000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  background: rgba(0, 0, 0, 0.48);
}

.debug-dialog-card {
  width: min(380px, 100%);
  padding: 22px;
  border-radius: var(--radius-xl);
  background: var(--md-surface-container-high);
  box-shadow: 0 18px 48px rgba(0, 0, 0, 0.3);

  h3 { margin: 0; font-size: 18px; }
  p {
    margin: 12px 0 20px;
    color: var(--md-on-surface-variant);
    font-size: 13px;
    line-height: 1.55;
  }
}

.debug-dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;

  button {
    padding: 8px 14px;
    border-radius: var(--radius-full);
    color: var(--md-on-surface);
    cursor: pointer;
  }

  button.danger {
    background: var(--md-error);
    color: var(--md-on-error);
  }
}

.hide-debug-section {
  margin-top: 32px;
  padding-top: 16px;
  border-top: 1px solid var(--md-outline-variant, rgba(255,255,255,0.08));
}

.hide-debug-btn {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 10px 20px;
  border-radius: 12px;
  background: var(--md-error-container, rgba(255, 80, 80, 0.12));
  color: var(--md-on-error-container, #ff5050);
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.15s, transform 0.1s;

  .material-symbols-rounded {
    font-size: 20px;
  }

  &:hover {
    background: var(--md-error-container, rgba(255, 80, 80, 0.18));
  }

  &:active {
    transform: scale(0.97);
  }
}

.debug-log-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 32px;
  padding: 0 12px;
  border-radius: var(--radius-full);
  background: var(--md-surface-container);
  color: var(--md-on-surface-variant);
  font-size: 12px;
  transition: background 150ms;

  .material-symbols-rounded { font-size: 16px; }
  &:hover { background: var(--md-surface-container-highest); color: var(--md-on-surface); }

  &:disabled {
    opacity: 0.5;
    cursor: default;
  }
}

.debug-crash-clear { color: var(--md-error); }

.debug-logs-empty {
  padding: 20px;
  text-align: center;
  font-size: 13px;
  color: var(--md-on-surface-variant);
}

.debug-crash-list { display: flex; flex-direction: column; gap: 6px; margin-top: 8px; }

.debug-crash-item {
  border-radius: 12px;
  background: var(--md-surface-container);
  overflow: hidden;
}

.debug-crash-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 10px 12px;
  color: var(--md-on-surface);
  font-size: 13px;
  transition: background 150ms;
  /* 吸顶 + 不透明背景，展开的日志滚动时不与文件名行重叠 */
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--md-surface-container);

  &:hover { background: var(--md-surface-container-high); }
}

.debug-crash-name { flex: 1; min-width: 0; text-align: left; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-mono); }
.debug-crash-size { color: var(--md-on-surface-variant); font-size: 12px; flex-shrink: 0; }

.debug-crash-body {
  margin: 0;
  padding: 12px;
  max-height: 280px;
  /* 保持原始排版并在容器内滚动，不做逐字符硬折行 */
  overflow: auto;
  overscroll-behavior: contain;
  font-family: var(--font-mono);
  font-size: 11px;
  line-height: 1.5;
  white-space: pre;
  border-top: 1px solid var(--md-outline-variant);
  color: var(--md-on-surface-variant);
}

.probe-group {
  border-radius: 12px;
  background: var(--md-surface-container);
  margin-bottom: 6px;
  overflow: hidden;
}

.probe-group-header {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 10px 12px;
  font-size: 14px;
  font-weight: 600;
  color: var(--md-on-surface);
  transition: background 150ms;

  &:hover { background: var(--md-surface-container-high); }
}

.probe-actions { padding: 4px 12px 10px; display: flex; flex-direction: column; gap: 8px; }

.probe-action-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.probe-action-label { flex: 1; min-width: 0; font-size: 13px; }
.probe-action-status {
  font-size: 12px;
  font-family: var(--font-mono);
  color: var(--md-on-surface-variant);
  &.failed { color: var(--md-error); }
  &.success { color: var(--md-primary); }
}

.probe-result {
  margin: 0;
  padding: 10px;
  max-height: 220px;
  /* JSON 保持缩进原样，超宽在容器内横向滚动 */
  overflow: auto;
  overscroll-behavior: contain;
  border-radius: 10px;
  background: var(--md-surface-container-lowest, var(--md-surface));
  font-family: var(--font-mono);
  font-size: 11px;
  line-height: 1.5;
  white-space: pre;

  &.failed { color: var(--md-error); }
}

.probe-expand-enter-active { transition: opacity 180ms ease-out, transform 200ms cubic-bezier(0.05, 0.7, 0.1, 1); }
.probe-expand-enter-from { opacity: 0; transform: translateY(-4px); }
.probe-expand-leave-active { transition: opacity 120ms ease-in; }
.probe-expand-leave-to { opacity: 0; }

.crash-test-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 8px;
}

.crash-test-btn {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  padding: 10px 12px;
  border-radius: 12px;
  background: var(--md-surface-container);
  text-align: left;
  transition: background 150ms;

  &:hover { background: var(--md-surface-container-high); }
  &.danger .crash-test-title { color: var(--md-error); }
}

.crash-test-title { font-size: 13px; font-weight: 600; color: var(--md-on-surface); }
.crash-test-desc { font-size: 11px; color: var(--md-on-surface-variant); }

.debug-switch {
  position: relative;
  display: inline-flex;
  flex-shrink: 0;
  cursor: pointer;

  input { position: absolute; opacity: 0; pointer-events: none; }
}

.debug-switch-track {
  width: 44px;
  height: 24px;
  border-radius: 12px;
  background: var(--md-surface-container-highest);
  transition: background 180ms;
  display: block;
}

.debug-switch-thumb {
  position: absolute;
  top: 3px;
  left: 3px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--md-outline);
  transition: transform 180ms cubic-bezier(0.05, 0.7, 0.1, 1), background 180ms;
  display: block;
}

.debug-switch input:checked + .debug-switch-track {
  background: var(--md-primary);
  .debug-switch-thumb { transform: translateX(20px); background: var(--md-on-primary); }
}

@media (prefers-reduced-motion: reduce) {
  .probe-expand-enter-active,
  .probe-expand-leave-active,
  .debug-switch-thumb { transition: none; }
}
</style>
