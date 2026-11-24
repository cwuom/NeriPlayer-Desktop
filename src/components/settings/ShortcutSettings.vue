<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useSettingsStore } from '@/stores/settings'
import { useToastStore } from '@/stores/toast'
import {
  DEFAULT_GLOBAL_SHORTCUTS,
  DEFAULT_LOCAL_SHORTCUTS,
  SHORTCUT_ACTIONS,
  comboFromEvent,
  comboFromMouseEvent,
  comboLabelParts,
  effectiveShortcuts,
  isMouseCombo,
  toAccelerator,
  withShortcut,
  type ShortcutActionId,
  type ShortcutScope,
} from '@/modules/shortcuts/bindings'
import { detectPlatform } from '@/utils/platform'
import { shortcutRecording } from '@/modules/shortcuts/recording'
import { systemShortcutFailures } from '@/modules/shortcuts/systemShortcuts'

/// 快捷键分区：应用内（窗口在前台）与系统级全局两套键位，逐项点击录制
const { t } = useI18n()
const settings = useSettingsStore()
const toast = useToastStore()

const localMap = computed(() => effectiveShortcuts(settings.shortcutBindings, 'local'))
const globalMap = computed(() => effectiveShortcuts(settings.shortcutBindings, 'global'))
const recording = ref<{ id: ShortcutActionId; scope: ShortcutScope } | null>(null)
const recordError = ref('')

const hasOverrides = computed(() =>
  Object.keys(settings.shortcutBindings.local).length > 0 || Object.keys(settings.shortcutBindings.global).length > 0,
)

function actionLabel(id: ShortcutActionId): string {
  return t(`shortcuts.${id}`)
}

function isOverridden(id: ShortcutActionId): boolean {
  return id in settings.shortcutBindings.local || id in settings.shortcutBindings.global
}

function isRecording(id: ShortcutActionId, scope: ShortcutScope): boolean {
  return recording.value?.id === id && recording.value.scope === scope
}

/// 全局键位的悬浮说明：鼠标键在非 Windows 上没有全局能力
function globalTitle(id: ShortcutActionId): string | undefined {
  if (settings.globalShortcutsEnabled && systemShortcutFailures.value.includes(id)) {
    return t('shortcuts.global_unavailable')
  }
  const combo = globalMap.value[id]
  if (combo && isMouseCombo(combo) && detectPlatform() !== 'windows') {
    return t('shortcuts.mouse_global_windows_only')
  }
  return undefined
}

/// 绑定框里的辅助说明：鼠标键单独标注
function keyHint(combo: string): string | undefined {
  if (combo && isMouseCombo(combo)) return t('shortcuts.mouse_only')
  return undefined
}

function startRecording(id: ShortcutActionId, scope: ShortcutScope) {
  if (isRecording(id, scope)) {
    stopRecording()
    return
  }
  recording.value = { id, scope }
  recordError.value = ''
  shortcutRecording.value = true
  document.addEventListener('keydown', onRecordKeydown, true)
  document.addEventListener('mousedown', onRecordMousedown, true)
}

function stopRecording() {
  recording.value = null
  shortcutRecording.value = false
  document.removeEventListener('keydown', onRecordKeydown, true)
  document.removeEventListener('mousedown', onRecordMousedown, true)
}

function assign(id: ShortcutActionId, scope: ShortcutScope, combo: string) {
  let bindings = settings.shortcutBindings
  const map = scope === 'local' ? localMap.value : globalMap.value
  // 组合键已被别的动作占用时转给当前动作，免得同一组合触发两件事
  const holder = combo ? (Object.keys(map) as ShortcutActionId[]).find(other => other !== id && map[other] === combo) : undefined
  if (holder) {
    bindings = withShortcut(bindings, scope, holder, '')
    toast.show(t('shortcuts.moved_from', { action: actionLabel(holder) }), 'info')
  }
  settings.shortcutBindings = withShortcut(bindings, scope, id, combo)
}

function onRecordKeydown(event: KeyboardEvent) {
  const current = recording.value
  if (!current) return
  event.preventDefault()
  event.stopImmediatePropagation()
  if (event.repeat) return
  if (event.key === 'Escape') {
    stopRecording()
    return
  }
  const plain = !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey
  if (plain && (event.key === 'Backspace' || event.key === 'Delete')) {
    assign(current.id, current.scope, '')
    stopRecording()
    return
  }
  const combo = comboFromEvent(event)
  if (!combo) return
  if (current.scope === 'global') {
    // 系统级热键不带修饰键会吞掉所有程序里的这个键
    // 鼠标键例外：中键与侧键本身就不参与普通操作，可以单独作为全局键
    const hasModifier = combo.includes('+')
    if (!hasModifier && !/^F\d+$/.test(combo)) {
      recordError.value = t('shortcuts.global_needs_modifier')
      return
    }
    if (!toAccelerator(combo)) {
      recordError.value = t('shortcuts.global_unsupported_key')
      return
    }
  }
  assign(current.id, current.scope, combo)
  stopRecording()
}

/// 录制鼠标键：只认中键与侧键，左右键留给正常界面操作
function onRecordMousedown(event: MouseEvent) {
  const current = recording.value
  if (!current) return
  // 左右键不做成快捷键，不打断用户点击
  if (event.button === 0 || event.button === 2) return
  event.preventDefault()
  event.stopImmediatePropagation()
  const combo = comboFromMouseEvent(event)
  if (!combo) return
  assign(current.id, current.scope, combo)
  stopRecording()
}

function resetAction(id: ShortcutActionId) {
  let bindings = withShortcut(settings.shortcutBindings, 'local', id, DEFAULT_LOCAL_SHORTCUTS[id] ?? '')
  bindings = withShortcut(bindings, 'global', id, DEFAULT_GLOBAL_SHORTCUTS[id] ?? '')
  settings.shortcutBindings = bindings
}

function resetAll() {
  stopRecording()
  settings.shortcutBindings = { local: {}, global: {} }
}

onBeforeUnmount(stopRecording)
</script>

<template>
  <div class="shortcut-settings">
    <div class="setting-card">
      <div class="setting-icon-wrap"><span class="material-symbols-rounded">public</span></div>
      <div class="setting-info">
        <div class="setting-title">{{ t('shortcuts.global_enable') }}</div>
        <div class="setting-desc">{{ t('shortcuts.global_enable_desc') }}</div>
      </div>
      <label class="m3-switch">
        <input v-model="settings.globalShortcutsEnabled" type="checkbox" />
        <span class="track"><span class="thumb"><span v-if="settings.globalShortcutsEnabled" class="material-symbols-rounded" style="font-size: 14px">check</span></span></span>
      </label>
    </div>

    <div class="shortcut-toolbar">
      <span class="shortcut-hint">{{ t('shortcuts.record_hint') }}</span>
      <button class="m3-chip sm" type="button" :disabled="!hasOverrides" @click="resetAll">
        <span class="material-symbols-rounded" style="font-size: 16px">restart_alt</span>
        {{ t('shortcuts.reset_all') }}
      </button>
    </div>

    <div class="shortcut-table" role="table" :aria-label="t('shortcuts.title')">
      <div class="shortcut-head" role="row">
        <span role="columnheader">{{ t('shortcuts.action') }}</span>
        <span role="columnheader">{{ t('shortcuts.local') }}</span>
        <span role="columnheader">{{ t('shortcuts.global') }}</span>
        <span />
      </div>
      <div v-for="action in SHORTCUT_ACTIONS" :key="action.id" class="shortcut-row" role="row">
        <span class="shortcut-action" role="cell">
          <span class="material-symbols-rounded">{{ action.icon }}</span>
          {{ actionLabel(action.id) }}
        </span>

        <span role="cell">
          <span v-if="action.globalOnly" class="shortcut-na">{{ t('shortcuts.global_only') }}</span>
          <button
            v-else
            type="button"
            class="shortcut-key"
            :class="{ recording: isRecording(action.id, 'local'), empty: !localMap[action.id] }"
            :aria-label="`${actionLabel(action.id)} · ${t('shortcuts.local')}`"
            @click="startRecording(action.id, 'local')"
            @blur="isRecording(action.id, 'local') && stopRecording()"
          >
            <template v-if="isRecording(action.id, 'local')">{{ t('shortcuts.press_keys') }}</template>
            <template v-else-if="localMap[action.id]">
              <kbd v-for="part in comboLabelParts(localMap[action.id])" :key="part">{{ part }}</kbd>
            </template>
            <template v-else>{{ t('shortcuts.unset') }}</template>
          </button>
          <span v-if="keyHint(localMap[action.id])" class="shortcut-na">{{ keyHint(localMap[action.id]) }}</span>
        </span>

        <span role="cell">
          <span v-if="!action.global" class="shortcut-na">{{ t('shortcuts.local_only') }}</span>
          <button
            v-else
            type="button"
            class="shortcut-key"
            :class="{
              recording: isRecording(action.id, 'global'),
              empty: !globalMap[action.id],
              inactive: !settings.globalShortcutsEnabled,
              failed: settings.globalShortcutsEnabled && systemShortcutFailures.includes(action.id),
            }"
            :title="globalTitle(action.id)"
            :aria-label="`${actionLabel(action.id)} · ${t('shortcuts.global')}`"
            @click="startRecording(action.id, 'global')"
            @blur="isRecording(action.id, 'global') && stopRecording()"
          >
            <template v-if="isRecording(action.id, 'global')">{{ t('shortcuts.press_keys') }}</template>
            <template v-else-if="globalMap[action.id]">
              <kbd v-for="part in comboLabelParts(globalMap[action.id])" :key="part">{{ part }}</kbd>
              <span v-if="settings.globalShortcutsEnabled && systemShortcutFailures.includes(action.id)" class="material-symbols-rounded shortcut-warn">error</span>
            </template>
            <template v-else>{{ t('shortcuts.unset') }}</template>
          </button>
          <span v-if="keyHint(globalMap[action.id])" class="shortcut-na">{{ keyHint(globalMap[action.id]) }}</span>
        </span>

        <span role="cell" class="shortcut-reset-cell">
          <button
            v-if="isOverridden(action.id)"
            type="button"
            class="shortcut-reset"
            :title="t('shortcuts.reset')"
            :aria-label="`${t('shortcuts.reset')} · ${actionLabel(action.id)}`"
            @click="resetAction(action.id)"
          >
            <span class="material-symbols-rounded">undo</span>
          </button>
        </span>
      </div>
    </div>
    <Transition name="fade">
      <p v-if="recordError && recording" class="shortcut-error">{{ recordError }}</p>
    </Transition>
    <p class="shortcut-footnote">{{ t('shortcuts.desc') }}</p>
  </div>
</template>

<style scoped lang="scss">
.shortcut-settings {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

/* 与设置页的卡片 / 胶囊按钮同款：那边是 scoped 样式，进不了子组件 */
.setting-card {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px 16px;
  border-radius: var(--radius-lg);
  background: var(--md-surface-container);
  transition: background var(--duration-short);
  &:hover { background: var(--md-surface-container-high); }
}

.setting-icon-wrap {
  width: 40px;
  height: 40px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-md);
  background: var(--md-surface-container-high);
  color: var(--md-on-surface-variant);
}

.setting-info { flex: 1; min-width: 0; }
.setting-title { font-size: 14px; font-weight: 500; }
.setting-desc { margin-top: 2px; font-size: 12px; color: var(--md-on-surface-variant); }

.m3-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 4px 12px;
  border: 1px solid var(--md-outline-variant);
  border-radius: var(--radius-full);
  background: var(--md-surface-container-highest);
  color: var(--md-on-surface-variant);
  font-family: inherit;
  font-size: 12px;
  font-weight: 500;
  white-space: nowrap;
  cursor: pointer;
  transition: background var(--duration-short) var(--ease-standard), opacity var(--duration-short) var(--ease-standard);
  &:hover:not(:disabled) { background: var(--md-surface-variant); }
  &:disabled { opacity: 0.45; cursor: default; }
}

.shortcut-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 4px 4px 0;
}

.shortcut-hint,
.shortcut-footnote {
  font-size: 12px;
  color: var(--md-on-surface-variant);
}

.shortcut-footnote { padding: 0 4px; }

.shortcut-table {
  display: flex;
  flex-direction: column;
  padding: 6px;
  border-radius: var(--radius-lg, 16px);
  background: var(--md-surface-container);
}

.shortcut-head,
.shortcut-row {
  display: grid;
  grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr) minmax(0, 1fr) 32px;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
}

.shortcut-head {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--md-on-surface-variant);
}

.shortcut-row {
  border-radius: 12px;
  transition: background 140ms var(--ease-standard);
  &:hover { background: color-mix(in srgb, var(--md-on-surface) 5%, transparent); }
}

.shortcut-action {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  font-size: 13px;
  .material-symbols-rounded { font-size: 18px; color: var(--md-on-surface-variant); }
}

.shortcut-key {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-height: 30px;
  max-width: 100%;
  padding: 3px 8px;
  border: 1px solid transparent;
  border-radius: 8px;
  font-size: 12px;
  color: var(--md-on-surface);
  background: var(--md-surface-container-high);
  transition: border-color 140ms var(--ease-standard), background 140ms var(--ease-standard), opacity 140ms var(--ease-standard);

  &:hover { border-color: var(--md-outline-variant); }
  &.empty { color: var(--md-on-surface-variant); font-style: italic; }
  &.inactive { opacity: 0.55; }
  &.failed { border-color: var(--md-error); }
  &.recording {
    border-color: var(--md-primary);
    background: color-mix(in srgb, var(--md-primary) 14%, var(--md-surface-container-high));
    color: var(--md-primary);
    font-style: normal;
    animation: shortcut-pulse 1.2s ease-in-out infinite;
  }

  kbd {
    min-width: 20px;
    padding: 1px 6px;
    border-radius: 5px;
    border: 1px solid var(--md-outline-variant);
    border-bottom-width: 2px;
    background: var(--md-surface-container-highest);
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 18px;
    text-align: center;
  }
}

.shortcut-warn { font-size: 16px; color: var(--md-error); }

.shortcut-na {
  font-size: 12px;
  color: var(--md-on-surface-variant);
  opacity: 0.7;
}

.shortcut-reset-cell { display: flex; justify-content: center; }

.shortcut-reset {
  width: 28px;
  height: 28px;
  display: grid;
  place-items: center;
  border-radius: 50%;
  color: var(--md-on-surface-variant);
  .material-symbols-rounded { font-size: 18px; }
  &:hover { background: var(--md-surface-container-highest); color: var(--md-on-surface); }
}

.shortcut-error {
  padding: 0 4px;
  font-size: 12px;
  color: var(--md-error);
}

@keyframes shortcut-pulse {
  50% { box-shadow: 0 0 0 3px color-mix(in srgb, var(--md-primary) 22%, transparent); }
}
</style>
