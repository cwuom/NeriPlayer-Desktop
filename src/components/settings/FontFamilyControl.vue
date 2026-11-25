<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import CustomSelect from '@/components/ui/CustomSelect.vue'
import { DESKTOP_LYRICS_FONT_PRESETS, sanitizeFontFamily } from '@/modules/desktopLyrics/style'
import { fontFamilyStack } from '@/modules/settings/fonts'

const props = withDefaults(defineProps<{
  modelValue: string
  label: string
  previewText: string
  previewWeight?: number
}>(), { previewWeight: 400 })
const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
const { t } = useI18n()
const CUSTOM_FONT = '__custom__'
const customMode = ref(false)
const isPreset = (value: string) => (DESKTOP_LYRICS_FONT_PRESETS as readonly string[]).includes(value)
const choice = computed(() => customMode.value || (props.modelValue && !isPreset(props.modelValue))
  ? CUSTOM_FONT : props.modelValue)
const options = computed(() => [
  { value: '', label: t('settings.font_default') },
  ...DESKTOP_LYRICS_FONT_PRESETS.map(font => ({ value: font, label: font })),
  { value: CUSTOM_FONT, label: t('settings.font_custom') },
])

watch(() => props.modelValue, value => {
  customMode.value = !!value && !isPreset(value)
})

function chooseFont(value: string): void {
  customMode.value = value === CUSTOM_FONT
  if (value !== CUSTOM_FONT) emit('update:modelValue', value)
}

function applyCustomFont(event: Event): void {
  const input = event.target as HTMLInputElement
  const family = sanitizeFontFamily(input.value)
  input.value = family
  emit('update:modelValue', family)
}
</script>

<template>
  <div class="font-family-control">
    <CustomSelect :model-value="choice" :options="options" :label="label" @update:model-value="chooseFont" />
    <input
      v-if="choice === CUSTOM_FONT"
      class="font-name-input"
      :value="modelValue"
      :aria-label="label"
      :placeholder="t('settings.font_custom_placeholder')"
      @change="applyCustomFont"
    />
    <div class="font-preview" :style="{ fontFamily: fontFamilyStack(modelValue), fontWeight: previewWeight }">
      {{ previewText }}
    </div>
    <div class="font-hint">{{ t('settings.font_fallback_hint') }}</div>
  </div>
</template>

<style scoped lang="scss">
.font-family-control {
  display: grid;
  gap: 10px;
  width: 100%;
  min-width: 0;
}

.font-name-input {
  width: 100%;
  min-width: 0;
  padding: 10px 12px;
  border: 1px solid var(--hairline);
  border-radius: var(--radius-sm);
  outline: none;
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface);
  font: inherit;
  font-size: var(--fs-13);

  &:focus-visible { border-color: var(--md-primary); }
}

.font-preview {
  padding: 12px;
  border-radius: var(--radius-sm);
  background: var(--md-surface-container-high);
  color: var(--md-on-surface);
  font-size: 20px;
  overflow-wrap: anywhere;
}

.font-hint {
  color: var(--md-on-surface-variant);
  font-size: var(--fs-12);
  line-height: 1.5;
}
</style>
