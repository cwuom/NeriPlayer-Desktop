<script setup lang="ts">
import { computed } from 'vue'
import { storeToRefs } from 'pinia'
import { useI18n } from 'vue-i18n'
import { useSettingsStore, type ColorMode } from '@/stores/settings'
import SegmentedControl from '@/components/ui/SegmentedControl.vue'
import { switchThemeWithRipple, type ThemeMode } from '@/utils/theme'
import { THEME_COLORS, switchThemeColorWithRipple } from '@/utils/themeColor'
import {
  BACKGROUND_TINT_PRESETS,
  BACKGROUND_TINT_STRENGTH_DEFAULT,
  clampTintStrength,
  normalizeBackgroundTint,
  rgb,
} from '@/utils/theme/cleanScheme'
import { normalizeTextScale, TEXT_SCALE_DEFAULT, TEXT_SCALE_MAX, TEXT_SCALE_MIN, TEXT_SCALE_STEP } from '@/modules/settings/textScale'

// 外观主题：明暗、强调色（及来源）、背景偏色与文字大小。强调色只落在交互元素上，
// 背景偏色决定表面带多少颜色，二者分开调节
const { t } = useI18n()
const {
  darkMode, themeColor, colorMode, backgroundTint, backgroundTintStrength, textScale,
} = storeToRefs(useSettingsStore())

const themeModeOptions = computed<{ value: ThemeMode; label: string; icon: string }[]>(() => [
  { value: 'system', label: t('settings.theme_mode_system'), icon: 'brightness_auto' },
  { value: 'light', label: t('settings.theme_mode_light'), icon: 'light_mode' },
  { value: 'dark', label: t('settings.theme_mode_dark'), icon: 'dark_mode' },
])

const colorModeOptions = computed<{ value: ColorMode; label: string }[]>(() => [
  { value: 'default', label: t('settings.color_mode_default') },
  { value: 'cover', label: t('settings.color_mode_cover') },
  { value: 'system', label: t('settings.color_mode_system') },
])

const colorModeDesc = computed(() => t(`settings.color_mode_${colorMode.value}_desc`))

const accentSwatches = THEME_COLORS.map(scheme => ({
  key: scheme.key,
  light: rgb(scheme.light),
  dark: rgb(scheme.dark),
}))

function centerOf(event: MouseEvent | KeyboardEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }
}

function onThemeMode(mode: ThemeMode, event: MouseEvent | KeyboardEvent) {
  const { x, y } = centerOf(event)
  document.documentElement.classList.add('theme-ripple-active')
  void switchThemeWithRipple(mode, x, y, false)
}

function selectAccent(key: string, event: MouseEvent) {
  if (themeColor.value === key) return
  themeColor.value = key
  const { x, y } = centerOf(event)
  void switchThemeColorWithRipple(key, x, y, false)
}

const tintPresets = BACKGROUND_TINT_PRESETS
const activeTint = computed(() => normalizeBackgroundTint(backgroundTint.value))
const isCustomTint = computed(() => !tintPresets.some(preset => preset.tint === activeTint.value))
const customTintValue = computed(() => (activeTint.value.startsWith('#') ? activeTint.value : '#8a7cf0'))

function swatchStyle(tint: string) {
  if (tint === 'accent') return { background: 'var(--md-primary)' }
  if (tint === 'neutral') return { background: 'linear-gradient(135deg, #f2f2f5 0 50%, #2a2a2f 50% 100%)' }
  return { background: tint }
}

function selectTint(tint: string) {
  backgroundTint.value = tint
  // 从「中性」切回有色调时强度为 0 看不出效果，给一个默认强度
  if (tint !== 'neutral' && backgroundTintStrength.value === 0) backgroundTintStrength.value = BACKGROUND_TINT_STRENGTH_DEFAULT
}

function onCustomTint(event: Event) {
  selectTint(normalizeBackgroundTint((event.target as HTMLInputElement).value))
}

const tintDisabled = computed(() => activeTint.value === 'neutral')

function onTintStrength(event: Event) {
  backgroundTintStrength.value = clampTintStrength(Number((event.target as HTMLInputElement).value))
}

function onTextScale(event: Event) {
  textScale.value = normalizeTextScale(Number((event.target as HTMLInputElement).value))
}
</script>

<template>
  <div class="theme-appearance">
    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">contrast</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('settings.theme_mode') }}</div>
      </div>
      <SegmentedControl
        v-model="darkMode"
        class="theme-mode-control"
        :options="themeModeOptions"
        :label="t('settings.theme_mode')"
        @select="onThemeMode"
      />
    </div>

    <div class="setting-card setting-card--stack">
      <div class="card-heading">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">palette</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.accent_color') }}</div>
          <div class="setting-desc">{{ colorMode === 'default' ? t('settings.accent_color_desc') : colorModeDesc }}</div>
        </div>
        <SegmentedControl v-model="colorMode" :options="colorModeOptions" :label="t('settings.accent_source')" />
      </div>
      <div class="swatch-row" :class="{ muted: colorMode !== 'default' }" role="radiogroup" :aria-label="t('settings.accent_color')">
        <button
          v-for="swatch in accentSwatches"
          :key="swatch.key"
          type="button"
          class="swatch accent-swatch"
          :class="{ selected: themeColor === swatch.key }"
          :style="{ '--swatch-light': swatch.light, '--swatch-dark': swatch.dark }"
          role="radio"
          :aria-checked="themeColor === swatch.key"
          :aria-label="swatch.key"
          :title="swatch.key"
          :disabled="colorMode !== 'default'"
          @click="selectAccent(swatch.key, $event)"
        />
      </div>
    </div>

    <div class="setting-card setting-card--stack">
      <div class="card-heading">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">format_color_fill</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.background_tint') }}</div>
          <div class="setting-desc">{{ t('settings.background_tint_desc') }}</div>
        </div>
      </div>
      <div class="swatch-row" role="radiogroup" :aria-label="t('settings.background_tint')">
        <button
          v-for="preset in tintPresets"
          :key="preset.key"
          type="button"
          class="swatch tint-swatch"
          :class="{ selected: activeTint === preset.tint }"
          :style="swatchStyle(preset.tint)"
          role="radio"
          :aria-checked="activeTint === preset.tint"
          :aria-label="t(`settings.background_tint_${preset.key}`)"
          :title="t(`settings.background_tint_${preset.key}`)"
          @click="selectTint(preset.tint)"
        >
          <span v-if="preset.tint === 'accent'" class="material-symbols-rounded swatch-glyph" aria-hidden="true">auto_awesome</span>
        </button>
        <label
          class="swatch tint-swatch custom-swatch"
          :class="{ selected: isCustomTint }"
          :style="isCustomTint ? { background: customTintValue } : undefined"
          :title="t('settings.background_tint_custom')"
        >
          <span class="material-symbols-rounded swatch-glyph" aria-hidden="true">colorize</span>
          <input type="color" :value="customTintValue" :aria-label="t('settings.background_tint_custom')" @input="onCustomTint" />
        </label>
      </div>
      <div class="slider-row" :class="{ muted: tintDisabled }">
        <span class="slider-label">{{ t('settings.background_tint_strength') }}</span>
        <input
          type="range"
          class="m3-slider"
          min="0"
          max="100"
          step="1"
          :value="backgroundTintStrength"
          :disabled="tintDisabled"
          :aria-label="t('settings.background_tint_strength')"
          @input="onTintStrength"
        />
        <span class="slider-value">{{ backgroundTintStrength }}</span>
      </div>
    </div>

    <div class="setting-card setting-card--stack">
      <div class="card-heading">
        <div class="setting-icon-wrap"><span class="material-symbols-rounded">format_size</span></div>
        <div class="setting-info">
          <div class="setting-title">{{ t('settings.text_size') }}</div>
          <div class="setting-desc">{{ t('settings.text_size_desc') }}</div>
        </div>
        <button
          type="button"
          class="m3-chip sm"
          :disabled="textScale === TEXT_SCALE_DEFAULT"
          @click="textScale = TEXT_SCALE_DEFAULT"
        >{{ t('settings.text_size_reset') }}</button>
      </div>
      <div class="slider-row">
        <span class="material-symbols-rounded slider-glyph small" aria-hidden="true">text_fields</span>
        <input
          type="range"
          class="m3-slider"
          :min="TEXT_SCALE_MIN"
          :max="TEXT_SCALE_MAX"
          :step="TEXT_SCALE_STEP"
          :value="textScale"
          :aria-label="t('settings.text_size')"
          @input="onTextScale"
        />
        <span class="material-symbols-rounded slider-glyph" aria-hidden="true">text_fields</span>
        <span class="slider-value">{{ textScale }}%</span>
      </div>
      <p class="text-preview">{{ t('settings.text_size_preview') }}</p>
    </div>
  </div>
</template>

<style scoped lang="scss">
@use '@/components/settings/settingsPrimitives.scss' as primitives;

@include primitives.card;
@include primitives.card-body;
@include primitives.card-text;
@include primitives.chip;

.setting-card--stack {
  flex-direction: column;
  align-items: stretch;
  gap: 14px;
}

.card-heading {
  display: flex;
  align-items: center;
  gap: 14px;
  flex-wrap: wrap;

  .setting-info { flex: 1 1 220px; }
}

.theme-mode-control { flex-shrink: 0; }

.swatch-row {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  padding-left: 50px;
  transition: opacity var(--duration-short);

  &.muted { opacity: 0.4; }
}

.swatch {
  position: relative;
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: 50%;
  box-shadow: inset 0 0 0 1px rgb(0 0 0 / 0.1);
  cursor: pointer;
  transition: transform var(--duration-short) var(--ease-standard), box-shadow var(--duration-short);

  &:hover:not(:disabled) { transform: scale(1.08); }
  &:disabled { cursor: default; }

  /* 选中环：外圈留出底色间隙，再套一圈墨色描边 */
  &.selected {
    box-shadow: inset 0 0 0 1px rgb(0 0 0 / 0.1),
                0 0 0 2px var(--md-surface-container-low),
                0 0 0 4px var(--md-on-surface);
  }
}

.accent-swatch { background: var(--swatch-light); }

.swatch-glyph {
  font-size: 16px;
  color: var(--md-on-primary);
  pointer-events: none;
}

.custom-swatch {
  background: conic-gradient(from 0deg, #f66, #fc5, #6d6, #5cf, #86f, #f6c, #f66);
  overflow: hidden;

  .swatch-glyph { color: #fff; text-shadow: 0 1px 2px rgb(0 0 0 / 0.4); }

  input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    cursor: pointer;
  }

  &:focus-within { outline: 2px solid var(--md-primary); outline-offset: 3px; }
}

.slider-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding-left: 50px;
  transition: opacity var(--duration-short);

  &.muted { opacity: 0.45; }

  .m3-slider { flex: 1 1 auto; width: auto; min-width: 120px; }
}

.slider-label {
  flex-shrink: 0;
  font-size: var(--fs-13);
  color: var(--md-on-surface-variant);
}

.slider-value {
  flex: 0 0 44px;
  text-align: right;
  font-size: var(--fs-13);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.slider-glyph {
  flex-shrink: 0;
  font-size: 22px;
  color: var(--md-on-surface-variant);

  &.small { font-size: 15px; }
}

.text-preview {
  margin-left: 50px;
  padding: 12px 14px;
  border-radius: var(--radius-md);
  background: var(--md-surface-container);
  font-size: var(--fs-15);
  line-height: 1.5;
}

@container (max-width: 560px) {
  .swatch-row, .slider-row { padding-left: 0; }
  .text-preview { margin-left: 0; }
}
</style>

<style lang="scss">
/* 深色下色块展示深色强调色；scoped 样式无法可靠匹配 html 上的主题类，单独放非 scoped 块 */
.dark-theme .theme-appearance .accent-swatch { background: var(--swatch-dark); }
</style>
