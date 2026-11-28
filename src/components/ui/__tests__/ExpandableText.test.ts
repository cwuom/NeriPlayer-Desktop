// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { createApp, nextTick, reactive, h, type App } from 'vue'
import { createI18n } from 'vue-i18n'
import ExpandableText from '@/components/ui/ExpandableText.vue'

// happy-dom 不做排版，scrollHeight / clientHeight 恒为 0；按文字长度模拟“被行数截断”
const CLAMPED_HEIGHT = 60
let fullHeightOf: (el: HTMLElement) => number
let app: App | null = null
let host: HTMLDivElement

beforeEach(() => {
  fullHeightOf = el => (el.textContent?.length ?? 0) > 20 ? 240 : 20
  Object.defineProperty(HTMLElement.prototype, 'scrollHeight', {
    configurable: true,
    get(this: HTMLElement) { return fullHeightOf(this) },
  })
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
    configurable: true,
    get(this: HTMLElement) { return Math.min(fullHeightOf(this), CLAMPED_HEIGHT) },
  })
  host = document.createElement('div')
  document.body.appendChild(host)
})

afterEach(() => {
  app?.unmount()
  app = null
  host.remove()
  Reflect.deleteProperty(HTMLElement.prototype, 'scrollHeight')
  Reflect.deleteProperty(HTMLElement.prototype, 'clientHeight')
})

async function mount(props: { text: string, lines?: number }) {
  const state = reactive({ ...props })
  const i18n = createI18n({
    legacy: false,
    locale: 'zh-CN',
    messages: { 'zh-CN': { common: { expand: '展开', collapse: '收起' } } },
  })
  app = createApp({ render: () => h(ExpandableText, { ...state }) })
  app.use(i18n)
  app.mount(host)
  await nextTick()
  return state
}

// 组件在 nextTick 里重新测量，等微任务全部跑完再断言
const settle = () => new Promise(resolve => setTimeout(resolve, 0))

const body = () => host.querySelector<HTMLElement>('.expandable-text-body')!
const toggle = () => host.querySelector<HTMLButtonElement>('.expandable-text-toggle')

describe('ExpandableText', () => {
  it('不需要截断的短文字不显示展开按钮', async () => {
    await mount({ text: '简短介绍' })
    expect(body().textContent).toBe('简短介绍')
    expect(toggle()).toBeNull()
  })

  it('被截断的长文字可以展开读完，再收起', async () => {
    const text = '陈奕迅与 The Duo Band 的合作专辑介绍，内容很长，会超出默认行数。\n第二段继续介绍。'
    await mount({ text, lines: 3 })
    expect(body().style.getPropertyValue('--expandable-text-lines')).toBe('3')
    expect(body().classList.contains('expanded')).toBe(false)
    expect(toggle()?.textContent).toContain('展开')
    expect(toggle()?.getAttribute('aria-expanded')).toBe('false')

    toggle()!.click()
    await nextTick()
    expect(body().classList.contains('expanded')).toBe(true)
    expect(body().textContent).toBe(text)
    // 展开后测量值不再溢出，按钮仍要留着用于收起
    expect(toggle()?.textContent).toContain('收起')
    expect(toggle()?.getAttribute('aria-expanded')).toBe('true')

    toggle()!.click()
    await settle()
    expect(body().classList.contains('expanded')).toBe(false)
    expect(toggle()?.textContent).toContain('展开')
  })

  it('切换到另一位歌手的简介时恢复折叠并重新判断', async () => {
    const state = await mount({ text: '这是一段足够长、会被截断的歌手简介文字内容。' })
    toggle()!.click()
    await nextTick()
    expect(body().classList.contains('expanded')).toBe(true)

    state.text = '短简介'
    await settle()
    expect(body().classList.contains('expanded')).toBe(false)
    expect(toggle()).toBeNull()
  })
})
