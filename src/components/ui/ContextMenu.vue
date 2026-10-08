<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { CSSProperties } from 'vue'
import {
  clampContextMenuPosition,
  isContextMenuSeparator,
  type ContextMenuActionItem,
  type ContextMenuItem,
  type ContextMenuPosition,
} from '@/utils/contextMenu'
import { useEscapeClose } from '@/composables/useEscapeClose'

const props = defineProps<{
  open: boolean
  x: number
  y: number
  items: readonly ContextMenuItem[]
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
  close: []
  click: [item: ContextMenuActionItem]
}>()

const menuRef = ref<HTMLElement | null>(null)
const menuPosition = ref<ContextMenuPosition>({ x: 0, y: 0 })
const isPositioned = ref(false)
let resizeObserver: ResizeObserver | null = null

const menuStyle = computed<CSSProperties>(() => ({
  left: `${menuPosition.value.x}px`,
  top: `${menuPosition.value.y}px`,
  visibility: isPositioned.value ? 'visible' : 'hidden',
}))

function isActionItem(item: ContextMenuItem): item is ContextMenuActionItem {
  return !isContextMenuSeparator(item)
}

function itemKey(item: ContextMenuItem, index: number) {
  const kind = isContextMenuSeparator(item) ? 'separator' : 'item'
  return `${kind}-${item.id ?? index}-${index}`
}

function itemLabel(item: ContextMenuItem) {
  return isActionItem(item) ? item.label : ''
}

function itemIcon(item: ContextMenuItem) {
  return isActionItem(item) ? item.icon : undefined
}

function itemShortcut(item: ContextMenuItem) {
  return isActionItem(item) ? item.shortcut : undefined
}

function itemIsDisabled(item: ContextMenuItem) {
  return isActionItem(item) && item.disabled === true
}

function itemIsDanger(item: ContextMenuItem) {
  return isActionItem(item) && item.danger === true
}

function updatePosition() {
  const menu = menuRef.value
  if (!menu) return

  menuPosition.value = clampContextMenuPosition(
    props.x,
    props.y,
    menu.offsetWidth,
    menu.offsetHeight,
    {
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
    },
  )
  isPositioned.value = true
}

function getFocusableItems() {
  return menuRef.value
    ? Array.from(menuRef.value.querySelectorAll<HTMLButtonElement>(
        '[role="menuitem"]:not(:disabled)',
      ))
    : []
}

function focusItem(index: number) {
  const items = getFocusableItems()
  const item = items[index]
  if (item) item.focus({ preventScroll: true })
}

function focusFirstItem() {
  const items = getFocusableItems()
  if (items.length > 0) {
    items[0].focus({ preventScroll: true })
    return
  }
  menuRef.value?.focus({ preventScroll: true })
}

function moveFocus(direction: 1 | -1) {
  const items = getFocusableItems()
  if (items.length === 0) return

  const activeElement = document.activeElement
  const currentIndex = activeElement instanceof HTMLButtonElement
    ? items.indexOf(activeElement)
    : -1
  const nextIndex = currentIndex < 0
    ? direction > 0 ? 0 : items.length - 1
    : (currentIndex + direction + items.length) % items.length

  focusItem(nextIndex)
}

function handleMenuKeydown(event: KeyboardEvent) {
  switch (event.key) {
    case 'ArrowDown':
      event.preventDefault()
      event.stopPropagation()
      moveFocus(1)
      break
    case 'ArrowUp':
      event.preventDefault()
      event.stopPropagation()
      moveFocus(-1)
      break
    case 'Home':
      event.preventDefault()
      event.stopPropagation()
      focusItem(0)
      break
    case 'End': {
      event.preventDefault()
      event.stopPropagation()
      const items = getFocusableItems()
      focusItem(items.length - 1)
      break
    }
  }
}

function close() {
  emit('update:open', false)
  emit('close')
}

useEscapeClose(() => props.open, close)

function handleOverlayClick(event: MouseEvent) {
  if (event.target === event.currentTarget) close()
}

function handleOverlayContextMenu(event: MouseEvent) {
  if (event.target !== event.currentTarget) return
  event.preventDefault()
  close()
}

function selectItem(item: ContextMenuItem) {
  if (!isActionItem(item) || item.disabled) return
  emit('click', item)
  close()
}

function disconnectResizeObserver() {
  resizeObserver?.disconnect()
  resizeObserver = null
}

function observeMenuSize() {
  disconnectResizeObserver()
  if (!menuRef.value || typeof ResizeObserver === 'undefined') return
  resizeObserver = new ResizeObserver(updatePosition)
  resizeObserver.observe(menuRef.value)
}

function handleViewportChange() {
  if (props.open) updatePosition()
}

watch(
  () => props.open,
  async (open) => {
    if (!open) {
      isPositioned.value = false
      disconnectResizeObserver()
      return
    }

    menuPosition.value = { x: props.x, y: props.y }
    isPositioned.value = false
    await nextTick()
    updatePosition()
    observeMenuSize()
    focusFirstItem()
  },
  { immediate: true },
)

watch(
  () => [props.x, props.y, props.items],
  async () => {
    if (!props.open) return
    await nextTick()
    updatePosition()
  },
  { deep: true },
)

onMounted(() => {
  window.addEventListener('resize', handleViewportChange)
  window.addEventListener('scroll', handleViewportChange, true)
})

onBeforeUnmount(() => {
  window.removeEventListener('resize', handleViewportChange)
  window.removeEventListener('scroll', handleViewportChange, true)
  disconnectResizeObserver()
})
</script>

<template>
  <Teleport to="body">
    <Transition name="context-menu">
      <div
        v-if="open"
        class="context-menu-overlay"
        @click="handleOverlayClick"
        @contextmenu="handleOverlayContextMenu"
      >
        <div
          ref="menuRef"
          class="context-menu-panel"
          :style="menuStyle"
          role="menu"
          tabindex="-1"
          aria-orientation="vertical"
          @click.stop
          @contextmenu.stop.prevent
          @keydown="handleMenuKeydown"
        >
          <template v-for="(item, index) in items" :key="itemKey(item, index)">
            <div v-if="isContextMenuSeparator(item)" class="context-menu-separator" role="separator" />
            <button
              v-else
              class="context-menu-item"
              :class="{ danger: itemIsDanger(item) }"
              :disabled="itemIsDisabled(item)"
              :aria-disabled="itemIsDisabled(item) || undefined"
              :tabindex="itemIsDisabled(item) ? -1 : 0"
              type="button"
              role="menuitem"
              @click="selectItem(item)"
            >
              <span
                class="context-menu-item__icon material-symbols-rounded"
                :class="{ empty: !itemIcon(item) }"
                aria-hidden="true"
              >{{ itemIcon(item) }}</span>
              <span class="context-menu-item__label">{{ itemLabel(item) }}</span>
              <span v-if="itemShortcut(item)" class="context-menu-item__shortcut">
                {{ itemShortcut(item) }}
              </span>
            </button>
          </template>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped lang="scss">
.context-menu-overlay {
  position: fixed;
  inset: 0;
  z-index: 1000;
}

.context-menu-panel {
  position: fixed;
  box-sizing: border-box;
  min-width: 200px;
  max-width: min(320px, calc(100vw - 16px));
  max-height: calc(100vh - 16px);
  max-height: calc(100dvh - 16px);
  overflow-y: auto;
  padding: 4px 0;
  border: 1px solid var(--md-outline-variant);
  border-radius: var(--radius-md, 12px);
  background: var(--md-surface-container-high);
  box-shadow:
    0 8px 32px rgba(0, 0, 0, 0.28),
    0 2px 8px rgba(0, 0, 0, 0.15);
  transform-origin: top left;
}

.context-menu-item {
  display: grid;
  grid-template-columns: 24px minmax(0, 1fr) auto;
  align-items: center;
  gap: 10px;
  width: calc(100% - 8px);
  min-height: 40px;
  margin: 0 4px;
  padding: 8px 12px;
  border: 0;
  border-radius: var(--radius-sm, 8px);
  background: transparent;
  color: var(--md-on-surface);
  font: inherit;
  font-size: 14px;
  font-weight: 500;
  line-height: 20px;
  text-align: left;
  cursor: pointer;
  transition: background 120ms ease, color 120ms ease, opacity 120ms ease;

  &:hover,
  &:focus-visible {
    background: var(--md-surface-container-highest);
  }

  &:focus-visible {
    outline: 2px solid var(--md-primary);
    outline-offset: -2px;
  }

  &.danger {
    color: var(--md-error);

    &:hover,
    &:focus-visible {
      background: color-mix(in srgb, var(--md-error) 10%, transparent);
    }
  }

  &:disabled {
    opacity: 0.42;
    cursor: default;
  }

  &:disabled:hover {
    background: transparent;
  }
}

.context-menu-item__icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  font-size: 20px;
  color: currentColor;

  &.empty {
    visibility: hidden;
  }
}

.context-menu-item__label {
  min-width: 0;
  overflow-wrap: anywhere;
}

.context-menu-item__shortcut {
  margin-left: 8px;
  color: var(--md-on-surface-variant);
  font-size: 12px;
  font-weight: 400;
  line-height: 18px;
  white-space: nowrap;
}

.context-menu-separator {
  height: 1px;
  margin: 4px 12px;
  background: var(--md-outline-variant);
  opacity: 0.7;
}

.context-menu-enter-active {
  transition: opacity 120ms ease;

  .context-menu-panel {
    transition: opacity 120ms ease, transform 120ms cubic-bezier(0.05, 0.7, 0.1, 1);
  }
}

.context-menu-leave-active {
  transition: opacity 80ms ease;

  .context-menu-panel {
    transition: opacity 80ms ease, transform 80ms ease;
  }
}

.context-menu-enter-from,
.context-menu-leave-to {
  opacity: 0;

  .context-menu-panel {
    opacity: 0;
    transform: scale(0.96) translateY(-4px);
  }
}
</style>
