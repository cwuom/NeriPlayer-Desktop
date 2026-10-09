<script setup lang="ts">
// 「定位到当前播放曲目」悬浮按钮：对齐 Android 端 HapticFloatingActionButton + PlaylistPlay 图标
// .detail-view 带 transform 会劫持 fixed 定位，因此 Teleport 出去；目标是 .app-layout 而不是 body：
// .app-layout 是独立层叠上下文（isolation），放到 body 上 z-index 会压过整个应用，播放页也挡不住
defineProps<{
  visible: boolean
  label: string
}>()

const emit = defineEmits<{ (e: 'click'): void }>()
</script>

<template>
  <Teleport to=".app-layout">
    <Transition name="locate-fab">
      <button
        v-if="visible"
        type="button"
        class="locate-fab"
        :aria-label="label"
        :title="label"
        @click="emit('click')"
      >
        <span class="material-symbols-rounded" aria-hidden="true">playlist_play</span>
      </button>
    </Transition>
  </Teleport>
</template>

<style scoped lang="scss">
.locate-fab {
  position: fixed;
  right: 24px;
  bottom: calc(var(--mini-player-height, 76px) + 24px);
  /* MiniPlayer(100) 之下，NowPlaying(200)/队列面板(250) 打开时被覆盖 */
  z-index: 90;
  width: 48px;
  height: 48px;
  border: none;
  border-radius: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--md-primary-container);
  color: var(--md-on-primary-container);
  cursor: pointer;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.28);
  transition:
    background var(--duration-short) var(--ease-standard),
    box-shadow var(--duration-short) var(--ease-standard),
    transform 120ms var(--ease-emphasized, cubic-bezier(0.2, 0, 0, 1));

  .material-symbols-rounded {
    font-size: 24px;
  }

  &:hover {
    background: color-mix(in srgb, var(--md-on-primary-container) 8%, var(--md-primary-container));
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.32);
  }

  &:active {
    transform: scale(0.94);
  }

  &:focus-visible {
    outline: 2px solid var(--md-primary);
    outline-offset: 2px;
  }
}

/* 出现/消失 200ms 过渡 */
.locate-fab-enter-active,
.locate-fab-leave-active {
  transition:
    opacity 200ms var(--ease-standard),
    transform 200ms var(--ease-standard);
}

.locate-fab-enter-from,
.locate-fab-leave-to {
  opacity: 0;
  transform: translateY(10px) scale(0.85);
}

@media (prefers-reduced-motion: reduce) {
  .locate-fab-enter-active,
  .locate-fab-leave-active {
    transition-duration: 1ms;
  }
}
</style>
