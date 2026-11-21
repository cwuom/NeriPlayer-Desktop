import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useToastStore } from '@/stores/toast'

describe('toast store', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(1_700_000_000_000)
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('dismisses each message after its own duration', () => {
    const toast = useToastStore()

    toast.show('info default')
    toast.success('quick', 500)
    toast.error('error default')
    expect(toast.messages.map(message => [message.text, message.type, message.duration])).toEqual([
      ['info default', 'info', 3000],
      ['quick', 'success', 500],
      ['error default', 'error', 4000],
    ])

    vi.advanceTimersByTime(500)
    expect(toast.messages.map(message => message.text)).toEqual(['info default', 'error default'])
    vi.advanceTimersByTime(2500)
    expect(toast.messages.map(message => message.text)).toEqual(['error default'])
    vi.advanceTimersByTime(1000)
    expect(toast.messages).toEqual([])
  })

  it('accepts an options object with an action and falls back to the default duration', async () => {
    const toast = useToastStore()
    const handler = vi.fn()

    toast.show('with action', 'info', { action: { label: 'Open', handler } })
    toast.error('error with duration', { duration: 100 })

    expect(toast.messages[0]).toMatchObject({ text: 'with action', duration: 3000, action: { label: 'Open' } })
    expect(toast.messages[1]).toMatchObject({ text: 'error with duration', duration: 100 })
    await toast.messages[0].action!.handler()
    expect(handler).toHaveBeenCalledOnce()
  })

  it('keeps manual dismissal independent of other messages', () => {
    const toast = useToastStore()
    toast.show('a')
    toast.show('b', 'info', 10_000)
    const [first] = toast.messages

    toast.dismiss(first.id)
    toast.dismiss(-1)

    expect(toast.messages.map(message => message.text)).toEqual(['b'])
    // 已手动关闭的消息到期时，定时器不能误删别的消息
    vi.advanceTimersByTime(3000)
    expect(toast.messages.map(message => message.text)).toEqual(['b'])
    vi.advanceTimersByTime(7000)
    expect(toast.messages).toEqual([])
  })

  it('records only errors in the notification history, newest first', () => {
    const toast = useToastStore()

    toast.success('saved')
    toast.show('note')
    toast.error('first failure')
    vi.setSystemTime(1_700_000_000_500)
    toast.error('second failure')

    expect(toast.history.map(entry => entry.text)).toEqual(['second failure', 'first failure'])
    expect(toast.history[0]).toMatchObject({ type: 'error', read: false, timestamp: 1_700_000_000_500 })
    expect(toast.unreadCount).toBe(2)
    // 气泡消失不影响通知中心
    vi.advanceTimersByTime(10_000)
    expect(toast.history).toHaveLength(2)
  })

  it('marks everything read and clears history on request', () => {
    const toast = useToastStore()
    toast.error('a')
    toast.error('b')

    toast.markAllRead()
    expect(toast.unreadCount).toBe(0)
    expect(toast.history.every(entry => entry.read)).toBe(true)

    toast.error('c')
    expect(toast.unreadCount).toBe(1)

    toast.clearHistory()
    expect(toast.history).toEqual([])
    expect(toast.unreadCount).toBe(0)
  })

  it('caps the history at 100 entries and drops the oldest', () => {
    const toast = useToastStore()

    for (let index = 0; index < 105; index++) toast.error(`e${index}`)

    expect(toast.history).toHaveLength(100)
    expect(toast.history[0].text).toBe('e104')
    expect(toast.history[99].text).toBe('e5')
  })
})
