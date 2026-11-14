// 图片字节进出：网络封面走后端代理换 data URL，本地封面读 asset 地址；
// 落盘统一用原生保存对话框 + save_file_bytes，调用方只关心成功/取消
import { invoke } from '@tauri-apps/api/core'
import { normalizeProxiedCoverUrl } from './bilibiliCoverCache'
import { resolveCoverImage } from './bilibiliCover'

export interface ImageBytes {
  bytes: Uint8Array
  extension: string
}

const DATA_IMAGE_RE = /^data:image\/([\w+.-]+);base64,(.*)$/is

export function imageExtension(subtype: string): string {
  const normalized = subtype.toLowerCase()
  if (normalized === 'jpeg' || normalized === 'pjpeg') return 'jpg'
  return /^(png|webp|gif|avif|bmp)$/.test(normalized) ? normalized : 'jpg'
}

/** 文件名里的非法字符换成下划线，Windows/macOS/Linux 都能落盘 */
export function safeFileName(value: string, fallback: string): string {
  return value.replace(/[\\/:*?"<>|]+/g, '_').trim() || fallback
}

/** data: 就地解码；代理白名单域名先换后端 data URL（带 Referer 绕开图床跨域）；其余按同源地址取 */
export async function readImageBytes(src: string): Promise<ImageBytes> {
  let dataUrl = src.startsWith('data:') ? src : ''
  if (!dataUrl && normalizeProxiedCoverUrl(src)) dataUrl = await resolveCoverImage(src)
  const match = DATA_IMAGE_RE.exec(dataUrl)
  if (match) {
    const binary = atob(match[2])
    const bytes = new Uint8Array(binary.length)
    for (let index = 0; index < binary.length; index++) bytes[index] = binary.charCodeAt(index)
    return { bytes, extension: imageExtension(match[1]) }
  }
  const response = await fetch(src)
  if (!response.ok) throw new Error(`image request failed: ${response.status}`)
  const blob = await response.blob()
  return {
    bytes: new Uint8Array(await blob.arrayBuffer()),
    extension: imageExtension(blob.type.replace(/^image\//, '')),
  }
}

/**
 * 弹出原生保存对话框写入字节。
 * 返回 false 表示用户取消（不是失败，调用方不该报错）；写盘失败会抛出，由调用方提示
 */
export async function saveImageWithDialog(bytes: Uint8Array, baseName: string, extension: string): Promise<boolean> {
  const { save } = await import('@tauri-apps/plugin-dialog')
  const filePath = await save({
    defaultPath: `${safeFileName(baseName, 'image')}.${extension}`,
    filters: [{ name: 'Image', extensions: [extension] }],
  })
  if (!filePath) return false
  await invoke('save_file_bytes', { path: filePath, data: Array.from(bytes) })
  return true
}
