import { COVER_SIZE_LIST, toHttpsImageUrl, withCoverSize } from './trackCover'

/** NetEase cover field compatibility: picUrl / blurPicUrl / coverImgUrl / numeric pic id */
export function resolveNeteaseCover(...candidates: unknown[]): string {
  for (const raw of candidates) {
    if (raw == null) continue
    if (typeof raw === 'string') {
      const value = raw.trim()
      if (!value) continue
      if (/^(https?:)?\/\//i.test(value)) return withCoverSize(toHttpsImageUrl(value), COVER_SIZE_LIST)
      // 少数接口只返回 pic 哈希/数字串
      if (/^[A-Za-z0-9_-]+$/.test(value) && value.length >= 8) {
        return withCoverSize(`https://p1.music.126.net/${value}.jpg`, COVER_SIZE_LIST)
      }
      continue
    }
    if (typeof raw === 'number' && Number.isFinite(raw) && raw > 0) {
      // 纯数字 pic 字段无法稳定还原 URL，跳过
      continue
    }
  }
  return ''
}
