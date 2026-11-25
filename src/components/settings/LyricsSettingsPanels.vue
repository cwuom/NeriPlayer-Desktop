<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import DesktopLyricsStage from '@/components/desktopLyrics/DesktopLyricsStage.vue'
import BiliSubtitleSettings from '@/components/bilibili/BiliSubtitleSettings.vue'
import EditableRangeValue from '@/components/ui/EditableRangeValue.vue'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import { desktopLyricsOpen } from '@/modules/desktopLyrics/bridge'
import {
  DESKTOP_LYRICS_ALIGNS,
  DESKTOP_LYRICS_BACKGROUND_OPACITY,
  DESKTOP_LYRICS_BACKGROUNDS,
  DESKTOP_LYRICS_FONT_SIZE,
  DESKTOP_LYRICS_MAX_WIDTH,
  DESKTOP_LYRICS_KARAOKE,
  DESKTOP_LYRICS_LAYOUTS,
  DESKTOP_LYRICS_LETTER_SPACING,
  DESKTOP_LYRICS_OPACITY,
  DESKTOP_LYRICS_SECONDARY_SCALE,
  DESKTOP_LYRICS_SHADOW_BLUR,
  DESKTOP_LYRICS_STROKE_WIDTH,
  DESKTOP_LYRICS_THEMES,
  type DesktopLyricsStyle,
} from '@/modules/desktopLyrics/style'
import { formatLyricOffsetMs } from '@/modules/lyrics/lyricOffset'
import { isMacPlatform } from '@/modules/shortcuts/platform'
import {
  LYRIC_DEFAULT_OFFSET_RANGE_MS,
  LYRIC_DEFAULT_OFFSET_STEP_MS,
  LYRIC_EMPHASIS_THRESHOLD_MAX_MS,
  LYRIC_EMPHASIS_THRESHOLD_MIN_MS,
  LYRIC_EMPHASIS_THRESHOLD_STEP_MS,
  LYRIC_FONT_SCALE_MAX,
  LYRIC_FONT_SCALE_MIN,
  LYRIC_FONT_SCALE_STEP,
  LYRIC_TRANSLATION_SCALE_MIN,
  LYRIC_TRANSLATION_SCALE_MAX,
  LYRIC_TRANSLATION_SCALE_STEP,
  normalizeLyricDefaultOffset,
  normalizeLyricEmphasisThresholdMs,
  useSettingsStore,
} from '@/stores/settings'
import { onExpandAfterEnter, onExpandAfterLeave, onExpandEnter, onExpandLeave } from './settingsShared'
import { CUSTOM_FONT, lyricOffsetSettings, type LyricsSettings } from './useLyricsSettings'
import type { SettingsSectionId } from './useSettingsSections'

// 歌词与桌面歌词两个分区面板。面板随分组切换整体重建，
// 需要跨切换保留的状态（折叠分组、桌面歌词忙碌 / 自定义字体等）由设置页传入
const props = defineProps<{
  activeSettingsSection: SettingsSectionId
  toggleSection: (key: string) => void
  isExpanded: (key: string) => boolean
  lyrics: LyricsSettings
}>()

const { t } = useI18n()
const settings = useSettingsStore()
const {
  lyricFontScale, lyricTranslationScale, showTranslation, showRomanization, preferWordTimedLyrics,
  defaultLyricSource, lyricEmphasisThresholdMs, lyricBlur, lyricBlurAmount, showMenuBarLyrics,
} = storeToRefs(settings)
const {
  defaultLyricSourceOptions, defaultLyricSourceDescription, changeDefaultLyricSource, lyricOffsetsChanged, resetLyricOffsets,
  desktopLyricsStyle, updateDesktopLyrics, setDesktopLyricsGradient, chooseDesktopLyricsTheme, resetDesktopLyrics,
  desktopLyricsBusy, toggleDesktopLyrics, desktopLyricsFontOptions, desktopLyricsFontChoice, chooseDesktopLyricsFont,
  desktopLyricsWeightOptions, desktopLyricsSecondaryOptions, themeSwatch, desktopLyricsAccent,
  desktopLyricsPreviewLines, desktopLyricsPreviewClock,
} = props.lyrics
</script>

<template>
  <!-- 歌词 -->
      <div v-show="activeSettingsSection === 'lyrics'" class="settings-section-panel" data-section="lyrics">
  <div class="section-label section-label--page clickable" @click="toggleSection('lyrics')">
    <span class="material-symbols-rounded" style="font-size: 18px">lyrics</span>
    <span>{{ t('settings.lyrics') }}</span>
    <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('lyrics') }">expand_more</span>
  </div>

    <div class="section-label">{{ t('settings.group_lyrics_display') }}</div>
    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">format_size</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.lyric_font_size') }}</div>
        <EditableRangeValue
          v-model="lyricFontScale"
          class="setting-desc"
          :min="LYRIC_FONT_SCALE_MIN"
          :max="LYRIC_FONT_SCALE_MAX"
          :step="LYRIC_FONT_SCALE_STEP"
          :display-value="`${Math.round(lyricFontScale * 100)}%`"
          :input-scale="100"
          input-suffix="%"
          :aria-label="t('settings.lyric_font_size')"
        />
      </div>
      <input type="range" class="m3-slider" v-model.number="lyricFontScale" :min="LYRIC_FONT_SCALE_MIN" :max="LYRIC_FONT_SCALE_MAX" :step="LYRIC_FONT_SCALE_STEP" />
    </div>


  <div class="setting-card">
    <div class="setting-icon-wrap"><span class="material-symbols-rounded">translate</span></div>
    <div class="setting-info">
      <div class="setting-title">{{ t('settings.lyric_translation_size') }}</div>
      <div class="setting-desc">{{ t('settings.lyric_translation_size_desc') }}</div>
      <EditableRangeValue
        v-model="lyricTranslationScale"
        class="setting-desc"
        :min="LYRIC_TRANSLATION_SCALE_MIN"
        :max="LYRIC_TRANSLATION_SCALE_MAX"
        :step="LYRIC_TRANSLATION_SCALE_STEP"
        :display-value="`${Math.round(lyricTranslationScale * 100)}%`"
        :input-scale="100"
        input-suffix="%"
        :aria-label="t('settings.lyric_translation_size')"
      />
    </div>
    <input type="range" class="m3-slider" v-model.number="lyricTranslationScale"
      :min="LYRIC_TRANSLATION_SCALE_MIN" :max="LYRIC_TRANSLATION_SCALE_MAX" :step="LYRIC_TRANSLATION_SCALE_STEP"
      :aria-label="t('settings.lyric_translation_size')" />
  </div>

  <div class="setting-card">
    <div class="setting-icon-wrap"><span class="material-symbols-rounded">translate</span></div>
    <div class="setting-info">
      <div class="setting-title">{{ t('settings.show_translation') }}</div>
      <div class="setting-desc">{{ t('settings.show_translation_desc') }}</div>
    </div>
    <label class="m3-switch"><input type="checkbox" v-model="showTranslation" /><span class="track"><span class="thumb"><span v-if="showTranslation" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
  </div>

  <div class="setting-card">
    <div class="setting-icon-wrap"><span class="material-symbols-rounded">abc</span></div>
    <div class="setting-info">
      <div class="setting-title">{{ t('settings.show_romanization') }}</div>
      <div class="setting-desc">{{ t('settings.show_romanization_desc') }}</div>
    </div>
    <label class="m3-switch"><input type="checkbox" v-model="showRomanization" /><span class="track"><span class="thumb"><span v-if="showRomanization" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
  </div>

  <!-- 来源偏好（对齐 Android SettingsLyricSourceSection） -->
  <div class="setting-card">
    <div class="setting-icon-wrap"><span class="material-symbols-rounded">timer</span></div>
    <div class="setting-info">
      <div class="setting-title">{{ t('settings.prefer_word_timed_lyrics') }}</div>
      <div class="setting-desc">{{ t('settings.prefer_word_timed_lyrics_desc') }}</div>
    </div>
    <label class="m3-switch"><input type="checkbox" v-model="preferWordTimedLyrics" /><span class="track"><span class="thumb"><span v-if="preferWordTimedLyrics" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
  </div>

  <div class="section-label">{{ t('settings.group_lyrics_source') }}</div>
  <div class="setting-card setting-card--select">
    <div class="setting-icon-wrap"><span class="material-symbols-rounded">source</span></div>
    <div class="setting-info">
      <div class="setting-title">{{ t('settings.default_lyric_source') }}</div>
      <div class="setting-desc">{{ t('settings.default_lyric_source_desc') }}</div>
      <div class="setting-desc">{{ defaultLyricSourceDescription }}</div>
    </div>
    <CustomSelect class="settings-select" :model-value="defaultLyricSource" :options="defaultLyricSourceOptions"
      :label="t('settings.default_lyric_source')" @update:model-value="changeDefaultLyricSource" />
  </div>

  <BiliSubtitleSettings variant="settings" :show-offset="false" />

  <div class="section-label">{{ t('settings.group_lyrics_effects') }}</div>
  <div class="setting-card">
    <div class="setting-icon-wrap"><span class="material-symbols-rounded">graphic_eq</span></div>
    <div class="setting-info">
      <div class="setting-title">{{ t('settings.lyric_emphasis_threshold') }}</div>
      <div class="setting-desc">{{ t('settings.lyric_emphasis_threshold_desc') }}</div>
      <EditableRangeValue
        :model-value="lyricEmphasisThresholdMs"
        class="setting-desc"
        :min="LYRIC_EMPHASIS_THRESHOLD_MIN_MS"
        :max="LYRIC_EMPHASIS_THRESHOLD_MAX_MS"
        :step="LYRIC_EMPHASIS_THRESHOLD_STEP_MS"
        :display-value="`${lyricEmphasisThresholdMs}ms`"
        input-suffix="ms"
        :aria-label="t('settings.lyric_emphasis_threshold')"
        @update:model-value="lyricEmphasisThresholdMs = normalizeLyricEmphasisThresholdMs($event)"
      />
    </div>
    <input type="range" class="m3-slider" :value="lyricEmphasisThresholdMs"
      :min="LYRIC_EMPHASIS_THRESHOLD_MIN_MS" :max="LYRIC_EMPHASIS_THRESHOLD_MAX_MS" :step="LYRIC_EMPHASIS_THRESHOLD_STEP_MS"
      :aria-label="t('settings.lyric_emphasis_threshold')"
      @input="lyricEmphasisThresholdMs = normalizeLyricEmphasisThresholdMs(Number(($event.target as HTMLInputElement).value))" />
  </div>

  <div class="setting-card">
    <div class="setting-icon-wrap"><span class="material-symbols-rounded">blur_on</span></div>
    <div class="setting-info">
      <div class="setting-title">{{ t('settings.lyric_blur') }}</div>
      <div class="setting-desc">{{ t('settings.lyric_blur_desc') }}</div>
    </div>
    <label class="m3-switch"><input type="checkbox" v-model="lyricBlur" /><span class="track"><span class="thumb"><span v-if="lyricBlur" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
  </div>

  <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('lyrics')">
    <div v-if="lyricBlur" class="setting-card sub-card">
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.blur_strength') }}</div>
        <EditableRangeValue
          v-model="lyricBlurAmount"
          class="setting-desc"
          :min="0"
          :max="8"
          :step="0.5"
          :display-value="`${lyricBlurAmount.toFixed(1)}px`"
          input-suffix="px"
          :aria-label="t('settings.blur_strength')"
        />
      </div>
      <input type="range" class="m3-slider" v-model.number="lyricBlurAmount" min="0" max="8" step="0.5" />
    </div>

    <!-- 各歌词来源的默认偏移（对齐 Android）：显示的就是这类歌词实际生效的偏移 -->
    <div v-for="item in lyricOffsetSettings" :key="item.key" class="setting-card">
      <div class="setting-icon-wrap"><span class="platform-icon" :style="{ maskImage: `url(${item.iconSvg})` }"></span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t(item.label) }}</div>
        <EditableRangeValue
          :model-value="settings[item.key]"
          class="setting-desc"
          :min="-LYRIC_DEFAULT_OFFSET_RANGE_MS"
          :max="LYRIC_DEFAULT_OFFSET_RANGE_MS"
          :step="LYRIC_DEFAULT_OFFSET_STEP_MS"
          :display-value="formatLyricOffsetMs(settings[item.key])"
          input-suffix="ms"
          :aria-label="t(item.label)"
          @update:model-value="settings[item.key] = normalizeLyricDefaultOffset($event, item.defaultMs)"
        />
      </div>
      <button
        v-if="settings[item.key] !== item.defaultMs"
        class="m3-chip sm"
        :title="t('settings.lyric_offset_reset_to', { value: formatLyricOffsetMs(item.defaultMs) })"
        @click="settings[item.key] = item.defaultMs"
      >{{ t('settings.lyric_offset_reset') }}</button>
      <input
        type="range" class="m3-slider"
        :value="settings[item.key]"
        :min="-LYRIC_DEFAULT_OFFSET_RANGE_MS" :max="LYRIC_DEFAULT_OFFSET_RANGE_MS" :step="LYRIC_DEFAULT_OFFSET_STEP_MS"
        :aria-label="t(item.label)"
        @input="settings[item.key] = normalizeLyricDefaultOffset(Number(($event.target as HTMLInputElement).value), item.defaultMs)"
      />
    </div>
    <div v-if="lyricOffsetsChanged" class="setting-card sub-card">
      <div class="setting-info">
        <div class="setting-desc">{{ t('settings.lyric_offset_reset_all_desc') }}</div>
      </div>
      <button class="m3-chip sm" @click="resetLyricOffsets">{{ t('settings.lyric_offset_reset_all') }}</button>
    </div>
  </div></Transition>

      </div>

  <!-- 桌面歌词 -->
      <div v-show="activeSettingsSection === 'desktop_lyrics'" class="settings-section-panel" data-section="desktop_lyrics">
  <div id="settings-desktop-lyrics" class="section-label section-label--page clickable" @click="toggleSection('desktop_lyrics')">
    <span class="material-symbols-rounded" style="font-size: 18px">subtitles</span>
    <span>{{ t('desktop_lyrics.title') }}</span>
    <span class="material-symbols-rounded section-arrow" :class="{ expanded: isExpanded('desktop_lyrics') }">expand_more</span>
  </div>

  <div class="section-label">{{ t('settings.group_dl_window') }}</div>
  <div class="setting-card">
    <div class="setting-icon-wrap"><span class="material-symbols-rounded">desktop_windows</span></div>
    <div class="setting-info">
      <div class="setting-title">{{ t('desktop_lyrics.open') }}</div>
      <div class="setting-desc">{{ t('desktop_lyrics.open_desc') }}</div>
    </div>
    <button
      v-if="desktopLyricsOpen"
      class="m3-chip sm"
      :class="{ active: desktopLyricsStyle.locked }"
      @click="updateDesktopLyrics({ locked: !desktopLyricsStyle.locked })"
    >
      {{ t(desktopLyricsStyle.locked ? 'desktop_lyrics.unlock' : 'desktop_lyrics.lock') }}
    </button>
    <label class="m3-switch"><input type="checkbox" :checked="desktopLyricsOpen" :disabled="desktopLyricsBusy" @change="toggleDesktopLyrics(($event.target as HTMLInputElement).checked)" /><span class="track"><span class="thumb"><span v-if="desktopLyricsOpen" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
  </div>

  <div v-if="isMacPlatform" class="setting-card">
    <div class="setting-icon-wrap"><span class="material-symbols-rounded">lyrics</span></div>
    <div class="setting-info">
      <div class="setting-title">{{ t('desktop_lyrics.menu_bar') }}</div>
      <div class="setting-desc">{{ t('desktop_lyrics.menu_bar_desc') }}</div>
    </div>
    <label class="m3-switch"><input type="checkbox" v-model="showMenuBarLyrics" :aria-label="t('desktop_lyrics.menu_bar')" /><span class="track"><span class="thumb"><span v-if="showMenuBarLyrics" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
  </div>

  <Transition @enter="onExpandEnter" @after-enter="onExpandAfterEnter" @leave="onExpandLeave" @after-leave="onExpandAfterLeave"><div v-if="isExpanded('desktop_lyrics')">
    <div class="setting-card dl-preview-card">
      <div class="dl-preview" :aria-label="t('desktop_lyrics.preview')">
        <DesktopLyricsStage
          :lines="desktopLyricsPreviewLines"
          :first-index="0"
          :clock="desktopLyricsPreviewClock"
          :style="desktopLyricsStyle"
          :accent="desktopLyricsAccent"
          :placeholder="t('desktop_lyrics.preview')"
        />
      </div>
    </div>

    <div class="section-label">{{ t('settings.group_dl_style') }}</div>
    <div class="setting-card dl-stack-card">
      <div class="setting-title">{{ t('desktop_lyrics.theme') }}</div>
      <div class="dl-swatches">
        <button
          v-for="theme in [...DESKTOP_LYRICS_THEMES.map(item => item.id), 'accent']"
          :key="theme"
          class="dl-swatch"
          :class="{ active: desktopLyricsStyle.theme === theme }"
          :title="t(`desktop_lyrics.theme_${theme}`)"
          @click="chooseDesktopLyricsTheme(theme)"
        >
          <span class="dl-swatch-dot" :style="{ background: themeSwatch(theme) }"></span>
          <span>{{ t(`desktop_lyrics.theme_${theme}`) }}</span>
        </button>
        <span v-if="desktopLyricsStyle.theme === 'custom'" class="m3-chip sm active">{{ t('desktop_lyrics.theme_custom') }}</span>
      </div>
    </div>

    <div class="setting-card dl-stack-card">
      <div class="setting-title">{{ t('desktop_lyrics.layout') }}</div>
      <div class="dl-chips">
        <button v-for="layout in DESKTOP_LYRICS_LAYOUTS" :key="layout" class="m3-chip sm" :class="{ active: desktopLyricsStyle.layout === layout }" @click="updateDesktopLyrics({ layout })">
          {{ t(`desktop_lyrics.layout_${layout}`) }}
        </button>
      </div>
      <div v-if="desktopLyricsStyle.layout === 'double'" class="setting-desc">{{ t('desktop_lyrics.layout_double_desc') }}</div>
    </div>

    <div class="setting-card dl-stack-card">
      <div class="setting-title">{{ t('desktop_lyrics.align') }}</div>
      <div class="dl-chips">
        <button v-for="align in DESKTOP_LYRICS_ALIGNS" :key="align" class="m3-chip sm" :class="{ active: desktopLyricsStyle.align === align }" @click="updateDesktopLyrics({ align })">
          {{ t(`desktop_lyrics.align_${align}`) }}
        </button>
      </div>
    </div>

    <div class="setting-card dl-stack-card">
      <div class="setting-title">{{ t('desktop_lyrics.karaoke') }}</div>
      <div class="dl-chips">
        <button v-for="mode in DESKTOP_LYRICS_KARAOKE" :key="mode" class="m3-chip sm" :class="{ active: desktopLyricsStyle.karaoke === mode }" @click="updateDesktopLyrics({ karaoke: mode })">
          {{ t(`desktop_lyrics.karaoke_${mode}`) }}
        </button>
      </div>
    </div>

    <div class="setting-card setting-card--select">
      <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.secondary') }}</div></div>
      <CustomSelect class="settings-select" :model-value="desktopLyricsStyle.secondary" :options="desktopLyricsSecondaryOptions"
        :label="t('desktop_lyrics.secondary')" :disabled="desktopLyricsStyle.layout === 'double'"
        @update:model-value="updateDesktopLyrics({ secondary: $event as DesktopLyricsStyle['secondary'] })" />
    </div>

    <div class="section-label">{{ t('settings.group_dl_text') }}</div>
    <div class="setting-card setting-card--select">
      <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.font') }}</div></div>
      <CustomSelect class="settings-select" :model-value="desktopLyricsFontChoice" :options="desktopLyricsFontOptions"
        :label="t('desktop_lyrics.font')" @update:model-value="chooseDesktopLyricsFont" />
    </div>
    <div v-if="desktopLyricsFontChoice === CUSTOM_FONT" class="setting-card sub-card">
      <input
        class="lt-url-input lt-input-left dl-font-input"
        :value="desktopLyricsStyle.fontFamily"
        :placeholder="t('desktop_lyrics.font_custom_placeholder')"
        maxlength="64"
        @change="updateDesktopLyrics({ fontFamily: ($event.target as HTMLInputElement).value })"
      />
    </div>

    <div class="setting-card sub-card">
      <div class="setting-info">
        <div class="setting-title">{{ t('desktop_lyrics.font_size') }}</div>
        <div class="setting-desc">{{ desktopLyricsStyle.fontSize }}px</div>
      </div>
      <input type="range" class="m3-slider" :value="desktopLyricsStyle.fontSize" :min="DESKTOP_LYRICS_FONT_SIZE.min" :max="DESKTOP_LYRICS_FONT_SIZE.max" :step="DESKTOP_LYRICS_FONT_SIZE.step"
        :aria-label="t('desktop_lyrics.font_size')" @input="updateDesktopLyrics({ fontSize: Number(($event.target as HTMLInputElement).value) })" />
    </div>

    <div class="setting-card sub-card">
      <div class="setting-info">
        <div class="setting-title">{{ t('desktop_lyrics.max_width') }}</div>
        <div class="setting-desc">{{ desktopLyricsStyle.maxWidth }}px · {{ t('desktop_lyrics.max_width_desc') }}</div>
      </div>
      <input type="range" class="m3-slider" :value="desktopLyricsStyle.maxWidth" :min="DESKTOP_LYRICS_MAX_WIDTH.min" :max="DESKTOP_LYRICS_MAX_WIDTH.max" :step="DESKTOP_LYRICS_MAX_WIDTH.step"
        :aria-label="t('desktop_lyrics.max_width')" @input="updateDesktopLyrics({ maxWidth: Number(($event.target as HTMLInputElement).value) })" />
    </div>

    <div class="setting-card setting-card--select sub-card">
      <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.font_weight') }}</div></div>
      <CustomSelect class="settings-select" :model-value="String(desktopLyricsStyle.fontWeight)" :options="desktopLyricsWeightOptions"
        :label="t('desktop_lyrics.font_weight')" @update:model-value="updateDesktopLyrics({ fontWeight: Number($event) })" />
    </div>

    <div class="setting-card sub-card">
      <div class="setting-info">
        <div class="setting-title">{{ t('desktop_lyrics.letter_spacing') }}</div>
        <div class="setting-desc">{{ desktopLyricsStyle.letterSpacing }}px</div>
      </div>
      <input type="range" class="m3-slider" :value="desktopLyricsStyle.letterSpacing" :min="DESKTOP_LYRICS_LETTER_SPACING.min" :max="DESKTOP_LYRICS_LETTER_SPACING.max" :step="DESKTOP_LYRICS_LETTER_SPACING.step"
        :aria-label="t('desktop_lyrics.letter_spacing')" @input="updateDesktopLyrics({ letterSpacing: Number(($event.target as HTMLInputElement).value) })" />
    </div>

    <div class="setting-card sub-card">
      <div class="setting-info">
        <div class="setting-title">{{ t('desktop_lyrics.secondary_size') }}</div>
        <div class="setting-desc">{{ Math.round(desktopLyricsStyle.secondaryScale * 100) }}%</div>
      </div>
      <input type="range" class="m3-slider" :value="desktopLyricsStyle.secondaryScale" :min="DESKTOP_LYRICS_SECONDARY_SCALE.min" :max="DESKTOP_LYRICS_SECONDARY_SCALE.max" :step="DESKTOP_LYRICS_SECONDARY_SCALE.step"
        :aria-label="t('desktop_lyrics.secondary_size')" @input="updateDesktopLyrics({ secondaryScale: Number(($event.target as HTMLInputElement).value) })" />
    </div>

    <div class="setting-card dl-stack-card">
      <div class="setting-desc">{{ t('desktop_lyrics.gradient_hint') }}</div>
      <div class="dl-color-rows">
        <label class="dl-color-row">
          <span>{{ t('desktop_lyrics.played') }}</span>
          <input type="color" :value="desktopLyricsStyle.playedColors[0]" @input="setDesktopLyricsGradient('playedColors', 0, ($event.target as HTMLInputElement).value)" />
          <input type="color" :value="desktopLyricsStyle.playedColors[1]" @input="setDesktopLyricsGradient('playedColors', 1, ($event.target as HTMLInputElement).value)" />
        </label>
        <label class="dl-color-row">
          <span>{{ t('desktop_lyrics.unplayed') }}</span>
          <input type="color" :value="desktopLyricsStyle.unplayedColors[0]" @input="setDesktopLyricsGradient('unplayedColors', 0, ($event.target as HTMLInputElement).value)" />
          <input type="color" :value="desktopLyricsStyle.unplayedColors[1]" @input="setDesktopLyricsGradient('unplayedColors', 1, ($event.target as HTMLInputElement).value)" />
        </label>
        <label class="dl-color-row">
          <span>{{ t('desktop_lyrics.secondary_color') }}</span>
          <input type="color" :value="desktopLyricsStyle.secondaryColor" @input="updateDesktopLyrics({ secondaryColor: ($event.target as HTMLInputElement).value, theme: 'custom' })" />
        </label>
      </div>
    </div>

    <div class="section-label">{{ t('settings.group_lyrics_effects') }}</div>
    <div class="setting-card">
      <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.stroke') }}</div></div>
      <input v-if="desktopLyricsStyle.strokeEnabled" type="color" class="dl-color-inline" :value="desktopLyricsStyle.strokeColor" :aria-label="t('desktop_lyrics.stroke')"
        @input="updateDesktopLyrics({ strokeColor: ($event.target as HTMLInputElement).value })" />
      <label class="m3-switch"><input type="checkbox" :checked="desktopLyricsStyle.strokeEnabled" @change="updateDesktopLyrics({ strokeEnabled: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"><span v-if="desktopLyricsStyle.strokeEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>
    <div v-if="desktopLyricsStyle.strokeEnabled" class="setting-card sub-card">
      <div class="setting-info">
        <div class="setting-title">{{ t('desktop_lyrics.stroke_width') }}</div>
        <div class="setting-desc">{{ desktopLyricsStyle.strokeWidth }}px</div>
      </div>
      <input type="range" class="m3-slider" :value="desktopLyricsStyle.strokeWidth" :min="DESKTOP_LYRICS_STROKE_WIDTH.min" :max="DESKTOP_LYRICS_STROKE_WIDTH.max" :step="DESKTOP_LYRICS_STROKE_WIDTH.step"
        :aria-label="t('desktop_lyrics.stroke_width')" @input="updateDesktopLyrics({ strokeWidth: Number(($event.target as HTMLInputElement).value) })" />
    </div>

    <div class="setting-card">
      <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.shadow') }}</div></div>
      <input v-if="desktopLyricsStyle.shadowEnabled" type="color" class="dl-color-inline" :value="desktopLyricsStyle.shadowColor" :aria-label="t('desktop_lyrics.shadow')"
        @input="updateDesktopLyrics({ shadowColor: ($event.target as HTMLInputElement).value })" />
      <label class="m3-switch"><input type="checkbox" :checked="desktopLyricsStyle.shadowEnabled" @change="updateDesktopLyrics({ shadowEnabled: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"><span v-if="desktopLyricsStyle.shadowEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>
    <div v-if="desktopLyricsStyle.shadowEnabled" class="setting-card sub-card">
      <div class="setting-info">
        <div class="setting-title">{{ t('desktop_lyrics.shadow_blur') }}</div>
        <div class="setting-desc">{{ desktopLyricsStyle.shadowBlur }}px</div>
      </div>
      <input type="range" class="m3-slider" :value="desktopLyricsStyle.shadowBlur" :min="DESKTOP_LYRICS_SHADOW_BLUR.min" :max="DESKTOP_LYRICS_SHADOW_BLUR.max" :step="DESKTOP_LYRICS_SHADOW_BLUR.step"
        :aria-label="t('desktop_lyrics.shadow_blur')" @input="updateDesktopLyrics({ shadowBlur: Number(($event.target as HTMLInputElement).value) })" />
    </div>

    <div class="setting-card">
      <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.glow') }}</div></div>
      <label class="m3-switch"><input type="checkbox" :checked="desktopLyricsStyle.glowEnabled" @change="updateDesktopLyrics({ glowEnabled: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"><span v-if="desktopLyricsStyle.glowEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <div class="setting-card">
      <div class="setting-info">
        <div class="setting-title">{{ t('desktop_lyrics.opacity') }}</div>
        <div class="setting-desc">{{ Math.round(desktopLyricsStyle.opacity * 100) }}%</div>
      </div>
      <input type="range" class="m3-slider" :value="desktopLyricsStyle.opacity" :min="DESKTOP_LYRICS_OPACITY.min" :max="DESKTOP_LYRICS_OPACITY.max" :step="DESKTOP_LYRICS_OPACITY.step"
        :aria-label="t('desktop_lyrics.opacity')" @input="updateDesktopLyrics({ opacity: Number(($event.target as HTMLInputElement).value) })" />
    </div>

    <div class="setting-card dl-stack-card">
      <div class="setting-title">{{ t('desktop_lyrics.background') }}</div>
      <div class="dl-chips">
        <button v-for="mode in DESKTOP_LYRICS_BACKGROUNDS" :key="mode" class="m3-chip sm" :class="{ active: desktopLyricsStyle.background === mode }" @click="updateDesktopLyrics({ background: mode })">
          {{ t(`desktop_lyrics.background_${mode}`) }}
        </button>
      </div>
    </div>
    <div v-if="desktopLyricsStyle.background !== 'never'" class="setting-card sub-card">
      <div class="setting-info">
        <div class="setting-title">{{ t('desktop_lyrics.background_opacity') }}</div>
        <div class="setting-desc">{{ Math.round(desktopLyricsStyle.backgroundOpacity * 100) }}%</div>
      </div>
      <input type="range" class="m3-slider" :value="desktopLyricsStyle.backgroundOpacity" :min="DESKTOP_LYRICS_BACKGROUND_OPACITY.min" :max="DESKTOP_LYRICS_BACKGROUND_OPACITY.max" :step="DESKTOP_LYRICS_BACKGROUND_OPACITY.step"
        :aria-label="t('desktop_lyrics.background_opacity')" @input="updateDesktopLyrics({ backgroundOpacity: Number(($event.target as HTMLInputElement).value) })" />
    </div>

    <div class="setting-card">
      <div class="setting-info"><div class="setting-title">{{ t('desktop_lyrics.show_track_info') }}</div></div>
      <label class="m3-switch"><input type="checkbox" :checked="desktopLyricsStyle.showTrackInfo" @change="updateDesktopLyrics({ showTrackInfo: ($event.target as HTMLInputElement).checked })" /><span class="track"><span class="thumb"><span v-if="desktopLyricsStyle.showTrackInfo" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span></label>
    </div>

    <div class="setting-card sub-card">
      <div class="setting-info">
        <div class="setting-title">{{ t('desktop_lyrics.reset') }}</div>
        <div class="setting-desc">{{ t('desktop_lyrics.reset_desc') }}</div>
      </div>
      <button class="m3-chip sm" @click="resetDesktopLyrics">{{ t('desktop_lyrics.reset') }}</button>
    </div>
  </div></Transition>
      </div>
</template>

<style scoped lang="scss">
@use '@/components/settings/settingsPrimitives.scss' as primitives;

@include primitives.select-card;

@include primitives.section-panel;

@include primitives.panel-first-label;

/* 桌面歌词设置 */
.dl-preview {
  width: 100%;
  height: 156px;
  padding: 12px 20px;
  box-sizing: border-box;
  border-radius: inherit;
  /* 模拟桌面：歌词实际叠在各种壁纸上，深浅交错更容易看出描边和阴影 */
  background:
    radial-gradient(circle at 20% 30%, rgb(122 160 255 / 55%), transparent 55%),
    radial-gradient(circle at 80% 70%, rgb(255 180 120 / 45%), transparent 55%),
    linear-gradient(135deg, #2a3350, #4b3a4f 60%, #a7b4c8);
  color: #fff;
}

/* 比 .setting-card 多一个类：基础规则写在后面，同等优先级会把这里的对齐盖掉 */
.setting-card.dl-stack-card {
  flex-direction: column;
  align-items: stretch;
  gap: 10px;
  text-align: left;
}

.setting-card.dl-preview-card {
  padding: 0;
  overflow: hidden;
}

.dl-chips,
.dl-swatches {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-start;
  gap: 6px;
}

.dl-swatch {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px 3px 3px;
  border-radius: var(--radius-full);
  border: 1px solid var(--hairline);
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface-variant);
  font-size: var(--fs-12);
  font-family: inherit;
  cursor: pointer;
  transition: border-color var(--duration-short), color var(--duration-short);

  &:hover { color: var(--md-on-surface); }
  &.active {
    border-color: var(--md-primary);
    color: var(--md-on-surface);
    box-shadow: inset 0 0 0 1px var(--md-primary);
  }
}

.dl-swatch-dot {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  box-shadow: inset 0 0 0 1px rgb(0 0 0 / 14%);
}

.dl-color-rows {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.dl-color-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-13);
  color: var(--md-on-surface);

  span { flex: 1; }
}

.dl-color-row input[type='color'],
.dl-color-inline {
  width: 38px;
  height: 26px;
  padding: 0;
  border: 1px solid var(--hairline);
  border-radius: 8px;
  background: none;
  cursor: pointer;
  flex-shrink: 0;
}

.dl-font-input {
  width: 100%;
}

@include primitives.card;

@include primitives.platform-icon;

@include primitives.card-body;

@include primitives.card-text;

@include primitives.collapsible;

@include primitives.chip;

@include primitives.input-slider;

@media (max-width: 480px) {
  @include primitives.card-narrow;
}

/* 页面级标题与右侧页头重复，隐藏（分区由普通 .section-label 承担） */
.section-label--page { display: none; }
</style>
