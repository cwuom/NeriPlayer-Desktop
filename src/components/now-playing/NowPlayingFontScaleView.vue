<script setup lang="ts">
// 更多面板的字号子视图：歌词字号、翻译字号与界面缩放
import { isTauri } from '@tauri-apps/api/core'
import { useI18n } from 'vue-i18n'
import {
  LYRIC_FONT_SCALE_MAX,
  LYRIC_FONT_SCALE_MIN,
  LYRIC_FONT_SCALE_STEP,
  LYRIC_TRANSLATION_SCALE_DEFAULT,
  LYRIC_TRANSLATION_SCALE_MAX,
  LYRIC_TRANSLATION_SCALE_MIN,
  LYRIC_TRANSLATION_SCALE_STEP,
  UI_SCALE_DEFAULT,
  UI_SCALE_MIN,
  UI_SCALE_STEP,
  normalizeUiScale,
  useSettingsStore,
} from '@/stores/settings'
import { uiScaleError, uiScaleMax, uiScaleSupported } from '@/modules/settings/uiScale'
import EditableRangeValue from '../ui/EditableRangeValue.vue'

const emit = defineEmits<{ back: [] }>()
const settings = useSettingsStore()
const { t } = useI18n()
</script>

<template>
  <div class="np-more-sub-header">
    <button class="np-more-back" @click="emit('back')">
      <span class="material-symbols-rounded">arrow_back</span>
    </button>
    <h4 class="np-more-title">{{ t('player.font_scale') }}</h4>
  </div>
  <div class="np-more-item">
    <p class="np-more-label">{{ t('settings.lyric_font_size') }}</p>
    <div class="np-more-row">
      <input type="range" :min="LYRIC_FONT_SCALE_MIN" :max="LYRIC_FONT_SCALE_MAX" :step="LYRIC_FONT_SCALE_STEP"
        :value="settings.lyricFontScale"
        class="np-more-slider"
        :aria-label="t('player.font_scale')"
        @input="settings.lyricFontScale = parseFloat(($event.target as HTMLInputElement).value)"
      />
      <EditableRangeValue
        v-model="settings.lyricFontScale"
        class="np-offset-value"
        :min="LYRIC_FONT_SCALE_MIN"
        :max="LYRIC_FONT_SCALE_MAX"
        :step="LYRIC_FONT_SCALE_STEP"
        :input-scale="100"
        :display-value="`${Math.round(settings.lyricFontScale * 100)}%`"
        input-suffix="%"
        :aria-label="t('player.font_scale')"
      />
    </div>
    <p class="np-more-preview" :style="{ fontSize: `${24 * settings.lyricFontScale}px` }">
      {{ t('player.font_preview') }}
    </p>
    <p class="np-more-preview" :style="{ fontSize: `${12 * settings.lyricFontScale * settings.lyricTranslationScale}px` }">
      {{ t('player.translation_font_preview') }}
    </p>
  </div>
  <div class="np-more-item">
    <p class="np-more-label">{{ t('settings.lyric_translation_size') }}</p>
    <div class="np-more-row">
      <input type="range" class="np-more-slider"
        v-model.number="settings.lyricTranslationScale"
        :min="LYRIC_TRANSLATION_SCALE_MIN" :max="LYRIC_TRANSLATION_SCALE_MAX" :step="LYRIC_TRANSLATION_SCALE_STEP"
        :aria-label="t('settings.lyric_translation_size')" />
      <EditableRangeValue
        v-model="settings.lyricTranslationScale" class="np-offset-value"
        :min="LYRIC_TRANSLATION_SCALE_MIN" :max="LYRIC_TRANSLATION_SCALE_MAX" :step="LYRIC_TRANSLATION_SCALE_STEP"
        :input-scale="100" input-suffix="%"
        :display-value="`${Math.round(settings.lyricTranslationScale * 100)}%`"
        :aria-label="t('settings.lyric_translation_size')" />
    </div>
    <p class="np-more-hint np-more-hint--below">{{ t('settings.lyric_translation_size_desc') }}</p>
    <button class="np-more-text-btn" :disabled="settings.lyricTranslationScale === LYRIC_TRANSLATION_SCALE_DEFAULT"
      @click="settings.lyricTranslationScale = LYRIC_TRANSLATION_SCALE_DEFAULT">
      <span class="material-symbols-rounded">restart_alt</span>
      {{ t('settings.ui_scale_reset') }}
    </button>
  </div>
  <div class="np-more-item">
    <p class="np-more-label">{{ t('settings.ui_scale') }}</p>
    <div class="np-more-row">
      <input type="range" class="np-more-slider" :value="settings.uiScale"
        :min="UI_SCALE_MIN" :max="uiScaleMax" :step="UI_SCALE_STEP"
        :disabled="!uiScaleSupported" :aria-label="t('settings.ui_scale')"
        @change="settings.uiScale = normalizeUiScale(Number(($event.target as HTMLInputElement).value))" />
      <span class="np-offset-value">{{ settings.uiScale }}%</span>
    </div>
    <p class="np-more-hint np-more-hint--below">{{ t('settings.ui_scale_desc') }}</p>
    <p v-if="uiScaleSupported" class="np-more-hint">{{ t('settings.ui_scale_limit', { value: uiScaleMax }) }}</p>
    <p v-if="uiScaleError" class="np-more-hint" role="alert">{{ t('settings.ui_scale_failed') }}</p>
    <p v-else-if="!uiScaleSupported" class="np-more-hint">{{ t(isTauri() ? 'settings.ui_scale_unsupported' : 'settings.ui_scale_preview') }}</p>
    <button class="np-more-text-btn" :disabled="!uiScaleSupported || settings.uiScale === UI_SCALE_DEFAULT"
      @click="settings.uiScale = UI_SCALE_DEFAULT">
      <span class="material-symbols-rounded">restart_alt</span>
      {{ t('settings.ui_scale_reset') }}
    </button>
  </div>
</template>
