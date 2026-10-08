// 探索页搜索历史策略（对齐 Android ExploreSearchHistoryPolicy）

export const SEARCH_HISTORY_LIMIT = 15

/** 新关键词放最前：去首尾空白，忽略大小写去重，最多保留 15 条 */
export function updatedSearchHistory(
  current: readonly string[],
  query: string,
  limit = SEARCH_HISTORY_LIMIT,
): string[] {
  const keyword = query.trim()
  if (!keyword || limit <= 0) return current.slice(0, Math.max(0, limit))
  return normalizeSearchHistory([keyword, ...current], limit)
}

/** 读回存储内容：只留非空字符串，忽略大小写去重，截到上限 */
export function normalizeSearchHistory(raw: unknown, limit = SEARCH_HISTORY_LIMIT): string[] {
  if (!Array.isArray(raw)) return []
  const result: string[] = []
  const seen = new Set<string>()
  for (const item of raw) {
    if (typeof item !== 'string') continue
    const keyword = item.trim()
    const key = keyword.toLowerCase()
    if (!keyword || seen.has(key)) continue
    seen.add(key)
    result.push(keyword)
    if (result.length >= limit) break
  }
  return result
}
