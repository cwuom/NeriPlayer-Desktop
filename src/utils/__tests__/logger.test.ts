import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

type ForwardFn = (message: string) => Promise<void>

const pluginLog = vi.hoisted(() => ({
  trace: vi.fn<ForwardFn>(),
  debug: vi.fn<ForwardFn>(),
  info: vi.fn<ForwardFn>(),
  warn: vi.fn<ForwardFn>(),
  error: vi.fn<ForwardFn>(),
}))

const consoleSpies = {
  debug: vi.fn<(...data: unknown[]) => void>(),
  info: vi.fn<(...data: unknown[]) => void>(),
  warn: vi.fn<(...data: unknown[]) => void>(),
  error: vi.fn<(...data: unknown[]) => void>(),
}

const TS_STYLE = 'color:#888'
const SCOPE_STYLE = 'color:#4dd0e1;font-weight:600'
const RESET_STYLE = 'color:inherit;font-weight:normal'

/** logger 在模块加载时 bind console 方法，所以必须先 spy 再重新加载模块 */
async function loadLogger(): Promise<typeof import('@/utils/logger')> {
  vi.spyOn(console, 'debug').mockImplementation(consoleSpies.debug)
  vi.spyOn(console, 'info').mockImplementation(consoleSpies.info)
  vi.spyOn(console, 'warn').mockImplementation(consoleSpies.warn)
  vi.spyOn(console, 'error').mockImplementation(consoleSpies.error)
  vi.resetModules()
  return import('@/utils/logger')
}

/** 等待惰性 import 与转发 promise 链跑完；并行负载下 import 可能晚于单个 macrotask */
async function flushForwarding(): Promise<void> {
  await vi.dynamicImportSettled()
  await new Promise(resolve => setTimeout(resolve, 0))
}

beforeEach(() => {
  for (const fn of Object.values(pluginLog)) fn.mockReset().mockResolvedValue(undefined)
  for (const fn of Object.values(consoleSpies)) fn.mockReset()
  vi.doMock('@tauri-apps/plugin-log', () => pluginLog)
  vi.useFakeTimers({ toFake: ['Date'] })
  // 用本地时间构造，断言与时区无关
  vi.setSystemTime(new Date(2026, 0, 2, 3, 4, 5, 6))
})

afterEach(() => {
  vi.useRealTimers()
  vi.doUnmock('@tauri-apps/plugin-log')
})

describe('createLogger console output', () => {
  it('prefixes a zero-padded local timestamp, scope, and level with %c styles', async () => {
    const { createLogger } = await loadLogger()
    const payload = { id: 1 }

    createLogger('player').info('loaded', payload)

    expect(consoleSpies.info).toHaveBeenCalledTimes(1)
    const [format, tsStyle, scopeStyle, levelStyle, resetStyle, ...rest] = consoleSpies.info.mock.calls[0]
    expect(format).toBe('%c2026-01-02 03:04:05.006 %c[player] %cINFO%c')
    expect([tsStyle, scopeStyle, resetStyle]).toEqual([TS_STYLE, SCOPE_STYLE, RESET_STYLE])
    expect(levelStyle).toBe('color:#66bb6a;font-weight:600')
    // 原始对象原样透传，DevTools 中可展开
    expect(rest).toEqual(['loaded', payload])
    expect(rest[1]).toBe(payload)
  })

  it('routes each level to the matching console method', async () => {
    const { createLogger } = await loadLogger()
    const log = createLogger('sync')

    log.trace('t')
    log.debug('d')
    log.info('i')
    log.warn('w')
    log.error('e')

    expect(consoleSpies.debug.mock.calls.map(call => call[0])).toEqual([
      expect.stringContaining('%cTRACE%c'),
      expect.stringContaining('%cDEBUG%c'),
    ])
    expect(consoleSpies.info).toHaveBeenCalledWith(expect.stringContaining('%cINFO%c'), TS_STYLE, SCOPE_STYLE, expect.any(String), RESET_STYLE, 'i')
    expect(consoleSpies.warn).toHaveBeenCalledWith(expect.stringContaining('%cWARN%c'), TS_STYLE, SCOPE_STYLE, expect.any(String), RESET_STYLE, 'w')
    expect(consoleSpies.error).toHaveBeenCalledWith(expect.stringContaining('%cERROR%c'), TS_STYLE, SCOPE_STYLE, expect.any(String), RESET_STYLE, 'e')
  })
})

describe('createLogger backend forwarding', () => {
  it('forwards a scoped message to the plugin method of the same level', async () => {
    const { createLogger } = await loadLogger()
    const log = createLogger('now-playing')

    log.trace('a')
    log.debug('b')
    log.info('c')
    log.warn('d')
    log.error('e')
    await flushForwarding()

    expect(pluginLog.trace).toHaveBeenCalledWith('[now-playing] a')
    expect(pluginLog.debug).toHaveBeenCalledWith('[now-playing] b')
    expect(pluginLog.info).toHaveBeenCalledWith('[now-playing] c')
    expect(pluginLog.warn).toHaveBeenCalledWith('[now-playing] d')
    expect(pluginLog.error).toHaveBeenCalledWith('[now-playing] e')
  })

  it('serializes objects as JSON and joins arguments with spaces', async () => {
    const { createLogger } = await loadLogger()

    createLogger('sync').info('merged', { added: 2, removed: [1] }, 3, null)
    await flushForwarding()

    expect(pluginLog.info).toHaveBeenCalledWith('[sync] merged {"added":2,"removed":[1]} 3 null')
  })

  it('uses the stack for errors and falls back to the message without one', async () => {
    const { createLogger } = await loadLogger()
    const withStack = new Error('boom')
    withStack.stack = 'Error: boom\n    at play'
    const withoutStack = new Error('quiet')
    withoutStack.stack = ''

    createLogger('player').error(withStack, withoutStack)
    await flushForwarding()

    expect(pluginLog.error).toHaveBeenCalledWith('[player] Error: boom\n    at play quiet')
  })

  it('falls back to String() for values JSON cannot serialize', async () => {
    const { createLogger } = await loadLogger()
    const circular: Record<string, unknown> = {}
    circular.self = circular

    createLogger('player').warn('state', circular, 10n)
    await flushForwarding()

    expect(pluginLog.warn).toHaveBeenCalledWith('[player] state [object Object] 10')
  })

  it('forwards just the scope when called without arguments', async () => {
    const { createLogger } = await loadLogger()

    createLogger('player').info()
    await flushForwarding()

    expect(pluginLog.info).toHaveBeenCalledWith('[player] ')
  })

  it('swallows rejections from the backend plugin', async () => {
    pluginLog.error.mockRejectedValue(new Error('ipc closed'))
    const { createLogger } = await loadLogger()

    expect(() => createLogger('player').error('x')).not.toThrow()
    await flushForwarding()

    expect(pluginLog.error).toHaveBeenCalledTimes(1)
    expect(consoleSpies.error).toHaveBeenCalledTimes(1)
  })

  it('keeps logging to the console when the plugin module cannot be loaded', async () => {
    vi.doMock('@tauri-apps/plugin-log', () => {
      throw new Error('not running inside Tauri')
    })
    const { createLogger } = await loadLogger()
    const log = createLogger('player')

    log.info('first')
    log.info('second')
    await flushForwarding()

    expect(consoleSpies.info).toHaveBeenCalledTimes(2)
    expect(pluginLog.info).not.toHaveBeenCalled()
  })
})
