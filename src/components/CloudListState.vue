<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import type { CloudListStatus } from '@/stores/recommend'

/// 云端列表（用户歌单、收藏专辑）的加载状态
///
/// banner：已经有列表（缓存或上次结果）时放在列表旁，显示「正在刷新」或「刷新失败」；
/// placeholder：还没有可显示的列表时占位，加载中、失败、确实为空分开显示，
/// 加载期间不能显示「暂无歌单」。图标与「为空」的内容由使用方通过插槽给出。
withDefaults(defineProps<{
  status?: CloudListStatus
  variant: 'banner' | 'placeholder'
  loadingText?: string
  failedText?: string
}>(), { status: undefined, loadingText: undefined, failedText: undefined })

defineEmits<{ retry: [] }>()

const { t } = useI18n()
</script>

<template>
  <div
    v-if="variant === 'banner' && (status?.loading || status?.error)"
    class="cloud-banner"
    :class="{ failed: !status?.loading }"
    role="status"
    :title="status?.loading ? undefined : status?.error ?? undefined"
  >
    <span class="material-symbols-rounded" :class="{ spinning: status?.loading }">
      {{ status?.loading ? 'progress_activity' : 'error' }}
    </span>
    <span>{{ status?.loading ? t('library.cloud_refreshing') : t('library.cloud_refresh_failed') }}</span>
    <button v-if="!status?.loading" type="button" class="cloud-retry cloud-retry--compact" @click.stop="$emit('retry')">
      {{ t('player.retry') }}
    </button>
  </div>
  <div v-else-if="variant === 'placeholder'" class="empty-tab" role="status">
    <slot name="icon" />
    <p v-if="status?.loading" class="cloud-title">
      <span class="material-symbols-rounded spinning">progress_activity</span>
      {{ loadingText ?? t('library.cloud_loading') }}
    </p>
    <template v-else-if="status?.error">
      <p class="cloud-title">{{ failedText ?? t('library.cloud_load_failed') }}</p>
      <p class="cloud-desc" :title="status.error">{{ status.error }}</p>
      <button type="button" class="cloud-retry" @click="$emit('retry')">{{ t('player.retry') }}</button>
    </template>
    <slot v-else />
  </div>
</template>

<style scoped lang="scss">
.cloud-banner {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 28px;
  font-size: 12px;
  color: var(--md-on-surface-variant);

  .material-symbols-rounded { font-size: 16px; }

  &.failed { color: var(--md-error); }
}

.cloud-title {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-size: 16px;
  font-weight: 600;
  color: var(--md-on-surface-variant);
  margin-bottom: 4px;

  .material-symbols-rounded { font-size: 20px; }
}

.cloud-desc {
  max-width: 420px;
  margin-bottom: 12px;
  overflow: hidden;
  font-size: 13px;
  color: var(--md-on-surface-variant);
  opacity: 0.6;
  text-align: center;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cloud-retry {
  padding: 8px 24px;
  border-radius: var(--radius-full);
  background: var(--md-primary);
  color: var(--md-on-primary);
  font-size: 14px;
  font-weight: 500;
  transition: opacity var(--duration-short);

  &:hover { opacity: 0.9; }

  &--compact {
    padding: 2px 10px;
    font-size: 12px;
  }
}

.spinning { animation: cloud-spin 1s linear infinite; }

@keyframes cloud-spin { to { transform: rotate(360deg); } }

@media (prefers-reduced-motion: reduce) {
  .spinning { animation-duration: 3s; }
}
</style>
