import { invoke } from '@tauri-apps/api/core'
import { detectPlatform } from '@/modules/shortcuts/platform'

/** 是否 Linux 桌面（Tauri 三端之一）喵 */
export function isLinuxDesktop(): boolean {
  return detectPlatform() === 'linux'
}

/**
 * 认领“首次运行总提示”的机会：第一次返回 `true`，之后（跨重启）一直 `false`。
 *
 * Linux 端整条渲染栈都建立在 WebKitGTK 上，2.54 还整体换过合成器：页面切换可能闪、
 * 性能也可能明显差；动态背景的闪烁更是光敏性癫痫风险。所以第一次运行时给一次总提示，
 * 并把动态背景默认关掉，标记用应用数据目录里的空文件（`create_new`）记录，不碰设置
 * 快照与同步信封。探测失败按“已提示过”处理，下次启动重试。
 *
 * Claim the one-and-only first-run notice: the first call returns `true`, later calls
 * (across restarts) return `false`. The whole Linux rendering stack sits on WebKitGTK,
 * which swapped its compositor in 2.54: page switches can flash, performance can be
 * poor, and the dynamic background's flicker is a photosensitivity risk. The first run
 * therefore shows one notice and disables the background, recorded with an empty marker
 * file created via `create_new` so the settings snapshot and its sync envelope stay
 * untouched. A failed call counts as "already shown" and is retried on the next start.
 */
export function claimFirstRunLinuxNotice(): Promise<boolean> {
  return invoke<boolean>('claim_linux_first_run_notice').catch(() => false)
}
