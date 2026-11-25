// 在弹窗里拖选文字、拖动滑块后把鼠标松在遮罩上时，浏览器会把 click 派发给两者的共同祖先
// （也就是遮罩），点遮罩关闭的弹窗于是被意外关掉（#53：手动搜歌词时选中文字就被关掉）。
// 这里统一拦下“按下与松开不在同一元素、且明显拖动过”的合成 click，正常点击不受影响。

const DRAG_THRESHOLD_PX = 6

export function installDragClickGuard(target: Window = window): () => void {
  let downTarget: EventTarget | null = null
  let downX = 0
  let downY = 0

  const onPointerDown = (event: PointerEvent) => {
    downTarget = event.target
    downX = event.clientX
    downY = event.clientY
  }

  const onClick = (event: MouseEvent) => {
    const origin = downTarget
    downTarget = null
    if (!origin || origin === event.target) return
    // 键盘触发的 click 没有坐标位移，也不会经过 pointerdown，不受影响
    if (event.detail === 0) return
    const moved = Math.hypot(event.clientX - downX, event.clientY - downY)
    if (moved < DRAG_THRESHOLD_PX) return
    if (!(event.target instanceof Node) || !(origin instanceof Node) || !event.target.contains(origin)) return
    event.stopImmediatePropagation()
    event.preventDefault()
  }

  target.addEventListener('pointerdown', onPointerDown, true)
  target.addEventListener('click', onClick, true)
  return () => {
    target.removeEventListener('pointerdown', onPointerDown, true)
    target.removeEventListener('click', onClick, true)
  }
}
