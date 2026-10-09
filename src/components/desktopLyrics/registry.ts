import type { InjectionKey } from 'vue'

export interface KaraokeHandle {
  update(timeMs: number): void
}

/** 舞台每帧驱动已登记的歌词行；返回值用于注销 */
export const KARAOKE_REGISTRY: InjectionKey<(handle: KaraokeHandle) => () => void> = Symbol('desktop-lyrics-karaoke')

/** 暂停时舞台会停表；行重新测量后要靠它补画一帧 */
export const KARAOKE_WAKE: InjectionKey<() => void> = Symbol('desktop-lyrics-karaoke-wake')
