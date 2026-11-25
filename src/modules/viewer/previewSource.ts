/**
 * 封面预览实际显示的图片：原图加载完成后用原图，加载前先显示已有的封面占位。
 * 原图加载失败（如离线播放时只有本地缓存封面）时保留占位，两者都不可用才显示“无图”
 */
export function previewImageSource(state: {
  src: string
  placeholder?: string | null
  loaded: boolean
  failed: boolean
}): string | null {
  if (state.loaded && state.src) return state.src
  if (state.placeholder) return state.placeholder
  return state.failed ? null : state.src || null
}
