<script setup lang="ts">
import { useToastStore } from '@/stores/toast'
import type { ToastMessage } from '@/stores/toast'
import { createLogger } from '@/utils/logger'

const log = createLogger('app-toast')

const toast = useToastStore()

// 退场动画交给 TransitionGroup 统一接管：手动点击与自动到期（store splice）
// 都会走 leave 过渡，不再有"自动消失无动画/上方条目瞬移"的问题（UI-008）
function onDismiss(id: number) {
  toast.dismiss(id)
}

function onAction(msg: ToastMessage) {
  if (!msg.action) return
  void Promise.resolve(msg.action.handler()).catch((e) => {
    log.error('Toast action failed:', e)
  })
  onDismiss(msg.id)
}
</script>

<template>
  <Teleport to="body">
    <TransitionGroup tag="div" name="toast" class="toast-container">
      <div
        v-for="msg in toast.messages"
        :key="msg.id"
        class="toast-item"
        :class="msg.type"
        @click="onDismiss(msg.id)"
      >
        <span class="material-symbols-rounded toast-icon">
          {{ msg.type === 'success' ? 'check_circle' : msg.type === 'error' ? 'error' : 'info' }}
        </span>
        <span class="toast-text">{{ msg.text }}</span>
        <button
          v-if="msg.action"
          class="toast-action"
          type="button"
          @click.stop="onAction(msg)"
        >
          {{ msg.action.label }}
        </button>
      </div>
    </TransitionGroup>
  </Teleport>
</template>

<style scoped>
.toast-container {
  position: fixed;
  bottom: calc(var(--mini-player-height, 76px) + 20px);
  /* 居中到内容区而不是整个窗口，避开左侧侧栏 */
  left: calc(50% + var(--sidebar-width, 0px) / 2);
  transform: translateX(-50%);
  z-index: 10000;
  display: flex;
  flex-direction: column-reverse;
  gap: 8px;
  pointer-events: none;
}

.toast-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 11px 18px;
  border-radius: var(--radius-md);
  background: var(--md-inverse-surface, #313033);
  color: var(--md-inverse-on-surface, #F4EFF4);
  font-size: var(--fs-14);
  font-weight: 450;
  box-shadow: var(--shadow-2);
  pointer-events: auto;
  cursor: pointer;
  min-width: 200px;
  max-width: min(460px, calc(100vw - 32px));
  user-select: none;
  backdrop-filter: blur(18px);
}

/* TransitionGroup 进出场与列表重排过渡（进/退场统一，含自动到期） */
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(16px) scale(0.95);
}

.toast-enter-active,
.toast-leave-active {
  transition: opacity 250ms cubic-bezier(0.2, 0, 0, 1),
    transform 250ms cubic-bezier(0.2, 0, 0, 1);
}

.toast-move {
  transition: transform 250ms cubic-bezier(0.2, 0, 0, 1);
}

.toast-icon {
  font-size: 20px;
  flex-shrink: 0;
}

/* toast 底色是 inverse-surface（随主题翻转），图标向 inverse-on-surface 混色以保证
   两个主题下都有足够对比度（UI-014），不再用固定浅色调 */
.toast-item.success .toast-icon {
  color: color-mix(in srgb, #2E7D32 55%, var(--md-inverse-on-surface, #F4EFF4));
}

.toast-item.error .toast-icon {
  color: color-mix(in srgb, #C62828 55%, var(--md-inverse-on-surface, #F4EFF4));
}

.toast-item.info .toast-icon {
  color: var(--md-inverse-primary, var(--md-primary, #D0BCFF));
}

.toast-text {
  line-height: 1.4;
  min-width: 0;
  /* 兜底：限制最多 4 行，避免后端/代理返回的超长文本或 HTML 撑满界面 */
  display: -webkit-box;
  -webkit-line-clamp: 4;
  line-clamp: 4;
  -webkit-box-orient: vertical;
  overflow: hidden;
  text-overflow: ellipsis;
  overflow-wrap: anywhere;
}

.toast-action {
  flex-shrink: 0;
  margin-left: 8px;
  border: 0;
  border-radius: 999px;
  padding: 6px 12px;
  background: color-mix(in srgb, var(--md-primary, #D0BCFF) 22%, transparent);
  color: var(--md-primary, #D0BCFF);
  font: inherit;
  font-size: var(--fs-13);
  font-weight: 650;
  letter-spacing: 0.01em;
  cursor: pointer;
  transition: background 160ms ease, transform 160ms ease, color 160ms ease;
}

.toast-action:hover {
  background: var(--md-primary, #D0BCFF);
  color: var(--md-on-primary, #381E72);
  transform: translateY(-1px);
}

.toast-action:active {
  transform: translateY(0) scale(0.98);
}
</style>
