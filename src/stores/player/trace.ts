import { invoke } from '@tauri-apps/api/core'
import { getPlaybackSourceKind } from '@/modules/playback/playbackSource'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'
import type { TrackInfo } from '@/types/track'

const uiLog = createLogger('playback-ui')

export function tracePlaybackUi(
  stage: string,
  track?: TrackInfo | null,
  detail?: string,
  requestGeneration?: number,
) {
  const source = track ? getPlaybackSourceKind(track) || track.source || 'local' : undefined
  const safeDetail = detail ? summarizeLogError(detail) : undefined
  uiLog.info(
    `stage=${stage}`,
    { generation: requestGeneration, id: track?.id, source, detail: safeDetail },
  )
  if (!import.meta.env.DEV) return
  void invoke<void>('trace_playback_ui', {
    request: {
      stage,
      trackId: track?.id,
      source,
      detail: safeDetail,
      requestGeneration,
    },
  }).catch((error) => {
    uiLog.warn('backend trace unavailable:', error)
  })
}
