<script setup lang="ts">
import { ref, watch } from 'vue'
import {
  normalizeCoverUrlForDisplay,
  normalizeProxiedCoverUrl,
  peekCoverImage,
  resolveCoverImage,
} from '@/utils/bilibiliCover'
import { createLogger } from '@/utils/logger'
import { summarizeLogError } from '@/utils/logSanitizer'

defineOptions({ inheritAttrs: false })

const props = withDefaults(defineProps<{
  src?: string | null
  alt?: string
}>(), {
  src: '',
  alt: '',
})

const log = createLogger('cover-image')

const resolvedSrc = ref('')
let resolveRequestToken = 0
let renderRetryCount = 0

watch(() => props.src, (src) => {
  resolveRequestToken++
  const proxiedUrl = normalizeProxiedCoverUrl(src || '')
  const cachedSrc = peekCoverImage(src || '')
  const normalizedSrc = proxiedUrl || normalizeCoverUrlForDisplay(src || '')
  resolvedSrc.value = cachedSrc || normalizedSrc
  renderRetryCount = 0
  if (!src) return

}, { immediate: true })

function describeSource(src: string): string {
  if (src.startsWith('data:')) return `data-url(${src.length})`
  try {
    return new URL(src).hostname || 'url'
  } catch {
    return `invalid(${src.length})`
  }
}

async function loadCover(src: string, forceRefresh: boolean) {
  const requestToken = ++resolveRequestToken
  const started = performance.now()
  log.info('proxy resolve begin:', {
    source: describeSource(src),
    forceRefresh,
  })
  try {
    const nextSrc = await resolveCoverImage(src, { forceRefresh })
    if (requestToken === resolveRequestToken) {
      resolvedSrc.value = nextSrc
      log.info('proxy resolve committed:', {
        source: describeSource(nextSrc),
        chars: nextSrc.length,
        elapsedMs: Math.round(performance.now() - started),
      })
    } else {
      log.info('proxy resolve ignored: stale source')
    }
  } catch (error) {
    if (requestToken === resolveRequestToken) {
      resolvedSrc.value = ''
      log.warn('proxy resolve failed:', {
        source: describeSource(src),
        elapsedMs: Math.round(performance.now() - started),
        error: summarizeLogError(error),
      })
    }
  }
}

function handleError(event: Event) {
  const image = event.currentTarget as HTMLImageElement
  const failedSrc = image.getAttribute('src') || image.src
  const proxiedUrl = normalizeProxiedCoverUrl(props.src || '')
  if (failedSrc !== resolvedSrc.value) return

  log.warn('image failed:', {
    source: describeSource(failedSrc),
    hasProxy: !!proxiedUrl,
    retry: renderRetryCount,
  })
  if (!proxiedUrl || renderRetryCount >= 2) {
    resolvedSrc.value = ''
    return
  }

  resolvedSrc.value = ''
  const forceRefresh = renderRetryCount > 0
  renderRetryCount++
  void loadCover(proxiedUrl, forceRefresh)
}
</script>

<template>
  <img
    v-if="resolvedSrc"
    v-bind="$attrs"
    :src="resolvedSrc"
    :alt="alt"
    loading="lazy"
    decoding="async"
    referrerpolicy="no-referrer"
    @error="handleError"
  />
  <slot v-else />
</template>
