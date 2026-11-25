// 视频标题常把歌名包在书名号里（如「【洛天依】《歌名》原创PV」），整段标题去搜歌词基本搜不到（#53）
const TITLE_MARKS: readonly (readonly [string, string])[] = [['《', '》'], ['『', '』'], ['「', '」']]
const OPEN_PARENS = new Set(['(', '（'])
const CLOSE_PARENS = new Set([')', '）'])

/**
 * 标题里书名号内的歌名，没有时为 null。圆括号里的书名号多是出处
 * （如「光年之外 (《太空旅客》主题曲)」），提取出来反而会搜错歌，因此跳过
 */
export function extractQuotedSongTitle(title: string | null | undefined): string | null {
  const text = (title ?? '').trim()
  for (const [open, close] of TITLE_MARKS) {
    const inner = quotedOutsideParens(text, open, close)
    if (inner) return inner
  }
  return null
}

/** 按一种书名号找第一段不在圆括号里的非空内容；找不到时为 null */
function quotedOutsideParens(text: string, open: string, close: string): string | null {
  let depth = 0
  for (let index = 0; index < text.length; index++) {
    const char = text[index]!
    if (OPEN_PARENS.has(char)) depth++
    else if (CLOSE_PARENS.has(char)) depth = Math.max(0, depth - 1)
    else if (char === open && depth === 0) {
      const end = text.indexOf(close, index + 1)
      if (end < 0) return null
      const inner = text.slice(index + 1, end).trim()
      if (inner) return inner
      index = end
    }
  }
  return null
}

/** 自动取词依次尝试的歌名：书名号内的歌名优先，原标题兜底 */
export function lyricSearchTitles(title: string): string[] {
  const quoted = extractQuotedSongTitle(title)
  return quoted && quoted !== title.trim() ? [quoted, title] : [title]
}
