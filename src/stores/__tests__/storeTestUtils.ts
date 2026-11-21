/**
 * store 测试共用工具：可控的 IPC 应答与微任务冲刷
 *
 * store 的竞态保护（序号 / 代际）只有在「旧请求晚于新请求返回」时才会被走到，
 * 所以测试需要手动决定每个 invoke 何时完成
 */

export interface Deferred<T> {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason: unknown) => void
}

export function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve
    reject = onReject
  })
  return { promise, resolve, reject }
}

/** store 内部常有多级 await，冲刷若干轮微任务让它们跑到下一个真正挂起的点 */
export async function flushPromises(rounds = 20): Promise<void> {
  for (let index = 0; index < rounds; index++) await Promise.resolve()
}

export type InvokeHandler = (args: Record<string, unknown> | undefined) => unknown

/**
 * 按命令名路由的 invoke 实现；同名命令可以排队多个应答，用完后回落到最后一个。
 * 未登记的命令直接失败，防止 store 悄悄多发请求而测试察觉不到
 */
export function createInvokeRouter() {
  const queues = new Map<string, InvokeHandler[]>()
  const calls: Array<{ command: string; args: Record<string, unknown> | undefined }> = []

  function on(command: string, ...handlers: InvokeHandler[]) {
    queues.set(command, [...(queues.get(command) ?? []), ...handlers])
  }

  /** 丢弃该命令尚未用掉的应答，换成新的 */
  function set(command: string, ...handlers: InvokeHandler[]) {
    queues.set(command, handlers)
  }

  async function invoke(command: string, args?: Record<string, unknown>): Promise<unknown> {
    calls.push({ command, args })
    const queue = queues.get(command)
    if (!queue?.length) throw new Error(`unexpected invoke: ${command}`)
    const handler = queue.length > 1 ? queue.shift()! : queue[0]
    return handler(args)
  }

  function callsOf(command: string) {
    return calls.filter(call => call.command === command)
  }

  return { on, set, invoke, calls, callsOf }
}
