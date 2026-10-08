// 平台前缀属于同步身份，界面展示时清理，原始 album 不应被改写
export function displayAlbum(album: unknown): string {
  if (typeof album !== 'string') return ''
  const value = album.trim()
  if (/^bilibili(?:$|\|)/i.test(value)) return ''
  return value.replace(/^netease/i, '').trim()
}
