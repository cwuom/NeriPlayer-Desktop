/** 断线重连策略（对齐 Android ListenTogetherReconnectPolicy） */
export const MAX_RECONNECT_ATTEMPTS = 15

const BASE_DELAYS_MS = [1500, 3000, 5000, 8000, 12000]
const JITTER = 0.2

/** 第 attempt 次（从 1 开始）重连前的等待；±20% 抖动，避免整个房间同一时刻一起重连 */
export function reconnectDelayMs(attempt: number, random: () => number = Math.random): number {
  const base = BASE_DELAYS_MS[Math.min(Math.max(attempt, 1), BASE_DELAYS_MS.length) - 1]
  return Math.round(base + base * JITTER * (random() * 2 - 1))
}

const TERMINAL_MESSAGES = ['unauthorized', 'room closed', 'room not initialized', 'not found in do']
// 桌面端传输层把握手失败报成 "HTTP error: 410 Gone" / "HTTP 404"
const TERMINAL_HTTP_STATUS = /\bhttp(?: error)?:? (401|404|410)\b/

/** 房间已关闭、凭据失效这类错误重连也救不回来，应在本地直接结束会话 */
export function isTerminalReconnectError(message: unknown): boolean {
  const text = String(message ?? '').toLowerCase()
  return TERMINAL_MESSAGES.some(pattern => text.includes(pattern)) || TERMINAL_HTTP_STATUS.test(text)
}
