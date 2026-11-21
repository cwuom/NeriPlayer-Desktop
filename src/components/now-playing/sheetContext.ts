import { inject, provide, type InjectionKey } from 'vue'
import type { NowPlayingSheetContext } from './types'

const nowPlayingSheetContextKey: InjectionKey<NowPlayingSheetContext> = Symbol('now-playing-sheet')

export function provideNowPlayingSheetContext(context: NowPlayingSheetContext) {
  provide(nowPlayingSheetContextKey, context)
}

/** 面板只在 NowPlaying.vue 里渲染，缺少上下文说明挂错了位置，直接报错比静默失效好查 */
export function useNowPlayingSheetContext(): NowPlayingSheetContext {
  const context = inject(nowPlayingSheetContextKey)
  if (!context) throw new Error('NowPlaying more sheet must be rendered inside NowPlaying')
  return context
}
