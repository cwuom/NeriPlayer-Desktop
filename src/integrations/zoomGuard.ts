// 屏蔽 WebView 自带的页面缩放：触控板双指捏合在 Chromium/WebView2 里会以 ctrl+wheel 送达，
// WebKit 则是 gesture* 事件。之前用户误触捏合后整页被放大/缩小且不会自动恢复，
// 表现为“界面莫名其妙缩放了”。缩放只允许通过设置里的界面缩放调整。

function isZoomWheel(event: WheelEvent): boolean {
  return event.ctrlKey || event.metaKey
}

export function installZoomGuard(target: Window = window): () => void {
  const onWheel = (event: WheelEvent) => {
    if (isZoomWheel(event)) event.preventDefault()
  }
  const onGesture = (event: Event) => event.preventDefault()
  // passive: false 才能在滚轮事件里取消默认缩放
  target.addEventListener('wheel', onWheel, { passive: false, capture: true })
  for (const type of ['gesturestart', 'gesturechange', 'gestureend']) {
    target.addEventListener(type, onGesture, { passive: false })
  }
  return () => {
    target.removeEventListener('wheel', onWheel, { capture: true })
    for (const type of ['gesturestart', 'gesturechange', 'gestureend']) {
      target.removeEventListener(type, onGesture)
    }
  }
}
