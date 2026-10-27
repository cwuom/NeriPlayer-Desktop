import { COVER_SIZE_DETAIL, COVER_SIZE_LIST, toHttpsImageUrl, withCoverSize, type CoverSize } from './trackCover'

/** NetEase cover field compatibility: picUrl / blurPicUrl / coverImgUrl / numeric pic id.
 * List callers get exactly the list size (an existing 1200px param is downgraded);
 * detail callers keep a larger existing parameter. */
export function resolveNeteaseCoverSized(size: CoverSize, ...candidates: unknown[]): string {
  const px = size === 'detail' ? COVER_SIZE_DETAIL : COVER_SIZE_LIST
  const exact = size === 'list'

  for (const raw of candidates) {
    if (raw == null) continue
    if (typeof raw === 'string') {
      const value = raw.trim()
      if (!value) continue
      if (/^(https?:)?\/\//i.test(value)) return withCoverSize(toHttpsImageUrl(value), px, exact)
      // 少数接口只返回 pic 哈希/数字串
      if (/^[A-Za-z0-9_-]+$/.test(value) && value.length >= 8) {
        return withCoverSize(`https://p1.music.126.net/${value}.jpg`, px, exact)
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

/** Backwards-compatible list-sized resolver */
export function resolveNeteaseCover(...candidates: unknown[]): string {
  return resolveNeteaseCoverSized('list', ...candidates)
}
