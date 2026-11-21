// 空白处不需要 WebView 自带的后退、刷新、打印、另存为菜单
// 只在输入框和选中的文字上保留它（剪切、复制、粘贴）；其余位置由各组件的自绘菜单接管，没有就不弹

const NON_TEXT_INPUT_TYPES = new Set(['checkbox', 'radio', 'range', 'button', 'submit', 'reset', 'color', 'file', 'image', 'hidden'])

function hasTextSelectionAt(target: Element): boolean {
  const selection = window.getSelection()
  if (!selection || selection.isCollapsed || !selection.toString().trim()) return false
  for (let index = 0; index < selection.rangeCount; index++) {
    if (selection.getRangeAt(index).intersectsNode(target)) return true
  }
  return false
}

export function allowsNativeContextMenu(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false
  const field = target.closest('input, textarea')
  if (field?.tagName === 'TEXTAREA') return true
  if (field?.tagName === 'INPUT') return !NON_TEXT_INPUT_TYPES.has((field as HTMLInputElement).type)
  if (target instanceof HTMLElement && target.isContentEditable) return true
  if (target === document.body || target === document.documentElement) return false
  return hasTextSelectionAt(target)
}

export function installNativeContextMenuPolicy(): () => void {
  const handler = (event: MouseEvent) => {
    if (event.defaultPrevented) return
    // 开发构建按住 Shift 右键仍能打开检查元素
    if (import.meta.env.DEV && event.shiftKey) return
    if (!allowsNativeContextMenu(event.target)) event.preventDefault()
  }
  window.addEventListener('contextmenu', handler)
  return () => window.removeEventListener('contextmenu', handler)
}
