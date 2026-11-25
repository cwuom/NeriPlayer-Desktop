<script setup lang="ts">
import { ref, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useEscapeClose } from '@/composables/useEscapeClose'

const { t } = useI18n()

const props = withDefaults(defineProps<{
  open: boolean
  title?: string
  icon?: string
  confirmText?: string
  cancelText?: string
  confirmDisabled?: boolean
  confirmDanger?: boolean
}>(), {
  confirmText: '',
  cancelText: '',
  confirmDisabled: false,
  confirmDanger: false,
})

// 使用 i18n 作为默认文本的 fallback
const displayConfirmText = computed(() => props.confirmText || t('common.confirm'))
const displayCancelText = computed(() => props.cancelText || t('common.cancel'))

const emit = defineEmits<{
  'update:open': [value: boolean]
  confirm: []
  cancel: []
}>()

const overlayRef = ref<HTMLDivElement>()

function close() {
  emit('update:open', false)
  emit('cancel')
}

function confirm() {
  if (props.confirmDisabled) return
  emit('confirm')
}

function onOverlayClick(e: MouseEvent) {
  if (e.target === overlayRef.value) close()
}

// 桌面端习惯：回车确认；多行文本、按钮里的回车，以及输入框已自行处理（defaultPrevented）的回车留给控件本身
function onEnter(e: KeyboardEvent) {
  const target = e.target as HTMLElement
  if (e.defaultPrevented || e.isComposing || target.closest('textarea, button, [contenteditable="true"]')) return
  e.preventDefault()
  confirm()
}

useEscapeClose(() => props.open, close)
</script>

<template>
  <Teleport to="body">
    <Transition name="dialog">
      <div v-if="open" ref="overlayRef" class="m3-dialog-overlay" @click="onOverlayClick">
        <div class="m3-dialog" role="dialog" aria-modal="true" :aria-label="title" @keydown.enter="onEnter">
          <div v-if="icon || title" class="dialog-header">
            <span v-if="icon" class="dialog-icon material-symbols-rounded" :class="{ danger: confirmDanger }">{{ icon }}</span>
            <div v-if="title" class="dialog-title">{{ title }}</div>
          </div>
          <div class="dialog-body">
            <slot />
          </div>
          <div class="dialog-actions">
            <button type="button" class="m3-btn tonal" @click="close">{{ displayCancelText }}</button>
            <button
              type="button"
              class="m3-btn filled"
              :class="{ disabled: confirmDisabled, danger: confirmDanger }"
              :aria-disabled="confirmDisabled || undefined"
              @click="confirm"
            >{{ displayConfirmText }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped lang="scss">
/* Clean 对话框：左对齐标题、白卡片、柔和阴影；确认为强调色胶囊，取消为中性填充 */
.m3-dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 9000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: rgb(0 0 0 / 0.32);
  -webkit-backdrop-filter: blur(6px) saturate(1.1);
  backdrop-filter: blur(6px) saturate(1.1);
}

.m3-dialog {
  width: 420px;
  max-width: 100%;
  max-height: calc(100vh - 48px);
  display: flex;
  flex-direction: column;
  background: var(--clean-dialog-surface);
  border-radius: var(--radius-xl);
  padding: 22px 22px 18px;
  box-shadow: var(--shadow-3), 0 0 0 1px var(--hairline);
}

.dialog-header {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 10px;
}

.dialog-icon {
  display: grid;
  place-items: center;
  flex-shrink: 0;
  width: 34px;
  height: 34px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--md-primary) 14%, transparent);
  color: var(--md-primary);
  font-size: 20px;

  &.danger {
    background: color-mix(in srgb, var(--md-error) 14%, transparent);
    color: var(--md-error);
  }
}

.dialog-title {
  flex: 1;
  min-width: 0;
  font-size: calc(18px * var(--text-scale));
  font-weight: 700;
  letter-spacing: var(--tracking-title);
  color: var(--md-on-surface);
  line-height: 1.3;
}

.dialog-body {
  min-height: 0;
  overflow: auto;
  color: var(--md-on-surface-variant);
  font-size: var(--fs-14);
  line-height: 1.6;
  margin-bottom: 20px;
  /* 文案里的空行要真的断成段落（目前只有光敏提示用了多段） */
  white-space: pre-line;
  user-select: text;
}

.dialog-actions {
  display: flex;
  flex-shrink: 0;
  justify-content: flex-end;
  gap: 8px;
}

.m3-btn {
  min-width: 84px;
  height: 36px;
  padding: 0 18px;
  border-radius: var(--radius-full);
  font-size: var(--fs-14);
  font-weight: 600;
  font-family: inherit;
  cursor: pointer;
  transition: background var(--duration-short), filter var(--duration-short), transform var(--duration-short);

  &:active { transform: scale(0.97); }

  &.tonal {
    background: var(--md-surface-container-high);
    color: var(--md-on-surface);

    &:hover { background: color-mix(in srgb, var(--md-on-surface) 8%, var(--md-surface-container-high)); }
  }

  &.filled {
    background: var(--md-primary);
    color: var(--md-on-primary);

    &:hover { filter: brightness(1.06); }
  }

  &.danger {
    background: var(--md-error);
    color: var(--md-on-error);
  }

  &.disabled {
    opacity: 0.4;
    pointer-events: none;
  }
}

// 过渡动画
.dialog-enter-active {
  transition: opacity 180ms ease-out;
  .m3-dialog { transition: transform 320ms cubic-bezier(0.2, 0.9, 0.3, 1.05), opacity 180ms; }
}
.dialog-leave-active {
  transition: opacity 140ms ease-in;
  .m3-dialog { transition: transform 160ms var(--ease-emphasized-accel), opacity 140ms; }
}
.dialog-enter-from {
  opacity: 0;
  .m3-dialog { transform: translateY(8px) scale(0.96); opacity: 0; }
}
.dialog-leave-to {
  opacity: 0;
  .m3-dialog { transform: scale(0.98); opacity: 0; }
}
</style>
