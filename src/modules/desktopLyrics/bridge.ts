// 依赖 store 的发帧桥在 integrations/desktopLyrics 安装后把控制器登记到这里，
// 界面层仍从模块层开关窗口，模块层不必反向依赖 store
import { ref, type Ref } from 'vue'

export interface DesktopLyricsController {
  open: () => Promise<void>
  close: () => Promise<void>
}

let controller: DesktopLyricsController | null = null

/** 桌面歌词窗口是否开着；设置页和「更多」面板据此显示开关 */
export const desktopLyricsOpen: Ref<boolean> = ref(false)

export function hasDesktopLyricsController(): boolean {
  return controller !== null
}

export function setDesktopLyricsController(next: DesktopLyricsController): void {
  controller = next
}

/** 只释放自己登记的控制器，避免旧桥的清理把新桥注销掉 */
export function releaseDesktopLyricsController(owner: DesktopLyricsController): void {
  if (controller === owner) controller = null
}

export async function openDesktopLyricsWindow(): Promise<void> {
  if (!controller) throw new Error('Desktop lyrics bridge is not ready')
  await controller.open()
}

export async function closeDesktopLyricsWindow(): Promise<void> {
  await controller?.close()
}
