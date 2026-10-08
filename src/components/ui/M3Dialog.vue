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

useEscapeClose(() => props.open, close)
</script>

<template>
  <Teleport to="body">
    <Transition name="dialog">
      <div v-if="open" ref="overlayRef" class="m3-dialog-overlay" @click="onOverlayClick">
        <div class="m3-dialog" role="dialog" aria-modal="true">
          <div v-if="icon" class="dialog-icon">
            <span class="material-symbols-rounded">{{ icon }}</span>
          </div>
          <div v-if="title" class="dialog-title">{{ title }}</div>
          <div class="dialog-body">
            <slot />
          </div>
          <div class="dialog-actions">
            <button class="m3-btn text" @click="close">{{ displayCancelText }}</button>
            <button
              class="m3-btn filled"
              :class="{ disabled: confirmDisabled, danger: confirmDanger }"
              @click="confirm"
            >{{ displayConfirmText }}</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped lang="scss">
.m3-dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 9000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(4px);
}

.m3-dialog {
  width: 360px;
  max-width: calc(100vw - 48px);
  background: var(--md-surface-container-high);
  border-radius: 28px;
  padding: 24px;
  box-shadow:
    0 8px 32px rgba(0, 0, 0, 0.35),
    0 2px 8px rgba(0, 0, 0, 0.2);
}

.dialog-icon {
  display: flex;
  justify-content: center;
  margin-bottom: 16px;
  color: var(--md-primary);

  .material-symbols-rounded { font-size: 28px; }
}

.dialog-title {
  font-size: 22px;
  font-weight: 600;
  color: var(--md-on-surface);
  text-align: center;
  margin-bottom: 16px;
  line-height: 1.3;
}

.dialog-body {
  color: var(--md-on-surface-variant);
  font-size: 14px;
  line-height: 1.5;
  margin-bottom: 24px;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

// M3 按钮
.m3-btn {
  height: 40px;
  padding: 0 24px;
  border-radius: 20px;
  font-size: 14px;
  font-weight: 500;
  font-family: inherit;
  cursor: pointer;
  transition: background 200ms, opacity 200ms;

  &.text {
    background: none;
    color: var(--md-primary);

    &:hover { background: rgba(208, 188, 255, 0.08); }
    &:active { background: rgba(208, 188, 255, 0.12); }
  }

  &.filled {
    background: var(--md-primary);
    color: var(--md-on-primary);

    &:hover { box-shadow: 0 1px 3px rgba(0,0,0,0.3); }
    &:active { background: var(--md-primary); opacity: 0.9; }
  }

  &.danger {
    background: var(--md-error);
    color: var(--md-on-error);
  }

  &.disabled {
    opacity: 0.38;
    pointer-events: none;
  }
}

// 过渡动画
.dialog-enter-active {
  transition: opacity 200ms ease-out;
  .m3-dialog { transition: transform 300ms cubic-bezier(0.05, 0.7, 0.1, 1), opacity 200ms; }
}
.dialog-leave-active {
  transition: opacity 150ms ease-in;
  .m3-dialog { transition: transform 200ms cubic-bezier(0.3, 0, 0.8, 0.15), opacity 150ms; }
}
.dialog-enter-from {
  opacity: 0;
  .m3-dialog { transform: scale(0.85); opacity: 0; }
}
.dialog-leave-to {
  opacity: 0;
  .m3-dialog { transform: scale(0.92); opacity: 0; }
}
</style>
