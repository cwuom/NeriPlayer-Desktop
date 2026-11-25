// 回归：Vue 编译 scoped 样式时会丢掉 :global(X) 后面的部分。
// TitleBar 里写成 `:global(html.sidebar-collapsed) .tb-brand { opacity: 0 }`，实际生成 `html.sidebar-collapsed { opacity: 0 }`，
// 侧栏一收起整页就变透明（黑屏）。这里用真实编译器编译所有 scoped 样式，
// 不允许出现「只剩全局根选择器」的规则（html / body / :root / 主题与平台类）
import assert from 'node:assert/strict'
import { readFile, readdir } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { compileStyle, parse } from 'vue/compiler-sfc'
import { compileString } from 'sass'

const root = fileURLToPath(new URL('../src/', import.meta.url))

async function vueFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true })
  const nested = await Promise.all(entries.map(entry => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) return entry.name === '__tests__' ? [] : vueFiles(path)
    return entry.name.endsWith('.vue') ? [path] : []
  }))
  return nested.flat()
}

// 只由全局状态类组成、没有任何组件内元素的选择器：这类规则会作用到整页
const BARE_GLOBAL = /^(?:html|body|:root)?(?:\.(?:dark-theme|light-theme|platform-[a-z]+|sidebar-collapsed|has-custom-bg|enhanced-blur|window-fullscreen|nvidia-compat|theme-ripple-active))*$/

const offenders = []
for (const file of await vueFiles(root)) {
  const { descriptor } = parse(await readFile(file, 'utf8'), { filename: file })
  descriptor.styles.forEach((style, index) => {
    if (!style.scoped) return
    const source = style.lang === 'scss'
      ? compileString(style.content, { url: pathToFileURL(file), loadPaths: [root, join(root, '..')], importers: [{ findFileUrl: url => url.startsWith('@/') ? new URL(url.slice(2), `file://${root}`) : null }] }).css
      : style.content
    const { code, errors } = compileStyle({ source, filename: file, id: `data-v-${index}`, scoped: true })
    assert.deepEqual(errors, [], `${file}: style compile errors`)
    // 去掉注释，否则注释会混进下一条规则的选择器里
    const css = code.replace(/\/\*[\s\S]*?\*\//g, '')
    for (const match of css.matchAll(/(^|[{};])\s*([^{}@;]+)\{/g)) {
      for (const selector of match[2].split(',').map(part => part.trim()).filter(Boolean)) {
        if (BARE_GLOBAL.test(selector)) offenders.push(`${file.replace(root, 'src/')}: ${selector}`)
      }
    }
  })
}

assert.deepEqual(offenders, [], `scoped styles compiled into page-wide rules (wrap the whole selector in :global()):\n${offenders.join('\n')}`)
console.log('scoped :global selectors: ok')
