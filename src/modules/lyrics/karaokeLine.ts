/**
 * KaraokeLine - DOM 逐字渲染
 */

import type { LyricWord } from '@/types/track'
import { createLogger } from '@/utils/logger'

const log = createLogger('karaoke-line')

interface InternalWord {
  source: LyricWord
  word: string
  startTime: number
  endTime: number
}

interface RealWord extends InternalWord {
  mainElement: HTMLSpanElement
  subElements: HTMLSpanElement[]
  elementAnimations: Animation[]
  maskAnimations: Animation[]
  width: number
  height: number
  padding: number
  shouldEmphasize: boolean
}

const ANIMATION_FRAME_QUANTITY = 32
const EMP_EASING_MID = 0.5
const WORD_FADE_WIDTH = 0.5

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value))
}

function clamp01(value: number): number {
  return clamp(value, 0, 1)
}

function clampPositive(value: number): number {
  return Math.max(0, value)
}

function norNum(min: number, max: number): (x: number) => number {
  return (x: number) => clamp01((x - min) / (max - min))
}

function cubicBezier(x1: number, y1: number, x2: number, y2: number): (x: number) => number {
  const cx = 3 * x1
  const bx = 3 * (x2 - x1) - cx
  const ax = 1 - cx - bx
  const cy = 3 * y1
  const by = 3 * (y2 - y1) - cy
  const ay = 1 - cy - by
  const sampleX = (t: number) => ((ax * t + bx) * t + cx) * t
  const sampleY = (t: number) => ((ay * t + by) * t + cy) * t
  const sampleDerivativeX = (t: number) => (3 * ax * t + 2 * bx) * t + cx

  return (x: number) => {
    let t = x
    for (let i = 0; i < 4; i++) {
      const dx = sampleX(t) - x
      const d = sampleDerivativeX(t)
      if (Math.abs(dx) < 1e-6 || Math.abs(d) < 1e-6) break
      t -= dx / d
    }
    return sampleY(clamp01(t))
  }
}

const beginNum = norNum(0, EMP_EASING_MID)
const endNum = norNum(EMP_EASING_MID, 1)
const bezIn = cubicBezier(0.2, 0.4, 0.58, 1.0)
const bezOut = cubicBezier(0.3, 0.0, 0.58, 1.0)

function makeEmpEasing(mid: number): (x: number) => number {
  return (x: number) => (x < mid ? bezIn(beginNum(x)) : 1 - bezOut(endNum(x)))
}

function createMatrix4(): number[] {
  return [
    1, 0, 0, 0,
    0, 1, 0, 0,
    0, 0, 1, 0,
    0, 0, 0, 1,
  ]
}

function scaleMatrix4(matrix: number[], scale: number): number[] {
  const result = [...matrix]
  result[0] *= scale
  result[5] *= scale
  result[10] *= scale
  return result
}

function matrix4ToCSS(matrix: number[], precision = 4): string {
  return `matrix3d(${matrix.map(v => Number(v.toFixed(precision))).join(',')})`
}

function generateFadeGradient(
  width: number,
  padding = 0,
  bright = 'rgba(0,0,0,var(--bright-mask-alpha, 1.0))',
  dark = 'rgba(0,0,0,var(--dark-mask-alpha, 1.0))',
): [string, number] {
  const totalAspect = 2 + width + padding
  const widthInTotal = width / totalAspect
  const leftPos = (1 - widthInTotal) / 2
  return [
    `linear-gradient(to right,${bright} ${leftPos * 100}%,${dark} ${(leftPos + widthInTotal) * 100}%)`,
    totalAspect,
  ]
}

function maskPositionFrame(offset: number, position: string): Keyframe {
  return {
    offset,
    maskPosition: position,
    webkitMaskPosition: position,
  } as Keyframe
}

function cleanKeyframes(frames: Keyframe[]): Keyframe[] {
  const seen = new Map<number, Keyframe>()
  for (const frame of frames) {
    const offset = frame.offset as number
    if (Number.isFinite(offset) && offset >= 0 && offset <= 1) {
      seen.set(Number(offset.toFixed(6)), frame)
    }
  }
  return Array.from(seen.entries())
    .sort(([a], [b]) => a - b)
    .map(([, frame]) => frame)
}

function splitGraphemes(text: string): string[] {
  const SegmenterCtor = (Intl as typeof Intl & {
    Segmenter?: new (
      locales?: string | string[],
      options?: { granularity: 'grapheme' },
    ) => { segment(input: string): Iterable<{ segment: string }> }
  }).Segmenter
  const segmenter = SegmenterCtor
    ? new SegmenterCtor(undefined, { granularity: 'grapheme' })
    : null
  if (!segmenter) return Array.from(text)
  return Array.from(segmenter.segment(text), part => part.segment)
}

function shouldEmphasize(word: InternalWord): boolean {
  return word.endTime - word.startTime > 1000 && word.word.trim().length > 1
}

function normalizeWordTimings(
  lyricWords: LyricWord[],
  lineStart: number,
  lineEnd: number,
): LyricWord[] {
  const timedWords = lyricWords.filter(w => w.durationMs > 0)
  if (timedWords.length === 0) return lyricWords

  const firstWordStart = Math.min(...timedWords.map(w => w.startMs))
  const lastWordEnd = Math.max(...timedWords.map(w => w.startMs + w.durationMs))
  const lineDuration = Math.max(0, lineEnd - lineStart)
  const startsBeforeLine = firstWordStart < lineStart - 250
  const fitsInsideLine = lastWordEnd <= lineDuration + 500

  if (!startsBeforeLine || !fitsInsideLine) return lyricWords

  return lyricWords.map(word => ({
    ...word,
    startMs: lineStart + word.startMs,
  }))
}

function restoreWhitespaceFromLineText(words: LyricWord[], lineText: string): LyricWord[] {
  if (!lineText || !/\s/.test(lineText) || words.some(word => /\s/.test(word.text))) return words

  const compactWords = words.map(word => word.text).join('').replace(/\s+/g, '')
  const compactLine = lineText.replace(/\s+/g, '')
  if (compactWords !== compactLine) return words

  const restored: LyricWord[] = []
  let cursor = 0

  for (const word of words) {
    const token = word.text
    const nextIndex = lineText.indexOf(token, cursor)
    if (nextIndex < 0) return words

    const between = lineText.slice(cursor, nextIndex)
    if (between) {
      if (between.trim()) return words
      restored.push({
        startMs: word.startMs,
        durationMs: 0,
        text: between,
      })
    }

    restored.push({
      ...word,
      text: lineText.slice(nextIndex, nextIndex + token.length),
    })
    cursor = nextIndex + token.length
  }

  const tail = lineText.slice(cursor)
  if (tail) {
    if (tail.trim()) return words
    const lastWord = words[words.length - 1]
    restored.push({
      startMs: lastWord ? lastWord.startMs + lastWord.durationMs : 0,
      durationMs: 0,
      text: tail,
    })
  }

  return restored
}

function toInternalWords(
  words: LyricWord[],
  lineStart: number,
  lineEnd: number,
  lineText: string,
): InternalWord[] {
  const normalizedWords = normalizeWordTimings(words, lineStart, lineEnd)
  return restoreWhitespaceFromLineText(normalizedWords, lineText).map(word => ({
    source: word,
    word: word.text,
    startTime: word.startMs,
    endTime: word.startMs + Math.max(0, word.durationMs),
  }))
}

export class KaraokeLine {
  private container: HTMLElement | null = null
  private splittedWords: RealWord[] = []
  private lineStartTime = 0
  private lineEndTime = 0
  private fallbackText = ''
  private isEnabled = false
  private initRafId = 0
  private disposed = false
  private pendingEnable: { currentTimeMs: number; shouldPlay: boolean } | null = null
  private renderMode: 'solid' | 'dynamic' = 'solid'
  private currentBrightAlpha = 1.0
  private currentDarkAlpha = 0.2
  private targetBrightAlpha = 1.0
  private targetDarkAlpha = 0.2

  build(
    container: HTMLElement,
    lyricWords: LyricWord[],
    lineStart: number,
    lineEnd: number,
    fallbackText = '',
  ): void {
    this.dispose()
    this.disposed = false
    this.container = container
    this.lineStartTime = lineStart
    const words = toInternalWords(lyricWords, lineStart, lineEnd, fallbackText)
    this.lineEndTime = Math.max(lineEnd, ...words.map(word => word.endTime))
    this.fallbackText = fallbackText || words.map(word => word.word).join('')
    container.replaceChildren()
    container.style.setProperty('--bright-mask-alpha', '1')
    container.style.setProperty('--dark-mask-alpha', '1')

    for (const word of words) {
      this.buildWord(word, container)
    }

    this.initRafId = requestAnimationFrame(() => {
      this.initRafId = 0
      if (this.disposed) return
      this.updateMaskImageSync()
      if (this.pendingEnable) {
        const { currentTimeMs, shouldPlay } = this.pendingEnable
        this.pendingEnable = null
        this.applyAnimations(currentTimeMs, shouldPlay)
      }
    })
  }

  private buildWord(word: InternalWord, container: HTMLElement): void {
    if (!word.word.trim()) {
      container.appendChild(document.createTextNode(word.word))
      return
    }

    const wrapper = document.createElement('span')
    wrapper.className = 'kw-wrapper'
    const mainWordEl = document.createElement('span')
    mainWordEl.className = 'kw'
    mainWordEl.style.visibility = 'hidden'

    const emp = shouldEmphasize(word)
    const subElements: HTMLSpanElement[] = []
    if (emp) {
      mainWordEl.classList.add('emphasize')
      for (const segment of splitGraphemes(word.word.trim())) {
        const charEl = document.createElement('span')
        charEl.textContent = segment
        subElements.push(charEl)
        mainWordEl.appendChild(charEl)
      }
    } else {
      mainWordEl.textContent = word.word.trim()
    }

    wrapper.appendChild(mainWordEl)
    container.appendChild(wrapper)
    const realWord: RealWord = {
      ...word,
      mainElement: mainWordEl,
      subElements,
      elementAnimations: [this.initFloatAnimation(word, mainWordEl)],
      maskAnimations: [],
      width: 0,
      height: 0,
      padding: 0,
      shouldEmphasize: emp,
    }
    this.splittedWords.push(realWord)
  }

  private initFloatAnimation(word: InternalWord, wordEl: HTMLSpanElement): Animation {
    const delay = word.startTime - this.lineStartTime
    const duration = Math.max(1000, word.endTime - word.startTime)
    const animation = wordEl.animate(
      [
        { transform: 'translateY(0px)' },
        { transform: 'translateY(-0.05em)' },
      ],
      {
        duration: Number.isFinite(duration) ? duration : 0,
        delay: Number.isFinite(delay) ? delay : 0,
        id: 'float-word',
        composite: 'add',
        fill: 'both',
        easing: 'ease-out',
      },
    )
    animation.pause()
    return animation
  }

  private initEmphasizeAnimation(
    word: InternalWord,
    characterElements: HTMLElement[],
    duration: number,
    delay: number,
  ): Animation[] {
    const de = clampPositive(delay)
    let du = Math.max(1000, duration)
    const anchorCharCount = Math.max(1, characterElements.length)
    let amount = du / 2000
    amount = amount > 1 ? Math.sqrt(amount) : amount ** 3
    let blur = du / 3000
    blur = blur > 1 ? Math.sqrt(blur) : blur ** 3
    amount *= 0.6
    blur *= 0.5

    const lastWord = this.splittedWords[this.splittedWords.length - 1]
    if (lastWord && word.word.includes(lastWord.word)) {
      amount *= 1.6
      blur *= 1.5
      du *= 1.2
    }
    amount = Math.min(1.2, amount)
    blur = Math.min(0.8, blur)

    const animateDu = Number.isFinite(du) ? du : 0
    const empEasing = makeEmpEasing(EMP_EASING_MID)
    return characterElements.flatMap((el, i, arr) => {
      const wordDe = de + (du / 2.5 / anchorCharCount) * i
      const frames: Keyframe[] = new Array(ANIMATION_FRAME_QUANTITY)
        .fill(0)
        .map((_, j) => {
          const x = (j + 1) / ANIMATION_FRAME_QUANTITY
          const transX = empEasing(x)
          const glowLevel = empEasing(x) * blur
          const mat = scaleMatrix4(createMatrix4(), 1 + transX * 0.1 * amount)
          const offsetX = -transX * 0.03 * amount * (arr.length / 2 - i)
          const offsetY = -transX * 0.025 * amount
          return {
            offset: x,
            transform: `${matrix4ToCSS(mat, 4)} translate(${offsetX}em, ${offsetY}em)`,
            textShadow: `0 0 ${Math.min(0.3, blur * 0.3)}em rgba(255, 255, 255, ${glowLevel})`,
          }
        })

      const glow = el.animate(frames, {
        duration: animateDu,
        delay: Number.isFinite(wordDe) ? wordDe : 0,
        id: `emphasize-word-${el.textContent}-${i}`,
        iterations: 1,
        composite: 'replace',
        fill: 'both',
      })
      glow.onfinish = () => glow.pause()
      glow.pause()

      const floatFrame: Keyframe[] = new Array(ANIMATION_FRAME_QUANTITY)
        .fill(0)
        .map((_, j) => {
          const x = (j + 1) / ANIMATION_FRAME_QUANTITY
          const y = Math.sin(x * Math.PI)
          return {
            offset: x,
            transform: `translateY(${-y * 0.05}em)`,
          }
        })
      const float = el.animate(floatFrame, {
        duration: animateDu * 1.4,
        delay: Number.isFinite(wordDe) ? wordDe - 400 : 0,
        id: 'emphasize-word-float',
        iterations: 1,
        composite: 'add',
        fill: 'both',
      })
      float.onfinish = () => float.pause()
      float.pause()

      return [glow, float]
    })
  }

  private get totalDuration(): number {
    return this.lineEndTime - this.lineStartTime
  }

  private updateMaskImageSync(): void {
    for (const word of this.splittedWords) {
      const style = getComputedStyle(word.mainElement)
      word.padding = Number.parseFloat(style.paddingLeft) || 0
      word.width = word.mainElement.clientWidth - word.padding * 2
      word.height = word.mainElement.clientHeight - word.padding * 2
      if (word.shouldEmphasize && word.subElements.length > 0) {
        word.elementAnimations.push(
          ...this.initEmphasizeAnimation(
            word,
            word.subElements,
            word.endTime - word.startTime,
            word.startTime - this.lineStartTime,
          ),
        )
      }
    }
    this.generateWebAnimationBasedMaskImage()
    for (const word of this.splittedWords) {
      word.mainElement.style.visibility = ''
    }
  }

  private generateWebAnimationBasedMaskImage(): void {
    const totalFadeDuration = Math.max(
      0,
      ...this.splittedWords.map(word => word.endTime),
      this.lineEndTime,
    ) - this.lineStartTime

    this.splittedWords.forEach((word, i) => {
      const wordEl = word.mainElement
      const fadeWidth = word.height * WORD_FADE_WIDTH
      const [maskImage, totalAspect] = generateFadeGradient(
        fadeWidth / Math.max(1, word.width + word.padding * 2),
      )
      const totalAspectStr = `${totalAspect * 100}% 100%`
      wordEl.style.maskImage = maskImage
      wordEl.style.maskRepeat = 'no-repeat'
      wordEl.style.maskOrigin = 'left'
      wordEl.style.maskSize = totalAspectStr
      wordEl.style.webkitMaskImage = maskImage
      wordEl.style.webkitMaskRepeat = 'no-repeat'
      wordEl.style.webkitMaskOrigin = 'left'
      wordEl.style.webkitMaskSize = totalAspectStr

      const widthBeforeSelf = this.splittedWords
        .slice(0, i)
        .reduce((sum, prev) => sum + prev.width, 0) + (this.splittedWords[0] ? fadeWidth : 0)
      const minOffset = -(word.width + word.padding * 2 + fadeWidth)
      const clampOffset = (x: number) => clamp(x, minOffset, 0)
      let curPos = -widthBeforeSelf - word.width - word.padding - fadeWidth
      let timeOffset = 0
      const frames: Keyframe[] = []
      let lastPos = curPos
      let lastTime = 0

      const pushFrame = () => {
        const moveOffset = curPos - lastPos
        const time = clamp01(timeOffset)
        const duration = time - lastTime
        const d = Math.abs(duration / moveOffset)
        if (curPos > minOffset && lastPos < minOffset) {
          const staticTime = Math.abs(lastPos - minOffset) * d
          frames.push(maskPositionFrame(lastTime + staticTime, `${clampOffset(lastPos)}px 0`))
        }
        if (curPos > 0 && lastPos < 0) {
          const staticTime = Math.abs(lastPos) * d
          frames.push(maskPositionFrame(lastTime + staticTime, `${clampOffset(curPos)}px 0`))
        }
        frames.push(maskPositionFrame(time, `${clampOffset(curPos)}px 0`))
        lastPos = curPos
        lastTime = time
      }

      pushFrame()
      let lastTimeStamp = 0
      this.splittedWords.forEach((otherWord, j) => {
        const curTimeStamp = otherWord.startTime - this.lineStartTime
        const staticDuration = curTimeStamp - lastTimeStamp
        timeOffset += staticDuration / totalFadeDuration
        if (staticDuration > 0) pushFrame()
        lastTimeStamp = curTimeStamp

        const fadeDuration = clampPositive(otherWord.endTime - otherWord.startTime)
        const segmentCount = 1
        const segmentWidth = otherWord.width / segmentCount
        const segmentDuration = fadeDuration / segmentCount
        for (let segmentIndex = 0; segmentIndex < segmentCount; segmentIndex++) {
          timeOffset += segmentDuration / totalFadeDuration
          curPos += segmentWidth
          if (j === 0 && segmentIndex === 0) curPos += fadeWidth * 1.5
          if (j === this.splittedWords.length - 1 && segmentIndex === segmentCount - 1) {
            curPos += fadeWidth * 0.5
          }
          if (segmentDuration > 0) pushFrame()
          lastTimeStamp += segmentDuration
        }
      })

      for (const animation of word.maskAnimations) animation.cancel()
      try {
        const animation = wordEl.animate(cleanKeyframes(frames), {
          duration: totalFadeDuration || 1,
          id: `fade-word-${word.word}-${i}`,
          fill: 'both',
        })
        animation.pause()
        word.maskAnimations = [animation]
      } catch (error) {
        log.warn('mask animation error:', error)
      }
    })
  }

  enable(currentTimeMs: number, shouldPlay = true): void {
    this.isEnabled = true
    this.renderMode = 'dynamic'
    if (this.initRafId) {
      this.pendingEnable = { currentTimeMs, shouldPlay }
      return
    }
    this.applyAnimations(currentTimeMs, shouldPlay)
  }

  private applyAnimations(currentTimeMs: number, shouldPlay: boolean): void {
    const relativeTime = clampPositive(currentTimeMs - this.lineStartTime)
    for (const word of this.splittedWords) {
      for (const animation of word.elementAnimations) {
        animation.currentTime = relativeTime
        animation.playbackRate = 1
        const timing = animation.effect?.getComputedTiming()
        const duration = Number(timing?.duration ?? 0)
        const delay = Number(timing?.delay ?? 0)
        const endTime = delay + duration
        if (shouldPlay && relativeTime < endTime) animation.play()
        else animation.pause()
      }
      for (const animation of word.maskAnimations) {
        const time = Math.min(this.totalDuration, relativeTime)
        animation.currentTime = time
        animation.playbackRate = 1
        const timing = animation.effect?.getComputedTiming()
        const duration = Number(timing?.duration ?? 0)
        const delay = Number(timing?.delay ?? 0)
        const endTime = delay + duration
        if (shouldPlay && time < endTime) animation.play()
        else animation.pause()
      }
    }
  }

  disable(): void {
    this.isEnabled = false
    this.pendingEnable = null
    this.renderMode = 'solid'
    for (const word of this.splittedWords) {
      for (const animation of word.elementAnimations) {
        if (animation.id === 'float-word' || animation.id.includes('emphasize-word-float-only')) {
          animation.playbackRate = -1
          animation.play()
        }
      }
      for (const animation of word.maskAnimations) animation.pause()
    }
  }

  seek(currentTimeMs: number): void {
    if (!this.isEnabled) return
    const time = currentTimeMs - this.lineStartTime
    for (const word of this.splittedWords) {
      for (const animation of word.maskAnimations) {
        animation.currentTime = clamp(time, 0, this.totalDuration)
        animation.playbackRate = 1
        if (time >= 0 && time < this.totalDuration) animation.play()
        else animation.pause()
      }
    }
  }

  pause(): void {
    if (!this.isEnabled) return
    for (const word of this.splittedWords) {
      for (const animation of word.elementAnimations) animation.pause()
      for (const animation of word.maskAnimations) animation.pause()
    }
  }

  resume(): void {
    if (!this.isEnabled) return
    for (const word of this.splittedWords) {
      for (const animation of word.elementAnimations) {
        const timing = animation.effect?.getComputedTiming()
        const duration = Number(timing?.duration ?? 0)
        const delay = Number(timing?.delay ?? 0)
        const endTime = delay + duration
        const currentTime = Number(animation.currentTime ?? 0)
        if (animation.playState !== 'finished' && currentTime < endTime) animation.play()
      }
      for (const animation of word.maskAnimations) {
        const timing = animation.effect?.getComputedTiming()
        const duration = Number(timing?.duration ?? 0)
        const delay = Number(timing?.delay ?? 0)
        const endTime = delay + duration
        const currentTime = Number(animation.currentTime ?? 0)
        if (animation.playState !== 'finished' && currentTime < endTime) animation.play()
      }
    }
  }

  updateMaskAlpha(scale: number, delta = 0.016, force = false): void {
    const factor = clamp01((scale - 0.97) / 0.03)
    const dynamicDarkAlpha = factor * 0.2 + 0.2
    const dynamicBrightAlpha = factor * 0.8 + 0.2
    if (this.renderMode === 'solid') {
      this.targetBrightAlpha = dynamicDarkAlpha
      this.targetDarkAlpha = dynamicDarkAlpha
    } else {
      this.targetBrightAlpha = dynamicBrightAlpha
      this.targetDarkAlpha = dynamicDarkAlpha
    }
    if (force) {
      this.currentBrightAlpha = this.targetBrightAlpha
      this.currentDarkAlpha = this.targetDarkAlpha
      this.writeMaskAlpha()
      return
    }
    this.applyAlphaToDom(delta)
  }

  private applyAlphaToDom(delta: number): void {
    const dt = delta || 0.016
    const attackSpeed = 50.0
    const releaseSpeed = 7.0
    const getFactor = (speed: number) => 1 - Math.exp(-speed * dt)
    const brightSpeed = this.targetBrightAlpha > this.currentBrightAlpha ? attackSpeed : releaseSpeed
    const darkSpeed = this.targetDarkAlpha > this.currentDarkAlpha ? attackSpeed : releaseSpeed
    this.currentBrightAlpha += (this.targetBrightAlpha - this.currentBrightAlpha) * getFactor(brightSpeed)
    this.currentDarkAlpha += (this.targetDarkAlpha - this.currentDarkAlpha) * getFactor(darkSpeed)
    if (Math.abs(this.targetBrightAlpha - this.currentBrightAlpha) < 0.001) {
      this.currentBrightAlpha = this.targetBrightAlpha
    }
    if (Math.abs(this.targetDarkAlpha - this.currentDarkAlpha) < 0.001) {
      this.currentDarkAlpha = this.targetDarkAlpha
    }
    this.writeMaskAlpha()
  }

  private writeMaskAlpha(): void {
    this.container?.style.setProperty('--bright-mask-alpha', this.currentBrightAlpha.toFixed(3))
    this.container?.style.setProperty('--dark-mask-alpha', this.currentDarkAlpha.toFixed(3))
  }

  dispose(): void {
    this.disposed = true
    this.pendingEnable = null
    if (this.initRafId) {
      cancelAnimationFrame(this.initRafId)
      this.initRafId = 0
    }
    for (const word of this.splittedWords) {
      for (const animation of word.elementAnimations) animation.cancel()
      for (const animation of word.maskAnimations) animation.cancel()
      for (const sub of word.subElements) sub.remove()
      word.mainElement.remove()
    }
    if (this.container?.isConnected && this.fallbackText) {
      this.container.textContent = this.fallbackText
    }
    this.splittedWords = []
    this.container = null
    this.fallbackText = ''
  }
}
