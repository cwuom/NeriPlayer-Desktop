import { describe, expect, it, vi } from 'vitest'
import {
  activeDownloadProgressText,
  activeDownloadStatusText,
  downloadTemplatePreview,
  formatDownloadSize,
} from '@/components/settings/useDownloadSettings'

// 纯函数不碰 store；真实 store 模块在导入时就会读 localStorage
vi.mock('@/stores/settings', () => ({ DEFAULT_DOWNLOAD_NAME_TEMPLATE: '{artist} - {title}', useSettingsStore: vi.fn() }))
vi.mock('@/stores/download', () => ({ useDownloadStore: vi.fn() }))

const t = (key: string) => key

describe('formatDownloadSize', () => {
  it('uses B, KB and MB with one decimal', () => {
    expect(formatDownloadSize()).toBe('')
    expect(formatDownloadSize(512)).toBe('512 B')
    expect(formatDownloadSize(1536)).toBe('1.5 KB')
    expect(formatDownloadSize(5 * 1024 * 1024)).toBe('5.0 MB')
  })
})

describe('activeDownloadProgressText', () => {
  it('shows the status for non-transferring states', () => {
    for (const status of ['queued', 'resolving', 'processing', 'cancelling', 'cancelled', 'already_exists']) {
      expect(activeDownloadProgressText({ status, downloadedBytes: 10, totalBytes: 20, progress: 50 }, t))
        .toBe(activeDownloadStatusText(status, t))
    }
    expect(activeDownloadStatusText('already_exists', t)).toBe('download.already_exists')
  })

  it('appends the failure message to errors when there is one', () => {
    expect(activeDownloadProgressText({ status: 'error', message: 'timeout' }, t)).toBe('download.download_failed · timeout')
    expect(activeDownloadProgressText({ status: 'error' }, t)).toBe('download.download_failed')
  })

  it('combines the transferred size, total and percentage that are known', () => {
    const mb = 1024 * 1024
    expect(activeDownloadProgressText({ status: 'downloading', downloadedBytes: mb, totalBytes: 4 * mb, progress: 25 }, t)).toBe('1.0 MB / 4.0 MB · 25%')
    expect(activeDownloadProgressText({ status: 'downloading', downloadedBytes: mb, totalBytes: 4 * mb }, t)).toBe('1.0 MB / 4.0 MB')
    expect(activeDownloadProgressText({ status: 'downloading', downloadedBytes: mb, totalBytes: 0, progress: 25 }, t)).toBe('1.0 MB · 25%')
    expect(activeDownloadProgressText({ status: 'downloading' }, t)).toBe('download.downloading')
  })
})

describe('downloadTemplatePreview', () => {
  it('expands both {token} and %token% placeholders with the sample track', () => {
    expect(downloadTemplatePreview('{artist} - {title} [%album%] %id% {subAudioId}{hash}')).toBe('周杰伦 - 晴天 [叶惠美] netease:123456 4c853a1f')
  })

  it('previews the default template when the input is empty', () => {
    expect(downloadTemplatePreview('')).toBe('周杰伦 - 晴天')
  })
})
