// 侧栏尺寸与折叠偏好：SideNav 与依赖侧栏宽度的浮层（迷你播放器等）共用
export const SIDEBAR_EXPANDED_WIDTH = 228
export const SIDEBAR_RAIL_WIDTH = 68
/** 窗口窄于此宽度时侧栏自动收成图标栏，内容区至少留出约 640px */
export const SIDEBAR_AUTO_RAIL_WIDTH = 900
/** macOS 红绿灯右缘（含间距）离窗口左边的距离；侧栏比它窄时内容区顶栏要让开 */
export const MAC_TRAFFIC_LIGHT_REACH = 92

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
