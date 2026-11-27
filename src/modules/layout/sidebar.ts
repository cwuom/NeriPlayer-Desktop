/** 窗口窄于此宽度时侧栏自动收成图标栏，内容区至少留出约 640px */
export const SIDEBAR_AUTO_RAIL_WIDTH = 900

const STORAGE_KEY = 'sidebar-collapsed'

export function readSidebarCollapsed(storage: Pick<Storage, 'getItem'> | undefined = globalThis.localStorage): boolean {
  try {
    return storage?.getItem(STORAGE_KEY) === '1'
  } catch {
    return false
  }
}

export function writeSidebarCollapsed(collapsed: boolean, storage: Pick<Storage, 'setItem'> | undefined = globalThis.localStorage): void {
  try {
    storage?.setItem(STORAGE_KEY, collapsed ? '1' : '0')
  } catch {
    // 隐私模式等存储不可用时只影响偏好记忆，不影响折叠本身
  }
}
