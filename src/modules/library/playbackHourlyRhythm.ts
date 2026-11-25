// 各时段收听：对齐 Android PlaybackStatsRhythm 的「小时」模式，数据来自同步的每小时收听总量

export const HOURS_PER_DAY = 24

export interface HourlyRhythm {
  /** 0..23 时各自的收听毫秒数 */
  hours: number[]
  peakIndex: number
  peakMs: number
}

/** 后端给的数组不足 24 项或全为 0 时不展示，和 Android 只在有数据时出图一致 */
export function buildHourlyRhythm(raw: readonly unknown[] | null | undefined): HourlyRhythm | null {
  if (!Array.isArray(raw) || raw.length !== HOURS_PER_DAY) return null
  const hours = raw.map(value => (typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : 0))
  const peakMs = Math.max(...hours)
  if (peakMs <= 0) return null
  return { hours, peakIndex: hours.indexOf(peakMs), peakMs }
}

/** 柱高占比；有数据的小时至少露出一点，免得被高峰压成看不见 */
export function hourBarRatio(value: number, peakMs: number): number {
  if (value <= 0 || peakMs <= 0) return 0
  return Math.max(0.04, Math.min(1, value / peakMs))
}

/** 坐标轴只标 0、6、12、18 和 23 点，与 Android 相同 */
export function isHourAxisTick(hour: number): boolean {
  return hour % 6 === 0 || hour === HOURS_PER_DAY - 1
}

export function clampHour(hour: number): number {
  return Math.min(HOURS_PER_DAY - 1, Math.max(0, Math.trunc(hour)))
}
