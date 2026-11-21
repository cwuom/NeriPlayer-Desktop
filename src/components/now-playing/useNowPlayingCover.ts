import { ref, watch } from 'vue'
import { usePlayerStore } from '@/stores/player'
import { extractPalette, type PaletteResult } from '@/utils/paletteExtractor'
import {
  normalizeCoverUrlForDisplay,
  normalizeProxiedCoverUrl,
  peekCoverImage,
  resolveCoverImage,
} from '@/utils/bilibiliCover'
import { getTrackCoverUrl } from '@/utils/trackCover'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'
import { coverSourceLabel } from './nowPlayingHelpers'

const log = createLogger('now-playing')

type ShaderColors = [number[], number[], number[], number[], number[]]

// 从封面提取的动态颜色（归一化 RGBA）
function createDefaultExtractedColors(): ShaderColors {
  return [
    [0.07, 0.27, 0.42, 1],
    [0.35, 0.24, 0.20, 1],
    [0.34, 0.12, 0.26, 1],
    [0.17, 0.14, 0.34, 1],
    [0.18, 0.34, 0.36, 1],
  ]
}

const PALETTE_COVER_DECODE_SIZE = 320

/** 详情封面的解析、失败重试与取色；背景着色器与 AccentBackdrop 都读这里的调色板 */
export function useNowPlayingCover() {
  const player = usePlayerStore()
  const coverLoadError = ref(false)
  const coverUrl = ref('')
  let coverRenderRetryCount = 0

  const extractedColors = ref(createDefaultExtractedColors())
  const paletteResult = ref<PaletteResult | null>(null)
  let paletteRequestToken = 0

  function resetExtractedPalette() {
    paletteResult.value = null
    extractedColors.value = createDefaultExtractedColors()
  }

  // 使用 Android 同尺寸封面采样，避免小图把细节压成单色
  function extractColorsFromCover(url: string): Promise<boolean> {
    const requestToken = ++paletteRequestToken
    return new Promise((resolve) => {
      const img = new Image()
      if (!url.startsWith('data:') && !url.startsWith('blob:')) {
        img.crossOrigin = 'anonymous'
      }
      img.referrerPolicy = 'no-referrer'
      img.onload = () => {
        if (requestToken !== paletteRequestToken) {
          resolve(false)
          return
        }
        try {
          const canvas = document.createElement('canvas')
          const size = PALETTE_COVER_DECODE_SIZE
          canvas.width = size
          canvas.height = size
          const ctx = canvas.getContext('2d', { willReadFrequently: true })
          if (!ctx) throw new Error('Canvas 2D context is unavailable')
          ctx.drawImage(img, 0, 0, size, size)
          const imageData = ctx.getImageData(0, 0, size, size)

          const palette = extractPalette(imageData, 16)
          paletteResult.value = palette
          extractedColors.value = palette.shaderColors.map(
            (c) => [c[0] / 255, c[1] / 255, c[2] / 255, 1]
          ) as ShaderColors
          resolve(true)
        } catch (error) {
          if (requestToken === paletteRequestToken) resetExtractedPalette()
          log.error('color extraction failed:', error)
          resolve(false)
        }
      }
      img.onerror = () => {
        if (requestToken === paletteRequestToken) {
          resetExtractedPalette()
          log.error('cover image load failed:', coverSourceLabel(url))
        }
        resolve(false)
      }
      img.src = url
    })
  }

  // 支持的平台封面统一走后端代理，保证显示与取色读取同一份已验证数据
  watch(
    () => [
      player.hasPlaybackSession,
      player.hasPlaybackSession ? getTrackCoverUrl(player.currentTrack, 'detail') : '',
      player.hasPlaybackSession ? player.currentTrack?.id || '' : '',
    ] as const,
    async ([, rawUrl], _, onCleanup) => {
      let active = true
      onCleanup(() => { active = false })

      paletteRequestToken++
      coverRenderRetryCount = 0
      coverLoadError.value = false

      const proxiedUrl = normalizeProxiedCoverUrl(rawUrl)
      const normalizedUrl = proxiedUrl || normalizeCoverUrlForDisplay(rawUrl)
      log.info('cover resolve begin:', {
        trackId: player.currentTrack?.id,
        rawSource: coverSourceLabel(rawUrl),
        proxied: !!proxiedUrl,
        rawChars: rawUrl.length,
      })
      if (!normalizedUrl) {
        coverUrl.value = ''
        resetExtractedPalette()
        log.warn('cover resolve skipped: no usable URL')
        return
      }

      // 先显示原始地址，代理解析在后台替换，避免详情页首帧出现占位符
      const cachedUrl = proxiedUrl ? peekCoverImage(proxiedUrl) : ''
      coverUrl.value = cachedUrl || normalizedUrl
      log.info('cover fallback displayed:', {
        trackId: player.currentTrack?.id,
        source: coverSourceLabel(coverUrl.value),
        cacheHit: !!cachedUrl,
      })

      let displayUrl = cachedUrl || normalizedUrl
      if (proxiedUrl && !cachedUrl) {
        const proxyStarted = performance.now()
        try {
          displayUrl = await resolveCoverImage(proxiedUrl)
          log.info('cover proxy resolved:', {
            trackId: player.currentTrack?.id,
            elapsedMs: Math.round(performance.now() - proxyStarted),
            dataUrlChars: displayUrl.length,
          })
        } catch (error) {
          log.error('failed to resolve proxied cover:', {
            trackId: player.currentTrack?.id,
            source: coverSourceLabel(proxiedUrl),
            elapsedMs: Math.round(performance.now() - proxyStarted),
            error: summarizeLogError(error),
          })
          if (!active) return
          // 代理封面解析失败时回退到原始 URL 直接显示，而非清空封面
          displayUrl = normalizedUrl
        }
      }

      if (!active) {
        log.info('cover proxy result ignored: stale track')
        return
      }
      // 封面解析成功即显示，取色仅作背景调色板的尽力而为，不再阻塞封面渲染
      // 否则取色失败（canvas 读取异常/token 竞态）会让已解析封面永远显示不出来
      coverUrl.value = displayUrl
      coverLoadError.value = false
      log.info('cover display committed:', {
        trackId: player.currentTrack?.id,
        source: coverSourceLabel(displayUrl),
        chars: displayUrl.length,
      })
      void extractColorsFromCover(displayUrl)
    },
    { immediate: true },
  )

  function handleNowPlayingCoverLoad(event: Event) {
    if (!player.hasPlaybackSession) return
    const src = (event.currentTarget as HTMLImageElement).src
    coverLoadError.value = false
    log.info('cover img loaded:', {
      trackId: player.currentTrack?.id,
      source: coverSourceLabel(src),
      chars: src.length,
    })
  }

  async function handleNowPlayingCoverError(event: Event) {
    if (!player.hasPlaybackSession) return
    const image = event.currentTarget as HTMLImageElement
    const failedSrc = image.getAttribute('src') || image.src
    const track = player.currentTrack
    const rawUrl = getTrackCoverUrl(track, 'detail')
    const proxiedUrl = normalizeProxiedCoverUrl(rawUrl)
    if (
      !track
      || !proxiedUrl
      || failedSrc !== coverUrl.value
      || coverRenderRetryCount >= 1
    ) {
      coverLoadError.value = true
      log.warn('cover img failed:', {
        trackId: track?.id,
        source: coverSourceLabel(failedSrc),
        retry: coverRenderRetryCount,
        hasProxy: !!proxiedUrl,
      })
      return
    }

    coverRenderRetryCount++
    paletteRequestToken++
    resetExtractedPalette()
    const expectedTrackId = track.id
    const expectedRawUrl = rawUrl
    const fallbackUrl = normalizeCoverUrlForDisplay(rawUrl)
    coverLoadError.value = false
    log.warn('cover img failed, refreshing proxy:', {
      trackId: expectedTrackId,
      source: coverSourceLabel(failedSrc),
      fallbackSource: coverSourceLabel(fallbackUrl),
    })

    try {
      const refreshedUrl = await resolveCoverImage(proxiedUrl, { forceRefresh: true })
      if (
        player.currentTrack?.id !== expectedTrackId
        || getTrackCoverUrl(player.currentTrack, 'detail') !== expectedRawUrl
      ) {
        log.info('cover refresh ignored: stale track')
        return
      }

      // 重新解析成功即恢复封面显示，取色失败不应再次把封面隐藏
      coverUrl.value = refreshedUrl
      coverLoadError.value = false
      log.info('cover refresh committed:', {
        trackId: expectedTrackId,
        dataUrlChars: refreshedUrl.length,
      })
      void extractColorsFromCover(refreshedUrl)
    } catch (error) {
      log.error('failed to refresh proxied cover:', summarizeLogError(error))
      if (fallbackUrl) {
        coverUrl.value = fallbackUrl
        coverLoadError.value = false
        log.info('cover refresh fallback committed:', {
          trackId: expectedTrackId,
          source: coverSourceLabel(fallbackUrl),
        })
      } else {
        coverLoadError.value = true
      }
    }
  }

  return {
    coverUrl,
    coverLoadError,
    extractedColors,
    paletteResult,
    handleNowPlayingCoverLoad,
    handleNowPlayingCoverError,
  }
}
