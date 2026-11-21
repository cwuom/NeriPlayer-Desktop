import { describe, expect, it } from 'vitest'
import {
  formatEditableNumber,
  normalizeFloatingPoint,
  parseEditableNumber,
} from '@/utils/editableRange'

describe('formatEditableNumber', () => {
  it('drops trailing zeros and a dangling decimal point', () => {
    expect(formatEditableNumber(1.5)).toBe('1.5')
    expect(formatEditableNumber(10)).toBe('10')
    expect(formatEditableNumber(100.1)).toBe('100.1')
    expect(formatEditableNumber(-0.5)).toBe('-0.5')
  })

  it('hides binary floating point noise beyond 8 decimals', () => {
    expect(formatEditableNumber(0.1 + 0.2)).toBe('0.3')
    expect(formatEditableNumber(1.123456789)).toBe('1.12345679')
  })

  it('multiplies by the display scale before formatting', () => {
    expect(formatEditableNumber(0.25, 100)).toBe('25')
    expect(formatEditableNumber(0.123, 1000)).toBe('123')
  })

  it('prints negative zero as 0', () => {
    expect(formatEditableNumber(-0)).toBe('0')
    expect(formatEditableNumber(0, -1)).toBe('0')
  })

  it('returns an empty string for non-finite values', () => {
    expect(formatEditableNumber(Number.NaN)).toBe('')
    expect(formatEditableNumber(Number.POSITIVE_INFINITY)).toBe('')
    expect(formatEditableNumber(1e308, 10)).toBe('')
  })
})

describe('parseEditableNumber', () => {
  it('parses trimmed text inside the range', () => {
    expect(parseEditableNumber('  12.5 ', 0, 100)).toBe(12.5)
    expect(parseEditableNumber('1e2', 0, 100)).toBe(100)
  })

  it('accepts numeric input directly', () => {
    expect(parseEditableNumber(42, 0, 100)).toBe(42)
  })

  it('rejects empty and non-numeric text', () => {
    expect(parseEditableNumber('', 0, 100)).toBeNull()
    expect(parseEditableNumber('   ', 0, 100)).toBeNull()
    expect(parseEditableNumber('abc', 0, 100)).toBeNull()
    expect(parseEditableNumber('12px', 0, 100)).toBeNull()
  })

  it('rejects non-finite input', () => {
    expect(parseEditableNumber('Infinity', 0, Number.POSITIVE_INFINITY)).toBeNull()
    expect(parseEditableNumber(Number.NaN, 0, 100)).toBeNull()
  })

  it('divides by the display scale to get the stored value', () => {
    expect(parseEditableNumber('25', 0, 1, 100)).toBe(0.25)
    expect(parseEditableNumber('33.3', 0, 1, 100)).toBe(0.333)
  })

  it('rejects a zero, negative, or non-finite scale', () => {
    expect(parseEditableNumber('1', 0, 10, 0)).toBeNull()
    expect(parseEditableNumber('1', 0, 10, -1)).toBeNull()
    expect(parseEditableNumber('1', 0, 10, Number.NaN)).toBeNull()
    expect(parseEditableNumber('1', 0, 10, Number.POSITIVE_INFINITY)).toBeNull()
  })

  it('rejects values whose stored form overflows after scaling', () => {
    expect(parseEditableNumber('1e308', 0, Number.POSITIVE_INFINITY, 0.001)).toBeNull()
  })

  it('accepts the exact range boundaries', () => {
    expect(parseEditableNumber('0', 0, 1)).toBe(0)
    expect(parseEditableNumber('1', 0, 1)).toBe(1)
  })

  it('tolerates floating point error just outside the boundaries', () => {
    expect(parseEditableNumber('1.0000000001', 0, 1)).toBe(1)
    expect(parseEditableNumber('0.4999999999', 0.5, 1)).toBe(0.5)
  })

  it('rejects values clearly outside the range', () => {
    expect(parseEditableNumber('1.001', 0, 1)).toBeNull()
    expect(parseEditableNumber('-0.001', 0, 1)).toBeNull()
    expect(parseEditableNumber('101', 0, 1, 100)).toBeNull()
  })

  it('round-trips with formatEditableNumber for scaled values', () => {
    for (const value of [0, 0.05, 0.333, 0.5, 1]) {
      const text = formatEditableNumber(value, 100)
      expect(parseEditableNumber(text, 0, 1, 100)).toBe(normalizeFloatingPoint(value))
    }
  })
})

describe('normalizeFloatingPoint', () => {
  it('rounds to 8 decimals to remove accumulated error', () => {
    expect(normalizeFloatingPoint(0.1 + 0.2)).toBe(0.3)
    expect(normalizeFloatingPoint(1.123456789)).toBe(1.12345679)
  })

  it('keeps integers and short decimals unchanged', () => {
    expect(normalizeFloatingPoint(3)).toBe(3)
    expect(normalizeFloatingPoint(-2.5)).toBe(-2.5)
  })
})
