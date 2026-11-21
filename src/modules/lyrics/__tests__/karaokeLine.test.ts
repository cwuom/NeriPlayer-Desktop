// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { LyricWord } from '@/types/track'
import { KaraokeLine } from '@/modules/lyrics/karaokeLine'

const logger = vi.hoisted(() => ({ warn: vi.fn<(...args: unknown[]) => void>() }))

vi.mock('@/utils/logger', () => ({
  createLogger: () => ({
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: logger.warn,
    error: vi.fn(),
  }),
}))

const LINE_START = 10_000
const CHAR_WIDTH_PX = 10
const LINE_HEIGHT_PX = 20
/** KaraokeLine 的渐变宽度取行高的一半 */
const FADE_PX = LINE_HEIGHT_PX / 2

/** 手动驱动的 requestAnimationFrame，用来区分「已构建」和「已测量」两个阶段 */
class FrameQueue {
  private nextId = 1
  private readonly callbacks = new Map<number, FrameRequestCallback>()
  readonly cancelled: number[] = []
  lastId = 0

  readonly request = (callback: FrameRequestCallback): number => {
    const id = this.nextId++
    this.lastId = id
    this.callbacks.set(id, callback)
    return id
  }

  readonly cancel = (id: number): void => {
    this.cancelled.push(id)
    this.callbacks.delete(id)
  }

  get pending(): number {
    return this.callbacks.size
  }

  take(id: number): FrameRequestCallback | undefined {
    return this.callbacks.get(id)
  }

  flush(): void {
    const callbacks = [...this.callbacks.values()]
    this.callbacks.clear()
    for (const callback of callbacks) callback(0)
  }
}

interface AnimationRecord {
  target: Element
  keyframes: Keyframe[]
  options: KeyframeAnimationOptions
  animation: Animation
}

let frames: FrameQueue
let records: AnimationRecord[]
let line: KaraokeLine
let container: HTMLDivElement

/**
 * 规范要求 cancel() 拒绝的 finished promise 标记为已处理，happy-dom 没有这样做，
 * 会在 dispose() 时产生大量未处理的 AbortError；这里拦截 finished 的每次重建并补上处理
 */
function markFinishedPromisesHandled(animation: Animation): void {
  const ignore = () => {}
  let finished = animation.finished
  finished.catch(ignore)
  Object.defineProperty(animation, 'finished', {
    configurable: true,
    get: () => finished,
    set: (next: Promise<Animation>) => {
      finished = next
      next.catch(ignore)
    },
  })
}

function installAnimateRecorder(): void {
  const original = Element.prototype.animate
  vi.spyOn(Element.prototype, 'animate').mockImplementation(function (this: Element, keyframes, options) {
    const animation = original.call(this, keyframes, options)
    markFinishedPromisesHandled(animation)
    const timing: KeyframeAnimationOptions = typeof options === 'object' && options ? options : {}
    // happy-dom 不会把 options.id 写回 Animation，这里按 Web Animations 规范补齐
    if (timing.id) animation.id = timing.id
    records.push({
      target: this,
      keyframes: Array.isArray(keyframes) ? keyframes : [],
      options: timing,
      animation,
    })
    return animation
  })
}

/** happy-dom 没有布局，按字符数给出稳定的宽高，便于推算遮罩位置 */
function installLayoutStub(): void {
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(function (this: HTMLElement) {
    return (this.textContent?.length ?? 0) * CHAR_WIDTH_PX
  })
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(LINE_HEIGHT_PX)
}

function word(text: string, startMs: number, durationMs: number): LyricWord {
  return { text, startMs, durationMs }
}

function build(words: LyricWord[], lineEnd: number, fallbackText = '', lineStart = LINE_START): void {
  line.build(container, words, lineStart, lineEnd, fallbackText)
}

function wordSpans(): HTMLSpanElement[] {
  return Array.from(container.querySelectorAll<HTMLSpanElement>('.kw'))
}

function spanFor(text: string): HTMLSpanElement {
  const span = wordSpans().find(element => element.textContent === text)
  if (!span) throw new Error(`no word span for ${text}`)
  return span
}

function recordFor(target: Element, idPrefix: string): AnimationRecord {
  const record = records.find(item => item.target === target && (item.options.id ?? '').startsWith(idPrefix))
  if (!record) throw new Error(`no ${idPrefix} animation`)
  return record
}

function floatOf(text: string): Animation {
  return recordFor(spanFor(text), 'float-word').animation
}

function maskOf(text: string): AnimationRecord {
  return recordFor(spanFor(text), 'fade-word')
}

function maskAnimations(): Animation[] {
  return records.filter(item => (item.options.id ?? '').startsWith('fade-word')).map(item => item.animation)
}

function currentTime(animation: Animation): number {
  return Number(animation.currentTime)
}

/** 运行中的动画 currentTime 跟随 performance.now() 漂移，只能比较近似值 */
function expectTimeNear(animation: Animation, expected: number): void {
  expect(Math.abs(currentTime(animation) - expected)).toBeLessThan(50)
}

function maskAlpha(name: 'bright' | 'dark'): number {
  return Number(container.style.getPropertyValue(`--${name}-mask-alpha`))
}

/** 两个词、总长 2s 的标准行：ab 0-0.5s，cdef 0.5-2s（会被强调） */
function buildStandardLine(fallbackText = 'abcdef'): void {
  build([word('ab', LINE_START, 500), word('cdef', LINE_START + 500, 1_500)], LINE_START + 2_000, fallbackText)
}

beforeEach(() => {
  frames = new FrameQueue()
  records = []
  vi.stubGlobal('requestAnimationFrame', frames.request)
  vi.stubGlobal('cancelAnimationFrame', frames.cancel)
  installAnimateRecorder()
  installLayoutStub()
  container = document.createElement('div')
  document.body.appendChild(container)
  line = new KaraokeLine()
})

afterEach(() => {
  line.dispose()
  document.body.replaceChildren()
  vi.unstubAllGlobals()
})

describe('KaraokeLine building', () => {
  it('renders one hidden span per word and reveals them after the first frame measures them', () => {
    build([word('Hel', LINE_START, 300), word('lo', LINE_START + 300, 300)], LINE_START + 600)

    expect(wordSpans().map(span => span.textContent)).toEqual(['Hel', 'lo'])
    expect(wordSpans().every(span => span.style.visibility === 'hidden')).toBe(true)
    expect(maskAlpha('bright')).toBe(1)
    expect(maskAlpha('dark')).toBe(1)

    frames.flush()
    expect(wordSpans().every(span => span.style.visibility === '')).toBe(true)
  })

  it('keeps whitespace-only words as plain text between word spans', () => {
    build([
      word('Hello', LINE_START, 400),
      word(' ', LINE_START + 400, 0),
      word('world', LINE_START + 400, 400),
    ], LINE_START + 800)

    expect(wordSpans()).toHaveLength(2)
    expect(container.childNodes[1].nodeType).toBe(Node.TEXT_NODE)
    expect(container.textContent).toBe('Hello world')
  })

  it('restores spaces from the line text when the timed words omit them', () => {
    build([word('Hello', LINE_START, 400), word('world', LINE_START + 400, 400)], LINE_START + 800, ' Hello  world ')

    expect(container.textContent).toBe(' Hello  world ')
    expect(wordSpans().map(span => span.textContent)).toEqual(['Hello', 'world'])
  })

  it('does not invent whitespace when the line text disagrees with the words', () => {
    build([word('Hello', LINE_START, 400), word('world', LINE_START + 400, 400)], LINE_START + 800, 'Hello, world')

    expect(container.textContent).toBe('Helloworld')
  })

  it('splits long multi-character words into grapheme spans for emphasis', () => {
    build([word('Hi👋🏽', LINE_START, 1_200)], LINE_START + 1_200)

    const span = spanFor('Hi👋🏽')
    expect(span.classList.contains('emphasize')).toBe(true)
    expect(Array.from(span.children, child => child.textContent)).toEqual(['H', 'i', '👋🏽'])
  })

  it('falls back to code point splitting when Intl.Segmenter is unavailable', () => {
    const descriptor = Object.getOwnPropertyDescriptor(Intl, 'Segmenter')
    Object.defineProperty(Intl, 'Segmenter', { value: undefined, configurable: true, writable: true })
    try {
      build([word('Hi👋🏽', LINE_START, 1_200)], LINE_START + 1_200)
    } finally {
      if (descriptor) Object.defineProperty(Intl, 'Segmenter', descriptor)
    }

    expect(Array.from(spanFor('Hi👋🏽').children, child => child.textContent)).toEqual(['H', 'i', '👋', '🏽'])
  })

  it('does not emphasize words of one second or less, nor single characters', () => {
    build([word('abc', LINE_START, 1_000), word('愛', LINE_START + 1_000, 3_000)], LINE_START + 4_000)

    for (const span of wordSpans()) {
      expect(span.classList.contains('emphasize')).toBe(false)
      expect(span.children).toHaveLength(0)
    }
  })

  it('treats word timings that start before the line and fit inside it as line-relative', () => {
    build([word('ab', 0, 500), word('cd', 600, 400)], LINE_START + 1_000)

    expect(recordFor(spanFor('ab'), 'float-word').options.delay).toBe(0)
    expect(recordFor(spanFor('cd'), 'float-word').options.delay).toBe(600)
  })

  it('keeps word timings that are already absolute or do not fit the line', () => {
    build([word('ab', LINE_START - 100, 500), word('cd', LINE_START + 600, 400)], LINE_START + 1_000)
    expect(recordFor(spanFor('ab'), 'float-word').options.delay).toBe(-100)
    expect(recordFor(spanFor('cd'), 'float-word').options.delay).toBe(600)

    records = []
    build([word('ef', 0, 500), word('gh', 4_000, 1_000)], LINE_START + 1_000)
    expect(recordFor(spanFor('ef'), 'float-word').options.delay).toBe(-LINE_START)
  })

  it('floats each word for at least one second', () => {
    build([word('ab', LINE_START, 300), word('cd', LINE_START + 300, 1_500)], LINE_START + 1_800)

    expect(recordFor(spanFor('ab'), 'float-word').options.duration).toBe(1_000)
    expect(recordFor(spanFor('cd'), 'float-word').options.duration).toBe(1_500)
  })

  it('staggers the emphasis glow across characters and lengthens it for the final word', () => {
    build([word('abcd', LINE_START, 1_500), word('xy', LINE_START + 1_500, 300)], LINE_START + 1_800)
    frames.flush()
    const middleGlows = records.filter(item => (item.options.id ?? '').startsWith('emphasize-word-') && item.options.id !== 'emphasize-word-float')
    const middleDelays = middleGlows.map(item => Number(item.options.delay))
    expect(middleGlows).toHaveLength(4)
    expect(middleDelays).toEqual([...middleDelays].sort((a, b) => a - b))
    expect(new Set(middleDelays).size).toBe(4)
    expect(middleGlows.every(item => item.options.duration === 1_500)).toBe(true)

    records = []
    build([word('xy', LINE_START, 300), word('abcd', LINE_START + 300, 1_500)], LINE_START + 1_800)
    frames.flush()
    const finalGlows = records.filter(item => (item.options.id ?? '').startsWith('emphasize-word-') && item.options.id !== 'emphasize-word-float')
    expect(finalGlows.every(item => Number(item.options.duration) > 1_500)).toBe(true)
  })

  it('creates a mask sweep per word that starts fully dark and ends fully lit', () => {
    buildStandardLine()
    frames.flush()

    for (const text of ['ab', 'cdef']) {
      const mask = maskOf(text)
      const offsets = mask.keyframes.map(frame => Number(frame.offset))
      expect(mask.options.duration).toBe(2_000)
      expect(offsets[0]).toBe(0)
      expect(offsets[offsets.length - 1]).toBe(1)
      expect(offsets.every((offset, i) => i === 0 || offset > offsets[i - 1])).toBe(true)
      expect(mask.keyframes[0].maskPosition).toBe(`${-(text.length * CHAR_WIDTH_PX + FADE_PX)}px 0`)
      expect(mask.keyframes[mask.keyframes.length - 1].maskPosition).toBe('0px 0')
    }
    expect(spanFor('ab').style.maskImage).toContain('linear-gradient')
  })

  it('reveals later words only after earlier ones', () => {
    buildStandardLine()
    frames.flush()

    // 最后一个仍处于起始（全暗）位置的关键帧，即开始点亮的时刻
    const startsLighting = (record: AnimationRecord) => {
      const dark = record.keyframes[0].maskPosition
      return Number(record.keyframes.filter(frame => frame.maskPosition === dark).pop()?.offset)
    }
    const fullyLit = (record: AnimationRecord) =>
      Number(record.keyframes.find(frame => frame.maskPosition === '0px 0')?.offset)

    expect(startsLighting(maskOf('cdef'))).toBeGreaterThan(startsLighting(maskOf('ab')))
    expect(fullyLit(maskOf('cdef'))).toBeGreaterThan(fullyLit(maskOf('ab')))
    expect(fullyLit(maskOf('ab'))).toBeLessThan(0.5)
  })

  it('replaces the previous words and cancels their animations when rebuilt', () => {
    buildStandardLine()
    frames.flush()
    const oldAnimations = records.map(item => item.animation)

    build([word('new', LINE_START, 500)], LINE_START + 500)

    expect(wordSpans().map(span => span.textContent)).toEqual(['new'])
    expect(oldAnimations.every(animation => animation.playState === 'idle')).toBe(true)
  })

  it('skips the mask sweep without failing when every word ends before the line starts', () => {
    build([word('ab', 1_000, 500)], 1_000, '', 5_000)
    frames.flush()

    expect(maskAnimations()).toHaveLength(0)
    expect(logger.warn).toHaveBeenCalled()
    expect(spanFor('ab').style.visibility).toBe('')
  })
})

describe('KaraokeLine playback control', () => {
  it('defers enabling until the first frame has measured the words', () => {
    buildStandardLine()
    line.enable(LINE_START + 800)
    expect(records.every(item => item.animation.playState === 'paused')).toBe(true)

    frames.flush()
    const mask = maskOf('ab').animation
    expect(mask.playState).toBe('running')
    expectTimeNear(mask, 800)
  })

  it('positions every animation at the current time without playing when asked not to play', () => {
    buildStandardLine()
    frames.flush()
    line.enable(LINE_START + 800, false)

    expect(maskAnimations().every(animation => animation.playState === 'paused')).toBe(true)
    expect(maskAnimations().map(currentTime)).toEqual([800, 800])
    expect(currentTime(floatOf('ab'))).toBe(800)
  })

  it('plays only the animations that have not finished at the current time', () => {
    buildStandardLine()
    frames.flush()
    line.enable(LINE_START + 1_500)

    expect(floatOf('ab').playState).toBe('paused')
    expect(floatOf('cdef').playState).toBe('running')
    expect(maskOf('cdef').animation.playState).toBe('running')
  })

  it('clamps times before the line start to the beginning', () => {
    buildStandardLine()
    frames.flush()
    line.enable(LINE_START - 5_000, false)

    expect(maskAnimations().map(currentTime)).toEqual([0, 0])
  })

  it('ignores seek until the line is enabled', () => {
    buildStandardLine()
    frames.flush()
    line.seek(LINE_START + 1_200)

    for (const animation of maskAnimations()) {
      expect(animation.playState).toBe('paused')
      expectTimeNear(animation, 0)
    }
  })

  it('seeks the mask sweep and only keeps it playing inside the line', () => {
    buildStandardLine()
    frames.flush()
    line.enable(LINE_START, false)

    line.seek(LINE_START + 1_200)
    expect(maskAnimations().every(animation => animation.playState === 'running')).toBe(true)
    maskAnimations().forEach(animation => expectTimeNear(animation, 1_200))

    line.seek(LINE_START + 9_000)
    expect(maskAnimations().every(animation => animation.playState === 'paused')).toBe(true)
    maskAnimations().forEach(animation => expectTimeNear(animation, 2_000))

    line.seek(LINE_START - 300)
    expect(maskAnimations().every(animation => animation.playState === 'paused')).toBe(true)
    expect(maskAnimations().map(currentTime)).toEqual([0, 0])
  })

  it('pauses everything and resumes only unfinished animations', () => {
    buildStandardLine()
    frames.flush()
    line.enable(LINE_START + 1_500)

    line.pause()
    expect(records.every(item => item.animation.playState !== 'running')).toBe(true)

    line.resume()
    expect(floatOf('ab').playState).toBe('paused')
    expect(floatOf('cdef').playState).toBe('running')
    expect(maskAnimations().every(animation => animation.playState === 'running')).toBe(true)
  })

  it('ignores resume while disabled', () => {
    buildStandardLine()
    frames.flush()
    line.resume()

    expect(records.every(item => item.animation.playState === 'paused')).toBe(true)
  })

  it('rewinds the float animations and freezes the mask sweep when disabled', () => {
    buildStandardLine()
    frames.flush()
    line.enable(LINE_START + 600)

    line.disable()

    for (const text of ['ab', 'cdef']) {
      expect(floatOf(text).playbackRate).toBe(-1)
      expect(floatOf(text).playState).toBe('running')
    }
    expect(maskAnimations().every(animation => animation.playState === 'paused')).toBe(true)

    line.seek(LINE_START + 1_000)
    expect(maskAnimations().every(animation => animation.playState === 'paused')).toBe(true)
  })

  it('drops a pending enable when disabled before the first frame', () => {
    buildStandardLine()
    line.enable(LINE_START + 800)
    line.disable()
    frames.flush()

    expect(maskAnimations().every(animation => animation.playState === 'paused')).toBe(true)
  })
})

describe('KaraokeLine mask alpha', () => {
  it('dims both layers equally in solid mode', () => {
    buildStandardLine()
    line.updateMaskAlpha(1, 0.016, true)
    expect(maskAlpha('bright')).toBeCloseTo(0.4)
    expect(maskAlpha('dark')).toBeCloseTo(0.4)

    line.updateMaskAlpha(0.9, 0.016, true)
    expect(maskAlpha('bright')).toBeCloseTo(0.2)
    expect(maskAlpha('dark')).toBeCloseTo(0.2)
  })

  it('brightens the sung part in dynamic mode according to the line scale', () => {
    buildStandardLine()
    line.enable(LINE_START)

    line.updateMaskAlpha(1, 0.016, true)
    expect(maskAlpha('bright')).toBeCloseTo(1)
    expect(maskAlpha('dark')).toBeCloseTo(0.4)

    line.updateMaskAlpha(0.985, 0.016, true)
    expect(maskAlpha('bright')).toBeCloseTo(0.6)
    expect(maskAlpha('dark')).toBeCloseTo(0.3)
  })

  it('eases towards the target, rising faster than it falls, and settles exactly', () => {
    buildStandardLine()
    line.updateMaskAlpha(0.97, 0.016, true)
    line.enable(LINE_START)

    line.updateMaskAlpha(1)
    const riseFraction = (maskAlpha('bright') - 0.2) / (1 - 0.2)

    line.updateMaskAlpha(1, 0.016, true)
    line.disable()
    line.updateMaskAlpha(0.97)
    const fallFraction = (1 - maskAlpha('bright')) / (1 - 0.2)

    expect(riseFraction).toBeGreaterThan(0)
    expect(riseFraction).toBeLessThan(1)
    expect(fallFraction).toBeGreaterThan(0)
    expect(riseFraction).toBeGreaterThan(fallFraction)

    for (let i = 0; i < 200; i++) line.updateMaskAlpha(0.97)
    expect(container.style.getPropertyValue('--bright-mask-alpha')).toBe('0.200')
    expect(container.style.getPropertyValue('--dark-mask-alpha')).toBe('0.200')
  })
})

describe('KaraokeLine dispose', () => {
  it('restores the plain line text and cancels every animation', () => {
    buildStandardLine('ab cdef')
    frames.flush()
    line.enable(LINE_START + 500)

    line.dispose()

    expect(container.textContent).toBe('ab cdef')
    expect(wordSpans()).toHaveLength(0)
    expect(records.every(item => item.animation.playState === 'idle')).toBe(true)
  })

  it('falls back to the joined word text when no line text was given', () => {
    buildStandardLine('')
    line.dispose()

    expect(container.textContent).toBe('abcdef')
  })

  it('does not write text into a container that is no longer in the document', () => {
    buildStandardLine()
    container.remove()

    line.dispose()
    expect(container.textContent).toBe('')
  })

  it('cancels a pending first frame and ignores it if it still fires', () => {
    buildStandardLine()
    const frameId = frames.lastId
    const staleFrame = frames.take(frameId)

    line.dispose()
    expect(frames.cancelled).toContain(frameId)
    expect(frames.pending).toBe(0)

    staleFrame?.(0)
    expect(maskAnimations()).toHaveLength(0)
  })
})
