// 应用内更新的数据形状与纯逻辑：与 src-tauri/src/commands/update_cmd.rs 的返回值一一对应

export type UnsupportedReason =
  | 'windowsMsi'
  | 'macDiskImage'
  | 'macTranslocated'
  | 'linuxPackage'
  | 'flatpak'
  | 'readOnly'
  | 'unknown'

export interface PlanSummary {
  /** 本次实际需要下载的字节数 */
  downloadBytes: number
  /** 不做增量时要下载的字节数 */
  fullBytes: number
  changedFiles: number
  totalFiles: number
}

export interface UpdateCheck {
  status: 'upToDate' | 'available' | 'disabled'
  currentVersion: string
  latestVersion: string | null
  releaseUrl: string | null
  inApp: boolean
  unsupportedReason: UnsupportedReason | null
  summary: PlanSummary | null
}

export interface UpdateProgress {
  downloaded: number
  total: number
}

export type UpdateErrorCode =
  | 'network' | 'signature' | 'manifest' | 'corrupted' | 'permission' | 'io'
  | 'cancelled' | 'busy' | 'noUpdate' | 'unknown'

const ERROR_CODES: readonly UpdateErrorCode[] = [
  'network', 'signature', 'manifest', 'corrupted', 'permission', 'io', 'cancelled', 'busy', 'noUpdate', 'unknown',
]

export const RELEASES_URL = 'https://github.com/cwuom/NeriPlayer-Desktop/releases/latest'

/** 自动检查的最小间隔：启动频繁时不必每次都请求 */
export const AUTO_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000

/** 后端错误是 `{ code, message }`；浏览器开发模式或异常路径下可能是字符串或 Error */
export function updateErrorCode(error: unknown): UpdateErrorCode {
  if (typeof error === 'object' && error !== null && 'code' in error) {
    const code = (error as { code: unknown }).code
    if (typeof code === 'string' && (ERROR_CODES as readonly string[]).includes(code)) return code as UpdateErrorCode
  }
  return 'unknown'
}

export function updateErrorMessage(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'message' in error) {
    return String((error as { message: unknown }).message)
  }
  return String(error)
}

/** 增量相对整包省下的比例（0–100 的整数）；没有可比较的整包大小时返回 0 */
export function savedPercent(summary: PlanSummary | null): number {
  if (!summary || summary.fullBytes <= 0) return 0
  const saved = 1 - summary.downloadBytes / summary.fullBytes
  return Math.max(0, Math.min(100, Math.round(saved * 100)))
}

/** 只有真正省下了一截才宣传「增量更新」，否则与整包无异，不必强调 */
export function isIncremental(summary: PlanSummary | null): boolean {
  return savedPercent(summary) >= 10
}

/** 本次计划下载的字节数；还没有下载计划时按 0 处理 */
export function plannedDownloadBytes(check: UpdateCheck | null): number {
  return check?.summary?.downloadBytes ?? 0
}

export function progressPercent(progress: UpdateProgress | null): number {
  if (!progress || progress.total <= 0) return progress ? 100 : 0
  return Math.max(0, Math.min(100, Math.floor((progress.downloaded / progress.total) * 100)))
}

export function shouldAutoCheck(lastCheckAt: number | null, now: number): boolean {
  return lastCheckAt === null || !Number.isFinite(lastCheckAt) || now - lastCheckAt >= AUTO_CHECK_INTERVAL_MS || now < lastCheckAt
}

/** 已选择「跳过此版本」的版本在自动检查时不再打扰，手动检查仍然提示 */
export function shouldPrompt(check: UpdateCheck, skippedVersion: string | null, manual: boolean): boolean {
  if (check.status !== 'available') return false
  return manual || !skippedVersion || check.latestVersion !== skippedVersion
}
