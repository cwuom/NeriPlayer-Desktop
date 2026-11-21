// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from 'vitest'
import { createApp, h, nextTick, ref, type App } from 'vue'
import { createI18n } from 'vue-i18n'
import SettingsSidebar from '@/components/settings/SettingsSidebar.vue'
import type { SettingsSearchResult } from '@/components/settings/useSettingsSearch'
import type { SettingsNavGroup, SettingsSectionId } from '@/components/settings/useSettingsSections'

const navGroups: SettingsNavGroup[] = [
  { items: [{ id: 'general', label: 'General', description: 'Basics', icon: 'settings' }] },
  { items: [{ id: 'lyrics', label: 'Lyrics', description: 'Lines', icon: 'lyrics' }] },
]
const results: SettingsSearchResult[] = [
  { key: 'lyrics:x', section: 'lyrics', title: 'Lyric blur', desc: '', keywords: [], isSection: false },
]

let app: App | null = null
afterEach(() => {
  app?.unmount()
  app = null
  document.body.innerHTML = ''
})

// 与设置页模板里 v-model:settings-query / @select / @open-result 编译出的事件名一致
function mount(initialQuery = '') {
  const query = ref(initialQuery)
  const selected: SettingsSectionId[] = []
  const opened: Array<SettingsSearchResult | undefined> = []
  const root = document.createElement('div')
  document.body.append(root)
  app = createApp({
    render: () => h(SettingsSidebar, {
      settingsQuery: query.value,
      'onUpdate:settingsQuery': (value: string) => { query.value = value },
      settingsSearchResults: query.value ? results : [],
      settingsNavGroups: navGroups,
      activeSettingsSection: 'general',
      onSelect: (id: SettingsSectionId) => { selected.push(id) },
      onOpenResult: (result: SettingsSearchResult | undefined) => { opened.push(result) },
    }),
  })
  app.use(createI18n({ legacy: false, locale: 'en', missingWarn: false, fallbackWarn: false, messages: { en: {} } }))
  app.mount(root)
  const input = root.querySelector<HTMLInputElement>('.settings-search input')
  if (!input) throw new Error('search input missing')
  return { root, input, query, selected, opened }
}

describe('SettingsSidebar', () => {
  it('writes typed queries back to the settings page and clears them with the clear button', async () => {
    const view = mount()
    view.input.value = 'blur'
    view.input.dispatchEvent(new Event('input'))
    expect(view.query.value).toBe('blur')
    await nextTick()

    view.root.querySelector<HTMLButtonElement>('.settings-search-clear')?.click()
    expect(view.query.value).toBe('')
    expect(document.activeElement).toBe(view.input)
  })

  it('opens the first result on Enter and clears the query on Escape without bubbling', async () => {
    const view = mount('blur')
    await nextTick()
    view.input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', cancelable: true }))
    expect(view.opened).toEqual([results[0]])

    let bubbled = false
    view.root.addEventListener('keydown', () => { bubbled = true })
    const escape = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    view.input.dispatchEvent(escape)
    expect(view.query.value).toBe('')
    expect(escape.defaultPrevented).toBe(true)
    expect(bubbled).toBe(false)
  })

  it('reports navigation clicks and marks the active section', () => {
    const view = mount()
    const items = [...view.root.querySelectorAll<HTMLButtonElement>('.settings-nav-item')]
    expect(items.map(item => item.getAttribute('aria-current'))).toEqual(['page', null])
    items[1].click()
    expect(view.selected).toEqual(['lyrics'])
  })
})
