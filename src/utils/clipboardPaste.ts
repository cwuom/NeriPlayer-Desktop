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

type TextSelection =
  | { kind: 'input'; start: number; end: number }
  | { kind: 'range'; range: Range }

/** Input types that support selectionStart/selectionEnd and are safe to edit. */
const SELECTABLE_INPUT_TYPES = new Set(['text', 'search', 'url', 'tel', 'password'])

function editableTarget(target: EventTarget | null): Editable | null {
  if (target instanceof HTMLInputElement) {
    return SELECTABLE_INPUT_TYPES.has(target.type) && !target.readOnly ? target : null
  }
  if (target instanceof HTMLTextAreaElement) return target.readOnly ? null : target
  if (target instanceof HTMLElement && target.isContentEditable) return target
  return null
}

function captureSelection(target: Editable): TextSelection | null {
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) {
    const start = target.selectionStart
    const end = target.selectionEnd
    if (start === null || end === null) return null
    return { kind: 'input', start, end }
  }
  const selection = window.getSelection()
  if (!selection || selection.rangeCount !== 1) return null
  const range = selection.getRangeAt(0)
  if (!target.contains(range.commonAncestorContainer)) return null
  return { kind: 'range', range: range.cloneRange() }
}

function selectionUnchanged(target: Editable, captured: TextSelection): boolean {
  if (document.activeElement !== target) return false
  if (captured.kind === 'input') {
    if (!(target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement)) return false
    return target.selectionStart === captured.start && target.selectionEnd === captured.end
  }
  const selection = window.getSelection()
  if (!selection || selection.rangeCount !== 1) return false
  const current = selection.getRangeAt(0)
  const saved = captured.range
  return current.startContainer === saved.startContainer
    && current.startOffset === saved.startOffset
    && current.endContainer === saved.endContainer
    && current.endOffset === saved.endOffset
}

function insertNumericText(target: HTMLInputElement, text: string): void {
  const value = text.trim()
  // Number inputs have no selection API. Accept only a complete number and
  // replace the whole value; do not let the number-input setter silently clear
  // an invalid paste.
  if (!/^[+-]?(?:\d+\.?\d*|\.\d+)$/.test(value)) return
  target.value = value
  target.dispatchEvent(new Event('input', { bubbles: true }))
}

function insertPlainText(target: Editable, text: string, captured: TextSelection): void {
  if (captured.kind === 'input') {
    if (!(target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement)) return
    target.setRangeText(text, captured.start, captured.end, 'end')
    target.dispatchEvent(new Event('input', { bubbles: true }))
    return
  }
  target.focus()
  const selection = window.getSelection()
  if (!selection) return
  selection.removeAllRanges()
  selection.addRange(captured.range)
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
    const rawTarget = event.target
    if (rawTarget instanceof HTMLInputElement && rawTarget.type === 'number' && !rawTarget.readOnly) {
      const target = rawTarget
      void readText()
        .then((text) => {
          if (!text || document.activeElement !== target) return
          insertNumericText(target, text)
        })
        .catch(() => {
          // 没有剪贴板权限时保持原样
        })
      return
    }
    const target = editableTarget(rawTarget)
    if (!target) return
    const captured = captureSelection(target)
    if (!captured) return
    void readText()
      .then((text) => {
        if (!text || !selectionUnchanged(target, captured)) return
        insertPlainText(target, text, captured)
      })
      .catch(() => {
        // 没有剪贴板权限时保持原样，不插入任何内容
      })
  }
  document.addEventListener('paste', onPaste, true)
  return () => document.removeEventListener('paste', onPaste, true)
}
