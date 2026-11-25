import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

export interface DurationPart {
  value: string
  unit: string
}

function splitDuration(ms: number) {
  const seconds = Math.floor(Math.max(0, Number.isFinite(ms) ? ms : 0) / 1000)
  return { hours: Math.floor(seconds / 3600), minutes: Math.floor(seconds / 60) % 60, seconds: seconds % 60 }
}

/** 统计页的数字、百分比、时长与日期格式，跟随当前界面语言 */
export function useStatsFormat() {
  const { t, locale } = useI18n()

  // 语言代码不被 Intl 认识时退回运行时默认语言，避免整页渲染抛错
  function safeLocale(): string | undefined {
    try {
      return Intl.NumberFormat.supportedLocalesOf([locale.value]).length ? locale.value : undefined
    } catch {
      return undefined
    }
  }

  const integerFormat = computed(() => new Intl.NumberFormat(safeLocale()))
  const percentFormat = computed(() => new Intl.NumberFormat(safeLocale(), { style: 'percent', maximumFractionDigits: 1 }))
  const dateFormat = computed(() => new Intl.DateTimeFormat(safeLocale(), { dateStyle: 'medium' }))

  function number(value: number): string {
    return integerFormat.value.format(Math.max(0, Math.round(value || 0)))
  }

  function percent(value: number): string {
    return percentFormat.value.format(Math.min(1, Math.max(0, value || 0)))
  }

  function duration(ms: number): string {
    const { hours, minutes, seconds } = splitDuration(ms)
    if (hours > 0) return t('stats.duration_hm', { h: number(hours), m: minutes })
    if (minutes > 0) return t('stats.duration_m', { m: minutes })
    return t('stats.duration_s', { s: seconds })
  }

  /** 大号展示用：数字与单位分开排版 */
  function durationParts(ms: number): DurationPart[] {
    const { hours, minutes, seconds } = splitDuration(ms)
    if (hours > 0) {
      return [
        { value: number(hours), unit: t('stats.unit_hour') },
        { value: String(minutes), unit: t('stats.unit_minute') },
      ]
    }
    if (minutes > 0) return [{ value: String(minutes), unit: t('stats.unit_minute') }]
    return [{ value: String(seconds), unit: t('stats.unit_second') }]
  }

  /** 「N 次」，显式传入复数参数，英文单复数才会正确选择 */
  function plays(count: number): string {
    return t('stats.times', { count: number(count) }, Math.max(0, Math.round(count || 0)))
  }

  function date(ms: number): string {
    return ms > 0 ? dateFormat.value.format(ms) : ''
  }

  return { number, percent, duration, durationParts, plays, date }
}
