<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { usePlayerStore } from '@/stores/player'
import { AUDIO_EFFECT_PRESETS } from '@/modules/audioEffects/model'
import CustomSelect from './ui/CustomSelect.vue'
import EditableRangeValue from './ui/EditableRangeValue.vue'

const emit = defineEmits<{ details: [] }>()
const player = usePlayerStore()
const { t } = useI18n()
const SPEED_PRESETS = [0.5, 0.75, 0.95, 1, 1.05, 1.25, 1.5, 2]
const presetOptions = computed(() => [
  ...AUDIO_EFFECT_PRESETS.filter(preset => preset.id !== 'custom').map(preset => ({ value: preset.id, label: t(`dsp.presets.${preset.id}`) })),
  ...player.audioEffects.userPresets.map(preset => ({ value: preset.id, label: preset.name })),
])
const presetLabel = computed(() => presetOptions.value.some(option => option.value === player.audioEffectsProfile.presetId) ? undefined : t('dsp.manual'))
// 自定义倍速不在预设里时隐藏滑块拇指，避免误指向某个预设
const speedIndex = computed(() => SPEED_PRESETS.indexOf(player.playbackSpeed))
const speedThumbStyle = computed(() => ({
  width: `calc((100% - 6px) / ${SPEED_PRESETS.length})`,
  transform: `translateX(${Math.max(0, speedIndex.value) * 100}%)`,
}))
const fill = (value: number, min: number, max: number) => ({
  '--slider-fill': `${Math.min(100, Math.max(0, (value - min) / (max - min) * 100))}%`,
})
</script>

<template>
  <div class="audiofx-panel">
    <section class="audiofx-section">
      <div class="audiofx-section-header">
        <span class="audiofx-section-title">{{ t('player.playback_speed') }}</span>
        <EditableRangeValue :model-value="player.playbackSpeed" class="audiofx-slider-value"
          :min="0.1" :max="4" :step="0.05" :display-value="`${player.playbackSpeed.toFixed(2)}x`"
          input-suffix="x" :aria-label="t('player.playback_speed')" @update:model-value="player.setSpeed($event)" />
      </div>
      <div class="audiofx-speed-grid" role="group" :aria-label="t('player.playback_speed')">
        <span v-if="speedIndex >= 0" class="audiofx-speed-thumb" :style="speedThumbStyle" aria-hidden="true"></span>
        <button v-for="speed in SPEED_PRESETS" :key="speed" type="button" class="speed-option"
          :class="{ active: player.playbackSpeed === speed }" :aria-pressed="player.playbackSpeed === speed"
          @click="player.setSpeed(speed)">{{ speed }}x</button>
      </div>
      <div class="audiofx-slider-row">
        <span class="audiofx-slider-label">0.1x</span>
        <input type="range" min="0.1" max="4" step="0.05" :value="player.playbackSpeed"
          class="m3-slider audiofx-slider" :style="fill(player.playbackSpeed, 0.1, 4)" :aria-label="t('player.playback_speed')"
          @input="player.setSpeed(parseFloat(($event.target as HTMLInputElement).value))" />
        <span class="audiofx-slider-label">4x</span>
      </div>
    </section>

    <section v-if="!player.vinylEffectEnabled" class="audiofx-section">
      <div class="audiofx-section-header">
        <span class="audiofx-section-title">{{ t('player.pitch') }}</span>
        <EditableRangeValue :model-value="player.playbackPitch" class="audiofx-slider-value"
          :min="0.25" :max="2" :step="0.05" :display-value="`${player.playbackPitch.toFixed(2)}x`"
          input-suffix="x" :aria-label="t('player.pitch')" @update:model-value="player.setPitch($event)" />
      </div>
      <div class="audiofx-slider-row">
        <span class="audiofx-slider-label">0.25x</span>
        <input type="range" min="0.25" max="2" step="0.05" :value="player.playbackPitch"
          class="m3-slider audiofx-slider" :style="fill(player.playbackPitch, 0.25, 2)" :aria-label="t('player.pitch')"
          @input="player.setPitch(parseFloat(($event.target as HTMLInputElement).value))" />
        <span class="audiofx-slider-label">2x</span>
      </div>
    </section>

    <div class="audiofx-group">
      <div class="audiofx-row">
        <span class="material-symbols-rounded audiofx-row-icon" aria-hidden="true">album</span>
        <div class="audiofx-row-info">
          <span class="audiofx-row-title">{{ t('player.vinyl_effect') }}</span>
          <span v-if="player.vinylEffectEnabled" class="audiofx-help">{{ t('dsp.vinyl_following', { value: player.effectivePlaybackPitch().toFixed(2) }) }}</span>
        </div>
        <label class="m3-switch audiofx-switch">
          <input type="checkbox" :checked="player.vinylEffectEnabled" :aria-label="t('player.vinyl_effect')"
            @change="player.setVinylEffect(($event.target as HTMLInputElement).checked)" />
          <span class="track"><span class="thumb"></span></span>
        </label>
      </div>
      <div class="audiofx-row">
        <span class="material-symbols-rounded audiofx-row-icon" aria-hidden="true">graphic_eq</span>
        <div class="audiofx-row-info">
          <span class="audiofx-row-title">{{ t('player.eq_preset') }}</span>
        </div>
        <CustomSelect class="audiofx-preset" surface="dark" :model-value="player.audioEffectsProfile.presetId" :selected-label="presetLabel" :label="t('player.eq_preset')"
          :options="presetOptions" @update:model-value="player.setAudioEffectsPreset($event)" />
      </div>
    </div>

    <div class="audiofx-actions">
      <button type="button" class="audiofx-action audiofx-reset" @click="player.resetAudioEffects()">{{ t('player.reset_effects') }}</button>
      <button type="button" class="audiofx-action audiofx-details" @click="emit('details')">{{ t('player.audio_effects_details') }}</button>
    </div>
  </div>
</template>

<style scoped lang="scss">
/* 面板只出现在播放页的深色玻璃上（工具栏弹层与「更多」面板），颜色固定为白色透明度阶梯，不随应用明暗主题变化 */
.audiofx-panel {
  --fx-accent: var(--np-primary, #fff);
  display: flex;
  flex-direction: column;
  gap: 18px;
  color: rgba(255, 255, 255, 0.92);
}

.audiofx-section { display: flex; flex-direction: column; gap: 10px; }

.audiofx-section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 26px;
}

.audiofx-section-title {
  color: rgba(255, 255, 255, 0.92);
  font-size: var(--fs-13);
  font-weight: 600;
  letter-spacing: 0.1px;
}

.audiofx-slider-value {
  min-width: 52px;
  padding: 3px 9px;
  border-radius: var(--radius-full);
  background: rgba(255, 255, 255, 0.08);
  color: rgba(255, 255, 255, 0.92);
  font-size: var(--fs-12);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  text-align: center;
}

/* 倍速预设：分段胶囊 + 滑动拇指（与设置页 SegmentedControl 同构，换成深色玻璃配色） */
.audiofx-speed-grid {
  position: relative;
  display: grid;
  grid-template-columns: repeat(8, minmax(0, 1fr));
  padding: 3px;
  border-radius: var(--radius-full);
  background: rgba(255, 255, 255, 0.07);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.05);
  isolation: isolate;
}

.audiofx-speed-thumb {
  position: absolute;
  top: 3px;
  bottom: 3px;
  left: 3px;
  z-index: -1;
  border-radius: var(--radius-full);
  background: rgba(255, 255, 255, 0.2);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.28), inset 0 0 0 0.5px rgba(255, 255, 255, 0.16);
  transition: transform 320ms cubic-bezier(0.3, 1.2, 0.5, 1);
}

.speed-option {
  min-width: 0;
  height: 30px;
  padding: 0;
  border: none;
  border-radius: var(--radius-full);
  background: transparent;
  color: rgba(255, 255, 255, 0.6);
  font-size: var(--fs-12);
  font-weight: 500;
  font-variant-numeric: tabular-nums;
  letter-spacing: -0.2px;
  white-space: nowrap;
  cursor: pointer;
  transition: color 0.15s;

  &:hover:not(.active) { color: rgba(255, 255, 255, 0.9); }
  &.active { color: #fff; font-weight: 600; }
  &:focus-visible { outline: 2px solid rgba(255, 255, 255, 0.85); outline-offset: -2px; }
}

.audiofx-slider-row { display: flex; align-items: center; gap: 10px; }

.audiofx-slider-label {
  flex-shrink: 0;
  min-width: 30px;
  color: rgba(255, 255, 255, 0.45);
  font-size: var(--fs-11);
  font-variant-numeric: tabular-nums;
  text-align: center;
}

/* 复用全局 .m3-slider 的拇指与填充同步，只把轨道换成深色玻璃上的白色透明度 */
.audiofx-slider {
  flex: 1;
  min-width: 0;
  --slider-track: rgba(255, 255, 255, 0.16);

  &::-webkit-slider-runnable-track {
    background: linear-gradient(to right, var(--fx-accent) var(--slider-fill, 0%), var(--slider-track) var(--slider-fill, 0%));
  }
  &::-moz-range-progress { background: var(--fx-accent); }
  &:focus-visible::-webkit-slider-thumb { box-shadow: 0 0 0 3px rgba(255, 255, 255, 0.4), 0 2px 6px rgb(0 0 0 / 0.3); }
}

/* 分组列表：与「更多」面板的 .np-more-group 同一套玻璃分组 */
.audiofx-group {
  overflow: hidden;
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.055);
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.05);
}

.audiofx-row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 52px;
  padding: 8px 12px 8px 10px;

  & + &::before {
    content: '';
    position: absolute;
    top: 0;
    left: 52px;
    right: 12px;
    height: 1px;
    background: rgba(255, 255, 255, 0.07);
  }
}

.audiofx-row-icon {
  display: grid;
  place-items: center;
  flex-shrink: 0;
  width: 30px;
  height: 30px;
  border-radius: 9px;
  background: rgba(255, 255, 255, 0.08);
  color: rgba(255, 255, 255, 0.75);
  font-size: 18px;
}

.audiofx-row-info {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.audiofx-row-title {
  overflow: hidden;
  color: rgba(255, 255, 255, 0.9);
  font-size: var(--fs-13);
  font-weight: 500;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.audiofx-help { margin: 0; color: rgba(255, 255, 255, 0.5); font-size: var(--fs-11); line-height: 1.45; }

.audiofx-preset { flex: 0 1 168px; min-width: 0; }

/* 全局 .m3-switch 的关态轨道取自应用主题表面色，浅色主题下会在深色玻璃上发白，这里固定为白色透明度 */
.audiofx-switch {
  .track { background: rgba(255, 255, 255, 0.18); }
  &:hover input:not(:disabled) + .track { background: rgba(255, 255, 255, 0.26); }
  input:checked + .track { background: var(--fx-accent); }
  input:focus-visible + .track { outline-color: rgba(255, 255, 255, 0.85); }
}

.audiofx-actions { display: flex; gap: 8px; }

.audiofx-action {
  flex: 1;
  min-height: 36px;
  padding: 0 14px;
  border: none;
  border-radius: var(--radius-full);
  font-size: var(--fs-13);
  font-weight: 600;
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;

  &:focus-visible { outline: 2px solid rgba(255, 255, 255, 0.85); outline-offset: 2px; }
}

.audiofx-reset {
  background: transparent;
  color: #ff8a80;
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.1);

  &:hover { background: rgba(255, 82, 82, 0.12); }
}

.audiofx-details {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 2px;
  background: rgba(255, 255, 255, 0.12);
  color: #fff;

  &::after {
    content: 'chevron_right';
    font-family: 'Material Symbols Rounded';
    font-size: 18px;
    font-weight: 400;
    margin-right: -6px;
  }

  &:hover { background: rgba(255, 255, 255, 0.18); }
}

@media (prefers-reduced-motion: reduce) {
  .audiofx-speed-thumb { transition: none; }
}
</style>
