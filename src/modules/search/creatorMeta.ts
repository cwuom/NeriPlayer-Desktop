// 探索页创作者行的副标题：B 站结果带粉丝数 / 投稿数，其余平台沿用接口给的文案

export interface CreatorStats {
  follower_count?: number | null
  video_count?: number | null
}

/// 大数字按界面语言缩写（zh: 1.2万，en: 1.2K）；非法值按 0 处理，避免出现 NaN
export function formatCompactCount(value: number, locale: string): string {
  const safe = Number.isFinite(value) && value > 0 ? Math.floor(value) : 0
  try {
    return new Intl.NumberFormat(locale, { notation: 'compact', maximumFractionDigits: 1 }).format(safe)
  } catch {
    return String(safe)
  }
}

/// 没有粉丝数时返回 null，由调用方回退到签名等文案
export function biliCreatorStats(item: CreatorStats, locale: string): { fans: string; videos: string } | null {
  if (typeof item.follower_count !== 'number') return null
  return {
    fans: formatCompactCount(item.follower_count, locale),
    videos: formatCompactCount(item.video_count ?? 0, locale),
  }
}
