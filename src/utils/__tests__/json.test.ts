import { describe, expect, it } from 'vitest'
import { asArray, asRecord, asString, dig, isJsonRecord, objectChildren } from '@/utils/json'

describe('json helpers', () => {
  it('treats only plain objects as records', () => {
    expect(isJsonRecord({ a: 1 })).toBe(true)
    expect(isJsonRecord([])).toBe(false)
    expect(isJsonRecord(null)).toBe(false)
    expect(isJsonRecord('text')).toBe(false)
    expect(asRecord([1])).toBeUndefined()
  })

  it('narrows arrays and strings without coercion', () => {
    const list = [1, 'two']
    expect(asArray(list)).toBe(list)
    expect(asArray({ length: 2 })).toEqual([])
    expect(asString('x')).toBe('x')
    expect(asString(1)).toBeUndefined()
  })

  it('digs like optional chaining: string keys on objects, numeric keys on arrays', () => {
    const root = { tabs: [{ title: { runs: [{ text: 'Home' }] } }], '0': 'object-key' }
    expect(dig(root, 'tabs', 0, 'title', 'runs', 0, 'text')).toBe('Home')
    expect(dig(root, 'tabs', 3, 'title')).toBeUndefined()
    // 数字键不读对象上的同名属性，字符串键也不读数组属性
    expect(dig(root, 0)).toBeUndefined()
    expect(dig(root, 'tabs', 'length')).toBeUndefined()
    expect(dig(null, 'a')).toBeUndefined()
    expect(dig('text', 'length')).toBeUndefined()
    // 中间层是 null 时与 `?.` 一样止步，而叶子上的 null 原样返回
    expect(dig({ a: null }, 'a')).toBeNull()
    expect(dig({ a: null }, 'a', 'b')).toBeUndefined()
  })

  it('expands only object children, keeping their order', () => {
    const first = { id: 1 }
    const second = [2]
    expect(objectChildren({ a: first, b: 'skip', c: null, d: second })).toEqual([first, second])
    expect(objectChildren([first, 3, second])).toEqual([first, second])
  })
})
