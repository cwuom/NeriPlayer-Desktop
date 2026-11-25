<script setup lang="ts">
import { useI18n } from 'vue-i18n'

// 桌面宽屏下的曲目表头：# / 标题 / 专辑 / 时长，列宽与 detail-view.scss 的曲目行一致；
// 窄内容区隐藏（专辑回到副标题里显示）
defineProps<{
  /** 是否有专辑列 */
  album?: boolean
  /** 时长前额外的一列（如「最近播放」的播放时间） */
  extra?: string
  /** 行尾是否有「更多」按钮，用于对齐 */
  more?: boolean
}>()

const { t } = useI18n()
</script>

<template>
  <div class="track-list-header" aria-hidden="true">
    <span class="col-index">#</span>
    <span class="track-list-header__cover" />
    <span class="track-list-header__title">{{ t('library.column_title') }}</span>
    <span v-if="album" class="col-album">{{ t('library.column_album') }}</span>
    <span v-if="extra" class="track-list-header__extra">{{ extra }}</span>
    <span class="col-duration"><span class="material-symbols-rounded">schedule</span></span>
    <span v-if="more" class="track-list-header__more" />
  </div>
</template>

<style scoped lang="scss">
/* 列宽与 detail-view.scss 中曲目行（序号 28 / 封面 44 / 专辑列 / 时长 44 / 更多 32）保持一致 */
.track-list-header {
  display: none;
  align-items: center;
  gap: 12px;
  height: 34px;
  padding: 0 10px;
  margin-top: 4px;
  border-bottom: 1px solid var(--hairline);
  font-size: var(--fs-12);
  font-weight: 600;
  color: var(--md-on-surface-variant);
}

@container (min-width: 760px) {
  .track-list-header { display: flex; }
}

.col-index { flex: 0 0 28px; text-align: center; }
.track-list-header__cover { flex: 0 0 44px; }
.track-list-header__title { flex: 1; min-width: 0; }
.col-album { flex: 0 0 clamp(140px, 24cqi, 320px); }
.track-list-header__extra { flex: 0 0 60px; text-align: right; }
.col-duration {
  flex: 0 0 44px;
  text-align: right;

  .material-symbols-rounded { font-size: 16px; vertical-align: -3px; }
}
.track-list-header__more { flex: 0 0 32px; }
</style>
