import { onActivated, onDeactivated, onMounted, onUnmounted, watch } from 'vue'

interface OverlayEntry {
  isOpen: () => boolean
}

// 所有可被 Escape 关闭的弹层共用一个栈，只关闭最后打开且仍然可见的那一个
const openStack: OverlayEntry[] = []

function topmostOpen(): OverlayEntry | undefined {
  for (let index = openStack.length - 1; index >= 0; index--) {
    if (openStack[index].isOpen()) return openStack[index]
  }
  return undefined
}

/**
 * Escape 关闭弹层：捕获阶段处理并 preventDefault，全局快捷键因此不会再关闭下层或触发播放键。
 * 宿主被 KeepAlive 停用时弹层不可见，暂时让出栈顶。
 */
export function useEscapeClose(isOpen: () => boolean, close: () => void): void {
  let active = true
  const entry: OverlayEntry = { isOpen: () => active && isOpen() }

  function remove() {
    const index = openStack.lastIndexOf(entry)
    if (index >= 0) openStack.splice(index, 1)
  }

  function sync() {
    remove()
    if (entry.isOpen()) openStack.push(entry)
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape' || event.defaultPrevented || topmostOpen() !== entry) return
    event.preventDefault()
    close()
  }

  watch(isOpen, sync, { immediate: true })
  onMounted(() => document.addEventListener('keydown', handleKeydown, true))
  onActivated(() => {
    active = true
    sync()
  })
  onDeactivated(() => {
    active = false
    remove()
  })
  onUnmounted(() => {
    document.removeEventListener('keydown', handleKeydown, true)
    remove()
  })
}
