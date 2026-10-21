// WebKitGTK 2.54 在 GTK 剪贴板没有 target 时会以 targets=null、n_targets=-1
// 回调 WebKit::Clipboard::formats，随后解引用空指针崩溃（SEGV_MAPERR）。
// 在可编辑控件上接管 paste，改用 Tauri 原生剪贴板读取并手动插入，绕开这条路径。
// WebKitGTK 2.54 calls WebKit::Clipboard::formats with targets=null and
// n_targets=-1 when the GTK clipboard has no targets, then dereferences the null
// pointer. Intercept paste on editable controls, read through the native Tauri
// clipboard, and insert the text ourselves to avoid that path.
import { isTauri } from '@tauri-apps/api/core'
import { readText } from '@tauri-apps/plugin-clipboard-manager'

type Editable = HTMLInputElement | HTMLTextAreaElement | HTMLElement

function editableTarget(target: EventTarget | null): Editable | null {
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return target
  if (target instanceof HTMLElement && target.isContentEditable) return target
  return null
}

function insertPlainText(target: Editable, text: string): void {
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    const start = target.selectionStart ?? target.value.length
    const end = target.selectionEnd ?? start
    try {
      target.setRangeText(text, start, end, 'end')
    } catch {
      target.value += text
    }
    target.dispatchEvent(new Event('input', { bubbles: true }))
    return
  }
  target.focus()
  document.execCommand('insertText', false, text)
}

export function installClipboardPasteGuard(): () => void {
  // 只在 Linux/WebKitGTK 上接管；Windows/macOS 的原生剪贴板没有这个崩溃。
  // Only intercept on Linux/WebKitGTK; native clipboard on Windows/macOS is fine.
  const linux = typeof navigator !== 'undefined' && /Linux/i.test(navigator.userAgent)
  if (!isTauri() || !linux) return () => {}
  const onPaste = (event: ClipboardEvent) => {
    // 即使焦点不在可编辑控件上也阻止默认粘贴，避免 WebKit 去读 GTK targets。
    // Block the default even outside editable controls so WebKit never reads GTK targets.
    event.preventDefault()
    event.stopPropagation()
    const target = editableTarget(event.target)
    if (!target) return
    void readText()
      .then((text) => {
        if (!text || document.activeElement !== target) return
        insertPlainText(target, text)
      })
      .catch(() => {
        // 没有剪贴板权限时保持原样，不插入任何内容
      })
  }
  document.addEventListener('paste', onPaste, true)
  return () => document.removeEventListener('paste', onPaste, true)
}
