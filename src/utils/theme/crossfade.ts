// 主题切换过渡：整屏柔和交叉淡化（新画面在旧快照上淡入），接近 macOS 切换外观的观感。
// 以前的硬边圆形扩散在桌面大窗口上显得突兀，且两套动画抢同一伪元素导致卡顿。
// 只动 ::view-transition-new 的 opacity，合成线程即可完成，不触发重排

export const THEME_CROSSFADE_MS = 320
export const THEME_CROSSFADE_EASING = 'cubic-bezier(0.2, 0, 0, 1)'

/** 在 View Transition 中执行 update 并淡入新画面；返回时已清理所有临时类名 */
export async function runThemeCrossfade(update: () => void, root: HTMLElement = document.documentElement): Promise<void> {
  root.classList.add('theme-ripple-active')
  const transition = document.startViewTransition(update)
  try {
    await transition.ready
    let anim: Animation | null = null
    try {
      anim = root.animate(
        { opacity: [0, 1] },
        {
          duration: THEME_CROSSFADE_MS,
          easing: THEME_CROSSFADE_EASING,
          fill: 'forwards',
          pseudoElement: '::view-transition-new(root)',
        },
      )
    } catch {
      anim = null
    }
    // 不支持伪元素 WAAPI 的 WebView 退回 CSS 关键帧
    if (!anim) root.classList.add('theme-ripple-css')
    const deadline = new Promise<void>(resolve => setTimeout(resolve, THEME_CROSSFADE_MS + 40))
    await (anim ? Promise.race([anim.finished.catch(() => {}), deadline]) : deadline)
    await transition.finished.catch(() => {})
  } catch {
    // transition 被中断（例如连续切换）也没关系，新状态已经写入
  } finally {
    root.classList.remove('theme-ripple-active', 'theme-ripple-css')
  }
}
