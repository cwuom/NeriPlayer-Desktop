// 放在 utils 层：主题工具、快捷键模块和入口都要按平台分支，不能反向依赖 modules

export type DesktopPlatform = 'mac' | 'windows' | 'linux'

function rawPlatform(): string {
  // 模块加载时就会调用；Node 20 等非浏览器环境没有全局 navigator
  if (typeof navigator === 'undefined') return ''
  const data = (navigator as unknown as { userAgentData?: { platform?: string } }).userAgentData
  return (data?.platform || navigator.platform || navigator.userAgent || '').toLowerCase()
}

export function detectPlatform(): DesktopPlatform {
  const value = rawPlatform()
  if (/mac|iphone|ipad|darwin/.test(value)) return 'mac'
  if (/win/.test(value)) return 'windows'
  return 'linux'
}
