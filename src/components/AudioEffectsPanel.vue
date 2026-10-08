<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { EQ_PRESETS, usePlayerStore } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import CustomSelect from './ui/CustomSelect.vue'
import EditableRangeValue from './ui/EditableRangeValue.vue'

/// 音频效果：播放速度、响度增益、均衡器与重置。工具栏弹层和「更多」面板共用这一份
const player = usePlayerStore()
const settings = useSettingsStore()
const { t } = useI18n()

const SPEED_PRESETS = [0.5, 0.75, 0.85, 1.0, 1.25, 1.5, 2.0, 3.0]
const LOUDNESS_PRESETS_MB = [0, 300, 600, 900, 1200, 1500]
const eqPresetIds = Object.keys(EQ_PRESETS)
const eqFreqLabels = ['60', '230', '910', '3.6k', '14k']

function onEqBandChange(index: number, value: number) {
  const bands = [...player.equalizerBands]
  bands[index] = Math.round(value)
  player.setEqualizer(player.equalizerEnabled, bands)
  player.equalizerPresetId = 'custom'
  settings.equalizerPresetId = 'custom'
}
</script>

<template>
  <div class="audiofx-panel">
    <!-- 播放速度 -->
    <div class="audiofx-section">
      <div class="audiofx-section-header">{{ t('player.speed') }}</div>
      <div class="audiofx-speed-grid">
        <button
          v-for="spd in SPEED_PRESETS"
          :key="spd"
          class="speed-option"
          :class="{ active: player.playbackSpeed === spd }"
          @click="player.setSpeed(spd)"
        >
          {{ spd }}x
        </button>
      </div>
      <div class="audiofx-slider-row">
        <span class="audiofx-slider-label">0.25x</span>
        <input
          type="range" min="0.25" max="3" step="0.05"
          :value="player.playbackSpeed"
          class="audiofx-slider"
          :aria-label="t('player.playback_speed')"
          @input="player.setSpeed(parseFloat(($event.target as HTMLInputElement).value))"
        />
        <span class="audiofx-slider-label">3x</span>
        <EditableRangeValue
          :model-value="player.playbackSpeed"
          class="audiofx-slider-value"
          :min="0.25"
          :max="3"
          :step="0.05"
          :display-value="`${player.playbackSpeed.toFixed(2)}x`"
          input-suffix="x"
          :aria-label="t('player.playback_speed')"
          @update:model-value="player.setSpeed($event)"
        />
      </div>
    </div>

    <!-- 响度增益 -->
    <div class="audiofx-section">
      <div class="audiofx-section-header">{{ t('player.loudness_gain') }}</div>
      <div class="audiofx-preset-row">
        <button v-for="db in LOUDNESS_PRESETS_MB" :key="db"
          class="speed-option" :class="{ active: player.loudnessGainMb === db }"
          @click="player.setLoudnessGain(db)"
        >
          {{ db === 0 ? '0' : '+' + (db / 100).toFixed(0) }}dB
        </button>
      </div>
      <div class="audiofx-slider-row">
        <span class="audiofx-slider-label">0</span>
        <input
          type="range" min="0" max="1500" step="50"
          :value="player.loudnessGainMb"
          class="audiofx-slider"
          :aria-label="t('player.loudness_gain')"
          @input="player.setLoudnessGain(parseFloat(($event.target as HTMLInputElement).value))"
        />
        <span class="audiofx-slider-label">+15dB</span>
        <EditableRangeValue
          :model-value="player.loudnessGainMb"
          class="audiofx-slider-value"
          :min="0"
          :max="1500"
          :step="50"
          :input-scale="0.01"
          :input-width="44"
          :display-value="`+${(player.loudnessGainMb / 100).toFixed(1)}dB`"
          input-suffix="dB"
          :aria-label="t('player.loudness_gain')"
          @update:model-value="player.setLoudnessGain($event)"
        />
      </div>
    </div>

    <!-- 均衡器 -->
    <div class="audiofx-section">
      <div class="audiofx-section-header">
        {{ t('player.equalizer') }}
        <label class="audiofx-toggle">
          <input type="checkbox" :checked="player.equalizerEnabled" :aria-label="t('player.equalizer')"
            @change="player.setEqualizer(($event.target as HTMLInputElement).checked, player.equalizerBands)" />
          <span class="audiofx-toggle-slider"></span>
        </label>
      </div>
      <CustomSelect
        surface="dark"
        :model-value="player.equalizerPresetId"
        :options="eqPresetIds.map(pid => ({ value: pid, label: t('player.eq_' + pid) }))"
        @update:model-value="player.setEqualizerPreset($event)"
      />
      <div v-if="player.equalizerEnabled" class="audiofx-eq-bands">
        <div v-for="(freq, i) in eqFreqLabels" :key="i" class="audiofx-eq-band">
          <EditableRangeValue
            :model-value="player.equalizerBands[i]"
            class="audiofx-eq-val"
            :min="-1500"
            :max="1500"
            :step="50"
            :input-scale="0.01"
            :input-width="44"
            :display-value="(player.equalizerBands[i] / 100).toFixed(1)"
            input-suffix="dB"
            :aria-label="`${freq} Hz`"
            @update:model-value="onEqBandChange(i, $event)"
          />
          <input type="range" min="-1500" max="1500" step="50"
            class="audiofx-eq-slider" orient="vertical"
            :value="player.equalizerBands[i]"
            :aria-label="`${freq} Hz`"
            @input="onEqBandChange(i, parseFloat(($event.target as HTMLInputElement).value))" />
          <span class="audiofx-eq-freq">{{ freq }}</span>
        </div>
      </div>
    </div>

    <!-- 重置 -->
    <button class="speed-option audiofx-reset" @click="player.resetAudioEffects()">
      {{ t('player.reset_effects') }}
    </button>
  </div>
</template>

<style scoped lang="scss">
.audiofx-panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.audiofx-section :deep(.custom-select) {
  width: 100%;
}

.audiofx-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.audiofx-section-header {
  font-size: 12px;
  font-weight: 600;
  color: rgba(255,255,255,0.5);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.audiofx-preset-row {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

// 八档速度排成整齐的两行，不在第二行剩一个
.audiofx-speed-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 4px;

  .speed-option { text-align: center; }
}

.speed-option {
  padding: 8px 16px;
  border: none;
  background: transparent;
  color: rgba(255,255,255,0.7);
  font-size: 13px;
  cursor: pointer;
  border-radius: 8px;
  text-align: left;
  white-space: nowrap;
  transition: all 0.15s;

  &:hover {
    background: color-mix(in srgb, var(--np-primary-container, rgba(255,255,255,0.12)) 16%, rgba(255,255,255,0.08));
    color: white;
  }

  &.active {
    color: var(--np-primary, var(--md-primary, #D0BCFF));
    font-weight: 600;
  }
}

.audiofx-speed-grid .speed-option,
.audiofx-preset-row .speed-option {
  padding: 5px 10px;
  font-size: 12px;
  min-width: unset;
  flex: 0 0 auto;
}

.audiofx-slider-row {
  display: flex;
  align-items: center;
  gap: 6px;
}

.audiofx-slider-label {
  font-size: 10px;
  color: rgba(255,255,255,0.35);
  min-width: 28px;
  text-align: center;
}

.audiofx-slider-value {
  font-size: 11px;
  font-weight: 600;
  color: var(--md-primary, #D0BCFF);
  min-width: 42px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.audiofx-slider {
  flex: 1;
  appearance: none;
  height: 4px;
  background: rgba(255,255,255,0.15);
  border-radius: 2px;
  outline: none;
  cursor: pointer;

  &::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--np-primary, var(--md-primary, #D0BCFF));
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
  }

  &::-moz-range-thumb {
    width: 14px;
    height: 14px;
    border: none;
    border-radius: 50%;
    background: var(--np-primary, var(--md-primary, #D0BCFF));
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
  }
}

.audiofx-toggle {
  position: relative;
  display: inline-block;
  width: 34px;
  height: 18px;

  input { opacity: 0; width: 0; height: 0; }

  .audiofx-toggle-slider {
    position: absolute;
    cursor: pointer;
    inset: 0;
    background: rgba(255,255,255,0.15);
    border-radius: 18px;
    transition: 0.2s;

    &::before {
      content: '';
      position: absolute;
      height: 14px;
      width: 14px;
      left: 2px;
      bottom: 2px;
      background: white;
      border-radius: 50%;
      transition: 0.2s;
    }
  }

  input:checked + .audiofx-toggle-slider {
    background: var(--md-primary, #D0BCFF);
  }

  input:checked + .audiofx-toggle-slider::before {
    transform: translateX(16px);
  }
}

.audiofx-eq-bands {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 4px;
  padding: 4px 0;
}

.audiofx-eq-band {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  flex: 1;
}

.audiofx-eq-val {
  font-size: 10px;
  color: var(--np-primary, var(--md-primary, #D0BCFF));
  font-variant-numeric: tabular-nums;
  min-height: 14px;
}

.audiofx-eq-freq {
  font-size: 9px;
  color: rgba(255,255,255,0.35);
}

.audiofx-eq-slider {
  writing-mode: vertical-lr;
  direction: rtl;
  appearance: none;
  width: 4px;
  height: 80px;
  background: rgba(255,255,255,0.15);
  border-radius: 2px;
  cursor: pointer;
  outline: none;

  &::-webkit-slider-thumb {
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--np-primary, var(--md-primary, #D0BCFF));
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
  }

  &::-moz-range-thumb {
    width: 12px;
    height: 12px;
    border: none;
    border-radius: 50%;
    background: var(--np-primary, var(--md-primary, #D0BCFF));
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.3);
  }
}

.audiofx-reset {
  width: 100%;
  text-align: center;
  color: #EF5350 !important;
  border-top: 1px solid rgba(255,255,255,0.06);
  margin-top: 2px;
  padding-top: 10px;
}
</style>
