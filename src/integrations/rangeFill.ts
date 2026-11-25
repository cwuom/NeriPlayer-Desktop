// 给所有 .m3-slider 同步 --slider-fill（已填充比例），让轨道左侧显示强调色进度。
// 原生 range 在 WebKit/Chromium 下没有可单独着色的“已填充段”，只能由脚本算比例。
// 触发点：拖动（input）、点击后（重置按钮等改值）、新滑块挂载；不轮询

const SELECTOR = 'input[type="range"].m3-slider'

export function sliderFillPercent(value: number, min: number, max: number): number {
  if (!Number.isFinite(value) || !Number.isFinite(min) || !Number.isFinite(max) || max <= min) return 0
  return Math.min(100, Math.max(0, ((value - min) / (max - min)) * 100))
}

function update(input: HTMLInputElement) {
  const percent = sliderFillPercent(Number(input.value), Number(input.min || 0), Number(input.max || 100))
  const next = `${Math.round(percent * 100) / 100}%`
  if (input.style.getPropertyValue('--slider-fill') !== next) input.style.setProperty('--slider-fill', next)
}

export function installRangeFill(root: Document = document): () => void {
  let frame = 0
  const syncAll = () => {
    frame = 0
    root.querySelectorAll<HTMLInputElement>(SELECTOR).forEach(update)
  }
  const schedule = () => {
    if (!frame) frame = requestAnimationFrame(syncAll)
  }
  const onInput = (event: Event) => {
    const target = event.target
    if (target instanceof HTMLInputElement && target.matches(SELECTOR)) update(target)
  }
  // 点击（重置、加减按钮）和键盘改值后，等 Vue 更新完再统一同步一次
  const onAfterInteraction = () => schedule()
  const observer = new MutationObserver(schedule)
  observer.observe(root.body, { childList: true, subtree: true })
  root.addEventListener('input', onInput, true)
  root.addEventListener('click', onAfterInteraction, true)
  root.addEventListener('keyup', onAfterInteraction, true)
  schedule()
  return () => {
    observer.disconnect()
    root.removeEventListener('input', onInput, true)
    root.removeEventListener('click', onAfterInteraction, true)
    root.removeEventListener('keyup', onAfterInteraction, true)
    if (frame) cancelAnimationFrame(frame)
  }
}
