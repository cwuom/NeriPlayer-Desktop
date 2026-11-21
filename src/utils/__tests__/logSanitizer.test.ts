import { describe, expect, it } from 'vitest'
import { summarizeLogError } from '@/utils/logSanitizer'

describe('summarizeLogError', () => {
  it('uses the stack of an Error and redacts URLs inside it', () => {
    const error = new Error('request failed')
    error.stack = 'Error: request failed\n    at fetch (https://cdn.example/app.js?sig=abc:1:2)'
    expect(summarizeLogError(error)).toBe('Error: request failed\n    at fetch ([url])')
  })

  it('falls back to the message when an Error has no stack', () => {
    const error = new Error('GET http://music.example/song.mp3?token=secret timed out')
    error.stack = ''
    expect(summarizeLogError(error)).toBe('GET [url] timed out')
  })

  it('stringifies non-Error values', () => {
    expect(summarizeLogError('plain text')).toBe('plain text')
    expect(summarizeLogError(404)).toBe('404')
    expect(summarizeLogError(null)).toBe('null')
    expect(summarizeLogError(undefined)).toBe('undefined')
  })

  it('redacts every URL and stops at closing brackets', () => {
    expect(summarizeLogError('a https://x.example/1 b http://y.example/2?q=1 c')).toBe('a [url] b [url] c')
    expect(summarizeLogError('(https://x.example/p) [https://y.example/q] {https://z.example/r}'))
      .toBe('([url]) [[url]] {[url]}')
  })

  it('leaves text without http(s) URLs untouched', () => {
    expect(summarizeLogError('ftp://files.example/a and file:///tmp/a')).toBe('ftp://files.example/a and file:///tmp/a')
  })
})
